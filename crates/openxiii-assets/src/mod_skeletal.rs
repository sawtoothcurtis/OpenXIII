//! Mods build only (feature `mods`): the mods' own animated meshes (skinned glTF models), by
//! mesh name, and their animations, by animation name.
//!
//! A mod's animated model becomes a skeletal mesh and a MeshAnimation of the game, built in
//! memory: the characters, weapons and other actors whose `Mesh` names it are animated, drawn
//! and hit by the engine's own code, as with the game's meshes. The table is set once, when the
//! game starts (docs/modding/README.md, "Animated models").
//!
//! The engine's conventions (USkeletalMesh RefSkeleton, MotionChunk tracks): the root bone's
//! quaternion is its frame; every other bone stores the conjugate of its rotation relative to
//! its parent; positions are relative to the parent's frame. Animation keys are times in
//! seconds of the track, quaternions packed on 16 bits.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::animation::{Move, Track};
use crate::skeletal::{Bone, Face, SkeletalMesh, Wedge};

/// A sequence of a mod's animation (as the game's AnimSeqs).
#[derive(Debug, Clone, PartialEq)]
pub struct ModSequence {
    pub name: String,
    pub num_frames: i32,
    /// Frames per second.
    pub rate: f32,
    /// (time as a fraction of the sequence, script function called).
    pub notifies: Vec<(f32, String)>,
}

/// A mod's animation: the bones it moves (by name) and one movement per sequence.
#[derive(Debug, Clone)]
pub struct ModAnimation {
    pub bones: Vec<String>,
    pub sequences: Vec<ModSequence>,
    pub moves: Vec<Move>,
}

/// A mod's animated mesh.
#[derive(Debug, Clone)]
pub struct ModSkeletal {
    /// The mesh; `Bone::name` is an index into `bone_names`.
    pub mesh: SkeletalMesh,
    pub bone_names: Vec<String>,
    /// Material (texture) name of each part (FMeshMaterial texture index = part index).
    pub textures: Vec<String>,
    /// Name of its animation (`animation`).
    pub animation_name: String,
    /// Bone names the game's scripts use → the model's bones (TagAliases → TagNames).
    pub aliases: Vec<(String, String)>,
    /// Box around the points in the reference pose (UPrimitive BoundingBox).
    pub bounds: ([f32; 3], [f32; 3]),
    /// Collision box of each bone (USkeletalMesh + 0x1A8): around the points it moves most, in
    /// its reference frame. A trace that asks for bones finds the bone it crosses.
    pub bone_boxes: Vec<crate::skeletal_collision::BoneBox>,
    /// UPrimitive+0x28..+0x30 bytes (outline, cartoon shading…): `mod_meshes::cartoon_bytes`.
    pub cartoon: [u8; 8],
    /// Made from a game mesh (`[arms.<Name>]`, mod_arms.rs): drawn as the game draws its own
    /// meshes (outline from its smooth normals, no outline shell; outline thickness from its own
    /// location in first person).
    pub game_look: bool,
}

/// The mesh of a mod's animated model as drawn, its batches (material, wedge indices, part index)
/// and the reference normals of its points. When the outline is on (`cartoon[0]`), one outline
/// shell batch is added for each part, as for the static models (mod_meshes::with_outline_shell):
/// the same triangles on points shared by position and bones (same influences, so the shell
/// follows the skeleton), each with the average of the distinct face directions around it. Its
/// material is `mod_meshes::OUTLINE_SHELL` + the part's; only the outline pass draws it. The
/// model's own points, faces and normals are unchanged. Only a mod's own models get it.
pub fn drawn(m: &ModSkeletal) -> (SkeletalMesh, Vec<(String, Vec<u32>, i32)>, Vec<[f32; 3]>) {
    let mut mesh = m.mesh.clone();
    let mut batches = Vec::new();
    for (i, texture) in m.textures.iter().enumerate() {
        let wedges: Vec<u32> = mesh.faces.iter().filter(|f| f.material as usize == i).flat_map(|f| f.wedges.map(|w| w as u32)).collect();
        if !wedges.is_empty() {
            batches.push((texture.clone(), wedges, i as i32));
        }
    }
    let mut normals = mesh.calculate_normals();
    if m.cartoon[0] == 0 || m.game_look {
        return (mesh, batches, normals);
    }
    let original_faces = mesh.faces.clone();
    for (i, texture) in m.textures.iter().enumerate() {
        let mut slot: HashMap<Vec<u32>, usize> = HashMap::new();
        let mut new_points: Vec<usize> = Vec::new();
        let mut dirs: Vec<Vec<[f32; 3]>> = Vec::new();
        let mut shell: Vec<[usize; 3]> = Vec::new();
        for f in original_faces.iter().filter(|f| f.material as usize == i) {
            let pts = f.wedges.map(|w| mesh.wedges[w as usize].point as usize);
            let p = pts.map(|k| mesh.points[k]);
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let mut n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            // Same side as the model's own normals of these points.
            let given = pts.iter().fold([0.0f32; 3], |a, &k| [a[0] + normals[k][0], a[1] + normals[k][1], a[2] + normals[k][2]]);
            if n[0] * given[0] + n[1] * given[1] + n[2] * given[2] < 0.0 {
                n = n.map(|c| -c);
            }
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let mut tri = [0usize; 3];
            for (k, &pt) in pts.iter().enumerate() {
                let mut key: Vec<u32> = mesh.points[pt].iter().map(|c| c.to_bits()).collect();
                for &(b, w) in &mesh.influences[pt] {
                    key.push(b as u32);
                    key.push(w.to_bits());
                }
                let v = *slot.entry(key).or_insert_with(|| {
                    new_points.push(pt);
                    dirs.push(Vec::new());
                    new_points.len() - 1
                });
                if len > 1e-12 {
                    let u = n.map(|c| c / len);
                    if !dirs[v].iter().any(|w: &[f32; 3]| w[0] * u[0] + w[1] * u[1] + w[2] * u[2] > 0.9999) {
                        dirs[v].push(u);
                    }
                }
                tri[k] = v;
            }
            shell.push(tri);
        }
        let base_point = mesh.points.len();
        let base_wedge = mesh.wedges.len();
        if shell.is_empty() || base_point + new_points.len() > 65535 || base_wedge + new_points.len() > 65535 || mesh.faces.len() + shell.len() > 65535 {
            continue;
        }
        for (v, &pt) in new_points.iter().enumerate() {
            mesh.points.push(mesh.points[pt]);
            mesh.influences.push(mesh.influences[pt].clone());
            mesh.wedges.push(Wedge { point: (base_point + v) as u16, uv: [0.0; 2] });
            let a = dirs[v].iter().fold([0.0f32; 3], |a, u| [a[0] + u[0], a[1] + u[1], a[2] + u[2]]);
            let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            normals.push(if l > 1e-12 { a.map(|c| c / l) } else { normals[pt] });
        }
        let material = (m.textures.len() + i) as u16;
        let mut wedges = Vec::with_capacity(shell.len() * 3);
        for t in &shell {
            let w = t.map(|v| (base_wedge + v) as u16);
            mesh.faces.push(Face { wedges: w, material });
            wedges.extend(w.map(|x| x as u32));
        }
        batches.push((format!("{}{}", crate::mod_meshes::OUTLINE_SHELL, texture), wedges, i as i32));
    }
    (mesh, batches, normals)
}

/// Scale of the bones a mod hides in a first-person mesh (FirstPersonHideBones, mod_weapons.rs):
/// 1/1024, exact in f32, so that the renderer can tell them apart (CharacterModel::vertices_bases).
pub const HIDDEN_BONE_SCALE: f32 = 1.0 / 1024.0;

/// For each bone frame of a pose, true if a mod hid it (scale HIDDEN_BONE_SCALE, set by
/// SetBoneScale on that bone only). None when no bone is hidden.
pub fn hidden_bones(bases: &[crate::coords::Coords]) -> Option<Vec<bool>> {
    // (Tested in tests::hidden_bones_found.)
    let hidden: Vec<bool> = bases
        .iter()
        .map(|b| {
            let l = (b.x[0] * b.x[0] + b.x[1] * b.x[1] + b.x[2] * b.x[2]).sqrt();
            (l - HIDDEN_BONE_SCALE).abs() < HIDDEN_BONE_SCALE * 1.0e-3
        })
        .collect();
    hidden.iter().any(|&h| h).then_some(hidden)
}

static MESHES: OnceLock<HashMap<String, Arc<ModSkeletal>>> = OnceLock::new();
static ANIMATIONS: OnceLock<HashMap<String, Arc<ModAnimation>>> = OnceLock::new();

/// Sets the mods' animated meshes by mesh name, and their animations by animation name (once;
/// a second call is ignored).
pub fn set(meshes: HashMap<String, Arc<ModSkeletal>>, animations: HashMap<String, Arc<ModAnimation>>) {
    let _ = MESHES.set(meshes.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect());
    let _ = ANIMATIONS.set(animations.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect());
}

/// The mod's animated mesh named `name`, if any (the mods' meshes, then those made while the
/// game runs).
pub fn get(name: &str) -> Option<Arc<ModSkeletal>> {
    let key = name.to_ascii_lowercase();
    if let Some(m) = MESHES.get().and_then(|t| t.get(&key)) {
        return Some(m.clone());
    }
    RUNTIME.get()?.read().ok()?.get(&key).cloned()
}

/// Animated meshes made while the game runs (the co-op players' first-person arms), by name.
static RUNTIME: OnceLock<std::sync::RwLock<HashMap<String, Arc<ModSkeletal>>>> = OnceLock::new();

/// Adds or replaces an animated mesh made while the game runs ("Mod.<Name>.<Mesh>"). A mod's
/// mesh with the same name keeps priority.
pub fn add_runtime(name: &str, mesh: Arc<ModSkeletal>) {
    let table = RUNTIME.get_or_init(Default::default);
    if let Ok(mut t) = table.write() {
        t.insert(name.to_ascii_lowercase(), mesh);
    }
}

/// The mod's animation named `name`, if any.
pub fn animation(name: &str) -> Option<Arc<ModAnimation>> {
    ANIMATIONS.get()?.get(&name.to_ascii_lowercase()).cloned()
}

/// A frame in mesh space: rotation (columns = the bone's axes) and position.
pub type Frame = ([[f32; 3]; 3], [f32; 3]);

/// One bone to build: name, parent (lower index; bone 0 is the root), frame in the bind pose.
pub struct BoneIn<'a> {
    pub name: &'a str,
    pub parent: usize,
    pub bind: Frame,
}

/// One animation to build: sequence name, frames per second, and for each sample the frame of
/// each bone in mesh space.
pub struct ClipIn<'a> {
    pub name: &'a str,
    pub rate: f32,
    pub samples: &'a [Vec<Frame>],
    pub notifies: Vec<(f32, String)>,
}

/// Rotation matrix R (rows) of a frame: R[row][col] = axes[col][row].
fn rows(axes: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [0, 1, 2].map(|r| [axes[0][r], axes[1][r], axes[2][r]])
}

fn mat_mul(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [0, 1, 2].map(|r| [0, 1, 2].map(|c| a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]))
}

fn transpose(a: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    [0, 1, 2].map(|r| [a[0][r], a[1][r], a[2][r]])
}

/// Quaternion (x, y, z, w) whose rotation matrix, as `coords::from_quat` builds it, is `m`.
pub fn quat_of(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let t = m[0][0] + m[1][1] + m[2][2];
    let q = if t > 0.0 {
        let s = (t + 1.0).sqrt() * 2.0;
        [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
    };
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt().max(1e-12);
    q.map(|c| c / l)
}

/// The engine's local pose (quaternion, position) of each bone, from their frames in mesh
/// space. The root keeps its frame; another bone stores the conjugate of its rotation relative
/// to its parent (the transpose of R_parentᵀ·R), and its position in the parent's frame.
pub fn local_poses(parents: &[usize], frames: &[Frame]) -> Vec<([f32; 4], [f32; 3])> {
    frames
        .iter()
        .enumerate()
        .map(|(b, (axes, position))| {
            let r = rows(axes);
            if b == 0 {
                return (quat_of(&r), *position);
            }
            let (pa, pp) = &frames[parents[b]];
            let rp = rows(pa);
            // Stored matrix: (R_pᵀ·R)ᵀ = Rᵀ·R_p.
            let stored = mat_mul(&transpose(&r), &rp);
            let d = [position[0] - pp[0], position[1] - pp[1], position[2] - pp[2]];
            // R_pᵀ·d: components along the parent's axes.
            let local = [0, 1, 2].map(|k| pa[k][0] * d[0] + pa[k][1] * d[1] + pa[k][2] * d[2]);
            (quat_of(&stored), local)
        })
        .collect()
}

/// One box per bone that is the strongest influence of at least one point: the box of those
/// points in the bone's reference frame (as USkeletalMesh::LineCheck brings a trace into it).
pub fn bone_boxes(mesh: &SkeletalMesh) -> Vec<crate::skeletal_collision::BoneBox> {
    let bases = mesh.ref_space_bases();
    let mut boxes: Vec<Option<([f32; 3], [f32; 3])>> = vec![None; bases.len()];
    for (p, inf) in mesh.points.iter().zip(&mesh.influences) {
        let Some(&(bone, _)) = inf.iter().max_by(|a, b| a.1.total_cmp(&b.1)) else { continue };
        let Some(base) = bases.get(bone as usize) else { continue };
        let inv = base.inverse();
        let local = [inv.x, inv.y, inv.z].map(|a| a[0] * p[0] + a[1] * p[1] + a[2] * p[2]);
        let local = [0, 1, 2].map(|k| local[k] + inv.origin[k]);
        let b = boxes[bone as usize].get_or_insert((local, local));
        for k in 0..3 {
            b.0[k] = b.0[k].min(local[k]);
            b.1[k] = b.1[k].max(local[k]);
        }
    }
    boxes.into_iter().enumerate().filter_map(|(bone, b)| b.map(|(min, max)| crate::skeletal_collision::BoneBox { min, max, valid: 1, bone: bone as u16 })).collect()
}

/// Packs a quaternion as the game's animations store it (4 × i16, 1 = 32767).
fn pack(q: [f32; 4]) -> [i16; 4] {
    q.map(|c| (c.clamp(-1.0, 1.0) * 32767.0).round() as i16)
}

/// Builds a mod's animated mesh and its animation. `points`, `uvs` and `influences` are per
/// point (one wedge per point); `parts` gives each part's triangles (point indices) and
/// material name.
pub fn build(
    points: &[[f32; 3]],
    uvs: &[[f32; 2]],
    influences: &[Vec<(u16, f32)>],
    parts: &[(Vec<[u32; 3]>, String)],
    bones: &[BoneIn],
    clips: &[ClipIn],
    animation_name: &str,
) -> Result<(ModSkeletal, ModAnimation), String> {
    if points.len() > 65535 || points.is_empty() {
        return Err("1 to 65535 vertices are supported".into());
    }
    if bones.is_empty() || bones.iter().enumerate().any(|(i, b)| i > 0 && b.parent >= i) {
        return Err("bone 0 must be the root and every bone must come after its parent".into());
    }
    let mut faces = Vec::new();
    for (m, (triangles, _)) in parts.iter().enumerate() {
        for t in triangles {
            if t.iter().any(|&i| i as usize >= points.len()) {
                return Err("a triangle points past the vertices".into());
            }
            faces.push(Face { wedges: t.map(|i| i as u16), material: m as u16 });
        }
    }
    if faces.len() > 65535 {
        return Err("at most 65535 triangles are supported".into());
    }
    let parents: Vec<usize> = bones.iter().map(|b| b.parent).collect();
    let bind: Vec<Frame> = bones.iter().map(|b| b.bind).collect();
    let ref_pose = local_poses(&parents, &bind);
    let mesh_bones = bones
        .iter()
        .zip(&ref_pose)
        .enumerate()
        .map(|(i, (b, &(q, p)))| Bone { name: i as i32, flags: 0, orientation: q, position: p, parent: if i == 0 { 0 } else { b.parent as i32 } })
        .collect();
    let mesh = SkeletalMesh {
        version: 1,
        legacy_vertices: Vec::new(),
        legacy_triangles: Vec::new(),
        textures: Vec::new(),
        scale: [1.0; 3],
        origin: [0.0; 3],
        rot_origin: [0; 3],
        faces,
        wedges: (0..points.len()).map(|i| Wedge { point: i as u16, uv: uvs.get(i).copied().unwrap_or([0.0; 2]) }).collect(),
        materials: (0..parts.len()).map(|m| (0, m as i32)).collect(),
        points: points.to_vec(),
        bones: mesh_bones,
        animation: 0,
        influences: (0..points.len()).map(|i| influences.get(i).cloned().unwrap_or_else(|| vec![(0, 1.0)])).collect(),
    };
    let bone_names: Vec<String> = bones.iter().map(|b| b.name.to_string()).collect();

    let mut sequences = Vec::new();
    let mut moves = Vec::new();
    for c in clips {
        let n = c.samples.len().max(1);
        let rate = if c.rate > 0.0 { c.rate } else { 30.0 };
        let mut mv = Move {
            root_speed: [0.0; 3],
            track_time: n as f32 / rate,
            start_bone: 0,
            flags: 0,
            bone_indices: (0..bones.len() as i32).collect(),
            tracks: Vec::new(),
            quats: Vec::new(),
            raw_quats: Vec::new(),
            positions: Vec::new(),
            times: Vec::new(),
        };
        let poses: Vec<Vec<([f32; 4], [f32; 3])>> = (0..n).map(|k| c.samples.get(k).map_or_else(|| ref_pose.clone(), |s| local_poses(&parents, s))).collect();
        for b in 0..bones.len() {
            let start = mv.quats.len();
            if start + n > u16::MAX as usize {
                return Err(format!("animation {}: too many keys (bones × samples above 65535)", c.name));
            }
            mv.tracks.push(Track { pos_start: start as u16, quat_start: start as u16, quat_count: n as u16, pos_count: n as u16 });
            let mut previous: Option<[f32; 4]> = None;
            for (k, pose) in poses.iter().enumerate() {
                let (mut q, p) = pose[b];
                // Keys of a track on the same side, so that the engine interpolates the short way.
                if let Some(prev) = previous {
                    if prev[0] * q[0] + prev[1] * q[1] + prev[2] * q[2] + prev[3] * q[3] < 0.0 {
                        q = q.map(|c| -c);
                    }
                }
                previous = Some(q);
                mv.quats.push(q);
                mv.raw_quats.push(pack(q));
                mv.positions.push(p);
                mv.times.push(k as f32 / rate);
            }
        }
        moves.push(mv);
        sequences.push(ModSequence { name: c.name.to_string(), num_frames: n as i32, rate, notifies: c.notifies.clone() });
    }
    let textures = parts.iter().map(|(_, m)| m.clone()).collect();
    let bone_boxes = bone_boxes(&mesh);
    let bounds = points.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| ([0, 1, 2].map(|k| lo[k].min(p[k])), [0, 1, 2].map(|k| hi[k].max(p[k]))));
    Ok((ModSkeletal { mesh, bone_names: bone_names.clone(), textures, animation_name: animation_name.to_string(), aliases: Vec::new(), bounds, bone_boxes, cartoon: crate::mod_meshes::cartoon_bytes(true, true, 1.0), game_look: false }, ModAnimation { bones: bone_names, sequences, moves }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn animated_outline_shell() {
        // A cube with a point per face corner (24), all on one bone.
        let mut points = Vec::new();
        let mut tris = Vec::new();
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                let b = points.len() as u32;
                for (a, c) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let mut p = [0.0f32; 3];
                    p[axis] = sign;
                    p[u] = a;
                    p[v] = c;
                    points.push(p);
                }
                // Same turning direction seen from outside on every face.
                if sign > 0.0 {
                    tris.push([b, b + 1, b + 2]);
                    tris.push([b, b + 2, b + 3]);
                } else {
                    tris.push([b, b + 2, b + 1]);
                    tris.push([b, b + 3, b + 2]);
                }
            }
        }
        let influences = vec![vec![(0u16, 1.0f32)]; points.len()];
        let id: Frame = ([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [0.0; 3]);
        let bones = [BoneIn { name: "Root", parent: 0, bind: id }];
        let (mut m, _) = build(&points, &[], &influences, &[(tris, "Mod.Box.0".into())], &bones, &[], "Mod.BoxAnim").unwrap();
        let (mesh0, b0, n0) = drawn(&m);
        assert_eq!(b0.len(), 2);
        assert_eq!((b0[1].0.as_str(), b0[1].2, b0[1].1.len()), ("Mod.#outline:Mod.Box.0", 0, 36));
        // The model's own points and normals are unchanged; 8 shell corners follow bone 0, each
        // normal along the diagonal.
        assert_eq!(mesh0.points.len(), 24 + 8);
        assert_eq!(n0[..24], m.mesh.calculate_normals()[..]);
        assert!(mesh0.influences[24..].iter().all(|i| i == &vec![(0u16, 1.0f32)]));
        // (Along the diagonal, on the side of the model's own normals.)
        let d = 1.0 / 3.0f32.sqrt();
        let side = (n0[0][0] * points[0][0] + n0[0][1] * points[0][1] + n0[0][2] * points[0][2]).signum();
        for k in 24..32 {
            let (p, n) = (mesh0.points[k], n0[k]);
            for c in 0..3 {
                assert!((n[c] - side * p[c].signum() * d).abs() < 1e-5, "{p:?} {n:?}");
            }
        }
        // Outline off: no shell.
        m.cartoon = crate::mod_meshes::cartoon_bytes(false, true, 1.0);
        let (mesh1, b1, _) = drawn(&m);
        assert_eq!((mesh1.points.len(), b1.len()), (24, 1));
    }

    #[test]
    fn hidden_bones_found() {
        use crate::coords::UNIT;
        let scaled = |s: f32| crate::coords::Coords { x: UNIT.x.map(|c| c * s), y: UNIT.y.map(|c| c * s), z: UNIT.z.map(|c| c * s), ..UNIT };
        assert_eq!(super::hidden_bones(&[UNIT, scaled(0.5), scaled(0.0)]), None);
        assert_eq!(super::hidden_bones(&[UNIT, scaled(super::HIDDEN_BONE_SCALE)]), Some(vec![false, true]));
        assert_eq!(super::hidden_bones(&[scaled(1.0e-3)]), None);
    }

    use super::*;
    use crate::coords::{self, Coords};

    fn rot_z(deg: f32) -> [[f32; 3]; 3] {
        let (s, c) = deg.to_radians().dsin_cos();
        // Columns: the rotated X and Y axes, Z.
        [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
    }

    fn rot_y(deg: f32) -> [[f32; 3]; 3] {
        let (s, c) = deg.to_radians().dsin_cos();
        [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]]
    }

    fn compose(a: &Frame, b: &Frame) -> Frame {
        // a ∘ b: axes a·b, position a(b.position).
        let ra = rows(&a.0);
        let rb = rows(&b.0);
        let r = mat_mul(&ra, &rb);
        let axes = [0, 1, 2].map(|c| [r[0][c], r[1][c], r[2][c]]);
        let p = [0, 1, 2].map(|k| ra[k][0] * b.1[0] + ra[k][1] * b.1[1] + ra[k][2] * b.1[2] + a.1[k]);
        (axes, p)
    }

    fn apply(f: &Frame, p: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|k| f.0[0][k] * p[0] + f.0[1][k] * p[1] + f.0[2][k] * p[2] + f.1[k])
    }

    fn inverse_apply(f: &Frame, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - f.1[0], p[1] - f.1[1], p[2] - f.1[2]];
        f.0.map(|a| a[0] * d[0] + a[1] * d[1] + a[2] * d[2])
    }

    /// Engine frames of a pose: quaternions → axes, composed with the parent (as GetFrame).
    fn engine_bases(mesh: &SkeletalMesh, locals: &[([f32; 4], [f32; 3])]) -> Vec<Coords> {
        let mut bases: Vec<Coords> = locals.iter().map(|&(q, p)| coords::from_quat(q, p)).collect();
        for i in 1..bases.len() {
            let parent = mesh.bones[i].parent as usize;
            bases[i] = bases[i].compose_bone(&bases[parent]);
        }
        bases
    }

    /// A three-bone arm (root, shoulder rotated and moved, elbow) with points along it: the
    /// engine's reference frames are the given ones, and the engine's skinning of a pose moves
    /// every point as the frames do (reference answer: bind⁻¹ then pose, weighted).
    #[test]
    fn engine_frames_and_skinning_match_the_frames_given() {
        let root: Frame = (rot_z(0.0), [0.0; 3]);
        let shoulder: Frame = (rot_z(30.0), [10.0, 5.0, 40.0]);
        let elbow: Frame = compose(&shoulder, &(rot_y(-20.0), [25.0, 0.0, 0.0]));
        let bind = [root, shoulder, elbow];
        let bones: Vec<BoneIn> = ["Root", "Shoulder", "Elbow"].iter().zip(bind.iter()).zip([0, 0, 1]).map(|((n, f), p)| BoneIn { name: n, parent: p, bind: *f }).collect();
        let points: Vec<[f32; 3]> = (0..12).map(|i| apply(&shoulder, [i as f32 * 4.0, (i % 3) as f32 - 1.0, (i % 2) as f32 * 2.0])).collect();
        let influences: Vec<Vec<(u16, f32)>> = (0..12).map(|i| if i < 4 { vec![(1, 1.0)] } else if i < 8 { vec![(1, 0.5), (2, 0.5)] } else { vec![(2, 0.75), (1, 0.25)] }).collect();
        // A pose: the shoulder turned about its own Y, the elbow bent more, the root moved.
        let p_root: Frame = (rot_z(15.0), [3.0, -2.0, 1.0]);
        let p_shoulder = compose(&p_root, &(mat_mul_cols(&rot_z(30.0), &rot_y(35.0)), [10.0, 5.0, 40.0]));
        let p_elbow = compose(&p_shoulder, &(rot_y(-70.0), [25.0, 0.0, 0.0]));
        let pose = vec![p_root, p_shoulder, p_elbow];
        let samples = vec![bind.to_vec(), pose.clone()];
        let (m, a) = build(&points, &[], &influences, &[(vec![[0, 1, 2]], "Mod.Paint".into())], &bones, &[ClipIn { name: "Bend", rate: 30.0, samples: &samples, notifies: Vec::new() }], "Mod.Arm").unwrap();
        let mesh = &m.mesh;
        // Reference frames: as given (Coords rows = the frame's rotation rows).
        for (b, f) in mesh.ref_space_bases().iter().zip(&bind) {
            let r = rows(&f.0);
            for k in 0..3 {
                assert!((b.origin[k] - f.1[k]).abs() < 1e-3, "{b:?} {f:?}");
                for c in 0..3 {
                    let axis = [b.x, b.y, b.z][k];
                    assert!((axis[c] - r[k][c]).abs() < 1e-4, "{b:?} {f:?}");
                }
            }
        }
        // Second key of the animation, through the engine's frames and skinning.
        let mv = &a.moves[0];
        assert_eq!((mv.track_time, a.sequences[0].num_frames), (2.0 / 30.0, 2));
        let locals: Vec<([f32; 4], [f32; 3])> = mv.tracks.iter().map(|t| (mv.quats[t.quat_start as usize + 1], mv.positions[t.pos_start as usize + 1])).collect();
        let skinned = mesh.skin(&engine_bases(mesh, &locals), &mesh.ref_bases());
        for (i, p) in points.iter().enumerate() {
            let mut expected = [0.0f32; 3];
            for &(b, w) in &influences[i] {
                let moved = apply(&pose[b as usize], inverse_apply(&bind[b as usize], *p));
                for k in 0..3 {
                    expected[k] += moved[k] * w;
                }
            }
            for k in 0..3 {
                assert!((skinned[i][k] - expected[k]).abs() < 0.02, "point {i}: {:?} instead of {expected:?}", skinned[i]);
            }
        }
        // Bone boxes: one per bone that moves points, holding them in the bone's reference frame.
        assert_eq!(m.bone_boxes.iter().map(|b| b.bone).collect::<Vec<_>>(), [1, 2]);
        let bases = mesh.ref_space_bases();
        for (p, inf) in points.iter().zip(&influences) {
            let bone = inf.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
            let b = m.bone_boxes.iter().find(|b| b.bone == bone).unwrap();
            let inv = bases[bone as usize].inverse();
            let local = [0, 1, 2].map(|k| [inv.x, inv.y, inv.z][k].iter().zip(p).map(|(a, c)| a * c).sum::<f32>() + inv.origin[k]);
            assert!((0..3).all(|k| b.min[k] - 1e-3 <= local[k] && local[k] <= b.max[k] + 1e-3), "{local:?} {b:?}");
        }
        // The packed keys are the quaternions.
        for (q, r) in mv.quats.iter().zip(&mv.raw_quats) {
            for k in 0..4 {
                assert!((q[k] - r[k] as f32 / 32767.0).abs() < 1e-4);
            }
        }
    }

    /// Product of two rotations given by their columns.
    fn mat_mul_cols(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
        let r = mat_mul(&rows(a), &rows(b));
        [0, 1, 2].map(|c| [r[0][c], r[1][c], r[2][c]])
    }

    #[test]
    fn quaternion_round_trip() {
        for (a, b) in [(0.0, 0.0), (90.0, 0.0), (180.0, 30.0), (-120.0, 170.0), (45.0, -179.0)] {
            let m = rows(&mat_mul_cols(&rot_z(a), &rot_y(b)));
            let c = coords::from_quat(quat_of(&m), [0.0; 3]);
            for (k, axis) in [c.x, c.y, c.z].iter().enumerate() {
                for j in 0..3 {
                    assert!((axis[j] - m[k][j]).abs() < 1e-5, "{a} {b}: {c:?} {m:?}");
                }
            }
        }
    }
}
