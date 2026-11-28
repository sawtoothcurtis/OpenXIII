//! C++ code of the menus (GUI.dll): virtual methods of GUIController, GUIComponent,
//! GUIMultiComponent and GUIPage. 32-bit floats in machine-code order.

use crate::canvas::HudDraw;
use crate::improved::gui as edition;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Window coordinates shared by menu drawing, cursor input and hit testing.
pub(crate) struct MenuLayout {
    /// Whole-window frame from the last GUI frame. MouseX and MouseY use its page.
    pub(crate) last_frame: Option<MenuFrame>,
    /// Scale read from video settings at the start of a GUI frame.
    ui_scale: f32,
}

impl Default for MenuLayout {
    fn default() -> Self {
        Self { last_frame: None, ui_scale: 1.0 }
    }
}

/// Menu frame in progress (`Vm::menu_frame_begin`): the canvas fields and the screen size to put
/// back, the frame, the first primitive drawn in it, and whether it is the whole window's 4:3 frame
/// (`Vm::menu_frame`) rather than a split-screen menu's (`Vm::split_menu_frame`).
struct MenuFrameState {
    saved: Vec<Value>,
    screen: (i32, i32),
    frame: MenuFrame,
    start: usize,
    whole_window: bool,
    /// Improved build: the in-game pages' bands and backdrop run to the edges of the frame's
    /// screen (the whole window, or a split-screen player's view), as in a whole window.
    wide: bool,
}

/// Input actions (EInputAction).
pub(crate) const IST_PRESS: u8 = 1;
const IST_HOLD: u8 = 2;
pub(crate) const IST_RELEASE: u8 = 3;
/// IK_LeftMouse, IK_RightMouse, IK_MiddleMouse, IK_MouseX, IK_MouseY, IK_MouseW, IK_MouseWheelUp,
/// IK_MouseWheelDown.
const MOUSE_KEYS: [u8; 8] = [0x01, 0x02, 0x04, 0xe4, 0xe5, 0xe6, 0xec, 0xed];
/// Virtual screen of a split-screen menu in a frame lower than its height: the 800 × 600 page the
/// PC menus are made for.
const VIRTUAL_MENU: (i32, i32) = (800, 600);
/// Improved build (MenuFrame::of_window): the original's modes 1280 pixels wide (1280 × 720,
/// 1280 × 800, 1280 × 960, 1280 × 1024) give the menus their reference size; a wider screen than
/// 16:9 keeps the share of 1280 × 720.
const REFERENCE_WIDTH: f32 = 1280.0;
const REFERENCE_HEIGHT: f32 = 720.0;

/// Where a split-screen viewport's menu goes (Vm::split_menu_frame): its virtual screen
/// `virtual_size`, scaled by `scale`, at (x, y) of the viewport's canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuFrame {
    /// Filter for text enlarged by this frame.
    pub(crate) bicubic: bool,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub virtual_size: (i32, i32),
}

impl MenuFrame {
    /// The frame of a split-screen viewport's `clip_x` × `clip_y` canvas (see
    /// Vm::split_menu_frame); None for a 4:3 canvas at least 600 pixels high (the menus' own layout).
    pub fn of_half(clip_x: i32, clip_y: i32) -> Option<MenuFrame> {
        // A canvas not sized yet (before its first frame) has no frame.
        if clip_x <= 0 || clip_y <= 0 {
            return None;
        }
        let [x, y, w, h] = menu_frame_43(clip_x, clip_y);
        if x == 0 && y == 0 && h >= VIRTUAL_MENU.1 {
            return None;
        }
        // The PC menus are made for 800 × 600 (fRatioX capped at 1.25 = 800 / 640; a wider screen
        // only centres them): smaller frames show an 800 × 600 page scaled down.
        let virtual_size = if h >= VIRTUAL_MENU.1 { (w, h) } else { VIRTUAL_MENU };
        Some(MenuFrame { bicubic: enlarged_text_bicubic(Default::default()), x: x as f32, y: y as f32, scale: h as f32 / virtual_size.1 as f32, virtual_size })
    }

    /// Improved build, a whole window's menus (Vm::window_menu_frame) and the split-screen halves
    /// larger than the reference: XIIIWindow.BeforePaint caps fRatioX / fRatioY at 800 × 600 for
    /// the main menu's pages (bMapMenu, bCenterInGame) and centres them, so on a larger screen they
    /// kept their size in pixels and shrank (a quarter of a 4K screen's width). The menus are drawn
    /// as on the original's 1280-wide mode of the same shape and the whole picture is scaled
    /// uniformly by min(width / 1280, height / 720) times `ui` (UIScale): fRatio = 1.25 × that
    /// scale for the capped pages, without steps; a 1280 × 720, 1280 × 800, 1280 × 960 or
    /// 1280 × 1024 window is exactly the original. They lay themselves out in the 4:3 frame of
    /// that mode (the improved build's frame, `menu_frame_43`), centred on whole pixels. The scale
    /// is held where the 800 × 600 page fills the 4:3 frame's height (a larger UIScale would only
    /// shrink the page's own layout). None when the scale is 1 or less: the game's own layout,
    /// never smaller than the original.
    pub fn of_window(clip_x: i32, clip_y: i32, ui: f32) -> Option<MenuFrame> {
        let (cw, ch) = (clip_x.max(1) as f32, clip_y.max(1) as f32);
        let [_, _, _, fh] = menu_frame_43(clip_x, clip_y);
        let scale = ((cw / REFERENCE_WIDTH).min(ch / REFERENCE_HEIGHT) * ui).min(fh as f32 / VIRTUAL_MENU.1 as f32);
        if scale <= 1.0 {
            return None;
        }
        // The 4:3 frame of the window seen at that scale (the 1280-wide mode's).
        let [_, _, vw, vh] = menu_frame_43((cw / scale).round() as i32, (ch / scale).round() as i32);
        let (x, y) = ((cw - vw as f32 * scale) / 2.0, (ch - vh as f32 * scale) / 2.0);
        Some(MenuFrame { bicubic: enlarged_text_bicubic(Default::default()), x: x.round(), y: y.round(), scale, virtual_size: (vw, vh) })
    }

    /// A primitive drawn on the virtual screen, moved into the frame ([`place_scaled`]).
    pub fn place(&self, d: &mut HudDraw) {
        place_scaled_in(d, [self.scale; 2], [self.x, self.y], self.bicubic);
    }

    /// A page drawn on the virtual screen (primitives from `start` on), moved into the frame of a
    /// `canvas`-sized canvas: a backdrop covering the whole virtual screen (page background) covers
    /// the whole canvas.
    pub fn place_page(&self, d: &mut [HudDraw], canvas: (f32, f32)) {
        let (vw, vh) = (self.virtual_size.0 as f32, self.virtual_size.1 as f32);
        let s = self.scale;
        for d in d.iter_mut() {
            if let HudDraw::Tile { x1, y1, x2, y2, .. } = d {
                if *x1 <= 0.5 && *y1 <= 0.5 && *x2 >= vw - 0.5 && *y2 >= vh - 0.5 {
                    (*x1, *y1, *x2, *y2) = (-self.x / s, -self.y / s, (canvas.0 - self.x) / s, (canvas.1 - self.y) / s);
                }
            }
            self.place(d);
        }
    }

    /// A point of the viewport's canvas (the window's mouse) on the virtual screen.
    pub fn unplace(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.scale, (y - self.y) / self.scale)
    }

    /// A point of the virtual screen on the viewport's canvas.
    pub fn to_canvas(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y + y * self.scale)
    }
}

/// How a finger works a menu control (Vm::menu_control_at, touch screens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTouch {
    /// Buttons: one click when the finger lifts over the control it was put on.
    Tap,
    /// A list's rows (a GUI list, the Select level list): a tap chooses the row as a button
    /// does; a finger dragged up or down scrolls the list as the mouse wheel does, and clicks nothing.
    List,
    /// Sliders and scroll bars of the GUI (they capture the mouse and follow it while pressed):
    /// pressed as the finger lands, held while it moves, released when it lifts.
    Hold,
    /// The XIII menus' sliders (XIIIHSliderControl): they read the cursor only when the button is
    /// pressed (InternalOnKeyEvent), so a click at each new place of the finger.
    Clicks,
}

/// Touch screens: whether a menu tap lifted over the control `now` (and the element drawn outside
/// the controls `now_part`) clicks, for a finger put down on `start` / `start_part`; `near`: the
/// finger stayed near where it was put down. A finger put down on a control clicks only if lifted
/// over that same control (slid off a button, it clicks nothing). A finger put down on no control
/// and lifted where it was put down clicks what is under it at the lift: the first menu after
/// launch (or a page just opened) is shown before its first layout and its rebuilt controls, a
/// tap made then was on no control yet and was dropped when lifted over the button.
pub fn menu_tap_clicks(start: Option<ObjId>, start_part: Option<usize>, now: Option<ObjId>, now_part: Option<usize>, near: bool) -> bool {
    match start {
        Some(_) => now == start && now_part == start_part,
        None => near && (start_part.is_none() || now_part == start_part),
    }
}

/// Where a page drawn outside the scripts over the menus goes (Vm::menu_screen; the improved
/// build's Mods page): the page is laid out on `size` and its primitives placed by `place`, the
/// same rule as the game's own pages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuScreen {
    /// The whole window's menu frame; None: the page is the window itself.
    pub frame: Option<MenuFrame>,
    /// Size of the page to lay out (the frame's virtual screen, or the window).
    pub size: (u32, u32),
    /// The window's size.
    pub canvas: (f32, f32),
}

impl MenuScreen {
    /// The page's primitives placed on the window (backdrops covering the page cover the window).
    pub fn place(&self, d: &mut [HudDraw]) {
        if let Some(f) = self.frame {
            f.place_page(d, self.canvas);
        }
    }

    /// A point of the window (a finger) on the page.
    pub fn to_page(&self, p: (f32, f32)) -> (f32, f32) {
        self.frame.map_or(p, |f| f.unplace(p.0, p.1))
    }
}

/// Improved build, the in-game HUD of a `clip_x` × `clip_y` viewport (Vm::hud_end): the HUD
/// scripts draw their icons, numbers and messages at fixed sizes in pixels (anchored to the
/// margins or by ClipX / ClipY), so on a large screen they shrank to a corner (a third of their
/// share on a 4K screen). As the menus (MenuFrame::of_window), the HUD is laid out on the screen of
/// the original's 1280-wide mode of the same shape, scale min(width / 1280, height / 720) times
/// `ui` (UIScale), and its whole picture is scaled into the viewport (enlarged font cells are
/// read with the bicubic filter, [`keeps_point`]). The virtual screen is a whole number of pixels: each axis gets the factor that
/// brings it exactly onto the viewport (the two differ by less than a pixel over the screen).
/// None when the scale is 1 or less: the game's own layout, never smaller than the original.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudScreen {
    /// Filter for text enlarged by this screen.
    pub(crate) bicubic: bool,
    pub scale: [f32; 2],
    pub virtual_size: (i32, i32),
}

impl HudScreen {
    pub fn of_viewport(clip_x: i32, clip_y: i32, ui: f32) -> Option<HudScreen> {
        if clip_x <= 0 || clip_y <= 0 {
            return None;
        }
        let (cw, ch) = (clip_x as f32, clip_y as f32);
        let scale = (cw / REFERENCE_WIDTH).min(ch / REFERENCE_HEIGHT) * ui;
        if scale <= 1.0 {
            return None;
        }
        let (vw, vh) = ((cw / scale).round().max(1.0) as i32, (ch / scale).round().max(1.0) as i32);
        Some(HudScreen { bicubic: enlarged_text_bicubic(Default::default()), scale: [cw / vw as f32, ch / vh as f32], virtual_size: (vw, vh) })
    }

    /// A primitive drawn on the virtual screen, moved onto the viewport.
    pub fn place(&self, d: &mut HudDraw) {
        place_scaled_in(d, self.scale, [0.0, 0.0], self.bicubic);
    }
}

/// The menus' common rule for a primitive drawn at `scale` times its size from `origin` (text,
/// cursor, pictures and boxes alike): positions scaled at the real resolution (never a picture of
/// the page enlarged). Point-sampled cells (font glyphs) keep their filtering as [`keeps_point`]
/// decides. Used by every menu frame (MenuFrame::place) and by the pages drawn outside the scripts.
pub fn place_scaled(d: &mut HudDraw, scale: f32, origin: [f32; 2]) {
    place_scaled_xy(d, [scale, scale], origin);
}

/// Does a point-sampled cell (a font glyph) keep its text mark once drawn at `sx` × `sy` times its
/// size? At scale 1 the mark draws it as the original does, pixel for pixel (POINT). With
/// `bicubic_enlarged` (the improved build's enhanced graphics, [`enlarged_text_bicubic`]) an
/// enlarged cell keeps its mark too and the renderer reads it with the bicubic filter (enlarged
/// tiles, `HudDraw::Tile::enlarged`): a glyph enlarged 2 or 3 times by repeating its texels showed
/// square blocks on 1440p and 4K screens, and bilinear filtering blurred it; a smaller cell is
/// smoothed. Otherwise a whole scale keeps it point-sampled and any other scale smooths it.
pub fn keeps_point(sx: f32, sy: f32, bicubic_enlarged: bool) -> bool {
    if bicubic_enlarged {
        sx >= 1.0 && sy >= 1.0
    } else {
        sx.fract() == 0.0 && sy.fract() == 0.0
    }
}

/// The improved build with its enhanced graphics: enlarged text keeps its mark and is read with
/// the bicubic filter ([`keeps_point`]).
pub fn enlarged_text_bicubic(graphics: crate::video_settings::SessionGraphics) -> bool {
    crate::display_format::active(graphics)
}

/// Is a canvas tile read with the bicubic filter rather than its own (POINT, or bilinear for the
/// text of ClippedPrint / DrawTextJustified)? Enlarged text with `bicubic_enlarged`
/// ([`enlarged_text_bicubic`]): a font letter (`glyph`, marked where every letter is emitted)
/// or a point-sampled cell; the renderer asks it for each batch.
pub fn reads_bicubic(point: bool, glyph: bool, enlarged: bool, bicubic_enlarged: bool) -> bool {
    bicubic_enlarged && (point || glyph) && enlarged
}

/// [`place_scaled`] with a factor per axis (HudScreen).
pub fn place_scaled_xy(d: &mut HudDraw, scale: [f32; 2], origin: [f32; 2]) {
    place_scaled_in(d, scale, origin, enlarged_text_bicubic(Default::default()));
}

/// [`place_scaled_in`] for the canvas's text tests (canvas_text_test.rs).
#[cfg(test)]
pub(crate) fn place_scaled_test(d: &mut HudDraw, scale: [f32; 2], bicubic_enlarged: bool) {
    place_scaled_in(d, scale, [0.0, 0.0], bicubic_enlarged);
}

/// [`place_scaled_xy`] with the text rule given ([`keeps_point`]).
pub(crate) fn place_scaled_in(d: &mut HudDraw, scale: [f32; 2], origin: [f32; 2], bicubic_enlarged: bool) {
    let ([sx, sy], [dx, dy]) = (scale, origin);
    let at = |p: &mut [f32; 2]| {
        p[0] = dx + p[0] * sx;
        p[1] = dy + p[1] * sy;
    };
    match d {
        HudDraw::Tile { x1, y1, x2, y2, point, enlarged, .. } => {
            *x1 = dx + *x1 * sx;
            *x2 = dx + *x2 * sx;
            *y1 = dy + *y1 * sy;
            *y2 = dy + *y2 * sy;
            *enlarged |= sx > 1.0 || sy > 1.0;
            if !keeps_point(sx, sy, bicubic_enlarged) {
                *point = false;
                // Enhanced graphics: an axis made smaller (a cell shrunk one way) is smoothed as
                // any smaller text, never read bicubic.
                if bicubic_enlarged {
                    *enlarged = false;
                }
            }
        }
        HudDraw::Triangle { p, .. } => p.iter_mut().for_each(at),
        HudDraw::Line { from, to, .. } => {
            at(from);
            at(to);
        }
        HudDraw::Portal { rect, .. } => {
            *rect = [dx + rect[0] * sx, dy + rect[1] * sy, rect[2] * sx, rect[3] * sy];
        }
    }
}

/// eMenuState.
const MSAT_BLURRY: u8 = 0;
const MSAT_WATCHED: u8 = 1;
pub(crate) const MSAT_PRESSED: u8 = 3;
const MSAT_DISABLED: u8 = 4;

/// _ftol: truncation towards zero, low 32 bits.
fn ftol(f: f32) -> i32 {
    (f as i64) as i32
}

/// FISTP dword (round to even) read back as a float: outside i32, infinity or NaN give the undefined
/// integer 0x80000000.
pub(crate) fn fistp_dword(x: f32) -> f32 {
    let r = x.round_ties_even();
    if (-2147483648.0..2147483648.0).contains(&r) {
        (r as i32) as f32
    } else {
        -2147483648.0
    }
}

/// Most derived virtual method according to the class (GUI.dll's tables).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GuiKind {
    Component,
    Button,
    GfxButton,
    Slider,
    ScrollZone,
    Image,
    Label,
    VertList,
    List,
    Multi,
    VertScrollBar,
    ListBoxBase,
    Page,
}

impl GuiKind {
    fn multi(self) -> bool {
        matches!(self, GuiKind::Multi | GuiKind::VertScrollBar | GuiKind::ListBoxBase | GuiKind::Page)
    }
}

impl Vm {
    pub(crate) fn gof_pub(&self, o: ObjId, name: &str) -> Option<ObjId> {
        self.gof(o, name)
    }

    fn gof(&self, o: ObjId, name: &str) -> Option<ObjId> {
        self.propr(o, name).as_object().filter(|&x| self.alive(x))
    }

    fn gfl(&self, o: ObjId, name: &str) -> f32 {
        self.propr(o, name).as_float()
    }

    fn gint(&self, o: ObjId, name: &str) -> i32 {
        self.propr(o, name).as_int()
    }

    fn gbool(&self, o: ObjId, name: &str) -> bool {
        self.propr(o, name).as_bool()
    }

    fn gbyte(&self, o: ObjId, name: &str) -> u8 {
        self.propr(o, name).as_byte()
    }

    fn objects_of(&self, o: ObjId, name: &str) -> Vec<Option<ObjId>> {
        match self.prop(o, name) {
            Value::Array(items) => items.iter().map(|v| v.as_object().filter(|&x| self.alive(x))).collect(),
            _ => Vec::new(),
        }
    }

    /// Platforms without a mouse (Android): the GUIController displaying a menu (non-empty
    /// MenuStack), with its cursor (MouseX, MouseY) and MenuMouseSens. The finger brings the cursor there through
    /// mouse movements, like DirectInput's mouse. `cache`: controller found earlier.
    pub fn cursor_menu(&self, cache: Option<ObjId>) -> Option<(ObjId, i32, i32, f32)> {
        let unlocked = |o: ObjId| self.alive(o) && self.is_a(o, "GUIController") && !self.objects_of(o, "MenuStack").is_empty();
        let ctrl = cache.filter(|&o| unlocked(o)).or_else(|| (0..self.objects.len()).find(|&o| self.objects[o].is_some() && unlocked(o)))?;
        Some((ctrl, self.gint(ctrl, "MouseX"), self.gint(ctrl, "MouseY"), self.gfl(ctrl, "MenuMouseSens")))
    }

    /// Platforms without a physical keyboard (Android): the text field that has the focus in the
    /// menu shown by `ctrl` (the FocusedControl chain from the ActivePage, the path the typed
    /// characters take in NativeKeyType), when it is an XIIIEditCtrl: on PC, that field takes the
    /// keyboard's characters (XIIIEditCtrl.InternalOnKeyType). Returns the field, its text
    /// (TextStr) and whether it is masked (bMaskText, passwords).
    pub fn menu_text_field(&self, ctrl: ObjId) -> Option<(ObjId, String, bool)> {
        let mut o = self.gof(ctrl, "ActivePage");
        for _ in 0..32 {
            let c = o?;
            if self.is_a(c, "XIIIEditCtrl") {
                if !self.gbool(c, "bVisible") {
                    return None;
                }
                return Some((c, self.propr(c, "TextStr").as_name(), self.gbool(c, "bMaskText")));
            }
            if !self.gui_kind(c).multi() {
                return None;
            }
            o = self.gof(c, "FocusedControl");
        }
        None
    }

    pub(crate) fn gui_kind(&self, o: ObjId) -> GuiKind {
        const ORDER: [(&str, GuiKind); 12] = [
            ("GUIPage", GuiKind::Page),
            ("GUIListBoxBase", GuiKind::ListBoxBase),
            ("GUIVertScrollBar", GuiKind::VertScrollBar),
            ("GUIMultiComponent", GuiKind::Multi),
            ("GUIList", GuiKind::List),
            ("GUIVertList", GuiKind::VertList),
            ("GUILabel", GuiKind::Label),
            ("GUIImage", GuiKind::Image),
            ("GUIVertScrollZone", GuiKind::ScrollZone),
            ("GUISlider", GuiKind::Slider),
            ("GUIGFXButton", GuiKind::GfxButton),
            ("GUIButton", GuiKind::Button),
        ];
        ORDER.iter().find(|(c, _)| self.is_a(o, c)).map_or(GuiKind::Component, |&(_, k)| k)
    }

    // ---- Delegates (UObject::ProcessDelegate) ----

    /// Delegate assigned to a live object (`*(int**)Delegate != 0 && !IsPendingKill`).
    pub(crate) fn gui_delegate_bound(&self, o: ObjId, name: &str) -> bool {
        matches!(self.prop(o, &format!("__{name}__Delegate")), Value::Delegate(Some(x), f) if self.alive(x) && !f.is_empty())
    }

    /// UObject::ProcessDelegate: the assigned function, otherwise the delegate's default
    /// body.
    pub(crate) fn gui_delegate(&mut self, o: ObjId, name: &str, args: Vec<Value>) -> (Value, Vec<Option<Value>>) {
        let key_tuple = format!("__{name}__Delegate");
        let (target, fname) = match self.prop(o, &key_tuple) {
            // IsPendingKill (vtable +0x38: object destroyed, bDeleteMe for an actor): the delegate is cleared, the call falls back on this.
            Value::Delegate(Some(x), _) if !self.alive(x) || self.native_prop(x, crate::NativeField::DeleteMe).as_bool() => {
                self.set_prop(o, &key_tuple, Value::Delegate(None, crate::value::NameStruct::EMPTY));
                (o, name.to_string())
            }
            Value::Delegate(Some(x), f) if !f.is_empty() => (x, f.to_string()),
            _ => (o, name.to_string()),
        };
        let Some(f) = self.find_function(target, &fname, false) else {
            return (Value::Void, Vec::new());
        };
        match self.call(f, target, args.into_iter().map(Some).collect()) {
            Ok(r) => r,
            Err(e) => {
                self.warn(format!("delegate {name}: {e}"));
                (Value::Void, Vec::new())
            }
        }
    }

    pub(crate) fn gui_event_pub(&mut self, o: ObjId, name: &str, args: Vec<Value>) -> Value {
        self.gui_event(o, name, args)
    }

    /// Script event of a component (eventMenuStateChange…).
    fn gui_event(&mut self, o: ObjId, name: &str, args: Vec<Value>) -> Value {
        let Some(f) = self.find_function(o, name, false) else { return Value::Void };
        match self.call(f, o, args.into_iter().map(Some).collect()) {
            Ok((v, _)) => v,
            Err(e) => {
                self.warn(format!("{name}: {e}"));
                Value::Void
            }
        }
    }

    // ---- Geometry (UGUIComponent::Actual*, UpdateBounds) ----

    /// Size of the player's viewport if Controller->Master->Client has a viewport, otherwise None.
    fn gui_viewport_size(&self, o: ObjId) -> Option<(i32, i32)> {
        let ctrl = self.gof(o, "Controller")?;
        self.gof(ctrl, "Master")?;
        self.local_viewport()?;
        let (w, h) = self.viewport_size();
        // Improved build: the whole window's menu page (`window_menu_frame`), also between two
        // frames (mouse events); inside a frame the size is already the page's and stays the same.
        // Split-screen menus keep their own frame (`split_menu_frame`).
        if w > 0 && h > 0 {
            if let Some(f) = self.window_menu_frame(w, h) {
                return Some(f.virtual_size);
            }
        }
        Some((w, h))
    }

    /// Geometry of `o` and of its MenuOwners (`Geo`, from it to the root).
    fn gui_string(&self, o: ObjId) -> Vec<Geo> {
        let mut out = Vec::new();
        let mut cur = Some(o);
        while let Some(c) = cur.filter(|_| out.len() < 64) {
            out.push(Geo {
                bound: self.gbool(c, "bBoundToParent"),
                scale: self.gbool(c, "bScaleToParent"),
                win: [self.gfl(c, "WinTop"), self.gfl(c, "WinLeft"), self.gfl(c, "WinWidth"), self.gfl(c, "WinHeight")],
            });
            cur = self.gof(c, "MenuOwner");
        }
        out
    }

    /// UGUIComponent::ActualWidth / ActualHeight.
    pub(crate) fn gui_actual_size(&self, o: ObjId, height: bool) -> f32 {
        let Some(screen) = self.gui_viewport_size(o) else { return 0.0 };
        actual_size(&self.gui_string(o), screen, height)
    }

    /// UGUIComponent::ActualLeft / ActualTop.
    pub(crate) fn gui_actual_pos(&self, o: ObjId, top: bool) -> f32 {
        let Some(screen) = self.gui_viewport_size(o) else { return 0.0 };
        actual_pos(&self.gui_string(o), screen, top)
    }

    pub(crate) fn set_float4(&mut self, o: ObjId, name: &str, v: [f32; 4]) {
        self.set_prop(o, name, Value::Array(v.iter().map(|&f| Value::Float(f)).collect()));
    }

    pub(crate) fn float4(&self, o: ObjId, name: &str) -> [f32; 4] {
        match self.prop(o, name) {
            Value::Array(a) if a.len() >= 4 => [a[0].as_float(), a[1].as_float(), a[2].as_float(), a[3].as_float()],
            _ => [0.0; 4],
        }
    }

    /// UGUIComponent::UpdateBounds: Bounds then ClientBounds (style borders).
    pub(crate) fn gui_update_bounds(&mut self, o: ObjId) {
        let left = self.gui_actual_pos(o, false);
        let top = self.gui_actual_pos(o, true);
        let l2 = self.gui_actual_pos(o, false);
        let right = self.gui_actual_size(o, false) + l2;
        let t2 = self.gui_actual_pos(o, true);
        let bottom = self.gui_actual_size(o, true) + t2;
        self.set_float4(o, "Bounds", [left, top, right, bottom]);
        let client = match self.gof(o, "Style") {
            Some(s) => {
                let b = match self.prop(s, "BorderOffsets") {
                    Value::Array(a) if a.len() >= 4 => [a[0].as_int(), a[1].as_int(), a[2].as_int(), a[3].as_int()],
                    _ => [0; 4],
                };
                [b[0] as f32 + left, b[1] as f32 + top, right - b[2] as f32, bottom - b[3] as f32]
            }
            None => [left, top, right, bottom],
        };
        self.set_float4(o, "ClientBounds", client);
    }

    /// UGUIComponent::CloneDims.
    #[allow(dead_code)]
    pub(crate) fn gui_clone_dims(&mut self, o: ObjId, from: Option<ObjId>) {
        let Some(f) = from else { return };
        let w = self.gui_actual_size(f, false);
        self.set_prop(o, "WinWidth", Value::Float(w));
        let h = self.gui_actual_size(f, true);
        self.set_prop(o, "WinHeight", Value::Float(h));
        let l = self.gui_actual_pos(f, false);
        self.set_prop(o, "WinLeft", Value::Float(l));
        let t = self.gui_actual_pos(f, true);
        self.set_prop(o, "WinTop", Value::Float(t));
    }

    // ---- Mouse (UnderCursor, PerformHitTest, SpecialHit) ----

    fn in_bounds(&self, o: ObjId, x: f32, y: f32) -> bool {
        let b = self.float4(o, "Bounds");
        b[0] <= x && x <= b[2] && b[1] <= y && y <= b[3]
    }

    /// PerformHitTest (vtable +0x74).
    pub(crate) fn gui_hit_test(&mut self, o: ObjId, x: i32, y: i32) -> bool {
        match self.gui_kind(o) {
            GuiKind::Multi | GuiKind::VertScrollBar | GuiKind::ListBoxBase => {
                // UGUIMultiComponent::PerformHitTest.
                if !self.gbool(o, "bVisible") {
                    return false;
                }
                let mut i = 0;
                while let Some(c) = self.objects_of(o, "Controls").get(i).copied() {
                    i += 1;
                    if let Some(c) = c {
                        if self.gui_hit_test(c, x, y) {
                            return true;
                        }
                    }
                }
                self.gui_component_hit_test(o, x, y)
            }
            // UGUIPage::PerformHitTest: the component's.
            _ => self.gui_component_hit_test(o, x, y),
        }
    }

    /// UGUIComponent::PerformHitTest.
    fn gui_component_hit_test(&mut self, o: ObjId, x: i32, y: i32) -> bool {
        if !self.gbool(o, "bVisible") || !self.gbool(o, "bAcceptsInput") || self.gbyte(o, "MenuState") == MSAT_DISABLED {
            return false;
        }
        if self.gui_delegate_bound(o, "OnHitTest") {
            self.gui_delegate(o, "OnHitTest", vec![Value::Float(x as f32), Value::Float(y as f32)]);
        }
        self.in_bounds(o, x as f32, y as f32)
    }

    /// UnderCursor (vtable +0x94).
    pub(crate) fn gui_under_cursor(&mut self, o: ObjId, x: f32, y: f32) -> Option<ObjId> {
        if self.gui_kind(o).multi() {
            // UGUIMultiComponent::UnderCursor: the control that has focus, then
            // the controls from last to first.
            if let Some(f) = self.gof(o, "FocusedControl") {
                if let Some(u) = self.gui_under_cursor(f, x, y) {
                    return Some(u);
                }
            }
            let mut i = self.objects_of(o, "Controls").len();
            while i >= 1 {
                if let Some(Some(c)) = self.objects_of(o, "Controls").get(i - 1).copied() {
                    if self.gbool(c, "bVisible") && self.gbyte(c, "MenuState") != MSAT_DISABLED {
                        if let Some(u) = self.gui_under_cursor(c, x, y) {
                            return Some(u);
                        }
                    }
                }
                i -= 1;
            }
        }
        // UGUIComponent::UnderCursor.
        if self.gbool(o, "bVisible") && self.gbyte(o, "MenuState") != MSAT_DISABLED && self.gui_hit_test(o, ftol(x), ftol(y)) {
            return Some(o);
        }
        None
    }

    /// SpecialHit (vtable +0xB8): MoveControl (moving a control with the mouse).
    #[allow(dead_code)]
    pub(crate) fn gui_special_hit(&mut self, o: ObjId) -> bool {
        if self.gui_component_special_hit(o) {
            return true;
        }
        if self.gui_kind(o).multi() {
            let mut i = 0;
            while let Some(c) = self.objects_of(o, "Controls").get(i).copied() {
                i += 1;
                if let Some(c) = c {
                    if self.gui_special_hit(c) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// UGUIComponent::SpecialHit.
    fn gui_component_special_hit(&mut self, o: ObjId) -> bool {
        let Some(ctrl) = self.gof(o, "Controller") else { return false };
        if !self.gbool(o, "bVisible") || self.gof(ctrl, "ActivePage") == Some(o) {
            return false;
        }
        let (mx, my) = (self.gint(ctrl, "MouseX") as f32, self.gint(ctrl, "MouseY") as f32);
        if !self.in_bounds(o, mx, my) {
            return false;
        }
        match self.gof(ctrl, "MoveControl") {
            Some(m) if m == o => {
                self.set_prop(ctrl, "MoveControl", Value::Object(None));
                false
            }
            None => {
                self.set_prop(ctrl, "MoveControl", Value::Object(Some(o)));
                true
            }
            _ => false,
        }
    }

    /// MouseMove (vtable +0x98): UGUIComponent::MouseMove.
    pub(crate) fn gui_mouse_move(&mut self, o: ObjId, dx: i32, dy: i32) -> bool {
        if self.gbyte(o, "MenuState") == MSAT_PRESSED && self.gbool(o, "bCaptureMouse") {
            return self.gui_delegate(o, "OnCapturedMouseMove", vec![Value::Float(dx as f32), Value::Float(dy as f32)]).0.as_bool();
        }
        false
    }

    /// UGUIController::PlayInterfaceSound, intentionally empty.
    pub(crate) fn gui_play_interface_sound(&mut self, _ctrl: ObjId, _sound: &str) {
    }

    /// MouseHover (vtable +0xA4): UGUIComponent::MouseHover, no override.
    pub(crate) fn gui_mouse_hover(&mut self, _o: ObjId) -> bool {
        false
    }

    /// NativeInvalidate (vtable +0xBC).
    #[allow(dead_code)]
    pub(crate) fn gui_invalidate(&mut self, o: ObjId) {
        self.gui_delegate(o, "OnInvalidate", vec![]);
        if self.gui_kind(o).multi() {
            if let Some(f) = self.gof(o, "FocusedControl") {
                self.gui_invalidate(f);
            }
        }
    }

    // ---- Keyboard ----

    /// NativeKeyEvent (vtable +0xAC).
    pub(crate) fn gui_key_event(&mut self, o: ObjId, key: &mut u8, action: &mut u8, delta: f32) -> bool {
        if self.gui_kind(o).multi() {
            // UGUIMultiComponent::NativeKeyEvent; UGUIPage::NativeKeyEvent
            // only calls it.
            if let Some(f) = self.gof(o, "FocusedControl") {
                if self.gui_key_event(f, key, action, delta) {
                    return true;
                }
            }
            if self.gbyte(o, "MenuState") != 2 {
                return false;
            }
        }
        self.gui_component_key_event(o, key, action, delta)
    }

    /// UGUIComponent::NativeKeyEvent: Ctrl+Tab changes page, otherwise OnKeyEvent.
    fn gui_component_key_event(&mut self, o: ObjId, key: &mut u8, action: &mut u8, delta: f32) -> bool {
        if *key == 9 {
            if let Some(ctrl) = self.gof(o, "Controller") {
                if self.gbool(ctrl, "CtrlPressed") && (*action == IST_PRESS || *action == IST_HOLD) {
                    let ev = if self.gbool(ctrl, "ShiftPressed") { "PrevPage" } else { "NextPage" };
                    if self.gui_event(o, ev, vec![]).as_bool() {
                        return true;
                    }
                }
            }
        }
        let (r, outs) = self.gui_delegate(o, "OnKeyEvent", vec![Value::Byte(*key), Value::Byte(*action), Value::Float(delta)]);
        if let Some(Some(k)) = outs.first() {
            *key = k.as_byte();
        }
        if let Some(Some(a)) = outs.get(1) {
            *action = a.as_byte();
        }
        r.as_bool()
    }

    /// NativeKeyType (vtable +0xA8).
    pub(crate) fn gui_key_type(&mut self, o: ObjId, key: &mut u8) -> bool {
        if self.gui_kind(o).multi() {
            // UGUIMultiComponent::NativeKeyType.
            if let Some(f) = self.gof(o, "FocusedControl") {
                if self.gui_key_type(f, key) {
                    return true;
                }
            }
            if self.gbyte(o, "MenuState") != 2 {
                return false;
            }
        }
        // UGUIComponent::NativeKeyType: Tab changes control, otherwise OnKeyType.
        if *key == 9 && !self.gbool(o, "bCaptureTabs") {
            let shift = self.gof(o, "Controller").is_some_and(|c| self.gbool(c, "ShiftPressed"));
            self.gui_event(o, if shift { "PrevControl" } else { "NextControl" }, vec![Value::Object(None)]);
            return true;
        }
        let (r, outs) = self.gui_delegate(o, "OnKeyType", vec![Value::Byte(*key)]);
        if let Some(Some(k)) = outs.first() {
            *key = k.as_byte();
        }
        r.as_bool()
    }

    // ---- Drawing ----

    /// UGUIComponent::SaveCanvasState.
    fn gui_save_canvas(&mut self, o: ObjId, c: ObjId) {
        for (from, to) in [("CurX", "SaveX"), ("CurY", "SaveY"), ("DrawColor", "SaveColor"), ("Style", "SaveStyle"), ("Font", "SaveFont")] {
            let v = self.prop(c, from);
            self.set_prop(o, to, v);
        }
    }

    /// UGUIComponent::RestoreCanvasState.
    fn gui_restore_canvas(&mut self, o: ObjId, c: ObjId) {
        for (to, from) in [("CurX", "SaveX"), ("CurY", "SaveY"), ("DrawColor", "SaveColor"), ("Style", "SaveStyle"), ("Font", "SaveFont")] {
            let v = self.prop(o, from);
            self.set_prop(c, to, v);
        }
    }

    /// PreDraw (vtable +0x6C).
    pub(crate) fn gui_pre_draw(&mut self, o: ObjId, c: ObjId) {
        self.gui_component_pre_draw(o, c);
        if self.gui_kind(o).multi() {
            // UGUIMultiComponent::PreDraw.
            let mut i = 0;
            while let Some(ctl) = self.objects_of(o, "Controls").get(i).copied() {
                i += 1;
                if let Some(ctl) = ctl {
                    self.gui_pre_draw(ctl, c);
                }
            }
        }
    }

    /// UGUIComponent::PreDraw.
    fn gui_component_pre_draw(&mut self, o: ObjId, c: ObjId) {
        self.gui_update_bounds(o);
        if self.gui_delegate_bound(o, "OnPreDraw") {
            self.gui_delegate(o, "OnPreDraw", vec![Value::Object(Some(c))]);
        }
    }

    /// Draw (vtable +0x70).
    pub(crate) fn gui_draw(&mut self, o: ObjId, c: ObjId) {
        match self.gui_kind(o) {
            GuiKind::Page => self.gui_page_draw(o, c),
            GuiKind::Multi | GuiKind::VertScrollBar | GuiKind::ListBoxBase => self.gui_multi_draw(o, c),
            GuiKind::Label => self.gui_label_draw(o, c),
            GuiKind::Image => self.gui_image_draw(o, c),
            GuiKind::Button => self.gui_button_draw(o, c),
            GuiKind::ScrollZone => self.gui_scroll_zone_draw(o, c),
            _ => self.gui_component_draw(o, c),
        }
    }

    /// Height, width, top, left: the call order of the Actual* functions before a drawing.
    fn gui_htlw(&self, o: ObjId) -> (f32, f32, f32, f32) {
        let h = self.gui_actual_size(o, true);
        let w = self.gui_actual_size(o, false);
        let t = self.gui_actual_pos(o, true);
        let l = self.gui_actual_pos(o, false);
        (h, w, t, l)
    }

    fn color_plane(&self, v: &Value) -> [f32; 4] {
        crate::canvas::fplane(crate::canvas::bgra(v))
    }

    /// UGUIButton::Draw: style image, then the centred text.
    fn gui_button_draw(&mut self, o: ObjId, c: ObjId) {
        let Some(style) = self.gof(o, "Style").filter(|_| self.gbool(o, "bVisible")) else { return };
        self.gui_component_draw(o, c);
        let state = self.gbyte(o, "MenuState");
        let (h, w, t, l) = self.gui_htlw(o);
        self.gui_style_draw(style, c, state, l, t, w, h);
        let caption = self.propr(o, "Caption").as_name();
        let (h, w, t, l) = self.gui_htlw(o);
        self.gui_style_draw_text(style, c, state, l, t, w, h, 1, &caption);
    }

    /// UGUIVertScrollZone::Draw: the style image, if the zone is larger than one pixel.
    fn gui_scroll_zone_draw(&mut self, o: ObjId, c: ObjId) {
        let Some(style) = self.gof(o, "Style").filter(|_| self.gbool(o, "bVisible")) else { return };
        if self.gui_actual_size(o, false) <= 1.0 || self.gui_actual_size(o, true) <= 1.0 {
            return;
        }
        self.gui_component_draw(o, c);
        let state = self.gbyte(o, "MenuState");
        let (h, w, t, l) = self.gui_htlw(o);
        self.gui_style_draw(style, c, state, l, t, w, h);
    }

    /// UGUILabel::Draw: background (DefaultPens[0]) unless bTransparent, then the text
    /// (one line, or split into lines by "|" if bMultiLine), through the style if there is one.
    fn gui_label_draw(&mut self, o: ObjId, c: ObjId) {
        if !self.gbool(o, "bVisible") {
            return;
        }
        self.gui_component_draw(o, c);
        let Some(ctrl) = self.gof(o, "Controller") else { return };
        let font_name = self.propr(o, "TextFont").as_name();
        let Some(gfont) = self.gui_event(ctrl, "GetMenuFont", vec![Value::Str(font_name)]).as_object().filter(|&f| self.alive(f)) else {
            return;
        };
        let size_x = self.propr(c, "SizeX").as_int();
        let font = self.gui_event(gfont, "GetFont", vec![Value::Int(size_x)]);
        if !font.as_bool() {
            return;
        }
        self.set_prop(c, "Font", font);
        if !self.gbool(o, "bTransparent") {
            let color = self.color_plane(&self.prop(o, "BackColor"));
            let (h, w, t, l) = self.gui_htlw(o);
            let pen = match self.prop(ctrl, "DefaultPens") {
                Value::Array(p) => p.first().cloned().unwrap_or(Value::Object(None)),
                _ => Value::Object(None),
            };
            self.canvas_draw_tile(c, &pen, l, t, w, h, 0.0, 0.0, 32.0, 32.0, 0.0, color, [0.0; 4]);
        }
        let style = self.gof(o, "Style");
        let align = self.gbyte(o, "TextAlign");
        let caption = self.propr(o, "Caption").as_name();
        let round = fistp_dword;
        if self.gbool(o, "bMultiLine") {
            let cb = self.float4(o, "ClientBounds");
            let x = cb[0];
            let mut y = cb[1];
            let w = cb[2] - cb[0];
            let bottom = cb[3];
            let (_, line_h) = self.canvas_clipped_str_len(c, &Value::Object(None), 1.0, 1.0, "W");
            let lines = self.canvas_wrap_string_to_array(c, &caption, w, &Value::Object(None), '|');
            if style.is_none() {
                let tc = self.prop(o, "TextColor");
                self.set_prop(c, "DrawColor", tc);
                let ts = self.gbyte(o, "TextStyle");
                self.set_prop(c, "Style", Value::Byte(ts));
            }
            let state = self.gbyte(o, "MenuState");
            for line in &lines {
                match style {
                    None => {
                        let y2 = round(line_h as f32 + y);
                        let x2 = round(w + x);
                        self.canvas_draw_text_justified(c, align, round(x), round(y), x2, y2, line);
                    }
                    Some(s) => self.gui_style_draw_text(s, c, state, x, y, w, line_h as f32, align, line),
                }
                y += line_h as f32;
                if bottom <= y + line_h as f32 {
                    break;
                }
            }
            return;
        }
        if let Some(s) = style {
            let state = self.gbyte(o, "MenuState");
            let (h, w, t, l) = self.gui_htlw(o);
            self.gui_style_draw_text(s, c, state, l, t, w, h, align, &caption);
            return;
        }
        let tc = self.prop(o, "TextColor");
        self.set_prop(c, "DrawColor", tc);
        let ts = self.gbyte(o, "TextStyle");
        self.set_prop(c, "Style", Value::Byte(ts));
        let h = self.gui_actual_size(o, true);
        let t = self.gui_actual_pos(o, true);
        let y2 = round(h + t);
        let w = self.gui_actual_size(o, false);
        let l = self.gui_actual_pos(o, false);
        let x2 = round(l + w);
        let y1 = round(self.gui_actual_pos(o, true));
        let x1 = round(self.gui_actual_pos(o, false));
        self.canvas_draw_text_justified(c, align, x1, y1, x2, y2, &caption);
    }

    /// UGUIImage::Draw depending on ImageStyle: normal (cut DimX1…DimY2, centred
    /// if ImageAlign = 1), stretched, ScaleBound, Bound or Justified.
    fn gui_image_draw(&mut self, o: ObjId, c: ObjId) {
        let image = self.prop(o, "Image");
        if !self.gbool(o, "bVisible") || !image.as_bool() {
            return;
        }
        self.gui_component_draw(o, c);
        let rs = self.gbyte(o, "ImageRenderStyle");
        self.set_prop(c, "Style", Value::Byte(rs));
        let ic = self.prop(o, "ImageColor");
        self.set_prop(c, "DrawColor", ic);
        let (dx1, dy1, dx2, dy2) = (self.gint(o, "DimX1"), self.gint(o, "DimY1"), self.gint(o, "DimX2"), self.gint(o, "DimY2"));
        let align = self.gbyte(o, "ImageAlign");
        // The smaller one (the first one on a tie).
        let min = |a: f32, b: f32| if a <= b { a } else { b };
        match self.gbyte(o, "ImageStyle") {
            0 => {
                let u = if dx1 == -1 { 0.0 } else { dx1 as f32 };
                let v = if dy1 == -1 { 0.0 } else { dy1 as f32 };
                let ul = if dx2 == -1 {
                    let us = self.material_size(&image).0;
                    min(self.gui_actual_size(o, false), us as f32)
                } else {
                    (dx2 - dx1) as f32
                };
                let vl = if dy2 == -1 {
                    let vs = self.material_size(&image).1;
                    min(self.gui_actual_size(o, true), vs as f32)
                } else {
                    (dy2 - dy1) as f32
                };
                let color = self.color_plane(&self.prop(c, "DrawColor"));
                let t = self.gui_actual_pos(o, true);
                let l = self.gui_actual_pos(o, false);
                let (x, y) = if align == 1 { (l - ul * 0.5, t - vl * 0.5) } else { (l, t) };
                self.canvas_draw_tile(c, &image, x, y, ul, vl, u, v, ul, vl, 0.0, color, [0.0; 4]);
            }
            1 => {
                if dx1 < 0 && dx2 < 0 && dy1 < 0 && dy2 < 0 {
                    let (h, w, t, l) = self.gui_htlw(o);
                    self.canvas_draw_tile_stretched(c, &image, l, t, w, h);
                    return;
                }
                // The original code takes DimX1 for U and V, and tests DimY2 for UL.
                let u = if dx1 == -1 { 0.0 } else { dx1 as f32 };
                let v = u;
                let ul = if dy2 == -1 {
                    let us = self.material_size(&image).0;
                    min(self.gui_actual_size(o, false), us as f32)
                } else {
                    (dx2 - dx1) as f32
                };
                let vl = if dy2 == -1 {
                    let vs = self.material_size(&image).1;
                    min(self.gui_actual_size(o, true), vs as f32)
                } else {
                    (dy2 - dy1) as f32
                };
                let color = self.color_plane(&self.prop(c, "DrawColor"));
                let (h, w, t, l) = self.gui_htlw(o);
                let (x, y) = if align == 1 { (l - ul * 0.5, t - vl * 0.5) } else { (l, t) };
                self.canvas_draw_tile(c, &image, x, y, w, h, u, v, ul, vl, 0.0, color, [0.0; 4]);
            }
            2 => {
                let (h, w, t, l) = self.gui_htlw(o);
                self.canvas_draw_tile_scale_bound(c, &image, l, t, w, h);
            }
            3 => {
                let (h, w, t, l) = self.gui_htlw(o);
                self.canvas_draw_tile_bound(c, &image, l, t, w, h);
            }
            4 => {
                let (h, w, t, l) = self.gui_htlw(o);
                self.canvas_draw_tile_justified(c, &image, l, t, w, h, align);
            }
            _ => {}
        }
    }

    /// UGUIComponent::Draw: position, colour, style and style font, OnDraw.
    fn gui_component_draw(&mut self, o: ObjId, c: ObjId) {
        if !self.gbool(o, "bVisible") {
            return;
        }
        let x = self.gui_actual_pos(o, false);
        self.set_prop(c, "CurX", Value::Float(x));
        let y = self.gui_actual_pos(o, true);
        self.set_prop(c, "CurY", Value::Float(y));
        if let Some(style) = self.gof(o, "Style") {
            let state = self.gbyte(o, "MenuState") as usize;
            if let Value::Array(colors) = self.prop(style, "FontColors") {
                if let Some(col) = colors.get(state) {
                    self.set_prop(c, "DrawColor", col.clone());
                }
            }
            if let Value::Array(styles) = self.prop(style, "RStyles") {
                if let Some(s) = styles.get(state) {
                    self.set_prop(c, "Style", Value::Byte(s.as_byte()));
                }
            }
            let font = match self.prop(style, "Fonts") {
                Value::Array(f) => f.get(state).and_then(|v| v.as_object()),
                _ => None,
            };
            if let Some(font) = font.filter(|&f| self.alive(f)) {
                let xres = self.viewport_size().0;
                let f = self.gui_event(font, "GetFont", vec![Value::Int(xres)]);
                self.set_prop(c, "Font", f);
            }
        }
        if self.gui_delegate_bound(o, "OnDraw") {
            self.gui_delegate(o, "OnDraw", vec![Value::Object(Some(c))]);
        }
    }

    /// UGUIMultiComponent::Draw: the controls, the one that has focus last.
    fn gui_multi_draw(&mut self, o: ObjId, c: ObjId) {
        if !self.gbool(o, "bVisible") {
            return;
        }
        self.gui_component_draw(o, c);
        let focused = self.propr(o, "FocusedControl").as_object();
        let mut last: Option<usize> = None;
        let mut i = 0;
        while let Some(ctl) = self.objects_of(o, "Controls").get(i).copied() {
            if let Some(ctl) = ctl {
                if Some(ctl) != focused {
                    self.gui_save_canvas(ctl, c);
                    self.gui_draw(ctl, c);
                    self.gui_restore_canvas(ctl, c);
                } else {
                    last = Some(i);
                }
            }
            i += 1;
        }
        if let Some(i) = last {
            if let Some(Some(ctl)) = self.objects_of(o, "Controls").get(i).copied() {
                self.gui_save_canvas(ctl, c);
                self.gui_draw(ctl, c);
                self.gui_restore_canvas(ctl, c);
            }
        }
    }

    /// UGUIPage::Draw: background (DrawTileScaleBound), controls, OnRender.
    fn gui_page_draw(&mut self, o: ObjId, c: ObjId) {
        if !self.gbool(o, "bVisible") {
            return;
        }
        self.gui_save_canvas(o, c);
        let background = self.prop(o, "Background");
        if background.as_bool() {
            let color = self.prop(o, "BackgroundColor");
            self.set_prop(c, "DrawColor", color);
            let style = self.gbyte(o, "BackgroundRStyle");
            self.set_prop(c, "Style", Value::Byte(style));
            let h = self.gui_actual_size(o, true);
            let w = self.gui_actual_size(o, false);
            let t = self.gui_actual_pos(o, true);
            let l = self.gui_actual_pos(o, false);
            self.canvas_draw_tile_scale_bound(c, &background, l, t, w, h);
        }
        self.gui_multi_draw(o, c);
        self.gui_delegate(o, "OnRender", vec![Value::Object(Some(c))]);
        self.gui_restore_canvas(o, c);
    }

    // ---- GUIPage: timers and state code ----

    /// UGUIPage::UpdateTimers.
    fn gui_update_timers(&mut self, page: ObjId, dt: f32) {
        let mut i: i32 = 0;
        let mut n = self.objects_of(page, "Timers").len() as i32;
        while i < n {
            let Some(Some(t)) = self.objects_of(page, "Timers").get(i as usize).copied() else {
                i += 1;
                continue;
            };
            self.set_prop(t, "TimerIndex", Value::Int(i));
            let countdown = self.gfl(t, "TimerCountdown") - dt;
            self.set_prop(t, "TimerCountdown", Value::Float(countdown));
            if countdown <= 0.0 && self.gfl(t, "TimerInterval") > 0.0 {
                // IsProbing(NAME_Timer): the Timer event only if it is not ignored.
                if !self.probe_blocked(t, "Timer") {
                    self.gui_event(t, "Timer", vec![]);
                }
                if !self.gbool(t, "bTimerRepeat") {
                    self.set_prop(t, "TimerInterval", Value::Float(0.0));
                    self.set_prop(t, "TimerCountdown", Value::Float(0.0));
                } else {
                    let c = self.gfl(t, "TimerInterval") + self.gfl(t, "TimerCountdown");
                    self.set_prop(t, "TimerCountdown", Value::Float(c));
                    if c <= 0.0 {
                        let iv = self.prop(t, "TimerInterval");
                        self.set_prop(t, "TimerCountdown", iv);
                    }
                }
            }
            if self.gfl(t, "TimerInterval") <= 0.0 {
                self.set_prop(t, "TimerIndex", Value::Int(-1));
                if let Value::Array(mut items) = self.prop(page, "Timers") {
                    if (i as usize) < items.len() {
                        items.remove(i as usize);
                    }
                    self.set_prop(page, "Timers", Value::Array(items));
                }
                i -= 1;
                n -= 1;
            }
            i += 1;
        }
    }

    // ---- GUIController ----

    fn page_at(&self, ctrl: ObjId, i: usize) -> Option<ObjId> {
        self.objects_of(ctrl, "MenuStack").get(i).copied().flatten()
    }

    /// First page drawn: from the last towards the first as long as it does not hide the
    /// previous ones (bHidePreviousPage).
    fn first_shown_page(&self, ctrl: ObjId) -> Option<usize> {
        let n = self.objects_of(ctrl, "MenuStack").len();
        if n == 0 {
            return None;
        }
        let mut i = n - 1;
        while i > 0 {
            match self.page_at(ctrl, i) {
                Some(p) if !self.gbool(p, "bHidePreviousPage") => i -= 1,
                _ => break,
            }
        }
        Some(i)
    }

    /// Local split screen (OpenXIII addition): the menu of a viewport other than the first
    /// player's. The window's mouse is the first player's (keyboard and mouse, windrv_input.rs
    /// update_input), the others play with a gamepad: their menu takes no mouse position, moves or
    /// clicks and draws no cursor, so the mouse player can never act in another player's menu.
    /// False outside split screen: the engine's behaviour. False in a lockstep co-op session too:
    /// there every player is alone at their own machine, whose window's mouse reaches their own
    /// viewport through their commands (lockstep.rs, `PlayerCmd::wmouse`) on every machine, so
    /// each player's menu takes their own mouse and shows their own cursor.
    pub(crate) fn gui_without_mouse(&self, ctrl: ObjId) -> bool {
        if (self.lockstep.is_some()) || !self.local_split() || self.engine.client.viewports.len() < 2 {
            return false;
        }
        let Some(owner) = self.gof(ctrl, "ViewportOwner") else { return false };
        let first = self.coop.players.first().and_then(|&pc| self.gof(pc, "Player"));
        first.is_some_and(|v| v != owner)
    }

    /// Local split screen with one menu per viewport (not a cinematic or menu over the whole
    /// window): the split-screen menu frame (`split_menu_frame`) applies instead of the whole
    /// window's 4:3 frame (`menu_frame`).
    fn split_screen_menus(&self) -> bool {
        self.local_split() && self.engine.client.viewports.len() >= 2 && self.engine.client.extended.is_none()
    }

    /// Local split screen (OpenXIII addition): the menus are laid out for a 640 × 480 screen
    /// (XIIIWindow: fRatioX = ClipX / 640 capped at 1.25, fRatioY = ClipY / 480; fonts chosen from
    /// the width), so in a half of the window they were squashed in height and partly cut. A menu of
    /// a split-screen viewport lays itself out on a virtual 4:3 screen and its whole picture (boxes,
    /// text, cursor) is scaled uniformly into the 4:3 frame centred in that viewport: nothing can
    /// overflow that does not overflow on a 4:3 screen, the game is still seen on the sides. The
    /// virtual screen is the frame itself from 600 pixels high, otherwise 800 × 600 (scale = frame
    /// height / 600, see MenuFrame::of_half). None: full screen (solo, a cinematic or menu over the
    /// whole window).
    pub(crate) fn split_menu_frame(&self, c: ObjId) -> Option<MenuFrame> {
        if !self.split_screen_menus() {
            return None;
        }
        // Improved build: the co-op character screen's page covers each half itself and scales
        // itself to it (character_page.rs); its live view is drawn in the half's own pixels.
        if edition::character_screen_open(self) {
            return None;
        }
        // A half larger than the reference (a 4K window): the menus at their reference share, as on
        // a whole window; smaller halves keep the 800 × 600 page (never smaller than the original).
        let (cx, cy) = (ftol(self.gfl(c, "ClipX")), ftol(self.gfl(c, "ClipY")));
        MenuFrame::of_window(cx, cy, self.menu_layout.ui_scale).or_else(|| MenuFrame::of_half(cx, cy)).map(|mut frame| {
            frame.bicubic = enlarged_text_bicubic(self.engine.client.session_graphics);
            frame
        })
    }

    /// Improved build, a whole window's menus (display_format.rs) on a `clip_x` × `clip_y`
    /// canvas: the menus are laid out on a 640 × 480 grid (XIIIWindow.BeforePaint: fRatioX =
    /// ClipX / 640, fRatioY = ClipY / 480, capped at 800 × 600 for the map menus and bCenterInGame
    /// pages), so they were stretched on a window that is not 4:3 and small on a large one. From a
    /// 4:3 frame higher than 600 pixels they are drawn on the 800 × 600 page scaled into it
    /// (MenuFrame::of_window, times UIScale); on a smaller window that is not 4:3, in the largest
    /// 4:3 frame centred in the canvas as if the screen were that frame (scale 1). None: a 4:3
    /// canvas up to 600 pixels high, faithful build, or split-screen menus (`split_menu_frame`).
    pub fn window_menu_frame(&self, clip_x: i32, clip_y: i32) -> Option<MenuFrame> {
        if !crate::display_format::active(self.engine.client.session_graphics) || self.split_screen_menus() {
            return None;
        }
        // Improved build: the character screen's page covers the whole view itself (as above).
        if edition::character_screen_open(self) {
            return None;
        }
        if let Some(mut f) = MenuFrame::of_window(clip_x, clip_y, self.menu_layout.ui_scale) {
            f.bicubic = true;
            return Some(f);
        }
        let [x, y, w, h] = menu_frame_43(clip_x, clip_y);
        (x != 0 || y != 0).then_some(MenuFrame { bicubic: true, x: x as f32, y: y as f32, scale: 1.0, virtual_size: (w, h) })
    }

    /// Where a page drawn outside the scripts goes on a `w` × `h` window (see MenuScreen): the
    /// whole window's menu frame as at the last GUI frame (its UIScale).
    pub fn menu_screen(&self, w: u32, h: u32) -> MenuScreen {
        let frame = self.window_menu_frame(w as i32, h as i32);
        let size = frame.map_or((w, h), |f| (f.virtual_size.0.max(1) as u32, f.virtual_size.1.max(1) as u32));
        MenuScreen { frame, size, canvas: (w as f32, h as f32) }
    }

    /// Puts the canvas and the GUI's screen size on the menu's screen (the split-screen menu's
    /// virtual screen, or the whole window's page); what to put back after. The menu draws in
    /// that screen's pixels (the GUI controls place themselves from the screen size without the
    /// canvas origin, the scripts with it at 0): its primitives are scaled and moved into the frame
    /// afterwards (`menu_frame_end`).
    fn menu_frame_begin(&mut self, c: ObjId) -> Option<MenuFrameState> {
        let (frame, whole_window) = match self.split_menu_frame(c) {
            Some(f) => (f, false),
            None => (self.window_menu_frame(ftol(self.gfl(c, "ClipX")), ftol(self.gfl(c, "ClipY")))?, true),
        };
        let (w, h) = frame.virtual_size;
        let names = ["ClipX", "ClipY", "SizeX", "SizeY"];
        let saved: Vec<Value> = names.iter().map(|n| self.prop(c, n)).collect();
        self.set_prop(c, "ClipX", Value::Float(w as f32));
        self.set_prop(c, "ClipY", Value::Float(h as f32));
        self.set_prop(c, "SizeX", Value::Int(w));
        self.set_prop(c, "SizeY", Value::Int(h));
        let screen = std::mem::replace(&mut self.viewport, (w, h));
        // Improved build: the in-game menus' bands to the window's edges (wide_menu.rs), and to
        // a split-screen player's view edges as well.
        let wide = edition::begin_wide_menu(self, &frame, saved[0].as_float(), whole_window);
        Some(MenuFrameState { saved, screen, frame, start: self.hud.len(), whole_window, wide })
    }

    fn menu_frame_end(&mut self, c: ObjId, state: Option<MenuFrameState>) {
        let Some(MenuFrameState { saved, screen, frame, start, whole_window: _, wide }) = state else { return };
        for (n, v) in ["ClipX", "ClipY", "SizeX", "SizeY"].iter().zip(saved) {
            self.set_prop(c, n, v);
        }
        self.viewport = screen;
        let start = start.min(self.hud.len());
        if wide {
            edition::end_wide_menu(self, start);
            // Whole window (or a player's view): a backdrop covering the whole page covers the
            // whole canvas.
            let canvas = (self.gfl(c, "ClipX"), self.gfl(c, "ClipY"));
            frame.place_page(&mut self.hud[start..], canvas);
        } else {
            for d in self.hud.iter_mut().skip(start) {
                frame.place(d);
            }
        }
    }

    /// UGUIController::NativePreRender.
    pub(crate) fn gui_controller_pre_render(&mut self, ctrl: ObjId, c: ObjId) {
        // UIScale of the improved build's menus, read once per frame (the post-render and the
        // pages drawn outside the scripts use the same value).
        if crate::display_format::active(self.engine.client.session_graphics) {
            self.menu_layout.ui_scale = self.ui_scale();
        }
        let state = self.menu_frame_begin(c);
        // The whole window's frame only: the GUI's MouseX / MouseY are on its page (canvas.rs,
        // cursor_menu_draw).
        self.menu_layout.last_frame = state.as_ref().filter(|s| s.whole_window).map(|s| s.frame);
        // Improved build: the menus' characters at the size they were last shown (menu_figures.rs).
        edition::prepare_menu_figures(self);
        self.gui_controller_pre_render_in(ctrl, c, state.as_ref().map(|s| s.frame));
        // Improved build: the help bar's side under the cursor (help_bar_click.rs).
        edition::update_menu_hover(self, ctrl);
        self.menu_frame_end(c, state);
    }

    fn gui_controller_pre_render_in(&mut self, ctrl: ObjId, c: ObjId, frame: Option<MenuFrame>) {
        let Some(player) = self.gof(ctrl, "ViewportOwner") else { return };
        if self.gui_without_mouse(ctrl) {
            self.set_prop(player, "bShowWindowsMouse", Value::Bool(false));
        } else if self.gbool(player, "bWindowsMouseAvailable") {
            let (mx, my) = (self.prop(ctrl, "MouseX"), self.prop(ctrl, "MouseY"));
            self.set_prop(ctrl, "LastMouseX", mx);
            self.set_prop(ctrl, "LastMouseY", my);
            // Menu frame: the mouse brought into the menu's screen (whole pixels shifted for the
            // unscaled 4:3 frame, scaled back for a split-screen menu).
            let (wx, wy) = (self.gfl(player, "WindowsMouseX"), self.gfl(player, "WindowsMouseY"));
            let (x, y) = match frame {
                None => (ftol(wx), ftol(wy)),
                Some(f) if f.scale == 1.0 => (ftol(wx) - f.x as i32, ftol(wy) - f.y as i32),
                Some(f) => {
                    let (x, y) = f.unplace(wx, wy);
                    (ftol(x), ftol(y))
                }
            };
            let (old_x, old_y) = (self.gint(ctrl, "MouseX"), self.gint(ctrl, "MouseY"));
            self.set_prop(ctrl, "MouseX", Value::Int(x));
            self.set_prop(ctrl, "MouseY", Value::Int(y));
            self.set_prop(player, "bShowWindowsMouse", Value::Bool(true));
            edition::track_cursor_motion(self, ctrl, (old_x, old_y), (x, y));
        } else {
            self.set_prop(player, "bShowWindowsMouse", Value::Bool(false));
        }
        let Some(mut i) = self.first_shown_page(ctrl) else { return };
        while i < self.objects_of(ctrl, "MenuStack").len() {
            let Some(p) = self.page_at(ctrl, i) else {
                self.log.push("MenuStack out of sync".into());
                return;
            };
            self.gui_pre_draw(p, c);
            i += 1;
        }
    }

    /// UGUIController::NativePostRender: pages, then the mouse cursor.
    pub(crate) fn gui_controller_post_render(&mut self, ctrl: ObjId, c: ObjId) {
        let frame = self.menu_frame_begin(c);
        edition::help_bar_frame(self, ctrl, false);
        self.gui_controller_post_render_in(ctrl, c);
        edition::help_bar_frame(self, ctrl, true);
        self.menu_frame_end(c, frame);
    }

    fn gui_controller_post_render_in(&mut self, ctrl: ObjId, c: ObjId) {
        let Some(player) = self.gof(ctrl, "ViewportOwner") else { return };
        self.set_prop(c, "CurX", Value::Float(0.0));
        self.set_prop(c, "Style", Value::Byte(1));
        if let Some(mut i) = self.first_shown_page(ctrl) {
            while i < self.objects_of(ctrl, "MenuStack").len() {
                let Some(p) = self.page_at(ctrl, i) else {
                    self.log.push("MenuStack out of sync".into());
                    break;
                };
                self.gui_draw(p, c);
                i += 1;
            }
        }
        let active = self.gof(ctrl, "ActiveControl");
        if self.gui_without_mouse(ctrl) {
            return;
        }
        if self.gbool(player, "bWindowsMouseAvailable") {
            if let Some(a) = active {
                let idx = self.gint(a, "MouseCursorIndex");
                self.set_prop(player, "SelectedCursor", Value::Byte(idx as u8));
            }
            return;
        }
        let index = active.map_or(0, |a| self.gint(a, "MouseCursorIndex")).max(0) as usize;
        let cursor = match self.prop(ctrl, "MouseCursors") {
            Value::Array(m) => m.get(index).cloned().unwrap_or(Value::Object(None)),
            _ => Value::Object(None),
        };
        let offset = match self.prop(ctrl, "MouseCursorOffset") {
            Value::Array(m) => m.get(index).map(|v| v.as_vector()).unwrap_or([0.0; 3]),
            _ => [0.0; 3],
        };
        if !cursor.as_bool() {
            return;
        }
        self.set_prop(c, "DrawColor", Value::Struct("object.color".into(), vec![Value::Byte(255); 4].into()));
        let sx = self.gint(c, "SizeX");
        let sy = self.gint(c, "SizeY");
        self.set_prop(c, "Style", Value::Byte(5));
        let scale = if sx < 640 { 1.0 } else { (sx / 640) as f32 };
        let (old_x, old_y) = (self.gint(ctrl, "MouseX"), self.gint(ctrl, "MouseY"));
        // Improved build: the in-game help bar runs to the window's edges (wide_menu.rs), beyond
        // the page the cursor is kept in: it goes as far as the bar's sides (help_bar_click.rs).
        let (lo, hi) = edition::cursor_range(self, sx);
        let mx = if old_x < lo { lo } else { old_x.min(hi) };
        let my = if old_y < 0 { 0 } else { old_y.min(sy) };
        let (u, v) = self.material_size(&cursor);
        let w = u as f32 * scale;
        let h = v as f32 * scale;
        let x = ftol(mx as f32 - offset[0] * w);
        let y = ftol(my as f32 - offset[1] * h);
        self.set_prop(ctrl, "MouseX", Value::Int(mx));
        self.set_prop(ctrl, "LastMouseX", Value::Int(old_x));
        self.set_prop(ctrl, "LastMouseY", Value::Int(old_y));
        self.set_prop(ctrl, "MouseY", Value::Int(my));
        // Touch screen (Android, the finger drives the menus): no cursor, the finger is the pointer.
        if self.touch_hides_cursor {
            return;
        }
        let z = self.gfl(c, "Z");
        self.canvas_draw_tile(c, &cursor, x as f32, y as f32, w, h, 0.0, 0.0, u as f32, v as f32, z, [1.0; 4], [0.0; 4]);
    }

    /// Touch screens (Android): which of the elements drawn outside the game's controls (the
    /// Select level equipment page's icons and buttons) is under the point (x, y) of the menu's
    /// page, so that a finger slid from one to another clicks nothing. None: no such element.
    pub fn menu_part_at(&self, x: f32, y: f32) -> Option<usize> {
        edition::menu_part_at(self, x, y)
    }

    /// Touch screens (Android): a finger put at (x, y) of `ctrl`'s menu page on the bottom help
    /// bar. None: the bar's zones are not known (faithful build: the caller keeps its own
    /// zones). Some(key): the key of the side under the finger (Escape, Enter), drawn highlighted
    /// until [`Vm::help_bar_lift`]; Some(None): not on a side of the bar as drawn.
    pub fn help_bar_touch(&mut self, ctrl: ObjId, x: f32, y: f32) -> Option<Option<u8>> {
        edition::help_bar_touch(self, ctrl, x, y)
    }

    /// The finger put by [`Vm::help_bar_touch`] lifted: its side no longer highlighted.
    pub fn help_bar_lift(&mut self) {
        edition::help_bar_lift(self)
    }

    /// Touch screens (Android): the control of `ctrl`'s menu under the point (x, y) of the menu's
    /// page, as UnderCursor finds it, and how a finger works it ([`MenuTouch`]).
    pub fn menu_control_at(&mut self, ctrl: ObjId, x: f32, y: f32) -> Option<(ObjId, MenuTouch)> {
        let u = self.gui_controller_under_cursor(ctrl, x, y)?;
        let list_row = edition::is_list_row(self, u);
        let kind = if self.is_a(u, "XIIIHSliderControl") {
            MenuTouch::Clicks
        } else if list_row || self.is_a(u, "GUIListBase") {
            MenuTouch::List
        } else if ["GUISlider","GUIScrollBarBase", "GUIVertScrollZone", "GUIVertGripButton"].iter().any(|c| self.is_a(u, c)) {
            MenuTouch::Hold
        } else {
            MenuTouch::Tap
        };
        Some((u, kind))
    }

    /// Touch screens (Android): whether a left click on `ctrl`'s menu would only give the focus
    /// (NativeKeyEvent: the hovered control is not the focused one, LookUnderCursor and the click
    /// is dropped), as after going back to a page with the cursor already on a button.
    pub fn menu_click_only_focuses(&self, ctrl: ObjId) -> bool {
        // The co-op character screen's page (character_page.rs) takes every click, the one the
        // controller turns into "no key" included: a focus click first would be a second click
        // (Valider pressed twice: the confirmation taken back at once, the character back in
        // its pose).
        if edition::character_screen_open(self) {
            return false;
        }
        let raw = self.gof(ctrl, "ActivePage").is_some_and(|p| self.gbool(p, "bNeedRawKey"));
        !raw && self.prop(ctrl, "ActiveControl") != self.prop(ctrl, "FocusedControl")
    }

    /// UGUIController::UnderCursor: delegated to the active page.
    pub(crate) fn gui_controller_under_cursor(&mut self, ctrl: ObjId, x: f32, y: f32) -> Option<ObjId> {
        self.gof(ctrl, "ActivePage").and_then(|p| self.gui_under_cursor(p, x, y))
    }

    /// UGUIController::LookUnderCursor.
    pub(crate) fn gui_look_under_cursor(&mut self, ctrl: ObjId, dx: f32, dy: f32) {
        // Improved build: a full-screen page of the window (Mods) covers the menu: the cursor
        // still moves (that page reads it) but nothing behind is hovered.
        if edition::overlay_open(self) {
            return;
        }
        let (mx, my) = (self.gint(ctrl, "MouseX") as f32, self.gint(ctrl, "MouseY") as f32);
        let under = self.gui_controller_under_cursor(ctrl, mx, my);
        let active = self.gof(ctrl, "ActiveControl");
        match active {
            None => {
                let Some(u) = under else { return };
                self.set_prop(ctrl, "ActiveControl", Value::Object(Some(u)));
                if self.gbyte(u, "MenuState") == MSAT_BLURRY {
                    self.gui_event(u, "MenuStateChange", vec![Value::Byte(MSAT_WATCHED)]);
                    // The sound only if the control has one (bMouseOverSound).
                    if self.gbool(u, "bMouseOverSound") {
                        let sound = self.propr(ctrl, "MouseOverSound").as_name();
                        self.gui_play_interface_sound(ctrl, &sound);
                    }
                }
            }
            Some(a) => {
                if self.gbyte(a, "MenuState") == MSAT_PRESSED && self.gbool(a, "bCaptureMouse") {
                    self.gui_mouse_move(a, ftol(dx), ftol(dy));
                    return;
                }
                if Some(a) != under && self.gbyte(a, "MenuState") == MSAT_WATCHED {
                    self.gui_event(a, "MenuStateChange", vec![Value::Byte(MSAT_BLURRY)]);
                }
                if let Some(u) = under {
                    if self.gbyte(u, "MenuState") == MSAT_BLURRY {
                        self.gui_event(u, "MenuStateChange", vec![Value::Byte(MSAT_WATCHED)]);
                        // The sound if the control has one and it was not already the
                        // active control; MouseOverSound is the controller's (+0xFC).
                        if self.gbool(u, "bMouseOverSound") && Some(u) != Some(a) {
                            let sound = self.propr(ctrl, "MouseOverSound").as_name();
                            self.gui_play_interface_sound(ctrl, &sound);
                        }
                    }
                }
                self.set_prop(ctrl, "ActiveControl", Value::Object(under));
            }
        }
        if let Some(p) = self.gof(ctrl, "ActivePage") {
            let hint = match self.gof(ctrl, "ActiveControl") {
                Some(a) => self.prop(a, "Hint"),
                None => Value::Str(String::new()),
            };
            self.gui_event(p, "ChangeHint", vec![hint]);
        }
        if let Some(a) = self.gof(ctrl, "ActiveControl") {
            self.gui_mouse_hover(a);
        }
    }

    /// UGUIController::NativeKeyEvent: gamepad and mouse keys
    /// mapped to menu keys, repeat, then the active page and the other pages.
    pub(crate) fn gui_controller_key_event(&mut self, ctrl: ObjId, key: u8, action: u8, delta: f32) -> bool {
        // Improved build: the device last used, and a click's key sound (menu_mouse.rs).
        if let Some(result) = edition::mouse_key_event(self, ctrl, key, action, delta) {
            return result;
        }
        self.gui_controller_key_event_in(ctrl, key, action, delta, false)
    }

    /// `click_sound`: a left button press whose release may play a key's sound (menu_mouse.rs),
    /// noted once the help bar and the split screen have had it.
    pub(crate) fn gui_controller_key_event_in(&mut self, ctrl: ObjId, key: u8, action: u8, delta: f32, click_sound: bool) -> bool {
        if self.gof(ctrl, "ViewportOwner").is_none() || self.gof(ctrl, "ViewportOwner") != self.local_viewport() {
            return false;
        }
        // Split screen: a gamepad player's menu ignores the mouse (moves, buttons, wheel), taken
        // here so that nothing else reacts to them either.
        if MOUSE_KEYS.contains(&key) && self.gui_without_mouse(ctrl) {
            return true;
        }
        // Improved build: the left mouse button on a side of the help bar is that side's key
        // (help_bar_click.rs).
        if let Some(result) = edition::menu_key_event(self, ctrl, key, action, click_sound) {
            return result;
        }
        if self.gbool(ctrl, "TimeOutArmed") {
            // Inline appSeconds (rdtsc × GSecondsPerCycle): the VM's clock.
            self.set_prop(ctrl, "TimeOfLastInput", Value::Float(self.app_seconds as f32));
        }
        let mut k = key;
        let mut a = action;
        let raw = self.gof(ctrl, "ActivePage").is_some_and(|p| self.gbool(p, "bNeedRawKey"));
        if raw {
            self.log.push(format!("bNeedRawKey {k}"));
        } else {
            let mut remap = true;
            if k == 1 {
                if self.prop(ctrl, "ActiveControl") == self.prop(ctrl, "FocusedControl") {
                    remap = false;
                } else {
                    let (mx, my) = (self.gint(ctrl, "MouseX") as f32, self.gint(ctrl, "MouseY") as f32);
                    self.gui_look_under_cursor(ctrl, mx, my);
                    k = 0;
                    self.set_prop(ctrl, "RepeatKey", Value::Byte(0));
                }
            }
            if remap {
                k = match k {
                    2 | 0xc8 => 8,
                    0xd4 => 0x26,
                    0xd6 => 0x28,
                    0xd7 => 0x25,
                    0xd5 => 0x27,
                    0xca => 0x0d,
                    0xd3 => {
                        let title = self.level.map(|l| self.propr(l, "Title").as_name()).unwrap_or_default();
                        if !title.eq_ignore_ascii_case("MapMenu") {
                            0x1b
                        } else {
                            let press_start = self
                                .gof(ctrl, "ActivePage")
                                .and_then(|p| self.objects[p].as_ref().map(|o| self.classes[o.class].name.clone()))
                                .is_some_and(|n| n.eq_ignore_ascii_case("XIIIMenuPressStart"));
                            if press_start {
                                k
                            } else {
                                0x0d
                            }
                        }
                    }
                    // test ah, 0x41: NaN counts as Delta ≤ 0.
                    0xe1 => {
                        if !(0.0 < delta) {
                            0x28
                        } else {
                            0x26
                        }
                    }
                    0xe0 => {
                        if !(0.0 < delta) {
                            0x25
                        } else {
                            0x27
                        }
                    }
                    other => other,
                };
            }
        }
        // OnNeedRawKeyPress delegate (key binding setup): takes everything, on release.
        if matches!(self.prop(ctrl, "__OnNeedRawKeyPress__Delegate"), Value::Delegate(Some(x), _) if self.alive(x)) {
            if a == IST_RELEASE {
                self.gui_delegate(ctrl, "OnNeedRawKeyPress", vec![Value::Byte(k)]);
            }
            return true;
        }
        let mouse_move = if k == 0x0d {
            if self.gbool(ctrl, "AltPressed") {
                return true;
            }
            self.gbool(ctrl, "bForceMouseCheck")
        } else {
            k == 0xe4 || k == 0xe5 || self.gbool(ctrl, "bForceMouseCheck")
        };
        if mouse_move {
            self.set_prop(ctrl, "bForceMouseCheck", Value::Bool(false));
            let sens = self.gfl(ctrl, "MenuMouseSens");
            let (mut dx, mut dy) = (0.0f32, 0.0f32);
            if k == 0xe4 {
                dx = fistp_dword(delta - 0.5);
                let x = ftol(dx * sens + self.gint(ctrl, "MouseX") as f32);
                self.set_prop(ctrl, "MouseX", Value::Int(x));
            } else {
                dy = fistp_dword(delta - 0.5);
                let y = ftol(self.gint(ctrl, "MouseY") as f32 - dy * sens);
                self.set_prop(ctrl, "MouseY", Value::Int(y));
            }
            if self.gint(ctrl, "MouseX") != self.gint(ctrl, "LastMouseX") || self.gint(ctrl, "MouseY") != self.gint(ctrl, "LastMouseY") {
                self.gui_look_under_cursor(ctrl, dx, dy);
            }
        }
        // Repeat of held keys (RepeatKey, restarted by NativeTick).
        let repeat = self.gbyte(ctrl, "RepeatKey");
        if k == repeat {
            if a != IST_PRESS && a != IST_HOLD {
                self.set_prop(ctrl, "RepeatKey", Value::Byte(0));
                self.set_prop(ctrl, "RepeatTime", Value::Float(0.0));
                self.set_prop(ctrl, "RepeatDelta", Value::Float(0.0));
            } else if a == IST_PRESS {
                return true;
            }
        } else if a == IST_PRESS && k != 0x10 && k != 0x11 && k != 0x12 {
            self.set_prop(ctrl, "RepeatKey", Value::Byte(k));
            self.set_prop(ctrl, "RepeatDelta", Value::Float(delta));
            let d = self.gfl(ctrl, "ButtonRepeatDelay");
            self.set_prop(ctrl, "RepeatTime", Value::Float(d + d));
        }
        let active = self.gof(ctrl, "ActivePage");
        if let Some(p) = active {
            if self.gui_key_event(p, &mut k, &mut a, delta) {
                return true;
            }
        }
        let mut i = self.objects_of(ctrl, "MenuStack").len();
        while i > 0 {
            i -= 1;
            let Some(p) = self.page_at(ctrl, i) else { continue };
            if Some(p) != self.gof(ctrl, "ActivePage") && self.gui_key_event(p, &mut k, &mut a, delta) {
                return true;
            }
        }
        false
    }

    /// UGUIController::NativeKeyType: always taken.
    pub(crate) fn gui_controller_key_type(&mut self, ctrl: ObjId, key: u8) -> bool {
        let mut k = key;
        self.set_prop(ctrl, "RepeatKey", Value::Byte(0));
        self.set_prop(ctrl, "RepeatTime", Value::Float(0.0));
        self.set_prop(ctrl, "RepeatDelta", Value::Float(0.0));
        if let Some(p) = self.gof(ctrl, "ActivePage") {
            if !self.gui_key_type(p, &mut k) {
                let mut i = self.objects_of(ctrl, "MenuStack").len();
                while i > 0 {
                    i -= 1;
                    let Some(q) = self.page_at(ctrl, i) else { continue };
                    if Some(q) != self.gof(ctrl, "ActivePage") && self.gui_key_type(q, &mut k) {
                        break;
                    }
                }
            }
        }
        true
    }

    /// UGUIController::NativeTick.
    pub(crate) fn gui_controller_tick(&mut self, ctrl: ObjId, dt: f32) {
        self.gui_event(ctrl, "Tick", vec![Value::Float(dt)]);
        let Some(player) = self.gof(ctrl, "ViewportOwner") else { return };
        let Some(pc) = self.gof(player, "Actor") else { return };
        let Some(level) = self.gof(pc, "Level") else { return };
        let t = dt / self.gfl(level, "TimeDilation");
        if self.gbyte(ctrl, "RepeatKey") != 0 {
            let rt = self.gfl(ctrl, "RepeatTime") - t;
            self.set_prop(ctrl, "RepeatTime", Value::Float(rt));
            if rt <= 0.0 {
                let (k, d) = (self.gbyte(ctrl, "RepeatKey"), self.gfl(ctrl, "RepeatDelta"));
                self.gui_controller_key_event(ctrl, k, IST_HOLD, d);
                let delay = self.prop(ctrl, "ButtonRepeatDelay");
                self.set_prop(ctrl, "RepeatTime", delay);
            }
        }
        for (fade, step, speed) in [("CursorFade", "CursorStep", 255.0f32), ("FastCursorFade", "FastCursorStep", 8192.0)] {
            let f = self.gint(ctrl, step) as f32 * t * speed + self.gfl(ctrl, fade);
            self.set_prop(ctrl, fade, Value::Float(f));
            // fcom: a NaN fade is neither ≤ 0 nor ≥ 255, it stays as is.
            if f <= 0.0 {
                self.set_prop(ctrl, fade, Value::Float(0.0));
                self.set_prop(ctrl, step, Value::Int(1));
            } else if f >= 255.0 {
                self.set_prop(ctrl, fade, Value::Float(255.0));
                self.set_prop(ctrl, step, Value::Int(-1));
            }
        }
        if let Some(page) = self.gof(ctrl, "ActivePage") {
            self.gui_update_timers(page, t);
            // UGUIPage::ProcessState: state code of the page (latent Sleep).
            self.process_state(page, t);
            if let Some(page) = self.gof(ctrl, "ActivePage") {
                self.gui_event(page, "Tick", vec![Value::Float(t)]);
            }
        }
        if self.gbool(ctrl, "TimeOutArmed") {
            let delay = self.gfl(ctrl, "DelayOfInactivityBeforeCallingTimeOut");
            let last = self.gfl(ctrl, "TimeOfLastInput") as f64;
            // 24-bit fsub: the difference is rounded to a float before the comparison.
            if delay < (self.app_seconds - last) as f32 {
                self.gui_event(ctrl, "NoInputTimeOut", vec![]);
                self.set_prop(ctrl, "TimeOutArmed", Value::Bool(false));
            }
        }
    }
}

/// Largest 4:3 frame centred in a `clip_x` × `clip_y` canvas: (x, y, width, height) in whole
/// pixels (the whole canvas when it is already 4:3).
pub(crate) fn menu_frame_43(clip_x: i32, clip_y: i32) -> [i32; 4] {
    let (clip_x, clip_y) = (clip_x.max(1), clip_y.max(1));
    let (w, h) = if clip_x * 3 > clip_y * 4 { ((clip_y * 4 + 1) / 3, clip_y) } else { (clip_x, (clip_x * 3 + 2) / 4) };
    let (w, h) = (w.min(clip_x), h.min(clip_y));
    [(clip_x - w) / 2, (clip_y - h) / 2, w, h]
}

#[cfg(test)]
#[test]
fn menu_frame_43_halves() {
    // Horizontal halves of a 1280 × 720 and of a 1366 × 768 window.
    assert_eq!(menu_frame_43(1280, 360), [400, 0, 480, 360]);
    assert_eq!(menu_frame_43(1366, 384), [427, 0, 512, 384]);
    // Already 4:3, or narrower (vertical split): full width, centred in height.
    assert_eq!(menu_frame_43(640, 480), [0, 0, 640, 480]);
    assert_eq!(menu_frame_43(640, 720), [0, 120, 640, 480]);
}

#[cfg(test)]
#[test]
fn menu_frame_scales_a_whole_page() {
    // 1366 × 768 split in two: an 800 × 600 page scaled by 0.64 into 512 × 384 at x = 427.
    let f = MenuFrame::of_half(1366, 384).unwrap();
    assert_eq!((f.x, f.y, f.virtual_size), (427.0, 0.0, (800, 600)));
    assert!((f.scale - 0.64).abs() < 1e-6);
    // 1600 × 900: 0.75; 1920 × 1080: 0.9; 1280 × 720: 0.6 (the smallest); a 600-high frame: itself.
    assert!((MenuFrame::of_half(1600, 450).unwrap().scale - 0.75).abs() < 1e-6);
    assert!((MenuFrame::of_half(1920, 540).unwrap().scale - 0.9).abs() < 1e-6);
    assert!((MenuFrame::of_half(1280, 360).unwrap().scale - 0.6).abs() < 1e-6);
    let big = MenuFrame::of_half(2560, 720).unwrap();
    assert_eq!((big.virtual_size, big.scale), ((960, 720), 1.0));
    // Small windows: 1024 × 576 (0.48), 854 × 480 (0.4), 800 × 600 (0.5).
    let small = MenuFrame::of_half(1024, 288).unwrap();
    assert_eq!((small.x, small.y, small.virtual_size), (320.0, 0.0, (800, 600)));
    assert!((small.scale - 0.48).abs() < 1e-6);
    assert!((MenuFrame::of_half(854, 240).unwrap().scale - 0.4).abs() < 1e-6);
    assert!((MenuFrame::of_half(800, 300).unwrap().scale - 0.5).abs() < 1e-6);
    // A small 4:3 view (four players) is scaled whole; a 4:3 view 600 high or more keeps its layout.
    assert_eq!(MenuFrame::of_half(640, 480).map(|f| (f.x, f.y, f.scale)), Some((0.0, 0.0, 0.8)));
    assert_eq!(MenuFrame::of_half(800, 600), None);
    assert_eq!(MenuFrame::of_half(1024, 768), None);
    assert_eq!(MenuFrame::of_half(0, 0), None);
    // A text tile and the mouse go through the same transform, both ways.
    let mut tile = HudDraw::Tile { x1: 100.0, y1: 50.0, x2: 140.0, y2: 66.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: true, enlarged: false, glyph: false };
    f.place(&mut tile);
    let HudDraw::Tile { x1, y1, x2, y2, point, .. } = tile else { unreachable!() };
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    assert!(near(x1, 491.0) && near(y1, 32.0) && near(x2, 516.6) && near(y2, 42.24) && !point, "{x1} {y1} {x2} {y2}");
    let (mx, my) = f.unplace(x1, y1);
    assert!((mx - 100.0).abs() < 1e-3 && (my - 50.0).abs() < 1e-3);
}

#[cfg(test)]
#[test]
fn menu_tap_on_a_page_not_laid_out_yet_clicks() {
    // The rule before: same control at the press and the lift, none for both only if near.
    let before = |s: Option<ObjId>, sp: Option<usize>, n: Option<ObjId>, np: Option<usize>, near: bool| n == s && np == sp && (s.is_some() || near);
    let mut cases = 0;
    let mut late = 0;
    for i in 0..400usize {
        let pick = |k: usize| match k % 4 {
            0 => None,
            v => Some(1700 + v),
        };
        let (s, n) = (pick(i), pick(i / 4 + i / 3));
        let (sp, np) = (if i % 5 == 0 { Some(i % 3) } else { None }, if i % 7 == 0 { Some(i % 3) } else { None });
        let near = i % 2 == 0;
        let got = menu_tap_clicks(s, sp, n, np, near);
        if s.is_some() {
            // Put down on a control: unchanged (a finger slid off a button clicks nothing).
            assert_eq!(got, before(s, sp, n, np, near), "{i}");
            assert_eq!(got, n == s && np == sp, "{i}");
        } else {
            assert_eq!(got, near && (sp.is_none() || np == sp), "{i}");
            if n.is_some() && got {
                // Negative check: the case of the bug, dropped by the rule before.
                assert!(!before(s, sp, n, np, near), "{i}");
                late += 1;
            }
        }
        cases += 1;
    }
    assert_eq!(cases, 400);
    assert!(late >= 40, "{late}");
    // The profile page right after launch: put down on nothing, lifted on the XIII button.
    assert!(menu_tap_clicks(None, None, Some(1726), None, true));
    // Slid away from where it was put down: nothing.
    assert!(!menu_tap_clicks(None, None, Some(1726), None, false));
    // Slid off a button onto another: nothing.
    assert!(!menu_tap_clicks(Some(1722), None, Some(1726), None, true));
}

#[cfg(test)]
#[test]
fn menu_screen_puts_a_finger_on_the_page() {
    // Landscape phone 2400 × 1080, UIScale 1: the 960 × 720 page scaled by 1.5 at x = 480.
    let f = MenuFrame::of_window(2400, 1080, 1.0).unwrap();
    assert_eq!((f.x, f.y, f.virtual_size), (480.0, 0.0, (960, 720)));
    assert!((f.scale - 1.5).abs() < 1e-6);
    let screen = MenuScreen { frame: Some(f), size: (960, 720), canvas: (2400.0, 1080.0) };
    // The window's centre is the page's centre, the frame's corners its corners.
    assert_eq!(screen.to_page((1200.0, 540.0)), (480.0, 360.0));
    assert_eq!(screen.to_page((480.0, 0.0)), (0.0, 0.0));
    assert_eq!(screen.to_page((1920.0, 1080.0)), (960.0, 720.0));
    // An unscaled 4:3 frame (1333 × 600): shifted by its left edge only.
    let [x, y, w, h] = menu_frame_43(1333, 600);
    let small = MenuScreen { frame: Some(MenuFrame { bicubic: enlarged_text_bicubic(Default::default()), x: x as f32, y: y as f32, scale: 1.0, virtual_size: (w, h) }), size: (w as u32, h as u32), canvas: (1333.0, 600.0) };
    assert_eq!(small.to_page((700.0, 300.0)), (700.0 - x as f32, 300.0));
    // No frame: the window itself.
    let none = MenuScreen { frame: None, size: (800, 600), canvas: (800.0, 600.0) };
    assert_eq!(none.to_page((12.0, 34.0)), (12.0, 34.0));
}

#[cfg(test)]
#[test]
fn menu_frame_43_screens() {
    // 4:3: the whole canvas (no change).
    assert_eq!(menu_frame_43(640, 480), [0, 0, 640, 480]);
    assert_eq!(menu_frame_43(1280, 960), [0, 0, 1280, 960]);
    assert_eq!(menu_frame_43(1024, 768), [0, 0, 1024, 768]);
    // Wide screens: full height, centred.
    assert_eq!(menu_frame_43(1920, 1080), [240, 0, 1440, 1080]);
    assert_eq!(menu_frame_43(1920, 1200), [160, 0, 1600, 1200]);
    assert_eq!(menu_frame_43(2560, 1080), [560, 0, 1440, 1080]);
    // Split-screen halves.
    assert_eq!(menu_frame_43(1280, 360), [400, 0, 480, 360]);
    assert_eq!(menu_frame_43(1366, 384), [427, 0, 512, 384]);
    // Narrower (portrait phone, vertical split): full width, centred in height.
    assert_eq!(menu_frame_43(1080, 2400), [0, 795, 1080, 810]);
    assert_eq!(menu_frame_43(640, 720), [0, 120, 640, 480]);
}

#[cfg(test)]
#[test]
fn menu_frame_mouse_lands_on_the_item_under_the_cursor() {
    // First player's half (top) of 1024 × 576, 1366 × 768 and 1920 × 1080 windows. A window pixel
    // brought into the menu's virtual screen (PreRender) and an item drawn there (the item under
    // the cursor) come back to the same window pixel; the scaled cursor tip too.
    for (w, h) in [(1024, 288), (1366, 384), (1920, 540)] {
        let f = MenuFrame::of_half(w, h).unwrap();
        // "Controls" of the pause list: around (411, 286) on the 800 × 600 page.
        let (sx, sy) = (f.x + 411.0 * f.scale, 286.0 * f.scale);
        let (vx, vy) = f.unplace(sx, sy);
        assert!((vx - 411.0).abs() < 1e-3 && (vy - 286.0).abs() < 1e-3, "{w}x{h}: {vx} {vy}");
        let mut item = HudDraw::Tile { x1: 300.0, y1: 275.0, x2: 520.0, y2: 297.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: false, enlarged: false, glyph: false };
        f.place(&mut item);
        let HudDraw::Tile { x1, y1, x2, y2, .. } = item else { unreachable!() };
        assert!(x1 <= sx && sx <= x2 && y1 <= sy && sy <= y2, "{w}x{h}: ({sx}, {sy}) outside {x1} {y1} {x2} {y2}");
        // Negative control: the window pixel taken as it is (no mapping) misses the item.
        assert!(!(300.0..=520.0).contains(&sx) || !(275.0..=297.0).contains(&sy), "{w}x{h}");
    }
}

#[cfg(test)]
#[test]
fn window_page_same_share_of_every_screen() {
    let f = |w, h, ui| MenuFrame::of_window(w, h, ui).map(|f| (f.x, f.y, f.scale, f.virtual_size));
    let near = |a: Option<(f32, f32, f32, (i32, i32))>, b: (f32, f32, f32, (i32, i32))| {
        a.is_some_and(|a| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3 && (a.2 - b.2).abs() < 1e-4 && a.3 == b.3)
    };
    // The original's 1280-wide modes are the reference: the game's own layout.
    for h in [720, 800, 960, 1024] {
        assert_eq!(f(1280, h, 1.0), None, "1280x{h}");
    }
    // Larger: the same mode's look, scaled (16:9 and 21:9 as 1280 x 720, 16:10 as 1280 x 800).
    assert!(near(f(1920, 1080, 1.0), (240.0, 0.0, 1.5, (960, 720))));
    assert!(near(f(2560, 1440, 1.0), (320.0, 0.0, 2.0, (960, 720))));
    assert!(near(f(3840, 2160, 1.0), (480.0, 0.0, 3.0, (960, 720))));
    assert!(near(f(2560, 1080, 1.0), (560.0, 0.0, 1.5, (960, 720))));
    assert!(near(f(1920, 1200, 1.0), (160.0, 0.0, 1.5, (1067, 800))));
    assert!(near(f(1600, 1200, 1.0), (0.0, 0.0, 1.25, (1280, 960))));
    // UIScale: smaller, centred; larger up to the 800 x 600 page filling the frame's height.
    assert!(near(f(1920, 1080, 0.75), (240.0, 0.0, 1.125, (1280, 960))));
    assert!(near(f(1920, 1080, 1.5), (240.0, 0.0, 1.8, (800, 600))));
    // Smaller than the original's mode: its own layout (never smaller than the original).
    assert_eq!(f(1024, 768, 1.0), None);
    assert_eq!(f(800, 600, 1.0), None);
    assert_eq!(f(1366, 768, 1.0).map(|f| f.3), Some((960, 720)));
    assert_eq!(f(1080, 2400, 1.0), None);
    assert_eq!(f(1920, 1080, 0.6), None);
    // A split-screen half of a 4K window (3840 x 1080): as a 1080-high wide window.
    assert!(near(f(3840, 1080, 1.0), (1200.0, 0.0, 1.5, (960, 720))));
    assert_eq!(f(1920, 540, 1.0), None);
    // Page point and window point, both ways.
    let m = MenuFrame::of_window(1920, 1080, 1.0).unwrap();
    let (x, y) = m.to_canvas(536.0, 567.0);
    assert!(((x - 1044.0).abs() < 1e-3) && ((y - 850.5).abs() < 1e-3), "{x} {y}");
    let (px, py) = m.unplace(x, y);
    assert!((px - 536.0).abs() < 1e-3 && (py - 567.0).abs() < 1e-3);
}

#[cfg(test)]
#[test]
fn whole_scales_keep_cells_sharp() {
    let tile = || HudDraw::Tile { x1: 10.0, y1: 20.0, x2: 18.0, y2: 36.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: true, enlarged: false, glyph: false };
    let mut d = tile();
    place_scaled(&mut d, 2.0, [160.0, 0.0]);
    let HudDraw::Tile { x1, y1, x2, y2, point, .. } = d else { unreachable!() };
    assert_eq!((x1, y1, x2, y2, point), (180.0, 40.0, 196.0, 72.0, true));
    let mut d = tile();
    place_scaled(&mut d, 1.8, [240.0, 0.0]);
    let HudDraw::Tile { point, .. } = d else { unreachable!() };
    assert!(!point);
    // A page backdrop covering the whole virtual screen (960 x 720) covers the whole window.
    let m = MenuFrame::of_window(1920, 1080, 1.0).unwrap();
    let mut page = vec![HudDraw::Tile { x1: 0.0, y1: 0.0, x2: 960.0, y2: 720.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: false, enlarged: false, glyph: false }];
    m.place_page(&mut page, (1920.0, 1080.0));
    let HudDraw::Tile { x1, y1, x2, y2, .. } = page[0] else { unreachable!() };
    assert!(x1.abs() < 1e-3 && y1.abs() < 1e-3 && (x2 - 1920.0).abs() < 1e-3 && (y2 - 1080.0).abs() < 1e-3);
}

#[test]
fn enlarged_text_is_smoothed() {
    // A glyph cell placed at `s` with the enhanced graphics' rule (`true`) or the faithful one:
    // (stays POINT, read bicubic).
    let read = |s: [f32; 2], bicubic: bool| {
        let mut d = HudDraw::Tile { x1: 10.0, y1: 20.0, x2: 18.0, y2: 36.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: true, enlarged: false, glyph: false };
        place_scaled_in(&mut d, s, [0.0, 0.0], bicubic);
        let HudDraw::Tile { point, enlarged, glyph, .. } = d else { unreachable!() };
        let cubic = reads_bicubic(point, glyph, enlarged, bicubic);
        (point && !cubic, cubic)
    };
    // Enhanced graphics: scale 1 keeps the original's POINT, every enlargement (1080p 1.5, 1440p
    // 2, 4K 3, UIScale factors) is read with the bicubic filter, a smaller page is smoothed.
    assert_eq!(read([1.0, 1.0], true), (true, false));
    for s in [1.5f32, 2.0, 3.0, 4.0, 2.25, 1.0001] {
        assert_eq!(read([s, s], true), (false, true), "scale {s}");
    }
    assert_eq!(read([1.0, 2.0], true), (false, true));
    assert_eq!(read([0.75, 0.75], true), (false, false));
    assert_eq!(read([0.75, 2.0], true), (false, false));
    // Faithful build and Original graphics: whole scales stay POINT, others smoothed, never bicubic.
    for s in [1.0f32, 2.0, 3.0] {
        assert_eq!(read([s, s], false), (true, false), "scale {s}");
    }
    for s in [1.5f32, 2.25, 0.75] {
        assert_eq!(read([s, s], false), (false, false), "scale {s}");
    }
    // A picture (not a glyph) is never read bicubic by this rule.
    assert!(!reads_bicubic(false, false, true, true));
    // A letter drawn without POINT (ClippedPrint, DrawTextJustified: dialogue bubbles, menu
    // buttons) is read bicubic once enlarged, as any other letter; at scale 1, shrunk on one
    // axis, or with the faithful rule it keeps its own filtering.
    let letter = |s: [f32; 2], bicubic: bool| {
        let mut d = HudDraw::Tile { x1: 10.0, y1: 20.0, x2: 18.0, y2: 36.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: false, enlarged: false, glyph: true };
        place_scaled_in(&mut d, s, [0.0, 0.0], bicubic);
        let HudDraw::Tile { point, enlarged, glyph, .. } = d else { unreachable!() };
        reads_bicubic(point, glyph, enlarged, bicubic)
    };
    for s in [1.5f32, 2.0, 3.0, 2.25] {
        assert!(letter([s, s], true), "scale {s}");
        assert!(!letter([s, s], false), "faithful scale {s}");
    }
    assert!(!letter([1.0, 1.0], true));
    assert!(!letter([0.75, 0.75], true));
    assert!(!letter([0.75, 2.0], true));
}

#[test]
fn hud_screen_of_every_viewport() {
    let f = |w, h, ui| HudScreen::of_viewport(w, h, ui).map(|s| (s.scale, s.virtual_size));
    // Up to the reference size, and smaller split-screen halves: the game's own layout.
    assert_eq!(f(1280, 720, 1.0), None);
    assert_eq!(f(1280, 960, 1.0), None);
    assert_eq!(f(1024, 768, 1.5), Some(([1024.0 / 853.0, 1.2], (853, 640))));
    assert_eq!(f(1920, 540, 1.0), None);
    assert_eq!(f(0, 0, 1.0), None);
    // 1080p, 1440p, 4K and a 4K half: the 1280 x 720 share, whole factors on 1440p and 4K.
    assert_eq!(f(1920, 1080, 1.0), Some(([1.5, 1.5], (1280, 720))));
    assert_eq!(f(2560, 1440, 1.0), Some(([2.0, 2.0], (1280, 720))));
    assert_eq!(f(3840, 2160, 1.0), Some(([3.0, 3.0], (1280, 720))));
    assert_eq!(f(3840, 1080, 1.0), Some(([1.5, 1.5], (2560, 720))));
    // A 4:3 or 16:10 screen keeps its shape; UIScale multiplies the factor.
    assert_eq!(f(1600, 1200, 1.0), Some(([1.25, 1.25], (1280, 960))));
    let (s, (vw, vh)) = f(3840, 2160, 0.75).unwrap();
    assert_eq!((vw, vh), (1707, 960));
    assert!((s[0] - 2.25).abs() < 2e-3 && s[1] == 2.25);
    // Every primitive lands on the viewport: the whole virtual screen is the whole window.
    let screen = HudScreen::of_viewport(3840, 2160, 0.75).unwrap();
    let mut d = HudDraw::Tile { x1: 0.0, y1: 0.0, x2: vw as f32, y2: vh as f32, uv: [0.0; 4], material: "".into(), color: [0; 4], point: true, enlarged: false, glyph: false };
    screen.place(&mut d);
    let HudDraw::Tile { x1, y1, x2, y2, point, .. } = d else { unreachable!() };
    assert!(x1 == 0.0 && y1 == 0.0 && (x2 - 3840.0).abs() < 1e-2 && (y2 - 2160.0).abs() < 1e-2 && !point);
    let mut d = HudDraw::Tile { x1: 4.0, y1: 4.0, x2: 20.0, y2: 24.0, uv: [0.0; 4], material: "".into(), color: [0; 4], point: true, enlarged: false, glyph: false };
    HudScreen::of_viewport(2560, 1440, 1.0).unwrap().place(&mut d);
    let HudDraw::Tile { x1, y1, x2, y2, point, .. } = d else { unreachable!() };
    assert_eq!((x1, y1, x2, y2, point), (8.0, 8.0, 40.0, 48.0, true));
}

/// Geometry of a control for Actual*: bBoundToParent, bScaleToParent, WinTop, WinLeft,
/// WinWidth, WinHeight.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geo {
    pub bound: bool,
    pub scale: bool,
    pub win: [f32; 4],
}

/// UGUIComponent::ActualWidth / ActualHeight on the chain (the control, then its MenuOwners):
/// above 1, the value itself; otherwise the fraction of the parent's size (bScaleToParent) or of
/// the screen (Viewports[0] SizeX / SizeY).
pub(crate) fn actual_size(string: &[Geo], screen: (i32, i32), height: bool) -> f32 {
    let Some(g) = string.first() else { return 0.0 };
    let w = g.win[if height { 3 } else { 2 }];
    if w > 1.0 {
        return w;
    }
    if g.scale && string.len() > 1 {
        return actual_size(&string[1..], screen, height) * w;
    }
    (if height { screen.1 } else { screen.0 }) as f32 * w
}

/// UGUIComponent::ActualLeft / ActualTop: above 1, the value (plus the parent's position if
/// bBoundToParent); otherwise the fraction of the parent's size plus its position, or of the screen.
pub(crate) fn actual_pos(string: &[Geo], screen: (i32, i32), top: bool) -> f32 {
    let Some(g) = string.first() else { return 0.0 };
    let p = g.win[if top { 0 } else { 1 }];
    let parent = (g.bound && string.len() > 1).then(|| &string[1..]);
    if p > 1.0 {
        return match parent {
            Some(m) => actual_pos(m, screen, top) + p,
            None => p,
        };
    }
    match parent {
        Some(m) => {
            let base = actual_pos(m, screen, top);
            actual_size(m, screen, top) * p + base
        }
        None => (if top { screen.1 } else { screen.0 }) as f32 * p,
    }
}

impl Vm {
}
