//! Improved build only (Graphics: Enhanced, PC): the menus' bottom help bar answers the mouse.
//!
//! XIIIWindow.DisplayHelpBar draws, on PC (CurrentPF 0) and in Grey3Color, the right side
//! (EnterText then, to its left, the action: Select, Start, Next…) and the left side (EscapeText
//! then the action: Back, Cancel…), then the centred Update text and the control's hint. In the
//! original it is only a reminder of the keys. Here each side is a button:
//! - its zone is the union of the rectangles of its two texts (a few pixels of margin), as drawn on
//!   the last frame, in the menu page's units (those of GUIController.MouseX / MouseY);
//! - under the menu cursor both texts are drawn in BlackColor, the buttons' highlight colour
//!   (XIIIWindow), and entering the zone plays MnCurseur once, as XIIIGUIBaseButton.MouseEnter;
//! - the left mouse button there does what the key does: the right side sends Enter (0x0D), the
//!   left side Escape (0x1B), through the keyboard's path (Vm::input_event); the mouse button is
//!   taken (no control under the bar is clicked). The left side is silent, as Escape in the
//!   original (Backspace is not sent, since it erases a character in a text field); the right
//!   side's Enter plays its own MnValid;
//! - on a page left without a focused control by the mouse (menu_mouse.rs: the cursor was over
//!   none when the page was entered), Enter would have nothing to validate: the right side is
//!   drawn greyed (DISABLED_GREY, Grey3Color at half opacity over the bar's white) and answers
//!   neither the cursor nor the button, until a control takes the focus.
//! The hint and the Update text are not buttons. Graphics: Original and the faithful build keep
//! the original's bar.
//!
//! Android (improved build, both Graphics settings): the bar's zones are noted the same way and a
//! finger is the button (touch_controls.rs in the viewer, through [`Vm::help_bar_touch`]): put on
//! a side, that side's key is pressed and its texts are drawn in BlackColor until the finger is
//! lifted. There is no mouse cursor, so nothing is hovered otherwise.

use crate::value::{ObjId, Value};
use crate::Vm;

/// A side of the help bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    /// EscapeText and its action (Back, Cancel…): Escape.
    Left,
    /// EnterText and its action (Select, Start…): Enter.
    Right,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
        }
    }

    /// The key a click on the side sends (EInputKey).
    pub(crate) fn key(self) -> u8 {
        match self {
            Side::Left => 0x1b,
            Side::Right => 0x0d,
        }
    }
}

/// Margin around the texts, in the page's units.
pub(crate) const MARGIN: f32 = 3.0;
/// Margin around the texts for a finger, wider than the cursor's point (8 units: 12 pixels on a
/// 1080-pixel-high phone, whose 720-unit page is scaled by 1.5).
pub(crate) const TOUCH_MARGIN: f32 = 8.0;

/// The help bar being drawn (one XIIIWindow.DisplayHelpBar call).
#[derive(Clone, Debug)]
pub(crate) struct Drawing {
    window: ObjId,
    canvas: ObjId,
    left: bool,
    right: bool,
    /// DrawText calls made on the canvas so far.
    texts: u8,
    rects: [Option<[f32; 4]>; 2],
}

/// The bar's state between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct HelpBarClick {
    /// A nested draw is using the already lifted help-bar canvas.
    pub(crate) lifting: bool,
    /// The bar being drawn.
    pub(crate) drawing: Option<Drawing>,
    /// Zones of the bar drawn on the current frame (committed at the frame's end).
    building: Option<(ObjId, [Option<[f32; 4]>; 2])>,
    /// Zones of the last frame's bar: the window that drew it, left and right.
    pub(crate) zones: Option<(ObjId, [Option<[f32; 4]>; 2])>,
    /// Side under the cursor (drawn in BlackColor).
    pub(crate) hovered: Option<Side>,
    /// Side whose key was pressed by the mouse button (released with it).
    pub(crate) pressed: Option<Side>,
    /// The key the bar sent, not yet seen by the menus: still the mouse's (menu_mouse.rs).
    pub(crate) injected: Option<u8>,
    /// The right side greyed (no focused control to validate): no hover, no click.
    pub(crate) right_off: bool,
    /// Side under a finger held on it (touch screens): drawn in BlackColor as the hovered one.
    pub(crate) touched: Option<Side>,
}

/// Union of two rectangles [x1, y1, x2, y2].
pub(crate) fn union(a: Option<[f32; 4]>, b: [f32; 4]) -> [f32; 4] {
    match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    }
}

/// The side of the bar the `n`-th DrawText (from 0) of DisplayHelpBar draws on PC: the right
/// side's two texts first when shown, then the left side's two; the later texts (Update, hint) are
/// none.
pub(crate) fn side_of_text(n: u8, left: bool, right: bool) -> Option<Side> {
    let n = if right {
        if n < 2 {
            return Some(Side::Right);
        }
        n - 2
    } else {
        n
    };
    (left && n < 2).then_some(Side::Left)
}

/// The greyed right side's colour: Grey3Color (127) at half opacity over white.
pub(crate) const DISABLED_GREY: [u8; 4] = [191, 191, 191, 255];

/// The side whose zone holds (x, y).
#[cfg(test)]
pub(crate) fn side_at(zones: &[Option<[f32; 4]>; 2], x: f32, y: f32) -> Option<Side> {
    side_within(zones, x, y, MARGIN)
}

/// The side whose zone, widened by `margin`, holds (x, y).
fn side_within(zones: &[Option<[f32; 4]>; 2], x: f32, y: f32, margin: f32) -> Option<Side> {
    [Side::Left, Side::Right].into_iter().find(|s| {
        zones[s.index()].is_some_and(|r| x >= r[0] - margin && x <= r[2] + margin && y >= r[1] - margin && y <= r[3] + margin)
    })
}

/// What a left mouse button event does on the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClickEffect {
    /// Not the bar's: the GUI handles the button.
    Pass,
    /// Taken by the bar: press `Some(key)` (or nothing).
    Press(Side),
    /// Taken by the bar: release the key of that side.
    Release(Side),
    /// Taken, nothing to do (a hold while the bar's key is down).
    Taken,
}

impl HelpBarClick {
    /// Start of a DisplayHelpBar call drawing on `canvas`.
    pub(crate) fn begin(&mut self, window: ObjId, canvas: ObjId, left: bool, right: bool) {
        self.drawing = Some(Drawing { window, canvas, left, right, texts: 0, rects: [None; 2] });
    }

    /// End of the call: the zones kept for the frame.
    pub(crate) fn end(&mut self) {
        if let Some(d) = self.drawing.take() {
            self.building = Some((d.window, d.rects));
        }
    }

    /// A DrawText on `canvas`: the side it belongs to (if the bar is being drawn there).
    pub(crate) fn text_begin(&mut self, canvas: ObjId) -> Option<Side> {
        let d = self.drawing.as_mut().filter(|d| d.canvas == canvas)?;
        let side = side_of_text(d.texts, d.left, d.right);
        d.texts = d.texts.saturating_add(1);
        side
    }

    /// The rectangle a text of `side` covered.
    pub(crate) fn text_rect(&mut self, side: Side, rect: [f32; 4]) {
        if let Some(d) = self.drawing.as_mut() {
            d.rects[side.index()] = Some(union(d.rects[side.index()], rect));
        }
    }

    /// Whether a text of `side` is drawn highlighted.
    pub(crate) fn highlighted(&self, side: Side) -> bool {
        self.hovered == Some(side) || self.touched == Some(side)
    }

    /// A finger put at (x, y) on the page `active`: the live side under it, held highlighted
    /// until [`HelpBarClick::untouch`].
    pub(crate) fn touch(&mut self, active: Option<ObjId>, x: f32, y: f32) -> Option<Side> {
        self.touched = self.live_side_within(active, x, y, TOUCH_MARGIN);
        self.touched
    }

    /// The finger lifted.
    pub(crate) fn untouch(&mut self) {
        self.touched = None;
    }

    /// The live side under (x, y) on the page `active` (the greyed right side is none).
    fn live_side(&self, active: Option<ObjId>, x: f32, y: f32) -> Option<Side> {
        self.live_side_within(active, x, y, MARGIN)
    }

    fn live_side_within(&self, active: Option<ObjId>, x: f32, y: f32, margin: f32) -> Option<Side> {
        match self.zones {
            Some((w, zones)) if Some(w) == active => side_within(&zones, x, y, margin).filter(|&s| !(s == Side::Right && self.right_off)),
            _ => None,
        }
    }

    /// Start of the menus' drawing for a frame.
    pub(crate) fn frame_begin(&mut self) {
        self.building = None;
    }

    /// End of the menus' drawing: this frame's zones replace the last ones.
    pub(crate) fn frame_end(&mut self) {
        self.zones = self.building.take();
        self.drawing = None;
    }

    /// The cursor at (x, y) on the page `active`: the side under it. True when the cursor has just
    /// entered a side (the tick to play).
    pub(crate) fn hover(&mut self, active: Option<ObjId>, at: Option<(f32, f32)>) -> bool {
        let now = at.and_then(|(x, y)| self.live_side(active, x, y));
        let entered = now.is_some() && now != self.hovered;
        self.hovered = now;
        entered
    }

    /// Left mouse button `action` (IST_PRESS 1, IST_HOLD 2, IST_RELEASE 3) with the cursor at
    /// (x, y) on the page `active`.
    pub(crate) fn click(&mut self, action: u8, active: Option<ObjId>, x: f32, y: f32) -> ClickEffect {
        match action {
            1 => {
                match self.live_side(active, x, y) {
                    Some(s) => {
                        // A press without the release of the previous one: that key first.
                        self.pressed = Some(s);
                        ClickEffect::Press(s)
                    }
                    None => ClickEffect::Pass,
                }
            }
            3 => match self.pressed.take() {
                Some(s) => ClickEffect::Release(s),
                None => ClickEffect::Pass,
            },
            _ if self.pressed.is_some() => ClickEffect::Taken,
            _ => ClickEffect::Pass,
        }
    }
}

/// The bar's mouse buttons and highlight: improved build, Graphics: Enhanced, not on Android.
pub(crate) use crate::improved::input::help_bar_mouse_enabled as enabled;

/// Whether the bar's zones are noted: for the mouse ([`enabled`]) or for a finger (Android).
fn tracked(graphics: crate::video_settings::SessionGraphics) -> bool {
    enabled(graphics) || cfg!(target_os = "android")
}

/// Flags of XIIIWindow.DisplayHelpBar's right side (and InternalOnKeyEvent's Enter).
const RIGHT_FLAGS: [&str; 7] = ["bShowSEL", "bShowRUN", "bShowNXT", "bShowACC", "bShowSCH", "bShowEDT", "bShowCHO"];
/// Flags of its left side.
const LEFT_FLAGS: [&str; 5] = ["bShowRTN", "bShowCCL", "bShowBCK", "bShowDEL", "bShowSAV"];

impl Vm {
    fn help_bar_any(&self, w: ObjId, flags: &[&str]) -> bool {
        flags.iter().any(|f| self.propr(w, f).as_bool())
    }

    /// Whether the bar's right side of `page` is greyed: the page left without a focused control
    /// by the mouse (menu_mouse.rs), Enter having nothing to validate.
    pub(crate) fn help_bar_right_off(&self, page: Option<ObjId>) -> bool {
        page.is_some() && self.improved.menu_mouse.unfocused == page && page.is_some_and(|p| self.propr(p, "FocusedControl").as_object().is_none())
    }

    /// Before XIIIWindow.DisplayHelpBar (Vm::call): its texts noted, on the page the local menu
    /// shows on top. True when the call is followed (help_bar_click_end after it).
    pub(crate) fn help_bar_click_begin(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if self.improved.help_bar.drawing.is_some() || self.functions[f].name != "DisplayHelpBar" || !tracked(self.engine.client.session_graphics) || !self.is_a(this, "XIIIWindow") {
            return false;
        }
        let Some(c) = args.first().and_then(|a| a.as_ref()).and_then(|a| a.as_object()) else { return false };
        let Some(root) = self.propr(this, "myRoot").as_object() else { return false };
        // PC keys only (the consoles' bar draws button pictures).
        if self.propr(root, "CurrentPF").as_int() != 0 {
            return false;
        }
        let ctrl = self.gui_local();
        if ctrl.is_none() || self.propr(this, "Controller").as_object() != ctrl || ctrl.and_then(|g| self.propr(g, "ActivePage").as_object()) != Some(this) {
            return false;
        }
        let (left, right) = (self.help_bar_any(this, &LEFT_FLAGS), self.help_bar_any(this, &RIGHT_FLAGS));
        self.improved.help_bar.begin(this, c, left, right);
        true
    }

    pub(crate) fn help_bar_click_end(&mut self) {
        self.improved.help_bar.end();
    }

    /// Touch screens: a finger put at (x, y) of the menu's page (`ctrl`'s units, those of
    /// MouseX / MouseY): the key of the bar's side under it (Escape or Enter), that side then
    /// drawn highlighted until [`Vm::help_bar_untouch`]: Some(None) when not on a live side of the
    /// bar the active page drew on the last frame. None: the bar is not noted for that menu
    /// (another player's menu, the consoles' bar of button pictures, a page drawn outside the
    /// scripts on top).
    pub(crate) fn help_bar_touch_key(&mut self, ctrl: ObjId, x: f32, y: f32) -> Option<Option<u8>> {
        if self.gui_local() != Some(ctrl) || !tracked(self.engine.client.session_graphics) || self.help_bar_overlay() {
            return None;
        }
        let active = self.propr(ctrl, "ActivePage").as_object();
        let root = active.and_then(|p| self.propr(p, "myRoot").as_object());
        if root.is_some_and(|r| self.propr(r, "CurrentPF").as_int() != 0) {
            return None;
        }
        self.improved.help_bar.right_off = self.help_bar_right_off(active);
        Some(self.improved.help_bar.touch(active, x, y).map(Side::key))
    }

    pub(crate) fn help_bar_untouch(&mut self) {
        self.improved.help_bar.untouch();
    }

    /// Before UCanvas::execDrawText on `c`: the bar's side drawn, with DrawColor put to BlackColor
    /// when that side is under the cursor. What to give help_bar_text_end.
    pub(crate) fn help_bar_text_begin(&mut self, c: ObjId) -> Option<(Side, usize, Option<Value>)> {
        let side = self.improved.help_bar.text_begin(c)?;
        let window = self.improved.help_bar.drawing.as_ref()?.window;
        let colour = if side == Side::Right && self.help_bar_right_off(Some(window)) {
            // Color's fields in their order (B, G, R, A).
            let [r, g, b, a] = DISABLED_GREY;
            Some(Value::Struct("object.color".into(), vec![Value::Byte(b), Value::Byte(g), Value::Byte(r), Value::Byte(a)].into()))
        } else if self.improved.help_bar.highlighted(side) {
            Some(self.prop(window, "BlackColor"))
        } else {
            None
        };
        let saved = colour.map(|colour| {
            let grey = self.prop(c, "DrawColor");
            self.set_prop(c, "DrawColor", colour);
            grey
        });
        Some((side, self.hud.len(), saved))
    }

    /// After the DrawText: its glyphs' rectangle noted, DrawColor put back.
    pub(crate) fn help_bar_text_end(&mut self, c: ObjId, note: Option<(Side, usize, Option<Value>)>) {
        let Some((side, start, saved)) = note else { return };
        if let Some(color) = saved {
            self.set_prop(c, "DrawColor", color);
        }
        let mut rect: Option<[f32; 4]> = None;
        for d in self.hud.iter().skip(start) {
            if let crate::canvas::HudDraw::Tile { x1, y1, x2, y2, .. } = *d {
                rect = Some(union(rect, [x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)]));
            }
        }
        if let Some(r) = rect {
            self.improved.help_bar.text_rect(side, r);
        }
    }

    /// Start and end of the menus' drawing of a frame (GUIController.NativePostRender).
    pub(crate) fn help_bar_frame(&mut self, ctrl: ObjId, end: bool) {
        if self.gui_local() != Some(ctrl) {
            return;
        }
        if end {
            self.improved.help_bar.frame_end();
        } else {
            self.improved.help_bar.frame_begin();
        }
    }

    /// The menu cursor's horizontal range on a page `size_x` wide (UGUIController's cursor
    /// drawing keeps it in 0..=SizeX): widened to the bar's sides when they lie beyond the page
    /// (the in-game bar of a wide window), so that they can be reached.
    pub(crate) fn help_bar_cursor_range(&self, size_x: i32) -> (i32, i32) {
        let (mut lo, mut hi) = (0, size_x);
        if let (true, Some((w, zones))) = (enabled(self.engine.client.session_graphics), self.improved.help_bar.zones) {
            let active = self.gui_local().and_then(|g| self.propr(g, "ActivePage").as_object());
            if active == Some(w) {
                for r in zones.iter().flatten() {
                    lo = lo.min((r[0] - MARGIN).floor() as i32);
                    hi = hi.max((r[2] + MARGIN).ceil() as i32);
                }
            }
        }
        (lo, hi)
    }

    /// After the cursor's place is known (GUIController.NativePreRender): the side under it, and
    /// the tick when it has just entered one.
    pub(crate) fn help_bar_hover(&mut self, ctrl: ObjId) {
        if self.gui_local() != Some(ctrl) {
            return;
        }
        let on = enabled(self.engine.client.session_graphics) && !self.gui_without_mouse(ctrl) && !self.help_bar_overlay();
        let at = on.then(|| (self.propr(ctrl, "MouseX").as_int() as f32, self.propr(ctrl, "MouseY").as_int() as f32));
        let active = self.propr(ctrl, "ActivePage").as_object();
        self.improved.help_bar.right_off = self.help_bar_right_off(active);
        let entered = self.improved.help_bar.hover(active, at);
        // A page entered with the mouse over no control (menu_mouse.rs): its tick once the bar
        // drawn for that page is known (or there is none), the side's own tick standing for it.
        let bar_known = self.improved.help_bar.zones.map_or(true, |(w, _)| Some(w) == active);
        if entered || (self.improved.menu_mouse.page_tick && bar_known) {
            self.improved.menu_mouse.page_tick = false;
            self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
        }
    }

    fn help_bar_overlay(&self) -> bool {

        if self.menu_overlay_open() {
            return true;
        }
        false
    }

    /// GUIController.NativeKeyEvent, left mouse button: taken by the bar when pressed on one of
    /// its sides (that side's key pressed, released with the button). None: not the bar's.
    pub(crate) fn help_bar_click_key(&mut self, ctrl: ObjId, key: u8, action: u8) -> Option<bool> {
        if key != 0x01 || self.gui_local() != Some(ctrl) {
            return None;
        }
        // A key pressed by the bar is always released, even if the bar went away meanwhile.
        if !enabled(self.engine.client.session_graphics) && self.improved.help_bar.pressed.is_none() {
            return None;
        }
        if !enabled(self.engine.client.session_graphics) || self.help_bar_overlay() || self.gui_without_mouse(ctrl) {
            if action != 3 {
                return None;
            }
        }
        let active = self.propr(ctrl, "ActivePage").as_object();
        self.improved.help_bar.right_off = self.help_bar_right_off(active);
        let (x, y) = (self.propr(ctrl, "MouseX").as_int() as f32, self.propr(ctrl, "MouseY").as_int() as f32);
        let effect = if enabled(self.engine.client.session_graphics) || action == 3 { self.improved.help_bar.click(action, active, x, y) } else { ClickEffect::Pass };
        match effect {
            ClickEffect::Pass => None,
            ClickEffect::Taken => Some(true),
            ClickEffect::Press(s) => {
                self.improved.help_bar.injected = Some(s.key());
                self.input_event(s.key(), crate::player_input::IST_PRESS, 0.0);
                Some(true)
            }
            ClickEffect::Release(s) => {
                self.input_event(s.key(), crate::player_input::IST_RELEASE, 0.0);
                Some(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: ObjId = 7;
    const C: ObjId = 9;

    /// The main menu's bar as DisplayHelpBar draws it on an 800 × 600 page: right "Select" then
    /// "Enter", left "Escape" then "Back", then the hint.
    fn drawn(left: bool, right: bool) -> HelpBarClick {
        let mut h = HelpBarClick::default();
        h.frame_begin();
        h.begin(W, C, left, right);
        let texts: Vec<[f32; 4]> = vec![[700.0, 560.0, 740.0, 576.0], [650.0, 560.0, 697.0, 576.0], [30.0, 560.0, 80.0, 576.0], [83.0, 560.0, 120.0, 576.0], [350.0, 560.0, 450.0, 576.0]];
        let mut i = 0;
        let order: Vec<Option<Side>> = (0..5).map(|_| h.text_begin(C)).collect();
        for (n, s) in order.iter().enumerate() {
            let r = if right { texts[n] } else { texts[(n + 2).min(4)] };
            if let Some(s) = s {
                h.text_rect(*s, r);
            }
            i += 1;
        }
        assert_eq!(i, 5);
        h.end();
        h.frame_end();
        h
    }

    #[test]
    fn texts_map_to_their_side() {
        assert_eq!((0..5).map(|n| side_of_text(n, true, true)).collect::<Vec<_>>(), vec![Some(Side::Right), Some(Side::Right), Some(Side::Left), Some(Side::Left), None]);
        assert_eq!((0..3).map(|n| side_of_text(n, true, false)).collect::<Vec<_>>(), vec![Some(Side::Left), Some(Side::Left), None]);
        assert_eq!((0..3).map(|n| side_of_text(n, false, true)).collect::<Vec<_>>(), vec![Some(Side::Right), Some(Side::Right), None]);
        assert_eq!(side_of_text(0, false, false), None);
    }

    #[test]
    fn zone_is_the_union_of_both_texts_with_a_margin() {
        let h = drawn(true, true);
        let (_, z) = h.zones.unwrap();
        assert_eq!(z[1], Some([650.0, 560.0, 740.0, 576.0]));
        assert_eq!(z[0], Some([30.0, 560.0, 120.0, 576.0]));
        // Inside each text, between them, within the margin.
        for (x, y, s) in [(720.0, 568.0, Some(Side::Right)), (660.0, 568.0, Some(Side::Right)), (698.5, 568.0, Some(Side::Right)), (742.0, 558.0, Some(Side::Right)), (40.0, 570.0, Some(Side::Left)), (100.0, 570.0, Some(Side::Left))] {
            assert_eq!(side_at(&z, x, y), s, "{x},{y}");
        }
        // Negative control: the hint, the bar's middle, outside the margin.
        for (x, y) in [(400.0, 568.0), (300.0, 568.0), (645.0, 568.0), (700.0, 552.0), (125.0, 568.0), (40.0, 580.0)] {
            assert_eq!(side_at(&z, x, y), None, "{x},{y}");
        }
    }

    #[test]
    fn only_drawn_sides_are_zones() {
        let h = drawn(true, false);
        let (_, z) = h.zones.unwrap();
        assert!(z[1].is_none());
        assert_eq!(side_at(&z, 720.0, 568.0), None);
        assert_eq!(side_at(&z, 40.0, 568.0), Some(Side::Left));
    }

    #[test]
    fn hover_highlights_and_ticks_once_per_entry() {
        let mut h = drawn(true, true);
        assert!(!h.hover(Some(W), Some((400.0, 300.0))));
        assert!(!h.highlighted(Side::Right) && !h.highlighted(Side::Left));
        assert!(h.hover(Some(W), Some((720.0, 568.0))));
        assert!(h.highlighted(Side::Right) && !h.highlighted(Side::Left));
        // Still inside: no second tick.
        assert!(!h.hover(Some(W), Some((660.0, 568.0))));
        // To the other side: one tick.
        assert!(h.hover(Some(W), Some((40.0, 568.0))));
        assert!(h.highlighted(Side::Left) && !h.highlighted(Side::Right));
        assert!(!h.hover(Some(W), Some((400.0, 300.0))));
        assert!(!h.highlighted(Side::Left));
        // Another page on top (the zones are the previous page's): nothing.
        assert!(!h.hover(Some(W + 1), Some((720.0, 568.0))));
        assert!(!h.highlighted(Side::Right));
        // No cursor (bar disabled, faithful): nothing.
        assert!(!h.hover(Some(W), None));
        assert_eq!(h.hovered, None);
    }

    #[test]
    fn a_frame_without_bar_has_no_zones() {
        let mut h = drawn(true, true);
        h.frame_begin();
        h.frame_end();
        assert!(h.zones.is_none());
        assert!(!h.hover(Some(W), Some((720.0, 568.0))));
        assert_eq!(h.click(1, Some(W), 720.0, 568.0), ClickEffect::Pass);
    }

    #[test]
    fn click_sends_the_sides_key_once_and_is_taken() {
        let mut h = drawn(true, true);
        assert_eq!(h.click(1, Some(W), 720.0, 568.0), ClickEffect::Press(Side::Right));
        assert_eq!(Side::Right.key(), 0x0d);
        assert_eq!(h.click(2, Some(W), 720.0, 568.0), ClickEffect::Taken);
        // Released elsewhere: the same key released, once.
        assert_eq!(h.click(3, Some(W), 300.0, 300.0), ClickEffect::Release(Side::Right));
        assert_eq!(h.click(3, Some(W), 300.0, 300.0), ClickEffect::Pass);
        assert_eq!(h.click(1, Some(W), 50.0, 568.0), ClickEffect::Press(Side::Left));
        assert_eq!(Side::Left.key(), 0x1b);
        assert_eq!(h.click(3, Some(W), 50.0, 568.0), ClickEffect::Release(Side::Left));
        // Negative control: a click on the hint or a control goes to the GUI.
        assert_eq!(h.click(1, Some(W), 400.0, 568.0), ClickEffect::Pass);
        assert_eq!(h.click(2, Some(W), 400.0, 568.0), ClickEffect::Pass);
        assert_eq!(h.click(3, Some(W), 400.0, 568.0), ClickEffect::Pass);
        assert_eq!(h.click(1, Some(W), 400.0, 300.0), ClickEffect::Pass);
        // Another page on top: not the bar's.
        assert_eq!(h.click(1, Some(W + 1), 720.0, 568.0), ClickEffect::Pass);
    }

    /// The greyed right side answers neither the cursor nor the button; the left side still does.
    #[test]
    fn greyed_right_side_is_inert() {
        let mut h = drawn(true, true);
        h.right_off = true;
        assert!(!h.hover(Some(W), Some((720.0, 568.0))));
        assert_eq!(h.hovered, None);
        assert_eq!(h.click(1, Some(W), 720.0, 568.0), ClickEffect::Pass);
        assert_eq!(h.click(3, Some(W), 720.0, 568.0), ClickEffect::Pass);
        assert!(h.hover(Some(W), Some((40.0, 568.0))));
        assert_eq!(h.click(1, Some(W), 50.0, 568.0), ClickEffect::Press(Side::Left));
        assert_eq!(h.click(3, Some(W), 50.0, 568.0), ClickEffect::Release(Side::Left));
        // Negative control: live again, the right side is hovered and clicked.
        h.right_off = false;
        assert!(h.hover(Some(W), Some((720.0, 568.0))));
        assert_eq!(h.click(1, Some(W), 720.0, 568.0), ClickEffect::Press(Side::Right));
    }

    /// A finger (touch screens) put on a side: that side's key, highlighted until lifted; the zone
    /// is the texts' own, not a band at the bottom of the window.
    #[test]
    fn a_finger_on_a_side_gives_its_key_and_highlights_it_until_lifted() {
        let mut h = drawn(true, true);
        assert_eq!(h.touch(Some(W), 720.0, 568.0), Some(Side::Right));
        assert!(h.highlighted(Side::Right) && !h.highlighted(Side::Left));
        // No mouse cursor on a touch screen: a hover without one does not clear the finger's side.
        assert!(!h.hover(Some(W), None));
        assert!(h.highlighted(Side::Right));
        h.untouch();
        assert!(!h.highlighted(Side::Right));
        assert_eq!(h.touch(Some(W), 40.0, 570.0), Some(Side::Left));
        assert!(h.highlighted(Side::Left) && !h.highlighted(Side::Right));
        h.untouch();
        // The whole texts and the finger's margin answer, top to bottom and end to end.
        for (x, y, s) in [(642.0, 552.0, Side::Right), (748.0, 584.0, Side::Right), (22.0, 552.0, Side::Left), (128.0, 584.0, Side::Left), (700.0, 556.0, Side::Right)] {
            assert_eq!(h.touch(Some(W), x, y), Some(s), "{x},{y}");
        }
        // The cursor keeps its narrower margin there.
        assert_eq!(side_at(&h.zones.unwrap().1, 700.0, 556.0), None);
        // Negative controls: below or above the texts (a fixed band at the window's bottom would
        // take these), the hint, another page on top, the greyed right side.
        for (x, y) in [(720.0, 590.0), (720.0, 598.0), (40.0, 586.0), (720.0, 550.0), (400.0, 568.0), (630.0, 568.0)] {
            assert_eq!(h.touch(Some(W), x, y), None, "{x},{y}");
            assert!(!h.highlighted(Side::Right) && !h.highlighted(Side::Left));
        }
        assert_eq!(h.touch(Some(W + 1), 720.0, 568.0), None);
        h.right_off = true;
        assert_eq!(h.touch(Some(W), 720.0, 568.0), None);
        assert!(!h.highlighted(Side::Right));
    }

    #[test]
    fn a_text_on_another_canvas_is_not_the_bars() {
        let mut h = HelpBarClick::default();
        h.begin(W, C, true, true);
        assert_eq!(h.text_begin(C + 1), None);
        assert_eq!(h.text_begin(C), Some(Side::Right));
    }

}

