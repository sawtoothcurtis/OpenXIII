//! Improved build only (co-op): a corpse carried by a co-op player is seen by the other players.
//!
//! In the game, picking up a corpse (XIIILeftHand.GrabPawn) makes the corpse itself invisible
//! (XIIIPawn state Grabbed: DrawType DT_None) and shows it only in the carrier's first-person
//! view: a copy of the body (HeldBody, the corpse's own mesh and skin, sequence CadavreWait) on
//! the bone Box01 of the left hand model fpsCadavreM. The game has no third-person view of it
//! (the solo game has nobody else to see it), so in co-op the other player saw the carrier's
//! hands empty and the corpse gone.
//!
//! Here, while a co-op player carries a corpse, a second copy of the body is put on that
//! player's shoulder, made the way the game makes the first-person one: a HeldBody with the
//! corpse's mesh, skin and size, playing the same sequence CadavreWait (the body lying over the
//! shoulder), placed where the carrier sees the first-person one, at full size (see
//! coop_carry_spot). The carrier does not see it (bOwnerNoSee): their own view keeps the game's
//! first-person hand. It is removed when the corpse is put down (the hand leaves Holding).
//!
//! Network co-op: the host makes it (a HeldBody is replicated, ROLE_SimulatedProxy, with its
//! mesh, size and instigator); the engine replicates neither the skin nor the animation, so the
//! guest's game gives it the skin of the hidden corpse, plays the sequence and places it itself.
//!
//! Outside a co-op session nothing here runs: the solo game is the original one.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::HashMap;

use crate::value::{ObjId, Value};
use crate::Vm;

/// DrawType DT_Mesh.
const DT_MESH: u8 = 2;
/// DrawType DT_None.
const DT_NONE: u8 = 0;

/// Where the body sits: from the carrier's eye, forward, right, up (see coop_carry_spot). The
/// first-person place brought to full size is (21.1, 9.8, 58.8); seen from outside the body then
/// floats beside the head (the first-person view hides it), so it is brought 10 units in and 10
/// down, onto the shoulder (captures from the front, the side and the back).
const PLACE: [f32; 3] = [21.1, 20.0, 49.0];

/// The sequences the game plays on the first-person copy (XIIILeftHand PlayGrabbing,
/// PlayWaiting, PlayUnGrabbing): picked up, held, put down.
const SEQ_UP: &str = "CadavreSelect";
const SEQUENCE: &str = "CadavreWait";
const SEQ_DOWN: &str = "CadavreDown";

/// Mark of a copy being put down, for the network guest: its DrawScale grows by this (the
/// engine replicates DrawScale exactly, vectors only as whole numbers, animations not at all).
const DROP_MARK: f32 = 0.002;

/// Where a copy is in its sequences, on this machine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Up,
    Hold,
    Down,
}

#[derive(Default)]
pub struct State {
    /// Carrier pawn → (shoulder copy, corpse).
    shown: HashMap<ObjId, (ObjId, ObjId)>,
    /// Copies already set up on this machine (skin): the guest's own set-up.
    ready: std::collections::HashSet<ObjId>,
    /// Sequence of each copy on this machine.
    phase: HashMap<ObjId, Phase>,
    /// DrawScale of each copy when first seen on this machine (see DROP_MARK).
    first_scale: HashMap<ObjId, f32>,
    /// Copies marked as being put down (host).
    dropping: std::collections::HashSet<ObjId>,
}

impl Vm {
    /// Once per level tick in a co-op game.
    pub(crate) fn coop_carry_frame(&mut self) {
        if !self.coop.on {
            if !self.improved.coop_carry.shown.is_empty() {
                let shown: Vec<ObjId> = self.improved.coop_carry.shown.drain().map(|(_, (c, _))| c).collect();
                for c in shown {
                    self.coop_carry_remove(c);
                }
            }
            self.improved.coop_carry.ready.clear();
            self.improved.coop_carry.phase.clear();
            self.improved.coop_carry.first_scale.clear();
            self.improved.coop_carry.dropping.clear();
            return;
        }
        self.coop_carry_authority();
        self.coop_carry_local();
    }

    /// The host (or the split-screen game): one copy per carrier of a corpse.
    fn coop_carry_authority(&mut self) {
        let pawns: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.alive(a) && self.is_a(a, "XIIIPlayerPawn")).collect();
        let mut carried: HashMap<ObjId, (ObjId, bool)> = HashMap::new();
        for p in pawns {
            if self.prop(p, "bDeleteMe").as_bool() {
                continue;
            }
            let Some(hand) = self.prop(p, "LHand").as_object().filter(|&h| self.alive(h)) else { continue };
            if !self.prop(hand, "bActive").as_bool() {
                continue;
            }
            let Some(corpse) = self.prop(hand, "pOnShoulder").as_object().filter(|&c| self.alive(c)) else { continue };
            if !self.prop(corpse, "bIsDead").as_bool() {
                continue;
            }
            // Grabbing, Holding, then UnGrabbing (putting it down, until the hand's AnimEnd puts
            // the corpse back on the ground and empties the hand).
            let ungrabbing = self.objects[hand].as_ref().and_then(|o| o.state.as_ref()).is_some_and(|s| s.to_string().eq_ignore_ascii_case("UnGrabbing"));
            carried.insert(p, (corpse, ungrabbing));
        }
        let old: Vec<(ObjId, (ObjId, ObjId))> = self.improved.coop_carry.shown.iter().map(|(&p, &v)| (p, v)).collect();
        for (p, (copy, corpse)) in old {
            if carried.get(&p).map(|c| c.0) != Some(corpse) || !self.alive(copy) {
                self.improved.coop_carry.shown.remove(&p);
                self.coop_carry_remove(copy);
            }
        }
        for (p, (corpse, ungrabbing)) in carried {
            if let Some(&(copy, _)) = self.improved.coop_carry.shown.get(&p) {
                if ungrabbing && copy != corpse && !self.improved.coop_carry.dropping.contains(&copy) {
                    self.improved.coop_carry.dropping.insert(copy);
                    let s = self.prop(copy, "DrawScale").as_float();
                    let _ = self.call_named(copy, "SetDrawScale", vec![Value::Float(s + DROP_MARK)]);
                    // Sent at the next network update (Actor replication: DrawScale if bNetDirty).
                    self.set_prop(copy, "bNetDirty", Value::Bool(true));
                }
                continue;
            }
            if ungrabbing {
                continue;
            }
            match self.coop_carry_make(p, corpse) {
                Some(copy) => {
                    self.improved.coop_carry.shown.insert(p, (copy, corpse));
                }
                None => {
                    // Not made: tried again next frame would spam; mark with the corpse itself.
                    self.improved.coop_carry.shown.insert(p, (corpse, corpse));
                }
            }
        }
    }

    fn coop_carry_remove(&mut self, copy: ObjId) {
        self.improved.coop_carry.ready.remove(&copy);
        self.improved.coop_carry.phase.remove(&copy);
        self.improved.coop_carry.first_scale.remove(&copy);
        self.improved.coop_carry.dropping.remove(&copy);
        if self.alive(copy) && self.is_a(copy, "HeldBody") {
            let _ = self.call_named(copy, "Destroy", vec![]);
        }
    }

    /// The copy of `corpse` on the shoulder of `carrier`.
    fn coop_carry_make(&mut self, carrier: ObjId, corpse: ObjId) -> Option<ObjId> {
        let class = self.class("XIII.HeldBody")?;
        let (l, r) = coop_carry_spot(self, carrier);
        let copy = self
            .spawn_actor(
                class,
                Value::Object(Some(carrier)),
                None,
                Value::vector(l[0], l[1], l[2]),
                Value::rotator(r[0], r[1], r[2]),
                Value::Object(Some(carrier)),
                true,
            )
            .ok()
            .flatten()?;
        let _ = self.call_named(copy, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
        let mesh = self.native_prop(corpse, crate::NativeField::Mesh).clone();
        self.set_prop(copy, "Mesh", mesh);
        let skins = self.prop(corpse, "Skins");
        self.set_prop(copy, "Skins", skins);
        let scale = self.prop(corpse, "DrawScale").as_float().max(MIN_SCALE);
        let _ = self.call_named(copy, "SetDrawScale", vec![Value::Float(scale)]);
        self.set_prop(copy, "DrawType", Value::Byte(DT_MESH));
        self.set_prop(copy, "bHidden", Value::Bool(false));
        self.set_prop(copy, "bOwnerNoSee", Value::Bool(true));
        // Each machine places it itself every frame (coop_carry_local): only its first place goes
        // over the network.
        self.set_prop(copy, "bReplicateMovement", Value::Bool(false));
        // Kept on the guest wherever its place is (above the eye, maybe in a ceiling).
        self.set_prop(copy, "bAlwaysRelevant", Value::Bool(true));
        // In the level's list of drawn actors (spawned hidden, as the hand's own copy is shown:
        // XIIILeftHand state Grabbing, bHidden then RefreshDisplaying).
        let _ = self.call_named(copy, "RefreshDisplaying", vec![]);
        self.improved.coop_carry.ready.insert(copy);
        self.log.push(format!("Log: Co-op: {} carries {}", self.obj_label(carrier), self.obj_label(corpse)));
        Some(copy)
    }

    /// Every machine: each copy follows its carrier, is not seen by him, and on the guest (where
    /// it arrives by the network without skin nor animation) takes the hidden corpse's skin and
    /// plays the sequence.
    fn coop_carry_local(&mut self) {
        // The copies: HeldBody of a player's full size (the first-person one is at DrawScale 0.3
        // on the hand).
        let copies: Vec<(ObjId, ObjId)> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.alive(a) && self.is_a(a, "HeldBody") && self.prop(a, "DrawScale").as_float() >= MIN_SCALE)
            .filter(|&a| !self.prop(a, "Base").as_object().is_some_and(|b| self.alive(b) && self.is_a(b, "XIIILeftHand")))
            .filter_map(|a| {
                let carrier = self.prop(a, "Instigator").as_object().filter(|&p| self.alive(p) && self.is_a(p, "XIIIPlayerPawn"))?;
                Some((a, carrier))
            })
            .collect();
        self.improved.coop_carry.ready.retain(|c| copies.iter().any(|(a, _)| a == c));
        self.improved.coop_carry.phase.retain(|c, _| copies.iter().any(|(a, _)| a == c));
        self.improved.coop_carry.first_scale.retain(|c, _| copies.iter().any(|(a, _)| a == c));
        for (copy, carrier) in copies {
            if self.prop(copy, "Owner").as_object() != Some(carrier) {
                self.set_prop(copy, "Owner", Value::Object(Some(carrier)));
            }
            if !self.prop(copy, "bOwnerNoSee").as_bool() {
                self.set_prop(copy, "bOwnerNoSee", Value::Bool(true));
            }
            let (l, r) = coop_carry_spot(self, carrier);
            let _ = self.call_named(copy, "SetLocation", vec![Value::vector(l[0], l[1], l[2])]);
            let _ = self.call_named(copy, "SetRotation", vec![Value::rotator(r[0], r[1], r[2])]);
            if self.improved.coop_carry.ready.contains(&copy) {
                self.coop_carry_animate(copy);
                continue;
            }
            if self.native_prop(copy, crate::NativeField::DrawType).as_byte() != DT_MESH {
                self.set_prop(copy, "DrawType", Value::Byte(DT_MESH));
            }
            if self.prop(copy, "bHidden").as_bool() {
                self.set_prop(copy, "bHidden", Value::Bool(false));
            }
            // The hidden corpse with the same mesh, nearest to the carrier.
            let mesh = self.native_prop(copy, crate::NativeField::Mesh).clone();
            let here = self.prop(carrier, "Location").as_vector();
            let corpse = self
                .actors
                .iter()
                .copied()
                .filter(|&a| self.alive(a) && a != carrier && self.is_a(a, "XIIIPawn") && self.prop(a, "bIsDead").as_bool())
                .filter(|&a| self.native_prop(a, crate::NativeField::DrawType).as_byte() == DT_NONE)
                .filter(|&a| *self.native_prop(a, crate::NativeField::Mesh) == mesh)
                .map(|a| {
                    let l = self.prop(a, "Location").as_vector();
                    (a, (0..3).map(|k| (l[k] - here[k]).powi(2)).sum::<f32>())
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(a, _)| a);
            if let Some(c) = corpse {
                let skins = self.prop(c, "Skins");
                self.set_prop(copy, "Skins", skins);
            }
            let _ = self.call_named(copy, "RefreshDisplaying", vec![]);
            self.improved.coop_carry.ready.insert(copy);
        }
    }

    /// The copy's sequences, as the game plays them on the first-person one: picked up
    /// (CadavreSelect, once), held (CadavreWait, looped), put down (CadavreDown, once, until the
    /// corpse is back on the ground).
    fn coop_carry_animate(&mut self, copy: ObjId) {
        let scale = self.prop(copy, "DrawScale").as_float();
        let first = *self.improved.coop_carry.first_scale.entry(copy).or_insert(scale);
        let dropping = scale > first + DROP_MARK / 2.0;
        let next = match (self.improved.coop_carry.phase.get(&copy).copied(), dropping) {
            (Some(Phase::Down), _) => return,
            (_, true) => {
                self.play_anim(copy, 0, SEQ_DOWN, 1.0, 0.0, false);
                Phase::Down
            }
            (None, false) => {
                self.play_anim(copy, 0, SEQ_UP, 1.0, 0.0, false);
                Phase::Up
            }
            (Some(Phase::Up), false) if !self.is_animating(copy, 0) => {
                self.play_anim(copy, 0, SEQUENCE, 1.0, 0.0, true);
                Phase::Hold
            }
            (Some(p), false) => p,
        };
        self.improved.coop_carry.phase.insert(copy, next);
    }
}

/// Smallest DrawScale of a shoulder copy (above the first-person one's 0.3).
const MIN_SCALE: f32 = 0.5;

/// Place of the copy for `carrier`: the place the game gives the first-person copy, seen from
/// the carrier's eye, brought to full size (the first-person copy is at DrawScale 0.3: the same
/// body three times nearer looks the same), looking straight ahead (the body stays on the
/// shoulder when the carrier looks up or down).
///
/// Measured in the game's first-person view (XIIILeftHand Holding, sequence CadavreWait, view
/// pitch 0): HeldBody at (6.33, 2.93, 17.65) from the eye in the view's frame (forward, right,
/// up), turned like the view; divided by 0.3 (then moved onto the shoulder, see PLACE).
fn coop_carry_spot(vm: &Vm, carrier: ObjId) -> ([f32; 3], [i32; 3]) {
    let (rel, yaw_offset) = coop_carry_place();
    let loc = vm.prop(carrier, "Location").as_vector();
    let eye = match vm.prop(carrier, "EyeHeight").as_float() {
        e if e > 1.0 => e,
        _ => vm.prop(carrier, "BaseEyeHeight").as_float(),
    };
    let yaw = vm.prop(carrier, "Rotation").as_rotator()[1];
    let a = yaw as f32 * std::f32::consts::TAU / 65536.0;
    let (s, c) = a.dsin_cos();
    let at = [loc[0] + rel[0] * c - rel[1] * s, loc[1] + rel[0] * s + rel[1] * c, loc[2] + eye + rel[2]];
    (at, [0, (yaw + yaw_offset) & 0xFFFF, 0])
}

fn coop_carry_place() -> ([f32; 3], i32) {
    (PLACE, 0)
}
