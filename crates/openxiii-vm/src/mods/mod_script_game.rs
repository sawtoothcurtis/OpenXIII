//! Mods build only (feature `mods`): the scripts' orders to characters and to the game
//! (mod_script.rs holds the API table that names these functions).
//!
//! Everything goes through the game's own functions, run by the VM as the game's scripts run
//! them:
//! - story characters (their controller is XIDCine's CineController2, the class of the game's
//!   `XIDCine.Cine2` characters) take the commands the level designers wrote for them, through
//!   `CineController2.Interpret` ("MovSeq W <actor>", "LookAt <actor>", "Anim Once <anim>",
//!   "StopSeq"): a position is given to them as a hidden marker actor placed there;
//! - soldiers (XIDPawn's IAController) follow their own AI: they can be given an enemy
//!   (`SetEnemy`, then `EnemyAcquired`, as the game's scripts do), but no route;
//! - the game: goals shown by the HUD's XIIIGoalMessage, `TriggerEvent`, `PlaySound` /
//!   `PlayMusic`, and the end of a mission through `GameInfo.EndGame` ("GoalComplete" travels to
//!   `MapInfo.NextMapLevelWithUnr`, "GoalIncomplete:<text>" is the game over of a failed goal).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use mlua::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Lua, MultiValue, Table};
use openxiii_mods::behaviour::Team;
use openxiii_mods::ModValue;

use crate::mod_script::{caller, read_position, Actor, Who};
use crate::types::Type;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Controller function of the story characters' commands (CineController2).
const CINE_COMMAND: &str = "Interpret";
/// Class of the hidden marker actors (positions given to story characters, sounds at a point).
const MARKER_CLASS: &str = "Engine.LineOfSightTrigger";
/// Damage type of `kill` (a gun shot: the usual death of a character).
const KILL_DAMAGE: &str = "XIII.DTGunned";
/// A go_to is over when the character is this near its target.
const ARRIVED: f32 = 120.0;
/// Doors within this distance of a stuck character are pushed.
const DOOR_REACH: f32 = 300.0;
/// A follower that got no closer for this long is given its way again.
const STUCK_SECONDS: f32 = 3.0;
/// A follower walking is given a new way (towards where its leader is now) this often.
const REROUTE_SECONDS: f32 = 3.0;
/// Seconds between two moves of a character that follows another.
pub(crate) const FOLLOW_EVERY: f32 = 0.5;
/// Seconds an `attack` order is given again until the soldier's AI holds it.
const ATTACK_SECONDS: f32 = 10.0;
/// A follower does not move closer than this.
const FOLLOW_DISTANCE: f32 = 200.0;
/// Text of a ClientMessage that carries a goal, a music or a sound of the host's scripts to a
/// player on another computer: "OXIIIMod.Script.<kind>.<number>|<text>" (as the co-op messages
/// and the skins messages travel: the game's own reliable ClientMessage).
pub const GUEST_MESSAGE: &str = "OXIIIMod.Script.";

/// "OXIIIMod.Script.Goal.1|Hold the beach".
pub fn guest_line(kind: &str, number: i32, text: &str) -> String {
    format!("{GUEST_MESSAGE}{kind}.{number}|{text}")
}

/// (kind, number, text) of a guest line.
pub fn parse_guest_line(line: &str) -> Option<(&str, i32, &str)> {
    let (head, text) = line.strip_prefix(GUEST_MESSAGE)?.split_once('|')?;
    let (kind, number) = head.rsplit_once('.')?;
    Some((kind, number.trim().parse().ok()?, text))
}

fn err<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

/// A target: an actor, or a position on the ground.
enum Target {
    Actor(ObjId),
    At([f32; 3]),
}

fn read_target(vm: &Vm, lua: &Lua, v: mlua::Value, what: &str) -> mlua::Result<Target> {
    match v {
        mlua::Value::Table(t) => Ok(Target::At(read_position(&t)?)),
        mlua::Value::Nil => err(format!("{what}: give an actor or a position {{x, y, z}}")),
        other => match vm.mod_script_resolve(&Who::from_lua(other, lua)?) {
            Some(a) => Ok(Target::Actor(a)),
            None => err(format!("{what}: nobody of that name")),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// Values between Lua and the game.

/// A game value as Lua sees it.
pub(crate) fn value_to_lua(vm: &mut Vm, lua: &Lua, v: &Value) -> mlua::Result<mlua::Value> {
    Ok(match v {
        Value::Void | Value::Object(None) | Value::Class(None) => mlua::Value::Nil,
        Value::Byte(b) => mlua::Value::Integer(*b as i64),
        Value::Int(i) => mlua::Value::Integer(*i as i64),
        Value::Bool(b) => mlua::Value::Boolean(*b),
        Value::Float(f) => mlua::Value::Number(*f as f64),
        Value::Name(n) => n.to_string().into_lua(lua)?,
        Value::Str(s) | Value::Asset(s) => s.clone().into_lua(lua)?,
        Value::Object(Some(o)) => {
            if vm.objects.get(*o).is_some_and(|x| x.is_some()) && vm.is_a(*o, "Actor") {
                mlua::Value::UserData(lua.create_userdata(Actor(*o))?)
            } else {
                vm.objects.get(*o).and_then(|x| x.as_ref()).map(|x| x.name.clone()).into_lua(lua)?
            }
        }
        Value::Class(Some(c)) => vm.class_path(*c).into_lua(lua)?,
        Value::Vector(p) => mlua::Value::Table(lua.create_sequence_from(p.iter().map(|x| *x as f64))?),
        Value::Rotator(r) => {
            let t = lua.create_table()?;
            for (k, x) in ["pitch", "yaw", "roll"].iter().zip(r) {
                t.raw_set(*k, *x)?;
            }
            mlua::Value::Table(t)
        }
        Value::Struct(name, fields) => {
            let names: Vec<(String, usize)> = vm.struct_def(name.as_str()).map(|d| d.index.iter().map(|(k, i)| (k.clone(), *i)).collect()).unwrap_or_default();
            let fields = fields.clone().into_vec();
            let t = lua.create_table()?;
            for (k, i) in names {
                if let Some(f) = fields.get(i) {
                    t.raw_set(k, value_to_lua(vm, lua, f)?)?;
                }
            }
            mlua::Value::Table(t)
        }
        Value::Array(items) => {
            let t = lua.create_table()?;
            for x in items {
                t.raw_push(value_to_lua(vm, lua, x)?)?;
            }
            mlua::Value::Table(t)
        }
        Value::Delegate(_, f) => f.to_string().into_lua(lua)?,
    })
}

/// A Lua value as a mod value (the values of the toml files), for the game's own checks.
fn lua_to_mod_value(v: &mlua::Value, what: &str) -> mlua::Result<ModValue> {
    Ok(match v {
        mlua::Value::Integer(i) => ModValue::Int(*i),
        mlua::Value::Number(n) => ModValue::Float(*n),
        mlua::Value::Boolean(b) => ModValue::Bool(*b),
        mlua::Value::String(s) => ModValue::Str(s.to_str()?.to_string()),
        mlua::Value::Table(t) => {
            let n = t.raw_len();
            if n > 0 {
                let mut out = Vec::new();
                for i in 1..=n {
                    out.push(lua_to_mod_value(&t.raw_get::<mlua::Value>(i)?, what)?);
                }
                ModValue::Array(out)
            } else {
                let mut out = Vec::new();
                for pair in t.clone().pairs::<String, mlua::Value>() {
                    let (k, x) = pair?;
                    out.push((k, lua_to_mod_value(&x, what)?));
                }
                ModValue::Table(out)
            }
        }
        other => return err(format!("{what}: a {} cannot be written in a property (number, string, true/false or a table)", other.type_name())),
    })
}

/// A Lua value as a toml value (the `change` table read by the toml reader).
fn lua_to_toml(v: &mlua::Value, key: &str) -> mlua::Result<openxiii_mods::toml::Value> {
    use openxiii_mods::toml::Value as T;
    Ok(match v {
        mlua::Value::Integer(i) => T::Integer(*i),
        mlua::Value::Number(n) => T::Float(*n),
        mlua::Value::Boolean(b) => T::Boolean(*b),
        mlua::Value::String(s) => T::String(s.to_str()?.to_string()),
        mlua::Value::Table(t) => {
            let n = t.raw_len();
            if n > 0 {
                let mut out = Vec::new();
                for i in 1..=n {
                    out.push(lua_to_toml(&t.raw_get::<mlua::Value>(i)?, key)?);
                }
                T::Array(out)
            } else {
                let mut out = openxiii_mods::toml::Table::new();
                for pair in t.clone().pairs::<String, mlua::Value>() {
                    let (k, x) = pair?;
                    let x = lua_to_toml(&x, &k)?;
                    out.insert(k, x);
                }
                T::Table(out)
            }
        }
        other => return err(format!("change: {key} = a {} (write a number, a string, true/false or a table)", other.type_name())),
    })
}

/// "Name[2]" → ("Name", Some(2)).
fn property_key(key: &str) -> (String, Option<usize>) {
    match key.split_once('[') {
        Some((p, rest)) => match rest.strip_suffix(']').and_then(|i| i.trim().parse().ok()) {
            Some(i) => (p.trim().to_string(), Some(i)),
            None => (key.to_string(), None),
        },
        None => (key.to_string(), None),
    }
}

/// The type of property `name` of class `c`, as written in an error.
fn type_text(t: &Type) -> String {
    match t {
        Type::Byte => "byte (a whole number 0-255 or an enum value's name)".into(),
        Type::Int => "int (a whole number)".into(),
        Type::Bool => "bool (true or false)".into(),
        Type::Float => "float (a number)".into(),
        Type::Name => "name (a string)".into(),
        Type::Str => "string".into(),
        Type::Object(c) => format!("object of class {c} (an actor, or a name in quotes)"),
        Type::Class(c) => format!("class {c} (a class name in quotes)"),
        Type::Struct(s) => format!("struct {s} (a table)"),
        Type::Array(inner) => format!("array of {}", type_text(inner)),
        Type::Delegate => "delegate (cannot be written)".into(),
        Type::Unknown => "unknown".into(),
    }
}

/// A Lua value as a parameter of type `ty` of a game function.
fn lua_to_param(vm: &Vm, lua: &Lua, v: mlua::Value, ty: &Type, what: &str) -> mlua::Result<Value> {
    let bad = |v: &mlua::Value| err(format!("{what}: a {} was given, a {} is expected", v.type_name(), type_text(ty)));
    Ok(match ty {
        Type::Byte => match &v {
            mlua::Value::Integer(i) if (0..=255).contains(i) => Value::Byte(*i as u8),
            _ => return bad(&v),
        },
        Type::Int => match &v {
            mlua::Value::Integer(i) => Value::Int(*i as i32),
            mlua::Value::Number(n) if n.fract() == 0.0 => Value::Int(*n as i32),
            _ => return bad(&v),
        },
        Type::Float => match &v {
            mlua::Value::Integer(i) => Value::Float(*i as f32),
            mlua::Value::Number(n) => Value::Float(*n as f32),
            _ => return bad(&v),
        },
        Type::Bool => match &v {
            mlua::Value::Boolean(b) => Value::Bool(*b),
            _ => return bad(&v),
        },
        Type::Name => match &v {
            mlua::Value::String(s) => Value::Name(crate::value::NameStruct::from_time(&s.to_str()?)),
            _ => return bad(&v),
        },
        Type::Str => match &v {
            mlua::Value::String(s) => Value::Str(s.to_str()?.to_string()),
            mlua::Value::Integer(_) | mlua::Value::Number(_) => Value::Str(v.to_string()?),
            _ => return bad(&v),
        },
        Type::Object(_) => match &v {
            mlua::Value::Nil => Value::Object(None),
            mlua::Value::UserData(_) | mlua::Value::String(_) => match vm.mod_script_resolve(&Who::from_lua(v.clone(), lua)?) {
                Some(a) => Value::Object(Some(a)),
                None => return err(format!("{what}: nobody of that name")),
            },
            _ => return bad(&v),
        },
        Type::Class(_) => match &v {
            mlua::Value::String(s) => {
                let s = s.to_str()?.to_string();
                match vm.class(&s) {
                    Some(c) => Value::Class(Some(c)),
                    None => return err(format!("{what}: unknown class {s}")),
                }
            }
            mlua::Value::Nil => Value::Class(None),
            _ => return bad(&v),
        },
        Type::Struct(s) if s.to_ascii_lowercase().ends_with("vector") => match &v {
            mlua::Value::Table(t) => {
                let p = read_position(t)?;
                Value::vector(p[0], p[1], p[2])
            }
            _ => return bad(&v),
        },
        Type::Struct(s) if s.to_ascii_lowercase().ends_with("rotator") => match &v {
            mlua::Value::Table(t) => {
                let get = |k: &str, i: usize| -> mlua::Result<i32> { Ok(t.raw_get::<Option<i32>>(k)?.or(t.raw_get::<Option<i32>>(i)?).unwrap_or(0)) };
                Value::rotator(get("pitch", 1)?, get("yaw", 2)?, get("roll", 3)?)
            }
            _ => return bad(&v),
        },
        _ => return err(format!("{what}: parameters of type {} cannot be given by a script", type_text(ty))),
    })
}

// ---------------------------------------------------------------------------------------------
// The functions of the API (names in mod_script.rs's API table).

/// Reads the first argument of a method: the actor it is called on (`actor:go_to(...)`).
pub(crate) fn this(lua: &Lua, args: MultiValue, what: &str) -> mlua::Result<(ObjId, Vec<mlua::Value>)> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.is_empty() {
        return err(format!("{what}: write actor:{what}(...)"));
    }
    let a = Actor::from_lua(args.remove(0), lua).map_err(|_| mlua::Error::runtime(format!("{what}: write actor:{what}(...), with a colon")))?;
    Ok((a.0, args))
}

pub(crate) fn alive_character(vm: &Vm, a: ObjId, what: &str) -> mlua::Result<()> {
    if !vm.objects.get(a).is_some_and(|o| o.is_some()) || !vm.mod_alive(a) {
        return err(format!("{what}: this actor is not alive"));
    }
    if !vm.is_a(a, "Pawn") {
        return err(format!("{what}: only characters take orders"));
    }
    Ok(())
}

pub(crate) fn api_go_to(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "go_to")?;
    alive_character(vm, a, "go_to")?;
    let target = read_target(vm, lua, if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) }, "go_to")?;
    let run = match rest.first() {
        Some(mlua::Value::String(s)) => s.to_str()?.eq_ignore_ascii_case("run"),
        Some(mlua::Value::Boolean(b)) => *b,
        _ => false,
    };
    vm.mod_script_stop_follow(lua, a);
    let goal_actor = match target {
        Target::Actor(t) => Some(t),
        Target::At(_) => None,
    };
    vm.mod_cine_go_to(lua, a, target, run).map_err(mlua::Error::runtime)?;
    // Watched until it arrives: a new way when it is stuck, the doors on its way opened.
    let goal = goal_actor.or_else(|| crate::mod_script::state_mut(lua).and_then(|st| st.markers.get(&a).copied()));
    if let (Some(goal), Some(mut st)) = (goal, crate::mod_script::state_mut(lua)) {
        let now = st.now;
        let here = vm.native_value(a, Field::Location).as_vector();
        let aim = vm.native_value(goal, Field::Location).as_vector();
        st.follows.insert(a, crate::mod_script::Follow { leader: goal, next: now + FOLLOW_EVERY, last: Some((here, aim, now)), catch_up: None, arrive: true });
    }
    true.into_lua_multi(lua)
}

pub(crate) fn api_follow(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "follow")?;
    alive_character(vm, a, "follow")?;
    let Target::Actor(t) = read_target(vm, lua, if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) }, "follow")? else {
        return err("follow: give an actor to follow");
    };
    let catch_up = match rest.first() {
        Some(mlua::Value::Table(o)) => {
            for pair in o.clone().pairs::<String, mlua::Value>() {
                let (k, _) = pair?;
                if k != "catch_up" {
                    return err(format!("follow: unknown setting {k} (catch_up)"));
                }
            }
            o.get::<Option<f32>>("catch_up")?
        }
        None | Some(mlua::Value::Nil) => None,
        Some(_) => return err("follow(actor[, {catch_up = distance}])"),
    };
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.follows.insert(a, crate::mod_script::Follow { leader: t, next: 0.0, last: None, catch_up, arrive: false });
    }
    true.into_lua_multi(lua)
}

pub(crate) fn api_attack(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "attack")?;
    alive_character(vm, a, "attack")?;
    let Target::Actor(t) = read_target(vm, lua, if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) }, "attack")? else {
        return err("attack: give an actor to attack");
    };
    vm.mod_script_attack(a, t).map_err(mlua::Error::runtime)?;
    true.into_lua_multi(lua)
}

pub(crate) fn api_look_at(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "look_at")?;
    alive_character(vm, a, "look_at")?;
    let target = read_target(vm, lua, if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) }, "look_at")?;
    vm.mod_script_look_at(lua, a, target).map_err(mlua::Error::runtime)?;
    true.into_lua_multi(lua)
}

/// `who:cine("line")`: a line of the levels' own scene language (CineController2.Interpret),
/// for a story character: what the game's scenes write ("Anim Once Rire", "Shoot", "Give
/// XIII.Beretta"...).
pub(crate) fn api_cine(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "cine")?;
    alive_character(vm, a, "cine")?;
    let line = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("cine(\"command words\")"))?;
    let line = line.trim();
    if line.is_empty() || line.contains(['\n', '\r']) {
        return err("cine: one command line, as the levels write them (\"Anim Once Rire\")");
    }
    vm.mod_cine_command(a, line).map_err(mlua::Error::runtime)?;
    vm.mod_script_log(format!("{}: {} cine \"{line}\" ({})", vm.mod_map_name(), vm.mod_actor_name(a), caller(lua)));
    true.into_lua_multi(lua)
}

pub(crate) fn api_stop(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, _) = this(lua, args, "stop")?;
    vm.mod_script_stop_follow(lua, a);
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.patrols.remove(&a);
    }
    if vm.mod_cine_controller(a).is_some() {
        vm.mod_cine_command(a, "StopSeq").map_err(mlua::Error::runtime)?;
    }
    true.into_lua_multi(lua)
}

pub(crate) fn api_play(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "play")?;
    alive_character(vm, a, "play")?;
    let (anim, looped) =
        <(String, Option<bool>)>::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("play(animation[, loop])"))?;
    if anim.contains(char::is_whitespace) {
        return err(format!("play: {anim:?} is not an animation name"));
    }
    let looped = looped.unwrap_or(false);
    if vm.mod_cine_controller(a).is_some() {
        vm.mod_cine_command(a, &format!("Anim {} {anim}", if looped { "Loop" } else { "OnceNB" })).map_err(mlua::Error::runtime)?;
    } else {
        let native = if looped { "LoopAnim" } else { "PlayAnim" };
        let mut args = [Value::Name(crate::value::NameStruct::from_time(&anim)), Value::Float(1.0), Value::Float(0.1)];
        crate::natives::call(vm, native, a, &mut args, &[true, true, true]).map_err(|e| mlua::Error::runtime(format!("play {anim}: {e}")))?;
    }
    true.into_lua_multi(lua)
}

pub(crate) fn api_restart(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, _) = this(lua, args, "restart")?;
    vm.mod_restart_look(a).into_lua_multi(lua)
}

pub(crate) fn api_give(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "give")?;
    alive_character(vm, a, "give")?;
    let item = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("give(weapon or item class)"))?;
    let label = caller(lua);
    match vm.mod_script_give(a, &item) {
        Ok(class) => {
            vm.mod_report(format!("{}: give {class} to {} ({label})", vm.mod_map_name(), vm.mod_actor_name(a)));
            true.into_lua_multi(lua)
        }
        Err(why) => err(format!("give {item}: {why}")),
    }
}

pub(crate) fn api_teleport(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "teleport")?;
    let target = read_target(vm, lua, if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) }, "teleport")?;
    let at = match target {
        Target::At(p) => p,
        Target::Actor(t) => {
            let [x, y, z] = vm.native_value(t, Field::Location).as_vector();
            [x, y, z - vm.native_prop(t, Field::CollisionHeight).as_float()]
        }
    };
    let h = vm.native_prop(a, Field::CollisionHeight).as_float();
    let mut call_args = [Value::vector(at[0], at[1], at[2] + h)];
    let ok = crate::natives::call(vm, "SetLocation", a, &mut call_args, &[true]).map_err(|e| mlua::Error::runtime(format!("teleport: {e}")))?;
    ok.as_bool().into_lua_multi(lua)
}

pub(crate) fn api_kill(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, _) = this(lua, args, "kill")?;
    alive_character(vm, a, "kill")?;
    vm.mod_script_kill(a).map_err(mlua::Error::runtime)?;
    (!vm.mod_alive(a)).into_lua_multi(lua)
}

/// The class and slot of property `key` of `a` (its own class's or a class default object).
fn property_slot(vm: &Vm, a: ObjId, key: &str) -> mlua::Result<(crate::value::ClassId, usize, String, Option<usize>)> {
    let (property, index) = property_key(key);
    let Some(class) = vm.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return err(format!("{key}: this actor is not in the game any more")) };
    match vm.classes[class].prop_index.get(&property.to_lowercase()) {
        Some(&slot) => Ok((class, slot, property, index)),
        None => err(format!("{} has no property {property}", vm.class_path(class))),
    }
}

pub(crate) fn api_get(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "get")?;
    let key = String::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("get(\"Property\")"))?;
    get_property(vm, lua, a, &key)
}

fn get_property(vm: &mut Vm, lua: &Lua, a: ObjId, key: &str) -> mlua::Result<MultiValue> {
    // "Emitters[0].Disabled": a property of an object the actor holds.
    let (a, key) = crate::mod_script_world::property_path(vm, a, key)?;
    let key = key.as_str();
    let (_, slot, _, index) = property_slot(vm, a, key)?;
    let v = vm.objects[a].as_ref().map(|o| o.props[slot].clone()).unwrap_or(Value::Void);
    let v = match (index, v) {
        (Some(i), Value::Array(items)) => items.get(i).cloned().ok_or_else(|| mlua::Error::runtime(format!("{key}: index out of the array")))?,
        (Some(_), _) => return err(format!("{key}: not an array")),
        (None, v) => v,
    };
    value_to_lua(vm, lua, &v)?.into_lua_multi(lua)
}

pub(crate) fn api_set(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "set")?;
    let (key, value) =
        <(String, mlua::Value)>::from_lua_multi(MultiValue::from_vec(rest), lua).map_err(|_| mlua::Error::runtime("set(\"Property\", value)"))?;
    set_property(vm, lua, a, &key, value)?;
    vm.mod_net_fx_set(a, &key);
    ().into_lua_multi(lua)
}

/// Writes property `key` of `a` with the toml values' checks; an actor goes into an object
/// property as such.
fn set_property(vm: &mut Vm, lua: &Lua, a: ObjId, key: &str, value: mlua::Value) -> mlua::Result<()> {
    let (a, key) = crate::mod_script_world::property_path(vm, a, key)?;
    let key = key.as_str();
    let (class, slot, property, index) = property_slot(vm, a, key)?;
    let ty = vm.classes[class].props[slot].ty.clone();
    if let (mlua::Value::UserData(_), Type::Object(_), None) = (&value, &ty, index) {
        let target = vm.mod_script_resolve(&Who::from_lua(value, lua)?);
        vm.set_prop(a, &property, Value::Object(target));
        return Ok(());
    }
    let mv = lua_to_mod_value(&value, key)?;
    let Some(mut props) = vm.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { return Ok(()) };
    let r = props.with_slice(|s| vm.mod_set_value(class, s, &property, index, &mv));
    if let Some(o) = vm.objects[a].as_mut() {
        o.props = props;
    }
    r.map_err(|why| mlua::Error::runtime(format!("{key}: {why} (expected: {})", type_text(&ty))))
}

pub(crate) fn api_call(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "call")?;
    if rest.is_empty() {
        return err("call(\"Function\", ...)");
    }
    let name = String::from_lua(rest.remove(0), lua)?;
    let Some(f) = vm.find_function(a, &name, false) else {
        return err(format!("{} has no function {name}", vm.objects[a].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default()));
    };
    let types: Vec<(Type, bool)> = vm.functions[f].params.iter().map(|p| (vm.functions[f].locals[p.local].ty.clone(), p.optional)).collect();
    if rest.len() > types.len() {
        return err(format!("{name}: {} parameter(s) given, it takes {}", rest.len(), types.len()));
    }
    let mut values = Vec::new();
    for (i, (ty, optional)) in types.iter().enumerate() {
        match rest.get(i).cloned() {
            Some(v) => values.push(Some(lua_to_param(vm, lua, v, ty, &format!("{name}, parameter {}", i + 1))?)),
            None if *optional => values.push(None),
            None => return err(format!("{name}: parameter {} ({}) is missing", i + 1, type_text(ty))),
        }
    }
    match vm.call(f, a, values) {
        Ok((ret, _)) => {
            vm.mod_net_fx_call(a, &name);
            value_to_lua(vm, lua, &ret)?.into_lua_multi(lua)
        }
        Err(e) => err(format!("{name}: {e}")),
    }
}

pub(crate) fn api_class_get(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (class, key) = <(String, String)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("class_get(\"Package.Class\", \"Property\")"))?;
    let Some(c) = vm.class(&class) else { return err(format!("class_get: unknown class {class}")) };
    vm.create_cdo(c);
    get_property(vm, lua, vm.classes[c].cdo, &key)
}

pub(crate) fn api_class_set(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (class, key, value) =
        <(String, String, mlua::Value)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("class_set(\"Package.Class\", \"Property\", value)"))?;
    let Some(c) = vm.class(&class) else { return err(format!("class_set: unknown class {class}")) };
    vm.create_cdo(c);
    set_property(vm, lua, vm.classes[c].cdo, &key, value)?;
    ().into_lua_multi(lua)
}

pub(crate) fn api_change(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (who, t) = <(mlua::Value, Table)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("change(who, {look = , health = , team = , ...})"))?;
    let mut table = openxiii_mods::toml::Table::new();
    for pair in t.pairs::<String, mlua::Value>() {
        let (k, v) = pair?;
        let v = lua_to_toml(&v, &k)?;
        table.insert(k, v);
    }
    // An actor given as such: found by its object name.
    let actor = match &who {
        mlua::Value::UserData(_) => Some(Actor::from_lua(who.clone(), lua)?.0),
        _ => None,
    };
    let name = match actor {
        Some(a) => vm.mod_actor_name(a),
        None => String::from_lua(who, lua)?,
    };
    table.insert("who".into(), openxiii_mods::toml::Value::String(name));
    let (change, keys) = openxiii_mods::placement::read_change(&table).map_err(|why| mlua::Error::runtime(format!("change: {why}")))?;
    let label = caller(lua);
    let n = vm.mod_script_change(actor, &change, &keys, &label).map_err(mlua::Error::runtime)?;
    n.into_lua_multi(lua)
}

/// The player an optional last argument names (message, objective, fade): None for every player.
pub(crate) fn only_player(vm: &Vm, lua: &Lua, v: Option<mlua::Value>, what: &str) -> mlua::Result<Option<ObjId>> {
    match v {
        None | Some(mlua::Value::Nil) => Ok(None),
        Some(v) => {
            let p = vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime(format!("{what}: no such player")))?;
            if !vm.mod_script_players().contains(&p) {
                return err(format!("{what}: {} is not a player", vm.mod_actor_name(p)));
            }
            Ok(Some(p))
        }
    }
}

pub(crate) fn api_objective(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (text, who) = <(String, Option<mlua::Value>)>::from_lua_multi(args, lua)?;
    let only = only_player(vm, lua, who, "objective")?;
    vm.mod_script_goal(&text, false, only);
    vm.mod_script_log(format!("{}: objective \"{text}\" ({})", vm.mod_map_name(), caller(lua)));
    ().into_lua_multi(lua)
}

pub(crate) fn api_objective_done(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (text, who) = <(String, Option<mlua::Value>)>::from_lua_multi(args, lua)?;
    let only = only_player(vm, lua, who, "objective_done")?;
    vm.mod_script_goal(&text, true, only);
    vm.mod_script_log(format!("{}: objective done \"{text}\" ({})", vm.mod_map_name(), caller(lua)));
    ().into_lua_multi(lua)
}

pub(crate) fn api_trigger(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (event, by) = <(String, Option<mlua::Value>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("trigger(event[, who])"))?;
    let instigator = match by {
        Some(v) => vm.mod_script_resolve(&Who::from_lua(v, lua)?),
        None => vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()),
    };
    let Some(from) = instigator.or_else(|| vm.level) else { return err("trigger: no level") };
    let event_name = Value::Name(crate::value::NameStruct::from_time(&event));
    let Some(f) = vm.find_function(from, "TriggerEvent", false) else { return err("trigger: the game has no TriggerEvent here") };
    let pawn = instigator.map(|p| Value::Object(Some(p))).unwrap_or(Value::Object(None));
    vm.call(f, from, vec![Some(event_name), Some(pawn.clone()), Some(pawn)]).map_err(|e| mlua::Error::runtime(format!("trigger {event}: {e}")))?;
    vm.mod_script_log(format!("{}: trigger {event} ({})", vm.mod_map_name(), caller(lua)));
    ().into_lua_multi(lua)
}

pub(crate) fn api_sound(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (name, at) = <(String, Option<mlua::Value>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("sound(name[, actor or position])"))?;
    let source = match at {
        None | Some(mlua::Value::Nil) => vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()),
        Some(v) => match read_target(vm, lua, v, "sound")? {
            Target::Actor(a) => Some(a),
            Target::At(p) => Some(vm.mod_script_marker(lua, None, p).map_err(mlua::Error::runtime)?),
        },
    };
    let Some(source) = source else { return err("sound: no player to play it on") };
    let why = format!("{}: sound {name}", vm.mod_map_name());
    vm.mod_play_sound(source, &name, &why).map_err(|e| mlua::Error::runtime(format!("sound {name}: {e}")))?;
    // From an effect: the guests play it from their copy of it (mod_net_fx.rs); otherwise from
    // their own player.
    if !vm.mod_net_fx_sound(source, &name) {
        vm.mod_script_to_guests("Sound", 0, &name);
    }
    // Its id, for stop_sound (mod_script_more.rs).
    crate::mod_script_more::note_sound(vm, source, &name).into_lua_multi(lua)
}

pub(crate) fn api_music(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let name = String::from_lua_multi(args, lua)?;
    let Some(p) = vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()) else { return err("music: no player") };
    let why = format!("{}: music {name}", vm.mod_map_name());
    vm.mod_play_music(p, &name, &why).map_err(|e| mlua::Error::runtime(format!("music {name}: {e}")))?;
    vm.mod_script_to_guests("Music", 0, &name);
    ().into_lua_multi(lua)
}

pub(crate) fn api_next_level(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let map = match Option::<String>::from_lua_multi(args, lua)? {
        Some(m) => m,
        // The next map of the campaign (campaign{...}) after this one.
        None => {
            let here = vm.mod_map_name();
            let st = crate::mod_script::state_mut(lua).ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
            let list = st.campaign.clone();
            drop(st);
            if list.is_empty() {
                return err("next_level(): give a map, or list the maps with campaign{...}");
            }
            match list.iter().position(|m| m.eq_ignore_ascii_case(&here)) {
                Some(i) if i + 1 < list.len() => list[i + 1].clone(),
                Some(_) => return err(format!("next_level(): {here} is the last map of the campaign")),
                None => list[0].clone(),
            }
        }
    };
    let label = caller(lua);
    vm.mod_script_end(Some(&map), None).map_err(mlua::Error::runtime)?;
    vm.mod_script_log(format!("{}: next_level {map} ({label})", vm.mod_map_name()));
    ().into_lua_multi(lua)
}

pub(crate) fn api_fail(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let reason = Option::<String>::from_lua_multi(args, lua)?.unwrap_or_default();
    let label = caller(lua);
    vm.mod_script_end(None, Some(&reason)).map_err(mlua::Error::runtime)?;
    vm.mod_script_log(format!("{}: fail \"{reason}\" ({label})", vm.mod_map_name()));
    ().into_lua_multi(lua)
}

/// A behaviour by its name (behaviour()) or a function.
pub(crate) fn behaviour_function(lua: &Lua, v: mlua::Value) -> mlua::Result<mlua::Function> {
    match v {
        mlua::Value::Function(f) => Ok(f),
        mlua::Value::String(s) => {
            let name = s.to_str()?.to_ascii_lowercase();
            let st = lua.app_data_ref::<crate::mod_script::State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
            st.behaviours.get(&name).cloned().ok_or_else(|| mlua::Error::runtime(format!("no behaviour named {name} (define it with behaviour(name, fn))")))
        }
        other => err(format!("a behaviour's name or a function was expected, got {}", other.type_name())),
    }
}

pub(crate) fn api_behave(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, mut rest) = this(lua, args, "behave")?;
    alive_character(vm, a, "behave")?;
    let v = if rest.is_empty() { mlua::Value::Nil } else { rest.remove(0) };
    let mut st = crate::mod_script::state_mut(lua).ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    if v.is_nil() {
        if let Some(old) = st.behaving.remove(&a) {
            st.stop_runs(old);
        }
        return true.into_lua_multi(lua);
    }
    drop(st);
    let f = behaviour_function(lua, v)?;
    crate::mod_script::state_mut(lua).ok_or_else(|| mlua::Error::runtime("no scripts state"))?.behaviours_new.push((a, f));
    true.into_lua_multi(lua)
}

pub(crate) fn api_patrol(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (a, rest) = this(lua, args, "patrol")?;
    alive_character(vm, a, "patrol")?;
    let (t, run) = <(Table, Option<String>)>::from_lua_multi(MultiValue::from_vec(rest), lua)
        .map_err(|_| mlua::Error::runtime("patrol{{point, point, ...}[, \"run\"])"))?;
    let mut points = Vec::new();
    for i in 1..=t.raw_len() {
        points.push(read_position(&t.raw_get::<Table>(i)?)?);
    }
    if points.len() < 2 {
        return err("patrol: give at least two points");
    }
    let run = run.is_some_and(|r| r.eq_ignore_ascii_case("run"));
    vm.mod_script_stop_follow(lua, a);
    vm.mod_cine_go_to(lua, a, Target::At(points[0]), run).map_err(mlua::Error::runtime)?;
    crate::mod_script::state_mut(lua).ok_or_else(|| mlua::Error::runtime("no scripts state"))?.patrols.insert(a, (points, 0, run));
    true.into_lua_multi(lua)
}

pub(crate) fn api_characters_near(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.len() != 2 {
        return err("characters_near(position or actor, radius)");
    }
    let radius = f32::from_lua(args.pop().unwrap_or(mlua::Value::Nil), lua)?;
    let at = match read_target(vm, lua, args.remove(0), "characters_near")? {
        Target::At(p) => p,
        Target::Actor(a) => vm.native_value(a, Field::Location).as_vector(),
    };
    let t = lua.create_table()?;
    for a in vm.mod_script_characters_near(at, radius) {
        t.raw_push(Actor(a))?;
    }
    t.into_lua_multi(lua)
}

pub(crate) fn api_camera(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    // camera{from =, look_at =, ...}: the table of settings (mod_script_camera.rs).
    if let [mlua::Value::Table(t)] = &args[..] {
        if t.contains_key("look_at")? {
            return crate::mod_script_camera::api_camera_table(vm, lua, t.clone());
        }
    }
    if args.len() < 2 {
        return err("camera(position, look_at[, seconds]) or camera{from =, look_at =, seconds =, fov =, follow =, path =, orbit =}");
    }
    let seconds = if args.len() > 2 { Some(f32::from_lua(args.remove(2), lua)?) } else { None };
    let look = match read_target(vm, lua, args.remove(1), "camera")? {
        Target::At(p) => p,
        Target::Actor(a) => vm.native_value(a, Field::Location).as_vector(),
    };
    let at = match read_target(vm, lua, args.remove(0), "camera")? {
        Target::At(p) => p,
        Target::Actor(a) => vm.native_value(a, Field::Location).as_vector(),
    };
    vm.mod_script_camera(lua, at, look).map_err(mlua::Error::runtime)?;
    let now = vm.mod_level_time();
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.camera_until = seconds.map(|s| now + s.max(0.0));
    }
    vm.mod_script_log(format!("{}: camera at {{{:.0}, {:.0}, {:.0}}} ({})", vm.mod_map_name(), at[0], at[1], at[2], caller(lua)));
    ().into_lua_multi(lua)
}

pub(crate) fn api_camera_back(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    vm.mod_script_camera_back();
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.camera_until = None;
    }
    ().into_lua_multi(lua)
}

pub(crate) fn api_weapon(vm: &mut Vm, lua: &Lua, a: ObjId) -> mlua::Result<mlua::Value> {
    if !vm.is_a(a, "Pawn") {
        return Ok(mlua::Value::Nil);
    }
    let w = vm.prop(a, "Weapon").as_object().and_then(|w| vm.objects.get(w).and_then(|o| o.as_ref()).map(|o| vm.class_path(o.class)));
    w.into_lua(lua)
}

// ---------------------------------------------------------------------------------------------
// The engine side.

impl Vm {
    /// The controller of a story character (one that takes CineController2's commands).
    pub(crate) fn mod_cine_controller(&self, pawn: ObjId) -> Option<ObjId> {
        let c = self.native_prop(pawn, Field::Controller).as_object()?;
        self.find_function(c, CINE_COMMAND, false).map(|_| c)
    }

    /// Gives command `text` to story character `pawn` (CineController2.Interpret). The
    /// controller moves and turns its pawn only in its state PlayingSequence. A controller in
    /// no state (its own script over) is put back there, and so is one waiting in STA_init
    /// once set up (PC known) when its pawn has no script lines (a character added by a mod):
    /// there it reads no line of its own. A character of the map still waiting for its scene
    /// is not taken (its scene would start).
    pub(crate) fn mod_cine_command(&mut self, pawn: ObjId, text: &str) -> Result<(), String> {
        let Some(c) = self.mod_cine_controller(pawn) else {
            return Err(format!("{} is not a story character (Cine2): only those take commands", self.mod_actor_name(pawn)));
        };
        let name = self.mod_actor_name(pawn);
        let state = self.current_state(c).map(|(_, s)| s.name.to_ascii_lowercase());
        match state.as_deref() {
            Some("playingsequence") => {}
            None => self.goto_state(c, "PlayingSequence", "").map_err(|e| format!("{text}: {e}"))?,
            Some("sta_init") => {
                if self.prop(c, "PC").as_object().is_none() {
                    // Just created: the order waits until it is set up (mod_cine_pending_step).
                    let until = self.mod_level_time() + 5.0;
                    self.mods.cine_pending.push((pawn, text.to_string(), until));
                    return Ok(());
                }
                let lines = self.call_named(pawn, "GetTabActionLength", vec![]).map(|v| v.as_int()).unwrap_or(0);
                if lines > 0 {
                    return Err(format!("{name} waits for its own scene of the map ({lines} script lines): it takes orders once that scene has started"));
                }
                self.goto_state(c, "PlayingSequence", "").map_err(|e| format!("{text}: {e}"))?;
            }
            Some(other) => return Err(format!("{name} cannot take orders now (state {other})")),
        }
        self.call_named(c, CINE_COMMAND, vec![Value::Str(text.to_string())]).map(|_| ()).map_err(|e| format!("{text}: {e}"))
    }

    /// The hidden marker actor of `owner` (or the sounds' one) moved to `at` (a point on the
    /// ground): positions given to the game's functions that want an actor.
    pub(crate) fn mod_script_marker(&mut self, lua: &Lua, owner: Option<ObjId>, at: [f32; 3]) -> Result<ObjId, String> {
        let key = owner.unwrap_or(usize::MAX);
        let known =
            crate::mod_script::state_mut(lua).and_then(|st| st.markers.get(&key).copied()).filter(|&m| self.objects.get(m).is_some_and(|o| o.is_some()));
        let m = match known {
            Some(m) => m,
            None => {
                let class = self.class(MARKER_CLASS).ok_or("the game has no marker class")?;
                let m = self
                    .spawn_actor(
                        class,
                        Value::Object(None),
                        None,
                        Value::vector(at[0], at[1], at[2] + 40.0),
                        Value::rotator(0, 0, 0),
                        Value::Object(None),
                        true,
                    )
                    .map_err(|e| e.to_string())?
                    .ok_or("no room for a marker there")?;
                self.set_prop(m, "bEnabled", Value::Bool(false));
                if let Some(mut st) = crate::mod_script::state_mut(lua) {
                    st.markers.insert(key, m);
                }
                m
            }
        };
        let mut args = [Value::vector(at[0], at[1], at[2] + 40.0)];
        let _ = crate::natives::call(self, "SetLocation", m, &mut args, &[true]);
        Ok(m)
    }

    /// `go_to`: a story character walks (or runs) to an actor or a position.
    pub(crate) fn mod_cine_go_to(&mut self, lua: &Lua, pawn: ObjId, target: Target, run: bool) -> Result<(), String> {
        if self.mod_cine_controller(pawn).is_none() {
            return self.mod_soldier_go_to(lua, pawn, target);
        }
        let goal = match target {
            Target::Actor(a) => a,
            Target::At(p) => self.mod_script_marker(lua, Some(pawn), p)?,
        };
        // No straight way: the nodes of the level's path network on the way (FindPathToward
        // fills RouteCache, as for the AI), then the target.
        let walk = if run { "R" } else { "W" };
        let mut steps: Vec<String> = Vec::new();
        if let Some(c) = self.mod_cine_controller(pawn) {
            let straight = self.call_named(c, "actorReachable", vec![Value::Object(Some(goal))]).is_ok_and(|v| v.as_bool());
            if !straight {
                steps = self.mod_route(c, goal).into_iter().map(|n| self.mod_actor_name(n)).collect();
            }
        }
        steps.push(self.mod_actor_name(goal));
        let sequence: Vec<String> = steps.iter().map(|s| format!("{walk} {s}")).collect();
        self.mod_cine_command(pawn, &format!("MovSeq {}", sequence.join(" ")))
    }

    /// The path network's nodes from controller `c`'s pawn towards `goal` (RouteCache after
    /// FindPathToward), empty without a path.
    fn mod_route(&mut self, c: ObjId, goal: ObjId) -> Vec<ObjId> {
        let next = self.call_named(c, "FindPathToward", vec![Value::Object(Some(goal)), Value::Bool(true)]).ok().and_then(|v| v.as_object());
        if next.is_none() {
            return Vec::new();
        }
        let mut route: Vec<ObjId> = match self.prop(c, "RouteCache") {
            Value::Array(items) => items.iter().map_while(|v| v.as_object()).filter(|&n| n != goal).collect(),
            _ => next.into_iter().collect(),
        };
        // The nearest node may be behind: skipped when the next one is reachable straight.
        while route.len() >= 2 && self.call_named(c, "actorReachable", vec![Value::Object(Some(route[1]))]).is_ok_and(|v| v.as_bool()) {
            route.remove(0);
        }
        route
    }

    /// A soldier (IAController) walks to the target as its AI walks to an alarm or a hiding
    /// place: state VaVers with MoveActor (the target, or a marker at the position) and
    /// bReachable, then its state `nextstate` there (ResteSurPlace: it stays). Only a direct
    /// way (actorReachable): the AI's paths (PointChemin) are made by its alarm machinery.
    fn mod_soldier_go_to(&mut self, lua: &Lua, pawn: ObjId, target: Target) -> Result<(), String> {
        let name = self.mod_actor_name(pawn);
        let ai = self.native_prop(pawn, Field::Controller).as_object().ok_or_else(|| format!("{name} has no controller"))?;
        if self.prop_slot(ai, "MoveActor").is_err() || self.find_function(ai, "SetEnemy", false).is_none() {
            return Err(format!("{name} takes no route (neither a story character nor a soldier)"));
        }
        let goal = match target {
            Target::Actor(a) => a,
            Target::At(p) => self.mod_script_marker(lua, Some(pawn), p)?,
        };
        let reachable = self.call_named(ai, "actorReachable", vec![Value::Object(Some(goal))]).is_ok_and(|v| v.as_bool());
        if !reachable {
            // The way through the path network, as the AI goes to an alarm or a hiding place:
            // FindBestPathToward, then RouteCache into PointChemin (VaVers walks it).
            let found = self.call_named(ai, "FindBestPathToward", vec![Value::Object(Some(goal))]).is_ok_and(|v| v.as_bool());
            let mut route: Vec<Value> = match self.prop(ai, "RouteCache") {
                Value::Array(items) => items.into_iter().take_while(|v| v.as_object().is_some()).collect(),
                _ => Vec::new(),
            };
            // The nearest node may be behind: skipped when the next one is reachable straight.
            while route.len() >= 2 && self.call_named(ai, "actorReachable", vec![route[1].clone()]).is_ok_and(|v| v.as_bool()) {
                route.remove(0);
            }
            if !found || route.is_empty() {
                return Err(format!("{name}: no way to the target (the level's paths do not lead there)"));
            }
            if let Value::Array(mut points) = self.prop(ai, "PointChemin") {
                let n = route.len().min(points.len());
                for (i, v) in route.into_iter().take(n).enumerate() {
                    points[i] = v;
                }
                self.set_prop(ai, "PointChemin", Value::Array(points));
                self.set_prop(ai, "NbPointChemin", Value::Int(n as i32));
            }
        }
        let Value::Struct(sname, fields) = self.prop(ai, "MoveActor") else { return Err("MoveActor is not a struct".into()) };
        let index = self.struct_def(sname.as_str()).map(|d| d.index.clone()).unwrap_or_default();
        let mut fields = fields.into_vec();
        if let (Some(&i), Some(&j)) = (index.get("moveactor"), index.get("breachable")) {
            fields[i] = Value::Object(Some(goal));
            fields[j] = Value::Bool(reachable);
        } else {
            return Err("MoveActor has no MoveActor/bReachable".into());
        }
        self.set_prop(ai, "MoveActor", Value::Struct(sname, crate::value::Fields::new(fields)));
        if self.prop_slot(ai, "bAlarmeInstigator").is_ok() {
            self.set_prop(ai, "bAlarmeInstigator", Value::Bool(false));
        }
        self.set_prop(ai, "nextstate", Value::Name(crate::value::NameStruct::from_time("ResteSurPlace")));
        self.goto_state(ai, "VaVers", "Deplacement").map_err(|e| e.to_string())
    }

    /// Patrols: a character reached its point (or stopped moving) goes on to the next one.
    pub(crate) fn mod_script_patrol_step(&mut self, lua: &Lua, patrols: &mut std::collections::BTreeMap<ObjId, (Vec<[f32; 3]>, usize, bool)>) {
        patrols.retain(|&p, _| self.mod_alive(p));
        let list: Vec<ObjId> = patrols.keys().copied().collect();
        for pawn in list {
            let Some((points, index, run)) = patrols.get(&pawn).cloned() else { continue };
            let [x, y, z] = self.native_value(pawn, Field::Location).as_vector();
            let feet = z - self.native_prop(pawn, Field::CollisionHeight).as_float();
            let at = points[index % points.len()];
            let close = ((x - at[0]).powi(2) + (y - at[1]).powi(2) + (feet - at[2]).powi(2)).sqrt() < 90.0;
            let ai = self.native_prop(pawn, Field::Controller).as_object();
            let idle = ai.is_some_and(|c| {
                let state = self.current_state(c).map(|(_, s)| s.name.to_ascii_lowercase());
                let moving = self.prop_slot(c, "bMoving").is_ok() && self.prop(c, "bMoving").as_bool();
                !moving && state.as_deref() != Some("vavers")
            });
            if close || idle {
                let next = if close { index + 1 } else { index };
                let goal = points[next % points.len()];
                let _ = self.mod_cine_go_to(lua, pawn, Target::At(goal), run);
                if let Some(e) = patrols.get_mut(&pawn) {
                    e.1 = next % points.len();
                }
            }
        }
    }

    /// Characters alive within `radius` of `at` (players not included), nearest first.
    fn mod_script_characters_near(&self, at: [f32; 3], radius: f32) -> Vec<ObjId> {
        let players = self.mod_script_players();
        let mut found: Vec<(f32, ObjId)> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.is_a(a, "Pawn") && self.mod_alive(a) && !players.contains(&a))
            .map(|a| {
                let [x, y, z] = self.native_value(a, Field::Location).as_vector();
                (((x - at[0]).powi(2) + (y - at[1]).powi(2) + (z - at[2]).powi(2)).sqrt(), a)
            })
            .filter(|(d, _)| *d <= radius)
            .collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, a)| a).collect()
    }

    /// A scene: each player's view goes to a marker at `at` turned to `look`, with the
    /// cinematic frame (Level.bCineFrame) and the controls frozen (state NoControl, as the
    /// game's own scenes do: CineController2's FreezePlayer).
    pub(crate) fn mod_script_camera(&mut self, lua: &Lua, at: [f32; 3], look: [f32; 3]) -> Result<(), String> {
        let m = self.mod_script_camera_aim(lua, at, look)?;
        for pc in self.mod_script_player_controllers() {
            let _ = self.call_named(pc, "SetViewTarget", vec![Value::Object(Some(m))]);
            if !self.mods.script_camera {
                let _ = self.goto_state(pc, "NoControl", "");
            }
        }
        if let Some(l) = self.level {
            self.set_prop(l, "bCineFrame", Value::Bool(true));
        }
        self.mods.script_camera = true;
        Ok(())
    }

    /// The scene's camera marker placed at `at` and turned to `look` (a camera that moves:
    /// each frame).
    pub(crate) fn mod_script_camera_aim(&mut self, lua: &Lua, at: [f32; 3], look: [f32; 3]) -> Result<ObjId, String> {
        let before = crate::mod_script::state_mut(lua)
            .and_then(|st| st.markers.get(&(usize::MAX - 1)).copied())
            .filter(|&m| self.objects.get(m).is_some_and(|o| o.is_some()))
            .map(|m| self.native_value(m, Field::Location));
        let m = self.mod_script_marker(lua, Some(usize::MAX - 1), at)?;
        // A point outside the level's geometry would show nothing (a black screen): the camera
        // stays where it was.
        let outside = matches!(self.native_value(m, Field::Region), Value::Struct(_, ref f) if matches!(f.get(1), Some(Value::Int(-1))));
        if outside {
            if let Some(place) = before {
                let mut args = [place];
                let _ = crate::natives::call(self, "SetLocation", m, &mut args, &[true]);
            }
            return Err(format!("the camera point {{{:.0}, {:.0}, {:.0}}} is outside the level (in a wall, a roof or the ground)", at[0], at[1], at[2]));
        }
        let (dx, dy, dz) = (look[0] - at[0], look[1] - at[1], look[2] - at[2]);
        let yaw = (dy.datan2(dx) * 32768.0 / std::f32::consts::PI).round() as i32;
        let pitch = (dz.datan2((dx * dx + dy * dy).sqrt()) * 32768.0 / std::f32::consts::PI).round() as i32;
        let mut args = [Value::rotator(pitch.rem_euclid(65536), yaw.rem_euclid(65536), 0)];
        let _ = crate::natives::call(self, "SetRotation", m, &mut args, &[true]);
        Ok(m)
    }

    /// The end of a scene: each player's own view and controls (PlayerWalking, as the game's
    /// ReleasePlayer), no cinematic frame.
    pub(crate) fn mod_script_camera_back(&mut self) {
        if !self.mods.script_camera {
            return;
        }
        self.mods.script_camera = false;
        for pc in self.mod_script_player_controllers() {
            // A scene's field of view (camera{fov = }): the player's own, as RestoreView does.
            let fov = self.prop(pc, "DefaultFOV");
            self.set_prop(pc, "DesiredFOV", fov.clone());
            self.set_prop(pc, "FovAngle", fov);
            if let Some(p) = self.prop(pc, "Pawn").as_object() {
                let _ = self.call_named(pc, "SetViewTarget", vec![Value::Object(Some(p))]);
            }
            let _ = self.goto_state(pc, "PlayerWalking", "");
        }
        if let Some(l) = self.level {
            self.set_prop(l, "bCineFrame", Value::Bool(false));
        }
    }

    /// The followers: each one walks again towards its leader when it is far.
    /// The followers. One far from its leader is given its way again (through the path
    /// network) when it stands idle, when it got no closer for STUCK_SECONDS, and at least every
    /// REROUTE_SECONDS (the leader moves); a soldier walking is not interrupted more often (each
    /// order starts with its turn). With catch_up, one beyond that distance that no player sees
    /// is put back behind its leader, where no player sees it either.
    pub(crate) fn mod_script_follow_step(&mut self, lua: Option<&Lua>, follows: &mut std::collections::BTreeMap<ObjId, crate::mod_script::Follow>, now: f32) {
        follows.retain(|&pawn, f| self.mod_alive(pawn) && (self.mod_alive(f.leader) || f.arrive && self.objects.get(f.leader).is_some_and(|o| o.is_some())));
        if !self.mods.locked_doors.is_empty() {
            self.mod_unlocked_doors();
        }
        let list: Vec<ObjId> = follows.keys().copied().collect();
        for pawn in list {
            let Some(mut f) = follows.get(&pawn).cloned() else { continue };
            if now < f.next {
                continue;
            }
            f.next = now + FOLLOW_EVERY;
            let d = self.mod_distance(pawn, f.leader);
            if f.arrive && d <= ARRIVED {
                // A go_to that arrived.
                follows.remove(&pawn);
                continue;
            }
            if !f.arrive && d <= FOLLOW_DISTANCE {
                f.last = None;
                follows.insert(pawn, f);
                continue;
            }
            if f.catch_up.is_some_and(|c| d > c) && self.mod_catch_up(pawn, f.leader) {
                f.last = None;
                follows.insert(pawn, f);
                continue;
            }
            // A new way only when it stands, has not moved for a while, or its leader went
            // elsewhere: a new way while it walks would start again from the node behind it.
            let here = self.native_value(pawn, Field::Location).as_vector();
            let leader_at = self.native_value(f.leader, Field::Location).as_vector();
            let far = |a: [f32; 3], b: [f32; 3], d: f32| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() > d;
            let idle = self.mod_follower_idle(pawn);
            let stuck = f.last.is_some_and(|(from, _, at)| now - at >= STUCK_SECONDS && !far(from, here, 60.0));
            let moved_on = f.last.is_some_and(|(_, aim, at)| now - at >= REROUTE_SECONDS && far(aim, leader_at, 400.0));
            if stuck {
                // In front of a closed door: opened as the game's AI opens one it bumps into
                // (PlayerTrigger); a door the level keeps locked stays locked.
                self.mod_open_doors_near(pawn);
            }
            if f.arrive && idle && !stuck && f.last.is_some_and(|(_, _, at)| now - at < STUCK_SECONDS) {
                follows.insert(pawn, f);
                continue;
            }
            if f.last.is_none() || idle || stuck || moved_on {
                if let Some(lua) = lua {
                    if let Err(why) = self.mod_cine_go_to(lua, pawn, Target::Actor(f.leader), false) {
                        // Said once per follow, not every half second (or when stuck).
                        if f.last.is_none() || stuck {
                            self.mod_report(format!("warning: {}: follow: {why}", self.mod_map_name()));
                        }
                    }
                }
                f.last = Some((here, leader_at, now));
            }
            follows.insert(pawn, f);
        }
    }

    /// The closed doors (XIIIPorte) within reach of `pawn`, triggered as the game's AI triggers
    /// a door it bumps into: `PlayerTrigger(its controller, it)`.
    fn mod_open_doors_near(&mut self, pawn: ObjId) {
        let Some(ai) = self.native_prop(pawn, Field::Controller).as_object() else { return };
        let doors: Vec<ObjId> = self
            .actors
            .iter()
            .copied()
            .filter(|&d| self.is_a(d, "XIIIPorte") && self.objects.get(d).is_some_and(|o| o.is_some()) && self.mod_distance(pawn, d) < DOOR_REACH)
            .collect();
        for d in doors {
            if !self.prop(d, "bClosed").as_bool() {
                continue;
            }
            let locked = self.current_state(d).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("Locked"));
            if locked {
                // A door the level keeps locked: no way through it for anyone; said once (the
                // level's paths may know no other way: catch_up, or another route, then).
                if !self.mods.locked_doors.iter().any(|(x, _)| *x == d) {
                    let line = format!(
                        "warning: {}: {} is stuck at door {}, which the level keeps locked (follow(..., {{catch_up = distance}}) brings a follower back)",
                        self.mod_map_name(),
                        self.mod_actor_name(pawn),
                        self.mod_actor_name(d)
                    );
                    self.mod_report(line);
                    self.mods.locked_doors.push((d, Vec::new()));
                }
                continue;
            }
            let _ = self.call_named(d, "PlayerTrigger", vec![Value::Object(Some(ai)), Value::Object(Some(pawn))]);
            let line = format!("{}: {} pushed door {}", self.mod_map_name(), self.mod_actor_name(pawn), self.mod_actor_name(d));
            self.mod_report(line);
        }
    }

    /// The doors closed to the paths by mod_open_doors_near that are no longer locked: their
    /// nodes open again.
    pub(crate) fn mod_unlocked_doors(&mut self) {
        for (d, nodes) in std::mem::take(&mut self.mods.locked_doors) {
            let still = self.objects.get(d).is_some_and(|o| o.is_some()) && self.current_state(d).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("Locked"));
            if still {
                self.mods.locked_doors.push((d, nodes));
            } else {
                for n in nodes {
                    self.set_prop(n, "bBlocked", Value::Bool(false));
                }
            }
        }
    }

    fn mod_distance(&self, a: ObjId, b: ObjId) -> f32 {
        let [x, y, z] = self.native_value(a, Field::Location).as_vector();
        let [p, q, r] = self.native_value(b, Field::Location).as_vector();
        ((x - p).powi(2) + (y - q).powi(2) + (z - r).powi(2)).sqrt()
    }

    /// The follower stands: a story character not moving, a soldier out of VaVers.
    fn mod_follower_idle(&self, pawn: ObjId) -> bool {
        let Some(c) = self.native_prop(pawn, Field::Controller).as_object() else { return true };
        if self.mod_cine_controller(pawn).is_some() {
            return !self.prop(c, "bMoving").as_bool();
        }
        !self.current_state(c).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("vavers"))
    }

    /// Whether a player sees `a` (its controller's LineOfSightTo).
    fn mod_seen_by_players(&mut self, a: ObjId) -> bool {
        self.mod_script_player_controllers().into_iter().any(|pc| self.call_named(pc, "LineOfSightTo", vec![Value::Object(Some(a))]).is_ok_and(|v| v.as_bool()))
    }

    /// catch_up: `pawn`, seen by no player, put back behind `leader` at a spot no player sees.
    fn mod_catch_up(&mut self, pawn: ObjId, leader: ObjId) -> bool {
        if self.mod_seen_by_players(pawn) {
            return false;
        }
        let [lx, ly, lz] = self.native_value(leader, Field::Location).as_vector();
        let yaw = self.native_value(leader, Field::Rotation).as_rotator()[1] as f32 * std::f32::consts::PI / 32768.0;
        let h = self.native_prop(pawn, Field::CollisionHeight).as_float() - self.native_prop(leader, Field::CollisionHeight).as_float();
        let from = self.native_value(pawn, Field::Location);
        // Behind the leader first, then to the sides.
        for turn in [180.0f32, 150.0, 210.0, 120.0, 240.0, 90.0, 270.0] {
            let a = yaw + turn.to_radians();
            for dist in [180.0, 260.0] {
                let spot = Value::vector(lx + a.dcos() * dist, ly + a.dsin() * dist, lz + h);
                let mut args = [spot];
                let placed = crate::natives::call(self, "SetLocation", pawn, &mut args, &[true]).is_ok_and(|v| v.as_bool());
                if !placed {
                    continue;
                }
                if self.mod_seen_by_players(pawn) {
                    let mut back = [from.clone()];
                    let _ = crate::natives::call(self, "SetLocation", pawn, &mut back, &[true]);
                    continue;
                }
                let line = format!("{}: follow: {} caught up with {}", self.mod_map_name(), self.mod_actor_name(pawn), self.mod_actor_name(leader));
                self.mod_script_log(line);
                return true;
            }
        }
        false
    }

    fn mod_script_stop_follow(&mut self, lua: &Lua, pawn: ObjId) {
        if let Some(mut st) = crate::mod_script::state_mut(lua) {
            st.follows.remove(&pawn);
        }
    }

    /// `attack`: a soldier's AI gets `target` as its enemy, as the game's scripts give one
    /// (SetEnemy, then EnemyAcquired when it took); a player target also makes it hostile to
    /// the players.
    fn mod_script_attack(&mut self, pawn: ObjId, target: ObjId) -> Result<(), String> {
        self.mod_script_attack_now(pawn, target)?;
        // A soldier just created is still setting up its AI (state init), which forgets the
        // enemy: the order is given again until it holds (at most ATTACK_SECONDS).
        let until = self.mod_level_time() + ATTACK_SECONDS;
        self.mods.script_attacks.retain(|a| a.0 != pawn);
        self.mods.script_attacks.push((pawn, target, until));
        Ok(())
    }

    /// Each frame: the orders given to story characters before they were set up.
    pub(crate) fn mod_cine_pending_step(&mut self, now: f32) {
        for (pawn, text, until) in std::mem::take(&mut self.mods.cine_pending) {
            if now > until || !self.mod_alive(pawn) {
                if now > until {
                    self.mod_report(format!("warning: {}: {} was never set up; order {text:?} dropped", self.mod_map_name(), self.mod_actor_name(pawn)));
                }
                continue;
            }
            let controller = self.mod_cine_controller(pawn);
            // A level without MapInfo (a map made for the multiplayer game): CineController2's
            // STA_init waits for MapInfo.XIIIController forever. A character of the mods is
            // given the first player instead (the loop takes PC as soon as it is set).
            if let Some(c) = controller.filter(|&c| self.prop(c, "PC").as_object().is_none() && self.mod_map_info().is_none()) {
                if let Some(pc) = self.mod_script_player_controllers().into_iter().find(|&pc| self.prop(pc, "Pawn").as_object().is_some()) {
                    self.set_prop(c, "PC", Value::Object(Some(pc)));
                }
            }
            let ready = controller.is_some_and(|c| self.prop(c, "PC").as_object().is_some());
            if ready {
                if let Err(why) = self.mod_cine_command(pawn, &text) {
                    self.mod_report(format!("warning: {}: {why}", self.mod_map_name()));
                }
            } else {
                self.mods.cine_pending.push((pawn, text, until));
            }
        }
    }

    /// Each frame: the attack orders not yet held by their soldier's AI.
    pub(crate) fn mod_script_attack_step(&mut self, now: f32) {
        for (pawn, target, until) in std::mem::take(&mut self.mods.script_attacks) {
            if now > until || !self.mod_alive(pawn) || !self.mod_alive(target) {
                continue;
            }
            let Some(ai) = self.native_prop(pawn, Field::Controller).as_object() else { continue };
            let starting = self.current_state(ai).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("init"));
            if !starting && self.prop(ai, "Enemy").as_object() == Some(target) {
                continue;
            }
            if !starting {
                let _ = self.mod_script_attack_now(pawn, target);
            }
            self.mods.script_attacks.push((pawn, target, until));
        }
    }

    fn mod_script_attack_now(&mut self, pawn: ObjId, target: ObjId) -> Result<(), String> {
        let Some(ai) = self.native_prop(pawn, Field::Controller).as_object() else { return Err(format!("{} has no controller", self.mod_actor_name(pawn))) };
        if self.find_function(ai, "SetEnemy", false).is_none() || self.mod_cine_controller(pawn).is_some() {
            return Err(format!("{} does not fight (only soldiers, IAController, attack)", self.mod_actor_name(pawn)));
        }
        let player = self.mod_script_players().contains(&target);
        if player {
            self.mods.script_teams.insert(pawn, Team::Enemy);
            if self.prop_slot(ai, "XIII").is_ok() {
                self.set_prop(ai, "XIII", Value::Object(Some(target)));
            }
        }
        let _ = self.call_named(ai, "SetEnemy", vec![Value::Object(Some(target))]);
        if self.prop(ai, "Enemy").as_object() != Some(target) {
            self.set_prop(ai, "Enemy", Value::Object(Some(target)));
        }
        if self.find_function(ai, "EnemyAcquired", false).is_some() {
            let _ = self.call_named(ai, "EnemyAcquired", vec![]);
        }
        Ok(())
    }

    /// `look_at`: a story character turns its head and body to the target (LookAt); another
    /// character is turned towards it.
    fn mod_script_look_at(&mut self, lua: &Lua, pawn: ObjId, target: Target) -> Result<(), String> {
        if self.mod_cine_controller(pawn).is_some() {
            let name = match target {
                Target::Actor(a) => self.mod_actor_name(a),
                Target::At(p) => {
                    let m = self.mod_script_marker(lua, Some(pawn), p)?;
                    self.mod_actor_name(m)
                }
            };
            return self.mod_cine_command(pawn, &format!("LookAt {name}"));
        }
        let to = match target {
            Target::Actor(a) => self.native_value(a, Field::Location).as_vector(),
            Target::At(p) => p,
        };
        let [x, y, _] = self.native_value(pawn, Field::Location).as_vector();
        let yaw = ((to[1] - y).datan2(to[0] - x) * 32768.0 / std::f32::consts::PI).round() as i32;
        let [p, _, r] = self.native_value(pawn, Field::Rotation).as_rotator();
        let mut args = [Value::rotator(p, yaw.rem_euclid(65536), r)];
        crate::natives::call(self, "SetRotation", pawn, &mut args, &[true]).map(|_| ()).map_err(|e| e.to_string())
    }

    /// `give`: a weapon (Pawn.GiveWeapon) or another item of the inventory (spawned, then
    /// GiveTo). The class given.
    pub(crate) fn mod_script_give(&mut self, pawn: ObjId, item: &str) -> Result<String, String> {
        let class_name = openxiii_mods::simple::resolve_weapon(item).unwrap_or_else(|_| item.to_string());
        let class = self.class(&class_name).ok_or_else(|| format!("unknown weapon or class {item}"))?;
        let weapon = self.class("Engine.Weapon").is_some_and(|w| self.is_child_of(class, w));
        if weapon {
            self.call_named(pawn, "GiveWeapon", vec![Value::Str(class_name.clone())]).map_err(|e| e.to_string())?;
            return Ok(class_name);
        }
        if !self.class("Engine.Inventory").is_some_and(|i| self.is_child_of(class, i)) {
            return Err(format!("{class_name} is not an item of the inventory"));
        }
        let at = self.native_value(pawn, Field::Location);
        let item = self
            .spawn_actor(class, Value::Object(None), None, at, Value::rotator(0, 0, 0), Value::Object(None), true)
            .map_err(|e| e.to_string())?
            .ok_or("the game could not create it")?;
        self.call_named(item, "GiveTo", vec![Value::Object(Some(pawn))]).map_err(|e| e.to_string())?;
        Ok(class_name)
    }

    /// `kill`: the character takes damage enough to die, as from a gun shot (TakeDamage).
    fn mod_script_kill(&mut self, pawn: ObjId) -> Result<(), String> {
        let damage = self.class(KILL_DAMAGE).map(|c| Value::Class(Some(c))).unwrap_or(Value::Class(None));
        let health = self.prop(pawn, "Health").as_int().max(1);
        let at = self.native_value(pawn, Field::Location);
        self.call_named(pawn, "TakeDamage", vec![Value::Int(health + 10_000), Value::Object(None), at, Value::vector(0.0, 0.0, 0.0), damage])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// `change`: a `[[map.<Map>.change]]` entry made now by a script: the toml's own code
    /// (mod_change.rs) for the look, health and properties; team through the scripts' teams,
    /// can_die through the controller's god mode, a weapon given. The number of actors changed.
    fn mod_script_change(
        &mut self,
        actor: Option<ObjId>,
        c: &openxiii_mods::placement::Change,
        keys: &openxiii_mods::simple::SimpleKeys,
        label: &str,
    ) -> Result<usize, String> {
        let map = self.mod_map_name();
        let found: Vec<ObjId> = match actor {
            Some(a) => vec![a],
            None => self.mod_find_placed(&c.who, c.near, c.radius).into_iter().filter(|&a| self.mod_alive(a)).collect(),
        };
        if found.is_empty() {
            return Err(format!("nobody named {} on this map", c.who));
        }
        let model = self.mod_change_model(c)?;
        for &a in &found {
            for why in self.mod_change_actor(a, c, model.as_deref()) {
                self.mod_report(format!("warning: {map}: script change {}: {why} ({label})", self.mod_actor_name(a)));
            }
            if !self.is_a(a, "Pawn") {
                continue;
            }
            if let Some(t) = keys.team {
                self.mods.script_teams.insert(a, t);
            }
            if let Some(can_die) = keys.can_die {
                if let Some(ai) = self.native_prop(a, Field::Controller).as_object() {
                    if self.prop_slot(ai, "bGodMode").is_ok() {
                        self.set_prop(ai, "bGodMode", Value::Bool(!can_die));
                    }
                }
            }
            if let Some(w) = &keys.weapon {
                if let Err(why) = self.mod_script_give(a, w) {
                    self.mod_report(format!("warning: {map}: script change {}: weapon {w}: {why} ({label})", self.mod_actor_name(a)));
                }
            }
        }
        let names: Vec<String> = found.iter().map(|&a| self.mod_actor_name(a)).collect();
        self.mod_report(format!("{map}: script {}: {} ({label})", c.describe(), names.join(", ")));
        Ok(found.len())
    }

    /// A goal shown by the HUD as the game's goals are (XIIIGoalMessage: 0 a new goal, 1 a goal
    /// reached). A player of another computer gets it as a message (the HUD is local).
    /// The players a goal is sent to. Co-op: a goal on the first player's HUD is repeated on
    /// the others' (coop_story.rs), here and on another computer; sent to each, it would show
    /// twice there.
    pub(crate) fn mod_script_goal_targets(&self) -> Vec<ObjId> {
        match self.coop.players.first().copied().filter(|&pc| self.coop.players.len() >= 2 && self.coop_object_alive(pc)) {
            Some(pc) => vec![pc],
            None => self.mod_script_player_controllers(),
        }
    }

    /// `only`: the player (a pawn) it is for; None: every player.
    pub(crate) fn mod_script_goal(&mut self, text: &str, done: bool, only: Option<ObjId>) {
        let info = self.mod_map_info();
        let class = self.class("XIII.XIIIGoalMessage");
        let targets = match only {
            Some(p) => self.mod_script_player_controllers().into_iter().filter(|&pc| self.native_prop(pc, Field::Pawn).as_object() == Some(p)).collect(),
            None => self.mod_script_goal_targets(),
        };
        for pc in targets {
            if self.local_player_controller(pc).is_none() {
                // A player on another computer: its own game shows it (mod_script_guest_call).
                let line = guest_line("Goal", done as i32, text);
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line), Value::Name("None".into())]);
                continue;
            }
            let hud = self.prop(pc, "myHUD").as_object();
            match (hud, class) {
                (Some(hud), Some(class)) => {
                    let pri = self.prop(pc, "PlayerReplicationInfo");
                    let args = vec![
                        Some(Value::Class(Some(class))),
                        Some(Value::Int(done as i32)),
                        Some(pri),
                        Some(Value::Object(None)),
                        Some(Value::Object(info)),
                        Some(Value::Str(text.to_string())),
                    ];
                    if let Some(f) = self.find_function(hud, "LocalizedMessage", false) {
                        let _ = self.call(f, hud, args);
                    }
                }
                _ => {
                    let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(text.to_string())]);
                }
            }
        }
    }

    /// Host: a music or a sound of the scripts for each player on another computer (the
    /// host's own PlaySound and PlayMusic are heard on the host only).
    pub(crate) fn mod_script_to_guests(&mut self, kind: &str, number: i32, text: &str) {
        for pc in self.mod_script_player_controllers() {
            if self.local_player_controller(pc).is_none() {
                let line = guest_line(kind, number, text);
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line), Value::Name("None".into())]);
            }
        }
    }

    /// Network guest: a ClientMessage of the host's scripts (`guest_line`) shown or played
    /// here: a goal by this player's HUD, a music or a sound from this player. `true`: taken
    /// (the call is not run).
    pub(crate) fn mod_script_guest_call(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if !self.functions[f].name.eq_ignore_ascii_case("ClientMessage") {
            return false;
        }
        let Some(Some(Value::Str(line))) = args.first() else { return false };
        let Some((kind, number, text)) = parse_guest_line(line) else { return false };
        let (kind, text) = (kind.to_string(), text.to_string());
        let pawn = self.prop(this, "Pawn").as_object().or_else(|| self.prop(this, "ViewTarget").as_object());
        let why = format!("{}: from the host", self.mod_map_name());
        let done = match kind.as_str() {
            "Goal" => {
                let hud = self.prop(this, "myHUD").as_object();
                match (hud, self.class("XIII.XIIIGoalMessage")) {
                    (Some(hud), Some(class)) => match self.find_function(hud, "LocalizedMessage", false) {
                        Some(lm) => {
                            let args = vec![
                                Some(Value::Class(Some(class))),
                                Some(Value::Int(number)),
                                Some(Value::Object(None)),
                                Some(Value::Object(None)),
                                Some(Value::Object(None)),
                                Some(Value::Str(text.clone())),
                            ];
                            self.call(lm, hud, args).map(|_| ()).map_err(|e| e.to_string())
                        }
                        None => Err("no HUD".into()),
                    },
                    _ => Err("no HUD".into()),
                }
            }
            "Music" => pawn.ok_or_else(|| "no player".to_string()).and_then(|p| self.mod_play_music(p, &text, &why)),
            "Sound" => pawn.ok_or_else(|| "no player".to_string()).and_then(|p| self.mod_play_sound(p, &text, &why)),
            crate::mod_script_panel::GUEST_KIND => self.mod_panel_from_host(this, &text),
            crate::mod_script_effects::FOCUS_KIND | crate::mod_script_effects::FADE_KIND => self.mod_effect_from_host(this, &kind, &text),
            crate::mod_script_keys::KEYS_KIND => {
                self.mod_guest_keys_from_host(&text);
                Ok(())
            }
            crate::mod_script_hud::HUD_KIND => self.mod_hud_from_host(&text),
            crate::mod_script_more::STOP_SOUND_KIND | crate::mod_script_more::STOP_MUSIC_KIND => self.mod_stop_from_host(this, &kind, &text),
            _ => Err(format!("unknown message {kind}")),
        };
        match done {
            // A panel's text is its settings: the kind is enough.
            Ok(())
                if [crate::mod_script_panel::GUEST_KIND, crate::mod_script_effects::FOCUS_KIND, crate::mod_script_effects::FADE_KIND, crate::mod_script_hud::HUD_KIND]
                    .contains(&kind.as_str()) =>
            {
                self.mod_script_log(format!("guest: {}", kind.to_lowercase()))
            }
            Ok(()) => self.mod_script_log(format!("guest: {} {text}", kind.to_lowercase())),
            Err(why) => self.mod_report(format!("warning: guest: {} {text}: {why}", kind.to_lowercase())),
        }
        true
    }

    /// The level's MapInfo (XIIIGameInfo.MapInfo).
    fn mod_map_info(&self) -> Option<ObjId> {
        let game = self.level.and_then(|l| self.prop(l, "Game").as_object())?;
        self.prop(game, "MapInfo").as_object()
    }

    /// The end of the mission: success towards `next` (MapInfo.NextMapLevelWithUnr, then
    /// EndGame "GoalComplete"), or the game over of a failed goal with `reason`.
    fn mod_script_end(&mut self, next: Option<&str>, reason: Option<&str>) -> Result<(), String> {
        let game = self.level.and_then(|l| self.prop(l, "Game").as_object()).ok_or("no game is running")?;
        if self.prop(game, "bGameEnded").as_bool() {
            return Err("the game has already ended".into());
        }
        let pri = self.mod_script_player_controllers().first().map(|&pc| self.prop(pc, "PlayerReplicationInfo")).unwrap_or(Value::Object(None));
        let text = match (next, reason) {
            (Some(map), _) => {
                let Some(info) = self.mod_map_info() else {
                    // A map without the story's MapInfo: the level's own travel (ServerTravel,
                    // the players' items kept).
                    let level = self.level.ok_or("no level")?;
                    return self.call_named(level, "ServerTravel", vec![Value::Str(map.to_string()), Value::Bool(true)]).map(|_| ()).map_err(|e| e.to_string());
                };
                self.set_prop(info, "NextMapLevelWithUnr", Value::Str(map.to_string()));
                "GoalComplete".to_string()
            }
            (None, r) => format!("GoalIncomplete:{}", r.unwrap_or_default()),
        };
        self.call_named(game, "EndGame", vec![pri, Value::Str(text)]).map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_script::parse_team;

    #[test]
    fn guest_lines_round_trip() {
        let line = guest_line("Goal", 1, "Hold the beach | now");
        assert_eq!(parse_guest_line(&line), Some(("Goal", 1, "Hold the beach | now")));
        assert_eq!(parse_guest_line(&guest_line("Music", 0, "XIIIMusic.Combat.hCombat1")), Some(("Music", 0, "XIIIMusic.Combat.hCombat1")));
        assert_eq!(parse_guest_line("OXIIIMod.Skins.3|x"), None);
        assert_eq!(parse_guest_line("Hello"), None);
    }

    #[test]
    fn property_keys() {
        assert_eq!(property_key("Health"), ("Health".into(), None));
        assert_eq!(property_key("Skins[2]"), ("Skins".into(), Some(2)));
        assert_eq!(property_key("Skins[x]"), ("Skins[x]".into(), None));
    }

    #[test]
    fn team_names() {
        assert!(matches!(parse_team("Enemy"), Ok(Team::Enemy)));
        assert!(parse_team("blue").is_err());
    }
}
