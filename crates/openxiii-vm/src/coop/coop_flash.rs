//! Local co-op campaign, flashbacks (see coop.rs, coop_story.rs).
//!
//! A flashback is run by GenFlash (XIDCine) for MapInfo's player only:
//! MapInfo.SaveInventoryForFlash hides the real pawn where it stands and makes the controller
//! possess a new XIIIPlayerPawn with the default inventory (MapInfo.FakePawn); the flashes are
//! given to that controller (ConstantGlow, FilterColorWanted, DesiredFlashScale) and it is made
//! invulnerable; each jump into the memory puts the new pawn at a PositionInfo; at the end
//! RestoreInventoryAfterFlash gives the real pawn back where it was left.
//!
//! In co-op every player goes into the memory, as the story player does:
//! - when MapInfo.FakePawn appears, the other players' pawns are hidden in the same way and
//!   each player gets a new pawn of their own (default inventory, made invulnerable);
//! - the story player's flashes are copied on the other players' controllers, and the other
//!   players are held while the story player is (state NoControl at the start and at the end);
//! - at each jump of the memory (GenFlash.IndexCurrentFlash grows) the others are put next to
//!   the story player, facing the same way, whatever the length of the jump;
//! - when MapInfo.FakePawn is gone, each player gets their real pawn back where they left it.
//!
//! The flashback's player is GenFlash's own (GenFlash.PC: MapInfo's player when it started, the
//! player who caused it, coop_world.rs), not always the first one. While the flashback holds
//! that player (state NoControl or NoMove: the flashes, a scene on a bed or a table), the
//! cinematic rule applies (coop_cine.rs): the others are parked and see that player's picture.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Fields of a player controller that make the flashes of a flashback.
const FLASH_FIELDS: [&str; 6] = ["ConstantGlowScale", "ConstantGlowFog", "DesiredFlashScale", "DesiredFlashFog", "FilterColorWanted", "FilterColorSpeed"];
/// EDrawType.
const DT_NONE: u8 = 0;
const DT_MESH: u8 = 2;

/// A player other than the story player during a flashback.
#[derive(Debug, Clone)]
pub(crate) struct FlashPlayer {
    pc: ObjId,
    real: ObjId,
    fake: Option<ObjId>,
    god: Value,
    fov: (Value, Value),
}

/// Flashback state of a co-op game.
#[derive(Debug, Default, Clone)]
pub(crate) struct CoopFlash {
    /// A flashback is running (MapInfo.FakePawn seen).
    pub(crate) active: bool,
    pub(crate) players: Vec<FlashPlayer>,
    /// GenFlash.IndexCurrentFlash on the previous frame.
    pub(crate) index: i32,
    /// Controllers put in state NoControl along with the story player.
    pub(crate) held: Vec<ObjId>,
    /// Rank of the flashback's player.
    pub(crate) leader: usize,
    /// Rank of the player who last walked in the memory (the flashback's player at its start).
    pub(crate) last_mover: usize,
    /// Each player's place on the previous frame (for `last_mover`).
    pub(crate) places: Vec<Option<[f32; 3]>>,
}

/// Longest move in one frame still counted as walking (not a jump of the memory or a gather).
const WALK_STEP: f32 = 100.0;

impl Vm {
    /// Once per level tick (coop_story_frame). `true`: a flashback runs or has just ended (its
    /// jumps are followed here, not as teleports).
    pub(crate) fn coop_flash_frame(&mut self) -> bool {
        let Some(map_info) = self.coop_story_map_info() else { return false };
        if self.coop.players.len() < 2 {
            return false;
        }
        let fake = self.prop(map_info, "FakePawn").as_object().filter(|&p| self.coop_object_alive(p));
        let manager = self.level.and_then(|l| self.prop(l, "FlashManager").as_object()).filter(|&m| self.coop_object_alive(m));
        let leader = self.coop_flash_leader();
        let first = self.coop.players[leader];
        if !self.coop_object_alive(first) {
            return false;
        }
        let mut moved = false;
        if fake.is_some() && !self.coop.flash.active {
            self.coop.flash.active = true;
            self.coop.flash.leader = leader;
            self.coop.flash.last_mover = leader;
            self.coop.flash.places.clear();
            self.coop.flash.index = manager.map_or(0, |m| self.prop(m, "IndexCurrentFlash").as_int());
            for i in (0..self.coop.players.len()).filter(|&i| i != leader) {
                if let Some(pawn) = self.coop_living_pawn(i) {
                    let pc = self.coop.players[i];
                    self.coop_flash_enter(map_info, pc, pawn);
                }
            }
            moved = true;
        } else if fake.is_none() && self.coop.flash.active {
            self.coop.flash.active = false;
            for p in std::mem::take(&mut self.coop.flash.players) {
                self.coop_flash_leave(p);
            }
            // Back from the memory: the others next to the flashback's player, not where they
            // were left (SPADS02a: a player left in the water behind a hatch that closed).
            let leader = self.coop.flash.leader;
            let _ = self.coop_regroup(leader);
            moved = true;
        } else if self.coop.flash.active {
            // A jump of the memory.
            let index = manager.map_or(0, |m| self.prop(m, "IndexCurrentFlash").as_int());
            if index != self.coop.flash.index {
                self.coop.flash.index = index;
                self.coop_flash_gather();
                moved = true;
            }
        }
        if self.coop.flash.active {
            self.coop_flash_track_movers();
        }
        // The flashes and the hold of the story player, on the others.
        let effects = manager.is_some_and(|m| self.current_state(m).is_some());
        if effects || self.coop.flash.active {
            let values: Vec<Value> = FLASH_FIELDS.iter().map(|f| self.prop(first, f)).collect();
            for pc in self.coop.players.clone() {
                if pc == first || !self.coop_object_alive(pc) {
                    continue;
                }
                for (f, v) in FLASH_FIELDS.iter().zip(&values) {
                    self.set_prop(pc, f, v.clone());
                }
            }
        }
        // While the flashback holds its player, the others are parked as in a cinematic
        // (coop_story.rs, coop_cine.rs); GenFlash shows no comic frame with its flashes.
        if self.coop.flash.active && self.coop_held_state(first) {
            if let Some(level) = self.level {
                self.set_prop(level, "bCineFrame", Value::Bool(false));
            }
        }
        // Earlier holds of this file (a game of an older build): let go.
        for pc in std::mem::take(&mut self.coop.flash.held) {
            if self.coop_object_alive(pc) && !self.coop_cine_is_parked(pc) && self.current_state(pc).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("NoControl")) {
                let _ = self.call_named(pc, "GotoState", vec![Value::Name("PlayerWalking".into())]);
            }
        }
        moved || self.coop.flash.active
    }

    /// MapInfo.SaveInventoryForFlash for another player.
    fn coop_flash_enter(&mut self, map_info: ObjId, pc: ObjId, real: ObjId) {
        let Some(game) = self.level.and_then(|l| self.prop(l, "Game").as_object()) else { return };
        let mut entry = FlashPlayer { pc, real, fake: None, god: self.prop(pc, "bGodMode"), fov: (self.prop(pc, "FOVAngle"), self.prop(pc, "DesiredFOV")) };
        let _ = self.call_named(real, "SetCollision", vec![Value::Bool(true), Value::Bool(false), Value::Bool(false)]);
        let _ = self.call_named(real, "SetDrawType", vec![Value::Byte(DT_NONE)]);
        self.set_prop(real, "bIsInCine", Value::Bool(true));
        let class = self.prop(map_info, "FakePawn").as_object().and_then(|f| self.objects[f].as_ref().map(|o| o.class));
        let rotation = self.prop(real, "Rotation");
        let mut places = vec![self.prop(real, "Location").as_vector()];
        places.extend(self.coop_places_near(real));
        let mut fake = None;
        if let Some(class) = class {
            for place in places {
                let mut args = [Value::Class(Some(class)), Value::Object(None), Value::Name(String::new().into()), Value::vector(place[0], place[1], place[2]), rotation.clone()];
                if let Some(p) = crate::natives::call(self, "Spawn", game, &mut args, &[true, false, false, true, true]).ok().and_then(|v| v.as_object()) {
                    fake = Some(p);
                    break;
                }
            }
        }
        let Some(fake) = fake else {
            // No room: this player stays as they are.
            self.log.push("Log: Co-op: no flashback pawn for a player".into());
            let _ = self.call_named(real, "SetCollision", vec![Value::Bool(true), Value::Bool(true), Value::Bool(true)]);
            let _ = self.call_named(real, "SetDrawType", vec![Value::Byte(DT_MESH)]);
            self.set_prop(real, "bIsInCine", Value::Bool(false));
            return;
        };
        let _ = self.call_named(game, "AddDefaultInventory", vec![Value::Object(Some(fake))]);
        let _ = self.call_named(fake, "SetRotation", vec![rotation.clone()]);
        if let Some(w) = self.prop(real, "Weapon").as_object().filter(|&w| self.coop_object_alive(w)) {
            let _ = self.call_named(w, "UnFire", vec![Value::Float(0.0)]);
        }
        let _ = self.call_named(pc, "UnPossess", vec![]);
        let _ = self.call_named(pc, "Possess", vec![Value::Object(Some(fake))]);
        let _ = self.call_named(pc, "SetViewTarget", vec![Value::Object(Some(fake))]);
        let default_fov = self.prop(pc, "DefaultFOV");
        self.set_prop(pc, "FOVAngle", default_fov.clone());
        self.set_prop(pc, "DesiredFOV", default_fov);
        self.set_prop(pc, "bGodMode", Value::Bool(true));
        // Its own sounds heard without position, as for a viewport's pawn (coop_respawn).
        if self.coop_is_local(pc) {
            for flag in ["bHasRollOff", "bHasPosition"] {
                self.set_prop(fake, flag, Value::Bool(false));
            }
        }
        let _ = self.call_named(pc, "ClientSetRotation", vec![rotation]);
        self.coop_flash_soldier_list(game, fake, true);
        entry.fake = Some(fake);
        self.coop.flash.players.push(entry);
        self.log.push("Log: Co-op: player enters the flashback".into());
    }

    /// MapInfo.RestoreInventoryAfterFlash for another player.
    fn coop_flash_leave(&mut self, p: FlashPlayer) {
        let game = self.level.and_then(|l| self.prop(l, "Game").as_object());
        if let (Some(game), Some(fake)) = (game, p.fake) {
            self.coop_flash_soldier_list(game, fake, false);
        }
        if self.coop_object_alive(p.pc) && self.coop_object_alive(p.real) {
            let _ = self.call_named(p.pc, "UnPossess", vec![]);
            let _ = self.call_named(p.pc, "Possess", vec![Value::Object(Some(p.real))]);
            let _ = self.call_named(p.pc, "SetViewTarget", vec![Value::Object(Some(p.real))]);
            self.set_prop(p.pc, "FOVAngle", p.fov.0);
            self.set_prop(p.pc, "DesiredFOV", p.fov.1);
            self.set_prop(p.pc, "bGodMode", p.god);
            let _ = self.call_named(p.real, "SetCollision", vec![Value::Bool(true), Value::Bool(true), Value::Bool(true)]);
            let _ = self.call_named(p.real, "SetDrawType", vec![Value::Byte(DT_MESH)]);
            self.set_prop(p.real, "bIsInCine", Value::Bool(false));
            self.set_prop(p.real, "Velocity", Value::vector(0.0, 0.0, 0.0));
            if self.coop_is_local(p.pc) {
                for flag in ["bHasRollOff", "bHasPosition"] {
                    self.set_prop(p.real, flag, Value::Bool(false));
                }
            }
            // Weapon or item back in hand.
            if self.prop(p.pc, "bWeaponMode").as_bool() {
                if let Some(w) = self.prop(p.real, "Weapon").as_object().filter(|&w| self.coop_object_alive(w)) {
                    if self.call_named(w, "HasAmmo", vec![]).is_ok_and(|v| v.as_bool()) {
                        let _ = self.call_named(w, "GotoState", vec![Value::Name("Idle".into())]);
                    } else {
                        let _ = self.call_named(p.pc, "NextWeapon", vec![]);
                    }
                }
            } else if let Some(item) = self.prop(p.real, "SelectedItem").as_object().filter(|&i| self.coop_object_alive(i)) {
                let _ = self.call_named(item, "GotoState", vec![Value::Name("Idle".into())]);
            } else {
                let _ = self.call_named(p.pc, "NextWeapon", vec![]);
                let _ = self.call_named(p.real, "ChangedWeapon", vec![]);
            }
        }
        if let Some(fake) = p.fake.filter(|&f| self.coop_object_alive(f)) {
            let _ = self.call_named(fake, "Destroy", vec![]);
        }
        self.log.push("Log: Co-op: player back from the flashback".into());
    }

    /// Rank of the flashback's player: GenFlash's own once it has one, MapInfo's otherwise.
    pub(crate) fn coop_flash_leader(&self) -> usize {
        let manager = self.level.and_then(|l| self.prop(l, "FlashManager").as_object()).filter(|&m| self.coop_object_alive(m));
        let pc = manager.and_then(|m| self.prop(m, "PC").as_object()).or_else(|| self.coop_story_map_info().and_then(|m| self.prop(m, "XIIIController").as_object()));
        pc.and_then(|pc| self.coop.players.iter().position(|&p| p == pc)).unwrap_or(0)
    }

    /// The flashback holds its player (a non-playable part of it). Returns the player whose
    /// picture is shown: the one who last walked in the memory (SPADS02a: the first player
    /// started the flashback, the second one played it, its end is shown from the second).
    pub(crate) fn coop_flash_holds(&self) -> Option<usize> {
        if !self.coop.flash.active {
            return None;
        }
        let leader = self.coop.flash.leader;
        let pc = *self.coop.players.get(leader)?;
        // Not when it is held only as a copy of another player's scene (coop_rules.rs): that
        // scene is the other player's, shown from their camera (Submarine 1: the second player
        // walks into Sheridan's office in the memory of a flashback of the first one).
        let copy = self.coop_held_copy(pc);
        if !(self.coop_object_alive(pc) && self.coop_held_state(pc) && !copy) {
            return None;
        }
        let shown = self.coop.flash.last_mover;
        if shown == leader || self.coop_living_pawn(shown).is_none() {
            return Some(leader);
        }
        Some(shown)
    }

    /// The player who last walked in the memory: a move of one frame across the ground, with
    /// control.
    pub(crate) fn coop_flash_track_movers(&mut self) {
        let places: Vec<Option<[f32; 3]>> = (0..self.coop.players.len()).map(|i| self.coop_living_pawn(i).map(|p| self.prop(p, "Location").as_vector())).collect();
        if self.coop.flash.places.len() == places.len() {
            for (i, (now, before)) in places.iter().zip(&self.coop.flash.places).enumerate() {
                let (Some(p), Some(q)) = (now, before) else { continue };
                // Across the ground only: a pawn landing after a jump of the memory does not walk.
                let step = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                let pc = self.coop.players[i];
                // Driven by the player (the controller's acceleration), not put aside by the
                // co-op (a pawn moved off another one at the end of a cinematic, a gather).
                let walking = self.coop_living_pawn(i).is_some_and(|pawn| {
                    let a = self.prop(pawn, "Acceleration").as_vector();
                    a[0].dhypot(a[1]) > 1.0
                });
                if walking && step > 1.0 && step < WALK_STEP && !self.coop_held_state(pc) && !self.coop_cine_is_parked(pc) {
                    self.coop.flash.last_mover = i;
                }
            }
        }
        self.coop.flash.places = places;
    }

    /// The other players next to the flashback's player, facing the same way.
    fn coop_flash_gather(&mut self) {
        let leader = self.coop.flash.leader;
        let Some(story) = self.coop_living_pawn(leader) else { return };
        let rotation = self.prop(self.coop.players[leader], "Rotation");
        for i in (0..self.coop.players.len()).filter(|&i| i != leader) {
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            // Parked while the flashback holds its player: put next to it when that ends.
            if self.coop_cine_is_parked(self.coop.players[i]) {
                continue;
            }
            self.coop_neutral(i);
            for place in self.coop_places_near(story) {
                if self.call_named(pawn, "SetLocation", vec![Value::vector(place[0], place[1], place[2])]).is_ok_and(|v| v.as_bool()) {
                    self.set_prop(pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
                    let pc = self.coop.players[i];
                    let _ = self.call_named(pawn, "SetRotation", vec![rotation.clone()]);
                    let _ = self.call_named(pc, "ClientSetRotation", vec![rotation.clone()]);
                    self.log.push(format!("Log: Co-op: player {} follows the flashback", i + 1));
                    break;
                }
            }
        }
    }

    /// The new pawn in (or out of) GameInfo.BaseSoldierList, as SaveInventoryForFlash does.
    fn coop_flash_soldier_list(&mut self, game: ObjId, pawn: ObjId, add: bool) {
        let Value::Array(mut list) = self.prop(game, "BaseSoldierList") else { return };
        if add {
            list.push(Value::Object(Some(pawn)));
        } else if let Some(k) = list.iter().position(|v| v.as_object() == Some(pawn)) {
            list.remove(k);
        } else {
            return;
        }
        self.set_prop(game, "BaseSoldierList", Value::Array(list));
    }
}
