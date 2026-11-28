//! Local co-op campaign, what the story hands to "the player" that every player needs (see
//! coop.rs). Found by going through the scripts of the whole campaign, map by map:
//!
//! - An item a character hands over in a scene (Cine2.GiveObject, "give N": Jones's grapple in
//!   Toits01) goes to the scene's player only: every other player receives the same kind of
//!   item, as for MapInfo.GiveSomething (coop.rs, coop_share_gift).
//! - A countdown (Chronometre: USA01 bomb, Palace01 room, SSH102b self-destruct) is one item of
//!   the player who received it: it is not copied (a copy would start its own default minute and
//!   would never be put away by the map's script), its display is shown on every player's HUD.
//! - Palace01's microphone (class micro) is set up by the map's script on the item it gave:
//!   the listened target and whether losing it ends the game. A copy had no target and kept the
//!   default "ends the game": using it ended the game after a few seconds. It is not copied
//!   either, one player listens.
//! - A boss's health bar (BossHealthBar, USA02's helicopter, Johansson, the Mangouste) is added
//!   to the HUD of one player: it is added to every player's HUD.
//! - A story message (a dialogue line, an objective, a MessageTrigger) sent to the HUD of a
//!   player other than the first (the player who started the dialogue, the head of the
//!   controller list) is repeated on the other players' HUDs, as the first player's ones are
//!   (coop_story.rs).
//! - Hual04a: the fuse is looked for in the story player's inventory only (state
//!   RechercheFusible): a fuse picked up by another player could never be put in place.
//!   Every player's inventory is looked at.
//! - Scripts that measure "the player" act with the player who matters (owner's choice, 15/11):
//!   Hual04c's escort of Carrington measures the player nearest to him; SPADS01's searchlight
//!   watches the player most inside its beam; the bosses that keep "the player" as their target
//!   (Johansson, USA02's helicopter) turn to a player they see, the nearest one first.
//! - KelloAmbush (Kello01a): the scripted ambush walks the player it took at the start of the
//!   map to its place; it takes the player nearest to it when it starts (rules of coop_rules.rs:
//!   the nearest player acts in a scene).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// What a sharing rule does around the call it watches.
pub(crate) enum Shared {
    /// Cine2.GiveObject: the item given, to the other players too.
    SceneGift { receiver: ObjId },
    /// XIIIBaseHud.AddBossBar on a player's HUD.
    BossBar { hud: ObjId },
    /// XIIIBaseHud.AddChronoDisplay called by a Chronometre.
    Chrono { hud: ObjId },
    /// A story message on the HUD of a player other than the first (rank of that player).
    Message { from: usize },
    /// A script that looks at the story player only: run once for each player until it moves
    /// on (Hual04a.RechercheFusible.Tick).
    EachPlayer,
    /// A script of MapInfo that looks at its player: run with this player as MapInfo's player.
    AsPlayer { pawn: ObjId },
}

/// Seconds between two looks of the bosses for their target.
const BOSS_PERIOD: f64 = 1.0;
/// A boss turns to another player in sight only if that one is this much closer.
const BOSS_CLOSER: f32 = 0.7;
/// Controllers of the bosses that keep their target in their variable XIII.
const BOSSES: [&str; 2] = ["JohanssonController", "USA02HelicoBossController"];

/// Classes whose scripts send the story's messages (dialogues, objectives).
const STORY_SENDERS: [&str; 3] = ["DialogueManager", "MapInfo", "MessageTrigger"];
const STORY_MESSAGES: [&str; 2] = ["XIIIDialogMessage", "XIIIGoalMessage"];

/// Story items that stay one item for the team (not copied to the other players).
const SINGLE_ITEMS: [&str; 2] = ["Chronometre", "micro"];

impl Vm {
    /// An item of this class stays with the player the story gave it to.
    pub(crate) fn coop_single_item(&self, class: crate::value::ClassId) -> bool {
        let mut c = Some(class);
        while let Some(k) = c {
            if SINGLE_ITEMS.iter().any(|n| self.classes[k].name.eq_ignore_ascii_case(n)) {
                return true;
            }
            c = self.classes[k].super_class;
        }
        false
    }

    /// Rank of the local co-op player who owns `hud`.
    fn coop_hud_rank(&self, hud: ObjId) -> Option<usize> {
        let pc = self.prop(hud, "PlayerOwner").as_object()?;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        self.coop_is_local(pc).then_some(rank)
    }

    /// HUDs of the local players other than `rank`.
    fn coop_other_huds(&self, rank: usize) -> Vec<ObjId> {
        (0..self.coop.players.len())
            .filter(|&i| i != rank)
            .filter_map(|i| {
                let pc = self.coop.players[i];
                (self.coop_object_alive(pc) && self.coop_is_local(pc)).then(|| self.prop(pc, "MyHUD").as_object()).flatten()
            })
            .filter(|&h| self.coop_object_alive(h) && self.is_a(h, "XIIIBaseHud"))
            .collect()
    }

    /// Before a call, in co-op: the sharing rule watching it, if any.
    pub(crate) fn coop_shared_before(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<Shared> {
        if self.coop.shared_busy || self.coop.players.len() < 2 {
            return None;
        }
        let function = &self.functions[f];
        let name = function.name.as_str();
        let in_state = |state: &str, class: &str| {
            function.state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(state)) && self.classes[function.owner].name.eq_ignore_ascii_case(class)
        };
        match name {
            name if name.eq_ignore_ascii_case("GiveObject") => {
                if !self.is_a(this, "Cine2") {
                    return None;
                }
                let receiver = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object)?;
                (0..self.coop.players.len()).any(|i| self.coop_living_pawn(i) == Some(receiver)).then_some(Shared::SceneGift { receiver })
            }
            name if name.eq_ignore_ascii_case("AddBossBar") => {
                (self.is_a(this, "XIIIBaseHud") && self.coop_hud_rank(this).is_some()).then_some(Shared::BossBar { hud: this })
            }
            name if name.eq_ignore_ascii_case("AddChronoDisplay") => {
                let caller = self.this_stack.last().copied()?;
                (self.is_a(this, "XIIIBaseHud") && self.is_a(caller, "Chronometre") && self.coop_hud_rank(this).is_some())
                    .then_some(Shared::Chrono { hud: this })
            }
            name if name.eq_ignore_ascii_case("LocalizedMessage") => {
                if self.coop.mirroring || !self.is_a(this, "XIIIBaseHud") {
                    return None;
                }
                let from = self.coop_hud_rank(this).filter(|&r| r > 0)?;
                let Some(Value::Class(Some(class))) = args.first().and_then(|a| a.as_ref()) else { return None };
                if !STORY_MESSAGES.iter().any(|m| self.classes[*class].name.eq_ignore_ascii_case(m)) {
                    return None;
                }
                let caller = self.this_stack.last().copied()?;
                STORY_SENDERS.iter().any(|c| self.is_a(caller, c)).then_some(Shared::Message { from })
            }
            name if name.eq_ignore_ascii_case("Tick") => {
                if in_state("RechercheFusible", "Hual04a") {
                    return Some(Shared::EachPlayer);
                }
                if in_state("STA_Check", "Spads01") {
                    let projo = self.prop(this, "Projo").as_object().filter(|&o| self.coop_object_alive(o))?;
                    return self.coop_in_beam(projo).map(|pawn| Shared::AsPlayer { pawn });
                }
                None
            }
            name if name.eq_ignore_ascii_case("Timer") => {
                if !in_state("CheckPlayerDistance", "Hual04c") {
                    return None;
                }
                let carrington = self.prop(this, "carrington").as_object().filter(|&o| self.coop_object_alive(o))?;
                self.coop_nearest_pawn(carrington).map(|pawn| Shared::AsPlayer { pawn })
            }
            name if name.eq_ignore_ascii_case("BeginState") => {
                if in_state("LetsMove_01", "KelloAmbush") {
                    self.coop_ambush_player(this);
                }
                None
            }
            _ => None,
        }
    }

    /// The call itself, with the rule's after part.
    pub(crate) fn coop_shared_call(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>, shared: Shared) -> crate::Result<(Value, Vec<Option<Value>>)> {
        if let Shared::EachPlayer = shared {
            return self.coop_each_player_call(f, this, args);
        }
        if let Shared::AsPlayer { pawn } = shared {
            return self.coop_as_player_call(f, this, args, pawn);
        }
        let copy = args.clone();
        self.coop.shared_busy = true;
        let r = self.call(f, this, args);
        self.coop.shared_busy = false;
        let values: Vec<Value> = copy.iter().map(|a| a.clone().unwrap_or(Value::Void)).collect();
        match shared {
            Shared::SceneGift { receiver } => {
                let item = r.as_ref().ok().and_then(|(v, _)| v.as_object()).filter(|&o| self.coop_object_alive(o));
                if let Some(item) = item {
                    self.coop_share_scene_gift(item, receiver);
                }
            }
            Shared::BossBar { hud } => {
                let rank = self.coop_hud_rank(hud).unwrap_or(0);
                self.coop.shared_busy = true;
                for other in self.coop_other_huds(rank) {
                    let _ = self.call_named(other, "AddBossBar", values.clone());
                }
                self.coop.shared_busy = false;
            }
            Shared::Chrono { hud } => {
                let rank = self.coop_hud_rank(hud).unwrap_or(0);
                let warning = self.prop(hud, "bDrawChronoWithWarningColors");
                self.coop.shared_busy = true;
                for other in self.coop_other_huds(rank) {
                    self.set_prop(other, "bDrawChronoWithWarningColors", warning.clone());
                    let _ = self.call_named(other, "AddChronoDisplay", values.clone());
                }
                self.coop.shared_busy = false;
            }
            Shared::Message { from } => {
                self.coop.mirroring = true;
                self.coop_mirror_message_from(copy, from);
                self.coop.mirroring = false;
            }
            Shared::EachPlayer | Shared::AsPlayer { .. } => {}
        }
        r
    }

    /// The item a scene's character gave to `receiver`: the same kind for every other player.
    fn coop_share_scene_gift(&mut self, item: ObjId, receiver: ObjId) {
        let Some(class) = self.objects[item].as_ref().map(|o| o.class) else { return };
        if self.coop_single_item(class) {
            return;
        }
        let Some(map_info) = self.coop_story_map_info() else { return };
        let pawns: Vec<ObjId> = (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).filter(|&p| p != receiver).collect();
        self.coop.sharing = true;
        for pawn in pawns {
            let _ = self.call_named(map_info, "GiveSomething", vec![Value::Class(Some(class)), Value::Object(Some(pawn))]);
            self.log.push(format!("Log: Co-op: {} given to {} too", self.classes[class].name, self.path_name(pawn)));
        }
        self.coop.sharing = false;
    }

    /// Runs a script that looks at MapInfo's player once with each living player as that
    /// player, until the script leaves its state.
    fn coop_each_player_call(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> crate::Result<(Value, Vec<Option<Value>>)> {
        let Some(map_info) = self.coop_story_map_info() else { return self.call(f, this, args) };
        let state = |vm: &Vm| vm.current_state(this).map(|(_, s)| s.name.clone());
        let before = state(self);
        let saved = (self.prop(map_info, "XIIIPawn"), self.prop(map_info, "XIIIController"));
        let mut result = None;
        for i in 0..self.coop.players.len() {
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
            self.set_prop(map_info, "XIIIController", Value::Object(Some(self.coop.players[i])));
            self.coop.shared_busy = true;
            let r = self.call(f, this, args.clone());
            self.coop.shared_busy = false;
            let moved = state(self) != before;
            result = Some(r);
            if moved {
                self.log.push(format!("Log: Co-op: {} done with player {}", self.path_name(this), i + 1));
            }
            if moved || result.as_ref().is_some_and(|r| r.is_err()) {
                break;
            }
        }
        self.set_prop(map_info, "XIIIPawn", saved.0);
        self.set_prop(map_info, "XIIIController", saved.1);
        match result {
            Some(r) => r,
            None => self.call(f, this, args),
        }
    }

    /// Living pawn of the player nearest to `actor`.
    fn coop_nearest_pawn(&self, actor: ObjId) -> Option<ObjId> {
        (0..self.coop.players.len())
            .filter_map(|i| self.coop_living_pawn(i))
            .min_by(|&a, &b| self.coop_distance(a, actor).total_cmp(&self.coop_distance(b, actor)))
    }

    /// Living pawn of the player most inside the beam of `light` (the direction of its
    /// Rotation), as the script measures it: Normal(pawn - light) Dot vector(Rotation).
    fn coop_in_beam(&self, light: ObjId) -> Option<ObjId> {
        let r = self.prop(light, "Rotation").as_rotator();
        let unit = std::f32::consts::TAU / 65536.0;
        let (pitch, yaw) = (r[0] as f32 * unit, r[1] as f32 * unit);
        let dir = [pitch.dcos() * yaw.dcos(), pitch.dcos() * yaw.dsin(), pitch.dsin()];
        let at = self.prop(light, "Location").as_vector();
        let score = |p: ObjId| {
            let l = self.prop(p, "Location").as_vector();
            let d = [l[0] - at[0], l[1] - at[1], l[2] - at[2]];
            let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3);
            (d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2]) / n
        };
        (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).max_by(|&a, &b| score(a).total_cmp(&score(b)))
    }

    /// Runs a script of MapInfo `this` with `pawn` as its player (XIIIPawn, XIIIController),
    /// the story player again afterwards unless the script changed it.
    fn coop_as_player_call(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>, pawn: ObjId) -> crate::Result<(Value, Vec<Option<Value>>)> {
        let Some(pc) = self.prop(pawn, "Controller").as_object() else { return self.call(f, this, args) };
        let saved = (self.prop(this, "XIIIPawn"), self.prop(this, "XIIIController"));
        self.set_prop(this, "XIIIPawn", Value::Object(Some(pawn)));
        self.set_prop(this, "XIIIController", Value::Object(Some(pc)));
        self.coop.shared_busy = true;
        let r = self.call(f, this, args);
        self.coop.shared_busy = false;
        if self.prop(this, "XIIIPawn").as_object() == Some(pawn) {
            self.set_prop(this, "XIIIPawn", saved.0);
        }
        if self.prop(this, "XIIIController").as_object() == Some(pc) {
            self.set_prop(this, "XIIIController", saved.1);
        }
        r
    }

    /// Once per co-op frame: the bosses that keep "the player" as their target look for the
    /// player to aim at, once a second.
    pub(crate) fn coop_shared_frame(&mut self) {
        if self.coop.players.len() < 2 || (self.level_time - self.coop.shared_time).abs() < BOSS_PERIOD {
            return;
        }
        self.coop.shared_time = self.level_time;
        let bosses: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.coop_object_alive(a) && BOSSES.iter().any(|b| self.is_a(a, b))).collect();
        for boss in bosses {
            self.coop_boss_target(boss);
        }
    }

    /// A boss's target (its variable XIII): a player it sees, the nearest one; it keeps the one
    /// it has unless that one is gone, out of sight while another is in sight, or much farther.
    fn coop_boss_target(&mut self, boss: ObjId) {
        let Some(body) = self.prop(boss, "Pawn").as_object().filter(|&p| self.coop_object_alive(p) && self.prop(p, "Health").as_int() > 0) else {
            return;
        };
        let Some(current) = self.prop(boss, "XIII").as_object() else { return };
        let pawns: Vec<ObjId> = (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).collect();
        let mut seen = Vec::new();
        for p in pawns {
            let sight = self.call_named(boss, "LineOfSightTo", vec![Value::Object(Some(p))]).is_ok_and(|v| v.as_bool());
            seen.push((p, sight, self.coop_distance(p, body)));
        }
        let Some((best, best_seen, best_distance)) = seen.iter().copied().min_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2))) else { return };
        let change = match seen.iter().copied().find(|s| s.0 == current) {
            None => true,
            Some((_, seen_now, distance)) => best != current && ((best_seen && !seen_now) || (best_seen == seen_now && best_distance < BOSS_CLOSER * distance)),
        };
        if !change {
            return;
        }
        self.set_prop(boss, "XIII", Value::Object(Some(best)));
        for field in ["Focus", "Enemy", "PeeredEnemy"] {
            if self.prop(boss, field).as_object() == Some(current) {
                self.set_prop(boss, field, Value::Object(Some(best)));
            }
        }
        self.log.push(format!("Log: Co-op: {} now aims at {}", self.path_name(boss), self.path_name(best)));
    }

    /// KelloAmbush starting: its player becomes the living player nearest to it.
    fn coop_ambush_player(&mut self, ambush: ObjId) {
        let nearest = (0..self.coop.players.len())
            .filter_map(|i| Some((self.coop.players[i], self.coop_living_pawn(i)?)))
            .min_by(|a, b| self.coop_distance(a.1, ambush).total_cmp(&self.coop_distance(b.1, ambush)));
        if let Some((pc, _)) = nearest {
            if self.prop(ambush, "PC").as_object() != Some(pc) {
                self.log.push(format!("Log: Co-op: {} plays the ambush {}", self.path_name(pc), self.path_name(ambush)));
            }
            self.set_prop(ambush, "PC", Value::Object(Some(pc)));
        }
    }
}
