//! Latent movement of controllers, reimplemented from Engine.dll: latent actions (StateFrame->
//! LatentAction and their Poll* natives), MoveTo / MoveToward, APawn::moveToward,
//! ReachedDestination, setMoveTimer, FinishRotation, WaitForLanding, FinishInterpolation,
//! StopWaiting, EndClimbLadder, PickWallAdjust, SuggestJumpVelocity; and the movement natives
//! of XIII's soldiers in XIDPawn.dll (NearWall, TestDirection, PseudoSteering).
//! Addresses at the top of each function; floating-point operations follow the machine code's
//! order (FPU at 24-bit precision).

#![allow(clippy::neg_multiply)] // the "× -1" reproduce the machine code's FMUL -1

use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;

/// StateFrame->LatentAction: number of the Poll* native that AActor::ProcessState calls.
pub(crate) const EPOLL_FINISH_INTERPOLATION: u16 = 0x12e;
pub(crate) const EPOLL_SLEEP: u16 = 0x180;
/// UGUIPage::execSleep (GUI.dll): native 431, polled by execPollSleep.
pub(crate) const GUI_POLL_SLEEP: u16 = 0x1AF;
pub(crate) const EPOLL_FINISH_ANIM: u16 = 0x181;
pub(crate) const AI_POLL_MOVE_TO: u16 = 0x1f5;
pub(crate) const AI_POLL_MOVE_TOWARD: u16 = 0x1f7;
pub(crate) const AI_POLL_FINISH_ROTATION: u16 = 0x1fd;
pub(crate) const AI_POLL_WAIT_FOR_LANDING: u16 = 0x210;

/// FVector::SizeSquared: Z, Y then X loaded on the x87 stack, (X² + Y²) + Z².
pub(crate) fn size_sq(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]
}

/// FVector::Size: (X² + Y²) + Z² in single, appSqrt in double. The sizes
/// computed inline that load X, Y then Z do (Z² + Y²) + X² (`physics::size`).
pub(crate) fn vsize(a: V) -> f32 {
    (size_sq(a) as f64).sqrt() as f32
}

/// FVector / FLOAT: multiplication by the inverse.
pub(crate) fn div(a: V, f: f32) -> V {
    let r = 1.0 / f;
    [r * a[0], r * a[1], r * a[2]]
}

/// FRotator::Vector: X axis of GMath.UnitCoords / R (GMath's sine table).
pub(crate) fn rotator_vector(r: [i32; 3]) -> V {
    openxiii_assets::coords::div_rotator(&openxiii_assets::coords::UNIT, r).x
}

/// Product of a float by a double constant of the machine code (FMUL qword), rounded at
/// 24-bit precision: the exact product (77 bits) is rounded only once, like the FPU. The
/// product in double then single only differs if it falls halfway between two floats;
/// the exact error of the double product (FMA) then breaks the tie.
pub(crate) fn fmul_d(a: f32, b: f64) -> f32 {
    let p = a as f64 * b;
    let r = p as f32;
    let e = (a as f64).mul_add(b, -p);
    let d = p - r as f64;
    if e == 0.0 || d == 0.0 || !r.is_finite() {
        return r;
    }
    let other = if d > 0.0 { r.next_up() } else { r.next_down() };
    if p == (r as f64 + other as f64) * 0.5 && (e > 0.0) == (d > 0.0) {
        return other;
    }
    r
}

/// What PollMoveToward reads and writes: fields of the controller (MoveTarget, NextMoveTarget,
/// bPreparingMove, bAdjusting, AdjustLoc, Destination, MoveTimer, LatentAction) and of the pawn
/// (Physics, Floor, DesiredSpeed, bIsWalking, bCanSwim), the targets (Location, CollisionHeight,
/// CollisionRadius, pawn or not, in the water), APawn::moveToward and UpdateTactics.
pub(crate) trait WorldPoll {
    fn move_target(&self) -> Option<ObjId>;
    fn next_move_target(&self) -> Option<ObjId>;
    fn to_pawn(&self) -> bool;
    fn preparing(&self) -> bool;
    fn adjusting(&self) -> bool;
    fn set_adjusting(&mut self, v: bool);
    fn adjust_loc(&self) -> V;
    fn destination(&self) -> V;
    fn set_destination(&mut self, v: V);
    fn location(&self, a: ObjId) -> V;
    fn height_value(&self, a: ObjId) -> f32;
    fn radius_list(&self, a: ObjId) -> f32;
    fn is_pawn(&self, a: ObjId) -> bool;
    fn in_water_volume(&self, a: ObjId) -> bool;
    fn physics(&self) -> u8;
    fn floor(&self) -> V;
    fn desired_speed(&self) -> f32;
    fn set_desired_speed(&mut self, v: f32);
    fn can_swim(&self) -> bool;
    fn stops(&mut self);
    fn end_latent(&mut self);
    fn set_move_timer(&mut self, v: f32);
    fn advanced_tactics(&self) -> bool;
    /// Level.TimeSeconds (single).
    fn time_value(&self) -> f32;
    fn tactical_offset(&self) -> f32;
    fn set_tactical_offset(&mut self, v: f32);
    fn event_update_tactics(&mut self);
    fn move_toward(&mut self, dest: V, goal: Option<ObjId>, next: Option<ObjId>) -> bool;
}

/// AController::execPollMoveToward without the world.
pub(crate) fn poll_to(m: &mut impl WorldPoll) {
    let (Some(mt), true) = (m.move_target(), m.to_pawn()) else {
        m.end_latent();
        return;
    };
    if m.preparing() {
        return;
    }
    if m.adjusting() {
        let adjust = m.adjust_loc();
        let next = m.next_move_target();
        let reached = m.move_toward(adjust, Some(mt), next);
        m.set_adjusting(!reached);
    }
    if m.adjusting() {
        return;
    }
    let mut dest = m.location(mt);
    let physics = m.physics();
    if physics == PHYS_FLYING && m.is_pawn(mt) {
        dest[2] += fmul_d(m.height_value(mt), 0.7);
    } else if physics == PHYS_SPIDER {
        dest = sub(dest, mul(m.floor(), m.radius_list(mt)));
    }
    m.set_destination(dest);
    let old_desired = m.desired_speed();
    // bAdvancedTactics: UpdateTactics at most every 0.5 s (TacticalOffset).
    if m.advanced_tactics() {
        let now = m.time_value();
        if m.tactical_offset() < now - 0.5 {
            m.set_tactical_offset(now);
            m.event_update_tactics();
        }
    }
    let next = m.next_move_target();
    // MoveTarget and Destination may have changed during UpdateTactics: the engine reads the
    // fields again (moveToward receives &Destination).
    let mt_now = m.move_target();
    if m.move_toward(m.destination(), mt_now, next) {
        m.end_latent();
        m.stops();
    }
    let Some(mt) = m.move_target() else { return };
    let l = m.location(mt);
    m.set_destination(l);
    if m.is_pawn(mt) {
        m.set_desired_speed(old_desired);
        if !m.can_swim() && m.in_water_volume(mt) {
            m.set_move_timer(-1.0);
        }
    }
}

/// PollMoveToward in the VM.
struct PollVm<'a> {
    vm: &'a mut Vm,
    c: ObjId,
}

impl PollVm<'_> {
    fn p(&self) -> Option<ObjId> {
        self.vm.native_object(self.c, crate::NativeField::Pawn)
    }
}

impl WorldPoll for PollVm<'_> {
    fn move_target(&self) -> Option<ObjId> {
        self.vm.po(self.c, "MoveTarget")
    }
    fn next_move_target(&self) -> Option<ObjId> {
        self.vm.po(self.c, "NextMoveTarget")
    }
    fn to_pawn(&self) -> bool {
        self.p().is_some()
    }
    fn preparing(&self) -> bool {
        self.vm.pb(self.c, "bPreparingMove")
    }
    fn adjusting(&self) -> bool {
        self.vm.pb(self.c, "bAdjusting")
    }
    fn set_adjusting(&mut self, v: bool) {
        self.vm.set_b(self.c, "bAdjusting", v)
    }
    fn adjust_loc(&self) -> V {
        self.vm.pv(self.c, "AdjustLoc")
    }
    fn destination(&self) -> V {
        self.vm.pv(self.c, "Destination")
    }
    fn set_destination(&mut self, v: V) {
        self.vm.set_v(self.c, "Destination", v)
    }
    fn location(&self, a: ObjId) -> V {
        self.vm.native_prop(a, crate::NativeField::Location).as_vector()
    }
    fn height_value(&self, a: ObjId) -> f32 {
        self.vm.native_prop(a, crate::NativeField::CollisionHeight).as_float()
    }
    fn radius_list(&self, a: ObjId) -> f32 {
        self.vm.native_prop(a, crate::NativeField::CollisionRadius).as_float()
    }
    fn is_pawn(&self, a: ObjId) -> bool {
        self.vm.is_a(a, "Pawn")
    }
    fn in_water_volume(&self, a: ObjId) -> bool {
        self.vm.native_object(a, crate::NativeField::PhysicsVolume).is_some_and(|v| self.vm.pb(v, "bWaterVolume"))
    }
    fn physics(&self) -> u8 {
        self.p().map_or(0, |p| self.vm.propr(p, "Physics").as_byte())
    }
    fn floor(&self) -> V {
        self.p().map_or([0.0; 3], |p| self.vm.pv(p, "Floor"))
    }
    fn desired_speed(&self) -> f32 {
        self.p().map_or(0.0, |p| self.vm.pf(p, "DesiredSpeed"))
    }
    fn set_desired_speed(&mut self, v: f32) {
        if let Some(p) = self.p() {
            self.vm.set_f(p, "DesiredSpeed", v)
        }
    }
    fn can_swim(&self) -> bool {
        self.p().is_some_and(|p| self.vm.pb(p, "bCanSwim"))
    }
    fn stops(&mut self) {
        if let Some(p) = self.p() {
            self.vm.set_b(p, "bIsWalking", false)
        }
    }
    fn end_latent(&mut self) {
        self.vm.set_latent_action(self.c, 0)
    }
    fn set_move_timer(&mut self, v: f32) {
        self.vm.set_f(self.c, "MoveTimer", v)
    }
    fn advanced_tactics(&self) -> bool {
        self.vm.pb(self.c, "bAdvancedTactics")
    }
    fn time_value(&self) -> f32 {
        self.vm.level.map_or(0.0, |l| self.vm.native_prop(l, crate::NativeField::TimeSeconds).as_float())
    }
    fn tactical_offset(&self) -> f32 {
        self.vm.pf(self.c, "TacticalOffset")
    }
    fn set_tactical_offset(&mut self, v: f32) {
        self.vm.set_f(self.c, "TacticalOffset", v)
    }
    fn event_update_tactics(&mut self) {
        // AController::eventUpdateTactics.
        self.vm.event_pub(self.c, "UpdateTactics", vec![]);
    }
    fn move_toward(&mut self, dest: V, goal: Option<ObjId>, next: Option<ObjId>) -> bool {
        let Some(p) = self.p() else { return false };
        self.vm.move_toward(p, dest, goal, next)
    }
}

/// (the three normalizations of the move towards NextTarget in APawn::moveToward): V × appRSqrt(|V|²) returned
/// by value, or the zero vector if |V|² < 1e-8; unlike FVector::SafeNormal (physics::safe_normal),
/// a NaN norm also gives the zero vector (fcomp then "test ah, 1").
pub(crate) fn normal_move_toward(v: V) -> V {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if !((sq as f64) >= 1e-8) {
        return [0.0; 3];
    }
    mul(v, 1.0 / sq.sqrt())
}

impl Vm {
    // ---- Latent actions ----

    /// StateFrame->LatentAction of the object (0 without a latent action or without state code).
    pub(crate) fn latent_action(&self, obj: ObjId) -> u16 {
        self.threads.get(&obj).map_or(0, |t| t.latent)
    }

    /// GetStateFrame()->LatentAction = `id`.
    pub(crate) fn set_latent_action(&mut self, obj: ObjId, id: u16) {
        if let Some(t) = self.threads.get_mut(&obj) {
            t.latent = id;
        }
    }

    /// AActor::execPollFinishAnim: vt 0xCC (CheckAnimFinished) of channel (INT)LatentFloat;
    /// true when LatentAction goes back to 0.
    pub(crate) fn poll_finish_anim(&mut self, a: ObjId) -> bool {
        let ch = self.pf(a, "LatentFloat") as i32;
        self.check_anim_finished(a, ch)
    }

    /// (this->*GNatives[LatentAction])(Stack, &DeltaSeconds), at the start of AActor::ProcessState
    pub(crate) fn poll_latent(&mut self, a: ObjId, latent: u16, dt: f32) {
        match latent {
            // UGUIPage::execPollSleep (GUI): same computation as for actors.
            EPOLL_SLEEP | GUI_POLL_SLEEP => self.poll_sleep(a, dt),
            EPOLL_FINISH_ANIM => {
                if self.poll_finish_anim(a) {
                    self.set_latent_action(a, 0);
                }
            }
            EPOLL_FINISH_INTERPOLATION => self.poll_finish_interpolation(a),
            AI_POLL_MOVE_TO => self.poll_move_to(a),
            AI_POLL_MOVE_TOWARD => self.poll_move_toward(a),
            AI_POLL_FINISH_ROTATION => self.poll_finish_rotation(a),
            AI_POLL_WAIT_FOR_LANDING => self.poll_wait_for_landing(a, dt),
            _ => self.set_latent_action(a, 0),
        }
    }

    /// AActor::execSleep: LatentAction = EPOLL_Sleep, LatentFloat = Seconds.
    pub(crate) fn exec_sleep(&mut self, a: ObjId, seconds: f32) {
        self.set_latent_action(a, EPOLL_SLEEP);
        self.set_f(a, "LatentFloat", seconds);
    }

    /// AActor::execPollSleep: LatentFloat -= DeltaSeconds; wakes up when it goes
    /// below DeltaSeconds × 0.5.
    fn poll_sleep(&mut self, a: ObjId, dt: f32) {
        let left = self.pf(a, "LatentFloat") - dt;
        self.set_f(a, "LatentFloat", left);
        if fmul_d(dt, 0.5) > left {
            self.set_latent_action(a, 0);
        }
    }

    /// AActor::execPollFinishInterpolation: ends when bInterpolating drops.
    fn poll_finish_interpolation(&mut self, a: ObjId) {
        if !self.pb(a, "bInterpolating") {
            self.set_latent_action(a, 0);
        }
    }

    /// AController::execStopWaiting: a Sleep in progress ends at the next
    /// pass (LatentFloat = -1).
    pub(crate) fn exec_stop_waiting(&mut self, c: ObjId) {
        if self.latent_action(c) == EPOLL_SLEEP {
            self.set_f(c, "LatentFloat", -1.0);
        }
    }

    // ---- MoveTo / MoveToward ----

    /// Speed requested by MoveTo / MoveToward: bIsWalking if Speed ≤ WalkingPct (Speed is
    /// then relative to walking), bReducedSpeed drops, DesiredSpeed = Speed clamped by
    /// MaxDesiredSpeed (zero if the latter is negative).
    fn set_move_speed(&mut self, p: ObjId, speed: f32) {
        let walking_pct = self.pf(p, "WalkingPct");
        let walking = speed <= walking_pct;
        self.set_b(p, "bIsWalking", walking);
        let speed = if walking { speed / walking_pct } else { speed };
        self.set_b(p, "bReducedSpeed", false);
        let max = self.pf(p, "MaxDesiredSpeed");
        let desired = if max < 0.0 {
            0.0
        } else if max < speed {
            max
        } else {
            speed
        };
        self.set_f(p, "DesiredSpeed", desired);
    }

    /// AController::execMoveTo.
    pub(crate) fn exec_move_to(&mut self, c: ObjId, dest: V, view_focus: Option<ObjId>, speed: f32) {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return };
        let walking_pct = self.pf(p, "WalkingPct");
        let walking = speed <= walking_pct;
        self.set_b(p, "bIsWalking", walking);
        let speed = if walking { speed / walking_pct } else { speed };
        // Inline, X, Y then Z: (Z² + Y²) + X², appSqrt in double.
        let d = sub(dest, self.native_prop(p, crate::NativeField::Location).as_vector());
        let dist = (((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]) as f64).sqrt() as f32;
        self.set_prop(c, "MoveTarget", Value::Object(None));
        self.set_b(p, "bReducedSpeed", false);
        let max = self.pf(p, "MaxDesiredSpeed");
        let desired = if max < 0.0 {
            0.0
        } else if max < speed {
            max
        } else {
            speed
        };
        self.set_f(p, "DesiredSpeed", desired);
        self.set_prop(c, "Focus", Value::Object(view_focus));
        self.pawn_set_move_timer(p, dist);
        self.set_latent_action(c, AI_POLL_MOVE_TO);
        self.set_v(c, "Destination", dest);
        if self.po(c, "Focus").is_none() {
            self.set_v(c, "FocalPoint", dest);
        }
        self.set_b(c, "bAdjusting", false);
        self.clear_current_path(c);
        let next = self.po(c, "NextMoveTarget");
        self.move_toward(p, dest, None, next);
    }

    /// AController::execPollMoveTo.
    fn poll_move_to(&mut self, c: ObjId) {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else {
            self.set_latent_action(c, 0);
            return;
        };
        if self.pb(c, "bAdjusting") {
            let adjust = self.pv(c, "AdjustLoc");
            let next = self.po(c, "NextMoveTarget");
            let reached = self.move_toward(p, adjust, None, next);
            self.set_b(c, "bAdjusting", !reached);
        }
        if !self.pb(c, "bAdjusting") {
            self.update_tactics(c);
            let dest = self.pv(c, "Destination");
            let next = self.po(c, "NextMoveTarget");
            if self.move_toward(p, dest, None, next) {
                self.set_latent_action(c, 0);
                self.set_b(p, "bIsWalking", false);
            }
        }
    }

    /// bAdvancedTactics: UpdateTactics at most every 0.5 s (TacticalOffset), in the
    /// Poll* functions of MoveTo / MoveToward.
    fn update_tactics(&mut self, c: ObjId) {
        if !self.pb(c, "bAdvancedTactics") {
            return;
        }
        let now = self.level.map_or(0.0, |l| self.native_prop(l, crate::NativeField::TimeSeconds).as_float());
        if self.pf(c, "TacticalOffset") < now - 0.5 {
            self.set_f(c, "TacticalOffset", now);
            // AController::eventUpdateTactics.
            self.event_pub(c, "UpdateTactics", vec![]);
        }
    }

    /// AController::execMoveToward.
    pub(crate) fn exec_move_toward(&mut self, c: ObjId, target: Option<ObjId>, view_focus: Option<ObjId>, speed: f32, next: Option<ObjId>) {
        let (Some(t), Some(p)) = (target, self.native_object(c, crate::NativeField::Pawn)) else { return };
        let to_target = sub(self.native_prop(t, crate::NativeField::Location).as_vector(), self.native_prop(p, crate::NativeField::Location).as_vector());
        self.set_move_speed(p, speed);
        self.set_prop(c, "MoveTarget", Value::Object(Some(t)));
        self.set_prop(c, "NextMoveTarget", Value::Object(next));
        self.set_prop(c, "Focus", Value::Object(view_focus));
        if self.is_a(t, "Pawn") {
            self.set_f(c, "MoveTimer", 1.2);
        } else {
            self.pawn_set_move_timer(p, vsize(to_target));
        }
        let dest = self.native_prop(t, crate::NativeField::Location).as_vector();
        self.set_v(c, "Destination", dest);
        self.set_latent_action(c, AI_POLL_MOVE_TOWARD);
        self.set_b(c, "bAdjusting", false);
        let nav_goal = Some(t).filter(|&t| self.is_a(t, "NavigationPoint"));
        self.clear_current_path(c);
        if let Some(nav) = nav_goal {
            if self.pb(nav, "bSpecialMove") {
                // ANavigationPoint::eventSuggestMovePreparation.
                self.event_ret(nav, "SuggestMovePreparation", vec![Value::Object(Some(p))]);
            }
            if self.valid_anchor(p) {
                if let Some(anchor) = self.po(p, "Anchor") {
                    let found = self.nav.get(&anchor).and_then(|specs| specs.iter().position(|s| s.end == t));
                    if let Some(i) = found {
                        self.set_current_path(c, anchor, i);
                        let spec = self.nav[&anchor][i];
                        let (r, h) = (self.native_prop(p, crate::NativeField::CollisionRadius).as_float() as i32, self.native_prop(p, crate::NativeField::CollisionHeight).as_float() as i32);
                        let flags = self.calc_move_flags(p);
                        let fall = self.pf(p, "MaxFallSpeed") as i32;
                        if !spec.supports(r, h, flags, fall) {
                            // PrepareForMove(Goal, Path): the native ReachSpec has no script
                            // object (the event is empty in Controller and AIController).
                            // AController::eventPrepareForMove.
                            self.event_pub(c, "PrepareForMove", vec![Value::Object(Some(nav)), Value::Object(None)]);
                        }
                    }
                }
            }
        }
        if !self.pb(c, "bPreparingMove") {
            let mt = self.po(c, "MoveTarget");
            self.move_toward(p, dest, mt, next);
        }
        if self.pb(c, "bAdvancedTactics") {
            if let Some(mt) = self.po(c, "MoveTarget") {
                if self.is_a(mt, "NavigationPoint") && self.pb(mt, "bNeverUseStrafing") {
                    self.set_b(c, "bAdvancedTactics", false);
                }
            }
        }
    }

    /// AController::execPollMoveToward: see `poll_to`.
    fn poll_move_toward(&mut self, c: ObjId) {
        poll_to(&mut PollVm { vm: self, c });
    }

    /// AController::execFinishRotation.
    pub(crate) fn exec_finish_rotation(&mut self, c: ObjId) {
        self.set_latent_action(c, AI_POLL_FINISH_ROTATION);
    }

    /// AController::execPollFinishRotation: ends when the pawn's yaw is less than
    /// 2000 units from DesiredRotation.Yaw (or more than 0xF830, on the other side of the turn).
    fn poll_finish_rotation(&mut self, c: ObjId) {
        if let Some(p) = self.native_object(c, crate::NativeField::Pawn) {
            let d = self.propr(p, "DesiredRotation").as_rotator()[1].wrapping_sub(self.propr(p, "Rotation").as_rotator()[1] & 0xffff);
            let a = d.wrapping_abs();
            if a > 1999 && a < 0xf830 {
                return;
            }
        }
        self.set_latent_action(c, 0);
    }

    /// AController::execWaitForLanding: LatentFloat = 4; latent action only
    /// if the pawn is falling.
    pub(crate) fn exec_wait_for_landing(&mut self, c: ObjId) {
        self.set_f(c, "LatentFloat", 4.0);
        if self.native_object(c, crate::NativeField::Pawn).is_some_and(|p| self.propr(p, "Physics").as_byte() == PHYS_FALLING) {
            self.set_latent_action(c, AI_POLL_WAIT_FOR_LANDING);
        }
    }

    /// AController::execPollWaitForLanding: ends on landing; LongFall when
    /// the 4 s have elapsed (and at each pass afterwards).
    fn poll_wait_for_landing(&mut self, c: ObjId, dt: f32) {
        if self.native_object(c, crate::NativeField::Pawn).is_some_and(|p| self.propr(p, "Physics").as_byte() != PHYS_FALLING) {
            self.set_latent_action(c, 0);
            return;
        }
        let left = self.pf(c, "LatentFloat") - dt;
        self.set_f(c, "LatentFloat", left);
        if left < 0.0 {
            // AController::eventLongFall, called by its FName from execPollWaitForLanding.
            self.event_pub(c, "LongFall", vec![]);
        }
    }

    /// AActor::execFinishInterpolation.
    pub(crate) fn exec_finish_interpolation(&mut self, a: ObjId) {
        self.set_latent_action(a, EPOLL_FINISH_INTERPOLATION);
    }

    /// AController::execEndClimbLadder: a MoveToward towards the ladder ends;
    /// it becomes the Anchor of the pawn that touches it.
    pub(crate) fn exec_end_climb_ladder(&mut self, c: ObjId) {
        self.exec_end_climb_ladder_with(c, |vm, p, mt| vm.is_overlapping(p, mt));
    }

    /// `overlap`: AActor::IsOverlapping of the pawn and the ladder.
    pub(crate) fn exec_end_climb_ladder_with(&mut self, c: ObjId, overlap: impl FnOnce(&Vm, ObjId, ObjId) -> bool) {
        if self.latent_action(c) != AI_POLL_MOVE_TOWARD {
            return;
        }
        let (Some(p), Some(mt)) = (self.native_object(c, crate::NativeField::Pawn), self.po(c, "MoveTarget")) else { return };
        if !self.is_a(mt, "Ladder") {
            return;
        }
        if overlap(self, p, mt) {
            let nav = Some(mt).filter(|&m| self.is_a(m, "NavigationPoint"));
            self.set_prop(p, "Anchor", Value::Object(nav));
        }
        self.set_latent_action(c, 0);
    }

    // ---- APawn ----

    /// APawn::setMoveTimer: MoveTimer = 2 × MoveTime / speed + 1 (0.5 without
    /// speed).
    pub(crate) fn pawn_set_move_timer(&mut self, p: ObjId, move_time: f32) {
        let Some(c) = self.native_object(p, crate::NativeField::Controller) else { return };
        let speed = match self.propr(p, "Physics").as_byte() {
            PHYS_WALKING | PHYS_FALLING | PHYS_SPIDER => self.pf(p, "GroundSpeed"),
            PHYS_FLYING => self.pf(p, "AirSpeed"),
            PHYS_SWIMMING => self.pf(p, "WaterSpeed"),
            _ => 200.0,
        };
        let max = speed * self.pf(p, "DesiredSpeed");
        if max == 0.0 {
            self.set_f(c, "MoveTimer", 0.5);
        } else {
            self.set_f(c, "MoveTimer", (move_time + move_time) / max + 1.0);
        }
    }

    /// APawn::calcMoveFlags: R_WALK, R_FLY, R_SWIM, R_JUMP, R_DOOR, R_SPECIAL,
    /// R_LADDER, R_FORCED and R_PLAYERONLY depending on the abilities of the pawn and its controller.
    pub(crate) fn calc_move_flags(&self, p: ObjId) -> i32 {
        let c = self.native_object(p, crate::NativeField::Controller);
        let cb = |name: &str| c.is_some_and(|c| self.pb(c, name)) as i32;
        let pb = |name: &str| self.pb(p, name) as i32;
        pb("bCanWalk")
            + pb("bCanFly") * 2
            + pb("bCanSwim") * 4
            + pb("bCanJump") * 8
            + cb("bCanOpenDoors") * 16
            + cb("bCanDoSpecial") * 32
            + pb("bCanClimbLadders") * 64
            + 256
            + cb("bIsPlayer") * 512
    }

    /// APawn::ValidAnchor: the Anchor exists, is not blocked and the pawn is on it.
    pub(crate) fn valid_anchor(&mut self, p: ObjId) -> bool {
        let Some(anchor) = self.po(p, "Anchor") else { return false };
        if self.pb(anchor, "bBlocked") {
            return false;
        }
        let d = sub(self.native_prop(anchor, crate::NativeField::Location).as_vector(), self.native_prop(p, crate::NativeField::Location).as_vector());
        self.reached_destination(p, d, Some(anchor))
    }

    /// FStateFrame::Describe: GetFullName of the node of the object's state
    /// ("State Package.Class.State"), the class if it is in no state.
    pub(crate) fn state_frame_describe(&self, o: ObjId) -> String {
        let node = self.objects.get(o).and_then(|x| x.as_ref()).map(|obj| {
            let c = obj.state_class.unwrap_or(obj.class);
            let package = &self.packages[self.classes[c].package].name;
            match &obj.state {
                Some(state) => format!("State {package}.{}.{state}", self.classes[c].name),
                None => format!("Class {package}.{}", self.classes[obj.class].name),
            }
        });
        Self::state_frame_describe_node(node.as_deref())
    }

    /// Body of FStateFrame::Describe: null node → "None",
    /// otherwise Node->GetFullName(0) as is.
    pub(crate) fn state_frame_describe_node(node_full_name: Option<&str>) -> String {
        node_full_name.map_or_else(|| "None".to_string(), str::to_string)
    }

    /// Warning of the path natives: GLog->Logf(NAME_DevPath, "Warning: … by %s in %s")
    /// with the controller's name and FStateFrame::Describe.
    pub(crate) fn dev_path_warning(&mut self, what: &str, c: ObjId) {
        let line = format!("DevPath: Warning: {what} by {} in {}", self.obj_label(c), self.state_frame_describe(c));
        self.log.push(line);
    }

    /// Default value (class default object) of a float property.
    pub(crate) fn default_float(&self, a: ObjId, name: &str) -> f32 {
        self.objects[a].as_ref().map_or(0.0, |o| self.pf(self.classes[o.class].cdo, name))
    }

    /// APawn::ReachedDestination: is the pawn at goal `dir` (goal − position)?
    /// Horizontal (radii) and vertical (heights) margins depending on the nature of the goal, plus
    /// GReachedDestinationXYMargin / HeightMargin; tolerance on slopes and steps.
    pub(crate) fn reached_destination(&mut self, p: ObjId, dir: V, goal: Option<ObjId>) -> bool {
        let default_height = self.default_float(p, "CollisionHeight");
        let height = self.native_prop(p, crate::NativeField::CollisionHeight).as_float();
        let radius_self = self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
        let mut down = height;
        let mut radius = radius_self;
        let mut up = (default_height + default_height) - height;
        if let Some(g) = goal {
            let goal_height = self.native_prop(g, crate::NativeField::CollisionHeight).as_float();
            let goal_radius = self.native_prop(g, crate::NativeField::CollisionRadius).as_float();
            if self.is_a(g, "NavigationPoint") {
                if self.propr(p, "Physics").as_byte() == PHYS_LADDER && self.is_a(g, "Ladder") {
                    let n = safe_normal(dir);
                    let climb = self.po(p, "OnLadder").map_or([0.0; 3], |l| self.pv(l, "ClimbDir"));
                    let d = n[1] * climb[1] + n[2] * climb[2] + n[0] * climb[0];
                    if d.abs() < 0.1 {
                        return true;
                    }
                    up *= 0.5;
                    down *= 0.5;
                }
                let t = ((goal_height + 2.0) - height) + 35.0;
                if up < t {
                    up = t;
                }
                let t = ((height + 2.0) + 35.0) - goal_height;
                if down < t {
                    down = t;
                }
            } else if self.is_a(g, "Pawn") {
                if up < goal_height {
                    up = goal_height;
                }
                if down < goal_height {
                    down = goal_height;
                }
                radius = goal_radius + radius_self;
            } else {
                up += goal_height;
                down += goal_height;
                if self.pb(g, "bBlockActors") {
                    radius = goal_radius + radius_self;
                }
            }
        }
        let radius = self.reach_xy_margin + radius;
        let up = self.reach_height_margin + up;
        let down = self.reach_height_margin + down;
        let zdiff = dir[2];
        if radius * radius < dir[1] * dir[1] + dir[0] * dir[0] {
            return false;
        }
        let threshold = if zdiff > 0.0 { up } else { down };
        if zdiff.abs() <= threshold {
            return true;
        }
        if zdiff.abs() > threshold + threshold {
            return false;
        }
        // On a slope or at the edge of a step: the floor under the pawn decides.
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(p), [loc[0], loc[1], loc[2] - 6.0], loc, TRACE_WORLD, [radius_self, radius_self, height]);
        // Read without testing the contact, as in the original. On a miss, SingleLineCheck
        // leaves there the Normal of its local variable (the previous call at the same stack depth,
        // most often zero): we keep zero, that of the FCheckResult built here.
        let nz = hit.normal[2];
        if (nz as f64) >= 0.95 || nz < 0.7 {
            return false;
        }
        let slope = ((1.0 / (nz * nz)) - 1.0).sqrt();
        if zdiff < 0.0 && radius_self * slope + default_height > -zdiff {
            return true;
        }
        let goal_radius = goal.map_or(80.0, |g| self.native_prop(g, crate::NativeField::CollisionRadius).as_float());
        if radius_self >= goal_radius {
            return false;
        }
        ((goal_radius + 15.0) - radius_self) * slope + default_height > zdiff
    }

    /// APawn::moveToward: Acceleration towards `dest` (true once arrived, or at the
    /// end of the MoveTimer). With NextTarget less than 250 units away, the pawn cuts towards the next
    /// point and looks at it (specific to XIII).
    pub(crate) fn move_toward(&mut self, p: ObjId, dest: V, goal: Option<ObjId>, next: Option<ObjId>) -> bool {
        let Some(c) = self.native_object(p, crate::NativeField::Controller) else { return false };
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut direction = sub(dest, loc);
        let physics = self.propr(p, "Physics").as_byte();
        if physics == PHYS_FALLING {
            // Air control only in reduced gravity.
            let volume = self.native_object(p, crate::NativeField::PhysicsVolume).or(self.default_volume);
            let gz = volume.map_or(-950.0, |v| self.pv(v, "Gravity")[2]);
            let default_gz = volume.and_then(|v| self.objects[v].as_ref()).map_or(-950.0, |o| self.pv(self.classes[o.class].cdo, "Gravity")[2]);
            if fmul_d(default_gz, 0.9) >= gz {
                return false;
            }
            direction[2] = 0.0;
            let accel = mul(safe_normal(direction), self.pf(p, "AccelRate"));
            self.set_v(p, "Acceleration", accel);
            if self.native_prop(p, crate::NativeField::Velocity).as_vector()[2] >= 0.0 {
                return false;
            }
            return loc[2] + 100.0 < dest[2];
        }
        if physics == PHYS_LADDER && self.po(p, "OnLadder").is_some() {
            if self.reached_destination(p, sub(dest, loc), goal) {
                self.set_v(p, "Acceleration", [0.0; 3]);
                if let Some(nav) = goal.filter(|&g| self.is_a(g, "NavigationPoint")) {
                    self.set_prop(p, "Anchor", Value::Object(Some(nav)));
                }
                return true;
            }
            let along = match self.current_path_spec(c) {
                Some(spec) => sub(self.native_prop(spec.end, crate::NativeField::Location).as_vector(), self.native_prop(spec.start, crate::NativeField::Location).as_vector()),
                None => direction,
            };
            let accel = mul(safe_normal(along), self.pf(p, "LadderSpeed"));
            self.set_v(p, "Acceleration", accel);
            return false;
        }
        if physics == PHYS_WALKING {
            direction[2] = 0.0;
        }
        // A pickup touched on the way (the engine does not wait for the contact).
        if let Some(mt) = self.po(c, "MoveTarget").filter(|&m| self.is_a(m, "Pickup")) {
            let ml = self.native_prop(mt, crate::NativeField::Location).as_vector();
            if (loc[2] - ml[2]).abs() < self.native_prop(p, crate::NativeField::CollisionHeight).as_float() {
                let (dx, dy) = (loc[0] - ml[0], loc[1] - ml[1]);
                let r = self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
                if dx * dx + dy * dy < r * r {
                    self.event_pub(mt, "Touch", vec![Value::Object(Some(p))]);
                }
            }
        }
        let distance = ((direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]) as f64).sqrt() as f32;
        let glider = !self.pb(p, "bCanStrafe") && physics == PHYS_FLYING;
        if self.reached_destination(p, sub(dest, loc), goal) {
            if !glider {
                self.set_v(p, "Acceleration", [0.0; 3]);
            }
            if let Some(nav) = goal.filter(|&g| self.is_a(g, "NavigationPoint")) {
                self.set_prop(p, "Anchor", Value::Object(Some(nav)));
            }
            return true;
        }
        if glider {
            direction = rotator_vector(self.propr(p, "Rotation").as_rotator());
        } else if distance > 0.0 {
            direction = div(direction, distance);
        }
        let accel_rate = self.pf(p, "AccelRate");
        let along = |d: V| [accel_rate * d[0], d[1] * accel_rate, accel_rate * d[2]];
        if let Some(nt) = next.filter(|_| distance < 250.0) {
            // Towards the point 250 units after the destination, in the direction of the next one.
            let mut ahead = sub(self.native_prop(nt, crate::NativeField::Location).as_vector(), dest);
            ahead[2] = 0.0;
            let ahead = normal_move_toward(ahead);
            let point = [ahead[0] * 250.0 + dest[0], ahead[1] * 250.0 + dest[1], 250.0 * ahead[2] + dest[2]];
            let mut to_point = sub(point, loc);
            to_point[2] = 0.0;
            let mut dist2 = vsize(to_point);
            let to_point = normal_move_toward(to_point);
            let span = 250.0f32 - 80.0;
            if dist2 > span {
                dist2 = span;
            }
            let f = 1.0 - dist2 / span;
            let a = [f * direction[0], direction[1] * f, direction[2] * f];
            let b = div([to_point[0] * dist2, to_point[1] * dist2, dist2 * to_point[2]], span);
            let blend = normal_move_toward([a[0] + b[0], a[1] + b[1], a[2] + b[2]]);
            self.set_v(p, "Acceleration", along(blend));
            let l = self.native_prop(p, crate::NativeField::Location).as_vector();
            self.set_v(c, "FocalPoint", [blend[0] * 50.0 + l[0], blend[1] * 50.0 + l[1], blend[2] * 50.0 + l[2]]);
            self.set_prop(c, "Focus", Value::Object(Some(nt)));
            if dist2 < 80.0 {
                return true;
            }
        }
        self.set_v(p, "Acceleration", along(direction));
        if self.pf(c, "MoveTimer") < 0.0 {
            // Delay elapsed: the movement stops as if it had succeeded.
            return true;
        }
        if let Some(mt) = self.po(c, "MoveTarget").filter(|&m| self.is_a(m, "Pawn")) {
            // fld + fadd stay at extended precision until the fcomp: the sum is not rounded to f32.
            return f64::from(distance) < f64::from(self.native_prop(mt, crate::NativeField::CollisionRadius).as_float()) + f64::from(self.native_prop(p, crate::NativeField::CollisionRadius).as_float());
        }
        let velocity = self.native_prop(p, crate::NativeField::Velocity).as_vector();
        let speed = vsize(velocity);
        if !glider && speed > 100.0 {
            // Braking of the velocity component that does not go towards the goal.
            let vel_dir = div(velocity, speed);
            let diff = sub(vel_dir, direction);
            let k = (1.0 - (vel_dir[0] * direction[0] + vel_dir[2] * direction[2] + vel_dir[1] * direction[1])) * speed * 0.2;
            let accel = sub(self.native_prop(p, crate::NativeField::Acceleration).as_vector(), mul(diff, k));
            self.set_v(p, "Acceleration", accel);
        }
        if distance < fmul_d(self.pf(p, "AvgPhysicsTime") * speed, 1.4) {
            // Slowing down when approaching.
            if !self.pb(p, "bReducedSpeed") {
                self.set_b(p, "bReducedSpeed", true);
                let d = fmul_d(self.pf(p, "DesiredSpeed"), 0.5);
                self.set_f(p, "DesiredSpeed", d);
            }
            if speed > 0.0 {
                let (a, b) = (self.pf(p, "DesiredSpeed"), 200.0 / speed);
                self.set_f(p, "DesiredSpeed", if a <= b { a } else { b });
            }
            if glider {
                return true;
            }
        }
        false
    }

    /// AActor::AssociatedLevelGeometry (vt 0x128): the actor if it is part of the geometry,
    /// ADoor's door.
    pub(crate) fn associated_level_geometry(&self, a: ObjId) -> Option<ObjId> {
        if self.is_a(a, "Door") {
            return self.po(a, "MyDoor");
        }
        self.pb(a, "bWorldGeometry").then_some(a)
    }

    fn set_adjust_location(&mut self, c: ObjId, loc: V) {
        if self.is_a(c, "AIController") {
            self.set_b(c, "bAdjusting", true);
            self.set_v(c, "AdjustLoc", loc);
        }
    }

    fn trace_clear(&self, p: ObjId, end: V, start: V, flags: u32, extent: V) -> CheckResult {
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(p), end, start, flags, extent);
        hit
    }

    /// APawn::PickWallAdjust: detour around a wall during a movement (to the
    /// left, then to the right, or by a jump); true if a detour position is
    /// chosen (AdjustLoc) or a jump launched.
    pub(crate) fn pick_wall_adjust(&mut self, p: ObjId, wall_normal: V) -> bool {
        let physics = self.propr(p, "Physics").as_byte();
        let Some(c) = self.native_object(p, crate::NativeField::Controller).filter(|_| physics != PHYS_FALLING) else { return false };
        if physics == PHYS_FLYING || physics == PHYS_SWIMMING {
            return self.pick_3d_wall_adjust(p, wall_normal);
        }
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let view = [loc[0], loc[1], self.pf(p, "BaseEyeHeight") + loc[2]];
        let destination = self.pv(c, "Destination");
        let zdiff = destination[2] - loc[2];
        let mut dir = [destination[0] - loc[0], destination[1] - loc[1], 0.0];
        let radius = self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
        let height = self.native_prop(p, crate::NativeField::CollisionHeight).as_float();
        let mut adjust_dist = radius * 1.5 + 16.0;
        let move_target = self.po(c, "MoveTarget").and_then(|m| self.associated_level_geometry(m));
        if zdiff < height && (dir[1] * dir[1] + dir[0] * dir[0]) - radius * radius < 0.0 {
            return false;
        }
        let dist = vsize(dir);
        if dist == 0.0 {
            return false;
        }
        dir = div(dir, dist);
        let hit = self.trace_clear(p, destination, view, 0x286, [0.0; 3]);
        if hit.actor != move_target {
            adjust_dist += radius;
        }
        // To the left, then to the right if the view is blocked.
        let mut left = [dir[1], -dir[0], 0.0];
        let mut check = [(dir[1] * 1.4) * radius, (-dir[0] * 1.4) * radius, 0.0 * radius];
        let start = [check[0] + view[0], view[1] + check[1], view[2] + check[2]];
        let mut check_right = false;
        if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).actor != move_target {
            left = [-dir[1], dir[0], 0.0];
            check = [-check[0], -check[1], -check[2]];
            check_right = true;
            let start = [check[0] + view[0], check[1] + view[1], check[2] + view[2]];
            if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).actor != move_target {
                return false;
            }
        }
        let out = [dir[0] * 14.0, dir[1] * 14.0, dir[2] * 14.0];
        let extent = self.cyl_extent(p);
        if physics == PHYS_WALKING && self.pb(p, "bCanJump") {
            // Jump over the obstacle if there is room above.
            let up = self.trace_clear(p, [loc[0], loc[1], loc[2] + 80.0], loc, TRACE_WORLD, extent);
            if (up.time as f64) > 0.5 {
                let t = up.time;
                let lift = [t * 0.0 + loc[0], t * 0.0 + loc[1], t * 80.0 + loc[2]];
                let end = [lift[0] + out[0], lift[1] + out[1], lift[2] + out[2]];
                if self.trace_clear(p, end, lift, 0x286, extent).time == 1.0 {
                    let jump_dir = [-wall_normal[0], -wall_normal[1], 0.0];
                    let ground = self.pf(p, "GroundSpeed");
                    self.set_v(p, "Velocity", mul(jump_dir, ground));
                    let accel = self.pf(p, "AccelRate");
                    self.set_v(p, "Acceleration", mul(jump_dir, accel));
                    let mut v = self.native_prop(p, crate::NativeField::Velocity).as_vector();
                    v[2] = self.pf(p, "JumpZ");
                    self.set_v(p, "Velocity", v);
                    self.set_physics(p, PHYS_FALLING, None, [0.0, 0.0, 1.0]);
                    return true;
                }
            }
        }
        let mut side = [left[0] * adjust_dist, left[1] * adjust_dist, adjust_dist * 0.0];
        for attempt in 0..2 {
            if attempt == 1 {
                // The other side, if it has not already been chosen.
                if check_right {
                    return false;
                }
                check = [-check[0], -check[1], -check[2]];
                let start = [check[0] + view[0], check[1] + view[1], check[2] + view[2]];
                if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).time < 1.0 {
                    return false;
                }
                side = [-side[0], -side[1], -side[2]];
            }
            let adjusted = [side[0] + loc[0], side[1] + loc[1], side[2] + loc[2]];
            if self.trace_clear(p, adjusted, loc, 0x286, extent).time == 1.0 {
                let end = [adjusted[0] + out[0], adjusted[1] + out[1], adjusted[2] + out[2]];
                if self.trace_clear(p, end, adjusted, 0x286, extent).time == 1.0 {
                    self.set_adjust_location(c, adjusted);
                    return true;
                }
            }
            if attempt == 1 {
                return false;
            }
        }
        false
    }

    /// APawn::Pick3DWallAdjust: detour when flying or swimming: above or
    /// below when the destination is close, left or right, then vertically.
    fn pick_3d_wall_adjust(&mut self, p: ObjId, _wall_normal: V) -> bool {
        let Some(c) = self.native_object(p, crate::NativeField::Controller) else { return false };
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let view = [loc[0], loc[1], self.pf(p, "BaseEyeHeight") + loc[2]];
        let destination = self.pv(c, "Destination");
        let zdiff = destination[2] - loc[2];
        let mut dir = [destination[0] - loc[0], destination[1] - loc[1], 0.0];
        let radius = self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
        let height = self.native_prop(p, crate::NativeField::CollisionHeight).as_float();
        let adjust_dist = radius * 1.5 + 16.0;
        let move_target = self.po(c, "MoveTarget").and_then(|m| self.associated_level_geometry(m));
        let box_extent = [radius, radius, height];
        // Already tried: 1 above, -1 below.
        let mut vertical = 0i32;
        if zdiff < height {
            let sq = (dir[1] * dir[1] + dir[0] * dir[0]) + dir[2] * dir[2];
            if sq - radius * radius < 0.0 {
                return false;
            }
            if sq < (height * height) * 4.0 {
                let mut up = height;
                vertical = 1;
                if loc[2] < destination[2] {
                    up *= -1.0;
                    vertical = -1;
                }
                let lifted = [loc[0], loc[1], up + loc[2]];
                if self.trace_clear(p, lifted, loc, 0x286, box_extent).time == 1.0 {
                    let n = safe_normal(dir);
                    let fwd = [n[0] * radius, n[1] * radius, n[2] * radius];
                    let end = [loc[0] + fwd[0], fwd[1] + loc[1], (up + loc[2]) + fwd[2]];
                    if self.trace_clear(p, end, lifted, 0x286, box_extent).time == 1.0 {
                        self.set_adjust_location(c, lifted);
                        return true;
                    }
                }
            }
        }
        let dist = (((dir[1] * dir[1] + dir[0] * dir[0]) + dir[2] * dir[2]) as f64).sqrt() as f32;
        if dist == 0.0 {
            return false;
        }
        let inv = 1.0 / dist;
        dir = [inv * dir[0], inv * dir[1], inv * dir[2]];
        let hit = self.trace_clear(p, destination, view, 0x286, [0.0; 3]);
        if hit.actor != move_target && zdiff > 0.0 {
            let end = [loc[0], loc[1], (height + height) + loc[2]];
            if self.trace_clear(p, end, loc, 0x286, box_extent).time == 1.0 {
                self.set_adjust_location(c, [loc[0], loc[1], height + loc[2]]);
                return true;
            }
        }
        // To the left, then to the right if the view is blocked.
        let mut left = [dir[1], dir[0] * -1.0, 0.0];
        let mut check = [(dir[1] * 1.4) * radius, ((dir[0] * -1.0) * 1.4) * radius, 0.0 * radius];
        let start = [check[0] + view[0], view[1] + check[1], view[2] + check[2]];
        let mut check_right = false;
        if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).actor != move_target {
            left = [dir[1] * -1.0, (dir[0] * -1.0) * -1.0, 0.0];
            check = [check[0] * -1.0, check[1] * -1.0, check[2] * -1.0];
            check_right = true;
            let start = [check[0] + view[0], check[1] + view[1], check[2] + view[2]];
            if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).actor != move_target {
                return false;
            }
        }
        let out = [dir[0] * 14.0, dir[1] * 14.0, dir[2] * 14.0];
        let mut side = [left[0] * adjust_dist, left[1] * adjust_dist, adjust_dist * 0.0];
        let cylinder = self.cyl_extent(p);
        let side_clear = |vm: &mut Vm, side: V, extent: V| -> Option<V> {
            let adjusted = [side[0] + loc[0], side[1] + loc[1], side[2] + loc[2]];
            if vm.trace_clear(p, adjusted, loc, 0x286, extent).time != 1.0 {
                return None;
            }
            let end = [adjusted[0] + out[0], adjusted[1] + out[1], adjusted[2] + out[2]];
            (vm.trace_clear(p, end, adjusted, 0x286, extent).time == 1.0).then_some(adjusted)
        };
        if let Some(adjusted) = side_clear(self, side, box_extent) {
            self.set_adjust_location(c, adjusted);
            return true;
        }
        if !check_right {
            // The other side.
            check = [check[0] * -1.0, check[1] * -1.0, check[2] * -1.0];
            let start = [check[0] + view[0], check[1] + view[1], check[2] + view[2]];
            if self.trace_clear(p, destination, start, 0x286, [0.0; 3]).time < 1.0 {
                return false;
            }
            side = [side[0] * -1.0, side[1] * -1.0, side[2] * -1.0];
            if let Some(adjusted) = side_clear(self, side, cylinder) {
                self.set_adjust_location(c, adjusted);
                return true;
            }
        }
        // Above, then below, except the height already tried.
        for (tried, z) in [(1, height), (-1, height * -1.0)] {
            if vertical == tried {
                if tried == -1 {
                    return false;
                }
                continue;
            }
            let lifted = [loc[0], loc[1], z + loc[2]];
            if self.trace_clear(p, lifted, loc, 0x286, box_extent).time == 1.0 {
                let end = [loc[0] + out[0], loc[1] + out[1], (z + loc[2]) + out[2]];
                if self.trace_clear(p, end, lifted, 0x286, cylinder).time == 1.0 {
                    self.set_adjust_location(c, lifted);
                    return true;
                }
            } else if tried == -1 {
                return false;
            }
        }
        false
    }

    /// APawn::SuggestJumpVelocity: jump velocity towards `dest` at horizontal
    /// speed `xy_speed`, starting at least at `base_z` upwards.
    pub(crate) fn suggest_jump_velocity(&self, p: ObjId, dest: V, xy_speed: f32, base_z: f32) -> V {
        crate::navigation::speed_jump(
            self.native_prop(p, crate::NativeField::Location).as_vector(),
            dest,
            xy_speed,
            base_z,
            self.pf(p, "JumpZ"),
            self.gravity_of(p)[2],
            self.pf(p, "GroundSpeed"),
        )
    }

    // ---- XIDPawn.dll natives ----

    /// AIAController::execNearWall (XIDPawn.dll): is there a wall at `wall_dist` in front of the
    /// soldier? If so, FocalPoint towards a clear side (at random), or far behind them.
    pub(crate) fn exec_near_wall(&mut self, c: ObjId, wall_dist: f32) -> bool {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return false };
        let x = rotator_vector(self.propr(c, "Rotation").as_rotator());
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let view = [loc[0], loc[1], loc[2] + self.pf(p, "BaseEyeHeight")];
        let ahead = [x[0] * wall_dist, x[1] * wall_dist, x[2] * wall_dist];
        let hit = self.trace_clear(c, [view[0] + ahead[0], view[1] + ahead[1], view[2] + ahead[2]], view, TRACE_WORLD, [0.0; 3]);
        if hit.actor.is_none() {
            return false;
        }
        let (mut sx, mut sy) = if (self.rand() as f64) < 0.5 { (hit.normal[1], -hit.normal[0]) } else { (-hit.normal[1], hit.normal[0]) };
        let sq = sx * sx + sy * sy;
        if (sq as f64) < 1e-8 {
            return false;
        }
        let s = 1.0 / sq.sqrt();
        self.set_prop(c, "Focus", Value::Object(None));
        sx = sx * s * wall_dist;
        sy = s * sy * wall_dist;
        let sz = wall_dist * 0.0;
        let hit = self.trace_clear(c, [view[0] + sx, view[1] + sy, view[2] + sz], view, 0x286, [0.0; 3]);
        let l = self.native_prop(p, crate::NativeField::Location).as_vector();
        if hit.actor.is_some() {
            self.set_v(c, "FocalPoint", [sx + l[0], sy + l[1], l[2]]);
            return true;
        }
        let hit = self.trace_clear(c, [view[0] - sx, view[1] - sy, view[2] - sz], view, 0x286, [0.0; 3]);
        if hit.actor.is_some() {
            self.set_v(c, "FocalPoint", [l[0] - sx, l[1] - sy, l[2]]);
            return true;
        }
        self.set_v(c, "FocalPoint", [l[0] - x[0] * 300.0, l[1] - x[1] * 300.0, l[2] - x[2] * 300.0]);
        true
    }

    /// AIAController::execTestDirection (XIDPawn.dll): point at `dist` in the
    /// direction `dir` (moved away from a wall if there is one); true if it is further than `min_dist`.
    pub(crate) fn exec_test_direction(&mut self, c: ObjId, min_dist: f32, dist: f32, dir: V) -> (bool, V) {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return (false, [0.0; 3]) };
        let mut pick = [dir[0] * dist, dir[1] * dist, dir[2] * dist];
        let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
        let r2 = self.native_prop(p, crate::NativeField::CollisionRadius).as_float() + self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
        let end = [dir[0] * r2 + (loc[0] + pick[0]), (loc[1] + pick[1]) + dir[1] * r2, dir[2] * r2 + (loc[2] + pick[2])];
        let hit = self.trace_clear(c, end, loc, TRACE_WORLD, [0.0; 3]);
        if hit.actor.is_none() {
            let l = self.native_prop(p, crate::NativeField::Location).as_vector();
            pick = [l[0] + pick[0], l[1] + pick[1], l[2] + pick[2]];
        } else {
            let r = self.native_prop(p, crate::NativeField::CollisionRadius).as_float();
            let bz = ((hit.normal[2] - dir[2]) + (hit.normal[2] - dir[2])) * r;
            let by = ((hit.normal[1] - dir[1]) + (hit.normal[1] - dir[1])) * r;
            pick = [hit.location[0] + ((hit.normal[0] - dir[0]) + (hit.normal[0] - dir[0])) * r, hit.location[1] + by, hit.location[2] + bz];
            let l = self.native_prop(p, crate::NativeField::Location).as_vector();
            if self.trace_clear(c, pick, l, 0x286, [0.0; 3]).actor.is_some() {
                return (false, pick);
            }
        }
        let l = self.native_prop(p, crate::NativeField::Location).as_vector();
        let (dx, dy) = (pick[0] - l[0], pick[1] - l[1]);
        (min_dist * min_dist < dx * dx + dy * dy, pick)
    }

    /// AIAController::execPseudoSteering (XIDPawn.dll): repulsion from the other soldiers
    /// in combat (GenAlerte.SoldierInFightList) as 1/distance², with a detour if a wall is
    /// on the path; zero vector when the repulsion is weak.
    pub(crate) fn exec_pseudo_steering(&mut self, c: ObjId) -> V {
        let Some(gen) = self.po(c, "GenAlerte") else { return [0.0; 3] };
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return [0.0; 3] };
        let list = match self.prop(gen, "SoldierInFightList") {
            Value::Array(v) => v.iter().filter_map(|x| x.as_object()).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut last: Option<ObjId> = None;
        let mut sum = [0.0f32; 3];
        for &other in &list {
            last = Some(other);
            if other == p || self.objects.get(other).is_none_or(|o| o.is_none()) {
                continue;
            }
            let pl = self.native_prop(p, crate::NativeField::Location).as_vector();
            let ol = self.native_prop(other, crate::NativeField::Location).as_vector();
            let dz = pl[2] - ol[2];
            let (dx, dy) = (pl[0] - ol[0], pl[1] - ol[1]);
            let k = 1.0 / ((dz * dz + dy * dy) + dx * dx);
            sum[0] += k * (pl[0] - ol[0]);
            sum[1] += (pl[1] - ol[1]) * k;
            sum[2] += dz * k;
        }
        // The machine code only computes the detour for a zero repulsion.
        if sum != [0.0; 3] {
            return [0.0; 3];
        }
        let Some(other) = last else { return [0.0; 3] };
        // (UnsafeNormal): the repulsion is zero here, so the direction is NaN
        // (0 × appInvSqrt(0)), as in the game.
        let unsafe_normal = |v: V| {
            let r = 1.0 / ((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]).sqrt();
            [v[0] * r, v[1] * r, v[2] * r]
        };
        let mut dir = unsafe_normal(sum);
        let pl = self.native_prop(p, crate::NativeField::Location).as_vector();
        let ol = self.native_prop(other, crate::NativeField::Location).as_vector();
        let start = [pl[0], pl[1], pl[2] - 30.0];
        let end = [dir[0] * 4000.0 + ol[0], dir[1] * 4000.0 + ol[1], (dir[2] * 4000.0 + ol[2]) - 30.0];
        let hit = self.trace_clear(c, end, start, TRACE_WORLD, [0.0; 3]);
        if hit.actor.is_some() {
            let pl = self.native_prop(p, crate::NativeField::Location).as_vector();
            let dz = pl[2] - ol[2];
            let (dy, dx) = (pl[1] - ol[1], pl[0] - ol[0]);
            // (X² + Y²) + Z², unlike the loop.
            let k = 1.0 / ((dx * dx + dy * dy) + dz * dz);
            sum[0] += k * (pl[0] - hit.location[0]);
            sum[1] += (pl[1] - hit.location[1]) * k;
            sum[2] += k * (pl[2] - hit.location[2]);
            dir = unsafe_normal(sum);
        }
        let scaled = [sum[0] * 50000.0, sum[1] * 50000.0, sum[2] * 50000.0];
        let sq = (scaled[2] * scaled[2] + scaled[1] * scaled[1]) + scaled[0] * scaled[0];
        if sq > 1.6e7 {
            return [dir[0] * 4000.0, dir[1] * 4000.0, dir[2] * 4000.0];
        }
        if sq > 2500.0 {
            return scaled;
        }
        [0.0; 3]
    }
}

impl Vm {
    /// Movement, navigation and perception natives of controllers (Engine.dll and
    /// XIDPawn.dll); optional arguments missing: the natives' default values.
    pub(crate) fn ai_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        let given = |i: usize| supplied.get(i) == Some(&true);
        let obj = |args: &[Value], i: usize| args.get(i).and_then(|v| v.as_object()).filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()));
        let flag = |args: &[Value], i: usize| if given(i) { args[i].as_bool() } else { true };
        let vv = |v: V| Value::vector(v[0], v[1], v[2]);
        Some(match name {
            "Sleep" => {
                let s = args.first()?.as_float();
                if self.is_a(this, "GUIPage") {
                    // UGUIPage::execSleep (GUI): LatentAction = 431 (its own native).
                    self.set_latent_action(this, GUI_POLL_SLEEP);
                    self.set_f(this, "LatentFloat", s);
                } else {
                    self.exec_sleep(this, s);
                }
                Value::Void
            }
            "MoveTo" => {
                let dest = args.first()?.as_vector();
                let focus = obj(args, 1);
                let speed = if given(2) { args[2].as_float() } else { 1.0 };
                self.exec_move_to(this, dest, focus, speed);
                Value::Void
            }
            "MoveToward" => {
                let target = obj(args, 0);
                let focus = if given(1) { obj(args, 1) } else { target };
                let speed = if given(2) { args[2].as_float() } else { 1.0 };
                let next = obj(args, 3);
                self.exec_move_toward(this, target, focus, speed, next);
                Value::Void
            }
            "FinishRotation" => {
                self.exec_finish_rotation(this);
                Value::Void
            }
            "WaitForLanding" => {
                self.exec_wait_for_landing(this);
                Value::Void
            }
            "FinishInterpolation" => {
                self.exec_finish_interpolation(this);
                Value::Void
            }
            "StopWaiting" => {
                self.exec_stop_waiting(this);
                Value::Void
            }
            "EndClimbLadder" => {
                self.exec_end_climb_ladder(this);
                Value::Void
            }
            // AController::execPickWallAdjust: without a pawn, false.
            "PickWallAdjust" => {
                let n = args.first()?.as_vector();
                Value::Bool(self.native_object(this, crate::NativeField::Pawn).is_some_and(|p| self.pick_wall_adjust(p, n)))
            }
            // AController::execEAdjustJump: BaseZ then XYSpeed, towards Destination.
            "EAdjustJump" => {
                let (base_z, xy) = (args.first()?.as_float(), args.get(1)?.as_float());
                let d = self.pv(this, "Destination");
                vv(self.native_object(this, crate::NativeField::Pawn).map_or([0.0; 3], |p| self.suggest_jump_velocity(p, d, xy, base_z)))
            }
            // AController::execFindPathTo: bClearPaths is 1 if omitted.
            "FindPathTo" => {
                let point = args.first()?.as_vector();
                Value::Object(self.find_path(this, point, None, flag(args, 1)))
            }
            "FindPathToward" => match obj(args, 0) {
                Some(goal) => Value::Object(self.find_path(this, [0.0; 3], Some(goal), flag(args, 1))),
                // AController::execFindPathToward.
                None => {
                    self.dev_path_warning("No goal for FindPathToward", this);
                    Value::Object(None)
                }
            },
            "FindPathTowardNearest" => {
                let class = args.first().and_then(|v| v.as_class());
                Value::Object(self.find_path_toward_nearest(this, class))
            }
            "FindRandomDest" => Value::Object(self.find_random_dest(this, flag(args, 0))),
            // AController::execClearPaths.
            "ClearPaths" => {
                if let Some(p) = self.native_object(this, crate::NativeField::Pawn) {
                    self.clear_paths(p);
                }
                Value::Void
            }
            "FindBestInventoryPath" => {
                let mut w = args.first()?.as_float();
                let r = self.find_best_inventory_path(this, &mut w);
                args[0] = Value::Float(w);
                Value::Object(r)
            }
            // AController::execactorReachable.
            "actorReachable" => {
                let target = obj(args, 0);
                match (target, self.native_object(this, crate::NativeField::Pawn)) {
                    (Some(t), Some(p)) => Value::Bool(self.actor_reachable(p, Some(t), false, false)),
                    _ => {
                        self.dev_path_warning("No pawn or goal for ActorReachable", this);
                        Value::Bool(false)
                    }
                }
            }
            // AController::execpointReachable.
            "pointReachable" => {
                let point = args.first()?.as_vector();
                match self.native_object(this, crate::NativeField::Pawn) {
                    Some(p) => Value::Bool(self.point_reachable(p, point, false)),
                    None => {
                        self.dev_path_warning("No pawn for pointReachable", this);
                        Value::Bool(false)
                    }
                }
            }
            "FindBestPathToward" => {
                let d = obj(args, 0);
                let xy = if given(1) { args[1].as_float() } else { 70.0 };
                let h = if given(2) { args[2].as_float() } else { 160.0 };
                Value::Bool(self.find_best_path_toward(this, d, xy, h))
            }
            "FindBestPathTo" => {
                let d = args.first()?.as_vector();
                Value::Bool(self.find_best_path_to(this, d))
            }
            // AController::execLineOfSightTo: LineOfSightTo(Other, bUseLOSFlag = 0).
            "LineOfSightTo" => {
                let o = obj(args, 0);
                Value::Bool(self.line_of_sight_to(this, o, false))
            }
            // AController::execCanSee: SeePawn(Other, bMaySkipChecks = 0).
            "CanSee" => {
                let o = obj(args, 0);
                Value::Bool(self.see_pawn(this, o, false))
            }
            "PlayerCanSeeMe" => Value::Bool(self.player_can_see_me(this)),
            "NearWall" => {
                let d = args.first()?.as_float();
                Value::Bool(self.exec_near_wall(this, d))
            }
            "TestDirection" => {
                let (min, dist, dir) = (args.first()?.as_float(), args.get(1)?.as_float(), args.get(2)?.as_vector());
                let (ok, pick) = self.exec_test_direction(this, min, dist, dir);
                if args.len() > 3 {
                    args[3] = vv(pick);
                }
                Value::Bool(ok)
            }
            "PseudoSteering" => vv(self.exec_pseudo_steering(this)),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::fmul_d;

    /// Exact product rounded once: 1.5414125 × 1.4 is 2.1579773 on the FPU, whereas the
    /// double product rounded to single gives 2.1579776.
    #[test]
    fn fmul_rounded_unique() {
        assert_eq!(fmul_d(1.541_412_5, 1.4), 2.157_977_3);
        assert_ne!((1.541_412_5f32 as f64 * 1.4) as f32, 2.157_977_3);
        assert_eq!(fmul_d(3.0, 0.5), 1.5);
    }
}

