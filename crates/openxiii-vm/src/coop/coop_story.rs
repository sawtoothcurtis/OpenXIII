//! Local co-op campaign, story part (see coop.rs): cinematics, scripted teleports (flashbacks
//! included), messages of the story and the equipment carried from map to map.
//!
//! - A cinematic (Level.bCineFrame) is the story player's: one picture for the whole window,
//!   the other players are parked where they stand and left there when it ends, unless the
//!   cinematic moved its player far away at once (then they follow, as for a teleport).
//! - When a script moves a player far away at once (a teleporter), the other players follow.
//!   Flashbacks take every player into the memory and back (coop_flash.rs).
//! - Dialogue lines and objectives are sent to the story player's HUD only: they are repeated
//!   on the other players' HUDs.
//! - Travel between maps: each player carries their own inventory, with their own ammunition,
//!   and the rule of the map being left applies to every player (MapInfo.NextMapKeepInventory
//!   False: weapons, armor, ammunition and items taken away, XIIIGameInfo.ProcessServerTravel;
//!   keys, cards and the map's own items taken away, MapInfo.DoTravel). The game's scripts do
//!   this for one player only. A player who did not come from the previous map (a new game, a
//!   loaded checkpoint, a Select level start, a late arrival) receives the first player's kinds
//!   of weapons and items a little after the start of the map.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// A player already this close to the one who moved is left where they are.
const NEAR: f32 = 400.0;
/// A move of more than this in one frame is a teleport, not a run, a fall or a grapple swing
/// (plus what the frame's duration allows at `FASTEST` units per second).
const JUMP: f32 = 800.0;
const FASTEST: f32 = 4000.0;
/// Seconds after the start of a map when the travel equipment is handed out (the story
/// player's travel inventory and MapInfo.FirstFrame are done by then).
const EQUIP_AT: f64 = 0.5;
/// Message classes of the story repeated on every player's HUD.
const STORY_MESSAGES: [&str; 2] = ["XIIIDialogMessage", "XIIIGoalMessage"];
/// End of map messages, repeated for players on another machine only.
pub(crate) const END_MESSAGES: [&str; 2] = ["XIIIEndGameMessage", "XIIIMissionCompletedMessage"];

impl Vm {
    /// Network co-op in lockstep: the player (rank) whose view every machine shows while a
    /// cinematic holds the game (the one split screen shows alone: its scene's player, the
    /// flashback's), None outside one. Read by the drawing only: which view a machine presents
    /// never changes the steps.
    pub fn coop_cine_shown_rank(&self) -> Option<usize> {
        if self.coop.on && self.coop.cine && !self.coop.defeat && self.coop.cine_leader < self.coop.players.len() {
            return Some(self.coop.cine_leader);
        }
        // The map's end picture (a mission complete, a failure: coop_end.rs), the view split
        // screen shows in the whole window: that player's, with its box and its menu, on every
        // machine (the other machine kept its own view, without the failure's box, and its
        // mouse could not reach the menu). (The team's defeat too: split screen shows the last
        // player down alone.)
        if !self.coop.on || !(self.coop.cine_extended || self.coop.defeat) {
            return None;
        }
        let slot = self.engine.client.extended?;
        self.coop.players.iter().position(|&pc| self.viewport_slot_of(pc) == Some(slot))
    }

    /// Once per level tick (called from the first view of a frame).
    pub(crate) fn coop_story_frame(&mut self) {
        let now = self.level_time;
        if now == self.coop.story_time {
            return;
        }
        let dt = if self.coop.story_time < 0.0 { 0.0 } else { (now - self.coop.story_time) as f32 };
        self.coop.story_time = now;
        self.coop_spawn_wait_frame();
        let Some(level) = self.level else { return };
        // Flashbacks: everybody goes into the memory (coop_flash.rs), first, so that a flashback
        // starting on this tick is seen by the cinematic rule on the same tick (no picture of
        // the second view without its effect).
        let flash_moved = self.coop_flash_frame();
        // Cinematic: one picture, then everybody together again.
        // A flashback holding its player (coop_flash.rs) is a cinematic too, without the frame.
        let flash_hold = self.coop_flash_holds();
        // A player held without control by a script (state NoControl: a scene, a faint, a forced
        // jump) is a cinematic too, even when the script drops the frame (BeachFinalFall).
        let held_alone = self.coop_cine_player();
        let cine = (self.prop(level, "bCineFrame").as_bool() || flash_hold.is_some() || held_alone.is_some()) && !self.coop.defeat;
        if cine != self.coop.cine {
            self.coop.cine = cine;
            if cine {
                // The cinematic is shown from the camera of the player it holds: the scene's
                // player, the one who made it go on (coop_cine.rs), the first player otherwise.
                self.coop.cine_leader = flash_hold.or_else(|| self.coop_cine_player()).unwrap_or(0);
                self.coop.cine_last_leader = self.coop.cine_leader;
                self.coop.cine_jumped = false;
                if self.engine.client.extended.is_none() {
                    let leader = self.coop.players.get(self.coop.cine_leader).and_then(|&pc| self.viewport_slot_of(pc));
                    self.engine.client.extended = leader.or_else(|| self.engine.client.viewports.first().copied());
                    self.coop.cine_extended = true;
                }
                // The others are parked where they are for its length (coop_cine.rs), and put
                // next to that player only at its end: placed at the start, a parked player could
                // stand where the scene's camera goes.
                let leader = self.coop.cine_leader;
                self.coop_cine_park(leader);
            } else {
                if std::mem::take(&mut self.coop.cine_extended) {
                    self.engine.client.extended = None;
                }
                self.coop_cine_unpark();
                let leader = self.coop.cine_leader;
                // Regrouped when the scene moved its player away at once, and always in a
                // flashback: the memory's player went into it while the others were parked
                // (coop_flash_gather leaves parked players to this).
                if std::mem::take(&mut self.coop.cine_jumped) || self.coop.flash.active {
                    let _ = self.coop_regroup(leader);
                }
                self.coop.cine_leader = 0;
            }
        } else {
            if cine {
                self.coop_cine_hold_parked();
            }
        }
        // The map is over (a player in state GameEnded…: the end of mission or the faint of
        // Plage00 and its screen): one picture, as in solo, from the player of the last
        // cinematic, kept until the next map.
        if !self.coop.defeat && self.engine.client.extended.is_none() {
            let ended =
                self.coop.players.iter().any(|&pc| self.coop_object_alive(pc) && self.current_state(pc).is_some_and(|(_, s)| s.name.starts_with("GameEnded")));
            if ended {
                // The player who ended the map (coop_end.rs), or the last cinematic's.
                let leader = self.coop_end_leader();
                let slot = self.coop.players.get(leader).and_then(|&pc| self.viewport_slot_of(pc));
                self.engine.client.extended = slot.or_else(|| self.engine.client.viewports.first().copied());
                self.coop.cine_extended = true;
            }
        }
        self.coop_rules_frame();
        self.coop_imposed_frame();
        self.coop_scene_wake_frame();
        self.coop_shared_frame();
        self.coop_dialog_copies_frame();
        self.coop_scenery_frame();
        self.coop_dazzles_frame();
        // Travel equipment.
        if !self.coop.equipped && now >= EQUIP_AT {
            if let (Some(first), Some(map_info)) = (self.coop_living_pawn(0), self.coop_story_map_info()) {
                self.coop.equipped = true;
                for i in 1..self.coop.players.len() {
                    if let Some(pawn) = self.coop_living_pawn(i) {
                        self.coop_arrival_equipment(i, map_info, first, pawn);
                    }
                }
            }
        }
        // Scripted teleports.
        let places: Vec<Option<[f32; 3]>> =
            (0..self.coop.players.len()).map(|i| self.coop_living_pawn(i).map(|p| self.prop(p, "Location").as_vector())).collect();
        // During a cinematic: its player moved far away at once is followed at its end.
        if cine && !flash_moved && self.coop.last_places.len() == places.len() {
            let limit = JUMP + FASTEST * dt;
            let k = self.coop.cine_leader;
            if let (Some(Some(p)), Some(Some(q))) = (places.get(k), self.coop.last_places.get(k)) {
                if ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() > limit {
                    self.coop.cine_jumped = true;
                }
            }
        }
        if !cine && !flash_moved && self.coop.last_places.len() == places.len() {
            let limit = JUMP + FASTEST * dt;
            let jumped = (0..places.len()).find(|&i| match (places[i], self.coop.last_places[i]) {
                (Some(p), Some(q)) => ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() > limit,
                _ => false,
            });
            if let Some(i) = jumped {
                let _ = self.coop_regroup(i);
            }
        }
        self.coop.last_places = (0..self.coop.players.len()).map(|i| self.coop_living_pawn(i).map(|p| self.prop(p, "Location").as_vector())).collect();
    }

    /// Equipment of player `i` at the start of a map: their own inventory of the previous map
    /// (already given by their login when they play on this machine), otherwise the first
    /// player's kinds of weapons and items. Returns how it was given.
    pub(crate) fn coop_arrival_equipment(&mut self, i: usize, map_info: ObjId, first: ObjId, pawn: ObjId) -> &'static str {
        if self.coop.own_travel.get(i).copied().unwrap_or(false) {
            return "own";
        }
        if let Some(text) = self.coop.arrived.get(i).cloned().flatten() {
            self.coop_give_carried(map_info, pawn, &text);
            self.log.push(format!("Log: Co-op: player {} gets their own equipment of the previous map", i + 1));
            return "carried";
        }
        self.coop_copy_equipment(map_info, first, pawn);
        "copied"
    }

    /// After the logins of a map in co-op: the weapon each player holds is seen by the others.
    /// Pawn.Weapon is a travel property, but its ThirdPersonActor is not: after a travel that
    /// keeps the inventory, AcceptInventory and Weapon.TravelPostAccept bring the weapon up
    /// without Inventory.AttachToPawn, so the pawn holds it with no third-person attachment
    /// until the next weapon switch (XIIIPawn.ChangedWeapon). Solo nobody sees the player's own
    /// pawn; in co-op the other players do, so each held weapon without its attachment gets it
    /// here, as ChangedWeapon would. Solo is untouched.
    pub(crate) fn coop_attach_held_weapons(&mut self) {
        if !self.coop.on || self.coop.players.len() < 2 {
            return;
        }
        for i in 0..self.coop.players.len() {
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            let Some(weapon) = self.prop(pawn, "Weapon").as_object().filter(|&w| self.coop_object_alive(w)) else { continue };
            if self.prop(weapon, "ThirdPersonActor").as_object().is_some_and(|a| self.coop_object_alive(a)) {
                continue;
            }
            if self.call_named(weapon, "AttachToPawn", vec![Value::Object(Some(pawn))]).is_ok() {
                self.log.push(format!("Log: Co-op: player {}'s weapon in hand attached to the pawn", i + 1));
            }
        }
    }

    /// A player whose login came without the travel inventory: their own
    /// inventory of the previous map (`text`, ExportTravel blocks) is given back as the map's
    /// own start equipment is (MapInfo.GiveSomething), with their own ammunition: the amount
    /// carried, plus what this map's InitialInv adds (Ammunition.AddAmmo, up to MaxAmmo).
    pub(crate) fn coop_give_carried(&mut self, map_info: ObjId, pawn: ObjId, text: &str) {
        let (pawn_class, ammo_class, saved_class) = (self.class("Pawn"), self.class("Ammunition"), self.class("XIIIThingsToSave"));
        let mut things: Vec<crate::value::ClassId> = Vec::new();
        let mut ammo: Vec<(crate::value::ClassId, i32)> = Vec::new();
        let mut current: Option<crate::value::ClassId> = None;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(name) = crate::value::parse_fstring(line, "CLASS=").filter(|_| line.starts_with("Class=")) {
                current = self.class(&name).filter(|&c| {
                    !pawn_class.is_some_and(|p| self.is_child_of(c, p)) && !saved_class.is_some_and(|t| self.is_child_of(c, t)) && !self.coop_single_item(c)
                });
                match current {
                    Some(c) if ammo_class.is_some_and(|a| self.is_child_of(c, a)) => ammo.push((c, self.prop(self.classes[c].cdo, "AmmoAmount").as_int())),
                    Some(c) => things.push(c),
                    None => {}
                }
            } else if let (Some(c), Some(v)) = (current, line.strip_prefix("AmmoAmount=")) {
                if let (Some(last), Ok(n)) = (ammo.last_mut().filter(|a| a.0 == c), v.trim().parse::<i32>()) {
                    last.1 = n;
                }
            }
        }
        let initial: Vec<crate::value::ClassId> = match self.prop(map_info, "InitialInv") {
            Value::Array(items) => items.iter().filter_map(|v| match v {
                Value::Class(Some(c)) => Some(*c),
                _ => None,
            }).collect(),
            _ => Vec::new(),
        };
        self.coop.sharing = true;
        let given: Vec<crate::value::ClassId> = initial.iter().copied().filter(|&c| !ammo_class.is_some_and(|a| self.is_child_of(c, a))).chain(things.iter().copied()).collect();
        for c in given {
            let _ = self.call_named(map_info, "GiveSomething", vec![Value::Class(Some(c)), Value::Object(Some(pawn))]);
        }
        for (c, carried) in ammo {
            let item = self.call_named(map_info, "GiveSomething", vec![Value::Class(Some(c)), Value::Object(Some(pawn))]).ok().and_then(|v| v.as_object());
            let Some(item) = item.filter(|&o| self.coop_object_alive(o)) else { continue };
            let added: i32 = initial.iter().filter(|&&k| k == c).map(|_| self.prop(self.classes[c].cdo, "AmmoAmount").as_int()).sum();
            let most = self.prop(item, "MaxAmmo").as_int();
            let amount = if added > 0 && carried < most { most.min(carried + added) } else { carried };
            self.set_prop(item, "AmmoAmount", Value::Int(amount));
        }
        self.coop.sharing = false;
    }

    /// XIIIGameInfo.ProcessServerTravel and MapInfo.DoTravel take the inventory away from one
    /// player: their own (`kind` 0 and 1, see coop_travel_rule_end).
    pub(crate) fn coop_travel_rule_begin(&self, f: FuncId, this: ObjId) -> Option<u8> {
        if !self.coop.on || self.coop.players.len() < 2 {
            return None;
        }
        let function = &self.functions[f];
        let owner = &self.classes[function.owner].name;
        if function.name.eq_ignore_ascii_case("ProcessServerTravel") && owner.eq_ignore_ascii_case("XIIIGameInfo") {
            return Some(0);
        }
        if function.name.eq_ignore_ascii_case("DoTravel") && owner.eq_ignore_ascii_case("MapInfo") && self.coop_story_map_info() == Some(this) {
            return Some(1);
        }
        None
    }

    /// After that call: the same for every other player.
    /// - 0, ProcessServerTravel: with MapInfo.NextMapKeepInventory False, every Weapon, Armor,
    ///   Ammunition and XIIIItems the player's pawn owns is destroyed and its health is the
    ///   default one (the script does it for the last local controller only).
    /// - 1, DoTravel: Keys (but LockPick), MagneticCard, scenery weapons (WHand 4) and the map's
    ///   InventoryToDestroyBeforeNextMap kinds (the script does it for MapInfo.XIIIPawn only).
    pub(crate) fn coop_travel_rule_end(&mut self, kind: u8, this: ObjId) {
        #[cfg(test)]
        if self.coop.travel_rule_off {
            return;
        }
        let pawns: Vec<ObjId> = (0..self.coop.players.len())
            .filter_map(|i| self.coop.players.get(i).copied())
            .filter(|&pc| self.coop_object_alive(pc))
            .filter_map(|pc| self.prop(pc, "Pawn").as_object())
            .filter(|&p| self.coop_object_alive(p))
            .collect();
        if kind == 0 {
            let Some(map_info) = self.coop_story_map_info() else { return };
            if self.prop(map_info, "NextMapKeepInventory").as_bool() {
                return;
            }
            for pawn in pawns {
                let owned: Vec<ObjId> = self
                    .actors
                    .iter()
                    .copied()
                    .filter(|&o| self.coop_object_alive(o) && self.is_a(o, "Inventory") && self.prop(o, "Owner").as_object() == Some(pawn))
                    .filter(|&o| ["Weapon", "Armor", "Ammunition", "XIIIItems"].iter().any(|k| self.is_a(o, k)))
                    .collect();
                for o in owned {
                    if self.coop_object_alive(o) {
                        let _ = self.call_named(o, "Destroy", vec![]);
                    }
                }
                let health = self.objects[pawn].as_ref().map(|x| self.prop(self.classes[x.class].cdo, "Health"));
                if let Some(h) = health {
                    self.set_prop(pawn, "Health", h);
                }
            }
            self.log.push(" > Co-op: no inventory kept for next map, for every player".to_string());
            return;
        }
        let done = self.prop(this, "XIIIPawn").as_object();
        for pawn in pawns.into_iter().filter(|&p| Some(p) != done) {
            let mut item = self.prop(pawn, "Inventory").as_object();
            let mut chain = Vec::new();
            while let Some(o) = item.filter(|&o| self.coop_object_alive(o) && chain.len() < 256) {
                chain.push(o);
                item = self.prop(o, "Inventory").as_object();
            }
            for o in chain {
                if !self.coop_object_alive(o) {
                    continue;
                }
                let key = (self.is_a(o, "Keys") && !self.is_a(o, "LockPick")) || self.is_a(o, "MagneticCard");
                let scenery = self.is_a(o, "Weapon") && self.prop(o, "WHand").as_byte() == 4;
                if scenery && self.prop(pawn, "Weapon").as_object() == Some(o) {
                    self.set_prop(pawn, "Weapon", Value::Object(None));
                }
                if key || scenery {
                    let _ = self.call_named(o, "Destroy", vec![]);
                }
            }
            if let Value::Array(kinds) = self.prop(this, "InventoryToDestroyBeforeNextMap") {
                for kind_name in kinds {
                    for _ in 0..256 {
                        let found = self.call_named(pawn, "FindInventoryKind", vec![kind_name.clone()]).ok().and_then(|v| v.as_object()).filter(|&o| self.coop_object_alive(o));
                        let Some(o) = found else { break };
                        let _ = self.call_named(o, "Destroy", vec![]);
                    }
                }
            }
        }
    }

    /// Rank of the first player whose controller is in state NoControl (the one a cinematic
    /// holds); None when no player is.
    fn coop_cine_player(&self) -> Option<usize> {
        self.coop.players.iter().position(|&pc| {
            self.coop_object_alive(pc)
                && !self.coop_held_copy(pc)
                && !self.coop_cine_is_parked(pc)
                && self.current_state(pc).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("NoControl"))
        })
    }

    pub(crate) fn coop_story_map_info(&self) -> Option<ObjId> {
        let game = self.level.and_then(|l| self.prop(l, "Game").as_object())?;
        self.prop(game, "MapInfo").as_object().filter(|&m| self.coop_object_alive(m))
    }

    /// Puts the other living players next to player `leader`, unless they are already near.
    /// Returns the ranks of the players for whom no free place was found there.
    pub(crate) fn coop_regroup(&mut self, leader: usize) -> Vec<usize> {
        let mut unplaced = Vec::new();
        let Some(to) = self.coop_living_pawn(leader) else { return unplaced };
        for i in 0..self.coop.players.len() {
            if i == leader {
                continue;
            }
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            if self.coop_distance(pawn, to) <= NEAR {
                continue;
            }
            // Parked during a cinematic: put next to its player when it ends (coop_cine.rs).
            if self.coop_cine_is_parked(self.coop.players[i]) {
                continue;
            }
            let placed = self.coop_bring_next_to(i, leader);
            if placed {
                self.log.push(format!("Log: Co-op: player {} brought next to player {}", i + 1, leader + 1));
            }
            if !placed {
                unplaced.push(i);
            }
        }
        unplaced
    }

    /// Player `i` put on free ground next to player `to` (never inside a door or another
    /// solid mover), facing where `to` faces when that way is open, the most open way
    /// otherwise (not a wall). False: no free place.
    pub(crate) fn coop_bring_next_to(&mut self, i: usize, to: usize) -> bool {
        let (Some(pawn), Some(target)) = (self.coop_living_pawn(i), self.coop_living_pawn(to)) else { return false };
        self.coop_neutral(i);
        let origin = self.prop(target, "Location").as_vector();
        let (r, h) = (self.prop(pawn, "CollisionRadius").as_float(), self.prop(pawn, "CollisionHeight").as_float());
        for place in self.coop_places_near(target) {
            // A box half the pawn's width swept from the teammate to the place: nothing solid on
            // the way (a closed door, a lift's wall). Half width, so that a teammate standing
            // against a wall is not taken for one stuck in it; the place itself is checked at full
            // size by SetLocation.
            let end = [place[0], place[1], origin[2]];
            let blocked = match self.line_check(target, origin, end, [r * 0.5, r * 0.5, h * 0.6], false) {
                // Already touching something where the teammate stands: a plain line instead.
                Some((t, _, _)) if t <= 0.0 => self.line_check(target, origin, end, [0.0; 3], false).is_some(),
                hit => hit.is_some(),
            };
            if blocked {
                continue;
            }
            if !self.call_named(pawn, "SetLocation", vec![Value::vector(place[0], place[1], place[2])]).is_ok_and(|v| v.as_bool()) {
                continue;
            }
            self.set_prop(pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
            let Some(&from) = self.coop.players.get(to) else { return true };
            let Some(&pc) = self.coop.players.get(i) else { return true };
            let yaw = self.coop_open_yaw(pawn, place, self.prop(from, "Rotation").as_rotator()[1]);
            self.set_prop(pc, "Rotation", Value::rotator(0, yaw, 0));
            self.set_prop(pawn, "Rotation", Value::rotator(0, yaw, 0));
            return true;
        }
        false
    }

    /// `preferred` when the way is free for a few steps from `at`, otherwise the nearest free
    /// one of eight ways around it, otherwise the freest.
    pub(crate) fn coop_open_yaw(&self, pawn: ObjId, at: [f32; 3], preferred: i32) -> i32 {
        const LOOK: f32 = 400.0;
        let free = |yaw: i32| -> f32 {
            let a = (yaw & 65535) as f32 * (std::f32::consts::PI / 32768.0);
            let end = [at[0] + a.dcos() * LOOK, at[1] + a.dsin() * LOOK, at[2]];
            match self.line_check(pawn, at, end, [0.0; 3], false) {
                Some((t, _, _)) => t * LOOK,
                None => LOOK,
            }
        };
        // The nearest way to the preferred one that is free for a few steps, turning an
        // eighth of a turn at a time each side; otherwise the freest.
        for k in [0, 1, -1, 2, -2, 3, -3, 4] {
            let yaw = preferred + k * 8192;
            if free(yaw) >= LOOK * 0.5 {
                return yaw & 65535;
            }
        }
        (0..8).map(|k| preferred + k * 8192).max_by(|&a, &b| free(a).total_cmp(&free(b))).unwrap_or(preferred) & 65535
    }

    /// HUD.LocalizedMessage of a story message on the story player's HUD.
    pub(crate) fn coop_mirrors_message(&self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if self.coop.mirroring || self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("LocalizedMessage") {
            return false;
        }
        let Some(&first) = self.coop.players.first() else { return false };
        if !self.coop_object_alive(first) || self.prop(first, "MyHUD").as_object() != Some(this) {
            return false;
        }
        match args.first().and_then(|a| a.as_ref()) {
            Some(Value::Class(Some(class))) => {
                STORY_MESSAGES.iter().any(|m| self.classes[*class].name.eq_ignore_ascii_case(m))
            }
            _ => false,
        }
    }

    /// End of the map (mission complete, game over): shown on the story player's screen, which a
    /// local player shares.
    fn coop_end_message(&self, class: crate::value::ClassId) -> bool {
        END_MESSAGES.iter().any(|m| self.classes[class].name.eq_ignore_ascii_case(m))
    }

    /// The same message on the other players' HUDs.
    pub(crate) fn coop_mirror_message(&mut self, args: Vec<Option<Value>>) {
        self.coop_mirror_message_from(args, 0);
    }

    /// The same message, first shown on the HUD of player `from`, on the other players' HUDs.
    pub(crate) fn coop_mirror_message_from(&mut self, args: Vec<Option<Value>>, from: usize) {
        // A dialogue line: its copies follow the story HUD's box (`coop_dialog_copies_frame`).
        let dialog = matches!(args.first().and_then(|a| a.as_ref()), Some(Value::Class(Some(c))) if self.classes[*c].name.eq_ignore_ascii_case("XIIIDialogMessage"));
        let source = dialog
            .then(|| self.coop.players.get(from).copied())
            .flatten()
            .and_then(|pc| self.prop(pc, "MyHUD").as_object())
            .filter(|&h| self.coop_object_alive(h))
            .and_then(|h| self.prop(h, "HudDlg").as_object().filter(|&d| self.coop_object_alive(d)).map(|d| (h, d)));
        let mut copies = Vec::new();
        for (i, pc) in self.coop.players.clone().into_iter().enumerate() {
            if i == from || !self.coop_object_alive(pc) {
                continue;
            }
            if matches!(args.first().and_then(|a| a.as_ref()), Some(Value::Class(Some(c))) if self.coop_end_message(*c)) {
                continue;
            }
            let Some(hud) = self.prop(pc, "MyHUD").as_object().filter(|&h| self.coop_object_alive(h)) else { continue };
            if let Some(f) = self.find_function(hud, "LocalizedMessage", false) {
                let _ = self.call(f, hud, args.clone());
                if let Some(copy) = source.and(self.prop(hud, "HudDlg").as_object()).filter(|&d| self.coop_object_alive(d)) {
                    copies.push((hud, copy));
                }
            }
        }
        if let (Some((hud, dlg)), false) = (source, copies.is_empty()) {
            self.coop.dialog_copies.retain(|e| e.0 != hud);
            self.coop.dialog_copies.push((hud, dlg, copies));
        }
    }

    /// DialogueManager.Speak gives the story HUD's box the voice's length
    /// (HudDlg.MyMessage.EndOfLife = WaveLength + Level.TimeSeconds, Lifetime = WaveLength) and
    /// removes it when the dialogue stops (HudDlg.RemoveMe, Destroyed, MustBeStop): on PC.myHUD
    /// only. Each frame, the copies on the other local HUDs take the same end of life, and go when
    /// that box goes (removed, or replaced by a line that was not shared), so every player reads
    /// the line as long as it is heard.
    pub(crate) fn coop_dialog_copies_frame(&mut self) {
        if self.coop.dialog_copies.is_empty() {
            return;
        }
        for (hud, dlg, copies) in std::mem::take(&mut self.coop.dialog_copies) {
            let live = |vm: &Self, h: ObjId, d: ObjId| vm.coop_object_alive(h) && vm.coop_object_alive(d) && vm.prop(h, "HudDlg").as_object() == Some(d);
            let copies: Vec<(ObjId, ObjId)> = copies.into_iter().filter(|&(h, d)| live(self, h, d)).collect();
            if copies.is_empty() {
                continue;
            }
            if !live(self, hud, dlg) {
                {
                    for (_, copy) in copies {
                        let _ = self.call_named(copy, "RemoveMe", vec![]);
                    }
                }
                continue;
            }
            // HUDLocalizedMessage: Message, Switch, OptionalObject, EndOfLife, Lifetime, ...
            let Value::Struct(_, fields) = self.prop(dlg, "MyMessage") else { continue };
            let (Some(end), Some(life)) = (fields.get(3).cloned(), fields.get(4).cloned()) else { continue };
            {
                for &(_, copy) in &copies {
                    if let Value::Struct(name, mut f) = self.prop(copy, "MyMessage") {
                        if f.len() > 4 && (f[3] != end || f[4] != life) {
                            f[3] = end.clone();
                            f[4] = life.clone();
                            self.set_prop(copy, "MyMessage", Value::Struct(name, f));
                        }
                    }
                }
            }
            self.coop.dialog_copies.push((hud, dlg, copies));
        }
    }

    /// HudDialog.DrawDlg lays its line out once, on the first canvas it is drawn on
    /// (ProcessMessage when MyMessage.NumLines == 0: box width fXSize from ClipX, line breaks
    /// "#N" written into StringMessage). In solo the canvas keeps its size during a line; in
    /// co-op the picture goes from split screen to one view (and back) at flashbacks and
    /// cinematics, and the line kept the other canvas's layout: one line running past the
    /// edge of the screen, its end wrapped to the left outside the box (SPADS02a). When the
    /// canvas size changed since the layout, the original text is given back with NumLines = 0
    /// and fXSize = 0 (as SetUpLocalizedMessage leaves them), so ProcessMessage runs again.
    /// Returns the canvas size and the text to remember once DrawDlg has laid it out.
    pub(crate) fn coop_dialog_layout_before(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<(f32, f32, String)> {
        if self.coop.dialog_layout_busy || !self.functions[f].name.eq_ignore_ascii_case("DrawDlg") || !self.is_a(this, "HudDialog") {
            return None;
        }
        let c = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object)?;
        let clip = (self.cf(c, "ClipX"), self.cf(c, "ClipY"));
        // HUDLocalizedMessage: Message, Switch, OptionalObject, EndOfLife, LifeTime, numLines,
        // StringMessage, DrawColor, fXSize.
        let Value::Struct(name, mut fields) = self.prop(this, "MyMessage") else { return None };
        if fields.len() < 9 {
            return None;
        }
        let text = fields[6].as_name();
        if fields[5].as_int() == 0 {
            return Some((clip.0, clip.1, text));
        }
        let i = self.coop.dialog_layouts.iter().position(|l| l.0 == this)?;
        let (_, x, y, ref original, ref laid_out) = self.coop.dialog_layouts[i];
        if *laid_out != text || (x, y) == clip {
            return None;
        }
        let original = original.clone();
        fields[5] = Value::Int(0);
        fields[6] = Value::Str(original.clone());
        fields[8] = Value::Float(0.0);
        self.set_prop(this, "MyMessage", Value::Struct(name, fields));
        Some((clip.0, clip.1, original))
    }

    /// After DrawDlg: the layout made on this canvas (see `coop_dialog_layout_before`).
    pub(crate) fn coop_dialog_layout_after(&mut self, this: ObjId, (x, y, original): (f32, f32, String)) {
        let Value::Struct(_, fields) = self.prop(this, "MyMessage") else { return };
        let Some(laid_out) = fields.get(6).map(Value::as_name) else { return };
        let mut layouts = std::mem::take(&mut self.coop.dialog_layouts);
        layouts.retain(|l| l.0 != this && self.coop_object_alive(l.0));
        layouts.push((this, x, y, original, laid_out));
        self.coop.dialog_layouts = layouts;
    }
}

