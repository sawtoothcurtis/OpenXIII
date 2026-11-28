//! Physics of actors that are neither pawns nor Movers, reimplemented from Engine.dll:
//! AActor::performPhysics, physFalling, processLanded (and ADecoration::processLanded),
//! processHitWall, physProjectile (with AProjectile::BoundProjectileVelocity), physicsRotation (and
//! AProjectile::physicsRotation), FindSlopeRotation and GetNetBuoyancy. Addresses at the top of each
//! function; floating-point operations follow the machine code's order.

#![allow(clippy::neg_multiply)] // the "× -1" reproduce the machine code's FMUL -1

use crate::NativeField as Field;
use crate::ai_move::{div, fmul_d, rotator_vector};
use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;

/// FVector::RotateAngleAxis: rotation of `v` by `angle` units around `axis`
/// (GMath's sine table).
fn rotate_angle_axis(v: V, angle: i32, axis: V) -> V {
    let s = openxiii_assets::coords::gmath_sin(angle);
    let c = openxiii_assets::coords::gmath_sin(angle.wrapping_add(0x4000));
    let (ax, ay, az) = (axis[0], axis[1], axis[2]);
    let omc = 1.0 - c;
    // OMC · (Y·X), then each component as ((diagonal · V) + …) + …
    // in the order (Z), (Y), (X).
    let xy = omc * (ay * ax);
    let yz = omc * (ay * az);
    let zx = omc * (az * ax);
    let (sx, sy, sz) = (s * ax, s * ay, s * az);
    [
        ((ax * ax * omc + c) * v[0] + (zx + sy) * v[2]) + (xy - sz) * v[1],
        ((ay * ay * omc + c) * v[1] + (yz - sx) * v[2]) + (xy + sz) * v[0],
        ((az * az * omc + c) * v[2] + (zx - sy) * v[0]) + (yz + sx) * v[1],
    ]
}

impl Vm {
    /// AActor::performPhysics: falling, projectile, PHYS_Trailer, PHYS_RootMotion;
    /// rotation if RotationRate is not zero (outside interpolation); pending PostTouch.
    pub(crate) fn actor_perform_physics(&mut self, a: ObjId, dt: f32) {
        match self.native_prop(a, Field::Physics).as_byte() {
            PHYS_FALLING => self.actor_phys_falling(a, dt, 0),
            PHYS_PROJECTILE => self.phys_projectile(a, dt, 0),
            PHYS_TRAILER => self.phys_trailer(a),
            PHYS_ROOT_MOTION => self.phys_root_motion(a, dt),
            _ => {}
        }
        self.actor_physics_tail(a, dt);
    }

    /// Common end of AActor::performPhysics and AMover::performPhysics: physicsRotation if
    /// RotationRate is not zero (outside bInterpolating), then pending PostTouch.
    pub(crate) fn actor_physics_tail(&mut self, a: ObjId, dt: f32) {
        if !self.alive_obj(a) {
            return;
        }
        let rate = self.lp(a, Field::RotationRate, "RotationRate").as_rotator();
        if (rate[0] & 0xffff != 0 || rate[1] & 0xffff != 0 || rate[2] & 0xffff != 0) && !self.lb(a, Field::Interpolating, "bInterpolating") {
            if self.is_a(a, "Projectile") {
                self.projectile_physics_rotation(a, dt);
            } else {
                self.actor_physics_rotation(a, dt);
            }
        }
        if !self.alive_obj(a) {
            return;
        }
        if let Some(pt) = self.po(a, "PendingTouch") {
            self.event_pub(pt, "PostTouch", vec![Value::Object(Some(a))]);
            if let Some(pt) = self.po(a, "PendingTouch") {
                let next = self.prop(pt, "PendingTouch");
                self.set_prop(a, "PendingTouch", next);
                self.set_prop(pt, "PendingTouch", Value::Object(None));
            }
        }
    }

    /// AActor::physTrailer: follows its Owner. Sprite: Owner.Location (+ PrePivot if
    /// bTrailerPrePivot, otherwise − Mass × Owner.Rotation.Vector() if bTrailerSameRotation). Other
    /// DrawType: FarMoveActor onto Owner.Location then MoveActor without movement towards the Owner's
    /// rotation (bTrailerSameRotation), (16384, 0, 0) if its speed is nearly zero, otherwise
    /// (−Owner.Velocity).Rotation().
    pub(crate) fn phys_trailer(&mut self, a: ObjId) {
        let Some(owner) = self.native_object(a, Field::Owner) else { return };
        let ol = self.native_prop(owner, Field::Location).as_vector();
        if self.native_prop(a, Field::DrawType).as_byte() != 1 {
            self.far_move_actor(a, ol, false, true, false);
            let rot = if self.lb(a, Field::TrailerSameRotation, "bTrailerSameRotation") {
                self.native_prop(owner, Field::Rotation).as_rotator()
            } else {
                let v = self.native_prop(owner, Field::Velocity).as_vector();
                if v.iter().all(|c| c.abs() < 0.0001) {
                    [0x4000, 0, 0]
                } else {
                    // FMUL by −1.0, then FVector::Rotation.
                    vector_rotation([v[0] * -1.0, v[1] * -1.0, v[2] * -1.0])
                }
            };
            let mut hit = CheckResult::new(1.0);
            self.move_actor(a, [0.0; 3], rot, &mut hit, false, false, false, false);
            return;
        }
        let dest = if self.lb(a, Field::TrailerPrePivot, "bTrailerPrePivot") {
            let pp = self.native_prop(a, Field::PrePivot).as_vector();
            [pp[0] + ol[0], pp[1] + ol[1], pp[2] + ol[2]]
        } else if self.lb(a, Field::TrailerSameRotation, "bTrailerSameRotation") {
            let v = rotator_vector(self.native_prop(owner, Field::Rotation).as_rotator());
            let m = self.lf(a, Field::Mass, "Mass");
            [ol[0] - m * v[0], ol[1] - m * v[1], ol[2] - m * v[2]]
        } else {
            ol
        };
        self.far_move_actor(a, dest, false, true, false);
    }

    /// AActor::physRootMotion. The game's version no longer moves the actor: with a
    /// skeletal mesh instance, Velocity = (Location − Location) × (1 / dt) (outside
    /// bJustTeleported), bCollideWorld restored as is; without it, Velocity and Acceleration zero.
    pub(crate) fn phys_root_motion(&mut self, a: ObjId, dt: f32) {
        // UMesh::MeshGetInstance (vt 0x90), then IsA(USkeletalMeshInstance).
        if self.mesh_instance(a).is_some() {
            if !self.lb(a, Field::JustTeleported, "bJustTeleported") {
                // Location before and after the root animation, which no longer changes it.
                let (old, l) = (self.native_prop(a, Field::Location).as_vector(), self.native_prop(a, Field::Location).as_vector());
                let k = 1.0 / dt;
                self.set_lv(a, Field::Velocity, "Velocity", [k * (l[0] - old[0]), (l[1] - old[1]) * k, (l[2] - old[2]) * k]);
            }
            return;
        }
        self.set_lv(a, Field::Velocity, "Velocity", [0.0; 3]);
        self.set_lv(a, Field::Acceleration, "Acceleration", [0.0; 3]);
    }

    fn alive_obj(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some())
    }

    pub(crate) fn zone_number_of(&self, a: ObjId) -> i32 {
        match self.native_value(a, Field::Region) {
            Value::Struct(_, f) => f.get(2).map_or(0, |z| z.as_int()),
            _ => 0,
        }
    }

    fn actor_move(&mut self, a: ObjId, delta: V, hit: &mut CheckResult) {
        let r = self.native_prop(a, Field::Rotation).as_rotator();
        self.move_actor(a, delta, r, hit, false, false, false, false);
    }

    /// AActor::GetNetBuoyancy: buoyancy and water friction, depending on the share of the
    /// cylinder that is submerged (top-down trace inside the volume's brush; out of the water, half
    /// of the share in a touched volume). The traces take PhysicsVolume as owner,
    /// as in the machine code.
    pub(crate) fn get_net_buoyancy(&self, a: ObjId) -> (f32, f32) {
        let Some(own) = self.native_object(a, Field::PhysicsVolume).or(self.default_volume) else { return (0.0, 0.0) };
        let l = self.native_prop(a, Field::Location).as_vector();
        let h = self.native_prop(a, Field::CollisionHeight).as_float();
        let top = [l[0], l[1], h + l[2]];
        let bottom = [l[0], l[1], l[2] - h];
        let frame = self.brush_frame(own);
        let trace = |volume: ObjId| -> Option<f32> {
            let model = self.brushes.get(&volume)?;
            let mut hit = openxiii_assets::bsp::BspHit::new(1.0);
            model.line_check(&mut hit, Some(&frame), bottom, top, [0.0; 3], 0, 0);
            Some(hit.time)
        };
        let (volume, part) = if self.lb(own, Field::WaterVolume, "bWaterVolume") {
            let mut part = 1.0f32;
            if let Some(t) = trace(own) {
                if t != 1.0 {
                    part = 1.0 - t;
                }
            }
            (Some(own), part)
        } else {
            let mut best = 1.0f32;
            let mut volume = None;
            for t in self.touching_of(a) {
                if !self.is_a(t, "PhysicsVolume") {
                    continue;
                }
                let Some(time) = trace(t) else { continue };
                if best > time {
                    best = time;
                    volume = Some(t);
                }
            }
            (volume, (1.0 - best) * 0.5)
        };
        match volume.filter(|&v| self.lb(v, Field::WaterVolume, "bWaterVolume")) {
            Some(v) => (part * self.lf(a, Field::Buoyancy, "Buoyancy"), part * self.lf(v, Field::FluidFriction, "FluidFriction")),
            None => (0.0, 0.0),
        }
    }

    /// AActor::physFalling: falling in steps of at most 0.05 s, bounces (bBounce, through
    /// HitWall), landing (processLanded), sliding along walls.
    pub(crate) fn actor_phys_falling(&mut self, a: ObjId, dt: f32, mut iterations: i32) {
        if self.zone_number_of(a) == 0 {
            if self.native_prop(a, Field::Role).as_byte() == 4 && (self.is_a(a, "Pickup") || self.is_a(a, "Decoration")) {
                let l = self.native_prop(a, Field::Location).as_vector();
                self.log.push(format!("{} fell out of the world at {} {} {}!", self.obj_label(a), l[0], l[1], l[2]));
            }
            self.event_pub(a, "FellOutOfWorld", vec![]);
            return;
        }
        let old_accel = self.native_prop(a, Field::Acceleration).as_vector();
        let mut hit = CheckResult::new(1.0);
        let mut remaining = dt;
        let mut bounces = 0;
        loop {
            if remaining <= 0.0 || iterations > 7 {
                self.set_lv(a, Field::Acceleration, "Acceleration", old_accel);
                return;
            }
            iterations += 1;
            let tick = if remaining <= 0.05 {
                remaining
            } else {
                let half = remaining * 0.5;
                if 0.05 > half {
                    half
                } else {
                    0.05
                }
            };
            let old_location = self.native_prop(a, Field::Location).as_vector();
            remaining -= tick;
            self.set_lb(a, Field::JustTeleported, "bJustTeleported", false);
            let mut old_velocity = self.native_prop(a, Field::Velocity).as_vector();
            let (buoyancy, friction) = self.get_net_buoyancy(a);
            let volume = self.native_object(a, Field::PhysicsVolume).or(self.default_volume);
            let gravity = volume.map_or([0.0, 0.0, -950.0], |v| self.lv(v, Field::Gravity, "Gravity"));
            let zone = volume.map_or([0.0; 3], |v| self.lv(v, Field::ZoneVelocity, "ZoneVelocity"));
            let acc = self.native_prop(a, Field::Acceleration).as_vector();
            let g = 1.0 - buoyancy / self.lf(a, Field::Mass, "Mass");
            let half = [(g * gravity[0] + acc[0]) * 0.5 * tick, (g * gravity[1] + acc[1]) * 0.5 * tick, (g * gravity[2] + acc[2]) * 0.5 * tick];
            let f = 1.0 - friction * tick;
            let v = [old_velocity[0] * f + half[0], old_velocity[1] * f + half[1], old_velocity[2] * f + half[2]];
            self.set_lv(a, Field::Velocity, "Velocity", v);
            let adjusted = [(v[0] + zone[0]) * tick, (zone[1] + v[1]) * tick, (zone[2] + v[2]) * tick];
            self.actor_move(a, adjusted, &mut hit);
            if !self.alive_obj(a) || self.native_prop(a, Field::DeleteMe).as_bool() {
                return;
            }
            if hit.time < 1.0 {
                if hit.actor.is_some_and(|o| self.is_a(o, "Pawn")) && self.is_a(a, "Decoration") {
                    let n = self.lp(a, Field::numLandings, "numLandings").as_int() - 1;
                    self.set_prop(a, "numLandings", Value::Int(if n <= 0 { 0 } else { n }));
                }
                let n = hit.normal;
                let nv = Value::vector(n[0], n[1], n[2]);
                if self.lb(a, Field::Bounce, "bBounce") {
                    self.event_pub(a, "HitWall", vec![nv, Value::Object(hit.actor)]);
                    if !self.alive_obj(a) || self.native_prop(a, Field::Physics).as_byte() == PHYS_NONE {
                        return;
                    }
                    if bounces < 2 {
                        remaining += (1.0 - hit.time) * tick;
                    }
                    bounces += 1;
                } else if n[2] >= 0.7 {
                    remaining += (1.0 - hit.time) * tick;
                    if !self.lb(a, Field::JustTeleported, "bJustTeleported") && hit.time > 0.1 && hit.time * tick > 0.003 {
                        let l = self.native_prop(a, Field::Location).as_vector();
                        self.set_lv(a, Field::Velocity, "Velocity", div(sub(l, old_location), hit.time * tick));
                    }
                    self.actor_process_landed(a, n, hit.actor, remaining, iterations);
                    return;
                } else {
                    self.actor_process_hit_wall(a, n, hit.actor);
                    if !self.alive_obj(a) {
                        return;
                    }
                    let old_normal = n;
                    let d = (n[0] * adjusted[0] + adjusted[2] * n[2]) + n[1] * adjusted[1];
                    let t = 1.0 - hit.time;
                    let mut delta = [(adjusted[0] - n[0] * d) * t, (adjusted[1] - n[1] * d) * t, (adjusted[2] - n[2] * d) * t];
                    if (delta[0] * adjusted[0] + adjusted[2] * delta[2]) + adjusted[1] * delta[1] >= 0.0 {
                        if delta[2] > 0.0 {
                            delta = [delta[0] * 0.5, delta[1] * 0.5, delta[2] * 0.5];
                        }
                        self.actor_move(a, delta, &mut hit);
                        if !self.alive_obj(a) {
                            return;
                        }
                        if hit.time < 1.0 {
                            if hit.normal[2] >= 0.7 {
                                self.actor_process_landed(a, hit.normal, hit.actor, 0.0, iterations);
                                return;
                            }
                            self.actor_process_hit_wall(a, hit.normal, hit.actor);
                            if !self.alive_obj(a) {
                                return;
                            }
                            let desired = safe_normal(adjusted);
                            Self::two_wall_adjust(desired, &mut delta, hit.normal, old_normal, hit.time);
                            let ditch = old_normal[2] > 0.0
                                && hit.normal[2] > 0.0
                                && delta[2] == 0.0
                                && (old_normal[0] * hit.normal[0] + old_normal[1] * hit.normal[1]) + old_normal[2] * hit.normal[2] < 0.0;
                            self.actor_move(a, delta, &mut hit);
                            if !self.alive_obj(a) {
                                return;
                            }
                            if ditch || hit.normal[2] >= 0.7 {
                                self.actor_process_landed(a, hit.normal, hit.actor, 0.0, iterations);
                                return;
                            }
                        }
                    }
                    let l = self.native_prop(a, Field::Location).as_vector();
                    let inv = 1.0 / tick;
                    old_velocity[0] = inv * (l[0] - old_location[0]);
                    old_velocity[1] = inv * (l[1] - old_location[1]);
                }
            }
            if !self.lb(a, Field::JustTeleported, "bJustTeleported") && !self.lb(a, Field::Bounce, "bBounce") {
                let l = self.native_prop(a, Field::Location).as_vector();
                let zone = self.native_object(a, Field::PhysicsVolume).or(self.default_volume).map_or([0.0; 3], |v| self.lv(v, Field::ZoneVelocity, "ZoneVelocity"));
                let inv = 1.0 / tick;
                let mut v = [inv * (l[0] - old_location[0]) - zone[0], inv * (l[1] - old_location[1]) - zone[1], inv * (l[2] - old_location[2]) - zone[2]];
                if v[2] < old_velocity[2] || old_velocity[2] >= 0.0 {
                    v = [(v[0] + v[0]) - old_velocity[0], (v[1] + v[1]) - old_velocity[1], (v[2] + v[2]) - old_velocity[2]];
                }
                let terminal = self.native_object(a, Field::PhysicsVolume).or(self.default_volume).map_or(4000.0, |volume| self.lf(volume, Field::TerminalVelocity, "TerminalVelocity"));
                if terminal * terminal < (v[2] * v[2] + v[1] * v[1]) + v[0] * v[0] {
                    v = mul(safe_normal(v), terminal);
                }
                self.set_lv(a, Field::Velocity, "Velocity", v);
            }
        }
    }

    /// AActor::processHitWall: HitWall, except against a pawn.
    fn actor_process_hit_wall(&mut self, a: ObjId, normal: V, other: Option<ObjId>) {
        if let Some(o) = other {
            if !self.is_a(o, "Pawn") {
                self.event_pub(a, "HitWall", vec![Value::vector(normal[0], normal[1], normal[2]), Value::Object(Some(o))]);
            }
        }
    }

    /// processLanded (vt 0x108): ADecoration::processLanded for decorations,
    /// otherwise AActor::processLanded.
    fn actor_process_landed(&mut self, a: ObjId, normal: V, other: Option<ObjId>, remaining: f32, iterations: i32) {
        if self.is_a(a, "Decoration") {
            let mut h = HostDecorationVm { vm: self, a };
            if decoration_landed_core(&mut h) {
                return;
            }
        } else if self.bounce_velocity(a) {
            return;
        }
        self.base_process_landed(a, normal, other, remaining, iterations);
    }

    /// bBounceVelocity volume with ZoneVelocity: the actor starts again (start of both processLanded).
    fn bounce_velocity(&mut self, a: ObjId) -> bool {
        let Some(v) = self.native_object(a, Field::PhysicsVolume) else { return false };
        if !self.lb(v, Field::BounceVelocity, "bBounceVelocity") {
            return false;
        }
        let zone = self.lv(v, Field::ZoneVelocity, "ZoneVelocity");
        if zone == [0.0; 3] {
            return false;
        }
        self.set_lv(a, Field::Velocity, "Velocity", add(zone, [0.0, 0.0, 80.0]));
        true
    }

    /// AActor::processLanded: Landed, the actor stops (PHYS_None); placed along the
    /// slope with bOrientOnSlope.
    fn base_process_landed(&mut self, a: ObjId, normal: V, other: Option<ObjId>, _remaining: f32, _iterations: i32) {
        self.event_pub(a, "Landed", vec![Value::vector(normal[0], normal[1], normal[2])]);
        if !self.alive_obj(a) {
            return;
        }
        if self.native_prop(a, Field::Physics).as_byte() == PHYS_FALLING {
            self.set_physics(a, PHYS_NONE, other, normal);
            self.set_lv(a, Field::Velocity, "Velocity", [0.0; 3]);
        }
        if self.lb(a, Field::OrientOnSlope, "bOrientOnSlope") && self.native_prop(a, Field::Physics).as_byte() == PHYS_NONE {
            let rot = self.native_prop(a, Field::Rotation).as_rotator();
            let r = self.actor_find_slope_rotation(normal, rot);
            let mut hit = CheckResult::new(1.0);
            self.move_actor(a, [0.0; 3], r, &mut hit, false, false, false, false);
        }
    }

    /// AActor::FindSlopeRotation: pitch and roll that place the actor on the ground
    /// with normal `floor`, yaw kept.
    pub(crate) fn actor_find_slope_rotation(&self, floor: V, rot: [i32; 3]) -> [i32; 3] {
        let nearly_zero = floor.iter().all(|c| c.abs() < 0.0001);
        if (floor[2] as f64) >= 0.95 || nearly_zero {
            return [0, rot[1], 0];
        }
        let yaw_dir = rotator_vector([0, rot[1], rot[2]]);
        // (Y + X) + Z.
        let d = (yaw_dir[1] * floor[1] + yaw_dir[0] * floor[0]) + yaw_dir[2] * floor[2];
        let pitch_dir = [yaw_dir[0] - floor[0] * d, yaw_dir[1] - floor[1] * d, yaw_dir[2] - floor[2] * d];
        let pitch = vector_rotation(pitch_dir)[0];
        let roll_dir = [
            pitch_dir[1] * floor[2] - pitch_dir[2] * floor[1],
            pitch_dir[2] * floor[0] - pitch_dir[0] * floor[2],
            floor[1] * pitch_dir[0] - pitch_dir[1] * floor[0],
        ];
        let roll = vector_rotation(roll_dir)[0];
        [pitch, rot[1], roll]
    }

    /// AActor::physProjectile: straight-line flight (water friction,
    /// Acceleration), HitWall on contact, bounces (bBounce, at most two per frame), falling if the
    /// script switches to PHYS_Falling; outside the world, the actor is destroyed.
    pub(crate) fn phys_projectile(&mut self, a: ObjId, dt: f32, iterations: i32) {
        phys_projectile_core(&mut HostVm { vm: self, a }, dt, iterations);
    }

    /// AProjectile::BoundProjectileVelocity: speed clamped to MaxSpeed.
    fn bound_projectile_velocity(&mut self, a: ObjId) {
        let v = bound_speed(self.native_prop(a, Field::Velocity).as_vector(), self.lf(a, Field::MaxSpeed, "MaxSpeed"));
        self.set_lv(a, Field::Velocity, "Velocity", v);
    }

    /// AActor::physicsRotation: rotation at RotationRate (towards DesiredRotation with
    /// bRotateToDesired, continuously with bFixedRotationDir), EndedRotation on arrival.
    fn actor_physics_rotation(&mut self, a: ObjId, dt: f32) {
        let to_desired = self.lb(a, Field::RotateToDesired, "bRotateToDesired");
        let rot = self.native_prop(a, Field::Rotation).as_rotator();
        let desired = self.lp(a, Field::DesiredRotation, "DesiredRotation").as_rotator();
        if !to_desired && !self.lb(a, Field::FixedRotationDir, "bFixedRotationDir") {
            return;
        }
        if to_desired && rot == desired {
            return;
        }
        let rate = self.lp(a, Field::RotationRate, "RotationRate").as_rotator();
        let d_roll = (rate[2] as f32 * dt) as i32;
        let d_yaw = (rate[1] as f32 * dt) as i32;
        let d_pitch = (rate[0] as f32 * dt) as i32;
        let mut new = rot;
        if d_yaw != 0 && (!to_desired || desired[1] != new[1]) {
            new[1] = self.fixed_turn(a, new[1], desired[1], d_yaw);
        }
        if d_pitch != 0 && (!to_desired || desired[0] != new[0]) {
            new[0] = self.fixed_turn(a, new[0], desired[0], d_pitch);
        }
        if d_roll != 0 && (!to_desired || desired[2] != new[2]) {
            new[2] = self.fixed_turn(a, new[2], desired[2], d_roll);
        }
        if new != rot {
            let mut hit = CheckResult::new(1.0);
            self.move_actor(a, [0.0; 3], new, &mut hit, false, false, false, false);
        }
        self.ended_rotation(a);
    }

    fn ended_rotation(&mut self, a: ObjId) {
        if self.alive_obj(a) && self.lb(a, Field::RotateToDesired, "bRotateToDesired") && self.native_prop(a, Field::Rotation).as_rotator() == self.lp(a, Field::DesiredRotation, "DesiredRotation").as_rotator() {
            self.event_pub(a, "EndedRotation", vec![]);
        }
    }

    /// AProjectile::physicsRotation: the projectile's axes turn around its
    /// own axes (yaw around Z, pitch around Y, roll around X).
    fn projectile_physics_rotation(&mut self, a: ObjId, dt: f32) {
        let to_desired = self.lb(a, Field::RotateToDesired, "bRotateToDesired");
        let rot = self.native_prop(a, Field::Rotation).as_rotator();
        if !to_desired && !self.lb(a, Field::FixedRotationDir, "bFixedRotationDir") {
            return;
        }
        if to_desired && rot == self.lp(a, Field::DesiredRotation, "DesiredRotation").as_rotator() {
            return;
        }
        let c = openxiii_assets::coords::div_rotator(&openxiii_assets::coords::UNIT, rot);
        let (mut x, mut y, mut z) = (c.x, c.y, c.z);
        let rate = self.lp(a, Field::RotationRate, "RotationRate").as_rotator();
        if rate[1] != 0 {
            let ang = (rate[1] as f32 * dt) as i32;
            x = rotate_angle_axis(x, ang, z);
            y = rotate_angle_axis(y, ang, z);
        }
        if rate[0] != 0 {
            let ang = (rate[0] as f32 * dt) as i32;
            x = rotate_angle_axis(x, ang, y);
            z = rotate_angle_axis(z, ang, y);
        }
        if rate[2] != 0 {
            let ang = (rate[2] as f32 * dt) as i32;
            z = rotate_angle_axis(z, ang, x);
            y = rotate_angle_axis(y, ang, x);
        }
        // FCoords::OrthoRotation of the rotated frame.
        let new = crate::world::ortho_rotation([x, y, z]);
        if new != rot {
            let mut hit = CheckResult::new(1.0);
            self.move_actor(a, [0.0; 3], new, &mut hit, false, false, false, false);
        }
        self.ended_rotation(a);
    }
}

/// What AActor::physProjectile reads and calls: the actor, its volume, ULevel::MoveActor, the
/// script events and physFalling (a trait, so that tests can supply the answers).
pub(crate) trait HostProjectile {
    /// Region.ZoneNumber zero (+0xA0): outside the world.
    fn zone_null(&mut self) -> bool;
    /// ULevel::DestroyActor (XLevel vtable +0xA8).
    fn destroy(&mut self);
    fn location(&mut self) -> V;
    fn velocity(&mut self) -> V;
    fn set_velocity(&mut self, v: V);
    fn acceleration(&mut self) -> V;
    /// PhysicsVolume: FluidFriction (+0x2B0) if bWaterVolume (+0x2CC bit 0x20), otherwise 0.
    fn friction_water(&mut self) -> f32;
    /// Vtable +0x100: AProjectile::BoundProjectileVelocity, nothing for another actor.
    fn bound_speed(&mut self);
    /// ULevel::MoveActor (XLevel vtable +0xA0), Delta, current rotation, without test.
    fn moves(&mut self, delta: V, hit: &mut CheckResult);
    fn alive_value(&mut self) -> bool;
    /// bDeleteMe (+0x2E bit 0).
    fn removes(&mut self) -> bool;
    /// bJustTeleported (+0x34 bit 0x8000000).
    fn teleports(&mut self) -> bool;
    fn clears_teleports(&mut self);
    /// bBounce (+0x34 bit 0x800000).
    fn bounces_list(&mut self) -> bool;
    fn physics(&mut self) -> u8;
    /// AActor::eventHitWall.
    fn hit_wall(&mut self, normal: V, other_one: Option<ObjId>);
    /// AActor::physFalling (vtable +0x10C).
    fn fall(&mut self, dt: f32, iterations: i32);
}

/// AProjectile::BoundProjectileVelocity: if MaxSpeed² < (X² + Y²) + Z², the speed
/// becomes MaxSpeed × SafeNormal.
pub(crate) fn bound_speed(v: V, max: f32) -> V {
    if max * max < (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2] {
        let n = safe_normal(v);
        [max * n[0], max * n[1], max * n[2]]
    } else {
        v
    }
}

/// AActor::physProjectile.
pub(crate) fn phys_projectile_core(h: &mut impl HostProjectile, dt: f32, mut iterations: i32) {
    if h.zone_null() {
        h.destroy();
        return;
    }
    let old_location = h.location();
    h.clears_teleports();
    let mut bounces = 0;
    let mut remaining = dt;
    // FCheckResult set once; Time reset to 1 before each MoveActor.
    let mut hit = CheckResult::new(1.0);
    while remaining > 0.0 {
        iterations += 1;
        // 1 − (FluidFriction × remainder) × 0.2, V × f, then remainder × A + V.
        let f = 1.0 - h.friction_water() * remaining * 0.2;
        let v = h.velocity();
        let v = [f * v[0], f * v[1], f * v[2]];
        let acc = h.acceleration();
        h.set_velocity([remaining * acc[0] + v[0], remaining * acc[1] + v[1], remaining * acc[2] + v[2]]);
        h.bound_speed();
        let tick = remaining;
        let v = h.velocity();
        // DeltaTime × V, not the remainder.
        let adjusted = [dt * v[0], dt * v[1], dt * v[2]];
        hit.time = 1.0;
        remaining = 0.0;
        h.moves(adjusted, &mut hit);
        if !h.alive_value() {
            return;
        }
        if hit.time < 1.0 && !h.removes() && !h.teleports() {
            h.hit_wall(hit.normal, hit.actor);
            if !h.alive_value() {
                return;
            }
            if h.bounces_list() {
                if bounces < 2 {
                    remaining = (1.0 - hit.time) * tick;
                }
                bounces += 1;
                if h.physics() == PHYS_FALLING {
                    h.fall(remaining, iterations);
                    if !h.alive_value() {
                        return;
                    }
                }
            }
        }
    }
    if !h.teleports() && !h.bounces_list() {
        let l = h.location();
        let k = 1.0 / dt;
        h.set_velocity([k * (l[0] - old_location[0]), (l[1] - old_location[1]) * k, (l[2] - old_location[2]) * k]);
    }
}

/// What ADecoration::processLanded reads: the actor, its volume and
/// ULevel::SingleLineCheck (XLevel vtable +0xD0).
pub(crate) trait DecorationHost {
    /// PhysicsVolume: ZoneVelocity if bBounceVelocity (+0x2CC bit 3).
    fn zone_bounce(&mut self) -> Option<V>;
    fn location(&mut self) -> V;
    fn velocity(&mut self) -> V;
    fn set_velocity(&mut self, v: V);
    fn radius_list(&mut self) -> f32;
    fn height_value(&mut self) -> f32;
    fn landings_list(&mut self) -> i32;
    fn set_landings(&mut self, n: i32);
    /// ULevel::SingleLineCheck: return value (true without a contact) and Hit.Actor non-null.
    fn trace(&mut self, end: V, start: V, flags: u32, extent: V) -> (bool, bool);
}

/// ADecoration::processLanded up to the call to AActor::processLanded: returns true if
/// the decoration starts again (bBounceVelocity volume, or placed on an edge: it tips towards the side where the ground
/// is missing under its corners), false if AActor::processLanded must be called.
pub(crate) fn decoration_landed_core(h: &mut impl DecorationHost) -> bool {
    // ZoneVelocity != (0,0,0): Velocity = ZoneVelocity + (0, 0, 80).
    if let Some(zone) = h.zone_bounce() {
        if zone != [0.0; 3] {
            h.set_velocity(add(zone, [0.0, 0.0, 80.0]));
            return true;
        }
    }
    let landings = h.landings_list();
    if landings < 5 {
        let l = h.location();
        let (r, ch) = (h.radius_list(), h.height_value());
        // Start = Location − (0, 0, CollisionHeight × 0.8 in double); End = Location −
        // (0, 0, (CollisionHeight + CollisionRadius) + 8); 0x2BF, without extent.
        let start = sub(l, [0.0, 0.0, fmul_d(ch, 0.8)]);
        let end = sub(l, [0.0, 0.0, (ch + r) + 8.0]);
        let (_, key_col) = h.trace(end, start, 0x2bf, [0.0; 3]);
        if !key_col {
            // GetCylinderExtent × 0.5, Z doubled.
            let mut extent = mul([r, r, ch], 0.5);
            extent[2] += extent[2];
            let half = fmul_d(r, 0.5);
            let mhalf = fmul_d(r, -0.5);
            let mut corner = |x: f32, y: f32| -> i32 { h.trace(add(l, [x, y, -8.0]), add(l, [x, y, 0.0]), 0x2df, extent).0 as i32 };
            let c1 = corner(half, half);
            let c2 = corner(mhalf, half);
            let c3 = corner(mhalf, mhalf);
            let c4 = corner(half, mhalf);
            if c1 + c2 + c3 + c4 > 1 && c1 + c3 != 0 && c2 + c4 != 0 {
                h.set_landings(landings + 1);
                let dir = [(c1 + c4 - c2 - c3) as f32, (c1 + c2 - c3 - c4) as f32, 0.5];
                // FClamp(−1 × Velocity.Z, 30, 30 + CollisionRadius) × 2.
                let vz = -1.0 * h.velocity()[2];
                let max = 30.0 + r;
                let clamped = if vz < 30.0 {
                    30.0
                } else if vz < max {
                    vz
                } else {
                    max
                };
                let k = clamped + clamped;
                h.set_velocity([k * dir[0], k * dir[1], k * dir[2]]);
                return true;
            }
        }
    }
    h.set_landings(0);
    false
}

struct HostDecorationVm<'a> {
    vm: &'a mut Vm,
    a: ObjId,
}

/// Host of physProjectile in the VM: the actor's properties.
struct HostVm<'a> {
    vm: &'a mut Vm,
    a: ObjId,
}

impl HostProjectile for HostVm<'_> {
    fn zone_null(&mut self) -> bool {
        self.vm.zone_number_of(self.a) == 0
    }
    fn destroy(&mut self) {
        let _ = crate::natives::call(self.vm, "Destroy", self.a, &mut [], &[]);
    }
    fn location(&mut self) -> V {
        self.vm.native_prop(self.a, Field::Location).as_vector()
    }
    fn velocity(&mut self) -> V {
        self.vm.native_prop(self.a, Field::Velocity).as_vector()
    }
    fn set_velocity(&mut self, v: V) {
        self.vm.set_v(self.a, "Velocity", v);
    }
    fn acceleration(&mut self) -> V {
        self.vm.native_prop(self.a, Field::Acceleration).as_vector()
    }
    fn friction_water(&mut self) -> f32 {
        let volume = self.vm.native_object(self.a, Field::PhysicsVolume).or(self.vm.default_volume);
        match volume {
            Some(v) if self.vm.pb(v, "bWaterVolume") => self.vm.pf(v, "FluidFriction"),
            _ => 0.0,
        }
    }
    fn bound_speed(&mut self) {
        if self.vm.is_a(self.a, "Projectile") {
            self.vm.bound_projectile_velocity(self.a);
        }
    }
    fn moves(&mut self, delta: V, hit: &mut CheckResult) {
        self.vm.actor_move(self.a, delta, hit);
    }
    fn alive_value(&mut self) -> bool {
        self.vm.alive_obj(self.a)
    }
    fn removes(&mut self) -> bool {
        self.vm.native_prop(self.a, Field::DeleteMe).as_bool()
    }
    fn teleports(&mut self) -> bool {
        self.vm.pb(self.a, "bJustTeleported")
    }
    fn clears_teleports(&mut self) {
        self.vm.set_b(self.a, "bJustTeleported", false);
    }
    fn bounces_list(&mut self) -> bool {
        self.vm.pb(self.a, "bBounce")
    }
    fn physics(&mut self) -> u8 {
        self.vm.native_prop(self.a, Field::Physics).as_byte()
    }
    fn hit_wall(&mut self, n: V, other_one: Option<ObjId>) {
        self.vm.event_pub(self.a, "HitWall", vec![Value::vector(n[0], n[1], n[2]), Value::Object(other_one)]);
    }
    fn fall(&mut self, dt: f32, iterations: i32) {
        self.vm.actor_phys_falling(self.a, dt, iterations);
    }
}

impl DecorationHost for HostDecorationVm<'_> {
    fn zone_bounce(&mut self) -> Option<V> {
        let v = self.vm.native_object(self.a, Field::PhysicsVolume)?;
        self.vm.pb(v, "bBounceVelocity").then(|| self.vm.pv(v, "ZoneVelocity"))
    }
    fn location(&mut self) -> V {
        self.vm.native_prop(self.a, Field::Location).as_vector()
    }
    fn velocity(&mut self) -> V {
        self.vm.native_prop(self.a, Field::Velocity).as_vector()
    }
    fn set_velocity(&mut self, v: V) {
        self.vm.set_v(self.a, "Velocity", v);
    }
    fn radius_list(&mut self) -> f32 {
        self.vm.native_prop(self.a, Field::CollisionRadius).as_float()
    }
    fn height_value(&mut self) -> f32 {
        self.vm.native_prop(self.a, Field::CollisionHeight).as_float()
    }
    fn landings_list(&mut self) -> i32 {
        self.vm.propr(self.a, "numLandings").as_int()
    }
    fn set_landings(&mut self, n: i32) {
        self.vm.set_prop(self.a, "numLandings", Value::Int(n));
    }
    fn trace(&mut self, end: V, start: V, flags: u32, extent: V) -> (bool, bool) {
        let mut hit = CheckResult::new(1.0);
        let r = self.vm.single_line_check(&mut hit, Some(self.a), end, start, flags, extent);
        (r, hit.actor.is_some())
    }
}
