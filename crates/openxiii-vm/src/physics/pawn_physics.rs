//! Pawn physics, reimplemented from Engine.dll: APawn::performPhysics, physWalking, physFalling,
//! physSwimming, physFlying, physLadder, calcVelocity, stepUp, CheckForLedges, processHitWall,
//! processLanded, Crouch / UnCrouch, physicsRotation (addresses at the top of each function).
//! physSpider is in spider_physics.rs.

use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;
use crate::NativeField as Field;

/// UCONST_MAXSTEPHEIGHT, MINFLOORZ, MAXSTEPSIDEZ (Engine.dll constants: 35, 0.7, 0.08).
const MAXSTEPHEIGHT: f32 = 35.0;
const MINFLOORZ: f32 = 0.7;
const MAXSTEPSIDEZ: f32 = 0.08;
/// Desired distance to the floor when walking (MINFLOORDIST 1.9, MAXFLOORDIST 2.4).
const MINFLOORDIST: f32 = 1.9;
const MAXFLOORDIST: f32 = 2.4;

/// FVector::SizeSquared: (X² + Y²) + Z², the form of all the calls and of the
/// inline copies in the functions reimplemented here, except APawn::stepUp (`size_sq_zyx`).
fn size_sq(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]
}
/// Inline |Delta|² of APawn::stepUp: (Z² + Y²) + X².
fn size_sq_zyx(a: V) -> f32 {
    (a[2] * a[2] + a[1] * a[1]) + a[0] * a[0]
}
/// FVector::Size2D (called from physWalking): sqrt(Y² + X²) in double.
pub(crate) fn size2d(a: V) -> f32 {
    ((a[1] * a[1] + a[0] * a[0]) as f64).sqrt() as f32
}
fn nearly_zero(a: V) -> bool {
    a.iter().all(|c| c.abs() < 0.0001)
}
fn is_zero(a: V) -> bool {
    a == [0.0; 3]
}

impl Vm {
    fn volume_v(&self, a: ObjId, name: &str, default: V) -> V {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).map_or(default, |v| self.pv(v, name))
    }
    pub(crate) fn volume_f(&self, a: ObjId, name: &str, default: f32) -> f32 {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).map_or(default, |v| self.pf(v, name))
    }
    fn volume_b(&self, a: ObjId, name: &str) -> bool {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).is_some_and(|v| self.pb(v, name))
    }
    fn volume_lv(&self, a: ObjId, field: Field, name: &str, default: V) -> V {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).map_or(default, |v| self.lv(v, field, name))
    }
    pub(crate) fn volume_lf(&self, a: ObjId, field: Field, name: &str, default: f32) -> f32 {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).map_or(default, |v| self.lf(v, field, name))
    }
    fn volume_lb(&self, a: ObjId, field: Field, name: &str) -> bool {
        self.native_object(a, crate::NativeField::PhysicsVolume).or(self.default_volume).is_some_and(|v| self.lb(v, field, name))
    }
    fn controller_lf(&self, a: ObjId, field: Field, name: &str) -> Option<f32> {
        self.native_object(a, crate::NativeField::Controller).map(|c| self.lf(c, field, name))
    }
    pub(crate) fn default_lf(&self, a: ObjId, field: Field, name: &str) -> f32 {
        self.objects[a].as_ref().map_or(0.0, |o| self.lp(self.classes[o.class].cdo, field, name).as_float())
    }
    pub(crate) fn gravity_of(&self, a: ObjId) -> V {
        self.volume_lv(a, Field::Gravity, "Gravity", [0.0, 0.0, -950.0])
    }
    /// GravDir of the physics functions: (0, 0, -1), or (0, 0, 1) if the zone's gravity points up.
    fn grav_dir(&self, a: ObjId) -> V {
        if self.gravity_of(a)[2] > 0.0 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 0.0, -1.0]
        }
    }
    fn controller_f(&self, a: ObjId, name: &str) -> Option<f32> {
        self.native_object(a, crate::NativeField::Controller).map(|c| self.pf(c, name))
    }
    fn set_move_timer(&mut self, a: ObjId, value: f32) {
        if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
            self.set_lf(c, Field::MoveTimer, "MoveTimer", value);
        }
    }
    fn physics_of(&self, a: ObjId) -> u8 {
        self.lp(a, Field::Physics, "Physics").as_byte()
    }
    /// GetLevel()->MoveActor(this, Delta, Rotation, Hit).
    pub(crate) fn mv(&mut self, a: ObjId, delta: V, hit: &mut CheckResult) -> bool {
        let r = self.lp(a, Field::Rotation, "Rotation").as_rotator();
        self.move_actor(a, delta, r, hit, false, false, false, false)
    }
    pub(crate) fn default_f(&self, a: ObjId, name: &str) -> f32 {
        self.objects[a].as_ref().map_or(0.0, |o| self.propr(self.classes[o.class].cdo, name).as_float())
    }

    /// APawn::performPhysics: crouching, physics of the current mode, rotation.
    /// With bCollideWorld outside any zone (Region.ZoneNumber zero): none of that, and
    /// FellOutOfWorld (with "%s fell out of the world!" in the log) except for a player.
    pub(crate) fn pawn_perform_physics(&mut self, a: ObjId, dt: f32) {
        if self.native_prop(a, crate::NativeField::CollideWorld).as_bool() && self.zone_number_of(a) == 0 {
            if !self.native_object(a, crate::NativeField::Controller).is_some_and(|c| self.native_prop(c, crate::NativeField::IsPlayer).as_bool()) {
                let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                self.log.push(format!("{name} fell out of the world!"));
                self.event_pub(a, "FellOutOfWorld", vec![]);
            }
            return;
        }
        let old_velocity = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        let z = self.native_prop(a, crate::NativeField::Location).as_vector()[2];
        self.set_native_prop(a, crate::NativeField::OldZ, Value::Float(z));
        let crouched = self.pp_is_crouched(a);
        self.set_native_prop(a, crate::NativeField::IsCrouched, Value::Bool(crouched));
        if self.physics_of(a) == PHYS_WALKING {
            if self.native_prop(a, crate::NativeField::WantsToCrouch).as_bool() {
                if !self.native_prop(a, crate::NativeField::IsCrouched).as_bool() {
                    self.pp_crouch(a);
                } else if self.native_prop(a, crate::NativeField::TryToUncrouch).as_bool() {
                    let t = self.native_prop(a, crate::NativeField::UncrouchTime).as_float() - dt;
                    self.set_native_prop(a, crate::NativeField::UncrouchTime, Value::Float(t));
                    if t <= 0.0 {
                        self.set_native_prop(a, crate::NativeField::WantsToCrouch, Value::Bool(false));
                        self.set_native_prop(a, crate::NativeField::TryToUncrouch, Value::Bool(false));
                    }
                }
            }
        } else if self.native_prop(a, crate::NativeField::IsCrouched).as_bool() {
            self.pp_uncrouch(a);
        }
        self.pp_start_new_physics(a, dt, 0);
        if !self.is_live(a) {
            return;
        }
        let physics = self.physics_of(a);
        self.set_native_prop(a, crate::NativeField::SimulateGravity, Value::Bool(physics == PHYS_FALLING || physics == PHYS_WALKING));
        if self.native_prop(a, crate::NativeField::IsCrouched).as_bool() && (physics != PHYS_WALKING || !self.native_prop(a, crate::NativeField::WantsToCrouch).as_bool()) {
            self.pp_uncrouch(a);
        }
        if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
            if !self.native_prop(c, crate::NativeField::PreparingMove).as_bool() {
                let t = self.native_prop(c, crate::NativeField::MoveTimer).as_float() - dt;
                self.set_native_prop(c, crate::NativeField::MoveTimer, Value::Float(t));
            }
            let desired = self.native_prop(a, crate::NativeField::DesiredRotation).as_rotator();
            let rotation = self.native_prop(a, crate::NativeField::Rotation).as_rotator();
            if !self.native_prop(a, crate::NativeField::Interpolating).as_bool()
                && (self.native_prop(a, crate::NativeField::Crawler).as_bool() || desired != rotation || self.native_prop(a, crate::NativeField::RotationRate).as_rotator()[2] > 0 || self.pp_is_human(a))
            {
                self.pp_rotation(a, dt, old_velocity);
            }
        }
        if !self.is_live(a) {
            return;
        }
        let accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        self.set_native_xyz(a, crate::NativeField::OldAcceleration, (accel).map(Value::Float), false);
        let avg = dt * 0.2 + self.native_prop(a, crate::NativeField::AvgPhysicsTime).as_float() * 0.8;
        self.set_native_prop(a, crate::NativeField::AvgPhysicsTime, Value::Float(avg));
        if let Some(pt) = self.native_object(a, crate::NativeField::PendingTouch) {
            self.event_pub(pt, "PostTouch", vec![Value::Object(Some(a))]);
            if let Some(pt) = self.native_object(a, crate::NativeField::PendingTouch) {
                let next = self.native_value(pt, crate::NativeField::PendingTouch);
                self.set_native_prop(a, crate::NativeField::PendingTouch, next);
                self.set_native_prop(pt, crate::NativeField::PendingTouch, Value::Object(None));
            }
        }
    }

    // Sub-functions of performPhysics: in tests, stubs can replace them.
    fn pp_is_crouched(&mut self, a: ObjId) -> bool {
        self.is_crouched(a)
    }

    fn pp_crouch(&mut self, a: ObjId) {
        self.crouch(a, false);
    }

    fn pp_uncrouch(&mut self, a: ObjId) {
        self.uncrouch(a, false);
    }

    fn pp_start_new_physics(&mut self, a: ObjId, dt: f32, iterations: i32) {
        self.start_new_physics(a, dt, iterations);
    }

    fn pp_rotation(&mut self, a: ObjId, dt: f32, old_velocity: V) {
        self.pawn_physics_rotation(a, dt, old_velocity);
    }

    fn pp_is_human(&mut self, a: ObjId) -> bool {
        self.is_human(a)
    }

    fn is_live(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some())
    }

    /// APawn::startNewPhysics.
    pub(crate) fn start_new_physics(&mut self, a: ObjId, dt: f32, iterations: i32) {
        if dt < 0.0003 || !self.is_live(a) {
            return;
        }
        match self.physics_of(a) {
            PHYS_WALKING if !stub_dispatch("walking", dt, iterations) => self.phys_walking(a, dt, iterations),
            PHYS_FALLING if !stub_dispatch("falling", dt, iterations) => self.phys_falling(a, dt, iterations),
            PHYS_SWIMMING if !stub_dispatch("swimming", dt, iterations) => self.phys_swimming(a, dt, iterations),
            PHYS_FLYING if !stub_dispatch("flying", dt, iterations) => self.phys_flying(a, dt, iterations),
            PHYS_LADDER if !stub_dispatch("ladder", dt, iterations) => self.phys_ladder(a, dt, iterations),
            PHYS_ROOT_MOTION if !stub_dispatch("root_motion", dt, 0) => self.phys_root_motion(a, dt),
            PHYS_SPIDER if !stub_dispatch("spider", dt, iterations) => self.phys_spider(a, dt, iterations),
            _ => {}
        }
    }

    /// GroundSpeed of physWalking, multiplied for the pawn of `cheat_speed` (cheat from the Android
    /// Edit panel, not in the original). Writing GroundSpeed is not enough: with a weapon in hand,
    /// XIIIPlayerController.PlayerWalking.ProcessMove resets it through SetGroundspeed every frame,
    /// before the physics.
    pub(crate) fn speed_walk(&self, a: ObjId) -> f32 {
        let gs = self.lf(a, Field::GroundSpeed, "GroundSpeed");
        match self.cheat_speed {
            Some((p, m)) if p == a => gs * m,
            _ => gs,
        }
    }

    /// APawn::calcVelocity.
    #[allow(clippy::too_many_arguments)]
    fn calc_velocity(&mut self, a: ObjId, accel_dir: V, dt: f32, max_speed: f32, friction: f32, fluid: bool, brake: bool, buoyant: bool) {
        let walking = self.lb(a, Field::IsWalking, "bIsWalking");
        let crouched = self.lb(a, Field::IsCrouched, "bIsCrouched");
        let physics = self.physics_of(a);
        let accel_rate = self.lf(a, Field::AccelRate, "AccelRate");
        let mut accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        if !brake || !is_zero(accel) {
            let vsize = size(v);
            if (!walking && !crouched) || physics == PHYS_LADDER {
                if accel_rate * accel_rate < size_sq(accel) {
                    accel = mul(accel_dir, accel_rate);
                }
            } else {
                let pct = self.lf(a, Field::WalkingPct, "WalkingPct");
                if pct * pct * accel_rate * accel_rate < size_sq(accel) {
                    accel = [0, 1, 2].map(|k| accel_dir[k] * accel_rate * pct);
                }
            }
            v = [0, 1, 2].map(|k| v[k] - (v[k] - accel_dir[k] * vsize) * dt * friction);
        } else {
            // Braking in steps of 0.03 s.
            let old = v;
            let mut sum = [0.0f32; 3];
            let mut remaining = dt;
            while remaining > 0.03 {
                v = [0, 1, 2].map(|k| v[k] - (v[k] + v[k]) * 0.03 * friction);
                if (old[1] * v[1] + old[2] * v[2]) + old[0] * v[0] > 0.0 {
                    // (V · 0.03) · (1 / dt).
                    let inv = 1.0 / dt;
                    sum = [0, 1, 2].map(|k| v[k] * 0.03 * inv + sum[k]);
                }
                remaining -= 0.03;
            }
            v = [0, 1, 2].map(|k| v[k] - (v[k] + v[k]) * remaining * friction);
            if (old[1] * v[1] + old[2] * v[2]) + old[0] * v[0] > 0.0 {
                // (Remainder · V) · (1 / dt).
                let inv = 1.0 / dt;
                sum = [0, 1, 2].map(|k| remaining * v[k] * inv + sum[k]);
            }
            v = sum;
            // (Old | V): (Y + Z) + X.
            if (old[1] * v[1] + old[2] * v[2]) + old[0] * v[0] < 0.0 || size_sq(v) < 100.0 {
                v = [0.0; 3];
            }
        }
        let k = 1.0 - f32::from(u8::from(fluid)) * dt * friction;
        v = [0, 1, 2].map(|i| k * v[i] + dt * accel[i]);
        let mut max_speed = max_speed;
        if !self.is_human(a) {
            max_speed *= self.lf(a, Field::DesiredSpeed, "DesiredSpeed");
        }
        if buoyant {
            let g = self.gravity_of(a);
            let f = 1.0 - self.lf(a, Field::Buoyancy, "Buoyancy") / self.lf(a, Field::Mass, "Mass");
            v = [0, 1, 2].map(|i| dt * g[i] * f + v[i]);
        }
        let cap = |v: V, m: f32| -> V {
            if size_sq(v) <= m * m {
                v
            } else {
                mul(safe_normal(v), m)
            }
        };
        let crouch_pct = self.lf(a, Field::CrouchingPct, "CrouchingPct");
        let walk_pct = self.lf(a, Field::WalkingPct, "WalkingPct");
        if physics == PHYS_LADDER {
            v = cap(v, max_speed);
        } else if crouched && !fluid && size_sq(v) > crouch_pct * crouch_pct * max_speed * max_speed {
            v = mul(safe_normal(v), max_speed * crouch_pct);
        } else if walking && !fluid && size_sq(v) > walk_pct * walk_pct * max_speed * max_speed {
            v = mul(safe_normal(v), max_speed * walk_pct);
        } else {
            v = cap(v, max_speed);
        }
        self.set_lv(a, Field::Acceleration, "Acceleration", accel);
        self.set_lv(a, Field::Velocity, "Velocity", v);
    }

    /// APawn::physWalking.
    fn phys_walking(&mut self, a: ObjId, dt: f32, mut iterations: i32) {
        if self.native_object(a, crate::NativeField::Controller).is_none() {
            return;
        }
        let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        v[2] = 0.0;
        self.set_native_xyz(a, crate::NativeField::Velocity, (v).map(Value::Float), false);
        let mut accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        accel[2] = 0.0;
        self.set_native_xyz(a, crate::NativeField::Acceleration, (accel).map(Value::Float), false);
        let accel_dir = if is_zero(accel) { accel } else { safe_normal(accel) };
        let (ground_speed, friction) = (self.speed_walk(a), self.volume_lf(a, Field::GroundFriction, "GroundFriction", 8.0));
        self.calc_velocity(a, accel_dir, dt, ground_speed, friction, false, true, false);
        let mut desired_move = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        let zone = self.volume_lv(a, Field::ZoneVelocity, "ZoneVelocity", [0.0; 3]);
        if size_sq(zone) > 0.0 && (self.is_human(a) || size_sq(zone) > 90000.0) {
            // (ZoneVelocity · 25) · DeltaTime.
            desired_move = add(desired_move, mul(mul(zone, 25.0), dt));
        }
        desired_move[2] = 0.0;
        if nearly_zero(desired_move) && self.native_object(a, crate::NativeField::Base).is_some_and(|b| self.native_prop(b, crate::NativeField::Static).as_bool()) {
            return;
        }
        let grav_dir = self.grav_dir(a);
        let down = [0.0, 0.0, grav_dir[2] * 37.0];
        let mut hit = CheckResult::new(1.0);
        let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        self.set_native_prop(a, crate::NativeField::JustTeleported, Value::Bool(false));
        let mut checked_fall = false;
        let mut must_jump = false;
        let mut remaining = dt;
        loop {
            if remaining <= 0.0 || iterations > 7 || self.native_object(a, crate::NativeField::Controller).is_none() || !self.is_live(a) {
                if !self.is_live(a) {
                    return;
                }
                if !self.native_prop(a, crate::NativeField::JustTeleported).as_bool() && !self.native_prop(a, crate::NativeField::NoVelocityUpdate).as_bool() {
                    let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                    let k = 1.0 / dt;
                    self.set_native_xyz(a, crate::NativeField::Velocity, ([k * (l[0] - old_location[0]), (l[1] - old_location[1]) * k, (l[2] - old_location[2]) * k]).map(Value::Float), false);
                }
                self.set_native_prop(a, crate::NativeField::NoVelocityUpdate, Value::Bool(false));
                let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                v[2] = 0.0;
                self.set_native_xyz(a, crate::NativeField::Velocity, (v).map(Value::Float), false);
                return;
            }
            iterations += 1;
            let radius = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
            let time_tick = if remaining > 0.05 && (self.is_human(a) || size_sq(desired_move) * remaining * remaining > (radius * radius).min(1225.0)) {
                (remaining * 0.5).min(0.05)
            } else {
                remaining
            };
            remaining -= time_tick;
            let mut delta = mul(desired_move, time_tick);
            let sub_loc = self.native_prop(a, crate::NativeField::Location).as_vector();
            if nearly_zero(delta) {
                remaining = 0.0;
            } else {
                if self.native_object(a, crate::NativeField::Controller).is_some()
                    && (!self.is_human(a) || (self.native_prop(a, crate::NativeField::IsWalking).as_bool() && !self.native_prop(a, crate::NativeField::CanWalkOffLedges).as_bool()))
                    && !self.native_prop(a, crate::NativeField::CanFly).as_bool()
                {
                    delta = self.check_for_ledges(a, accel_dir, delta, grav_dir, &mut checked_fall, &mut must_jump, time_tick);
                    if self.controller_lf(a, Field::MoveTimer, "MoveTimer") == Some(-1.0) {
                        remaining = 0.0;
                    }
                }
                self.mv(a, delta, &mut hit);
                if !self.is_live(a) {
                    return;
                }
                if hit.time < 1.0 {
                    let desired_dir = safe_normal(delta);
                    self.step_up(a, grav_dir, desired_dir, mul(delta, 1.0 - hit.time), &mut hit);
                    if !self.is_live(a) {
                        return;
                    }
                    if self.physics_of(a) == PHYS_FALLING {
                        let desired = size(delta);
                        let actual = size2d(sub(self.native_prop(a, crate::NativeField::Location).as_vector(), sub_loc));
                        let mut frac = actual / desired;
                        if frac >= 1.0 {
                            frac = 1.0;
                        }
                        remaining += (1.0 - frac) * time_tick;
                        self.event_pub(a, "Falling", vec![]);
                        if self.physics_of(a) == PHYS_FLYING {
                            let air = self.native_prop(a, crate::NativeField::AirSpeed).as_float();
                            self.set_native_xyz(a, crate::NativeField::Velocity, ([0.0, 0.0, air]).map(Value::Float), false);
                            let rate = self.native_prop(a, crate::NativeField::AccelRate).as_float();
                            self.set_native_xyz(a, crate::NativeField::Acceleration, ([0.0, 0.0, rate]).map(Value::Float), false);
                        }
                        self.start_new_physics(a, remaining, iterations);
                        return;
                    }
                }
                if self.physics_of(a) == PHYS_SWIMMING {
                    let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                    self.start_swimming(a, old_location, v, time_tick, remaining, iterations);
                    return;
                }
            }
            // Floor search, 37 units below the feet.
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            let ext = self.cyl_extent(a);
            self.single_line_check(&mut hit, Some(a), add(l, down), l, 0x10df, ext);
            if hit.time < 1.0 {
                self.set_last_collided_material(a, hit.material);
            }
            if (hit.time as f64) < 1.0 {
                let floor_dist = (hit.time as f64 * 37.0) as f32;
                self.set_native_xyz(a, crate::NativeField::Floor, (hit.normal).map(Value::Float), false);
                // (Delta | Normal): (Z + Y) + X.
                let n = hit.normal;
                if hit.normal[2] >= MINFLOORZ || nearly_zero(delta) || (delta[2] * n[2] + n[1] * delta[1]) + n[0] * delta[0] >= 0.0 {
                    if hit.time >= 1.0 || (hit.actor == self.native_object(a, crate::NativeField::Base) && floor_dist <= MAXFLOORDIST) {
                        if floor_dist < MINFLOORDIST {
                            let normal = hit.normal;
                            self.mv(a, [0.0, 0.0, (MINFLOORDIST + MAXFLOORDIST) * 0.5 - floor_dist], &mut hit);
                            hit.time = 0.0;
                            hit.normal = normal;
                        }
                    } else {
                        self.mv(a, down, &mut hit);
                        if hit.actor != self.native_object(a, crate::NativeField::Base) && self.physics_of(a) == PHYS_WALKING {
                            self.set_base_full(a, hit.actor, hit.normal, true);
                        }
                    }
                } else {
                    // Slope too steep under the feet: we slide down.
                    let n = hit.normal;
                    let k = n[2] * 35.0 + (n[1] + n[0]) * 0.0;
                    let slide = [k * n[0], n[1] * k, -(35.0 - n[2] * k)];
                    self.mv(a, slide, &mut hit);
                    if hit.actor != self.native_object(a, crate::NativeField::Base) && self.physics_of(a) == PHYS_WALKING {
                        self.set_base_full(a, hit.actor, hit.normal, true);
                    }
                }
            }
            if !self.is_live(a) {
                return;
            }
            if self.physics_of(a) == PHYS_SWIMMING {
                let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                self.start_swimming(a, old_location, v, time_tick, remaining, iterations);
                return;
            }
            let no_floor = hit.time >= 1.0 || hit.normal[2] < MINFLOORZ;
            if !must_jump && no_floor && self.native_prop(a, crate::NativeField::CanJump).as_bool() && !checked_fall {
                if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
                    if self.is_probing(c, "MayFall") {
                        checked_fall = true;
                        // AController::eventMayFall.
                        self.event_pub(c, "MayFall", vec![]);
                    }
                }
            }
            if must_jump || no_floor {
                if !self.native_prop(a, crate::NativeField::JustTeleported).as_bool() && !must_jump && (!self.native_prop(a, crate::NativeField::CanJump).as_bool() || (self.native_prop(a, crate::NativeField::IsWalking).as_bool() && !self.native_prop(a, crate::NativeField::CanWalkOffLedges).as_bool())) {
                    // The pawn must not fall: back to the starting position.
                    self.set_native_xyz(a, crate::NativeField::Velocity, ([0.0; 3]).map(Value::Float), false);
                    self.set_native_xyz(a, crate::NativeField::Acceleration, ([0.0; 3]).map(Value::Float), false);
                    self.far_move_actor(a, old_location, false, false, false);
                    self.set_move_timer(a, -1.0);
                    return;
                }
                let desired = size(delta);
                if desired == 0.0 {
                    remaining = 0.0;
                } else {
                    let actual = size2d(sub(self.native_prop(a, crate::NativeField::Location).as_vector(), sub_loc));
                    let mut frac = actual / desired;
                    if frac >= 1.0 {
                        frac = 1.0;
                    }
                    remaining += (1.0 - frac) * time_tick;
                }
                let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                v[2] = 0.0;
                self.set_native_xyz(a, crate::NativeField::Velocity, (v).map(Value::Float), false);
                self.event_pub(a, "Falling", vec![]);
                if self.physics_of(a) == PHYS_WALKING {
                    self.set_physics(a, PHYS_FALLING, None, [0.0, 0.0, 1.0]);
                }
                self.start_new_physics(a, remaining, iterations);
                return;
            }
            let ground_friction = self.volume_lf(a, Field::GroundFriction, "GroundFriction", 8.0);
            if hit.normal[2] < 1.0 && hit.normal[2] * ground_friction < 3.3 {
                // Slippery floor: gravity drags the pawn along the slope.
                let g = self.gravity_of(a);
                let f = ground_friction.max(0.5);
                let slide_v = mul(mul(mul(g, dt), 1.0 / (f + f)), dt);
                let n = hit.normal;
                // (SlideV | N): (Z + Y) + X; (Slide | SlideV): (Y + Z) + X.
                let k = (slide_v[2] * n[2] + slide_v[1] * n[1]) + slide_v[0] * n[0];
                let slide = sub(slide_v, mul(n, k));
                if (slide_v[1] * slide[1] + slide_v[2] * slide[2]) + slide[0] * slide_v[0] >= 0.0 {
                    self.mv(a, slide, &mut hit);
                }
                if self.physics_of(a) == PHYS_SWIMMING {
                    let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                    self.start_swimming(a, old_location, v, time_tick, remaining, iterations);
                    return;
                }
            }
        }
    }

    pub(crate) fn set_last_collided_material(&mut self, a: ObjId, material: Option<u32>) {
        if self.prop_slot(a, "LastCollidedMaterial").is_err() {
            return;
        }
        let value = match material.and_then(|t| self.collision_materials.get(t as usize)) {
            Some(m) if !m.is_empty() => Value::Asset(m.clone()),
            _ => Value::Object(None),
        };
        self.set_prop(a, "LastCollidedMaterial", value);
    }

    /// APawn::CheckForLedges: keeps the pawn from walking into the void (AI, or a player
    /// walking without bCanWalkOffLedges). Returns the corrected movement.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_for_ledges(&mut self, a: ObjId, accel_dir: V, delta: V, grav_dir: V, checked_fall: &mut bool, must_jump: &mut bool, time_tick: f32) -> V {
        let mut delta = delta;
        let r = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
        let h = self.native_prop(a, crate::NativeField::CollisionHeight).as_float();
        let cyl = self.cyl_extent(a);
        let mut side = mul(accel_dir, r);
        if !self.native_prop(a, crate::NativeField::AvoidLedges).as_bool() {
            side = mul(side, 0.5);
        }
        let location = self.native_prop(a, crate::NativeField::Location).as_vector();
        let zero = [0.0f32; 3];
        let mut hit = CheckResult::new(1.0);
        let p = add(add(location, delta), side);
        self.single_line_check(&mut hit, Some(a), p, location, 0x2df, zero);
        if hit.time != 1.0 {
            return delta;
        }
        let delta_size = size(delta);
        let mut d = delta_size + r;
        if d <= MAXSTEPHEIGHT {
            d = MAXSTEPHEIGHT;
        }
        let dist = d + h + 4.0;
        self.single_line_check(&mut hit, Some(a), add(p, mul(grav_dir, dist)), p, 0xdf, zero);
        let mut m = r * 0.5;
        if m >= 14.0 {
            m = 14.0;
        }
        let deep = hit.time == 1.0
            || hit.normal[2] < MINFLOORZ
            || ((h as f64 + 39.0) < (dist * hit.time) as f64 && {
                let s = ((1.0 - hit.normal[2] * hit.normal[2]) as f64).sqrt();
                (h + 4.0) as f64 + (delta_size + r) as f64 * s / (hit.normal[2] as f64) < (dist * hit.time) as f64
            });
        if deep {
            self.single_line_check(&mut hit, Some(a), add(p, mul(grav_dir, 39.0)), p, 0x2df, [m, m, h]);
        }
        if hit.time != 1.0 {
            return delta;
        }
        // Ledge: where is the wall below the edge?
        let dir = mul(delta, 1.0 / delta_size);
        let p2 = add(add(location, mul(accel_dir, delta_size)), side);
        let down = mul(grav_dir, h + 6.0);
        self.single_line_check(&mut hit, Some(a), sub(add(location, down), mul(accel_dir, r + r)), add(p2, down), TRACE_WORLD, zero);
        let grav41 = mul(grav_dir, 41.0);
        let mut floor_ok = false;
        let mut try_fall;
        if hit.time < 1.0 {
            if self.native_prop(a, crate::NativeField::AvoidLedges).as_bool() {
                let s = -(self.native_prop(a, crate::NativeField::GroundSpeed).as_float() * self.native_prop(a, crate::NativeField::DesiredSpeed).as_float() * time_tick);
                delta = [s * hit.normal[0], s * hit.normal[1], 0.0 * s];
                if self.native_prop(a, crate::NativeField::StopAtLedges).as_bool() {
                    self.set_move_timer(a, -1.0);
                } else if let Some(t) = self.controller_lf(a, Field::MoveTimer, "MoveTimer") {
                    self.set_move_timer(a, t - 0.25);
                }
                try_fall = false;
            } else {
                let mut wall = safe_normal([hit.normal[1], -hit.normal[0], 0.0]);
                // (AccelDir | Wall): (Y + Z) + X.
                if (accel_dir[1] * wall[1] + accel_dir[2] * wall[2]) + accel_dir[0] * wall[0] < 0.0 {
                    wall = mul(wall, -1.0);
                }
                let along = ((accel_dir[1] * wall[1] + accel_dir[2] * wall[2]) + accel_dir[0] * wall[0]) as f64;
                try_fall = along < 0.5 || (self.native_prop(a, crate::NativeField::CanJump).as_bool() && along < 0.7);
                let speed = if along < 0.7 {
                    self.native_prop(a, crate::NativeField::DesiredSpeed).as_float().min(0.8) * self.native_prop(a, crate::NativeField::GroundSpeed).as_float() * time_tick
                } else {
                    self.native_prop(a, crate::NativeField::GroundSpeed).as_float() * self.native_prop(a, crate::NativeField::DesiredSpeed).as_float() * time_tick
                };
                delta = mul(wall, speed);
            }
        } else {
            try_fall = true;
            let perp = [dir[1], -dir[0], 0.0];
            let mut ds = self.native_prop(a, crate::NativeField::DesiredSpeed).as_float();
            if ds >= 0.8 {
                ds = 0.8;
            }
            let speed = ds * self.native_prop(a, crate::NativeField::GroundSpeed).as_float() * time_tick;
            delta = [perp[0] * speed, perp[1] * speed, 0.0 * speed];
            let p4 = add(location, delta);
            self.single_line_check(&mut hit, Some(a), p4, location, 0x2df, cyl);
            floor_ok = true;
            if hit.time == 1.0 {
                self.single_line_check(&mut hit, Some(a), add(p4, grav41), p4, 0x2df, cyl);
                if hit.time == 1.0 {
                    delta = mul(delta, -1.0);
                    floor_ok = false;
                }
            }
        }
        // "(IsHumanControlled() != 0) && (bTryFall = false, bFloorFailed == 0)": the player never falls
        // here, even with a floor.
        if self.is_human(a) {
            try_fall = false;
        }
        if self.is_human(a) && !floor_ok {
            let p5 = add(add(location, delta), side);
            self.single_line_check(&mut hit, Some(a), p5, location, 0x2df, cyl);
            if hit.time == 1.0 {
                self.single_line_check(&mut hit, Some(a), add(p5, grav41), p5, 0x2df, [m, m, h]);
                if hit.time == 1.0 {
                    self.set_native_xyz(a, crate::NativeField::Acceleration, ([0.0; 3]).map(Value::Float), false);
                    delta = [0.0; 3];
                }
            }
        }
        if self.native_prop(a, crate::NativeField::CanJump).as_bool() {
            if try_fall {
                match self.native_object(a, crate::NativeField::Controller) {
                    Some(c) if self.is_probing(c, "MayFall") => {
                        if !*checked_fall {
                            *checked_fall = true;
                            try_fall = false;
                            self.event_pub(c, "MayFall", vec![]);
                            if self.native_prop(a, crate::NativeField::CanJump).as_bool() {
                                *must_jump = true;
                                delta = mul(accel_dir, delta_size);
                            }
                        }
                    }
                    _ => delta = mul(accel_dir, delta_size),
                }
            }
            if self.native_prop(a, crate::NativeField::CanJump).as_bool() {
                return delta;
            }
        }
        if try_fall {
            let location = self.native_prop(a, crate::NativeField::Location).as_vector();
            let p6 = add(location, mul(dir, delta_size + r));
            self.single_line_check(&mut hit, Some(a), p6, location, 0x2df, cyl);
            if hit.time == 1.0 {
                self.single_line_check(&mut hit, Some(a), add(p6, grav41), p6, 0x2df, cyl);
                if hit.time < 1.0 {
                    let p7 = add(location, mul(dir, delta_size));
                    self.single_line_check(&mut hit, Some(a), add(p7, grav41), p7, 0x2df, cyl);
                }
            }
            if hit.time < 1.0 {
                return mul(dir, delta_size);
            }
            if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
                let roll = self.rand();
                if time_tick + time_tick <= roll {
                    let t = self.native_prop(c, crate::NativeField::MoveTimer).as_float() - 0.1;
                    self.set_native_prop(c, crate::NativeField::MoveTimer, Value::Float(t));
                } else {
                    self.set_native_prop(c, crate::NativeField::MoveTimer, Value::Float(-1.0));
                }
            }
        }
        if floor_ok {
            return delta;
        }
        let location = self.native_prop(a, crate::NativeField::Location).as_vector();
        let p8 = add(add(location, delta), side);
        self.single_line_check(&mut hit, Some(a), p8, location, 0x2df, cyl);
        if hit.time == 1.0 {
            self.single_line_check(&mut hit, Some(a), add(p8, grav41), p8, 0x2df, cyl);
        } else if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
            // (Dir | Normal): (Y + Z) + X.
            let n = hit.normal;
            if (dir[1] * n[1] + dir[2] * n[2]) + dir[0] * n[0] < self.native_prop(c, crate::NativeField::MinHitWall).as_float() {
                self.set_native_prop(c, crate::NativeField::MoveTimer, Value::Float(-1.0));
            }
        }
        if hit.time != 1.0 {
            return delta;
        }
        self.single_line_check(&mut hit, Some(a), add(location, grav41), location, 0x2df, [m, m, h]);
        self.set_move_timer(a, -1.0);
        self.set_native_xyz(a, crate::NativeField::Acceleration, ([0.0; 3]).map(Value::Float), false);
        if hit.time == 1.0 {
            mul(dir, -(time_tick * self.native_prop(a, crate::NativeField::GroundSpeed).as_float()))
        } else {
            [0.0; 3]
        }
    }

    /// AActor::TwoWallAdjust: corner of 90° or less ((HitNormal | OldHitNormal)
    /// ≤ 0): along the edge (normalized cross product); otherwise sliding along the
    /// new wall, nothing if it no longer goes towards DesiredDir. Sums of the dot products in
    /// machine-code order.
    pub(crate) fn two_wall_adjust(desired_dir: V, delta: &mut V, hit_normal: V, old_hit_normal: V, hit_time: f32) {
        let (h, o, d) = (hit_normal, old_hit_normal, *delta);
        if (h[1] * o[1] + o[2] * h[2]) + h[0] * o[0] <= 0.0 {
            let c = [h[1] * o[2] - h[2] * o[1], o[0] * h[2] - h[0] * o[2], h[0] * o[1] - h[1] * o[0]];
            let n = safe_normal(c);
            let k = ((n[0] * d[0] + n[2] * d[2]) + n[1] * d[1]) * (1.0 - hit_time);
            *delta = [n[0] * k, n[1] * k, n[2] * k];
            let e = *delta;
            if (desired_dir[2] * e[2] + desired_dir[1] * e[1]) + desired_dir[0] * e[0] < 0.0 {
                *delta = [e[0] * -1.0, e[1] * -1.0, e[2] * -1.0];
            }
        } else {
            let dd = (d[0] * h[0] + h[1] * d[1]) + d[2] * h[2];
            let t = 1.0 - hit_time;
            *delta = [(d[0] - dd * h[0]) * t, (d[1] - dd * h[1]) * t, (d[2] - dd * h[2]) * t];
            let e = *delta;
            if (desired_dir[2] * e[2] + desired_dir[1] * e[1]) + desired_dir[0] * e[0] <= 0.0 {
                *delta = [0.0; 3];
            }
        }
    }

    /// APawn::stepUp: climbs a step (or follows a slope); against a wall, slides.
    /// For an actor that is not a pawn, AActor::stepUp: without the case of slopes
    /// too steep when walking or that of decorations to push.
    fn step_up(&mut self, a: ObjId, grav_dir: V, desired_dir: V, delta: V, hit: &mut CheckResult) {
        let down = mul(grav_dir, MAXSTEPHEIGHT);
        let mut delta = delta;
        let pawn = self.is_a(a, "Pawn");
        if hit.normal[2].abs() < MAXSTEPSIDEZ {
            // Vertical wall: step up, then move forward.
            self.mv(a, mul(down, -1.0), hit);
            self.mv(a, delta, hit);
        } else if !(pawn && hit.normal[2] < MINFLOORZ && self.physics_of(a) == PHYS_WALKING) {
            // Walkable slope: we follow its rise.
            // Inline |Delta|: appSqrt of (Z² + Y²) + X².
            let up = (size_sq_zyx(delta) as f64).sqrt() as f32 * hit.normal[2] + delta[2];
            self.mv(a, [delta[0], delta[1], up], hit);
        }
        if !self.is_live(a) {
            return;
        }
        if hit.time < 1.0 {
            let pushable = pawn
                && self.is_human(a)
                && hit.actor.is_some_and(|o| self.is_a(o, "Decoration") && self.native_prop(o, crate::NativeField::Pushable).as_bool())
                // (DesiredDir | Normal): (Y + Z) + X.
                && ((((desired_dir[1] * hit.normal[1] + desired_dir[2] * hit.normal[2]) + desired_dir[0] * hit.normal[0]) as f64) < -0.9);
            if pushable {
                let o = hit.actor.unwrap();
                self.set_native_prop(a, crate::NativeField::NoVelocityUpdate, Value::Bool(true));
                let k = self.native_prop(a, crate::NativeField::Mass).as_float() / (self.native_prop(o, crate::NativeField::Mass).as_float() + self.native_prop(a, crate::NativeField::Mass).as_float());
                let v = mul(self.native_prop(a, crate::NativeField::Velocity).as_vector(), k);
                self.set_native_xyz(a, crate::NativeField::Velocity, (v).map(Value::Float), false);
                self.process_hit_wall(a, hit.normal, hit.actor);
                if self.physics_of(a) == PHYS_FALLING {
                    return;
                }
            } else if hit.normal[2].abs() < MAXSTEPSIDEZ && size_sq_zyx(delta) * hit.time > 144.0 {
                // Another step: go back down, then start again with the rest of the path.
                self.mv(a, down, hit);
                let rest = mul(delta, 1.0 - hit.time);
                self.step_up(a, grav_dir, desired_dir, rest, hit);
                return;
            }
            self.process_hit_wall(a, hit.normal, hit.actor);
            if self.physics_of(a) == PHYS_FALLING || !self.is_live(a) {
                return;
            }
            // Sliding along the wall (normal brought back to horizontal).
            hit.normal[2] = 0.0;
            hit.normal = safe_normal(hit.normal);
            let old_hit_normal = hit.normal;
            let original = delta;
            // (Delta | Normal): (Z + Y) + X; (Delta | Original): (Y + Z) + X.
            let n = hit.normal;
            let k = (delta[2] * n[2] + delta[1] * n[1]) + delta[0] * n[0];
            delta = mul(sub(delta, mul(hit.normal, k)), 1.0 - hit.time);
            if (original[1] * delta[1] + original[2] * delta[2]) + delta[0] * original[0] >= 0.0 {
                self.mv(a, delta, hit);
                if hit.time < 1.0 {
                    self.process_hit_wall(a, hit.normal, hit.actor);
                    if self.physics_of(a) == PHYS_FALLING || !self.is_live(a) {
                        return;
                    }
                    Self::two_wall_adjust(desired_dir, &mut delta, hit.normal, old_hit_normal, hit.time);
                    self.mv(a, delta, hit);
                }
            }
        }
        self.mv(a, down, hit);
    }

    /// APawn::processHitWall; AActor::processHitWall for the others.
    pub(crate) fn process_hit_wall(&mut self, a: ObjId, normal: V, actor: Option<ObjId>) {
        let Some(other) = actor else { return };
        if self.is_a(other, "Pawn") {
            return;
        }
        let nv = |n: V| Value::vector(n[0], n[1], n[2]);
        if !self.is_a(a, "Pawn") {
            self.event_pub(a, "HitWall", vec![nv(normal), Value::Object(Some(other))]);
            return;
        }
        if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
            if is_zero(self.native_prop(a, crate::NativeField::Acceleration).as_vector()) {
                return;
            }
            let mut dir = safe_normal(sub(self.lv(c, Field::Destination, "Destination"), self.native_prop(a, crate::NativeField::Location).as_vector()));
            let mut n = normal;
            let physics = self.physics_of(a);
            if physics == PHYS_WALKING {
                n[2] = 0.0;
                dir[2] = 0.0;
            }
            // (Normal | Dir): (Y + X) + Z.
            if (n[1] * dir[1] + n[0] * dir[0]) + dir[2] * n[2] > self.lf(c, Field::MinHitWall, "MinHitWall") {
                return;
            }
            // AController::eventNotifyHitWall.
            if self.event_ret(c, "NotifyHitWall", vec![nv(n), Value::Object(Some(other))]) {
                return;
            }
            if physics != PHYS_FALLING {
                let crouch_walk = physics == PHYS_WALKING && !self.pp_is_human(a) && self.lb(a, Field::CanCrouch, "bCanCrouch") && !self.pp_is_crouched(a);
                if crouch_walk {
                    let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                    let ahead = add(l, mul(dir, self.native_prop(a, crate::NativeField::CollisionRadius).as_float()));
                    if self.can_crouch_walk(a, l, ahead) {
                        return;
                    }
                }
                let mut h = CheckResult::new(1.0);
                self.mv(a, [0.0, 0.0, -35.0], &mut h);
                if crouch_walk {
                    let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                    let ahead = add(l, mul(dir, self.native_prop(a, crate::NativeField::CollisionRadius).as_float()));
                    if self.can_crouch_walk(a, l, ahead) {
                        return;
                    }
                }
                self.adjust_from_wall(c, n, other);
            }
            self.event_pub(a, "HitWall", vec![nv(n), Value::Object(Some(other))]);
            return;
        }
        self.event_pub(a, "HitWall", vec![nv(normal), Value::Object(Some(other))]);
    }

    /// AAIController::AdjustFromWall, during a MoveTo / MoveToward: against the door
    /// of the target point, SuggestMovePreparation or NotifyHitMover; otherwise detour
    /// (PickWallAdjust), and failing that the movement stops (MoveTimer = -1).
    fn adjust_from_wall(&mut self, c: ObjId, normal: V, other: ObjId) {
        // Other controllers: AController::AdjustFromWall does nothing.
        if !self.is_a(c, "AIController") || !self.lb(c, Field::AdjustFromWalls, "bAdjustFromWalls") {
            return;
        }
        let latent = self.latent_action(c);
        if latent != crate::ai_move::AI_POLL_MOVE_TO && latent != crate::ai_move::AI_POLL_MOVE_TOWARD {
            return;
        }
        let pawn = self.native_object(c, crate::NativeField::Pawn);
        if let (Some(p), Some(mt)) = (pawn, self.po(c, "MoveTarget")) {
            if self.is_a(other, "Mover") && self.associated_level_geometry(mt) == Some(other) {
                if self.is_a(mt, "NavigationPoint") && self.event_ret(mt, "SuggestMovePreparation", vec![Value::Object(Some(p))]) {
                    return;
                }
                // AController::eventNotifyHitMover.
                self.event_pub(c, "NotifyHitMover", vec![Value::vector(normal[0], normal[1], normal[2]), Value::Object(Some(other))]);
                return;
            }
        }
        if !self.lb(c, Field::Adjusting, "bAdjusting") {
            let t = self.lb(c, Field::TacticalDir, "bTacticalDir");
            self.set_lb(c, Field::TacticalDir, "bTacticalDir", !t);
            if pawn.is_some_and(|p| self.pick_wall_adjust(p, normal)) {
                return;
            }
        }
        self.set_lf(c, Field::MoveTimer, "MoveTimer", -1.0);
    }

    fn can_crouch_walk(&mut self, a: ObjId, start: V, end: V) -> bool {
        let lower = self.lf(a, Field::CrouchHeight, "CrouchHeight") - self.native_prop(a, crate::NativeField::CollisionHeight).as_float();
        let mut hit = CheckResult::new(1.0);
        let (s, e) = ([start[0], start[1], start[2] + lower], [end[0], end[1], end[2] + lower]);
        self.single_line_check(&mut hit, Some(a), e, s, 0x286, [0.0; 3]);
        if hit.actor.is_some() {
            return false;
        }
        let ext = [self.lf(a, Field::CrouchRadius, "CrouchRadius"), self.lf(a, Field::CrouchRadius, "CrouchRadius"), self.lf(a, Field::CrouchHeight, "CrouchHeight")];
        let (s, e) = ([end[0], end[1], end[2] + lower], [start[0], start[1], start[2] + lower]);
        self.single_line_check(&mut hit, Some(a), s, e, TRACE_WORLD, ext);
        if hit.time == 1.0 {
            self.set_lb(a, Field::WantsToCrouch, "bWantsToCrouch", true);
            self.set_lb(a, Field::TryToUncrouch, "bTryToUncrouch", true);
            self.set_lf(a, Field::UncrouchTime, "UncrouchTime", 0.5);
            return true;
        }
        false
    }

    /// APawn::processLanded.
    pub(crate) fn process_landed(&mut self, a: ObjId, normal: V, actor: Option<ObjId>, remaining: f32, iterations: i32) {
        let mut hit = CheckResult::new(1.0);
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let below = [l[0], l[1], l[2] - (0.2 * self.native_prop(a, crate::NativeField::CollisionHeight).as_float() + 8.0)];
        let extent = mul(self.cyl_extent(a), 0.9);
        if !pl_trace(&mut hit, below, l, extent) {
            self.single_line_check(&mut hit, Some(a), below, l, 0x297, extent);
        }
        if hit.time == 1.0 {
            // Nothing under the pawn (resting on an edge): it is pushed towards a free spot.
            let mut adjusted = l;
            let extent_spot = mul(self.cyl_extent(a), 1.1);
            let found = match pl_spot(extent_spot, &mut adjusted) {
                Some(r) => r,
                None => self.find_spot(extent_spot, &mut adjusted),
            };
            if found && adjusted != l {
                if !pl_op(|| format!("M{:08x},{:08x},{:08x},000", adjusted[0].to_bits(), adjusted[1].to_bits(), adjusted[2].to_bits())) {
                    self.far_move_actor(a, adjusted, false, false, false);
                }
                let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                v[0] += self.rand() * 60.0 - 30.0;
                v[1] += self.rand() * 60.0 - 30.0;
                self.set_lv(a, Field::Velocity, "Velocity", v);
                return;
            }
        }
        self.set_lv(a, Field::Floor, "Floor", normal);
        let nv = Value::vector(normal[0], normal[1], normal[2]);
        let bits = |n: V| format!("{:08x},{:08x},{:08x}", n[0].to_bits(), n[1].to_bits(), n[2].to_bits());
        let handled = match self.native_object(a, crate::NativeField::Controller) {
            // AController::eventNotifyLanded.
            Some(c) => match pl_notify(|| format!("N{}", bits(normal))) {
                Some(r) => r,
                None => self.event_ret(c, "NotifyLanded", vec![nv.clone()]),
            },
            None => false,
        };
        if !handled && !pl_op(|| format!("L{}", bits(normal))) {
            self.event_pub(a, "Landed", vec![nv]);
        }
        if !self.is_live(a) {
            return;
        }
        if self.physics_of(a) == PHYS_FALLING {
            let next = if self.lp(a, Field::Health, "Health").as_int() < 1 { PHYS_NONE } else { PHYS_WALKING };
            self.pl_set_physics(a, next, actor, normal);
        }
        if self.physics_of(a) == PHYS_WALKING {
            let acc = safe_normal(self.native_prop(a, crate::NativeField::Acceleration).as_vector());
            self.set_lv(a, Field::Acceleration, "Acceleration", acc);
        }
        // Switched to PHYS_Spider by Landed, the pawn falls again (setPhysics(PHYS_Falling)).
        // XIIIPawn.LandedSpecial uses this for a body resting on another pawn or with no floor below
        // ("Trick to keep the PHYS_Falling in the c++Code"): it bounces then falls again and settles
        // on the ground.
        if self.physics_of(a) == PHYS_SPIDER {
            self.pl_set_physics(a, PHYS_FALLING, actor, normal);
        }
        if !pl_op(|| format!("R{:08x},{iterations}", remaining.to_bits())) {
            self.start_new_physics(a, remaining, iterations);
        }
    }

    /// setPhysics of processLanded (vtable +0xF8); in the processLanded test, recorded and reduced to writing Physics.
    fn pl_set_physics(&mut self, a: ObjId, physics: u8, floor: Option<ObjId>, normal: V) {
        if pl_op(|| format!("P{physics},{},{:08x},{:08x},{:08x}", if floor.is_some() { "A" } else { "0" }, normal[0].to_bits(), normal[1].to_bits(), normal[2].to_bits())) {
            self.set_prop(a, "Physics", Value::Byte(physics));
            return;
        }
        self.set_physics(a, physics, floor, normal);
    }

    /// APawn::NewFallVelocity.
    fn new_fall_velocity(&self, a: ObjId, old: V, accel: V, dt: f32) -> V {
        let (nb, nff) = self.net_buoyancy(a);
        let f = 1.0 - nb / self.lf(a, Field::Mass, "Mass");
        let k = 1.0 - nff * dt;
        [old[0] * k + accel[0] * f * 0.5 * dt, old[1] * k + accel[1] * f * 0.5 * dt, old[2] * k + accel[2] * f * 0.5 * dt]
    }

    fn net_buoyancy(&self, a: ObjId) -> (f32, f32) {
        self.get_net_buoyancy(a)
    }

    /// APawn::physFalling: falling with air control, landing, walls.
    pub(crate) fn phys_falling(&mut self, a: ObjId, dt: f32, mut iterations: i32) {
        let old_accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let mut hit = CheckResult::new(1.0);
        self.set_last_collided_material(a, None);
        let mut air_control = self.lf(a, Field::AirControl, "AirControl");
        let mut accel = old_accel;
        accel[2] = 0.0;
        self.set_lv(a, Field::Acceleration, "Acceleration", accel);
        let mut max_xy_speed = 0.0f32;
        let accel_rate = self.lf(a, Field::AccelRate, "AccelRate");
        if air_control > 0.05 {
            // No air control if there is an obstacle right in front.
            let n = safe_normal(accel);
            let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            let k = air_control * accel_rate;
            let end = [(k * n[0] + v[0]) * dt + l[0], (k * n[1] + v[1]) * dt + l[1], l[2]];
            let ext = self.cyl_extent(a);
            self.single_line_check(&mut hit, Some(a), end, l, 0x1286, ext);
            if hit.actor.is_some() {
                air_control = 0.0;
            }
        }
        let mut max_accel = air_control * accel_rate;
        let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        let v2d = size2d(v);
        if v2d < 10.0 && air_control > 0.0 {
            max_accel += (10.0 - v2d) / dt;
        } else if self.lf(a, Field::GroundSpeed, "GroundSpeed") <= v2d {
            if air_control > 0.05 {
                max_xy_speed = v2d;
            } else {
                max_accel = 1.0;
            }
        }
        let accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        if max_accel * max_accel < size_sq(accel) {
            self.set_lv(a, Field::Acceleration, "Acceleration", mul(safe_normal(accel), max_accel));
        }
        let mut remaining = dt;
        while remaining > 0.0 && iterations <= 7 {
            iterations += 1;
            let time_tick = if remaining > 0.05 { (remaining * 0.5).min(0.05) } else { remaining };
            remaining -= time_tick;
            let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
            self.set_lb(a, Field::JustTeleported, "bJustTeleported", false);
            let mut old_velocity = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            let g = self.gravity_of(a);
            let acc = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
            let mut v = self.new_fall_velocity(a, old_velocity, add(g, acc), time_tick);
            if max_xy_speed != 0.0 {
                let v2 = [v[0], v[1], 0.0];
                if max_xy_speed * max_xy_speed < size_sq(v2) {
                    let n = safe_normal(v2);
                    v = [n[0] * max_xy_speed, n[1] * max_xy_speed, v[2]];
                }
            }
            self.set_lv(a, Field::Velocity, "Velocity", v);
            let zone = self.volume_lv(a, Field::ZoneVelocity, "ZoneVelocity", [0.0; 3]);
            let adjusted = mul(add(v, zone), time_tick);
            self.mv(a, adjusted, &mut hit);
            if !self.is_live(a) || self.native_prop(a, crate::NativeField::DeleteMe).as_bool() {
                return;
            }
            if self.physics_of(a) == PHYS_SWIMMING {
                remaining += (1.0 - hit.time) * time_tick;
                self.start_swimming(a, old_location, old_velocity, time_tick, remaining, iterations);
                return;
            }
            if hit.time < 1.0 {
                if hit.normal[2] >= MINFLOORZ {
                    // Landing.
                    self.set_last_collided_material(a, hit.material);
                    remaining += (1.0 - hit.time) * time_tick;
                    if !self.lb(a, Field::JustTeleported, "bJustTeleported") && hit.time > 0.1 && hit.time * time_tick > 0.003 {
                        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                        self.set_lv(a, Field::Velocity, "Velocity", mul(sub(l, old_location), 1.0 / (hit.time * time_tick)));
                    }
                    self.process_landed(a, hit.normal, hit.actor, remaining, iterations);
                    return;
                }
                self.process_hit_wall(a, hit.normal, hit.actor);
                if !self.is_live(a) {
                    return;
                }
                let old_hit_normal = hit.normal;
                // (Normal | Adjusted): (X + Z) + Y; (Delta | Adjusted): (Z + Y) + X.
                let n = hit.normal;
                let k = (n[0] * adjusted[0] + adjusted[2] * n[2]) + n[1] * adjusted[1];
                let mut delta = mul(sub(adjusted, mul(hit.normal, k)), 1.0 - hit.time);
                let mut landed = false;
                if (adjusted[2] * delta[2] + adjusted[1] * delta[1]) + delta[0] * adjusted[0] >= 0.0 {
                    self.mv(a, delta, &mut hit);
                    if hit.time < 1.0 {
                        landed = true;
                        if hit.normal[2] < MINFLOORZ {
                            self.process_hit_wall(a, hit.normal, hit.actor);
                            let desired_dir = safe_normal(adjusted);
                            Self::two_wall_adjust(desired_dir, &mut delta, hit.normal, old_hit_normal, hit.time);
                            // bDitch: stuck between two slopes too steep to stand on.
                            let ditch = old_hit_normal[2] > 0.0 && hit.normal[2] > 0.0 && delta[2] == 0.0 && (old_hit_normal[2] * hit.normal[2] + old_hit_normal[1] * hit.normal[1]) + old_hit_normal[0] * hit.normal[0] < 0.0;
                            self.mv(a, delta, &mut hit);
                            landed = ditch || hit.normal[2] >= MINFLOORZ;
                        }
                    }
                }
                if !self.is_live(a) {
                    return;
                }
                if landed {
                    self.set_last_collided_material(a, hit.material);
                    self.process_landed(a, hit.normal, hit.actor, 0.0, iterations);
                    return;
                }
                // Horizontal speed actually obtained along the wall.
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                let inv = 1.0 / time_tick;
                old_velocity[0] = inv * (l[0] - old_location[0]);
                old_velocity[1] = inv * (l[1] - old_location[1]);
            }
            if !self.lb(a, Field::JustTeleported, "bJustTeleported") {
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                let zone = self.volume_lv(a, Field::ZoneVelocity, "ZoneVelocity", [0.0; 3]);
                let inv = 1.0 / time_tick;
                let mut v = [inv * (l[0] - old_location[0]) - zone[0], inv * (l[1] - old_location[1]) - zone[1], inv * (l[2] - old_location[2]) - zone[2]];
                if v[2] < old_velocity[2] || old_velocity[2] >= 0.0 {
                    v = sub(add(v, v), old_velocity);
                }
                let terminal = self.volume_lf(a, Field::TerminalVelocity, "TerminalVelocity", 4000.0);
                if terminal * terminal < size_sq(v) {
                    v = mul(safe_normal(v), terminal);
                }
                self.set_lv(a, Field::Velocity, "Velocity", v);
            }
        }
        self.set_lv(a, Field::Acceleration, "Acceleration", old_accel);
    }

    /// APawn::startSwimming.
    pub(crate) fn start_swimming(&mut self, a: ObjId, old_location: V, old_velocity: V, time_tick: f32, remaining: f32, iterations: i32) {
        let mut remaining = remaining;
        if !self.lb(a, Field::JustTeleported, "bJustTeleported") && !self.lb(a, Field::Bounce, "bBounce") {
            if time_tick > 0.0 {
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                self.set_lv(a, Field::Velocity, "Velocity", mul(sub(l, old_location), 1.0 / time_tick));
            }
            let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            let v = sub(add(v, v), old_velocity);
            let terminal = self.volume_lf(a, Field::TerminalVelocity, "TerminalVelocity", 4000.0);
            let v = if terminal * terminal < size_sq(v) { mul(safe_normal(v), terminal) } else { v };
            self.set_lv(a, Field::Velocity, "Velocity", v);
        }
        let l = self.native_prop(a, crate::NativeField::Location).as_vector();
        let end = self.find_water_line(a, l, old_location);
        if end != l {
            let wanted = size(sub(end, l));
            let traveled = size(sub(l, old_location));
            remaining += (wanted / traveled) * time_tick;
            let mut hit = CheckResult::new(1.0);
            self.mv(a, sub(end, l), &mut hit);
        }
        let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        if v[2] > -160.0 && v[2] < 0.0 {
            let mut v = v;
            v[2] = speed_input_water(size2d(v));
            self.set_lv(a, Field::Velocity, "Velocity", v);
        }
        if remaining > 0.01 {
            self.phys_swimming(a, remaining, iterations);
        }
    }

    /// APawn::findWaterLine: crossing point of the water surface between two
    /// positions (brushes of the water volumes crossed by the trace).
    pub(crate) fn find_water_line(&self, a: ObjId, start: V, end: V) -> V {
        // As in the engine, the trace goes from `end` to `start` (End and Start swapped).
        let hits = self.multi_line_check(start, end, [0.0; 3], true, 0x8e, Some(a));
        for h in hits {
            let Some(o) = h.actor else { continue };
            if self.is_owned_by(a, o) {
                continue;
            }
            if self.lb(o, Field::WorldGeometry, "bWorldGeometry") {
                return end;
            }
            if self.is_a(o, "PhysicsVolume") && self.lb(o, Field::WaterVolume, "bWaterVolume") {
                let dir = safe_normal(sub(start, end));
                return if Some(o) == self.native_object(a, crate::NativeField::PhysicsVolume) { add(h.location, mul(dir, 0.1)) } else { sub(h.location, mul(dir, 0.1)) };
            }
        }
        end
    }

    /// APawn::Swim: movement in the water, stopping at the surface.
    fn swim(&mut self, a: ObjId, delta: V, hit: &mut CheckResult) -> f32 {
        let start = self.native_prop(a, crate::NativeField::Location).as_vector();
        let mut frac = 0.0f32;
        self.mv(a, delta, hit);
        if !self.volume_lb(a, Field::WaterVolume, "bWaterVolume") {
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            let water = self.find_water_line(a, start, l);
            if water != l {
                // |Water − Location| / |Delta|, Delta summed (z² + y²) + x².
                let w = sub(water, l);
                let len_delta = ((delta[2] * delta[2] + delta[1] * delta[1] + delta[0] * delta[0]) as f64).sqrt() as f32;
                frac = (((w[0] * w[0] + w[1] * w[1] + w[2] * w[2]) as f64).sqrt() as f32) / len_delta;
                if dot(sub(l, start), sub(water, l)) > 0.0 {
                    frac = 0.0;
                }
                self.mv(a, sub(water, l), hit);
            }
        }
        frac
    }

    /// APawn::physSwimming.
    fn phys_swimming(&mut self, a: ObjId, dt: f32, iterations: i32) {
        let (nb, _) = self.net_buoyancy(a);
        let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        if v[2] > 100.0 {
            v[2] *= nb / self.lf(a, Field::Buoyancy, "Buoyancy");
            self.set_lv(a, Field::Velocity, "Velocity", v);
        }
        let mut old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        self.set_lb(a, Field::JustTeleported, "bJustTeleported", false);
        let accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let accel_dir = if is_zero(accel) { accel } else { safe_normal(accel) };
        let (speed, friction) = (self.lf(a, Field::WaterSpeed, "WaterSpeed"), self.volume_lf(a, Field::FluidFriction, "FluidFriction", 0.0) * 0.5);
        self.calc_velocity(a, accel_dir, dt, speed, friction, true, false, true);
        let zone = self.volume_lv(a, Field::ZoneVelocity, "ZoneVelocity", [0.0; 3]);
        // (ZoneVelocity · 25) · DeltaTime, component by component.
        let zone_vel = if self.is_human(a) || size_sq(zone) > 90000.0 { [0, 1, 2].map(|k| (zone[k] * 25.0) * dt) } else { [0.0; 3] };
        let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
        let adjusted = mul(add(v, zone_vel), dt);
        let mut hit = CheckResult::new(1.0);
        let mut remaining = self.swim(a, adjusted, &mut hit) * dt;
        if hit.time >= 1.0 {
            self.set_lv(a, Field::Floor, "Floor", [0.0, 0.0, 1.0]);
        } else {
            self.set_lv(a, Field::Floor, "Floor", hit.normal);
            let grav_dir = self.grav_dir(a);
            let desired_dir = safe_normal(adjusted);
            let vel_dir = safe_normal(self.native_prop(a, crate::NativeField::Velocity).as_vector());
            let up_down = dot(grav_dir, vel_dir);
            let head_water = self.po(a, "HeadVolume").is_some_and(|h| self.lb(h, Field::WaterVolume, "bWaterVolume"));
            if hit.normal[2].abs() < 0.2 && up_down < 0.5 && up_down > -0.2 && !head_water {
                let step_z = self.native_prop(a, crate::NativeField::Location).as_vector()[2];
                self.step_up(a, grav_dir, desired_dir, mul(adjusted, 1.0 - hit.time), &mut hit);
                if !self.is_live(a) {
                    return;
                }
                old_location[2] = (old_location[2] - step_z) + self.native_prop(a, crate::NativeField::Location).as_vector()[2];
            } else {
                self.process_hit_wall(a, hit.normal, hit.actor);
                let old_hit_normal = hit.normal;
                // (Adjusted | Normal): (y·y + x·x) + z·z.
                let n = hit.normal;
                let k = (n[1] * adjusted[1] + n[0] * adjusted[0]) + adjusted[2] * n[2];
                let mut delta = mul(sub(adjusted, mul(hit.normal, k)), 1.0 - hit.time);
                // (Delta | Adjusted): (z·z + y·y) + x·x.
                if (adjusted[2] * delta[2] + adjusted[1] * delta[1]) + delta[0] * adjusted[0] >= 0.0 {
                    // ((1 − Hit.Time) · Swim) · remainingTime, Hit.Time being the one that Swim
                    // has just filled.
                    let f = self.swim(a, delta, &mut hit);
                    remaining *= (1.0 - hit.time) * f;
                    if hit.time < 1.0 {
                        self.process_hit_wall(a, hit.normal, hit.actor);
                        Self::two_wall_adjust(desired_dir, &mut delta, hit.normal, old_hit_normal, hit.time);
                        let f = self.swim(a, delta, &mut hit);
                        remaining *= (1.0 - hit.time) * f;
                    }
                }
            }
        }
        if !self.is_live(a) {
            return;
        }
        if !self.lb(a, Field::JustTeleported, "bJustTeleported") && remaining < dt {
            let keep_z = !self.volume_lb(a, Field::WaterVolume, "bWaterVolume");
            let vz = self.native_prop(a, crate::NativeField::Velocity).as_vector()[2];
            if !self.lb(a, Field::NoVelocityUpdate, "bNoVelocityUpdate") {
                let l = self.native_prop(a, crate::NativeField::Location).as_vector();
                self.set_lv(a, Field::Velocity, "Velocity", mul(sub(l, old_location), 1.0 / (dt - remaining)));
            }
            self.set_lb(a, Field::NoVelocityUpdate, "bNoVelocityUpdate", false);
            if keep_z {
                let mut v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
                v[2] = vz;
                self.set_lv(a, Field::Velocity, "Velocity", v);
            }
        }
        if !self.volume_lb(a, Field::WaterVolume, "bWaterVolume") {
            if self.physics_of(a) == PHYS_SWIMMING {
                self.set_physics(a, PHYS_FALLING, None, [0.0, 0.0, 1.0]);
            }
            let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            if v[2] < 160.0 && v[2] > 0.0 {
                let mut v = v;
                // Computed in double under the FPU's 24-bit precision: each operation rounded.
                v[2] = (((size2d(v) as f64 * 0.4) as f32) as f64 + 40.0) as f32;
                self.set_lv(a, Field::Velocity, "Velocity", v);
            }
        }
        if self.physics_of(a) != PHYS_SWIMMING {
            self.start_new_physics(a, remaining, iterations + 1);
        }
    }

    /// Movement shared by physFlying and physLadder after computing
    /// the velocity: movement, step or sliding against the obstacle, actual velocity.
    /// `desired_dir`: Adjusted.SafeNormal() for physFlying, Velocity.SafeNormal() for physLadder.
    #[allow(clippy::too_many_arguments)]
    fn fly_move(&mut self, a: ObjId, dt: f32, adjusted: V, grav_dir: V, desired_dir: V, old_location: V, ladder: bool) {
        let mut old_location = old_location;
        let mut hit = CheckResult::new(1.0);
        self.mv(a, adjusted, &mut hit);
        if hit.time >= 1.0 {
            self.set_lv(a, Field::Floor, "Floor", [0.0, 0.0, 1.0]);
        } else {
            self.set_lv(a, Field::Floor, "Floor", hit.normal);
            let vel_dir = safe_normal(self.native_prop(a, crate::NativeField::Velocity).as_vector());
            let up_down = dot(grav_dir, vel_dir);
            if hit.normal[2].abs() < 0.2 && up_down < 0.5 && up_down > -0.2 {
                let step_z = self.native_prop(a, crate::NativeField::Location).as_vector()[2];
                self.step_up(a, grav_dir, desired_dir, mul(adjusted, 1.0 - hit.time), &mut hit);
                if !self.is_live(a) {
                    return;
                }
                old_location[2] = self.native_prop(a, crate::NativeField::Location).as_vector()[2] + (old_location[2] - step_z);
            } else {
                self.process_hit_wall(a, hit.normal, hit.actor);
                let old_hit_normal = hit.normal;
                // (Adjusted | Normal): (Y + X) + Z.
                let n = hit.normal;
                let k = (n[1] * adjusted[1] + n[0] * adjusted[0]) + adjusted[2] * n[2];
                let mut delta = mul(sub(adjusted, mul(hit.normal, k)), 1.0 - hit.time);
                // (Delta | Adjusted): (Z + Y) + X when flying, (X + Y) + Z on a
                // ladder.
                let along = if ladder {
                    (delta[0] * adjusted[0] + delta[1] * adjusted[1]) + delta[2] * adjusted[2]
                } else {
                    (adjusted[2] * delta[2] + adjusted[1] * delta[1]) + delta[0] * adjusted[0]
                };
                if along >= 0.0 {
                    self.mv(a, delta, &mut hit);
                    if hit.time < 1.0 {
                        self.process_hit_wall(a, hit.normal, hit.actor);
                        Self::two_wall_adjust(desired_dir, &mut delta, hit.normal, old_hit_normal, hit.time);
                        self.mv(a, delta, &mut hit);
                    }
                }
            }
        }
        if !self.is_live(a) {
            return;
        }
        if !self.lb(a, Field::JustTeleported, "bJustTeleported") && !self.lb(a, Field::NoVelocityUpdate, "bNoVelocityUpdate") {
            let l = self.native_prop(a, crate::NativeField::Location).as_vector();
            let k = 1.0 / dt;
            self.set_lv(a, Field::Velocity, "Velocity", [k * (l[0] - old_location[0]), (l[1] - old_location[1]) * k, (l[2] - old_location[2]) * k]);
        }
        self.set_lb(a, Field::NoVelocityUpdate, "bNoVelocityUpdate", false);
    }

    /// APawn::physFlying.
    fn phys_flying(&mut self, a: ObjId, dt: f32, _iterations: i32) {
        // Outside the world (ZoneNumber 0) with bCollideWorld: nothing; destroyed ("%s flew out of the
        // world!") without a player controller.
        if self.lb(a, Field::CollideWorld, "bCollideWorld") && self.zone_number_of(a) == 0 {
            if !self.native_object(a, crate::NativeField::Controller).is_some_and(|c| self.lb(c, Field::IsPlayer, "bIsPlayer")) {
                let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                self.log.push(format!("{name} flew out of the world!"));
                let _ = crate::natives::call(self, "Destroy", a, &mut [], &[]);
            }
            return;
        }
        let accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let accel_dir = if is_zero(accel) { accel } else { safe_normal(accel) };
        let (speed, friction) = (self.lf(a, Field::AirSpeed, "AirSpeed"), self.volume_lf(a, Field::FluidFriction, "FluidFriction", 0.0) * 0.5);
        self.calc_velocity(a, accel_dir, dt, speed, friction, true, false, false);
        let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        self.set_lb(a, Field::JustTeleported, "bJustTeleported", false);
        let zone = self.volume_lv(a, Field::ZoneVelocity, "ZoneVelocity", [0.0; 3]);
        let zone_vel = if self.is_human(a) || size_sq(zone) > 90000.0 { zone } else { [0.0; 3] };
        let adjusted = mul(add(self.native_prop(a, crate::NativeField::Velocity).as_vector(), zone_vel), dt);
        let grav_dir = self.grav_dir(a);
        self.fly_move(a, dt, adjusted, grav_dir, safe_normal(adjusted), old_location, false);
    }

    /// APawn::physLadder.
    fn phys_ladder(&mut self, a: ObjId, dt: f32, _iterations: i32) {
        let accel = self.native_prop(a, crate::NativeField::Acceleration).as_vector();
        let accel_dir = if is_zero(accel) {
            accel
        } else {
            let n = safe_normal(accel);
            [n[0], n[1], if n[2] >= 0.0 { 1.0 } else { -1.0 }]
        };
        let (speed, friction) = (self.lf(a, Field::LadderSpeed, "LadderSpeed"), self.volume_lf(a, Field::GroundFriction, "GroundFriction", 8.0));
        self.calc_velocity(a, accel_dir, dt, speed, friction, true, false, false);
        let old_location = self.native_prop(a, crate::NativeField::Location).as_vector();
        let adjusted = mul(self.native_prop(a, crate::NativeField::Velocity).as_vector(), dt);
        let desired_dir = safe_normal(self.native_prop(a, crate::NativeField::Velocity).as_vector());
        self.fly_move(a, dt, adjusted, [0.0, 0.0, -1.0], desired_dir, old_location, true);
    }

    fn is_crouched(&self, a: ObjId) -> bool {
        let (h, r) = (self.native_prop(a, crate::NativeField::CollisionHeight).as_float(), self.native_prop(a, crate::NativeField::CollisionRadius).as_float());
        (h == self.lf(a, Field::CrouchHeight, "CrouchHeight") && h < self.default_lf(a, Field::CollisionHeight, "CollisionHeight"))
            || (r == self.lf(a, Field::CrouchRadius, "CrouchRadius") && r < self.default_lf(a, Field::CollisionRadius, "CollisionRadius"))
    }

    /// APawn::Crouch, see `reachability::crouch`.
    pub(crate) fn crouch(&mut self, a: ObjId, client_simulation: bool) {
        crate::reachability::crouch(&mut crate::reachability::CrouchedVm(self, a), client_simulation);
    }

    /// APawn::UnCrouch, see `reachability::uncrouch`.
    pub(crate) fn uncrouch(&mut self, a: ObjId, client_simulation: bool) {
        crate::reachability::uncrouch(&mut crate::reachability::CrouchedVm(self, a), client_simulation);
    }

    /// AActor::fixedTurn.
    pub(crate) fn fixed_turn(&self, a: ObjId, current: i32, desired: i32, delta_rate: i32) -> i32 {
        let mut cur = (current & 0xffff) as u32;
        if delta_rate == 0 {
            return cur as i32;
        }
        let fixed_dir = self.lb(a, Field::FixedRotationDir, "bFixedRotationDir");
        let to_desired = self.lb(a, Field::RotateToDesired, "bRotateToDesired");
        let desired_low16 = (desired & 0xffff) as u32;
        if fixed_dir {
            if !to_desired {
                return (cur as i32 + delta_rate) & 0xffff;
            }
            let step = if delta_rate < 1 {
                let c = if cur < desired_low16 { cur + 0x10000 } else { cur };
                let d = desired_low16 as i32 - c as i32;
                if d <= delta_rate {
                    delta_rate
                } else {
                    d
                }
            } else {
                let d2 = if desired_low16 < cur { desired_low16 + 0x10000 } else { desired_low16 };
                let d = d2 as i32 - cur as i32;
                if delta_rate <= d {
                    return (cur as i32 + delta_rate) & 0xffff;
                }
                d
            };
            return (cur as i32 + step) & 0xffff;
        }
        if !to_desired {
            return cur as i32;
        }
        let rate = delta_rate.abs();
        if desired_low16 < cur {
            let diff = cur as i32 - desired_low16 as i32;
            if diff > 0x7fff {
                let up = (desired_low16 as i32 - cur as i32) + 0x10000;
                return if rate < up { (cur as i32 + rate) & 0xffff } else { (cur as i32 + up) & 0xffff };
            }
            let step = rate.min(diff);
            cur = (cur as i32 - step) as u32;
        } else {
            let diff = desired_low16 as i32 - cur as i32;
            if diff < 0x8000 {
                return (cur as i32 + rate.min(diff)) & 0xffff;
            }
            let down = (cur as i32 - desired_low16 as i32) + 0x10000;
            let step = rate.min(down);
            cur = (cur as i32 - step) as u32;
        }
        (cur & 0xffff) as i32
    }

    /// APawn::FindSlopeRotation: in PHYS_Spider with a controller, OrthoRotation of the
    /// frame (Controller.ViewY ^ Floor, ViewY, Floor); otherwise AActor::FindSlopeRotation
    fn find_slope_rotation(&self, a: ObjId, floor: V, rot: [i32; 3]) -> [i32; 3] {
        if let (PHYS_SPIDER, Some(c)) = (self.physics_of(a), self.native_object(a, crate::NativeField::Controller)) {
            let (f, vy) = (self.lv(a, Field::Floor, "Floor"), self.lv(c, Field::ViewY, "ViewY"));
            let x = [f[2] * vy[1] - vy[2] * f[1], vy[2] * f[0] - f[2] * vy[0], f[1] * vy[0] - f[0] * vy[1]];
            return crate::world::ortho_rotation([x, vy, f]);
        }
        self.actor_find_slope_rotation(floor, rot)
    }

    /// APawn::physicsRotation: rotation towards DesiredRotation at RotationRate (AI) or at
    /// EnemyTurnSpeed (player following a target), roll depending on the lateral acceleration.
    fn pawn_physics_rotation(&mut self, a: ObjId, dt: f32, old_velocity: V) {
        let mut new_rot = self.lp(a, Field::Rotation, "Rotation").as_rotator();
        let physics = self.physics_of(a);
        let walking = physics == PHYS_WALKING || physics == PHYS_ROOT_MOTION;
        self.set_lb(a, Field::FixedRotationDir, "bFixedRotationDir", false);
        let controller = self.native_object(a, crate::NativeField::Controller);
        let rate = self.lp(a, Field::RotationRate, "RotationRate").as_rotator();
        let mut delta_yaw = 0i32;
        if !self.is_human(a) {
            delta_yaw = ((rate[1] as f32 * dt) as f64).round_ties_even() as i32;
            self.set_lb(a, Field::RotateToDesired, "bRotateToDesired", controller.is_some());
        } else if let Some(c) = controller {
            let to_desired = self.lb(c, Field::RotateToDesired, "bRotateToDesired");
            self.set_lb(a, Field::RotateToDesired, "bRotateToDesired", to_desired);
            if to_desired {
                let speed = if self.is_a(c, "PlayerController") { self.lp(c, Field::EnemyTurnSpeed, "EnemyTurnSpeed").as_int() } else { 0 };
                delta_yaw = (((speed << 1) as f32 * dt) as f64).round_ties_even() as i32;
                let cd = self.lp(c, Field::DesiredRotation, "DesiredRotation").as_rotator();
                let mut d = self.lp(a, Field::DesiredRotation, "DesiredRotation").as_rotator();
                d[0] = cd[0];
                d[1] = cd[1];
                self.set_prop(a, "DesiredRotation", Value::rotator(d[0], d[1], d[2]));
            }
        }
        let on_ladder = if physics == PHYS_LADDER { self.po(a, "OnLadder") } else { None };
        if let Some(ladder) = on_ladder {
            new_rot = self.lp(ladder, Field::WallDir, "WallDir").as_rotator();
        } else if self.lb(a, Field::RotateToDesired, "bRotateToDesired") {
            let mut desired = self.lp(a, Field::DesiredRotation, "DesiredRotation").as_rotator();
            if desired[1] != new_rot[1] {
                new_rot[1] = self.fixed_turn(a, new_rot[1], desired[1], delta_yaw);
            }
            if walking || physics == PHYS_FALLING {
                desired[0] = 0;
                self.set_prop(a, "DesiredRotation", Value::rotator(desired[0], desired[1], desired[2]));
            }
            if (!self.lb(a, Field::Crawler, "bCrawler") || !walking) && desired[0] != new_rot[0] {
                new_rot[0] = self.fixed_turn(a, new_rot[0], desired[0], delta_yaw);
            }
            if let Some(c) = controller.filter(|&c| self.lb(c, Field::RotateToDesired, "bRotateToDesired")) {
                let mut cr = self.lp(c, Field::Rotation, "Rotation").as_rotator();
                cr[1] = new_rot[1];
                let cd = self.lp(c, Field::DesiredRotation, "DesiredRotation").as_rotator();
                cr[0] = self.fixed_turn(a, cr[0], cd[0], delta_yaw);
                self.set_prop(c, "Rotation", Value::rotator(cr[0], cr[1], cr[2]));
            }
        }
        let mut roll = new_rot[2];
        if self.lb(a, Field::Crawler, "bCrawler") {
            if !walking {
                let d = (rate[1] as f32 * dt) as i32;
                new_rot[0] = self.fixed_turn(a, new_rot[0], 0, d);
                roll = self.fixed_turn(a, roll, 0, d);
            } else {
                let floor = self.lv(a, Field::Floor, "Floor");
                let r = self.find_slope_rotation(a, floor, [new_rot[0], new_rot[1], roll]);
                new_rot[0] = r[0];
                new_rot[1] = r[1];
                roll = r[2];
            }
        } else if rate[2] < 1 {
            roll = 0;
        } else {
            let v = self.native_prop(a, crate::NativeField::Velocity).as_vector();
            if physics == PHYS_WALKING && size_sq(v) < 40000.0 {
                let s = (8.0 * dt).min(1.0);
                roll = Self::smooth_roll(roll, s);
            } else {
                let inv = 1.0 / dt;
                let real_accel = [inv * (v[0] - old_velocity[0]), inv * (v[1] - old_velocity[1]), inv * (v[2] - old_velocity[2])];
                if size_sq(real_accel) > 10000.0 {
                    let max_roll = if physics == PHYS_WALKING { 4096.0f32 } else { 28000.0f32 };
                    let axes = crate::ai_natives::axes([new_rot[0], new_rot[1], 0]);
                    let y = dot(real_accel, axes[1]);
                    let target = if y > 0.0 {
                        let t = (y / self.lf(a, Field::AccelRate, "AccelRate") * max_roll) as i32;
                        rate[2].min(t)
                    } else {
                        let t = (y / self.lf(a, Field::AccelRate, "AccelRate") * max_roll + 65536.0) as i32;
                        (0x10000 - rate[2]).max(t)
                    };
                    let mut current = self.lp(a, Field::Rotation, "Rotation").as_rotator();
                    current[2] &= 0xffff;
                    if target > 0x8000 {
                        if current[2] < 0x8000 {
                            current[2] += 0x10000;
                        }
                    } else if current[2] > 0x8000 {
                        current[2] -= 0x10000;
                    }
                    self.set_prop(a, "Rotation", Value::rotator(current[0], current[1], current[2]));
                    let s = (5.0 * dt).min(1.0);
                    roll = ((1.0 - s) * current[2] as f32 + target as f32 * s) as i32;
                } else {
                    let s = (8.0 * dt).min(1.0);
                    roll = Self::smooth_roll(roll, s);
                }
            }
        }
        new_rot[2] = roll;
        if new_rot != self.lp(a, Field::Rotation, "Rotation").as_rotator() {
            let mut hit = CheckResult::new(1.0);
            self.move_actor(a, [0.0; 3], new_rot, &mut hit, false, false, false, false);
        }
    }

    fn smooth_roll(roll: i32, s: f32) -> i32 {
        if roll < 0x8000 {
            ((1.0 - s) * roll as f32) as i32
        } else {
            ((0x10000 - roll) as f32 * s + roll as f32) as i32
        }
    }

    /// APawn::rotateToward: DesiredRotation towards a point (AController::Tick).
    pub(crate) fn rotate_toward(&mut self, a: ObjId, focal: V) {
        if self.physics_of(a) == PHYS_SPIDER {
            return;
        }
        let mut d = vector_rotation(sub(focal, self.native_prop(a, crate::NativeField::Location).as_vector()));
        d[1] &= 0xffff;
        if self.physics_of(a) == PHYS_WALKING {
            let toward_pawn = self.native_object(a, crate::NativeField::Controller).and_then(|c| self.po(c, "MoveTarget")).is_some_and(|t| self.is_a(t, "Pawn"));
            if !toward_pawn {
                d[0] = 0;
            }
        }
        self.set_prop(a, "DesiredRotation", Value::rotator(d[0], d[1], d[2]));
    }

    /// AActor::moveSmooth: movement that climbs steps and slides along
    /// walls (the MoveSmooth native).
    pub(crate) fn move_smooth(&mut self, a: ObjId, delta: V) -> bool {
        let mut hit = CheckResult::new(1.0);
        let moved = self.mv(a, delta, &mut hit);
        if hit.time < 1.0 && self.is_live(a) {
            let grav_dir = self.grav_dir(a);
            let dir = safe_normal(delta);
            let up_down = dir[2] * grav_dir[2] + (dir[1] + dir[0]) * 0.0;
            if hit.normal[2].abs() >= 0.2 || up_down >= 0.5 || up_down <= -0.2 {
                // (Normal | Delta): (Y + X) + Z; (Delta | D2): (Y + Z) + X.
                let n = hit.normal;
                let k = (n[1] * delta[1] + n[0] * delta[0]) + delta[2] * n[2];
                let mut d2 = mul(sub(delta, mul(hit.normal, k)), 1.0 - hit.time);
                if (delta[1] * d2[1] + delta[2] * d2[2]) + d2[0] * delta[0] >= 0.0 {
                    let old_hit_normal = hit.normal;
                    let desired_dir = safe_normal(delta);
                    self.mv(a, d2, &mut hit);
                    if hit.time < 1.0 {
                        self.smooth_hit_wall(a, hit.normal, hit.actor);
                        Self::two_wall_adjust(desired_dir, &mut d2, hit.normal, old_hit_normal, hit.time);
                        self.mv(a, d2, &mut hit);
                    }
                }
            } else {
                self.step_up(a, grav_dir, dir, mul(delta, 1.0 - hit.time), &mut hit);
            }
        }
        moved
    }

    /// APawn::SmoothHitWall: the controller's NotifyHitWall (horizontal normal when
    /// walking), otherwise HitWall.
    pub(crate) fn smooth_hit_wall(&mut self, a: ObjId, normal: V, actor: Option<ObjId>) {
        let mut n = normal;
        if self.is_a(a, "Pawn") {
            if let Some(c) = self.native_object(a, crate::NativeField::Controller) {
                if self.physics_of(a) == PHYS_WALKING {
                    n[2] = 0.0;
                }
                if self.event_ret(c, "NotifyHitWall", vec![Value::vector(n[0], n[1], n[2]), Value::Object(actor)]) {
                    return;
                }
            }
        }
        self.event_pub(a, "HitWall", vec![Value::vector(n[0], n[1], n[2]), Value::Object(actor)]);
    }
}

/// Velocity.Z when entering the water in startSwimming: −80 − |V.xy| × 0.7 on
/// the 24-bit FPU, fmul qword 0.7 rounded then fsubr qword −80 rounded.
pub(crate) fn speed_input_water(horizontal: f32) -> f32 {
    let product = crate::ai_natives::x87_d24(horizontal as f64, 0.7, false);
    crate::ai_natives::x87_d24(-80.0, -(product as f64), true)
}

/// Test hook of APawn::startNewPhysics: the dispatch records (name, DeltaTime, Iterations) and skips the physics (outside tests: always false).
#[inline(always)]
fn stub_dispatch(_name: &'static str, _dt: f32, _iterations: i32) -> bool {
    false
}

// Test hook of APawn::processLanded: in tests, with an open log, the outgoing calls are
// recorded and return the original's answers; outside tests these are functions that do nothing.
mod pl_outside_test {
    use super::*;
    pub(super) fn pl_trace(_: &mut CheckResult, _: V, _: V, _: V) -> bool {
        false
    }
    pub(super) fn pl_spot(_: V, _: &mut V) -> Option<bool> {
        None
    }
    pub(super) fn pl_op(_: impl FnOnce() -> String) -> bool {
        false
    }
    pub(super) fn pl_notify(_: impl FnOnce() -> String) -> Option<bool> {
        None
    }
}
use pl_outside_test::*;

