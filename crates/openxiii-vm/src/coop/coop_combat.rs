//! Local co-op campaign, combat part (see coop.rs): enemies that deal with both players, a
//! player's death and return, and the team's defeat.
//!
//! What the solo scripts do, and what co-op changes:
//! - IAController keeps "the player" in its variable XIII (MapInfo.XIIIPawn at start) and
//!   attacks it with SetEnemy(XIII) whoever it saw, heard or was hit by. Co-op points XIII at
//!   the player the event is about, just before the script handles the event.
//! - XIIISoloGameRules.ScoreKill ends the game when a player dies (EndGame "PlayerKilled"), and
//!   LethalVolume when one falls ("PlayerFalling"). While another player is alive, co-op keeps
//!   the game running and brings the player back next to a teammate a few seconds later. When
//!   nobody is left, the solo ending runs as it is.

use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Seconds between a player's death and their return.
const RESPAWN_DELAY: f64 = 3.0;
/// Seconds between two checks of the enemies' "player" variable.
const AI_PERIOD: f64 = 0.25;
/// An enemy busy with one player turns to another one it sees only if that one is this much
/// closer (ratio of the distances).
const CLOSER: f32 = 0.7;

impl Vm {
    pub(crate) fn coop_object_alive(&self, o: ObjId) -> bool {
        self.objects.get(o).is_some_and(|x| x.is_some())
    }

    /// Living pawn of player `i` (not down, health above zero).
    pub(crate) fn coop_living_pawn(&self, i: usize) -> Option<ObjId> {
        if self.coop.down.get(i).copied().flatten().is_some() {
            return None;
        }
        let pc = *self.coop.players.get(i)?;
        if !self.coop_object_alive(pc) {
            return None;
        }
        let pawn = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p))?;
        (self.prop(pawn, "Health").as_int() > 0).then_some(pawn)
    }

    /// `hud` belongs to a co-op player who is down, waiting to come back.
    pub(crate) fn coop_owner_down(&self, hud: ObjId) -> bool {
        if !self.coop.on || self.coop.defeat {
            return false;
        }
        let Some(pc) = self.prop(hud, "PlayerOwner").as_object() else { return false };
        self.coop.players.iter().position(|&p| p == pc).is_some_and(|i| self.coop.down.get(i).copied().flatten().is_some())
    }

    /// Pawns of the living players.
    fn coop_living_pawns(&self) -> Vec<ObjId> {
        (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).collect()
    }

    pub(crate) fn coop_distance(&self, a: ObjId, b: ObjId) -> f32 {
        let (p, q) = (self.prop(a, "Location").as_vector(), self.prop(b, "Location").as_vector());
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
    }

    /// Before an enemy's script handles SeePlayer, HearNoise or DamageAttitudeTo: its "player"
    /// becomes the one the event is about.
    /// Returns true for such an event of an enemy (`coop_after_ai_event` follows the script).
    pub(crate) fn coop_before_call(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if self.coop.ai_event {
            return false;
        }
        if self.functions[f].name.eq_ignore_ascii_case("NotifyKilled") && self.is_a(this, "IAController") {
            let killed = args.get(2).and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&k| self.coop_object_alive(k));
            self.coop_enemy_carries_on(this, killed);
            return false;
        }
        let is_event =
            ["SeePlayer", "DamageAttitudeTo", "HearNoise"].iter().any(|n| self.functions[f].name.eq_ignore_ascii_case(n)) && self.is_a(this, "IAController");
        if is_event {
            self.coop_point_player(f, this, args);
        }
        is_event
    }

    /// After the script handled the event: it may have taken a new enemy itself.
    pub(crate) fn coop_after_ai_event(&mut self, this: ObjId) {
        if self.coop_object_alive(this) {
            self.coop_player_is_enemy(this);
        }
    }

    fn coop_point_player(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) {
        let name = self.functions[f].name.as_str();
        let (about, kind) = if name.eq_ignore_ascii_case("SeePlayer") {
            (args.first().and_then(|a| a.as_ref()).and_then(Value::as_object), 0)
        } else if name.eq_ignore_ascii_case("DamageAttitudeTo") {
            (args.first().and_then(|a| a.as_ref()).and_then(Value::as_object), 1)
        } else if name.eq_ignore_ascii_case("HearNoise") {
            let maker = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&m| self.coop_object_alive(m));
            (maker.and_then(|m| self.prop(m, "Instigator").as_object()), 2)
        } else {
            return;
        };
        let Some(about) = about else { return };
        if !self.is_a(this, "IAController") || !self.coop_living_pawns().contains(&about) {
            return;
        }
        // Its scripts compare Enemy with XIII all the time: while it fights a living player,
        // that player is its "player" (a guard fighting one player while its variable named
        // the other one hesitated instead of attacking).
        self.coop_player_is_enemy(this);
        if self.prop(this, "XIII").as_object() == Some(about) {
            return;
        }
        // Busy with another living player: only a hit, or a much closer player, changes that.
        let enemy = self.prop(this, "Enemy").as_object().filter(|&e| e != about && self.coop_living_pawns().contains(&e));
        // A noise made by the other player does not distract it from the one it fights.
        if kind == 2 && enemy.is_some() {
            return;
        }
        let switch = match (kind, enemy) {
            (_, None) | (1, _) => true,
            (0, Some(enemy)) => match self.prop(this, "Pawn").as_object() {
                // Much closer, or the one it was fighting is out of its sight.
                Some(own) => {
                    self.coop_distance(own, about) < CLOSER * self.coop_distance(own, enemy)
                        || !self.call_named(this, "CanSee", vec![Value::Object(Some(enemy))]).is_ok_and(|v| v.as_bool())
                }
                None => false,
            },
            _ => false,
        };
        if switch {
            self.set_prop(this, "XIII", Value::Object(Some(about)));
            // Busy with another player: several of its states only look at their current enemy
            // in SeePlayer and would ignore this player standing in front of them until they
            // lose the other one. The enemy is changed here, as the scripts do it themselves
            // (SetEnemy, then EnemyAcquired when it took).
            if enemy.is_some() {
                let _ = self.call_named(this, "SetEnemy", vec![Value::Object(Some(about))]);
                if self.coop_object_alive(this) && self.prop(this, "Enemy").as_object() == Some(about) {
                    let _ = self.call_named(this, "EnemyAcquired", vec![]);
                }
            }
        }
    }

    /// While a player's view and HUD are made, the other players' pawns stop being aiming
    /// targets (bProjTarget): otherwise the crosshair's target search picks the teammate, the
    /// weapon in hand swings towards them and their name shows under the crosshair. Returns
    /// what to put back; collisions and damage outside the drawing are untouched.
    pub(crate) fn coop_hide_teammate_targets(&mut self, viewer: Option<ObjId>) -> Vec<ObjId> {
        let mut hidden = Vec::new();
        for pc in self.coop.players.clone() {
            if Some(pc) == viewer || !self.coop_object_alive(pc) {
                continue;
            }
            let Some(pawn) = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p)) else { continue };
            if self.prop(pawn, "bProjTarget").as_bool() {
                self.set_prop(pawn, "bProjTarget", Value::Bool(false));
                hidden.push(pawn);
            }
        }
        hidden
    }

    pub(crate) fn coop_restore_teammate_targets(&mut self, hidden: Vec<ObjId>) {
        for pawn in hidden {
            if self.coop_object_alive(pawn) {
                self.set_prop(pawn, "bProjTarget", Value::Bool(true));
            }
        }
    }

    /// CineController2 (the actors of the scripted scenes) keeps "the player" in PC and Player,
    /// taken from MapInfo once, and many scenes wait until the player sees the actor, or no
    /// longer sees it (PC.CanSee). With the story player elsewhere, a scene the other player
    /// walks into never started. Its player becomes a living player who sees the actor, if
    /// there is one (so "seen" means seen by anybody, "not seen" by nobody), else the nearest
    /// player. Not while the scene holds its player (FreezePlayer, FreezePlayerLocation).
    fn coop_scene_player(&mut self, scene: ObjId) {
        if self.prop(scene, "bFrozenPlayer").as_bool() {
            return;
        }
        let Some(actor) = self.prop(scene, "Pawn").as_object().filter(|&p| self.coop_object_alive(p)) else { return };
        let current = self.prop(scene, "PC").as_object();
        // Nobody else while the scene holds its player in place (FreezePlayerLocation).
        if current.is_some_and(|pc| self.coop_object_alive(pc) && self.coop_held_state(pc)) {
            return;
        }
        // A player made the scene go on: theirs until another player does (coop_cine.rs).
        if let Some((pc, pawn)) = self.coop_scene_bound(scene) {
            if current != Some(pc) {
                let before = self.prop(scene, "Player").as_object();
                self.set_prop(scene, "PC", Value::Object(Some(pc)));
                self.set_prop(scene, "Player", Value::Object(Some(pawn)));
                if let Some(old) = before.filter(|&o| o != pawn) {
                    self.coop_scene_retarget(scene, old, pawn);
                }
            }
            return;
        }
        let mut players: Vec<(ObjId, ObjId)> = (0..self.coop.players.len()).filter_map(|i| Some((self.coop.players[i], self.coop_living_pawn(i)?))).collect();
        // The current one first (no change while it sees the actor), then the nearest ones.
        players.sort_by(|a, b| {
            (Some(a.0) != current).cmp(&(Some(b.0) != current)).then(self.coop_distance(a.1, actor).total_cmp(&self.coop_distance(b.1, actor)))
        });
        let seeing = players.iter().copied().find(|&(pc, _)| self.call_named(pc, "CanSee", vec![Value::Object(Some(actor))]).is_ok_and(|v| v.as_bool()));
        // Nobody sees it: the nearest player (coop_rules.rs).
        let nearest = players.iter().copied().min_by(|a, b| self.coop_distance(a.1, actor).total_cmp(&self.coop_distance(b.1, actor)));
        let chosen = seeing.or(nearest);
        if let Some((pc, pawn)) = chosen {
            if current != Some(pc) {
                let before = self.prop(scene, "Player").as_object();
                self.set_prop(scene, "PC", Value::Object(Some(pc)));
                self.set_prop(scene, "Player", Value::Object(Some(pawn)));
                if let Some(old) = before.filter(|&o| o != pawn) {
                    self.coop_scene_retarget(scene, old, pawn);
                }
            }
        }
    }

    /// An enemy fighting a living player has that player as its "player" (XIII).
    fn coop_player_is_enemy(&mut self, ai: ObjId) -> bool {
        let Some(enemy) = self.prop(ai, "Enemy").as_object().filter(|e| self.coop_living_pawns().contains(e)) else { return false };
        if self.prop(ai, "XIII").as_object() != Some(enemy) {
            self.set_prop(ai, "XIII", Value::Object(Some(enemy)));
        }
        true
    }

    /// XIIIGameInfo.EndGame(Winner, Reason) for a player's death or fall: `true` when co-op
    /// keeps the game running (the player is down and will come back).
    pub(crate) fn coop_keeps_game(&mut self, f: FuncId, args: &[Option<Value>]) -> bool {
        let function = &self.functions[f];
        if function.name != "EndGame" || !self.classes[function.owner].name.eq_ignore_ascii_case("XIIIGameInfo") || self.coop.defeat {
            return false;
        }
        let Some(winner) = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object) else { return false };
        let reason = match args.get(1).and_then(|a| a.as_ref()) {
            Some(Value::Str(s)) => s.to_string(),
            _ => return false,
        };
        let Some(i) = self.coop.players.iter().position(|&pc| self.coop_object_alive(pc) && self.prop(pc, "PlayerReplicationInfo").as_object() == Some(winner))
        else {
            return false;
        };
        let pc = self.coop.players[i];
        let pawn = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p));
        let dead = pawn.is_none_or(|p| self.prop(p, "Health").as_int() <= 0);
        // A death by an alarm laser and the like ends the mission for everybody, as in solo.
        if !((reason == "PlayerKilled" && dead) || reason == "PlayerFalling") {
            return false;
        }
        if self.coop.down.len() < self.coop.players.len() {
            self.coop.down.resize(self.coop.players.len(), None);
            self.coop.bodies.resize(self.coop.players.len(), None);
        }
        if self.coop.down[i].is_some() {
            return true;
        }
        let others = (0..self.coop.players.len()).any(|k| k != i && self.coop_living_pawn(k).is_some());
        if !others {
            // Nobody left: the solo ending. The players already down get their body back as
            // pawn, which the ending's cameras and HUD expect.
            self.coop.defeat = true;
            for k in 0..self.coop.players.len() {
                let (pc, body) = (self.coop.players[k], self.coop.bodies[k]);
                if let Some(body) = body.filter(|&b| self.coop_object_alive(b)) {
                    if self.coop_object_alive(pc) && self.prop(pc, "Pawn").as_object().is_none() {
                        self.set_prop(pc, "Pawn", Value::Object(Some(body)));
                    }
                }
            }
            self.log.push("Log: Co-op: team defeated".to_string());
            return false;
        }
        self.coop.down[i] = Some(self.level_time);
        self.coop.bodies[i] = pawn;
        // The body is no longer a player for the map: in solo a death ends the game, in co-op
        // the body stays where it fell and, still an XIIIPlayerPawn, kept touching the
        // triggers and scenes there ("wait player": Wax's suicide began for a body while both
        // players were at the start). It leaves them and touches nothing more.
        if let Some(body) = pawn {
            self.coop_body_untouched(body);
        }
        // In solo the death view is GameEndedDeath's; co-op leaves the controller in state Dead,
        // whose behind view keeps the field of view of the moment: a player killed while
        // aiming through a scope would watch their body through the zoom (the body seemed to
        // fill the screen and sink). PlayerController.ResetFOV, as the scripts do on leaving a
        // zoomed view.
        if self.coop_object_alive(pc) {
            let _ = self.call_named(pc, "ResetFOV", vec![]);
        }
        self.log.push(format!("Log: Co-op: player {} down ({reason})", i + 1));
        true
    }

    /// GameInfo.RestartPlayer(aPlayer) for a player who is down: the dead controller's own
    /// restart (fire or jump in state Dead → ServerReStartPlayer) would put them back at the
    /// map's first start, then `coop_respawn` would move them next to a teammate a few seconds
    /// later. `true`: ignored, the player comes back once, next to a teammate.
    pub(crate) fn coop_waits_respawn(&self, f: FuncId, args: &[Option<Value>]) -> bool {
        let function = &self.functions[f];
        if function.name != "RestartPlayer" || self.coop.defeat {
            return false;
        }
        let owner = &self.classes[function.owner].name;
        if !(owner.eq_ignore_ascii_case("XIIIGameInfo") || owner.eq_ignore_ascii_case("GameInfo")) {
            return false;
        }
        let Some(pc) = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object) else { return false };
        self.coop.players.iter().position(|&p| p == pc).is_some_and(|i| self.coop.down.get(i).is_some_and(Option::is_some))
    }

    /// Controller.NotifyKilled(Killer, Killed, Other) clears Enemy when Other was it. For an
    /// enemy whose target, a player, has just been killed while it can see another living
    /// player: it carries on with that one at once. Left alone, it would drop back to an idle
    /// state and be "surprised" a moment later by a player it was already shooting at. An
    /// enemy that sees nobody else is left to its scripts (it can be surprised again).
    fn coop_enemy_carries_on(&mut self, ai: ObjId, killed: Option<ObjId>) {
        let Some(killed) = killed else { return };
        if self.prop(ai, "Enemy").as_object() != Some(killed) || !self.is_a(killed, "XIIIPlayerPawn") {
            return;
        }
        let own = self.prop(ai, "Pawn").as_object().filter(|&p| self.coop_object_alive(p));
        let mut seen: Vec<ObjId> = self
            .coop_living_pawns()
            .into_iter()
            .filter(|&p| p != killed && self.call_named(ai, "CanSee", vec![Value::Object(Some(p))]).is_ok_and(|v| v.as_bool()))
            .collect();
        if let Some(own) = own {
            seen.sort_by(|&p, &q| self.coop_distance(own, p).total_cmp(&self.coop_distance(own, q)));
        }
        if let Some(&next) = seen.first() {
            self.set_prop(ai, "XIII", Value::Object(Some(next)));
            let _ = self.call_named(ai, "SetEnemy", vec![Value::Object(Some(next))]);
            // SetEnemy keeps an enemy it is fighting (measured): the enemy is replaced the way
            // NotifyKilled itself is about to clear it, by writing the variable.
            if self.prop(ai, "Enemy").as_object() != Some(next) {
                self.set_prop(ai, "Enemy", Value::Object(Some(next)));
            }
        }
    }

    /// Once per frame: returns of the players who are down, upkeep of the enemies' "player".
    pub(crate) fn coop_combat_frame(&mut self) {
        self.coop_story_frame();
        self.coop_end_menu_frame();
        let now = self.level_time;
        if self.coop.defeat {
            // One death screen for the team, from the player who died last (their HUD has the
            // message, coop_end.rs): their viewport takes the whole window, as the game's own
            // split-screen menus do for the first one (UWindowsViewport::Exec).
            if !std::mem::replace(&mut self.coop.defeat_shown, true) {
                let last = self.coop.end.shown_by.filter(|_| true);
                let slot = last.and_then(|r| self.coop.players.get(r).copied()).filter(|&pc| self.coop_object_alive(pc)).and_then(|pc| self.viewport_slot_of(pc));
                match slot {
                    Some(slot) if self.engine.client.wanted != 1 => self.engine.client.extended = Some(slot),
                    _ => {
                        if let Some(&pc) = self.coop.players.first().filter(|&&pc| self.coop_object_alive(pc)) {
                            let _ = self.call_named(pc, "ConsoleCommand", vec![Value::Str("ExtendViewportBeforeMenu".into())]);
                        }
                    }
                }
            }
            return;
        }
        for i in 0..self.coop.down.len() {
            if self.coop.down[i].is_some_and(|t| now - t >= RESPAWN_DELAY) && self.coop_respawn(i) {
                self.coop.down[i] = None;
                self.coop.bodies[i] = None;
            }
        }
        self.coop_hook_orphans();
        // The carcass's own scripts (XIIIPawn.ReduceMyCylinder, on landing) give it back its
        // collision: kept off for as long as the player is down.
        for i in 0..self.coop.bodies.len() {
            if let Some(body) = self.coop.bodies[i].filter(|&b| self.coop_object_alive(b) && self.prop(b, "bCollideActors").as_bool()) {
                self.coop_body_untouched(body);
            }
        }
        if now - self.coop.ai_time < AI_PERIOD && now >= self.coop.ai_time {
            return;
        }
        self.coop.ai_time = now;
        let living = self.coop_living_pawns();
        if living.is_empty() {
            return;
        }
        for a in self.actors.clone() {
            if !self.coop_object_alive(a) {
                continue;
            }
            if self.is_a(a, "CineController2") {
                self.coop_scene_player(a);
                self.coop_scene_follow(a);
                continue;
            }
            if !self.is_a(a, "IAController") {
                continue;
            }
            if self.coop_player_is_enemy(a) || self.prop(a, "XIII").as_object().is_some_and(|x| living.contains(&x)) {
                continue;
            }
            // Its "player" is dead or gone: the nearest living one.
            let own = self.prop(a, "Pawn").as_object().filter(|&p| self.coop_object_alive(p));
            let nearest = match own {
                Some(own) => living.iter().copied().min_by(|&p, &q| self.coop_distance(own, p).total_cmp(&self.coop_distance(own, q))),
                None => living.first().copied(),
            };
            if let Some(nearest) = nearest {
                self.set_prop(a, "XIII", Value::Object(Some(nearest)));
            }
        }
    }

    /// Gives `pawn` the kinds of weapons and items `from` carries (not the ammunition counts),
    /// through MapInfo.GiveSomething as the story does. A key (Keys, but not a LockPick) also
    /// gets the first one's KeyCodeName and ItemName: a key given with other values than its
    /// class's own (a Select level start) opens the same doors for both players.
    pub(crate) fn coop_copy_equipment(&mut self, map_info: ObjId, from: ObjId, pawn: ObjId) {
        let mut items = Vec::new();
        let mut item = self.prop(from, "Inventory").as_object();
        while let Some(o) = item.filter(|&o| self.coop_object_alive(o) && items.len() < 200) {
            if !self.is_a(o, "Ammunition") && !self.is_a(o, "XIIIThingsToSave") {
                if let Some(c) = self.objects[o].as_ref().map(|x| x.class).filter(|&c| !self.coop_single_item(c)) {
                    items.push((o, c));
                }
            }
            item = self.prop(o, "Inventory").as_object();
        }
        self.coop.sharing = true;
        for (source, class) in items {
            let given = self.call_named(map_info, "GiveSomething", vec![Value::Class(Some(class)), Value::Object(Some(pawn))]);
            if self.is_a(source, "Keys") && !self.is_a(source, "LockPick") {
                let received = given.ok().and_then(|v| v.as_object()).filter(|&o| self.coop_object_alive(o)).or_else(|| {
                    self.call_named(pawn, "FindInventoryType", vec![Value::Class(Some(class))]).ok().and_then(|v| v.as_object()).filter(|&o| self.coop_object_alive(o))
                });
                if let Some(key) = received.filter(|&k| k != source) {
                    for name in ["KeyCodeName", "ItemName"] {
                        let v = self.prop(source, name);
                        self.set_prop(key, name, v);
                    }
                }
            }
        }
        self.coop.sharing = false;
    }

    /// Pawn `a` was just placed (SetLocation, a scene sliding its player, a co-op move) over
    /// the pawn `o` of another co-op player. In the game such a pawn is crushed (Pawn's
    /// EncroachedBy → gibbedBy): in solo nobody else stands on a player's way. In co-op the
    /// other player steps aside to the nearest free place instead (Plage01's opening scene
    /// slid the host's player out of bed onto the network guest and killed them). `true`: done,
    /// no EncroachedBy.
    pub(crate) fn coop_player_steps_aside(&mut self, a: ObjId, o: ObjId) -> bool {
        if self.coop.players.len() < 2 || !self.is_a(a, "Pawn") || !self.is_a(o, "Pawn") {
            return false;
        }
        let player = |vm: &Vm, p: ObjId| vm.prop(p, "Controller").as_object().is_some_and(|c| vm.coop.players.contains(&c));
        if !player(self, a) || !player(self, o) || self.prop(o, "Health").as_int() <= 0 {
            return false;
        }
        for place in self.coop_places_near(o) {
            if self.call_named(o, "SetLocation", vec![Value::vector(place[0], place[1], place[2])]).is_ok_and(|v| v.as_bool()) {
                self.set_prop(o, "Velocity", Value::vector(0.0, 0.0, 0.0));
                break;
            }
        }
        self.log.push(format!("Log: Co-op: {} stepped aside for {}", self.path_name(o), self.path_name(a)));
        true
    }

    /// A down player's body leaves what it touches and touches nothing more (no actor
    /// collision): it is no longer a player for the map's triggers and scenes.
    fn coop_body_untouched(&mut self, body: ObjId) {
        for other in self.touching_of(body) {
            self.end_touch(body, other, false);
        }
        let _ = self.call_named(body, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
    }

    /// HookProjectile.ProcessTouch with another player's pawn: in the game any actor touched
    /// on the way stops the grapple, so the hook thrown at a point where a teammate hangs fell
    /// on the teammate. In co-op a grapple goes through the players (each one their own rope
    /// on the same hook point); the call is skipped.
    pub(crate) fn coop_hook_passes(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if !self.functions[f].name.eq_ignore_ascii_case("ProcessTouch") || !self.is_a(this, "HookProjectile") {
            return false;
        }
        let Some(other) = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&o| self.coop_object_alive(o)) else { return false };
        if !self.is_a(other, "Pawn") || self.prop(this, "Instigator").as_object() == Some(other) {
            return false;
        }
        self.prop(other, "Controller").as_object().is_some_and(|c| self.coop.players.contains(&c))
    }

    /// A player who died hanging from the grapple: in the game a death ends the game, so
    /// nothing ever cleans the grapple of a dead player. In co-op the controller lives on: its
    /// rope (projectile, links, mechanism, harness) goes with the dead pawn's grapple, and the
    /// controller is no longer hooked, so that the grapple of the next pawn works.
    fn coop_hook_orphans(&mut self) {
        for i in 0..self.coop.players.len() {
            let pc = self.coop.players[i];
            if !self.coop_object_alive(pc) {
                continue;
            }
            let hook = self.prop(pc, "HookUsed").as_object().filter(|&h| self.coop_object_alive(h));
            if hook.is_none() && !self.prop(pc, "bHooked").as_bool() {
                continue;
            }
            let pawn = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p) && self.prop(p, "Health").as_int() > 0);
            let owner = hook.and_then(|h| self.prop(h, "Owner").as_object());
            if pawn.is_some() && (hook.is_none() || owner == pawn) {
                continue;
            }
            if let Some(h) = hook.filter(|_| owner.is_none_or(|o| Some(o) != pawn)) {
                // The dead pawn's grapple: its Destroyed takes the projectile, the links, the
                // mechanism and the harness with it.
                let _ = self.call_named(h, "Destroy", vec![]);
            }
            self.set_prop(pc, "bHooked", Value::Bool(false));
            self.set_prop(pc, "HookUsed", Value::Object(None));
            self.log.push(format!("Log: Co-op: player {}'s grapple released with the dead pawn", i + 1));
        }
    }

    /// Brings player `i` back next to a living teammate. `false`: no teammate or no free place
    /// right now (tried again on the next frame).
    fn coop_respawn(&mut self, i: usize) -> bool {
        let pc = self.coop.players[i];
        if !self.coop_object_alive(pc) {
            return false;
        }
        let Some(mate) = (0..self.coop.players.len()).filter(|&k| k != i).find_map(|k| self.coop_living_pawn(k)) else { return false };
        let Some(game) = self.level.and_then(|l| self.prop(l, "Game").as_object()) else { return false };
        let facing = Value::rotator(0, self.prop(mate, "Rotation").as_rotator()[1], 0);
        // A player who fell is still alive: moved back, not replaced.
        let fallen = self.prop(pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p) && self.prop(p, "Health").as_int() > 0);
        let class = self.objects[mate].as_ref().map(|o| o.class);
        let mut pawn = None;
        for place in self.coop_places_near(mate) {
            let place = Value::vector(place[0], place[1], place[2]);
            match fallen {
                Some(p) => {
                    if self.call_named(p, "SetLocation", vec![place]).is_ok_and(|v| v.as_bool()) {
                        self.set_prop(p, "Velocity", Value::vector(0.0, 0.0, 0.0));
                        pawn = Some(p);
                        break;
                    }
                }
                None => {
                    let Some(class) = class else { return false };
                    let mut args =
                        [Value::Class(Some(class)), Value::Object(None), Value::Name(crate::value::NameStruct::from(String::new())), place, facing.clone()];
                    if let Some(p) = crate::natives::call(self, "Spawn", game, &mut args, &[true, false, false, true, true]).ok().and_then(|v| v.as_object()) {
                        pawn = Some(p);
                        break;
                    }
                }
            }
        }
        let Some(pawn) = pawn else { return false };
        if fallen.is_some() {
            self.log.push(format!("Log: Co-op: player {} back next to a teammate", i + 1));
            return true;
        }
        // New pawn: possessed like at the start of a map.
        self.set_prop(pc, "Pawn", Value::Object(None));
        if let Err(e) = self.call_named(pc, "Possess", vec![Value::Object(Some(pawn))]) {
            self.log.push(format!("Log: Co-op: Possess failed: {e}"));
            return false;
        }
        self.set_prop(pc, "bBehindView", Value::Bool(false));
        // As Audio->SetViewport does for a viewport's pawn at the start of a map: its own
        // sounds have no position (heard at full volume by its player). Not for a player without
        // a viewport here: heard where they stand.
        if self.coop_is_local(pc) {
            for flag in ["bHasRollOff", "bHasPosition"] {
                self.set_prop(pawn, flag, Value::Bool(false));
            }
        }
        let _ = self.call_named(pc, "ClientSetRotation", vec![facing]);
        // Equipment: the map's starting equipment, then what the teammate carries (weapons and
        // items, so that nobody is left without the grapple or a key).
        let map_info = self.prop(game, "MapInfo").as_object().filter(|&m| self.coop_object_alive(m));
        if let Some(map_info) = map_info {
            self.coop.sharing = true;
            let _ = self.call_named(map_info, "SetUpInitialInventory", vec![Value::Object(Some(pawn))]);
            self.coop.sharing = false;
            self.coop_copy_equipment(map_info, mate, pawn);
            // The story player's pawn is the one the map's scripts know.
            if i == 0 {
                self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
            }
        }
        // Weapon in hand: the teammate's kind if the new pawn has it.
        let wanted = self.prop(mate, "Weapon").as_object().filter(|&w| self.coop_object_alive(w)).and_then(|w| self.objects[w].as_ref().map(|x| x.class));
        if let Some(class) = wanted {
            if let Some(weapon) = self.call_named(pawn, "FindInventoryType", vec![Value::Class(Some(class))]).ok().and_then(|v| v.as_object()) {
                for flag in ["bWeaponMode", "bWaitForWeaponMode"] {
                    self.set_prop(pc, flag, Value::Bool(true));
                }
                self.set_prop(pc, "bWeaponBlock", Value::Bool(false));
                self.set_prop(pawn, "PendingWeapon", Value::Object(Some(weapon)));
                let _ = self.call_named(pawn, "ChangedWeapon", vec![]);
            }
        }
        self.log.push(format!("Log: Co-op: player {} back", i + 1));
        true
    }
}
