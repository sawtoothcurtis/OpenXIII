//! Lockstep co-op, drawing only: the other actors (the other players, enemies, characters,
//! vehicles, projectiles) drawn between the last two steps' places at the time of the frame,
//! instead of jumping once a step. Each step's end keeps every moving actor's place; a frame
//! puts the actors at the place between the previous step's and the last one's for its share of
//! the step's time (`lockstep_interp_begin`), is drawn, and gets the step's places back exactly
//! (`lockstep_interp_end`): the game never sees them. An actor that appeared, jumped far (a
//! teleport, a respawn) or belongs to this machine's player (its camera, its first-person
//! weapon) is drawn where the step left it.

use crate::value::{ObjId, Value};
use crate::NativeField as Field;
use crate::Vm;

/// A jump longer than this between two steps (Unreal units) is not interpolated.
pub const SNAP_DISTANCE: f32 = 256.0;

/// The places kept at the end of the last two steps.
#[derive(Default, Debug, Clone)]
pub struct Places {
    prev: std::collections::HashMap<ObjId, ([f32; 3], [i32; 3])>,
    last: std::collections::HashMap<ObjId, ([f32; 3], [i32; 3])>,
}

/// What a frame changed, to be put back (actor, Location, Rotation as they were).
pub struct Interpolated(Vec<(ObjId, Value, Value)>);

impl Interpolated {
    /// Actors drawn between two places.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The actors drawn between two places.
    pub fn actors(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.0.iter().map(|x| x.0)
    }
}

/// A camera's forward, right and up axes (Unreal rotation units: pitch, yaw, roll; roll left out).
pub fn camera_axes(rot: [i32; 3]) -> [[f32; 3]; 3] {
    let rad = |u: i32| (u as f32) * std::f32::consts::PI / 32768.0;
    let (sp, cp) = rad(rot[0]).sin_cos();
    let (sy, cy) = rad(rot[1]).sin_cos();
    [[cp * cy, cp * sy, sp], [-sy, cy, 0.0], [-sp * cy, -sp * sy, cp]]
}

/// A point in a camera's space (forward, right, up from the camera).
pub fn camera_space(p: [f32; 3], camera: ([f32; 3], [i32; 3])) -> [f32; 3] {
    let d = [p[0] - camera.0[0], p[1] - camera.0[1], p[2] - camera.0[2]];
    camera_axes(camera.1).map(|a| a[0] * d[0] + a[1] * d[1] + a[2] * d[2])
}

/// The place `alpha` (0 to 1) of the way from `a` to `b`; rotations by the shortest way.
pub fn between(a: ([f32; 3], [i32; 3]), b: ([f32; 3], [i32; 3]), alpha: f32) -> ([f32; 3], [i32; 3]) {
    let l = [0, 1, 2].map(|i| a.0[i] + (b.0[i] - a.0[i]) * alpha);
    let r = [0, 1, 2].map(|i| {
        let d = (b.1[i].wrapping_sub(a.1[i])) as i16 as f32;
        a.1[i].wrapping_add((d * alpha).round() as i32)
    });
    (l, r)
}

impl Vm {
    /// End of a step (lockstep co-op): every dynamic actor's place kept for the frames.
    pub fn lockstep_interp_keep(&mut self) {
        if self.lockstep.is_none() {
            return;
        }
        let mut places = std::mem::take(&mut self.interp_places);
        places.prev = std::mem::take(&mut places.last);
        let from = self.first_dynamic.min(self.actors.len());
        for &a in &self.actors[from..] {
            if self.objects.get(a).is_none_or(|o| o.is_none()) {
                continue;
            }
            let loc = self.native_prop(a, Field::Location).as_vector();
            let rot = self.native_prop(a, Field::Rotation).as_rotator();
            places.last.insert(a, (loc, rot));
        }
        self.interp_places = places;
    }

    /// Actor `a`'s places (location, rotation) at the end of the previous step and of the last one.
    pub fn lockstep_interp_places(&self, a: ObjId) -> Option<(([f32; 3], [i32; 3]), ([f32; 3], [i32; 3]))> {
        Some((*self.interp_places.prev.get(&a)?, *self.interp_places.last.get(&a)?))
    }

    /// A map change: nothing to interpolate from.
    pub fn lockstep_interp_clear(&mut self) {
        self.interp_places = Places::default();
    }

    /// Before drawing a frame: the actors put `alpha` of the way from the previous step's place
    /// to the last one's. None when nothing was moved.
    /// `whole` (the frame's camera between the same two steps' cameras): every actor but `mine`
    /// (this machine's player's controller) and the first-person models (bDelayDisplay, drawn in
    /// front of the step's camera, `lockstep_interp_first_person`). Otherwise (the step's camera):
    /// all but `mine`, the controller and pawn, and what they own.
    pub fn lockstep_interp_begin(&mut self, alpha: f32, mine: &[ObjId], whole: bool) -> Option<Interpolated> {
        if self.lockstep.is_none() {
            return None;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let mut changed = Vec::new();
        let places = std::mem::take(&mut self.interp_places);
        for (&a, &last) in &places.last {
            let Some(&prev) = places.prev.get(&a) else { continue };
            if prev == last || self.objects.get(a).is_none_or(|o| o.is_none()) || if whole { mine.contains(&a) || self.draw_list(a).0 != 0 } else { self.interp_mine(a, mine) } {
                continue;
            }
            let d = [0, 1, 2].map(|i| last.0[i] - prev.0[i]);
            if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > SNAP_DISTANCE {
                continue;
            }
            // Where the step left it (a frame after another drawing-only change: as kept).
            let now_l = self.native_prop(a, Field::Location).clone();
            let now_r = self.native_prop(a, Field::Rotation).clone();
            if now_l.as_vector() != last.0 || now_r.as_rotator() != last.1 {
                continue;
            }
            let (l, r) = between(prev, last, alpha);
            self.set_native_prop(a, Field::Location, Value::Vector(l));
            self.set_native_prop(a, Field::Rotation, Value::Rotator(r));
            changed.push((a, now_l, now_r));
        }
        self.interp_places = places;
        (!changed.is_empty()).then_some(Interpolated(changed))
    }

    /// Before drawing, after `lockstep_interp_begin`: this machine's first-person actors (what
    /// the local pawn owns: weapon, hands) drawn at their place in the camera's space between
    /// the last two steps (`offsets`: actor, previous step's and last step's place in the space
    /// of that step's camera), in front of the last step's camera (`camera`), which draws them;
    /// so the weapon's sway and bob move every frame instead of once a step. Added to `moved`,
    /// put back by `lockstep_interp_end`.
    pub fn lockstep_interp_first_person(&mut self, moved: &mut Option<Interpolated>, offsets: &[(ObjId, [f32; 3], [f32; 3])], camera: ([f32; 3], [i32; 3]), alpha: f32) {
        if self.lockstep.is_none() {
            return;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let axes = camera_axes(camera.1);
        for &(a, prev, last) in offsets {
            if self.objects.get(a).is_none_or(|o| o.is_none()) {
                continue;
            }
            // The step's own place, as the step left it, must be the last offset's.
            let now_l = self.native_prop(a, Field::Location).clone();
            if (0..3).any(|i| (camera_space(now_l.as_vector(), camera)[i] - last[i]).abs() > 0.01) {
                continue;
            }
            let d = [0, 1, 2].map(|i| last[i] - prev[i]);
            if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() > SNAP_DISTANCE / 8.0 {
                continue;
            }
            let off = [0, 1, 2].map(|i| prev[i] + d[i] * alpha);
            let l = [0, 1, 2].map(|k| camera.0[k] + axes[0][k] * off[0] + axes[1][k] * off[1] + axes[2][k] * off[2]);
            let now_r = self.native_prop(a, Field::Rotation).clone();
            self.set_native_prop(a, Field::Location, Value::Vector(l));
            moved.get_or_insert_with(|| Interpolated(Vec::new())).0.push((a, now_l, now_r));
        }
    }

    /// This machine's first-person models (bDelayDisplay, owned by `pawn`: weapon, hands, muzzle
    /// flash) with their place in the space of the previous step's camera (`prev_camera`) and of
    /// the last step's (`camera`), for `lockstep_interp_first_person`.
    pub fn lockstep_first_person_offsets(&self, pawn: ObjId, prev_camera: ([f32; 3], [i32; 3]), camera: ([f32; 3], [i32; 3])) -> Vec<(ObjId, [f32; 3], [f32; 3])> {
        let places = &self.interp_places;
        places
            .last
            .iter()
            .filter_map(|(&a, &last)| {
                let prev = places.prev.get(&a)?;
                if self.objects.get(a).is_none_or(|o| o.is_none()) || self.draw_list(a).0 == 0 || !self.interp_mine(a, &[pawn]) {
                    return None;
                }
                Some((a, camera_space(prev.0, prev_camera), camera_space(last.0, camera)))
            })
            .collect()
    }

    /// After drawing: every actor back exactly where the step left it.
    pub fn lockstep_interp_end(&mut self, moved: Option<Interpolated>) {
        let Some(Interpolated(moved)) = moved else { return };
        for (a, l, r) in moved {
            if self.objects.get(a).is_some_and(|o| o.is_some()) {
                self.set_native_prop(a, Field::Location, l);
                self.set_native_prop(a, Field::Rotation, r);
            }
        }
    }

    /// Actor `a` is this machine's player's own (its controller, its pawn, or owned by them:
    /// the first-person weapon and hands), drawn with the instant look's camera as it is.
    fn interp_mine(&self, a: ObjId, mine: &[ObjId]) -> bool {
        let mut o = Some(a);
        for _ in 0..4 {
            let Some(x) = o else { return false };
            if mine.contains(&x) {
                return true;
            }
            o = self.native_prop(x, Field::Owner).as_object();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point and back through a camera's space (turned and pitched): the same point.
    #[test]
    fn camera_space_round_trip() {
        let cam = ([100.0, -20.0, 50.0], [3000, 12000, 0]);
        let p = [130.0, 10.0, 45.0];
        let off = camera_space(p, cam);
        let a = camera_axes(cam.1);
        let back = [0, 1, 2].map(|k| cam.0[k] + a[0][k] * off[0] + a[1][k] * off[1] + a[2][k] * off[2]);
        for k in 0..3 {
            assert!((back[k] - p[k]).abs() < 1e-3, "{back:?} {p:?}");
        }
    }

    /// Halfway between two places: the middle point and the middle rotation by the shortest way
    /// (across 0 as well); the ends at 0 and 1.
    #[test]
    fn between_places() {
        let a = ([0.0, 0.0, 0.0], [0, 65000, 0]);
        let b = ([100.0, -50.0, 10.0], [100, 1000, 0]);
        assert_eq!(between(a, b, 0.0), a);
        assert_eq!(between(a, b, 1.0).0, b.0);
        let (l, r) = between(a, b, 0.5);
        assert_eq!(l, [50.0, -25.0, 5.0]);
        assert_eq!(r[0], 50);
        // 65000 to 1000 is 1536 units forward through 0, not 64000 back.
        assert_eq!(r[1], 65000 + 768);
    }

}
