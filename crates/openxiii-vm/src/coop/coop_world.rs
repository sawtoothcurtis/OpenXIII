//! Local co-op campaign, world part (see coop.rs): the map's scripts and the fixed guns.
//!
//! - Many scripts of the campaign ask MapInfo for "the player" (XIIIPawn, XIIIController): a
//!   clue or a magnetic pass checked against that pawn, a breakable door measured from it, a
//!   grapple lesson given to that controller. While a script handles an event caused by another
//!   player (that player touched, triggered, bumped or used something), MapInfo answers with
//!   that player; afterwards the story player is back.
//! - State PlayerGunning hides the gunner's pawn for everybody (bHidden) because the view is
//!   the gun's, not the pawn's. In co-op the pawn is hidden in the gunner's own view only.
//! - Some effects of a place are given by its scripts to "the player" only, while they belong
//!   to everybody there: the colour of an AlarmFilter (SSH102b: the red alarm of the
//!   self-destruct) is copied to every player; the dazzle and the shake of a BreakableMover are
//!   given to each player by that player's own distance, as the script does for the story player.
//!   Effects of the story player's own sequences (BeachInBedWithXIII, BeachFinalFall,
//!   BeachBoatRock, Telepherique) stay theirs; GenFlash flashbacks are shared (coop_flash.rs).

use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// MapInfo's player before an event caused by another player.
pub(crate) struct Acting {
    map_info: ObjId,
    pawn: Value,
    controller: Value,
    set_pawn: ObjId,
    set_controller: ObjId,
}

/// A place effect script running (coop_place_effect_begin).
pub(crate) enum PlaceEffect {
    /// AlarmFilter.TurnFilterOff / UseCurrentColor: the colour it gives its PC.
    Filter,
    /// BreakableMover.InitDazzleAndShake(bBroken), MapInfo's player when it was called.
    Dazzle { broken: bool, reference: Option<ObjId> },
}

/// Dazzle of a broken mover on a player other than the script's (BreakableMover.Tick:
/// ClientInstantFlash(Lerp(time, -intensity, 0), FD_VectorColor) while time is within [0, 1]).
#[derive(Debug, Clone)]
pub(crate) struct Dazzle {
    pc: ObjId,
    start: f64,
    lifetime: f32,
    intensity: f32,
    color: [f32; 3],
}

/// Players farther apart than this are in different places of the map.
const APART: f32 = 2000.0;

/// Events an actor receives because of another actor.
const EVENTS: [&str; 7] = ["Touch", "UnTouch", "Trigger", "UnTrigger", "Bump", "UsedBy", "Used"];

impl Vm {
    fn coop_map_info(&self) -> Option<ObjId> {
        let game = self.level.and_then(|l| self.prop(l, "Game").as_object())?;
        self.prop(game, "MapInfo").as_object().filter(|&m| self.objects.get(m).is_some_and(|o| o.is_some()))
    }

    /// Start of a script event: when a player other than the story player caused it, MapInfo's
    /// player becomes that one. Returns what to put back.
    pub(crate) fn coop_acting_begin(&mut self, f: FuncId, args: &[Option<Value>]) -> Option<Acting> {
        if self.coop.acting || self.coop.players.len() < 2 {
            return None;
        }
        let name = self.functions[f].name.as_str();
        if !EVENTS.iter().any(|e| name.eq_ignore_ascii_case(e)) {
            return None;
        }
        // The last parameter that is a player (Trigger: Other, then EventInstigator).
        let mut found = None;
        for object in args.iter().filter_map(|a| a.as_ref()).filter_map(Value::as_object) {
            for (i, &pc) in self.coop.players.iter().enumerate() {
                if self.objects.get(pc).is_none_or(|o| o.is_none()) {
                    continue;
                }
                let pawn = self.prop(pc, "Pawn").as_object();
                if object == pc || pawn == Some(object) {
                    found = pawn.map(|pawn| (i, pc, pawn));
                }
            }
        }
        // An event a scripted scene causes (CineController2.TriggerEvent: Other = the scene's
        // controller) acts with the scene's player (coop_rules.rs).
        if found.is_none() {
            for object in args.iter().filter_map(|a| a.as_ref()).filter_map(Value::as_object) {
                if self.objects.get(object).is_some_and(|o| o.is_some()) && self.is_a(object, "CineController2") {
                    if let Some(pc) = self.prop(object, "PC").as_object() {
                        if let Some(i) = self.coop.players.iter().position(|&p| p == pc) {
                            found = self.prop(pc, "Pawn").as_object().map(|pawn| (i, pc, pawn));
                        }
                    }
                    break;
                }
            }
        }
        let (i, pc, pawn) = found?;
        if i == 0 || self.prop(pawn, "Health").as_int() <= 0 {
            return None;
        }
        let map_info = self.coop_map_info()?;
        let saved =
            Acting { map_info, pawn: self.prop(map_info, "XIIIPawn"), controller: self.prop(map_info, "XIIIController"), set_pawn: pawn, set_controller: pc };
        self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
        self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
        self.coop.acting = true;
        Some(saved)
    }

    /// End of that event: the story player again, unless the script itself changed MapInfo's
    /// player meanwhile.
    pub(crate) fn coop_acting_end(&mut self, saved: Acting) {
        self.coop.acting = false;
        if self.objects.get(saved.map_info).is_none_or(|o| o.is_none()) {
            return;
        }
        if self.prop(saved.map_info, "XIIIPawn").as_object() == Some(saved.set_pawn) {
            self.set_prop(saved.map_info, "XIIIPawn", saved.pawn);
        }
        if self.prop(saved.map_info, "XIIIController").as_object() == Some(saved.set_controller) {
            self.set_prop(saved.map_info, "XIIIController", saved.controller);
        }
    }

    /// Force-feedback client of a ForceFeedbackController (an object within its player
    /// controller): the solo game has one client for "the player"; in co-op each player has
    /// their own, so that a player's shots do not shake another player's gamepad.
    pub(crate) fn coop_feedback_client(&self, controller: ObjId) -> i32 {
        if !self.local_split() {
            return crate::force_feedback::CLIENT_PLAYER;
        }
        self.coop
            .players
            .iter()
            .position(|&pc| self.objects.get(pc).is_some_and(|o| o.is_some()) && self.prop(pc, "FFController").as_object() == Some(controller))
            .map_or(crate::force_feedback::CLIENT_PLAYER, |i| i.min(3) as i32)
    }

    /// Motor strengths per gamepad from the strengths per force-feedback client: each
    /// player's go to the gamepad that player holds (none for a player on the keyboard).
    pub fn coop_vibrations_by_pad(&self, forces: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
        if !self.local_split() {
            return forces;
        }
        let mut out = [[0.0; 2]; 4];
        for (i, slot) in self.engine.client.viewports.iter().enumerate().take(4) {
            if let Some(pad) = self.engine.client.viewport_pads.get(usize::from(*slot)).copied().flatten().filter(|&p| p < 4) {
                out[pad] = forces[i];
            }
        }
        out
    }

    /// Two living players farther apart than `APART`.
    fn coop_players_apart(&self) -> bool {
        let pawns: Vec<ObjId> = (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).collect();
        let apart = APART;
        pawns.iter().enumerate().any(|(k, &a)| pawns[k + 1..].iter().any(|&b| self.coop_distance(a, b) > apart))
    }

    /// TriggerPetitCachottier.ShowAndHide toggles bHidden and the collision of a list of actors.
    /// Maps use it to drop the scenery of a place the player has left for good (Hual01a: two
    /// large meshes of the start, hidden near the end of the map). With the players apart, the
    /// one left behind would lose the ground under their feet: scenery about to be hidden is
    /// kept until the players are together again. Characters are hidden as the map asks.
    /// Returns the trigger's own list, to put back after the script ran on the masked one.
    pub(crate) fn coop_scenery_begin(&mut self, f: FuncId, this: ObjId) -> Option<Value> {
        if self.coop.scenery
            || self.coop.players.len() < 2
            || !self.functions[f].name.eq_ignore_ascii_case("ShowAndHide")
            || !self.is_a(this, "TriggerPetitCachottier")
        {
            return None;
        }
        let list = self.prop(this, "PersosACacher");
        let Value::Array(items) = &list else { return None };
        let apart = self.coop_players_apart();
        let mut masked: Vec<Value> = items.to_vec();
        let mut changed = false;
        for (i, item) in items.iter().enumerate() {
            let Some(a) = item.as_object().filter(|&a| self.coop_object_alive(a)) else { continue };
            if let Some(k) = self.coop.kept_scenery.iter().position(|&e| e == (this, i)) {
                // Kept instead of hidden, and toggled again: it simply stays.
                self.coop.kept_scenery.remove(k);
                masked[i] = Value::Object(None);
                changed = true;
            } else if apart && !self.prop(a, "bHidden").as_bool() && !self.is_a(a, "Pawn") {
                self.coop.kept_scenery.push((this, i));
                self.log.push(format!("Log: Co-op: scenery {} kept while the players are apart", self.path_name(a)));
                masked[i] = Value::Object(None);
                changed = true;
            }
        }
        if !changed {
            return None;
        }
        self.set_prop(this, "PersosACacher", Value::Array(masked.into()));
        self.coop.scenery = true;
        Some(list)
    }

    pub(crate) fn coop_scenery_end(&mut self, this: ObjId, list: Value) {
        self.coop.scenery = false;
        if self.coop_object_alive(this) {
            self.set_prop(this, "PersosACacher", list);
        }
    }

    /// Once per tick: the kept scenery is hidden once the players are together again.
    pub(crate) fn coop_scenery_frame(&mut self) {
        if self.coop.kept_scenery.is_empty() || self.coop_players_apart() {
            return;
        }
        let kept = std::mem::take(&mut self.coop.kept_scenery);
        let mut triggers: Vec<ObjId> = kept.iter().map(|e| e.0).collect();
        triggers.dedup();
        for trigger in triggers {
            if !self.coop_object_alive(trigger) {
                continue;
            }
            let list = self.prop(trigger, "PersosACacher");
            let Value::Array(items) = &list else { continue };
            let only: Vec<Value> = items.iter().enumerate().map(|(i, v)| if kept.contains(&(trigger, i)) { v.clone() } else { Value::Object(None) }).collect();
            self.set_prop(trigger, "PersosACacher", Value::Array(only.into()));
            self.coop.scenery = true;
            let _ = self.call_named(trigger, "ShowAndHide", vec![]);
            self.coop.scenery = false;
            self.set_prop(trigger, "PersosACacher", list);
            self.log.push("Log: Co-op: scenery left behind hidden now that the players are together".to_string());
        }
    }

    /// Start of a script of a place effect (see PlaceEffect), in co-op with several players.
    pub(crate) fn coop_place_effect_begin(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<PlaceEffect> {
        if self.coop.place_effect || self.coop.players.len() < 2 {
            return None;
        }
        let name = self.functions[f].name.as_str();
        let effect = if name.eq_ignore_ascii_case("UseCurrentColor") || name.eq_ignore_ascii_case("TurnFilterOff") {
            if !self.is_a(this, "AlarmFilter") {
                return None;
            }
            PlaceEffect::Filter
        } else if name.eq_ignore_ascii_case("InitDazzleAndShake") {
            if !self.is_a(this, "BreakableMover") {
                return None;
            }
            let broken = args.first().and_then(|a| a.as_ref()).is_some_and(Value::as_bool);
            let reference = self.coop_map_info().and_then(|m| self.prop(m, "XIIIController").as_object());
            PlaceEffect::Dazzle { broken, reference }
        } else {
            return None;
        };
        self.coop.place_effect = true;
        Some(effect)
    }

    pub(crate) fn coop_place_effect_end(&mut self, this: ObjId, effect: PlaceEffect) {
        self.coop.place_effect = false;
        if !self.coop_object_alive(this) {
            return;
        }
        match effect {
            PlaceEffect::Filter => {
                let Some(pc) = self.prop(this, "PC").as_object().filter(|&pc| self.coop_object_alive(pc)) else { return };
                let (want, speed) = (self.prop(pc, "FilterColorWanted"), self.prop(pc, "FilterColorSpeed"));
                for other in self.coop.players.clone() {
                    if other != pc && self.coop_object_alive(other) {
                        self.set_prop(other, "FilterColorWanted", want.clone());
                        self.set_prop(other, "FilterColorSpeed", speed.clone());
                    }
                }
            }
            PlaceEffect::Dazzle { broken, reference } => self.coop_dazzle_others(this, broken, reference),
        }
    }

    /// BreakableMover.InitDazzleAndShake for the players other than `reference`: intensity by
    /// the distance from the mover to each one's pawn, full under distance_start, none from
    /// distance_end.
    fn coop_dazzle_others(&mut self, mover: ObjId, broken: bool, reference: Option<ObjId>) {
        if !self.level.is_some_and(|l| self.prop(l, "bLonePlayer").as_bool()) {
            return;
        }
        let f = |vm: &Self, n: &str| vm.prop(mover, n).as_float();
        let b = |vm: &Self, n: &str| vm.prop(mover, n).as_bool();
        let local = |dist: f32, start: f32, end: f32, intensity: f32| {
            if dist < start {
                Some(intensity)
            } else if dist < end {
                Some((end - dist) / (end - start) * intensity)
            } else {
                None
            }
        };
        let dazzle = b(self, "FD_bActive") && b(self, "FD_StartAfterBreakingAnim") == broken;
        let shake = b(self, "SC_bActive") && b(self, "SC_StartAfterBreakingAnim") == broken;
        if !dazzle && !shake {
            return;
        }
        let place = self.prop(mover, "Location").as_vector();
        for i in 0..self.coop.players.len() {
            let pc = self.coop.players[i];
            if Some(pc) == reference {
                continue;
            }
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            let q = self.prop(pawn, "Location").as_vector();
            let dist = ((place[0] - q[0]).powi(2) + (place[1] - q[1]).powi(2) + (place[2] - q[2]).powi(2)).sqrt();
            if dazzle {
                if let Some(intensity) = local(dist, f(self, "FD_distance_start"), f(self, "FD_distance_end"), f(self, "FD_intensity")) {
                    // The script's FD_VectorColor: FD_Color's R, G, B times 3.9.
                    let c = crate::filter::bgra(&self.prop(mover, "FD_Color")).map(f32::from);
                    let color = [c[2] * 3.9, c[1] * 3.9, c[0] * 3.9];
                    let (start, lifetime) = (self.level_time + f64::from(f(self, "FD_delay")), f(self, "FD_lifetime"));
                    self.coop.dazzles.retain(|d| d.pc != pc);
                    self.coop.dazzles.push(Dazzle { pc, start, lifetime, intensity, color });
                }
            }
            if shake {
                if let Some(li) = local(dist, f(self, "SC_distance_start"), f(self, "SC_distance_end"), f(self, "SC_intensity")) {
                    let args = vec![
                        Value::Float(f(self, "SC_lifetime") * 25.6),
                        Value::Float(li * 1600.0),
                        Value::Vector([0.0, 0.0, 30.0]),
                        Value::Float(li * 120000.0),
                        Value::Vector([0.0; 3]),
                        Value::Float(0.0),
                    ];
                    let _ = self.call_named(pc, "ShakeView", args);
                }
            }
        }
    }

    /// Once per tick: the dazzles of the other players fade out.
    pub(crate) fn coop_dazzles_frame(&mut self) {
        if self.coop.dazzles.is_empty() {
            return;
        }
        let now = self.level_time;
        for d in std::mem::take(&mut self.coop.dazzles) {
            if !self.coop_object_alive(d.pc) {
                continue;
            }
            let time = ((now - d.start) / f64::from(d.lifetime.max(f32::MIN_POSITIVE))) as f32;
            if time > 1.0 {
                continue;
            }
            if time >= 0.0 {
                // Lerp(time, -intensity, 0).
                let scale = -d.intensity + time * d.intensity;
                let _ = self.call_named(d.pc, "ClientInstantFlash", vec![Value::Float(scale), Value::Vector(d.color)]);
            }
            self.coop.dazzles.push(d);
        }
    }

    /// Before a player's view is made: the pawns of the players at a fixed gun are hidden in
    /// their own view and shown in the others' (PlayerGunning.EndState shows them again).
    pub(crate) fn coop_gunners_for_view(&mut self, viewer: Option<ObjId>) {
        for pc in self.coop.players.clone() {
            if self.objects.get(pc).is_none_or(|o| o.is_none()) || !self.state_chain_has(pc, "PlayerGunning") {
                continue;
            }
            let Some(pawn) = self.prop(pc, "Pawn").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { continue };
            let hidden = Some(pc) == viewer;
            if self.prop(pawn, "bHidden").as_bool() != hidden {
                self.set_prop(pawn, "bHidden", Value::Bool(hidden));
                let _ = self.call_named(pawn, "RefreshDisplaying", vec![]);
            }
        }
    }

}
