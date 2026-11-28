//! UStaticMesh::Build: rebuilds sections, vertices, UVs, indices and
//! collision (full and simplified) from the RawTriangles, for meshes from a file with
//! version +0x1BC < 13 (called by UStaticMesh::PostLoad). The logs (GLog) and the
//! "package modified" flag are not reproduced.
//!
//! Helpers: nearby vertex (tolerance 0.008), nearby normal (8e-5),
//! find or add a vertex, simplified collision vertex, edge,
//! class of a triangle against a plane, tree, node box,
//! quantisation.

use crate::coords;
use crate::static_collision::{CollisionNode, CollisionSet, RawArrays, StaticCollision};
use crate::static_mesh::{Section, StaticMesh};
use crate::stripify::cache_optimize;

const POS_TOL: f32 = f32::from_bits(0x3c03126f); // 0.008
const NORMAL_TOL: f32 = f32::from_bits(0x38a7c5ac); // 8e-5
const UV_TOL: f32 = f32::from_bits(0x3a800000); // 1 / 1024
const PLANE_EPS: f32 = f32::from_bits(0x38d1b717); // 1e-4
const INV31: f32 = f32::from_bits(0x3d042108);
const ROUND_UP: f32 = f32::from_bits(0x3f7ff972); // 0.9999

/// FStaticMeshTriangle (0x104 bytes): vertices, UVs of the eight sets of each vertex, number of
/// sets, material, smoothing mask. Colours are not used by the rebuild.
#[derive(Clone, Debug, Default)]
pub struct RawTriangle {
    pub v: [[f32; 3]; 3],
    pub uv: [[[f32; 2]; 8]; 3],
    pub nuv: i32,
    pub mat: i32,
    pub smooth: u32,
}

impl RawTriangle {
    /// In-memory layout of the object (UStaticMesh+0x198): vertices +0, UVs +0x24 + 0x40 × vertex + 8 × set,
    /// material +0xF0, smoothing +0xF4, sets +0xF8.
    pub fn from_image(b: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let i = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let mut t = RawTriangle::default();
        for k in 0..3 {
            t.v[k] = [f(12 * k), f(12 * k + 4), f(12 * k + 8)];
            for s in 0..8 {
                t.uv[k][s] = [f(0x24 + 0x40 * k + 8 * s), f(0x28 + 0x40 * k + 8 * s)];
            }
        }
        t.mat = i(0xF0);
        t.smooth = i(0xF4) as u32;
        t.nuv = i(0xF8);
        t
    }
}

/// What Build reads from a material: UMaterial::RequiresSorting (vtable +0x78) and, for a
/// USinusModifier, its byte at +0x90.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaterialProps {
    pub requires_sorting: bool,
    pub sine_kind: Option<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct BuildInput {
    pub raw: Vec<RawTriangle>,
    pub simple: Vec<RawTriangle>,
    /// (reference, flags +4) of each Materials entry; bit 0 enables collision.
    pub materials: Vec<(i32, u32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Built {
    pub sections: Vec<Section>,
    pub vertices: Vec<[f32; 6]>,
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub indices: Vec<u16>,
    /// Edge indices (UStaticMesh+0xE4, +0x168).
    pub wire: Vec<u16>,
    pub wire_simple: Vec<u16>,
    pub full: CollisionSet,
    pub simple: CollisionSet,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub bounds_valid: bool,
    pub sphere: [f32; 4],
    pub materials: Vec<(i32, u32)>,
    /// UStaticMesh+0x17C, +0x180.
    pub scale_bias: [f32; 2],
    /// Bits 3 (sorting), 4 (USinusModifier) and 5 of UStaticMesh+0x188.
    pub flags: u32,
    /// UStaticMesh+0x184.
    pub raw_count: u16,
}

type V3 = [f32; 3];

/// Each difference is at most `tol` (an undefined difference passes).
fn near(a: &[f32], b: &[f32], tol: f32) -> bool {
    (0..3).all(|i| !((a[i] - b[i]).abs() > tol))
}

/// FBox += FVector: the bounds only move if the point exceeds them.
struct FBox {
    min: V3,
    max: V3,
    valid: bool,
}

impl FBox {
    fn empty() -> Self {
        FBox { min: [0.0; 3], max: [0.0; 3], valid: false }
    }

    fn add(&mut self, p: &[f32]) {
        if !self.valid {
            self.min = [p[0], p[1], p[2]];
            self.max = [p[0], p[1], p[2]];
            self.valid = true;
            return;
        }
        for i in 0..3 {
            if !(self.min[i] <= p[i]) {
                self.min[i] = p[i];
            }
            if !(self.max[i] >= p[i]) {
                self.max[i] = p[i];
            }
        }
    }

    /// FBox += FBox.
    fn merge(&mut self, o: &FBox) {
        if !self.valid || !o.valid {
            self.min = o.min;
            self.max = o.max;
            self.valid = o.valid;
            return;
        }
        for i in 0..3 {
            if !(self.min[i] <= o.min[i]) {
                self.min[i] = o.min[i];
            }
            if !(self.max[i] >= o.max[i]) {
                self.max[i] = o.max[i];
            }
        }
    }
}

/// FPlane(A, B, C): normalised normal of (B − A) ^ (C − A), zero otherwise; W = N · A.
fn plane3(a: V3, b: V3, c: V3) -> [f32; 4] {
    let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let mut n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    if !coords::normalize(&mut n) {
        n = [0.0; 3];
    }
    [n[0], n[1], n[2], (n[0] * a[0] + n[2] * a[2]) + n[1] * a[1]]
}

fn nonzero(n: &[f32]) -> bool {
    !(n[0] == 0.0 && n[1] == 0.0 && n[2] == 0.0)
}

/// Sides of the three points relative to the plane (1 front, 2 back, 3 straddling, 0 on the plane).
fn classify(p: &[f32; 4], pts: &[V3; 3]) -> u8 {
    let mut c = 0u8;
    for q in pts {
        let d = ((p[1] * q[1] + p[2] * q[2]) + p[0] * q[0]) - p[3];
        if d < -PLANE_EPS {
            c |= 2;
        } else if d >= PLANE_EPS {
            c |= 1;
        }
    }
    c
}

/// _ftol: truncation.
fn ftol(x: f32) -> i32 {
    (x as i64) as i32
}

struct Tree<'a> {
    verts: &'a [V3],
    tris: &'a [[u16; 4]],
    table: &'a [u8],
    nodes: Vec<CollisionNode>,
    boxes: Vec<FBox>,
}

impl Tree<'_> {
    /// Node of the best splitter, then back, front and coplanar.
    fn build(&mut self, list: &[usize]) -> u16 {
        if list.is_empty() {
            return 0xFFFF;
        }
        let n = self.tris.len();
        let (mut best, mut best_score, mut best_front, mut best_back) = (list[0], i32::MAX, 0i32, 0i32);
        for &cand in list {
            let (mut front, mut back, mut span) = (0i32, 0i32, 0i32);
            for &t in list {
                match self.table[cand * n + t] {
                    1 => front += 1,
                    2 => back += 1,
                    3 => span += 1,
                    _ => {}
                }
            }
            if span < best_score || (span == best_score && (front - back).abs() < (best_front - best_back).abs()) {
                best = cand;
                best_score = span;
                best_front = front;
                best_back = back;
            }
        }
        let index = self.nodes.len();
        self.boxes.push(FBox::empty());
        self.nodes.push(CollisionNode { triangle: best as u16, ..Default::default() });
        let (mut front, mut back, mut cop) = (Vec::new(), Vec::new(), Vec::new());
        for &t in list {
            if t == best {
                continue;
            }
            match self.table[best * n + t] {
                0 | 3 => cop.push(t),
                1 => front.push(t),
                2 => back.push(t),
                _ => {}
            }
        }
        let b = self.build(&back);
        let f = self.build(&front);
        let c = self.build(&cop);
        self.nodes[index].coplanar = c;
        self.nodes[index].back = b;
        self.nodes[index].front = f;
        let mut bx = FBox::empty();
        for ch in [c, b, f] {
            if ch != 0xFFFF {
                bx.merge(&self.boxes[ch as usize]);
            }
        }
        let t = self.tris[best];
        for k in 0..3 {
            bx.add(&self.verts[t[k] as usize]);
        }
        self.boxes[index] = bx;
        index as u16
    }

    /// Node box quantised (5 bits per bound) within its parent's.
    fn quantize(&mut self, node: usize, parent: &([f32; 3], [f32; 3])) {
        if node >= self.nodes.len() {
            return;
        }
        let nb = &self.boxes[node];
        let (nmin, nmax) = (nb.min, nb.max);
        let n = self.nodes[node];
        let (pmin, pmax) = (parent.0, parent.1);
        let mut bounds = 0u32;
        let mut child = ([0.0f32; 3], [0.0f32; 3]);
        // (offset of the minimum, offset of the maximum) per axis
        const SHIFT: [(u32, u32); 3] = [(27, 11), (22, 6), (17, 1)];
        for a in 0..3 {
            let ext = pmax[a] - pmin[a];
            let q = |v: f32, up: bool| -> i32 {
                if !(ext > 0.0) {
                    return 0;
                }
                let mut t = (v - pmin[a]) / ext * 31.0;
                if up {
                    t += ROUND_UP;
                }
                ftol(t).clamp(0, 31)
            };
            let (qmin, qmax) = (q(nmin[a], false), q(nmax[a], true));
            child.0[a] = (qmin as f32 * INV31) * ext + pmin[a];
            child.1[a] = (qmax as f32 * INV31) * ext + pmin[a];
            bounds |= (qmin as u32) << SHIFT[a].0;
            bounds |= (qmax as u32) << SHIFT[a].1;
        }
        self.nodes[node].bounds = bounds;
        for c in [n.front, n.coplanar, n.back] {
            if c != 0xFFFF {
                self.quantize(c as usize, &child);
            }
        }
    }
}

/// Full or simplified tree: class table (plane triangle × triangle), list of all
/// triangles, then quantisation of the nodes within the mesh box.
fn collision_tree(verts: &[V3], tris: &[[u16; 4]], mesh_box: &([f32; 3], [f32; 3])) -> Vec<CollisionNode> {
    let n = tris.len();
    let mut table = vec![0u8; n * n];
    for i in 0..n {
        let t = tris[i];
        let p = plane3(verts[t[2] as usize], verts[t[1] as usize], verts[t[0] as usize]);
        for j in 0..n {
            let u = tris[j];
            table[i * n + j] = classify(&p, &[verts[u[0] as usize], verts[u[1] as usize], verts[u[2] as usize]]);
        }
    }
    let list: Vec<usize> = (0..n).collect();
    let mut tree = Tree { verts, tris, table: &table, nodes: Vec::new(), boxes: Vec::new() };
    tree.build(&list);
    if !tree.nodes.is_empty() {
        tree.quantize(0, mesh_box);
    }
    tree.nodes
}

/// Edge list: an edge whose reverse already exists is not added; returns the
/// endpoints of each edge.
fn edge_indices(idx: &[u16]) -> Vec<u16> {
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for t in 0..idx.len() / 3 {
        for k in 0..3 {
            let (a, b) = (idx[3 * t + k], idx[3 * t + (k + 1) % 3]);
            if !edges.iter().any(|&(x, y)| x == b && y == a) {
                edges.push((a, b));
            }
        }
    }
    edges.into_iter().flat_map(|(a, b)| [a, b]).collect()
}

/// FSphere(2, &box): the sphere of the box's two corners (coords::sphere_from_points).
fn bounding_sphere(min: V3, max: V3) -> [f32; 4] {
    crate::coords::sphere_from_points(&[min, max])
}

pub fn build(input: &BuildInput, props: &dyn Fn(i32) -> MaterialProps) -> Built {
    let mut raw = input.raw.clone();
    let mut simple = input.simple.clone();
    let mut materials = input.materials.clone();
    let mut out = Built::default();

    // Identical materials (same object, same collision bit) merged, triangles renumbered.
    let mut i = 0;
    while i < materials.len() {
        let mut j = i + 1;
        while j < materials.len() {
            if materials[i].0 == materials[j].0 && (materials[i].1 ^ materials[j].1) & 1 == 0 {
                materials.remove(j);
                for t in raw.iter_mut().chain(simple.iter_mut()) {
                    if t.mat == j as i32 {
                        t.mat = i as i32;
                    } else if t.mat > j as i32 {
                        t.mat -= 1;
                    }
                }
            } else {
                j += 1;
            }
        }
        i += 1;
    }

    let m = materials.len();
    let normals_of = |tris: &[RawTriangle]| -> Vec<V3> { tris.iter().map(|t| { let p = plane3(t.v[2], t.v[1], t.v[0]); [p[0], p[1], p[2]] }).collect() };
    let planes = normals_of(&raw);
    let planes2 = normals_of(&simple);

    out.sections = vec![Section { is_strip: 0, first_index: 0, min_vertex: 0xFFFF, max_vertex: 0xFFFF, triangles: 0, primitives: 0 }; m];
    let mut lists: Vec<Vec<u16>> = vec![Vec::new(); m];
    let nstreams = raw.iter().map(|t| t.nuv.max(0) as usize).max().unwrap_or(0);
    out.uvs = vec![Vec::new(); nstreams];
    out.raw_count = raw.len() as u16;

    for bi in 0..raw.len() {
        let t = &raw[bi];
        let mut normals = [[0.0f32; 3]; 3];
        for (j, o) in raw.iter().enumerate() {
            if t.smooth & o.smooth != 0 || bi == j {
                for k in 0..3 {
                    for i in 0..3 {
                        if near(&t.v[k], &o.v[i], POS_TOL) {
                            for a in 0..3 {
                                normals[k][a] = planes[j][a] + normals[k][a];
                            }
                            break;
                        }
                    }
                }
            }
        }
        for n in normals.iter_mut() {
            coords::normalize(n);
        }
        let nuv = t.nuv.max(0) as usize;
        let mut idx = [0usize; 3];
        for k in 0..3 {
            idx[k] = add_vertex(&mut out, t.v[k], normals[k], &t.uv[k], nuv);
        }
        if idx[0] != idx[1] && idx[1] != idx[2] && idx[0] != idx[2] {
            let Some(list) = lists.get_mut(t.mat as usize) else { continue };
            list.extend(idx.map(|v| v as u16));
            let s = &mut out.sections[t.mat as usize];
            s.triangles = s.triangles.wrapping_add(1);
            s.primitives = s.primitives.wrapping_add(1);
            if materials[t.mat as usize].1 & 1 != 0 && nonzero(&planes[bi]) {
                out.full.triangles.push([idx[0] as u16, idx[1] as u16, idx[2] as u16, t.mat as u16]);
            }
        }
    }

    // Second list: simplified collision (vertices within 0.008, no normals or UVs).
    let mut simple_list: Vec<u16> = Vec::new();
    for bi in 0..simple.len() {
        simple[bi].mat = -1;
        let t = &simple[bi];
        let mut idx = [0usize; 3];
        for k in 0..3 {
            idx[k] = match out.simple.vertices.iter().position(|v| near(v, &t.v[k], POS_TOL)) {
                Some(i) => i,
                None => {
                    out.simple.vertices.push(t.v[k]);
                    out.simple.vertices.len() - 1
                }
            };
        }
        if idx[0] != idx[1] && idx[1] != idx[2] && idx[0] != idx[2] {
            simple_list.extend(idx.map(|v| v as u16));
            if nonzero(&planes2[bi]) {
                out.simple.triangles.push([idx[0] as u16, idx[1] as u16, idx[2] as u16, 0xFFFF]);
            }
        }
    }

    out.full.vertices = out.vertices.iter().map(|v| [v[0], v[1], v[2]]).collect();

    // Indices: each section reordered by strip, vertex bounds.
    for (i, list) in lists.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let list = cache_optimize(list);
        let s = &mut out.sections[i];
        s.primitives = (list.len() / 3) as u16;
        s.first_index = out.indices.len() as u16;
        s.min_vertex = list[0];
        s.max_vertex = list[0];
        for &v in &list {
            s.min_vertex = s.min_vertex.min(v);
            s.max_vertex = s.max_vertex.max(v);
        }
        out.indices.extend(list);
    }
    out.wire = edge_indices(&out.indices);
    out.wire_simple = edge_indices(&simple_list);

    // Box and sphere.
    let mut bx = FBox::empty();
    for v in &out.vertices {
        bx.add(&v[0..3]);
    }
    for v in &out.simple.vertices {
        bx.add(v);
    }
    out.sphere = bounding_sphere(bx.min, bx.max);
    out.bounds_min = bx.min;
    out.bounds_max = bx.max;
    out.bounds_valid = bx.valid;

    let mesh_box = (bx.min, bx.max);
    out.full.nodes = collision_tree(&out.full.vertices, &out.full.triangles, &mesh_box);
    out.simple.nodes = collision_tree(&out.simple.vertices, &out.simple.triangles, &mesh_box);

    // Sorting, USinusModifier, scale and offset from the materials.
    let mut flags = 0u32;
    let mut sb = [0.0f32; 2];
    for (i, mat) in materials.iter().enumerate() {
        if mat.0 == 0 {
            continue;
        }
        let p = props(mat.0);
        if p.requires_sorting {
            flags |= 8;
        }
        let Some(kind) = p.sine_kind else { continue };
        flags |= 0x10;
        if raw.is_empty() || kind == 0 {
            continue;
        }
        let (mut mn, mut mx) = (1_000_000.0f32, -1_000_000.0f32);
        for t in raw.iter().filter(|t| t.mat == i as i32) {
            for k in 0..3 {
                let x = t.v[k][0];
                if x < mn {
                    mn = x;
                }
                if x > mx {
                    mx = x;
                }
            }
        }
        match kind {
            1 => sb = [mx, 1.0 / (mx - mn)],
            2 => sb = [mn, 1.0 / (mx - mn)],
            3 => sb = [(mx + mn) * 0.5, 2.0 / (mx - mn)],
            _ => {}
        }
    }
    if flags & 0x18 != 0 {
        flags |= 0x20;
    }
    out.flags = flags;
    out.scale_bias = sb;
    out.materials = materials;
    out
}

/// Existing vertex (close position, normal and UV) or a new vertex.
fn add_vertex(out: &mut Built, pos: V3, normal: V3, uv: &[[f32; 2]; 8], nuv: usize) -> usize {
    'cand: for i in 0..out.vertices.len() {
        let v = out.vertices[i];
        if !near(&v[0..3], &pos, POS_TOL) || !near(&v[3..6], &normal, NORMAL_TOL) {
            continue;
        }
        for c in 0..nuv {
            let s = out.uvs[c][i];
            if (s[0] - uv[c][0]).abs() > UV_TOL || (s[1] - uv[c][1]).abs() > UV_TOL {
                continue 'cand;
            }
        }
        return i;
    }
    out.vertices.push([pos[0], pos[1], pos[2], normal[0], normal[1], normal[2]]);
    for c in 0..out.uvs.len() {
        out.uvs[c].push(if c < nuv { uv[c] } else { [0.0, 0.0] });
    }
    out.vertices.len() - 1
}

/// Build on the RawTriangles read from the file (sorting and USinusModifiers do not change what is kept).
pub fn rebuild(raw: &RawArrays) -> Built {
    build(&BuildInput { raw: raw.raw.clone(), simple: raw.simple.clone(), materials: raw.materials.clone() }, &|_| MaterialProps::default())
}

/// PostLoad's collision instead of the file's.
pub fn apply_collision(mut c: StaticCollision, raw: &RawArrays) -> StaticCollision {
    let b = rebuild(raw);
    c.full = b.full;
    c.simple = b.simple;
    c.raw_triangles = b.raw_count as i16;
    c.native_materials = b.materials.iter().map(|&(r, f)| (r, (f & 3) as u8)).collect();
    c
}

/// PostLoad's mesh instead of the file's.
pub fn apply_mesh(m: &mut StaticMesh, raw: &RawArrays) {
    let b = rebuild(raw);
    m.sections = b.sections;
    m.positions = b.vertices.iter().map(|v| [v[0], v[1], v[2]]).collect();
    m.normals = b.vertices.iter().map(|v| [v[3], v[4], v[5]]).collect();
    m.uvs = b.uvs;
    m.indices = b.indices;
    m.bounds_min = b.bounds_min;
    m.bounds_max = b.bounds_max;
    m.sphere = b.sphere;
    m.materials = b.materials.iter().map(|&(r, _)| r).collect();
    m.material_flags = b.materials.iter().map(|&(_, f)| (f & 3) as u8).collect();
}

