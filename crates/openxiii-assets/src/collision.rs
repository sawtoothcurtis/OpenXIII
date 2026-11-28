//! Collision: a map's solid triangles, bounding volume hierarchy (BVH), ray casts
//! and sphere sweeps. Unreal coordinates.

pub type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: Vec3, s: f32) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}
fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 1e-12 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Contact point of a ray or sweep.
#[derive(Debug, Clone, Copy)]
pub struct Hit {
    /// Fraction of the path travelled before contact (0..1).
    pub time: f32,
    pub location: Vec3,
    pub normal: Vec3,
    pub triangle: usize,
}

#[derive(Debug, Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    fn empty() -> Self {
        Aabb { min: [f32::MAX; 3], max: [f32::MIN; 3] }
    }
    fn grow(&mut self, p: Vec3) {
        for (k, &c) in p.iter().enumerate() {
            self.min[k] = self.min[k].min(c);
            self.max[k] = self.max[k].max(c);
        }
    }
    /// Intersection of the segment start + t·dir (t in 0..1) with the box grown by `pad`.
    fn segment(&self, start: Vec3, inv: Vec3, pad: f32) -> bool {
        self.segment_box(start, inv, [pad; 3])
    }

    /// Like `segment`, box grown by a half-size per axis.
    fn segment_box(&self, start: Vec3, inv: Vec3, pad: Vec3) -> bool {
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            let (lo, hi) = (self.min[k] - pad[k], self.max[k] + pad[k]);
            if inv[k].is_infinite() {
                if start[k] < lo || start[k] > hi {
                    return false;
                }
                continue;
            }
            let (mut a, mut b) = ((lo - start[k]) * inv[k], (hi - start[k]) * inv[k]);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            t0 = t0.max(a);
            t1 = t1.min(b);
            if t0 > t1 {
                return false;
            }
        }
        true
    }
}

enum Node {
    Leaf { bounds: Aabb, first: usize, count: usize },
    Inner { bounds: Aabb, left: usize, right: usize },
}

pub struct CollisionMesh {
    pub triangles: Vec<[Vec3; 3]>,
    /// Tag of each triangle (material index in the map, 0 if none).
    pub tags: Vec<u32>,
    normals: Vec<Vec3>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}

impl CollisionMesh {
    pub fn new(triangles: Vec<[Vec3; 3]>) -> Self {
        let n = triangles.len();
        Self::new_tagged(triangles, vec![0; n])
    }

    /// Like `new`, with one tag per triangle (material…), returned by `Hit::triangle`.
    pub fn new_tagged(triangles: Vec<[Vec3; 3]>, tags: Vec<u32>) -> Self {
        let (triangles, tags): (Vec<[Vec3; 3]>, Vec<u32>) = triangles
            .into_iter()
            .zip(tags.into_iter().chain(std::iter::repeat(0)))
            .filter(|(t, _)| length(cross(sub(t[1], t[0]), sub(t[2], t[0]))) > 1e-6)
            .unzip();
        let normals = triangles.iter().map(|t| normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])))).collect();
        let mut mesh = CollisionMesh { order: (0..triangles.len()).collect(), triangles, tags, normals, nodes: Vec::new() };
        if !mesh.triangles.is_empty() {
            let n = mesh.triangles.len();
            mesh.build(0, n);
        }
        mesh
    }

    fn bounds(&self, first: usize, count: usize) -> Aabb {
        let mut b = Aabb::empty();
        for &i in &self.order[first..first + count] {
            for p in self.triangles[i] {
                b.grow(p);
            }
        }
        b
    }

    fn build(&mut self, first: usize, count: usize) -> usize {
        let bounds = self.bounds(first, count);
        let index = self.nodes.len();
        if count <= 8 {
            self.nodes.push(Node::Leaf { bounds, first, count });
            return index;
        }
        let extent = sub(bounds.max, bounds.min);
        let axis = if extent[0] >= extent[1] && extent[0] >= extent[2] {
            0
        } else if extent[1] >= extent[2] {
            1
        } else {
            2
        };
        let center = |t: &[Vec3; 3]| (t[0][axis] + t[1][axis] + t[2][axis]) / 3.0;
        let tris = &self.triangles;
        self.order[first..first + count].sort_by(|&a, &b| center(&tris[a]).total_cmp(&center(&tris[b])));
        self.nodes.push(Node::Leaf { bounds, first, count });
        let half = count / 2;
        let left = self.build(first, half);
        let right = self.build(first + half, count - half);
        self.nodes[index] = Node::Inner { bounds, left, right };
        index
    }

    /// First contact of a sphere of radius `radius` (0 = plain ray) from `start` to `end`.
    pub fn sweep(&self, start: Vec3, end: Vec3, radius: f32) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let dir = sub(end, start);
        let inv = dir.map(|d| if d.abs() < 1e-12 { f32::INFINITY } else { 1.0 / d });
        let mut best: Option<Hit> = None;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            match &self.nodes[n] {
                Node::Inner { bounds, left, right } => {
                    if bounds.segment(start, inv, radius + 1.0) {
                        stack.push(*left);
                        stack.push(*right);
                    }
                }
                Node::Leaf { bounds, first, count } => {
                    if !bounds.segment(start, inv, radius + 1.0) {
                        continue;
                    }
                    for &i in &self.order[*first..*first + *count] {
                        let limit = best.map_or(1.0, |h| h.time);
                        if let Some((t, normal)) = sweep_triangle(start, dir, radius, &self.triangles[i], self.normals[i], limit) {
                            best = Some(Hit { time: t, location: add(start, scale(dir, t)), normal, triangle: i });
                        }
                    }
                }
            }
        }
        best
    }

    /// Sweep of an axis-aligned box (half-sizes `extent`, zero for a ray) from
    /// `start` to `end`, like Unreal 2's extent traces against triangles (separating axes
    /// of FindSeparatingAxis: triangle normal, box axes, edges × axes). First contact.
    pub fn sweep_box(&self, start: Vec3, end: Vec3, extent: Vec3) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let dir = sub(end, start);
        let inv = dir.map(|d| if d.abs() < 1e-12 { f32::INFINITY } else { 1.0 / d });
        let pad = extent.map(|e| e + 1.0);
        let mut best: Option<Hit> = None;
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            match &self.nodes[n] {
                Node::Inner { bounds, left, right } => {
                    if bounds.segment_box(start, inv, pad) {
                        stack.push(*left);
                        stack.push(*right);
                    }
                }
                Node::Leaf { bounds, first, count } => {
                    if !bounds.segment_box(start, inv, pad) {
                        continue;
                    }
                    for &i in &self.order[*first..*first + *count] {
                        let limit = best.map_or(1.0, |h| h.time);
                        if let Some((t, normal)) = box_triangle(start, end, extent, &self.triangles[i], self.normals[i]) {
                            if t < limit || (best.is_none() && t <= limit) {
                                best = Some(Hit { time: t, location: add(start, scale(dir, t)), normal, triangle: i });
                            }
                        }
                    }
                }
            }
        }
        best
    }

    /// Overlap of a stationary box (centre, half-sizes) with the triangles: first triangle
    /// that cuts it, with the axis of least penetration as the normal (towards the box).
    pub fn overlap_box(&self, center: Vec3, extent: Vec3) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inv = [f32::INFINITY; 3];
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            match &self.nodes[n] {
                Node::Inner { bounds, left, right } => {
                    if bounds.segment_box(center, inv, extent) {
                        stack.push(*left);
                        stack.push(*right);
                    }
                }
                Node::Leaf { bounds, first, count } => {
                    if !bounds.segment_box(center, inv, extent) {
                        continue;
                    }
                    for &i in &self.order[*first..*first + *count] {
                        if let Some(normal) = box_triangle_overlap(center, extent, &self.triangles[i], self.normals[i]) {
                            return Some(Hit { time: 0.0, location: center, normal, triangle: i });
                        }
                    }
                }
            }
        }
        None
    }
}

/// Threshold for zero projections (Unreal's DELTA).
const DELTA: f32 = 0.00001;

/// Separating axes of a triangle and an axis-aligned box: normal, X, Y, Z, then the nine
/// edge × axis products (nearly zero axes are ignored, as are those parallel to the
/// normal: already tested, their normalisation off by one ulp would give a flat triangle an
/// empty contact interval for a zero-extent trace crossing it).
fn separating_axes(t: &[Vec3; 3], n: Vec3) -> impl Iterator<Item = Vec3> {
    let edges = [sub(t[1], t[0]), sub(t[2], t[1]), sub(t[0], t[2])];
    let boxes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let crosses = (0..9)
        .map(move |k| cross(edges[k / 3], boxes[k % 3]))
        .filter(|a| dot(*a, *a) > 1e-8)
        .map(normalize)
        .filter(move |a| dot(*a, n).abs() < 0.999_999);
    std::iter::once(n).chain(boxes).chain(crosses)
}

/// One step of FindSeparatingAxis (TestSeparatingAxis): narrows the time interval during which the
/// moving box overlaps the triangle along axis `line`. False if the axis separates the whole path.
#[allow(clippy::too_many_arguments)]
fn test_axis(t: &[Vec3; 3], line: Vec3, start: Vec3, end: Vec3, extent: Vec3, min_t: &mut f32, max_t: &mut f32, normal: &mut Vec3) -> bool {
    let projected_extent = line[0].abs() * extent[0] + line[1].abs() * extent[1] + line[2].abs() * extent[2];
    let (ps, pe) = (dot(line, start), dot(line, end));
    let pd = pe - ps;
    let (p0, p1, p2) = (dot(line, t[0]), dot(line, t[1]), dot(line, t[2]));
    let tri_min = p0.min(p1.min(p2)) - projected_extent;
    let tri_max = p0.max(p1.max(p2)) + projected_extent;
    if ps < tri_min {
        if pd < DELTA {
            return false;
        }
        let time = (tri_min - ps) / pd;
        if time > *min_t {
            *min_t = time;
            *normal = scale(line, -1.0);
        }
        *max_t = max_t.min((tri_max - ps) / pd);
    } else if ps > tri_max {
        if pd > -DELTA {
            return false;
        }
        let time = (tri_max - ps) / pd;
        if time > *min_t {
            *min_t = time;
            *normal = line;
        }
        *max_t = max_t.min((tri_min - ps) / pd);
    } else if pd > DELTA {
        *max_t = max_t.min((tri_max - ps) / pd);
    } else if pd < -DELTA {
        *max_t = max_t.min((tri_min - ps) / pd);
    }
    *max_t >= *min_t
}

/// Time (0..1) and normal of the first contact of a box moving from `start` to `end` with a
/// triangle. A box already overlapping the triangle at the start is in contact along the axis of
/// least penetration (facing the box): it hits immediately if it pushes in
/// along that axis, like a trace starting inside BSP solid; it can always move out
/// or slide along.
fn box_triangle(start: Vec3, end: Vec3, extent: Vec3, t: &[Vec3; 3], n: Vec3) -> Option<(f32, Vec3)> {
    let (mut min_t, mut max_t) = (-f32::MAX, f32::MAX);
    let mut normal = [0.0; 3];
    for axis in separating_axes(t, n) {
        if !test_axis(t, axis, start, end, extent, &mut min_t, &mut max_t, &mut normal) {
            return None;
        }
    }
    if min_t == -f32::MAX {
        let side = box_triangle_overlap(start, extent, t, n)?;
        // Relative threshold (0.1 % of the path): sliding along the contact does not block.
        let motion = sub(end, start);
        let into = dot(motion, side);
        return (into < -1e-3 * length(motion)).then_some((0.0, side));
    }
    (0.0..=1.0).contains(&min_t).then_some((min_t, normal))
}

/// Overlap of a stationary box and a triangle; normal of least penetration.
fn box_triangle_overlap(center: Vec3, extent: Vec3, t: &[Vec3; 3], n: Vec3) -> Option<Vec3> {
    let mut best: Option<(f32, Vec3)> = None;
    for axis in separating_axes(t, n) {
        let r = axis[0].abs() * extent[0] + axis[1].abs() * extent[1] + axis[2].abs() * extent[2];
        let c = dot(axis, center);
        let (p0, p1, p2) = (dot(axis, t[0]), dot(axis, t[1]), dot(axis, t[2]));
        let (lo, hi) = (p0.min(p1.min(p2)) - r, p0.max(p1.max(p2)) + r);
        if c <= lo || c >= hi {
            return None;
        }
        let (depth, dir) = if c - lo < hi - c { (c - lo, scale(axis, -1.0)) } else { (hi - c, axis) };
        if best.is_none_or(|(d, _)| depth < d) {
            best = Some((depth, dir));
        }
    }
    best.map(|(_, d)| d)
}

/// Smallest root in [0, limit] of a·t² + b·t + c = 0.
fn lowest_root(a: f32, b: f32, c: f32, limit: f32) -> Option<f32> {
    let det = b * b - 4.0 * a * c;
    if det < 0.0 || a.abs() < 1e-12 {
        return None;
    }
    let s = det.sqrt();
    let (mut r1, mut r2) = ((-b - s) / (2.0 * a), (-b + s) / (2.0 * a));
    if r1 > r2 {
        std::mem::swap(&mut r1, &mut r2);
    }
    if r1 >= 0.0 && r1 <= limit {
        return Some(r1);
    }
    if r2 >= 0.0 && r2 <= limit {
        return Some(r2);
    }
    None
}

/// Sweep of a sphere against a triangle (K. Fauerby's method); triangles are
/// treated as double-sided. Returns the contact fraction and the contact normal.
fn sweep_triangle(start: Vec3, dir: Vec3, r: f32, t: &[Vec3; 3], n: Vec3, limit: f32) -> Option<(f32, Vec3)> {
    // A movement whose component towards the obstacle is below 0.1 % is a slide: it
    // must not block a sphere already in contact (rounding errors, barely tilted walls).
    let len = dot(dir, dir).sqrt();
    let grazing = 1e-3 * len;
    let mut normal = n;
    let mut dist = dot(sub(start, t[0]), normal);
    if dist < 0.0 {
        normal = scale(normal, -1.0);
        dist = -dist;
    }
    let along = dot(dir, normal);
    // Triangle face: only if the movement really goes towards it.
    if along < -grazing {
        let t0 = (r - dist) / along;
        if t0 >= -1e-4 && t0 <= limit {
            let t0 = t0.max(0.0);
            let p = sub(add(start, scale(dir, t0)), scale(normal, r));
            if inside(p, t) {
                return Some((t0, normal));
            }
        } else if t0 < 0.0 && dist < r && dist > 0.0 {
            // Already in contact at the start: no further penetration allowed.
            let p = sub(start, scale(normal, dist));
            if inside(p, t) {
                return Some((0.0, normal));
            }
        }
    }
    if r <= 0.0 {
        return None;
    }
    // Vertices and edges.
    let mut best: Option<(f32, Vec3)> = None;
    let a = dot(dir, dir);
    for v in t {
        let b = 2.0 * dot(dir, sub(start, *v));
        let c = dot(sub(start, *v), sub(start, *v)) - r * r;
        // Already in contact and moving away (or sliding): let it out.
        if c <= 0.0 && b >= -2.0 * grazing * dot(sub(start, *v), sub(start, *v)).sqrt() {
            continue;
        }
        let lim = best.map_or(limit, |h| h.0);
        if let Some(x) = lowest_root(a, b, c, lim) {
            let p = add(start, scale(dir, x));
            best = Some((x, normalize(sub(p, *v))));
        }
    }
    for k in 0..3 {
        let (p0, p1) = (t[k], t[(k + 1) % 3]);
        let edge = sub(p1, p0);
        let base = sub(p0, start);
        let (ee, ed, eb) = (dot(edge, edge), dot(edge, dir), dot(edge, base));
        let aa = ee * -a + ed * ed;
        let bb = ee * (2.0 * dot(dir, base)) - 2.0 * ed * eb;
        let cc = ee * (r * r - dot(base, base)) + eb * eb;
        let lim = best.map_or(limit, |h| h.0);
        // Initial contact with the edge and movement away from it: ignored.
        let f0 = (-eb / ee).clamp(0.0, 1.0);
        let closest = add(p0, scale(edge, f0));
        let away = sub(start, closest);
        if dot(away, away) <= r * r && dot(away, dir) >= -grazing * dot(away, away).sqrt() {
            continue;
        }
        if let Some(x) = lowest_root(aa, bb, cc, lim) {
            let f = (ed * x - eb) / ee;
            if (0.0..=1.0).contains(&f) {
                let p = add(start, scale(dir, x));
                let q = add(p0, scale(edge, f));
                best = Some((x, normalize(sub(p, q))));
            }
        }
    }
    best
}

fn inside(p: Vec3, t: &[Vec3; 3]) -> bool {
    let (e0, e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]), sub(p, t[0]));
    let (d00, d01, d11, d20, d21) = (dot(e0, e0), dot(e0, e1), dot(e1, e1), dot(e2, e0), dot(e2, e1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return false;
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    v >= -1e-4 && w >= -1e-4 && v + w <= 1.0 + 1e-4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> CollisionMesh {
        CollisionMesh::new(vec![
            [[-100.0, -100.0, 0.0], [100.0, -100.0, 0.0], [100.0, 100.0, 0.0]],
            [[-100.0, -100.0, 0.0], [100.0, 100.0, 0.0], [-100.0, 100.0, 0.0]],
        ])
    }

    #[test]
    fn ray_hits_floor() {
        let h = floor().sweep([0.0, 0.0, 50.0], [0.0, 0.0, -50.0], 0.0).unwrap();
        assert!((h.time - 0.5).abs() < 1e-4);
        assert!((h.normal[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sphere_stops_above_floor() {
        let h = floor().sweep([0.0, 0.0, 50.0], [0.0, 0.0, -50.0], 10.0).unwrap();
        assert!((h.location[2] - 10.0).abs() < 1e-3, "{h:?}");
    }

    #[test]
    fn misses_outside() {
        assert!(floor().sweep([500.0, 0.0, 50.0], [500.0, 0.0, -50.0], 10.0).is_none());
    }

    #[test]
    fn box_stops_above_floor() {
        let h = floor().sweep_box([0.0, 0.0, 50.0], [0.0, 0.0, -50.0], [10.0, 10.0, 20.0]).unwrap();
        assert!((h.location[2] - 20.0).abs() < 1e-3, "{h:?}");
        assert!((h.normal[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn box_slides_and_leaves_floor() {
        let f = floor();
        // Resting on the ground: sliding along it or moving away does not block.
        assert!(f.sweep_box([0.0, 0.0, 19.5], [50.0, 0.0, 19.5], [10.0, 10.0, 20.0]).is_none());
        assert!(f.sweep_box([0.0, 0.0, 19.5], [0.0, 0.0, 60.0], [10.0, 10.0, 20.0]).is_none());
        // Pushing into it: immediate contact, normal pointing up.
        let h = f.sweep_box([0.0, 0.0, 19.5], [0.0, 0.0, 10.0], [10.0, 10.0, 20.0]).unwrap();
        assert_eq!(h.time, 0.0);
        assert!((h.normal[2] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sphere_hits_edge() {
        let h = floor().sweep([105.0, 0.0, 50.0], [105.0, 0.0, -50.0], 10.0).unwrap();
        assert!(h.location[2] > 0.0 && h.location[2] < 10.0, "{h:?}");
    }
}

#[cfg(test)]
mod tests_flat {
    use super::*;

    /// Flat floor of Toits01 under PositionInfo18: a vertical zero-extent trace hits it
    /// whatever its length.
    #[test]
    fn trace_vertical_triangle_flat() {
        let t = [[7810.999, 141.00024, 1335.9966], [7718.4287, 224.0481, 1335.9966], [7810.999, 3629.0002, 1335.9966]];
        let n = normalize(cross(sub(t[1], t[0]), sub(t[2], t[0])));
        for end in [398.26f32, 0.0, -1000.0, -3000.0] {
            let (s, e) = ([7785.66f32, 1613.68, 1398.26], [7785.66f32, 1613.68, end]);
            let hit = box_triangle(s, e, [0.0; 3], &t, n).map(|(time, _)| s[2] + (end - s[2]) * time);
            assert!(hit.is_some_and(|z| (z - 1335.9966).abs() < 0.01), "end {end}: {hit:?}");
        }
    }
}
