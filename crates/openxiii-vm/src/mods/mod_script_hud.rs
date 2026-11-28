//! Mods build only (feature `mods`): what scripts keep on the screen (`hud_text`, `hud_counter`,
//! `hud_timer`, `hud_bar`, `hud_image`, `hud_line`), and the loading screen between two maps.
//!
//! - **Items.** Each item has an id, a place (x, y from 0 to 1 of the player's view), and stays
//!   until `hud_remove`; `hud_update` changes it. An item given `player =` is drawn on that
//!   player's view only (split screen), the others on every view. They are drawn by the HUD's
//!   canvas after the game's own HUD (`hud_end`), in the order they were made, and belong to the
//!   map: a new map starts without them.
//! - **Network.** The host's items go to each guest as the scripts' other messages do (a reliable
//!   `ClientMessage`, mod_script_game.rs): made, changed or removed there too.
//! - **Loading screen.** When a map change begins, the scripts' `on("map_end")` then
//!   `on("loading")` handlers run; the items they make are the loading screen's, drawn on a black
//!   640×480 picture scaled to the window (as the console version's recap bubbles were). With
//!   `hold_start()` the next level does not start before `release_start()`: the window keeps the
//!   picture while the level loads, runs `on("loaded")` once it is ready and `on("loading_key")`
//!   at each press. Without `hold_start()` the loading screen is the game's.
//!
//! Nothing is drawn, sent or kept without a script that makes items.

use mlua::{FromLua, FromLuaMulti, IntoLuaMulti, Lua, MultiValue, Table};

use crate::canvas::{ansi, clipped_str_len, HudDraw};
use crate::mod_script::Who;
use crate::mod_script_game::guest_line;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Kind of the messages that carry the host's screen items to a network guest.
pub(crate) const HUD_KIND: &str = "Hud";
/// The loading screen's virtual size (the console layout's).
pub const LOADING_W: f32 = 640.0;
pub const LOADING_H: f32 = 480.0;
/// The fonts of `size =`.
const FONTS: &[(&str, &str)] = &[("small", "XIIIFonts.PoliceF16"), ("medium", "XIIIFonts.PoliceF20")];

fn rt<T>(text: impl Into<String>) -> mlua::Result<T> {
    Err(mlua::Error::runtime(text.into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    Counter,
    Timer,
    Bar,
    Image,
    Line,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Counter => "counter",
            Kind::Timer => "timer",
            Kind::Bar => "bar",
            Kind::Image => "image",
            Kind::Line => "line",
        }
    }

    fn from_name(s: &str) -> Option<Kind> {
        [Kind::Text, Kind::Counter, Kind::Timer, Kind::Bar, Kind::Image, Kind::Line].into_iter().find(|k| k.name() == s)
    }

    /// The settings it takes.
    fn keys(self) -> &'static [&'static str] {
        match self {
            Kind::Text => &["id", "text", "x", "y", "size", "font", "colour", "color", "align", "player"],
            Kind::Counter => &["id", "label", "value", "x", "y", "size", "font", "colour", "color", "align", "player"],
            Kind::Timer => &["id", "label", "seconds", "x", "y", "size", "font", "colour", "color", "align", "player"],
            Kind::Bar => &["id", "x", "y", "w", "h", "value", "colour", "color", "back", "player"],
            Kind::Image => &["id", "texture", "x", "y", "w", "h", "colour", "color", "player"],
            Kind::Line => &["id", "from", "to", "colour", "color", "player"],
        }
    }
}

/// One item on the screen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub id: String,
    pub kind: Kind,
    /// Place (0 to 1 of the view); a line's start.
    pub x: f64,
    pub y: f64,
    /// Size of a bar or an image (0 to 1 of the view); a line's end.
    pub w: f64,
    pub h: f64,
    /// A text, or the label of a counter or a timer.
    pub text: String,
    /// A counter's value, a bar's fill (0 to 1).
    pub value: f64,
    /// A timer's end, in level time.
    pub ends_at: f32,
    pub font: String,
    /// R, G, B, A.
    pub colour: [u8; 4],
    pub back: Option<[u8; 4]>,
    /// 0 left, 1 centre, 2 right (the text's x is that point of it).
    pub align: u8,
    pub texture: String,
    /// Only on this player's view (a pawn).
    pub player: Option<ObjId>,
}

impl Item {
    fn new(id: String, kind: Kind) -> Item {
        Item {
            id,
            kind,
            x: 0.0,
            y: 0.0,
            w: 0.1,
            h: 0.02,
            text: String::new(),
            value: if kind == Kind::Bar { 1.0 } else { 0.0 },
            ends_at: 0.0,
            font: FONTS[0].1.to_string(),
            colour: [255; 4],
            back: None,
            align: 0,
            texture: String::new(),
            player: None,
        }
    }

    /// The text it shows (`now`: level time, for a timer).
    fn shown(&self, now: f32) -> String {
        match self.kind {
            Kind::Text => self.text.clone(),
            Kind::Counter => format!("{}{}", self.text, number(self.value)),
            Kind::Timer => {
                let left = (self.ends_at - now).max(0.0).ceil() as i64;
                format!("{}{}:{:02}", self.text, left / 60, left % 60)
            }
            _ => String::new(),
        }
    }
}

/// A number as a counter shows it: whole numbers without a point.
fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// The loading screen between two maps.
#[derive(Clone, Debug, Default)]
pub struct Loading {
    /// A map change is under way (from `mod_loading_begin` to `mod_loading_end`).
    pub(crate) active: bool,
    /// A loading handler is running: the items made are the loading screen's.
    pub(crate) in_handler: bool,
    pub(crate) held: bool,
    pub(crate) released: bool,
    pub(crate) items: Vec<Item>,
}

/// The scripts' screen items for the current VM.
#[derive(Clone, Debug, Default)]
pub struct Hud {
    pub(crate) items: Vec<Item>,
    pub(crate) loading: Loading,
    next_id: u64,
}

impl Hud {
    fn list(&mut self) -> &mut Vec<Item> {
        if self.loading.in_handler {
            &mut self.loading.items
        } else {
            &mut self.items
        }
    }
}

/// `{r, g, b[, a]}` (0 to 255) or a colour's name.
fn read_colour(v: mlua::Value, what: &str) -> mlua::Result<[u8; 4]> {
    match v {
        mlua::Value::String(s) => match s.to_str()?.to_ascii_lowercase().as_str() {
            "white" => Ok([255, 255, 255, 255]),
            "black" => Ok([0, 0, 0, 255]),
            "grey" | "gray" => Ok([0xC4, 0xC4, 0xC4, 255]),
            "red" => Ok([255, 40, 40, 255]),
            "green" => Ok([40, 220, 40, 255]),
            "yellow" => Ok([255, 220, 0, 255]),
            "blue" => Ok([60, 120, 255, 255]),
            other => rt(format!("{what}: colour \"{other}\": white, black, grey, red, green, yellow, blue or {{r, g, b[, a]}}")),
        },
        mlua::Value::Table(t) => {
            let c = |i: usize, d: f64| -> mlua::Result<u8> { Ok(t.raw_get::<Option<f64>>(i)?.unwrap_or(d).clamp(0.0, 255.0).round() as u8) };
            Ok([c(1, 255.0)?, c(2, 255.0)?, c(3, 255.0)?, c(4, 255.0)?])
        }
        _ => rt(format!("{what}: a colour is a name or {{r, g, b[, a]}}")),
    }
}

/// `{x, y}` from 0 to 1.
fn read_point(v: mlua::Value, what: &str) -> mlua::Result<(f64, f64)> {
    match v {
        mlua::Value::Table(t) => Ok((t.raw_get::<f64>(1)?, t.raw_get::<f64>(2)?)),
        _ => rt(format!("{what}: a point is {{x, y}} (0 to 1 of the screen)")),
    }
}

/// The settings of table `t` written on `item` (only those given).
fn read_settings(vm: &Vm, lua: &Lua, item: &mut Item, t: &Table, what: &str) -> mlua::Result<()> {
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        let (k, v) = pair?;
        let mlua::Value::String(k) = k else { return rt(format!("{what}: settings are named (x =, y =, ...)")) };
        let k = k.to_str()?.to_string();
        if !item.kind.keys().contains(&k.as_str()) {
            return rt(format!("{what}: unknown setting {k} ({})", item.kind.keys().join(", ")));
        }
        let num = |v: &mlua::Value| -> mlua::Result<f64> { f64::from_lua(v.clone(), lua).map_err(|_| mlua::Error::runtime(format!("{what}: {k} is a number"))) };
        match k.as_str() {
            "id" => {}
            "text" | "label" => item.text = String::from_lua(v, lua)?,
            "x" => item.x = num(&v)?,
            "y" => item.y = num(&v)?,
            "w" => item.w = num(&v)?,
            "h" => item.h = num(&v)?,
            "value" => item.value = num(&v)?,
            "seconds" => item.ends_at = vm.mod_level_time() + num(&v)?.max(0.0) as f32,
            "size" => {
                let s = String::from_lua(v, lua)?.to_ascii_lowercase();
                item.font = FONTS.iter().find(|(n, _)| *n == s).map(|(_, f)| f.to_string()).ok_or_else(|| mlua::Error::runtime(format!("{what}: size \"{s}\": small or medium (or font = \"Package.Font\")")))?;
            }
            "font" => item.font = String::from_lua(v, lua)?,
            "colour" | "color" => item.colour = read_colour(v, what)?,
            "back" => item.back = if v.is_nil() { None } else { Some(read_colour(v, what)?) },
            "align" => {
                item.align = match String::from_lua(v, lua)?.to_ascii_lowercase().as_str() {
                    "left" => 0,
                    "centre" | "center" => 1,
                    "right" => 2,
                    other => return rt(format!("{what}: align \"{other}\": left, centre or right")),
                }
            }
            "texture" => item.texture = String::from_lua(v, lua)?,
            "from" => (item.x, item.y) = read_point(v, what)?,
            "to" => (item.w, item.h) = read_point(v, what)?,
            "player" => {
                item.player = if v.is_nil() { None } else { Some(vm.mod_script_resolve(&Who::from_lua(v, lua)?).ok_or_else(|| mlua::Error::runtime(format!("{what}: player = nobody of that name")))?) }
            }
            _ => {}
        }
    }
    Ok(())
}

fn add(vm: &mut Vm, lua: &Lua, args: MultiValue, kind: Kind) -> mlua::Result<MultiValue> {
    let what = format!("hud_{}", kind.name());
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime(format!("{what}{{...}}: a table of settings")))?;
    let id = match t.raw_get::<Option<String>>("id")? {
        Some(id) => id,
        None => {
            vm.mods.hud.next_id += 1;
            format!("hud{}", vm.mods.hud.next_id)
        }
    };
    let mut item = Item::new(id.clone(), kind);
    read_settings(vm, lua, &mut item, &t, &what)?;
    if kind == Kind::Image && item.texture.is_empty() {
        return rt("hud_image: give texture = \"Package.Texture\"");
    }
    let list = vm.mods.hud.list();
    match list.iter_mut().find(|i| i.id == id) {
        Some(old) => *old = item.clone(),
        None => list.push(item.clone()),
    }
    if !vm.mods.hud.loading.in_handler {
        vm.mod_hud_to_guests(Some(&item), &id);
    }
    id.into_lua_multi(lua)
}

pub(crate) fn api_hud_text(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Text)
}
pub(crate) fn api_hud_counter(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Counter)
}
pub(crate) fn api_hud_timer(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Timer)
}
pub(crate) fn api_hud_bar(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Bar)
}
pub(crate) fn api_hud_image(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Image)
}
pub(crate) fn api_hud_line(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add(vm, lua, args, Kind::Line)
}

pub(crate) fn api_hud_update(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (id, t) = <(String, Table)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("hud_update(id, {settings})"))?;
    let Some(mut item) = vm.mods.hud.list().iter().find(|i| i.id == id).cloned() else { return false.into_lua_multi(lua) };
    read_settings(vm, lua, &mut item, &t, "hud_update")?;
    if let Some(old) = vm.mods.hud.list().iter_mut().find(|i| i.id == id).filter(|_| true) {
        *old = item.clone();
    }
    if !vm.mods.hud.loading.in_handler {
        vm.mod_hud_to_guests(Some(&item), &id);
    }
    true.into_lua_multi(lua)
}

pub(crate) fn api_hud_remove(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let id = Option::<String>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("hud_remove([id])"))?;
    let list = vm.mods.hud.list();
    let before = list.len();
    match &id {
        Some(id) => list.retain(|i| &i.id != id),
        None => list.clear(),
    }
    let removed = before - vm.mods.hud.list().len();
    if removed > 0 && !vm.mods.hud.loading.in_handler {
        vm.mod_hud_to_guests(None, id.as_deref().unwrap_or(""));
    }
    (removed > 0).into_lua_multi(lua)
}

/// `text_size(text[, size or font])`: the width and height of a line of text, as parts of a
/// 640×480 screen.
pub(crate) fn api_text_size(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (text, font) = <(String, Option<String>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("text_size(text[, size or font])"))?;
    let font = match font {
        None => FONTS[0].1.to_string(),
        Some(f) => FONTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(&f)).map_or(f, |(_, p)| p.to_string()),
    };
    let (w, h) = vm.mod_text_size(&font, &text);
    let w = w;
    (w as f64 / LOADING_W as f64, h as f64 / LOADING_H as f64).into_lua_multi(lua)
}

/// `text(section, key[, file])`: a text of the game's language files, in the player's language
/// (file "Engine" by default), or nil.
pub(crate) fn api_text(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (section, key, file) = <(String, String, Option<String>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("text(section, key[, file])"))?;
    if vm.config.is_none() || key.is_empty() {
        return mlua::Value::Nil.into_lua_multi(lua);
    }
    let t = vm.localize(&section, &key, file.as_deref().unwrap_or("Engine"), true);
    let t = t;
    (!t.is_empty()).then_some(t).into_lua_multi(lua)
}

pub(crate) fn api_hold_start(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    if !vm.mods.hud.loading.in_handler {
        return rt("hold_start: only in an on(\"loading\") handler");
    }
    vm.mods.hud.loading.held = true;
    ().into_lua_multi(lua)
}

pub(crate) fn api_release_start(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    let l = &mut vm.mods.hud.loading;
    let was = l.active && l.held && !l.released;
    l.released = true;
    was.into_lua_multi(lua)
}

impl Vm {
    /// A line of `text` in `font`: its width and height in pixels.
    pub(crate) fn mod_text_size(&mut self, font: &str, text: &str) -> (i32, i32) {
        let Some(f) = self.font(font) else { return (0, 0) };
        let space_x = self.canvas.filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some())).map_or(0.0, |c| self.propr(c, "SpaceX").as_float());
        clipped_str_len(&f, space_x, 1.0, 1.0, &ansi(text))
    }

    /// Host: an item made or changed (`item`), or removed (`None`: `id`, empty for all), for the
    /// guests whose view shows it.
    fn mod_hud_to_guests(&mut self, item: Option<&Item>, id: &str) {
        if !self.mod_net_host() {
            return;
        }
        let now = self.mod_level_time();
        for pc in self.mod_script_player_controllers() {
            if self.local_player_controller(pc).is_some() {
                continue;
            }
            let pawn = self.native_prop(pc, Field::Pawn).as_object();
            let text = match item {
                Some(i) if i.player.is_some_and(|p| Some(p) != pawn) => continue,
                Some(i) => item_json(i, now),
                None => serde_json::json!({"op": "del", "id": id}).to_string(),
            };
            let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(guest_line(HUD_KIND, 0, &text)), Value::Name("None".into())]);
        }
    }

    /// Network guest: the host's item, here.
    pub(crate) fn mod_hud_from_host(&mut self, text: &str) -> Result<(), String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let id = v["id"].as_str().unwrap_or_default().to_string();
        if v["op"] == "del" {
            if id.is_empty() {
                self.mods.hud.items.clear();
            } else {
                self.mods.hud.items.retain(|i| i.id != id);
            }
            return Ok(());
        }
        let kind = Kind::from_name(v["k"].as_str().unwrap_or_default()).ok_or("unknown item")?;
        let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
        let c = |k: &str| -> Option<[u8; 4]> {
            let a = v[k].as_array()?;
            Some([0, 1, 2, 3].map(|i| a.get(i).and_then(|x| x.as_u64()).unwrap_or(255) as u8))
        };
        let mut item = Item::new(id.clone(), kind);
        (item.x, item.y, item.w, item.h, item.value) = (f("x"), f("y"), f("w"), f("h"), f("v"));
        item.text = v["t"].as_str().unwrap_or_default().to_string();
        item.font = v["f"].as_str().unwrap_or(FONTS[0].1).to_string();
        item.texture = v["tx"].as_str().unwrap_or_default().to_string();
        item.align = v["a"].as_u64().unwrap_or(0) as u8;
        item.colour = c("c").unwrap_or([255; 4]);
        item.back = c("b");
        item.ends_at = self.mod_level_time() + f("left") as f32;
        match self.mods.hud.items.iter_mut().find(|i| i.id == id) {
            Some(old) => *old = item,
            None => self.mods.hud.items.push(item),
        }
        Ok(())
    }

    /// End of HUD.PostRender for player controller `pc` (canvas.rs hud_end): the scripts' items
    /// of its view.
    pub(crate) fn mod_hud_draw(&mut self, pc: ObjId, canvas: ObjId) {
        let pawn = self.native_prop(pc, Field::Pawn).as_object();
        let (ox, oy) = (self.cf(canvas, "OrgX"), self.cf(canvas, "OrgY"));
        let (cx, cy) = (self.cf(canvas, "ClipX"), self.cf(canvas, "ClipY"));
        let now = self.mod_level_time();
        let items: Vec<Item> = self.mods.hud.items.iter().filter(|i| i.player.is_none_or(|p| Some(p) == pawn)).filter(|_| true).cloned().collect();
        for i in &items {
            self.mod_hud_item(canvas, i, now, (ox, oy), (cx, cy), false);
        }
    }

    /// One item drawn on a view of size `clip` at `org` (`exact`: the loading screen's places, on
    /// whole pixels of its 640×480 picture, text without smoothing).
    fn mod_hud_item(&mut self, canvas: ObjId, i: &Item, now: f32, org: (f32, f32), clip: (f32, f32), exact: bool) {
        let px = |v: f64, size: f32| -> f32 {
            let p = v * size as f64;
            if exact { p.round() as f32 } else { p as f32 }
        };
        let colour = bgra(i.colour);
        match i.kind {
            Kind::Text | Kind::Counter | Kind::Timer => {
                let bytes = ansi(&i.shown(now));
                let w = if i.align == 0 { 0 } else { self.font(&i.font).map_or(0, |f| clipped_str_len(&f, self.cf(canvas, "SpaceX"), 1.0, 1.0, &bytes).0) };
                let x = px(i.x, clip.0) as i32 - w * i.align as i32 / 2;
                let y = px(i.y, clip.1) as i32;
                if exact {
                    self.canvas_state.point_filter = true;
                }
                self.draw_string(canvas, &i.font, x, y, &bytes, colour, false, false);
                if exact {
                    self.canvas_state.point_filter = false;
                }
            }
            Kind::Bar => {
                let (x, y) = (org.0 + px(i.x, clip.0), org.1 + px(i.y, clip.1));
                let (w, h) = (px(i.w, clip.0), px(i.h, clip.1));
                if let Some(b) = i.back {
                    self.hud.push(Vm::rectangle_free(x, y, x + w, y + h, bgra(b)));
                }
                let fill = w * i.value.clamp(0.0, 1.0) as f32;
                if fill > 0.0 {
                    self.hud.push(Vm::rectangle_free(x, y, x + fill, y + h, colour));
                }
            }
            Kind::Image => {
                let (x, y) = (org.0 + px(i.x, clip.0), org.1 + px(i.y, clip.1));
                let (w, h) = (px(i.w, clip.0), px(i.h, clip.1));
                if let Some(t) = self.texture_free(&i.texture, x, y, x + w, y + h, colour) {
                    self.hud.push(t);
                }
            }
            Kind::Line => {
                let (x1, y1) = (org.0 + px(i.x, clip.0), org.1 + px(i.y, clip.1));
                let (x2, y2) = (org.0 + px(i.w, clip.0), org.1 + px(i.h, clip.1));
                self.util_line(x1, y1, x2, y2, colour, 1);
            }
        }
    }

    /// The loading screen's frame: its items on a 640×480 picture (cleared to black by the
    /// renderer), scaled to a `width` × `height` window as Canvas primitives in `self.hud`.
    pub fn mod_loading_frame(&mut self, width: u32, height: u32) {
        self.hud.clear();
        self.viewport = (LOADING_W as i32, LOADING_H as i32);
        let canvas = match self.canvas {
            Some(c) if self.objects[c].is_some() => c,
            _ => match self.class("Canvas") {
                Some(class) => {
                    let c = self.spawn(class);
                    self.canvas = Some(c);
                    c
                }
                None => return,
            },
        };
        self.canvas_update(canvas);
        let now = self.mod_level_time();
        let items = self.mods.hud.loading.items.clone();
        for i in &items {
            self.mod_hud_item(canvas, i, now, (0.0, 0.0), (LOADING_W, LOADING_H), true);
        }
        self.viewport = (width as i32, height as i32);
        self.canvas_update(canvas);
        let (s, ox) = screen_transform(width, height);
        let hud = std::mem::take(&mut self.hud);
        self.hud = scale_hud(hud, s, ox, crate::gui::enlarged_text_bicubic(self.engine.client.session_graphics));
    }

    /// One line of text in a comic bubble in the middle of a black screen (the menus' pictures
    /// being made on a first start, Android and Linux), drawn as a loading screen.
    pub fn loading_text_frame(&mut self, text: &str, width: u32, height: u32) {
        let (w, _) = self.mod_text_size(FONTS[0].1, text);
        let h = 22;
        // A bubble centred on (320, 240), its tail at 50 pixels from its left edge.
        let (x, y) = (320 - w / 2, 240 - h / 2);
        let (r, b) = (x + w + 40, y + h + 20);
        let (a, c) = (x + 50, x + 68);
        let lines = [[x, y, r, y], [x, y, x, b], [r, y, r, b], [x, b, a, b], [c, b, r, b], [a, b, c, b + 30], [c, b + 30, c, b]];
        let saved = std::mem::take(&mut self.mods.hud.loading.items);
        let f = |v: i32, size: f32| v as f64 / size as f64;
        for (k, [x1, y1, x2, y2]) in lines.into_iter().enumerate() {
            let mut i = Item::new(format!("l{k}"), Kind::Line);
            (i.x, i.y, i.w, i.h) = (f(x1, LOADING_W), f(y1, LOADING_H), f(x2, LOADING_W), f(y2, LOADING_H));
            i.colour = [0xC4, 0xC4, 0xC4, 0xFF];
            self.mods.hud.loading.items.push(i);
        }
        let mut t = Item::new("text".into(), Kind::Text);
        (t.x, t.y, t.text) = (f(x + 20, LOADING_W), f(y + 8, LOADING_H), text.to_string());
        self.mods.hud.loading.items.push(t);
        self.mod_loading_frame(width, height);
        self.mods.hud.loading.items = saved;
    }

    /// A map change to `to` begins (the window, before the next map loads; `failed`: after a lost
    /// mission): the scripts' `map_end` and `loading` handlers run. True when one of them holds the
    /// start (`hold_start`): the window then shows `mod_loading_frame` until `mod_loading_released`.
    pub fn mod_loading_begin(&mut self, to: &str, failed: bool) -> bool {
        self.mods.hud.loading = Loading { active: true, ..Default::default() };
        if self.mods.scripts.is_none() {
            return false;
        }
        let from = self.mod_map_name();
        self.mod_scripts_fire_event("map_end", vec![from.to_string()], false);
        self.mod_scripts_fire_event("loading", vec![from.to_string(), to.to_string(), if failed { "failed".into() } else { String::new() }], true);
        self.mods.hud.loading.held
    }

    /// The next level is ready behind the loading screen: the scripts' `loaded` handlers run.
    pub fn mod_loading_loaded(&mut self, map: &str) {
        if self.mods.hud.loading.held && !self.mods.hud.loading.released {
            self.mod_scripts_fire_event("loaded", vec![map.to_string()], true);
        }
    }

    /// A press on the loading screen: the scripts' `loading_key` handlers run. True when the
    /// start is released.
    pub fn mod_loading_key(&mut self) -> bool {
        if self.mods.hud.loading.held && !self.mods.hud.loading.released {
            self.mod_scripts_fire_event("loading_key", Vec::new(), true);
        }
        self.mod_loading_released()
    }

    /// The scripts released the start (or never held it).
    pub fn mod_loading_released(&self) -> bool {
        !self.mods.hud.loading.held || self.mods.hud.loading.released
    }

    /// The level started: the loading screen is over.
    pub fn mod_loading_end(&mut self) {
        self.mods.hud.loading = Loading::default();
    }
}

/// Scale and offset of the 640×480 loading screen on a `width` × `height` window: s = height /
/// 480, centred horizontally.
pub fn screen_transform(width: u32, height: u32) -> (f32, f32) {
    let s = height as f32 / LOADING_H;
    (s, (width as f32 - LOADING_W * s) * 0.5)
}

/// Primitives of the 640×480 picture moved to the window. Lines get a thickness of about `s`
/// pixels (copies shifted across the segment), text tiles are filtered smoothly once scaled.
fn scale_hud(hud: Vec<HudDraw>, s: f32, ox: f32, bicubic: bool) -> Vec<HudDraw> {
    let thick = (s.round() as i32).max(1);
    let mut out = Vec::with_capacity(hud.len() * thick as usize);
    for d in hud {
        match d {
            HudDraw::Tile { x1, y1, x2, y2, uv, material, color, point, enlarged, glyph } => {
                // Enlarged text keeps its mark with the bicubic filter (gui::keeps_point).
                let point = point && (s == 1.0 || (s > 1.0 && bicubic));
                out.push(HudDraw::Tile { x1: x1 * s + ox, y1: y1 * s, x2: x2 * s + ox, y2: y2 * s, uv, material, color, point, enlarged: enlarged || s > 1.0, glyph });
            }
            HudDraw::Line { from, to, material, color } => {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let (nx, ny) = (-dy / len, dx / len);
                for k in 0..thick {
                    let o = k as f32 - (thick - 1) as f32 * 0.5;
                    let p = |q: [f32; 2]| [q[0] * s + ox + nx * o, q[1] * s + ny * o];
                    out.push(HudDraw::Line { from: p(from), to: p(to), material: material.clone(), color });
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// FColor order (B, G, R, A) of an R, G, B, A colour.
fn bgra(c: [u8; 4]) -> [u8; 4] {
    [c[2], c[1], c[0], c[3]]
}

/// An item as the network sends it.
fn item_json(i: &Item, now: f32) -> String {
    let mut v = serde_json::json!({
        "op": "set", "id": i.id, "k": i.kind.name(),
        "x": i.x, "y": i.y, "w": i.w, "h": i.h, "v": i.value,
        "t": i.text, "f": i.font, "a": i.align, "c": i.colour,
        "left": (i.ends_at - now).max(0.0),
    });
    if !i.texture.is_empty() {
        v["tx"] = serde_json::json!(i.texture);
    }
    if let Some(b) = i.back {
        v["b"] = serde_json::json!(b);
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_texts() {
        let mut c = Item::new("c".into(), Kind::Counter);
        c.text = "Kills: ".into();
        c.value = 3.0;
        assert_eq!(c.shown(0.0), "Kills: 3");
        c.value = 2.5;
        assert_eq!(c.shown(0.0), "Kills: 2.5");
        let mut t = Item::new("t".into(), Kind::Timer);
        t.ends_at = 100.0;
        assert_eq!(t.shown(25.0), "1:15");
        assert_eq!(t.shown(99.2), "0:01");
        assert_eq!(t.shown(120.0), "0:00", "never below zero");
    }

    #[test]
    fn transform() {
        assert_eq!(screen_transform(640, 480), (1.0, 0.0));
        assert_eq!(screen_transform(1920, 1080), (2.25, 240.0));
        let (s, ox) = screen_transform(1920, 1080);
        let out = scale_hud(vec![HudDraw::Line { from: [0.0, 0.0], to: [640.0, 0.0], material: "#canvasline:0:0".into(), color: [0xC4, 0xC4, 0xC4, 0xFF] }], s, ox, false);
        assert_eq!(out.len(), 2);
        let HudDraw::Line { from, to, .. } = &out[0] else { panic!() };
        assert_eq!((from[0], to[0]), (240.0, 1680.0));
    }

    #[test]
    fn network_items_round_trip() {
        let mut vm = Vm::new();
        let mut i = Item::new("score".into(), Kind::Bar);
        (i.x, i.y, i.w, i.h, i.value) = (0.1, 0.2, 0.3, 0.04, 0.5);
        i.colour = [1, 2, 3, 4];
        i.back = Some([5, 6, 7, 8]);
        vm.mod_hud_from_host(&item_json(&i, 0.0)).unwrap();
        assert_eq!(vm.mods.hud.items, vec![i.clone()]);
        vm.mod_hud_from_host(&serde_json::json!({"op": "del", "id": "score"}).to_string()).unwrap();
        assert!(vm.mods.hud.items.is_empty());
        // Negative control: an unknown kind is refused, nothing is added.
        assert!(vm.mod_hud_from_host("{\"op\":\"set\",\"id\":\"x\",\"k\":\"nothing\"}").is_err());
        assert!(vm.mods.hud.items.is_empty());
    }
}
