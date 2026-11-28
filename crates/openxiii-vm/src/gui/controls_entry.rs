//! Improved build only: a "Gamepad" box on the PC Controls page (XIIIMenuInputPC, opened by
//! the Options page's Controls box and by the in-game menu's Controls), which opens the gamepad
//! settings page (the viewer's gamepad_page.rs) in the view of the player whose menu it is.
//!
//! The box is an XIIIButton made after Created and appended to Controls, so mouse hover, focus
//! and clicks work as for the page's own boxes. It is drawn as a comic-strip panel in the left
//! column, above the keyboard picture: the word as its title, and below it the gamepad of the
//! console controls page (XIIIMenuStart.Control_Console.controlsps2_1…4) framed in black, with the
//! scene around the gamepad covered in white (the white of its own outline, so the cut does not
//! show). The picture keeps the shape the console page gives it. Focused, the box is drawn black
//! with a white title and frame.
//!
//! Shown only to a player who can use it: in split screen, a player who has a gamepad (a player
//! on the keyboard and mouse keeps the page's keyboard settings only); alone, when a gamepad is
//! connected. It follows the gamepads of the moment (checked at each paint of the page).
//!
//! Keys: the page's key handler moves the focus with a table by control index (Look, Move,
//! Shoot, Other, Reset) that knows no sixth control. While the box is shown: left from Look goes
//! to it (it sits left of and below Look, above the keyboard picture), the box goes right to
//! Look, left to Move, up / down to Reset, and up / down from Reset come back to it. Every other
//! key is the page's own. Enter or a click on it hands the request to the game window
//! (`take_gamepad_settings_request`) with the viewport of the page's player: in split screen each
//! player has their own menu and pages, and the settings page opens in that player's view.

use crate::added_text::Text;
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// The scene around the gamepad in the console picture, covered in white.
#[path = "controls_entry_picture.rs"]
mod picture;
pub use picture::PANEL_BACKGROUND;

/// The box, in the page's 640 × 480 units (x, y, w, h): the free part of the left column (Look's
/// box starts at x 128 from y 99 to 132, the onomatopoeia at x 143 from y 150 to 208, the
/// keyboard picture at y 273).
pub const ENTRY: [f32; 4] = [4.0, 152.0, 138.0, 116.0];
/// Height of the title strip, page units.
pub const TITLE: f32 = 28.0;

/// The console gamepad picture's four 256 × 256 quarters (512 × 512 texels).
pub const PICTURE: [&str; 4] = [
    "XIIIMenuStart.Control_Console.controlsps2_1",
    "XIIIMenuStart.Control_Console.controlsps2_2",
    "XIIIMenuStart.Control_Console.controlsps2_3",
    "XIIIMenuStart.Control_Console.controlsps2_4",
];
/// The front of the gamepad with its grips and its white outline in that picture (texels x, y,
/// w, h).
pub const PICTURE_UV: [f32; 4] = [104.0, 34.0, 304.0, 252.0];
/// Width / height of that part as the console page shows it (its 512 × 512 texels fill 640 ×
/// 480: 1.25 across, 0.9375 down).
pub const PICTURE_ASPECT: f32 = (304.0 * 1.25) / (252.0 * 0.9375);

/// The picture in a box `w` × `h` pixels at `s` pixels a page unit: under the title strip,
/// inside a margin, as large as fits with its shape kept (x, y, w, h).
pub fn picture_rect(w: f32, h: f32, s: f32) -> [f32; 4] {
    let (x, y) = (6.0 * s, (TITLE + 4.0) * s);
    let (rw, rh) = ((w - 12.0 * s).max(0.0), (h - (TITLE + 10.0) * s).max(0.0));
    let (pw, ph) = if rw / PICTURE_ASPECT <= rh { (rw, rw / PICTURE_ASPECT) } else { (rh * PICTURE_ASPECT, rh) };
    [x + (rw - pw) / 2.0, y + (rh - ph) / 2.0, pw, ph]
}

/// Where the text goes: its y in a box at `s` pixels a page unit, centred in the title strip.
pub fn title_y(s: f32, text_h: f32) -> f32 {
    (TITLE * s - text_h) / 2.0
}

/// Texel rectangle `r` (x0, y0, x1, y1) of the picture drawn at `at` (x, y, w, h pixels).
pub fn texels_to_pixels(r: [f32; 4], at: [f32; 4]) -> [f32; 4] {
    let [u0, v0, uw, vh] = PICTURE_UV;
    let (sx, sy) = (at[2] / uw, at[3] / vh);
    [at[0] + (r[0] - u0) * sx, at[1] + (r[1] - v0) * sy, at[0] + (r[2] - u0) * sx, at[1] + (r[3] - v0) * sy]
}

/// A control of the page, for the key table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    Look,
    Move,
    Shoot,
    Other,
    Reset,
    Entry,
}

impl Spot {
    /// Index in the page's Controls (the entry is appended after Reset).
    pub fn index(self) -> usize {
        match self {
            Spot::Look => 0,
            Spot::Move => 1,
            Spot::Shoot => 2,
            Spot::Other => 3,
            Spot::Reset => 4,
            Spot::Entry => 5,
        }
    }

    pub fn of_index(i: usize) -> Option<Spot> {
        [Spot::Look, Spot::Move, Spot::Shoot, Spot::Other, Spot::Reset, Spot::Entry].get(i).copied()
    }
}

const KEY_LEFT: u8 = 37;
const KEY_UP: u8 = 38;
const KEY_RIGHT: u8 = 39;
const KEY_DOWN: u8 = 40;

/// Where a key press moves the focus from `focused`, when it is not the page's own move: `None`
/// lets the page's handler do as in the original. `available`: the entry can be chosen (a
/// gamepad is there).
///
/// Left / Right go round each row of the page as in the weapon menu: the top row Gamepad, Look,
/// Move (then back to Gamepad), the bottom row Reset, Shoot, Other (then back to Reset). While
/// the entry cannot be chosen, the top row is Look, Move.
pub fn entry_nav(focused: Spot, key: u8, available: bool) -> Option<Spot> {
    match (focused, key) {
        // Not choosable while it had the focus (the gamepad went away): back to the first box.
        (Spot::Entry, _) if !available => Some(Spot::Look),
        (Spot::Look, KEY_LEFT) => Some(if available { Spot::Entry } else { Spot::Move }),
        (Spot::Move, KEY_RIGHT) => Some(if available { Spot::Entry } else { Spot::Look }),
        (Spot::Entry, KEY_LEFT) => Some(Spot::Move),
        (Spot::Entry, KEY_RIGHT) => Some(Spot::Look),
        (Spot::Reset, KEY_RIGHT) => Some(Spot::Shoot),
        (Spot::Reset, KEY_LEFT) => Some(Spot::Other),
        (Spot::Other, KEY_RIGHT) => Some(Spot::Reset),
        (Spot::Shoot, KEY_LEFT) => Some(Spot::Reset),
        (Spot::Reset, KEY_UP | KEY_DOWN) if available => Some(Spot::Entry),
        (Spot::Entry, KEY_UP | KEY_DOWN) => Some(Spot::Reset),
        _ => None,
    }
}

/// The entry can be chosen by a player: in split screen when that player has a gamepad,
/// otherwise when a gamepad is connected. It is always drawn, greyed out when it cannot.
pub fn entry_available(split: bool, own_gamepad: bool, any_gamepad: bool) -> bool {
    if split {
        own_gamepad
    } else {
        any_gamepad
    }
}

#[derive(Default)]
pub struct ControlsEntry {
    /// Each open Controls page (one per player's menu) and its entry.
    pages: Vec<(ObjId, ObjId)>,
    /// Viewport (0 = the first player's) of the menu whose entry was chosen.
    requested: Option<usize>,
}

impl Vm {
    /// The Gamepad entry was chosen since the last call (taken): the viewport of the player whose
    /// menu it is (0 = the first player's).
    pub fn take_gamepad_settings_request(&mut self) -> Option<usize> {
        self.improved.controls_entry.requested.take()
    }

    fn controls_entry_of(&self, page: ObjId) -> Option<ObjId> {
        self.improved.controls_entry.pages.iter().find(|p| p.0 == page).map(|p| p.1)
    }

    fn controls_entry_page(&self, entry: ObjId) -> Option<ObjId> {
        self.improved.controls_entry.pages.iter().find(|p| p.1 == entry).map(|p| p.0)
    }

    /// The page's player (XIIIWindow.GetPlayerOwner) and their viewport.
    fn controls_entry_slot(&mut self, page: ObjId) -> Option<u8> {
        let owner = self.call_named(page, "GetPlayerOwner", vec![]).ok().and_then(|v| v.as_object());
        owner.and_then(|pc| self.viewport_slot_of(pc))
    }

    /// The entry can be chosen on `page` (its player's devices now).
    fn controls_entry_wanted(&mut self, page: ObjId) -> bool {
        let any = self.pads.states.iter().any(Option::is_some);
        let split = self.split_pads();
        let own = split
            && self.controls_entry_slot(page).is_some_and(|slot| self.engine.client.viewport_pads.get(usize::from(slot)).copied().flatten().is_some());
        entry_available(split, own, any)
    }

    /// Makes the entry of `page` choosable or not as wanted now (it stays drawn, greyed out when
    /// it cannot be chosen); the focus leaves it when it cannot.
    fn controls_entry_update(&mut self, page: ObjId) -> bool {
        let Some(entry) = self.controls_entry_of(page) else { return false };
        let shown = self.controls_entry_wanted(page);
        if !self.propr(entry, "bVisible").as_bool() {
            self.set_prop(entry, "bVisible", Value::Bool(true));
        }
        if self.propr(entry, "bAcceptsInput").as_bool() != shown {
            self.set_prop(entry, "bAcceptsInput", Value::Bool(shown));
            self.set_prop(entry, "bNeverFocus", Value::Bool(!shown));
        }
        if !shown && self.propr(entry, "bHasFocus").as_bool() {
            let look = match self.prop(page, "Controls") {
                Value::Array(c) => c.first().and_then(|v| v.as_object()),
                _ => None,
            };
            if let Some(look) = look {
                let _ = self.call_named(look, "SetFocus", vec![Value::Object(None)]);
            }
        }
        shown
    }

    /// Hook of XIIIMenuInputPC's Created, Paint, InternalOnClick and InternalOnKeyEvent, and of
    /// the entry's own BeforePaint and Paint (Vm::call): the result when handled here, otherwise
    /// the arguments back.
    #[allow(clippy::type_complexity)]
    pub(crate) fn controls_entry_call(&mut self, f: usize, this: ObjId, args: Vec<Option<Value>>) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        let d = &self.functions[f];
        if d.state.is_some() || !matches!(d.name.as_str(), "Created" | "Paint" | "BeforePaint" | "InternalOnClick" | "InternalOnKeyEvent") {
            return Err(args);
        }
        let owner = &self.classes[d.owner].name;
        if owner.eq_ignore_ascii_case("XIIIButton") {
            return match d.name.as_str() {
                "BeforePaint" | "Paint" if self.controls_entry_page(this).is_some() => Ok(self.controls_entry_paint(f, this, args)),
                _ => Err(args),
            };
        }
        if !owner.eq_ignore_ascii_case("XIIIMenuInputPC") {
            return Err(args);
        }
        let name = d.name.clone();
        match name.as_str() {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.controls_entry_created(this);
                }
                Ok(r)
            }
            "Paint" if self.controls_entry_of(this).is_some() => {
                self.controls_entry_update(this);
                Ok(self.call_inner(f, this, args))
            }
            "InternalOnClick" if self.controls_entry_of(this).is_some() => {
                let sender = args.first().cloned().flatten().and_then(|v| v.as_object());
                if sender.is_some() && sender == self.controls_entry_of(this) {
                    if self.controls_entry_update(this) {
                        let slot = self.controls_entry_slot(this).map_or(0, usize::from);
                        self.improved.controls_entry.requested = Some(slot);
                    }
                    return Ok(Ok((Value::Bool(true), Vec::new())));
                }
                Ok(self.call_inner(f, this, args))
            }
            "InternalOnKeyEvent" if self.controls_entry_of(this).is_some() => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                let shown = self.controls_entry_update(this);
                let focused = self.propr(this, "FocusedControl").as_object();
                let controls: Vec<Option<ObjId>> = match self.prop(this, "Controls") {
                    Value::Array(c) => c.iter().map(|v| v.as_object()).collect(),
                    _ => Vec::new(),
                };
                let spot = focused.and_then(|o| controls.iter().position(|&c| c == Some(o))).and_then(Spot::of_index);
                let target = match (state, spot, u8::try_from(key)) {
                    (1, Some(spot), Ok(key)) => entry_nav(spot, key, shown),
                    _ => None,
                };
                if let Some(t) = target.and_then(|t| controls.get(t.index()).copied().flatten()) {
                    let _ = self.call_named(t, "SetFocus", vec![Value::Object(None)]);
                    let outs = self.functions[f].params.iter().enumerate().map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Byte(0))) } else { None }).collect();
                    return Ok(Ok((Value::Bool(true), outs)));
                }
                Ok(self.call_inner(f, this, args))
            }
            _ => Err(args),
        }
    }

    /// The box's BeforePaint (title placed in its strip) and Paint (colours by focus, then the
    /// title's edge, the frame and the picture).
    fn controls_entry_paint(&mut self, f: usize, entry: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let c = args.first().cloned().flatten().and_then(|v| v.as_object());
        let page = self.controls_entry_page(entry);
        let (rx, ry) = (self.propr(entry, "fRatioX").as_float(), self.propr(entry, "fRatioY").as_float());
        let s = rx.min(ry).max(0.1);
        let (w, h) = (self.propr(entry, "WinWidth").as_float() * 640.0 * rx, self.propr(entry, "WinHeight").as_float() * 480.0 * ry);
        if self.functions[f].name == "BeforePaint" {
            let r = self.call_inner(f, entry, args);
            // The text's height in the font BeforePaint has just set; BeforePaint centred it across.
            let text = self.prop(entry, "Text").as_name();
            let text_h = match c {
                Some(c) => {
                    let mut a = [Value::Str(text), Value::Float(0.0), Value::Float(0.0)];
                    self.canvas_native("TextSize", c, &mut a, &[true; 3]);
                    a[2].as_float()
                }
                None => 0.0,
            };
            self.set_prop(entry, "TextY", Value::Int(title_y(s, text_h).round() as i32));
            return r;
        }
        // Focused: black box, white text; otherwise the page's white box, black text.
        let focused = self.propr(entry, "bHasFocus").as_bool();
        if let Some(page) = page {
            let (black, white) = (self.prop(page, "BlackColor"), self.prop(page, "WhiteColor"));
            let back = self.prop(entry, "BackColor");
            self.set_prop(entry, "BoxColor", if focused { black.clone() } else { back });
            self.set_prop(entry, "TextColor", if focused { white } else { black });
        }
        let r = self.call_inner(f, entry, args);
        let (Some(c), Some(page)) = (c, page) else { return r };
        let (ox, oy) = self.controls_entry_origin(entry, page, c);
        let ink = if focused { [255u8, 255, 255, 255] } else { [0u8, 0, 0, 255] };
        let tile = |r: [f32; 4], color: [u8; 4]| Vm::rectangle_free(ox + r[0], oy + r[1], ox + r[2], oy + r[3], color);
        // The title strip's lower edge, then the picture's frame.
        let line = (1.0 * s).max(1.0);
        self.hud.push(tile([0.0, TITLE * s - line, w, TITLE * s + line], ink));
        let at = picture_rect(w, h, s);
        let frame = (2.0 * s).max(1.0);
        self.hud.push(tile([at[0] - frame, at[1] - frame, at[0] + at[2] + frame, at[1] + at[3] + frame], ink));
        self.controls_entry_picture(c, (ox, oy), at);
        // The scene around the gamepad, in the white of its outline.
        let white = [255u8, 255, 255, 255];
        for b in PANEL_BACKGROUND {
            let p = texels_to_pixels(b.map(f32::from), at);
            self.hud.push(tile(p, white));
        }
        // No gamepad to set: the whole box greyed out under a white veil.
        if !self.propr(entry, "bAcceptsInput").as_bool() {
            self.hud.push(tile([0.0, 0.0, w, h], [255, 255, 255, 170]));
        }
        r
    }

    /// The box's place on the canvas (pixels): as XIIIGUIBaseButton.DrawStretchedTexture sets
    /// the origin for the box itself, the menu centred in 800 × 600 when the window is larger.
    fn controls_entry_origin(&self, entry: ObjId, page: ObjId, c: ObjId) -> (f32, f32) {
        let (rx, ry) = (self.propr(entry, "fRatioX").as_float(), self.propr(entry, "fRatioY").as_float());
        let (mut nx, mut ny) = (self.propr(entry, "WinLeft").as_float() * 640.0 * rx, self.propr(entry, "WinTop").as_float() * 480.0 * ry);
        let root = self.propr(entry, "myRoot").as_object();
        let centred = root.is_some_and(|r| self.propr(r, "bMapMenu").as_bool()) || self.propr(page, "bCenterInGame").as_bool();
        if centred {
            let (clip_x, clip_y) = (self.propr(c, "ClipX").as_float(), self.propr(c, "ClipY").as_float());
            if clip_x > 800.0 {
                nx += (clip_x - 800.0) / 2.0;
            }
            if clip_y > 600.0 {
                ny += (clip_y - 600.0) / 2.0;
            }
        }
        (nx, ny)
    }

    /// The console gamepad picture's [`PICTURE_UV`] part over `at` (x, y, w, h pixels of the box
    /// placed at `origin` on the canvas): one tile for each of its quarters it crosses.
    fn controls_entry_picture(&mut self, c: ObjId, origin: (f32, f32), at: [f32; 4]) {
        let [x, y, w, h] = at;
        let (org_x, org_y, style) = (self.prop(c, "OrgX"), self.prop(c, "OrgY"), self.prop(c, "Style"));
        self.canvas_native("SetOrigin", c, &mut [Value::Float(origin.0), Value::Float(origin.1)], &[true; 2]);
        self.set_prop(c, "Style", Value::Byte(1));
        self.canvas_native("SetDrawColor", c, &mut [255u8, 255, 255, 255].map(Value::Byte), &[true; 4]);
        let [u0, v0, uw, vh] = PICTURE_UV;
        for (k, tex) in PICTURE.iter().enumerate() {
            let (qx, qy) = ((k % 2) as f32 * 256.0, (k / 2) as f32 * 256.0);
            let (a0, a1) = (u0.max(qx), (u0 + uw).min(qx + 256.0));
            let (b0, b1) = (v0.max(qy), (v0 + vh).min(qy + 256.0));
            if a1 <= a0 || b1 <= b0 {
                continue;
            }
            let (sx, sy) = (x + (a0 - u0) / uw * w, y + (b0 - v0) / vh * h);
            let (tw, th) = ((a1 - a0) / uw * w, (b1 - b0) / vh * h);
            self.canvas_native("SetPos", c, &mut [Value::Float(sx), Value::Float(sy)], &[true; 2]);
            // Textures are named by their path (the canvas' materials).
            let mut a = [Value::Asset(tex.to_string()), Value::Float(tw), Value::Float(th), Value::Float(a0 - qx), Value::Float(b0 - qy), Value::Float(a1 - a0), Value::Float(b1 - b0)];
            self.canvas_native("DrawTile", c, &mut a, &[true; 7]);
        }
        self.set_prop(c, "Style", style);
        self.set_prop(c, "OrgX", org_x);
        self.set_prop(c, "OrgY", org_y);
    }

    /// After XIIIMenuInputPC.Created: the entry, an XIIIButton framed like Reset.
    fn controls_entry_created(&mut self, page: ObjId) {
        // Pages of menus since closed are forgotten.
        let live = |vm: &Vm, o: ObjId| vm.objects.get(o).is_some_and(|x| x.is_some());
        let kept: Vec<(ObjId, ObjId)> = self.improved.controls_entry.pages.iter().copied().filter(|&(p, _)| p != page && live(self, p)).collect();
        self.improved.controls_entry.pages = kept;
        let Some(class) = self.class("XIIIButton") else { return };
        let [x, y, w, h] = ENTRY;
        let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y), Value::Float(w), Value::Float(h)];
        let Some(entry) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { return };
        self.set_prop(entry, "Text", Value::Str(self.added_text(Text::Gamepad).to_string()));
        self.set_prop(entry, "bUseBorder", Value::Bool(true));
        if let Value::Array(mut controls) = self.prop(page, "Controls") {
            controls.push(Value::Object(Some(entry)));
            self.set_prop(page, "Controls", Value::Array(controls));
        }
        self.improved.controls_entry.pages.push((page, entry));
        self.controls_entry_update(page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [u8; 4] = [KEY_LEFT, KEY_UP, KEY_RIGHT, KEY_DOWN];
    const SPOTS: [Spot; 6] = [Spot::Look, Spot::Move, Spot::Shoot, Spot::Other, Spot::Reset, Spot::Entry];

    /// The page's own table (XIIIMenuInputPC.InternalOnKeyEvent), for the moves this file leaves to it.
    fn original(focused: Spot, key: u8) -> Option<Spot> {
        let ud = matches!(key, KEY_UP | KEY_DOWN);
        match focused {
            Spot::Look if ud => Some(Spot::Shoot),
            Spot::Look if key == KEY_LEFT => Some(Spot::Reset),
            Spot::Look => Some(Spot::Move),
            Spot::Move if ud => Some(Spot::Other),
            Spot::Move => Some(Spot::Look),
            Spot::Shoot if ud => Some(Spot::Look),
            Spot::Shoot if key == KEY_LEFT => Some(Spot::Reset),
            Spot::Shoot => Some(Spot::Other),
            Spot::Other if ud => Some(Spot::Move),
            Spot::Other => Some(Spot::Shoot),
            Spot::Reset if key == KEY_LEFT => Some(Spot::Move),
            Spot::Reset if key == KEY_RIGHT => Some(Spot::Look),
            Spot::Reset | Spot::Entry => None,
        }
    }

    fn step(at: Spot, key: u8, shown: bool, nav: &dyn Fn(Spot, u8, bool) -> Option<Spot>) -> Spot {
        nav(at, key, shown).or_else(|| original(at, key)).unwrap_or(at)
    }

    /// Spots reachable with the four directions from the page's first focus (Look).
    fn reachable(shown: bool, nav: &dyn Fn(Spot, u8, bool) -> Option<Spot>) -> Vec<Spot> {
        let mut seen = vec![Spot::Look];
        let mut i = 0;
        while i < seen.len() {
            for k in KEYS {
                let n = step(seen[i], k, shown, nav);
                if !seen.contains(&n) {
                    seen.push(n);
                }
            }
            i += 1;
        }
        seen
    }

    #[test]
    fn entry_reached_from_its_neighbours_and_left_again() {
        // From Look (its right neighbour) one press left; from Reset (below) one press up.
        assert_eq!(step(Spot::Look, KEY_LEFT, true, &entry_nav), Spot::Entry);
        assert_eq!(step(Spot::Reset, KEY_UP, true, &entry_nav), Spot::Entry);
        assert_eq!(step(Spot::Reset, KEY_DOWN, true, &entry_nav), Spot::Entry);
        // From Shoot (bottom left) left goes to Reset as in the original, then up to the entry.
        assert_eq!(step(step(Spot::Shoot, KEY_LEFT, true, &entry_nav), KEY_UP, true, &entry_nav), Spot::Entry);
        // From the entry every direction goes somewhere.
        for k in KEYS {
            assert_ne!(step(Spot::Entry, k, true, &entry_nav), Spot::Entry, "key {k}");
        }
        // Every control is reachable from the first focus, the entry included.
        let r = reachable(true, &entry_nav);
        for s in SPOTS {
            assert!(r.contains(&s), "{s:?} not reachable: {r:?}");
        }
        // Negative control: without this table (the page's own only) the entry is unreachable,
        // which is the reported defect.
        let none = |_: Spot, _: u8, _: bool| None;
        assert!(!reachable(true, &none).contains(&Spot::Entry));
        // Up / down from Reset only (the former table): the entry is unreachable from Look,
        // Move, Shoot or Other through up / down, and only from Reset.
        let former = |at: Spot, key: u8, _: bool| match (at, key) {
            (Spot::Reset, KEY_UP | KEY_DOWN) => Some(Spot::Entry),
            (Spot::Entry, KEY_UP | KEY_DOWN) => Some(Spot::Reset),
            _ => None,
        };
        assert_ne!(step(Spot::Look, KEY_LEFT, true, &former), Spot::Entry);
    }

    /// Left / Right go round each row as in the weapon menu: top row Gamepad, Look, Move, back to
    /// Gamepad (Look, Move without a gamepad); bottom row Reset, Shoot, Other, back to Reset.
    /// Negative control: the page's own table goes from Move to Look and from Reset to Move.
    #[test]
    fn rows_wrap_like_the_weapon_menu() {
        let walk = |start: Spot, key: u8, available: bool| {
            let mut out = vec![start];
            for _ in 0..3 {
                out.push(step(*out.last().unwrap(), key, available, &entry_nav));
            }
            out
        };
        use Spot::*;
        assert_eq!(walk(Entry, KEY_RIGHT, true), [Entry, Look, Move, Entry]);
        assert_eq!(walk(Entry, KEY_LEFT, true), [Entry, Move, Look, Entry]);
        assert_eq!(walk(Look, KEY_RIGHT, false), [Look, Move, Look, Move]);
        assert_eq!(walk(Look, KEY_LEFT, false), [Look, Move, Look, Move]);
        for available in [true, false] {
            assert_eq!(walk(Reset, KEY_RIGHT, available), [Reset, Shoot, Other, Reset]);
            assert_eq!(walk(Reset, KEY_LEFT, available), [Reset, Other, Shoot, Reset]);
        }
        let none = |_: Spot, _: u8, _: bool| None;
        assert_eq!(step(Move, KEY_RIGHT, true, &none), Look);
        assert_eq!(step(Reset, KEY_LEFT, true, &none), Move);
    }

    #[test]
    fn entry_without_a_gamepad_is_never_reached() {
        // Greyed out: the entry is never reached, and the focus leaves it if it had it.
        assert!(!reachable(false, &entry_nav).contains(&Spot::Entry));
        assert_eq!(entry_nav(Spot::Entry, KEY_UP, false), Some(Spot::Look));
        assert_eq!(entry_nav(Spot::Reset, KEY_UP, false), None, "up / down from Reset stay the page's own");
        // Every other control stays reachable.
        let r = reachable(false, &entry_nav);
        for s in SPOTS.into_iter().filter(|&s| s != Spot::Entry) {
            assert!(r.contains(&s), "{s:?}");
        }
        // Negative control: with a gamepad it is reached.
        assert!(reachable(true, &entry_nav).contains(&Spot::Entry));
    }

    #[test]
    fn entry_available_to_the_players_who_have_a_gamepad() {
        // Split screen: own gamepad only (a keyboard and mouse player never sees it, even when
        // the other player has a gamepad).
        assert!(entry_available(true, true, true));
        assert!(!entry_available(true, false, true));
        assert!(!entry_available(true, false, false));
        // Alone: whenever a gamepad is connected.
        assert!(entry_available(false, false, true));
        assert!(!entry_available(false, false, false));
        // Negative control: a rule on "any gamepad" would show it to the keyboard player.
        let any_rule = |_split: bool, _own: bool, any: bool| any;
        assert_ne!(any_rule(true, false, true), entry_available(true, false, true));
    }
    #[test]
    fn box_fits_the_left_column_and_keeps_the_picture_shape() {
        let overlaps = |a: [f32; 4], b: [f32; 4]| a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3];
        // Look's box, the onomatopoeia, the keyboard picture, the title (page units).
        let taken = [[128.0, 99.0, 160.0, 33.0], [143.0, 150.0, 130.0, 58.0], [18.0, 273.0, 180.0, 127.0], [0.0, 25.0, 155.0, 56.0]];
        let r = ENTRY;
        assert!(r[0] >= 0.0 && r[1] >= 82.0 && r[1] + r[3] <= 270.0, "{r:?}");
        for t in taken {
            assert!(!overlaps(r, t), "over {t:?}");
        }
        let mut checked = 0;
        // 800 × 600 and smaller frames (split screen, small windows), and larger ones.
        for s in [0.6, 0.72, 0.9, 1.0, 1.25, 1.6875, 2.25] {
            let (w, h) = (r[2] * s, r[3] * s);
            let [x, y, pw, ph] = picture_rect(w, h, s);
            assert!((pw / ph - PICTURE_ASPECT).abs() < 0.01, "{s}");
            assert!(x >= 0.0 && x + pw <= w + 0.01 && y + ph <= h + 0.01, "{s}");
            // Under the title strip, and large: most of the box across.
            assert!(y >= TITLE * s, "{s}: picture over the title");
            assert!(pw >= 0.85 * w, "{s}: picture too small");
            for text_h in [12.0, 16.0, 20.0] {
                let ty = title_y(s, text_h * s);
                assert!(ty >= 0.0 && ty + text_h * s <= TITLE * s + 0.01, "{s}: title out of its strip");
            }
            checked += 1;
        }
        assert_eq!(checked, 7);
        // Negative controls: the picture stretched to a square box (the former entry's or the
        // keyboard's room) would lose its shape, which the check above sees; a box placed over the
        // keyboard is caught.
        let stretched = (r[2] - 12.0) / (r[2] - 12.0);
        assert!((stretched - PICTURE_ASPECT).abs() > 0.1);
        let [_, _, pw, ph] = picture_rect(r[2], r[2], 1.0);
        assert!((pw / ph - PICTURE_ASPECT).abs() < 0.01, "a square box still gets the picture's shape");
        assert!(overlaps([4.0, 200.0, 138.0, 116.0], taken[2]));
    }

    /// The white covers stay inside the picture's part of the console picture and keep its
    /// gamepad: the texels at the centre of the gamepad's parts (d-pad, sticks, buttons, grips)
    /// are never covered, while the corners of the crop (scene) are.
    #[test]
    fn background_covers_the_scene_only() {
        let [u0, v0, uw, vh] = PICTURE_UV;
        let covered = |u: f32, v: f32| PANEL_BACKGROUND.iter().any(|b| u >= f32::from(b[0]) && u < f32::from(b[2]) && v >= f32::from(b[1]) && v < f32::from(b[3]));
        for b in PANEL_BACKGROUND {
            let b = b.map(f32::from);
            assert!(b[0] >= u0 && b[2] <= u0 + uw && b[1] >= v0 && b[3] <= v0 + vh && b[0] < b[2] && b[1] < b[3], "{b:?}");
        }
        // Gamepad parts (texels): d-pad, both sticks, the four buttons' middle, both grips.
        for (u, v) in [(180.0, 120.0), (210.0, 170.0), (300.0, 170.0), (330.0, 120.0), (150.0, 230.0), (360.0, 230.0), (255.0, 120.0)] {
            assert!(!covered(u, v), "gamepad covered at {u} {v}");
        }
        // Scene: the crop's four corners and the gap between the grips.
        for (u, v) in [(u0 + 1.0, v0 + 1.0), (u0 + uw - 2.0, v0 + 1.0), (u0 + 1.0, v0 + vh - 2.0), (u0 + uw - 2.0, v0 + vh - 2.0), (255.0, 270.0)] {
            assert!(covered(u, v), "scene left at {u} {v}");
        }
        // Mapped onto any box: inside the picture's rectangle.
        let at = [10.0, 20.0, 160.0, 160.0 / PICTURE_ASPECT];
        for b in PANEL_BACKGROUND {
            let p = texels_to_pixels(b.map(f32::from), at);
            assert!(p[0] >= at[0] - 0.01 && p[2] <= at[0] + at[2] + 0.01 && p[1] >= at[1] - 0.01 && p[3] <= at[1] + at[3] + 0.01);
        }
        // Negative control: a cover over the whole crop would hide the gamepad.
        let whole = |u: f32, v: f32| u >= u0 && u < u0 + uw && v >= v0 && v < v0 + vh;
        assert!(whole(180.0, 120.0));
    }
}
