//! Mods build only (feature `mods`): more of the scripts' functions (mod_script.rs holds the API
//! table that names them):
//! - what the combat events need: the shots a character fires (its weapon's `FlashCount`, which
//!   the game's `PlayFiring` raises at every shot, read each frame only while an `on("fire")`
//!   handler exists) and who hit a character and with what (the game's `TakeDamage` calls, noted
//!   only while an `on("damage")` handler exists);
//! - shortcuts on actors: `is_a`, `trigger`, `ammo`, `has`, `take`, `aim`;
//! - `game()`, `preload()`, the sounds a script can stop (`stop_sound`, `music_stop`).
//!
//! Nothing here runs without a script that uses it.

use std::collections::{BTreeMap, BTreeSet};

use mlua::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Lua, MultiValue};

use crate::mod_script::{caller, Actor, Who};
use crate::mod_script_game::this;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

/// Kinds of the messages that stop a sound or the music on a network guest.
pub(crate) const STOP_SOUND_KIND: &str = "StopSound";
pub(crate) const STOP_MUSIC_KIND: &str = "StopMusic";

/// The last hit an actor took (the game's TakeDamage): who caused it and with what.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Hit {
    pub by: Option<ObjId>,
    /// The damage type's class ("XIII.DTGunned").
    pub damage_type: Option<String>,
    /// The class of the weapon in the hands of who caused it, then.
    pub weapon: Option<String>,
}

/// The state of these functions for the current VM.
#[derive(Default)]
pub struct More {
    /// An `on("damage")` handler exists: the game's TakeDamage calls are noted.
    pub(crate) watch_hits: bool,
    /// The hits since the scripts' last frame, by actor (sorted: the same order on every run).
    pub(crate) hits: BTreeMap<ObjId, Hit>,
    /// preload(): the objects the scripts asked for, sorted.
    pub(crate) preload: BTreeSet<String>,
    /// Literal object and class names in successfully compiled Lua source for this map.
    pub(crate) script_references: BTreeSet<String>,
    /// preload() is allowed (the scripts' main code and their "start" handlers).
    pub(crate) preload_open: bool,
    /// sound(): the sounds played, by id: (the actor they play from, the sound).
    pub(crate) sounds: BTreeMap<u64, (ObjId, String)>,
    pub(crate) next_sound: u64,
}

/// A character's weapon as the fire event last saw it: (weapon, FlashCount, AltFlashCount).
pub(crate) type FireSeen = BTreeMap<ObjId, (Option<ObjId>, i32, i32)>;

/// Shots counted at most in one frame (a weapon given in the same frame as a long burst).
const MAX_SHOTS: i32 = 16;
/// How far aim() looks for what the view meets.
const AIM_REACH: f32 = 20000.0;

/// Shots between two readings of a byte counter.
fn shots(before: i32, now: i32) -> i32 {
    (now - before).rem_euclid(256)
}

impl Vm {
    /// A function call of the game (Vm::call) while an `on("damage")` handler exists: a
    /// TakeDamage(Damage, InstigatedBy, HitLocation, Momentum, DamageType) is noted.
    pub(crate) fn mod_note_call(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) {
        if !self.functions[f].name.eq_ignore_ascii_case("TakeDamage") {
            return;
        }
        let by = args.get(1).and_then(|a| a.as_ref()).and_then(Value::as_object).filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()));
        let damage_type = match args.get(4).and_then(|a| a.as_ref()) {
            Some(Value::Class(Some(c))) => Some(self.class_path(*c)),
            _ => None,
        };
        let weapon = by.filter(|&p| self.is_a(p, "Pawn")).and_then(|p| self.prop(p, "Weapon").as_object()).and_then(|w| self.objects.get(w).and_then(|o| o.as_ref()).map(|o| self.class_path(o.class)));
        self.mods.script_events.hits.insert(this, Hit { by, damage_type, weapon });
    }

    /// The shots `pawn` fired since `seen` last looked, and the class of its weapon. A weapon
    /// newly in its hands is only looked at (its counter is not the shots of this frame).
    pub(crate) fn mod_fire_shots(&self, pawn: ObjId, seen: &mut FireSeen) -> (i32, Option<String>) {
        let w = self.prop(pawn, "Weapon").as_object().filter(|&w| self.objects.get(w).is_some_and(|o| o.is_some()));
        let (flash, alt) = match w {
            Some(w) => (self.prop(w, "FlashCount").as_int(), self.prop(w, "AltFlashCount").as_int()),
            None => (0, 0),
        };
        let before = seen.insert(pawn, (w, flash, alt));
        let n = match before {
            Some((bw, bf, ba)) if bw == w && w.is_some() => (shots(bf, flash) + shots(ba, alt)).min(MAX_SHOTS),
            _ => 0,
        };
        let class = w.and_then(|w| self.objects[w].as_ref().map(|o| self.class_path(o.class)));
        (n, class)
    }

    /// The characters an `on("fire", who)` handler looks at, sorted: the players (no who), every
    /// character ("all"), or the actors who names.
    pub(crate) fn mod_fire_candidates(&self, who: Option<&Who>) -> Vec<ObjId> {
        let mut out = match who {
            None => self.mod_script_players(),
            Some(Who::Name(n)) if n.eq_ignore_ascii_case("all") => self.actors.iter().copied().filter(|&a| self.is_a(a, "Pawn") && self.mod_alive(a)).collect(),
            Some(Who::Name(n)) => self.mod_script_resolve_all(n),
            Some(w @ Who::Actor(_)) => self.mod_script_resolve(w).into_iter().collect(),
        };
        out.sort();
        out.dedup();
        out
    }

    /// Collect explicit preload requests and dependencies of mod declarations and script literals
    /// for the current map, in deterministic order.
    pub fn mod_precache_roots(&self) -> Vec<String> {
        let map = self.mod_map_name();
        let mut out: BTreeSet<String> = self.mods.script_events.preload.clone();
        if let Some(loads) = self.mods.loads.as_ref() {
            for l in loads.iter().filter(|l| l.map.as_deref().is_none_or(|m| openxiii_mods::maps::matches(m, &map))) {
                out.extend(l.preload.iter().cloned());
            }
        }
        self.add_mod_asset_roots(&mut out);
        out.into_iter().collect()
    }

    /// The weapon an actor's ammunition is: the actor itself (a weapon) or the one in its hands.
    fn mod_ammo_of(&self, a: ObjId) -> Option<ObjId> {
        let w = if self.is_a(a, "Weapon") { Some(a) } else if self.is_a(a, "Pawn") { self.prop(a, "Weapon").as_object() } else { None };
        let w = w.filter(|&w| self.objects.get(w).is_some_and(|o| o.is_some()))?;
        self.prop(w, "AmmoType").as_object().filter(|&x| self.objects.get(x).is_some_and(|o| o.is_some()))
    }

    /// The items `pawn` carries (its Inventory chain), in the chain's order.
    pub(crate) fn mod_inventory_list(&self, pawn: ObjId) -> Vec<ObjId> {
        let mut out = Vec::new();
        let mut item = self.prop(pawn, "Inventory").as_object();
        while let Some(i) = item.filter(|i| out.len() < 256 && self.objects.get(*i).is_some_and(|o| o.is_some())) {
            if out.contains(&i) {
                break;
            }
            out.push(i);
            item = self.prop(i, "Inventory").as_object();
        }
        out
    }

    /// The class a script names for has()/take(): a name of the toml's weapon list or a class.
    fn mod_item_class(&self, item: &str) -> Option<crate::value::ClassId> {
        let name = openxiii_mods::simple::resolve_weapon(item).unwrap_or_else(|_| item.to_string());
        self.class(&name)
    }

    /// `actor:is_a(class)`: a full path ("XIII.Beretta") or a class's own name ("Pawn").
    pub(crate) fn mod_is_a(&self, a: ObjId, class: &str) -> bool {
        let Some(o) = self.objects.get(a).and_then(|o| o.as_ref()) else { return false };
        if class.contains('.') {
            return self.class(class).is_some_and(|c| self.is_child_of(o.class, c));
        }
        self.is_a(a, class)
    }
}

// ---------------------------------------------------------------------------------------------
// The functions.

pub(crate) fn api_is_a(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "is_a")?;
    let class = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("is_a(\"Class\")"))?;
    vm.mod_is_a(a, &class).into_lua_multi(lua)
}

pub(crate) fn api_actor_trigger(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "trigger")?;
    let by = match rest.pop() {
        None | Some(mlua::Value::Nil) => vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()),
        Some(v) => Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime("trigger: by = nobody of that name"))?),
    };
    let Some(f) = vm.find_function(a, "Trigger", false) else { return rt(format!("trigger: {} cannot be triggered", vm.mod_actor_name(a))) };
    let pawn = Value::Object(by);
    {
        vm.call(f, a, vec![Some(pawn.clone()), Some(pawn)]).map_err(|e| mlua::Error::runtime(format!("trigger: {e}")))?;
    }
    vm.mod_net_fx_call(a, "Trigger");
    ().into_lua_multi(lua)
}

/// `actor.ammo`: the ammunition of the weapon in its hands (or of a weapon); nil without one.
pub(crate) fn ammo_get(vm: &mut Vm, lua: &Lua, a: ObjId) -> mlua::Result<mlua::Value> {
    match vm.mod_ammo_of(a) {
        Some(x) => vm.prop(x, "AmmoAmount").as_int().into_lua(lua),
        None => Ok(mlua::Value::Nil),
    }
}

pub(crate) fn ammo_set(vm: &mut Vm, lua: &Lua, a: ObjId, value: mlua::Value) -> mlua::Result<()> {
    let n = i32::from_lua(value, lua).map_err(|_| mlua::Error::runtime("ammo: a whole number"))?;
    let Some(x) = vm.mod_ammo_of(a) else { return rt(format!("ammo: {} has no weapon in its hands", vm.mod_actor_name(a))) };
    {
        vm.set_prop(x, "AmmoAmount", Value::Int(n.max(0)));
    }
    Ok(())
}

pub(crate) fn api_has(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "has")?;
    let item = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("has(weapon or class)"))?;
    let Some(class) = vm.mod_item_class(&item) else { return rt(format!("has: unknown weapon or class {item}")) };
    let found = vm.mod_inventory_list(a).into_iter().any(|i| vm.objects[i].as_ref().is_some_and(|o| vm.is_child_of(o.class, class)));
    (found).into_lua_multi(lua)
}

pub(crate) fn api_take(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "take")?;
    let item = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("take(weapon or class)"))?;
    let Some(class) = vm.mod_item_class(&item) else { return rt(format!("take: unknown weapon or class {item}")) };
    let items: Vec<ObjId> = vm.mod_inventory_list(a).into_iter().filter(|&i| vm.objects[i].as_ref().is_some_and(|o| vm.is_child_of(o.class, class))).collect();
    for &i in &items {
        let _ = vm.call_named(a, "DeleteInventory", vec![Value::Object(Some(i))]);
        if vm.objects.get(i).is_some_and(|o| o.is_some()) && !vm.native_prop(i, Field::DeleteMe).as_bool() {
            let _ = vm.destroy_actor(i);
        }
    }
    if !items.is_empty() {
        vm.mod_report(format!("{}: take {item} from {} ({})", vm.mod_map_name(), vm.mod_actor_name(a), caller(lua)));
    }
    items.len().into_lua_multi(lua)
}

/// `actor:aim()`: {from =, direction =, hit =, actor =}: the point the actor looks from (its
/// eyes), the direction it looks in (its controller's view: where a player aims), and the first
/// point the view meets (nil when nothing is within reach) with the actor there (nil for the level).
pub(crate) fn api_aim(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, _) = this(lua, args, "aim")?;
    if !vm.mod_alive(a) {
        return rt("aim: this actor is not alive");
    }
    let [x, y, z] = vm.native_value(a, Field::Location).as_vector();
    let eye = if vm.is_a(a, "Pawn") { vm.call_named(a, "EyePosition", vec![]).map(|v| v.as_vector()).unwrap_or([0.0; 3]) } else { [0.0; 3] };
    let from = [x + eye[0], y + eye[1], z + eye[2]];
    let view = vm.native_prop(a, Field::Controller).as_object().filter(|&c| vm.objects.get(c).is_some_and(|o| o.is_some())).unwrap_or(a);
    let [pitch, yaw, _] = vm.native_value(view, Field::Rotation).as_rotator();
    let d = crate::mod_script_world::rotator_direction(pitch, yaw);
    let to = [from[0] + d[0] * AIM_REACH, from[1] + d[1] * AIM_REACH, from[2] + d[2] * AIM_REACH];
    let list = |p: [f32; 3]| lua.create_sequence_from(p.iter().map(|v| *v as f64));
    let hit = crate::mod_script_world::api_trace(vm, lua, (list(from)?, list(to)?, Actor(a)).into_lua_multi(lua)?)?;
    let t = lua.create_table()?;
    t.raw_set("from", list(from)?)?;
    t.raw_set("direction", list(d)?)?;
    if let Some(mlua::Value::Table(h)) = hit.into_iter().next() {
        t.raw_set("hit", h.raw_get::<mlua::Value>("position")?)?;
        t.raw_set("actor", h.raw_get::<mlua::Value>("actor")?)?;
    }
    t.into_lua_multi(lua)
}

/// `actor:hurt(amount[, by[, damage_type[, at[, push]]]])`: the game's own TakeDamage, as a
/// bullet's (the game decides whether it can be hurt, and how much: difficulty, armour...).
/// Returns the health it really lost (characters; 0 for anything else).
pub(crate) fn api_hurt(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "hurt")?;
    let mut next = || if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) };
    let amount = i32::from_lua(next(), lua).map_err(|_| mlua::Error::runtime("hurt(amount[, by[, damage_type[, at[, push]]]]): a whole number first"))?;
    let by = next();
    let kind = match next() {
        mlua::Value::Nil => mlua::Value::String(lua.create_string("XIII.DTGunned")?),
        v => v,
    };
    let list = |p: [f32; 3]| lua.create_sequence_from(p.iter().map(|v| *v as f64)).map(mlua::Value::Table);
    let at = match next() {
        mlua::Value::Nil => list(vm.native_value(a, Field::Location).as_vector())?,
        v => v,
    };
    let push = match next() {
        mlua::Value::Nil => list([0.0; 3])?,
        v => v,
    };
    let pawn = vm.is_a(a, "Pawn");
    let before = if pawn { vm.prop(a, "Health").as_int() } else { 0 };
    let call = (Actor(a), "TakeDamage", amount.max(0), by, at, push, kind).into_lua_multi(lua)?;
    {
        crate::mod_script_game::api_call(vm, lua, call)?;
    }
    let after = if pawn && vm.objects.get(a).is_some_and(|o| o.is_some()) { vm.prop(a, "Health").as_int().max(0) } else { before.max(0) };
    (before.max(0) - after).max(0).into_lua_multi(lua)
}

/// `actor:can_see(other)`: the character's own sight of `other` (its controller's
/// LineOfSightTo: walls hide him). False for an actor without a controller.
pub(crate) fn api_can_see(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "can_see")?;
    let other = rest.into_iter().next().ok_or_else(|| mlua::Error::runtime("can_see(actor)"))?;
    let controller = if vm.prop_slot(a, "Controller").is_ok() { vm.prop(a, "Controller").as_object() } else { None };
    let Some(c) = controller.filter(|&c| vm.objects.get(c).is_some_and(|o| o.is_some())) else {
        return false.into_lua_multi(lua);
    };
    let r = crate::mod_script_game::api_call(vm, lua, (Actor(c), "LineOfSightTo", other).into_lua_multi(lua)?)?;
    matches!(r.into_iter().next(), Some(mlua::Value::Boolean(true))).into_lua_multi(lua)
}

/// `game()`: {difficulty =, language =, mode =}.
pub(crate) fn api_game(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    let t = lua.create_table()?;
    let game = vm.level.and_then(|l| vm.prop(l, "Game").as_object());
    let difficulty = game.filter(|&g| vm.prop_slot(g, "Difficulty").is_ok()).map(|g| vm.prop(g, "Difficulty").as_int());
    t.raw_set("difficulty", difficulty)?;
    t.raw_set("language", vm.text_language().to_string())?;
    let local = vm.mod_script_player_controllers().into_iter().filter(|&pc| vm.local_player_controller(pc).is_some()).count();
    let mode = if vm.ipdrv.net_driver.is_some() {
        "network"
    } else if local > 1 {
        "split"
    } else {
        "solo"
    };
    t.raw_set("mode", mode)?;
    t.into_lua_multi(lua)
}

pub(crate) fn api_preload(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let name = String::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("preload(\"Package.Name\")"))?;
    if !vm.mods.script_events.preload_open {
        return rt("preload: only in the script's main code or in its on(\"start\") handler");
    }
    if !openxiii_mods::content::valid_object_name(name.trim()) {
        return rt(format!("preload: {name:?} is not an object's name (Package.Name)"));
    }
    vm.mods.script_events.preload.insert(name.trim().to_string());
    true.into_lua_multi(lua)
}

/// A sound played by sound(): its id for stop_sound.
pub(crate) fn note_sound(vm: &mut Vm, source: ObjId, name: &str) -> u64 {
    vm.mods.script_events.next_sound += 1;
    let id = vm.mods.script_events.next_sound;
    vm.mods.script_events.sounds.insert(id, (source, name.to_string()));
    id
}

pub(crate) fn api_stop_sound(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let id = u64::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("stop_sound(id): the number sound() gave"))?;
    let removed = vm.mods.script_events.sounds.remove(&id);
    let Some((source, name)) = removed else { return false.into_lua_multi(lua) };
    if vm.objects.get(source).is_some_and(|o| o.is_some()) {
        let mut a = [Value::Asset(crate::mod_sounds::object_path(&name).to_string())];
        let _ = crate::natives::call(vm, "StopSound", source, &mut a, &[true]);
    }
    vm.mod_script_to_guests(STOP_SOUND_KIND, 0, &name);
    true.into_lua_multi(lua)
}

pub(crate) fn api_music_stop(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    let Some(p) = vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()).or(vm.level) else { return rt("music_stop: no level") };
    let _ = crate::natives::call(vm, "StopMusic", p, &mut [], &[]);
    vm.mod_script_to_guests(STOP_MUSIC_KIND, 0, "");
    vm.mod_script_log(format!("{}: music_stop ({})", vm.mod_map_name(), caller(lua)));
    ().into_lua_multi(lua)
}

impl Vm {
    /// A network guest: the host's stop_sound or music_stop, here.
    pub(crate) fn mod_stop_from_host(&mut self, pc: ObjId, kind: &str, text: &str) -> Result<(), String> {
        let pawn = self.prop(pc, "Pawn").as_object().or_else(|| self.prop(pc, "ViewTarget").as_object()).ok_or("no player")?;
        if kind == STOP_MUSIC_KIND {
            crate::natives::call(self, "StopMusic", pawn, &mut [], &[]).map_err(|e| e.to_string())?;
        } else {
            let mut a = [Value::Asset(crate::mod_sounds::object_path(text).to_string())];
            crate::natives::call(self, "StopSound", pawn, &mut a, &[true]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_counter_wraps() {
        assert_eq!(shots(3, 5), 2);
        assert_eq!(shots(255, 1), 2);
        assert_eq!(shots(7, 7), 0);
        // Negative control: a plain difference would be wrong across the wrap.
        assert_ne!(1 - 255, shots(255, 1));
    }
}
