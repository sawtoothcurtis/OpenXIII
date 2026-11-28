//! Improved build only: in co-op, the hostage a player holds, seen by the other players.
//!
//! In the game (solo only), taking a hostage (XIIILeftHand.GrabPawn) hides the hostage's pawn
//! (XIIIPawn state Prisonner: DrawType DT_None) and draws it in the holder's first-person view
//! only: a HeldBody with the hostage's mesh and skin, attached to bone Box01 of the left hand
//! (XIIIArmes.fpsOtageM, DrawScale 0.3, in front of the eyes), playing the hostage's own
//! sequences OtageSelect / OtageWait / OtageMov / OtageDownKill in step with the hand. With two
//! players the other one sees nothing: the hostage vanishes from the world.
//!
//! Here each player who holds a live hostage gets one more HeldBody (the "puppet"), seen by
//! every view but the holder's own (bOwnerNoSee, owner = the holder's pawn): the same hostage
//! mesh, skin and sequence as the first-person one, at the same instant (what the hostage does
//! in the holder's view is what the others see), standing upright against the holder's chest.
//! The game has no third-person side of a hostage taking; it is put together here from the
//! game's own data: the hostage's legs borrow the walking sequence's when the holder moves,
//! and the holder's full body puts its left arm around the hostage's throat, fist clenched
//! (coop_left_arm.rs), which leaves it the right hand only. The game's own actors are untouched.
//!
//! Network co-op: the host makes the puppets; they reach the guests by the game's replication
//! (HeldBody is a simulated proxy: mesh, place, sequence through SimAnim), their skin by the
//! skins message of mod_net_skins.rs. A guest only hides the puppet of its own player.
//!
//! Nothing here runs outside a co-op session (the hostage of the solo game is the original one)
//! nor for a carried corpse.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{ObjId, Value};
use crate::Vm;
use std::collections::HashMap;

/// Height of the hostage's lowest bone (an ankle) above the ground, when the mesh has no
/// standing sequence to measure it.
const FOOT: f32 = 6.0;
/// Place of the hostage's spine from the holder's centre: ahead, and to the right.
const FRONT: f32 = 16.0;
const SIDE: f32 = -9.0;
/// How much further ahead when the holder moves at full speed.
const RUN_FRONT: f32 = 9.0;
/// How much the hostage's trunk leans back against the holder (tangent).
const LEAN: f32 = 0.06;
/// Without the first-person hand model's own place: the holder's left hand and the side of
/// its elbow, from the hostage's neck: ahead, to the right, up (the holder's directions).
const HAND: [f32; 3] = [15.0, 9.0, 6.5];
const ELBOW: [f32; 3] = [14.0, -14.0, 0.0];
/// The first-person left hand model that holds a hostage (XIIILeftHand.mPrisonner), and what
/// the hostage's sequences have in front of the hand model's names.
const HAND_MESH: &str = "XIIIArmes.fpsOtageM";
const SEQ_PREFIX: &str = "Otage";
/// Most the holder's torso and head lean ahead while they hold a hostage (radians).
const SPINE_LEAN: f32 = 0.10;
const HEAD_LEAN: f32 = 0.30;
/// Bones of the characters' skeleton.
const SPINE: &str = "X Spine1";
const PELVIS: &str = "X Pelvis";
const NECK: &str = "X Neck";
/// The hostage's waiting sequence (first-person hostage, XIIILeftHand.PlayWaiting) and a
/// character's plain standing one.
const WAIT: &str = "OtageWait";
const STAND: &str = "WaitNeutre";
const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// Walking sequences whose legs the hostage borrows, the holder's speed they are made for,
/// and the speed from which the legs start to step.
const STEP: &str = "Walk";
const STEP_BACK: &str = "MarcheArrStandar";
const STEP_SPEED: f32 = 150.0;
const STEP_FROM: f32 = 15.0;
/// Weight of the walking legs over the sequence's still ones: small steps.
const STEP_WEIGHT: f32 = 0.6;

/// How a puppet stands against its holder, in the holder's directions (ahead, right, up).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fit {
    /// Where the puppet's forward, right and up axes point.
    axes: [[f32; 3]; 3],
    /// The puppet's Location from the holder's: ahead, right, above the holder's ground.
    offset: [f32; 3],
}

struct Tune {
    front: f32,
    side: f32,
    lean: f32,
    hand: [f32; 3],
    elbow: [f32; 3],
}

fn tune() -> Tune {
    #[allow(unused_mut)]
    let mut t = Tune { front: FRONT, side: SIDE, lean: LEAN, hand: HAND, elbow: ELBOW };
    t
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-6 {
        [0.0, 0.0, 1.0]
    } else {
        v.map(|c| c / l)
    }
}

/// Matrix of the shortest turn from unit vector `a` to unit vector `d`.
fn arc(a: [f32; 3], d: [f32; 3]) -> [[f32; 3]; 3] {
    let k = [a[1] * d[2] - a[2] * d[1], a[2] * d[0] - a[0] * d[2], a[0] * d[1] - a[1] * d[0]];
    let (s2, c) = (k[0] * k[0] + k[1] * k[1] + k[2] * k[2], a[0] * d[0] + a[1] * d[1] + a[2] * d[2]);
    if s2 < 1e-10 {
        return IDENTITY;
    }
    let f = (1.0 - c) / s2;
    [
        [c + f * k[0] * k[0], f * k[0] * k[1] - k[2], f * k[0] * k[2] + k[1]],
        [f * k[1] * k[0] + k[2], c + f * k[1] * k[1], f * k[1] * k[2] - k[0]],
        [f * k[2] * k[0] - k[1], f * k[2] * k[1] + k[0], c + f * k[2] * k[2]],
    ]
}

/// Puppets by holder's pawn.
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) puppets: HashMap<ObjId, ObjId>,
    /// How each puppet stands against its holder.
    pub(crate) fits: HashMap<ObjId, Fit>,
    /// Weight of each puppet's walking legs (0 = the sequence's still legs).
    pub(crate) steps: HashMap<ObjId, f32>,
}

/// What a player holds as a hostage: (first-person HeldBody, hostage's pawn).
struct Held {
    body: ObjId,
    hostage: ObjId,
}

impl Vm {
    /// Once per level tick in a co-op game (tick.rs, after the players' looks).
    pub(crate) fn coop_hostage_frame(&mut self) {
        let on = !self.improved.character_screen.on && self.coop.on;
        if !on {
            self.improved.coop_hostage.puppets.clear();
            self.improved.coop_hostage.fits.clear();
            self.improved.coop_hostage.steps.clear();
            return;
        }
        let pawns: Vec<ObjId> =
            self.actors.iter().copied().filter(|&a| self.alive(a) && self.is_a(a, "XIIIPlayerPawn")).collect();
        let mut held: Vec<(ObjId, Held)> = Vec::new();
        for pawn in pawns {
            if let Some(h) = self.coop_held_hostage(pawn) {
                held.push((pawn, h));
            }
        }
        // Puppets whose hostage was let go (or whose holder is gone).
        let old: Vec<(ObjId, ObjId)> = self.improved.coop_hostage.puppets.iter().map(|(&p, &q)| (p, q)).collect();
        for (pawn, puppet) in old {
            if !held.iter().any(|(p, _)| *p == pawn) || !self.alive(puppet) {
                self.improved.coop_hostage.puppets.remove(&pawn);
                self.improved.coop_hostage.fits.remove(&puppet);
                self.improved.coop_hostage.steps.remove(&puppet);
                if self.alive(puppet) {
                    let _ = self.destroy_actor(puppet);
                }
                self.log.push(format!("Log: Co-op: hostage of {} let go", self.obj_label(pawn)));
            }
        }
        for (pawn, h) in held {
            let puppet = match self.improved.coop_hostage.puppets.get(&pawn).copied() {
                Some(p) => p,
                None => {
                    let Some(p) = self.coop_hostage_spawn(pawn, &h) else { continue };
                    self.improved.coop_hostage.puppets.insert(pawn, p);
                    self.log.push(format!("Log: Co-op: {} holds {} as a hostage", self.obj_label(pawn), self.obj_label(h.hostage)));
                    p
                }
            };
            self.coop_hostage_place(pawn, &h, puppet);
        }
    }

    /// The live hostage `pawn` holds in its left hand, if any.
    fn coop_held_hostage(&self, pawn: ObjId) -> Option<Held> {
        let hand = self.prop(pawn, "LHand").as_object().filter(|&h| self.alive(h))?;
        if !self.prop(hand, "bActive").as_bool() {
            return None;
        }
        let hostage = self.prop(hand, "pOnShoulder").as_object().filter(|&h| self.alive(h))?;
        // A carried corpse is not a hostage.
        if self.prop(hostage, "bIsDead").as_bool() || !self.is_a(hostage, "XIIIPawn") {
            return None;
        }
        let body = self.prop(hand, "TheHeldBody").as_object().filter(|&b| self.alive(b))?;
        Some(Held { body, hostage })
    }

    /// A new puppet for the hostage `h` of `pawn`.
    fn coop_hostage_spawn(&mut self, pawn: ObjId, h: &Held) -> Option<ObjId> {
        let class = self.class("XIII.HeldBody")?;
        let (l, r) = (self.prop(pawn, "Location"), self.prop(pawn, "Rotation"));
        let puppet = self.spawn_actor(class, Value::Object(Some(pawn)), None, l, r, Value::Object(None), true).ok().flatten()?;
        let _ = self.call_named(puppet, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
        self.set_prop(puppet, "Mesh", self.prop(h.body, "Mesh"));
        self.set_prop(puppet, "Skins", self.prop(h.body, "Skins"));
        let scale = self.prop(h.hostage, "DrawScale").as_float();
        self.set_prop(puppet, "DrawScale", Value::Float(if scale > 0.0 { scale } else { 1.0 }));
        self.set_prop(puppet, "DrawScale3D", self.prop(h.hostage, "DrawScale3D"));
        self.set_prop(puppet, "PrePivot", self.prop(h.hostage, "PrePivot"));
        for name in ["AmbientGlow", "bUnlit"] {
            self.set_prop(puppet, name, self.prop(h.hostage, name));
        }
        self.set_prop(puppet, "bHidden", Value::Bool(false));
        self.set_prop(puppet, "bOwnerNoSee", Value::Bool(true));
        self.set_prop(puppet, "bIgnoreVignetteAlpha", Value::Bool(false));
        self.set_prop(puppet, "bUpdateSimulatedPosition", Value::Bool(true));
        self.set_prop(puppet, "bReplicateAnimations", Value::Bool(true));
        // Spawned hidden (InventoryAttachment): into the level's display list now
        // (AActor::RefreshDisplaying).
        self.refresh_dyn_disp(puppet);
        Some(puppet)
    }

    /// The puppet in front of its holder, with the first-person hostage's sequence at the same
    /// instant, its legs walking with the holder and the holder's left arm around its throat.
    fn coop_hostage_place(&mut self, pawn: ObjId, h: &Held, puppet: ObjId) {
        // The first-person hostage's sequence, frame for frame.
        let ch = self.mesh_insts.get(&h.body).and_then(|i| i.channels.first().cloned());
        // The puppet's animation list as the first-person body's (the mesh's default one,
        // linked at the first lookup of a sequence).
        if let Some(seq) = ch.as_ref().map(|c| c.sequence.clone()).filter(|s| !s.is_empty()) {
            let _ = self.anim_named(puppet, &seq);
        }
        if let (Some(ch), Some(inst)) = (ch, self.mesh_instance(puppet)) {
            if inst.validate(0) {
                inst.channels[0] = ch;
                inst.channels[0].notify_ignore = true;
            }
        }
        let fit = match self.improved.coop_hostage.fits.get(&puppet).copied() {
            Some(f) => f,
            None => {
                let Some(f) = self.coop_hostage_fit(puppet, h.hostage) else { return };
                self.improved.coop_hostage.fits.insert(puppet, f);
                f
            }
        };
        let yaw = self.prop(pawn, "Rotation").as_rotator()[1];
        let (sy, cy) = (yaw as f32 * std::f32::consts::PI / 32768.0).dsin_cos();
        let at = self.coop_left_arm_place(pawn);
        let ground = at[2] - self.prop(pawn, "CollisionHeight").as_float();
        // The same place for every sequence: the bone that carries the first-person hostage
        // never moves in the left hand's sequences, all that the hostage does is in its own.
        // A moving holder's body leans ahead: the hostage is pushed that much further.
        let pushed = self.improved.coop_hostage.steps.get(&puppet).copied().unwrap_or(0.0) / STEP_WEIGHT * RUN_FRONT;
        let o = [fit.offset[0] + pushed, fit.offset[1], fit.offset[2]];
        let place = [at[0] + o[0] * cy - o[1] * sy, at[1] + o[0] * sy + o[1] * cy, ground + o[2]];
        let rot = crate::world::ortho_rotation(fit.axes.map(|a| [a[0] * cy - a[1] * sy, a[0] * sy + a[1] * cy, a[2]]));
        let _ = self.call_named(puppet, "SetLocation", vec![Value::vector(place[0], place[1], place[2])]);
        let _ = self.call_named(puppet, "SetRotation", vec![Value::rotator(rot[0], rot[1], rot[2])]);
        self.coop_hostage_visual(pawn, puppet);
    }

    /// How the puppet stands against its holder: measured once on the first frame of the
    /// hostage's waiting sequence, kept for all the sequences (they are made around the same
    /// place), so that what the hostage does in them is seen as it is.
    fn coop_hostage_fit(&mut self, puppet: ObjId, hostage: ObjId) -> Option<Fit> {
        let t = tune();
        let info = self.mesh_instance(puppet).map(|i| i.info.clone())?;
        let bone = |n: &str| info.bones.iter().position(|b| b.eq_ignore_ascii_case(n));
        let (pelvis, neck, spine) = (bone(PELVIS)?, bone(NECK)?, bone(SPINE)?);
        let (pre, scale, ds3) = (self.prop(puppet, "PrePivot").as_vector(), self.prop(puppet, "DrawScale").as_float(), self.prop(puppet, "DrawScale3D").as_vector());
        let frame = |rot: [i32; 3]| with_origin(&info, crate::skel::frame_actor([0.0; 3], pre, rot, scale, ds3, &info));
        let wait = self.coop_pose_of(puppet, WAIT, 0.0)?;
        // The sequence leans the hostage (it is made for the holder's eyes): the trunk is set
        // upright, a little back against the holder.
        let f0 = frame([0; 3]);
        let (p, n) = (wait.get(pelvis)?.mul(&f0).origin, wait.get(neck)?.mul(&f0).origin);
        let trunk = unit([n[0] - p[0], n[1] - p[1], n[2] - p[2]]);
        let q = arc(trunk, unit([-t.lean, 0.0, 1.0]));
        // Columns of q: where the actor's forward, right and up go.
        let axes = [0, 1, 2].map(|k| [q[0][k], q[1][k], q[2][k]]);
        let rot = crate::world::ortho_rotation(axes);
        let f = frame(rot);
        let lowest = wait.iter().map(|b| b.mul(&f).origin[2]).fold(f32::INFINITY, f32::min);
        let s = wait.get(spine)?.mul(&f).origin;
        // Height of the lowest bone of this mesh above the ground when its pawn stands.
        let ch = self.prop(hostage, "CollisionHeight").as_float();
        let sole = self
            .coop_pose_of(puppet, STAND, 0.0)
            .map(|b| b.iter().map(|b| b.mul(&f0).origin[2]).fold(f32::INFINITY, f32::min) + ch)
            .filter(|s| (-12.0..=20.0).contains(s))
            .unwrap_or(FOOT);
        if !lowest.is_finite() {
            return None;
        }
        Some(Fit { axes, offset: [t.front - s[0], t.side - s[1], sole - lowest] })
    }

    /// SpaceBases of `actor`'s mesh at `frame` (0…1) of `sequence`; the instance is untouched.
    fn coop_pose_of(&mut self, actor: ObjId, sequence: &str, frame: f32) -> Option<Vec<openxiii_assets::coords::Coords>> {
        let (anim, _, _) = self.anim_named(actor, sequence)?;
        let inst = self.mesh_insts.get(&actor)?;
        let (info, anims) = (inst.info.clone(), inst.anims.clone());
        let mut channels = vec![crate::anim::Channel { anim_index: anim as i32, sequence: sequence.into(), frame, ..Default::default() }];
        let mut pose = crate::skel::PoseState::default();
        Some(crate::skel::compute_pose(&info, &anims, &mut channels, &mut pose, 0))
    }

    /// What every machine adds to a puppet it draws: the legs' small steps when the holder
    /// moves, and the holder's left arm around the hostage's throat.
    pub(crate) fn coop_hostage_visual(&mut self, pawn: ObjId, puppet: ObjId) {
        let t = tune();
        let yaw = self.prop(pawn, "Rotation").as_rotator()[1];
        let (sy, cy) = (yaw as f32 * std::f32::consts::PI / 32768.0).dsin_cos();
        // Legs: the sequences keep them still (they are never in the holder's view).
        let v = self.prop(pawn, "Velocity").as_vector();
        #[allow(unused_mut)]
        let mut speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
        let back = v[0] * cy + v[1] * sy < -0.3 * speed;
        let want = ((speed - STEP_FROM) / STEP_FROM).clamp(0.0, 1.0) * STEP_WEIGHT;
        let dt = self.engine_delta.clamp(0.0, 0.1);
        let alpha = {
            let a = self.improved.coop_hostage.steps.entry(puppet).or_insert(0.0);
            *a += (want - *a) * (dt * 10.0).min(1.0);
            *a
        };
        let seq = if back { STEP_BACK } else { STEP };
        for (ch, bone) in [(1, "X L Thigh"), (2, "X R Thigh")] {
            let playing = self.mesh_insts.get(&puppet).and_then(|i| i.channels.get(ch as usize)).is_some_and(|c| c.sequence.eq_ignore_ascii_case(seq));
            if !playing && (alpha <= 0.01 || !self.play_anim(puppet, ch, seq, 1.0, 0.0, true)) {
                continue;
            }
            if let Some(inst) = self.mesh_insts.get_mut(&puppet) {
                let _ = inst.params_blend(ch, alpha, 0.0, 0.0, bone);
                let c = &mut inst.channels[ch as usize];
                c.notify_ignore = true;
                c.rate = c.orig_rate * (speed / STEP_SPEED).clamp(0.6, 1.6);
            }
        }
        // A holder who looks down would put their head through the hostage: the body the
        // others see keeps its torso and head nearly upright (the view itself is untouched).
        self.coop_limit_lean(pawn, "X Spine", SPINE_LEAN);
        self.coop_limit_lean(pawn, "X Head", HEAD_LEAN);
        // The holder's left arm, where the first-person left hand model has its own around
        // the hostage at this instant of its sequence (the hand model's sequences are the
        // hostage's without their prefix, started together: XIIILeftHand.PlayWaiting…).
        let body = self.mesh_insts.get(&puppet).and_then(|i| i.channels.first()).map(|c| (c.sequence.clone(), c.frame.max(0.0)));
        let held_at = (self.prop(puppet, "RelativeLocation").as_vector(), self.prop(puppet, "RelativeRotation").as_rotator());
        let grip = body.and_then(|(seq, frame)| {
            let hand_seq = seq.get(SEQ_PREFIX.len()..).filter(|_| seq.len() > SEQ_PREFIX.len() && seq[..SEQ_PREFIX.len()].eq_ignore_ascii_case(SEQ_PREFIX))?.to_string();
            self.coop_left_arm_grip(HAND_MESH, &hand_seq, frame, held_at)
        });
        if let Some(g) = grip {
            let (wrist, elbow, hand) = g.around(self.prop(puppet, "Location").as_vector(), self.prop(puppet, "Rotation").as_rotator(), self.prop(puppet, "DrawScale").as_float());
            // The fist keeps the forearm's line (the first-person fist is turned for the holder's
            // eyes only).
            #[allow(unused_mut)]
            let mut hand_turn = false;
            self.coop_left_arm_reach(pawn, crate::coop_left_arm::Reach { hand: wrist, elbow, hand_axes: hand_turn.then_some(hand), fist: true });
            return;
        }
        // A hand model without these bones or sequences: forearm across the throat, fist on
        // the far side.
        let Some(info) = self.mesh_instance(puppet).map(|i| i.info.clone()) else { return };
        let Some(neck) = info.bones.iter().position(|b| b.eq_ignore_ascii_case(NECK)) else { return };
        let Some(bones) = self.get_frame(puppet) else { return };
        let frame = with_origin(&info, crate::skel::frame_actor(
            self.prop(puppet, "Location").as_vector(),
            self.prop(puppet, "PrePivot").as_vector(),
            self.prop(puppet, "Rotation").as_rotator(),
            self.prop(puppet, "DrawScale").as_float(),
            self.prop(puppet, "DrawScale3D").as_vector(),
            &info,
        ));
        let Some(n) = bones.get(neck).map(|b| b.mul(&frame).origin) else { return };
        let point = |o: [f32; 3]| [n[0] + o[0] * cy - o[1] * sy, n[1] + o[0] * sy + o[1] * cy, n[2] + o[2]];
        self.coop_left_arm_reach(pawn, crate::coop_left_arm::Reach { hand: point(t.hand), elbow: point(t.elbow), hand_axes: None, fist: true });
    }

}

/// The world → mesh frame of an actor with the mesh's Origin (skel.rs actor_coords).
fn with_origin(info: &crate::anim::MeshInfo, mut c: openxiii_assets::coords::Coords) -> openxiii_assets::coords::Coords {
    c.origin = [info.origin[0] + c.origin[0], info.origin[1] + c.origin[1], info.origin[2] + c.origin[2]];
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_turns_one_vector_onto_the_other() {
        let (a, d) = (unit([0.1, -0.08, 0.99]), unit([-0.06, 0.0, 1.0]));
        let q = arc(a, d);
        let got = [0, 1, 2].map(|i| q[i][0] * a[0] + q[i][1] * a[1] + q[i][2] * a[2]);
        for i in 0..3 {
            assert!((got[i] - d[i]).abs() < 1e-5, "{got:?} {d:?}");
        }
        // A turn: a vector across both keeps its length.
        let v = [1.0, 0.0, 0.0];
        let t = [0, 1, 2].map(|i| q[i][0] * v[0] + q[i][1] * v[1] + q[i][2] * v[2]);
        assert!(((t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt() - 1.0).abs() < 1e-5);
        // Already there: nothing turns.
        assert_eq!(arc(d, d), IDENTITY);
    }

    #[test]
    fn upright_axes_give_back_their_rotator() {
        // What coop_hostage_fit hands to ortho_rotation: forward, right, up of a plain yaw.
        let (s, c) = (0.5f32.dsin(), 0.5f32.dcos());
        let rot = crate::world::ortho_rotation([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(rot[0], 0);
        assert_eq!(rot[2], 0);
        assert!((rot[1] - (0.5 * 32768.0 / std::f32::consts::PI) as i32).abs() <= 1);
    }
}
