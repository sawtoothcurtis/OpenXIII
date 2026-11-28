//! PHYS_Spider for pawns, reimplemented from Engine.dll: APawn::physSpider, findNewFloor,
//! checkFloor and SpiderstepUp. The script puts there the bodies
//! that fall back with no floor under them or onto another pawn (XIIIPawn.LandedSpecial) and the
//! PlayerSpidering state. Floating-point operations in machine-code order (24-bit FPU).

#![allow(clippy::neg_multiply)] // the "× -1" reproduce the machine code's FMUL -1

use crate::ai_move::div;
use crate::physics::*;
use crate::value::ObjId;
use crate::Vm;

/// UCONST_MAXSTEPHEIGHT (35).
const MAXSTEPHEIGHT: f32 = 35.0;

/// FVector::IsNearlyZero.
fn nearly_zero(a: V) -> bool {
    a.iter().all(|c| c.abs() < 0.0001)
}

/// FVector::Normalize: (X² + Y²) + Z², no effect below 1e-8.
fn normalize(v: &mut V) {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if (sq as f64) >= 1e-8 {
        let r = 1.0 / sq.sqrt();
        *v = [r * v[0], r * v[1], r * v[2]];
    }
}

/// Cross product a ^ b.
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// a | b in the (X + Z) + Y order of SpiderstepUp's dot products (0.1 is a double there).
fn dot_xzy(a: V, b: V) -> f32 {
    (a[0] * b[0] + a[2] * b[2]) + a[1] * b[1]
}

impl Vm {
    /// APawn::physSpider: walks on any surface, Floor being the floor normal.
    pub(crate) fn phys_spider(&mut self, a: ObjId, dt: f32, mut iterations: i32) {
        if self.native_object(a, crate::NativeField::Controller).is_none() {
            return;
        }
        if nearly_zero(self.pv(a, "Floor")) {
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            if !self.find_new_floor(a, l, dt, dt, iterations) {
                return;
            }
        }
        let friction = self.volume_f(a, "GroundFriction", 8.0);
        let mut accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        if accel[0] == 0.0 && accel[1] == 0.0 && accel[2] == 0.0 {
            // Braking: V − ((V + V) × dt) × GroundFriction.
            let old = v;
            v = [0, 1, 2].map(|k| v[k] - ((v[k] + v[k]) * dt) * friction);
            if (old[1] * v[1] + old[2] * v[2]) + old[0] * v[0] < 0.0 {
                v = accel;
            }
        } else {
            let dir = safe_normal(accel);
            let vsize = (((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]) as f64).sqrt() as f32;
            let rate = self.pf(a, "AccelRate");
            if rate * rate < (accel[0] * accel[0] + accel[1] * accel[1]) + accel[2] * accel[2] {
                accel = [dir[0] * rate, dir[1] * rate, dir[2] * rate];
                self.set_v(a, "Acceleration", accel);
            }
            v = [0, 1, 2].map(|k| v[k] - ((v[k] - dir[k] * vsize) * dt) * friction);
        }
        v = [0, 1, 2].map(|k| dt * accel[k] + v[k]);
        // Velocity brought back into the floor plane.
        let floor = self.pv(a, "Floor");
        let d = (floor[2] * v[2] + floor[1] * v[1]) + v[0] * floor[0];
        v = [0, 1, 2].map(|k| v[k] - d * floor[k]);
        let max_speed = self.pf(a, "DesiredSpeed") * self.pf(a, "GroundSpeed");
        iterations += 1;
        if max_speed * max_speed < (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2] {
            let n = safe_normal(v);
            v = [max_speed * n[0], max_speed * n[1], max_speed * n[2]];
        }
        self.set_v(a, "Velocity", v);
        let desired_move = v;
        let mut hit = CheckResult::new(1.0);
        let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        self.set_b(a, "bJustTeleported", false);
        let mut remaining = dt;
        while remaining > 0.0 && iterations < 8 {
            iterations += 1;
            let mut time_tick = remaining;
            if remaining > 0.05 {
                let split = self.is_human(a) || {
                    let r2 = {
                        let cr = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
                        cr * cr
                    };
                    let min = if r2 <= 1225.0 { r2 } else { 1225.0 };
                    let dm = desired_move;
                    (((dm[2] * dm[2] + dm[1] * dm[1]) + dm[0] * dm[0]) * remaining) * remaining > min
                };
                if split {
                    let t = remaining * 0.5;
                    time_tick = if 0.05 > t { t } else { 0.05 };
                }
            }
            remaining -= time_tick;
            let delta = [desired_move[0] * time_tick, desired_move[1] * time_tick, desired_move[2] * time_tick];
            if !nearly_zero(delta) {
                self.mv(a, delta, &mut hit);
                if hit.time < 1.0 {
                    let desired_dir = safe_normal(delta);
                    let k = 1.0 - hit.time;
                    self.spider_step_up(a, desired_dir, [delta[0] * k, delta[1] * k, delta[2] * k], &mut hit);
                }
                if self.propr(a, "Physics").as_byte() == PHYS_SWIMMING {
                    let vel = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                    self.start_swimming(a, old_location, vel, time_tick, remaining, iterations);
                    return;
                }
            } else {
                // Without movement: the support must stay between 4.1 and 4.6 units below the feet.
                let (l, f, ch) = (self.native_prop(a, crate::NativeField::Location).as_vector(), self.pv(a, "Floor"), self.native_prop(a, crate::NativeField::CollisionHeight).as_float());
                remaining = 0.0;
                let adjusted = [l[0] - ch * f[0], l[1] - ch * f[1], l[2] - ch * f[2]];
                let end = [adjusted[0] - f[0] * 20.0, adjusted[1] - f[1] * 20.0, adjusted[2] - f[2] * 20.0];
                self.single_line_check(&mut hit, Some(a), end, adjusted, TRACE_WORLD, [0.0; 3]);
                let dist = hit.time * 20.0;
                if self.native_object(a, crate::NativeField::Base) == hit.actor && dist <= 2.2 + 2.4 && dist >= 1.9 + 2.2 {
                    continue;
                }
            }
            // Floor under the cylinder, 37 units along −Floor.
            let (l, f) = (self.native_prop(a, crate::NativeField::Location).as_vector(), self.pv(a, "Floor"));
            let r = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
            let extent = [r, r, self.native_prop(a, crate::NativeField::CollisionHeight).as_float()];
            let end = [l[0] - f[0] * 37.0, l[1] - f[1] * 37.0, l[2] - f[2] * 37.0];
            self.single_line_check(&mut hit, Some(a), end, l, 0xdf, extent);
            if hit.time != 1.0 {
                self.set_v(a, "Floor", hit.normal);
                let f = hit.normal;
                self.mv(a, [(f[0] * -1.0) * 37.0, (f[1] * -1.0) * 37.0, (f[2] * -1.0) * 37.0], &mut hit);
                if hit.actor != self.native_object(a, crate::NativeField::Base) {
                    self.set_base_full(a, hit.actor, hit.normal, true);
                }
                continue;
            }
            self.mv(a, [f[0] * -8.0, f[1] * -8.0, f[2] * -8.0], &mut hit);
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            if !self.find_new_floor(a, l, dt, dt, iterations) {
                return;
            }
        }
        if self.pb(a, "bJustTeleported") {
            return;
        }
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let k = 1.0 / dt;
        self.set_v(a, "Velocity", [k * (l[0] - old_location[0]), (l[1] - old_location[1]) * k, (l[2] - old_location[2]) * k]);
    }

    /// APawn::findNewFloor: looks for a floor in the six directions; otherwise Falling,
    /// PHYS_Falling and physFalling for the remaining time. False if no floor is found.
    fn find_new_floor(&mut self, a: ObjId, old_location: V, dt: f32, remaining: f32, iterations: i32) -> bool {
        let mut hit = CheckResult::new(1.0);
        for dir in [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, -1.0]] {
            if self.check_floor(a, dir, &mut hit) {
                return true;
            }
        }
        self.event_pub(a, "Falling", vec![]);
        if self.propr(a, "Physics").as_byte() == PHYS_SPIDER {
            self.set_physics(a, PHYS_FALLING, None, [0.0, 0.0, 1.0]);
        }
        if self.propr(a, "Physics").as_byte() == PHYS_FALLING {
            let vz = self.native_prop(a, crate::NativeField::Velocity).as_vector()[2];
            if !self.pb(a, "bJustTeleported") && dt > remaining {
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                let v = div([l[0] - old_location[0], l[1] - old_location[1], l[2] - old_location[2]], dt - remaining);
                self.set_v(a, "Velocity", v);
            }
            let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            v[2] = vz;
            self.set_v(a, "Velocity", v);
            if remaining > 0.005 {
                self.phys_falling(a, remaining, iterations);
            }
        }
        false
    }

    /// APawn::checkFloor: trace of the cylinder over 35 units along −dir (without a
    /// source actor); a contact becomes the support (SetBase, Floor = normal).
    fn check_floor(&mut self, a: ObjId, dir: V, hit: &mut CheckResult) -> bool {
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let r = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
        let extent = [r, r, self.native_prop(a, crate::NativeField::CollisionHeight).as_float()];
        let end = [l[0] - dir[0] * MAXSTEPHEIGHT, l[1] - dir[1] * MAXSTEPHEIGHT, l[2] - dir[2] * MAXSTEPHEIGHT];
        self.single_line_check(hit, None, end, l, TRACE_WORLD, extent);
        if hit.time < 1.0 {
            self.set_base_full(a, hit.actor, hit.normal, true);
            return true;
        }
        false
    }

    /// APawn::SpiderstepUp.
    fn spider_step_up(&mut self, a: ObjId, desired_dir: V, delta: V, hit: &mut CheckResult) {
        let mut delta = delta;
        let floor = self.pv(a, "Floor");
        let down_of = |f: V| [(f[0] * -1.0) * MAXSTEPHEIGHT, (f[1] * -1.0) * MAXSTEPHEIGHT, (f[2] * -1.0) * MAXSTEPHEIGHT];
        let mut down = down_of(floor);
        if (((hit.normal[2] * floor[2] + hit.normal[1] * floor[1]) + floor[0] * hit.normal[0]) as f64) < 0.1 {
            // Wall: step up, then move forward.
            self.mv(a, [down[0] * -1.0, down[1] * -1.0, down[2] * -1.0], hit);
            self.mv(a, delta, hit);
        } else {
            // Slope: it becomes the floor.
            self.set_v(a, "Floor", hit.normal);
            down = down_of(hit.normal);
            let dist = (dot_xzy(delta, delta) as f64).sqrt() as f32;
            self.mv(a, [delta[0], delta[1], dist * hit.normal[2] + delta[2]], hit);
        }
        if hit.time < 1.0 {
            let floor = self.pv(a, "Floor");
            let along = (hit.normal[2] * floor[2] + hit.normal[1] * floor[1]) + floor[0] * hit.normal[0];
            if (along as f64) < 0.1 && dot_xzy(delta, delta) * hit.time > 144.0 {
                self.mv(a, down, hit);
                let k = 1.0 - hit.time;
                delta = [k * delta[0], delta[1] * k, delta[2] * k];
                self.spider_step_up(a, desired_dir, delta, hit);
                return;
            }
            // Sliding along the wall, in the frame of the old and the new floor.
            let old_floor = floor;
            let floor = hit.normal;
            self.set_v(a, "Floor", floor);
            down = down_of(floor);
            hit.normal[2] = 0.0;
            hit.normal = safe_normal(hit.normal);
            let old_hit_normal = hit.normal;
            let old_delta = delta;
            let mut c = cross(floor, old_floor);
            normalize(&mut c);
            let mut d = cross(c, old_floor);
            normalize(&mut d);
            let ka = dot_xzy(d, delta);
            let kb = dot_xzy(c, delta);
            let kc = dot_xzy(old_floor, delta);
            let e = cross(c, floor);
            delta = [(e[0] * ka + kb * c[0]) + kc * floor[0], (e[1] * ka + c[1] * kb) + kc * floor[1], (e[2] * ka + c[2] * kb) + kc * floor[2]];
            if (old_delta[1] * delta[1] + old_delta[2] * delta[2]) + delta[0] * old_delta[0] >= 0.0 {
                self.mv(a, delta, hit);
                if hit.time < 1.0 {
                    self.process_hit_wall(a, hit.normal, hit.actor);
                    if self.propr(a, "Physics").as_byte() == PHYS_FALLING {
                        return;
                    }
                    Self::two_wall_adjust(desired_dir, &mut delta, hit.normal, old_hit_normal, hit.time);
                    self.mv(a, delta, hit);
                }
            }
        }
        self.mv(a, down, hit);
    }
}

