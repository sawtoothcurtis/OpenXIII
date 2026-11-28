//! Mods build only (feature `mods`): `camera{...}`, a scene's camera with settings.
//!
//! The levels' scenes freeze the player and show the view of a point turned towards a target
//! (CineController2's FreezePlayer, with the cinematic frame); the security cameras' view
//! (CamViewTrigger) follows an object with its own field of view. `camera{}` gives that view, and
//! moves it each frame when asked: along a path, following an actor, or around a target. The
//! view itself is the scene's (mod_script_game.rs: one hidden actor that every player's
//! controller looks through).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use mlua::{FromLua, IntoLuaMulti, Lua, MultiValue, Table};

use crate::mod_script::{caller, read_position, Who};
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// What the camera looks at: an actor (followed) or a point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Aim {
    Actor(ObjId),
    At([f32; 3]),
}

/// How the camera moves during the scene.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Motion {
    Still([f32; 3]),
    /// Keeps its offset from the actor.
    Follow {
        actor: ObjId,
        offset: [f32; 3],
    },
    /// From point to point, at an even speed, over the scene's seconds.
    Path(Vec<[f32; 3]>),
    /// Around what it looks at: radius, height above it, turns over the scene's seconds, start angle.
    Orbit {
        radius: f32,
        height: f32,
        turns: f32,
        start: f32,
    },
}

/// A camera that moves: from `start`, over `seconds`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CameraMove {
    pub aim: Aim,
    pub motion: Motion,
    pub start: f32,
    pub seconds: f32,
    /// A point of the way was in a wall (said once; the camera stays at its last good place).
    pub warned: bool,
}

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

fn read_aim(vm: &Vm, lua: &Lua, v: mlua::Value, what: &str) -> mlua::Result<Aim> {
    match v {
        mlua::Value::Table(t) => Ok(Aim::At(read_position(&t)?)),
        mlua::Value::Nil => rt(format!("camera: {what} = an actor or a position {{x, y, z}}")),
        other => match vm.mod_script_resolve(&Who::from_lua(other, lua)?) {
            Some(a) => Ok(Aim::Actor(a)),
            None => rt(format!("camera: {what}: nobody of that name")),
        },
    }
}

impl Vm {
    fn mod_aim_point(&self, aim: &Aim) -> [f32; 3] {
        match aim {
            Aim::At(p) => *p,
            Aim::Actor(a) => self.native_value(*a, Field::Location).as_vector(),
        }
    }

    /// The camera's point and target at level time `now`.
    pub(crate) fn mod_camera_place(&self, m: &CameraMove, now: f32) -> ([f32; 3], [f32; 3]) {
        let look = self.mod_aim_point(&m.aim);
        let t = if m.seconds > 0.0 { ((now - m.start) / m.seconds).clamp(0.0, 1.0) } else { 0.0 };
        let at = match &m.motion {
            Motion::Still(p) => *p,
            Motion::Follow { actor, offset } => {
                let p = self.native_value(*actor, Field::Location).as_vector();
                [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]]
            }
            Motion::Path(points) => {
                // Even speed along the whole path.
                let lengths: Vec<f32> =
                    points.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).collect();
                let total: f32 = lengths.iter().sum();
                let mut left = t * total;
                let mut at = *points.last().unwrap_or(&look);
                for (i, l) in lengths.iter().enumerate() {
                    if left <= *l || i + 1 == lengths.len() {
                        let k = if *l > 0.0 { (left / l).clamp(0.0, 1.0) } else { 1.0 };
                        let (a, b) = (points[i], points[i + 1]);
                        at = [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
                        break;
                    }
                    left -= l;
                }
                at
            }
            Motion::Orbit { radius, height, turns, start } => {
                let angle = start + t * turns * std::f32::consts::TAU;
                [look[0] + radius * angle.dcos(), look[1] + radius * angle.dsin(), look[2] + height]
            }
        };
        (at, look)
    }

    /// Each scripts' frame: a moving camera goes on. A point in a wall leaves the camera where
    /// it was; the first one is said in the log (returns true once it has been said).
    pub(crate) fn mod_camera_step(&mut self, lua: &Lua, m: &CameraMove, now: f32) -> bool {
        let (at, look) = self.mod_camera_place(m, now);
        match self.mod_script_camera_aim(lua, at, look) {
            Ok(_) => m.warned,
            Err(why) => {
                if !m.warned {
                    let map = self.mod_map_name();
                    self.mod_report(format!("warning: {map}: camera: {why}; the camera waits there until its way is clear"));
                }
                true
            }
        }
    }
}

/// `camera{...}`: the table form of camera (the positional form stays in mod_script_game.rs).
pub(crate) fn api_camera_table(vm: &mut Vm, lua: &Lua, t: Table) -> mlua::Result<MultiValue> {
    let known = ["from", "look_at", "seconds", "fov", "follow", "path", "orbit"];
    for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        match &k {
            mlua::Value::String(s) if known.contains(&&*s.to_str()?) => {}
            _ => return rt(format!("camera: unknown setting {} ({})", k.to_string().unwrap_or_default(), known.join(", "))),
        }
    }
    let aim = read_aim(vm, lua, t.get("look_at")?, "look_at")?;
    let seconds = t.get::<Option<f32>>("seconds")?;
    if seconds.is_some_and(|s| !(s > 0.0 && s <= 600.0)) {
        return rt("camera: seconds between 0 and 600");
    }
    let fov = t.get::<Option<f32>>("fov")?;
    if fov.is_some_and(|f| !(1.0..=170.0).contains(&f)) {
        return rt("camera: fov between 1 and 170");
    }
    let from = match t.get::<mlua::Value>("from")? {
        mlua::Value::Nil => None,
        v => Some(vm.mod_aim_point(&read_aim(vm, lua, v, "from")?)),
    };
    let look_point = vm.mod_aim_point(&aim);
    let motion = if let Some(path) = t.get::<Option<Table>>("path")? {
        let mut points: Vec<[f32; 3]> = from.into_iter().collect();
        for i in 1..=path.raw_len() {
            points.push(read_position(&path.raw_get::<Table>(i)?)?);
        }
        if points.len() < 2 || seconds.is_none() {
            return rt("camera: path = {point, point, ...} (two points or more, with from) and seconds = the time to go along it");
        }
        Motion::Path(points)
    } else if let Some(o) = t.get::<Option<Table>>("orbit")? {
        let Some(_) = seconds else { return rt("camera: orbit needs seconds = the time of the turns") };
        let start_from = from.unwrap_or([look_point[0] + 300.0, look_point[1], look_point[2] + 100.0]);
        let (dx, dy) = (start_from[0] - look_point[0], start_from[1] - look_point[1]);
        let radius = o.get::<Option<f32>>("radius")?.unwrap_or((dx * dx + dy * dy).sqrt().max(50.0));
        let height = o.get::<Option<f32>>("height")?.unwrap_or(start_from[2] - look_point[2]);
        let turns = o.get::<Option<f32>>("turns")?.unwrap_or(1.0);
        Motion::Orbit { radius, height, turns, start: dy.datan2(dx) }
    } else if let Some(v) = t.get::<Option<mlua::Value>>("follow")? {
        let actor = match read_aim(vm, lua, v, "follow")? {
            Aim::Actor(a) => a,
            Aim::At(_) => return rt("camera: follow = an actor (the camera keeps its place next to it)"),
        };
        let Some(from) = from else { return rt("camera: follow needs from = where the camera starts") };
        let p = vm.native_value(actor, Field::Location).as_vector();
        Motion::Follow { actor, offset: [from[0] - p[0], from[1] - p[1], from[2] - p[2]] }
    } else {
        match from {
            Some(p) => Motion::Still(p),
            None => return rt("camera{from = point, look_at = actor or point, ...}"),
        }
    };
    let now = vm.mod_level_time();
    let m = CameraMove { aim, motion, start: now, seconds: seconds.unwrap_or(0.0), warned: false };
    let (at, look) = vm.mod_camera_place(&m, now);
    vm.mod_script_camera(lua, at, look).map_err(mlua::Error::runtime)?;
    if let Some(f) = fov {
        for pc in vm.mod_script_player_controllers() {
            vm.set_prop(pc, "DesiredFOV", Value::Float(f));
            vm.set_prop(pc, "FovAngle", Value::Float(f));
        }
    }
    let moving = !matches!(m.motion, Motion::Still(_));
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.camera_until = seconds.map(|s| now + s);
        st.camera_move = moving.then_some(m);
    }
    vm.mod_script_log(format!("{}: camera at {{{:.0}, {:.0}, {:.0}}} ({})", vm.mod_map_name(), at[0], at[1], at[2], caller(lua)));
    ().into_lua_multi(lua)
}
