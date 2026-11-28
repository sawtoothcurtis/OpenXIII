//! Bone hit by a trace (TRACETYPE_RequestBones, 0x10000): USkeletalMesh::LineCheck
//! refines the cylinder contact (UPrimitive::LineCheck) with the collision boxes
//! of the bones, and AActor::execTrace / execTraceActors derive from it the name read
//! by GetLastTraceBone (global).
//! Operations in machine-code order, in 32-bit floats (24-bit FPU).

use crate::physics::CheckResult;
use crate::value::ObjId;
use crate::Vm;
use openxiii_assets::coords::{self, Coords};
use openxiii_assets::skeletal_collision::BoneBox;
use std::rc::Rc;

type V = [f32; 3];

/// TRACETYPE_RequestBones.
pub const TRACE_REQUEST_BONES: u32 = 0x10000;

/// Point transformed by a frame, the origin added after the products
/// ((axis.y·P.y + axis.z·P.z) + P.x·axis.x).
fn transform_by(p: V, c: &Coords) -> V {
    let tx = (c.x[1] * p[1] + c.x[2] * p[2]) + p[0] * c.x[0];
    let ty = (c.y[1] * p[1] + c.y[2] * p[2]) + c.y[0] * p[0];
    let tz = ((c.z[1] * p[1] + c.z[2] * p[2]) + c.z[0] * p[0]) + c.origin[2];
    [tx + c.origin[0], ty + c.origin[1], tz]
}

/// FLineBoxIntersection(Box, Start, End, Direction, OneOverDirection).
fn line_box_intersection(b: &BoneBox, start: V, dir: V, inv: V) -> bool {
    let mut inside = true;
    let mut time = [0.0f32; 3];
    for k in 0..3 {
        if start[k] < b.min[k] {
            if dir[k] <= 0.0 {
                return false;
            }
            inside = false;
            time[k] = (b.min[k] - start[k]) * inv[k];
        } else if start[k] > b.max[k] {
            if dir[k] >= 0.0 {
                return false;
            }
            inside = false;
            time[k] = (b.max[k] - start[k]) * inv[k];
        } else if k == 2 && inside {
            // Start inside the box.
            return true;
        }
    }
    // Max(Time.X, Max(Time.Y, Time.Z)).
    let yz = if time[1] < time[2] { time[2] } else { time[1] };
    let t = if time[0] < yz { yz } else { time[0] };
    if t < 0.0 || t > 1.0 || t.is_nan() {
        return false;
    }
    // Start + Direction · t; margin BOX_SIDE_THRESHOLD = 0.1.
    let hit = [0, 1, 2].map(|k| start[k] + dir[k] * t);
    (0..3).all(|k| b.min[k] - 0.1 < hit[k] && b.max[k] + 0.1 > hit[k])
}

/// Loop of USkeletalMesh::LineCheck: for each box in array
/// order, frame of its bone (GetBoneCoords), inverse, Start then End
/// brought into the frame, FLineBoxIntersection; the first box
/// crossed gives the bone.
pub(crate) fn os_key(boxes: &[BoneBox], mut marker: impl FnMut(u16) -> Coords, end: V, start: V) -> Option<u16> {
    for b in boxes {
        let inv = marker(b.bone).inverse();
        let s = transform_by(start, &inv);
        let e = transform_by(end, &inv);
        let dir = [e[0] - s[0], e[1] - s[1], e[2] - s[2]];
        let one_over = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
        if line_box_intersection(b, s, dir, one_over) {
            return Some(b.bone);
        }
    }
    None
}

impl Vm {
    /// Collision boxes of the bones of a skeletal mesh (USkeletalMesh + 0x1A8), cached.
    pub(crate) fn bone_boxes(&mut self, mesh: &str) -> Option<Rc<Vec<BoneBox>>> {
        let key = mesh.to_lowercase();
        if let Some(b) = self.bone_boxes.get(&key) {
            return b.clone();
        }
        // Mods build only: the boxes made for a mod's animated model.
        #[cfg(feature = "mods")]
        let own = openxiii_assets::mod_skeletal::get(mesh).map(|m| Rc::new(m.bone_boxes.clone()));
        #[cfg(not(feature = "mods"))]
        let own = None;
        let boxes = own.or_else(|| {
            self.library.as_mut().and_then(|lib| {
                let (loaded, index) = lib.resolve(mesh)?;
                if loaded.package.export_class(index) != "SkeletalMesh" {
                    return None;
                }
                openxiii_assets::skeletal_collision::read_bone_boxes(&loaded.package, &loaded.data, index).ok().map(Rc::new)
            })
        });
        self.bone_boxes.insert(key, boxes.clone());
        boxes
    }

    /// USkeletalMeshInstance::GetBoneCoords by bone index: frame of the bone in the
    /// world, normalized axes (like `get_bone_coords`); index outside the skeleton: UnitCoords.
    fn bone_coords_at(&mut self, actor: ObjId, idx: usize) -> Coords {
        let Some(info) = self.mesh_instance(actor).map(|i| i.info.clone()) else {
            return coords::UNIT;
        };
        let bases = self.get_frame(actor).unwrap_or_default();
        let Some(b) = bases.get(idx) else {
            return coords::UNIT;
        };
        crate::player_native::frame_os(b, &self.mesh_to_world(actor, &info))
    }

    /// USkeletalMesh::LineCheck of every actor whose cylinder is hit by a trace with
    /// TRACE_RequestBones, even if it is not the closest at the end: its bones are tested
    /// (GetBoneCoords, hence GetFrame, once per frame and per actor).
    /// Without this, the player's pawn only had a cached pose for the shots that hit it last.
    pub(crate) fn pose_candidates_os(&mut self) {
        for a in self.octree.take_bone_candidates() {
            let crate::value::Value::Asset(mesh) = self.prop(a, "Mesh") else { continue };
            if self.bone_boxes(&mesh).is_some() {
                let _ = self.get_frame(a);
            }
        }
    }

    /// End of USkeletalMesh::LineCheck after a cylinder contact, if the trace
    /// requests bones: first box crossed in array order → Hit.Item = its bone.
    /// The contact (time, position, normal) stays that of the cylinder.
    pub(crate) fn skeletal_bone_check(&mut self, hit: &mut CheckResult, end: V, start: V, flags: u32) {
        if flags & TRACE_REQUEST_BONES == 0 {
            return;
        }
        let Some(actor) = hit.actor else { return };
        // AActor::GetPrimitive: the Mesh, if it is skeletal.
        let crate::value::Value::Asset(mesh) = self.prop(actor, "Mesh") else { return };
        let Some(boxes) = self.bone_boxes(&mesh) else { return };
        let key_col = os_key(&boxes, |os| {
            let c = self.bone_coords_at(actor, os as usize);
            c
        }, end, start);
        if let Some(os) = key_col {
            hit.item = os as i32;
        }
    }

    /// Bone name of Hit.Item for GetLastTraceBone (execTrace / execTraceActors): actor with
    /// a skeletal Mesh and DrawType == DT_Mesh, 0 ≤ Item < RefSkeleton.Num.
    pub(crate) fn trace_bone_name(&mut self, hit: &CheckResult) -> Option<String> {
        let actor = hit.actor?;
        if hit.item == -1 || self.propr(actor, "DrawType").as_byte() != 2 {
            return None;
        }
        let crate::value::Value::Asset(mesh) = self.prop(actor, "Mesh") else { return None };
        let info = self.mesh_info(&mesh)?;
        (hit.item >= 0).then(|| info.bones.get(hit.item as usize).cloned()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_crossing() {
        let b = BoneBox { min: [-11.0; 3], max: [11.0; 3], valid: 1, bone: 0 };
        let start = [-100.0, 0.0, 0.0];
        let dir = [200.0, 0.0, 0.0];
        let inv = dir.map(|d| 1.0 / d);
        assert!(line_box_intersection(&b, start, dir, inv));
        let start = [-100.0, 20.0, 0.0];
        assert!(!line_box_intersection(&b, start, dir, inv));
        assert!(line_box_intersection(&b, [0.0; 3], dir, inv));
    }
}
