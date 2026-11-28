//! Improved build, co-op campaign (whatever the Graphics setting): how the full body of a
//! player, the one the other players see, follows where that player looks.
//!
//! The engine (APawn::UpdateSpineControl, anim.rs `update_bone_control`) only turns a player's
//! torso ("X Spine") straight to the aim, and the whole body turns with the player's yaw: the
//! head stays still on the torso (players have no head control, bHeadControl), looking down
//! bends the whole torso, looking aside turns the whole body at once. Here, as in split-screen
//! shooters, the head ("X Head") follows the look at once, up, down and aside (a nod or a shake
//! of the head reads at a glance); the torso only takes over for the last part of a look very
//! high or very low, so that the neck never breaks; the body itself (hips and legs) stays where
//! it stands until the look goes far aside, then turns to catch up. Only the body others see
//! changes: the player's view, aim, collision and the game's state are untouched. The solo game
//! and the multiplayer games keep the engine's own pose.
//!
//! A player who fires: the torso, and with it the arms and the weapon they hold, turns straight
//! to the aim, up, down and aside, so that the weapon the others see points where the shot goes;
//! it stays there while the player keeps firing and a moment after, then the free look comes
//! back. The hips and legs keep their own rule (they would slide on the spot at every shot).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::ObjId;
use crate::Vm;
use crate::NativeField as Field;
use std::collections::HashMap;

const DEG: f32 = std::f32::consts::PI / 180.0;
/// Unreal angle units per degree.
const UNITS: f32 = 65536.0 / 360.0;

/// Up or down, the torso starts to lean with the look past this angle, and has reached its
/// limit at `SPINE_FULL` (smoothly in between); the head does the rest.
const SPINE_START: f32 = 45.0 * DEG;
const SPINE_FULL: f32 = 85.0 * DEG;
/// Most the torso leans ahead (looking down) or back (looking up).
const SPINE_DOWN: f32 = 35.0 * DEG;
const SPINE_UP: f32 = 25.0 * DEG;
/// Aside, the share of the angle between the look and the body that the torso takes (the head
/// the rest).
const SPINE_YAW_SHARE: f32 = 0.25;
/// Look this far aside from the body and the body turns to face it again.
const BODY_TRIGGER: f32 = 60.0 * DEG;
/// The body stops turning when it is this close to the look.
const BODY_SETTLED: f32 = 3.0 * DEG;
/// The body never lags further behind the look.
const BODY_MAX: f32 = 85.0 * DEG;
/// How fast the body catches up (fraction of the gap per second), standing or moving.
const BODY_RATE: f32 = 5.0;
const BODY_RATE_MOVING: f32 = 14.0;
/// Faster than this (units per second, along the ground), the player is moving: their legs
/// already run where they look, the body follows at once.
const MOVING_SPEED: f32 = 20.0;
/// The head bone (the torso is the engine's own "X Spine").
const HEAD: &str = "X Head";
/// A shot: the torso is straight on the aim this long after it (seconds)...
const AIM_IN: f32 = 0.08;
/// ...stays there this long after the last shot...
const AIM_HOLD: f32 = 1.0;
/// ...then goes back to the free look in this long.
const AIM_OUT: f32 = 0.35;
/// Most the torso leans with the aim, up or down: the engine's own limit (the 9000 of check_os,
/// 40.6°).
const AIM_LEAN_MAX: f32 = (16384.0 - 9000.0) / UNITS * DEG;

#[derive(Debug, Default)]
pub(crate) struct State {
    by_pawn: HashMap<ObjId, Look>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Look {
    /// Yaw the body faces (radians).
    body: f32,
    /// The body is turning to catch up with the look.
    turning: bool,
    /// Angle from the body to the look this tick (radians).
    gap: f32,
    /// The weapon in hand (its attachment) and how many times it fired, last tick.
    shots: Option<(ObjId, u8, u8)>,
    /// Time left with the torso on the aim after the last shot (seconds).
    hold: f32,
    /// How far the torso is on the aim: 0 the free look, 1 straight on the aim.
    aim: f32,
}

impl Vm {
    /// The players' bodies follow their look in this game (see the module's comment).
    fn coop_look_on(&self, pawn: ObjId) -> bool {
        !self.improved.character_screen.on
            && self.coop.on
            && self.is_a(pawn, "XIIIPlayerPawn")
    }

    /// The direction `rotation` the engine is about to give a torso (`head` false) or a head,
    /// as this module wants it for `pawn` (anim.rs update_bone_control, which only turns the
    /// players' torsos). Each tick, the torso's call also moves the body's own yaw and gives
    /// the head its direction.
    pub(crate) fn coop_look_bone(&mut self, pawn: ObjId, head: bool, dt: f32, rotation: [i32; 3]) -> [i32; 3] {
        if !self.coop_look_on(pawn) {
            if self.improved.coop_look.by_pawn.remove(&pawn).is_some() {
                self.coop_look_turn(pawn, 0);
                self.set_bone_direction(pawn, HEAD, [0; 3], [0.0; 3], 0.0);
            }
            return rotation;
        }
        if head {
            return rotation;
        }
        // Where the player looks: their controller's rotation, else the pitch replicated with
        // the player's body (ControllerPitch).
        let pc = self.native_prop(pawn, Field::Controller).as_object().filter(|&c| self.alive(c));
        let pawn_rot = self.native_prop(pawn, Field::Rotation).as_rotator();
        let view = match pc.map(|c| self.native_prop(c, Field::Rotation).as_rotator()) {
            Some(r) => r,
            None => [(self.propr(pawn, "ControllerPitch").as_byte() as i32) << 8, pawn_rot[1], 0],
        };
        let pitch = (view[0] as u16 as i16) as f32 / UNITS * DEG;
        let aim = view[1] as f32 / UNITS * DEG;
        let v = self.native_prop(pawn, Field::Velocity).as_vector();
        let moving = (v[0] * v[0] + v[1] * v[1]).sqrt() > MOVING_SPEED;
        // A player holding someone or carrying a corpse: what they hold is placed in front of
        // where they look (coop_hostage.rs, coop_carry.rs), the body faces it.
        let busy = self.coop_left_arm_busy(pawn);
        let old = self.improved.coop_look.by_pawn.get(&pawn).copied().unwrap_or(Look { body: aim, ..Look::default() });
        let look = follow(old, aim, dt, moving || busy);
        let shots = self.coop_look_shots(pawn);
        let look = aim_step(look, shots, dt);
        self.improved.coop_look.by_pawn.insert(pawn, look);
        // The whole skeleton turned by the body's lag (skel.rs, compute_pose_render); the
        // directions below are given without that turn, which then takes them where asked.
        let lag = -look.gap;
        self.coop_look_turn(pawn, (lag / DEG * UNITS).round() as i32);
        let (lean, share) = torso_on_aim(pitch, look.aim);
        let spine = bone_frame(lean, look.body + share * look.gap - lag);
        let head_dir = bone_frame(pitch, aim - lag);
        self.set_bone_direction(pawn, HEAD, head_dir, [0.0; 3], 1.0);
        spine
    }

    /// The weapon `pawn` holds (its attachment, among the actors attached to the body) and how
    /// many times it fired: WeaponAttachment FlashCount and AltFlashCount, which
    /// Weapon.IncrementFlashCount moves at every shot of every weapon, fists and thrown ones
    /// included, and the network sends to the other machines.
    fn coop_look_shots(&self, pawn: ObjId) -> Option<(ObjId, u8, u8)> {
        let crate::value::Value::Array(attached) = self.prop(pawn, "Attached") else { return None };
        let weapon = attached.iter().filter_map(crate::value::Value::as_object).find(|&a| self.alive(a) && self.is_a(a, "WeaponAttachment"))?;
        Some((weapon, self.propr(weapon, "FlashCount").as_byte(), self.propr(weapon, "AltFlashCount").as_byte()))
    }

    fn coop_look_turn(&mut self, pawn: ObjId, turn: i32) {
        if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
            if inst.pose.body_turn != turn {
                inst.pose.body_turn = turn;
                inst.pose.last_tick = None;
            }
        }
    }
}

/// The angle `a` brought within (−π, π].
fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let r = (a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI;
    if r <= -std::f32::consts::PI { r + t } else { r }
}

/// The body's yaw a tick of `dt` seconds later, the player looking at yaw `aim`.
fn follow(mut look: Look, aim: f32, dt: f32, at_once: bool) -> Look {
    let mut gap = wrap(aim - look.body);
    if at_once || gap.abs() > BODY_TRIGGER {
        look.turning = true;
    }
    if look.turning {
        let rate = if at_once { BODY_RATE_MOVING } else { BODY_RATE };
        let step = gap * (dt * rate).min(1.0);
        look.body = wrap(look.body + step);
        gap = wrap(aim - look.body);
        if gap.abs() < BODY_SETTLED && !at_once {
            look.turning = false;
        }
    }
    if gap.abs() > BODY_MAX {
        look.body = wrap(aim - BODY_MAX * gap.signum());
        gap = BODY_MAX * gap.signum();
    }
    look.gap = gap;
    look
}

/// The look a tick of `dt` seconds later, the weapon in hand having fired `shots` times: a shot
/// (the same weapon, its count moved) puts the torso on the aim, a new weapon in hand does not.
fn aim_step(mut look: Look, shots: Option<(ObjId, u8, u8)>, dt: f32) -> Look {
    let fired = matches!((look.shots, shots), (Some(a), Some(b)) if a.0 == b.0 && a != b);
    look.shots = shots;
    look.hold = if fired { AIM_HOLD } else { (look.hold - dt).max(0.0) };
    look.aim = if look.hold > 0.0 { (look.aim + dt / AIM_IN).min(1.0) } else { (look.aim - dt / AIM_OUT).max(0.0) };
    look
}

/// The torso's lean and its share of the look aside (the rest: the head), the player looking
/// `pitch` radians up and the torso `aim` of the way on the aim (`aim_step`), smoothly from the
/// free look (`spine_lean`, `SPINE_YAW_SHARE`) to straight on the aim (the whole look, up to
/// the engine's lean limit).
fn torso_on_aim(pitch: f32, aim: f32) -> (f32, f32) {
    let s = aim * aim * (3.0 - 2.0 * aim);
    let free = spine_lean(pitch);
    let lean = free + (pitch.clamp(-AIM_LEAN_MAX, AIM_LEAN_MAX) - free) * s;
    (lean, SPINE_YAW_SHARE + (1.0 - SPINE_YAW_SHARE) * s)
}

/// How much the torso leans when the player looks `pitch` radians up (positive) or down:
/// nothing up to `SPINE_START`, then smoothly up to its limit at `SPINE_FULL`.
pub(crate) fn spine_lean(pitch: f32) -> f32 {
    let t = ((pitch.abs() - SPINE_START) / (SPINE_FULL - SPINE_START)).clamp(0.0, 1.0);
    let limit = if pitch > 0.0 { SPINE_UP } else { SPINE_DOWN };
    pitch.signum() * limit * t * t * (3.0 - 2.0 * t)
}

/// The world direction SetBoneDirection takes for a torso or head looking `pitch` up and
/// facing `yaw` (radians): the frame of that look, axes permuted as APawn::UpdateSpineControl
/// does (X = Z, Y = −X, Z = −Y, anim.rs check_os) so that the bone's X axis runs up the body.
pub(crate) fn bone_frame(pitch: f32, yaw: f32) -> [i32; 3] {
    let r = [(pitch / DEG * UNITS).round() as i32, (yaw / DEG * UNITS).round() as i32, 0];
    let [x, y, z] = crate::ai_natives::axes(r);
    let neg = |v: [f32; 3]| [-v[0], -v[1], -v[2]];
    crate::world::ortho_rotation([z, neg(x), neg(y)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torso_only_for_the_last_part_of_a_look_up_or_down() {
        for d in 0..=45 {
            assert_eq!(spine_lean(d as f32 * DEG), 0.0);
            assert_eq!(spine_lean(-(d as f32) * DEG), 0.0);
        }
        let (mut up, mut down) = (0.0, 0.0);
        for d in 46..=85 {
            let u = spine_lean(d as f32 * DEG);
            let l = spine_lean(-(d as f32) * DEG);
            assert!(u > up && u <= SPINE_UP + 1e-6, "{d} {u}");
            assert!(l < down && l >= -SPINE_DOWN - 1e-6, "{d} {l}");
            (up, down) = (u, l);
        }
        assert!((spine_lean(89.0 * DEG) - SPINE_UP).abs() < 1e-6);
        // Smooth: no jump between two neighbouring looks.
        for k in 0..1800 {
            let p = (k as f32 / 10.0 - 90.0) * DEG;
            assert!((spine_lean(p + 0.1 * DEG) - spine_lean(p)).abs() < 0.2 * DEG, "{p}");
        }
    }

    #[test]
    fn bone_frame_is_the_engines_own_for_the_same_look() {
        // The engine's torso direction (check_os) for a player looking (pitch, yaw), where its
        // own limit (40.6° of lean, the 9000 of check_os) does not cut in: the same frame.
        for (pitch, yaw) in [(0, 0), (-6000, 12000), (-7000, -30000), (4000, 47000), (7000, 100), (-2000, 33000)] {
            let input = crate::anim::InputOs {
                pri: true,
                controller: true,
                controller_rotation: Some([pitch, yaw, 0]),
                local: true,
                focus: None,
                pawn_rotation: [0, yaw, 0],
                pawn_location: [0.0; 3],
                controller_pitch: 0,
                flag: false,
                max_yaw: 0,
                yaw: 0,
                speed: 0.0,
                angle: 0.0,
                dt: 0.0,
            };
            let engine = crate::anim::check_os(&input).rotation;
            let ours = bone_frame(pitch as f32 / UNITS * DEG, yaw as f32 / UNITS * DEG);
            let (a, b) = (crate::ai_natives::axes(engine), crate::ai_natives::axes(ours));
            for k in 0..3 {
                for c in 0..3 {
                    assert!((a[k][c] - b[k][c]).abs() < 2e-3, "{pitch} {yaw}: {engine:?} {ours:?}");
                }
            }
        }
    }

    #[test]
    fn head_follows_every_small_look_at_once() {
        // A nod: each degree of look moves the head's up axis by that degree, no dead zone.
        let up = |p: f32| crate::ai_natives::axes(bone_frame(p * DEG, 0.0))[0];
        for d in -60..60 {
            let (a, b) = (up(d as f32), up(d as f32 + 1.0));
            let angle = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0).dacos() / DEG;
            assert!((angle - 1.0).abs() < 0.05, "{d}: {angle}");
        }
    }

    #[test]
    fn body_waits_then_catches_up() {
        let mut look = Look::default();
        // A look aside within the trigger: the body does not move.
        for _ in 0..60 {
            look = follow(look, 45.0 * DEG, 1.0 / 60.0, false);
        }
        assert_eq!(look.body, 0.0);
        assert!((look.gap - 45.0 * DEG).abs() < 1e-5);
        // Past the trigger, it turns until it faces the look again, then rests.
        for _ in 0..120 {
            look = follow(look, 80.0 * DEG, 1.0 / 60.0, false);
        }
        assert!(look.gap.abs() < BODY_SETTLED && !look.turning, "{look:?}");
        // A flick never leaves the body further behind than the limit.
        look = follow(look, look.body + 170.0 * DEG, 1.0 / 60.0, false);
        assert!(look.gap.abs() <= BODY_MAX + 1e-4, "{look:?}");
        // Moving: the body follows at once.
        let mut m = Look::default();
        for _ in 0..30 {
            m = follow(m, 20.0 * DEG, 1.0 / 60.0, true);
        }
        assert!(m.gap.abs() < 1.0 * DEG, "{m:?}");
    }

    #[test]
    fn a_shot_puts_the_torso_on_the_aim_then_lets_it_go() {
        let gun: ObjId = 7;
        let tick = 1.0 / 60.0;
        let mut look = aim_step(Look::default(), Some((gun, 3, 0)), tick);
        // Holding a weapon is no shot.
        assert_eq!(look.aim, 0.0);
        for _ in 0..30 {
            look = aim_step(look, Some((gun, 3, 0)), tick);
        }
        assert_eq!(look.aim, 0.0);
        // A shot (or a shot of the other fire): on the aim within AIM_IN.
        for (k, n) in [(4u8, 0u8), (4, 1)] {
            look = aim_step(look, Some((gun, k, n)), tick);
            for _ in 0..(AIM_IN / tick).ceil() as usize {
                look = aim_step(look, Some((gun, k, n)), tick);
            }
            assert_eq!(look.aim, 1.0, "{look:?}");
        }
        // The count going round (255 -> 0) is a shot too.
        let mut w = aim_step(Look::default(), Some((gun, 255, 0)), tick);
        w = aim_step(w, Some((gun, 0, 0)), tick);
        assert!(w.hold > 0.0);
        // Kept while the hold lasts, then back to the free look within AIM_OUT.
        // (counted from the last shot, AIM_IN ticks ago)
        let mut t = (AIM_IN / tick).ceil() * tick;
        while look.aim == 1.0 {
            look = aim_step(look, Some((gun, 4, 1)), tick);
            t += tick;
        }
        assert!((t - AIM_HOLD).abs() < 3.0 * tick, "{t}");
        for _ in 0..(AIM_OUT / tick).ceil() as usize + 1 {
            look = aim_step(look, Some((gun, 4, 1)), tick);
        }
        assert_eq!(look.aim, 0.0);
        // Another weapon in hand, or none: no shot.
        let other: ObjId = 8;
        look = aim_step(look, Some((other, 0, 0)), tick);
        look = aim_step(look, None, tick);
        look = aim_step(look, Some((gun, 9, 9)), tick);
        assert_eq!(look.hold, 0.0);
    }

    #[test]
    fn the_torso_on_the_aim_takes_the_whole_look() {
        for d in -89..=89 {
            let pitch = d as f32 * DEG;
            // Free: the old pose, unchanged.
            assert_eq!(torso_on_aim(pitch, 0.0), (spine_lean(pitch), SPINE_YAW_SHARE));
            // On the aim: the whole look aside, and up or down within the engine's limit.
            let (lean, share) = torso_on_aim(pitch, 1.0);
            assert_eq!(share, 1.0);
            assert!((lean - pitch.clamp(-AIM_LEAN_MAX, AIM_LEAN_MAX)).abs() < 1e-6, "{d} {lean}");
            // Smooth in between.
            let mut last = torso_on_aim(pitch, 0.0);
            for k in 1..=50 {
                let now = torso_on_aim(pitch, k as f32 / 50.0);
                assert!((now.0 - last.0).abs() < 3.0 * DEG && (now.1 - last.1).abs() < 0.05, "{d} {k}");
                last = now;
            }
        }
        assert!((AIM_LEAN_MAX / DEG - 40.56).abs() < 0.01);
    }

    #[test]
    fn wrap_is_within_a_half_turn() {
        for k in -20..20 {
            let a = k as f32 * 0.7;
            let w = wrap(a);
            assert!(w > -std::f32::consts::PI - 1e-6 && w <= std::f32::consts::PI + 1e-6);
            let turns = (a - w) / std::f32::consts::TAU;
            assert!((turns - turns.round()).abs() < 1e-4);
        }
    }
}
