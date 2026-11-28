//! AActor's radius iterators (Engine.dll): RadiusActors, VisibleActors,
//! CollidingActors, VisibleCollidingActors and
//! VisibleDamageableActors, with FCollisionOctree::ActorRadiusCheck (
//! FOctreeNode::ActorRadiusCheck) and the victims' render box
//! (GetRenderBoundingBox, FBox::TransformBy).
//! Operations in machine-code order, in 32-bit floats (24-bit FPU).
//!
//! Known differences: the engine walks its lists during the script's loop (an actor destroyed
//! or moved by the loop body is taken into account), here the list is made at the start; the order
//! of the octree's results (node traversal, head insertion) is not reproduced.

use crate::NativeField as Field;
use crate::physics::CheckResult;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::coords::{self, Matrix};

type V = [f32; 3];

/// Box (Min, Max).
type Bx = (V, V);

/// FMatrix::TransformFVector.
pub(crate) fn transform_fvector(m: &Matrix, v: V) -> V {
    [0, 1, 2].map(|c| ((m[8 + c] * v[2] + m[4 + c] * v[1]) + v[0] * m[c]) + m[12 + c])
}

/// FBox += point after point into an empty box: the first point sets Min and Max,
/// then Min keeps the old value unless it is greater than the point or not comparable (NaN), Max keeps
/// the old value unless it is less than the point or not comparable.
pub(crate) fn box_points(points: &[V]) -> Option<Bx> {
    let (first, rest) = points.split_first()?;
    let (mut lo, mut hi) = (*first, *first);
    for v in rest {
        for k in 0..3 {
            lo[k] = if lo[k] <= v[k] { lo[k] } else { v[k] };
            hi[k] = if hi[k] >= v[k] { hi[k] } else { v[k] };
        }
    }
    Some((lo, hi))
}

/// AActor::execRecomputeBoundingVolume, the instance's box: each point of the pose
/// goes through MeshToWorld (TransformFVector) before the FBox +=.
pub(crate) fn box_skeleton(points: &[V], m2w: &Matrix) -> Option<Bx> {
    let world_value: Vec<V> = points.iter().map(|p| transform_fvector(m2w, *p)).collect();
    box_points(&world_value)
}

/// FBox::TransformBy: centre transformed, half-extent
/// Σ |M[i][k]| · e[i], then the box of the two corners C + E and C − E.
pub(crate) fn transform_box(b: &Bx, m: &Matrix) -> Bx {
    let center = [0, 1, 2].map(|k| (b.1[k] + b.0[k]) * 0.5);
    let c = transform_fvector(m, center);
    let e = [0, 1, 2].map(|k| (b.1[k] - b.0[k]) * 0.5);
    let ext = [0, 1, 2].map(|k| (m[8 + k].abs() * e[2] + m[4 + k].abs() * e[1]) + m[k].abs() * e[0]);
    let hi = [0, 1, 2].map(|k| c[k] + ext[k]);
    let lo = [0, 1, 2].map(|k| c[k] - ext[k]);
    // FBox +=: C + E into an empty box, then C − E.
    ([0, 1, 2].map(|k| if lo[k] < hi[k] { lo[k] } else { hi[k] }), [0, 1, 2].map(|k| if lo[k] > hi[k] { lo[k] } else { hi[k] }))
}

/// UStaticMesh::GetCollisionBoundingBox (without bUseCylinderCollision): the mesh's
/// box passed through FCoords (Rotation, DrawScale3D, Location, PrePivot,
/// DrawScale) then FBox::TransformBy.
pub(crate) fn static_mesh_world_box(b: &Bx, rotation: [i32; 3], draw_scale3d: V, location: V, pre_pivot: V, draw_scale: f32) -> Bx {
    transform_box(b, &local_to_world(rotation, draw_scale3d, location, pre_pivot, draw_scale))
}

/// UStaticMesh::GetEncroachCenter (FBox::GetCenter: (max + min) × 0.5) and
/// GetEncroachExtent ((max − min) × 0.5) of a box.
pub(crate) fn box_center_extent(b: &Bx) -> (V, V) {
    ([0, 1, 2].map(|k| (b.1[k] + b.0[k]) * 0.5), [0, 1, 2].map(|k| (b.1[k] - b.0[k]) * 0.5))
}

/// LocalToWorld matrix (Rotation, DrawScale3D, Location, PrePivot, DrawScale).
pub(crate) fn local_to_world(r: [i32; 3], s3: V, l: V, pp: V, ds: f32) -> Matrix {
    let sin = coords::gmath_sin;
    let sr = sin(r[2].wrapping_neg());
    let cr = sin(0x4000i32.wrapping_sub(r[2]));
    let sy = sin(r[1]);
    let cy = sin(r[1].wrapping_add(0x4000));
    let sp = sin(r[0].wrapping_neg());
    let cp = sin(0x4000i32.wrapping_sub(r[0]));
    let mut m = [0.0f32; 16];
    // cp × (DrawScale × DrawScale3D.X), then × cos / sin of the yaw.
    let a = cp * (ds * s3[0]);
    m[0] = a * cy;
    m[1] = a * sy;
    m[2] = -(ds * s3[0] * sp);
    let b = ds * s3[1];
    m[4] = (sp * sr * cy - sy * cr) * b;
    m[5] = (cy * cr + sp * sr * sy) * b;
    m[6] = b * cp * sr;
    let c = ds * s3[2];
    m[8] = (sy * sr + sp * cr * cy) * c;
    m[9] = (sp * cr * sy - cy * sr) * c;
    m[10] = c * cp * cr;
    // Origin: Location − transformed PrePivot, sums in the machine code's order.
    m[12] = l[0] - ((pp[0] * m[0] + m[8] * pp[2]) + pp[1] * m[4]);
    m[13] = l[1] - ((pp[1] * m[5] + pp[0] * m[1]) + m[9] * pp[2]);
    m[14] = l[2] - ((m[10] * pp[2] + pp[0] * m[2]) + pp[1] * m[6]);
    m[15] = 1.0;
    m
}

impl Vm {
    /// UPrimitive::BoundingBox (+ 0x30) of a mesh or a StaticMesh, cached.
    pub(crate) fn primitive_box(&mut self, asset: &str) -> Option<Bx> {
        if let Some(cached) = crate::with_lower(asset, |key| self.primitive_boxes.get(key).copied()) {
            return cached;
        }
        // Mods build only: a mod's own static mesh (new, or drawn instead of a game mesh) has
        // the box of its own vertices; without it the game sees an empty box and never draws it.
        #[cfg(feature = "mods")]
        let own = openxiii_assets::mod_meshes::get(asset).map(|m| (m.0.bounds_min, m.0.bounds_max)).or_else(|| openxiii_assets::mod_skeletal::get(asset).map(|m| m.bounds));
        #[cfg(not(feature = "mods"))]
        let own = None;
        let b = own.or_else(|| {
            self.library.as_mut().and_then(|lib| {
                let (loaded, index) = lib.resolve(asset)?;
                openxiii_assets::skeletal_collision::read_primitive_box(&loaded.package, &loaded.data, index).ok().map(|(lo, hi, _)| (lo, hi))
            })
        });
        self.primitive_boxes.insert(asset.to_lowercase(), b);
        b
    }

    /// FCollisionOctree::ActorRadiusCheck (see `octree.rs`).
    pub(crate) fn actor_radius_check(&mut self, loc: V, radius: f32) -> Vec<ObjId> {
        #[cfg(test)]
        if let Some(r) = hook_radius::list_value(loc, radius) {
            return r;
        }
        self.octree_radius_check(loc, radius)
    }

    /// GetPrimitive()->GetRenderBoundingBox(Actor, matrix): the matrix is MeshToWorld for a
    /// DT_Mesh with a mesh if `mesh_matrix` (iterators), otherwise LocalToWorld (
    /// AActor::execGetBoundingBox). Mesh or StaticMesh: UPrimitive::BoundingBox
    /// transformed (FBox::TransformBy); GEngine->Cylinder (UPrimitive, without Mesh or
    /// StaticMesh): ±(CollisionRadius + 1, CollisionRadius + 1, CollisionHeight + 1) transformed
    /// (its own box +0x30, zero, only without an actor). GetPrimitive is overridden: ABrush takes Brush, without looking at
    /// Mesh or StaticMesh (UModel::GetRenderBoundingBox: BoundingBox transformed);
    /// AEmitter takes GEngine+0x48, a UPrimitiveNS: origin of the
    /// matrix ± (CollisionRadius + 1, CollisionRadius + 1, CollisionHeight + 1).
    /// UStaticMesh::GetRenderBoundingBox would take the StaticMeshInstance's box
    /// (+0x4C, valid at +0x64) without transforming it, but UStaticMeshInstance::Serialize
    /// only reads it before licensee version 0x10: it stays empty in XIII. A skeletal mesh
    /// goes through USkeletalMesh::GetRenderBoundingBox, which calls the instance's.
    pub fn render_bounding_box(&mut self, o: ObjId, mesh_matrix: bool) -> Bx {
        self.render_box_with_matrix(o, mesh_matrix, None)
    }

    /// Refresh has already produced its matrix. The special Emitter and
    /// Brush primitives keep their LocalToWorld, even if drawing uses MeshToWorld.
    pub(crate) fn render_box_with_matrix(&mut self, o: ObjId, mesh_matrix: bool, prepared: Option<&Matrix>) -> Bx {
        use crate::flags_classes::D;
        if self.is(o, D::Emitter) || self.is(o, D::Brush) {
            let ds = self.native_prop(o, Field::DrawScale).as_float();
            let m = local_to_world(
                self.native_prop(o, Field::Rotation).as_rotator(),
                self.native_prop(o, Field::DrawScale3D).as_vector(),
                self.native_prop(o, Field::Location).as_vector(),
                self.native_prop(o, Field::PrePivot).as_vector(),
                ds,
            );
            if self.is(o, D::Emitter) {
                let r = self.native_prop(o, Field::CollisionRadius).as_float() + 1.0;
                let h = self.native_prop(o, Field::CollisionHeight).as_float() + 1.0;
                return ([-r + m[12], -r + m[13], -h + m[14]], [r + m[12], r + m[13], h + m[14]]);
            }
            if !crate::physics::coll::prior(crate::physics::coll::CLONES) {
                if let Value::Asset(a) = self.propr(o, "Brush") {
                    if let Some(b) = crate::with_lower(a, |key| self.primitive_boxes.get(key).copied()) {
                        return transform_box(&b.unwrap_or(([0.0; 3], [0.0; 3])), &m);
                    }
                }
            }
            let b = match self.prop(o, "Brush") {
                Value::Asset(a) => self.primitive_box(&a).unwrap_or(([0.0; 3], [0.0; 3])),
                _ => {
                    // ABrush::GetPrimitive: without a Brush, GEngine->Cylinder.
                    // UPrimitive::GetRenderBoundingBox: cylinder before TransformBy.
                    let h = self.native_prop(o, Field::CollisionHeight).as_float() + 1.0;
                    let r = self.native_prop(o, Field::CollisionRadius).as_float() + 1.0;
                    ([-r, -r, -h], [r, r, h])
                }
            };
            return transform_box(&b, &m);
        }
        let has_mesh = matches!(self.native_prop(o, Field::Mesh), Value::Asset(_));
        // USkeletalMeshInstance::GetRenderBoundingBox: the box computed by
        // RecomputeBoundingVolume as long as it is valid.
        if has_mesh {
            if let Some(&b) = self.skel_render_boxes.get(&o) {
                return b;
            }
        }
        let matrix = prepared.copied().unwrap_or_else(|| {
            // The mesh path is only copied if it is used (DrawType 2 and mesh matrix).
            let serves = crate::physics::coll::prior(crate::physics::coll::CLONES) || (mesh_matrix && self.native_prop(o, Field::DrawType).as_byte() == 2);
            if let (true, Value::Asset(m), 2, true) = (serves, if serves { self.native_value(o, Field::Mesh) } else { Value::Void }, self.native_prop(o, Field::DrawType).as_byte(), mesh_matrix) {
                if let Some(info) = self.mesh_info(&m) { return self.mesh_to_world(o, &info); }
            }
            local_to_world(
                self.native_prop(o, Field::Rotation).as_rotator(),
                self.native_prop(o, Field::DrawScale3D).as_vector(),
                self.native_prop(o, Field::Location).as_vector(),
                self.native_prop(o, Field::PrePivot).as_vector(),
                self.native_prop(o, Field::DrawScale).as_float(),
            )
        });
        // Mesh, otherwise StaticMesh; box already read looked up by name in place, without copying.
        let field = if has_mesh { Field::Mesh } else { Field::StaticMesh };
        let cached = match self.native_prop(o, field) {
            Value::Asset(a) => Some(crate::with_lower(a, |key| self.primitive_boxes.get(key).copied())),
            _ => None,
        };
        let b = match cached {
            Some(Some(b)) => b.unwrap_or(([0.0; 3], [0.0; 3])),
            Some(None) => {
                let Value::Asset(a) = self.native_value(o, field) else { unreachable!() };
                self.primitive_box(&a).unwrap_or(([0.0; 3], [0.0; 3]))
            }
            None => {
                // CollisionHeight + 1, CollisionRadius + 1.
                let h = self.native_prop(o, Field::CollisionHeight).as_float() + 1.0;
                let r = self.native_prop(o, Field::CollisionRadius).as_float() + 1.0;
                ([-r, -r, -h], [r, r, h])
            }
        };
        transform_box(&b, &matrix)
    }

    /// AActor::execRecomputeBoundingVolume: for a skeletal mesh, false
    /// invalidates the instance's box; true recomputes it on the pose's vertices (GetMeshVerts,
    /// vt 0x120: GetFrame) placed in the world, FBox += point by
    /// point. USkeletalMesh::RenderPreProcess, called before, only prepares the
    /// normals and draw indices.
    pub(crate) fn recompute_bounding_volume(&mut self, a: ObjId, compute: bool) {
        let Value::Asset(mesh) = self.prop(a, "Mesh") else { return };
        if self.mesh_instance(a).is_none() {
            return;
        }
        if !compute {
            self.skel_render_boxes.remove(&a);
            return;
        }
        let key = mesh.to_lowercase();
        if !self.skel_models.contains_key(&key) {
            let model = self.library.as_mut().and_then(|lib| openxiii_assets::character::CharacterModel::load(lib, &mesh)).map(std::rc::Rc::new);
            self.skel_models.insert(key.clone(), model);
        }
        let Some(model) = self.skel_models.get(&key).cloned().flatten() else { return };
        let Some(bases) = self.get_frame(a) else { return };
        let Some(info) = self.mesh_instance(a).map(|i| i.info.clone()) else { return };
        let points = model.skin_points(&bases);
        let m2w = self.mesh_to_world(a, &info);
        let Some((lo, hi)) = box_skeleton(&points, &m2w) else { return };
        self.skel_render_boxes.insert(a, (lo, hi));
    }

    /// Centre of the render box of a VisibleDamageableActors victim.
    fn render_box_center(&mut self, o: ObjId) -> V {
        #[cfg(test)]
        if let Some((lo, hi)) = hook_radius::bbox(o) {
            return [0, 1, 2].map(|k| (hi[k] + lo[k]) * 0.5);
        }
        let (lo, hi) = self.render_bounding_box(o, true);
        [0, 1, 2].map(|k| (hi[k] + lo[k]) * 0.5)
    }

    /// Elements of the radius iterators, in the order the engine returns them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn radius_iterator(
        &mut self,
        name: &str,
        this: ObjId,
        class: Option<crate::value::ClassId>,
        radius: f32,
        loc: V,
        ignore_hidden: bool,
    ) -> Vec<ObjId> {
        let class = class.or_else(|| self.class("Actor"));
        let is_a = |vm: &Vm, o: ObjId| vm.objects.get(o).and_then(|x| x.as_ref()).is_some_and(|x| class.is_some_and(|c| vm.is_child_of(x.class, c)));
        match name {
            // RadiusActors: XLevel->Actors, (dx² + dy²) + dz² < (Radius + CollisionRadius)².
            "RadiusActors" => self
                .actors
                .clone()
                .into_iter()
                .filter(|&o| is_a(self, o))
                .filter(|&o| {
                    let l = self.native_prop(o, Field::Location).as_vector();
                    let d = [l[0] - loc[0], l[1] - loc[1], l[2] - loc[2]];
                    let r = radius + self.native_prop(o, Field::CollisionRadius).as_float();
                    (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0] < r * r
                })
                .collect(),
            // VisibleActors: XLevel->Actors not hidden, Radius zero or distance < Radius, then
            // trace TRACE_World | StopAtFirstHit (0x286) from the actor to Loc, free or on it.
            "VisibleActors" => self
                .actors
                .clone()
                .into_iter()
                .filter(|&o| !self.native_prop(o, Field::Hidden).as_bool() && is_a(self, o))
                .filter(|&o| {
                    let l = self.native_prop(o, Field::Location).as_vector();
                    let d = [l[0] - loc[0], l[1] - loc[1], l[2] - loc[2]];
                    // (dZ² + dY²) + dX².
                    radius == 0.0 || (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0] < radius * radius
                })
                .filter(|&o| {
                    let mut hit = CheckResult::new(1.0);
                    self.single_line_check(&mut hit, Some(this), self.native_prop(o, Field::Location).as_vector(), loc, 0x286, [0.0; 3]);
                    hit.actor.is_none() || hit.actor == Some(o)
                })
                .collect(),
            // CollidingActors: the octree alone.
            "CollidingActors" => self.actor_radius_check(loc, radius).into_iter().filter(|&o| is_a(self, o)).collect(),
            // VisibleCollidingActors / VisibleDamageableActors: the octree, bIgnoreHidden, (no
            // StaticMeshActor for Damageable), TRACE_World trace from Loc to the centre of the render box; for a
            // pawn, a second try towards its eyes (Location.Z + EyeHeight).
            _ => {
                let candidates = self.visible_radius_candidates(name, this, loc, radius);
                let damageable = name == "VisibleDamageableActors";
                candidates.into_iter().filter(|&o| self.visible_radius_ok(damageable, this, class, loc, ignore_hidden, o)).collect()
            }
        }
    }

    /// Octree list (ULevel::Hash->ActorRadiusCheck) taken at the start of the loop of
    /// VisibleCollidingActors / VisibleDamageableActors.
    pub(crate) fn visible_radius_candidates(&mut self, name: &str, this: ObjId, loc: V, radius: f32) -> Vec<ObjId> {
        let candidates = self.actor_radius_check(loc, radius);
        let _ = (name, this);
        candidates
    }

    /// A candidate of VisibleCollidingActors / VisibleDamageableActors: of the class, bIgnoreHidden,
    /// (no StaticMeshActor for Damageable), TRACE_World trace from Loc to the centre of the render
    /// box; for a pawn, a second try towards its eyes (Location.Z + EyeHeight). The engine does this
    /// test on the loop iteration that reaches the candidate, after the body of the previous iteration
    /// (then body for Colliding):
    /// a grate broken by TakeDamage no longer hides what is behind it.
    pub(crate) fn visible_radius_ok(&mut self, damageable: bool, this: ObjId, class: Option<crate::value::ClassId>, loc: V, ignore_hidden: bool, o: ObjId) -> bool {
        let class = class.or_else(|| self.class("Actor"));
        let is_a = self.objects.get(o).and_then(|x| x.as_ref()).is_some_and(|x| class.is_some_and(|c| self.is_child_of(x.class, c)));
        if !is_a || (ignore_hidden && self.native_prop(o, Field::Hidden).as_bool()) || (damageable && self.is_a(o, "StaticMeshActor")) {
            return false;
        }
        let clear = |vm: &Vm, end: V| {
            let mut hit = CheckResult::new(1.0);
            vm.single_line_check(&mut hit, Some(this), end, loc, 0x86, [0.0; 3]);
            hit.actor.is_none() || hit.actor == Some(o)
        };
        let center = self.render_box_center(o);
        if clear(self, center) {
            return true;
        }
        if self.is_a(o, "Pawn") {
            let l = self.native_prop(o, Field::Location).as_vector();
            let eye = [l[0], l[1], self.propr(o, "EyeHeight").as_float() + l[2]];
            return clear(self, eye);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_transformed() {
        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        m[12] = 10.0;
        let (lo, hi) = transform_box(&([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]), &m);
        assert_eq!(lo, [9.0, -2.0, -3.0]);
        assert_eq!(hi, [11.0, 2.0, 3.0]);
    }
}

/// Test: octree list and render boxes forced (CollidingActors, VisibleCollidingActors
/// and VisibleDamageableActors).
#[cfg(test)]
pub(crate) mod hook_radius {
    use super::{ObjId, V};
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Default)]
    struct State {
        list_value: Vec<ObjId>,
        boxes: HashMap<ObjId, (V, V)>,
        calls_list: Vec<(V, f32)>,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub(crate) fn pose(list_value: Vec<ObjId>, boxes: HashMap<ObjId, (V, V)>) {
        STATE.with(|e| *e.borrow_mut() = Some(State { list_value, boxes, calls_list: Vec::new() }));
    }

    pub(crate) fn removes() -> Vec<(V, f32)> {
        STATE.with(|e| e.borrow_mut().take().map(|x| x.calls_list).unwrap_or_default())
    }

    pub(super) fn list_value(loc: V, radius: f32) -> Option<Vec<ObjId>> {
        STATE.with(|e| {
            let mut e = e.borrow_mut();
            let state_value = e.as_mut()?;
            state_value.calls_list.push((loc, radius));
            Some(state_value.list_value.clone())
        })
    }

    pub(super) fn bbox(o: ObjId) -> Option<(V, V)> {
        STATE.with(|e| e.borrow().as_ref().and_then(|x| x.boxes.get(&o).copied()))
    }
}

