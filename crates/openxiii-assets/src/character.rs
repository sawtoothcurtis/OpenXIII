//! Character model ready to animate: skeletal mesh, default animation and
//! faces split by material. Produces the vertices of a pose every frame.

use crate::animation::MeshAnimation;
use crate::level::MeshActor;
use crate::skeletal::SkeletalMesh;
use crate::world::Vertex;
use openxiii_package::library::Library;
use openxiii_package::ObjectRef;
use std::collections::HashMap;

pub struct CharacterModel {
    pub mesh: SkeletalMesh,
    anim: Option<MeshAnimation>,
    /// Name table of the animation's package (bone and sequence names).
    anim_names: Vec<String>,
    mesh_bone_names: HashMap<i32, String>,
    /// (material, local wedge indices): one batch per mesh texture.
    pub batches: Vec<(String, Vec<u32>)>,
    /// TextureIndex (FMeshMaterial) of each batch, passed to USkeletalMeshInstance::GetTexture.
    pub batch_textures: Vec<i32>,
    /// USkeletalMesh::Textures (+0x74), qualified paths.
    pub textures: Vec<String>,
    /// The mesh's RefBases (inverses of the reference frames), for skinning.
    ref_bases: Vec<crate::coords::Coords>,
    /// Reference pose normals (USkeletalMesh::CalculateNormals), per point.
    ref_normals: Vec<[f32; 3]>,
    /// First-person mesh that reaches the eye (`REACHES_EYE`).
    pub reaches_eye: bool,
}

/// First-person meshes whose rear end goes past the camera's near plane, up to the eye, and the
/// material (texture) of that part: the bazooka's tube rests on the shoulder, so its last third
/// is cut by the plane at 5 units. In 4:3 the cut is outside the picture; a wider window shows it
/// as a black hole (the inside of the tube's outline shell) on the right. When the view is widened
/// past 4:3, the renderer draws the model with a closer near plane and keeps its other parts (the
/// hands) cut at the usual one; in 4:3 nothing changes.
pub const REACHES_EYE: &[(&str, &str)] = &[("XIIIArmes.FpsBazookaM", "XIIIMeshArmes.bazooka")];

fn reaches_eye(mesh_path: &str) -> bool {
    REACHES_EYE.iter().any(|(m, _)| m.eq_ignore_ascii_case(mesh_path))
}

/// Does a mesh built from a game mesh (`[arms.<Name>]`, mod_arms.rs: "Mod.Jones.FpsBazookaM")
/// copy a `REACHES_EYE` mesh? Its object name is that of the mesh it copies.
#[cfg(feature = "mods")]
fn copies_reaches_eye(mesh_path: &str) -> bool {
    let name = |p: &str| p.rsplit('.').next().unwrap_or(p).to_string();
    REACHES_EYE.iter().any(|(m, _)| name(m).eq_ignore_ascii_case(&name(mesh_path)))
}

/// Is `material` the part of a `REACHES_EYE` mesh that reaches the eye?
pub fn reaches_eye_material(material: &str) -> bool {
    REACHES_EYE.iter().any(|(_, t)| t.eq_ignore_ascii_case(material))
}

fn qualified(package_name: &str, package: &openxiii_package::Package, r: i32) -> String {
    match ObjectRef::from(r) {
        ObjectRef::None => String::new(),
        ObjectRef::Export(_) => format!("{package_name}.{}", package.object_path(r.into())),
        _ => package.object_path(r.into()),
    }
}

impl CharacterModel {
    pub fn load(library: &mut Library, mesh_path: &str) -> Option<Self> {
        // Mods build only: a mod's animated model.
        #[cfg(feature = "mods")]
        if let Some(m) = crate::mod_skeletal::get(mesh_path) {
            // Mod arms are built from the game's first-person meshes and keep their shape: the
            // co-op characters' arms ("Mod.Jones.FpsBazookaM", coop_looks.rs) reach the eye as
            // the mesh they copy.
            let reaches_eye = reaches_eye(mesh_path) || (m.game_look && copies_reaches_eye(mesh_path));
            return Some(CharacterModel { reaches_eye, ..Self::from_mod(&m) });
        }
        let (mesh, anim_path, mesh_bone_names, batches, batch_textures, textures) = {
            let (loaded, index) = library.resolve(mesh_path)?;
            if loaded.package.export_class(index) != "SkeletalMesh" {
                return None;
            }
            let mesh = SkeletalMesh::read(&loaded.package, &loaded.data, index).ok()?;
            let anim_path = (mesh.animation != 0).then(|| qualified(&loaded.name, &loaded.package, mesh.animation));
            let names = mesh.bones.iter().map(|b| (b.name, loaded.package.name(b.name).to_string())).collect();
            // Faces grouped by the TextureIndex of their FMeshMaterial.
            let mut groups: std::collections::BTreeMap<i32, Vec<u32>> = std::collections::BTreeMap::new();
            for f in &mesh.faces {
                let index = mesh.materials.get(f.material as usize).map_or(f.material as i32, |m| m.1);
                groups.entry(index).or_default().extend(f.wedges.map(|w| w as u32));
            }
            let textures: Vec<String> = mesh.textures.iter().map(|&t| qualified(&loaded.name, &loaded.package, t)).collect();
            let mut batches = Vec::new();
            let mut batch_textures = Vec::new();
            for (index, wedges) in groups {
                batches.push((textures.get(index.max(0) as usize).cloned().unwrap_or_default(), wedges));
                batch_textures.push(index);
            }
            (mesh, anim_path, names, batches, batch_textures, textures)
        };
        let (anim, anim_names) = match anim_path.and_then(|p| library.resolve(&p)) {
            Some((loaded, index)) => {
                (MeshAnimation::read(&loaded.package, &loaded.data, index).ok(), loaded.package.names.iter().map(|n| n.name.clone()).collect())
            }
            None => (None, Vec::new()),
        };
        let ref_bases = mesh.ref_bases();
        let ref_normals = mesh.calculate_normals();
        let reaches_eye = reaches_eye(mesh_path);
        Some(CharacterModel { mesh, anim, anim_names, mesh_bone_names, batches, batch_textures, textures, ref_bases, ref_normals, reaches_eye })
    }

    /// Mods build only: the model of a mod's animated mesh (its animation is played through
    /// the engine's frames, `vertices_bases`; `vertices` draws its reference pose).
    #[cfg(feature = "mods")]
    fn from_mod(m: &crate::mod_skeletal::ModSkeletal) -> Self {
        // With its outline shell batches (mod_skeletal::drawn); batch_textures gives each batch
        // its part, so that the actor's Skins apply to a shell as to its part.
        let (mesh, drawn, ref_normals) = crate::mod_skeletal::drawn(m);
        let mut batches = Vec::new();
        let mut batch_textures = Vec::new();
        for (material, wedges, part) in drawn {
            batches.push((material, wedges));
            batch_textures.push(part);
        }
        let mesh_bone_names = m.bone_names.iter().enumerate().map(|(i, n)| (i as i32, n.clone())).collect();
        let ref_bases = mesh.ref_bases();
        CharacterModel { mesh, anim: None, anim_names: Vec::new(), mesh_bone_names, batches, batch_textures, textures: m.textures.clone(), ref_bases, ref_normals, reaches_eye: false }
    }

    pub fn vertex_count(&self) -> usize {
        self.mesh.wedges.len()
    }

    fn sequence(&self, name: &str) -> Option<usize> {
        let anim = self.anim.as_ref()?;
        anim.sequences.iter().position(|s| self.anim_names.get(s.name as usize).is_some_and(|n| n.eq_ignore_ascii_case(name)))
    }

    /// Vertices (one per wedge) for sequence `pose` at the given frame (0…frames, fractional),
    /// placed by `actor`.
    pub fn vertices(&self, pose: Option<(&str, f32)>, actor: &MeshActor) -> Vec<Vertex> {
        let points = match (pose, &self.anim) {
            (Some((name, frame)), Some(anim)) => match self.sequence(name) {
                Some(seq) => {
                    let info = &anim.sequences[seq];
                    let frame = frame.rem_euclid(info.num_frames.max(1) as f32);
                    let names = |n: i32| self.anim_names.get(n as usize).cloned().unwrap_or_default();
                    let bones = |n: i32| self.mesh_bone_names.get(&n).cloned().unwrap_or_default();
                    self.mesh.pose(anim, &names, &bones, seq, frame)
                }
                None => self.mesh.points.clone(),
            },
            _ => self.mesh.points.clone(),
        };
        self.place(points, actor)
    }

    /// Mesh points in pose `bases`, without world placement.
    pub fn skin_points(&self, bases: &[crate::coords::Coords]) -> Vec<[f32; 3]> {
        self.mesh.skin(bases, &self.ref_bases)
    }

    /// Vertices for the bone frames computed by the engine (GetFrame), placed by `actor`.
    /// Normals: the reference ones rotated by the bones, as the engine
    /// passes them to the renderer, then brought into world space and normalised.
    pub fn vertices_bases(&self, bases: &[crate::coords::Coords], actor: &MeshActor) -> Vec<Vertex> {
        // Bone matrices computed once for points and normals.
        let (points, normals) = self.mesh.skin_with_normals(bases, &self.ref_bases, &self.ref_normals);
        // The pose only changes the points: no copy of the whole mesh (with_points), and
        // the two rotation matrices computed once.
        let mesh = &self.mesh;
        let o = crate::level::rotation_matrix(mesh.rot_origin);
        let r = crate::level::rotation_matrix(actor.rotation);
        let positions: Vec<[f32; 3]> = points.iter().map(|&p| actor.transform_with(&r, mesh.transform_point_with(&o, p))).collect();
        let unit = |n: [f32; 3]| {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
            [n[0] / l, n[1] / l, n[2] / l]
        };
        // Normal brought into world space and normalised once per point (same operations as
        // per wedge: several wedges share a point).
        #[allow(unused_mut)]
        let mut normals: Vec<[f32; 3]> = normals.iter().map(|&n| unit(actor.transform_normal_with(&r, mesh.transform_normal_with(&o, n)))).collect();
        // Mods build only: the points held only by bones a mod hid (FirstPersonHideBones, shrunk
        // to HIDDEN_BONE_SCALE) get a zero normal, so that the outline pass does not push them
        // out (d3d::main_shader): shrunk to a point, they would still show as a black dot of the
        // outline's thickness. Game bones are never at that exact scale.
        #[cfg(feature = "mods")]
        if let Some(hidden) = crate::mod_skeletal::hidden_bones(bases) {
            for (p, n) in normals.iter_mut().enumerate() {
                let inf = mesh.influences.get(p).map(|v| v.as_slice()).unwrap_or(&[]);
                if !inf.is_empty() && inf.iter().all(|&(b, _)| hidden.get(b as usize).copied().unwrap_or(false)) {
                    *n = [0.0; 3];
                }
            }
        }
        let fallback = unit([0.0, 0.0, 1.0]);
        mesh.wedges
            .iter()
            .map(|w| {
                let p = w.point as usize;
                Vertex { position: positions[p], normal: normals.get(p).copied().unwrap_or(fallback), uv: w.uv }
            })
            .collect()
    }

    fn place(&self, points: Vec<[f32; 3]>, actor: &MeshActor) -> Vec<Vertex> {
        let posed = &self.mesh;
        let o = crate::level::rotation_matrix(posed.rot_origin);
        let r = crate::level::rotation_matrix(actor.rotation);
        let positions: Vec<[f32; 3]> = points.iter().map(|&p| actor.transform_with(&r, posed.transform_point_with(&o, p))).collect();
        let mut normals = vec![[0.0f32; 3]; positions.len()];
        for f in &posed.faces {
            let [a, b, c] = f.wedges.map(|w| posed.wedges[w as usize].point as usize);
            let (pa, pb, pc) = (positions[a], positions[b], positions[c]);
            let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
            // Outward normal: front faces are clockwise in a left-handed frame, i.e.
            // e2 × e1 (DrawCartoonSM's cartoon outline is pushed along this normal).
            let n = [e2[1] * e1[2] - e2[2] * e1[1], e2[2] * e1[0] - e2[0] * e1[2], e2[0] * e1[1] - e2[1] * e1[0]];
            for i in [a, b, c] {
                for k in 0..3 {
                    normals[i][k] += n[k];
                }
            }
        }
        posed
            .wedges
            .iter()
            .map(|w| {
                let p = w.point as usize;
                let n = normals[p];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                Vertex { position: positions[p], normal: [n[0] / l, n[1] / l, n[2] / l], uv: w.uv }
            })
            .collect()
    }
}

/// Static mesh object moved by the game (door, lift…).
pub struct PropModel {
    pub mesh: crate::static_mesh::StaticMesh,
    pub batches: Vec<(String, Vec<u32>)>,
    /// Sections whose material (UStaticMesh.Materials) is a USinusModifier:
    /// draws them through SetSinusVertexStreams whatever the actor (Movers included), like the
    /// StaticMeshes of the static geometry (world.rs). Their vertices are copied after the
    /// mesh's (`copies`) so they can be waved without touching neighbouring sections.
    pub sine: Vec<PropSine>,
    /// Vertex of the original mesh for each copied vertex (indices `positions.len()..`).
    copies: Vec<u32>,
    /// X fade bounds (when the StaticMesh is loaded).
    sine_fade: [f32; 2],
    /// Index of the first occurrence of each distinct position of the mesh (`bounds_list`).
    uniques: Vec<u32>,
}

/// SinusModifier section of a PropModel.
pub struct PropSine {
    /// Path of the modifier, lowercase (key of Vm::sine_drawn).
    pub modifier: String,
    params: crate::sine::SineParams,
    /// Index of the section's first copied vertex among the model's vertices.
    first: usize,
    /// Local positions of its copied vertices.
    local: Vec<[f32; 3]>,
}

impl PropModel {
    pub fn load(library: &mut Library, mesh_path: &str) -> Option<Self> {
        let (mesh, materials) = match crate::mod_mesh(mesh_path) {
            Some(m) => m,
            None => {
                let (loaded, index) = library.resolve(mesh_path)?;
                let mesh = crate::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
                let materials: Vec<String> =
                    (0..mesh.sections.len()).map(|s| mesh.materials.get(s).map(|&m| qualified(&loaded.name, &loaded.package, m)).unwrap_or_default()).collect();
                (mesh, materials)
            }
        };
        let mut props = crate::props::PropCache::new();
        let params: Vec<Option<crate::sine::SineParams>> = materials
            .iter()
            .map(|m| if m.is_empty() { None } else { props.object(library, m).filter(|p| p.is_a("SinusModifier")).map(crate::sine::SineParams::read) })
            .collect();
        let mut batches = Vec::new();
        let (mut sine, mut copies) = (Vec::new(), Vec::new());
        let mut per_material: Vec<(u8, Vec<[f32; 3]>)> = Vec::new();
        for (s, section) in mesh.sections.iter().enumerate() {
            let triangles = mesh.section_triangles(section);
            let indices: Vec<u32> = match &params[s] {
                Some(p) => {
                    per_material.push((p.fade, triangles.iter().flatten().map(|&i| mesh.positions[i as usize]).collect()));
                    let first = mesh.positions.len() + copies.len();
                    let mut copy: HashMap<u16, u32> = HashMap::new();
                    let mut local = Vec::new();
                    let indices = triangles
                        .iter()
                        .flatten()
                        .map(|&i| {
                            *copy.entry(i).or_insert_with(|| {
                                local.push(mesh.positions[i as usize]);
                                copies.push(i as u32);
                                (first + local.len() - 1) as u32
                            })
                        })
                        .collect();
                    sine.push(PropSine { modifier: materials[s].to_ascii_lowercase(), params: p.clone(), first, local });
                    indices
                }
                None => triangles.into_iter().flat_map(|t| t.map(|i| i as u32)).collect(),
            };
            batches.push((materials[s].clone(), indices));
        }
        let sine_fade = crate::sine::fade_bounds(&per_material, [0.0; 2]);
        // First occurrence of each position (bits): copies and vertices split by
        // their normals or texture coordinates do not change the box.
        let mut views_list = std::collections::HashSet::new();
        let uniques = (0..mesh.positions.len() as u32).filter(|&i| views_list.insert(mesh.positions[i as usize].map(f32::to_bits))).collect();
        Some(PropModel { mesh, batches, sine, copies, sine_fade, uniques })
    }

    /// End for this model: `drawn` gives, in the VM's drawing order,
    /// (modifier, CurrentTime, amplitude) of each SetSinusVertexStreams of the actor
    /// (Vm::sine_drawn); sections with the same modifier are taken in order. The
    /// waved positions are placed like `vertices` and written into `out`.
    pub fn wave(&self, actor: &MeshActor, drawn: &[(&str, f32, [f32; 3])], out: &mut [Vertex]) {
        let m = crate::level::rotation_matrix(actor.rotation);
        let mut used = vec![false; self.sine.len()];
        for &(key, t, amp) in drawn {
            let Some(i) = (0..self.sine.len()).find(|&i| !used[i] && self.sine[i].modifier == key) else { continue };
            used[i] = true;
            let s = &self.sine[i];
            let local = crate::sine::wave(&s.params, t, amp, self.sine_fade, &s.local);
            for (v, p) in out[s.first..].iter_mut().zip(&local) {
                v.position = actor.transform_with(&m, *p);
            }
        }
    }

    pub fn vertex_count(&self) -> usize {
        self.mesh.positions.len() + self.copies.len()
    }

    /// Positions and normals of `vertices` (same computations, same order), without texture
    /// coordinates or an intermediate array: the geometry of moving props, rebuilt every frame.
    /// Box (minimum, maximum) of the positions of `positions_normals`: same positions, taken
    /// in the same order, without keeping them or computing normals.
    pub fn bounds_list(&self, actor: &MeshActor) -> ([f32; 3], [f32; 3]) {
        let m = crate::level::rotation_matrix(actor.rotation);
        let bounds_list = |ranks: &mut dyn Iterator<Item = usize>| {
            let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
            for i in ranks {
                let position = actor.transform_with(&m, self.mesh.positions[i]);
                for k in 0..3 {
                    min[k] = min[k].min(position[k]);
                    max[k] = max[k].max(position[k]);
                }
            }
            (min, max)
        };
        // A repeated position gives the same bits: the box of distinct positions is the
        // same, unless an extremum is zero (0 and -0, equal, may then depend on order).
        let (min, max) = bounds_list(&mut self.uniques.iter().map(|&i| i as usize));
        if min.iter().chain(&max).all(|&v| v != 0.0) {
            return (min, max);
        }
        bounds_list(&mut (0..self.mesh.positions.len()).chain(self.copies.iter().map(|&i| i as usize)))
    }

    pub fn positions_normals(&self, actor: &MeshActor, positions: &mut Vec<[f32; 3]>, normals_list: &mut Vec<[f32; 3]>) {
        let m = crate::level::rotation_matrix(actor.rotation);
        positions.clear();
        normals_list.clear();
        positions.reserve(self.vertex_count());
        normals_list.reserve(self.vertex_count());
        for i in (0..self.mesh.positions.len()).chain(self.copies.iter().map(|&i| i as usize)) {
            let p = self.mesh.positions[i];
            let n = self.mesh.normals[i];
            normals_list.push([0, 1, 2].map(|k| n[0] * m[0][k] + n[1] * m[1][k] + n[2] * m[2][k]));
            positions.push(actor.transform_with(&m, p));
        }
    }

    pub fn vertices(&self, actor: &MeshActor) -> Vec<Vertex> {
        let m = crate::level::rotation_matrix(actor.rotation);
        (0..self.mesh.positions.len())
            .chain(self.copies.iter().map(|&i| i as usize))
            .map(|i| {
                let p = self.mesh.positions[i];
                let n = self.mesh.normals[i];
                let n = [0, 1, 2].map(|k| n[0] * m[0][k] + n[1] * m[1][k] + n[2] * m[2][k]);
                let uv = self.mesh.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]);
                Vertex { position: actor.transform_with(&m, p), normal: n, uv }
            })
            .collect()
    }
}

#[cfg(all(test, feature = "mods"))]
#[test]
fn coop_arms_reach_the_eye_as_the_mesh_they_copy() {
    assert!(copies_reaches_eye("Mod.Jones.FpsBazookaM"));
    assert!(copies_reaches_eye("Mod.SheridanGrey.fpsbazookam"));
    assert!(!copies_reaches_eye("Mod.Jones.FpsBerrettaM"));
    assert!(!copies_reaches_eye("Mod.Jones.FpsBazookaMX"));
}
