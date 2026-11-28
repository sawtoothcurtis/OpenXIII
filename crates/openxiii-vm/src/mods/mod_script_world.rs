//! Mods build only (feature `mods`): time(), trace() and effect{}.
//!
//! - time: the level's clock (`LevelInfo.TimeSeconds`), the one `wait` and `every` count with:
//!   what a script times with it does not depend on the frame rate;
//! - trace: the game's own line check (`Actor.Trace`, actors included), as weapons and the AI
//!   use it to see what a line meets;
//! - effect: an actor of any class (an emitter, a projector, a light...) put exactly at a point
//!   and turned as asked, with no search for the ground, optionally carried by another actor
//!   (`SetBase`) and taken out after a time (`LifeSpan`). The engine moves, draws and removes it
//!   as it does the game's own effects.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use mlua::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Lua, MultiValue, Table};

use crate::mod_script::{caller, read_position, Actor, Who};
use crate::value::{ObjId, Value};
use crate::Vm;

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

fn vector(p: [f32; 3]) -> Value {
    Value::vector(p[0], p[1], p[2])
}

/// Game angle units of a direction: (pitch, yaw), as `rotator(vector)` gives them.
pub(crate) fn direction_rotator(d: [f32; 3]) -> (i32, i32) {
    let to_units = |radians: f32| (radians * 32768.0 / std::f32::consts::PI).round() as i32;
    let yaw = to_units(d[1].datan2(d[0])).rem_euclid(65536);
    let pitch = to_units(d[2].datan2((d[0] * d[0] + d[1] * d[1]).sqrt())).rem_euclid(65536);
    (pitch, yaw)
}

/// The unit direction of game angle units (pitch, yaw), as `vector(rotator)` gives it.
pub(crate) fn rotator_direction(pitch: i32, yaw: i32) -> [f32; 3] {
    let to_radians = |units: i32| units as f32 * std::f32::consts::PI / 32768.0;
    let (p, y) = (to_radians(pitch), to_radians(yaw));
    [p.dcos() * y.dcos(), p.dcos() * y.dsin(), p.dsin()]
}

/// `time()`: seconds of game time since the level started.
pub(crate) fn api_time(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    (vm.mod_level_time() as f64).into_lua_multi(lua)
}

/// `trace(from, to[, by])`: what the line from `from` to `to` meets first, the level or an actor
/// (a character, a door, a decoration), as {position =, normal =, actor =, material =}, or nil when
/// nothing is in the way (material: the name of the surface's texture or shader, when it has one). The line starts from actor `by` (the player by default), which it never meets.
pub(crate) fn api_trace(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (from, to, by) =
        <(Table, Table, Option<mlua::Value>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("trace(from {x, y, z}, to {x, y, z}[, by])"))?;
    let (from, to) = (read_position(&from)?, read_position(&to)?);
    let source = match by {
        None | Some(mlua::Value::Nil) => vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()).or(vm.level),
        Some(v) => Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime("trace: by = nobody of that name"))?),
    };
    let Some(source) = source else { return rt("trace: no level") };
    let Some(f) = vm.find_function(source, "Trace", false) else { return rt("trace: the game has no Trace here") };
    // The Material out parameter asked for: the surface's material (texture or shader) by name.
    let args = vec![
        Some(vector([0.0; 3])),
        Some(vector([0.0; 3])),
        Some(vector(to)),
        Some(vector(from)),
        Some(Value::Bool(true)),
        Some(vector([0.0; 3])),
        Some(Value::Object(None)),
    ];
    let (hit, outs) = vm.call(f, source, args).map_err(|e| mlua::Error::runtime(format!("trace: {e}")))?;
    let Some(actor) = hit.as_object() else { return mlua::Value::Nil.into_lua_multi(lua) };
    let (Some(Some(at)), Some(Some(normal))) = (outs.first(), outs.get(1)) else { return mlua::Value::Nil.into_lua_multi(lua) };
    let t = lua.create_table()?;
    let list = |p: [f32; 3]| lua.create_sequence_from(p.iter().map(|x| *x as f64));
    t.raw_set("position", list(at.as_vector())?)?;
    t.raw_set("normal", list(normal.as_vector())?)?;
    match outs.get(6) {
        Some(Some(Value::Asset(m))) if !m.is_empty() => t.raw_set("material", m.as_str())?,
        Some(Some(Value::Object(Some(o)))) => {
            if let Some(name) = vm.objects.get(*o).and_then(|x| x.as_ref()).map(|_| vm.path_name(*o)) {
                t.raw_set("material", name)?;
            }
        }
        _ => {}
    }
    // The level itself is not an actor of the scripts: nil, the surface is the world's.
    if Some(actor) != vm.level && vm.is_a(actor, "Actor") && !vm.is_a(actor, "LevelInfo") {
        t.raw_set("actor", Actor(actor))?;
    }
    t.into_lua_multi(lua)
}

/// `effect{class =, position =, rotation = or facing =, on =, seconds =}`.
pub(crate) fn api_effect(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    const KEYS: &str = "class, position, rotation, facing, on, seconds";
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime(format!("effect{{{KEYS}}}")))?;
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        let ok = matches!(&k, mlua::Value::String(s) if matches!(s.to_str().map(|s| s.to_string()).as_deref(), Ok("class" | "position" | "rotation" | "facing" | "on" | "seconds")));
        if !ok {
            return rt(format!("effect: unknown setting {k:?} ({KEYS})"));
        }
    }
    let class_name: String = t.get::<Option<String>>("class")?.ok_or_else(|| mlua::Error::runtime("effect: give class ="))?;
    let Some(class) = vm.class(&class_name) else { return rt(format!("effect: unknown class {class_name}")) };
    vm.create_cdo(class);
    if !vm.is_a(vm.classes[class].cdo, "Actor") {
        return rt(format!("effect: {class_name} is not a class of actors"));
    }
    let on = match t.get::<mlua::Value>("on")? {
        mlua::Value::Nil => None,
        v => Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime("effect: on = nobody of that name"))?),
    };
    let position = match t.get::<Option<Table>>("position")? {
        Some(p) => read_position(&p)?,
        None => match on {
            Some(a) => vm.prop(a, "Location").as_vector(),
            None => return rt("effect: give position = {x, y, z} (or on = an actor)"),
        },
    };
    let degrees = |x: f32| ((x * 65536.0 / 360.0).round() as i32).rem_euclid(65536);
    let rotation = match (t.get::<Option<Table>>("rotation")?, t.get::<Option<Table>>("facing")?) {
        (Some(_), Some(_)) => return rt("effect: give rotation = or facing =, not both"),
        (Some(r), None) => {
            let get = |k: &str, i: usize| -> mlua::Result<f32> { Ok(r.raw_get::<Option<f32>>(k)?.or(r.raw_get::<Option<f32>>(i)?).unwrap_or(0.0)) };
            Value::rotator(degrees(get("pitch", 1)?), degrees(get("yaw", 2)?), degrees(get("roll", 3)?))
        }
        (None, Some(d)) => {
            let (pitch, yaw) = direction_rotator(read_position(&d)?);
            Value::rotator(pitch, yaw, 0)
        }
        (None, None) => Value::rotator(0, 0, 0),
    };
    let seconds: Option<f32> = t.get("seconds")?;
    if seconds.is_some_and(|s| !s.is_finite() || s <= 0.0) {
        return rt("effect: seconds = a number above 0");
    }
    let label = caller(lua);
    let a = vm
        .spawn_actor(class, Value::Object(None), None, vector(position), rotation, Value::Object(None), true)
        .map_err(|e| mlua::Error::runtime(format!("effect {class_name}: {e}")))?;
    let Some(a) = a else {
        vm.mod_report(format!("warning: {}: effect {class_name}: the game could not place it ({label})", vm.mod_map_name()));
        return mlua::Value::Nil.into_lua_multi(lua);
    };
    if let Some(base) = on {
        let mut args = [Value::Object(Some(base)), vector([0.0; 3])];
        crate::natives::call(vm, "SetBase", a, &mut args, &[true, false]).map_err(|e| mlua::Error::runtime(format!("effect {class_name}: on: {e}")))?;
    }
    if let Some(s) = seconds {
        vm.set_prop(a, "LifeSpan", Value::Float(s));
    }
    // A network host's guests see it too (mod_net_fx.rs).
    vm.mod_net_fx_spawned(a);
    if seconds.is_some() {
        vm.mod_net_fx_set(a, "LifeSpan");
    }
    Actor(a).into_lua(lua)?.into_lua_multi(lua)
}

/// The object a property path leads to, and the last property's name: "Emitters[0].Disabled"
/// is property "Disabled" of the object in `Emitters[0]` (a sub-object of the actor).
pub(crate) fn property_path(vm: &Vm, a: ObjId, key: &str) -> mlua::Result<(ObjId, String)> {
    let mut parts: Vec<&str> = key.split('.').map(str::trim).collect();
    let last = parts.pop().unwrap_or_default().to_string();
    let mut at = a;
    let mut walked = String::new();
    for part in parts {
        let (name, index) = match part.split_once('[') {
            Some((n, rest)) => (n.trim(), rest.strip_suffix(']').and_then(|i| i.trim().parse::<usize>().ok())),
            None => (part, None),
        };
        let Some(class) = vm.objects.get(at).and_then(|o| o.as_ref()).map(|o| o.class) else { return rt(format!("{key}: this actor is not in the game any more")) };
        let Some(&slot) = vm.classes[class].prop_index.get(&name.to_lowercase()) else { return rt(format!("{key}: {} has no property {name}", vm.class_path(class))) };
        let v = vm.objects[at].as_ref().map(|o| o.props[slot].clone()).unwrap_or(Value::Void);
        let v = match (index, v) {
            (Some(i), Value::Array(items)) => items.get(i).cloned().ok_or_else(|| mlua::Error::runtime(format!("{key}: {name}[{i}] is out of the array")))?,
            (Some(_), _) => return rt(format!("{key}: {name} is not an array")),
            (None, v) => v,
        };
        walked.push_str(part);
        match v {
            Value::Object(Some(o)) if vm.objects.get(o).is_some_and(|x| x.is_some()) => at = o,
            _ => return rt(format!("{key}: {walked} holds no object")),
        }
        walked.push('.');
    }
    Ok((at, last))
}

/// `heat{at =, radius =, seconds =, strength =, name =}`.
pub(crate) fn api_heat(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    const KEYS: &str = "at, radius, seconds, strength, name";
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime(format!("heat{{{KEYS}}}")))?;
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        let ok = matches!(&k, mlua::Value::String(s) if matches!(s.to_str().map(|s| s.to_string()).as_deref(), Ok("at" | "radius" | "seconds" | "strength" | "name")));
        if !ok {
            return rt(format!("heat: unknown setting {k:?} ({KEYS})"));
        }
    }
    let (at, on) = match t.get::<mlua::Value>("at")? {
        mlua::Value::Table(p) => (read_position(&p)?, None),
        mlua::Value::Nil => return rt("heat: give at = a position or an actor"),
        v => (
            [0.0; 3],
            Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime("heat: at = nobody of that name"))?),
        ),
    };
    let radius: f32 = t.get::<Option<f32>>("radius")?.unwrap_or(100.0);
    let seconds: f32 = t.get::<Option<f32>>("seconds")?.unwrap_or(1.0);
    let strength: f32 = t.get::<Option<f32>>("strength")?.unwrap_or(1.0);
    if !(radius.is_finite() && radius > 0.0 && seconds.is_finite() && seconds > 0.0 && strength.is_finite()) {
        return rt("heat: radius and seconds above 0");
    }
    let name: String = t.get::<Option<String>>("name")?.unwrap_or_default();
    let until = vm.mod_level_time() + seconds;
    let zone = crate::mod_heat::HeatZone { name, at, on, radius, strength, until };
    vm.mod_net_heat(&zone, seconds);
    vm.mod_heat_add(zone);
    ().into_lua_multi(lua)
}

/// `actor:retexture(from, to)`.
pub(crate) fn api_retexture(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.len() != 3 {
        return rt("retexture: write actor:retexture(from, to)");
    }
    let a = Actor::from_lua(args.remove(0), lua).map_err(|_| mlua::Error::runtime("retexture: write actor:retexture(from, to), with a colon"))?.0;
    let from = String::from_lua(args.remove(0), lua)?;
    let to = String::from_lua(args.remove(0), lua)?;
    if from.is_empty() {
        return rt("retexture: give the texture to replace (from)");
    }
    let name = vm.mod_actor_name(a);
    vm.mods.surfaces.set(&name, &from, &to);
    vm.mod_net_level_step(&["rt", &name, &from, &to]);
    ().into_lua_multi(lua)
}

/// `actor:pin_to(other[, {forward, right, up}])`: the actor (an effect) is kept every frame at
/// that point of `other`'s own frame; `actor:pin_to(nil)` lets it go. A gun held in a hand is
/// taken as its character is posed now; in a network game each machine places it on `other` as
/// it draws it (mod_net_fx.rs).
pub(crate) fn api_pin_to(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.is_empty() || args.len() > 3 {
        return rt("pin_to: write actor:pin_to(other[, {forward, right, up}])");
    }
    let a = Actor::from_lua(args.remove(0), lua).map_err(|_| mlua::Error::runtime("pin_to: write actor:pin_to(other), with a colon"))?.0;
    if vm.is_a(a, "Pawn") {
        return rt("pin_to: a character cannot be pinned (it walks by itself)");
    }
    let to = match args.first().cloned().unwrap_or(mlua::Value::Nil) {
        mlua::Value::Nil => None,
        v => Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime("pin_to: nobody of that name"))?),
    };
    if to == Some(a) {
        return rt("pin_to: an actor cannot be pinned to itself");
    }
    let at = match args.get(1) {
        Some(mlua::Value::Table(t)) => read_position(t)?,
        Some(mlua::Value::Nil) | None => [0.0; 3],
        Some(_) => return rt("pin_to: the point is {forward, right, up}"),
    };
    vm.mod_pin(a, to.map(|t| (t, at)));
    ().into_lua_multi(lua)
}

/// `actors_near(position or actor, radius[, class])`: the actors of the level within radius of a
/// point (the scenery's static meshes, its movers and decorations, the characters alive or dead;
/// players and actors being destroyed left out), of that class if given, nearest first.
pub(crate) fn api_actors_near(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.len() < 2 || args.len() > 3 {
        return rt("actors_near(position or actor, radius[, class])");
    }
    let class = if args.len() == 3 { Some(String::from_lua(args.pop().unwrap_or(mlua::Value::Nil), lua)?) } else { None };
    let radius = f32::from_lua(args.pop().unwrap_or(mlua::Value::Nil), lua)?;
    let at = match args.remove(0) {
        mlua::Value::Table(t) => read_position(&t)?,
        v => {
            let a = Actor::from_lua(v, lua).map_err(|_| mlua::Error::runtime("actors_near: give a position {x, y, z} or an actor"))?.0;
            vm.native_value(a, crate::NativeField::Location).as_vector()
        }
    };
    let players = vm.mod_script_players();
    let mut found: Vec<(f32, ObjId)> = vm
        .actors
        .iter()
        .copied()
        .filter(|&a| !players.contains(&a) && !vm.prop(a, "bDeleteMe").as_bool())
        .filter(|&a| class.as_deref().is_none_or(|c| vm.is_a(a, c)))
        .map(|a| {
            let [x, y, z] = vm.native_value(a, crate::NativeField::Location).as_vector();
            (((x - at[0]).powi(2) + (y - at[1]).powi(2) + (z - at[2]).powi(2)).sqrt(), a)
        })
        .filter(|(d, _)| *d <= radius)
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let t = lua.create_table()?;
    for (_, a) in found {
        t.raw_push(Actor(a))?;
    }
    t.into_lua_multi(lua)
}

/// `char_area(position, radius[, amount])`: the level's own walls, floors and ceilings within
/// radius are drawn charred (amount from 0 to 1, 1 by default); it stays.
pub(crate) fn api_char_area(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.len() < 2 || args.len() > 3 {
        return rt("char_area(position, radius[, amount])");
    }
    let amount = if args.len() == 3 { f32::from_lua(args.pop().unwrap_or(mlua::Value::Nil), lua)? } else { 1.0 };
    let radius = f32::from_lua(args.pop().unwrap_or(mlua::Value::Nil), lua)?;
    let at = match args.remove(0) {
        mlua::Value::Table(t) => read_position(&t)?,
        _ => return rt("char_area: give a position {x, y, z}"),
    };
    if !(radius.is_finite() && radius > 0.0 && amount.is_finite()) {
        return rt("char_area: a radius above 0 and an amount from 0 to 1");
    }
    vm.mods.surfaces.char_area(at, radius, amount.clamp(0.0, 1.0));
    // A network host sends the areas as they are (mod_net_fx.rs), not each call.
    ().into_lua_multi(lua)
}
