//! XIII's skeletal meshes (characters, animals, animated objects): reference pose
//! geometry, materials and skeleton.
//!
//! Format (deduced from the files):
//! - UPrimitive: FBox, sphere, 8 XIII-specific bytes;
//! - ULodMesh: Version u32, VertexCount u32, compact word array and legacy triangle array, Textures (compact n ×
//!   reference), MeshScale, MeshOrigin (3 × f32), RotOrigin (3 × i32), CollapsePointThus (u16),
//!   FaceLevel (u16), Faces (3 × u16 wedge + u16 material), CollapseWedgeThus (u16),
//!   Wedges (u16 point + U + V f32), Materials (u32 PolyFlags + i32 texture index),
//!   MeshScaleMax, LODHysteresis, LODStrength (f32), LODMinVerts (i32), LODMorph, LODZDisplace (f32);
//! - USkeletalMesh: Points (FVector), RefSkeleton (compact name, u32 flags, quaternion,
//!   position, 4 × f32, i32 parent), Animation (reference), SkeletalDepth i32, WeightIndices,
//!   BoneInfluences (see `read`), then attachments (not read).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::coords::{self, Coords};
use openxiii_package::{Package, Reader, Result};

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: i32,
    pub flags: u32,
    pub orientation: [f32; 4],
    pub position: [f32; 3],
    pub parent: i32,
}

#[derive(Debug, Clone)]
pub struct Face {
    pub wedges: [u16; 3],
    pub material: u16,
}

#[derive(Debug, Clone)]
pub struct Wedge {
    pub point: u16,
    pub uv: [f32; 2],
}

/// Triangle of the historical ULodMesh format: 3 indices then 8 raw words.
/// The two padding bytes of the native structure are not serialized.
#[derive(Debug, Clone)]
pub struct LegacyTriangle {
    pub indices: [u16; 3],
    pub words: [u32; 8],
}

#[derive(Debug, Clone)]
pub struct SkeletalMesh {
    pub version: u32,
    pub legacy_vertices: Vec<u32>,
    pub legacy_triangles: Vec<LegacyTriangle>,
    pub textures: Vec<i32>,
    pub scale: [f32; 3],
    pub origin: [f32; 3],
    pub rot_origin: [i32; 3],
    pub faces: Vec<Face>,
    pub wedges: Vec<Wedge>,
    /// (PolyFlags, index into `textures`).
    pub materials: Vec<(u32, i32)>,
    pub points: Vec<[f32; 3]>,
    pub bones: Vec<Bone>,
    /// Default animation (object reference to a MeshAnimation).
    pub animation: i32,
    /// Influences per point: (bone, weight).
    pub influences: Vec<Vec<(u16, f32)>>,
}

/// 1/65535, scale of the influence weights (u16).
const WEIGHT_SCALE: f32 = 1.525_902_2e-5;

fn f32r(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([f32r(r)?, f32r(r)?, f32r(r)?])
}

fn u16s(r: &mut Reader) -> Result<Vec<u16>> {
    (0..r.compact()?).map(|_| r.u16()).collect()
}

// Loading branch: compact count then fields.
// These readers are used by read_native.
pub(crate) fn read_legacy_vertices(r: &mut Reader) -> Result<Vec<u32>> {
    (0..r.compact()?).map(|_| r.u32()).collect()
}

pub(crate) fn read_legacy_triangles(r: &mut Reader) -> Result<Vec<LegacyTriangle>> {
    (0..r.compact()?)
        .map(|_| Ok(LegacyTriangle {
            indices: [r.u16()?, r.u16()?, r.u16()?],
            words: [r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?],
        }))
        .collect()
}

// Readers shared with the full loader.
fn read_faces(r: &mut Reader) -> Result<Vec<Face>> {
    (0..r.compact()?).map(|_| Ok(Face { wedges: [r.u16()?, r.u16()?, r.u16()?], material: r.u16()? })).collect()
}

fn read_wedges(r: &mut Reader) -> Result<Vec<Wedge>> {
    (0..r.compact()?).map(|_| Ok(Wedge { point: r.u16()?, uv: [f32r(r)?, f32r(r)?] })).collect()
}

fn read_materials(r: &mut Reader) -> Result<Vec<(u32, i32)>> {
    (0..r.compact()?).map(|_| Ok((r.u32()?, r.i32()?))).collect()
}

// UPrimitive then UMesh, persistent read of a package.
// Old transient fields are not kept by UMesh.
pub(crate) fn skip_mesh_prefix(r: &mut Reader, licensee: u16) -> Result<()> {
    r.take(25 + 16)?; // FBox and FSphere of UPrimitive.
    if licensee >= 19 {
        r.take(8)?; // UPrimitive: XIII metadata.
    }
    if (4..19).contains(&licensee) {
        r.take(11)?; // UMesh: three bytes, a 32-bit word and a 32-bit colour.
        if licensee >= 6 {
            r.take(1)?;
        }
    }
    Ok(())
}

impl SkeletalMesh {
    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        Self::read_native_versioned(&data[object.end..export.serial_offset as usize + export.serial_size as usize], package.header.licensee)
    }

    /// Native part of the object (after the properties), up to the end of the export.
    pub fn read_native(data: &[u8]) -> Result<Self> {
        Self::read_native_versioned(data, 58)
    }

    fn read_native_versioned(data: &[u8], licensee: u16) -> Result<Self> {
        let mut r = Reader::new(data, 0);
        skip_mesh_prefix(&mut r, licensee)?;
        let version = r.u32()?;
        let _vertex_count = r.u32()?;
        let legacy_vertices = read_legacy_vertices(&mut r)?;
        let legacy_triangles = read_legacy_triangles(&mut r)?;
        let textures = (0..r.compact()?).map(|_| r.compact()).collect::<Result<Vec<_>>>()?;
        let scale = vec3(&mut r)?;
        let origin = vec3(&mut r)?;
        let rot_origin = [r.i32()?, r.i32()?, r.i32()?];
        let _collapse_points = u16s(&mut r)?;
        let _face_level = u16s(&mut r)?;
        let faces = read_faces(&mut r)?;
        let _collapse_wedges = u16s(&mut r)?;
        let wedges = read_wedges(&mut r)?;
        let materials = read_materials(&mut r)?;
        for _ in 0..3 {
            f32r(&mut r)?; // MeshScaleMax, LODHysteresis, LODStrength
        }
        r.i32()?; // LODMinVerts
        f32r(&mut r)?; // LODMorph
        f32r(&mut r)?; // LODZDisplace
        let points = (0..r.compact()?).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        let bones = (0..r.compact()?)
            .map(|_| {
                let name = r.compact()?;
                let flags = r.u32()?;
                let orientation = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
                let position = vec3(&mut r)?;
                r.take(16)?;
                Ok(Bone { name, flags, orientation, position, parent: r.i32()? })
            })
            .collect::<Result<Vec<_>>>()?;
        // Animation, SkeletalDepth, then the grouped influences: WeightIndices (compact n ×
        // [compact m × u16 point, i32 first index]) where group g has g+1 influences per point,
        // and BoneInfluences (compact n × [u16 weight / 65535, u16 bone]).
        let animation = r.compact()?;
        let _depth = r.i32()?;
        let groups = (0..r.compact()?).map(|_| Ok((u16s(&mut r)?, r.i32()?))).collect::<Result<Vec<_>>>()?;
        let raw = (0..r.compact()?).map(|_| Ok((r.u16()? as f32 * WEIGHT_SCALE, r.u16()?))).collect::<Result<Vec<_>>>()?;
        let mut influences = vec![Vec::new(); points.len()];
        for (g, (list, start)) in groups.iter().enumerate() {
            let per_point = g + 1;
            for (k, &point) in list.iter().enumerate() {
                let first = *start as usize + k * per_point;
                if let (Some(slot), Some(items)) = (influences.get_mut(point as usize), raw.get(first..first + per_point)) {
                    *slot = items.iter().map(|&(w, bone)| (bone, w)).collect();
                }
            }
        }
        let mesh = SkeletalMesh { version, legacy_vertices, legacy_triangles, textures, scale, origin, rot_origin, faces, wedges, materials, points, bones, animation, influences };
        mesh.validate()?;
        Ok(mesh)
    }

    fn validate(&self) -> Result<()> {
        let bad = |v: usize| openxiii_package::Error::BadIndex(v as i32);
        if let Some(w) = self.wedges.iter().find(|w| w.point as usize >= self.points.len()) {
            return Err(bad(w.point as usize));
        }
        for f in &self.faces {
            if let Some(&w) = f.wedges.iter().find(|&&w| w as usize >= self.wedges.len()) {
                return Err(bad(w as usize));
            }
        }
        if let Some(b) = self.bones.iter().find(|b| b.parent < 0 || b.parent as usize >= self.bones.len().max(1)) {
            return Err(openxiii_package::Error::BadIndex(b.parent));
        }
        Ok(())
    }

    /// Texture (object reference) of a face material.
    pub fn face_texture(&self, material: u16) -> i32 {
        let index = self.materials.get(material as usize).map_or(material as i32, |m| m.1);
        self.textures.get(index.max(0) as usize).copied().unwrap_or(0)
    }

    /// Position of a point in actor space: (P − MeshOrigin) × MeshScale, then RotOrigin.
    /// (Checked on the soldiers: they face their walking direction.)
    pub fn transformed_point(&self, i: usize) -> [f32; 3] {
        self.transform_point_with(&crate::level::rotation_matrix(self.rot_origin), self.points[i])
    }

    /// `transformed_point` of an arbitrary point `p` (a pose), with the matrix
    /// `rotation_matrix(self.rot_origin)` already computed.
    pub fn transform_point_with(&self, m: &[[f32; 3]; 3], p: [f32; 3]) -> [f32; 3] {
        let s = [0, 1, 2].map(|k| (p[k] - self.origin[k]) * self.scale[k]);
        [0, 1, 2].map(|k| s[0] * m[0][k] + s[1] * m[1][k] + s[2] * m[2][k])
    }

    /// OBJ export of the reference pose (Y up, like `StaticMesh::to_obj`).
    pub fn to_obj(&self, material_names: &dyn Fn(i32) -> String) -> String {
        let mut s = String::new();
        for i in 0..self.points.len() {
            let p = self.transformed_point(i);
            s += &format!("v {} {} {}\n", p[0], p[2], p[1]);
        }
        for w in &self.wedges {
            s += &format!("vt {} {}\n", w.uv[0], 1.0 - w.uv[1]);
        }
        let mut current = None;
        for f in &self.faces {
            let texture = self.face_texture(f.material);
            if current != Some(texture) {
                s += &format!("usemtl {}\n", material_names(texture));
                current = Some(texture);
            }
            let v = |w: u16| format!("{}/{}", self.wedges[w as usize].point as usize + 1, w as usize + 1);
            s += &format!("f {} {} {}\n", v(f.wedges[0]), v(f.wedges[2]), v(f.wedges[1]));
        }
        s
    }
}

// ---- Skeleton and skinning ----

/// Bone matrix for skinning: animated frame `s` applied after the reference inverse `r`.
fn bone_matrix(s: &Coords, r: &Coords) -> Coords {
    let axis = |a: [f32; 3]| {
        [a[1] * r.y[0] + a[0] * r.x[0] + a[2] * r.z[0], a[0] * r.x[1] + a[1] * r.y[1] + a[2] * r.z[1], a[0] * r.x[2] + a[1] * r.y[2] + a[2] * r.z[2]]
    };
    let o = r.origin;
    Coords {
        origin: [
            // ((z + y) + x) + origin.
            s.x[2] * o[2] + s.x[1] * o[1] + o[0] * s.x[0] + s.origin[0],
            s.y[2] * o[2] + s.y[1] * o[1] + s.y[0] * o[0] + s.origin[1],
            s.z[2] * o[2] + s.z[1] * o[1] + o[0] * s.z[0] + s.origin[2],
        ],
        x: axis(s.x),
        y: axis(s.y),
        z: axis(s.z),
    }
}

/// apply_matrix in the order used for multi-bone points: ((y + z) + x) + origin.
fn apply_matrix_yzx(m: &Coords, v: [f32; 3]) -> [f32; 3] {
    [
        m.x[1] * v[1] + m.x[2] * v[2] + m.x[0] * v[0] + m.origin[0],
        m.y[1] * v[1] + m.y[2] * v[2] + m.y[0] * v[0] + m.origin[1],
        m.z[1] * v[1] + m.z[2] * v[2] + m.z[0] * v[0] + m.origin[2],
    ]
}

type Quat = [f32; 4];

fn q_mul(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn q_conj(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}

fn q_norm(q: Quat) -> Quat {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 0.0 {
        q.map(|c| c / l)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn q_rotate(q: Quat, v: [f32; 3]) -> [f32; 3] {
    let p = q_mul(q_mul(q, [v[0], v[1], v[2], 0.0]), q_conj(q));
    [p[0], p[1], p[2]]
}

fn q_slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let mut d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let b = if d < 0.0 {
        d = -d;
        b.map(|c| -c)
    } else {
        b
    };
    if d > 0.9995 {
        return q_norm([0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * t));
    }
    let theta = d.dacos();
    let (its, sb) = (((1.0 - t) * theta).dsin() / theta.dsin(), (t * theta).dsin() / theta.dsin());
    [0, 1, 2, 3].map(|k| a[k] * its + b[k] * sb)
}

/// Bone transform: rotation then translation.
#[derive(Debug, Clone, Copy)]
struct Joint {
    rotation: Quat,
    position: [f32; 3],
}

impl Joint {
    fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        let r = q_rotate(self.rotation, v);
        [r[0] + self.position[0], r[1] + self.position[1], r[2] + self.position[2]]
    }
    fn inverse_apply(&self, v: [f32; 3]) -> [f32; 3] {
        q_rotate(q_conj(self.rotation), [v[0] - self.position[0], v[1] - self.position[1], v[2] - self.position[2]])
    }
    fn then(parent: &Joint, local: &Joint) -> Joint {
        Joint { rotation: q_norm(q_mul(parent.rotation, local.rotation)), position: parent.apply(local.position) }
    }
}

/// Unreal convention (PSK/PSA): the root keeps its quaternion, the other bones are conjugated.
fn local_joint(index: usize, rotation: Quat, position: [f32; 3]) -> Joint {
    Joint { rotation: q_norm(if index == 0 { rotation } else { q_conj(rotation) }), position }
}

impl SkeletalMesh {
    fn globals(&self, locals: &[Joint]) -> Vec<Joint> {
        let mut globals: Vec<Joint> = Vec::with_capacity(locals.len());
        for (i, b) in self.bones.iter().enumerate() {
            let g = if i == 0 || b.parent as usize >= i { locals[i] } else { Joint::then(&globals[b.parent as usize], &locals[i]) };
            globals.push(g);
        }
        globals
    }

    /// Points deformed by an animation sequence at frame `frame` (mesh space).
    pub fn pose(
        &self,
        anim: &crate::animation::MeshAnimation,
        anim_names: &dyn Fn(i32) -> String,
        bone_names: &dyn Fn(i32) -> String,
        sequence: usize,
        frame: f32,
    ) -> Vec<[f32; 3]> {
        let reference: Vec<Joint> = self.bones.iter().enumerate().map(|(i, b)| local_joint(i, b.orientation, b.position)).collect();
        let animated: Vec<Joint> =
            self.sample_locals(anim, anim_names, bone_names, sequence, frame).iter().enumerate().map(|(i, &(q, p))| local_joint(i, q, p)).collect();
        self.skin_joints(&reference, &animated)
    }

    /// Same pose through the engine's path: bone frames (quaternion → axes, composition with
    /// the parent) then skinning by RefBases. Used to check one against the other.
    pub fn pose_engine(
        &self,
        anim: &crate::animation::MeshAnimation,
        anim_names: &dyn Fn(i32) -> String,
        bone_names: &dyn Fn(i32) -> String,
        sequence: usize,
        frame: f32,
    ) -> Vec<[f32; 3]> {
        let locals = self.sample_locals(anim, anim_names, bone_names, sequence, frame);
        let mut bases: Vec<Coords> = locals.iter().map(|&(q, p)| coords::from_quat(q, p)).collect();
        for i in 1..bases.len() {
            let parent = self.bones[i].parent as usize;
            bases[i] = bases[i].compose_bone(&bases[parent]);
        }
        self.skin(&bases, &self.ref_bases())
    }

    /// Local rotation and position of each bone for a sequence (reference pose for
    /// non-animated bones), simple sampling (one track per bone, no blending).
    fn sample_locals(
        &self,
        anim: &crate::animation::MeshAnimation,
        anim_names: &dyn Fn(i32) -> String,
        bone_names: &dyn Fn(i32) -> String,
        sequence: usize,
        frame: f32,
    ) -> Vec<(Quat, [f32; 3])> {
        let mut animated: Vec<(Quat, [f32; 3])> = self.bones.iter().map(|b| (b.orientation, b.position)).collect();
        if let Some(m) = anim.moves.get(sequence) {
            // Mapping from animation bones to mesh bones, by name.
            let mesh_index: std::collections::HashMap<String, usize> =
                self.bones.iter().enumerate().map(|(i, b)| (bone_names(b.name).to_lowercase(), i)).collect();
            for (ti, track) in m.tracks.iter().enumerate() {
                let bone_index = m.bone_indices.get(ti).copied().unwrap_or(ti as i32) as usize;
                let anim_bone = anim.bones.get(bone_index).map(|b| anim_names(b.0).to_lowercase());
                let pos_start = track.pos_start as usize;
                let target = anim_bone.and_then(|n| mesh_index.get(&n).copied());
                let qs = track.quat_start as usize;
                let qn = track.quat_count as usize;
                let pn = track.pos_count as usize;
                if let (Some(i), true) = (target, qn > 0) {
                    let times = &m.times[qs..qs + qn];
                    let quats = &m.quats[qs..qs + qn];
                    let k = times.iter().rposition(|&t| t <= frame).unwrap_or(0);
                    let rotation = if k + 1 < qn && times[k + 1] > times[k] {
                        q_slerp(quats[k], quats[k + 1], (frame - times[k]) / (times[k + 1] - times[k]))
                    } else {
                        quats[k]
                    };
                    let positions = &m.positions[pos_start..pos_start + pn];
                    let position = if pn == qn && pn > 1 {
                        let a = positions[k];
                        let b = positions[(k + 1).min(pn - 1)];
                        let t = if k + 1 < qn && times[k + 1] > times[k] { (frame - times[k]) / (times[k + 1] - times[k]) } else { 0.0 };
                        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t)
                    } else if pn >= 1 {
                        positions[0]
                    } else {
                        self.bones[i].position
                    };
                    animated[i] = (rotation, position);
                }
            }
        }
        animated
    }

    fn skin_joints(&self, reference: &[Joint], animated: &[Joint]) -> Vec<[f32; 3]> {
        let (ref_g, anim_g) = (self.globals(reference), self.globals(animated));
        self.points
            .iter()
            .enumerate()
            .map(|(p, &v)| {
                let inf = &self.influences[p];
                if inf.is_empty() {
                    return v;
                }
                let mut out = [0.0f32; 3];
                let mut total = 0.0;
                for &(bone, w) in inf {
                    let b = bone as usize;
                    if b >= ref_g.len() {
                        continue;
                    }
                    let local = ref_g[b].inverse_apply(v);
                    let moved = anim_g[b].apply(local);
                    for c in 0..3 {
                        out[c] += moved[c] * w;
                    }
                    total += w;
                }
                if total > 0.0 {
                    out.map(|c| c / total)
                } else {
                    v
                }
            })
            .collect()
    }

    /// Bone frames in the reference pose (GetFrame: quaternion and position of each bone,
    /// composed with the parent).
    pub fn ref_space_bases(&self) -> Vec<Coords> {
        let mut bases: Vec<Coords> = self.bones.iter().map(|b| coords::from_quat(b.orientation, b.position)).collect();
        for i in 1..bases.len() {
            let parent = self.bones[i].parent as usize;
            bases[i] = bases[i].compose_bone(&bases[parent]);
        }
        bases
    }

    /// The mesh's RefBases (+0x16C): inverses of the reference frames.
    pub fn ref_bases(&self) -> Vec<Coords> {
        self.ref_space_bases().iter().map(|b| b.inverse()).collect()
    }

    /// Engine skinning: each bone combines its animated frame and the inverse of its
    /// reference frame, each point is the weighted sum of the transformed points (weights
    /// u16 / 65535, not normalised; a single bone: no weight).
    pub fn skin(&self, bases: &[Coords], ref_bases: &[Coords]) -> Vec<[f32; 3]> {
        self.skin_matrices(&Self::bone_matrices(bases, ref_bases))
    }

    /// `skin` and `skin_normals` of the same pose, with the bone matrices computed only once.
    pub fn skin_with_normals(&self, bases: &[Coords], ref_bases: &[Coords], normals: &[[f32; 3]]) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let matrices = Self::bone_matrices(bases, ref_bases);
        (self.skin_matrices(&matrices), self.skin_normals_matrices(&matrices, normals))
    }

    fn bone_matrices(bases: &[Coords], ref_bases: &[Coords]) -> Vec<Coords> {
        bases.iter().zip(ref_bases).map(|(s, r)| bone_matrix(s, r)).collect()
    }

    fn skin_matrices(&self, matrices: &[Coords]) -> Vec<[f32; 3]> {
        self.points
            .iter()
            .enumerate()
            .map(|(p, &v)| {
                let inf = &self.influences[p];
                // One bone as for several: ((Y·y + Z·z) + X·x) + origin.
                let apply = |b: u16| matrices.get(b as usize).map(|m| apply_matrix_yzx(m, v));
                match inf.len() {
                    0 => v,
                    1 => apply(inf[0].0).unwrap_or(v),
                    _ => {
                        // Several bones (and following): × weight, added to the sum.
                        let mut out = [0.0f32; 3];
                        for (k, &(bone, w)) in inf.iter().enumerate() {
                            let Some(t) = apply(bone) else { continue };
                            for c in 0..3 {
                                out[c] = if k == 0 { t[c] * w } else { t[c] * w + out[c] };
                            }
                        }
                        out
                    }
                }
            })
            .collect()
    }

    /// USkeletalMesh::CalculateNormals, called once by RenderPreProcess
    /// normal of each point in the reference pose (mesh space).
    /// Each face adds (P0 − P1) × (P2 − P0), not normalised, to its three points; each
    /// sum is then divided by √(x² + y² + z² + 0.001).
    pub fn calculate_normals(&self) -> Vec<[f32; 3]> {
        let mut acc = vec![[0.0f32; 3]; self.points.len()];
        for f in &self.faces {
            let i = f.wedges.map(|w| self.wedges.get(w as usize).map_or(0, |x| x.point as usize));
            let (Some(&p0), Some(&p1), Some(&p2)) = (self.points.get(i[0]), self.points.get(i[1]), self.points.get(i[2])) else { continue };
            let d20 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
            let d01 = [p0[0] - p1[0], p0[1] - p1[1], p0[2] - p1[2]];
            let n = [d01[1] * d20[2] - d01[2] * d20[1], d01[2] * d20[0] - d20[2] * d01[0], d20[1] * d01[0] - d01[1] * d20[0]];
            for &p in &i {
                for k in 0..3 {
                    acc[p][k] += n[k];
                }
            }
        }
        acc.iter()
            .map(|n| {
                // ((x² + y²) + z²) + 0.001, sqrt (fsqrt, rounded to 24 bits by the
                // precision control), then 1 / root and the products.
                let l = (((n[0] * n[0] + n[1] * n[1]) + n[2] * n[2]) + 0.001f32) as f64;
                let inv = 1.0 / (l.sqrt() as f32);
                [inv * n[0], inv * n[1], inv * n[2]]
            })
            .collect()
    }

    /// Pose normals: for each point, the sum of the reference normals
    /// rotated by each of its bones, without weights (only positions are
    /// weighted) and without normalisation.
    pub fn skin_normals(&self, bases: &[Coords], ref_bases: &[Coords], normals: &[[f32; 3]]) -> Vec<[f32; 3]> {
        self.skin_normals_matrices(&Self::bone_matrices(bases, ref_bases), normals)
    }

    fn skin_normals_matrices(&self, matrices: &[Coords], normals: &[[f32; 3]]) -> Vec<[f32; 3]> {
        normals
            .iter()
            .enumerate()
            .map(|(p, &n)| {
                let inf = self.influences.get(p).map(|v| v.as_slice()).unwrap_or(&[]);
                let mut out: Option<[f32; 3]> = None;
                for &(bone, _) in inf {
                    let Some(m) = matrices.get(bone as usize) else { continue };
                    // ((Y·y + Z·z) + X·x), without origin or weight.
                    let r = [m.x[1] * n[1] + m.x[2] * n[2] + m.x[0] * n[0], m.y[1] * n[1] + m.y[2] * n[2] + m.y[0] * n[0], m.z[1] * n[1] + m.z[2] * n[2] + m.z[0] * n[0]];
                    out = Some(match out {
                        None => r,
                        Some(o) => [r[0] + o[0], r[1] + o[1], r[2] + o[2]],
                    });
                }
                out.unwrap_or(n)
            })
            .collect()
    }

    /// Normal from mesh space to actor space (MeshToWorld without translation):
    /// inverse transpose of the mesh scale then RotOrigin, like Direct3D's normal
    /// transformation.
    pub fn transformed_normal(&self, n: [f32; 3]) -> [f32; 3] {
        self.transform_normal_with(&crate::level::rotation_matrix(self.rot_origin), n)
    }

    /// `transformed_normal` with the matrix `rotation_matrix(self.rot_origin)` already computed.
    pub fn transform_normal_with(&self, m: &[[f32; 3]; 3], n: [f32; 3]) -> [f32; 3] {
        let s = [0, 1, 2].map(|k| if self.scale[k] != 0.0 { n[k] / self.scale[k] } else { n[k] });
        [0, 1, 2].map(|k| s[0] * m[0][k] + s[1] * m[1][k] + s[2] * m[2][k])
    }

    /// Replaces the points with an animated pose (for export or display).
    pub fn with_points(&self, points: Vec<[f32; 3]>) -> SkeletalMesh {
        let mut m = self.clone();
        m.points = points;
        m
    }
}

