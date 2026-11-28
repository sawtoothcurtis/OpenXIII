//! Mover physics, reimplemented from Engine.dll: AMover::performPhysics and
//! AMover::physMovingBrush. The Mover moves through ULevel::MoveActor: an actor it
//! would overlap that refuses (EncroachingOn) stops it without advancing PhysAlpha. Floating-point
//! operations in machine-code order (24-bit FPU).

use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;

/// (_ftol): truncation.
fn ftol(x: f32) -> i32 {
    crate::value::ftol(x) as i32
}

impl Vm {
    /// AMover::performPhysics: falling, projectile, PHYS_MovingBrush, PHYS_Trailer;
    /// then rotation and PostTouch as in AActor::performPhysics.
    pub(crate) fn mover_perform_physics(&mut self, a: ObjId, dt: f32) {
        match self.propr(a, "Physics").as_byte() {
            PHYS_FALLING => self.actor_phys_falling(a, dt, 0),
            PHYS_PROJECTILE => self.phys_projectile(a, dt, 0),
            PHYS_MOVING_BRUSH => self.phys_moving_brush(a, dt),
            PHYS_TRAILER => self.phys_trailer(a),
            _ => {}
        }
        self.actor_physics_tail(a, dt);
    }

    /// AMover::physMovingBrush: interpolation from OldPos / OldRot towards
    /// BasePos + KeyPos[KeyNum] / BaseRot + KeyRot[KeyNum], smoothed in MV_GlideByTime.
    fn phys_moving_brush(&mut self, a: ObjId, dt: f32) {
        let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        // KeyNum brought back into 0..8 (the engine reads KeyPos[8] past the array; nothing here).
        let key = (self.propr(a, "KeyNum").as_byte() as usize).min(8);
        fn element(v: &Value, k: usize) -> &Value {
            match v {
                Value::Array(items) => items.get(k).unwrap_or(&Value::Void),
                other if k == 0 => other,
                _ => &Value::Void,
            }
        }
        let mut delta_time = dt;
        while self.pb(a, "bInterpolating") && delta_time > 0.0 {
            let phys_alpha = self.pf(a, "PhysAlpha");
            let mut new_alpha = delta_time * self.pf(a, "PhysRate") + phys_alpha;
            if new_alpha > 1.0 {
                delta_time *= (new_alpha - 1.0) / (new_alpha - phys_alpha);
                new_alpha = 1.0;
            } else {
                delta_time = 0.0;
            }
            // MV_GlideByTime: 3a² − 2a³.
            let render = if self.propr(a, "MoverGlideType").as_byte() == 1 {
                let aa = new_alpha * new_alpha;
                let aaa = aa * new_alpha;
                3.0 * aa - (aaa + aaa)
            } else {
                new_alpha
            };
            let key_rot = element(self.propr(a, "KeyRot"), key).as_rotator();
            let key_pos = element(self.propr(a, "KeyPos"), key).as_vector();
            let (base_rot, old_rot) = (self.propr(a, "BaseRot").as_rotator(), self.propr(a, "OldRot").as_rotator());
            let (base_pos, old_pos) = (self.pv(a, "BasePos"), self.pv(a, "OldPos"));
            // FILD of the integer difference, FMUL by alpha (rounded to 24 bits), then _ftol.
            let turn = |k: usize| {
                let d = key_rot[k].wrapping_add(base_rot[k]).wrapping_sub(old_rot[k]);
                old_rot[k].wrapping_add(ftol((d as f64 * render as f64) as f32))
            };
            let rot = [turn(0), turn(1), turn(2)];
            let target = [0, 1, 2].map(|k| ((key_pos[k] + base_pos[k]) - old_pos[k]) * render + old_pos[k]);
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            let delta = [target[0] - l[0], target[1] - l[1], target[2] - l[2]];
            let mut hit = CheckResult::new(1.0);
            if self.move_actor(a, delta, rot, &mut hit, false, false, false, false) {
                self.set_f(a, "PhysAlpha", new_alpha);
                if new_alpha == 1.0 {
                    self.set_b(a, "bInterpolating", false);
                    self.event_pub(a, "KeyFrameReached", vec![]); // AMover::eventKeyFrameReached.
                }
            }
        }
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let k = 1.0 / dt;
        self.set_v(a, "Velocity", [k * (l[0] - old_location[0]), (l[1] - old_location[1]) * k, (l[2] - old_location[2]) * k]);
    }
}
