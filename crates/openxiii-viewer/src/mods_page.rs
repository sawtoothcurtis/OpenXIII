//! Improved build only: the Mods page, opened by the main menu's Mods box. Lists the mods of the
//! Mods folder, each with a check box; turning one on or off writes the Mods folder's list of
//! mods turned off (openxiii_mods::DISABLED_FILE), which the next launch honours (mods load at
//! startup). Two checked mods that set the same thing are shown in red, live, with the other mod
//! and what both set (openxiii_mods::find_conflicts, the checks made at load time).
//!
//! Drawn like the in-game Skills page (XIIIMenuCompetencesIngame): the white message box of
//! UCanvas::DrawMsgboxBackground with its bands, the title on a white tab, one row every 40
//! units of the 640 × 480 menu (arrow on the focused row, check box, name), and the focused
//! row's text at the bottom of the box. The check mark and the arrow are the game's own
//! textures, read from the installed game. Keyboard, mouse, gamepad and touch.
//!
//! The first row is Co-op, shipped with the game: checked by default, it can be unchecked but
//! not removed (no folder). Its state is a line of the same list of mods turned off
//! ([`COOP_ENTRY`]) and, like the mods, it applies at next launch: unchecked, the main menu has
//! no Co-op box and the multiplayer pages no players setting (openxiii_vm::menu_boxes::set_coop_box).
//!
//! Below it, the mods turned on, then the mods turned off, each group in folder-name order
//! (case ignored). A row keeps its place while the page is open, so that the row just checked
//! or unchecked does not move under the cursor: the order is made again when the page is opened.
//! The mods load in folder-name order whatever their place here, and the conflicts shown follow
//! that load order. The example mods (Mods\_Examples) are not listed: an example becomes a mod
//! when its folder is copied into the Mods folder. The names catalog is written by the game
//! itself at startup (openxiii_vm::mod_catalog), so the page has no row for it.
//!
//! The mods (and Co-op) are read once, at startup: a row whose box differs from what this run
//! loaded (turned on or off here, or a mod copied into Mods while the game runs) says "Restart
//! needed" in red under its name, until its box is set back. A mod in conflict with another
//! checked one has a red "!" beside its box, on both rows of the pair.
//!
//! It is a full-screen page: an opaque white background covers the whole screen, and while it is
//! open the main menu behind gets no input (keys, buttons, wheel and gamepad go to the page;
//! the cursor still moves, but the menu is not hovered: Vm::set_menu_overlay).

use crate::choice_level::{ButtonsMenu, Finger};
use openxiii_vm::added_text::{self, fill, Text};
use openxiii_vm::canvas::HudDraw;
use openxiii_vm::Vm;

const FONT: &str = "XIIIFonts.PoliceF16";
/// The Skills page's font for the text at the bottom of the box.
const SMALL_FONT: &str = "XIIIFonts.XIIIConsoleFont";
/// XIIIMenuCompetencesIngame.ValidatedTex: the check mark.
const VALID_TEX: &str = "XIIIMenu.HUD.Valid";
/// XIIIMenuCompetencesIngame.ArrowText: the arrow of the focused row.
const ARROW_TEX: &str = "XIIIMenuStart.Control_Console.fleche_droite";
// Colours B, G, R, A (HudDraw).
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const RED: [u8; 4] = [0, 0, 200, 255];
const GREY: [u8; 4] = [110, 110, 110, 255];
/// The page's background: opaque white over the whole screen (nothing of the menu shows).
const PAGE: [u8; 4] = WHITE;
/// Rows shown at once (one fewer than the Skills page's eight: the room under the list holds a
/// mod's whole description at 480 pixels high).
const ROWS: usize = 7;
/// Message box of the Skills page, in 640 × 480 units: x, y, band width, band height, w, h.
const MSGBOX: [f32; 6] = [120.0, 50.0, 10.0, 10.0, 420.0, 390.0];

/// The built-in Co-op entry's name in the list of mods turned off. `*` cannot be in a Windows
/// folder name, so no mod folder can take it.
pub const COOP_ENTRY: &str = "*co-op";

/// Co-op is on for the Mods folder `folder` (on when there is no folder or no list).
pub fn coop_enabled(folder: Option<&std::path::Path>) -> bool {
    folder.is_none_or(|f| !openxiii_mods::is_disabled(&openxiii_mods::read_disabled(f), COOP_ENTRY))
}

/// The lines earlier builds wrote for their built-in flamethrower and Xbox loading recaps rows
/// (both are now example mods like the others): they turn nothing on or off, and go when the
/// list is written.
const OLD_BUILTIN_LINES: &[&str] = &["*flamethrower=on", "*xbox-recaps=on"];

/// What the game read at startup: the folders of the mods it loaded, and whether Co-op was on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AtStart {
    pub mods: Vec<String>,
    pub coop: bool,
}

/// Co-op as the game read it at startup (main.rs, with the Co-op box of the main menu).
static COOP_AT_START: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Records Co-op's state read at startup (first call wins).
pub fn note_coop_at_start(on: bool) {
    let _ = COOP_AT_START.set(on);
}

/// What this run read at startup; None when the mods were not read (no restart to ask for).
fn at_start() -> Option<AtStart> {
    Some(AtStart { mods: openxiii_mods::loaded_at_start()?.to_vec(), coop: COOP_AT_START.get().copied().unwrap_or(true) })
}

/// Row name `name` is one of the entries shipped with the game (no folder).
fn builtin_name(name: &str) -> bool {
    name == COOP_ENTRY
}

/// The line of the list of mods turned off for row `m`, if its state needs one: a mod or Co-op
/// unchecked.
fn off_line(m: &openxiii_mods::Listing) -> Option<String> {
    (!m.enabled).then(|| m.folder_name.clone())
}

/// Keys of the page (keyboard or gamepad).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Toggle,
    Back,
}

/// The mods of the folder, which are turned off and the conflicts of the checked ones: the
/// page's model, without drawing.
#[derive(Default, Debug)]
pub struct ModList {
    pub folder: Option<std::path::PathBuf>,
    pub mods: Vec<openxiii_mods::Listing>,
    /// Every mod of the folder as the game reads it (turned off or not), for the conflicts.
    loaded: Vec<openxiii_mods::Mod>,
    /// Conflicts between the checked mods.
    pub conflicts: Vec<openxiii_mods::Conflict>,
    /// What the game read at startup (None: no restart is ever asked for).
    pub at_start: Option<AtStart>,
}

impl ModList {
    /// Reads the Mods folder (`folder`; none: nothing to list), against what this run loaded.
    pub fn read(folder: Option<std::path::PathBuf>) -> ModList {
        Self::read_with(folder, at_start())
    }

    /// Reads the Mods folder against `at_start`, what the game read when it started.
    pub fn read_with(folder: Option<std::path::PathBuf>, at_start: Option<AtStart>) -> ModList {
        let mut mods = folder.as_deref().map(openxiii_mods::list_mods).unwrap_or_default();
        // Shown: the mods turned on first, then the others, by folder name with case ignored.
        mods.sort_by_cached_key(|m| (!m.enabled, m.folder_name.to_lowercase(), m.folder_name.clone()));
        let coop = openxiii_mods::Listing { folder_name: COOP_ENTRY.into(), name: "Co-op".into(), version: String::new(), enabled: coop_enabled(folder.as_deref()) };
        mods.insert(0, coop);
        let loaded = folder.as_deref().map(|f| openxiii_mods::discover_all(f).mods).unwrap_or_default();
        let mut l = ModList { folder, mods, loaded, conflicts: Vec::new(), at_start };
        l.recompute();
        l
    }

    /// Conflicts of the checked mods, loaded in folder order as at startup.
    fn recompute(&mut self) {
        let on: Vec<openxiii_mods::Mod> =
            self.loaded.iter().filter(|m| self.mods.iter().any(|l| l.enabled && l.folder_name.eq_ignore_ascii_case(&m.folder_name))).cloned().collect();
        self.conflicts = openxiii_mods::find_conflicts(&on);
    }

    /// Turns mod `i` on or off, checks the conflicts again and writes the list of the mods
    /// turned off. The folder names already in the file that are not listed (a mod folder
    /// removed since) are kept.
    pub fn toggle(&mut self, i: usize) -> std::io::Result<()> {
        let Some(m) = self.mods.get_mut(i) else { return Ok(()) };
        m.enabled = !m.enabled;
        self.recompute();
        let Some(folder) = &self.folder else { return Ok(()) };
        // The built-in entry is listed even without a Mods folder: the folder is made for its line.
        if !folder.is_dir() {
            std::fs::create_dir_all(folder)?;
        }
        let mut off: Vec<String> = openxiii_mods::read_disabled(folder)
            .into_iter()
            .filter(|d| !OLD_BUILTIN_LINES.iter().any(|o| d.eq_ignore_ascii_case(o)) && !self.mods.iter().any(|m| m.folder_name.eq_ignore_ascii_case(d)))
            .collect();
        off.extend(self.mods.iter().filter_map(off_line));
        openxiii_mods::write_disabled(folder, &off)
    }

    /// Row `i`'s box differs from what the game read at startup: a restart applies it.
    pub fn needs_restart(&self, i: usize) -> bool {
        let (Some(s), Some(m)) = (&self.at_start, self.mods.get(i)) else { return false };
        match m.folder_name.as_str() {
            COOP_ENTRY => m.enabled != s.coop,
            f => m.enabled != s.mods.iter().any(|l| l.eq_ignore_ascii_case(f)),
        }
    }

    /// Row `i` is a checked mod in conflict with another checked one (its "!").
    pub fn conflict_mark(&self, i: usize) -> bool {
        !self.conflicts_of(i).is_empty()
    }

    /// Row `i` is the built-in entry (Co-op).
    pub fn is_builtin(&self, i: usize) -> bool {
        self.mods.get(i).is_some_and(|m| builtin_name(&m.folder_name))
    }

    /// Number of built-in rows (first in the list).
    fn builtin_count(&self) -> usize {
        self.mods.iter().take_while(|m| builtin_name(&m.folder_name)).count()
    }

    /// Name shown on row `i` in language `lang`.
    fn shown_name(&self, i: usize, lang: &str) -> String {
        match self.mods.get(i) {
            Some(_) if self.is_builtin(i) => added_text::text(lang, Text::MenuCoop).to_string(),
            Some(m) => m.name.clone(),
            None => String::new(),
        }
    }

    /// Name shown for mod folder `folder` (its mod.toml name, else the folder).
    fn name_of<'a>(&'a self, folder: &'a str) -> &'a str {
        self.mods.iter().find(|m| m.folder_name.eq_ignore_ascii_case(folder)).map_or(folder, |m| m.name.as_str())
    }

    /// Conflicts of mod `i` (checked): the other mod's name, what both set, and whether this one
    /// wins (loaded later).
    pub fn conflicts_of(&self, i: usize) -> Vec<(&str, &str, bool)> {
        let Some(m) = self.mods.get(i).filter(|m| m.enabled) else { return Vec::new() };
        self.conflicts
            .iter()
            .filter(|c| c.involves(&m.folder_name))
            .map(|c| (self.name_of(c.other(&m.folder_name)), c.what.as_str(), c.second.eq_ignore_ascii_case(&m.folder_name)))
            .collect()
    }

    /// Description of mod `i` from its mod.toml (author, description), if any.
    /// `lang`: the game's language (added_text).
    fn about(&self, i: usize, lang: &str) -> String {
        let Some(l) = self.mods.get(i) else { return String::new() };
        let version = |author: Option<&str>| match author {
            Some(a) => fill(added_text::text(lang, Text::VersionBy), &[&l.version, a]) + ".",
            None => fill(added_text::text(lang, Text::Version), &[&l.version]) + ".",
        };
        let Some(m) = self.loaded.iter().find(|m| m.folder_name.eq_ignore_ascii_case(&l.folder_name)) else { return version(None) };
        let mut s = version(m.info.author.as_deref());
        if let Some(d) = &m.info.description {
            s.push(' ');
            s.push_str(d);
        }
        s
    }

    /// Paragraphs shown at the bottom of the box for mod `i`, before wrapping: its full name
    /// (cut with "..." in the list when long), its description, then its conflicts in red.
    fn details(&self, i: usize, lang: &str) -> Vec<(String, [u8; 4])> {
        if self.mods.get(i).is_none() {
            return Vec::new();
        }
        if self.is_builtin(i) {
            let about = format!("{}. {}", added_text::text(lang, Text::IncludedWithGame), added_text::text(lang, Text::CoopEntryAbout));
            let mut out = vec![(self.shown_name(i, lang), BLACK), (about, BLACK)];
            if self.needs_restart(i) {
                out.push((added_text::text(lang, Text::RestartNeededAbout).to_string(), RED));
            }
            return out;
        }
        let mut out = vec![(self.shown_name(i, lang), BLACK), (self.about(i, lang), BLACK)];
        if self.needs_restart(i) {
            out.push((added_text::text(lang, Text::RestartNeededAbout).to_string(), RED));
        }
        for (other, what, wins) in self.conflicts_of(i) {
            let t = if wins { Text::ConflictThisWins } else { Text::ConflictOtherWins };
            out.push((fill(added_text::text(lang, t), &[other, &added_text::what(lang, what)]), RED));
        }
        out
    }
}

/// Layout of the last frame, in screen pixels (for the mouse and fingers).
#[derive(Clone, Copy, Default)]
struct Set {
    back: [f32; 4],
    msgbox: [f32; 4],
    /// Rows: left, top of the first, right, row step.
    rows: [f32; 4],
    row_h: f32,
}

fn inside(r: [f32; 4], (x, y): (f32, f32)) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

/// A press (mouse or finger) followed until release: where it started and the first row then.
#[derive(Clone, Copy)]
struct Press {
    start: (f32, f32),
    high: usize,
    slides: bool,
}

#[derive(Default)]
pub struct ModsPage {
    /// Page open over the main menu.
    pub open: bool,
    list: ModList,
    choice: usize,
    high: usize,
    set: Set,
    mouse: (f32, f32),
    /// Error of the last write (the system's message; shown on the page).
    error: Option<String>,
    /// Gamepads: previous state (edges); hidden from the game while open and until released.
    pad_before: ButtonsMenu,
    pad_mask: bool,
    /// Improved build: up / down held repeat like the other menus (the VM's menu_repeat.rs).
    pad_repeat: openxiii_vm::menu_repeat::DirectionRepeat,
    /// A mouse press taken by the page: its release too.
    click_taken: bool,
    press: Option<Press>,
    /// Touch screen: the finger followed by the page.
    finger: Option<u64>,
    /// Focus moves since the last [`ModsPage::take_ticks`]: each plays the menus' focus tick.
    ticks: u32,
    /// The row under the mouse at its last move (None: over no row).
    mouse_row: Option<usize>,
}

impl ModsPage {

    /// Opens the page on the mods of the Mods folder.
    pub fn open(&mut self) {
        *self = ModsPage {
            open: true,
            list: ModList::read(openxiii_mods::mods_folder()),
            pad_mask: true,
            pad_before: self.pad_before,
            mouse: self.mouse,
            ..Default::default()
        };
    }

    /// Focus on row `i`; a move to another row is a focus tick, as on the game's own buttons.
    fn focus_row(&mut self, i: usize) {
        if i != self.choice {
            self.ticks += 1;
        }
        self.choice = i;
    }

    /// The mouse over row `i`: the focus moves there with its tick, and coming back onto the
    /// focused row after the mouse was over no row ticks too (this page is OpenXIII's own: every
    /// entry onto a row ticks, the owner's choice).
    fn hover_row(&mut self, i: usize) {
        if i == self.choice && self.mouse_row.is_none() {
            self.ticks += 1;
        }
        self.focus_row(i);
    }

    /// Focus ticks to play since the last call (the game window plays them with the game's
    /// MnCurseur, openxiii_vm::menu_sounds).
    pub fn take_ticks(&mut self) -> u32 {
        std::mem::take(&mut self.ticks)
    }

    /// Rows of the list: the built-in entries and the mods.
    fn rows(&self) -> usize {
        self.list.mods.len()
    }

    fn toggle(&mut self) {
        self.error = self.list.toggle(self.choice).err().map(|e| e.to_string());
    }

    /// Paragraphs at the bottom of the box for the focused row, before wrapping: an error or the
    /// row's text, then the on / off help in grey.
    fn bottom_paragraphs(&self, lang: &str, gamepad: bool) -> Vec<(String, [u8; 4])> {
        let mut out: Vec<(String, [u8; 4])> = Vec::new();
        if let Some(e) = &self.error {
            out.push((fill(added_text::text(lang, Text::CannotWrite), &[openxiii_mods::DISABLED_FILE, e]), RED));
        } else if self.choice < self.list.mods.len() {
            out.extend(self.list.details(self.choice, lang));
        }
        out.push((added_text::text(lang, if gamepad { Text::HelpPad } else { Text::HelpKeys }).to_string(), GREY));
        out.retain(|(t, _)| !t.trim().is_empty());
        out
    }

    /// Keeps the focused row on screen.
    fn follow(&mut self) {
        if self.choice < self.high {
            self.high = self.choice;
        } else if self.choice >= self.high + ROWS {
            self.high = self.choice + 1 - ROWS;
        }
    }

    /// Keyboard or gamepad key, page open. As on the Skills page, up from the first row goes to
    /// the last one and down from the last to the first.
    pub fn key(&mut self, k: Key) {
        let n = self.rows();
        match k {
            Key::Up if n > 0 => self.focus_row((self.choice + n - 1) % n),
            Key::Down if n > 0 => self.focus_row((self.choice + 1) % n),
            Key::Toggle if n > 0 => self.toggle(),
            Key::Back => self.open = false,
            _ => {}
        }
        self.follow();
    }

    /// Row under screen point `pos`, if any.
    fn row_at(&self, pos: (f32, f32)) -> Option<usize> {
        let [left, top, right, step] = self.set.rows;
        if pos.0 < left || pos.0 >= right || step <= 0.0 {
            return None;
        }
        let k = ((pos.1 - top) / step).floor();
        // The row's band: half a step above and below its centre line.
        if k < 0.0 || k >= ROWS as f32 {
            return None;
        }
        let i = self.high + k as usize;
        (i < self.rows()).then_some(i)
    }

    /// A tap or a click released at `pos`: a row turns its mod on or off; Back closes the page
    /// (elsewhere on the page: nothing).
    fn tap(&mut self, pos: (f32, f32)) {
        if inside(self.set.back, pos) {
            self.open = false;
        } else if let Some(i) = self.row_at(pos) {
            self.focus_row(i);
            self.toggle();
        }
    }

    /// A press (or a finger) moved to `pos`: drags the list when it has more rows than shown.
    fn moves(&mut self, pos: (f32, f32)) {
        let step = self.set.row_h.max(1.0);
        let max = self.rows().saturating_sub(ROWS);
        if let Some(p) = &mut self.press {
            let dy = pos.1 - p.start.1;
            if !p.slides && (dy.abs() > step * 0.5 || (pos.0 - p.start.0).abs() > step * 0.5) {
                p.slides = true;
            }
            if p.slides {
                self.high = (p.high as f32 - dy / step).round().clamp(0.0, max as f32) as usize;
            }
        }
    }

    fn release(&mut self, pos: (f32, f32)) {
        if let Some(p) = self.press.take() {
            if !p.slides {
                self.tap(pos);
            }
        }
    }

    /// Mouse button, page open: a click on a row turns its mod on or off, on Back closes.
    pub fn click(&mut self, pressed: bool) -> bool {
        if !self.open {
            return std::mem::take(&mut self.click_taken) && !pressed;
        }
        if pressed {
            self.click_taken = true;
            self.press = Some(Press { start: self.mouse, high: self.high, slides: false });
            return true;
        }
        // The release of the click that opened the page (on the menu box) is the game's.
        if !std::mem::take(&mut self.click_taken) {
            return false;
        }
        self.release(self.mouse);
        true
    }

    /// Touch screen, page open: a tap acts as a click, a drag scrolls the list. True if the
    /// page takes the finger.
    pub fn touch(&mut self, id: u64, phase: Finger, pos: (f32, f32)) -> bool {
        match phase {
            Finger::Pose if self.open && self.finger.is_none() => {
                self.finger = Some(id);
                self.press = Some(Press { start: pos, high: self.high, slides: false });
                true
            }
            Finger::Moves if self.finger == Some(id) => {
                self.moves(pos);
                true
            }
            Finger::Raised if self.finger == Some(id) => {
                self.finger = None;
                if self.open {
                    self.release(pos);
                }
                true
            }
            Finger::Cancels if self.finger == Some(id) => {
                self.finger = None;
                self.press = None;
                true
            }
            _ => false,
        }
    }

    /// Mouse wheel, page open.
    pub fn wheel(&mut self, notches: f32) {
        let max = self.rows().saturating_sub(ROWS) as i32;
        self.high = (self.high as i32 - notches.round() as i32).clamp(0, max) as usize;
    }

    /// Gamepads: d-pad up / down, A toggles, B goes back. Returns whether the gamepads are hidden
    /// from the game this frame.
    pub fn pad(&mut self, b: ButtonsMenu) -> bool {
        let before = std::mem::replace(&mut self.pad_before, b);
        let held = [self.open && b.high, self.open && b.low_value, false, false];
        let repeats = self.pad_repeat.update(held, openxiii_vm::menu_repeat::clock());
        if self.open {
            if (b.high && !before.high) || repeats[0] {
                self.key(Key::Up);
            } else if (b.low_value && !before.low_value) || repeats[1] {
                self.key(Key::Down);
            } else if b.validate && !before.validate {
                self.key(Key::Toggle);
            } else if b.return_value && !before.return_value {
                self.key(Key::Back);
            }
        }
        if !self.open && b == ButtonsMenu::default() {
            self.pad_mask = false;
        }
        self.pad_mask
    }

    /// Draws the page on a `w` × `h` image when open (closed outside the main menu).
    pub fn draw(&mut self, vm: &mut Vm, w: u32, h: u32, gamepad: bool, quads: &mut Vec<HudDraw>) {
        if !self.open {
            return;
        }
        if vm.menu_main().is_none() {
            self.open = false;
            return;
        }
        self.draw_page(vm, w, h, gamepad, quads);
    }

    pub(crate) fn draw_page(&mut self, vm: &mut Vm, w: u32, h: u32, gamepad: bool, quads: &mut Vec<HudDraw>) {
        let (wf, hf) = (w as f32, h as f32);
        // fRatioX / fRatioY of the menus: 640 × 480 units stretched over the screen; the fonts
        // grow by whole steps.
        let (rx, ry) = (wf / 640.0, hf / 480.0);
        let s = (h / 480).max(1) as f32;
        // On the page (main.rs lays it out on Vm::menu_screen and places it with the menus).
        let cursor = vm.cursor_menu_page(w);
        let mouse = cursor.as_ref().map_or(self.mouse, |c| c.0);
        let moved = mouse != self.mouse;
        self.mouse = mouse;
        if self.click_taken && moved {
            self.moves(mouse);
        }
        let px = |x: f32, y: f32, w: f32, h: f32| [x * rx, y * ry, (x + w) * rx, (y + h) * ry];

        quads.push(Vm::rectangle_free(0.0, 0.0, wf, hf, PAGE));
        let [ox, oy, lw, lh, bw, bh] = MSGBOX;
        msgbox_background(quads, px(ox, oy, bw, bh), lw * rx, lh * ry, wf, hf);
        self.set.msgbox = px(ox, oy, bw, bh);

        // Title on its white tab, black text centred.
        let lang = vm.text_language().to_string();
        let tr = |t: Text| added_text::text(&lang, t);
        let title = tr(Text::ModsTitle);
        let (tw, th) = text(vm, FONT, 0.0, 0.0, s, title, BLACK, &mut Vec::new());
        let tab = [80.0 * rx, 110.0 * ry, 80.0 * rx + tw + 40.0 * rx, 150.0 * ry];
        bordered(quads, tab, WHITE);
        text(vm, FONT, tab[0] + 20.0 * rx, tab[1] + (tab[3] - tab[1] - th) / 2.0, s, title, BLACK, quads);

        // Rows: centre line at 75 + 40 k; arrow at 230, check box at 260, name at 300.
        let step = 40.0 * ry;
        self.set.rows = [225.0 * rx, 55.0 * ry, (ox + bw) * rx, step];
        self.set.row_h = step;
        let n = self.list.mods.len();
        let rows = self.rows();
        self.high = self.high.min(rows.saturating_sub(ROWS));
        if moved && self.press.is_none() {
            let under = self.row_at(mouse);
            if let Some(i) = under {
                self.hover_row(i);
            }
            self.mouse_row = under;
        }
        let right = (ox + bw - 10.0) * rx;
        // No mod besides the built-in entries: the message on the row below them.
        let builtin = self.list.builtin_count();
        let no_mods = match &self.list.folder {
            Some(f) if f.is_dir() => tr(Text::NoMods),
            _ => tr(Text::NoModsFolder),
        };
        if n == builtin && self.high == 0 {
            text(vm, FONT, 160.0 * rx, (75.0 + 40.0 * builtin as f32) * ry - th / 2.0, s, no_mods, BLACK, quads);
        }
        for k in 0..ROWS.min(rows.saturating_sub(self.high)) {
            let i = self.high + k;
            let yc = (75.0 + 40.0 * k as f32) * ry;
            let cell = |x: f32| [x * rx, yc - 10.0 * ry, (x + 20.0) * rx, yc + 10.0 * ry];
            if i == self.choice {
                let a = cell(230.0);
                quads.push(Vm::rectangle_free(a[0], a[1], a[2], a[3], WHITE));
                match vm.texture_free(ARROW_TEX, a[0], a[1], a[2], a[3], WHITE) {
                    Some(t) => quads.push(t),
                    None => {
                        text(vm, FONT, a[0] + 4.0 * rx, yc - th / 2.0, s, ">", BLACK, quads);
                    }
                }
            }
            let b = cell(260.0);
            bordered(quads, b, WHITE);
            let m = &self.list.mods[i];
            if m.enabled {
                match vm.texture_free(VALID_TEX, b[0], b[1], b[2], b[3], BLACK) {
                    Some(t) => quads.push(t),
                    None => {
                        text(vm, FONT, b[0] + 4.0 * rx, yc - th / 2.0, s, "X", BLACK, quads);
                    }
                }
            }
            let enabled = m.enabled;
            let conflicts = self.list.conflicts_of(i);
            let builtin = self.list.is_builtin(i);
            // A mod in conflict: a red "!" between its box and its name.
            if self.list.conflict_mark(i) {
                text(vm, FONT, 284.0 * rx, yc - th / 2.0, s, "!", RED, quads);
            }
            // The line under the name: "Restart needed" and the first conflict in red, else the
            // built-in tag in grey.
            let mut below: Vec<String> = Vec::new();
            if self.list.needs_restart(i) {
                below.push(tr(Text::RestartNeeded).to_string());
            }
            if let Some((other, what, _)) = conflicts.first() {
                let more = if conflicts.len() > 1 { fill(tr(Text::ConflictMore), &[&(conflicts.len() - 1).to_string()]) } else { String::new() };
                below.push(fill(tr(Text::ConflictShort), &[other, &added_text::what(&lang, what)]) + &more);
            }
            let (below, below_color) = match (below.is_empty(), builtin) {
                (false, _) => (below.join(" - "), RED),
                (true, true) => (tr(Text::IncludedWithGame).to_string(), GREY),
                (true, false) => (String::new(), GREY),
            };
            let name = fit(vm, FONT, s, &self.list.shown_name(i, &lang), right - 300.0 * rx);
            // With a line below it, the name moves up to leave room for it.
            let ny = if below.is_empty() { yc - th / 2.0 } else { yc - th + 2.0 * ry };
            text(vm, FONT, 300.0 * rx, ny, s, &name, if enabled { BLACK } else { GREY }, quads);
            if !below.is_empty() {
                let line = fit(vm, SMALL_FONT, s, &below, right - 300.0 * rx);
                text(vm, SMALL_FONT, 300.0 * rx, yc + 2.0 * ry, s, &line, below_color, quads);
            }
        }
        // "more above" / "more below": in the left column of the box, where no row text goes
        // (the arrow starts at 230, the names at 260 and 300).
        if self.high > 0 {
            let (x, y) = more_label_at(true, rx, ry);
            text(vm, SMALL_FONT, x, y, s, tr(Text::MoreAbove), GREY, quads);
        }
        if self.high + ROWS < rows {
            let (x, y) = more_label_at(false, rx, ry);
            text(vm, SMALL_FONT, x, y, s, tr(Text::MoreBelow), GREY, quads);
        }

        // Back tab, across the bottom edge of the box (taps and clicks; Esc / B too).
        // "Back" is the original game's word ([XIIIWindow] BackText of XIDInterf), in capitals.
        let word = vm.original_text("XIIIWindow", "BackText", "XIDInterf").unwrap_or_else(|| "Back".into()).to_uppercase();
        let back = back_label(&word, gamepad);
        let (tab, kh) = back_tab(vm, &back, s, rx, ry);
        self.set.back = tab;
        bordered(quads, tab, if inside(tab, mouse) { [225, 225, 225, 255] } else { WHITE });
        text(vm, FONT, tab[0] + 15.0 * rx, tab[1] + (tab[3] - tab[1] - kh) / 2.0, s, &back, BLACK, quads);

        // Text of the focused row at the bottom of the box, wrapped around the Back tab and made
        // smaller when it needs more room than the box has: no line is ever dropped.
        let paragraphs = self.bottom_paragraphs(&lang, gamepad);
        for l in bottom_layout(vm, s, &BottomArea::of(rx, ry, tab), &paragraphs) {
            text(vm, SMALL_FONT, l.x, l.y, l.scale, &l.text, l.color, quads);
        }
        if let Some((_, Some(tile))) = cursor {
            quads.push(tile);
        }
    }
}

/// UCanvas::DrawMsgboxBackground (the canvas.rs native) with `only_center` false:
/// the white box `b` outlined in black, and the white bands outlined in black around it (`lw`,
/// `lh` wide) out to the screen's edges, as the Skills page draws it.
fn msgbox_background(quads: &mut Vec<HudDraw>, b: [f32; 4], lw: f32, lh: f32, clip_x: f32, clip_y: f32) {
    let [ox, oy, x2, y2] = b;
    let (w, h) = (x2 - ox, y2 - oy);
    let rect = |q: &mut Vec<HudDraw>, x: f32, y: f32, xl: f32, yl: f32, c: [u8; 4]| q.push(Vm::rectangle_free(x, y, x + xl, y + yl, c));
    let low = clip_y * 0.95 - oy;
    rect(quads, 0.0, oy - lh - 2.0, clip_x, lh + 4.0, BLACK);
    rect(quads, w + ox - 2.0, oy, lw + 4.0, h, BLACK);
    rect(quads, ox, h + oy - 2.0, clip_x - ox, lh + 4.0, BLACK);
    rect(quads, ox - lw - 2.0, oy, lw + 4.0, low, BLACK);
    rect(quads, 0.0, oy - lh, clip_x, lh, WHITE);
    rect(quads, w + ox, oy, lw, h, WHITE);
    rect(quads, ox, h + oy, clip_x - ox, lh, WHITE);
    rect(quads, ox - lw, oy, lw, low, WHITE);
    rect(quads, ox, oy, w, h, WHITE);
    rect(quads, ox - 1.0, oy - 1.0, w + 2.0, 2.0, BLACK);
    rect(quads, ox - 1.0, oy - 1.0, 2.0, h + 2.0, BLACK);
    rect(quads, ox - 1.0, h + oy - 1.0, w + 2.0, 2.0, BLACK);
    rect(quads, w + ox - 1.0, oy - 1.0, 2.0, h + 2.0, BLACK);
}

/// Where "more above" (`above`) or "more below" is written, in screen pixels: the left column
/// of the box, beside the first and the last row shown.
fn more_label_at(above: bool, rx: f32, ry: f32) -> (f32, f32) {
    let y = if above { 62.0 } else { 62.0 + 40.0 * (ROWS - 1) as f32 };
    (130.0 * rx, y * ry)
}

/// Left side of the arrow of the focused row (230), the leftmost thing a row draws.
const ROW_LEFT: f32 = 230.0;

/// Label of the Back tab: the game's word and the key or button that does the same.
fn back_label(word: &str, gamepad: bool) -> String {
    if gamepad {
        format!("{word} (B)")
    } else if cfg!(target_os = "android") {
        word.to_string()
    } else {
        format!("{word} [Esc]")
    }
}

/// The Back tab across the bottom edge of the box, in screen pixels, and its text's height.
fn back_tab(vm: &mut Vm, label: &str, s: f32, rx: f32, ry: f32) -> ([f32; 4], f32) {
    let [_, oy, _, _, bw, bh] = MSGBOX;
    let ox = MSGBOX[0];
    let (kw, kh) = text(vm, FONT, 0.0, 0.0, s, label, BLACK, &mut Vec::new());
    ([(ox + bw - 10.0) * rx - kw - 30.0 * rx, (oy + bh - 15.0) * ry, (ox + bw - 10.0) * rx, (oy + bh + 15.0) * ry], kh)
}

/// Where the text of the focused row goes, in screen pixels: from under the list (342) to the
/// box's bottom edge, the whole width of the box above the Back tab and up to the tab beside it.
/// The menus are 640 × 480 units stretched over the window while the fonts only grow by whole
/// steps (one step up to 959 pixels high), so the number of lines that fit changes with the
/// window: the layout is made in pixels, with the text measured.
#[derive(Clone, Copy, Debug)]
struct BottomArea {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    tab: [f32; 4],
    /// Gap kept from the tab.
    gap: f32,
}

impl BottomArea {
    fn of(rx: f32, ry: f32, tab: [f32; 4]) -> BottomArea {
        let [ox, oy, _, _, bw, bh] = MSGBOX;
        BottomArea { left: 130.0 * rx, top: 342.0 * ry, right: (ox + bw - 10.0) * rx, bottom: (oy + bh) * ry - 3.0 * ry, tab, gap: 8.0 * rx }
    }

    /// Right end of a line from `y` to `y + h`: the box's, or the tab's left side less the gap
    /// when the line is beside the tab.
    fn right_at(&self, y: f32, h: f32) -> f32 {
        if y + h > self.tab[1] - 2.0 && y < self.tab[3] + 2.0 {
            self.right.min(self.tab[0] - self.gap)
        } else {
            self.right
        }
    }
}

/// One line of the text at the bottom of the box, in screen pixels.
#[derive(Clone, Debug)]
struct BottomLine {
    x: f32,
    y: f32,
    /// Width of the text drawn.
    width: f32,
    /// Width the line had room for.
    room: f32,
    height: f32,
    scale: f32,
    text: String,
    color: [u8; 4],
}

/// Sizes of the text tried, from the menus' own down: the first where every line fits.
const BOTTOM_SCALES: [f32; 5] = [1.0, 0.9, 0.8, 0.7, 0.6];

/// The paragraphs laid out in `area`: wrapped at spaces (a word wider than a line, such as a
/// file path, goes on over the next lines), at the largest size where all of them fit. Only a
/// text too long even at the smallest size (a mod's own description) ends with "...".
fn bottom_layout(vm: &mut Vm, s: f32, area: &BottomArea, paragraphs: &[(String, [u8; 4])]) -> Vec<BottomLine> {
    for (k, f) in BOTTOM_SCALES.iter().enumerate() {
        let last = k + 1 == BOTTOM_SCALES.len();
        if let Some(lines) = bottom_lines(vm, s * f, area, paragraphs, last) {
            return lines;
        }
    }
    Vec::new()
}

/// Height of a line of the small font at scale `scale`.
fn small_line_height(vm: &mut Vm, scale: f32) -> f32 {
    let (_, h) = text(vm, SMALL_FONT, 0.0, 0.0, scale, "Ag", BLACK, &mut Vec::new());
    h.max(6.0 * scale)
}

/// The paragraphs at scale `scale`; None if they do not all fit (unless `last`: then the last
/// line that fits ends with "...").
fn bottom_lines(vm: &mut Vm, scale: f32, area: &BottomArea, paragraphs: &[(String, [u8; 4])], last: bool) -> Option<Vec<BottomLine>> {
    let sh = small_line_height(vm, scale);
    let mut out: Vec<BottomLine> = Vec::new();
    let mut y = area.top;
    for (paragraph, color) in paragraphs {
        let mut words: std::collections::VecDeque<String> = paragraph.split_whitespace().map(String::from).collect();
        let mut line = String::new();
        loop {
            let width = area.right_at(y, sh) - area.left;
            let word = words.pop_front();
            if let Some(w) = &word {
                let candidate = if line.is_empty() { w.clone() } else { format!("{line} {w}") };
                if width_of(vm, SMALL_FONT, scale, &candidate) <= width {
                    line = candidate;
                    continue;
                }
                if line.is_empty() {
                    // A word wider than the line: as much of it as fits, the rest on the next.
                    let (head, rest) = split_word(vm, scale, w, width);
                    line = head;
                    words.push_front(rest);
                } else {
                    words.push_front(w.clone());
                }
            } else if line.is_empty() {
                break;
            }
            if y + sh > area.bottom {
                if !last {
                    return None;
                }
                if let Some(l) = out.last_mut() {
                    l.text = fit(vm, SMALL_FONT, scale, &format!("{} ...", l.text), l.room);
                    l.width = width_of(vm, SMALL_FONT, scale, &l.text);
                }
                return Some(out);
            }
            let text_width = width_of(vm, SMALL_FONT, scale, &line);
            out.push(BottomLine { x: area.left, y, width: text_width, room: width, height: sh, scale, text: std::mem::take(&mut line), color: *color });
            y += sh;
            if word.is_none() {
                break;
            }
        }
    }
    Some(out)
}

/// The longest start of `word` that fits in `width` (one character at least), and the rest.
fn split_word(vm: &mut Vm, scale: f32, word: &str, width: f32) -> (String, String) {
    let chars: Vec<char> = word.chars().collect();
    let mut n = 1;
    while n < chars.len() && width_of(vm, SMALL_FONT, scale, &chars[..n + 1].iter().collect::<String>()) <= width {
        n += 1;
    }
    (chars[..n].iter().collect(), chars[n..].iter().collect())
}

/// A rectangle of colour `c` with the menus' black border (bUseBorder).
fn bordered(quads: &mut Vec<HudDraw>, r: [f32; 4], c: [u8; 4]) {
    quads.push(Vm::rectangle_free(r[0] - 2.0, r[1] - 2.0, r[2] + 2.0, r[3] + 2.0, BLACK));
    quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], c));
}

/// Text in font `font` (the menu font if missing) at (x, y), scaled `s` times; its scaled width
/// and height.
#[allow(clippy::too_many_arguments)]
fn text(vm: &mut Vm, font: &str, x: f32, y: f32, s: f32, t: &str, color: [u8; 4], quads: &mut Vec<HudDraw>) -> (f32, f32) {
    let (mut cases, mut tw, mut th) = vm.scaled_text(font, x, y, s, t, color);
    if cases.is_empty() && font != FONT && !t.is_empty() {
        (cases, tw, th) = vm.scaled_text(FONT, x, y, s, t, color);
    }
    quads.extend(cases);
    (tw, th)
}

fn width_of(vm: &mut Vm, font: &str, s: f32, t: &str) -> f32 {
    text(vm, font, 0.0, 0.0, s, t, WHITE, &mut Vec::new()).0
}

/// `t` cut with "..." to fit in `width` pixels.
fn fit(vm: &mut Vm, font: &str, s: f32, t: &str, width: f32) -> String {
    if width_of(vm, font, s, t) <= width {
        return t.to_string();
    }
    let mut cut: String = t.to_string();
    while !cut.is_empty() {
        cut.pop();
        let c = format!("{}...", cut.trim_end());
        if width_of(vm, font, s, &c) <= width {
            return c;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_mod(root: &std::path::Path, folder: &str, name: &str, extra: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(openxiii_mods::MANIFEST), format!("[mod]\nname = \"{name}\"\nversion = \"1.0\"\n{extra}")).unwrap();
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oxiii-mods-page-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Turning a mod off and on rewrites the list the next launch reads; a folder already listed
    /// but gone is kept.
    #[test]
    fn toggle_writes_the_disabled_list() {
        let dir = temp("toggle");
        write_mod(&dir, "alpha", "Alpha", "");
        write_mod(&dir, "beta", "Beta", "");
        openxiii_mods::write_disabled(&dir, &["gone".into()]).unwrap();
        let mut l = ModList::read(Some(dir.clone()));
        assert_eq!(l.mods.iter().map(|m| (m.name.as_str(), m.enabled)).collect::<Vec<_>>(), [("Co-op", true), ("Alpha", true), ("Beta", true)]);
        l.toggle(2).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&dir), ["gone", "beta"]);
        assert_eq!(openxiii_mods::discover(&dir).mods.len(), 1);
        l.toggle(2).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&dir), ["gone"]);
        assert!(ModList::read(Some(dir.clone())).mods.iter().all(|m| m.enabled));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Conflicts follow the check boxes live: two checked mods setting the same value are shown
    /// on both rows with the other's name; unchecking one clears them.
    #[test]
    fn conflicts_follow_the_boxes() {
        let dir = temp("conflicts");
        write_mod(&dir, "a", "Fast Magnum", "[class.\"XIII.Magnum\"]\nReloadCount = 5\n");
        write_mod(&dir, "b", "Slow Magnum", "[class.\"XIII.Magnum\"]\nReloadCount = 9\n");
        write_mod(&dir, "c", "Other", "[class.\"XIII.Beretta\"]\nReloadCount = 2\n");
        let mut l = ModList::read(Some(dir.clone()));
        assert_eq!(l.conflicts_of(1), [("Slow Magnum", "value XIII.Magnum.ReloadCount", false)]);
        assert_eq!(l.conflicts_of(2), [("Fast Magnum", "value XIII.Magnum.ReloadCount", true)]);
        assert_eq!(l.details(1, "int")[2].0, "Conflicts with Slow Magnum on value XIII.Magnum.ReloadCount: Slow Magnum's value is used.");
        assert_eq!(l.details(2, "frt")[2].0, "Conflit avec Fast Magnum sur valeur XIII.Magnum.ReloadCount : la valeur de ce mod est utilisée.");
        assert!(l.conflicts_of(3).is_empty() && l.conflicts_of(0).is_empty());
        l.toggle(2).unwrap();
        assert!(l.conflicts.is_empty());
        assert!(l.conflicts_of(1).is_empty() && l.conflicts_of(2).is_empty());
        l.toggle(2).unwrap();
        assert_eq!(l.conflicts.len(), 1);
        // Read again with a turned off: listed after the mods turned on, no conflict until it is
        // checked.
        l.toggle(1).unwrap();
        let mut l = ModList::read(Some(dir.clone()));
        assert_eq!(l.mods[3].folder_name, "a");
        assert!(!l.mods[3].enabled && l.conflicts.is_empty());
        l.toggle(3).unwrap();
        assert_eq!(l.conflicts.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Co-op is the first row, checked by default, shipped with the game (no folder, no
    /// conflicts). Unchecking it writes its line in the list of mods turned off, which the next
    /// launch reads (coop_enabled); the mods' own lines are kept; checking it again removes it.
    /// Without a Mods folder, the folder is made for the line.
    #[test]
    fn coop_entry_is_saved_with_the_mods() {
        let dir = temp("coop");
        write_mod(&dir, "alpha", "Alpha", "");
        openxiii_mods::write_disabled(&dir, &["alpha".into()]).unwrap();
        assert!(coop_enabled(Some(&dir)) && coop_enabled(None));
        let mut l = ModList::read(Some(dir.clone()));
        assert!(l.is_builtin(0) && !l.is_builtin(1));
        assert!(l.mods[0].enabled && !l.mods[1].enabled);
        assert_eq!(l.shown_name(0, "det"), "Koop");
        assert_eq!(l.details(0, "frt")[1].0.split(". ").next(), Some("Fourni avec le jeu"));
        l.toggle(0).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&dir), [COOP_ENTRY, "alpha"]);
        assert!(!coop_enabled(Some(&dir)));
        // The mods do not see the line (no folder of that name), and reading again keeps it off.
        assert!(openxiii_mods::discover_all(&dir).mods.iter().all(|m| m.folder_name != COOP_ENTRY));
        let mut l = ModList::read(Some(dir.clone()));
        assert!(!l.mods[0].enabled && l.conflicts.is_empty());
        l.toggle(0).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&dir), ["alpha"]);
        assert!(coop_enabled(Some(&dir)));
        let _ = std::fs::remove_dir_all(&dir);
        // No Mods folder yet.
        let none = temp("coop-none");
        let mut l = ModList::read(Some(none.clone()));
        assert_eq!(l.mods.len(), 1);
        l.toggle(0).unwrap();
        assert!(!coop_enabled(Some(&none)));
        let _ = std::fs::remove_dir_all(&none);
    }

    /// The Xbox loading recaps are an example mod now (examples/mods/xbox-recaps): the line an
    /// earlier build wrote for its built-in row gives no row, turns no mod off, and goes when the
    /// list is written again (the mods' own lines and Co-op's are kept). Negative control: the
    /// list read before any writing still has it.
    #[test]
    fn old_recaps_line_is_ignored_and_dropped() {
        let dir = temp("recaps");
        write_mod(&dir, "alpha", "Alpha", "");
        openxiii_mods::write_disabled(&dir, &[COOP_ENTRY.into(), "*xbox-recaps=on".into()]).unwrap();
        let mut l = ModList::read(Some(dir.clone()));
        assert_eq!(l.mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), [COOP_ENTRY, "alpha"]);
        assert_eq!(openxiii_mods::discover(&dir).mods.len(), 1, "the line turns no mod off");
        assert_eq!(openxiii_mods::read_disabled(&dir), [COOP_ENTRY, "*xbox-recaps=on"]);
        l.toggle(1).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&dir), [COOP_ENTRY, "alpha"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Below Co-op: the mods turned on, then those turned off, each group by
    /// folder name with case ignored; a row keeps its place until the page is read again. The
    /// load order (folder names, as the game loads them) is unchanged, and the conflicts follow
    /// it. Negative control: that load order is not the order shown.
    #[test]
    fn mods_on_first_then_off_by_folder_name() {
        let dir = temp("order");
        let same = "[class.\"XIII.Magnum\"]\nReloadCount = 5\n";
        write_mod(&dir, "Zeta", "Zeta", same);
        write_mod(&dir, "alpha", "Alpha", "");
        write_mod(&dir, "beta", "Beta", same);
        write_mod(&dir, "Gamma", "Gamma", "");
        openxiii_mods::write_disabled(&dir, &["alpha".into(), "Gamma".into()]).unwrap();
        let shown = |l: &ModList| l.mods.iter().map(|m| m.folder_name.clone()).collect::<Vec<_>>();
        let mut l = ModList::read(Some(dir.clone()));
        assert_eq!(shown(&l), [COOP_ENTRY, "beta", "Zeta", "alpha", "Gamma"]);
        // Load order: folder names as they sort, "Zeta" before "beta"; beta, loaded later, wins.
        let loaded: Vec<String> = openxiii_mods::discover_all(&dir).mods.iter().map(|m| m.folder_name.clone()).collect();
        assert_eq!(loaded, ["Gamma", "Zeta", "alpha", "beta"]);
        assert_ne!(loaded, shown(&l)[1..], "negative control: the load order is not the order shown");
        assert_eq!(l.conflicts_of(1), [("Zeta", "value XIII.Magnum.ReloadCount", true)]);
        assert_eq!(l.conflicts_of(2), [("Beta", "value XIII.Magnum.ReloadCount", false)]);
        // Toggled: the row stays where it is while the page is open...
        l.toggle(3).unwrap();
        l.toggle(1).unwrap();
        assert_eq!(shown(&l), [COOP_ENTRY, "beta", "Zeta", "alpha", "Gamma"]);
        assert!(l.mods[3].enabled && !l.mods[1].enabled);
        // ...and the next reading puts it in its group.
        let l = ModList::read(Some(dir.clone()));
        assert_eq!(shown(&l), [COOP_ENTRY, "alpha", "Zeta", "beta", "Gamma"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The flamethrower is an example mod like the others: in Mods\_Examples it is neither
    /// listed nor loaded, and the line an earlier build wrote for its built-in row is ignored
    /// (no error, no row) and dropped when the list is written again; copied into the Mods
    /// folder, it is listed and loaded at its folder name's place among the mods.
    #[test]
    fn flamethrower_is_an_example_like_the_others() {
        let mods = temp("flamethrower");
        write_mod(&mods, "alpha", "Alpha", "");
        write_mod(&mods.join(openxiii_mods::EXAMPLES_DIR), "flamethrower", "Flamethrower", "");
        openxiii_mods::write_disabled(&mods, &[OLD_BUILTIN_LINES[0].into()]).unwrap();
        let names = |d: &openxiii_mods::Discovery| d.mods.iter().map(|m| m.folder_name.clone()).collect::<Vec<_>>();
        let d = openxiii_mods::discover(&mods);
        assert_eq!(names(&d), ["alpha"]);
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        let mut l = ModList::read(Some(mods.clone()));
        assert_eq!(l.mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), [COOP_ENTRY, "alpha"]);
        l.toggle(1).unwrap();
        assert_eq!(openxiii_mods::read_disabled(&mods), ["alpha"], "the old line goes");
        l.toggle(1).unwrap();
        // Copied into the Mods folder: a mod as any other.
        write_mod(&mods, "flamethrower", "Flamethrower", "");
        assert_eq!(names(&openxiii_mods::discover(&mods)), ["alpha", "flamethrower"]);
        let l = ModList::read(Some(mods.clone()));
        assert_eq!(l.mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), [COOP_ENTRY, "alpha", "flamethrower"]);
        assert!(!l.is_builtin(2) && l.mods[2].enabled);
        let _ = std::fs::remove_dir_all(&mods);
    }

    /// Restart needed: a row whose box differs from what the game read at startup, toggled here
    /// and not set back, or a mod copied into Mods since; Co-op too. Negative control: without a
    /// startup state (mods not read then), no row asks for it.
    #[test]
    fn restart_needed_follows_the_startup_state() {
        let dir = temp("restart");
        write_mod(&dir, "alpha", "Alpha", "");
        write_mod(&dir, "copied", "Copied later", "");
        let start = AtStart { mods: vec!["alpha".into()], coop: true };
        let needs = |l: &ModList| l.mods.iter().enumerate().filter(|(i, _)| l.needs_restart(*i)).map(|(_, m)| m.folder_name.clone()).collect::<Vec<_>>();
        let mut l = ModList::read_with(Some(dir.clone()), Some(start.clone()));
        assert_eq!(l.mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), [COOP_ENTRY, "alpha", "copied"]);
        assert_eq!(needs(&l), ["copied"], "copied while the game runs: shown checked, not loaded");
        l.toggle(1).unwrap();
        assert_eq!(needs(&l), ["alpha", "copied"], "turned off here: still loaded");
        l.toggle(1).unwrap();
        assert_eq!(needs(&l), ["copied"], "set back: nothing to apply");
        l.toggle(0).unwrap();
        assert_eq!(needs(&l), [COOP_ENTRY, "copied"], "Co-op too");
        assert_eq!(l.details(0, "frt").last().map(|d| (d.0.as_str(), d.1)), Some(("Ce changement s'appliquera au prochain lancement du jeu.", RED)));
        let l = ModList::read_with(Some(dir.clone()), None);
        assert!(needs(&l).is_empty(), "negative control: no startup state, no restart asked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two checked mods in conflict: the "!" on both rows; on neither once one is unchecked, nor
    /// on the third mod. Negative control: the rows of mods that share nothing never have it.
    #[test]
    fn conflict_mark_on_both_rows() {
        let dir = temp("mark");
        write_mod(&dir, "a", "Fast Magnum", "[class.\"XIII.Magnum\"]\nReloadCount = 5\n");
        write_mod(&dir, "b", "Slow Magnum", "[class.\"XIII.Magnum\"]\nReloadCount = 9\n");
        write_mod(&dir, "c", "Other", "[class.\"XIII.Beretta\"]\nReloadCount = 2\n");
        let mut l = ModList::read(Some(dir.clone()));
        let marks = |l: &ModList| (0..l.mods.len()).filter(|&i| l.conflict_mark(i)).collect::<Vec<_>>();
        assert_eq!(marks(&l), [1, 2]);
        l.toggle(2).unwrap();
        assert!(marks(&l).is_empty());
        assert!(!l.conflict_mark(3) && !l.conflict_mark(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn page(n: usize) -> ModsPage {
        let mods = (0..n).map(|i| openxiii_mods::Listing { folder_name: format!("m{i}"), name: format!("M{i}"), version: "1".into(), enabled: true }).collect();
        ModsPage { open: true, list: ModList { mods, ..Default::default() }, ..Default::default() }
    }

    /// The bottom text starts with the full name, even when the list cuts it.
    #[test]
    fn details_start_with_the_full_name() {
        let mut p = page(2);
        let long = "A Very Long Mod Name That Does Not Fit In The List Row At All";
        p.list.mods[1].name = long.into();
        let d = p.list.details(1, "int");
        assert_eq!(d[0], (long.to_string(), BLACK));
        assert_eq!(d[1], ("Version 1.".to_string(), BLACK));
        assert_eq!(d.len(), 2);
        assert!(p.list.details(5, "int").is_empty());
        assert_eq!(p.list.details(1, "frt")[1].0, "Version 1.");
        assert_eq!(p.list.details(1, "itt")[1].0, "Versione 1.");
    }

    /// Keys move within the list (wrapping as on the Skills page) and Back closes.
    #[test]
    fn keys() {
        let mut p = page(3);
        p.key(Key::Down);
        p.key(Key::Down);
        assert_eq!(p.choice, 2);
        p.key(Key::Toggle);
        assert!(!p.list.mods[2].enabled);
        p.key(Key::Down);
        assert_eq!(p.choice, 0);
        p.key(Key::Up);
        assert_eq!(p.choice, p.rows() - 1);
        p.key(Key::Back);
        assert!(!p.open);
    }

    /// Every focus move plays the menus' tick, the wraps from the last row to the first and back
    /// included; Enter and Back move nothing and tick nothing. Negative control: a key on a list
    /// of one row moves nothing and ticks nothing.
    #[test]
    fn focus_moves_tick() {
        let mut p = page(3);
        let n = p.rows();
        for _ in 0..n {
            p.key(Key::Down);
        }
        assert_eq!((p.choice, p.take_ticks()), (0, n as u32), "down through the list and round to the top");
        p.key(Key::Up);
        assert_eq!((p.choice, p.take_ticks()), (n - 1, 1), "up from the top: the last row, one tick");
        p.key(Key::Toggle);
        p.key(Key::Back);
        assert_eq!(p.take_ticks(), 0, "Enter and Back tick nothing");
        let mut one = page(1);
        assert_eq!(one.rows(), 1);
        one.key(Key::Down);
        one.key(Key::Up);
        assert_eq!(one.take_ticks(), 0, "a single row: no move, no tick");
    }

    /// The mouse ticks on entering another row, and on coming back onto the focused row after
    /// being over no row; moving within the row ticks nothing. Negative control: without the
    /// mouse leaving (mouse_row kept), the same row ticks nothing.
    #[test]
    fn mouse_back_onto_the_focused_row_ticks() {
        let mut p = page(3);
        let mut at = |p: &mut ModsPage, row: Option<usize>| {
            if let Some(i) = row {
                p.hover_row(i);
            }
            p.mouse_row = row;
            p.take_ticks()
        };
        assert_eq!(at(&mut p, Some(1)), 1, "onto another row");
        assert_eq!(at(&mut p, Some(1)), 0, "within the same row");
        assert_eq!(at(&mut p, None), 0, "over no row");
        assert_eq!(at(&mut p, Some(1)), 1, "back onto the focused row");
        assert_eq!(p.choice, 1);
    }

    /// The focused row stays on screen with more mods than rows.
    #[test]
    fn long_list_scrolls() {
        let mut p = page(12);
        for _ in 0..9 {
            p.key(Key::Down);
        }
        assert_eq!((p.choice, p.high), (9, 9 + 1 - ROWS));
        for _ in 9..p.rows() {
            p.key(Key::Down);
        }
        assert_eq!((p.choice, p.high), (0, 0));
    }

    /// Taps: on a row it toggles that mod, on Back it closes, elsewhere nothing (the page covers
    /// the whole screen); a drag scrolls without toggling.
    #[test]
    fn taps_and_drags() {
        let mut p = page(12);
        p.set = Set { back: [400.0, 420.0, 500.0, 460.0], msgbox: [120.0, 50.0, 540.0, 440.0], rows: [225.0, 55.0, 540.0, 40.0], row_h: 40.0 };
        assert!(p.touch(1, Finger::Pose, (320.0, 120.0)));
        assert!(p.touch(1, Finger::Raised, (320.0, 120.0)));
        assert_eq!(p.choice, 1);
        assert!(!p.list.mods[1].enabled);
        p.touch(2, Finger::Pose, (320.0, 300.0));
        p.touch(2, Finger::Moves, (320.0, 200.0));
        p.touch(2, Finger::Raised, (320.0, 200.0));
        assert_eq!(p.high, 3);
        assert!(p.list.mods.iter().filter(|m| !m.enabled).count() == 1);
        p.touch(3, Finger::Pose, (450.0, 440.0));
        p.touch(3, Finger::Raised, (450.0, 440.0));
        assert!(!p.open);
        let mut p = page(2);
        p.set = Set { back: [400.0, 420.0, 500.0, 460.0], msgbox: [120.0, 50.0, 540.0, 440.0], rows: [225.0, 55.0, 540.0, 40.0], row_h: 40.0 };
        p.touch(1, Finger::Pose, (20.0, 20.0));
        p.touch(1, Finger::Raised, (20.0, 20.0));
        assert!(p.open, "outside the box: the page stays open");
        assert!(p.list.mods.iter().all(|m| m.enabled));
    }

    /// What the layout test checks: every character of the paragraphs drawn (no line dropped,
    /// nothing cut), each line inside the area, and none over the Back tab.
    fn check_bottom(paragraphs: &[(String, [u8; 4])], lines: &[BottomLine], area: &BottomArea) -> Result<(), String> {
        let chars = |t: &mut dyn Iterator<Item = &str>| t.flat_map(|x| x.chars()).filter(|c| !c.is_whitespace()).collect::<String>();
        let wanted = chars(&mut paragraphs.iter().map(|(t, _)| t.as_str()));
        let drawn = chars(&mut lines.iter().map(|l| l.text.as_str()));
        if wanted != drawn {
            return Err(format!("text lost: wanted {wanted:?}, drawn {drawn:?}"));
        }
        let t = area.tab;
        for l in lines {
            let r = [l.x, l.y, l.x + l.width, l.y + l.height];
            if r[0] < area.left - 0.5 || r[1] < area.top - 0.5 || r[3] > area.bottom + 0.5 || r[2] > area.right + 0.5 {
                return Err(format!("line {:?} out of the box {area:?}", l.text));
            }
            if r[0] < t[2] && r[2] > t[0] - 2.0 && r[1] < t[3] && r[3] > t[1] - 2.0 {
                return Err(format!("line {:?} {r:?} over the Back tab {t:?}", l.text));
            }
        }
        Ok(())
    }

    /// Every row's text at the bottom of the box (Co-op, every example mod with its
    /// conflicts, an error), in the 5 languages, with the keyboard
    /// and the gamepad, at 854 × 480, 800 × 600, 1024 × 576 and 1280 × 720: all of it drawn, inside
    /// the box, clear of the Back tab. Negative controls: a dropped line, a line moved onto the
    /// tab and a description too long for any size are each caught.
    #[test]
    fn bottom_text_fits_at_every_size() {
        let Ok(game) = openxiii_package::env::var("OXIII_GAME") else {
            eprintln!("skipped: needs the game's files (OXIII_GAME)");
            return;
        };
        let mut vm = Vm::new();
        vm.load_dir(&std::path::Path::new(&game).join("System")).unwrap();
        assert!(width_of(&mut vm, SMALL_FONT, 1.0, "Mods") > 0.0, "the game's fonts are read");
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods");
        // Nothing loaded at startup, Co-op off then: every row says a restart is needed, the
        // longest texts.
        let start = AtStart { mods: Vec::new(), coop: false };
        let mut p = ModsPage { open: true, list: ModList::read_with(Some(examples), Some(start)), ..Default::default() };
        assert!(p.list.mods.len() >= 10, "the example mods are listed");
        assert!((0..p.list.mods.len()).all(|i| p.list.needs_restart(i)));
        // The original game's word for Back in each language, in capitals.
        let backs = ["BACK", "RETOUR", "ZURÜCK", "INDIETRO", "ATRÁS"];
        let (mut cases, mut smallest) = (0, 1.0f32);
        for (li, lang) in openxiii_vm::added_text::LANGUAGES.iter().enumerate() {
            for (w, h) in [(854u32, 480u32), (800, 600), (1024, 576), (1280, 720)] {
                let (rx, ry, s) = (w as f32 / 640.0, h as f32 / 480.0, (h / 480).max(1) as f32);
                for gamepad in [false, true] {
                    let (tab, _) = back_tab(&mut vm, &back_label(backs[li], gamepad), s, rx, ry);
                    let area = BottomArea::of(rx, ry, tab);
                    let mut sets = Vec::new();
                    for choice in 0..p.rows() {
                        p.choice = choice;
                        sets.push(p.bottom_paragraphs(lang, gamepad));
                    }
                    p.choice = 1;
                    p.error = Some("Access is denied. (os error 5)".into());
                    sets.push(p.bottom_paragraphs(lang, gamepad));
                    p.error = None;
                    for paragraphs in &sets {
                        let lines = bottom_layout(&mut vm, s, &area, paragraphs);
                        if let Err(e) = check_bottom(paragraphs, &lines, &area) {
                            panic!("{lang} {w}x{h} gamepad {gamepad}: {e}\n{lines:#?}");
                        }
                        smallest = smallest.min(lines.first().map_or(s, |l| l.scale) / s);
                        cases += 1;
                    }
                }
            }
        }
        // "more above" / "more below": left of every row's drawing (arrow, box, name), inside the
        // box, in every language and size. Negative control: at x = 470 (where they were) they
        // reach the names' column.
        for lang in openxiii_vm::added_text::LANGUAGES {
            for (w, h) in [(854u32, 480u32), (800, 600), (1024, 576), (1280, 720)] {
                let (rx, ry, s) = (w as f32 / 640.0, h as f32 / 480.0, (h / 480).max(1) as f32);
                for (above, t) in [(true, Text::MoreAbove), (false, Text::MoreBelow)] {
                    let label = added_text::text(lang, t);
                    let (x, _) = more_label_at(above, rx, ry);
                    let lw = width_of(&mut vm, SMALL_FONT, s, label);
                    assert!(x >= MSGBOX[0] * rx && x + lw <= ROW_LEFT * rx - 2.0, "{lang} {w}x{h}: {label:?} from {x} to {} reaches the rows (from {})", x + lw, ROW_LEFT * rx);
                    // The names' column: from 300 to the box's right side less 10.
                    let names = [300.0 * rx, (MSGBOX[0] + MSGBOX[4] - 10.0) * rx];
                    assert!(470.0 * rx < names[1] && 470.0 * rx + lw > names[0], "negative control: the old place is in the names' column");
                    assert!(!(x < names[1] && x + lw > names[0]), "{lang} {w}x{h}: {label:?} in the names' column");
                }
            }
        }
        eprintln!("{cases} texts laid out; smallest size used: {smallest:.1} of the menus' own");
        // At least: every language, size and input, with the 9 example mods, Co-op and the text
        // after an error.
        assert!(cases >= openxiii_vm::added_text::LANGUAGES.len() * 4 * 2 * (9 + 1 + 1), "{cases} texts laid out");
        // Readable everywhere: never under 0.7 of the menus' own size (the longest is an example mod
        // with its conflict lines at 854 × 480).
        assert!(smallest >= 0.7 - 1e-3, "a text needs a size under 0.7: shorten it");

        // Negative controls, at 854 × 480 in French on the Co-op row.
        let (rx, ry) = (854.0 / 640.0, 1.0);
        let (tab, _) = back_tab(&mut vm, &back_label("RETOUR", false), 1.0, rx, ry);
        let area = BottomArea::of(rx, ry, tab);
        p.choice = 0;
        let paragraphs = p.bottom_paragraphs("frt", false);
        let lines = bottom_layout(&mut vm, 1.0, &area, &paragraphs);
        assert!(check_bottom(&paragraphs, &lines, &area).is_ok());
        let mut dropped = lines.clone();
        dropped.pop();
        assert!(check_bottom(&paragraphs, &dropped, &area).is_err(), "a dropped line is caught");
        let mut onto_tab = lines.clone();
        let l = onto_tab.last_mut().unwrap();
        (l.x, l.y) = (tab[0] - l.width / 2.0, tab[1]);
        assert!(check_bottom(&paragraphs, &onto_tab, &area).is_err(), "a line over the tab is caught");
        let long = vec![("word ".repeat(400), BLACK)];
        let lines = bottom_layout(&mut vm, 1.0, &area, &long);
        assert!(lines.last().is_some_and(|l| l.text.ends_with("...")), "a text too long ends with ...");
        assert!(check_bottom(&long, &lines, &area).is_err(), "a cut text is caught");
    }
}
