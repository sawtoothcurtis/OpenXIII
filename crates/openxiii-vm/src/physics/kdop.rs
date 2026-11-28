//! Collision tree of StaticMeshes (kDOP), walked by UStaticMesh::LineCheck and
//! PointCheck. The tree is serialized with the mesh: nodes of
//! 12 bytes (triangle, coplanar node, back child, front child, 0xFFFF: none) whose
//! box is quantized on 5 bits per bound within the parent's; the root
//! starts from the primitive's BoundingBox (UPrimitive + 0x30). Everything happens in the mesh's
//! frame (AActor::WorldToLocal), except the triangle tests with extent, done
//! in the world (vertices passed through LocalToWorld).
//! - Trace without extent: ray against the node's box
//!   (FLineBoxIntersection), distances to the triangle's plane (±0.001), the coplanar
//!   node, the near side, then the ray-triangle intersection (two edge tests); the
//!   first contact below Hit.Time ends it.
//! - Trace with extent: node box grown by the local
//!   half-extent, plane at ± r, triangle tested once per trace (array)
//!   through swept separating axes (the triangle's box, its plane both ways,
//!   edges × axes normalized).
//! - PointCheck: box containing the point, static separating
//!   axes, the smallest penetration wins.
//!
//! Operations in machine-code order, in 32-bit floats (24-bit FPU).

use crate::physics::V;
use crate::radius_iter::{transform_box, transform_fvector};
use openxiii_assets::coords::{self, Matrix};
use openxiii_assets::static_collision::CollisionNode;
use std::cell::RefCell;

const NONE: u16 = 0xFFFF;
/// 1 / 31.
const QUANT: f32 = 0.032_258_063;
const PLANE_EPS: f32 = 0.001;
/// BOX_SIDE_THRESHOLD.
const BOX_SIDE: f32 = 0.1;

/// Box: min x, y, z then max x, y, z.
pub(crate) type Bx6 = [f32; 6];
type Plane = [f32; 4];

/// One collision set of the mesh (full or simplified): vertices, triangles (three vertices,
/// material), tree, material label of each triangle.
pub(crate) struct KdopSet {
    pub(crate) verts: Vec<V>,
    pub(crate) tris: Vec<[u16; 4]>,
    pub(crate) nodes: Vec<CollisionNode>,
    pub(crate) tags: Vec<u32>,
    /// Plane of each triangle in the mesh's frame (`plane`), computed once.
    planes: Vec<Plane>,
    /// The set's own number, for the triangles kept per frame (`Frame::world`).
    id: u64,
}

/// Frame of the actor: LocalToWorld, WorldToLocal and the sign of LocalToWorld's determinant.
pub(crate) struct Frame {
    pub(crate) l2w: Matrix,
    pub(crate) w2l: Matrix,
    pub(crate) mirrored: bool,
    /// The frame's own number (valid entries of `world_value`).
    number: u64,
    /// Triangles already transformed into the world by this frame (`world_triangle`), for the set whose
    /// number is recorded: the frame never changes, the result is the same every time.
    world: RefCell<(u64, Vec<Option<([V; 3], Plane)>>)>,
    world_value: RefCell<CacheWorld>,
}

/// World triangles kept by a frame: (number of the frame and of the set, vertices, plane).
#[derive(Default)]
struct CacheWorld {
    tris: Vec<((u64, u64), [V; 3], Plane)>,
}

/// A frame's own number (never 0: the key of an empty entry).
fn number_frame() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Contact: Hit.Location (PointCheck), Hit.Normal (not normalized), Hit.Time, Hit.Item.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KHit {
    pub(crate) location: V,
    pub(crate) normal: V,
    pub(crate) time: f32,
    pub(crate) triangle: usize,
}

thread_local! {
    /// Triangles already tested by the current trace with extent (triangle tested if
    /// its number equals the trace's, instead of clearing the array for each trace).
    static VISITED: RefCell<(Vec<u32>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

/// FInverseRotationMatrix (same computation as in visibility.rs).
fn inverse_rotation_matrix(r: [i32; 3]) -> Matrix {
    let s = coords::gmath_sin;
    let sr = s(r[2].wrapping_neg());
    let cr = s(0x4000i32.wrapping_sub(r[2]));
    let sy = s(r[1].wrapping_neg());
    let cy = s(0x4000i32.wrapping_sub(r[1]));
    let sp = s(r[0].wrapping_neg());
    let cp = s(0x4000i32.wrapping_sub(r[0]));
    let mut m = [0.0f32; 16];
    m[0] = cp * cy;
    m[1] = sy * cr + sp * cy * sr;
    m[2] = sp * cy * cr - sy * sr;
    m[4] = -(cp * sy);
    m[5] = cy * cr - sp * sy * sr;
    m[6] = -(cy * sr) - sp * sy * cr;
    m[8] = -sp;
    m[9] = cp * sr;
    m[10] = cp * cr;
    m[15] = 1.0;
    m
}

pub(crate) fn translation(t: V) -> Matrix {
    let mut m = [0.0f32; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m[15] = 1.0;
    m
}

/// Sign of the determinant (computed on 80 bits by the engine).
fn determinant(m: &Matrix) -> f64 {
    let m = m.map(f64::from);
    let f2 = m[10] * m[15] - m[14] * m[11];
    let f3 = m[15] * m[6] - m[14] * m[7];
    let f4 = m[11] * m[6] - m[10] * m[7];
    let f5 = m[15] * m[2] - m[14] * m[3];
    let f6 = m[11] * m[2] - m[10] * m[3];
    let f1 = m[7] * m[2] - m[6] * m[3];
    ((f1 * m[13] + (f3 * m[1] - f5 * m[5])) * m[8] + ((f4 * m[13] + (f2 * m[5] - f3 * m[9])) * m[0] - (f6 * m[13] + (f2 * m[1] - f5 * m[9])) * m[4]))
        - (f1 * m[9] + (f4 * m[1] - f6 * m[5])) * m[12]
}

/// Frame of an actor (Rotation, DrawScale3D, Location, PrePivot, DrawScale). WorldToLocal
/// ((T(−Location) × InverseRotation) × Scale(1 / DrawScale / DrawScale3D)) ×
/// T(PrePivot).
pub(crate) fn frame(r: [i32; 3], s3: V, l: V, pp: V, ds: f32) -> Frame {
    let l2w = crate::radius_iter::local_to_world(r, s3, l, pp, ds);
    let s = 1.0 / ds;
    let mut scale = [0.0f32; 16];
    scale[0] = s * (1.0 / s3[0]);
    scale[5] = (1.0 / s3[1]) * s;
    scale[10] = (1.0 / s3[2]) * s;
    scale[15] = 1.0;
    let a = coords::matrix_mul(&translation([-l[0], -l[1], -l[2]]), &inverse_rotation_matrix(r));
    let b = coords::matrix_mul(&a, &scale);
    let w2l = coords::matrix_mul(&b, &translation(pp));
    let mirrored = determinant(&l2w) < 0.0;
    Frame { l2w, w2l, mirrored, world: RefCell::new((0, Vec::new())), world_value: RefCell::new(CacheWorld::default()), number: number_frame() }
}

/// `frame` that takes over the triangle cache array of `prior` (replaced frame of the same
/// actor): its entries are no longer valid (another number), its memory is reused.
pub(crate) fn frame_resume(r: [i32; 3], s3: V, l: V, pp: V, ds: f32, prior: Frame) -> Frame {
    let f = frame(r, s3, l, pp, ds);
    *f.world_value.borrow_mut() = prior.world_value.into_inner();
    f
}

/// Box of a node within its parent's.
fn decode(b: u32, p: &Bx6) -> Bx6 {
    let fx = (p[3] - p[0]) * QUANT;
    let fy = (p[4] - p[1]) * QUANT;
    let fz = (p[5] - p[2]) * QUANT;
    [
        (b >> 27) as f32 * fx + p[0],
        (b >> 22 & 0x1F) as f32 * fy + p[1],
        (b >> 17 & 0x1F) as f32 * fz + p[2],
        (b >> 11 & 0x1F) as f32 * fx + p[0],
        (b >> 6 & 0x1F) as f32 * fy + p[1],
        (b >> 1 & 0x1F) as f32 * fz + p[2],
    ]
}

/// FLineBoxIntersection.
fn line_box(b: &Bx6, s: V, dir: V, inv: V) -> bool {
    let mut inside = true;
    let mut t = [0.0f32; 3];
    for i in 0..3 {
        if b[i] <= s[i] {
            if s[i] > b[i + 3] {
                if 0.0 <= dir[i] {
                    return false;
                }
                inside = false;
                t[i] = (b[i + 3] - s[i]) * inv[i];
            }
        } else {
            if dir[i] <= 0.0 {
                return false;
            }
            inside = false;
            t[i] = (b[i] - s[i]) * inv[i];
        }
    }
    if inside {
        return true;
    }
    let mut m = t[2];
    if m <= t[1] {
        m = t[1];
    }
    if m <= t[0] {
        m = t[0];
    }
    if !(0.0..=1.0).contains(&m) {
        return false;
    }
    let h = [0, 1, 2].map(|i| s[i] + dir[i] * m);
    (0..3).all(|i| b[i] - BOX_SIDE < h[i] && h[i] < b[i + 3] + BOX_SIDE)
}

/// FPlane(A, B, C), normal ((B − A) × (C − A)) normalized (zero otherwise),
/// W = (N.X·A.X + N.Z·A.Z) + N.Y·A.Y.
fn plane3(a: V, b: V, c: V) -> Plane {
    let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let mut n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    if !coords::normalize(&mut n) {
        n = [0.0; 3];
    }
    [n[0], n[1], n[2], (n[0] * a[0] + n[2] * a[2]) + n[1] * a[1]]
}

/// Signed distance to the plane: ((N.Y·P.Y + N.Z·P.Z) + N.X·P.X) − W.
fn dist(p: &Plane, v: V) -> f32 {
    ((p[1] * v[1] + p[2] * v[2]) + p[0] * v[0]) - p[3]
}

/// Projection of the half-extent onto the normal: (|N.Z·E.Z| + |N.Y·E.Y|) + |N.X·E.X|.
fn reach(p: &[f32], e: V) -> f32 {
    ((p[2] * e[2]).abs() + (p[1] * e[1]).abs()) + (p[0] * e[0]).abs()
}

/// Normalization ((X² + Y²) + Z², appRSqrt) if the norm is not zero.
pub(crate) fn normalize_nonzero(v: &mut V) {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if 0.0 < sq {
        let s = coords::rsqrt(sq);
        *v = [s * v[0], s * v[1], s * v[2]];
    }
}

impl KdopSet {
    pub(crate) fn new(verts: Vec<V>, tris: Vec<[u16; 4]>, nodes: Vec<CollisionNode>, tags: Vec<u32>) -> Self {
        static IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut set = KdopSet { verts, tris, nodes, tags, planes: Vec::new(), id };
        set.planes = set.tris.iter().map(|t| set.plane(t)).collect();
        set
    }

    /// Node, its triangle and the triangle's plane.
    #[inline]
    fn node(&self, i: u16) -> Option<(CollisionNode, [u16; 4], Plane)> {
        let n = *self.nodes.get(i as usize)?;
        let k = n.triangle as usize;
        Some((n, *self.tris.get(k)?, self.planes[k]))
    }

    fn green(&self, i: u16) -> V {
        self.verts.get(i as usize).copied().unwrap_or([0.0; 3])
    }

    /// Plane of the triangle in the mesh's frame: FPlane(V2, V1, V0).
    fn plane(&self, t: &[u16; 4]) -> Plane {
        plane3(self.green(t[2]), self.green(t[1]), self.green(t[0]))
    }

    /// Vertices of the triangle in the world (through LocalToWorld), V0 and V2 swapped if
    /// the frame is inverted, and their plane FPlane(V2, V1, V0) from before the swap.
    fn world_triangle(&self, t: &[u16; 4], f: &Frame) -> ([V; 3], Plane) {
        let mut w = [0, 1, 2].map(|k| transform_fvector(&f.l2w, self.green(t[k])));
        let p = plane3(w[2], w[1], w[0]);
        if f.mirrored {
            w.swap(0, 2);
        }
        (w, p)
    }

    /// `world_triangle` of triangle `k`, kept by the frame.
    #[inline]
    fn world_triangle_k(&self, k: usize, t: &[u16; 4], f: &Frame) -> ([V; 3], Plane) {
        if !crate::physics::coll::prior(crate::physics::coll::TRIANGLES) {
            // Key: number of the frame and number of the set (a frame can serve both its sets).
            let key_tuple = (f.number, self.id);
            let mut c = f.world_value.borrow_mut();
            if c.tris.len() < self.tris.len() {
                c.tris.resize(self.tris.len(), ((0, 0), [[0.0; 3]; 3], [0.0; 4]));
            }
            let e = &mut c.tris[k];
            if e.0 != key_tuple {
                let (w, p) = self.world_triangle(t, f);
                *e = (key_tuple, w, p);
            }
            return (e.1, e.2);
        }
        let mut c = f.world.borrow_mut();
        let (id, tris) = &mut *c;
        if *id != self.id || tris.len() != self.tris.len() {
            *id = self.id;
            tris.clear();
            tris.resize(self.tris.len(), None);
        }
        *tris[k].get_or_insert_with(|| self.world_triangle(t, f))
    }
}

fn child(n: &CollisionNode, k: usize) -> u16 {
    if k == 2 {
        n.back
    } else {
        n.front
    }
}

/// FBox of the three vertices (ties keep the bound).
fn tri_box(w: &[V; 3]) -> Bx6 {
    let mut b = [w[0][0], w[0][1], w[0][2], w[0][0], w[0][1], w[0][2]];
    for v in &w[1..] {
        for i in 0..3 {
            if v[i] < b[i] {
                b[i] = v[i];
            }
            if v[i] > b[i + 3] {
                b[i + 3] = v[i];
            }
        }
    }
    b
}

/// Planes of the six faces of a box, in the order.
fn box_planes(b: &Bx6) -> [Plane; 6] {
    [
        [-1.0, 0.0, 0.0, -b[0]],
        [1.0, 0.0, 0.0, b[3]],
        [0.0, -1.0, 0.0, -b[1]],
        [0.0, 1.0, 0.0, b[4]],
        [0.0, 0.0, -1.0, -b[2]],
        [0.0, 0.0, 1.0, b[5]],
    ]
}

/// Plane of the separating axis Axis × Edge normalized, turned towards
/// (Plane × Edge), through the vertex.
fn edge_plane(v: V, e: V, c: V, a: V) -> Plane {
    let mut n = [a[1] * e[2] - e[1] * a[2], e[0] * a[2] - a[0] * e[2], e[1] * a[0] - e[0] * a[1]];
    normalize_nonzero(&mut n);
    if (n[2] * c[2] + n[1] * c[1]) + n[0] * c[0] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    [n[0], n[1], n[2], (n[2] * v[2] + n[1] * v[1]) + n[0] * v[0]]
}

/// The separating axes of a triangle: box, plane, flipped
/// plane, then edge × axis for each edge and each axis (skipped if the edge is
/// parallel to it). `test` returns false as soon as an axis separates.
fn separating_axes(w: &[V; 3], p: &Plane, mut test: impl FnMut(&Plane) -> bool) -> bool {
    if !box_planes(&tri_box(w)).iter().all(&mut test) {
        return false;
    }
    if !test(p) || !test(&[-p[0], -p[1], -p[2], -p[3]]) {
        return false;
    }
    for i in 0..3 {
        let v = w[i];
        let o = w[(i + 1) % 3];
        let e = [o[0] - v[0], o[1] - v[1], o[2] - v[2]];
        let c = [p[1] * e[2] - p[2] * e[1], p[2] * e[0] - p[0] * e[2], p[0] * e[1] - p[1] * e[0]];
        if (e[1] != 0.0 || e[2] != 0.0) && !test(&edge_plane(v, e, c, [1.0, 0.0, 0.0])) {
            return false;
        }
        if (e[0] != 0.0 || e[2] != 0.0) && !test(&edge_plane(v, e, c, [0.0, 1.0, 0.0])) {
            return false;
        }
        if (e[0] != 0.0 || e[1] != 0.0) && !test(&edge_plane(v, e, c, [0.0, 0.0, 1.0])) {
            return false;
        }
    }
    true
}

/// Sweep of a box against a triangle, in the world.
struct Sweep {
    start: V,
    end: V,
    extent: V,
    normal: V,
    tmin: f32,
    tmax: f32,
    hit: bool,
}

impl Sweep {
    /// Time interval where the box is on the inner side of the plane.
    fn plane(&mut self, p: &Plane) -> bool {
        let r = reach(p, self.extent);
        let d0 = ((self.start[1] * p[1] + self.start[2] * p[2]) + p[0] * self.start[0]) - p[3];
        let d1 = ((self.end[1] * p[1] + self.end[2] * p[2]) + p[0] * self.end[0]) - p[3];
        if d0 > d1 {
            let t = (r - d0) / (d1 - d0);
            if t > self.tmin {
                self.tmin = t;
                self.normal = [p[0], p[1], p[2]];
                self.hit = true;
            }
        } else if d0 < d1 {
            let t = (r - d0) / (d1 - d0);
            if t < self.tmax {
                self.tmax = t;
            }
        } else if d0 > r && d1 > r {
            return false;
        }
        self.tmin < self.tmax && 0.0 < self.tmax
    }
}

/// Static overlap of a box and a triangle, in the world.
struct Overlap {
    location: V,
    extent: V,
    normal: V,
    best: f32,
}

impl Overlap {
    /// Penetration along the plane; the smallest is kept.
    fn plane(&mut self, p: &Plane) -> bool {
        let r = reach(p, self.extent);
        let depth = r - (((self.location[1] * p[1] + self.location[2] * p[2]) + p[0] * self.location[0]) - p[3]);
        if depth < self.best {
            self.normal = [p[0], p[1], p[2]];
            self.best = depth;
        }
        0.0 < depth
    }
}

/// Trace without extent, mesh frame.
struct Ray<'a> {
    set: &'a KdopSet,
    f: &'a Frame,
    start: V,
    end: V,
    dir: V,
    inv: V,
    hit: KHit,
}

impl Ray<'_> {
    fn node(&mut self, mut i: u16, parent: &Bx6) -> bool {
        let mut parent = *parent;
        let mut found = false;
        loop {
            if i == NONE {
                return found;
            }
            let Some((n, t, p)) = self.set.node(i) else { return found };
            let b = decode(n.bounds, &parent);
            if !line_box(&b, self.start, self.dir, self.inv) {
                return found;
            }
            let d0 = dist(&p, self.start);
            let d1 = dist(&p, self.end);
            if self.node(n.coplanar, &b) {
                found = true;
            }
            i = if d0 <= -PLANE_EPS || d1 <= -PLANE_EPS {
                if PLANE_EPS <= d0 || PLANE_EPS <= d1 {
                    let side = (d0 > -PLANE_EPS) as usize;
                    if self.node(child(&n, side + 2), &b) {
                        return true;
                    }
                    let time = -(d0 / (d1 - d0));
                    if time < self.hit.time && self.inside(&t, time) {
                        self.hit.normal = coords::transform_normal(&self.f.l2w, [p[0], p[1], p[2]]);
                        self.hit.time = time;
                        self.hit.triangle = n.triangle as usize;
                        return true;
                    }
                    child(&n, 3 - side)
                } else {
                    n.back
                }
            } else {
                n.front
            };
            parent = b;
        }
    }

    /// The ray's point at time `time` is inside the triangle: (V1 − V0) × (P − V0),
    /// (V2 − V1) × (P − V1) and (V0 − V2) × (P − V2) point the same way.
    fn inside(&self, t: &[u16; 4], time: f32) -> bool {
        let (v0, v1, v2) = (self.set.green(t[0]), self.set.green(t[1]), self.set.green(t[2]));
        let p = [time * self.dir[0] + self.start[0], time * self.dir[1] + self.start[1], time * self.dir[2] + self.start[2]];
        let cross = |a: V, b: V| -> V {
            let (e, d) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [p[0] - a[0], p[1] - a[1], p[2] - a[2]]);
            [e[1] * d[2] - e[2] * d[1], e[2] * d[0] - d[2] * e[0], d[1] * e[0] - e[1] * d[0]]
        };
        let c1 = cross(v0, v1);
        let c2 = cross(v1, v2);
        let dot = (c2[2] * c1[2] + c2[1] * c1[1]) + c2[0] * c1[0];
        if dot.is_nan() || dot < 0.0 {
            return false;
        }
        let c3 = cross(v2, v0);
        (c2[2] * c3[2] + c3[1] * c2[1]) + c3[0] * c2[0] >= 0.0
    }
}

/// Trace with extent: traversal in the mesh's frame,
/// triangles swept in the world.
struct BoxTrace<'a> {
    set: &'a KdopSet,
    f: &'a Frame,
    start: V,
    end: V,
    extent: V,
    local_start: V,
    local_end: V,
    dir: V,
    inv: V,
    half: V,
    visited: &'a mut [u32],
    stamp: u32,
    hit: KHit,
}

impl BoxTrace<'_> {
    fn node(&mut self, mut i: u16, parent: &Bx6) -> bool {
        let mut parent = *parent;
        let mut found = false;
        if i == NONE {
            return false;
        }
        loop {
            let Some((n, t, p)) = self.set.node(i) else { return found };
            let b = decode(n.bounds, &parent);
            let h = self.half;
            let grown = [b[0] - h[0], b[1] - h[1], b[2] - h[2], b[3] + h[0], b[4] + h[1], b[5] + h[2]];
            if !line_box(&grown, self.local_start, self.dir, self.inv) {
                return found;
            }
            let d0 = dist(&p, self.local_start);
            let d1 = dist(&p, self.local_end);
            let r = reach(&p, h);
            if self.node(n.coplanar, &b) {
                found = true;
            }
            let nr = -r;
            i = if d0 > nr || d1 > nr {
                if d0 < r || d1 < r {
                    let side = if d0 < d1 { 0 } else { 1 };
                    if self.node(child(&n, side + 2), &b) {
                        found = true;
                        if (side == 0 && d1 < nr) || (side == 1 && r < d1) {
                            return true;
                        }
                    }
                    let k = n.triangle as usize;
                    if self.visited.get(k).is_some_and(|&v| v != self.stamp) {
                        let (w, wp) = self.set.world_triangle_k(n.triangle as usize, &t, self.f);
                        let mut s = Sweep { start: self.start, end: self.end, extent: self.extent, normal: [0.0; 3], tmin: -1.0, tmax: self.hit.time, hit: false };
                        if separating_axes(&w, &wp, |q| s.plane(q)) && s.hit && s.tmin < self.hit.time {
                            self.hit.normal = s.normal;
                            self.hit.time = s.tmin;
                            self.hit.triangle = k;
                            found = true;
                        }
                        self.visited[k] = self.stamp;
                    }
                    // Continue on the other side: the kept contact is not entirely there.
                    let (mut go_back, mut go_front) = (true, true);
                    if found {
                        let time = self.hit.time;
                        let q = [0, 1, 2].map(|a| time * self.dir[a] + self.local_start[a]);
                        let dh = dist(&p, q);
                        go_back = d0 <= r || dh <= r;
                        go_front = !(d0 < nr && dh < nr);
                    }
                    if !(if side == 0 { go_front } else { go_back }) {
                        return found;
                    }
                    child(&n, 3 - side)
                } else {
                    n.front
                }
            } else {
                n.back
            };
            if i == NONE {
                return found;
            }
            parent = b;
        }
    }
}

/// PointCheck: traversal in the mesh's frame, triangles
/// tested in the world.
struct PointTrace<'a> {
    set: &'a KdopSet,
    f: &'a Frame,
    location: V,
    extent: V,
    local: V,
    half: V,
    best: f32,
    hit: KHit,
}

impl PointTrace<'_> {
    fn node(&mut self, mut i: u16, parent: &Bx6) -> bool {
        let mut parent = *parent;
        let mut found = false;
        loop {
            if i == NONE {
                return found;
            }
            let Some((n, t, p)) = self.set.node(i) else { return found };
            let b = decode(n.bounds, &parent);
            let (l, h) = (self.local, self.half);
            if b[0] - h[0] > l[0] || l[0] > b[3] + h[0] || l[1] < b[1] - h[1] || l[1] > b[4] + h[1] || l[2] < b[2] - h[2] || l[2] > b[5] + h[2] {
                return found;
            }
            let d = dist(&p, l);
            let r = reach(&p, h);
            if self.node(n.coplanar, &b) {
                found = true;
            }
            i = if -r < d {
                if d < r {
                    let side = (d > 0.0) as usize;
                    if self.node(child(&n, side + 2), &b) {
                        found = true;
                    }
                    let (w, wp) = self.set.world_triangle_k(n.triangle as usize, &t, self.f);
                    let mut o = Overlap { location: self.location, extent: self.extent, normal: self.hit.normal, best: self.best };
                    if separating_axes(&w, &wp, |q| o.plane(q)) && o.best < self.best {
                        self.hit.location = [0, 1, 2].map(|a| o.normal[a] * o.best + self.location[a]);
                        self.hit.normal = o.normal;
                        self.hit.triangle = n.triangle as usize;
                        self.best = o.best;
                        found = true;
                    }
                    child(&n, 3 - side)
                } else {
                    n.front
                }
            } else {
                n.back
            };
            parent = b;
        }
    }
}

/// Local half-extent: FBox(−E, E).TransformBy(WorldToLocal), (Max − Min) / 2.
fn local_half_extent(f: &Frame, e: V) -> V {
    let (lo, hi) = transform_box(&([-e[0], -e[1], -e[2]], e), &f.w2l);
    [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5]
}

/// LineCheck traversal from the root (`root`: the mesh's BoundingBox). Hit.Time starts at
/// 1; the returned contact is the engine's before the pull-back and the orientation of the normal.
pub(crate) fn line_check(set: &KdopSet, root: &Bx6, f: &Frame, end: V, start: V, extent: V) -> Option<KHit> {
    let hit = KHit { location: [0.0; 3], normal: [0.0; 3], time: 1.0, triangle: 0 };
    let ls = transform_fvector(&f.w2l, start);
    let le = transform_fvector(&f.w2l, end);
    let dir = [le[0] - ls[0], le[1] - ls[1], le[2] - ls[2]];
    let inv = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
    if extent[0] == 0.0 && extent[1] == 0.0 && extent[2] == 0.0 {
        let mut ray = Ray { set, f, start: ls, end: le, dir, inv, hit };
        return ray.node(0, root).then_some(ray.hit);
    }
    VISITED.with(|v| {
        let (visited, last) = &mut *v.borrow_mut();
        *last = last.wrapping_add(1);
        if *last == 0 {
            // Numbers exhausted: array reset to zero, number 1.
            visited.iter_mut().for_each(|x| *x = 0);
            *last = 1;
        }
        if visited.len() < set.tris.len() {
            visited.resize(set.tris.len(), 0);
        }
        let mut tr = BoxTrace { set, f, start, end, extent, local_start: ls, local_end: le, dir, inv, half: local_half_extent(f, extent), visited: &mut visited[..set.tris.len()], stamp: *last, hit };
        tr.node(0, root).then_some(tr.hit)
    })
}

/// PointCheck traversal from the root: Hit.Location, Hit.Normal (not normalized).
pub(crate) fn point_check(set: &KdopSet, root: &Bx6, f: &Frame, location: V, extent: V) -> Option<KHit> {
    let hit = KHit { location: [0.0; 3], normal: [0.0; 3], time: 1.0, triangle: 0 };
    let mut pt = PointTrace { set, f, location, extent, local: transform_fvector(&f.w2l, location), half: local_half_extent(f, extent), best: 100_000.0, hit };
    pt.node(0, root).then_some(pt.hit)
}

/// Mods build only: a collision tree for a mod's own 3D model (the game's meshes carry theirs).
/// A BSP on the triangles' planes, as the traversal above expects: each node splits by its
/// triangle's plane (front: positive side), triangles in that plane go in its coplanar chain,
/// and a triangle crossing the plane is cut in two along it, so that every subtree lies entirely
/// on its side (the traversal stops at the first contact on the near side). Node boxes are their
/// parent's whole box (no pruning by box). Returns the collision vertices and triangles (cut
/// ones included, with their material) and the nodes; None if the tree would need more than
/// 65535 vertices, triangles or nodes, or 1000 levels. A coplanar chain holds at most 64 triangles,
/// the others of that plane go to the front side (the traversal recurses once per chain node, so
/// a large flat surface must not make one long chain).
#[cfg(feature = "mods")]
pub(crate) fn build_tree(verts: &[V], tris: &[[u16; 4]]) -> Option<(Vec<V>, Vec<[u16; 4]>, Vec<CollisionNode>)> {
    const WHOLE: u32 = 31 << 11 | 31 << 6 | 31 << 1;
    const EPS: f32 = 0.01;
    struct B {
        verts: Vec<V>,
        tris: Vec<[u16; 4]>,
        planes: Vec<Plane>,
        nodes: Vec<CollisionNode>,
    }
    impl B {
        fn vertex(&mut self, v: V) -> Option<u16> {
            if self.verts.len() >= NONE as usize {
                return None;
            }
            self.verts.push(v);
            Some((self.verts.len() - 1) as u16)
        }
        /// A new triangle (fan of a cut polygon) with material `m`; None if it has no area.
        fn triangle(&mut self, a: u16, b: u16, c: u16, m: u16) -> Option<Option<usize>> {
            let at = |i: u16| self.verts[i as usize];
            let p = plane3(at(c), at(b), at(a));
            if p[..3].iter().all(|&x| x == 0.0) {
                return Some(None);
            }
            if self.tris.len() >= NONE as usize {
                return None;
            }
            self.tris.push([a, b, c, m]);
            self.planes.push(p);
            Some(Some(self.tris.len() - 1))
        }
        /// Triangle `k` cut by plane `p` into its front and back pieces.
        fn cut(&mut self, k: usize, p: &Plane, front: &mut Vec<usize>, back: &mut Vec<usize>) -> Option<()> {
            let t = self.tris[k];
            let d: Vec<f32> = t[..3].iter().map(|&i| dist(p, self.verts[i as usize])).collect();
            let (mut f, mut b) = (Vec::new(), Vec::new());
            for e in 0..3 {
                let (i, j) = (t[e], t[(e + 1) % 3]);
                let (di, dj) = (d[e], d[(e + 1) % 3]);
                if di >= -EPS {
                    f.push(i);
                }
                if di <= EPS {
                    b.push(i);
                }
                if (di > EPS && dj < -EPS) || (di < -EPS && dj > EPS) {
                    let s = di / (di - dj);
                    let (vi, vj) = (self.verts[i as usize], self.verts[j as usize]);
                    let x = self.vertex([0, 1, 2].map(|a| vi[a] + (vj[a] - vi[a]) * s))?;
                    f.push(x);
                    b.push(x);
                }
            }
            for (poly, out) in [(f, front), (b, back)] {
                for n in 1..poly.len().saturating_sub(1) {
                    if let Some(new) = self.triangle(poly[0], poly[n], poly[n + 1], t[3])? {
                        out.push(new);
                    }
                }
            }
            Some(())
        }
        fn build(&mut self, list: Vec<usize>, depth: usize) -> Option<u16> {
            let Some((&first, rest)) = list.split_first() else { return Some(NONE) };
            if depth > 1000 {
                return None;
            }
            let p = self.planes[first];
            let (mut coplanar, mut back, mut front) = (Vec::new(), Vec::new(), Vec::new());
            for &k in rest {
                let d: Vec<f32> = self.tris[k][..3].iter().map(|&i| dist(&p, self.verts[i as usize])).collect();
                let (any_front, any_back) = (d.iter().any(|&x| x > EPS), d.iter().any(|&x| x < -EPS));
                match (any_front, any_back) {
                    (false, false) if coplanar.len() < 64 => coplanar.push(k),
                    (false, false) => front.push(k),
                    (true, false) => front.push(k),
                    (false, true) => back.push(k),
                    (true, true) => self.cut(k, &p, &mut front, &mut back)?,
                }
            }
            let index = self.nodes.len();
            if index + 1 + coplanar.len() >= NONE as usize {
                return None;
            }
            self.nodes.push(CollisionNode { triangle: first as u16, coplanar: NONE, back: NONE, front: NONE, bounds: WHOLE });
            // Coplanar chain: one node per triangle of the plane, linked by `coplanar`.
            let mut last = index;
            for k in coplanar {
                let c = self.nodes.len();
                self.nodes.push(CollisionNode { triangle: k as u16, coplanar: NONE, back: NONE, front: NONE, bounds: WHOLE });
                self.nodes[last].coplanar = c as u16;
                last = c;
            }
            let b = self.build(back, depth + 1)?;
            let f = self.build(front, depth + 1)?;
            self.nodes[index].back = b;
            self.nodes[index].front = f;
            Some(index as u16)
        }
    }
    let mut b = B { verts: verts.to_vec(), tris: Vec::new(), planes: Vec::new(), nodes: Vec::new() };
    let mut list = Vec::new();
    for t in tris {
        // Triangles without area have no plane: left out.
        if let Some(k) = b.triangle(t[0], t[1], t[2], t[3])? {
            list.push(k);
        }
    }
    if list.is_empty() {
        return None;
    }
    b.build(list, 0)?;
    Some((b.verts, b.tris, b.nodes))
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A single triangle: the root is its box, the node takes it whole.
    fn one(t: [V; 3]) -> (KdopSet, Bx6) {
        let set = KdopSet::new(t.to_vec(), vec![[0, 1, 2, 0]], vec![CollisionNode { triangle: 0, coplanar: NONE, back: NONE, front: NONE, bounds: 31 << 11 | 31 << 6 | 31 << 1 }], vec![0]);
        let b = tri_box(&t);
        (set, b)
    }

    fn unit() -> Frame {
        frame([0; 3], [1.0; 3], [0.0; 3], [0.0; 3], 1.0)
    }

    /// Flat floor of Toits01 under PositionInfo18: a vertical trace without extent hits it
    /// whatever its length.
    #[test]
    fn trace_vertical_triangle_flat() {
        let t = [[7810.999, 141.00024, 1335.9966], [7718.4287, 224.0481, 1335.9966], [7810.999, 3629.0002, 1335.9966]];
        let (set, root) = one(t);
        let f = unit();
        for end in [398.26f32, 0.0, -1000.0, -3000.0] {
            let (s, e) = ([7785.66f32, 1613.68, 1398.26], [7785.66f32, 1613.68, end]);
            let hit = line_check(&set, &root, &f, e, s, [0.0; 3]).map(|h| s[2] + (end - s[2]) * h.time);
            assert!(hit.is_some_and(|z| (z - 1335.9966).abs() < 0.01), "end {end}: {hit:?}");
        }
    }

    #[test]
    fn box_placed_on_floor() {
        let t = [[-100.0, -100.0, 0.0], [100.0, -100.0, 0.0], [0.0, 100.0, 0.0]];
        let (set, root) = one(t);
        let f = unit();
        let h = line_check(&set, &root, &f, [0.0, 0.0, -50.0], [0.0, 0.0, 50.0], [10.0, 10.0, 20.0]).unwrap();
        assert!((50.0 - 100.0 * h.time - 20.0).abs() < 1e-3, "{h:?}");
        assert!(h.normal[2].abs() > 0.999);
        assert!(line_check(&set, &root, &f, [500.0, 0.0, -50.0], [500.0, 0.0, 50.0], [10.0, 10.0, 20.0]).is_none());
        let p = point_check(&set, &root, &f, [0.0, 0.0, 10.0], [10.0, 10.0, 20.0]).unwrap();
        assert!(p.normal[2].abs() > 0.999, "{p:?}");
        assert!(point_check(&set, &root, &f, [0.0, 0.0, 30.0], [10.0, 10.0, 20.0]).is_none());
    }
}

