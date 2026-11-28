//! Actors attached to a bone (AttachToBone): the engine places them when their support is rendered.
//! FDynamicActor::Render of a DT_Mesh actor with a skeletal mesh walks its
//! Attached; for each one that has an AttachmentBone and a DrawType, it calls
//! USkeletalMesh::SetAttachmentLocation, draws the attached actor if the bone is found, then
//! sets its bHidden (bit 0x1000 of the word + 0x2C): the render loop does not draw it again.
//! Operations in machine-code order, in 32-bit floats (24-bit FPU).

use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::coords::{self, Coords};
use openxiii_assets::skeletal_collision::AttachTags;
use std::rc::Rc;

impl Vm {
    /// The mesh's TagAliases / TagNames / TagCoords (as names), cached.
    #[allow(clippy::type_complexity)]
    fn attach_tags(&mut self, mesh: &str) -> Option<Rc<(Vec<String>, Vec<String>, Vec<Coords>)>> {
        let key = mesh.to_lowercase();
        if let Some(t) = self.attach_tags_cache.get(&key) {
            return t.clone();
        }
        let tags = self.library.as_mut().and_then(|lib| {
            let (loaded, index) = lib.resolve(mesh)?;
            if loaded.package.export_class(index) != "SkeletalMesh" {
                return None;
            }
            let t: AttachTags = openxiii_assets::skeletal_collision::read_attach_tags(&loaded.package, &loaded.data, index).ok()?;
            let name = |n: &i32| loaded.package.name(*n).to_string();
            let coords = t
                .coords
                .iter()
                .map(|c| Coords { origin: [c[0], c[1], c[2]], x: [c[3], c[4], c[5]], y: [c[6], c[7], c[8]], z: [c[9], c[10], c[11]] })
                .collect();
            Some(Rc::new((t.aliases.iter().map(name).collect(), t.names.iter().map(name).collect(), coords)))
        });
        self.attach_tags_cache.insert(key, tags.clone());
        tags
    }

    /// USkeletalMesh::SetAttachmentLocation: places `attached` on its bone of `base`
    /// (Location, Rotation) from the SpaceBases of the last pose and RelativeLocation /
    /// RelativeRotation. False ("Attachment not found for bone") if the bone does not exist.
    pub fn set_attachment_location(&mut self, base: ObjId, attached: ObjId) -> bool {
        let bone_name = self.propr(attached, "AttachmentBone").as_name();
        let Value::Asset(mesh) = self.prop(base, "Mesh") else { return false };
        let Some(info) = self.mesh_instance(base).map(|i| i.info.clone()) else { return false };
        // Attachment alias; otherwise the bone of the same name and GMath.UnitCoords.
        let tags = self.attach_tags(&mesh);
        let empty = (Vec::new(), Vec::new(), Vec::new());
        let t = tags.as_deref().unwrap_or(&empty);
        let (bone, tag) = match os_attach(&t.0, &t.1, &t.2, &info.bones, &bone_name) {
            Some((b, tag)) => (Some(b), tag),
            None => (None, coords::UNIT),
        };
        let bases = self.get_frame(base).unwrap_or_default();
        let (Some(bone), Some(scale)) = (bone, self.mesh_world_coords(base)) else {
            self.log.push(format!("Log: Attachment not found for bone {bone_name} "));
            return false;
        };
        let Some(bone_coords) = bases.get(bone).copied() else {
            self.log.push(format!("Log: Attachment not found for bone {bone_name} "));
            return false;
        };
        let rel = self.propr(attached, "RelativeLocation").as_vector();
        let (location, rot) = pose_attach(&bone_coords, &tag, rel, self.propr(attached, "RelativeRotation").as_rotator(), &scale);
        self.set_prop(attached, "Location", Value::vector(location[0], location[1], location[2]));
        self.set_prop(attached, "Rotation", Value::rotator(rot[0], rot[1], rot[2]));
        // FCollisionHash Remove / Add if bCollideActors.
        self.refresh_collider(attached);
        true
    }

    /// `render_bone_attachments` for a drawing-only scene (lockstep co-op, Vm::preview_scene):
    /// nothing is written. The attachments of a support the step's own scene drew, where that
    /// render placed them; none for a support it did not draw.
    pub(crate) fn preview_bone_attachments(&self, base: ObjId) -> Vec<ObjId> {
        use crate::NativeField as Field;
        if !self.preview_step_drawn.contains(&base) || self.native_prop(base, Field::DrawType).as_byte() != 2 {
            return Vec::new();
        }
        let Value::Array(items) = self.lp(base, Field::Attached, "Attached") else { return Vec::new() };
        items
            .iter()
            .filter_map(Value::as_object)
            .filter(|&a| self.objects.get(a).is_some_and(|x| x.is_some()))
            .filter(|&a| !self.native_prop(a, Field::AttachmentBone).as_name().is_empty() && self.native_prop(a, Field::DrawType).as_byte() != 0)
            .collect()
    }

    /// Part of FDynamicActor::Render specific to attached actors: called when `base`
    /// (DT_Mesh, skeletal mesh) is drawn. Returns the attached actors to place and draw;
    /// each one receives bHidden as in the engine.
    pub fn render_bone_attachments(&mut self, base: ObjId) -> Vec<ObjId> {
        let mut drawn = Vec::new();
        if self.native_prop(base, crate::NativeField::DrawType).as_byte() != 2 || self.mesh_instance(base).is_none() {
            return drawn;
        }
        // List read once at the start, like the copy before (the numbers only, without copying the Values).
        use crate::NativeField as Field;
        let attached: Vec<Option<ObjId>> = match self.lp(base, Field::Attached, "Attached") {
            Value::Array(items) if items.is_empty() => return drawn,
            Value::Array(items) => items.iter().map(Value::as_object).collect(),
            _ => return drawn,
        };
        let prior = crate::physics::coll::prior(crate::physics::coll::ATTACHES);
        for v in attached {
            let Some(a) = v.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) else { continue };
            let (os, drawing) = if prior {
                (self.propr(a, "AttachmentBone").as_name().is_empty(), self.propr(a, "DrawType").as_byte())
            } else {
                (self.native_prop(a, Field::AttachmentBone).as_name().is_empty(), self.native_prop(a, Field::DrawType).as_byte())
            };
            if os || drawing == 0 {
                continue;
            }
            if self.set_attachment_location(base, a) {
                drawn.push(a);
            }
            if prior {
                self.set_prop(a, "bHidden", Value::Bool(true));
            } else {
                self.set_native_prop(a, Field::Hidden, Value::Bool(true));
            }
        }
        drawn
    }
}

/// Then the bone lookup: attachment alias (target bone and frame), otherwise the bone of the
/// same name and GMath.UnitCoords. None: "Attachment not found".
pub(crate) fn os_attach(aliases: &[String], targets_list: &[String], markers: &[Coords], os: &[String], name_value: &str) -> Option<(usize, Coords)> {
    let (target_value, tag) = match aliases.iter().position(|a| a.eq_ignore_ascii_case(name_value)) {
        Some(i) => (targets_list.get(i).cloned().unwrap_or_default(), markers.get(i).copied().unwrap_or(coords::UNIT)),
        None => (name_value.to_string(), coords::UNIT),
    };
    os.iter().position(|b| b.eq_ignore_ascii_case(&target_value)).map(|b| (b, tag))
}

/// End of SetAttachmentLocation: Location and Rotation of the attached actor from the bone's frame
/// (SpaceBase), the alias's frame, RelativeLocation, RelativeRotation and the world → mesh frame
/// (+0x2A8).
pub(crate) fn pose_attach(bone: &Coords, tag: &Coords, rel: [f32; 3], rel_rot: [i32; 3], scale: &Coords) -> ([f32; 3], [i32; 3]) {
    // Attachment frame composed with the bone's.
    let mut c = tag.compose_bone(bone);
    // RelativeLocation through TransformVectorBy (order ((x·X + y·Y) + z·Z)).
    let rel = coords::transform_vector(rel, &c);
    // GMath.UnitCoords / RelativeRotation.
    let r = coords::div_rotator(&coords::UNIT, rel_rot);
    c.origin = [rel[0] + c.origin[0], rel[1] + c.origin[1], rel[2] + c.origin[2]];
    // Position: (C * S).Origin, S being the world → mesh frame (+ 0x2A8).
    let t = c.mul(scale);
    // Rotation: OrthoRotation((C * R).Transpose() * S).
    let u = c.mul(&r).transpose().mul(scale);
    (t.origin, crate::world::ortho_rotation([u.x, u.y, u.z]))
}
