//! Co-op (two players or more): the end of a map (the blue "mission complete" box, or a game
//! over box) is shown from the player who ended it.
//!
//! XIIIGameInfo.EndGame (MapInfo.DoTravel's "GoalComplete", SetGoalComplete's anti-goals...)
//! gives its box (XIIIMissionCompletedMessage / XIIIEndGameMessage, XIIIBaseHud.LocalizedMessage
//! then AddHudEndMessage) to MapInfo.XIIIController's HUD: the player who set the end off when
//! the co-op made them MapInfo's player for that call (coop_acting_begin, scenes), the story
//! player otherwise. The co-op's end picture (one view for the whole window, coop_story.rs) was
//! the player of the last cinematic: the first player's view, without the box, when the second
//! one ended the map. Here:
//! - the end picture is the view of the player whose HUD got the box.
//!
//! A mission failure (an anti-goal: an alarm, an ally killed, an objective missed; a death that
//! ends the mission; the team's death) is shown the same way, from the player who caused it, and
//! both players use its menu:
//! - EndGame's message goes to MapInfo.XIIIController's HUD, the story player when the failure
//!   came from a script that does not say who caused it (MapInfo.SetGoalComplete, an alarm, a
//!   goal trigger's timer). That player is found from EndGame's Winner (a death, a fall, a laser),
//!   else from the scripts running at that moment (a player, the player an enemy was dealing with,
//!   the player who has just hit the pawn that is dying), and is MapInfo's player for the call;
//! - every local player's controller opens its own failure menu (GameEnded's ShowTheMenu), but only
//!   the view in the whole window is seen: the keys of every local player go to that view's menu.

use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// Seconds after a player's kill when a failure without a named player is still that player's.
const RECENT_KILL: f64 = 1.0;

#[derive(Debug, Default, Clone)]
pub(crate) struct CoopEnd {
    /// Rank of the local player whose HUD shows the end box.
    pub(crate) shown_by: Option<usize>,
    /// The box's LocalizedMessage arguments.
    pub(crate) message: Option<Vec<Option<Value>>>,
    /// An EndGame call is running.
    busy: bool,
    /// Last hit on a pawn that is not a player's: that pawn, the player who hit it (None: not a
    /// player) and the level time.
    hit: Option<(ObjId, Option<usize>, f64)>,
    /// The viewport whose menu every local player's keys go to (the failure menu in the whole
    /// window), set once per frame.
    pub(crate) menu_viewport: Option<ObjId>,
}

impl Vm {
    /// Before HUD.LocalizedMessage in co-op: notes an end box given to a local player's HUD.
    pub(crate) fn coop_end_note_message(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) {
        if self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("LocalizedMessage") {
            return;
        }
        let Some(Value::Class(Some(class))) = args.first().and_then(|a| a.as_ref()) else { return };
        if !crate::coop_story::END_MESSAGES.iter().any(|m| self.classes[*class].name.eq_ignore_ascii_case(m)) || !self.is_a(this, "XIIIBaseHud") {
            return;
        }
        let rank = self.coop.players.iter().position(|&pc| self.alive(pc) && self.coop_is_local(pc) && self.prop(pc, "MyHUD").as_object() == Some(this));
        if let Some(rank) = rank {
            if self.coop.end.shown_by.is_none() {
                self.coop.end.shown_by = Some(rank);
                self.log.push(format!("Log: Co-op: end box on player {}'s HUD", rank + 1));
            }
            self.coop.end.message = Some(args.to_vec());
        }
    }

    /// Before Pawn.TakeDamage(Damage, instigatedBy, ...) in co-op: who hit a pawn that is not a
    /// player's (the failure of a "must not be killed" character, XIIIPawn.TakeDamage).
    pub(crate) fn coop_end_note_hit(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) {
        if self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("TakeDamage") {
            return;
        }
        if !self.is_a(this, "Pawn") || self.coop_player_of(this).is_some() {
            return;
        }
        let by = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object);
        let rank = by.and_then(|p| self.coop_player_of(p));
        self.coop.end.hit = Some((this, rank, self.level_time));
    }

    /// Rank of the co-op player whose controller or pawn `o` is.
    fn coop_player_of(&self, o: ObjId) -> Option<usize> {
        self.coop.players.iter().position(|&pc| self.alive(pc) && (pc == o || self.prop(pc, "Pawn").as_object() == Some(o)))
    }

    /// The player an actor running a script stands for: a player, a hit pawn's attacker, an
    /// enemy's opponent.
    fn coop_fail_actor(&self, o: ObjId) -> Option<usize> {
        if !self.alive(o) {
            return None;
        }
        if let Some(rank) = self.coop_player_of(o) {
            return Some(rank);
        }
        if let Some((pawn, rank, _)) = self.coop.end.hit {
            if pawn == o {
                return rank;
            }
        }
        let controller = if self.is_a(o, "Controller") { Some(o) } else { self.prop(o, "Controller").as_object().filter(|&c| self.alive(c)) };
        let ai = controller.filter(|&c| self.is_a(c, "IAController"))?;
        ["Enemy", "XIII"].iter().find_map(|v| self.prop(ai, v).as_object().and_then(|p| self.coop_player_of(p)))
    }

    /// Rank of the player who set a mission failure off (`reason` is EndGame's).
    fn coop_fail_culprit(&self, map_info: ObjId, winner: Option<ObjId>, reason: &str) -> Option<usize> {
        let winner =
            winner.and_then(|pri| self.coop.players.iter().position(|&pc| self.alive(pc) && self.prop(pc, "PlayerReplicationInfo").as_object() == Some(pri)));
        // A death, a fall: the player it is about.
        if reason == "PlayerKilled" || reason == "PlayerFalling" {
            return winner;
        }
        // A player other than the story player is already MapInfo's (coop_acting_begin), or
        // EndGame names them.
        let acting = self.prop(map_info, "XIIIController").as_object().and_then(|pc| self.coop_player_of(pc));
        if let Some(rank) = acting.filter(|&r| r > 0).or(winner.filter(|&r| r > 0)) {
            return Some(rank);
        }
        // The scripts running, the innermost first, and what set them off.
        let running: Vec<ObjId> = self.this_stack.iter().rev().copied().chain(self.state_code_obj).collect();
        for o in running {
            if let Some(rank) = self.coop_fail_actor(o) {
                return Some(rank);
            }
            let instigator = if self.alive(o) { self.prop(o, "Instigator").as_object() } else { None };
            if let Some(rank) = instigator.and_then(|i| self.coop_fail_actor(i)) {
                return Some(rank);
            }
        }
        // A map's script that watches its characters (SSH101a's MapInfo.Tick: a soldier who must
        // not be killed is found dead a frame later): the player who has just killed one.
        if let Some((pawn, Some(rank), at)) = self.coop.end.hit {
            let dead = !self.alive(pawn) || self.prop(pawn, "Health").as_int() <= 0 || self.prop(pawn, "bIsDead").as_bool();
            if dead && self.level_time - at <= RECENT_KILL {
                return Some(rank);
            }
        }
        acting.or(winner)
    }

    /// XIIIGameInfo.EndGame in co-op (not a down player's: coop_keeps_game). `None`: not it.
    pub(crate) fn coop_end_call(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<Result<(Value, Vec<Option<Value>>)>> {
        if self.coop.end.busy || self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("EndGame") {
            return None;
        }
        if !self.classes[self.functions[f].owner].name.eq_ignore_ascii_case("XIIIGameInfo") {
            return None;
        }
        let map_info = self.coop_story_map_info()?;
        let reason = match args.get(1).and_then(|a| a.as_ref()) {
            Some(Value::Str(s)) => s.split(':').next().unwrap_or_default().to_string(),
            _ => String::new(),
        };
        // A failure: the player who caused it is MapInfo's player for the call (their HUD gets
        // the message, their view the whole window).
        let story = self.prop(map_info, "XIIIController");
        let mut culprit_set = None;
        if reason != "GoalComplete" && !self.prop(this, "bGameEnded").as_bool() {
            let winner = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object);
            if let Some(pc) = self.coop_fail_culprit(map_info, winner, &reason).and_then(|r| self.coop.players.get(r).copied()) {
                if story.as_object() != Some(pc) {
                    self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
                    culprit_set = Some(pc);
                }
                let rank = self.coop_player_of(pc).map_or(0, |r| r + 1);
                self.log.push(format!("Log: Co-op: mission failure ({reason}) caused by player {rank}"));
            }
        }
        self.coop.end.busy = true;
        let r = self.call(f, this, args.to_vec());
        self.coop.end.busy = false;
        if culprit_set.is_some() && self.alive(map_info) && self.prop(map_info, "XIIIController").as_object() == culprit_set {
            self.set_prop(map_info, "XIIIController", story);
        }
        Some(r)
    }

    /// Rank of the player whose view is the end picture.
    pub(crate) fn coop_end_leader(&self) -> usize {
        self.coop.end.shown_by.unwrap_or(self.coop.cine_last_leader)
    }

    /// Once per frame: the failure menu of the view in the whole window, which every local
    /// player's keys go to (`coop_end_key_viewport`).
    pub(crate) fn coop_end_menu_frame(&mut self) {
        self.coop.end.menu_viewport = None;
        if self.coop.players.len() < 2 {
            return;
        }
        let Some(slot) = self.engine.client.extended else { return };
        let ended = self.level.and_then(|l| self.prop(l, "Game").as_object()).is_some_and(|g| self.alive(g) && self.prop(g, "bGameEnded").as_bool());
        if !ended {
            return;
        }
        let shown = self.coop.players.iter().copied().find(|&pc| self.alive(pc) && self.coop_is_local(pc) && self.viewport_slot_of(pc) == Some(slot));
        let Some(pc) = shown else { return };
        if self.coop_menu_open(pc) {
            self.coop.end.menu_viewport = self.prop(pc, "Player").as_object().filter(|&v| self.alive(v));
        }
    }

    /// Viewport whose menus get a key event (UInteractionMaster::MasterProcessKeyEvent): the
    /// failure menu in the whole window, whichever local player pressed the key; None: the
    /// player's own (as the game does).
    pub(crate) fn coop_end_key_viewport(&self) -> Option<ObjId> {
        self.coop.end.menu_viewport.filter(|&v| self.alive(v))
    }

    /// A local player's gamepad moves in the failure menu in the whole window.
    pub(crate) fn coop_end_menu_open(&self, pc: ObjId) -> bool {
        self.coop_end_key_viewport().is_some() && self.coop.players.contains(&pc) && self.coop_is_local(pc)
    }
}

