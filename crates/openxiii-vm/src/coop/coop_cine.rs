//! Local and network co-op campaign: one rule for every scripted scene and cinematic, whatever
//! the map (see coop.rs, coop_story.rs, coop_rules.rs).
//!
//! - The player who makes a scene go on is that scene's player from then on: a player's pawn
//!   touching a scene that waits for "the player" (CineController2.Touch / UnTouch: "wait
//!   player", "wait playeraway", "on player"), a trigger touched by a player that wakes a scene
//!   (CineController2.Trigger: "wait event", "on event"; Cine2.Trigger: a reaction started),
//!   with the player as the event's instigator. The scene's PC and Player then name that player
//!   for everything the scene does to "the player" (FreezePlayer, its camera, the place it moves
//!   the player to, gifts, player events, the events it causes: coop_world.rs gives MapInfo's
//!   player the scene's player for them). The scene keeps that player until another player makes
//!   it go on again; while it holds its player in place nobody else is taken.
//! - A cinematic (Level.bCineFrame, the scene's player in state NoControl) is that player's:
//!   their camera is shown (whole window in split screen, the view of every player on another
//!   machine), and every other player is parked for its length: no control (state NoControl,
//!   as the original does to its lone player), pawn hidden and passing through the actors, so
//!   that it is neither seen in the shot nor in the way of the script. At the end the parked
//!   players get their control and pawn back, next to the scene's player (coop_story.rs).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// A player parked during another player's cinematic: what to give back.
#[derive(Debug, Clone)]
pub(crate) struct Parked {
    pc: ObjId,
    pawn: ObjId,
    hidden: bool,
    collide: bool,
    block_actors: bool,
    block_players: bool,
    /// Put in state NoControl here (not by a scene's hold).
    stopped: bool,
    draw_type: Value,
    physics: Value,
    /// Actors drawn with the pawn (weapon in the hands of the third person view, shadow...):
    /// hidden too, with their own bHidden.
    attached: Vec<(ObjId, bool)>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CoopCine {
    /// Scenes and the player who last made them go on: (CineController2, player controller).
    pub(crate) bound: Vec<(ObjId, ObjId)>,
    pub(crate) parked: Vec<Parked>,
    /// A MapInfo call is being watched (coop_imposed_begin); each player's `IMPOSED` values on
    /// the last tick.
    pub(crate) imposing: bool,
    pub(crate) imposed_seen: Vec<Option<Vec<Value>>>,
    /// What MapInfo imposed on its player, for the players who join later.
    pub(crate) imposed_story: Vec<(&'static str, Value)>,
    /// Scenes triggered before they were ready: (Cine2, Other, EventInstigator).
    pub(crate) early: Vec<(ObjId, Option<ObjId>, Option<ObjId>)>,
    /// The flashback's player's own pawn, hidden during the flashback (coop_flash_owner_begin).
    pub(crate) flash_real: Option<ObjId>,
    /// The player who last made a scene go on (coop_scene_take).
    pub(crate) last_player: Option<ObjId>,
    /// A focus window's closing is being run (coop_focus_window_begin).
    pub(crate) focus_busy: bool,
    /// Parked pawns held still (PHYS_None) by the parking, given their physics back after.
    pub(crate) parked_frozen: Vec<ObjId>,
}

const SCENE_EVENTS: [&str; 3] = ["Touch", "UnTouch", "Trigger"];

impl Vm {
    /// Before an event of a scene (co-op): a player making it go on becomes its player.
    pub(crate) fn coop_scene_bind(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) {
        let name = self.functions[f].name.as_str();
        if !SCENE_EVENTS.iter().any(|e| name.eq_ignore_ascii_case(e)) {
            return;
        }
        // A scene woken before it is ready (Cine2.Play does nothing until bInitialized): in co-op
        // the second player can stand in the trigger of a scene from the start of the map
        // (SPADS02a's showers), the event comes on the first frames and would be lost. It is
        // given again once the scene is ready (coop_scene_wake_frame).
        if name.eq_ignore_ascii_case("Trigger") && self.is_a(this, "Cine2") && !self.prop(this, "bInitialized").as_bool() {
            let other = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object);
            let by = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object);
            if !self.coop.cine_rules.early.iter().any(|e| e.0 == this) {
                self.coop.cine_rules.early.push((this, other, by));
            }
        }
        if self.coop.players.len() < 2 {
            return;
        }
        let name = self.functions[f].name.as_str();
        let scene = if self.is_a(this, "CineController2") {
            this
        } else if name.eq_ignore_ascii_case("Trigger") && self.is_a(this, "Cine2") {
            match self.prop(this, "CineController").as_object().filter(|&c| self.coop_object_alive(c) && self.is_a(c, "CineController2")) {
                Some(c) => c,
                None => return,
            }
        } else {
            return;
        };
        // The last parameter that is a player (Trigger: Other, then EventInstigator).
        let mut found = None;
        for object in args.iter().filter_map(|a| a.as_ref()).filter_map(Value::as_object) {
            for i in 0..self.coop.players.len() {
                let pc = self.coop.players[i];
                if !self.coop_object_alive(pc) {
                    continue;
                }
                let pawn = self.coop_living_pawn(i);
                if object == pc || (pawn.is_some() && pawn == Some(object)) {
                    found = pawn.map(|p| (pc, p));
                }
            }
        }
        let Some((pc, pawn)) = found else { return };
        let why = name.to_string();
        self.coop_scene_take(scene, pc, pawn, &why);
    }

    /// Scene `scene` acts with player `pc` (pawn `pawn`) from now on.
    fn coop_scene_take(&mut self, scene: ObjId, pc: ObjId, pawn: ObjId, why: &str) {
        let current = self.prop(scene, "PC").as_object().filter(|&c| self.coop_object_alive(c));
        // A scene holding its player keeps it.
        if current.is_some_and(|c| c != pc && self.coop_held_state(c)) {
            return;
        }
        self.coop.cine_rules.bound.retain(|b| b.0 != scene);
        self.coop.cine_rules.bound.push((scene, pc));
        self.coop.cine_rules.last_player = Some(pc);
        if current != Some(pc) || self.prop(scene, "Player").as_object() != Some(pawn) {
            let _ = why;
            let before = self.prop(scene, "Player").as_object();
            self.set_prop(scene, "PC", Value::Object(Some(pc)));
            self.set_prop(scene, "Player", Value::Object(Some(pawn)));
            if let Some(old) = before.filter(|&o| o != pawn) {
                self.coop_scene_retarget(scene, old, pawn);
            }
        }
    }

    /// The player a scene was given by `coop_scene_bind`, while that player is in play.
    pub(crate) fn coop_scene_bound(&self, scene: ObjId) -> Option<(ObjId, ObjId)> {
        let pc = self.coop.cine_rules.bound.iter().find(|b| b.0 == scene)?.1;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        Some((pc, self.coop_living_pawn(rank)?))
    }

    /// Start of a cinematic of player `leader`: the other players are parked.
    pub(crate) fn coop_cine_park(&mut self, leader: usize) {
        for i in 0..self.coop.players.len() {
            let pc = self.coop.players[i];
            if i == leader || !self.coop_object_alive(pc) || self.coop.cine_rules.parked.iter().any(|p| p.pc == pc) {
                continue;
            }
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            let parked = Parked {
                pc,
                pawn,
                hidden: self.prop(pawn, "bHidden").as_bool(),
                collide: self.prop(pawn, "bCollideActors").as_bool(),
                block_actors: self.prop(pawn, "bBlockActors").as_bool(),
                block_players: self.prop(pawn, "bBlockPlayers").as_bool(),
                stopped: !self.coop_held_state(pc),
                draw_type: self.prop(pawn, "DrawType"),
                physics: self.prop(pawn, "Physics"),
                attached: self
                    .actors
                    .iter()
                    .copied()
                    .filter(|&a| a != pawn && a != pc && self.coop_object_alive(a))
                    .filter(|&a| self.prop(a, "Base").as_object() == Some(pawn) || (self.prop(a, "Owner").as_object() == Some(pawn) && !self.is_a(a, "Inventory")))
                    .map(|a| (a, self.prop(a, "bHidden").as_bool()))
                    .collect(),
            };
            let _ = self.call_named(pc, "StopFiring", vec![]);
            if parked.stopped {
                let _ = self.call_named(pc, "GotoState", vec![Value::Name("NoControl".into())]);
            }
            // Not drawn at all, nor what goes with it (a parked player must never be seen in
            // the scene's camera).
            self.set_prop(pawn, "bHidden", Value::Bool(true));
            let _ = self.call_named(pawn, "SetDrawType", vec![Value::Byte(0)]);
            for &(a, _) in &parked.attached {
                self.set_prop(a, "bHidden", Value::Bool(true));
            }
            let _ = self.call_named(pawn, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
            // Held still where it is (PHYS_None): it does not sink or fall meanwhile (a parked
            // player sinking under the water put the whole game's sound under water, Plage00's
            // boat flashback).
            // Not a player hanging from the grapple (Hook.Tick lets go of the rope when the
            // physics is neither walking, falling nor swimming) nor one on a ladder.
            let traversing = self.prop(pc, "bHooked").as_bool() || !matches!(parked.physics.as_int(), 1 | 2 | 3);
            if !traversing {
                let _ = self.call_named(pawn, "SetPhysics", vec![Value::Byte(0)]);
            }
            self.coop.cine_rules.parked_frozen.retain(|&p| p != pawn);
            if !traversing {
                self.coop.cine_rules.parked_frozen.push(pawn);
            }
            self.set_prop(pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
            self.set_prop(pawn, "Acceleration", Value::vector(0.0, 0.0, 0.0));
            self.log.push(format!("Log: Co-op: player {} parked during player {}'s cinematic", i + 1, leader + 1));
            self.coop.cine_rules.parked.push(parked);
        }
    }

    /// While a cinematic runs: a parked player stays still (a script or a menu may have
    /// changed its state).
    pub(crate) fn coop_cine_hold_parked(&mut self) {
        let physics_now = |vm: &Vm, p: ObjId| vm.prop(p, "Physics").as_int();
        for k in 0..self.coop.cine_rules.parked.len() {
            let p = self.coop.cine_rules.parked[k].clone();
            if !self.coop_object_alive(p.pc) {
                continue;
            }
            // The controller has another pawn now (a flashback started meanwhile gives it its
            // memory pawn): that pawn is parked instead; the former one is the flashback's.
            if let Some(now) = self.prop(p.pc, "Pawn").as_object().filter(|&n| n != p.pawn && self.coop_object_alive(n)) {
                // The former pawn is the flashback's to hide now (coop_flash.rs): it gets back
                // what parking changed, or it stays invisible once given back after the memory.
                if self.coop_object_alive(p.pawn) {
                    self.set_prop(p.pawn, "bHidden", Value::Bool(p.hidden));
                    for &(a, hidden) in &p.attached {
                        if self.coop_object_alive(a) {
                            self.set_prop(a, "bHidden", Value::Bool(hidden));
                        }
                    }
                    if self.coop.cine_rules.parked_frozen.contains(&p.pawn) {
                        let physics = match p.physics.as_int() {
                            0 | 1 => 2,
                            other => other,
                        };
                        let _ = self.call_named(p.pawn, "SetPhysics", vec![Value::Byte(physics as u8)]);
                        self.coop.cine_rules.parked_frozen.retain(|&f| f != p.pawn);
                    }
                }
                let entry = &mut self.coop.cine_rules.parked[k];
                entry.pawn = now;
                entry.hidden = false;
                entry.collide = true;
                entry.block_actors = true;
                entry.block_players = true;
                entry.attached.clear();
                let draw = self.prop(now, "DrawType");
                let physics = self.prop(now, "Physics");
                let entry = &mut self.coop.cine_rules.parked[k];
                entry.draw_type = draw;
                entry.physics = physics;
                self.set_prop(now, "bHidden", Value::Bool(true));
                let _ = self.call_named(now, "SetDrawType", vec![Value::Byte(0)]);
                let _ = self.call_named(now, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
                if matches!(physics_now(self, now), 1 | 2 | 3) && !self.prop(p.pc, "bHooked").as_bool() {
                    let _ = self.call_named(now, "SetPhysics", vec![Value::Byte(0)]);
                    self.coop.cine_rules.parked_frozen.push(now);
                }
            }
            let p = self.coop.cine_rules.parked[k].clone();
            if !self.coop_object_alive(p.pawn) {
                continue;
            }
            if p.stopped && !self.coop_held_state(p.pc) {
                let _ = self.call_named(p.pc, "GotoState", vec![Value::Name("NoControl".into())]);
            }
            self.set_prop(p.pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
            self.set_prop(p.pawn, "Acceleration", Value::vector(0.0, 0.0, 0.0));
        }
    }

    /// End of the cinematic: the parked players get their control and pawn back.
    pub(crate) fn coop_cine_unpark(&mut self) {
        for p in std::mem::take(&mut self.coop.cine_rules.parked) {
            if !self.coop_object_alive(p.pc) {
                continue;
            }
            if self.coop_object_alive(p.pawn) && self.prop(p.pc, "Pawn").as_object() == Some(p.pawn) {
                self.set_prop(p.pawn, "bHidden", Value::Bool(p.hidden));
                let _ = self.call_named(p.pawn, "SetDrawType", vec![p.draw_type.clone()]);
                // Moving again: falling lands it walking; swimming stays swimming. Only for a
                // pawn held still here.
                if self.coop.cine_rules.parked_frozen.contains(&p.pawn) {
                    let physics = match p.physics.as_int() {
                        0 | 1 => 2,
                        other => other,
                    };
                    let _ = self.call_named(p.pawn, "SetPhysics", vec![Value::Byte(physics as u8)]);
                }
                for &(a, hidden) in &p.attached {
                    if self.coop_object_alive(a) {
                        self.set_prop(a, "bHidden", Value::Bool(hidden));
                    }
                }
                let _ = self.call_named(p.pawn, "SetCollision", vec![Value::Bool(p.collide), Value::Bool(p.block_actors), Value::Bool(p.block_players)]);
                // Solid again inside another pawn (the scene moved its player or an actor onto
                // the parked one): a free place on the ground next to the cinematic's player.
                let at = self.prop(p.pawn, "Location").as_vector();
                let (r, h) = (self.prop(p.pawn, "CollisionRadius").as_float(), self.prop(p.pawn, "CollisionHeight").as_float());
                let others: Vec<ObjId> = (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).filter(|&o| o != p.pawn).collect();
                let leader = self.coop_living_pawn(self.coop.cine_leader).filter(|&l| l != p.pawn).or(others.first().copied());
                if let (false, Some(leader)) = (self.coop_place_free_but(at, r, h, &[p.pawn]), leader) {
                    if let Some(rank) = self.coop.players.iter().position(|&x| x == p.pc) {
                        self.coop_neutral(rank);
                    }
                    // The nearest free place to where it stands first (it keeps its side of
                    // the scene and its facing), next to the cinematic's player otherwise.
                    let mut places = self.coop_places_near(p.pawn);
                    places.extend(self.coop_places_near(leader));
                    for place in places {
                        if self.call_named(p.pawn, "SetLocation", vec![Value::vector(place[0], place[1], place[2])]).is_ok_and(|v| v.as_bool()) {
                            self.set_prop(p.pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
                            self.log.push(format!("Log: Co-op: {} moved off another pawn after the cinematic", self.path_name(p.pc)));
                            break;
                        }
                    }
                }
            }
            // Held by a scene meanwhile (coop_rules.rs): that scene lets it go.
            if p.stopped && !self.coop_held_copy(p.pc) && self.coop_held_state(p.pc) {
                let _ = self.call_named(p.pc, "GotoState", vec![Value::Name("PlayerWalking".into())]);
            }
        }
        self.coop.cine_rules.parked_frozen.clear();
    }

    /// A parked player (testing and the other co-op parts).
    pub(crate) fn coop_cine_is_parked(&self, pc: ObjId) -> bool {
        self.coop.cine_rules.parked.iter().any(|p| p.pc == pc)
    }

}

/// What a script may impose on "the player" and the co-op gives every player (coop_cine.rs):
/// the pawn's speed limit (Plage00's wounded walk, lifted by Plage01's medical kit) and its
/// kind of footsteps.
const IMPOSED: [&str; 2] = ["SpeedFactorLimit", "SoundStepCategory"];

/// A map script's call running (MapInfo): its player's values before it.
pub(crate) struct Imposing {
    pawn: ObjId,
    health: Value,
    values: Vec<Value>,
}

impl Vm {
    /// Before a function of the map's MapInfo (FirstFrame, a goal, a trigger): the values of its
    /// player that the call may change.
    pub(crate) fn coop_imposed_begin(&mut self, this: ObjId) -> Option<Imposing> {
        if self.coop.players.is_empty() || self.coop.cine_rules.imposing {
            return None;
        }
        let map_info = self.coop_story_map_info().filter(|&m| m == this)?;
        // MapInfo.FirstFrame (from its Timer) finds its player itself: the first player then.
        let pawn = self.prop(map_info, "XIIIPawn").as_object().filter(|&p| self.coop_object_alive(p)).or_else(|| self.coop_living_pawn(0))?;
        self.coop.cine_rules.imposing = true;
        Some(Imposing { pawn, health: self.prop(pawn, "Health"), values: IMPOSED.iter().map(|p| self.prop(pawn, p)).collect() })
    }

    /// After it: what it changed on its player is given to the other players too.
    pub(crate) fn coop_imposed_end(&mut self, before: Imposing) {
        self.coop.cine_rules.imposing = false;
        if !self.coop_object_alive(before.pawn) {
            return;
        }
        let health = self.prop(before.pawn, "Health");
        let changed: Vec<(&'static str, Value)> =
            IMPOSED.iter().zip(before.values).filter_map(|(p, v)| (self.prop(before.pawn, p) != v).then(|| (*p, self.prop(before.pawn, p)))).collect();
        let health_set = health != before.health;
        if changed.is_empty() && !health_set {
            return;
        }
        // Kept for the players who join later (MapInfo.FirstFrame runs before the second
        // player's pawn exists: Plage00's wounded start).
        for &(p, ref v) in &changed {
            self.coop.cine_rules.imposed_story.retain(|&(q, _)| q != p);
            self.coop.cine_rules.imposed_story.push((p, v.clone()));
        }
        if health_set && health.as_int() > 0 {
            self.coop.cine_rules.imposed_story.retain(|(q, _)| *q != "Health");
            self.coop.cine_rules.imposed_story.push(("Health", health.clone()));
        }
        for i in 0..self.coop.players.len() {
            let Some(pawn) = self.coop_living_pawn(i).filter(|&p| p != before.pawn) else { continue };
            for (p, v) in &changed {
                self.set_prop(pawn, p, v.clone());
            }
            // The story's own health (Plage00 starts wounded): the same for everybody.
            if health_set && health.as_int() > 0 {
                self.set_prop(pawn, "Health", health.clone());
            }
            if changed.iter().any(|(p, _)| *p == "SpeedFactorLimit") {
                let _ = self.call_named(pawn, "SetGroundSpeed", vec![Value::Float(1.0)]);
            }
            self.log.push(format!("Log: Co-op: player {} given the story's {:?}{}", i + 1, changed.iter().map(|c| c.0).collect::<Vec<_>>(), if health_set { " and health" } else { "" }));
        }
        self.coop_imposed_seen();
    }

    /// Once per tick: a value of `IMPOSED` changed on one player since the last tick (an item, a
    /// scene) is given to the others ("lifted for both" too).
    pub(crate) fn coop_imposed_frame(&mut self) {
        let n = self.coop.players.len();
        if n < 2 {
            return;
        }
        // A player whose pawn appears: what the story imposed before (coop_imposed_end).
        let seen_before = self.coop.cine_rules.imposed_seen.clone();
        for i in 0..n {
            if seen_before.get(i).is_some_and(|s| s.is_some()) {
                continue;
            }
            let Some(pawn) = self.coop_living_pawn(i) else { continue };
            for (p, v) in self.coop.cine_rules.imposed_story.clone() {
                self.set_prop(pawn, p, v.clone());
                if p == "SpeedFactorLimit" {
                    let _ = self.call_named(pawn, "SetGroundSpeed", vec![Value::Float(1.0)]);
                }
                self.log.push(format!("Log: Co-op: player {} given the story's {p} on arrival", i + 1));
            }
        }
        let now: Vec<Option<Vec<Value>>> = (0..n).map(|i| self.coop_living_pawn(i).map(|p| IMPOSED.iter().map(|k| self.prop(p, k)).collect())).collect();
        let seen = std::mem::take(&mut self.coop.cine_rules.imposed_seen);
        // First look at this map: a value of the first player that is not the pawn's default
        // was imposed before (MapInfo.FirstFrame can run before this frame): the others get it.
        if seen.is_empty() {
            if let Some(Some(first)) = now.first() {
                let first = first.clone();
                let pawn0 = self.coop_living_pawn(0);
                for k in 0..IMPOSED.len() {
                    let default = pawn0.and_then(|p| self.objects[p].as_ref().map(|o| self.classes[o.class].cdo)).map(|cdo| self.prop(cdo, IMPOSED[k]));
                    if default.as_ref() == Some(&first[k]) {
                        continue;
                    }
                    for i in 1..n {
                        let Some(pawn) = self.coop_living_pawn(i) else { continue };
                        if self.prop(pawn, IMPOSED[k]) != first[k] {
                            self.set_prop(pawn, IMPOSED[k], first[k].clone());
                            if IMPOSED[k] == "SpeedFactorLimit" {
                                let _ = self.call_named(pawn, "SetGroundSpeed", vec![Value::Float(1.0)]);
                            }
                            self.log.push(format!("Log: Co-op: player 1's {} given to player {} at the start", IMPOSED[k], i + 1));
                        }
                    }
                }
            }
        }
        if seen.len() == n {
            for k in 0..IMPOSED.len() {
                let changed: Vec<usize> = (0..n)
                    .filter(|&i| match (&now[i], &seen[i]) {
                        (Some(a), Some(b)) => a[k] != b[k],
                        _ => false,
                    })
                    .collect();
                if changed.len() != 1 {
                    continue;
                }
                let from = changed[0];
                let value = now[from].as_ref().map(|v| v[k].clone()).unwrap_or(Value::Int(0));
                for i in (0..n).filter(|&i| i != from) {
                    let Some(pawn) = self.coop_living_pawn(i) else { continue };
                    if self.prop(pawn, IMPOSED[k]) == value {
                        continue;
                    }
                    self.set_prop(pawn, IMPOSED[k], value.clone());
                    if IMPOSED[k] == "SpeedFactorLimit" {
                        let _ = self.call_named(pawn, "SetGroundSpeed", vec![Value::Float(1.0)]);
                    }
                    self.log.push(format!("Log: Co-op: player {}'s {} given to player {}", from + 1, IMPOSED[k], i + 1));
                }
            }
        }
        self.coop_imposed_seen();
    }

    fn coop_imposed_seen(&mut self) {
        let n = self.coop.players.len();
        self.coop.cine_rules.imposed_seen = (0..n).map(|i| self.coop_living_pawn(i).map(|p| IMPOSED.iter().map(|k| self.prop(p, k)).collect())).collect();
    }
}

impl Vm {
}

/// MapInfo's player changed for MapInfo.SaveInventoryForFlash / RestoreInventoryAfterFlash.
pub(crate) struct FlashOwner {
    map_info: ObjId,
    pawn: Value,
    controller: Value,
}

impl Vm {
    /// MapInfo.SaveInventoryForFlash(P) and RestoreInventoryAfterFlash(P) work on MapInfo's
    /// player (XIIIPawn, XIIIController), GenFlash on its own (the one who caused the
    /// flashback, coop_flash.rs): in co-op the two are made the same, the flashback's player.
    pub(crate) fn coop_flash_owner_begin(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<FlashOwner> {
        if self.coop.players.len() < 2 {
            return None;
        }
        let name = self.functions[f].name.clone();
        let save = name.eq_ignore_ascii_case("SaveInventoryForFlash");
        if !save && !name.eq_ignore_ascii_case("RestoreInventoryAfterFlash") {
            return None;
        }
        let map_info = self.coop_story_map_info().filter(|&m| m == this)?;
        let manager = self.level.and_then(|l| self.prop(l, "FlashManager").as_object()).filter(|&m| self.coop_object_alive(m))?;
        let pc = self.prop(manager, "PC").as_object().filter(|&p| self.coop_object_alive(p) && self.coop.players.contains(&p))?;
        if self.prop(map_info, "XIIIController").as_object() == Some(pc) {
            return None;
        }
        // The original does nothing once the memory pawn exists (Save) or is gone (Restore).
        // IntroFlash started again (a second BeginBeginFlash) saves again with the memory pawn
        // as P: kept as the real pawn, it was the one given back at the end, then destroyed.
        let fake = self.prop(map_info, "FakePawn").as_object().filter(|&p| self.coop_object_alive(p));
        if save == fake.is_some() {
            return None;
        }
        // Save: P is that player's own pawn. Restore: the pawn kept at the save.
        let pawn = if save {
            let p = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&p| self.coop_object_alive(p))?;
            self.coop.cine_rules.flash_real = Some(p);
            p
        } else {
            self.coop.cine_rules.flash_real.take().filter(|&p| self.coop_object_alive(p))?
        };
        let saved = FlashOwner { map_info, pawn: self.prop(map_info, "XIIIPawn"), controller: self.prop(map_info, "XIIIController") };
        self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
        self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
        self.log.push(format!("Log: Co-op: {name} for the flashback's player {}", self.path_name(pc)));
        Some(saved)
    }

    pub(crate) fn coop_flash_owner_end(&mut self, saved: FlashOwner) {
        if self.coop_object_alive(saved.map_info) {
            self.set_prop(saved.map_info, "XIIIPawn", saved.pawn);
            self.set_prop(saved.map_info, "XIIIController", saved.controller);
        }
    }
}

/// Classes of the campaign that take hold of "the player" by code (MapInfo's player when they
/// start): a flashback, Jones's grapple lesson, the boat and beach sequences of Plage00, the
/// ambush of Kello01a, the cable car, the Mongoose of SSH1.
const STORY_MACHINES: [&str; 8] =
    ["GenFlash", "RoofGrapnleDemonstrator", "BeachBoatRock", "BeachFinalFall", "BeachInBedWithXIII", "KelloAmbush", "Telepherique", "MangousteSSH1Controller"];

impl Vm {
    /// A story machine started without a player as instigator (an event relayed by a
    /// dispatcher, a scene's own event): MapInfo's player is the player who last made the story
    /// go on (the last scene's player, coop_scene_take) for that call.
    pub(crate) fn coop_story_machine_begin(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<FlashOwner> {
        if self.coop.players.len() < 2 {
            return None;
        }
        let name = self.functions[f].name.as_str();
        // GenFlash.BeginBeginFlash: a flashback started by the engine itself (Hual02).
        if !name.eq_ignore_ascii_case("Trigger") && !name.eq_ignore_ascii_case("Touch") && !name.eq_ignore_ascii_case("BeginBeginFlash") {
            return None;
        }
        if !STORY_MACHINES.iter().any(|c| self.is_a(this, c)) {
            return None;
        }
        // The player it is for: the one in the call (coop_acting_begin also gives MapInfo that
        // player), or the one who last made the story go on.
        let in_call = args.iter().filter_map(|a| a.as_ref()).filter_map(Value::as_object).find_map(|o| {
            self.coop.players.iter().copied().find(|&pc| pc == o || self.prop(pc, "Pawn").as_object() == Some(o))
        });
        // Started by a scene (an event of its script, directly or through the scene on the call
        // stack): that scene's player, so that the scene and the machine act with the same one
        // (Kello01b: Kim's scene starts the flashback and walks to its player).
        let scene_player = args
            .iter()
            .filter_map(|a| a.as_ref())
            .filter_map(Value::as_object)
            .chain(self.this_stack.iter().rev().copied())
            .find(|&o| self.coop_object_alive(o) && self.is_a(o, "CineController2"))
            .and_then(|scene| self.prop(scene, "PC").as_object())
            .filter(|pc| self.coop.players.contains(pc));
        let target = in_call.or(scene_player).or(self.coop.cine_rules.last_player).filter(|&p| self.coop_object_alive(p));
        // A machine that chose its player when the map started (BeachFinalFall: MapInfo's player
        // of its first Timer) takes that player when it starts, unless it holds its own already.
        // (Not GenFlash: it keeps the player of its first flash for all its portions.)
        if let (Some(target), false) = (target, self.is_a(this, "GenFlash")) {
            if let Some(own) = self.prop(this, "PC").as_object().filter(|&o| o != target && self.coop.players.contains(&o)) {
                if !self.coop_held_state(own) {
                    let pawn = self.prop(target, "Pawn");
                    self.set_prop(this, "PC", Value::Object(Some(target)));
                    if !matches!(self.prop(this, "Player"), Value::Void) {
                        self.set_prop(this, "Player", pawn);
                    }
                    self.log.push(format!("Log: Co-op: {} now for {}", self.path_name(this), self.path_name(target)));
                }
            }
        }
        if self.coop.acting || in_call.is_some() {
            return None;
        }
        let pc = target?;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        let pawn = self.coop_living_pawn(rank)?;
        let map_info = self.coop_story_map_info()?;
        if self.prop(map_info, "XIIIController").as_object() == Some(pc) {
            return None;
        }
        let saved = FlashOwner { map_info, pawn: self.prop(map_info, "XIIIPawn"), controller: self.prop(map_info, "XIIIController") };
        self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
        self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
        self.log.push(format!("Log: Co-op: {} started for player {}", self.path_name(this), rank + 1));
        Some(saved)
    }
}

impl Vm {
    /// Once per tick: a scene triggered before it was ready gets its event now that it is.
    pub(crate) fn coop_scene_wake_frame(&mut self) {
        if self.coop.cine_rules.early.is_empty() {
            return;
        }
        for (cine, other, by) in std::mem::take(&mut self.coop.cine_rules.early) {
            if !self.coop_object_alive(cine) {
                continue;
            }
            if !self.prop(cine, "bInitialized").as_bool() {
                self.coop.cine_rules.early.push((cine, other, by));
                continue;
            }
            // Started meanwhile by another event: nothing to give again.
            if self.prop(cine, "CurrentScript").as_int() != -1 {
                continue;
            }
            self.log.push(format!("Log: Co-op: {} woken now that it is ready", self.path_name(cine)));
            self.event_pub(cine, "Trigger", vec![Value::Object(other), Value::Object(by)]);
        }
    }
}

impl Vm {
    /// Before a player is put somewhere else by the co-op (gathered, parked back, following a
    /// flashback): out of what holds them where they are, as a player who just arrives. The
    /// grapple's rope let go (Hook.Release, the controller no longer hooked), off a ladder or a
    /// rope (falling, the ground makes it walking), standing up, no zoom.
    pub(crate) fn coop_neutral(&mut self, rank: usize) {
        let Some(&pc) = self.coop.players.get(rank) else { return };
        let Some(pawn) = self.coop_living_pawn(rank) else { return };
        if self.prop(pc, "bHooked").as_bool() {
            if let Some(hook) = self.prop(pc, "HookUsed").as_object().filter(|&h| self.coop_object_alive(h)) {
                let _ = self.call_named(hook, "Release", vec![]);
            }
            self.set_prop(pc, "bHooked", Value::Bool(false));
        }
        // PHYS_Walking 1, PHYS_Falling 2, PHYS_Swimming 3, PHYS_Ladder 11 (and the rope's).
        let physics = self.prop(pawn, "Physics").as_int();
        if physics != 1 && physics != 2 {
            let _ = self.call_named(pawn, "SetPhysics", vec![Value::Byte(2)]);
        }
        let state = self.current_state(pc).map(|(_, s)| s.name.clone()).unwrap_or_default();
        if ["PlayerClimbing", "PlayerSwimming", "ClimbToHookNavPoint"].iter().any(|s| state.eq_ignore_ascii_case(s)) {
            let _ = self.call_named(pc, "GotoState", vec![Value::Name("PlayerWalking".into())]);
        }
        if self.prop(pawn, "bIsCrouched").as_bool() {
            let _ = self.call_named(pawn, "ShouldCrouch", vec![Value::Bool(false)]);
        }
        let fov = self.prop(pc, "DefaultFOV");
        self.set_prop(pc, "DesiredFOV", fov.clone());
        self.set_prop(pc, "FOVAngle", fov);
    }
}

impl Vm {
    /// CWndFocusTrigger (a picture of something to look at, opened on "the player's" HUD when
    /// that player sees it, closed by its EventFinFocus) looks for itself in the HUD of
    /// MapInfo's player when it is closed, in a loop that only stops when it finds itself. The
    /// event that closes it can come from another player (a hook point a player's grapple
    /// catches: coop_world.rs then gives MapInfo that player), whose HUD does not hold it: the
    /// loop never ended and the script was cut off, the grapple with it (Toits01). The closing
    /// is done with the player whose HUD holds it; when no HUD holds it, nothing is done.
    pub(crate) fn coop_focus_window_begin(&mut self, f: FuncId, this: ObjId) -> Option<Option<FlashOwner>> {
        if self.coop.players.len() < 2 || self.coop.cine_rules.focus_busy || !self.functions[f].name.eq_ignore_ascii_case("Trigger") || !self.is_a(this, "CWndFocusTrigger") {
            return None;
        }
        if !self.current_state(this).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("WaitEndFocus")) {
            return None;
        }
        let holder = self.coop.players.iter().copied().find(|&pc| {
            self.coop_object_alive(pc)
                && self.prop(pc, "MyHUD").as_object().filter(|&h| self.coop_object_alive(h)).is_some_and(|hud| match self.propr(hud, "tFocusTrigger") {
                    Value::Array(list) => list.iter().any(|v| v.as_object() == Some(this)),
                    _ => false,
                })
        });
        let Some(pc) = holder else {
            self.log.push(format!("Log: Co-op: {} closed by nobody's HUD, left as it is", self.path_name(this)));
            return Some(None);
        };
        let map_info = self.coop_story_map_info()?;
        let pawn = self.prop(pc, "Pawn");
        let saved = FlashOwner { map_info, pawn: self.prop(map_info, "XIIIPawn"), controller: self.prop(map_info, "XIIIController") };
        self.set_prop(map_info, "XIIIPawn", pawn);
        self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
        self.coop.cine_rules.focus_busy = true;
        Some(Some(saved))
    }
}

impl Vm {
    /// CineController2.Interpret (a line of a scene's script): some commands use MapInfo's
    /// player instead of the scene's own (TPInFrontOf: Pam put in front of MapInfo.XIIIPawn,
    /// Plage00). While a scene runs one of its lines, MapInfo's player is the scene's player.
    pub(crate) fn coop_scene_line_begin(&mut self, f: FuncId, this: ObjId) -> Option<FlashOwner> {
        if self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("Interpret") || !self.is_a(this, "CineController2") {
            return None;
        }
        let pc = self.prop(this, "PC").as_object().filter(|&p| self.coop_object_alive(p) && self.coop.players.contains(&p))?;
        let map_info = self.coop_story_map_info()?;
        if self.prop(map_info, "XIIIController").as_object() == Some(pc) {
            return None;
        }
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        let pawn = self.coop_living_pawn(rank)?;
        let saved = FlashOwner { map_info, pawn: self.prop(map_info, "XIIIPawn"), controller: self.prop(map_info, "XIIIController") };
        self.set_prop(map_info, "XIIIPawn", Value::Object(Some(pawn)));
        self.set_prop(map_info, "XIIIController", Value::Object(Some(pc)));
        Some(saved)
    }
}

impl Vm {
    /// The scene's player changed from pawn `old` to pawn `new`: what the scene's actor was told
    /// to look at as "player" (PeerAt: its head, LookAt: its body, ViewFocus) follows, as it
    /// would have looked at the new player had it been the scene's player then.
    /// Not only the former player's current pawn: any player pawn the scene still looks at (a
    /// pawn of a player who died and came back since the line ran, the other player's pawn)
    /// is "the player" of that line, as FindAnActor("player") is the only way a script names one.
    pub(crate) fn coop_scene_retarget(&mut self, scene: ObjId, old: ObjId, new: ObjId) {
        let is_player = |vm: &Vm, o: Option<ObjId>| o.is_some_and(|o| o != new && (o == old || vm.coop_player_pawn_object(o)));
        for field in ["Focus", "LockedActor"] {
            if is_player(self, self.prop(scene, field).as_object()) {
                self.set_prop(scene, field, Value::Object(Some(new)));
            }
        }
        if let Some(actor) = self.prop(scene, "MyPawn").as_object().filter(|&a| self.coop_object_alive(a)) {
            if is_player(self, self.prop(actor, "PeeredActor").as_object()) {
                self.set_prop(actor, "PeeredActor", Value::Object(Some(new)));
            }
        }
    }

    /// A pawn that is or was a co-op player's (living, down, or left behind by a respawn).
    fn coop_player_pawn_object(&self, o: ObjId) -> bool {
        self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "XIIIPlayerPawn")
    }

    /// Each pass, for every scene: what it looks at as "the player" (Focus, LockedActor, its
    /// actor's PeeredActor) is its current player, when that player is in play. The scene's
    /// player can change without a rebinding (Tick takes PC.Pawn again after a respawn), and a
    /// line run before the triggering player arrived keeps the pawn of that moment.
    pub(crate) fn coop_scene_follow(&mut self, scene: ObjId) {
        let Some(pc) = self.prop(scene, "PC").as_object() else { return };
        let Some(rank) = self.coop.players.iter().position(|&p| p == pc) else { return };
        let Some(pawn) = self.coop_living_pawn(rank) else { return };
        self.coop_scene_retarget(scene, pawn, pawn);
    }
}
