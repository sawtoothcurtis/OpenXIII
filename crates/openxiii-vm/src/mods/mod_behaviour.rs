//! Mods build only (feature `mods`): keys bound to mod actions (`[[action]]`) and mod characters
//! (`[character.<Name>]`), docs/modding/README.md.
//!
//! - Actions run from the player's key press (the game window, mods build) or from the
//!   `modkey <key>` console command.
//! - A character spawned by an action is created by the engine's SpawnActor while
//!   `Level.bStartup` is set, so it gets its AI controller exactly as the characters placed in a
//!   map do (Pawn.PostBeginPlay); its `properties` are written before its PreBeginPlay.
//! - Behaviour, through three generic hooks: GotoState (world.rs) keeps the character's AI
//!   controller in its `allowed_states` and starts or stops the per-state animation and sound;
//!   AllianceLevel (natives.rs) gives its `team` towards the player; `can_die = false` sets the
//!   controller's own `bGodMode` ("can't be killed or hurt").

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::sync::{Arc, OnceLock};

use openxiii_mods::behaviour::{ActionKind, CharacterDef, ModAction, Team};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Actions and characters of the mods.
#[derive(Debug, Default)]
pub struct ModBehaviour {
    pub actions: Vec<ModAction>,
    pub characters: Vec<CharacterDef>,
}

static GLOBAL: OnceLock<Arc<ModBehaviour>> = OnceLock::new();

/// Sets the actions and characters used by every Vm created afterwards (once, at startup).
pub fn set_mod_behaviour(actions: Vec<ModAction>, characters: Vec<CharacterDef>) {
    let _ = GLOBAL.set(Arc::new(ModBehaviour { actions, characters }));
}

pub(crate) fn global_mod_behaviour() -> Option<Arc<ModBehaviour>> {
    GLOBAL.get().cloned().filter(|b| !b.actions.is_empty() || !b.characters.is_empty())
}

/// A mod character alive in the level.
#[derive(Debug, Clone)]
pub(crate) struct LiveCharacter {
    /// Index in `ModBehaviour::characters`.
    pub def: usize,
    pub pawn: ObjId,
    /// Allowed states actually found in its controller's class (empty: no restriction).
    pub allowed: Vec<String>,
    /// State of its controller when it was created: its start-up (such as `init`, which sets
    /// up the AI), never redirected.
    pub start_state: String,
    /// The sound of its state's look, while it plays.
    pub look_sound: Option<LookSound>,
}

/// The sound of a mod character's state (`[character.<Name>.state.<State>] sound`): started at
/// `started` (level time), `seconds` long when its sound bank gives its length; when the state's
/// look loops, it starts again as it ends.
#[derive(Debug, Clone)]
pub(crate) struct LookSound {
    pub path: String,
    pub started: f32,
    pub seconds: Option<f32>,
    pub looping: bool,
}

fn yaw_units_to_radians(units: i32) -> f32 {
    units as f32 * std::f32::consts::TAU / 65536.0
}

impl Vm {
    /// Actions and characters of this Vm only (tests).
    pub fn set_mod_behaviour_here(&mut self, actions: Vec<ModAction>, characters: Vec<CharacterDef>) {
        self.mods.behaviour = Some(Arc::new(ModBehaviour { actions, characters }));
    }

    /// Keys bound to mod actions (as written), for the game window.
    pub fn mod_action_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.mods.behaviour.as_ref().map(|b| b.actions.iter().map(|a| a.key.clone()).collect()).unwrap_or_default();
        // And the keys of the scripts of this map (on("key", ...), mod_script.rs).
        keys.extend(self.mod_script_keys());
        // A network guest: the keys of the host's scripts (mod_script_keys.rs).
        keys.extend(self.mods.guest_keys.iter().cloned());
        keys
    }

    /// The mod character whose pawn or AI controller is `obj`.
    fn mod_character_of(&self, obj: ObjId) -> Option<&LiveCharacter> {
        self.mods.live.get(&obj).or_else(|| self.mods.live_controllers.get(&obj).and_then(|p| self.mods.live.get(p)))
    }

    fn mod_def(&self, live: &LiveCharacter) -> Option<CharacterDef> {
        self.mods.behaviour.as_ref().and_then(|b| b.characters.get(live.def).cloned())
    }

    /// Runs the action bound to `key` for player controller `pc`. False: no action on that key.
    pub fn mod_key(&mut self, pc: ObjId, key: &str) -> bool {
        // A network guest: a key of the host's scripts goes to the host (mod_script_keys.rs).
        if self.mod_guest_key(pc, key) {
            return true;
        }
        // The scripts' handlers of that key run at the next frame (mod_script.rs).
        let scripted = self.mod_script_key(pc, key);
        let Some(action) = self.mods.behaviour.as_ref().and_then(|b| b.actions.iter().find(|a| a.key.eq_ignore_ascii_case(key)).cloned()) else {
            return scripted;
        };
        match &action.kind {
            ActionKind::Spawn { what, distance } => {
                self.mod_spawn_in_front(pc, what, *distance, &action);
            }
            ActionKind::Give { class } => {
                let Some(pawn) = self.native_prop(pc, Field::Pawn).as_object() else { return true };
                if self.class(class).is_none() {
                    self.mod_report(format!("warning: key {}: unknown class {class} ({})", action.key, action.source));
                } else if let Some(f) = self.find_function(pawn, "GiveWeapon", false) {
                    if let Err(e) = self.call(f, pawn, vec![Some(Value::Str(class.clone()))]) {
                        self.mod_report(format!("warning: key {}: give {class}: {e} ({})", action.key, action.source));
                    }
                }
            }
            ActionKind::Sound { sound } => {
                let Some(pawn) = self.native_prop(pc, Field::Pawn).as_object() else { return true };
                if let Err(why) = self.mod_play_sound(pawn, sound, &format!("key {}", action.key)) {
                    self.mod_report(format!("warning: key {}: {why} ({})", action.key, action.source));
                }
            }
            ActionKind::Command { text } => {
                let (ok, out) = self.console_command_from_pc(pc, text);
                if !ok {
                    self.mod_report(format!("warning: key {}: the game did not take the command {text:?} ({})", action.key, action.source));
                } else if !out.is_empty() {
                    self.log.push(out);
                }
            }
        }
        true
    }

    /// Spawns a [character] (by name) or an actor of a class `distance` in front of the player,
    /// on the ground, facing the player.
    fn mod_spawn_in_front(&mut self, pc: ObjId, what: &str, distance: f32, action: &ModAction) -> Option<ObjId> {
        let pawn = self.native_prop(pc, Field::Pawn).as_object()?;
        let def_index = self.mods.behaviour.as_ref().and_then(|b| b.characters.iter().position(|c| c.name.eq_ignore_ascii_case(what)));
        let def = def_index.and_then(|i| self.mods.behaviour.as_ref().map(|b| b.characters[i].clone()));
        let class_name = def.as_ref().map_or(what.to_string(), |d| d.base.clone());
        let Some(class) = self.class(&class_name) else {
            let hint = if what.contains('.') { "" } else { " (no [character] of that name either)" };
            self.mod_report(format!("warning: key {}: unknown class {class_name}{hint} ({})", action.key, action.source));
            return None;
        };
        self.create_cdo(class);
        let yaw = self.native_value(pc, Field::Rotation).as_rotator()[1];
        let a = yaw_units_to_radians(yaw);
        let [x, y, z] = self.native_value(pawn, Field::Location).as_vector();
        let (tx, ty) = (x + a.dcos() * distance, y + a.dsin() * distance);
        // That point, else the first spot around it with room (at 150, 250 then 350 units, eight
        // directions), as a script's spawn{near = ...}.
        let mut spots = vec![(tx, ty)];
        for r in [150.0f32, 250.0, 350.0] {
            for i in 0..8 {
                let d = i as f32 * std::f32::consts::FRAC_PI_4;
                spots.push((tx + r * d.dcos(), ty + r * d.dsin()));
            }
        }
        // Nothing below (no level geometry loaded, or a hole): at the height of the player's feet.
        let feet = z - self.native_prop(pawn, Field::CollisionHeight).as_float();
        let height = self.native_prop(self.classes[class].cdo, Field::CollisionHeight).as_float();
        let facing = Value::rotator(0, yaw.wrapping_add(32768).rem_euclid(65536), 0);
        let mut found = None;
        for (sx, sy) in spots {
            // On the ground: trace down from the player's height.
            let mut hit = crate::physics::CheckResult::new(1.0);
            self.single_line_check(&mut hit, Some(pawn), [sx, sy, z - 2000.0], [sx, sy, z + 50.0], crate::physics::TRACE_WORLD, [0.0; 3]);
            let ground = if hit.time < 1.0 { z + 50.0 - hit.time * 2050.0 } else { feet };
            // Level.bStartup on during the spawn: a character gets its controller as a placed one.
            let level = self.level;
            let startup = level.map(|l| self.prop(l, "bStartup"));
            if let Some(l) = level {
                self.set_prop(l, "bStartup", Value::Bool(true));
            }
            self.mods.spawn_values = def.as_ref().map(|d| d.properties.clone());
            let r = self.spawn_actor(class, Value::Object(None), None, Value::vector(sx, sy, ground + height), facing.clone(), Value::Object(None), false);
            self.mods.spawn_values = None;
            if let (Some(l), Some(v)) = (level, startup) {
                self.set_prop(l, "bStartup", v);
            }
            match r {
                Ok(Some(s)) => {
                    found = Some((s, ground));
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    self.mod_report(format!("warning: key {}: {what}: {e} ({})", action.key, action.source));
                    return None;
                }
            }
        }
        let Some((spawned, ground)) = found else {
            self.mod_report(format!("warning: key {}: no room to create {what} in front of the player, nor around ({})", action.key, action.source));
            return None;
        };
        let name = self.objects[spawned].as_ref().map(|o| o.name.clone()).unwrap_or_default();
        let at = self.native_value(spawned, Field::Location).as_vector();
        let line = format!("mods: key {}: {what} created ({name} at [{:.0}, {:.0}, {:.0}], ground {ground:.0})", action.key, at[0], at[1], at[2]);
        openxiii_mods::log::note(&line);
        self.log.push(line);
        if let (Some(i), Some(d)) = (def_index, def) {
            self.mod_take_character(spawned, i, &d);
        }
        self.mod_check_look(spawned, &format!("key {}: {what}", action.key), &action.source.to_string());
        Some(spawned)
    }

    /// A character a mod adds that is drawn with the hero's mesh (XIIIPlayerPawn's): the game's
    /// character classes such as XIDPawn.BaseSoldier have no look of their own, each character
    /// placed in a map gets its Mesh from the level designer. One warning, with what to do.
    pub(crate) fn mod_check_look(&mut self, a: ObjId, what: &str, source: &str) {
        let Some(pawn) = self.class("Engine.Pawn") else { return };
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        if !self.is_child_of(class, pawn) || self.class("XIII.XIIIPlayerPawn").is_some_and(|p| self.is_child_of(class, p)) {
            return;
        }
        let Some(hero) = self.class("XIII.XIIIPlayerPawn") else { return };
        self.create_cdo(hero);
        let hero_mesh = self.prop(self.classes[hero].cdo, "Mesh");
        let mesh = self.prop(a, "Mesh");
        if mesh == hero_mesh || matches!(mesh, Value::Object(None) | Value::Void) {
            self.mod_report(format!(
                "warning: {what}: it is drawn with XIII's own look ({}), its class has no look of its own; give it a Mesh (and Skins), for example Mesh = \"XIIIPersos.SpadsM\" (characters.md lists the looks) ({source})",
                self.class_path(class)
            ));
        }
    }

    /// Registers a spawned mod character and applies its team, states and can_die.
    pub(crate) fn mod_take_character(&mut self, pawn: ObjId, def: usize, d: &CharacterDef) {
        let controller = self.native_prop(pawn, Field::Controller).as_object();
        // Allowed states that exist in the controller's class.
        let mut allowed = Vec::new();
        if let Some(c) = controller {
            let class = self.objects[c].as_ref().map(|o| o.class);
            for s in &d.allowed_states {
                match class.and_then(|k| self.find_state(k, s).map(|(_, sd)| sd.name.clone())) {
                    Some(name) => allowed.push(name),
                    None => {
                        let list = class.map(|k| self.mod_state_names(k).join(", ")).unwrap_or_default();
                        self.mod_report(format!("warning: character {}: its AI has no state {s} (states: {list}) ({})", d.name, d.source));
                    }
                }
            }
        } else if !d.allowed_states.is_empty() || d.team.is_some() || !d.can_die {
            self.mod_report(format!(
                "warning: character {}: {} has no AI controller: team, allowed_states and can_die do nothing ({})",
                d.name, d.base, d.source
            ));
        }
        // The controller's start-up state runs as usual (it sets up the AI); the state it asks
        // for next is the first one checked.
        let start_state = controller.and_then(|c| self.objects[c].as_ref().and_then(|o| o.state.clone())).unwrap_or_default();
        self.mods.live.insert(pawn, LiveCharacter { def, pawn, allowed, start_state, look_sound: None });
        if let Some(c) = controller {
            self.mods.live_controllers.insert(c, pawn);
            if !d.can_die {
                self.set_prop(c, "bGodMode", Value::Bool(true));
            }
            self.mod_state_entered(c, "");
        }
    }

    /// States of class `class` and its parents, sorted.
    pub(crate) fn mod_state_names(&self, class: crate::value::ClassId) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut k = Some(class);
        while let Some(c) = k {
            for sd in self.classes[c].states.values() {
                if !out.iter().any(|n| n.eq_ignore_ascii_case(&sd.name)) {
                    out.push(sd.name.clone());
                }
            }
            k = self.classes[c].super_class;
        }
        out.sort_by_key(|s| s.to_lowercase());
        out
    }

    /// GotoState hook, before the change: the state a mod character's AI controller is sent to
    /// instead of `state` when `state` is not one of its allowed states. None: no change.
    pub(crate) fn mod_redirect_state(&mut self, obj: ObjId, state: &str) -> Option<String> {
        let pawn = *self.mods.live_controllers.get(&obj)?;
        let live = self.mods.live.get(&pawn)?.clone();
        let first = live.allowed.first()?.clone();
        // A dead character follows the game's own death states.
        if self.prop(pawn, "Health").as_int() <= 0 || self.objects[pawn].is_none() {
            return None;
        }
        if state.is_empty() || state.eq_ignore_ascii_case(&live.start_state) || live.allowed.iter().any(|s| s.eq_ignore_ascii_case(state)) {
            return None;
        }
        self.mod_redirects += 1;
        let name = self.mod_def(&live).map(|d| d.name).unwrap_or_default();
        self.mod_report(format!("character {name}: state {state} kept in {first}"));
        Some(first)
    }

    /// GotoState hook, after the change: the look of the new state (animation, sound) of a mod
    /// character's controller, or its death animation when its pawn starts dying.
    pub(crate) fn mod_state_entered(&mut self, obj: ObjId, old: &str) {
        let Some(live) = self.mod_character_of(obj).cloned() else { return };
        let Some(def) = self.mod_def(&live) else { return };
        let now = self.objects[obj].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
        let pawn = live.pawn;
        if obj == pawn {
            if now.eq_ignore_ascii_case("Dying") {
                // Its state's sound ends with it (a song, a call...).
                self.mod_stop_look_sound(pawn);
                if let Some(anim) = &def.death_animation {
                    self.mod_play(pawn, &def, anim, false);
                }
            }
            return;
        }
        let look = |s: &str| def.states.iter().find(|l| l.state.eq_ignore_ascii_case(s)).cloned();
        if let (Some(previous), false) = (look(old), old.eq_ignore_ascii_case(&now)) {
            // Leaving a state with a look: its loop stops, and the character's own animation code
            // takes over at the next AnimEnd.
            if previous.animation.is_some() && previous.looping && self.objects[pawn].is_some() {
                let mut args = [];
                let _ = crate::natives::call(self, "StopAnimating", pawn, &mut args, &[]);
                self.event_pub(pawn, "AnimEnd", vec![Value::Int(0)]);
            }
            // And its sound stops.
            if previous.sound.is_some() {
                self.mod_stop_look_sound(pawn);
            }
        }
        // The same state asked for again (the AI's reaction to the player, kept in its allowed
        // state): the character never left it, its look goes on as it is. Started again, the
        // sound would play a second time over the first one.
        let again = !old.is_empty() && old.eq_ignore_ascii_case(&now);
        if let Some(l) = look(&now) {
            if let Some(anim) = &l.animation {
                if !again {
                    self.mod_play(pawn, &def, anim, l.looping);
                }
            }
            if let Some(sound) = &l.sound {
                let playing = self.mods.live.get(&pawn).is_some_and(|x| x.look_sound.as_ref().is_some_and(|s| s.path.eq_ignore_ascii_case(sound)));
                if !(again && playing) {
                    self.mod_sound(pawn, &def, sound, l.looping);
                }
            }
        }
    }

    /// End of each tick: a mod character in a state whose look loops gets its animation back
    /// if the state's own code played another one (the AI often restarts its state, which plays
    /// the state's usual animation once).
    pub(crate) fn mod_keep_looks(&mut self) {
        let live: Vec<LiveCharacter> = self.mods.live.values().cloned().collect();
        for l in live {
            if self.objects.get(l.pawn).is_none_or(|o| o.is_none()) || self.prop(l.pawn, "Health").as_int() <= 0 {
                continue;
            }
            // A looping state sound that has ended starts again.
            if let Some(s) = l.look_sound.as_ref().filter(|s| s.looping) {
                let now = self.mod_level_time();
                if s.seconds.is_some_and(|len| now - s.started >= len) {
                    let why = format!("character {}", self.mod_def(&l).map(|d| d.name).unwrap_or_default());
                    let _ = self.mod_play_sound(l.pawn, &s.path, &why);
                    if let Some(x) = self.mods.live.get_mut(&l.pawn).and_then(|x| x.look_sound.as_mut()) {
                        x.started = now;
                    }
                }
            }
            let Some(&c) = self.mods.live_controllers.iter().find(|(_, &p)| p == l.pawn).map(|(c, _)| c) else { continue };
            let Some(def) = self.mod_def(&l) else { continue };
            let now = self.objects[c].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
            let Some(look) = def.states.iter().find(|s| s.state.eq_ignore_ascii_case(&now) && s.looping) else { continue };
            let Some(anim) = &look.animation else { continue };
            let playing = self.mesh_insts.get(&l.pawn).map(|i| (i.active_state(0).0, i.in_loop(0)));
            if !matches!(&playing, Some((name, true)) if name.eq_ignore_ascii_case(anim)) {
                self.play_anim(l.pawn, 0, anim, 1.0, 0.2, true);
            }
            self.mod_body_layers_off(l.pawn);
        }
    }

    /// The looping animation of a state is the whole body's: the layers blended over channel 0
    /// are turned off. XIIIPawn.PlayWaiting, run at each AnimEnd of channel 0, plays the
    /// weapon's waiting animation on channel 12 with AnimBlendParams(12, 1.0, .., 'X Spine'),
    /// which would otherwise keep the whole upper body in that waiting pose (as a Cine2 looping
    /// its scripted animation, LoopAnim on channel 0 alone).
    fn mod_body_layers_off(&mut self, pawn: ObjId) {
        let Some(inst) = self.mesh_insts.get_mut(&pawn) else { return };
        for c in inst.channels.iter_mut().skip(1) {
            if c.blend_alpha > 0.0 || (c.timed_blend && c.blend_target > 0.0) {
                c.blend_alpha = 0.0;
                c.blend_target = 0.0;
                c.timed_blend = false;
                c.time_to_go = 0.0;
            }
        }
    }

    fn mod_play(&mut self, pawn: ObjId, def: &CharacterDef, anim: &str, looping: bool) {
        if self.objects[pawn].is_none() {
            return;
        }
        if self.play_anim(pawn, 0, anim, 1.0, 0.2, looping) {
            if looping {
                self.mod_body_layers_off(pawn);
            }
            return;
        }
        let names: Vec<String> =
            self.mesh_insts.get(&pawn).map(|i| i.anims.iter().flat_map(|s| s.seqs.iter().map(|q| q.name.clone())).collect()).unwrap_or_default();
        self.mod_report(format!("warning: character {}: its mesh has no animation {anim} (animations: {}) ({})", def.name, names.join(", "), def.source));
    }

    /// Plays the sound of a character's state, from the character; a sound of any level (its
    /// bank is loaded when needed, mod_sounds.rs). `looping`: started again as it ends.
    fn mod_sound(&mut self, pawn: ObjId, def: &CharacterDef, sound: &str, looping: bool) {
        if let Err(why) = self.mod_play_sound(pawn, sound, &format!("character {}", def.name)) {
            self.mod_report(format!("warning: character {}: {why} ({})", def.name, def.source));
            return;
        }
        let seconds = self.mod_sound_seconds(sound);
        if looping && seconds.is_none() && self.audio.is_some() {
            self.mod_report(format!(
                "warning: character {}: sound {sound} is a sound program of the game, its length is not known: it is played once, not in a loop ({})",
                def.name, def.source
            ));
        }
        let started = self.mod_level_time();
        if let Some(l) = self.mods.live.get_mut(&pawn) {
            l.look_sound = Some(LookSound { path: sound.to_string(), started, seconds, looping });
        }
    }

    /// Stops the state sound of mod character `pawn` (its death, a new state, its removal).
    pub(crate) fn mod_stop_look_sound(&mut self, pawn: ObjId) {
        let Some(s) = self.mods.live.get_mut(&pawn).and_then(|l| l.look_sound.take()) else { return };
        if self.objects.get(pawn).is_some_and(|o| o.is_some()) {
            let mut args = [Value::Asset(crate::mod_sounds::object_path(&s.path).to_string())];
            let _ = crate::natives::call(self, "StopSound", pawn, &mut args, &[true]);
        }
    }

    /// `restart`: the look of mod character `pawn`'s state starts again from the beginning, its
    /// animation from its first frame and its sound. False: not a mods [character] alive, or no
    /// look in its state.
    pub(crate) fn mod_restart_look(&mut self, pawn: ObjId) -> bool {
        let Some(live) = self.mods.live.get(&pawn).cloned() else { return false };
        if !self.mod_alive(pawn) {
            return false;
        }
        let Some(def) = self.mod_def(&live) else { return false };
        let Some(c) = self.mods.live_controllers.iter().find(|(_, &p)| p == pawn).map(|(&c, _)| c) else { return false };
        let now = self.objects[c].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
        let Some(look) = def.states.iter().find(|s| s.state.eq_ignore_ascii_case(&now)).cloned() else { return false };
        if let Some(anim) = &look.animation {
            let mut args = [];
            let _ = crate::natives::call(self, "StopAnimating", pawn, &mut args, &[]);
            let _ = self.play_anim(pawn, 0, anim, 1.0, 0.0, look.looping);
        }
        if let Some(sound) = &look.sound {
            self.mod_stop_look_sound(pawn);
            self.mod_sound(pawn, &def, sound, look.looping);
        }
        true
    }

    /// AllianceLevel hook: how a mod character with a `team` treats the player.
    pub(crate) fn mod_alliance(&self, controller: ObjId, other: Option<ObjId>) -> Option<i32> {
        // A team given by a script (mod_script.rs) to any character, else the mods [character]'s.
        let scripted = (!self.mods.script_teams.is_empty())
            .then(|| self.native_prop(controller, Field::Pawn).as_object())
            .flatten()
            .and_then(|p| self.mods.script_teams.get(&p).copied());
        let team = match scripted {
            Some(t) => t,
            None => {
                let pawn = *self.mods.live_controllers.get(&controller)?;
                let live = self.mods.live.get(&pawn)?;
                self.mod_def(live)?.team?
            }
        };
        // The player: a pawn driven by a player controller (the AI's own XIII field is only set
        // once its start-up has run).
        let other = other?;
        let driver = self.native_prop(other, Field::Controller).as_object()?;
        if !self.is_a(driver, "PlayerController") {
            return None;
        }
        Some(match team {
            Team::Friend => 1,
            Team::Neutral => 0,
            Team::Enemy => -1,
        })
    }

    /// Mod characters alive (pawn, controller), for tests and the debug dump.
    pub fn mod_characters_alive(&self) -> Vec<(ObjId, Option<ObjId>)> {
        let mut v: Vec<(ObjId, Option<ObjId>)> = self
            .mods.live
            .keys()
            .filter(|&&p| self.objects.get(p).is_some_and(|o| o.is_some()))
            .map(|&p| (p, self.mods.live_controllers.iter().find(|(_, &x)| x == p).map(|(&c, _)| c)))
            .collect();
        v.sort();
        v
    }
}
