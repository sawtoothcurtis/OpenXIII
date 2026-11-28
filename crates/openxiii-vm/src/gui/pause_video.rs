//! Improved build only, with Graphics Original or Enhanced: a "Video" line in the in-game menu
//! (XIIIMenuInGame), which opens the Video options page of the main menu
//! (XIIIMenuVideoClientWindow, with the lines of video_menu.rs) over the game. Its settings are
//! applied as soon as the page is left with Enter (the viewer reads them at every frame:
//! apply_render_quality, SETRES).
//!
//! The original in-game menu has six lines (Save, Return, Controls, Skills, Restart, Quit; no
//! video settings), as in the faithful build (where this module is not compiled). The improved
//! build adds the line with Graphics: Original too: it is the way back to Enhanced without
//! leaving the game. It does not depend on any setting, so every machine of a lockstep session
//! builds the same menu.
//!
//! The line is an XIIIButton like the menu's own, put after Controls; the seven lines keep the
//! menu's 30-unit spacing and are moved up by half a line so that they stay in the menu's box
//! (DrawMsgboxBackground from y 130 to 360; the six lines start at 155). Paint only clears the
//! frame of its first six controls before framing the focused one: the others are cleared here
//! first. Split screen: each player has their own in-game menu, so the page opens in the view
//! of the player who chose the line; the settings are the game's (shared by both views). A
//! gamepad player's menu is set to the console platform (coop.rs, coop_menu_platform), where the
//! page would build the Xbox's lines (no Resolution, none of video_menu.rs's): the page opened
//! from the line is built and used on the PC platform, its footer keeping the gamepad's buttons.
//!
//! Multiplayer (network or split screen) has its own in-game menu, XIIIMenuInGameMulti, whose
//! lines depend on the game (Return, then Change team, Change class and Kick a player when they
//! apply, Options, Quit). The same Video line goes after Options. The menu places its n lines
//! from y 250 - 18 n, 35 apart (Created: BoxPosY, LineSpace); with the line added they are placed
//! by that rule for n + 1 lines, or 30 apart and centred in the box when that would leave it (seven
//! lines). Multiplayer is not paused while the page is open, as the menu itself; the settings
//! only change this machine's display.

use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// Where the line goes in the menu's Controls (after Save, Return and Controls).
pub const LINE_INDEX: usize = 3;
/// First line's y with the original six lines (XIIIMenuInGame.Created: OffsetInit) and the
/// spacing of the lines, page units.
pub const OFFSET_INIT: f32 = 155.0;
pub const LINE_SPACE: f32 = 30.0;
/// First line's y with seven lines: half a line higher, so that the last one ends at 350 (the
/// menu's box ends at 360).
pub const OFFSET_SEVEN: f32 = OFFSET_INIT - LINE_SPACE / 2.0;
/// The page the line opens (as the main menu's Options page: XIIIMenuOptions.InternalOnClick).
pub const VIDEO_PAGE: &str = "XIDInterf.XIIIMenuVideoClientWindow";
pub const VIDEO_PAGE_CLASS: &str = "XIIIMenuVideoClientWindow";
/// The multiplayer in-game menu.
pub const MULTI_MENU_CLASS: &str = "XIIIMenuInGameMulti";
/// XIIIMenuInGameMulti.Created: the first line's y for n lines is 250 - 18 n, the lines are 35
/// apart and 30 high, in a box from y 130 to 360 (BackgroundPosY, BackgroundHeight).
pub const MULTI_SPACE: f32 = 35.0;
pub const MULTI_LINE_HEIGHT: f32 = 30.0;
/// Seven lines would leave the box at 35 apart: 30 apart, centred in it.
pub const MULTI_SPACE_TIGHT: f32 = 30.0;
const MULTI_BOX_TOP: f32 = 130.0;
const MULTI_BOX_BOTTOM: f32 = 360.0;

/// y of line `i` (page units) in a menu of `n` lines.
pub fn line_y(i: usize, n: usize) -> f32 {
    let first = if n > 6 { OFFSET_SEVEN } else { OFFSET_INIT };
    first + LINE_SPACE * i as f32
}

/// y of line `i` (page units) in a multiplayer menu of `n` lines.
pub fn multi_line_y(i: usize, n: usize) -> f32 {
    let first = 250.0 - 18.0 * n as f32;
    if first + MULTI_SPACE * (n as f32 - 1.0) + MULTI_LINE_HEIGHT <= MULTI_BOX_BOTTOM - 5.0 {
        return first + MULTI_SPACE * i as f32;
    }
    let first = (MULTI_BOX_TOP + MULTI_BOX_BOTTOM) / 2.0 - MULTI_SPACE_TIGHT * n as f32 / 2.0;
    first + MULTI_SPACE_TIGHT * i as f32
}

#[derive(Default)]
pub struct PauseVideo {
    /// Each open in-game menu (one per player's menu in split screen) and its Video line.
    pages: Vec<(ObjId, ObjId)>,
    /// The line was chosen: the Video page being opened comes from it.
    opening: bool,
}

impl Vm {
    fn pause_video_of(&self, page: ObjId) -> Option<ObjId> {
        self.improved.pause_video.pages.iter().find(|p| p.0 == page).map(|p| p.1)
    }

    /// Hook of XIIIMenuInGame's Created, Paint and InternalOnClick (Vm::call): the result when
    /// handled here, otherwise the arguments back.
    #[allow(clippy::type_complexity)]
    pub(crate) fn pause_video_call(
        &mut self,
        f: usize,
        this: ObjId,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        let d = &self.functions[f];
        if d.state.is_some() {
            return Err(args);
        }
        let owner = &self.classes[d.owner].name;
        if self.improved.pause_video.opening && d.name == "Created" && owner.eq_ignore_ascii_case("XIIIMenuVideoClientWindow") {
            self.improved.pause_video.opening = false;
            return Ok(self.pause_video_page_created(f, this, args));
        }
        let multi = owner.eq_ignore_ascii_case(MULTI_MENU_CLASS);
        if !multi && !owner.eq_ignore_ascii_case("XIIIMenuInGame") {
            return Err(args);
        }
        match d.name.as_str() {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    crate::improved::gui::offer_pause_video(self, this, multi);
                }
                Ok(r)
            }
            "Paint" if self.pause_video_of(this).is_some() => {
                // Paint clears the frame of its own lines only (Controls[0..6], or NbButtons in
                // multiplayer): the added one is cleared here.
                for c in self.controls(this).into_iter().flatten() {
                    self.set_prop(c, "bUseBorder", Value::Bool(false));
                }
                Ok(self.call_inner(f, this, args))
            }
            "InternalOnClick" if self.pause_video_of(this).is_some() => {
                let sender = args.first().cloned().flatten().and_then(|v| v.as_object());
                if sender.is_some() && sender == self.pause_video_of(this) {
                    if let Some(root) = self.propr(this, "myRoot").as_object() {
                        self.improved.pause_video.opening = true;
                        // In a level the root's OpenMenu only knows the in-game pages
                        // (XIIIMenuData.XIIIIngameMenuStackStrings): the page by its class.
                        let _ = match self.class(VIDEO_PAGE_CLASS) {
                            Some(class) => self.call_named(root, "OpenMenuWithClass", vec![Value::Class(Some(class))]),
                            None => self.call_named(root, "OpenMenu", vec![Value::Str(VIDEO_PAGE.to_string())]),
                        };
                        self.improved.pause_video.opening = false;
                    }
                    return Ok(Ok((Value::Bool(true), Vec::new())));
                }
                Ok(self.call_inner(f, this, args))
            }
            _ => Err(args),
        }
    }

    /// Created of the Video page opened from the line: built on the PC platform (its Resolution
    /// line and video_menu.rs's), which it keeps while open (ShowWindow and Enter read it); the
    /// footer keeps the buttons of the menu's platform.
    fn pause_video_page_created(&mut self, f: usize, page: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        let root = self.propr(page, "myRoot").as_object();
        let pf = root.map_or(0, |r| self.propr(r, "CurrentPF").as_int());
        if let Some(r) = root.filter(|_| pf != 0) {
            self.set_prop(r, "CurrentPF", Value::Int(0));
        }
        let r = match self.video_menu_call(f, page, args) {
            Ok(r) => r,
            Err(args) => self.call_inner(f, page, args),
        };
        self.pause_video_page_help_bar(page);
        if pf > 0 {
            if let Value::Array(b) = self.prop(page, "tButton") {
                let pick = |i: i32| b.get(i as usize).cloned();
                if let (Some(a), Some(c)) = (pick(pf * 2), pick(pf * 2 + 1)) {
                    self.set_prop(page, "tAccept", a);
                    self.set_prop(page, "tCancel", c);
                }
            }
        }
        r
    }

    /// The page's help bar (Escape Back ... Accept Enter) drawn over the game as on the main menu.
    /// XIIIWindow.DisplayHelpBar draws it only when `myRoot.bMapMenu || bForceHelp`: bMapMenu is
    /// set on the main menu's map only, and the in-game pages set bForceHelp in their
    /// defaultproperties (XIIIMenuInGame, XIIIMenuControlsWindow, XIIIMenuCompetencesIngame...);
    /// the Video page, a main-menu page in the original, does not. Set on this page object only:
    /// the page opened from the main menu keeps its own defaults.
    pub(crate) fn pause_video_page_help_bar(&mut self, page: ObjId) {
        self.set_prop(page, "bForceHelp", Value::Bool(true));
    }

    /// The line's text: the Options page's own label of its video button, word for word, in the
    /// game's language (XIIIMenuOptions.VideoText of XIDInterf: Video, Vidéo, Grafik, Video,
    /// Gráficos), never a translation of ours.
    pub(crate) fn pause_video_label(&mut self) -> String {
        self.original_text("XIIIMenuOptions", "VideoText", "XIDInterf").unwrap_or_else(|| "Video".to_string())
    }

    /// Menus since closed, and `page` if it is made again, are forgotten.
    fn pause_video_forget(&mut self, page: ObjId) {
        let live = |vm: &Vm, o: ObjId| vm.objects.get(o).is_some_and(|x| x.is_some());
        let kept: Vec<(ObjId, ObjId)> = self.improved.pause_video.pages.iter().copied().filter(|&(p, _)| p != page && live(self, p)).collect();
        self.improved.pause_video.pages = kept;
    }

    /// After XIIIMenuInGameMulti's Created: the Video line made like the menu's own lines, put
    /// after Options, and the lines laid out again for one more.
    pub(crate) fn pause_video_multi_created(&mut self, page: ObjId) {
        self.pause_video_forget(page);
        let Some(class) = self.class("XIIIButton") else { return };
        // The menu's lines in their order; those the game did not make are None.
        let lines: Vec<ObjId> = ["ReturnButton", "TeamButton", "BombButton", "KickButton", "OptionsButton", "QuitButton"]
            .iter()
            .filter_map(|n| self.propr(page, n).as_object())
            .filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()))
            .collect();
        let Some(options) = self.propr(page, "OptionsButton").as_object() else { return };
        let mut controls = self.controls(page);
        let Some(at) = controls.iter().position(|&c| c == Some(options)) else { return };
        let Some(after) = lines.iter().position(|&c| c == options) else { return };
        let s = self.page_scale(page);
        let n = lines.len() + 1;
        let x = self.propr(page, "BoxPosX").as_int() as f32;
        let w = self.propr(page, "BoxWidth").as_int() as f32;
        let args = vec![
            Value::Class(Some(class)),
            Value::Float(x),
            Value::Float(multi_line_y(after + 1, n) * s),
            Value::Float(w),
            Value::Float(MULTI_LINE_HEIGHT * s),
        ];
        let Some(line) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { return };
        let text = self.pause_video_label();
        self.set_prop(line, "Text", Value::Str(text));
        self.set_prop(line, "bUseBorder", Value::Bool(false));
        let split = self.prop(options, "NbMultiSplit");
        self.set_prop(line, "NbMultiSplit", split);
        controls.insert(at + 1, Some(line));
        let mut placed = lines;
        placed.insert(after + 1, line);
        for (i, &c) in placed.iter().enumerate() {
            self.set_prop(c, "WinTop", Value::Float(multi_line_y(i, n) * s / 480.0));
        }
        self.set_prop(page, "Controls", Value::Array(controls.into_iter().map(Value::Object).collect()));
        self.improved.pause_video.pages.push((page, line));
    }

    /// After Created: the Video line made like the menu's own lines, put after Controls, and the
    /// seven lines laid out.
    pub(crate) fn pause_video_created(&mut self, page: ObjId) {
        self.pause_video_forget(page);
        let Some(class) = self.class("XIIIButton") else { return };
        // The menu's six lines, by name (Controls may hold other controls after them).
        let named: Vec<Option<ObjId>> =
            ["SaveBtn", "ReturnBtn", "ControlsBtn", "CompetencesBtn", "RestartBtn", "QuitBtn"].iter().map(|n| self.propr(page, n).as_object()).collect();
        if named.iter().any(Option::is_none) {
            return;
        }
        let mut controls = self.controls(page);
        let Some(at) = controls.iter().position(|&c| c == named[2]) else { return };
        let s = self.page_scale(page);
        let args =
            vec![Value::Class(Some(class)), Value::Float(230.0), Value::Float(line_y(LINE_INDEX, 7) * s), Value::Float(200.0), Value::Float(LINE_SPACE * s)];
        let Some(line) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { return };
        let text = self.pause_video_label();
        self.set_prop(line, "Text", Value::Str(text));
        self.set_prop(line, "bUseBorder", Value::Bool(false));
        controls.insert(at + 1, Some(line));
        let mut lines = named;
        lines.insert(LINE_INDEX, Some(line));
        for (i, c) in lines.iter().enumerate() {
            if let Some(c) = *c {
                self.set_prop(c, "WinTop", Value::Float(line_y(i, 7) * s / 480.0));
            }
        }
        self.set_prop(page, "Controls", Value::Array(controls.into_iter().map(Value::Object).collect()));
        self.improved.pause_video.pages.push((page, line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six lines: the original's places; seven: within the menu's box (130 to 360).
    #[test]
    fn lines_in_the_box() {
        for i in 0..6 {
            assert_eq!(line_y(i, 6), 155.0 + 30.0 * i as f32);
        }
        assert!(line_y(0, 7) >= 130.0);
        assert!(line_y(6, 7) + LINE_SPACE <= 360.0);
    }

    /// Multiplayer: n + 1 lines placed by the menu's own rule while they fit in its box (n of the
    /// game's lines give the original's places), seven 30 apart within it.
    #[test]
    fn multi_lines_in_the_box() {
        for n in 3..=6 {
            for i in 0..n {
                assert_eq!(multi_line_y(i, n), 250.0 - 18.0 * n as f32 + 35.0 * i as f32, "{i} of {n}");
            }
        }
        assert_eq!(multi_line_y(0, 7), 140.0);
        for n in 3..=7 {
            assert!(multi_line_y(0, n) >= MULTI_BOX_TOP, "{n}");
            assert!(multi_line_y(n - 1, n) + MULTI_LINE_HEIGHT <= MULTI_BOX_BOTTOM, "{n}");
        }
    }

    /// The in-game menu (XIIIMenuInGame) made by the game's own Created: seven lines (the Video
    /// line) with Graphics Original and Enhanced, so that Enhanced can be chosen again in game.
    /// Needs OXIII_GAME.
    #[test]
    fn video_line_with_both_graphics() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let class = vm.class("XIIIMenuInGame").expect("XIIIMenuInGame");
        let mut buttons = Vec::new();
        for enhanced in [false, true] {
            crate::video_settings::set_graphics_enhanced(enhanced);
            let page = vm.exec_new(None, None, 0, Some(class)).0.expect("new XIIIMenuInGame");
            let _ = vm.call_named(page, "Created", Vec::new());
            let n = vm.controls(page).into_iter().flatten().filter(|&c| vm.is_a(c, "XIIIButton")).count();
            buttons.push((n, vm.pause_video_of(page).is_some()));
        }
        crate::video_settings::set_graphics_enhanced(false);
        assert_eq!(buttons, [(7, true), (7, true)], "(buttons, Video line) with Original, Enhanced");
    }

    /// The multiplayer in-game menu (XIIIMenuInGameMulti) made by the game's own Created, outside
    /// a game (Return, Options, Quit): the Video line after Options with Graphics Original and
    /// Enhanced, the four lines laid out by the menu's rule for four. Needs OXIII_GAME.
    #[test]
    fn multiplayer_menu_has_the_video_line() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let class = vm.class(MULTI_MENU_CLASS).expect("XIIIMenuInGameMulti");
        let root_class = vm.class("XIIIRootWindow").expect("XIIIRootWindow");
        let root = vm.exec_new(None, None, 0, Some(root_class)).0.expect("new XIIIRootWindow");
        vm.set_prop(root, "fTextureScaleFactorForConsole", Value::Float(1.0));
        for enhanced in [false, true] {
            crate::video_settings::set_graphics_enhanced(enhanced);
            let page = vm.exec_new(None, None, 0, Some(class)).0.expect("new XIIIMenuInGameMulti");
            // Outside a game the page has no root: XIIIWindow.Created takes fScaleTo from it
            // (myRoot.fTextureScaleFactorForConsole), so without one every line is at y 0.
            vm.set_prop(page, "myRoot", Value::Object(Some(root)));
            let _ = vm.call_named(page, "Created", Vec::new());
            let buttons: Vec<ObjId> = vm.controls(page).into_iter().flatten().filter(|&c| vm.is_a(c, "XIIIButton")).collect();
            let s = vm.page_scale(page);
            let tops: Vec<f32> = buttons.iter().map(|&b| vm.propr(b, "WinTop").as_float() * 480.0 / s).collect();
            let names: Vec<Option<ObjId>> = ["ReturnButton", "OptionsButton", "QuitButton"].iter().map(|n| vm.propr(page, n).as_object()).collect();
            let line = vm.pause_video_of(page);
            assert!(line.is_some(), "Video line (enhanced: {enhanced})");
            let expected: Vec<Option<ObjId>> = vec![names[0], names[1], line, names[2]];
            assert_eq!(buttons.iter().map(|&b| Some(b)).collect::<Vec<_>>(), expected, "order (enhanced: {enhanced})");
            let n = buttons.len();
            for (i, t) in tops.iter().enumerate() {
                assert!((t - multi_line_y(i, n)).abs() < 0.5, "line {i} of {n} at {t}, not {}", multi_line_y(i, n));
            }
            if let Some(line) = line {
                assert_eq!(vm.propr(line, "Text").as_name(), vm.pause_video_label());
            }
        }
        crate::video_settings::set_graphics_enhanced(false);
    }

    /// The Video page opened from the in-game menu shows its help bar as the other in-game pages
    /// do (bForceHelp, as their defaults); the main menu's page keeps the original's defaults.
    /// Control: without the hook the page has no bForceHelp, so no bar in a level. Needs
    /// OXIII_GAME.
    #[test]
    fn the_page_opened_in_game_shows_its_help_bar() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| g.join("System").is_dir()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let new_of = |vm: &mut Vm, class: &str| {
            let c = vm.class(class).unwrap_or_else(|| panic!("{class}"));
            vm.exec_new(None, None, 0, Some(c)).0.unwrap_or_else(|| panic!("new {class}"))
        };
        for in_game in ["XIIIMenuInGame", "XIIIMenuControlsWindow"] {
            let p = new_of(&mut vm, in_game);
            assert!(vm.propr(p, "bForceHelp").as_bool(), "{in_game} forces its bar in a level");
        }
        let main_menu_page = new_of(&mut vm, VIDEO_PAGE_CLASS);
        let opened_in_game = new_of(&mut vm, VIDEO_PAGE_CLASS);
        assert!(!vm.propr(opened_in_game, "bForceHelp").as_bool(), "control: the original page has no bar in a level");
        vm.pause_video_page_help_bar(opened_in_game);
        assert!(vm.propr(opened_in_game, "bForceHelp").as_bool(), "the page opened in game has its bar");
        assert!(!vm.propr(main_menu_page, "bForceHelp").as_bool(), "the main menu's page keeps its defaults");
        assert!(vm.propr(opened_in_game, "bDisplayBar").as_bool());
    }

    /// The line carries the Options page's label of its video button in each of the game's five
    /// languages, as the game's own text files give it (needs OXIII_GAME).
    #[test]
    fn label_is_the_options_page_word_in_every_language() {
        let Some(game) = std::env::var_os("OXIII_GAME").map(std::path::PathBuf::from) else { return };
        let system = game.join("System");
        let mut checked = 0;
        for ext in ["int", "frt", "det", "itt", "est"] {
            let wanted = format!("XIDInterf.{ext}");
            let Some(file) = std::fs::read_dir(&system)
                .ok()
                .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&wanted))))
            else {
                continue;
            };
            let bytes = std::fs::read(&file).unwrap();
            let text: String = if bytes.starts_with(&[0xFF, 0xFE]) {
                let units: Vec<u16> = bytes[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
                String::from_utf16_lossy(&units)
            } else {
                bytes.iter().map(|&b| b as char).collect()
            };
            let mut in_section = false;
            let mut expected = None;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_section = line.eq_ignore_ascii_case("[XIIIMenuOptions]");
                } else if let Some(v) = line.strip_prefix("VideoText=").filter(|_| in_section) {
                    expected = Some(v.trim().to_string());
                }
            }
            let expected = expected.expect("VideoText in the game's file");
            let dir = std::env::temp_dir().join(format!("oxiii-pause-video-{}-{ext}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("player")).unwrap();
            let ini = format!("[Engine.Engine]\r\nLanguage={ext}\r\n[Core.System]\r\nPaths=../System/*.u\r\n");
            std::fs::write(dir.join("Default.ini"), &ini).unwrap();
            std::fs::write(dir.join("XIII.ini"), &ini).unwrap();
            std::fs::write(dir.join("DefUser.ini"), "[DefaultPlayer]\r\nName=XIII\r\n").unwrap();
            std::fs::copy(&file, dir.join(&wanted)).unwrap();
            let mut vm = Vm::new();
            vm.config = Some(crate::config::ConfigCache::with_command_line(dir.clone(), Some(dir.join("player")), "XIII.exe", true));
            assert_eq!(vm.pause_video_label(), expected, "{ext}");
            checked += 1;
            let _ = std::fs::remove_dir_all(&dir);
        }
        assert_eq!(checked, 5, "the five language files of the game");
    }
}
