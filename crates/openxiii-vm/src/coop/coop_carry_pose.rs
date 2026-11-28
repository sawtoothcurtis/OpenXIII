//! Improved build only (co-op): the player who carries a corpse holds it with the left arm, seen
//! by the other players as the carrier sees it in first person.
//!
//! The game shows a carried corpse only in the carrier's first-person view: the left hand model
//! (XIIILeftHand, mesh fpsCadavreM: a forearm and a hand) holds a copy of the body attached to
//! its bone Box01, through the sequences Select (picked up), Wait (held) and Down (put down).
//! The full body of the player has no sequence for it: seen by another co-op player, the
//! carrier kept both arms in their usual pose under the body on the shoulder (coop_carry.rs), and
//! went on using the left hand (two-handed poses, reloads, punches).
//!
//! Nothing is stored for this: each frame, the place of the first-person forearm and hand around
//! the body is computed from the game's own data (same hand model, same sequence, same time as
//! the body seen on the shoulder: coop_left_arm_grip), and the left arm of the carrier's full
//! body is asked there (coop_left_arm.rs): the hand at the first-person hand's place and turned
//! like it, the elbow bent towards the first-person forearm. While the corpse is carried the
//! left arm takes no part in firing, reloading, items or punches, which the right arm goes on
//! playing.
//!
//! Every machine does it for the carriers it sees (the shoulder copy is found the same way as
//! in coop_carry.rs). Outside a co-op session nothing here runs: the solo game is the original
//! one.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;

use openxiii_assets::coords;

use crate::coop_left_arm::Reach;
use crate::value::ObjId;
use crate::Vm;

type V = [f32; 3];

/// The first-person left hand model that carries a corpse (XIIILeftHand.mCorpse).
const HAND_MESH: &str = "XIIIArmes.fpsCadavreM";
/// The carrier's left arm: upper arm, forearm, hand.
const ARM: [&str; 3] = ["X L UpperArm", "X L Forearm", "X L Hand"];
/// The body's sequences start with this; the hand model's are the same names without it
/// (XIIILeftHand PlayGrabbing, PlayWaiting, PlayUnGrabbing).
const BODY_PREFIX: &str = "Cadavre";
/// Sequence of the hand model while the body is held.
const HOLD: &str = "Wait";
/// Smallest DrawScale of a shoulder copy (coop_carry.rs; the first-person one is at 0.3).
const MIN_SCALE: f32 = 0.5;
/// Seconds the arm takes to leave its own pose when the corpse is picked up.
const TAKE: f32 = 0.25;

#[derive(Default)]
pub struct State {
    /// Carrier pawn → the hand model's sequence and the level time it started at.
    posed: HashMap<ObjId, (String, f32)>,
    /// Carrier pawn → the level time the arm left its own pose, and that pose: elbow and wrist
    /// from the shoulder, in the carrier's frame.
    taken: HashMap<ObjId, (f32, [V; 2])>,
}

impl Vm {
    /// Once per level tick, after coop_carry_frame and before coop_left_arm_frame.
    pub(crate) fn coop_carry_pose_frame(&mut self) {
        let on = self.coop.on;
        let mut carried: Vec<(ObjId, ObjId)> = Vec::new();
        if on {
            // The shoulder copies, as coop_carry_local finds them.
            carried = self
                .actors
                .iter()
                .copied()
                .filter(|&a| self.alive(a) && self.is_a(a, "HeldBody") && self.prop(a, "DrawScale").as_float() >= MIN_SCALE)
                .filter(|&a| !self.prop(a, "Base").as_object().is_some_and(|b| self.alive(b) && self.is_a(b, "XIIILeftHand")))
                .filter_map(|a| {
                    let carrier = self.prop(a, "Instigator").as_object().filter(|&p| self.alive(p) && self.is_a(p, "XIIIPlayerPawn"))?;
                    Some((carrier, a))
                })
                .collect();
        }
        // A carrier without a copy any more: no request, the arm goes back to the body's sequences.
        self.improved.coop_carry_pose.posed.retain(|p, _| carried.iter().any(|(c, _)| c == p));
        self.improved.coop_carry_pose.taken.retain(|p, _| carried.iter().any(|(c, _)| c == p));
        for (carrier, copy) in carried {
            self.coop_carry_pose_arm(carrier, copy);
        }
    }

    /// The left arm of `carrier` around the shoulder copy `copy`.
    fn coop_carry_pose_arm(&mut self, carrier: ObjId, copy: ObjId) {
        // The hand model's sequence and time, from the body's: both start together in the game.
        let body = self.mesh_insts.get(&copy).and_then(|i| i.channels.first()).map(|c| (c.sequence.clone(), c.frame, c.rate));
        let (sequence, since) = match body {
            Some((s, frame, rate)) if s.len() > BODY_PREFIX.len() && s[..BODY_PREFIX.len()].eq_ignore_ascii_case(BODY_PREFIX) => {
                let since = if rate > 0.0 && frame > 0.0 { frame / rate } else { 0.0 };
                (s[BODY_PREFIX.len()..].to_string(), since)
            }
            _ => (HOLD.to_string(), 0.0),
        };
        let now = self.level_time as f32;
        let start = match self.improved.coop_carry_pose.posed.get(&carrier) {
            Some((s, t)) if s.eq_ignore_ascii_case(&sequence) => *t,
            _ => {
                let t = now - since;
                self.improved.coop_carry_pose.posed.insert(carrier, (sequence.clone(), t));
                t
            }
        };
        // Where the hand model's sequence is, at its own rate (PlayAnim at rate 1).
        let Some(frame) = self.coop_carry_pose_instant(&sequence, (now - start).max(0.0)) else { return };
        // Where the game attaches the body on the hand model's bone (HeldBody's defaults).
        let held_at = (self.prop(copy, "RelativeLocation").as_vector(), self.prop(copy, "RelativeRotation").as_rotator());
        let Some(grip) = self.coop_left_arm_grip(HAND_MESH, &sequence, frame, held_at) else { return };
        // The grip in the world, around the copy.
        let (wrist, elbow_hint, hand) =
            grip.around(self.prop(copy, "Location").as_vector(), self.prop(copy, "Rotation").as_rotator(), self.prop(copy, "DrawScale").as_float());

        // The arm leaves its own pose for the grip over TAKE seconds: its shoulder, elbow and
        // wrist when the corpse is picked up.
        let Some(info) = self.mesh_instance(carrier).map(|i| i.info.clone()) else { return };
        let bones = ARM.map(|n| crate::skel::match_ref_bone(&info, n));
        if bones.iter().any(|&b| b < 0) {
            return;
        }
        let Some(bases) = self.get_frame(carrier) else { return };
        let Some(world) = self.mesh_world_coords(carrier) else { return };
        let place = |b: i32| bases.get(b as usize).map(|c| crate::bone_attach::pose_attach(c, &coords::UNIT, [0.0; 3], [0; 3], &world).0);
        let (Some(shoulder), Some(elbow), Some(palm)) = (place(bones[0]), place(bones[1]), place(bones[2])) else { return };
        let own = crate::ai_natives::axes([0, self.prop(carrier, "Rotation").as_rotator()[1], 0]);
        let (since, rest) = *self.improved.coop_carry_pose.taken.entry(carrier).or_insert_with(|| {
            let within = |p: V| -> V { std::array::from_fn(|k| (0..3).map(|c| (p[c] - shoulder[c]) * own[k][c]).sum()) };
            (now, [within(elbow), within(palm)])
        });
        let part = ((now - since) / TAKE).clamp(0.0, 1.0);
        let part = part * part * (3.0 - 2.0 * part);
        let from = |p: V| -> V { std::array::from_fn(|k| shoulder[k] + p[0] * own[0][k] + p[1] * own[1][k] + p[2] * own[2][k]) };
        let blend = |a: V, b: V| -> V { std::array::from_fn(|k| a[k] + (b[k] - a[k]) * part) };
        let (elbow_hint, wrist) = (blend(from(rest[0]), elbow_hint), blend(from(rest[1]), wrist));

        self.coop_left_arm_reach(carrier, Reach { hand: wrist, elbow: elbow_hint, hand_axes: Some(hand), fist: false });
    }

    /// Where the hand model's `sequence` is (0…1) `seconds` after its start: played once, the
    /// held one looped.
    fn coop_carry_pose_instant(&mut self, sequence: &str, seconds: f32) -> Option<f32> {
        let info = self.mesh_info(HAND_MESH)?;
        let set = self.anim_set(info.default_anim.as_deref()?)?;
        let seq = &set.seqs[set.seq(sequence)?];
        // Sequence rate in frames per second.
        let frame = if seq.num_frames > 0 { seconds * seq.rate / seq.num_frames as f32 } else { 0.0 };
        Some(if sequence.eq_ignore_ascii_case(HOLD) { frame.fract() } else { frame.min(1.0) })
    }
}
