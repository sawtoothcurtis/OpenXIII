//! Mods build only (feature `mods`): checkpoint(), focus{}, slow_motion() and fade{}.
//!
//! Each one drives what the game already has:
//! - checkpoint: the levels' save point, `XIII.XIIISaveGameTrigger` (its description, and the
//!   map's teleporter where the game takes up again), triggered by the player;
//! - focus: the "sixth sense" inset of the levels' CWndFocusTrigger, which hands its actor to the
//!   HUD (`XIIIBaseHud.AddHudCartoonFocus`): here on every player's HUD;
//! - slow_motion: the level's time scale (`LevelInfo.TimeDilation`, which the game sends to
//!   network guests itself);
//! - fade: the view's constant glow (`PlayerController.ConstantGlowScale` / `ConstantGlowFog`,
//!   which the engine's view flash eases towards, as for a head under water).

use mlua::{FromLuaMulti, IntoLuaMulti, Lua, MultiValue, Table};

use crate::mod_script::{caller, read_position, Who};
use crate::mod_script_game::guest_line;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Kinds of the messages that give a focus or a fade to a player on another computer.
pub(crate) const FOCUS_KIND: &str = "Focus";
pub(crate) const FADE_KIND: &str = "Fade";

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

/// A fade under way: the glow goes from `from` to `to` over `seconds`, in `color` (0 to 1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fade {
    pub from: f32,
    pub to: f32,
    pub color: [f32; 3],
    pub start: f32,
    pub seconds: f32,
    /// fade{player =}: only that player's view (a pawn); None: every player's.
    pub only: Option<ObjId>,
}

/// `checkpoint("Description"[, "Teleporter"])`.
pub(crate) fn api_checkpoint(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (text, teleporter) =
        <(String, Option<String>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("checkpoint(\"Description\"[, \"teleporter of the map\"])"))?;
    let Some(pawn) = vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied()) else { return rt("checkpoint: no player") };
    if let Some(t) = &teleporter {
        let found = vm.actors.iter().any(|&a| {
            vm.objects.get(a).is_some_and(|o| o.is_some())
                && vm.is_a(a, "Teleporter")
                && (vm.mod_actor_name(a).eq_ignore_ascii_case(t) || vm.prop(a, "Tag").to_string().trim_matches('\'').eq_ignore_ascii_case(t))
        });
        if !found {
            return rt(format!("checkpoint: the map has no teleporter {t} (the catalog lists them; without one the game takes up at the map's start)"));
        }
    }
    let class = vm.class("XIII.XIIISaveGameTrigger").ok_or_else(|| mlua::Error::runtime("checkpoint: the game has no save point"))?;
    let at = vm.prop(pawn, "Location");
    let s = vm
        .spawn_actor(class, Value::Object(None), None, at, Value::rotator(0, 0, 0), Value::Object(None), true)
        .map_err(|e| mlua::Error::runtime(format!("checkpoint: {e}")))?
        .ok_or_else(|| mlua::Error::runtime("checkpoint: no room for a save point"))?;
    vm.set_prop(s, "SaveDescription", Value::Str(text.clone()));
    if let Some(t) = &teleporter {
        vm.set_prop(s, "TeleporterName", Value::Str(t.clone()));
    }
    let n = vm.mod_level_time().to_bits();
    vm.set_prop(s, "Tag", Value::Name(crate::value::NameStruct::from_time(&format!("ModCheckpoint{n:x}"))));
    vm.call_named(s, "Trigger", vec![Value::Object(Some(pawn)), Value::Object(Some(pawn))]).map_err(|e| mlua::Error::runtime(format!("checkpoint: {e}")))?;
    vm.mod_script_log(format!("{}: checkpoint \"{text}\" ({})", vm.mod_map_name(), caller(lua)));
    true.into_lua_multi(lua)
}

/// `focus{on =, seconds =, zoom =, zoom_fov =, zoom_distance =, vignette =, danger =}`.
pub(crate) fn api_focus(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("focus{on = actor or point, seconds = 2, ...}"))?;
    let known = ["on", "seconds", "zoom", "zoom_fov", "zoom_distance", "vignette", "danger"];
    for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        match &k {
            mlua::Value::String(s) if known.contains(&&*s.to_str()?) => {}
            _ => return rt(format!("focus: unknown setting {} ({})", k.to_string().unwrap_or_default(), known.join(", "))),
        }
    }
    let on = match t.get::<mlua::Value>("on")? {
        mlua::Value::Table(p) => {
            let p = read_position(&p)?;
            vm.mod_panel_point(p, t.get::<Option<f32>>("seconds")?.unwrap_or(2.0)).map_err(mlua::Error::runtime)?
        }
        mlua::Value::Nil => return rt("focus: on = an actor or a position"),
        v => vm
            .mod_script_resolve(&Who::from_lua_multi(MultiValue::from_vec(vec![v]), lua)?)
            .ok_or_else(|| mlua::Error::runtime("focus: nobody of that name"))?,
    };
    let f = FocusSettings {
        seconds: t.get::<Option<f32>>("seconds")?.unwrap_or(2.0),
        zoom: t.get::<Option<bool>>("zoom")?.unwrap_or(false),
        zoom_fov: t.get::<Option<f32>>("zoom_fov")?.unwrap_or(30.0),
        zoom_distance: t.get::<Option<f32>>("zoom_distance")?.unwrap_or(0.0),
        vignette: t.get::<Option<bool>>("vignette")?.unwrap_or(true),
        danger: t.get::<Option<bool>>("danger")?.unwrap_or(false),
    };
    if !(f.seconds > 0.0 && f.seconds <= 60.0) {
        return rt("focus: seconds between 0 and 60");
    }
    for pc in vm.mod_script_player_controllers() {
        if vm.local_player_controller(pc).is_none() {
            let at = vm.native_value(on, Field::Location).as_vector();
            let text =
                serde_json::json!({"at": at, "s": f.seconds, "z": f.zoom, "zf": f.zoom_fov, "zd": f.zoom_distance, "v": f.vignette, "d": f.danger}).to_string();
            let _ = vm.call_named(pc, "ClientMessage", vec![Value::Str(guest_line(FOCUS_KIND, 0, &text)), Value::Name("None".into())]);
            continue;
        }
        if let Err(why) = vm.mod_focus_open(pc, on, &f) {
            let map = vm.mod_map_name();
            vm.mod_report(format!("warning: {map}: focus: {why}"));
        }
    }
    vm.mod_script_log(format!("{}: focus on {} ({})", vm.mod_map_name(), vm.mod_actor_name(on), caller(lua)));
    true.into_lua_multi(lua)
}

/// What a focus shows (CWndFocusTrigger's settings).
pub(crate) struct FocusSettings {
    pub seconds: f32,
    pub zoom: bool,
    pub zoom_fov: f32,
    pub zoom_distance: f32,
    pub vignette: bool,
    pub danger: bool,
}

/// `slow_motion(factor[, seconds])`: the level's time runs `factor` times as fast.
pub(crate) fn api_slow_motion(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (factor, seconds) =
        <(f32, Option<f32>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("slow_motion(factor[, seconds]): 0.5 is half speed, 1 normal"))?;
    if !(0.1..=2.0).contains(&factor) {
        return rt("slow_motion: a factor between 0.1 and 2 (1 is normal speed)");
    }
    let Some(level) = vm.level else { return rt("slow_motion: no level") };
    vm.set_prop(level, "TimeDilation", Value::Float(factor));
    let now = vm.mod_level_time();
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        // Level time runs at the new speed: seconds of the player's time are factor × as long.
        st.slow_until = seconds.map(|s| now + s * factor);
    }
    vm.mod_script_log(format!("{}: slow motion {factor} ({})", vm.mod_map_name(), caller(lua)));
    true.into_lua_multi(lua)
}

/// `fade{to = "black" or {r, g, b}, seconds =}` and `fade{back = true, seconds =}`.
pub(crate) fn api_fade(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("fade{to = \"black\", seconds = 1} or fade{back = true}"))?;
    let back = t.get::<Option<bool>>("back")?.unwrap_or(false);
    let color = match t.get::<mlua::Value>("to")? {
        mlua::Value::Nil if back => [0.0; 3],
        mlua::Value::Nil => [0.0; 3],
        mlua::Value::String(s) => match s.to_str()?.to_ascii_lowercase().as_str() {
            "black" => [0.0; 3],
            "white" => [1.0; 3],
            "red" => [1.0, 0.0, 0.0],
            other => return rt(format!("fade: to = \"{other}\": black, white, red or {{r, g, b}} (0 to 255)")),
        },
        mlua::Value::Table(c) => {
            let k = |i: usize| -> mlua::Result<f32> { Ok((c.raw_get::<f32>(i)? / 255.0).clamp(0.0, 1.0)) };
            [k(1)?, k(2)?, k(3)?]
        }
        _ => return rt("fade: to = \"black\" or {r, g, b}"),
    };
    let seconds = t.get::<Option<f32>>("seconds")?.unwrap_or(1.0).clamp(0.0, 30.0);
    let only = crate::mod_script_game::only_player(vm, lua, t.get::<Option<mlua::Value>>("player")?, "fade")?;
    let for_this = |vm: &Vm, pc: ObjId| only.is_none_or(|p| vm.native_prop(pc, Field::Pawn).as_object() == Some(p));
    let pc = vm.mod_script_player_controllers().into_iter().find(|&pc| vm.local_player_controller(pc).is_some() && for_this(vm, pc));
    let from = pc.map_or(0.0, |pc| vm.prop(pc, "ConstantGlowScale").as_float());
    let to = if back { 0.0 } else { -1.0 };
    let old_color = pc.map_or([0.0; 3], |pc| vm.prop(pc, "ConstantGlowFog").as_vector());
    let fade = Fade { from, to, color: if back { old_color } else { color }, start: vm.mod_level_time(), seconds, only };
    for pc in vm.mod_script_player_controllers() {
        if vm.local_player_controller(pc).is_none() && for_this(vm, pc) {
            // Another computer: the end of the fade at once (its own view flash eases it).
            let text = serde_json::json!({"to": to, "c": fade.color}).to_string();
            let _ = vm.call_named(pc, "ClientMessage", vec![Value::Str(guest_line(FADE_KIND, 0, &text)), Value::Name("None".into())]);
        }
    }
    if let Some(mut st) = crate::mod_script::state_mut(lua) {
        st.fade = Some(fade);
    }
    vm.mod_script_log(format!("{}: fade {} ({})", vm.mod_map_name(), if back { "back" } else { "out" }, caller(lua)));
    true.into_lua_multi(lua)
}

impl Vm {
    /// The levels' focus inset on the HUD of player controller `pc`.
    pub(crate) fn mod_focus_open(&mut self, pc: ObjId, on: ObjId, f: &FocusSettings) -> Result<(), String> {
        let hud = self.prop(pc, "myHUD").as_object().ok_or("no HUD")?;
        let args = vec![
            Value::Object(Some(on)),
            Value::Int(0),
            Value::Bool(f.vignette),
            Value::Float(f.seconds),
            Value::Bool(f.zoom),
            Value::Float(f.zoom_fov),
            Value::Float(f.zoom_distance),
            Value::Bool(f.danger),
        ];
        self.call_named(hud, "AddHudCartoonFocus", args).map(|_| ()).map_err(|e| e.to_string())
    }

    /// The glow of every local player's view (a fade's frame), or of player `only` (a pawn).
    pub(crate) fn mod_set_glow(&mut self, scale: f32, color: [f32; 3], only: Option<ObjId>) {
        for pc in self.mod_script_player_controllers() {
            if self.local_player_controller(pc).is_some() && only.is_none_or(|p| self.native_prop(pc, Field::Pawn).as_object() == Some(p)) {
                self.set_prop(pc, "ConstantGlowScale", Value::Float(scale));
                self.set_prop(pc, "ConstantGlowFog", Value::vector(color[0], color[1], color[2]));
            }
        }
    }

    /// Each scripts' frame: a fade goes on (true while it lasts).
    pub(crate) fn mod_fade_step(&mut self, f: &Fade, now: f32) -> bool {
        let k = if f.seconds > 0.0 { ((now - f.start) / f.seconds).clamp(0.0, 1.0) } else { 1.0 };
        self.mod_set_glow(f.from + (f.to - f.from) * k, f.color, f.only);
        k < 1.0
    }

    /// A player on another computer: the host's focus or fade on this computer.
    pub(crate) fn mod_effect_from_host(&mut self, pc: ObjId, kind: &str, text: &str) -> Result<(), String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let vec3 = |k: &str| -> [f32; 3] {
            let a = v[k].as_array().cloned().unwrap_or_default();
            [0, 1, 2].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32)
        };
        if kind == FADE_KIND {
            let to = v["to"].as_f64().unwrap_or(0.0) as f32;
            self.mod_set_glow(to, vec3("c"), None);
            return Ok(());
        }
        let f = FocusSettings {
            seconds: v["s"].as_f64().unwrap_or(2.0) as f32,
            zoom: v["z"].as_bool().unwrap_or(false),
            zoom_fov: v["zf"].as_f64().unwrap_or(30.0) as f32,
            zoom_distance: v["zd"].as_f64().unwrap_or(0.0) as f32,
            vignette: v["v"].as_bool().unwrap_or(true),
            danger: v["d"].as_bool().unwrap_or(false),
        };
        let on = self.mod_panel_point(vec3("at"), f.seconds)?;
        self.mod_focus_open(pc, on, &f)
    }
}
