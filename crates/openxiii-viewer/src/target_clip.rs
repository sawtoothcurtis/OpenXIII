//! Rejection of fixed opaque batches outside a comic panel's projection.
use super::{Mat, World};
use std::ops::Range;

const BLOCK_INDICES: usize = 32 * 3;

#[derive(Clone, Copy)]
pub(super) struct Bounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl Bounds {
    pub(super) fn from_world(points: impl IntoIterator<Item = [f32; 3]>) -> Option<Self> {
        let mut b = Self::empty();
        for [x, y, z] in points {
            if !x.is_finite() || !y.is_finite() || !z.is_finite() { return None; }
            b.include([x, -y, z]);
        }
        b.min[0].is_finite().then_some(b)
    }
    fn empty() -> Self {
        Self { min: [f32::INFINITY; 3], max: [f32::NEG_INFINITY; 3] }
    }

    fn include(&mut self, p: [f32; 3]) {
        for (k, v) in p.into_iter().enumerate() {
            self.min[k] = self.min[k].min(v);
            self.max[k] = self.max[k].max(v);
        }
    }
}

pub(super) struct Section {
    whole: Bounds,
    blocks: Vec<Bounds>,
    count: u32,
}

pub(super) fn world_bounds(world: &World) -> Vec<Option<Section>> {
    world
        .batches
        .iter()
        .map(|batch| {
            let first = batch.first_index as usize;
            let indices = world.indices.get(first..first.checked_add(batch.index_count as usize)?)?;
            if indices.is_empty() || indices.len() % 3 != 0 {
                return None;
            }
            let mut whole = Bounds::empty();
            let mut blocks = Vec::new();
            let (mut low, mut high) = (u32::MAX, 0);
            for chunk in indices.chunks(BLOCK_INDICES) {
                let mut block = Bounds::empty();
                for &i in chunk {
                    let [x, y, z] = world.vertices.get(i as usize)?.position;
                    if ![x, y, z].iter().all(|v| v.is_finite()) {
                        return None;
                    }
                    // Same Y flip as vs_main, before fr.view_proj.
                    block.include([x, -y, z]);
                    low = low.min(i);
                    high = high.max(i);
                }
                whole.include(block.min);
                whole.include(block.max);
                if indices.len() > BLOCK_INDICES {
                    blocks.push(block);
                }
            }
            // SinusModifier vertices are rewritten after the map is built.
            let animated = world.sine.iter().any(|s| high >= s.base && (low as usize) < s.base as usize + s.local.len());
            (!animated).then_some(Section { whole, blocks, count: batch.index_count })
        })
        .collect()
}

pub(super) struct Frustum {
    planes: [[f64; 4]; 6],
    linear: f64,
    translation: f64,
}

impl Frustum {
    pub(super) fn new(m: Mat) -> Self {
        // wgpu homogeneous coordinates: -w <= x,y <= w and 0 <= z <= w.
        let planes = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let col = m[j].map(f64::from);
                match i {
                    0 => col[3] + col[0],
                    1 => col[3] - col[0],
                    2 => col[3] + col[1],
                    3 => col[3] - col[1],
                    4 => col[2],
                    _ => col[3] - col[2],
                }
            })
        });
        let norm = |col: [f32; 4]| col.into_iter().map(|v| f64::from(v).abs()).fold(0.0, f64::max);
        let linear = if m.iter().flatten().all(|v| v.is_finite()) { m[..3].iter().copied().map(norm).sum() } else { f64::INFINITY };
        let translation = norm(m[3]);
        Self { planes, linear, translation }
    }

    fn outside(&self, b: &Bounds) -> bool {
        let extent = b.min.iter().chain(&b.max).map(|&v| f64::from(v).abs()).fold(0.0, f64::max);
        let scale = self.linear * extent + self.translation;
        // Two f32 matrix-vector products (components w and x/y/z), with or without
        // contraction. Keep a wide margin for rounding and denormals.
        if !scale.is_finite() || scale >= f64::from(f32::MAX) / 16.0 {
            return false;
        }
        let margin = 64.0 * f64::from(f32::EPSILON) * scale.max(1.0);
        self.planes.iter().any(|p| {
            let mut maximum = p[3];
            for k in 0..3 {
                maximum += p[k] * f64::from(if p[k] >= 0.0 { b.max[k] } else { b.min[k] });
            }
            maximum < -margin
        })
    }

    // Keep the internal holes: a single range in the original immutable buffer.
    fn range(&self, section: &Section) -> Range<u32> {
        if self.outside(&section.whole) {
            return 0..0;
        }
        if section.blocks.is_empty() {
            return 0..section.count;
        }
        let visible = |b: &Bounds| !self.outside(b);
        let Some(first) = section.blocks.iter().position(visible) else {
            return 0..0;
        };
        let last = section.blocks[first + 1..].iter().rposition(visible).map_or(first, |r| first + 1 + r);
        (first * BLOCK_INDICES) as u32..((last + 1) * BLOCK_INDICES).min(section.count as usize) as u32
    }
}

pub(super) struct TargetClip {
    pub(super) scene: Frustum,
    pub(super) sky: Option<Frustum>,
}

impl TargetClip {
    pub(super) fn visible_stages(&self, b: &Bounds) -> (bool, bool) {
        (!self.scene.outside(b), self.sky.as_ref().is_some_and(|s| !s.outside(b)))
    }
}

type MatrixBits = [[u32; 4]; 4];

#[derive(Default)]
pub(super) struct RangeCache {
    pub(super) generation: u64,
    projection: Option<(MatrixBits, Option<MatrixBits>)>,
    sections: Vec<Option<(Range<u32>, Range<u32>)>>,
    pub(super) projector_stages: Vec<Option<(bool, bool)>>,
}

impl RangeCache {
    pub(super) fn begin(&mut self, scene: Mat, sky: Option<Mat>, sections: usize) {
        let bits = |m: Mat| m.map(|col| col.map(f32::to_bits));
        let projection = (bits(scene), sky.map(bits));
        if self.projection != Some(projection) {
            self.projection = Some(projection);
            self.generation = self.generation.wrapping_add(1);
            self.sections.clear();
            self.sections.resize(sections, None);
            self.projector_stages.clear();
        }
    }

    pub(super) fn visible(&mut self, i: usize, section: &Section, clip: &TargetClip, light_sky: Option<bool>) -> Option<(Range<u32>, Option<bool>)> {
        let (scene, sky) = self.sections[i].get_or_insert_with(|| (clip.scene.range(section), clip.sky.as_ref().map_or(0..0, |s| s.range(section))));
        match (light_sky != Some(true) && !scene.is_empty(), light_sky != Some(false) && !sky.is_empty()) {
            (true, true) => Some((scene.start.min(sky.start)..scene.end.max(sky.end), None)),
            (true, false) => Some((scene.clone(), Some(false))),
            (false, true) => Some((sky.clone(), Some(true))),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Camera, Renderer, FAR_PLANE, NEAR_PLANE};
    use super::*;

    #[test]
    fn target_clips_only_wholly_outside_each_camera() {
        let view = |yaw| {
            let camera = Camera { position: [0.; 3], pitch: 0., yaw, roll: 0. };
            Frustum::new(Renderer::frame_uniform(&camera, 90f32.to_radians(), 1., NEAR_PLANE, FAR_PLANE, None).view_proj)
        };
        let clip = TargetClip { scene: view(0.), sky: Some(view(180.)) };
        let front = Bounds { min: [10., -1., -1.], max: [11., 1., 1.] };
        let section = |whole| Section { whole, blocks: Vec::new(), count: 3 };
        let mut cache = RangeCache { sections: vec![None; 2], ..Default::default() };
        assert_eq!(cache.visible(0, &section(front), &clip, None), Some((0..3, Some(false))));
        assert_eq!(cache.visible(0, &section(front), &clip, Some(true)), None);
        let behind = Bounds { min: [-11., -1., -1.], max: [-10., 1., 1.] };
        assert_eq!(cache.visible(1, &section(behind), &clip, None), Some((0..3, Some(true))));
        let tangent = Bounds { min: [10., 10., -1.], max: [10., 16., 1.] };
        assert!(!clip.scene.outside(&tangent));
        let spanning = Bounds { min: [10., -20., -20.], max: [10., 20., 20.] };
        assert!(!clip.scene.outside(&spanning));
    }

    #[test]
    fn target_bounds_exclude_animated_and_invalid_indices() {
        use openxiii_assets::level::MeshActor;
        use openxiii_assets::world::{Batch, SineSection, Vertex};
        let mut world = World::default();
        world.vertices = vec![Vertex::default(); 4];
        world.indices = vec![0, 1, 2, 1, 2, 3, 0, 1, 99];
        world.batches = (0..3).map(|i| Batch { material: String::new(), first_index: 3 * i, index_count: 3 }).collect();
        world.sine.push(SineSection {
            modifier: String::new(),
            owner: None,
            base: 3,
            local: vec![[0.; 3]],
            fade: [0.; 2],
            actor: MeshActor {
                actor: String::new(),
                mesh: String::new(),
                location: [0.; 3],
                rotation: [0; 3],
                scale: [1.; 3],
                pre_pivot: [0.; 3],
                blocks: false,
            },
        });
        assert_eq!(world_bounds(&world).iter().map(Option::is_some).collect::<Vec<_>>(), [true, false, false]);
        world.vertices[0].position[1] = f32::NAN;
        assert!(world_bounds(&world)[0].is_none());
    }

    #[test]
    fn target_range_preserves_interior_holes_and_refreshes_after_a_camera_move() {
        let matrix = |yaw| {
            Renderer::frame_uniform(&Camera { position: [0.; 3], pitch: 0., yaw, roll: 0. }, 90f32.to_radians(), 1., NEAR_PLANE, FAR_PLANE, None).view_proj
        };
        let (a, b) = (matrix(0.), matrix(180.));
        let near = Bounds { min: [10., -1., -1.], max: [11., 1., 1.] };
        let far = Bounds { min: [-11., -1., -1.], max: [-10., 1., 1.] };
        let section = Section { whole: Bounds { min: far.min, max: near.max }, blocks: vec![far, near, far, near, far], count: 4 * 96 + 3 };
        let mut cache = RangeCache::default();
        let mut clip = TargetClip { scene: Frustum::new(a), sky: None };
        cache.begin(a, None, 1);
        assert_eq!(cache.visible(0, &section, &clip, None), Some((96..384, Some(false))));
        clip.sky = Some(Frustum::new(b));
        cache.begin(a, Some(b), 1);
        assert_eq!(cache.visible(0, &section, &clip, None), Some((0..387, None)));
        assert_eq!(cache.visible(0, &section, &clip, Some(false)), Some((96..384, Some(false))));
        clip.scene = Frustum::new(b);
        clip.sky = None;
        cache.begin(b, None, 1);
        assert_eq!(cache.visible(0, &section, &clip, None), Some((0..387, Some(false))));
    }
}
