//! Mods build only (feature `mods`): changes to the actors of a map (`[[map.<Map>.add]]`,
//! `remove`, `replace`, `start`, docs/modding/README.md), and the `modpos` and `modkey <key>`
//! console commands.
//!
//! When they happen, as the level starts (`begin_play`):
//! - removals, and the actors a replacement takes away, before any script of the level runs:
//!   the actor leaves the level's actor list, as the actors deleted in the level editor do;
//! - additions, and the actors a replacement puts back, at the end of the start-up, while
//!   `Level.bStartup` is still true: they are spawned by the engine's SpawnActor, so they get
//!   PreBeginPlay, BeginPlay, PostBeginPlay… like any spawned actor, and a character gets its
//!   AI controller as the characters placed in the map do (Pawn.PostBeginPlay);
//! - start weapons and ammo, when the player has logged in (after GameInfo.PostLogin), with the
//!   game's own Pawn.GiveWeapon.
//!
//! XIII's saved games are checkpoints: the level is loaded again from its map file, then the
//! inventory is restored. So the changes are made again, once, at each load: added actors are
//! neither lost nor doubled, removed ones stay removed. A level whose actors were saved with it
//! (`bBegunPlay`) is not changed again.

use std::sync::{Arc, OnceLock};

use openxiii_mods::placement::{ActorValue, Edit, MapEdit};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

static GLOBAL: OnceLock<Arc<Vec<MapEdit>>> = OnceLock::new();

/// Sets the map changes used by every Vm created afterwards (once, at startup). A second call
/// is ignored.
pub fn set_mod_edits(list: Vec<MapEdit>) {
    let _ = GLOBAL.set(Arc::new(list));
}

pub(crate) fn global_mod_edits() -> Option<Arc<Vec<MapEdit>>> {
    GLOBAL.get().cloned().filter(|l| !l.is_empty())
}

/// An actor to spawn at the end of the level start-up.
#[derive(Debug, Clone)]
pub(crate) struct PendingSpawn {
    pub edit: MapEdit,
    pub class: crate::value::ClassId,
    pub location: [f32; 3],
    pub rotation: Value,
    /// Tag and Event of the actor a replacement took away.
    pub tag_event: Option<(Value, Value)>,
    pub properties: Vec<ActorValue>,
    /// `character = "<Name>"`: the [character] it is (index in ModBehaviour::characters).
    pub character: Option<usize>,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn yaw_units(degrees: f32) -> i32 {
    (degrees * 65536.0 / 360.0).round() as i32
}

impl Vm {
    /// Map changes of this Vm only (tests). Must be called before the map is started.
    pub fn set_mod_edits_here(&mut self, list: Vec<MapEdit>) {
        self.mods.edits = Some(Arc::new(list)).filter(|l| !l.is_empty());
    }

    /// Changes of the map being played, in mod order.
    /// Changes of the map being played, in mod order: those of every selector that is for it
    /// (the map's name, "Hual*", "solo", "*"…). Two replacements of the same class: the most
    /// precise selector's wins.
    pub(crate) fn mod_edits_here(&self) -> Vec<MapEdit> {
        let map = self.mod_map_name();
        let Some(list) = self.mods.edits.as_ref() else { return Vec::new() };
        // Changes for some checkpoints only (from_checkpoint, checkpoint.<N>.start): the
        // checkpoint the level starts from.
        let checkpoint = if list.iter().any(|e| e.checkpoint != openxiii_mods::placement::Checkpoint::Any) { self.mod_checkpoint_here() } else { 1 };
        let here: Vec<(u32, &MapEdit)> = list.iter().filter(|e| e.checkpoint.applies(checkpoint)).filter_map(|e| openxiii_mods::maps::rank(&e.map, &map).map(|r| (r, e))).collect();
        let best_replace = |from: &str| here.iter().filter(|(_, e)| matches!(&e.edit, Edit::Replace { from: f, .. } if f.eq_ignore_ascii_case(from))).map(|(r, _)| *r).max();
        here.iter()
            .filter(|(r, e)| match &e.edit {
                Edit::Replace { from, .. } => best_replace(from) == Some(*r),
                _ => true,
            })
            .map(|(_, e)| (*e).clone())
            .collect()
    }

    /// The level is a saved one whose actors already carry the changes.
    fn mod_level_restored(&self) -> bool {
        self.level.is_some_and(|l| self.native_prop(l, Field::BegunPlay).as_bool())
    }

    pub(crate) fn mod_actor_name(&self, a: ObjId) -> String {
        self.objects.get(a).and_then(|o| o.as_ref()).map_or_else(|| format!("#{a}"), |o| o.name.clone())
    }

    /// Takes actor `a` out of the level before it starts, as the level editor's deleted actors.
    fn mod_take_out(&mut self, a: ObjId) {
        self.actors.retain(|&x| x != a);
        self.set_prop(a, "bDeleteMe", Value::Bool(true));
        self.actors_changed();
    }

    /// Start of begin_play (before any script): removals, and the actors replacements take away.
    pub(crate) fn apply_mod_removals(&mut self) {
        self.mods.pending.clear();
        if self.mod_net_client() {
            return;
        }
        if self.mod_level_restored() {
            return;
        }
        let map = self.mod_map_name();
        for e in self.mod_edits_here() {
            let found: Vec<ObjId> = match &e.edit {
                Edit::RemoveName { name } => self.actors.iter().copied().filter(|&a| self.mod_actor_name(a).eq_ignore_ascii_case(name)).collect(),
                Edit::RemoveNear { class, near, radius } => {
                    let Some(c) = self.class(class) else {
                        self.mod_report(format!("warning: {map}: {}: unknown class {class} ({})", e.describe(), e.source));
                        continue;
                    };
                    self.actors
                        .iter()
                        .copied()
                        .filter(|&a| self.objects[a].as_ref().is_some_and(|o| self.is_child_of(o.class, c)))
                        .filter(|&a| distance(self.native_value(a, Field::Location).as_vector(), *near) <= *radius)
                        .collect()
                }
                Edit::Replace { from, to } => {
                    let (Some(fc), Some(tc)) = (self.class(from), self.class(to)) else {
                        let unknown = if self.class(from).is_none() { from } else { to };
                        self.mod_report(format!("warning: {map}: {}: unknown class {unknown} ({})", e.describe(), e.source));
                        continue;
                    };
                    let found: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.objects[a].as_ref().is_some_and(|o| o.class == fc)).collect();
                    for &a in &found {
                        if self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool() {
                            continue;
                        }
                        let tag_event = Some((self.prop(a, "Tag"), self.prop(a, "Event")));
                        self.mods.pending.push(PendingSpawn {
                            edit: e.clone(),
                            class: tc,
                            location: self.native_value(a, Field::Location).as_vector(),
                            rotation: self.native_value(a, Field::Rotation),
                            tag_event,
                            properties: Vec::new(),
                            character: None,
                        });
                    }
                    found
                }
                _ => continue,
            };
            // Static actors are part of the level's geometry and cannot be taken out.
            let (fixed, found): (Vec<ObjId>, Vec<ObjId>) = found.into_iter().partition(|&a| self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool());
            if !fixed.is_empty() {
                let names: Vec<String> = fixed.iter().map(|&a| self.mod_actor_name(a)).collect();
                self.mod_report(format!("warning: {map}: {}: {} is part of the level (bStatic or bNoDelete) and stays ({})", e.describe(), names.join(", "), e.source));
            }
            if found.is_empty() {
                if fixed.is_empty() {
                    self.mod_report(format!("warning: {map}: {}: no such actor in the map ({})", e.describe(), e.source));
                }
                continue;
            }
            let names: Vec<String> = found.iter().map(|&a| self.mod_actor_name(a)).collect();
            for a in found {
                self.mod_take_out(a);
            }
            self.mod_report(format!("{map}: {}: {} ({})", e.describe(), names.join(", "), e.source));
        }
        // Additions: checked now, spawned at the end of the start-up.
        for e in self.mod_edits_here() {
            if let Edit::Add { class, position, yaw, properties } = &e.edit {
                // `character = "<Name>"`: that [character]'s base class and values, then the entry's.
                let mut properties = properties.clone();
                let mut character = None;
                let mut class = class.clone();
                if let Some(name) = class.strip_prefix(openxiii_mods::placement::CHARACTER_PREFIX).map(str::to_string) {
                    let found = self.mods.behaviour.as_ref().and_then(|b| b.characters.iter().position(|d| d.name.eq_ignore_ascii_case(&name)).map(|i| (i, b.characters[i].clone())));
                    let Some((i, def)) = found else {
                        self.mod_report(format!("warning: {map}: {}: no [character.{name}] in the mods ({})", e.describe(), e.source));
                        continue;
                    };
                    let mut values = def.properties.clone();
                    values.extend(properties);
                    properties = values;
                    character = Some(i);
                    class = def.base.clone();
                }
                let class = &class;
                let Some(c) = self.class(class) else {
                    self.mod_report(format!("warning: {map}: {}: unknown class {class} ({})", e.describe(), e.source));
                    continue;
                };
                self.create_cdo(c);
                // `position` is the point on the ground: the actor's centre is its collision
                // height above it.
                let height = self.native_prop(self.classes[c].cdo, Field::CollisionHeight).as_float();
                let rotation = Value::rotator(0, yaw_units(yaw.unwrap_or(0.0)), 0);
                self.mods.pending.push(PendingSpawn {
                    edit: e.clone(),
                    class: c,
                    location: [position[0], position[1], position[2] + height],
                    rotation,
                    tag_event: None,
                    properties: properties.clone(),
                    character,
                });
            }
        }
    }

    /// A network client: the server changes the map's actors and gives the start weapons, and
    /// they reach the client by replication (a client doing it too would double them).
    pub(crate) fn mod_net_client(&self) -> bool {
        self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_some())
    }

    /// End of begin_play (Level.bStartup still true): the actors of additions and replacements,
    /// prepared by apply_mod_removals (nothing for a saved level: begin_play itself sets
    /// bBegunPlay by now, so the check is made at the start).
    pub(crate) fn apply_mod_additions(&mut self) {
        let map = self.mod_map_name();
        for p in std::mem::take(&mut self.mods.pending) {
            let [x, y, z] = p.location;
            self.mods.spawn_values = Some(p.properties.clone());
            let r = self.spawn_actor(p.class, Value::Object(None), None, Value::vector(x, y, z), p.rotation.clone(), Value::Object(None), false);
            self.mods.spawn_values = None;
            let what = p.edit.describe();
            match r {
                Ok(Some(a)) => {
                    // Outside the level's geometry: no BSP leaf.
                    let outside = matches!(self.native_value(a, Field::Region), Value::Struct(_, ref f) if matches!(f.get(1), Some(Value::Int(-1))));
                    if outside {
                        let _ = self.destroy_actor(a);
                        self.mod_report(format!("warning: {map}: {what}: the position is outside the map, actor not added ({})", p.edit.source));
                        continue;
                    }
                    if let Some((tag, event)) = p.tag_event {
                        self.set_prop(a, "Tag", tag);
                        self.set_prop(a, "Event", event);
                    }
                    let at = self.native_value(a, Field::Location).as_vector();
                    self.mod_report(format!("{map}: {what}: {} at [{:.0}, {:.0}, {:.0}] ({})", self.mod_actor_name(a), at[0], at[1], at[2], p.edit.source));
                    if let Some(i) = p.character {
                        if let Some(def) = self.mods.behaviour.as_ref().and_then(|b| b.characters.get(i).cloned()) {
                            self.mod_take_character(a, i, &def);
                        }
                    }
                    self.mod_check_look(a, &format!("{map}: {what}"), &p.edit.source.to_string());
                }
                Ok(None) => self.mod_report(format!("warning: {map}: {what}: the game could not place it (no room there, or the class cannot be spawned) ({})", p.edit.source)),
                Err(e) => self.mod_report(format!("warning: {map}: {what}: {e} ({})", p.edit.source)),
            }
        }
        // Sounds replaced on this map, before the level's sounds are loaded.
        self.mod_sound_swaps_start();
        // Lines said by characters, once the added characters are there.
        self.mod_dialogues_start();
        // The mods' scripts, last: they find the map as the other changes left it.
        self.mod_scripts_start();
    }

    /// In spawn_actor, before PreBeginPlay: the `properties` of the actor a mod adds.
    pub(crate) fn apply_mod_spawn_values(&mut self, a: ObjId) {
        let Some(values) = self.mods.spawn_values.take() else { return };
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let Some(mut props) = self.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { return };
        let mut errors = Vec::new();
        for v in &values {
            if let Err(why) = props.with_slice(|s| self.mod_set_value(class, s, &v.property, v.index, &v.value)) {
                errors.push(format!("{}: {why}", v.property_label()));
            }
        }
        if let Some(o) = self.objects[a].as_mut() {
            o.props = props;
        }
        for e in errors {
            let line = format!("warning: {}: properties: {e}, value ignored", self.mod_actor_name(a));
            self.mod_report(line);
        }
        // Skins of an added actor (simple `skin`, or raw Skins): sent to the network guests,
        // which do not receive skins from the game (mod_net_skins.rs).
        if values.iter().any(|v| v.property.eq_ignore_ascii_case("Skins")) {
            self.mod_net_note_skins(a);
        }
    }

    /// After GameInfo.PostLogin: start weapons (Pawn.GiveWeapon, which skips a weapon the player
    /// already has), then start ammo amounts (at most the ammo's MaxAmmo).
    pub(crate) fn apply_mod_start(&mut self, pc: ObjId) {
        if self.mod_net_client() {
            return;
        }
        let Some(pawn) = self.native_prop(pc, Field::Pawn).as_object() else { return };
        let map = self.mod_map_name();
        for e in self.mod_edits_here() {
            match &e.edit {
                Edit::StartWeapon { class } => {
                    if self.class(class).is_none() {
                        self.mod_report(format!("warning: {map}: {}: unknown class {class} ({})", e.describe(), e.source));
                        continue;
                    }
                    let Some(f) = self.find_function(pawn, "GiveWeapon", false) else { continue };
                    match self.call(f, pawn, vec![Some(Value::Str(class.clone()))]) {
                        Ok(_) => self.mod_report(format!("{map}: {} ({})", e.describe(), e.source)),
                        Err(err) => self.mod_report(format!("warning: {map}: {}: {err} ({})", e.describe(), e.source)),
                    }
                }
                Edit::StartAmmo { class, amount } => {
                    let Some(c) = self.class(class) else {
                        self.mod_report(format!("warning: {map}: {}: unknown class {class} ({})", e.describe(), e.source));
                        continue;
                    };
                    let mut item = self.prop(pawn, "Inventory").as_object();
                    let mut found = None;
                    while let Some(i) = item {
                        if self.objects.get(i).and_then(|o| o.as_ref()).is_some_and(|o| self.is_child_of(o.class, c)) {
                            found = Some(i);
                            break;
                        }
                        item = self.prop(i, "Inventory").as_object();
                    }
                    let Some(ammo) = found else {
                        self.mod_report(format!("warning: {map}: {}: the player has no {class} (give a weapon that uses it) ({})", e.describe(), e.source));
                        continue;
                    };
                    let max = match self.prop(ammo, "MaxAmmo") {
                        Value::Int(m) if m > 0 => m,
                        _ => i32::MAX,
                    };
                    self.set_prop(ammo, "AmmoAmount", Value::Int((*amount).min(max)));
                    self.mod_report(format!("{map}: {} ({})", e.describe(), e.source));
                }
                _ => {}
            }
        }
    }

    /// `modpos` console command: the player's position (on the ground) and yaw, ready to paste
    /// into an `[[map.<Map>.add]]` entry. Also written to the log.
    pub(crate) fn mod_console(&mut self, pc: ObjId, command: &str) -> Option<String> {
        // modkey <key>: the action bound to that key, as if it was pressed (mod_behaviour.rs).
        let words: Vec<&str> = command.split_whitespace().collect();
        if let [w, key] = words.as_slice() {
            if w.eq_ignore_ascii_case("modkey") {
                return Some(if self.mod_key(pc, key) { format!("modkey {key}") } else { format!("modkey: no action on key {key}") });
            }
        }
        // modwho: what the player looks at (mod_change.rs).
        if command.trim().eq_ignore_ascii_case("modwho") {
            return Some(self.mod_who(pc));
        }
        // modreload: the scripts of this map, started again from their files (mod_script.rs).
        if command.trim().eq_ignore_ascii_case("modreload") {
            let text = self.mod_scripts_reload();
            self.log.push(text.clone());
            return Some(text);
        }
        if !command.trim().eq_ignore_ascii_case("modpos") {
            return None;
        }
        let Some(pawn) = self.native_prop(pc, Field::Pawn).as_object() else {
            return Some("modpos: no player".into());
        };
        let [x, y, z] = self.native_value(pawn, Field::Location).as_vector();
        let ground = z - self.native_prop(pawn, Field::CollisionHeight).as_float();
        let units = self.native_value(pc, Field::Rotation).as_rotator()[1];
        let yaw = (units.rem_euclid(65536) as f32 * 360.0 / 65536.0).round() as i32 % 360;
        let text = format!("# {}\nposition = [{x:.0}, {y:.0}, {ground:.0}]\nyaw = {yaw}", self.mod_map_name());
        for line in text.lines() {
            self.log.push(format!("modpos: {line}"));
        }
        // Also in the mods log (Mods/mods-log.txt), with the eye's height and the view's pitch, so
        // that a place read in the console can be found again after the game is closed.
        let eye = z + self.propr(pawn, "BaseEyeHeight").as_float();
        let pitch = (self.native_value(pc, Field::Rotation).as_rotator()[0] + 32768).rem_euclid(65536) - 32768;
        openxiii_mods::log::note(&format!(
            "modpos: {} position [{x:.0}, {y:.0}, {ground:.0}] yaw {yaw} (centre z {z:.0}, eye z {eye:.0}, yaw units {}, pitch units {pitch})",
            self.mod_map_name(),
            units.rem_euclid(65536)
        ));
        // On screen too, as the game's messages: the console closes on Enter and does not show
        // what a command writes.
        for line in text.lines() {
            let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line.to_string()), Value::Name("None".into())]);
        }
        Some(text)
    }
}
