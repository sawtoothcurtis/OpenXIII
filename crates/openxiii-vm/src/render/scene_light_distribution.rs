//! FLight lists of Scene.
//! The traversal/culling stays that of the real render_level; each visible node
//! distributes the lists before its two children. The indices identify the
//! immutable records of this single pass, without compacting the NULL slots.

use openxiii_assets::{bsp::BspModel, lighting::Light};

/// One head per destination and a single array of links. A view without
/// light keeps the number of destinations, without allocating these arrays.
#[derive(Clone, Debug, Default)]
pub struct LightLists {
    count: usize,
    heads: Vec<usize>,
    links: Vec<(usize, usize)>, // (light, previous head), 0 = end
}

impl LightLists {
    fn new(count: usize) -> Self {
        Self { count, ..Self::default() }
    }

    fn head(&self, index: usize) -> usize {
        self.heads.get(index).copied().unwrap_or(0)
    }

    fn push(&mut self, index: usize, light: usize) {
        if index >= self.count { return; }
        if self.heads.is_empty() {
            self.heads.resize(self.count, 0);
        }
        self.links.push((light, self.heads[index]));
        self.heads[index] = self.links.len();
    }

    pub fn lights(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let mut head = self.head(index);
        std::iter::from_fn(move || {
            let &(light, next) = self.links.get(head.checked_sub(1)?)?;
            head = next;
            Some(light)
        })
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }
}

impl PartialEq for LightLists {
    fn eq(&self, other: &Self) -> bool {
        self.count == other.count && self.links.len() == other.links.len()
            && (self.links.is_empty() || (0..self.count).all(|i| self.lights(i).eq(other.lights(i))))
    }
}

// To rebuild a snapshot from a record's ordered lists.
impl FromIterator<Vec<usize>> for LightLists {
    fn from_iter<T: IntoIterator<Item = Vec<usize>>>(iter: T) -> Self {
        let mut result = Self::default();
        for list in iter {
            let index = result.count;
            result.count += 1;
            if !result.heads.is_empty() { result.heads.push(0); }
            for light in list.into_iter().rev() { result.push(index, light); }
        }
        result
    }
}

#[derive(Default)]
pub(crate) struct SceneLightDistribution {
    pub node_lights: LightLists,
    pub leaf_lights: LightLists,
    pub surface_lights: LightLists,
}

impl SceneLightDistribution {
    pub fn new(model: &BspModel, root: &[usize], mut node_lights: LightLists) -> Self {
        // The parent restores this buffer after its traversal, before the child views.
        node_lights.count = model.nodes.len();
        node_lights.heads.clear();
        node_lights.links.clear();
        for &light in root.iter().rev() { node_lights.push(0, light); }
        Self {
            node_lights,
            leaf_lights: LightLists::new(model.leaf_zones.len()),
            surface_lights: LightLists::new(model.surfs.len()),
        }
    }

    /// Child if it exists, otherwise leaf. Each insertion is
    /// at the head; two sides giving the same leaf keep the duplicates.
    fn child(&mut self, model: &BspModel, node: usize, side: usize, light: usize) {
        let n = &model.nodes[node];
        if n.child[side] != -1 {
            self.node_lights.push(n.child[side] as usize, light);
        } else if n.leaf[side] != -1 {
            self.leaf_lights.push(n.leaf[side] as usize, light);
        }
    }

    /// The original deduplicates a surface's list by FLight pointer, then
    /// inserts at the head. The child/leaf lists do not do this deduplication.
    fn surface(&mut self, surf: i16, light: usize) {
        let index = surf as usize;
        if !self.surface_lights.lights(index).any(|candidate| candidate == light) {
            self.surface_lights.push(index, light);
        }
    }

    /// Called only after the zone/frustum guards of the real traversal.
    /// f32 rounding: plane (Z*nZ + Y*nY) + X*nX, sphere (X*X + Y*Y) + Z*Z.
    pub fn node(&mut self, model: &BspModel, node: usize, lights: &[Light]) {
        let mut head = self.node_lights.head(node);
        if head == 0 { return; }
        let plane = model.nodes[node].plane;
        while head != 0 {
            let (index, next) = self.node_lights.links[head - 1];
            head = next;
            let Some(light) = lights.get(index) else { continue };
            if light.effect == 20 {
                // Sun on all coplanar surfaces.
                let mut current = node as i32;
                while current != -1 {
                    let n = &model.nodes[current as usize];
                    self.surface(n.surf, index);
                    current = n.coplanar as i32;
                }
            } else {
                let p = light.location;
                let distance = ((p[2] * plane[2] + p[1] * plane[1]) + p[0] * plane[0]) - plane[3];
                if distance < -light.radius {
                    self.child(model, node, 0, index);
                    continue;
                }
                if distance > light.radius {
                    self.child(model, node, 1, index);
                    continue;
                }
                // Strict intersection of the surface spheres.
                let mut current = node as i32;
                while current != -1 {
                    let n = &model.nodes[current as usize];
                    let d = [p[0] - n.sphere[0], p[1] - n.sphere[1], p[2] - n.sphere[2]];
                    let radius = n.sphere[3] + light.radius;
                    let squared = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
                    if radius * radius > squared {
                        self.surface(n.surf, index);
                    }
                    current = n.coplanar as i32;
                }
            }
            // Front first, back next, by head insertion.
            self.child(model, node, 1, index);
            self.child(model, node, 0, index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_shared_and_empty_lists() {
        let mut distribution = SceneLightDistribution {
            leaf_lights: LightLists::new(50_000),
            surface_lights: LightLists::new(3),
            ..Default::default()
        };
        assert!(distribution.leaf_lights.lights(49_999).next().is_none());
        assert_eq!(distribution.leaf_lights.heads.capacity(), 0);
        assert_eq!(distribution.leaf_lights.links.capacity(), 0);

        for (surface, light) in [(1, 3), (0, 7), (1, 5), (1, 3), (-1, 9)] {
            distribution.surface(surface, light);
        }
        let expected: LightLists = [vec![7], vec![5, 3], vec![]].into_iter().collect();
        assert_eq!(distribution.surface_lights, expected);

        for light in [3, 5, 3] { distribution.leaf_lights.push(1, light); }
        assert!(distribution.leaf_lights.lights(1).eq([3, 5, 3]));
    }
}
