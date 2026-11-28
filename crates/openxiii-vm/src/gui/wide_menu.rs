//! Improved build: the in-game menus' comic-strip bands on a screen wider than 4:3.
//!
//! The whole window's menus are drawn on the virtual screen of a 4:3 frame (gui.rs, MenuFrame),
//! then placed centred in the window. The in-game pages (pause menu, save page, skills page) draw
//! bands that run to the edges of their screen: UCanvas::execDrawMsgboxBackground
//! with its bands, XIIIMenuInGame.SetObjectives' strip behind the objectives, and
//! XIIIWindow.DisplayHelpBar's bar at the bottom. On a wide window they stopped at the 4:3 frame
//! and the game showed on both sides. Here those three, and only those, reach the window's edges:
//!
//! - the bands and the strip: a tile ending on the virtual screen's left or right edge ends on the
//!   window's edge instead (plain white or black tiles, nothing visible is stretched);
//! - the help bar: drawn on a canvas whose origin and clip span the whole window, so the bar keeps
//!   its 30-pixel margins from the window's edges and its texts follow its ends.
//!
//! Everything else on the pages (texts, buttons, pictures) keeps its place in the 4:3 frame.

use crate::canvas::HudDraw;
use crate::gui::MenuFrame;
use crate::types::FuncId;
use crate::value::ObjId;
use crate::Vm;

/// What a range of primitives was drawn by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WideMark {
    /// DrawMsgboxBackground's bands: any tile on the left or right edge.
    Bands,
    /// SetObjectives' strip: a tile spanning the whole virtual width.
    Strip,
}

/// The whole window's menu frame of the GUI frame being drawn, in the virtual screen's units.
#[derive(Debug, Default)]
pub(crate) struct WideMenu {
    /// The window's left and right edges on the virtual screen.
    pub(crate) left: f32,
    pub(crate) right: f32,
    /// The virtual screen's width.
    pub(crate) width: f32,
    /// Primitives (start, end in Vm::hud) to bring to the window's edges.
    pub(crate) marks: Vec<(usize, usize, WideMark)>,
    /// A hooked script call is running (Vm::call comes back through the hook).
    busy: bool,
}

impl WideMenu {
    /// The frame of a `canvas_w`-wide window; None when the frame already spans the window.
    pub(crate) fn of_frame(frame: &MenuFrame, canvas_w: f32) -> Option<WideMenu> {
        let (left, right) = window_edges(frame, canvas_w);
        let width = frame.virtual_size.0 as f32;
        (left < -0.5 || right > width + 0.5).then(|| WideMenu { left, right, width, ..Default::default() })
    }
}

/// The window's left and right edges on the frame's virtual screen.
pub(crate) fn window_edges(frame: &MenuFrame, canvas_w: f32) -> (f32, f32) {
    (-frame.x / frame.scale, (canvas_w - frame.x) / frame.scale)
}

/// The marked primitives brought to the window's edges (virtual screen's units, before the
/// frame's placing).
pub(crate) fn extend_marks(hud: &mut [HudDraw], marks: &[(usize, usize, WideMark)], left: f32, right: f32, width: f32) {
    for &(start, end, mark) in marks {
        let end = end.min(hud.len());
        for d in hud.iter_mut().take(end).skip(start) {
            let HudDraw::Tile { x1, x2, .. } = d else { continue };
            let (on_left, on_right) = (*x1 <= 0.5, *x2 >= width - 0.5);
            if mark == WideMark::Strip && !(on_left && on_right) {
                continue;
            }
            if on_left {
                *x1 = left;
            }
            if on_right {
                *x2 = right;
            }
        }
    }
}

/// The help bar's canvas origin and clip width spanning the window (`org_x`, `clip_x`: the page's).
pub(crate) fn help_bar_span(org_x: f32, clip_x: f32, left: f32, right: f32, width: f32) -> (f32, f32) {
    (org_x + left, clip_x - left + (right - width))
}

/// A hooked script call in progress (Vm::wide_menu_call_end).
pub(crate) enum WideCall {
    Strip(usize),
    HelpBar { c: ObjId, org_x: f32, clip_x: f32 },
}

impl Vm {
    /// Starts the frame's marking (Vm::menu_frame_begin, whole window only).
    pub(crate) fn wide_menu_begin(&mut self, frame: &MenuFrame, canvas_w: f32) {
        self.improved.wide_menu = WideMenu::of_frame(frame, canvas_w);
    }

    /// Brings the marked primitives from `start` on to the window's edges (Vm::menu_frame_end,
    /// before the frame's placing) and ends the marking.
    pub(crate) fn wide_menu_end(&mut self, start: usize) {
        let Some(w) = self.improved.wide_menu.take() else { return };
        let marks: Vec<_> = w.marks.iter().filter(|m| m.0 >= start).copied().collect();
        extend_marks(&mut self.hud, &marks, w.left, w.right, w.width);
    }

    /// DrawMsgboxBackground's bands drawn from `start` (canvas.rs).
    pub(crate) fn wide_menu_note_bands(&mut self, start: usize) {
        let end = self.hud.len();
        if let Some(w) = self.improved.wide_menu.as_mut() {
            w.marks.push((start, end, WideMark::Bands));
        }
    }

    /// Before a script call (Vm::call): XIIIMenuInGame.SetObjectives and the in-game pages'
    /// XIIIWindow.DisplayHelpBar are hooked while a wide frame is drawn.
    pub(crate) fn wide_menu_call(&mut self, f: FuncId, this: ObjId, c: Option<ObjId>) -> Option<WideCall> {
        let w = self.improved.wide_menu.as_ref()?;
        if w.busy {
            return None;
        }
        let (left, right, width) = (w.left, w.right, w.width);
        let name = self.functions[f].name.as_str();
        let call = match name {
            "SetObjectives" if self.is_a(this, "XIIIMenuInGame") => WideCall::Strip(self.hud.len()),
            "DisplayHelpBar" if self.is_a(this, "XIIIWindow") => {
                let root = self.propr(this, "myRoot").as_object()?;
                // In game, keyboard or gamepad help bar alike (a split-screen player's gamepad
                // bar has the gamepad's buttons); a multiplayer game too (bIamInGame is only set
                // on a map with a MapInfo, the campaign's).
                if !self.propr(root, "bIamInGame").as_bool() && !self.propr(root, "bIamInMulti").as_bool() {
                    return None;
                }
                let c = c?;
                let (org_x, clip_x) = (self.cf(c, "OrgX"), self.cf(c, "ClipX"));
                let (o, cl) = help_bar_span(org_x, clip_x, left, right, width);
                self.set_cf(c, "OrgX", o);
                self.set_cf(c, "ClipX", cl);
                WideCall::HelpBar { c, org_x, clip_x }
            }
            _ => return None,
        };
        if let Some(w) = self.improved.wide_menu.as_mut() {
            w.busy = true;
        }
        Some(call)
    }

    pub(crate) fn wide_menu_call_end(&mut self, call: WideCall) {
        let end = self.hud.len();
        match call {
            WideCall::Strip(start) => {
                if let Some(w) = self.improved.wide_menu.as_mut() {
                    w.marks.push((start, end, WideMark::Strip));
                }
            }
            WideCall::HelpBar { c, org_x, clip_x } => {
                if self.objects.get(c).is_some_and(|o| o.is_some()) {
                    self.set_cf(c, "OrgX", org_x);
                    self.set_cf(c, "ClipX", clip_x);
                }
            }
        }
        if let Some(w) = self.improved.wide_menu.as_mut() {
            w.busy = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x1: f32, x2: f32) -> HudDraw {
        HudDraw::Tile { x1, y1: 10.0, x2, y2: 20.0, uv: [0.0; 4], material: "w".into(), color: [255; 4], point: false, enlarged: false, glyph: false }
    }

    fn xs(d: &HudDraw) -> (f32, f32) {
        match d {
            HudDraw::Tile { x1, x2, .. } => (*x1, *x2),
            _ => unreachable!(),
        }
    }

    /// The frame of a window at UIScale 1, as Vm::window_menu_frame picks it: the scaled frame,
    /// or the unscaled 4:3 frame on a window no larger than the reference.
    fn frame(w: i32, h: i32) -> MenuFrame {
        MenuFrame::of_window(w, h, 1.0).unwrap_or_else(|| {
            let [x, y, vw, vh] = crate::gui::menu_frame_43(w, h);
            MenuFrame { bicubic: false, x: x as f32, y: y as f32, scale: 1.0, virtual_size: (vw, vh) }
        })
    }

    /// The pause menu's primitives: bands (top full width, bottom from ox, centre box), strip,
    /// a text glyph.
    fn page(width: f32) -> Vec<HudDraw> {
        vec![tile(0.0, width), tile(330.0, width), tile(330.0, 660.0), tile(0.0, width), tile(60.0, 70.0)]
    }

    fn placed(w: i32, h: i32, marks: &[(usize, usize, WideMark)]) -> (Vec<(f32, f32)>, MenuFrame) {
        let f = frame(w, h);
        let width = f.virtual_size.0 as f32;
        let mut d = page(width);
        if let Some(m) = WideMenu::of_frame(&f, w as f32) {
            extend_marks(&mut d, marks, m.left, m.right, m.width);
        }
        f.place_page(&mut d, (w as f32, h as f32));
        (d.iter().map(xs).collect(), f)
    }

    const MARKS: [(usize, usize, WideMark); 2] = [(0, 3, WideMark::Bands), (3, 5, WideMark::Strip)];

    #[test]
    fn wide_windows_bands_and_strip_reach_the_window_edges() {
        for (w, h) in [(1920, 1080), (2560, 1080), (3840, 1080), (1280, 720)] {
            let (d, f) = placed(w, h, &MARKS);
            let close = |a: f32, b: f32| (a - b).abs() < 0.01;
            assert!(close(d[0].0, 0.0) && close(d[0].1, w as f32), "{w}x{h} top band {:?}", d[0]);
            // The bottom band keeps its start in the frame and ends on the window's edge.
            assert!(close(d[1].0, f.x + 330.0 * f.scale) && close(d[1].1, w as f32), "{w}x{h} bottom band {:?}", d[1]);
            // The centre box and the texts keep their place.
            assert!(close(d[2].0, f.x + 330.0 * f.scale) && close(d[2].1, f.x + 660.0 * f.scale));
            assert!(close(d[3].0, 0.0) && close(d[3].1, w as f32), "{w}x{h} strip {:?}", d[3]);
            assert!(close(d[4].0, f.x + 60.0 * f.scale) && close(d[4].1, f.x + 70.0 * f.scale));
        }
    }

    #[test]
    fn a_4_3_window_and_unmarked_primitives_are_unchanged() {
        // 4:3: the frame spans the window, nothing to do.
        assert!(WideMenu::of_frame(&frame(1600, 1200), 1600.0).is_none());
        // Without marks, the primitives are placed as before (the frame's edges).
        let (d, f) = placed(1920, 1080, &[]);
        assert_eq!(d[0], (f.x, f.x + f.virtual_size.0 as f32 * f.scale));
        // The strip rule leaves a tile on one edge only alone.
        let mut one = vec![tile(0.0, 300.0)];
        extend_marks(&mut one, &[(0, 1, WideMark::Strip)], -160.0, 1120.0, 960.0);
        assert_eq!(xs(&one[0]), (0.0, 300.0));
    }

    #[test]
    fn the_help_bar_keeps_its_margins_from_the_window_edges() {
        let f = frame(2560, 1080);
        let m = WideMenu::of_frame(&f, 2560.0).unwrap();
        let (org, clip) = help_bar_span(0.0, m.width, m.left, m.right, m.width);
        // XPos = 30, BarWidth = ClipX - 60: on the window, 30 virtual pixels from each edge.
        let (x1, x2) = (f.to_canvas(org + 30.0, 0.0).0, f.to_canvas(org + 30.0 + clip - 60.0, 0.0).0);
        assert!((x1 - 30.0 * f.scale).abs() < 0.01 && (x2 - (2560.0 - 30.0 * f.scale)).abs() < 0.01, "{x1} {x2}");
    }
}
