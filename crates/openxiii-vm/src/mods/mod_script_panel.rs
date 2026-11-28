//! Mods build only (feature `mods`): `panel{...}`, the comic-book insets of the levels.
//!
//! The levels show them with `XIII.CWndSFXTrigger` (up to four insets, one after the other, each
//! with its own camera: a point, a target, a field of view, a place and a size on the screen, an
//! appearing effect, a sound, an onomatopoeia picture and a text). That trigger only spaces the
//! insets out and hands each one to the game's inset window, `XIII.CWndOnTrigger`, on the first
//! HUD it finds. Here each inset is given to that same window on the HUD of every player of this
//! computer (split screen), and a player on another computer gets it through a message: the
//! window is a HUD's, it never travels by itself.

use mlua::{FromLua, FromLuaMulti, IntoLuaMulti, Lua, MultiValue, Table};
use openxiii_mods::ModValue;

use crate::mod_script::{caller, read_position, Who};
use crate::value::{ObjId, Value};
use crate::Vm;

/// The game's inset window (a HUD's).
const WINDOW_CLASS: &str = "XIII.CWndOnTrigger";
/// Class of the hidden actors that hold a camera point or a target point.
const POINT_CLASS: &str = "Engine.LineOfSightTrigger";
/// Package of the levels' onomatopoeia pictures (a `word` without a package is looked for there).
const WORD_PACKAGE: &str = "XIIIvignettes";
/// Kind of the message that gives an inset to a player on another computer.
pub(crate) const GUEST_KIND: &str = "Panel";

/// `appear =`: CWndBase's appearing effects, in the order of the game's enum.
pub(crate) const APPEAR: [&str; 12] =
    ["none", "left", "above", "stretch", "stretch_up", "zoom", "right", "below", "top_left", "top_right", "bottom_left", "bottom_right"];
/// `sound =`: the inset's sound, in the order of the game's enum.
pub(crate) const SOUND: [&str; 4] = ["none", "tip", "warning", "dialog"];

/// What an inset shows: its camera looks from `from` at the target.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Look {
    Actor(ObjId),
    At([f32; 3]),
}

/// One inset, as the levels' CWndSFXTrigger describes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Panel {
    pub from: [f32; 3],
    pub look: Look,
    pub fov: f32,
    pub seconds: f32,
    /// Seconds before the next inset of the same call.
    pub after: f32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub appear: i32,
    pub sound: i32,
    pub text: Vec<String>,
    pub word: Option<String>,
    /// The player's own character is seen in the inset (bSeeXIII).
    pub see_player: bool,
}

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

/// A point or an actor of a panel's setting.
fn read_look(vm: &Vm, lua: &Lua, v: mlua::Value, what: &str) -> mlua::Result<Look> {
    match v {
        mlua::Value::Table(t) => Ok(Look::At(read_position(&t)?)),
        mlua::Value::Nil => rt(format!("panel: {what} = an actor or a position {{x, y, z}}")),
        other => match vm.mod_script_resolve(&Who::from_lua(other, lua)?) {
            Some(a) => Ok(Look::Actor(a)),
            None => rt(format!("panel: {what}: nobody of that name")),
        },
    }
}

fn read_choice(t: &Table, key: &str, names: &[&str]) -> mlua::Result<i32> {
    match t.get::<Option<String>>(key)? {
        None => Ok(0),
        Some(s) => match names.iter().position(|n| n.eq_ignore_ascii_case(s.trim())) {
            Some(i) => Ok(i as i32),
            None => rt(format!("panel: {key} = \"{s}\": one of {}", names.join(", "))),
        },
    }
}

/// One inset's table; `rank` gives the default place (side by side, as the game's defaults).
fn read_panel(vm: &mut Vm, lua: &Lua, t: &Table, rank: usize) -> mlua::Result<Panel> {
    for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        let known = ["from", "look_at", "fov", "seconds", "after", "x", "y", "size", "appear", "sound", "text", "word", "see_player"];
        match &k {
            mlua::Value::String(s) if known.contains(&&*s.to_str()?) => {}
            _ => return rt(format!("panel: unknown setting {} ({})", k.to_string().unwrap_or_default(), known.join(", "))),
        }
    }
    let look = read_look(vm, lua, t.get("look_at")?, "look_at")?;
    let from = match t.get::<mlua::Value>("from")? {
        mlua::Value::Nil => return rt("panel: from = the camera's point {x, y, z} (or an actor)"),
        v => match read_look(vm, lua, v, "from")? {
            Look::At(p) => p,
            // An actor: the camera stands at its eyes' height.
            Look::Actor(a) => {
                let [x, y, z] = vm.native_value(a, crate::NativeField::Location).as_vector();
                [x, y, z + vm.native_prop(a, crate::NativeField::CollisionHeight).as_float() * 0.8]
            }
        },
    };
    let (w, h) = match t.get::<mlua::Value>("size")? {
        mlua::Value::Nil => (128, 128),
        mlua::Value::Table(s) => (s.raw_get::<i32>(1)?, s.raw_get::<i32>(2)?),
        v => {
            let n = i32::from_lua(v, lua)?;
            (n, n)
        }
    };
    // The insets shown at the same time share the HUD's 256x256 picture (CWndMat): four of 128.
    if !(16..=256).contains(&w) || !(16..=256).contains(&h) {
        return rt(format!("panel: size {w}x{h}: between 16 and 256 (insets shown together share a 256x256 picture: four of 128)"));
    }
    let text = match t.get::<mlua::Value>("text")? {
        mlua::Value::Nil => Vec::new(),
        mlua::Value::Table(l) => (1..=l.raw_len()).map(|i| l.raw_get::<String>(i)).collect::<mlua::Result<_>>()?,
        v => vec![String::from_lua(v, lua)?],
    };
    let word = t.get::<Option<String>>("word")?.map(|w| if w.contains('.') { w } else { format!("{WORD_PACKAGE}.{w}") });
    let seconds = t.get::<Option<f32>>("seconds")?.unwrap_or(2.0);
    let fov = t.get::<Option<f32>>("fov")?.unwrap_or(45.0);
    if !(seconds > 0.0 && seconds <= 60.0) || !(fov >= 1.0 && fov <= 170.0) {
        return rt(format!("panel: seconds {seconds} (0 to 60), fov {fov} (1 to 170)"));
    }
    Ok(Panel {
        from,
        look,
        fov,
        seconds,
        after: t.get::<Option<f32>>("after")?.unwrap_or(0.5).max(0.0),
        x: t.get::<Option<i32>>("x")?.unwrap_or(20 + 132 * (rank as i32 % 4)),
        y: t.get::<Option<i32>>("y")?.unwrap_or(20),
        w,
        h,
        appear: read_choice(t, "appear", &APPEAR)?,
        sound: read_choice(t, "sound", &SOUND)?,
        text,
        word,
        see_player: t.get::<Option<bool>>("see_player")?.unwrap_or(false),
    })
}

/// `panel{...}`: one inset (a table of settings) or several, one after the other.
pub(crate) fn api_panel(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("panel{from = point, look_at = actor or point, ...} or panel{{...}, {...}}"))?;
    let tables: Vec<Table> = if t.raw_len() > 0 { (1..=t.raw_len()).map(|i| t.raw_get::<Table>(i)).collect::<mlua::Result<_>>()? } else { vec![t] };
    let mut panels = Vec::new();
    for (i, p) in tables.iter().enumerate() {
        panels.push(read_panel(vm, lua, p, i)?);
    }
    let label = caller(lua);
    let mut st = crate::mod_script::state_mut(lua).ok_or_else(|| mlua::Error::runtime("panel: no scripts state"))?;
    let mut at = st.now;
    let mut end = at;
    for p in panels {
        end = end.max(at + p.seconds);
        let next = at + p.after;
        st.panels.push((at, p, label.clone()));
        at = next;
    }
    let total = end - st.now;
    drop(st);
    total.into_lua_multi(lua)
}

impl Vm {
    /// An inset on the HUD of every player: this computer's, and the others' through a message.
    pub(crate) fn mod_panel_show(&mut self, panel: &Panel, label: &str) {
        for pc in self.mod_script_player_controllers() {
            if self.local_player_controller(pc).is_none() {
                let line = crate::mod_script_game::guest_line(GUEST_KIND, 0, &self.mod_panel_text(panel));
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line), Value::Name("None".into())]);
                continue;
            }
            if let Err(why) = self.mod_panel_open(pc, panel) {
                let map = self.mod_map_name();
                self.mod_report(format!("warning: {map}: panel ({label}): {why}"));
            }
        }
        let map = self.mod_map_name();
        self.mod_script_log(format!("{map}: panel from {{{:.0}, {:.0}, {:.0}}} ({label})", panel.from[0], panel.from[1], panel.from[2]));
    }

    /// A hidden actor standing at `at` for `seconds` (a camera or a target point).
    pub(crate) fn mod_panel_point(&mut self, at: [f32; 3], seconds: f32) -> Result<ObjId, String> {
        let class = self.class(POINT_CLASS).ok_or("the game has no point class")?;
        let p = self
            .spawn_actor(class, Value::Object(None), None, Value::vector(at[0], at[1], at[2]), Value::rotator(0, 0, 0), Value::Object(None), true)
            .map_err(|e| e.to_string())?
            .ok_or("no room for a camera point there")?;
        self.set_prop(p, "bEnabled", Value::Bool(false));
        let mut args = [Value::vector(at[0], at[1], at[2])];
        let _ = crate::natives::call(self, "SetLocation", p, &mut args, &[true]);
        self.set_prop(p, "LifeSpan", Value::Float(seconds + 2.0));
        Ok(p)
    }

    /// Writes a property with the toml values' rules (a texture by its name, a list of texts).
    fn mod_panel_set(&mut self, a: ObjId, property: &str, value: &ModValue) -> Result<(), String> {
        let class = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class).ok_or("the inset is gone")?;
        let Some(mut props) = self.objects[a].as_mut().map(|o| std::mem::take(&mut o.props)) else { return Ok(()) };
        let r = props.with_slice(|s| self.mod_set_value(class, s, property, None, value));
        if let Some(o) = self.objects[a].as_mut() {
            o.props = props;
        }
        r.map_err(|why| format!("{property}: {why}"))
    }

    /// The game's inset window on the HUD of player controller `pc`, as CWndSFXTrigger.Timer
    /// fills it, then opened (its own Timer).
    pub(crate) fn mod_panel_open(&mut self, pc: ObjId, panel: &Panel) -> Result<ObjId, String> {
        let hud = self.prop(pc, "myHUD").as_object().ok_or("no HUD")?;
        let pawn = self.prop(pc, "Pawn").as_object().or_else(|| self.prop(pc, "ViewTarget").as_object()).ok_or("no player")?;
        let cam = self.mod_panel_point(panel.from, panel.seconds)?;
        let target = match &panel.look {
            Look::Actor(a) if self.objects.get(*a).is_some_and(|o| o.is_some()) => *a,
            Look::Actor(_) => return Err("the target is gone".into()),
            Look::At(p) => self.mod_panel_point(*p, panel.seconds)?,
        };
        let class = self.class(WINDOW_CLASS).ok_or("the game has no inset window")?;
        let at = self.prop(pawn, "Location");
        let w = self
            .spawn_actor(class, Value::Object(Some(pawn)), None, at, Value::rotator(0, 0, 0), Value::Object(Some(pawn)), true)
            .map_err(|e| e.to_string())?
            .ok_or("the inset window could not be made")?;
        self.set_prop(w, "bSeeXIII", Value::Bool(panel.see_player));
        self.set_prop(w, "bAnimatedInRealtime", Value::Bool(true));
        // CWndSFXTrigger: the camera follows the target for the inset's time (15 s at most).
        self.set_prop(w, "animationDuration", Value::Float((panel.seconds - 0.01).min(15.0)));
        self.set_prop(w, "Cam", Value::Object(Some(cam)));
        self.set_prop(w, "Tar", Value::Object(Some(target)));
        self.set_prop(w, "CWndDuration", Value::Float(panel.seconds));
        self.set_prop(w, "vScrPosX", Value::Int(panel.x));
        self.set_prop(w, "vScrPosY", Value::Int(panel.y));
        self.set_prop(w, "CWndSizeX", Value::Int(panel.w));
        self.set_prop(w, "CWndSizeY", Value::Int(panel.h));
        self.set_prop(w, "CWndFOV", Value::Float(panel.fov));
        self.set_prop(w, "MyHudForFX", Value::Object(Some(hud)));
        self.set_prop(w, "CWndAppearFX", Value::Int(panel.appear));
        self.set_prop(w, "CWndSoundType", Value::Int(panel.sound));
        if !panel.text.is_empty() {
            let texts = ModValue::Array(panel.text.iter().map(|s| ModValue::Str(s.clone())).collect());
            self.mod_panel_set(w, "OverlayMsg", &texts)?;
        }
        if let Some(word) = &panel.word {
            if let Err(why) = self.mod_panel_set(w, "OnoTexture", &ModValue::Str(word.clone())) {
                let _ = self.call_named(w, "Destroy", Vec::new());
                return Err(format!("word \"{word}\": {why}"));
            }
            if self.prop(w, "OnoTexture").as_object().is_none() && !matches!(self.prop(w, "OnoTexture"), Value::Asset(_)) {
                let _ = self.call_named(w, "Destroy", Vec::new());
                return Err(format!("word \"{word}\": no such picture (the levels' pictures are in {WORD_PACKAGE})"));
            }
            self.set_prop(w, "OnoPosX", Value::Int(panel.x));
            self.set_prop(w, "OnoPosY", Value::Int(panel.y));
            self.set_prop(w, "OnoSize", Value::Int(panel.w));
            self.set_prop(w, "OnoSizeY", Value::Int(panel.h));
        }
        self.call_named(w, "Timer", Vec::new()).map_err(|e| e.to_string())?;
        Ok(w)
    }

    /// The inset as a message for a player on another computer: the target as a point.
    fn mod_panel_text(&self, p: &Panel) -> String {
        let look = match &p.look {
            Look::At(q) => *q,
            Look::Actor(a) => self.native_value(*a, crate::NativeField::Location).as_vector(),
        };
        let mut v = serde_json::json!({
            "from": p.from, "look": look, "fov": p.fov, "seconds": p.seconds, "x": p.x, "y": p.y, "w": p.w, "h": p.h,
            "appear": p.appear, "sound": p.sound, "text": p.text, "see": p.see_player,
        });
        if let Some(w) = &p.word {
            v["word"] = serde_json::Value::String(w.clone());
        }
        v.to_string()
    }

    /// A player on another computer: the host's inset on this computer's HUD.
    pub(crate) fn mod_panel_from_host(&mut self, pc: ObjId, text: &str) -> Result<(), String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let point = |k: &str| -> Result<[f32; 3], String> {
            let a = v[k].as_array().filter(|a| a.len() == 3).ok_or(format!("{k}: no point"))?;
            Ok([a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32])
        };
        let num = |k: &str| v[k].as_f64().unwrap_or(0.0);
        let panel = Panel {
            from: point("from")?,
            look: Look::At(point("look")?),
            fov: num("fov") as f32,
            seconds: num("seconds") as f32,
            after: 0.0,
            x: num("x") as i32,
            y: num("y") as i32,
            w: num("w") as i32,
            h: num("h") as i32,
            appear: num("appear") as i32,
            sound: num("sound") as i32,
            text: v["text"].as_array().map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            word: v["word"].as_str().map(str::to_string),
            see_player: v["see"].as_bool().unwrap_or(false),
        };
        self.mod_panel_open(pc, &panel).map(|_| ())
    }
}
