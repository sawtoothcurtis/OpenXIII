//! StaticMesh collision as in UStaticMesh::LineCheck / PointCheck.
//! An actor whose primitive is a StaticMesh (AActor::GetPrimitive: Mesh, otherwise
//! StaticMesh, otherwise GEngine->Cylinder; a Mover also goes through here) is an octree actor
//! like the others: Hit.Actor is the actor, plus the LevelInfo. Its shape (`SmShape`) is made
//! of the mesh's collision sets and the actor's frame (LocalToWorld:
//! Rotation, DrawScale3D, Location, PrePivot, DrawScale; WorldToLocal), rebuilt when
//! the actor moves.
//! - Triangle set: simplified collision if its tree exists and the mesh asks for it, bit 0
//!   of +0x188 (UseSimpleLineCollision) for traces without extent, bit 1
//!   (UseSimpleBoxCollision) for traces with extent and PointCheck; TRACE_ShadowCast
//!   (0x100) always takes the full triangles. bUseCylinderCollision: the cylinder.
//! - Traversal of the chosen set's kDOP tree (`kdop.rs`); without a node, no contact.
//! - After the contact: Time = Clamp(Time − Clamp(0.1; 0.1 / Dist; 1 / Dist), 0, 1),
//!   Location = Start + (End − Start) · Time, normal normalized then flipped if
//!   it follows the path. PointCheck normalizes the normal.

use crate::NativeField as Field;
use crate::kdop::{self, Bx6, Frame, KdopSet};
use crate::physics::{CheckResult, TRACE_SHADOW_CAST, V};
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_package::ObjectRef;
use std::sync::Arc;

thread_local! {
    pub(crate) static CHECK_SHAPES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Collision of a StaticMesh (mesh frame): full and simplified sets with the material label
/// of each triangle, flags +0x188, BoundingBox (root of the tree).
pub(crate) struct LocalSets {
    full: KdopSet,
    simple: KdopSet,
    use_simple_line: bool,
    use_simple_box: bool,
    root: Bx6,
}

/// Collision shape of a StaticMesh actor: its mesh and its frame.
pub struct SmShape {
    pub(crate) sets: Arc<LocalSets>,
    pub(crate) frame: Frame,
    /// Mesh and frame that were used (the shape is rebuilt when they change).
    key: (String, [u32; 13]),
    /// Previous frame of the same mesh: CheckEncroachment sets the tested position then puts back
    /// the old one, and the frame (a function of the inputs alone) is then reused as is.
    prev: Option<(Frame, [u32; 13])>,
}

/// Primitive of an actor (GetPrimitive): AActor, ABrush (Brush, otherwise
/// the cylinder; AMover takes AActor's), AEmitter (GEngine->CylinderNS).
pub(crate) enum PrimKind {
    Model,
    Mesh,
    StaticMesh(String),
    Cylinder,
    CylinderNs,
}

impl Vm {
    /// GetPrimitive of the actor.
    pub(crate) fn prim_kind(&self, a: ObjId) -> PrimKind {
        if self.is(a, crate::flags_classes::D::Emitter) {
            return PrimKind::CylinderNs;
        }
        if self.is(a, crate::flags_classes::D::Brush) && !self.is(a, crate::flags_classes::D::Mover) {
            return if matches!(self.propr(a, "Brush"), Value::Object(None) | Value::Void) { PrimKind::Cylinder } else { PrimKind::Model };
        }
        if !matches!(self.native_prop(a, Field::Mesh), Value::Object(None) | Value::Void) {
            return PrimKind::Mesh;
        }
        match self.native_prop(a, Field::StaticMesh) {
            Value::Asset(path) => PrimKind::StaticMesh(path.clone()),
            _ => PrimKind::Cylinder,
        }
    }

    /// Path of the StaticMesh if `prim_kind` is StaticMesh, without copying.
    pub(crate) fn prim_static_mesh(&self, a: ObjId) -> Option<&str> {
        if self.is(a, crate::flags_classes::D::Emitter) || (self.is(a, crate::flags_classes::D::Brush) && !self.is(a, crate::flags_classes::D::Mover)) || !matches!(self.native_prop(a, Field::Mesh), Value::Object(None) | Value::Void) {
            return None;
        }
        match self.native_prop(a, Field::StaticMesh) {
            Value::Asset(path) => Some(path.as_str()),
            _ => None,
        }
    }

    /// Materials of a map's fixed geometry (`build_world`) added to `collision_materials`
    /// without changing their indices: the triangles of the StaticMeshes already read (sm_local, kept from one
    /// map to the next) refer to them. Returns the labels of the world triangles in this list.
    pub fn adopt_world_materials(&mut self, materials: &[String], tags: &[u32]) -> Vec<u32> {
        let remap: Vec<u32> = materials.iter().map(|m| self.material_tag(m)).collect();
        tags.iter().map(|&t| remap.get(t as usize).copied().unwrap_or(0)).collect()
    }

    /// Label (index in `collision_materials`) of a material name.
    pub(crate) fn material_tag(&mut self, m: &str) -> u32 {
        if self.collision_materials.is_empty() {
            self.collision_materials.push(String::new());
        }
        match self.collision_materials.iter().position(|x| x == m) {
            Some(i) => i as u32,
            None => {
                self.collision_materials.push(m.to_string());
                (self.collision_materials.len() - 1) as u32
            }
        }
    }

    fn read_mesh_collision(&mut self, path: &str) -> Option<LocalSets> {
        // Mods build only: a mod's own 3D model collides with its own triangles.
        #[cfg(feature = "mods")]
        if let Some(m) = openxiii_assets::mod_meshes::get(path) {
            return self.mod_mesh_collision(&m);
        }
        // Triangle without a material (index < 0): the mesh's SimplifiedColMaterial (UStaticMesh
        // + 0x178).
        let simplified = match self.asset_prop(path, "SimplifiedColMaterial") {
            Some(crate::value::Value::Asset(m)) => m,
            _ => String::new(),
        };
        let (full, simple, use_simple_line, use_simple_box) = {
            let lib = self.library.as_mut()?;
            let (loaded, index) = lib.resolve(path)?;
            if loaded.package.export_class(index) != "StaticMesh" {
                return None;
            }
            let mesh = openxiii_assets::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
            let col = openxiii_assets::static_collision::read(&loaded.package, &loaded.data, index).ok()?;
            let qualified = |m: i32| match ObjectRef::from(m) {
                ObjectRef::None => String::new(),
                ObjectRef::Export(_) => format!("{}.{}", loaded.name, loaded.package.object_path(m.into())),
                _ => loaded.package.object_path(m.into()),
            };
            // Otherwise Materials[i].Material, or Engine.DefaultTexture if it is empty.
            let material = |i: u16| {
                if (i as i16) < 0 {
                    return simplified.clone();
                }
                match mesh.materials.get(i as usize) {
                    Some(&m) if !matches!(ObjectRef::from(m), ObjectRef::None) => qualified(m),
                    Some(_) => "Engine.DefaultTexture".to_string(),
                    None => String::new(),
                }
            };
            let names = |set: &openxiii_assets::static_collision::CollisionSet| -> Vec<String> { set.triangles.iter().map(|t| material(t[3])).collect() };
            let (nf, ns) = (names(&col.full), names(&col.simple));
            ((col.full, nf), (col.simple, ns), col.use_simple_line, col.use_simple_box)
        };
        let (lo, hi) = self.primitive_box(path)?;
        let mut set = |(c, names): (openxiii_assets::static_collision::CollisionSet, Vec<String>)| {
            let tags = names.iter().map(|m| self.material_tag(m)).collect();
            KdopSet::new(c.vertices, c.triangles, c.nodes, tags)
        };
        let full = set(full);
        let simple = set(simple);
        Some(LocalSets { full, simple, use_simple_line, use_simple_box, root: [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]] })
    }

    /// Mods build only: collision of a mod's own 3D model (the full set, no simplified set), as
    /// its `collision` asks: its triangles (each one labelled with its part's material), the
    /// 12 triangles of its box (labelled with the first part's material), or nothing.
    #[cfg(feature = "mods")]
    fn mod_mesh_collision(&mut self, m: &openxiii_assets::mod_meshes::ModMesh) -> Option<LocalSets> {
        use openxiii_assets::mod_meshes::Collision;
        let (mesh, materials) = (&m.0, &m.1);
        let mut tris = Vec::new();
        let box_corners;
        let positions = match m.2 {
            Collision::None => return None,
            Collision::Mesh => {
                for (s, section) in mesh.sections.iter().enumerate() {
                    for t in mesh.section_triangles(section) {
                        tris.push([t[0], t[1], t[2], s as u16]);
                    }
                }
                &mesh.positions
            }
            Collision::Box => {
                let (lo, hi) = (mesh.bounds_min, mesh.bounds_max);
                box_corners = (0..8).map(|i| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }]).collect::<Vec<_>>();
                // Bottom, top, -Y, +Y, -X, +X (traces hit a triangle from either side).
                for f in [[0u16, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
                    tris.push([f[0], f[1], f[2], 0]);
                    tris.push([f[0], f[2], f[3], 0]);
                }
                &box_corners
            }
        };
        let (verts, tris, nodes) = kdop::build_tree(positions, &tris)?;
        // Each collision triangle (cut ones included) keeps its part's material.
        let section_tags: Vec<u32> = (0..mesh.sections.len()).map(|s| self.material_tag(materials.get(s).map_or("", String::as_str))).collect();
        let tags = tris.iter().map(|t| section_tags.get(t[3] as usize).copied().unwrap_or(0)).collect();
        // The model's box, one unit wider: its faces lie on the box's sides.
        let (lo, hi) = (mesh.bounds_min.map(|x| x - 1.0), mesh.bounds_max.map(|x| x + 1.0));
        let full = KdopSet::new(verts, tris, nodes, tags);
        let simple = KdopSet::new(Vec::new(), Vec::new(), Vec::new(), Vec::new());
        Some(LocalSets { full, simple, use_simple_line: false, use_simple_box: false, root: [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]] })
    }

    /// Collision triangles of a StaticMesh (read once per mesh).
    fn mesh_collision(&mut self, path: &str) -> Option<Arc<LocalSets>> {
        if let Some(c) = self.sm_local.get(path) {
            return c.clone();
        }
        let c = self.read_mesh_collision(path).map(Arc::new);
        self.sm_local.insert(path.to_string(), c.clone());
        c
    }

    /// Shape of the actor at its current position (rebuilt if its mesh or frame changed; removed if it
    /// no longer has a StaticMesh as its primitive). Skipped when nothing can have changed since the
    /// last pass: inputs read = the object's class (fixed), its properties Mesh, StaticMesh, Rotation,
    /// DrawScale3D, Location, PrePivot and DrawScale (generation of `props`, Props::generation), the
    /// mesh table `sm_local` (which only grows), the slots of the native fields (`sm_generations`
    /// cleared along with them by clear_slot_cache) and `sm_shapes` itself (replaced on travel:
    /// `sm_generations` too). "panique": stop).
    pub(crate) fn sm_shape_update(&mut self, a: ObjId) {
        let generation = self.objects.get(a).and_then(|o| o.as_ref()).or_else(|| self.freed.get(&a)).map(|o| o.props.generation());
        if let Some(g) = generation {
            if self.sm_generations.get(&a) == Some(&g) {
                return;
            }
        }
        self.sm_shape_compute(a);
        if let Some(g) = generation {
            self.sm_generations.insert(a, g);
        }
    }

    fn sm_shape_compute(&mut self, a: ObjId) {
        let Some(path_ref) = self.prim_static_mesh(a) else {
            self.sm_shapes.remove(&a);
            return;
        };
        let (r, s3, l, pp, ds) = (self.native_prop(a, Field::Rotation).as_rotator(), self.native_prop(a, Field::DrawScale3D).as_vector(), self.native_prop(a, Field::Location).as_vector(), self.native_prop(a, Field::PrePivot).as_vector(), self.native_prop(a, Field::DrawScale).as_float());
        let bits = [
            r[0] as u32,
            r[1] as u32,
            r[2] as u32,
            s3[0].to_bits(),
            s3[1].to_bits(),
            s3[2].to_bits(),
            l[0].to_bits(),
            l[1].to_bits(),
            l[2].to_bits(),
            pp[0].to_bits(),
            pp[1].to_bits(),
            pp[2].to_bits(),
            ds.to_bits(),
        ];
        // Same mesh: only the frame changes (no copy of the path or lookup of the mesh).
        if self.sm_shapes.get(&a).is_some_and(|s| s.key.0 == path_ref) {
            let Some(s) = self.sm_shapes.get_mut(&a) else { return };
            if s.key.1 == bits {
                return;
            }
            let frame = match s.prev.take() {
                Some((f, b)) if b == bits => f,
                Some((f, _)) if !crate::physics::coll::prior(crate::physics::coll::TRIANGLES) => kdop::frame_resume(r, s3, l, pp, ds, f),
                _ => kdop::frame(r, s3, l, pp, ds),
            };
            let old = std::mem::replace(&mut s.frame, frame);
            s.prev = Some((old, s.key.1));
            s.key.1 = bits;
            return;
        }
        let path = path_ref.to_string();
        let Some(sets) = self.mesh_collision(&path) else {
            self.sm_shapes.remove(&a);
            return;
        };
        let shape = SmShape { sets, frame: kdop::frame(r, s3, l, pp, ds), key: (path, bits), prev: None };
        self.sm_shapes.insert(a, shape);
    }

    /// UStaticMesh::LineCheck on the shape of actor `o` (without
    /// bUseCylinderCollision, already handled by `primitive`).
    pub(crate) fn static_mesh_line_check(&self, o: ObjId, shape: &SmShape, end: V, start: V, extent: V, flags: u32) -> Option<CheckResult> {
        let sets = &shape.sets;
        let zero = extent[0] == 0.0 && extent[1] == 0.0 && extent[2] == 0.0;
        let simple = !sets.simple.nodes.is_empty() && ((sets.use_simple_box && !zero) || (sets.use_simple_line && zero)) && flags & TRACE_SHADOW_CAST == 0;
        let set = if simple { &sets.simple } else { &sets.full };
        if set.nodes.is_empty() {
            return None;
        }
        let h = {
            kdop::line_check(set, &sets.root, &shape.frame, end, start, extent)
        }?;
        let (dx, dy, dz) = (end[0] - start[0], end[1] - start[1], end[2] - start[2]);
        // appSqrt (FSQRT) then 1 / Dist, each rounded to 24 bits by the FPU.
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let inv = 1.0 / dist;
        let a = inv * 0.1;
        // Clamp(0.1; 0.1 / Dist; 1 / Dist).
        let pull = if 0.1 < a {
            a
        } else if 0.1 < inv {
            0.1
        } else {
            inv
        };
        let t = h.time - pull;
        let time = if t >= 0.0 || t.is_nan() {
            if t < 1.0 {
                t
            } else {
                1.0
            }
        } else {
            0.0
        };
        let mut normal = h.normal;
        kdop::normalize_nonzero(&mut normal);
        if (dz * normal[2] + dy * normal[1]) + dx * normal[0] > 0.0 {
            normal = [-normal[0], -normal[1], -normal[2]];
        }
        Some(CheckResult {
            actor: Some(o),
            location: [dx * time + start[0], dy * time + start[1], dz * time + start[2]],
            normal,
            time,
            item: h.triangle as i32,
            material: set.tags.get(h.triangle).copied(),
        })
    }

    /// UStaticMesh::PointCheck: simplified set if its tree exists and bit 1
    /// of +0x188 is set.
    pub(crate) fn static_mesh_point_check(&self, o: ObjId, shape: &SmShape, location: V, extent: V) -> Option<CheckResult> {
        let sets = &shape.sets;
        let set = if !sets.simple.nodes.is_empty() && sets.use_simple_box { &sets.simple } else { &sets.full };
        if set.nodes.is_empty() {
            return None;
        }
        let h = {
            kdop::point_check(set, &sets.root, &shape.frame, location, extent)
        }?;
        let mut normal = h.normal;
        openxiii_assets::coords::normalize(&mut normal);
        Some(CheckResult { actor: Some(o), location: h.location, normal, time: h.time, item: h.triangle as i32, material: set.tags.get(h.triangle).copied() })
    }
}

