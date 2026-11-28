//! A mod's own animated models: glTF 2.0 files with a skin (a skeleton the vertices follow) and
//! animations, read for `[models.<Name>]` tables (own_models.rs).
//!
//! Everything is given in game space (glTF point (x, y, z) becomes (-z, x, y) × scale, as for the
//! static models): the points and texture coordinates of the mesh, its triangles per part, the
//! bones (bone 0 is an added root, "Root", at the model's origin) with their frame in the bind
//! pose, the influences of the bones on each point, and each animation sampled at a fixed rate,
//! as the frame of every bone at every sampled instant. The engine's own conventions (local
//! frames, quaternions) are applied by the game's side (openxiii-assets, `mod_skeletal`).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Samples per second of the animations.
pub const SAMPLE_RATE: f32 = 30.0;
/// Longest animation clip accepted (its samples are all kept in memory).
pub const MAX_CLIP_SECONDS: f32 = 600.0;
/// Bone frames kept for all the clips of a model: two clips of MAX_CLIP_SECONDS with the most
/// bones (254 joints and the root), about 440 MB.
pub const MAX_SAMPLED_FRAMES: usize = 2 * (MAX_CLIP_SECONDS * SAMPLE_RATE) as usize * 255;
/// Node transforms computed to sample all the clips (samples times nodes of the file).
pub const MAX_SAMPLED_NODES: usize = 100_000_000;
/// Name of the bone added as the root of every skeleton.
pub const ROOT_BONE: &str = "Root";
/// At most this many bones influence a point (the strongest are kept).
pub const MAX_INFLUENCES: usize = 4;

/// A frame in game space: rotation (columns = the bone's X, Y, Z axes) and position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub axes: [[f32; 3]; 3],
    pub position: [f32; 3],
}

impl Frame {
    pub const IDENTITY: Frame = Frame { axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], position: [0.0; 3] };

    /// The point `p` given in the frame, in game space.
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let a = &self.axes;
        [0, 1, 2].map(|k| a[0][k] * p[0] + a[1][k] * p[1] + a[2][k] * p[2] + self.position[k])
    }

    /// The point `p` of game space, in the frame.
    pub fn inverse_apply(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.position[0], p[1] - self.position[1], p[2] - self.position[2]];
        self.axes.map(|axis| axis[0] * d[0] + axis[1] * d[1] + axis[2] * d[2])
    }
}

/// One bone of an animated model.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index of the parent bone (always lower; the root is its own parent, 0).
    pub parent: usize,
    /// Frame of the bone in the bind pose, in game space.
    pub bind: Frame,
}

/// One part (glTF primitive) of an animated model.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Triangles, as point indices.
    pub triangles: Vec<[u32; 3]>,
    /// PNG file of the base colour texture, if any.
    pub texture: Option<PathBuf>,
}

/// One animation of an animated model, sampled at [`SAMPLE_RATE`].
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Length in seconds (at least one sample).
    pub duration: f32,
    /// For each sample (instant k / SAMPLE_RATE, k = 0…), the frame of each bone in game space.
    pub samples: Vec<Vec<Frame>>,
}

/// An animated model in game space.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedModel {
    pub points: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// For each point, (bone, weight) with weights adding up to 1.
    pub influences: Vec<Vec<(u16, f32)>>,
    pub parts: Vec<Part>,
    pub bones: Vec<Bone>,
    pub clips: Vec<Clip>,
}

type Mat4 = [[f32; 4]; 4];

const IDENTITY4: Mat4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    // Column-major: r[c][row] = sum a[k][row] * b[c][k].
    let mut r = [[0.0; 4]; 4];
    for c in 0..4 {
        for row in 0..4 {
            r[c][row] = (0..4).map(|k| a[k][row] * b[c][k]).sum();
        }
    }
    r
}

/// Inverse of a rigid (or scaled rigid) transform, by the general 4×4 inverse.
fn inverse(m: &Mat4) -> Option<Mat4> {
    // Cofactors of the 3×3 part, then the translation.
    let a = |c: usize, r: usize| m[c][r];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(2, 1) * a(1, 2)) - a(1, 0) * (a(0, 1) * a(2, 2) - a(2, 1) * a(0, 2)) + a(2, 0) * (a(0, 1) * a(1, 2) - a(1, 1) * a(0, 2));
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let mut r = IDENTITY4;
    r[0][0] = (a(1, 1) * a(2, 2) - a(2, 1) * a(1, 2)) * inv;
    r[1][0] = -(a(1, 0) * a(2, 2) - a(2, 0) * a(1, 2)) * inv;
    r[2][0] = (a(1, 0) * a(2, 1) - a(2, 0) * a(1, 1)) * inv;
    r[0][1] = -(a(0, 1) * a(2, 2) - a(2, 1) * a(0, 2)) * inv;
    r[1][1] = (a(0, 0) * a(2, 2) - a(2, 0) * a(0, 2)) * inv;
    r[2][1] = -(a(0, 0) * a(2, 1) - a(2, 0) * a(0, 1)) * inv;
    r[0][2] = (a(0, 1) * a(1, 2) - a(1, 1) * a(0, 2)) * inv;
    r[1][2] = -(a(0, 0) * a(1, 2) - a(1, 0) * a(0, 2)) * inv;
    r[2][2] = (a(0, 0) * a(1, 1) - a(1, 0) * a(0, 1)) * inv;
    for row in 0..3 {
        r[3][row] = -(r[0][row] * m[3][0] + r[1][row] * m[3][1] + r[2][row] * m[3][2]);
    }
    Some(r)
}

/// glTF vector to game vector: (x, y, z) → (-z, x, y).
fn to_game(v: [f32; 3]) -> [f32; 3] {
    [-v[2], v[0], v[1]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 {
        v.map(|c| c / l)
    } else {
        v
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A glTF transform (model space) as a game frame: the axes are mapped and made orthonormal
/// (a bone's scale is not kept), the position mapped and scaled.
fn frame_of(m: &Mat4, scale: f32) -> Frame {
    // The rotation R seen in game space is map · R · map⁻¹ (map: glTF vector → game vector,
    // (x, y, z) → (-z, x, y)). Its columns, the game's X, Y, Z axes, are the rotated images of the
    // glTF axes that map to them: map(R·(-Z)), map(R·X), map(R·Y).
    let x = normalize(to_game([-m[2][0], -m[2][1], -m[2][2]]));
    let y0 = to_game([m[0][0], m[0][1], m[0][2]]);
    // Gram-Schmidt: Y orthogonal to X, Z = X × Y (a proper rotation of the game).
    let d = x[0] * y0[0] + x[1] * y0[1] + x[2] * y0[2];
    let y = normalize([y0[0] - d * x[0], y0[1] - d * x[1], y0[2] - d * x[2]]);
    let z = cross(x, y);
    Frame { axes: [x, y, z], position: to_game([m[3][0], m[3][1], m[3][2]]).map(|c| c * scale) }
}

/// Local transform of a node: translation, rotation (x, y, z, w), scale.
#[derive(Debug, Clone, Copy)]
struct Trs {
    t: [f32; 3],
    r: [f32; 4],
    s: [f32; 3],
}

impl Trs {
    fn matrix(&self) -> Mat4 {
        let [x, y, z, w] = self.r;
        let (xx, yy, zz, xy, xz, yz, wx, wy, wz) = (x * x, y * y, z * z, x * y, x * z, y * z, w * x, w * y, w * z);
        let [sx, sy, sz] = self.s;
        [
            [(1.0 - 2.0 * (yy + zz)) * sx, 2.0 * (xy + wz) * sx, 2.0 * (xz - wy) * sx, 0.0],
            [2.0 * (xy - wz) * sy, (1.0 - 2.0 * (xx + zz)) * sy, 2.0 * (yz + wx) * sy, 0.0],
            [2.0 * (xz + wy) * sz, 2.0 * (yz - wx) * sz, (1.0 - 2.0 * (xx + yy)) * sz, 0.0],
            [self.t[0], self.t[1], self.t[2], 1.0],
        ]
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if d < 0.0 {
        b = b.map(|c| -c);
        d = -d;
    }
    let (wa, wb) = if d > 0.9995 {
        (1.0 - t, t)
    } else {
        let th = d.clamp(-1.0, 1.0).dacos();
        let s = th.dsin();
        (((1.0 - t) * th).dsin() / s, (t * th).dsin() / s)
    };
    let q = [0, 1, 2, 3].map(|k| a[k] * wa + b[k] * wb);
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    q.map(|c| c / l.max(1e-12))
}

/// One animated property of a node.
enum Values {
    Translation(Vec<[f32; 3]>),
    Rotation(Vec<[f32; 4]>),
    Scale(Vec<[f32; 3]>),
}

struct Channel {
    node: usize,
    times: Vec<f32>,
    values: Values,
    step: bool,
    /// Cubic spline: each key has (in-tangent, value, out-tangent); the value is used.
    cubic: bool,
}

impl Channel {
    /// Index of the key before `t` and the fraction towards the next one.
    fn locate(&self, t: f32) -> (usize, usize, f32) {
        let n = self.times.len();
        if n == 0 || t <= self.times[0] {
            return (0, 0, 0.0);
        }
        if t >= self.times[n - 1] {
            return (n - 1, n - 1, 0.0);
        }
        let k = self.times.partition_point(|&x| x <= t) - 1;
        let span = self.times[k + 1] - self.times[k];
        let f = if self.step || span <= 0.0 { 0.0 } else { (t - self.times[k]) / span };
        (k, k + 1, f)
    }

    fn value<T: Copy>(&self, v: &[T], k: usize) -> T {
        if self.cubic {
            v[3 * k + 1]
        } else {
            v[k]
        }
    }

    fn apply(&self, t: f32, trs: &mut Trs) {
        let (a, b, f) = self.locate(t);
        match &self.values {
            Values::Translation(v) if !v.is_empty() => trs.t = lerp3(self.value(v, a), self.value(v, b), f),
            Values::Scale(v) if !v.is_empty() => trs.s = lerp3(self.value(v, a), self.value(v, b), f),
            Values::Rotation(v) if !v.is_empty() => trs.r = slerp(self.value(v, a), self.value(v, b), f),
            _ => {}
        }
    }
}

/// Reads an animated glTF 2.0 model (the first skinned mesh of the default scene). Err if the
/// file has no skin, or cannot be read.
pub fn load_animated_gltf(path: &Path, scale: f32) -> Result<AnimatedModel, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| at(&e))?;
    let g = gltf::Gltf::from_slice(&bytes).map_err(|e| at(&e))?;
    let dir = path.parent().unwrap_or(Path::new(""));
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for b in g.document.buffers() {
        let data = match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().ok_or_else(|| at(&"the .glb has no binary chunk"))?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => return Err(at(&"embedded data URIs are not supported: use a .glb or separate .bin files")),
            gltf::buffer::Source::Uri(uri) => {
                let file = crate::path_in(dir, uri).ok_or_else(|| at(&format!("{uri}: the file must be inside the mod folder")))?;
                std::fs::read(file).map_err(|e| at(&format!("{uri}: {e}")))?
            }
        };
        buffers.push(data);
    }
    crate::check_gltf_data(&g.document, &buffers).map_err(|e| at(&e))?;
    let get = |b: gltf::Buffer| buffers.get(b.index()).map(|v| v.as_slice());
    let nodes: Vec<gltf::Node> = g.document.nodes().collect();
    let parent_of = crate::check_gltf_tree(&g.document).map_err(|e| at(&e))?;
    let rest: Vec<Trs> = nodes
        .iter()
        .map(|n| {
            let (t, r, s) = n.transform().decomposed();
            Trs { t, r, s }
        })
        .collect();
    // Every node after its parent (the tree is checked: the roots reach every node), so the
    // matrices are computed without recursion, whatever the depth of the tree.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (i, p) in parent_of.iter().enumerate() {
        if let Some(p) = p {
            children[*p].push(i);
        }
    }
    let mut parents_first: Vec<usize> = (0..nodes.len()).filter(|&i| parent_of[i].is_none()).collect();
    let mut k = 0;
    while k < parents_first.len() {
        let i = parents_first[k];
        parents_first.extend_from_slice(&children[i]);
        k += 1;
    }
    // Model-space matrix of every node for a set of local transforms (its parent's matrix times
    // its local matrix).
    let globals = |locals: &[Trs]| -> Vec<Mat4> {
        let mut out: Vec<Mat4> = vec![IDENTITY4; locals.len()];
        for &i in &parents_first {
            let local = locals[i].matrix();
            out[i] = match parent_of[i] {
                Some(p) => mul(&out[p], &local),
                None => local,
            };
        }
        out
    };

    let scene = g.document.default_scene().or_else(|| g.document.scenes().next()).ok_or_else(|| at(&"no scene"))?;
    let mut in_scene = vec![false; nodes.len()];
    let mut todo: Vec<gltf::Node> = scene.nodes().collect();
    while let Some(n) = todo.pop() {
        in_scene[n.index()] = true;
        todo.extend(n.children());
    }
    let Some(mesh_node) = nodes.iter().find(|n| in_scene[n.index()] && n.mesh().is_some() && n.skin().is_some()) else {
        return Err(at(&"no skinned mesh in the default scene"));
    };
    let skin = mesh_node.skin().ok_or_else(|| at(&"no skin"))?;
    let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
    if joints.is_empty() || joints.len() > 254 {
        return Err(at(&format!("the skin has {} joints, 1 to 254 are supported", joints.len())));
    }
    let ibms: Vec<Mat4> = match skin.reader(get).read_inverse_bind_matrices() {
        Some(m) => m.collect(),
        None => vec![IDENTITY4; joints.len()],
    };
    if ibms.len() != joints.len() {
        return Err(at(&"the skin's inverseBindMatrices do not match its joints"));
    }

    // Bones: the added root, then the joints parents first (each joint's parent is its nearest
    // ancestor among the joints, or the root).
    let joint_parent = |j: usize| -> Option<usize> {
        let mut p = parent_of[joints[j]];
        while let Some(n) = p {
            if let Some(k) = joints.iter().position(|&x| x == n) {
                return Some(k);
            }
            p = parent_of[n];
        }
        None
    };
    let mut order: Vec<usize> = Vec::new();
    let mut placed = vec![false; joints.len()];
    while order.len() < joints.len() {
        let before = order.len();
        for j in 0..joints.len() {
            if !placed[j] && joint_parent(j).is_none_or(|p| placed[p]) {
                placed[j] = true;
                order.push(j);
            }
        }
        if order.len() == before {
            return Err(at(&"the skin's joints form a loop"));
        }
    }
    // Bone index of each joint (bone 0 is the root).
    let mut bone_of = vec![0usize; joints.len()];
    for (b, &j) in order.iter().enumerate() {
        bone_of[j] = b + 1;
    }
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut bones = vec![Bone { name: ROOT_BONE.into(), parent: 0, bind: Frame::IDENTITY }];
    names.insert(ROOT_BONE.to_ascii_lowercase(), 0);
    for &j in &order {
        let node = &nodes[joints[j]];
        // A unique name: the node's, or "Bone<n>".
        let mut name = node.name().filter(|n| !n.trim().is_empty()).map_or_else(|| format!("Bone{}", joints[j]), |n| n.trim().to_string());
        let mut k = 2;
        while names.contains_key(&name.to_ascii_lowercase()) {
            name = format!("{}_{k}", node.name().unwrap_or("Bone"));
            k += 1;
        }
        names.insert(name.to_ascii_lowercase(), bones.len());
        let bind = inverse(&ibms[j]).ok_or_else(|| at(&format!("joint {name}: its inverse bind matrix cannot be inverted")))?;
        bones.push(Bone { name, parent: joint_parent(j).map_or(0, |p| bone_of[p]), bind: frame_of(&bind, scale) });
    }

    // The mesh, in the bind pose (glTF: a skinned mesh's vertices are in model space; its node's
    // own transform is not used).
    let mesh = mesh_node.mesh().ok_or_else(|| at(&"no mesh"))?;
    let (mut points, mut uvs, mut influences, mut parts) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for prim in mesh.primitives() {
        if prim.mode() != gltf::mesh::Mode::Triangles {
            continue;
        }
        let reader = prim.reader(get);
        let Some(positions) = reader.read_positions() else { continue };
        if points.len() + positions.len() > 65535 {
            return Err(at(&"at most 65535 vertices and 65535 triangles are supported"));
        }
        let base = points.len() as u32;
        let count_before = points.len();
        points.extend(positions.map(|p| to_game(p).map(|c| c * scale)));
        let n = points.len() - count_before;
        match reader.read_tex_coords(0) {
            Some(t) => uvs.extend(t.into_f32()),
            None => uvs.extend(std::iter::repeat_n([0.0, 0.0], n)),
        }
        uvs.resize(points.len(), [0.0, 0.0]);
        let joint_sets: Vec<[u16; 4]> = reader.read_joints(0).map(|j| j.into_u16().collect()).unwrap_or_default();
        let weight_sets: Vec<[f32; 4]> = reader.read_weights(0).map(|w| w.into_f32().collect()).unwrap_or_default();
        for i in 0..n {
            let mut inf: Vec<(u16, f32)> = Vec::new();
            if let (Some(js), Some(ws)) = (joint_sets.get(i), weight_sets.get(i)) {
                for k in 0..4 {
                    let (j, w) = (js[k] as usize, ws[k]);
                    if w > 0.0 && j < joints.len() {
                        let b = bone_of[j] as u16;
                        match inf.iter_mut().find(|x| x.0 == b) {
                            Some(x) => x.1 += w,
                            None => inf.push((b, w)),
                        }
                    }
                }
            }
            inf.sort_by(|a, b| b.1.total_cmp(&a.1));
            inf.truncate(MAX_INFLUENCES);
            let total: f32 = inf.iter().map(|x| x.1).sum();
            if total > 0.0 {
                inf.iter_mut().for_each(|x| x.1 /= total);
            } else {
                // No weight: the point follows the root.
                inf = vec![(0, 1.0)];
            }
            influences.push(inf);
        }
        let indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..n as u32).collect(),
        };
        if indices.iter().any(|&i| i as usize >= n) {
            return Err(at(&"an index points past the vertices"));
        }
        let triangles = indices.chunks_exact(3).map(|t| [base + t[0], base + t[1], base + t[2]]).collect();
        let texture = prim.material().pbr_metallic_roughness().base_color_texture().and_then(|t| match t.texture().source().source() {
            gltf::image::Source::Uri { uri, .. } if uri.to_ascii_lowercase().ends_with(".png") && !uri.starts_with("data:") => crate::path_in(dir, uri),
            _ => None,
        });
        parts.push(Part { triangles, texture });
    }
    if parts.is_empty() || points.is_empty() {
        return Err(at(&"no triangles in the skinned mesh"));
    }
    if points.len() > 65535 || parts.iter().map(|p| p.triangles.len()).sum::<usize>() > 65535 {
        return Err(at(&"at most 65535 vertices and 65535 triangles are supported"));
    }

    // Animations: every channel that moves a node, sampled at SAMPLE_RATE.
    let mut clips = Vec::new();
    let (mut sampled_frames, mut sampled_nodes) = (0usize, 0usize);
    for (ai, anim) in g.document.animations().enumerate() {
        let mut channels = Vec::new();
        for ch in anim.channels() {
            let reader = ch.reader(get);
            let Some(times) = reader.read_inputs() else { continue };
            let times: Vec<f32> = times.collect();
            let values = match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(v)) => Values::Translation(v.collect()),
                Some(gltf::animation::util::ReadOutputs::Rotations(v)) => Values::Rotation(v.into_f32().collect()),
                Some(gltf::animation::util::ReadOutputs::Scales(v)) => Values::Scale(v.collect()),
                _ => continue,
            };
            let interpolation = ch.sampler().interpolation();
            channels.push(Channel {
                node: ch.target().node().index(),
                times,
                values,
                step: interpolation == gltf::animation::Interpolation::Step,
                cubic: interpolation == gltf::animation::Interpolation::CubicSpline,
            });
        }
        let duration = channels.iter().filter_map(|c| c.times.last().copied()).fold(0.0f32, f32::max);
        if !(duration <= MAX_CLIP_SECONDS) {
            return Err(at(&format!("animation {ai} lasts {duration} s, at most {MAX_CLIP_SECONDS} s are supported")));
        }
        let count = ((duration * SAMPLE_RATE).round() as usize).max(1);
        sampled_frames += count * (order.len() + 1);
        sampled_nodes += count * nodes.len();
        if sampled_frames > MAX_SAMPLED_FRAMES || sampled_nodes > MAX_SAMPLED_NODES {
            return Err(at(&format!("the animations are too long for this skeleton: at most {MAX_SAMPLED_FRAMES} bone frames and {MAX_SAMPLED_NODES} node transforms in all")));
        }
        let mut samples = Vec::with_capacity(count);
        for k in 0..count {
            let t = k as f32 / SAMPLE_RATE;
            let mut locals = rest.clone();
            for c in &channels {
                c.apply(t, &mut locals[c.node]);
            }
            let world = globals(&locals);
            let mut frames = vec![Frame::IDENTITY];
            frames.extend(order.iter().map(|&j| frame_of(&world[joints[j]], scale)));
            samples.push(frames);
        }
        let name = anim.name().filter(|n| !n.trim().is_empty()).map_or_else(|| format!("Anim{ai}"), |n| n.trim().to_string());
        clips.push(Clip { name, duration: (count as f32 / SAMPLE_RATE).max(duration), samples });
    }
    Ok(AnimatedModel { points, uvs, influences, parts, bones, clips })
}

/// True when the glTF file at `path` has a skinned mesh (it is then read as an animated model).
pub fn is_animated(path: &Path) -> bool {
    std::fs::read(path).ok().and_then(|b| gltf::Gltf::from_slice(&b).ok()).is_some_and(|g| g.document.nodes().any(|n| n.skin().is_some() && n.mesh().is_some()))
}

/// The points of `model` for the frames `pose` of its bones (one per bone): each point follows
/// its bones from the bind pose, weighted. The reference answer the game's skinning is checked
/// against.
pub fn skin(model: &AnimatedModel, pose: &[Frame]) -> Vec<[f32; 3]> {
    model
        .points
        .iter()
        .zip(&model.influences)
        .map(|(&p, inf)| {
            let mut out = [0.0f32; 3];
            for &(b, w) in inf {
                let b = b as usize;
                let moved = pose[b].apply(model.bones[b].bind.inverse_apply(p));
                for k in 0..3 {
                    out[k] += moved[k] * w;
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A skinned glTF model: joint "Hips" at (0, 1, 0), its child "Arm" at (0, 1.5, 0); four
    /// points, two on each joint; animation "wave" turns the arm 90° about glTF Z in one second.
    pub(crate) fn write_arm(dir: &Path) -> PathBuf {
        let mut bin: Vec<u8> = Vec::new();
        let f = |bin: &mut Vec<u8>, v: &[f32]| v.iter().for_each(|x| bin.extend(x.to_le_bytes()));
        f(&mut bin, &[-0.5, 1.0, 0.0, 0.5, 1.0, 0.0, 0.5, 1.5, 0.0, 0.5, 2.0, 0.0]); // 0..48 positions
        bin.extend([0u8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]); // 48..64 joints
        f(&mut bin, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]); // 64..128 weights
        [0u16, 1, 2, 1, 3, 2].iter().for_each(|i| bin.extend(i.to_le_bytes())); // 128..140 indices
        for y in [-1.0f32, -1.5] {
            f(&mut bin, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, y, 0.0, 1.0]); // 140..268 IBMs
        }
        f(&mut bin, &[0.0, 1.0]); // 268..276 times
        let h = std::f32::consts::FRAC_1_SQRT_2;
        f(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, h, h]); // 276..308 rotations
        std::fs::write(dir.join("arm.bin"), &bin).unwrap();
        let json = r#"{
  "asset": {"version": "2.0"},
  "scene": 0,
  "scenes": [{"nodes": [0, 2]}],
  "nodes": [
    {"name": "Hips", "translation": [0, 1, 0], "children": [1]},
    {"name": "Arm", "translation": [0, 0.5, 0]},
    {"name": "Body", "mesh": 0, "skin": 0}
  ],
  "skins": [{"joints": [0, 1], "inverseBindMatrices": 4}],
  "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "indices": 3}]}],
  "animations": [{"name": "wave", "channels": [{"sampler": 0, "target": {"node": 1, "path": "rotation"}}], "samplers": [{"input": 5, "output": 6}]}],
  "buffers": [{"uri": "arm.bin", "byteLength": 308}],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 48},
    {"buffer": 0, "byteOffset": 48, "byteLength": 16},
    {"buffer": 0, "byteOffset": 64, "byteLength": 64},
    {"buffer": 0, "byteOffset": 128, "byteLength": 12},
    {"buffer": 0, "byteOffset": 140, "byteLength": 128},
    {"buffer": 0, "byteOffset": 268, "byteLength": 8},
    {"buffer": 0, "byteOffset": 276, "byteLength": 32}
  ],
  "accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-0.5, 1, 0], "max": [0.5, 2, 0]},
    {"bufferView": 1, "componentType": 5121, "count": 4, "type": "VEC4"},
    {"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC4"},
    {"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"},
    {"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"},
    {"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]},
    {"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC4"}
  ]
}"#;
        let path = dir.join("arm.gltf");
        std::fs::write(&path, json).unwrap();
        path
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 0.01)
    }

    #[test]
    fn skinned_model_in_game_space() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_arm(tmp.path());
        assert!(is_animated(&path));
        assert!(!is_animated(&crate::own_models::tests::write_triangle(tmp.path())));
        let m = load_animated_gltf(&path, 50.0).unwrap();
        let names: Vec<(&str, usize)> = m.bones.iter().map(|b| (b.name.as_str(), b.parent)).collect();
        assert_eq!(names, [("Root", 0), ("Hips", 0), ("Arm", 1)]);
        // glTF (0, 1.5, 0) m is 75 units up.
        assert!(close(m.bones[2].bind.position, [0.0, 0.0, 75.0]), "{:?}", m.bones[2].bind);
        assert!(close(m.points[2], [0.0, 25.0, 75.0]), "{:?}", m.points);
        assert_eq!(m.influences[2], [(2, 1.0)]);
        assert_eq!(m.parts[0].triangles, [[0, 1, 2], [1, 3, 2]]);
        let clip = &m.clips[0];
        assert_eq!((clip.name.as_str(), clip.samples.len(), clip.duration), ("wave", 30, 1.0));
        // Bind pose: the points do not move.
        let bind: Vec<Frame> = m.bones.iter().map(|b| b.bind).collect();
        for (a, b) in skin(&m, &bind).iter().zip(&m.points) {
            assert!(close(*a, *b));
        }
        // Half way (45°): the arm's point (0.5, 1.5, 0) turns about the arm's joint, glTF Z.
        let s = std::f32::consts::FRAC_1_SQRT_2 * 0.5;
        let expected = [0.0, s * 50.0, (1.5 + s) * 50.0];
        let half = skin(&m, &clip.samples[15]);
        assert!(close(half[2], expected), "{:?} instead of {expected:?}", half[2]);
        // The hips' points stay.
        assert!(close(half[0], m.points[0]) && close(half[1], m.points[1]));
        // Every frame is a rotation (orthonormal, right-handed).
        for f in clip.samples.iter().flatten() {
            let [x, y, z] = f.axes;
            let c = cross(x, y);
            assert!((0..3).all(|k| (c[k] - z[k]).abs() < 1e-4), "{f:?}");
        }
    }

    #[test]
    fn files_without_skin_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = crate::own_models::tests::write_triangle(tmp.path());
        assert!(load_animated_gltf(&path, 50.0).unwrap_err().contains("no skinned mesh"));
    }
}
