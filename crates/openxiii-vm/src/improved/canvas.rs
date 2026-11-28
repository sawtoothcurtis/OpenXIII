//! Wide-screen and interactive menu drawing for the improved edition.
use crate::value::Value;
use crate::{ObjId, Vm};

/// The HUD being drawn on its virtual screen (Vm::hud_scale_begin): the canvas fields and the
/// viewport's size to put back, and the first primitive drawn on it.
pub(crate) struct HudScale {
    screen: crate::gui::HudScreen,
    saved: Vec<Value>,
    viewport: (i32, i32),
    start: usize,
}

const HUD_SCALE_FIELDS: [&str; 4] = ["ClipX", "ClipY", "SizeX", "SizeY"];

impl Vm {
    pub(crate) fn menu_tile_style(&self, inner: &str, style: u8, xl: f32, yl: f32, ul: f32) -> u8 {
        // Improved build: the size a menu character's tile is shown at (menu_figures.rs).
        if ul > 0.0 && crate::menu_figures::maybe_tile(inner) {
            let frame = self.menu_layout.last_frame.map_or(1.0, |f| f.scale);
            crate::menu_figures::note_shown(inner, xl / ul * frame);
            crate::menu_figures::note_shown_size(inner, xl * frame, yl * frame);
        }
        // A character's tile with its edge anti-aliased is drawn with its alpha: a page that
        // draws the game's hard-masked tile as it is, or with an alpha test, would cut the soft
        // edge back to all or nothing.
        if matches!(style, 1 | 2) && crate::menu_figures::maybe_tile(inner) && openxiii_assets::mod_textures::is_runtime(inner) {
            5
        } else {
            style
        }
    }

    pub(crate) fn begin_message_bands(&self) -> usize {
        self.hud.len()
    }

    pub(crate) fn finish_message_bands(&mut self, start: usize) {
        self.wide_menu_note_bands(start);
    }

    pub(crate) fn begin_canvas_text(&mut self, c: ObjId) -> Option<(crate::help_bar_click::Side, usize, Option<Value>)> {
        if self.improved.help_bar.drawing.is_some() {
            self.help_bar_text_begin(c)
        } else {
            None
        }
    }

    pub(crate) fn finish_canvas_text(&mut self, c: ObjId, note: Option<(crate::help_bar_click::Side, usize, Option<Value>)>) {
        self.help_bar_text_end(c, note);
    }

    pub(crate) fn reset_loading_origin(&mut self, canvas: ObjId) {
        // Improved build: the loading screen (XIIIBaseHud.DrawLevelAction, LEVACT_Loading) is a whole
        // window's picture drawn from (0, 0): the menu pages' pre-draw (XIIIWindow.InternalOnPreDraw,
        // SetOrigin to centre the page) must not move it (it was drawn shifted, the page around it
        // left showing).
        if crate::display_format::active(self.engine.client.session_graphics) && self.level.is_some_and(|l| self.propr(l, "LevelAction").as_int() == 1) {
            self.set_cf(canvas, "OrgX", 0.0);
            self.set_cf(canvas, "OrgY", 0.0);
        }
    }

    /// Improved build, HELP_BAR_FIT: XIIIWindow.DisplayHelpBar on PC out of a game puts the bar
    /// at 600 − BarHeight − 6 whenever the canvas is wider than 800, as if every such mode were at
    /// least 600 high (true of the 4:3 modes the game offered): in a lower window (1024 × 576,
    /// 854 × 480) the bar went out of the window. The canvas is moved up by what is missing for
    /// that call. In a game the bar goes at the window's bottom (ClipY − BarHeight − 6), but the
    /// pages that centre themselves in a window larger than 800 × 600 (bCenterInGame: Controls and
    /// its pages) have moved the canvas origin first, so the bar was drawn below the window and the
    /// in-game Controls pages had none: their bar is drawn from the window's corner, as the pause
    /// menu's. Returns the canvas, its OrgX, OrgY and ClipY to restore.
    pub(crate) fn help_bar_fit(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> Option<(ObjId, f32, f32, f32)> {
        if self.functions[f].name != "DisplayHelpBar" || !self.is_a(this, "XIIIWindow") {
            return None;
        }
        // Our own Co-op page always keeps its bar in the window; the game's pages when HELP_BAR_FIT.
        if !help_bar_fit_on(self.engine.client.session_graphics) && self.improved.menu_boxes.coop_page.page != Some(this) {
            return None;
        }
        let c = args.first()?.as_ref()?.as_object()?;
        let (clip_x, clip_y) = (self.cf(c, "ClipX"), self.cf(c, "ClipY"));
        let root = self.propr(this, "myRoot").as_object()?;
        let pc = self.propr(root, "CurrentPF").as_int() == 0;
        let in_game = self.propr(root, "bIamInGame").as_bool() || self.propr(root, "bIamInMulti").as_bool();
        let (org_x, org_y) = (self.cf(c, "OrgX"), self.cf(c, "OrgY"));
        let lift = help_bar_lift(clip_x, clip_y, pc && !in_game);
        // XIIIWindow.InternalOnRender's centring of a bCenterInGame page (out of the main menu's map).
        let centred = pc && in_game && !self.propr(root, "bMapMenu").as_bool() && self.propr(this, "bCenterInGame").as_bool();
        let (win_left, win_top) = (self.propr(this, "WinLeft").as_float(), self.propr(this, "WinTop").as_float());
        let shift = centring_shift(clip_x, clip_y, win_left, win_top).filter(|_| centred && lift.is_none());
        if lift.is_none() && shift.is_none() {
            return None;
        }
        // The call made with the moved canvas comes back through Vm::call: not moved again.
        if std::mem::replace(&mut self.improved.help_bar.lifting, true) {
            return None;
        }
        if let Some(lift) = lift {
            // Origin moved up and clip height grown by as much: the clip keeps the window's bottom.
            self.set_cf(c, "OrgY", org_y - lift);
            self.set_cf(c, "ClipY", clip_y + lift);
        } else if let Some((dx, dy)) = shift {
            self.set_cf(c, "OrgX", org_x - dx);
            self.set_cf(c, "OrgY", org_y - dy);
        }
        Some((c, org_x, org_y, clip_y))
    }

    pub(crate) fn help_bar_fit_end(&mut self, c: ObjId, org_x: f32, org_y: f32, clip_y: f32) {
        self.improved.help_bar.lifting = false;
        if self.objects.get(c).is_some_and(|o| o.is_some()) {
            self.set_cf(c, "OrgX", org_x);
            self.set_cf(c, "OrgY", org_y);
            self.set_cf(c, "ClipY", clip_y);
        }
    }

    /// Improved build (display_format.rs): the canvas of a call of XIIIBaseHud.HUDSetup, `None`
    /// for any other function.
    pub(crate) fn hud_setup_wide_canvas(&self, f: crate::types::FuncId, args: &[Option<Value>]) -> Option<ObjId> {
        let d = &self.functions[f];
        if d.name != "HUDSetup" || !self.classes[d.owner].name.eq_ignore_ascii_case("XIIIBaseHud") {
            return None;
        }
        args.first()?.as_ref()?.as_object()
    }

    /// After XIIIBaseHud.HUDSetup on a window wider than 4:3 (improved build): HUDSetup halves
    /// the HUD (fDrawScale = 0.5) when ClipX / ClipY > 2 or ClipY < 440, a rule made for the
    /// split-screen views of a 4:3 screen, so a 21:9 window or a phone got a half-size HUD. The
    /// ratio is taken as the same view would have on a 4:3 window of the same height: full screen
    /// is never halved, a split-screen view is halved as on a 4:3 screen.
    pub(crate) fn hud_setup_wide(&mut self, hud: ObjId, c: ObjId) {
        if self.objects.get(c).is_none_or(|o| o.is_none()) || self.objects.get(hud).is_none_or(|o| o.is_none()) {
            return;
        }
        let window = self.display_window_size();
        if crate::display_format::hor_plus(window.0, window.1, self.engine.client.session_graphics).is_none() {
            return;
        }
        let (x, y) = (self.cf(c, "ClipX"), self.cf(c, "ClipY"));
        let ratio = crate::display_format::ratio_as_43((x, y), window, self.engine.client.session_graphics);
        self.set_cf(hud, "fDrawScale", if ratio > 2.0 || y < 440.0 { 0.5 } else { 1.0 });
    }

    /// Improved build: the black line between the views of a local co-op split screen, as the
    /// multiplayer HUD draws it (XIIIMPHud.DrawViewPortSeparator, at the end of DrawHud): each
    /// side of player `pc`'s view that touches another view gets a black band, 1 % of the view's
    /// height on a top or bottom side, 0.75 % of its width on a left or right side, with the
    /// HUD's own BlackTex and BlackColor. The co-op HUD is the solo one, which never draws it; a
    /// view alone on the screen (solo, network co-op) touches no other view.
    pub(crate) fn coop_view_separator(&mut self, pc: ObjId, hud: ObjId, canvas: ObjId) {
        if !self.coop.on || self.coop.versus || self.improved.character_screen.on {
            return;
        }
        if self.level.is_some_and(|l| self.propr(l, "LevelAction").as_int() == 1) {
            return;
        }
        let client = self.client_size;
        let [x, y, w, h] = self.viewport_rect_of(pc, client);
        if w <= 0 || h <= 0 {
            return;
        }
        let [top, bottom, left, right] = separator_sides([x, y, w, h], client);
        if !(top || bottom || left || right) {
            return;
        }
        let tex = self.prop(hud, "BlackTex");
        if matches!(tex, Value::Object(None) | Value::Void) {
            return;
        }
        let (clip_x, clip_y) = (self.cf(canvas, "ClipX"), self.cf(canvas, "ClipY"));
        let (band_h, band_w) = (clip_y * 0.01, clip_x * 0.0075);
        let (u, v) = match &tex {
            Value::Object(Some(t)) => (self.prop(*t, "USize").as_float(), self.prop(*t, "VSize").as_float()),
            _ => (1.0, 1.0),
        };
        let saved = (self.prop(canvas, "DrawColor"), self.prop(canvas, "Style"), self.prop(canvas, "bUseBorder"));
        self.set_prop(canvas, "bUseBorder", Value::Bool(false));
        self.set_prop(canvas, "Style", Value::Byte(1));
        let black = self.prop(hud, "BlackColor");
        self.set_prop(canvas, "DrawColor", black);
        let mut band = |vm: &mut Vm, bx: f32, by: f32, bw: f32, bh: f32| {
            let _ = vm.call_named(canvas, "SetPos", vec![Value::Float(bx), Value::Float(by)]);
            let _ = vm.call_named(
                canvas,
                "DrawTile",
                vec![tex.clone(), Value::Float(bw), Value::Float(bh), Value::Float(0.0), Value::Float(0.0), Value::Float(u), Value::Float(v)],
            );
        };
        if top {
            band(self, 0.0, 0.0, clip_x, band_h);
        }
        if bottom {
            band(self, 0.0, clip_y - band_h, clip_x, band_h);
        }
        if left {
            band(self, 0.0, 0.0, band_w, clip_y);
        }
        if right {
            band(self, clip_x - band_w, 0.0, band_w, clip_y);
        }
        self.set_prop(canvas, "DrawColor", saved.0);
        self.set_prop(canvas, "Style", saved.1);
        self.set_prop(canvas, "bUseBorder", saved.2);
    }

    /// Improved build: the HUD scaled with the screen (gui::HudScreen). The canvas and the
    /// viewport's size are put on the HUD's virtual screen for HUD.PostRender (its scripts and
    /// WorldToScreen lay out and project on it); `hud_scale_end` puts them back and scales the
    /// primitives drawn meanwhile onto the viewport. None: faithful build, a viewport at most the
    /// reference size, or the co-op character screen (it scales its own page).
    pub(crate) fn hud_scale_begin(&mut self, c: ObjId) -> Option<HudScale> {
        if !crate::display_format::active(self.engine.client.session_graphics) {
            return None;
        }
        // The screen's own page; its loading frame (leaving it) is scaled as any other.
        if self.improved.character_screen.on && !self.level.is_some_and(|l| self.propr(l, "LevelAction").as_int() == 1) {
            return None;
        }
        let mut screen = crate::gui::HudScreen::of_viewport(self.viewport.0, self.viewport.1, self.ui_scale())?;
        screen.bicubic = crate::gui::enlarged_text_bicubic(self.engine.client.session_graphics);
        let (w, h) = screen.virtual_size;
        let saved: Vec<Value> = HUD_SCALE_FIELDS.iter().map(|n| self.prop(c, n)).collect();
        self.set_cf(c, "ClipX", w as f32);
        self.set_cf(c, "ClipY", h as f32);
        self.set_prop(c, "SizeX", Value::Int(w));
        self.set_prop(c, "SizeY", Value::Int(h));
        let viewport = std::mem::replace(&mut self.viewport, (w, h));
        Some(HudScale { screen, saved, viewport, start: self.hud.len() })
    }

    pub(crate) fn hud_scale_end(&mut self, c: ObjId, scaled: Option<HudScale>) {
        let Some(HudScale { screen, saved, viewport, start }) = scaled else { return };
        for (n, v) in HUD_SCALE_FIELDS.iter().zip(saved) {
            self.set_prop(c, n, v);
        }
        self.viewport = viewport;
        let start = start.min(self.hud.len());
        for d in &mut self.hud[start..] {
            screen.place(d);
        }
        // Lockstep co-op: the icons over a world point move by viewport pixels.
        for a in self.hud_anchors.iter_mut().filter(|a| a.start >= start) {
            a.scale = screen.scale;
        }
    }
}

/// Improved build: the game's help bar kept in low wide windows (on; the faithful build keeps the
/// original's place, out of the window there).
const HELP_BAR_FIT: bool = true;

fn help_bar_fit_on(graphics: crate::video_settings::SessionGraphics) -> bool {
    // Graphics: Original keeps the original's place.
    HELP_BAR_FIT && graphics.enhanced()
}

/// How far up the help bar's canvas goes for a `clip_x` × `clip_y` canvas: only on the PC menu
/// path (`pc_menu`) that places the bar at 600 − height when wider than 800, and only by what the
/// window lacks under 600; never in the faithful build.
pub(crate) fn help_bar_lift(clip_x: f32, clip_y: f32, pc_menu: bool) -> Option<f32> {
    (pc_menu && clip_x > 800.0 && clip_y < 600.0).then_some(600.0 - clip_y)
}

/// The origin shift XIIIWindow.InternalOnRender gives a centred page on a `clip_x` × `clip_y`
/// canvas (the 800 × 600 page in the middle of a larger window), `None` when there is none.
pub(crate) fn centring_shift(clip_x: f32, clip_y: f32, win_left: f32, win_top: f32) -> Option<(f32, f32)> {
    let dx = if clip_x > 800.0 { win_left * clip_x + (clip_x - 800.0) / 2.0 } else { 0.0 };
    let dy = if clip_y > 600.0 { win_top * clip_y + (clip_y - 600.0) / 2.0 } else { 0.0 };
    (dx != 0.0 || dy != 0.0).then_some((dx, dy))
}

#[cfg(test)]
mod help_bar_tests {
    use super::{centring_shift, help_bar_lift};

    /// In-game Controls pages (bCenterInGame) in a window larger than 800 × 600: the bar is drawn
    /// as if the page were not centred, at the window's bottom like the pause menu's.
    #[test]
    fn a_centred_in_game_page_draws_its_bar_from_the_window_corner() {
        assert_eq!(centring_shift(1280.0, 960.0, 0.0, 0.0), Some((240.0, 180.0)));
        assert_eq!(centring_shift(1920.0, 1080.0, 0.0, 0.0), Some((560.0, 240.0)));
        // 800 × 600 and smaller: the page is not moved, nor is the bar.
        assert_eq!(centring_shift(800.0, 600.0, 0.0, 0.0), None);
        assert_eq!(centring_shift(640.0, 480.0, 0.0, 0.0), None);
    }

    #[test]
    fn the_bar_is_lifted_only_in_the_improved_build_and_only_where_it_was_out_of_the_window() {
        assert!(super::HELP_BAR_FIT, "on by default in the improved build");
        assert_eq!(help_bar_lift(1024.0, 576.0, true), Some(24.0));
        assert_eq!(help_bar_lift(854.0, 480.0, true), Some(120.0));
        // 4:3 modes, 800 wide or less, 600 high or more: the original's own place.
        assert_eq!(help_bar_lift(1024.0, 768.0, true), None);
        assert_eq!(help_bar_lift(800.0, 450.0, true), None);
        assert_eq!(help_bar_lift(1280.0, 720.0, true), None);
        // In game, in multiplayer or on consoles the original already uses the window's height.
        assert_eq!(help_bar_lift(1024.0, 576.0, false), None);
    }
}

/// Improved build: which sides (top, bottom, left, right) of a view at `rect` (x, y, width,
/// height) in a `client` window touch another view: those get the split screen's black line.
pub(crate) fn separator_sides(rect: [i32; 4], client: (i32, i32)) -> [bool; 4] {
    let [x, y, w, h] = rect;
    [y > 0, y + h < client.1, x > 0, x + w < client.0]
}

#[cfg(test)]
mod separator_tests {
    use super::separator_sides;

    /// The original's two stacked views, the improved build's side by side views and the
    /// quarters: a line on each inner side only. Negative control: a view alone (solo, network
    /// co-op) has none.
    #[test]
    fn a_line_on_each_side_shared_with_another_view() {
        let c = (1920, 1080);
        assert_eq!(separator_sides([0, 0, 1920, 540], c), [false, true, false, false], "top view");
        assert_eq!(separator_sides([0, 540, 1920, 540], c), [true, false, false, false], "bottom view");
        assert_eq!(separator_sides([0, 0, 960, 1080], c), [false, false, false, true], "left view");
        assert_eq!(separator_sides([960, 0, 960, 1080], c), [false, false, true, false], "right view");
        assert_eq!(separator_sides([960, 540, 960, 540], c), [true, false, true, false], "bottom right quarter");
        assert_eq!(separator_sides([0, 0, 1920, 1080], c), [false; 4], "a view alone");
    }
}
