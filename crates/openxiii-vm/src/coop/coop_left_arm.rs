//! Improved build only: in co-op, a player whose left hand is busy, seen by the other players.
//!
//! In the game a player who holds a hostage or carries a body has one hand left
//! (XIIIPawn.bHaveOnlyOneHandFree): the first-person view shows the left arm (the left hand
//! model, XIIILeftHand) around the hostage's throat or under the body. The full body the other
//! players see knows nothing of it: it keeps the two-handed gestures of its weapon.
//!
//! Here whoever shows something held in the left hand (coop_hostage.rs, the carried body)
//! asks, once per tick, where the left hand must be (`coop_left_arm_reach`). The left arm of
//! the player's full body then leaves the body's sequences: upper arm and forearm are turned
//! to bring the hand there (two-bone solve from the shoulder, lengths kept), through the
//! engine's own bone directions (SetBoneDirection), which replace whatever the sequences give
//! these bones. The right arm keeps the weapon's gestures: only the right hand is used. A tick
//! without a request gives the arm back to the sequences.
//!
//! `coop_left_arm_grip` tells where the first-person forearm and hand are around what they
//! hold, at a given instant of the hand model's sequence: asking for that place makes the arm
//! seen by the others do what the holder sees their own arm do.
//!
//! Nothing is stored in the game's files: all of it is computed from the skeletons and the
//! sequences of the game's own meshes. Every machine does it for the bodies it draws.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::anim::Channel;
use crate::value::ObjId;
use crate::Vm;
use openxiii_assets::coords;
use std::collections::{HashMap, HashSet};

type V = [f32; 3];

/// The player's left arm: upper arm, forearm, hand.
const ARM: [&str; 3] = ["X L UpperArm", "X L Forearm", "X L Hand"];
/// The game's left punch: its hand is a fist.
const FIST_SEQUENCE: &str = "AtakHandG";
/// Channel of the fist on the player's body, above the game's own (0, 12 to 15).
const FIST_CHANNEL: i32 = 20;
/// The first-person left hand models' bone that carries what is held (XIIILeftHand.GrabPawn),
/// their forearm and their hand.
const HELD_BONE: &str = "Box01";
const FP_FOREARM: &str = "FPSg_bras";
const FP_HAND: &str = "FPSg_main01";

/// Where a left hand must be, in the world.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reach {
    /// Place of the wrist.
    pub(crate) hand: V,
    /// Side the elbow bends to (any point on that side of the shoulder → wrist line).
    pub(crate) elbow: V,
    /// Where the axes of the first-person hand bone point (`Grip::hand` in the world): the
    /// full body's hand is turned like it. None: the hand follows the forearm.
    pub(crate) hand_axes: Option<[V; 3]>,
    /// The hand clenched (the game's left punch) instead of the body's own.
    pub(crate) fist: bool,
}

/// Where a first-person forearm and hand are, in the frame of what they hold.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Grip {
    pub(crate) elbow: V,
    pub(crate) wrist: V,
    /// Axes of the hand bone.
    pub(crate) hand: [V; 3],
}

#[derive(Debug, Default)]
pub(crate) struct State {
    /// Requests of this tick, by player's pawn.
    wanted: HashMap<ObjId, Reach>,
    /// Pawns whose left arm is posed.
    posed: HashSet<ObjId>,
    /// Lean limits asked this tick, by pawn then bone (radians).
    lean_wanted: HashMap<ObjId, Vec<(String, f32)>>,
    /// Those of the last tick: applied when the engine turns the bone (anim.rs).
    lean: HashMap<ObjId, Vec<(String, f32)>>,
}

impl Vm {
    /// Asks for the left hand of `pawn` at `reach` for this tick (to call every tick while the
    /// hand is busy, before `coop_left_arm_frame`).
    pub(crate) fn coop_left_arm_reach(&mut self, pawn: ObjId, reach: Reach) {
        self.improved.coop_left_arm.wanted.insert(pawn, reach);
    }

    /// Once per level tick, after those who ask (tick.rs).
    pub(crate) fn coop_left_arm_frame(&mut self) {
        self.improved.coop_left_arm.lean = std::mem::take(&mut self.improved.coop_left_arm.lean_wanted);
        let wanted = std::mem::take(&mut self.improved.coop_left_arm.wanted);
        let free: Vec<ObjId> = self.improved.coop_left_arm.posed.iter().copied().filter(|p| !wanted.contains_key(p)).collect();
        for pawn in free {
            self.improved.coop_left_arm.posed.remove(&pawn);
            if self.alive(pawn) {
                for bone in ARM {
                    self.set_bone_direction(pawn, bone, [0; 3], [0.0; 3], 0.0);
                }
                self.coop_left_arm_fist(pawn, None);
            }
        }
        for (pawn, reach) in wanted {
            if self.alive(pawn) && self.coop_left_arm_pose(pawn, &reach) {
                self.improved.coop_left_arm.posed.insert(pawn);
            }
        }
    }

    fn coop_left_arm_pose(&mut self, pawn: ObjId, reach: &Reach) -> bool {
        let Some(info) = self.mesh_instance(pawn).map(|i| i.info.clone()) else { return false };
        let bones = ARM.map(|n| info.bones.iter().position(|b| b.eq_ignore_ascii_case(n)));
        let [Some(upper), Some(fore), Some(hand)] = bones else { return false };
        let Some(bases) = self.get_frame(pawn) else { return false };
        // The body's world frame where the pawn stands now (the pose's own frame may be the
        // previous tick's: a moving player would reach behind).
        let mut w = crate::skel::frame_actor(
            self.coop_left_arm_place(pawn),
            self.prop(pawn, "PrePivot").as_vector(),
            self.prop(pawn, "Rotation").as_rotator(),
            self.prop(pawn, "DrawScale").as_float(),
            self.prop(pawn, "DrawScale3D").as_vector(),
            &info,
        );
        w.origin = add(w.origin, info.origin);
        let at = |i: usize| bases.get(i).map(|b| b.mul(&w).origin);
        let (Some(s), Some(e0), Some(h0)) = (at(upper), at(fore), at(hand)) else { return false };
        let (l1, l2) = (len(sub(e0, s)), len(sub(h0, e0)));
        if l1 < 1e-3 || l2 < 1e-3 {
            return false;
        }
        let (elbow, wrist, hinge) = solve_arm(s, reach.hand, reach.elbow, l1, l2);
        let x1 = unit(sub(elbow, s)).unwrap_or([1.0, 0.0, 0.0]);
        let x2 = unit(sub(wrist, elbow)).unwrap_or(x1);
        // The bones run along their X axis; the elbow's hinge is their Z axis.
        let axes_of = |x: V| -> [V; 3] { [x, cross(hinge, x), hinge] };
        self.set_bone_direction(pawn, ARM[0], crate::world::ortho_rotation(axes_of(x1)), [0.0; 3], 1.0);
        self.set_bone_direction(pawn, ARM[1], crate::world::ortho_rotation(axes_of(x2)), [0.0; 3], 1.0);
        match reach.hand_axes {
            // Fingers along the first-person hand's −Z, thumb side its −X (the full body's hand
            // has its fingers along X and its thumb side along −Z).
            Some(a) => {
                let axes = [mul(a[2], -1.0), a[1], a[0]];
                self.set_bone_direction(pawn, ARM[2], crate::world::ortho_rotation(axes), [0.0; 3], 1.0);
            }
            None => {
                self.set_bone_direction(pawn, ARM[2], [0; 3], [0.0; 3], 0.0);
            }
        }
        self.coop_left_arm_fist(pawn, reach.fist.then_some(hand));
        true
    }

    /// Where `pawn` will be drawn this tick: the players move after the co-op passes of the
    /// tick, by their velocity.
    pub(crate) fn coop_left_arm_place(&self, pawn: ObjId) -> V {
        let (at, v) = (self.prop(pawn, "Location").as_vector(), self.prop(pawn, "Velocity").as_vector());
        #[allow(unused_mut)]
        let mut lead = self.engine_delta.clamp(0.0, 0.1);
        [at[0] + v[0] * lead, at[1] + v[1] * lead, at[2]]
    }

    /// Keeps bone `bone` of `pawn`'s full body (the torso "X Spine", the head "X Head", which
    /// the engine turns to the player's aim every tick) from leaning ahead by more than `limit`
    /// radians: a player who looks down while holding someone against their chest would put
    /// their head through them. To call every tick while it holds: the limit then applies each
    /// time the engine turns the bone (`coop_lean_limited`, from anim.rs), and to the direction
    /// it has now. The player's view and aim are untouched: only the body the others see.
    pub(crate) fn coop_limit_lean(&mut self, pawn: ObjId, bone: &str, limit: f32) {
        self.improved.coop_left_arm.lean_wanted.entry(pawn).or_default().push((bone.to_string(), limit));
        let yaw = self.prop(pawn, "Rotation").as_rotator()[1];
        let Some(inst) = self.mesh_insts.get_mut(&pawn) else { return };
        let index = crate::skel::match_ref_bone(&inst.info, bone);
        let Some(d) = inst.pose.mesh_dirs.iter_mut().find(|d| d.bone == index && index >= 0 && d.rot_alpha > 0.0) else { return };
        let Some(rotation) = limited_lean(d.rotation, yaw, limit) else { return };
        d.rotation = rotation;
        // The pose of this tick, if already made, is made again with the limit.
        inst.pose.last_tick = None;
    }

    /// `pawn`'s left arm holds someone or something this tick (coop_look.rs).
    pub(crate) fn coop_left_arm_busy(&self, pawn: ObjId) -> bool {
        self.improved.coop_left_arm.posed.contains(&pawn) || self.improved.coop_left_arm.lean.contains_key(&pawn)
    }

    /// The direction the engine is about to give bone `bone` of `pawn`, within the lean limit
    /// asked for it, if any (anim.rs update_bone_control).
    pub(crate) fn coop_lean_limited(&self, pawn: ObjId, bone: &str, rotation: [i32; 3]) -> [i32; 3] {
        let Some(limit) = self.improved.coop_left_arm.lean.get(&pawn).and_then(|l| l.iter().find(|(b, _)| b.eq_ignore_ascii_case(bone))).map(|l| l.1) else {
            return rotation;
        };
        let yaw = self.prop(pawn, "Rotation").as_rotator()[1];
        limited_lean(rotation, yaw, limit).unwrap_or(rotation)
    }

    /// The fist on the hand bone `hand` of `pawn`'s body (None: the body's own hand).
    fn coop_left_arm_fist(&mut self, pawn: ObjId, hand: Option<usize>) {
        let anim = match hand {
            Some(_) => self.anim_named(pawn, FIST_SEQUENCE).map(|(i, _, _)| i),
            None => None,
        };
        let Some(inst) = self.mesh_insts.get_mut(&pawn) else { return };
        match (hand, anim) {
            (Some(hand), Some(anim)) => {
                if inst.validate(FIST_CHANNEL) {
                    inst.channels[FIST_CHANNEL as usize] = Channel {
                        anim_index: anim as i32,
                        sequence: FIST_SEQUENCE.into(),
                        blend_alpha: 1.0,
                        start_bone: hand as i32,
                        notify_ignore: true,
                        ..Default::default()
                    };
                }
            }
            _ => {
                if let Some(c) = inst.channels.get_mut(FIST_CHANNEL as usize) {
                    *c = Channel::default();
                }
            }
        }
    }

    /// The forearm and hand of the first-person left hand model `hand_mesh` around what it
    /// holds, at `frame` (0…1) of its `sequence`: the model's pose computed as the engine does,
    /// then read in the frame of the actor attached to its bone Box01 (`held_at`: that actor's
    /// RelativeLocation and RelativeRotation). Lengths in the model's units: the held actor is
    /// drawn at the model's scale.
    pub(crate) fn coop_left_arm_grip(&mut self, hand_mesh: &str, sequence: &str, frame: f32, held_at: (V, [i32; 3])) -> Option<Grip> {
        let info = self.mesh_info(hand_mesh)?;
        let set = self.anim_set(info.default_anim.as_deref()?)?;
        set.seq(sequence)?;
        let mut channels = vec![Channel { sequence: sequence.to_string(), frame: frame.clamp(0.0, 1.0), ..Default::default() }];
        let mut pose = crate::skel::PoseState::default();
        let bases = crate::skel::compute_pose(&info, &[set], &mut channels, &mut pose, 0);
        // The model at the world's origin, unturned, at scale 1.
        let mut world = crate::skel::frame_actor([0.0; 3], [0.0; 3], [0; 3], 1.0, [1.0; 3], &info);
        world.origin = add(world.origin, info.origin);
        let place_at = |name: &str, rel: V, rel_rot: [i32; 3]| {
            let b = crate::skel::match_ref_bone(&info, name);
            let c = bases.get(usize::try_from(b).ok()?)?;
            let (at, turn) = crate::bone_attach::pose_attach(c, &coords::UNIT, rel, rel_rot, &world);
            Some((at, crate::ai_natives::axes(turn)))
        };
        let place = |name: &str| place_at(name, [0.0; 3], [0; 3]);
        let (held, held_axes) = place_at(HELD_BONE, held_at.0, held_at.1)?;
        let (elbow, _) = place(FP_FOREARM)?;
        let (wrist, hand) = place(FP_HAND)?;
        let within = |p: V| -> V { std::array::from_fn(|k| dot(p, held_axes[k])) };
        Some(Grip { elbow: within(sub(elbow, held)), wrist: within(sub(wrist, held)), hand: hand.map(within) })
    }
}

impl Grip {
    /// The grip around an actor at `at`, turned `rotation`, drawn at `scale`: wrist, elbow and
    /// hand axes in the world.
    pub(crate) fn around(&self, at: V, rotation: [i32; 3], scale: f32) -> (V, V, [V; 3]) {
        let frame = crate::ai_natives::axes(rotation);
        let turn = |p: V| -> V { std::array::from_fn(|k| p[0] * frame[0][k] + p[1] * frame[1][k] + p[2] * frame[2][k]) };
        let out = |p: V| add(at, mul(turn(p), scale));
        (out(self.wrist), out(self.elbow), self.hand.map(turn))
    }
}

/// Two-bone solve from `shoulder`: the wrist as near `wrist` as an arm of these two lengths
/// reaches, the elbow on the side of `hint`. Returns the elbow, the wrist and the elbow's hinge
/// axis.
pub(crate) fn solve_arm(shoulder: V, wrist: V, hint: V, upper: f32, fore: f32) -> (V, V, V) {
    let reach = sub(wrist, shoulder);
    let dist = len(reach).clamp((upper - fore).abs() + 0.5, upper + fore - 0.5);
    let dir = unit(reach).unwrap_or([0.0, 0.0, 1.0]);
    let along = (upper * upper - fore * fore + dist * dist) / (2.0 * dist);
    let height = (upper * upper - along * along).max(0.0).sqrt();
    let hint = sub(hint, shoulder);
    let side = sub(hint, mul(dir, dot(hint, dir)));
    let side = unit(side).or_else(|| unit(cross(dir, [0.0, 0.0, 1.0]))).unwrap_or([1.0, 0.0, 0.0]);
    let elbow = add(add(shoulder, mul(dir, along)), mul(side, height));
    let wrist = add(shoulder, mul(dir, dist));
    let hinge = unit(cross(sub(elbow, shoulder), sub(wrist, elbow))).or_else(|| unit(cross(dir, side))).unwrap_or([0.0, 0.0, 1.0]);
    (elbow, wrist, hinge)
}

/// The rotator of a bone direction (its X axis runs up the body) brought back to at most
/// `limit` radians of lean towards the front of a body facing `yaw`; None when it is within.
pub(crate) fn limited_lean(rotation: [i32; 3], yaw: i32, limit: f32) -> Option<[i32; 3]> {
    let axes = crate::ai_natives::axes(rotation);
    let (sy, cy) = (yaw as f32 * std::f32::consts::PI / 32768.0).dsin_cos();
    let lean = |x: V| (x[0] * cy + x[1] * sy).datan2(x[2]);
    let over = lean(axes[0]) - limit;
    if over <= 0.0 {
        return None;
    }
    // Turned back around the body's side axis, whichever way reduces the lean.
    let side = [-sy, cy, 0.0];
    let turned = |a: f32| -> [V; 3] {
        let (s, c) = a.dsin_cos();
        axes.map(|v| add(add(mul(v, c), mul(cross(side, v), s)), mul(side, dot(side, v) * (1.0 - c))))
    };
    let (a, b) = (turned(over), turned(-over));
    let best = if lean(a[0]).abs() < lean(b[0]).abs() { a } else { b };
    Some(crate::world::ortho_rotation(best))
}

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn len(a: V) -> f32 {
    dot(a, a).sqrt()
}

fn unit(a: V) -> Option<V> {
    let l = len(a);
    (l > 1e-4).then(|| mul(a, 1.0 / l))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The solve keeps the arm's two lengths, reaches the wrist when it can, stops at full reach
    /// when it cannot, and bends the elbow towards the hint.
    #[test]
    fn arm_solve() {
        let (upper, fore) = (23.0, 21.0);
        let shoulder = [10.0, -5.0, 40.0];
        let mut seed = 12345u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0 * 2.0 - 1.0
        };
        for _ in 0..200 {
            let wrist = add(shoulder, [next() * 60.0, next() * 60.0, next() * 60.0]);
            let hint = add(shoulder, [next() * 30.0, next() * 30.0, next() * 30.0]);
            let (elbow, hand, hinge) = solve_arm(shoulder, wrist, hint, upper, fore);
            assert!((len(sub(elbow, shoulder)) - upper).abs() < 1e-2, "upper arm length");
            assert!((len(sub(hand, elbow)) - fore).abs() < 1e-2, "forearm length");
            let far = len(sub(wrist, shoulder));
            if far > (upper - fore).abs() + 0.5 && far < upper + fore - 0.5 {
                assert!(len(sub(hand, wrist)) < 1e-2, "wrist reached");
            } else {
                assert!(len(sub(hand, shoulder)) <= upper + fore, "wrist within reach");
            }
            // The elbow is on the hint's side of the shoulder-wrist line.
            let dir = unit(sub(hand, shoulder)).unwrap();
            let off = |p: V| sub(sub(p, shoulder), mul(dir, dot(sub(p, shoulder), dir)));
            if len(off(hint)) > 1.0 {
                assert!(dot(off(elbow), off(hint)) >= -1e-3, "elbow towards the hint");
            }
            assert!(dot(hinge, sub(elbow, shoulder)).abs() < 1e-2 && dot(hinge, sub(hand, elbow)).abs() < 1e-2, "hinge across the arm");
        }
    }

    /// A torso leaning ahead beyond the limit is brought back to it, about the body's side
    /// axis; one within the limit, or leaning back, is left alone.
    #[test]
    fn lean_is_limited() {
        let limit = 0.1f32;
        for yaw in [0, 9000, -20000, 40000] {
            let (sy, cy) = (yaw as f32 * std::f32::consts::PI / 32768.0).dsin_cos();
            let lean = |r: [i32; 3]| {
                let x = crate::ai_natives::axes(r)[0];
                (x[0] * cy + x[1] * sy).datan2(x[2])
            };
            // The engine's torso rotator for a view pitched by `pitch` (anim.rs check_os).
            let torso = |pitch: i32| {
                let [x, y, z] = crate::ai_natives::axes([pitch, yaw, 0]);
                let neg = |v: V| mul(v, -1.0);
                crate::world::ortho_rotation([z, neg(x), neg(y)])
            };
            for pitch in [-7000, -4000, -1500] {
                let before = torso(pitch);
                assert!(lean(before) > limit, "looking down leans ahead: {}", lean(before));
                let after = limited_lean(before, yaw, limit).expect("limited");
                assert!((lean(after) - limit).abs() < 0.01, "yaw {yaw} pitch {pitch}: {} -> {}", lean(before), lean(after));
                // Still square to the body: the side axis is kept.
                let (zb, za) = (crate::ai_natives::axes(before)[2], crate::ai_natives::axes(after)[2]);
                assert!(len(sub(zb, za)) < 0.02, "{zb:?} {za:?}");
            }
            for pitch in [0, 300, 5000] {
                assert!(limited_lean(torso(pitch), yaw, limit).is_none(), "pitch {pitch}");
            }
        }
    }

    /// A grip moves and turns with the actor it is around.
    #[test]
    fn grip_around_an_actor() {
        let g = Grip { elbow: [10.0, 0.0, 0.0], wrist: [0.0, 5.0, 0.0], hand: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] };
        let (wrist, elbow, hand) = g.around([100.0, 200.0, 300.0], [0, 16384, 0], 2.0);
        // A quarter turn of yaw: ahead becomes +Y.
        assert!(len(sub(elbow, [100.0, 220.0, 300.0])) < 1e-2, "{elbow:?}");
        assert!((len(sub(wrist, [100.0, 200.0, 300.0])) - 10.0).abs() < 1e-2, "{wrist:?}");
        assert!(len(sub(hand[0], [0.0, 1.0, 0.0])) < 1e-3, "{hand:?}");
        assert!(len(sub(g.around([0.0; 3], [0; 3], 1.0).1, g.elbow)) < 1e-4);
    }
}
