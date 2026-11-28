//! Local co-op campaign, the rules of the scripted story (see coop.rs), chosen with the
//! player once for every map, instead of mission by mission:
//!
//! - A scripted scene acts with its own player: an event a scene causes (CineController2
//!   TriggerEvent, Other = the scene) gives MapInfo's player the scene's player for that event
//!   (coop_world.rs, coop_acting_begin), and the scene's player is the nearest one when nobody
//!   sees its actor (coop_combat.rs, coop_scene_player).
//! - A scene that holds its player (FreezePlayer, FreezePlayerLocation, DUCK, ForceCrouch) holds
//!   every player; ReleasePlayer, or the scene's player let go, releases them all.
//! - A player is moved only when they can no longer come back (owner's rule, 17/11): never at a
//!   checkpoint (XIIISaveGameTrigger) nor at a magnetic pass, whose doors open again. A lift
//!   cage or a cable car that leaves with a player and stops far away: the players left at its
//!   start are brought next to its riders (its only button rides with it). A door a
//!   scene opens (the clerk of Banque01 leading the way) and that closes again: a player left on
//!   the far side from the scene's actor is brought next to a player on the actor's side; a
//!   player already through stays where they are. A door a scene locks ("lockdoor": SMarin01,
//!   PRock01a, Hual02, Bateau01, SPADS02a, SMarin02) is handled the same way once it is shut.
//!   Only a door no player can open again from either side separates them: a door a player
//!   opens by using it, bumping or standing at it brings nobody.
//! - A cinematic parks the other players where they stand (coop_story.rs).
//!
//! The improved build also keeps the weapon in hand when the bank clerk takes the key back
//! (Banque01, solo too): see `bank_key_timer`.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// A player held by a scene because the scene held its own player.
#[derive(Debug, Clone)]
pub(crate) struct Held {
    pub(crate) scene: ObjId,
    pub(crate) pc: ObjId,
    /// FreezePlayer put the weapon (or item) down: given back on release.
    pub(crate) weapon_down: bool,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct CoopRules {
    /// A rule's own call of a script is running.
    busy: bool,
    pub(crate) held: Vec<Held>,
    /// Doors opened by a scene: (door, scene, opened yet, level time of the opening event).
    doors: Vec<WatchedDoor>,
    /// Lift cages and cable cars of the map, found once (None: not looked for yet).
    lifts: Option<Vec<ObjId>>,
    /// A lift's journey under way.
    trips: Vec<Trip>,
}

/// A lift cage or a cable car moving with players on it.
#[derive(Debug, Clone)]
pub(crate) struct Trip {
    lift: ObjId,
    from: [f32; 3],
    last: [f32; 3],
    /// Ranks of the players carried at some point of the journey.
    riders: Vec<usize>,
    /// Level time of its last move.
    moved: f64,
}

/// A lift still for this long has arrived.
const LIFT_STILL: f64 = 1.0;
/// A journey shorter than this is no departure.
const LIFT_TRIP: f32 = 300.0;

/// A door a scene triggered, with its pose then (shut).
#[derive(Debug, Clone)]
pub(crate) struct WatchedDoor {
    door: ObjId,
    scene: ObjId,
    at: [f32; 3],
    rot: [i32; 3],
    opened: bool,
    since: f64,
    /// Shut while its scene was still to open it again: since when.
    shut_at: Option<f64>,
}

/// A door shut that its scene is still to open again: after this long shut, the players left
/// behind are brought through all the same (the scene may wait for ever, for an alarm).
const REOPEN_WAIT: f64 = 20.0;
/// A door watched longer than this is forgotten (a door left open).
const DOOR_WATCH: f64 = 120.0;
/// Beyond the door's plane by more than this, on the far side from the scene's actor.
const BEHIND: f32 = 48.0;

/// What a rule does after the call it watches.
pub(crate) enum After {
    /// CineController2.Interpret: the scene's command (first word, upper case).
    Scene { scene: ObjId, command: String, line: String },
    /// A mover triggered by a scene.
    SceneDoor { door: ObjId, scene: ObjId },
    /// Cine2.LockDoor called by a scene ("lockdoor", "ld"): a door the scene locks.
    SceneLock { door: ObjId, scene: ObjId },
    /// A door shut for good by a player passing a once-only trigger (Banque01's vault door,
    /// toggled shut by a trigger at the back of the vault).
    OnceDoor { door: ObjId, by: ObjId },
    /// A checkpoint (XIIISaveGameTrigger) touched or triggered: saved with the first player
    /// (`coop_checkpoint_saver`).
    Checkpoint { trigger: ObjId },
    /// A door opened by a magnetic card reader (MagneticPassTrigger's OutEvent).
    PassDoor { door: ObjId, pass: ObjId },
}

const HOLDS: [&str; 6] = ["FPC", "FREEZEPLAYER", "FPL", "FREEZEPLAYERLOCATION", "DUCK", "FORCECROUCH"];
const RELEASES: [&str; 2] = ["RPC", "RELEASEPLAYER"];

impl Vm {
    /// Before a call, in co-op: the rule watching it, if any.
    pub(crate) fn coop_rules_before(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<After> {
        if self.coop.rules.busy || self.coop.players.len() < 2 {
            return None;
        }
        let function = &self.functions[f];
        let name = function.name.as_str();
        let owner = self.classes[function.owner].name.as_str();
        if name.eq_ignore_ascii_case("Interpret") && owner.eq_ignore_ascii_case("CineController2") {
            let Some(Value::Str(line)) = args.first().and_then(|a| a.as_ref()) else { return None };
            let line = line.to_string();
            let command = line.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
            if HOLDS.contains(&command.as_str()) || RELEASES.contains(&command.as_str()) {
                return Some(After::Scene { scene: this, command, line });
            }
            return None;
        }
        if (name.eq_ignore_ascii_case("Touch") || name.eq_ignore_ascii_case("Trigger")) && self.is_a(this, "XIIISaveGameTrigger") {
            return Some(After::Checkpoint { trigger: this });
        }
        // A scene's "lockdoor" (Cine2.LockDoor, static, called by its CineController2): the
        // owner chose the same rule as for the clerk's door (15/11).
        if name.eq_ignore_ascii_case("LockDoor") && owner.eq_ignore_ascii_case("Cine2") {
            let door = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&d| self.coop_object_alive(d) && self.is_a(d, "Mover"))?;
            let scene = self.this_stack.last().copied().filter(|&s| self.coop_object_alive(s) && self.is_a(s, "CineController2"))?;
            return Some(After::SceneLock { door, scene });
        }
        // An open door toggled by a once-only trigger that a player touched: it shuts and
        // nothing opens it again.
        if name.eq_ignore_ascii_case("Trigger")
            && self.is_a(this, "Porte")
            && self.current_state(this).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("TriggerToggle"))
        {
            // Trigger.Touch passes the player as Other and Instigator; the trigger is the caller.
            let other = self.this_stack.last().copied().filter(|&o| o != this && self.coop_object_alive(o));
            let by = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&p| self.coop_object_alive(p) && self.coop_is_player_pawn(p));
            if let (Some(other), Some(by)) = (other, by) {
                let base = self.prop(this, "BasePos").as_vector();
                let open = base != [0.0, 0.0, 0.0] && {
                    let at = self.prop(this, "Location").as_vector();
                    let (r, b) = (self.prop(this, "Rotation").as_rotator(), self.prop(this, "BaseRot").as_rotator());
                    (0..3).any(|k| (at[k] - base[k]).abs() >= 1.0) || (0..3).any(|k| ((r[k] - b[k]) & 0xffff).min((b[k] - r[k]) & 0xffff) >= 64)
                };
                if open && !self.is_a(other, "Mover") && !self.is_a(other, "Pawn") && self.prop(other, "bTriggerOnceOnly").as_bool() {
                    return Some(After::OnceDoor { door: this, by });
                }
            }
        }
        // A door a card reader opens (the card's holder used it): watched until it shuts.
        if name.eq_ignore_ascii_case("Trigger") && self.is_a(this, "Mover") {
            let pass = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&o| self.coop_object_alive(o) && self.is_a(o, "MagneticPassTrigger"));
            if let Some(pass) = pass {
                return Some(After::PassDoor { door: this, pass });
            }
        }
        // A door a scene opens: Trigger from the scene's controller (an event of its script), or
        // XIIIPorte.PlayerTrigger from the scene's pawn (Cine2.OpenDoor, "opendoor").
        if (name.eq_ignore_ascii_case("Trigger") || name.eq_ignore_ascii_case("PlayerTrigger")) && self.is_a(this, "Mover") {
            let other = args.first().and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&o| self.coop_object_alive(o))?;
            let scene = if self.is_a(other, "CineController2") {
                other
            } else {
                self.prop(other, "Controller").as_object().filter(|&c| self.coop_object_alive(c) && self.is_a(c, "CineController2"))?
            };
            return Some(After::SceneDoor { door: this, scene });
        }
        None
    }

    /// The call itself, with the rule's after part.
    pub(crate) fn coop_rules_call(&mut self, f: FuncId, this: ObjId, mut args: Vec<Option<Value>>, after: After) -> crate::Result<(Value, Vec<Option<Value>>)> {
        if let After::Checkpoint { trigger } = &after {
            self.coop_checkpoint_saver(*trigger, f, &mut args);
        }
        self.coop.rules.busy = true;
        let r = self.call(f, this, args);
        self.coop.rules.busy = false;
        match after {
            After::Scene { scene, command, line } => self.coop_scene_holds(scene, &command, &line),
            After::Checkpoint { .. } => {}
            After::SceneDoor { door, scene } => {
                if !self.coop.rules.doors.iter().any(|d| d.door == door) {
                    let now = self.level_time;
                    let (at, rot) = (self.prop(door, "Location").as_vector(), self.prop(door, "Rotation").as_rotator());
                    self.coop.rules.doors.push(WatchedDoor { door, scene, at, rot, opened: false, since: now, shut_at: None });
                }
            }
            // Watched from its shut pose, already open: once shut, the players on the far side
            // from the one who touched the trigger are brought to them.
            After::OnceDoor { door, by } => {
                let now = self.level_time;
                let (at, rot) = (self.prop(door, "BasePos").as_vector(), self.prop(door, "BaseRot").as_rotator());
                self.coop.rules.doors.retain(|d| d.door != door);
                self.coop.rules.doors.push(WatchedDoor { door, scene: by, at, rot, opened: true, since: now, shut_at: None });
                self.log.push(format!("Log: Co-op: {} shut for good by {}", self.path_name(door), self.path_name(by)));
            }
            After::PassDoor { door, pass } => {
                if !self.coop.rules.doors.iter().any(|d| d.door == door) {
                    let now = self.level_time;
                    let (at, rot) = (self.prop(door, "Location").as_vector(), self.prop(door, "Rotation").as_rotator());
                    self.coop.rules.doors.push(WatchedDoor { door, scene: pass, at, rot, opened: false, since: now, shut_at: None });
                }
            }
            // Locked: watched as a door opened by the scene, checked once it is shut.
            After::SceneLock { door, scene } => {
                let locked = self.current_state(door).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("Locked"));
                if locked {
                    let now = self.level_time;
                    // Watched from its shut pose (the mover's key 0), already opened: shut, it
                    // brings the players left behind through.
                    let base = self.prop(door, "BasePos").as_vector();
                    let at = if base == [0.0, 0.0, 0.0] { self.prop(door, "Location").as_vector() } else { base };
                    let rot = if base == [0.0, 0.0, 0.0] { self.prop(door, "Rotation").as_rotator() } else { self.prop(door, "BaseRot").as_rotator() };
                    self.coop.rules.doors.retain(|d| d.door != door);
                    self.coop.rules.doors.push(WatchedDoor { door, scene, at, rot, opened: true, since: now, shut_at: None });
                    self.log.push(format!("Log: Co-op: {} locked by {}", self.path_name(door), self.path_name(scene)));
                }
            }
        }
        r
    }

    /// Rank of the player whose pawn or controller is `object`.
    fn coop_rank_of(&self, object: ObjId) -> Option<usize> {
        (0..self.coop.players.len()).find(|&i| self.coop.players[i] == object || self.coop_living_pawn(i) == Some(object))
    }

    /// A checkpoint (XIIISaveGameTrigger) touched by another player than the first one: the game
    /// saves the first player's travelling inventory (SaveAtCheckpoint, the viewport 0 player),
    /// where the checkpoint's XIIIThingsToSave must be for "Retry" to load that checkpoint
    /// (otherwise the map starts over from its beginning: Jones back at Toits01's start). The
    /// touch (Touch's Other, Trigger's EventInstigator) is handed to the first player's living
    /// pawn.
    fn coop_checkpoint_saver(&mut self, trigger: ObjId, f: FuncId, args: &mut [Option<Value>]) {
        if !self.is_a(trigger, "XIIISaveGameTrigger") {
            return;
        }
        let slot = if self.functions[f].name.eq_ignore_ascii_case("Touch") { 0 } else { 1 };
        let Some(toucher) = args.get(slot).and_then(|a| a.as_ref()).and_then(Value::as_object) else { return };
        let Some(rank) = self.coop_rank_of(toucher).filter(|&r| r > 0) else { return };
        let Some(first) = self.coop_living_pawn(0).filter(|&p| !self.prop(p, "bIsDead").as_bool()) else { return };
        args[slot] = Some(Value::Object(Some(first)));
        self.log.push(format!("Log: Co-op: checkpoint {} reached by player {}, saved with player 1", self.path_name(trigger), rank + 1));
    }

    /// After a scene's command that holds or releases its player: the same for the others.
    fn coop_scene_holds(&mut self, scene: ObjId, command: &str, line: &str) {
        if !self.coop_object_alive(scene) {
            return;
        }
        if RELEASES.contains(&command) {
            let held: Vec<Held> = self.coop.rules.held.iter().filter(|h| h.scene == scene).cloned().collect();
            for h in held {
                self.coop_release_held(&h, Some(line));
            }
            self.coop.rules.held.retain(|h| h.scene != scene);
            return;
        }
        let Some(own) = self.prop(scene, "PC").as_object() else { return };
        let others: Vec<(ObjId, ObjId)> = (0..self.coop.players.len())
            .filter(|&i| self.coop.players[i] != own)
            .filter_map(|i| Some((self.coop.players[i], self.coop_living_pawn(i)?)))
            .collect();
        let freeze = command == "FPC" || command == "FREEZEPLAYER";
        // Interpret moves the scene to its next line (++ScriptedActionIndex): the scene's own
        // line already did, the copies must not, or the lines after it are skipped.
        let saved: Vec<(&str, Value)> =
            ["PC", "Player", "bFrozenPlayer", "FPCTarget", "FPCLocation", "ScriptedActionIndex"].into_iter().map(|p| (p, self.prop(scene, p))).collect();
        for (pc, pawn) in others {
            let known = self.coop.rules.held.iter().position(|h| h.scene == scene && h.pc == pc);
            // FreezePlayer puts the weapon down once per hold, as for the scene's player.
            let first_freeze = freeze && !known.is_some_and(|k| self.coop.rules.held[k].weapon_down);
            self.set_prop(scene, "PC", Value::Object(Some(pc)));
            self.set_prop(scene, "Player", Value::Object(Some(pawn)));
            self.set_prop(scene, "bFrozenPlayer", Value::Bool(!first_freeze));
            self.coop.rules.busy = true;
            let _ = self.call_named(scene, "Interpret", vec![Value::Str(line.into())]);
            self.coop.rules.busy = false;
            self.log.push(format!("Log: Co-op: {} held by {} ({command})", self.path_name(pc), self.path_name(scene)));
            match known {
                Some(k) => self.coop.rules.held[k].weapon_down |= first_freeze,
                None => self.coop.rules.held.push(Held { scene, pc, weapon_down: first_freeze }),
            }
        }
        for (p, v) in saved {
            self.set_prop(scene, p, v);
        }
    }

    /// Lets a held player go: ReleasePlayer as the scene does it (`line`), or the state
    /// PlayerWalking when the scene let its own player go some other way; then the weapon back
    /// (CineController2.Tick, the end of bFrozenPlayer).
    fn coop_release_held(&mut self, h: &Held, line: Option<&str>) {
        if !self.coop_object_alive(h.pc) {
            return;
        }
        self.log.push(format!("Log: Co-op: {} let go by {}", self.path_name(h.pc), self.path_name(h.scene)));
        let pawn = self.prop(h.pc, "Pawn").as_object().filter(|&p| self.coop_object_alive(p));
        match (line, self.coop_object_alive(h.scene)) {
            (Some(line), true) => {
                let saved: Vec<(&str, Value)> = ["PC", "Player", "ScriptedActionIndex"].into_iter().map(|p| (p, self.prop(h.scene, p))).collect();
                self.set_prop(h.scene, "PC", Value::Object(Some(h.pc)));
                self.set_prop(h.scene, "Player", Value::Object(pawn));
                self.coop.rules.busy = true;
                let _ = self.call_named(h.scene, "Interpret", vec![Value::Str(line.into())]);
                self.coop.rules.busy = false;
                for (p, v) in saved {
                    self.set_prop(h.scene, p, v);
                }
            }
            _ => {
                let _ = self.call_named(h.pc, "GotoState", vec![Value::Name("PlayerWalking".into())]);
                if let Some(input) = self.prop(h.pc, "PlayerInput").as_object() {
                    self.set_prop(input, "bForceCrouch", Value::Bool(false));
                }
            }
        }
        let Some(pawn) = pawn else { return };
        if !h.weapon_down {
            return;
        }
        self.set_prop(h.pc, "bWeaponBlock", Value::Bool(false));
        if self.prop(h.pc, "bWeaponMode").as_bool() {
            let group = self.prop(h.pc, "OldWeap");
            let _ = self.call_named(h.pc, "SwitchWeapon", vec![group]);
        } else {
            let _ = self.call_named(h.pc, "cNextItem", vec![]);
            let item = self.prop(h.pc, "OldItem");
            self.set_prop(pawn, "PendingItem", item);
        }
        let _ = self.call_named(pawn, "ChangedWeapon", vec![]);
    }

    /// Once per tick: players held for a scene whose own player is free again (the scene ended,
    /// died, or let its player go without ReleasePlayer) are let go too.
    pub(crate) fn coop_rules_frame(&mut self) {
        self.coop_lifts_frame();
        if !self.coop.rules.doors.is_empty() {
            self.coop_doors_frame();
        }
        if self.coop.rules.held.is_empty() {
            return;
        }
        let held = std::mem::take(&mut self.coop.rules.held);
        let mut kept = Vec::new();
        for h in held {
            let own = self.coop_object_alive(h.scene).then(|| self.prop(h.scene, "PC").as_object()).flatten();
            let still = own.is_some_and(|pc| self.coop_object_alive(pc) && self.coop_held_state(pc));
            if still && self.coop_object_alive(h.pc) {
                kept.push(h);
            } else {
                self.coop_release_held(&h, None);
            }
        }
        self.coop.rules.held = kept;
    }

    /// Doors opened by a scene: once closed again, the players left behind are brought through.
    fn coop_doors_frame(&mut self) {
        let doors = std::mem::take(&mut self.coop.rules.doors);
        let mut kept = Vec::new();
        for mut d in doors {
            if !self.coop_object_alive(d.door) || self.level_time - d.since > DOOR_WATCH {
                continue;
            }
            // Shut: back in its pose when the scene triggered it, or in its closed pose (key 0:
            // a door left ajar that the scene shuts, like the bank clerk's), and not moving.
            let (at, rot) = (self.prop(d.door, "Location").as_vector(), self.prop(d.door, "Rotation").as_rotator());
            let near = |pos: [f32; 3], r: [i32; 3]| (0..3).all(|k| (at[k] - pos[k]).abs() < 1.0) && (0..3).all(|k| ((rot[k] - r[k]) & 0xffff).min((r[k] - rot[k]) & 0xffff) < 64);
            let base = self.prop(d.door, "BasePos").as_vector();
            let closed = base != [0.0, 0.0, 0.0] && near(base, self.prop(d.door, "BaseRot").as_rotator());
            let shut = (near(d.at, d.rot) || closed) && !self.prop(d.door, "bInterpolating").as_bool();
            if !shut {
                d.opened = true;
                d.shut_at = None;
                kept.push(d);
            } else if !d.opened {
                kept.push(d);
            } else if self.coop_scene_reopens(d.scene, d.door) && self.level_time - *d.shut_at.get_or_insert(self.level_time) < REOPEN_WAIT {
                // Shut for now: the scene opens it again further on (SMarin02's Pacha locks the
                // airlock, then opens it again). Watched a while.
                d.since = self.level_time;
                kept.push(d);
            } else {
                self.coop_door_shut(d.door, d.scene);
            }
        }
        self.coop.rules.doors = kept;
    }

    /// Side of each player against a closed door, the scene's actor giving the far side.
    fn coop_door_shut(&mut self, door: ObjId, scene: ObjId) {
        if self.coop_object_alive(scene) && self.is_a(scene, "MagneticPassTrigger") {
            self.coop_pass_door_shut(door, scene);
            return;
        }
        // The scene's actor, or the player who shut a door for good.
        let actor = if self.coop_object_alive(scene) && self.is_a(scene, "Pawn") {
            Some(scene)
        } else {
            self.coop_object_alive(scene).then(|| self.prop(scene, "Pawn").as_object()).flatten()
        };
        let Some(actor) = actor.filter(|&a| self.coop_object_alive(a)) else {
            return;
        };
        let base = self.prop(door, "BasePos").as_vector();
        let at = if base == [0.0, 0.0, 0.0] { self.prop(door, "Location").as_vector() } else { base };
        // The player who made the scene go on (touched its trigger: PRock01a's end of the
        // escort, in the showers) stays where they are and the others come to them, whichever
        // side the scene's actor stands on. Without one, the actor's side is the far side.
        let lead = self.coop_door_lead(scene);
        let a = self.prop(lead.map_or(actor, |l| l.1), "Location").as_vector();
        // The door's own plane when it is known (a swinging door: from its hinge to its middle;
        // a sliding one: the way it slides), its far side being the actor's; otherwise the way
        // from the door to the actor.
        let along = self.coop_door_plane(door);
        let normal = along.map(|v| [-v[1], v[0]]).filter(|n| (a[0] - at[0]) * n[0] + (a[1] - at[1]) * n[1] != 0.0);
        let (nx, ny) = match normal {
            Some(n) => {
                let k = if (a[0] - at[0]) * n[0] + (a[1] - at[1]) * n[1] > 0.0 { 1.0 } else { -1.0 };
                (n[0] * k, n[1] * k)
            }
            None => (a[0] - at[0], a[1] - at[1]),
        };
        let len = (nx * nx + ny * ny).sqrt();
        if len < 1.0 {
            return;
        }
        let side = |p: [f32; 3]| ((p[0] - at[0]) * nx + (p[1] - at[1]) * ny) / len;
        let pawns: Vec<(usize, ObjId)> = (0..self.coop.players.len()).filter_map(|i| Some((i, self.coop_living_pawn(i)?))).collect();
        let sides: Vec<f32> = pawns.iter().map(|&(_, p)| side(self.prop(p, "Location").as_vector())).collect();
        // The player through the door: the one who made the scene go on, or the one nearest
        // to the actor on its side.
        let Some(&(through, _)) = pawns
            .iter()
            .zip(&sides)
            .filter(|(p, &s)| s > 0.0 && lead.is_none_or(|l| l.0 == p.0))
            .map(|(p, _)| p)
            .min_by(|x, y| self.coop_distance(x.1, actor).total_cmp(&self.coop_distance(y.1, actor)))
        else {
            return;
        };
        // Separated for good only when the player left behind cannot open the door again from
        // where they stand (locked, a one-time door, events only, or blocked on that side).
        for (k, &(i, pawn)) in pawns.iter().enumerate() {
            if sides[k] >= -BEHIND || i == through {
                continue;
            }
            if self.coop_player_can_open(door, pawn) {
                self.log.push(format!("Log: Co-op: {} shut but a player can open it again: nobody moved", self.path_name(door)));
                continue;
            }
            if self.coop_bring_next_to(i, through) {
                self.log.push(format!("Log: Co-op: player {} brought through {} next to player {}", i + 1, self.path_name(door), through + 1));
            }
        }
    }

    /// The way along a door's own plane, when it is known: a swinging door from its hinge to
    /// its middle, a sliding one the way it slides.
    fn coop_door_plane(&self, door: ObjId) -> Option<[f32; 2]> {
        let dd = self.prop(door, "DoorDirection").as_vector();
        let slide = match self.prop(door, "KeyPos") {
            Value::Array(keys) => keys.get(1).map(Value::as_vector).unwrap_or_default(),
            _ => [0.0; 3],
        };
        let swing = self.is_a(door, "XIIIPorte") && (dd[0] * dd[0] + dd[1] * dd[1]).sqrt() > 8.0;
        if swing {
            Some([dd[0], dd[1]])
        } else if (slide[0] * slide[0] + slide[1] * slide[1]).sqrt() > 8.0 && self.prop(door, "BaseRot").as_rotator() == self.prop(door, "Rotation").as_rotator() {
            Some([slide[0], slide[1]])
        } else {
            None
        }
    }

    /// A door a card reader opened, shut again: the reader is on the outside only, and only the
    /// card's holder can use it, so a player left on the reader's side without the card is
    /// cut off from the players who went through (Sanc02a, the doors to the stone's room).
    /// They are brought to them. Nobody moves when nobody went through.
    fn coop_pass_door_shut(&mut self, door: ObjId, pass: ObjId) {
        let base = self.prop(door, "BasePos").as_vector();
        let at = if base == [0.0, 0.0, 0.0] { self.prop(door, "Location").as_vector() } else { base };
        let reader = self.prop(pass, "Location").as_vector();
        let (rx, ry) = (reader[0] - at[0], reader[1] - at[1]);
        // The normal of the door's plane, the reader's side negative; the way to the reader
        // otherwise.
        let (nx, ny) = match self.coop_door_plane(door).map(|v| [-v[1], v[0]]).filter(|n| rx * n[0] + ry * n[1] != 0.0) {
            Some(n) => {
                let k = if rx * n[0] + ry * n[1] > 0.0 { -1.0 } else { 1.0 };
                (n[0] * k, n[1] * k)
            }
            None => (-rx, -ry),
        };
        let len = (nx * nx + ny * ny).sqrt();
        if len < 1.0 {
            return;
        }
        let side = |p: [f32; 3]| ((p[0] - at[0]) * nx + (p[1] - at[1]) * ny) / len;
        let pawns: Vec<(usize, ObjId)> = (0..self.coop.players.len()).filter_map(|i| Some((i, self.coop_living_pawn(i)?))).collect();
        let sides: Vec<f32> = pawns.iter().map(|&(_, p)| side(self.prop(p, "Location").as_vector())).collect();
        let inside: Vec<(usize, ObjId)> = pawns.iter().zip(&sides).filter(|(_, &s)| s > BEHIND).map(|(p, _)| *p).collect();
        if inside.is_empty() {
            return;
        }
        let tag = self.prop(pass, "Tag").as_name();
        for (k, &(i, pawn)) in pawns.iter().enumerate() {
            if sides[k] >= -BEHIND {
                continue;
            }
            // Holding a card for this reader: they can open it again.
            let mut item = self.prop(pawn, "Inventory").as_object();
            let mut card = false;
            for _ in 0..64 {
                let Some(it) = item.filter(|&it| self.coop_object_alive(it)) else { break };
                if self.is_a(it, "EventItem") && self.prop(it, "Event").as_name().eq_ignore_ascii_case(&tag) {
                    card = true;
                    break;
                }
                item = self.prop(it, "Inventory").as_object();
            }
            if card {
                self.log.push(format!("Log: Co-op: {} shut but player {} holds its card: nobody moved", self.path_name(door), i + 1));
                continue;
            }
            let Some(&(to, to_pawn)) = inside.iter().min_by(|a, b| self.coop_distance(a.1, pawn).total_cmp(&self.coop_distance(b.1, pawn))) else { continue };
            let _ = to_pawn;
            if self.coop_bring_next_to(i, to) {
                self.log.push(format!("Log: Co-op: player {} brought through {} (card reader) next to player {}", i + 1, self.path_name(door), to + 1));
            }
        }
    }

    /// The player (rank, pawn) who made a scene go on: the one an escort's guard escorted, else
    /// the one who last made the story go on (touched a trigger, coop_cine.rs), else the one
    /// the scene was bound to. None for a door shut by a player themself (`scene` is that
    /// player's pawn).
    fn coop_door_lead(&self, scene: ObjId) -> Option<(usize, ObjId)> {
        if !self.coop_object_alive(scene) || !self.is_a(scene, "CineController2") {
            return None;
        }
        // An escort's guard (PRock01a's EscortManager): the player it escorted.
        let escorted = self.actors.iter().copied().filter(|&m| self.coop_object_alive(m) && self.is_a(m, "EscortManager")).find_map(|m| {
            let guard = ["PawnMaton1", "PawnMaton2"].iter().any(|g| {
                self.prop(m, g).as_object().is_some_and(|p| self.coop_object_alive(p) && self.prop(p, "CineController").as_object() == Some(scene))
            });
            guard.then(|| self.prop(m, "PlayerPawn").as_object()).flatten()
        });
        if let Some(rank) = escorted.and_then(|p| (0..self.coop.players.len()).find(|&i| self.coop_living_pawn(i) == Some(p))) {
            return Some((rank, self.coop_living_pawn(rank)?));
        }
        let pc = self.coop.cine_rules.last_player.filter(|&p| self.coop_object_alive(p)).or_else(|| self.coop_scene_bound(scene).map(|b| b.0))?;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        Some((rank, self.coop_living_pawn(rank)?))
    }

    /// `scene` (a playing CineController2) still has, after its current line, a line that
    /// opens `door`: "opendoor"/"od" with the door's name (or its number), or an event with
    /// the door's tag.
    fn coop_scene_reopens(&self, scene: ObjId, door: ObjId) -> bool {
        if !self.coop_object_alive(scene) || !self.is_a(scene, "CineController2") {
            return false;
        }
        let playing = self.current_state(scene).is_some_and(|(_, s)| {
            let s = s.name.to_ascii_lowercase();
            s == "playingsequence" || s == "preplayingsequence" || s.starts_with("sta_falling") || s.starts_with("sta_jumping")
        });
        let Some(cine) = self.prop(scene, "MyPawn").as_object().filter(|&c| playing && self.coop_object_alive(c)) else {
            return false;
        };
        let table = match self.prop(cine, "CurrentTabActionIndex").as_int() {
            2 => "tabActions2",
            3 => "tabActions3",
            _ => "tabActions",
        };
        let Value::Array(lines) = self.prop(cine, table) else { return false };
        let from = self.prop(scene, "ScriptedActionIndex").as_int().max(0) as usize;
        let name = self.objects[door].as_ref().map(|o| o.name.to_ascii_lowercase()).unwrap_or_default();
        let tag = self.prop(door, "Tag").as_name().to_ascii_lowercase();
        lines.iter().skip(from).any(|line| {
            let Value::Str(line) = line else { return false };
            let line = line.to_ascii_lowercase();
            let mut words = line.split_whitespace();
            let command = words.next().unwrap_or("");
            // "od porte 130": the words after the command, joined.
            let arg: String = words.collect();
            let arg = if !arg.is_empty() && arg.chars().all(|c| c.is_ascii_digit()) { format!("porte{arg}") } else { arg };
            match command {
                "opendoor" | "od" => arg == name,
                "event" | "ev" | "playerevent" => !tag.is_empty() && tag != "none" && arg == tag,
                _ => false,
            }
        })
    }

    /// A player at `pawn` can open the shut `door` again, as the door's own script decides:
    /// it reacts to players (used, bumped or stood at), it was not a one-time door, and it is not
    /// blocked for players on that side (`XIIIPorte.PlayerTriggerToggle.PlayerTrigger`). A door
    /// that only reacts to the map's events, or is locked, cannot be opened by a player.
    pub(crate) fn coop_player_can_open(&self, door: ObjId, pawn: ObjId) -> bool {
        let state = self.current_state(door).map(|(_, s)| s.name.to_ascii_lowercase()).unwrap_or_default();
        if !["player", "bump", "stand"].iter().any(|k| state.starts_with(k)) {
            return false;
        }
        if self.prop(door, "bTriggerOnceOnly").as_bool() || !self.prop(door, "bInteractive").as_bool() {
            return false;
        }
        if !self.is_a(door, "XIIIPorte") || !state.starts_with("player") {
            return true;
        }
        let blocked = self.prop(door, "BloqueePourLeJoueur").as_byte();
        if blocked == 0 {
            return true;
        }
        if blocked == 3 {
            return false;
        }
        let base = self.prop(door, "BasePos").as_vector();
        let at = self.prop(pawn, "Location").as_vector();
        let to_hinge = [base[0] - at[0], base[1] - at[1]];
        let dir = self.prop(door, "DoorDirection").as_vector();
        // Both ways: the side of the door's plane. One way: the side the player sees the hinge
        // on while looking at the middle of the door.
        let along = if self.prop(door, "bDansLes2Sens").as_bool() {
            dir
        } else {
            let loc = self.prop(door, "Location").as_vector();
            [loc[0] + dir[0] - at[0], loc[1] + dir[1] - at[1], 0.0]
        };
        let positive = along[0] * to_hinge[1] - along[1] * to_hinge[0] > 0.0;
        !(if positive { blocked == 1 } else { blocked == 2 })
    }

    /// `pawn` is a co-op player's pawn.
    fn coop_is_player_pawn(&self, pawn: ObjId) -> bool {
        (0..self.coop.players.len()).any(|i| self.coop_living_pawn(i) == Some(pawn))
    }

    /// Lift cages and cable cars: a journey with players on board that ends far from its start
    /// brings the players not on board next to a rider, wherever they are: the lift is called
    /// back only from inside.
    fn coop_lifts_frame(&mut self) {
        if self.coop.rules.lifts.is_none() {
            let found: Vec<ObjId> =
                self.actors.iter().copied().filter(|&a| self.coop_object_alive(a) && (self.is_a(a, "LiftCage") || self.is_a(a, "Telepherique"))).collect();
            self.coop.rules.lifts = Some(found);
        }
        let lifts = self.coop.rules.lifts.clone().unwrap_or_default();
        if lifts.is_empty() {
            return;
        }
        let now = self.level_time;
        let pawns: Vec<(usize, ObjId)> = (0..self.coop.players.len()).filter_map(|i| Some((i, self.coop_living_pawn(i)?))).collect();
        for lift in lifts {
            if !self.coop_object_alive(lift) {
                continue;
            }
            let at = self.prop(lift, "Location").as_vector();
            let on: Vec<usize> = pawns.iter().filter(|&&(_, p)| self.prop(p, "Base").as_object() == Some(lift)).map(|&(i, _)| i).collect();
            match self.coop.rules.trips.iter().position(|t| t.lift == lift) {
                None => {
                    // A journey starts with somebody on board.
                    if !on.is_empty() {
                        self.coop.rules.trips.push(Trip { lift, from: at, last: at, riders: on, moved: now });
                    }
                }
                Some(k) => {
                    let t = &mut self.coop.rules.trips[k];
                    for i in on {
                        if !t.riders.contains(&i) {
                            t.riders.push(i);
                        }
                    }
                    if (0..3).any(|c| (at[c] - t.last[c]).abs() > 0.5) {
                        t.last = at;
                        t.moved = now;
                        continue;
                    }
                    if now - t.moved < LIFT_STILL {
                        continue;
                    }
                    let t = self.coop.rules.trips.remove(k);
                    let dist = |a: [f32; 3], b: [f32; 3]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                    if dist(at, t.from) < LIFT_TRIP {
                        continue;
                    }
                    // A rider still near the lift.
                    let Some(&(rider, rider_pawn)) = pawns
                        .iter()
                        .filter(|(i, _)| t.riders.contains(i))
                        .min_by(|a, b| dist(self.prop(a.1, "Location").as_vector(), at).total_cmp(&dist(self.prop(b.1, "Location").as_vector(), at)))
                    else {
                        continue;
                    };
                    for &(i, p) in &pawns {
                        let here = self.prop(p, "Location").as_vector();
                        if t.riders.contains(&i) || self.coop_distance(p, rider_pawn) <= 400.0 {
                            continue;
                        }
                        if self.coop_bring_next_to(i, rider) {
                            self.coop_face_lift_door(i);
                            self.log.push(format!(
                                "Log: Co-op: player {} left behind by {}, brought next to player {}",
                                i + 1,
                                self.path_name(lift),
                                rider + 1
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Player `i` brought into a lift cage faces the middle of its doors (the way out), not
    /// the way the rider looks (often the cage's buttons, at the back).
    fn coop_face_lift_door(&mut self, i: usize) {
        let (Some(pawn), Some(&pc)) = (self.coop_living_pawn(i), self.coop.players.get(i)) else { return };
        let at = self.prop(pawn, "Location").as_vector();
        let doors: Vec<[f32; 3]> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.coop_object_alive(a) && self.is_a(a, "LiftDoor"))
            .map(|a| self.prop(a, "Location").as_vector())
            .filter(|l| ((l[0] - at[0]).powi(2) + (l[1] - at[1]).powi(2) + (l[2] - at[2]).powi(2)).sqrt() < 500.0)
            .collect();
        if doors.is_empty() {
            return;
        }
        let n = doors.len() as f32;
        let l = [doors.iter().map(|d| d[0]).sum::<f32>() / n, doors.iter().map(|d| d[1]).sum::<f32>() / n];
        let (dx, dy) = (l[0] - at[0], l[1] - at[1]);
        if dx.abs() + dy.abs() < 1.0 {
            return;
        }
        let yaw = (dy.datan2(dx) * (32768.0 / std::f32::consts::PI)).round() as i32 & 65535;
        self.set_prop(pc, "Rotation", Value::rotator(0, yaw, 0));
        self.set_prop(pawn, "Rotation", Value::rotator(0, yaw, 0));
    }

    /// A controller in a state a scene holds it in.
    pub(crate) fn coop_held_state(&self, pc: ObjId) -> bool {
        self.current_state(pc).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("NoControl") || s.name.eq_ignore_ascii_case("NoMove"))
    }

    /// A player held only because another player's scene holds its own (not the scene's player).
    pub(crate) fn coop_held_copy(&self, pc: ObjId) -> bool {
        self.coop.rules.held.iter().any(|h| h.pc == pc)
    }

}
