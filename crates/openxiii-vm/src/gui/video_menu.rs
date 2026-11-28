//! Improved build only: the new lines of the Video options page (XIIIMenuVideoClientWindow, PC).
//!
//! After the page's Created, combo boxes like its Resolution line are added: Display mode
//! (window, borderless fullscreen, exclusive fullscreen; not on Android), V-sync (Off or On:
//! VSync of [OpenXIII.Video]; never greyed, it does not change the look; Left /
//! Right go round from one end to the other, so both arrows always show), Graphics (Original or
//! Enhanced, video_settings::graphics_enhanced), then after Resolution,
//! Render scale, Anti-aliasing (MSAA), Texture filtering (anisotropic), Shadows (Auto, Low,
//! High, Very high: side of the shadow targets) and Interface size (UIScale, the menus' size).
//! The page's own lines
//! are moved so that all of them share its 370-unit panel with the page's own spacing formula
//! (LineSpace = 29 + (370 - n * 29) / (n + 1), YOrg = 27 + LineSpace), which also places the
//! focus highlight drawn by Paint.
//!
//! Left / Right change a new line (as the page does for Resolution); Enter writes the values to
//! [OpenXIII.Video] (video_settings.rs) and asks the display for the new mode before the page's
//! own Enter (SETRES with the chosen resolution, SaveConfig, which writes the ini files); Escape
//! leaves them. Default also puts the new quality lines back to their defaults.
//!
//! Graphics: Original (the default) shows the original's page: its own lines (Brightness,
//! Gamma, Contrast, Resolution with the page's own list, GetAvailableRes's 4:3 modes, and
//! Default) and the Graphics line only, the way back to Enhanced. Display mode, V-sync and the
//! quality lines are hidden: the display mode stays as chosen, V-sync follows the original's
//! UseVSync key, and the quality values are kept and come back with Enhanced; Enter writes them
//! unchanged, so that switching back and forth compares both looks without losing a setting.

use crate::added_text::Text;
use crate::value::{ObjId, Value};
use crate::video_settings::{self as vs, FullscreenMode, VideoSettings, VsyncMode};
use crate::{Result, Vm};

/// The page's new combo boxes.
#[derive(Default, Clone, Debug)]
pub struct VideoMenu {
    pub page: Option<ObjId>,
    pub mode: Option<ObjId>,
    pub scale: Option<ObjId>,
    pub msaa: Option<ObjId>,
    pub anisotropy: Option<ObjId>,
    pub shadows: Option<ObjId>,
    pub graphics: Option<ObjId>,
    /// V-sync: Off, On (Classic) (VsyncMode::ALL). Never greyed (it does not change
    /// the look); it goes round, with both arrows shown.
    pub vsync: Option<ObjId>,
    /// Unapplied choices for Original and Enhanced while the video page is open.
    vsync_choices: [Option<VsyncMode>; 2],
    pub res: Option<ObjId>,
    /// Graphics as chosen on the page (Enhanced: true).
    pub enhanced: bool,
    /// With Original chosen: the Enhanced values the greyed lines stand for (quality, shadows).
    pub kept: Option<(VideoSettings, u32)>,
    /// Resolution line's lists: every mode of the monitor (Enhanced) and the page's own (Original).
    pub res_all: Vec<(u32, u32)>,
    pub res_original: Vec<(u32, u32)>,
    /// TextColor of a quality line before it was greyed.
    pub text_color: Option<Value>,
    pub ui_scale: Option<ObjId>,
    /// Values of the three quality lines, as listed when the page was built (what the device
    /// accepted then, `Vm::video_caps`).
    pub scale_choices: Vec<u32>,
    pub msaa_choices: Vec<u32>,
    pub anisotropy_choices: Vec<u32>,
    /// Sides of the shadow targets offered (0: Auto).
    pub shadow_choices: Vec<u32>,
    pub ui_scale_choices: Vec<u32>,
    /// The page's own lines before Resolution (Brightness, Gamma, Contrast) and after it
    /// (Default), as Created made them.
    before: Vec<Option<ObjId>>,
    after: Vec<Option<ObjId>>,
    /// Display mode line on Borderless (Graphics: Enhanced): the Resolution line greyed on the
    /// desktop's size, with its index and text colour from before, given back when the line
    /// leaves Borderless.
    res_kept: Option<(i32, Value)>,
    /// Enter with the Display mode line on Borderless: the page's SETRES keeps the saved window
    /// and fullscreen sizes (a borderless window always covers the desktop).
    pub(crate) keep_sizes: bool,
}

/// The page's spacing (XIIIMenuVideoClientWindow.Created): (LineSpace, YOrg) for `n` lines.
pub fn line_layout(n: i32) -> (i32, i32) {
    let line_space = 29 + (370 - n * 29) / (n + 1);
    (line_space, 27 + line_space)
}

/// The Display mode line is offered (not on Android: the screen is the window).
pub const MODE_LINE: bool = !cfg!(target_os = "android");

/// Text of each choice of a line.
pub fn mode_label(m: FullscreenMode) -> Text {
    match m {
        FullscreenMode::Windowed => Text::VideoWindowed,
        FullscreenMode::Borderless => Text::VideoBorderless,
        FullscreenMode::Exclusive => Text::VideoExclusive,
    }
}

/// "100%", "4x"; 1 sample (or no anisotropy) is "Off".
fn factor_label(v: u32, off: &str) -> String {
    if v <= 1 {
        off.to_string()
    } else {
        format!("{v}x")
    }
}

/// Text of a shadow quality choice (side of the shadow targets, 0: Auto).
pub fn shadow_label(v: u32) -> Text {
    match v {
        256 => Text::VideoShadowsLow,
        1024 => Text::VideoShadowsHigh,
        2048 => Text::VideoShadowsVeryHigh,
        _ => Text::VideoShadowsAuto,
    }
}

impl Vm {
    /// Hook of XIIIMenuVideoClientWindow.Created, ShowWindow and InternalOnKeyEvent (Vm::call):
    /// the result when handled here, otherwise the arguments back.
    #[allow(clippy::type_complexity)]
    pub(crate) fn video_menu_call(
        &mut self,
        f: usize,
        this: ObjId,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        let d = &self.functions[f];
        // The Texture filter line's caption: the short form when the full one does not fit.
        if d.name == "BeforePaint" && self.improved.video_menu.page.is_some() {
            if self.improved.video_menu.anisotropy == Some(this) {
                return Ok(self.video_menu_caption(f, this, args, Text::VideoTextureFilter, Text::VideoTextureFilterShort));
            }
            if self.improved.video_menu.vsync == Some(this) {
                return Ok(self.video_menu_caption(f, this, args, Text::VideoVsync, Text::VideoVsyncShort));
            }
        }
        if d.state.is_some() || !self.classes[d.owner].name.eq_ignore_ascii_case("XIIIMenuVideoClientWindow") {
            return Err(args);
        }
        match d.name.as_str() {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.video_menu_created(this);
                }
                Ok(r)
            }
            // A click on the Graphics line's arrows (XIIIComboControl.InternalOnClick) changes its
            // Index without a key: the other lines follow before the page is drawn.
            "Paint" if self.improved.video_menu.page == Some(this) => {
                self.video_menu_graphics_sync();
                Err(args)
            }
            "ShowWindow" if self.improved.video_menu.page == Some(this) => {
                let r = self.call_inner(f, this, args);
                self.video_menu_show();
                Ok(r)
            }
            "InternalOnKeyEvent" if self.improved.video_menu.page == Some(this) => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                if state != 1 && state != 2 {
                    return Ok(self.call_inner(f, this, args));
                }
                let focused = self.propr(this, "FocusedControl").as_object();
                let default_button = self.propr(this, "DefaultButton").as_object();
                if matches!(key, 0x0D | 0x01) && focused.is_some() && focused == default_button {
                    let r = self.call_inner(f, this, args);
                    self.video_menu_defaults();
                    return Ok(r);
                }
                if key == 0x0D {
                    self.video_menu_apply();
                    let r = self.call_inner(f, this, args);
                    self.improved.video_menu.keep_sizes = false;
                    return Ok(r);
                }
                if matches!(key, 0x25 | 0x27) {
                    let m = self.improved.video_menu.clone();
                    if let Some(c) = focused.filter(|&c| [m.mode, m.vsync, m.graphics, m.scale, m.msaa, m.anisotropy, m.shadows, m.ui_scale].contains(&Some(c))) {
                        let n = match self.prop(c, "Items") {
                            Value::Array(a) => a.len() as i32,
                            _ => 0,
                        };
                        // Greyed quality lines (Graphics: Original) do not change.
                        if m.enhanced || Some(c) == m.mode || Some(c) == m.vsync || Some(c) == m.graphics {
                            let i = self.propr(c, "Index").as_int() + if key == 0x25 { -1 } else { 1 };
                            // Not in the original: the V-sync line goes round (Off after On).
                            let i = if Some(c) == m.vsync && n > 0 { i.rem_euclid(n) } else { i.clamp(0, (n - 1).max(0)) };
                            self.combo_select(c, i);
                            self.video_menu_graphics_sync();
                        }
                        return Ok(Ok((Value::Bool(true), self.key_event_outs(f, &args))));
                    }
                }
                Ok(self.call_inner(f, this, args))
            }
            _ => Err(args),
        }
    }

    /// XIIIComboControl.BeforePaint of the Texture filter and V-sync lines: the full caption, or its
    /// short form when the full one is wider than the caption's box (XIIIComboControl.Paint draws it
    /// with the canvas's SmallFont, PoliceF16, centred in FirstBoxWidth - 32 * fSplitX, its arrow
    /// beyond), with a margin so that it does not touch the box's edge or the arrow.
    fn video_menu_caption(&mut self, f: usize, combo: ObjId, args: Vec<Option<Value>>, long: Text, short: Text) -> Result<(Value, Vec<Option<Value>>)> {
        let full = self.added_text(long).to_string();
        self.set_prop(combo, "Text", Value::Str(full.clone()));
        let r = self.call_inner(f, combo, args.clone());
        let Some(c) = args.first().cloned().flatten().and_then(|v| v.as_object()) else { return r };
        let split = self.propr(combo, "fSplitX").as_float();
        let room = self.propr(combo, "FirstBoxWidth").as_float() - (32.0 + 2.0 * CAPTION_MARGIN) * split;
        let font = self.prop(c, "Font");
        let small = self.prop(c, "SmallFont");
        self.set_prop(c, "Font", small);
        let mut a = [Value::Str(full.into()), Value::Float(0.0), Value::Float(0.0)];
        self.canvas_native("TextSize", c, &mut a, &[true; 3]);
        self.set_prop(c, "Font", font);
        if a[1].as_float() <= room {
            return r;
        }
        let short = self.added_text(short).to_string();
        self.set_prop(combo, "Text", Value::Str(short));
        self.call_inner(f, combo, args)
    }

    /// Index `i` of combo box `c` selected (XIIIComboControl.SetSelectedIndex: arrows too).
    fn combo_select(&mut self, c: ObjId, i: i32) {
        let _ = self.call_named(c, "SetSelectedIndex", vec![Value::Int(i)]);
        // The V-sync line goes round: both arrows stay (SetSelectedIndex hides one at each end).
        if Some(c) == self.improved.video_menu.vsync {
            self.set_prop(c, "bShowLeftArrow", Value::Bool(true));
            self.set_prop(c, "bShowRightArrow", Value::Bool(true));
        }
    }

    /// A combo box like the page's Resolution line (308, width 300, height 29), with `items`.
    fn video_combo(&mut self, page: ObjId, label: Text, items: Vec<String>) -> Option<ObjId> {
        let class = self.class("XIIIComboControl")?;
        let scale = self.page_scale(page);
        let args = vec![Value::Class(Some(class)), Value::Float(308.0), Value::Float(0.0), Value::Float(300.0), Value::Float(29.0 * scale)];
        let c = self.call_named(page, "CreateControl", args).ok()?.as_object()?;
        self.set_prop(c, "Items", Value::Array(items.into_iter().map(Value::Str).collect()));
        self.set_prop(c, "Text", Value::Str(self.added_text(label).to_string()));
        self.set_prop(c, "bArrows", Value::Bool(true));
        Some(c)
    }

    /// After Created (PC page, with its Resolution line): the new lines, then all lines laid out.
    fn video_menu_created(&mut self, page: ObjId) {
        self.improved.video_menu = VideoMenu::default();
        let Some(res) = self.propr(page, "ResCombo").as_object() else { return };
        // Resolution: every mode of the monitor instead of GetAvailableRes's 4:3 list (an empty
        // list, without modes, keeps the page's own).
        // The window's own size is in the list even when the monitor has no such mode.
        // With Graphics: Original, the page's own list (the original's GetAvailableRes).
        let (w, h) = self.viewport_size_x_y();
        let res_all = crate::video_settings::resolutions_with_current(self.menu_resolutions(), (w.max(0) as u32, h.max(0) as u32));
        let mut res_original: Vec<(u32, u32)> = self.combo_items(res).iter().filter_map(|s| parse_res(s)).collect();
        if res_original.is_empty() {
            res_original = vs::original_resolutions(&res_all);
        }
        let graphics_items = [Text::VideoGraphicsOriginal, Text::VideoGraphicsEnhanced].map(|t| self.added_text(t).to_string()).to_vec();
        let graphics = self.video_combo(page, Text::VideoGraphics, graphics_items);
        let off = self.added_text(Text::VideoOff).to_string();
        let mode = if MODE_LINE {
            let items = FullscreenMode::ALL.iter().map(|&m| self.added_text(mode_label(m)).to_string()).collect();
            self.video_combo(page, Text::VideoDisplayMode, items)
        } else {
            None
        };
        let vsync_items = VsyncMode::ALL.map(|m| self.added_text(vsync_label(m)).to_string()).to_vec();
        let vsync = self.video_combo(page, Text::VideoVsync, vsync_items);
        let caps = self.video_caps.clone();
        let (scale_choices, msaa_choices, anisotropy_choices) = (caps.render_scale_choices(), caps.msaa_choices(), caps.anisotropy_choices());
        let scale_items = scale_choices.iter().map(|v| format!("{v}%")).collect();
        let scale = self.video_combo(page, Text::VideoRenderScale, scale_items);
        let msaa_items = msaa_choices.iter().map(|&v| factor_label(v, &off)).collect();
        let msaa = self.video_combo(page, Text::VideoAntialiasing, msaa_items);
        let aniso_items = anisotropy_choices.iter().map(|&v| factor_label(v, &off)).collect();
        let anisotropy = self.video_combo(page, Text::VideoTextureFilter, aniso_items);
        let shadow_choices = vs::shadow_quality_choices();
        let shadow_items = shadow_choices.iter().map(|&v| self.added_text(shadow_label(v)).to_string()).collect();
        let shadows = self.video_combo(page, Text::VideoShadows, shadow_items);
        // Interface size is no longer offered: the menus and the HUD keep the original's size on
        // every screen (Vm::ui_scale). Without its line the page writes the default back.
        let ui_scale_choices = vs::ui_scale_choices();
        let ui_scale = None;
        let text_color = scale.map(|c| self.prop(c, "TextColor"));
        self.improved.video_menu = VideoMenu {
            page: Some(page),
            mode,
            scale,
            msaa,
            anisotropy,
            shadows,
            graphics,
            vsync,
            vsync_choices: [None; 2],
            res: Some(res),
            enhanced: true,
            kept: None,
            res_all,
            res_original,
            text_color,
            ui_scale,
            scale_choices,
            msaa_choices,
            anisotropy_choices,
            shadow_choices,
            ui_scale_choices,
            before: Vec::new(),
            after: Vec::new(),
            res_kept: None,
            keep_sizes: false,
        };
        let old: Vec<Option<ObjId>> = self.controls(page);
        let at = old.iter().position(|&c| c == Some(res)).unwrap_or(old.len());
        self.improved.video_menu.before = old[..at].to_vec();
        self.improved.video_menu.after = old.get(at + 1..).unwrap_or(&[]).to_vec();
        self.video_menu_layout(true);
        self.video_menu_show();
    }

    /// The page's lines for Graphics `enhanced`, laid out. Enhanced: the lines before
    /// Resolution, Display mode, V-sync, Graphics, Resolution, the quality lines, then the rest
    /// (Default). Original: the original page's lines (Brightness, Gamma, Contrast, Resolution,
    /// Default) and Graphics only, the way back to Enhanced. The other new lines stay in the
    /// page's Controls (the page initialises its Controls when opened: their style), after the
    /// shown ones so that the focus highlight (Paint: FindComponentIndex) keeps its place, hidden
    /// (bVisible) and never focused; their values are kept and come back with Enhanced.
    fn video_menu_layout(&mut self, enhanced: bool) {
        let m = self.improved.video_menu.clone();
        let (Some(page), Some(res)) = (m.page, m.res) else { return };
        let all_lines = enhanced;
        let top = [m.mode, m.vsync];
        let quality = [m.scale, m.msaa, m.anisotropy, m.shadows, m.ui_scale];
        let mut lines: Vec<Option<ObjId>> = m.before.clone();
        if all_lines {
            lines.extend(top.into_iter().flatten().map(Some));
        }
        lines.extend(m.graphics.map(Some));
        lines.push(Some(res));
        if all_lines {
            lines.extend(quality.into_iter().flatten().map(Some));
        }
        lines.extend(m.after.iter().copied());
        let (line_space, y_org) = line_layout(lines.len() as i32);
        self.set_prop(page, "LineSpace", Value::Int(line_space));
        self.set_prop(page, "YOrg", Value::Int(y_org));
        let s = self.page_scale(page);
        for (i, c) in lines.iter().enumerate() {
            if let Some(c) = *c {
                self.set_prop(c, "WinTop", Value::Float((y_org + line_space * i as i32) as f32 * s / 480.0));
            }
        }
        for c in top.into_iter().chain(quality).flatten() {
            self.set_prop(c, "bVisible", Value::Bool(all_lines));
            if !all_lines {
                lines.push(Some(c));
                self.set_prop(c, "bNeverFocus", Value::Bool(true));
            }
        }
        for c in top.into_iter().flatten().filter(|_| all_lines) {
            self.set_prop(c, "bNeverFocus", Value::Bool(false));
        }
        self.set_prop(page, "Controls", Value::Array(lines.into_iter().map(Value::Object).collect()));
    }

    /// The new lines show the current values (ShowWindow, and after Created).
    fn video_menu_show(&mut self) {
        let s = self.video_settings();
        let m = self.improved.video_menu.clone();
        if let Some(c) = m.mode {
            let cur = self.menu_fullscreen_mode();
            let i = FullscreenMode::ALL.iter().position(|&x| x == cur).unwrap_or(0);
            self.combo_select(c, i as i32);
        }
        self.improved.video_menu.vsync_choices = [None; 2];
        let q = self.shadow_quality();
        self.improved.video_menu.enhanced = true;
        self.improved.video_menu.kept = None;
        self.video_menu_quality(&s);
        self.video_menu_shadows(q);
        let enhanced = self.graphics_setting();
        if let Some(c) = m.graphics {
            self.combo_select(c, i32::from(enhanced));
        }
        self.video_menu_set_graphics(enhanced, Some((s, q)));
    }

    /// Items of a combo box.
    fn combo_items(&self, c: ObjId) -> Vec<String> {
        match self.prop(c, "Items") {
            Value::Array(a) => a.iter().filter_map(|v| if let Value::Str(s) = v { Some(s.to_string()) } else { None }).collect(),
            _ => Vec::new(),
        }
    }

    /// The Graphics line's Index changed (key or click): the page follows it.
    fn video_menu_graphics_sync(&mut self) {
        let Some(c) = self.improved.video_menu.graphics else { return };
        let on = self.propr(c, "Index").as_int() == 1;
        if on != self.improved.video_menu.enhanced {
            self.video_menu_set_graphics(on, None);
        }
        self.video_menu_res_borderless();
    }

    /// The Display mode line's value on the page (None without the line).
    fn video_menu_mode_shown(&self) -> Option<FullscreenMode> {
        let c = self.improved.video_menu.mode?;
        FullscreenMode::ALL.get(self.propr(c, "Index").as_int().max(0) as usize).copied()
    }

    /// Graphics: Enhanced with the Display mode line on Borderless: the Resolution line is greyed
    /// (a borderless window covers the desktop, whatever the size) and shows the desktop's size;
    /// another mode gives the line and its choice back. Graphics: Original never greys it here
    /// (its fullscreen is exclusive).
    fn video_menu_res_borderless(&mut self) {
        let m = self.improved.video_menu.clone();
        let Some(res) = m.res else { return };
        let borderless = m.enhanced && self.video_menu_mode_shown() == Some(FullscreenMode::Borderless);
        match (borderless, m.res_kept) {
            (true, None) => {
                let desktop = self.screen_native.unwrap_or_else(|| {
                    let (w, h) = self.viewport_size_x_y();
                    (w.max(0) as u32, h.max(0) as u32)
                });
                let shown: Vec<(u32, u32)> = self.combo_items(res).iter().filter_map(|s| parse_res(s)).collect();
                let i = vs::nearest_resolution(&shown, desktop).unwrap_or(0) as i32;
                self.improved.video_menu.res_kept = Some((self.propr(res, "Index").as_int(), self.prop(res, "TextColor")));
                self.combo_select(res, i);
                if let Some(page) = m.page {
                    self.set_prop(page, "OnRes", Value::Int(i));
                }
                self.video_menu_grey(res, None);
            }
            (false, Some((i, color))) => {
                self.improved.video_menu.res_kept = None;
                self.video_menu_grey(res, Some(color));
                self.combo_select(res, i);
                if let Some(page) = m.page {
                    self.set_prop(page, "OnRes", Value::Int(i));
                }
            }
            _ => {}
        }
    }

    /// A line greyed (None: no focus, no input, no arrows, the greyed colour) or given back with
    /// its text colour (its arrows come back with its next combo_select).
    fn video_menu_grey(&mut self, c: ObjId, color: Option<Value>) {
        let greyed = color.is_none();
        self.set_prop(c, "bNeverFocus", Value::Bool(greyed));
        self.set_prop(c, "bAcceptsInput", Value::Bool(!greyed));
        if greyed {
            self.set_prop(c, "bShowLeftArrow", Value::Bool(false));
            self.set_prop(c, "bShowRightArrow", Value::Bool(false));
        }
        self.set_prop(c, "TextColor", color.unwrap_or_else(|| crate::canvas::color_value(GREYED_TEXT)));
    }

    /// The page shown with Graphics `enhanced`: Resolution's list, the quality lines' values
    /// (Enhanced: those kept while Original was chosen, or `values`; Original: the original's,
    /// greyed, while `values`, or else the values on the lines, are kept).
    fn video_menu_set_graphics(&mut self, enhanced: bool, values: Option<(VideoSettings, u32)>) {
        // The Resolution line greyed for Borderless is given back first (its selection is
        // chosen again below), then greyed again if it still applies.
        if let (Some(res), Some((_, color))) = (self.improved.video_menu.res, self.improved.video_menu.res_kept.take()) {
            self.video_menu_grey(res, Some(color));
        }
        let m = self.improved.video_menu.clone();
        if let Some(c) = m.vsync {
            if values.is_none() {
                let shown = VsyncMode::from_index(self.propr(c, "Index").as_int());
                self.improved.video_menu.vsync_choices[m.enhanced as usize] =
                    (shown != self.profile_vsync(m.enhanced)).then_some(shown);
            }
            let mode = m.vsync_choices[enhanced as usize].unwrap_or_else(|| self.profile_vsync(enhanced));
            self.combo_select(c, mode.index() as i32);
        }
        if !enhanced && m.enhanced {
            let shown = self.video_menu_chosen().map(|(_, s)| (s, self.video_menu_shadow_chosen()));
            self.improved.video_menu.kept = values.or(shown);
        }
        self.improved.video_menu.enhanced = enhanced;
        self.video_menu_layout(enhanced);
        // Resolution: the list of the mode, the selection kept or the nearest size.
        if let Some(res) = m.res {
            let list = if enhanced { &m.res_all } else { &m.res_original };
            if !list.is_empty() {
                // Page shown or back to Enhanced: the window's size; switched to Original: the
                // original's size nearest to the one on the line.
                let shown = self.combo_items(res);
                let on_line = shown.get(self.propr(res, "Index").as_int().max(0) as usize).and_then(|s| parse_res(s));
                let cur = on_line.filter(|_| values.is_none() && !enhanced).unwrap_or_else(|| {
                    let (w, h) = self.viewport_size_x_y();
                    (w.max(0) as u32, h.max(0) as u32)
                });
                let i = vs::nearest_resolution(list, cur).unwrap_or(0) as i32;
                let items = list.iter().map(|(w, h)| Value::Str(format!("{w}x{h}"))).collect();
                self.set_prop(res, "Items", Value::Array(items));
                self.combo_select(res, i);
                if let Some(page) = m.page {
                    self.set_prop(page, "OnRes", Value::Int(i));
                }
            }
        }
        let lines = [m.scale, m.msaa, m.anisotropy, m.shadows, m.ui_scale];
        if enhanced {
            if let Some((s, q)) = self.improved.video_menu.kept.take().or(values) {
                self.video_menu_quality(&s);
                self.video_menu_shadows(q);
            }
            for c in lines.into_iter().flatten() {
                self.set_prop(c, "bNeverFocus", Value::Bool(false));
                self.set_prop(c, "bAcceptsInput", Value::Bool(true));
                if let Some(t) = m.text_color.clone() {
                    self.set_prop(c, "TextColor", t);
                }
                // The arrows back (SetSelectedIndex sets them from the index).
                let i = self.propr(c, "Index").as_int();
                self.combo_select(c, i);
            }
        } else {
            self.video_menu_quality(&original_look());
            self.video_menu_shadows(ORIGINAL_SHADOWS);
            for c in lines.into_iter().flatten() {
                self.set_prop(c, "bNeverFocus", Value::Bool(true));
                self.set_prop(c, "bAcceptsInput", Value::Bool(false));
                self.set_prop(c, "bShowLeftArrow", Value::Bool(false));
                self.set_prop(c, "bShowRightArrow", Value::Bool(false));
                self.set_prop(c, "TextColor", crate::canvas::color_value(GREYED_TEXT));
            }
        }
        self.video_menu_res_borderless();
    }

    /// Shadows line's value on the page.
    fn video_menu_shadow_chosen(&self) -> u32 {
        let m = &self.improved.video_menu;
        m.shadows.and_then(|c| m.shadow_choices.get(self.propr(c, "Index").as_int().max(0) as usize).copied()).unwrap_or(0)
    }

    fn video_menu_shadows(&mut self, quality: u32) {
        let m = self.improved.video_menu.clone();
        if let Some(c) = m.shadows {
            self.combo_select(c, m.shadow_choices.iter().position(|&v| v == quality).unwrap_or(0) as i32);
        }
    }

    fn video_menu_quality(&mut self, s: &VideoSettings) {
        let m = self.improved.video_menu.clone();
        for (c, choices, v) in [
            (m.scale, m.scale_choices, s.render_scale),
            (m.msaa, m.msaa_choices, s.msaa),
            (m.anisotropy, m.anisotropy_choices, s.anisotropy),
            (m.ui_scale, m.ui_scale_choices, s.ui_scale),
        ] {
            if let Some(c) = c {
                self.combo_select(c, vs::closest_index(&choices, v) as i32);
            }
        }
    }

    /// Default: the quality lines back to their defaults (the display mode and Graphics stay;
    /// with Original, the kept values go back to theirs, the greyed lines still show the original's).
    fn video_menu_defaults(&mut self) {
        // With Original the V-sync line is hidden: Default leaves it, as the original's page.
        if let Some(c) = self.improved.video_menu.vsync.filter(|_| self.improved.video_menu.enhanced) {
            let mode = vs::vsync_profile_default(self.improved.video_menu.enhanced);
            self.combo_select(c, mode.index() as i32);
        }
        if !self.improved.video_menu.enhanced {
            self.improved.video_menu.kept = Some((VideoSettings::default(), 0));
            return;
        }
        self.video_menu_quality(&VideoSettings::default());
        self.video_menu_shadows(0);
    }

    /// The values on the page's lines (with Graphics: Original they show the original's; Enter
    /// writes the kept ones instead).
    pub fn video_menu_chosen(&mut self) -> Option<(Option<FullscreenMode>, VideoSettings)> {
        let m = self.improved.video_menu.clone();
        m.page?;
        let pick = |vm: &Self, c: Option<ObjId>, choices: &[u32], fallback: u32| {
            c.and_then(|c| choices.get(vm.propr(c, "Index").as_int().max(0) as usize).copied()).unwrap_or(fallback)
        };
        let d = VideoSettings::default();
        let mode = m.mode.and_then(|c| FullscreenMode::ALL.get(self.propr(c, "Index").as_int().max(0) as usize).copied());
        let s = VideoSettings {
            fullscreen_mode: None,
            render_scale: pick(self, m.scale, &m.scale_choices, d.render_scale),
            msaa: pick(self, m.msaa, &m.msaa_choices, d.msaa),
            anisotropy: pick(self, m.anisotropy, &m.anisotropy_choices, d.anisotropy),
            ui_scale: pick(self, m.ui_scale, &m.ui_scale_choices, d.ui_scale),
        };
        Some((mode, s))
    }

    /// Enter: the values written; a new display mode asked of the display, the window's state
    /// changed first so that the page's SETRES keeps the size in the keys of the new mode.
    fn video_menu_apply(&mut self) {
        // Lockstep co-op: the Video page of the other machine's player (made here too, as every
        // player's menus are) changes that machine's display settings, not this one's.
        if self.improved.video_menu.page.and_then(|p| self.page_owner(p)).is_some_and(|pc| self.lockstep_foreign(pc)) {
            return;
        }
        let Some((mode, mut s)) = self.video_menu_chosen() else { return };
        // Graphics: Original keeps the Enhanced values as they were (the lines show the original's).
        let enhanced = self.improved.video_menu.enhanced;
        // Borderless (its Resolution line greyed): the page's SETRES keeps the saved sizes.
        self.improved.video_menu.keep_sizes = self.improved.video_menu.res_kept.is_some();
        let mut shadow = self.video_menu_shadow_chosen();
        if !enhanced {
            let (k, q) = match self.improved.video_menu.kept {
                Some(k) => k,
                None => (self.video_settings(), self.shadow_quality()),
            };
            s = k;
            shadow = q;
        }
        // The window's mode now, and the line's value when the page was shown (they differ
        // for a Borderless key run exclusive by Graphics: Original).
        let was = self.current_fullscreen_mode();
        let shown = self.menu_fullscreen_mode();
        if let Some(mode) = mode.filter(|&m| m != shown) {
            s.fullscreen_mode = Some(mode);
            self.fullscreen = mode.is_fullscreen();
        } else {
            // Unchanged: the key keeps its value (absent: the original behaviour).
            s.fullscreen_mode = self.video_settings().fullscreen_mode;
        }
        self.set_video_settings(&s);
        if self.improved.video_menu.shadows.is_some() {
            self.set_shadow_quality(shadow);
        }
        if self.improved.video_menu.graphics.is_some() {
            self.set_graphics_setting(enhanced);
        }
        // A new display mode asked of the display: the line's, or Graphics' own (Original has
        // no borderless mode: from Borderless it goes exclusive, as from exclusive fullscreen).
        let now = self.current_fullscreen_mode();
        if now != was {
            self.pending_display_mode = Some(now);
        }
        if let Some(c) = self.improved.video_menu.vsync {
            let mut choices = self.improved.video_menu.vsync_choices;
            choices[enhanced as usize] = Some(VsyncMode::from_index(self.propr(c, "Index").as_int()));
            for (profile, choice) in [false, true].into_iter().zip(choices) {
                if let Some(mode) = choice {
                    if mode != self.profile_vsync(profile) {
                        self.write_profile_vsync(profile, mode);
                    }
                }
            }
            self.refresh_vsync();
        }
    }
}

/// Free space kept on each side of the Texture filter line's caption (page units across).
const CAPTION_MARGIN: f32 = 4.0;

/// Text colour of a greyed line (as the Co-op page's greyed settings).
const GREYED_TEXT: [u8; 4] = [120, 120, 120, 255];

/// Shadows line with Graphics: Original: the original's 256 x 256 targets (Low).
const ORIGINAL_SHADOWS: u32 = 256;

/// What the quality lines show with Graphics: Original: the original game's values.
fn original_look() -> VideoSettings {
    VideoSettings { fullscreen_mode: None, render_scale: 100, msaa: 1, anisotropy: 1, ui_scale: vs::UI_SCALE_DEFAULT }
}

/// The V-sync line's label of a mode.
fn vsync_label(mode: VsyncMode) -> Text {
    match mode {
        VsyncMode::Off => Text::VideoVsyncOff,
        VsyncMode::Classic => Text::VideoVsyncOn,
    }
}

/// "1024x768" read.
fn parse_res(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_formula() {
        // The original page: 5 lines (Brightness, Gamma, Contrast, Resolution, Default).
        assert_eq!(line_layout(5), (66, 93));
        // With the five new lines, every line still fits in the 370-unit panel (55 to 425).
        let (ls, y) = line_layout(10);
        assert_eq!((ls, y), (36, 63));
        assert!(y + 9 * ls + 29 <= 425);
        // With the seven new lines (Graphics included): 12 lines, still in the panel.
        let (ls, y) = line_layout(11);
        assert!(y + 10 * ls + 29 <= 425 && ls >= 29, "{ls} {y}");
        // With V-sync too: 12 lines on PC, 11 on Android (no Display mode line).
        let (ls, y) = line_layout(12);
        assert!(y + 11 * ls + 29 <= 425 && ls >= 29, "{ls} {y}");
        let (ls, y) = line_layout(9);
        assert!(y + 8 * ls + 29 <= 425 && ls >= 29);
    }

    /// The page made by the game's own Created: with Graphics: Original only the original page's lines and Graphics are
    /// shown (Display mode, V-sync and the quality lines hidden, after them in its Controls, never focused); with
    /// Enhanced every new line. Switching back and forth keeps the values of the hidden lines. Needs OXIII_GAME.
    #[test]
    fn original_shows_the_original_lines_and_graphics() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let root_class = vm.class("XIIIRootWindow").expect("XIIIRootWindow");
        let root = vm.exec_new(None, None, 0, Some(root_class)).0.expect("new XIIIRootWindow");
        vm.set_prop(root, "fTextureScaleFactorForConsole", Value::Float(1.0));
        let class = vm.class("XIIIMenuVideoClientWindow").expect("XIIIMenuVideoClientWindow");
        let page = vm.exec_new(None, None, 0, Some(class)).0.expect("new XIIIMenuVideoClientWindow");
        vm.set_prop(page, "myRoot", Value::Object(Some(root)));
        let _ = vm.call_named(page, "Created", Vec::new());
        let m = vm.improved.video_menu.clone();
        assert_eq!(m.page, Some(page), "the page's new lines were made");
        let graphics = m.graphics.expect("Graphics line");
        let added = [m.mode, m.vsync, m.scale, m.msaa, m.anisotropy, m.shadows].into_iter().flatten().collect::<Vec<_>>();
        assert!(added.len() >= 5 && m.vsync.is_some(), "{added:?}");
        let lines = |vm: &Vm| vm.controls(page).into_iter().flatten().filter(|&c| vm.propr(c, "bVisible").as_bool()).collect::<Vec<ObjId>>();
        let placed = |vm: &Vm| {
            let s = vm.page_scale(page);
            lines(vm).iter().map(|&c| (vm.propr(c, "WinTop").as_float() * 480.0 / s).round() as i32).collect::<Vec<i32>>()
        };
        let mut seen = Vec::new();
        for enhanced in [false, true, false, true] {
            vm.combo_select(graphics, i32::from(enhanced));
            vm.video_menu_graphics_sync();
            let shown = lines(&vm);
            let n = shown.len() as i32;
            let (ls, y) = line_layout(n);
            assert_eq!(placed(&vm), (0..n).map(|i| y + ls * i).collect::<Vec<_>>(), "laid out for {n} lines");
            assert!(shown.contains(&graphics) && shown.contains(&m.res.unwrap()), "Graphics and Resolution always");
            let all = vm.controls(page).into_iter().flatten().collect::<Vec<ObjId>>();
            assert_eq!(&all[..shown.len()], &shown[..], "the shown lines first (the focus highlight's index)");
            for &c in &added {
                assert_eq!(shown.contains(&c), enhanced, "added line {c:?} (enhanced: {enhanced})");
                assert!(all.contains(&c), "kept in Controls");
                assert_eq!(vm.propr(c, "bNeverFocus").as_bool(), !enhanced);
            }
            // The original page: Brightness, Gamma, Contrast, Resolution, Default, and Graphics.
            if !enhanced {
                assert_eq!(n, 6, "original lines and Graphics");
            }
            seen.push(n);
        }
        assert_eq!(seen[0], seen[2]);
        assert_eq!(seen[1], seen[3]);
    }

    /// Graphics: Original chosen in borderless fullscreen gives exclusive fullscreen, as from
    /// exclusive fullscreen (the original had no borderless mode); the Borderless key stays and
    /// Enhanced again gives borderless back. From exclusive fullscreen nothing new is asked.
    /// Without the fix the switch asked nothing: the game stayed borderless at the desktop's
    /// size. Needs OXIII_GAME.
    #[test]
    fn original_from_borderless_is_exclusive_fullscreen() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let player = std::env::temp_dir().join(format!("oxiii-video-borderless-{}", std::process::id()));
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        vm.config = Some(crate::config::ConfigCache::with_user_dir(game.join("System"), Some(player.clone())));
        let root_class = vm.class("XIIIRootWindow").expect("XIIIRootWindow");
        let root = vm.exec_new(None, None, 0, Some(root_class)).0.expect("new XIIIRootWindow");
        vm.set_prop(root, "fTextureScaleFactorForConsole", Value::Float(1.0));
        let class = vm.class("XIIIMenuVideoClientWindow").expect("XIIIMenuVideoClientWindow");
        let page = vm.exec_new(None, None, 0, Some(class)).0.expect("new XIIIMenuVideoClientWindow");
        vm.set_prop(page, "myRoot", Value::Object(Some(root)));
        let _ = vm.call_named(page, "Created", Vec::new());
        let m = vm.improved.video_menu.clone();
        let (graphics, mode_line) = (m.graphics.expect("Graphics line"), m.mode.expect("Display mode line"));
        let line_mode = |vm: &Vm| FullscreenMode::ALL[vm.propr(mode_line, "Index").as_int().max(0) as usize];
        // Shows the page with Graphics `from`, switches it to `to` and accepts it: the mode asked
        // of the display.
        let switch = |vm: &mut Vm, key: FullscreenMode, from: bool, to: bool| {
            vm.set_video_settings(&VideoSettings { fullscreen_mode: Some(key), ..VideoSettings::default() });
            vm.set_graphics_setting(from);
            vm.fullscreen = true;
            vm.pending_display_mode = None;
            vm.video_menu_show();
            assert_eq!(line_mode(vm), key, "the line shows the key's mode (Graphics enhanced: {from})");
            vm.combo_select(graphics, i32::from(to));
            vm.video_menu_graphics_sync();
            vm.video_menu_apply();
            assert_eq!(vm.video_settings().fullscreen_mode, Some(key), "the key stays");
            assert!(vm.fullscreen);
            vm.pending_display_mode.take()
        };
        let original_from_borderless = switch(&mut vm, FullscreenMode::Borderless, true, false);
        let enhanced_again = switch(&mut vm, FullscreenMode::Borderless, false, true);
        let original_from_exclusive = switch(&mut vm, FullscreenMode::Exclusive, true, false);
        let original_kept = switch(&mut vm, FullscreenMode::Borderless, false, false);
        crate::video_settings::set_graphics_enhanced(false);
        let _ = std::fs::remove_dir_all(&player);
        assert_eq!(original_from_borderless, Some(FullscreenMode::Exclusive), "Original from borderless: exclusive fullscreen");
        assert_eq!(enhanced_again, Some(FullscreenMode::Borderless), "Enhanced again: borderless");
        assert_eq!(original_from_exclusive, None, "Original from exclusive: already exclusive");
        assert_eq!(original_kept, None, "Original kept: unchanged");
    }

    /// Graphics: Enhanced with the Display mode line on Borderless: the Resolution line is greyed
    /// on the desktop's size, live as the line changes; another mode gives its choice back;
    /// Original never greys it (its fullscreen is exclusive, at the original's 4:3 size); Enter in
    /// Borderless keeps the saved sizes (the page's SETRES resizes nothing). The Display mode line
    /// reads Fullscreen, Borderless, Window. Needs OXIII_GAME.
    #[test]
    fn borderless_greys_the_resolution_line() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let player = std::env::temp_dir().join(format!("oxiii-video-res-borderless-{}", std::process::id()));
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        vm.config = Some(crate::config::ConfigCache::with_user_dir(game.join("System"), Some(player.clone())));
        // A 1920 x 1200 monitor, the game in a 1280 x 720 window.
        let sizes = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 1024), (1440, 1080), (1600, 1200), (1680, 1050), (1920, 1080), (1920, 1200)];
        vm.display_modes = sizes.iter().map(|&(w, h)| (w, h, 32)).collect();
        vm.screen_native = Some((1920, 1200));
        vm.engine.client.window_open = true;
        vm.viewport = (1280, 720);
        vm.fullscreen = false;
        vm.set_video_settings(&VideoSettings { fullscreen_mode: Some(FullscreenMode::Windowed), ..VideoSettings::default() });
        vm.set_graphics_setting(true);
        let c = vm.config.as_mut().unwrap();
        c.set_string("WinDrv.WindowsClient", "FullscreenViewportX", "1600", "System");
        c.set_string("WinDrv.WindowsClient", "FullscreenViewportY", "1200", "System");
        let root_class = vm.class("XIIIRootWindow").expect("XIIIRootWindow");
        let root = vm.exec_new(None, None, 0, Some(root_class)).0.expect("new XIIIRootWindow");
        vm.set_prop(root, "fTextureScaleFactorForConsole", Value::Float(1.0));
        let class = vm.class("XIIIMenuVideoClientWindow").expect("XIIIMenuVideoClientWindow");
        let page = vm.exec_new(None, None, 0, Some(class)).0.expect("new XIIIMenuVideoClientWindow");
        vm.set_prop(page, "myRoot", Value::Object(Some(root)));
        let _ = vm.call_named(page, "Created", Vec::new());
        vm.video_menu_show();
        let m = vm.improved.video_menu.clone();
        let (graphics, mode_line, res) = (m.graphics.expect("Graphics line"), m.mode.expect("Display mode line"), m.res.expect("Resolution line"));
        let labels: Vec<String> = [FullscreenMode::Exclusive, FullscreenMode::Borderless, FullscreenMode::Windowed]
            .iter()
            .map(|&m| vm.added_text(mode_label(m)).to_string())
            .collect();
        assert_eq!(vm.combo_items(mode_line), labels, "Fullscreen, Borderless, Window");
        let set_mode = |vm: &mut Vm, mode: FullscreenMode| {
            vm.combo_select(mode_line, FullscreenMode::ALL.iter().position(|&x| x == mode).unwrap() as i32);
            vm.video_menu_graphics_sync();
        };
        let set_graphics = |vm: &mut Vm, enhanced: bool| {
            vm.combo_select(graphics, i32::from(enhanced));
            vm.video_menu_graphics_sync();
        };
        let shown = |vm: &Vm| vm.combo_items(res).get(vm.propr(res, "Index").as_int().max(0) as usize).cloned().unwrap_or_default();
        let greyed = |vm: &Vm| (vm.propr(res, "bNeverFocus").as_bool(), vm.propr(res, "bAcceptsInput").as_bool(), vm.propr(res, "bShowLeftArrow").as_bool() || vm.propr(res, "bShowRightArrow").as_bool());
        let mut seen = Vec::new();
        seen.push(("window", shown(&vm), greyed(&vm)));
        set_mode(&mut vm, FullscreenMode::Borderless);
        seen.push(("borderless", shown(&vm), greyed(&vm)));
        let grey_color = vm.prop(res, "TextColor");
        set_mode(&mut vm, FullscreenMode::Exclusive);
        seen.push(("fullscreen", shown(&vm), greyed(&vm)));
        set_mode(&mut vm, FullscreenMode::Borderless);
        set_graphics(&mut vm, false);
        seen.push(("original", shown(&vm), greyed(&vm)));
        set_graphics(&mut vm, true);
        seen.push(("enhanced again", shown(&vm), greyed(&vm)));
        // Enter in Borderless, then the page's own SETRES with the line's value.
        vm.video_menu_apply();
        let _ = vm.windows_viewport_exec(&format!("SETRES {}", shown(&vm)));
        let resized = vm.pending_resize.take();
        let mode_asked = vm.pending_display_mode.take();
        let c = vm.config.as_mut().unwrap();
        let keys = ["FullscreenViewportX", "FullscreenViewportY", "WindowedViewportX"].map(|k| c.get_string("WinDrv.WindowsClient", k, "System").unwrap_or_default());
        vm.improved.video_menu.keep_sizes = false;
        crate::video_settings::set_graphics_enhanced(false);
        let _ = std::fs::remove_dir_all(&player);
        let free = (false, true, true);
        assert_eq!(seen[0], ("window", "1280x720".to_string(), free));
        assert_eq!(seen[1], ("borderless", "1920x1200".to_string(), (true, false, false)), "Borderless: greyed on the desktop's size");
        assert_eq!(grey_color, crate::canvas::color_value(GREYED_TEXT));
        assert_eq!(seen[2], ("fullscreen", "1280x720".to_string(), free), "another mode: the choice back");
        assert_eq!(seen[3].0, "original");
        assert_eq!((seen[3].1.as_str(), seen[3].2 .0, seen[3].2 .1), ("1600x1200", false, true), "Original: the original's size, never greyed");
        assert_eq!(seen[4].2, (true, false, false), "Enhanced again on Borderless: greyed");
        assert_eq!(mode_asked, Some(FullscreenMode::Borderless));
        assert_eq!(resized, None, "Enter in Borderless: no resize");
        assert_eq!(keys[..2], ["1600".to_string(), "1200".to_string()], "the saved fullscreen size stays");
    }

    #[test]
    fn vsync_order() {
        // Off, then On; each index reads back its mode.
        assert_eq!(VsyncMode::ALL, [VsyncMode::Off, VsyncMode::Classic]);
        for m in VsyncMode::ALL {
            assert_eq!(VsyncMode::from_index(m.index() as i32), m);
        }
        assert_eq!(crate::added_text::text(crate::added_text::LANGUAGES[1], vsync_label(VsyncMode::Classic)), "Activée");
    }

    #[test]
    fn labels() {
        assert_eq!(factor_label(1, "Off"), "Off");
        assert_eq!(factor_label(4, "Off"), "4x");
        assert_eq!(factor_label(16, "Aucun"), "16x");
        for m in FullscreenMode::ALL {
            for l in crate::added_text::LANGUAGES {
                assert!(!crate::added_text::text(l, mode_label(m)).is_empty());
            }
        }
        let shadow_texts = vs::shadow_quality_choices().into_iter().map(shadow_label);
        for t in [Text::VideoDisplayMode, Text::VideoVsync, Text::VideoVsyncShort, Text::VideoVsyncOn, Text::VideoVsyncOff, Text::VideoGraphics, Text::VideoGraphicsOriginal, Text::VideoGraphicsEnhanced, Text::VideoRenderScale, Text::VideoAntialiasing, Text::VideoTextureFilter, Text::VideoOff, Text::VideoShadows, Text::VideoUiScale].into_iter().chain(shadow_texts) {
            for l in crate::added_text::LANGUAGES {
                assert!(!crate::added_text::text(l, t).is_empty(), "{t:?} {l}");
            }
        }
    }
}
