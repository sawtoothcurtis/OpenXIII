//! Improved build only: the Select level page, a real menu page of the game (an
//! XIIIWindowMainMenu opened over the Load page, or over the main menu with L / gamepad Y).
//!
//! It is laid out with the Load page's own rectangles (XIIIMenuLoadGameWindow.Paint): the
//! picture in the left panel (36, 33, 348 x 393 units, black border), the title in the game's
//! label style (white box, black border, 0, 40, 170 x 40), and the rows are XIIIButtons like the
//! save slots (50, width 320, glass look: half-transparent white, opaque with a black border when
//! focused). The help bar is the page's own (DisplayHelpBar: "Escape Back ... Select Enter" in
//! the player's language). The picture is the Cine14 frame and Jones stands on its right
//! (select_level_art.rs).
//!
//! Levels: one flat list, every mission start and every checkpoint as its own row, all at the
//! same inset, in story order. Up / Down, Page Up / Down, Home / End, the wheel and the mouse move
//! the focus (a scroll bar shows the place in the list); Enter (gamepad A) or a click on a row
//! opens the equipment page of that start; Escape closes the page. Names are the save
//! descriptions the game writes at each start
//! (XIIISaveGameTrigger.SaveDescription, which the game does not localise: its `localized`
//! keyword is commented out, so the Load page shows them in English in every language).
//!
//! Equipment: the same page with the test weapons as a grid of the HUD's own weapon icons
//! (XIIIMenu.HUD.Weapon1Icons), grey when left out and in colour when chosen, like the main
//! menu's boxes; the focused one's name in a label (the chosen start's otherwise); All / None, then Start level. Arrows, mouse,
//! Enter (gamepad A) toggle; Escape goes back to the list on the same row.
//!
//! The navigation (`Nav`) knows nothing of the VM; the start itself is handed to the game window
//! (`take_select_level_start`), which loads it as before (Vm::from_menu_main) between frames.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::menu_boxes::{crop_to_aspect, Crop, Image};
use crate::added_text::{text, Text};
use crate::select_level::{starts_list, WEAPONS, WEAPONS_ALL, WEAPONS_DEFAULT};
use crate::select_level_art;
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// One mission of the list: its start's description and URL, then its checkpoints (description,
/// URL), in the order of select_level.tsv (campaign order).
#[derive(Clone, Debug, PartialEq)]
pub struct Mission {
    pub name: String,
    pub url: String,
    pub checkpoints: Vec<(String, String)>,
}

/// Missions from select_level.tsv: the start of each map ("PlayerStart") then its checkpoints.
pub fn missions() -> Vec<Mission> {
    let mut out: Vec<Mission> = Vec::new();
    for s in starts_list() {
        let name = s.description.trim().to_string();
        if s.portal_value.eq_ignore_ascii_case("PlayerStart") {
            out.push(Mission { name, url: s.map_value.clone(), checkpoints: Vec::new() });
        } else if let Some(m) = out.last_mut().filter(|m| m.url.eq_ignore_ascii_case(&s.map_value)) {
            m.checkpoints.push((name, format!("{}#{}", s.map_value, s.portal_value)));
        }
    }
    out
}

//// A row of the levels page: a mission start or a checkpoint, named as the game names it in its
/// saves, and its URL ("map" or "map#portal").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Start {
    pub name: String,
    pub url: String,
}

/// Every mission start and every checkpoint, one row each, in story order (each mission's start
/// followed by its checkpoints).
pub fn starts() -> Vec<Start> {
    starts_of(&missions())
}

fn starts_of(missions: &[Mission]) -> Vec<Start> {
    let mut out = Vec::new();
    for m in missions {
        out.push(Start { name: m.name.clone(), url: m.url.clone() });
        out.extend(m.checkpoints.iter().map(|(name, url)| Start { name: name.clone(), url: url.clone() }));
    }
    out
}

/// What has the focus on the equipment page: a weapon of the grid, All, None, Start level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gear {
    Cell(usize),
    All,
    None,
    Start,
}

/// Columns of the equipment grid.
pub const GRID_COLUMNS: usize = 5;

/// First grid column under None (All is under the columns before it).
const COLUMN_OF_NONE: usize = GRID_COLUMNS / 2 + 1;

/// The lowest weapon of grid column `c` (the last row may be shorter).
fn lowest_in_column(c: usize, n: usize) -> usize {
    let c = c.min(GRID_COLUMNS - 1).min(n.saturating_sub(1));
    c + (n - 1 - c) / GRID_COLUMNS * GRID_COLUMNS
}

/// One arrow on the equipment page from `g` (see Nav::move_gear); `column` is the grid column kept
/// through All / None and Start level, updated on the grid.
fn gear_step(g: Gear, k: NavKey, column: &mut usize) -> Gear {
    let n = WEAPONS.len();
    let row_len = |row: usize| (n - row * GRID_COLUMNS).min(GRID_COLUMNS);
    // All / None of a column, and a column kept for a button.
    let button = |c: usize| if c < COLUMN_OF_NONE { Gear::All } else { Gear::None };
    let column_for = |b: Gear, c: usize| match b {
        Gear::All if c >= COLUMN_OF_NONE => 0,
        Gear::None if c < COLUMN_OF_NONE => COLUMN_OF_NONE,
        _ => c,
    };
    let next = match (g, k) {
        (Gear::Cell(i), NavKey::Left | NavKey::Right) => {
            let (row, col) = (i / GRID_COLUMNS, i % GRID_COLUMNS);
            let len = row_len(row);
            let col = if k == NavKey::Right { (col + 1) % len } else { (col + len - 1) % len };
            Gear::Cell(row * GRID_COLUMNS + col)
        }
        (Gear::Cell(i), NavKey::Up) if i >= GRID_COLUMNS => Gear::Cell(i - GRID_COLUMNS),
        // Up from the top row: the bottom-most control, Start level.
        (Gear::Cell(_), NavKey::Up) => Gear::Start,
        (Gear::Cell(i), NavKey::Down) if i + GRID_COLUMNS < n => Gear::Cell(i + GRID_COLUMNS),
        // Down from the lowest weapon of a column: All or None below that column.
        (Gear::Cell(i), NavKey::Down) => button(i % GRID_COLUMNS),
        (Gear::All, NavKey::Left | NavKey::Right) => Gear::None,
        (Gear::None, NavKey::Left | NavKey::Right) => Gear::All,
        (Gear::All | Gear::None, NavKey::Down) => Gear::Start,
        (b @ (Gear::All | Gear::None), NavKey::Up) => Gear::Cell(lowest_in_column(column_for(b, *column), n)),
        (Gear::Start, NavKey::Up) => button(*column),
        // Down from Start level: the grid's top row, in the kept column.
        (Gear::Start, NavKey::Down) => Gear::Cell((*column).min(n - 1)),
        (Gear::Start, NavKey::Left | NavKey::Right) => Gear::Start,
        (_, NavKey::Home) | (_, NavKey::PageUp) => Gear::Cell(0),
        (_, NavKey::End) | (_, NavKey::PageDown) => Gear::Start,
        (g, _) => g,
    };
    match (g, next) {
        (_, Gear::Cell(i)) => *column = i % GRID_COLUMNS,
        // Left / Right between All and None: the column follows the button.
        (Gear::All | Gear::None, b @ (Gear::All | Gear::None)) if g != b => *column = column_for(b, *column),
        _ => {}
    }
    next
}

/// Input of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    Enter,
    Back,
    /// Wheel steps (positive: down).
    Wheel(i32),
    /// The mouse moved the focus to this visible row.
    Hover(usize),
    /// Click on this visible row.
    Click(usize),
    /// The mouse over, or a click on, an element of the equipment page.
    HoverGear(Gear),
    ClickGear(Gear),
    /// The mouse moved over no element of the equipment page.
    HoverNone,
}

/// What an input asks of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Start(String),
    Close,
    WeaponsChanged(u32),
}

/// Navigation of the page: the levels list (focused row, first visible row), then the equipment
/// page of the chosen start.
#[derive(Clone, Debug)]
pub struct Nav {
    pub starts: Vec<Start>,
    pub focus: usize,
    pub top: usize,
    pub visible: usize,
    /// Equipment page open for this start (URL, description).
    pub chosen: Option<(String, String)>,
    pub gear: Gear,
    /// The focused element is shown (its frame, the weapon's name): false once the mouse that
    /// placed it moved over nothing, true again with the mouse over an element or a key.
    pub gear_shown: bool,
    /// Column of the grid the arrows keep while they go through All / None and Start level.
    pub gear_column: usize,
    pub weapons: u32,
}

impl Nav {
    pub fn new(starts: Vec<Start>, visible: usize) -> Nav {
        Nav { starts, focus: 0, top: 0, visible: visible.max(1), chosen: None, gear: Gear::Start, gear_shown: true, gear_column: 0, weapons: WEAPONS_DEFAULT }
    }

    /// Rows `top`..`top + visible`.
    pub fn visible_rows(&self) -> &[Start] {
        let end = (self.top + self.visible).min(self.starts.len());
        &self.starts[self.top.min(end)..end]
    }

    pub fn focused(&self) -> Option<&Start> {
        self.starts.get(self.focus)
    }

    fn len(&self) -> usize {
        self.starts.len()
    }

    /// The focus stays visible.
    fn show_focus(&mut self) {
        let n = self.len();
        self.focus = self.focus.min(n.saturating_sub(1));
        if self.focus < self.top {
            self.top = self.focus;
        } else if self.focus >= self.top + self.visible {
            self.top = self.focus + 1 - self.visible;
        }
        self.top = self.top.min(n.saturating_sub(self.visible));
    }

    /// The equipment page of row `i`, its Start level button focused.
    fn choose(&mut self, i: usize) {
        if let Some(s) = self.starts.get(i) {
            self.chosen = Some((s.url.clone(), s.name.clone()));
            self.gear = Gear::Start;
            self.gear_shown = true;
        }
    }

    /// Enter or a click on the equipment page.
    fn activate_gear(&mut self, g: Gear) -> Outcome {
        self.gear = g;
        match g {
            Gear::Cell(i) => self.weapons ^= 1 << i,
            Gear::All => self.weapons = WEAPONS_ALL,
            Gear::None => self.weapons = 0,
            Gear::Start => return self.chosen.as_ref().map_or(Outcome::None, |c| Outcome::Start(c.0.clone())),
        }
        Outcome::WeaponsChanged(self.weapons)
    }

    /// Arrows on the equipment page (improved build: every way round, like an on-screen keyboard):
    /// the grid, then All / None below it, then Start level, then the grid's top row again.
    /// Left / Right go round a row (All / None between themselves); Up / Down go round a column,
    /// whose place is kept through All / None and Start level.
    fn move_gear(&mut self, k: NavKey) {
        self.gear = gear_step(self.gear, k, &mut self.gear_column);
    }

    pub fn key(&mut self, k: NavKey) -> Outcome {
        if self.chosen.is_some() {
            return match k {
                NavKey::Back => {
                    self.chosen = None;
                    Outcome::None
                }
                // A hidden focus is shown first: nothing is toggled that the player cannot see.
                NavKey::Enter if !self.gear_shown => {
                    self.gear_shown = true;
                    Outcome::None
                }
                NavKey::Enter => self.activate_gear(self.gear),
                NavKey::ClickGear(g) => {
                    self.gear_shown = true;
                    self.activate_gear(g)
                }
                NavKey::HoverGear(g) => {
                    self.gear = g;
                    if let Gear::Cell(i) = g {
                        self.gear_column = i % GRID_COLUMNS;
                    }
                    self.gear_shown = true;
                    Outcome::None
                }
                NavKey::HoverNone => {
                    self.gear_shown = false;
                    Outcome::None
                }
                NavKey::Up | NavKey::Down | NavKey::Left | NavKey::Right | NavKey::Home | NavKey::End | NavKey::PageUp | NavKey::PageDown => {
                    // The first key shows the focus where it is, the next ones move it.
                    if self.gear_shown {
                        self.move_gear(k);
                    }
                    self.gear_shown = true;
                    Outcome::None
                }
                _ => Outcome::None,
            };
        }
        let n = self.len();
        let last = n.saturating_sub(1);
        match k {
            // Past the first row: the last one; past the last row: the first one.
            NavKey::Up => self.focus = if self.focus == 0 { last } else { self.focus - 1 },
            NavKey::Down => self.focus = if self.focus >= last { 0 } else { self.focus + 1 },
            NavKey::PageUp => self.focus = self.focus.saturating_sub(self.visible),
            NavKey::PageDown => self.focus = (self.focus + self.visible).min(last),
            NavKey::Home => self.focus = 0,
            NavKey::End => self.focus = last,
            NavKey::Wheel(steps) => {
                let max_top = n.saturating_sub(self.visible) as i64;
                // The wheel at the top of the list goes on to its bottom, and the other way round.
                if steps < 0 && self.top == 0 && n > 0 {
                    self.top = max_top as usize;
                    self.focus = last;
                    return Outcome::None;
                }
                if steps > 0 && self.top as i64 >= max_top && n > 0 {
                    self.top = 0;
                    self.focus = 0;
                    return Outcome::None;
                }
                self.top = (self.top as i64 + steps as i64).clamp(0, max_top) as usize;
                self.focus = self.focus.clamp(self.top, (self.top + self.visible).min(n).saturating_sub(1));
                return Outcome::None;
            }
            NavKey::Hover(v) => {
                if v < self.visible && self.top + v < n {
                    self.focus = self.top + v;
                }
                return Outcome::None;
            }
            NavKey::Click(v) => {
                if v < self.visible && self.top + v < n {
                    self.focus = self.top + v;
                    self.choose(self.focus);
                }
                return Outcome::None;
            }
            NavKey::Enter => {
                self.choose(self.focus);
                return Outcome::None;
            }
            NavKey::Back => return Outcome::Close,
            NavKey::Left | NavKey::Right | NavKey::HoverGear(_) | NavKey::ClickGear(_) | NavKey::HoverNone => {}
        }
        self.show_focus();
        Outcome::None
    }

    /// Fraction of the list above the visible rows and fraction shown (scroll bar).
    pub fn scroll(&self) -> Option<(f32, f32)> {
        let n = self.len();
        (n > self.visible).then(|| (self.top as f32 / n as f32, self.visible as f32 / n as f32))
    }
}

/// The equipment page's focus (element, shown) before and after a key, the page open both times:
/// another element focused, or a hidden focus shown again, is a focus tick. Enter, a toggle or
/// a key that cannot move (Left / Right on Start level) is none.
pub fn gear_tick(before: Option<(Gear, bool)>, after: Option<(Gear, bool)>) -> bool {
    match (before, after) {
        (Some((g0, s0)), Some((g1, s1))) => s1 && (g0 != g1 || !s0),
        _ => false,
    }
}

/// The equipment page's focus before and after the mouse moved, the page open both times: the
/// mouse now on another element is a focus tick, as a game's button ticks when the mouse enters
/// it (XIIIGUIBaseButton.MouseEnter, through LookUnderCursor), and so is coming back onto the
/// element that kept the focus after the mouse was over nothing (this page is OpenXIII's own: the
/// owner's choice, every entry onto an element ticks). Leaving every element ticks nothing.
pub fn mouse_gear_tick(before: Option<(Gear, bool)>, after: Option<(Gear, bool)>) -> bool {
    match (before, after) {
        (Some((g0, s0)), Some((g1, s1))) => s1 && (g0 != g1 || !s0),
        _ => false,
    }
}

/// The rectangle of a weapon cell `r` (pixels) the mouse finds it in: the cell widened by half
/// the gap to its neighbours on every side (`gap` = the gaps in pixels, across and down), so that
/// the mouse between two cells is always over one of them.
fn cell_hit_rect(r: [f32; 4], gap: (f32, f32)) -> [f32; 4] {
    let (hx, hy) = (gap.0 / 2.0, gap.1 / 2.0);
    [r[0] - hx, r[1] - hy, r[2] + 2.0 * hx, r[3] + 2.0 * hy]
}

/// The focus moved to another row of the list while staying on the same visible row (the list
/// scrolled under it): `applied`/`applied_row` the visible row and list row focused last, `want`
/// and `focus` the new ones.
pub fn focus_moved_in_place(applied: Option<usize>, applied_row: Option<usize>, want: Option<usize>, focus: usize) -> bool {
    want.is_some() && want == applied && applied_row.is_some_and(|r| r != focus)
}

/// The point `(mx, my)` is on scroll bar `[x, y, w, h]` (screen pixels), taken a little wider than
/// the thin bar so that a click beside it does not reach a row.
pub fn on_bar([x, y, w, h]: [f32; 4], (mx, my): (f32, f32)) -> bool {
    let margin = w.max(4.0);
    mx >= x - margin && mx <= x + w + margin && my >= y && my <= y + h
}

// Layout, in the page's 640 x 480 units (heights multiplied by fScaleTo), from
// XIIIMenuLoadGameWindow.Paint and its save slots.
const PICTURE: [f32; 4] = [36.0, 33.0, 348.0, 393.0];
const TITLE: [f32; 4] = [0.0, 40.0, 170.0, 40.0];
const ROW_X: f32 = 50.0;
const ROW_W: f32 = 320.0;
const ROW_H: f32 = 30.0;
/// First row and row pitch: the save slots' height, closer than the slots (40) so that eight
/// rows fit in the picture.
const ROW_Y: f32 = 112.0;
const ROW_PITCH: f32 = 36.0;
pub const ROWS: usize = 8;
/// Text inset of every row (one level: no indentation).
const ROW_INSET: f32 = 10.0;
/// Scroll bar, right of the rows.
const SCROLL_X: f32 = 374.0;
const SCROLL_W: f32 = 7.0;
/// The scroll bar in the page's own row look (the owner's choice): the track is the glass of an
/// unfocused row (white, half transparent, drawn with the alpha style, without a border), the
/// thumb the opaque white bordered box of the focused row.
const SCROLL_TRACK: [u8; 4] = [255, 255, 255, 128];
const SCROLL_THUMB: [u8; 4] = [255, 255, 255, 255];
const SCROLL_TRACK_BORDER: bool = false;
const SCROLL_THUMB_BORDER: bool = true;
/// Smallest thumb height (pixels).
const SCROLL_MIN_THUMB: f32 = 4.0;

/// Equipment page: the grid of weapon icons (cells of the HUD's 2:1 icon size), the label naming
/// the focused weapon, the All / None buttons and Start level.
const GRID_X: f32 = 44.0;
const GRID_Y: f32 = 92.0;
const CELL_W: f32 = 60.0;
const CELL_H: f32 = 30.0;
const CELL_PITCH_X: f32 = 64.0;
const CELL_PITCH_Y: f32 = 36.0;
const NAME_Y: f32 = 274.0;
const ALL_BOX: [f32; 4] = [44.0, 314.0, 154.0, 30.0];
const NONE_BOX: [f32; 4] = [206.0, 314.0, 154.0, 30.0];
const START_BOX: [f32; 4] = [44.0, 354.0, 316.0, 30.0];
/// A chosen weapon's cell (pale yellow, like a highlighter) and an unchosen one's thin frame.
const SELECTED_FILL: [u8; 4] = [255, 222, 89, 255];
const UNSELECTED_FRAME: [u8; 4] = [170, 170, 170, 255];
/// The hovered or focused weapon's name over the lift picture (light grey).
const WEAPON_NAME_COLOUR: [u8; 4] = [0xD8, 0xD8, 0xD8, 255];
/// Frame of the weapon under the mouse or focused by the keys / gamepad: the menus' orange,
/// visible on the dark lift background and on the white and yellow cells alike.
const HOVER_FRAME_COLOUR: [u8; 4] = [247, 170, 30, 255];

/// The hover frame of a weapon cell `r` (pixels) at `unit` pixels per page unit: the rectangle
/// around the cell, 3 units out, and the frame's thickness (3 units, at least 2 pixels).
fn hover_frame(r: [f32; 4], unit: f32) -> ([f32; 4], f32) {
    let m = (3.0 * unit).round();
    ([r[0] - m, r[1] - m, r[2] + 2.0 * m, r[3] + 2.0 * m], (3.0 * unit).round().max(2.0))
}
/// A weapon left out: its icon in this tint on the white cell, its lightest parts in the tint and
/// its outlines black (colour × luminance / 255).
const UNSELECTED_ICON_TINT: [u8; 3] = [0x8A, 0x8A, 0x8A];

/// `img` with the check mark (`check_mark`, 40 % of its height) in its top right corner: the
/// picture of a chosen weapon (the game's own "Valid" mark is a cross, which reads as "off").
/// A weapon left out: the icon in [`UNSELECTED_ICON_TINT`] (alpha kept), readable on its white cell.
fn dark_icon(img: &Image) -> Image {
    #[allow(unused_mut)]
    let mut tint = Some(UNSELECTED_ICON_TINT);
    let Some(tint) = tint else { return crate::menu_boxes::to_grey(img) };
    let mut out = img.clone();
    for px in out.rgba.chunks_exact_mut(4) {
        let y = (0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32) / 255.0;
        for c in 0..3 {
            px[c] = (tint[c] as f32 * y).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The mark is placed one texel inside the top and right edges: the picture's texture wraps, and
/// its filter would read a mark touching an edge on the opposite edge too (a dark line down the
/// left of the square helmet and vest icons, under the mark's black frame).
fn with_check_mark(img: &Image) -> Image {
    const INSET: u32 = 1;
    let s = ((img.h as f32 * 0.4).round() as u32).clamp(4, img.w.min(img.h).saturating_sub(INSET).max(4));
    let mark = check_mark(s);
    let mut out = img.clone();
    if img.w < s + INSET || img.h < s + INSET {
        return out;
    }
    let x0 = img.w - s - INSET;
    for y in 0..s {
        let (from, to) = (((y * s) * 4) as usize, ((((y + INSET) * img.w) + x0) * 4) as usize);
        out.rgba[to..to + (s * 4) as usize].copy_from_slice(&mark.rgba[from..from + (s * 4) as usize]);
    }
    out
}

/// The check mark: a white square with a black frame and a black tick, `size` x `size`, the
/// tick's strokes softened over one texel.
fn check_mark(size: u32) -> Image {
    let n = size as f32;
    let (a, b, c) = ((0.18 * n, 0.52 * n), (0.40 * n, 0.76 * n), (0.84 * n, 0.22 * n));
    let half = 0.085 * n;
    let frame = (size / 12).max(1);
    let dist = |p: (f32, f32), s: (f32, f32), e: (f32, f32)| {
        let (dx, dy) = (e.0 - s.0, e.1 - s.1);
        let t = (((p.0 - s.0) * dx + (p.1 - s.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((p.0 - s.0 - t * dx).powi(2) + (p.1 - s.1 - t * dy).powi(2)).sqrt()
    };
    let mut img = Image::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let d = dist(p, a, b).min(dist(p, b, c));
            let edge = x < frame || y < frame || x >= size - frame || y >= size - frame;
            let ink = if edge { 1.0 } else { (half - d + 0.5).clamp(0.0, 1.0) };
            let v = (255.0 * (1.0 - ink)).round() as u8;
            let i = ((y * size + x) * 4) as usize;
            img.rgba[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    img
}

/// The weapons' icons as the HUD draws them (XIIIBaseHud: HudWIcons = XIIIMenu.HUD.Weapon1Icons,
/// 64 x 32 cells, four per row, a weapon's cell is its InventoryGroup, an item's its IconNumber);
/// the armour pieces, which have none there, take their 32 x 32 cell of XIIIMenu.HUD.HudIcons.
/// In the order of select_level::WEAPONS.
const WEAPON_ATLAS: &str = "XIIIMenu.HUD.Weapon1Icons";
const ITEM_ATLAS: &str = "XIIIMenu.HUD.HudIcons";
const ICONS: [(bool, u32); 21] = [
    (true, 2),  // Beretta
    (true, 3),  // magnum
    (true, 11), // m16
    (true, 12), // kalash
    (true, 16), // m60
    (true, 13), // MiniUzi
    (true, 9),  // FusilPompe
    (true, 10), // FusilChasse
    (true, 14), // FusilSnipe
    (true, 6),  // arbalete
    (true, 7),  // ArbaleteX3
    (true, 8),  // LHarpon
    (true, 15), // Bazook
    (true, 4),  // GrenadB
    (true, 1),  // TKnife (XIIIWeapon's default InventoryGroup)
    (true, 21), // Hook
    (true, 20), // LockPick
    (true, 18), // MedKit
    (true, 19), // FullMedKit
    (false, 1), // GiletMk1
    (false, 2), // Casque
];

/// `img` with `n` transparent texels added on the left, right and top, and its last row repeated
/// `n` times below: the tile's bilinear filter then reads that row again past her feet (a hard
/// bottom edge), and the wrapping texture never brings her feet above the hat.
fn pad(img: &Image, n: u32) -> Image {
    let mut out = Image::new(img.w + 2 * n, img.h + 2 * n);
    if img.h == 0 {
        return out;
    }
    let row = (img.w * 4) as usize;
    for y in 0..img.h + n {
        let s = (y.min(img.h - 1) * img.w * 4) as usize;
        let d = (((y + n) * out.w + n) * 4) as usize;
        out.rgba[d..d + row].copy_from_slice(&img.rgba[s..s + row]);
    }
    out
}

/// Texture names of the pictures (the "Mod." prefix gives them their USize / VSize).
const BACKGROUND_TEXTURE: &str = "Mod.SelectLevel.Background";
const JONES_TEXTURE: &str = "Mod.SelectLevel.Jones";

/// Disk cache folder of the pictures, set by the game window.
static ART_CACHE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Jones's picture in place: None before the first, Some(None) the cut-out itself, otherwise its
/// enlargement (in 1 / select_level_art::SCALE_ONE) and variant (true: the simple one).
static JONES_APPLIED: Mutex<Option<Option<(u32, bool)>>> = Mutex::new(None);
/// Size and variant of Jones's enlarged picture being made in the background.
static JONES_WAITING: Mutex<Option<(u32, bool)>> = Mutex::new(None);
/// Pixels of the screen per texel of the cut-out Jones was last drawn at (0: not drawn yet).
static JONES_SHOWN: Mutex<f32> = Mutex::new(0.0);

/// The pictures' disk cache folder, when the game window set one.
pub fn art_cache_dir() -> Option<PathBuf> {
    ART_CACHE.lock().ok().and_then(|c| c.clone())
}

/// Where the pictures (Jones, and the video frames of every added menu: still_frames.rs) are
/// kept between runs (None: made again each run).
pub fn set_art_cache(dir: Option<PathBuf>) {
    crate::still_frames::set_cache_dir(dir.clone());
    if let Ok(mut c) = ART_CACHE.lock() {
        *c = dir;
    }
}

/// True when the pictures are kept between runs: the added menus' video frames are then made in
/// the background as soon as the game folder is known (`menu_boxes::prefetch_pictures`).
pub(crate) fn art_cache_set() -> bool {
    ART_CACHE.lock().is_ok_and(|c| c.is_some())
}

/// The icon of each weapon: (colour, grey) textures and its size in the cell.
type Icon = (Value, Value, (f32, f32));

/// The Select level page in front: its object, row buttons, navigation and pictures.
#[derive(Default)]
pub struct SelectPage {
    /// Set while the page is being opened, so its Created makes the rows.
    pending: bool,
    pub(crate) page: Option<ObjId>,
    rows: Vec<ObjId>,
    pub nav: Option<Nav>,
    /// Visible row given the focus last (the mouse moving it shows as a change).
    applied: Option<usize>,
    /// The list's row focused last (its place in the whole list).
    applied_row: Option<usize>,
    background: Option<Value>,
    /// Width / height of the picture's box, and a number for each background registered.
    background_aspect: f32,
    background_serial: u32,
    jones: Option<(Value, (u32, u32), [u32; 4])>,
    icons: Vec<Option<Icon>>,
    /// Equipment page: where each element was drawn (screen pixels), the mouse last seen.
    gear_rects: Vec<(Gear, [f32; 4])>,
    mouse: (i32, i32),
    /// The levels list's scroll bar as last drawn (screen pixels: x, y, width, height), and the
    /// thumb being dragged (pixels between the cursor and the thumb's top).
    scroll_rect: Option<[f32; 4]>,
    /// Weapons checked, kept between openings and handed to the game window.
    weapons: Option<u32>,
    weapons_changed: Option<u32>,
    start: Option<String>,
}

impl SelectPage {
    /// `o` is one of the levels list's rows (a list that scrolls: gui.rs, menu_control_at).
    pub(crate) fn is_row(&self, o: ObjId) -> bool {
        self.page.is_some() && self.rows.contains(&o)
    }

    /// The page is being opened (its Created is hooked).
    pub(crate) fn is_pending(&self) -> bool {
        self.pending
    }
}

impl Vm {
    /// The Select level page in front.
    pub fn select_level_page_in_front(&self) -> Option<ObjId> {
        let page = self.improved.menu_boxes.select.page?;
        let active = self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object())?;
        (active == page && self.objects.get(page).is_some_and(|o| o.is_some())).then_some(page)
    }

    /// Start chosen on the page since the last call (the game window loads it).
    pub fn take_select_level_start(&mut self) -> Option<String> {
        self.improved.menu_boxes.select.start.take()
    }

    /// Test weapons as the game window keeps them (one bit per line of select_level::WEAPONS).
    pub fn set_select_level_weapons(&mut self, mask: u32) {
        self.improved.menu_boxes.select.weapons = Some(mask);
        if let Some(n) = &mut self.improved.menu_boxes.select.nav {
            n.weapons = mask;
        }
    }

    /// Test weapons changed on the page since the last call.
    pub fn take_select_level_weapons_changed(&mut self) -> Option<u32> {
        self.improved.menu_boxes.select.weapons_changed.take()
    }

    /// Opens the page over the menu page in front. False without the level table or a menu.
    pub fn open_select_level_page(&mut self) -> bool {
        if !crate::select_level::available() {
            self.warn(crate::select_level::UNAVAILABLE.to_string());
            return false;
        }
        if self.select_level_page_in_front().is_some() {
            return true;
        }
        let (Some(root), Some(class)) = (self.gui_local(), self.class("XIIIWindowMainMenu")) else { return false };
        self.improved.menu_boxes.select.pending = true;
        let opened = self.call_named(root, "OpenMenuWithClass", vec![Value::Class(Some(class))]).map(|v| v.as_bool()).unwrap_or(false);
        self.improved.menu_boxes.select.pending = false;
        let page = self.propr(root, "ActivePage").as_object();
        if !opened || page.is_none() || page != self.improved.menu_boxes.select.page {
            self.warn("Select level: page not opened".to_string());
            return false;
        }
        true
    }

    /// Hooks of the page's XIIIWindow functions (menu_boxes_call): Created makes the rows, Paint
    /// draws the pictures, title and equipment page and lays out the rows, AfterPaint the scroll
    /// bar, InternalOnKeyEvent the navigation.
    #[allow(clippy::type_complexity)]
    pub(crate) fn select_page_call(
        &mut self,
        f: usize,
        this: ObjId,
        name: &str,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        match name {
            "Created" if self.improved.menu_boxes.select.pending && self.is_a(this, "XIIIWindowMainMenu") => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.keep_out_of_saved_menu_stack(this);
                    self.select_page_created(this);
                }
                Ok(r)
            }
            _ if Some(this) != self.improved.menu_boxes.select.page => Err(args),
            "Paint" => {
                let canvas = args.first().cloned().flatten().and_then(|v| v.as_object());
                let r = self.call_inner(f, this, args);
                if let Some(c) = canvas {
                    self.select_page_paint(this, c);
                }
                Ok(r)
            }
            "AfterPaint" => {
                let canvas = args.first().cloned().flatten().and_then(|v| v.as_object());
                let r = self.call_inner(f, this, args);
                if let Some(c) = canvas {
                    self.select_page_after_paint(this, c);
                }
                Ok(r)
            }
            "InternalOnKeyEvent" => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                let outs = self.functions[f]
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Byte(0))) } else { None })
                    .collect();
                // The page's own key event first: the menu sounds of Enter and Backspace, F4.
                let r = self.call_inner(f, this, args);
                if key == 0x73 {
                    return Ok(r);
                }
                if state == 1 {
                    self.select_page_key(this, key);
                }
                Ok(Ok((Value::Bool(true), outs)))
            }
            _ => Err(args),
        }
    }

    fn select_page_created(&mut self, page: ObjId) {
        let weapons = self.improved.menu_boxes.select.weapons.unwrap_or(WEAPONS_DEFAULT);
        let s = &mut self.improved.menu_boxes.select;
        s.page = Some(page);
        s.rows.clear();
        s.applied = None;
        s.applied_row = None;
        s.gear_rects.clear();
        let mut nav = Nav::new(starts(), ROWS);
        nav.weapons = weapons;
        s.nav = Some(nav);
        let scale = self.page_scale_of(page);
        self.set_prop(page, "bShowBCK", Value::Bool(true));
        self.set_prop(page, "bShowSEL", Value::Bool(true));
        if let Some(class) = self.class("XIIIButton") {
            for i in 0..ROWS {
                let y = ROW_Y + i as f32 * ROW_PITCH;
                let args = vec![Value::Class(Some(class)), Value::Float(ROW_X), Value::Float(y * scale), Value::Float(ROW_W), Value::Float(ROW_H * scale)];
                let Some(row) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { continue };
                if let Value::Array(mut controls) = self.prop(page, "Controls") {
                    controls.push(Value::Object(Some(row)));
                    self.set_prop(page, "Controls", Value::Array(controls));
                }
                self.improved.menu_boxes.select.rows.push(row);
            }
        }
        self.select_page_pictures(scale);
    }

    /// The pictures (made once per run, see select_level_art.rs) and the weapons' icons (colour,
    /// and grey as the menu's grey pictures), registered as textures.
    fn select_page_pictures(&mut self, scale: f32) {
        self.improved.menu_boxes.select.background = None;
        self.improved.menu_boxes.select.background_aspect = PICTURE[2] / (PICTURE[3] * scale);
        self.select_page_background();
        self.select_page_jones();
        if self.improved.menu_boxes.select.icons.len() != WEAPONS.len() {
            self.select_page_icons();
        }
    }

    /// Jones's picture wanted now: her edge redrawn at the size she was last shown on the
    /// screen, or None for the cut-out itself (the Original setting, not drawn yet, or shown
    /// near her own size).
    fn select_page_jones_wanted(&mut self) -> Option<(u32, bool)> {
        // One master size for every screen size (menu_figures::master_scale): the graphics card
        // reduces it, nothing is made again when the resolution changes.
        self.character_edge().map(|e| (crate::menu_figures::master_scale_kept(&self.display_modes), e))
    }

    /// Jones's picture made again when the page has one and the setting, or the size she is
    /// shown at, changed (every frame of the menus: nothing is done otherwise).
    pub(crate) fn select_page_jones_fit(&mut self) {
        if self.improved.menu_boxes.select.jones.is_none() {
            return;
        }
        let wanted = self.select_page_jones_wanted();
        // Her enlarged picture is being made in the background: the plain one stays until then.
        let waiting = JONES_WAITING.lock().ok().and_then(|w| *w);
        if let (Some(w), Some((k, plain))) = (waiting, wanted) {
            if w == (k, plain) && select_level_art::jones_fine_made(k, plain).is_none() {
                return;
            }
        }
        let applied = JONES_APPLIED.lock().ok().and_then(|a| *a);
        // The size read back from an enlarged picture differs by a rounding: not a new size.
        let same = match (applied, wanted) {
            (Some(Some((k0, e0))), Some((k1, e1))) => e0 == e1 && k0.abs_diff(k1) <= 3,
            (Some(a), w) => a == w,
            (None, _) => false,
        };
        if !same {
            self.select_page_jones();
        }
    }

    /// Jones's enlarged picture made on a background thread (once for a size and variant; the
    /// menus' characters usually made her already).
    fn select_page_jones_later(&mut self, cache: Option<PathBuf>, k: u32, plain: bool) {
        if let Ok(mut waiting) = JONES_WAITING.lock() {
            if *waiting == Some((k, plain)) {
                return;
            }
            *waiting = Some((k, plain));
        }
        let quarters: Vec<Option<Image>> = select_level_art::JONES_TEXTURES.iter().map(|p| self.menu_texture_image(p)).collect();
        let math = openxiii_math::Mode::current();
        let _ = std::thread::Builder::new().name("select-jones".into()).spawn(move || {
            let _math = math.enter();
            openxiii_package::background::lower_this_thread();
            let _ = select_level_art::jones_fine(cache.as_deref(), k, plain, |p| select_level_art::JONES_TEXTURES.iter().position(|t| *t == p).and_then(|i| quarters[i].clone()));
        });
    }

    /// Jones's picture for the current edge setting and the size she is shown at, registered as
    /// a texture.
    pub(crate) fn select_page_jones(&mut self) {
        let cache = ART_CACHE.lock().ok().and_then(|c| c.clone());
        let mut wanted = self.select_page_jones_wanted();
        if let Some((k, plain)) = wanted {
            if select_level_art::jones_fine_made(k, plain).is_none() {
                // Not made yet: made in the background, her plain picture shown meanwhile (the
                // game's frames never wait for her).
                self.select_page_jones_later(cache.clone(), k, plain);
                wanted = None;
            }
        }
        if let Ok(mut applied) = JONES_APPLIED.lock() {
            *applied = Some(wanted);
        }
        let jones = match wanted {
            Some((k, plain)) => select_level_art::jones_fine_made(k, plain).flatten(),
            None => select_level_art::jones(cache.as_deref(), |p| self.menu_texture_image(p)),
        };
        self.improved.menu_boxes.select.jones = jones.and_then(|img| {
            // Padded (see `pad`); jones_tile places her by her lowest opaque row, the texel
            // offsets of the padding added to her figure's bounds.
            let f = select_level_art::figure_bounds(&img)?;
            let figure = [f[0] + 2, f[1] + 2, f[2] + 2, f[3] + 2];
            let img = pad(&img, 2);
            let size = (img.w, img.h);
            let fine = wanted.is_some();
            Some((Self::menu_register_with(JONES_TEXTURE, img, fine), size, figure))
        });
    }

    /// The weapons' icons (colour, and grey as the menu's grey pictures), registered as textures.
    fn select_page_icons(&mut self) {
        {
            let atlases = [self.menu_texture_image(WEAPON_ATLAS), self.menu_texture_image(ITEM_ATLAS)];
            self.improved.menu_boxes.select.icons = ICONS
                .iter()
                .enumerate()
                .map(|(i, &(weapon, cell))| {
                    let atlas = atlases[usize::from(!weapon)].as_ref()?;
                    // Cells of 64 x 32 (four per row) or 32 x 32 (four per row), as the HUD reads
                    // them from the 256 x 256 and 128 x 128 textures; scaled if the texture is not.
                    let (cw, ch, base) = if weapon { (64, 32, 256) } else { (32, 32, 128) };
                    let k = atlas.w as f32 / base as f32;
                    let (cw, ch) = ((cw as f32 * k) as u32, (ch as f32 * k) as u32);
                    let img = atlas.region((cell % 4) * cw, (cell / 4) * ch, cw, ch);
                    let size = if weapon { (CELL_W - 4.0, CELL_H - 2.0) } else { (CELL_H - 2.0, CELL_H - 2.0) };
                    let grey = dark_icon(&img);
                    Some((
                        Self::menu_register(&format!("Mod.SelectLevel.IconChosen{i}"), with_check_mark(&img)),
                        Self::menu_register(&format!("Mod.SelectLevel.IconGrey{i}"), grey),
                        size,
                    ))
                })
                .collect();
        }
    }

    /// Jones cut out once (about 30 ms the first time, then read from the cache), when the main
    /// menu is made: with the video frames started at load, every added picture is ready before
    /// its page opens.
    pub(crate) fn select_level_prefetch_jones(&mut self) {
        let Some(cache) = ART_CACHE.lock().ok().and_then(|c| c.clone()) else { return };
        let _ = select_level_art::jones(Some(&cache), |p| self.menu_texture_image(p));
    }

    /// The background picture once its video frame is ready (made in the background,
    /// still_frames.rs): until then the page shows without it. Each new picture gets its own
    /// texture name (the renderer keeps what it uploaded for a name).
    fn select_page_background(&mut self) {
        if self.improved.menu_boxes.select.background.is_some() {
            return;
        }
        let video_dir = self.engine.video.dir.clone();
        let Some(img) = select_level_art::background(&video_dir) else { return };
        let fitted = crop_to_aspect(&img, self.improved.menu_boxes.select.background_aspect, Crop { cx: 0.5, cy: 0.5, h: 1.0 });
        let s = &mut self.improved.menu_boxes.select;
        s.background_serial += 1;
        let name = format!("{BACKGROUND_TEXTURE}{}", s.background_serial);
        self.improved.menu_boxes.select.background = Some(Self::menu_register(&name, fitted));
    }

    pub(crate) fn page_scale_of(&self, page: ObjId) -> f32 {
        let scale = self.propr(page, "fScaleTo").as_float();
        if scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    /// Touch screens (gui.rs, menu_control_at): the equipment page's element under the point
    /// (x, y) of the menu's page, as its place in the drawn elements (the elements are drawn
    /// here, not made of the game's controls). None: another page, or no element there.
    pub(crate) fn select_gear_at(&self, x: f32, y: f32) -> Option<usize> {
        let s = &self.improved.menu_boxes.select;
        s.nav.as_ref().filter(|n| n.chosen.is_some())?;
        s.gear_rects.iter().position(|(_, r)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3])
    }

    /// The equipment element under the mouse.
    fn select_gear_under_mouse(&self) -> Option<Gear> {
        let ctrl = self.gui_local()?;
        let (x, y) = (self.propr(ctrl, "MouseX").as_int() as f32, self.propr(ctrl, "MouseY").as_int() as f32);
        self.improved.menu_boxes.select.gear_rects.iter().find(|(_, r)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]).map(|(g, _)| *g)
    }

    fn select_page_key(&mut self, page: ObjId, key: i32) {
        let rows = self.improved.menu_boxes.select.rows.clone();
        let gear = self.improved.menu_boxes.select.nav.as_ref().is_some_and(|n| n.chosen.is_some());
        let k = match key {
            0x26 => NavKey::Up,
            0x28 => NavKey::Down,
            0x21 => NavKey::PageUp,
            0x22 => NavKey::PageDown,
            0x24 => NavKey::Home,
            0x23 => NavKey::End,
            0x25 => NavKey::Left,
            0x27 => NavKey::Right,
            0x0D => NavKey::Enter,
            0x1B | 0x08 => NavKey::Back,
            // IK_MouseWheelUp, IK_MouseWheelDown.
            0xEC => NavKey::Wheel(-1),
            0xED => NavKey::Wheel(1),
            // IK_LeftMouse: a click on the element under the cursor (none elsewhere on the page).
            0x01 if gear => match self.select_gear_under_mouse() {
                Some(g) => NavKey::ClickGear(g),
                None => return,
            },
            // On the scroll bar (a position indicator only): nothing.
            0x01 if self.select_mouse_on_bar() => return,
            0x01 => {
                match rows.iter().position(|&r| self.propr(r, "bVisible").as_bool() && self.call_named(r, "IsInBounds", Vec::new()).is_ok_and(|v| v.as_bool()))
                {
                    Some(v) => NavKey::Click(v),
                    None => return,
                }
            }
            _ => return,
        };
        let Some(nav) = &mut self.improved.menu_boxes.select.nav else { return };
        let gear_before = nav.chosen.as_ref().map(|_| (nav.gear, nav.gear_shown));
        let outcome = nav.key(k);
        // The equipment page is drawn here, not made of the game's buttons: a focus moved to
        // another element (or shown again) plays the game's focus tick, as a button's MouseEnter
        // does (the list's rows are real buttons and tick through SetFocus below).
        let gear_after = nav.chosen.as_ref().map(|_| (nav.gear, nav.gear_shown));
        let ticked = gear_tick(gear_before, gear_after) && self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
        let _ = ticked;
        match outcome {
            Outcome::None => {}
            Outcome::Start(url) => self.improved.menu_boxes.select.start = Some(url),
            Outcome::Close => {
                if let Some(root) = self.propr(page, "myRoot").as_object() {
                    let _ = self.call_named(root, "CloseMenu", vec![Value::Bool(true)]);
                }
                // Closed: no row of this page takes the focus back (SetFocus blurred the page
                // below, which CloseMenu had just focused, and its arrows were then lost).
                return;
            }
            Outcome::WeaponsChanged(mask) => {
                self.improved.menu_boxes.select.weapons = Some(mask);
                self.improved.menu_boxes.select.weapons_changed = Some(mask);
            }
        }
        self.select_page_sync_focus();
    }

    /// The cursor's position (screen pixels, as the scroll bar's rectangle).
    fn select_mouse(&self) -> Option<(f32, f32)> {
        let ctrl = self.gui_local()?;
        Some((self.propr(ctrl, "MouseX").as_float(), self.propr(ctrl, "MouseY").as_float()))
    }

    /// The cursor is on the levels list's scroll bar (a little wider than the thin bar). The bar
    /// only shows the place in the list (the owner's choice): the mouse wheel, the keys and the
    /// gamepad move the list, a click there does nothing.
    fn select_mouse_on_bar(&self) -> bool {
        let (Some(bar), Some(mouse)) = (self.improved.menu_boxes.select.scroll_rect, self.select_mouse()) else { return false };
        on_bar(bar, mouse)
    }

    /// The focused row of the navigation given the input focus (its border).
    fn select_page_sync_focus(&mut self) {
        let Some(nav) = &self.improved.menu_boxes.select.nav else { return };
        if nav.chosen.is_some() {
            return;
        }
        let want = nav.focus.checked_sub(nav.top).filter(|v| *v < self.improved.menu_boxes.select.rows.len());
        let focus = nav.focus;
        let again = focus_moved_in_place(self.improved.menu_boxes.select.applied, self.improved.menu_boxes.select.applied_row, want, focus);
        if let Some(v) = want {
            let row = self.improved.menu_boxes.select.rows[v];
            // The list scrolled under a row that keeps the focus (the focus moved past the
            // first or last visible row): the row loses its focus through the game's own
            // LoseFocus, then takes it again, so that SetFocus -> MenuStateChange ->
            // XIIIGUIBaseButton.MouseEnter plays the focus tick (MnCurseur) and the row is drawn
            // focused, as for a move between visible rows. (Clearing bHasFocus by hand left the
            // controller holding the row: SetFocus did nothing, no tick and no white frame.)
            // Only when the row is the controller's focused control: the mouse may have moved the
            // focus to another row since the last frame (a wheel step before the next Paint), and
            // LoseFocus on a row that is not focused cleared the controller's FocusedControl, so
            // SetFocus below never took the focus from that other row: both stayed focused (white).
            let held = self.gui_local().and_then(|c| self.propr(c, "FocusedControl").as_object());
            if again && held == Some(row) {
                let _ = self.call_named(row, "LoseFocus", vec![Value::Object(None)]);
            }
            if !self.propr(row, "bHasFocus").as_bool() {
                let _ = self.call_named(row, "SetFocus", vec![Value::Object(None)]);
            }
        }
        self.improved.menu_boxes.select.applied = want;
        self.improved.menu_boxes.select.applied_row = want.map(|_| focus);
    }

    /// Text size in the canvas font.
    pub(crate) fn select_text_size(&mut self, c: ObjId, t: &str) -> (f32, f32) {
        let mut args = [Value::Str(t.to_string()), Value::Float(0.0), Value::Float(0.0)];
        self.canvas_native("TextSize", c, &mut args, &[true; 3]);
        (args[1].as_float(), args[2].as_float())
    }

    pub(crate) fn select_color(&mut self, c: ObjId, rgba: [u8; 4]) {
        self.canvas_native("SetDrawColor", c, &mut rgba.map(Value::Byte), &[true; 4]);
    }

    pub(crate) fn select_draw_text(&mut self, c: ObjId, x: f32, y: f32, t: &str, rgba: [u8; 4]) {
        self.select_color(c, rgba);
        self.canvas_native("SetPos", c, &mut [Value::Float(x), Value::Float(y)], &[true; 2]);
        self.canvas_native("DrawText", c, &mut [Value::Str(t.to_string()), Value::Bool(false)], &[true; 2]);
    }

    pub(crate) fn select_draw_texture(&mut self, page: ObjId, c: ObjId, [x, y, w, h]: [f32; 4], tex: Value) {
        let _ = self.call_named(
            page,
            "DrawStretchedTexture",
            vec![Value::Object(Some(c)), Value::Float(x), Value::Float(y), Value::Float(w), Value::Float(h), tex],
        );
    }

    /// A box drawn as an XIIIButton draws itself (glass look: FondMenu half transparent, opaque
    /// with a black border when focused), its text centred in black. `r` in pixels.
    fn select_draw_button(&mut self, page: ObjId, c: ObjId, r: [f32; 4], label: &str, focus: bool) {
        let Some(glass) = self.gui_local().map(|g| self.propr(g, "FondMenu").clone()) else { return };
        self.set_prop(c, "Style", Value::Byte(5));
        self.select_color(c, [255, 255, 255, if focus { 255 } else { 128 }]);
        self.set_prop(c, "bUseBorder", Value::Bool(focus));
        self.select_draw_texture(page, c, r, glass);
        self.set_prop(c, "bUseBorder", Value::Bool(false));
        self.set_prop(c, "Style", Value::Byte(1));
        if !label.is_empty() {
            let (tw, th) = self.select_text_size(c, label);
            self.select_draw_text(c, r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - th) / 2.0, label, [0, 0, 0, 255]);
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// A label in the game's style (XIIIWindow.DrawLabel): white box with a black border, black
    /// text, the box widened to the text. `r` in units.
    pub(crate) fn select_draw_label(&mut self, page: ObjId, c: ObjId, r: [f32; 4], label: &str, (rx, ry): (f32, f32), centred: bool) {
        let (tw, th) = self.select_text_size(c, label);
        let mut r = r;
        r[2] = r[2].max(tw / rx.max(0.01) + 24.0);
        if let Some(glass) = self.gui_local().map(|g| self.propr(g, "FondMenu").clone()) {
            self.select_color(c, [255, 255, 255, 255]);
            self.set_prop(c, "Style", Value::Byte(1));
            self.set_prop(c, "bUseBorder", Value::Bool(true));
            self.select_draw_texture(page, c, [(r[0] + 2.0) * rx, (r[1] + 2.0) * ry, (r[2] - 4.0) * rx, (r[3] - 4.0) * ry], glass);
            self.set_prop(c, "bUseBorder", Value::Bool(false));
        }
        let x = if centred { (r[0] + r[2] / 2.0) * rx - tw / 2.0 } else { (r[0] + 12.0) * rx };
        self.select_draw_text(c, x, (r[1] + r[3] / 2.0) * ry - th / 2.0, label, [0, 0, 0, 255]);
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// After the page's Paint (white page): the picture, Jones, the title, then the levels rows
    /// laid out (texts, inset, visibility, focus) before the controls draw themselves, or the
    /// equipment page.
    fn select_page_paint(&mut self, page: ObjId, c: ObjId) {
        let scale = self.page_scale_of(page);
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        let lang = self.text_language().to_string();
        let picture = [PICTURE[0], PICTURE[1] * scale, PICTURE[2], PICTURE[3] * scale];
        let px = |r: [f32; 4]| [r[0] * rx, r[1] * ry, r[2] * rx, r[3] * ry];
        self.select_color(c, [255, 255, 255, 255]);
        self.set_prop(c, "Style", Value::Byte(1));
        self.select_page_background();
        // The same picture (the lift shaft) behind both pages.
        if let Some(bg) = self.improved.menu_boxes.select.background.clone() {
            self.set_prop(c, "bUseBorder", Value::Bool(true));
            self.select_draw_texture(page, c, px(picture), bg);
            self.set_prop(c, "bUseBorder", Value::Bool(false));
        }
        if let Some((tex, size, figure)) = self.improved.menu_boxes.select.jones.clone() {
            // Her feet on the picture's frame: DrawTile's border lines run along y + h and
            // y + h + 1 (screen); a line at Y fills the pixel row whose lower edge is ceil(Y)
            // (measured at 640 x 480, where Y is whole, and 1280 x 960 / 1920 x 1080, where it is
            // not), so her last row ends on ceil(y + h + 1), in whole pixels at any scaling.
            let p = px(picture);
            let oy = self.propr(c, "OrgY").as_float();
            let bottom = (oy + p[1] + p[3] + 1.0).ceil() - oy;
            let (r, uv) = select_level_art::jones_tile(p, bottom, size, figure);
            // The size she is shown at, per texel of the cut-out (the picture in place may be an
            // enlargement of it): select_page_jones_fit makes her picture for that size.
            if uv[3] > 0.0 {
                let enlarged = JONES_APPLIED.lock().ok().and_then(|a| (*a).flatten()).map_or(1.0, |(k, _)| k as f32 / select_level_art::SCALE_ONE as f32);
                let frame = self.menu_layout.last_frame.map_or(1.0, |f| f.scale);
                if let Ok(mut shown) = JONES_SHOWN.lock() {
                    *shown = r[3] / uv[3] * enlarged * frame;
                }
            }
            self.set_prop(c, "Style", Value::Byte(5));
            self.set_prop(c, "CurX", Value::Float(r[0]));
            self.set_prop(c, "CurY", Value::Float(r[1]));
            let mut args = vec![tex, Value::Float(r[2]), Value::Float(r[3]), Value::Float(uv[0]), Value::Float(uv[1]), Value::Float(uv[2]), Value::Float(uv[3])];
            let _ = self.canvas_native("DrawTile", c, &mut args, &[true; 7]);
            self.set_prop(c, "Style", Value::Byte(1));
        }
        let Some(nav) = self.improved.menu_boxes.select.nav.clone() else { return };
        let equipment = nav.chosen.is_some();
        // Equipment page: the chosen start's name as the title (said once, at the top).
        let title = match &nav.chosen {
            Some((_, name)) => name.clone(),
            None => text(&lang, Text::SelectLevel).to_string(),
        };
        self.select_draw_label(page, c, TITLE, &title, (rx, ry), true);

        // Help bar: Escape Back / Select (Choose on the equipment page) Enter.
        self.set_prop(page, "bShowSEL", Value::Bool(!equipment));
        self.set_prop(page, "bShowCHO", Value::Bool(equipment));

        let rows = self.improved.menu_boxes.select.rows.clone();
        if equipment {
            for &row in &rows {
                self.set_prop(row, "bVisible", Value::Bool(false));
                self.set_prop(row, "bNeverFocus", Value::Bool(true));
            }
            self.select_release_row_focus(page, &rows);
            self.select_paint_equipment(page, c, (rx, ry), scale, &lang);
            return;
        }
        // A focus moved by the mouse becomes the navigation's.
        let focused = rows.iter().position(|&r| self.propr(r, "bHasFocus").as_bool() && self.propr(r, "bVisible").as_bool());
        // The cursor on the scroll bar never moves the focus (the bar is not a control).
        if focused.is_some() && focused != self.improved.menu_boxes.select.applied && !self.select_mouse_on_bar() {
            if let (Some(v), Some(nav)) = (focused, &mut self.improved.menu_boxes.select.nav) {
                nav.key(NavKey::Hover(v));
            }
        }
        self.select_page_sync_focus();
        let Some(nav) = self.improved.menu_boxes.select.nav.clone() else { return };
        let visible = nav.visible_rows();
        for (i, &row) in rows.iter().enumerate() {
            let label = visible.get(i).map(|s| s.name.clone()).unwrap_or_default();
            let shown = i < visible.len();
            self.set_prop(row, "bVisible", Value::Bool(shown));
            self.set_prop(row, "bNeverFocus", Value::Bool(!shown));
            self.set_prop(row, "Text", Value::Str(label));
            self.set_prop(row, "TextAlign", Value::Byte(0));
            self.set_prop(row, "TextX", Value::Int((ROW_INSET * rx).round() as i32));
            // Only the focused row has a border (as the Load page's slots).
            let focus = Some(i) == self.improved.menu_boxes.select.applied;
            self.set_prop(row, "bUseBorder", Value::Bool(focus));
        }
    }

    /// Equipment page: the hidden level row that was chosen must not keep the input focus. The
    /// controller hands a left click to the page only when the control under the cursor (here
    /// the page itself: the grid is drawn, not made of controls) is the focused one
    /// (GUIController NativeKeyEvent, `gui_controller_key_event`); with a hidden row focused,
    /// every click became key 0 and the weapons never toggled. The page takes the focus back.
    fn select_release_row_focus(&mut self, page: ObjId, rows: &[ObjId]) {
        let Some(ctrl) = self.gui_local() else { return };
        let focused = self.propr(ctrl, "FocusedControl").as_object();
        if focused.is_some_and(|f| rows.contains(&f)) {
            let _ = self.call_named(page, "SetFocus", vec![Value::Object(None)]);
            if self.propr(ctrl, "FocusedControl").as_object().is_some_and(|f| rows.contains(&f)) {
                self.set_prop(ctrl, "FocusedControl", Value::Object(Some(page)));
            }
            for &r in rows {
                self.set_prop(r, "bHasFocus", Value::Bool(false));
            }
        }
        // The hidden rows back to the idle menu state, and none left as the controller's active
        // control: the row chosen before kept its focused state (MenuState 2) through the
        // equipment page, and back on the list the mouse never lit it again (LookUnderCursor
        // only watches an idle control: the 4th or 6th row, whichever had been chosen).
        for &r in rows {
            if !matches!(self.propr(r, "MenuState").as_byte(), 0 | 4) {
                self.set_prop(r, "MenuState", Value::Byte(0));
            }
        }
        if self.propr(ctrl, "ActiveControl").as_object().is_some_and(|a| rows.contains(&a)) {
            self.set_prop(ctrl, "ActiveControl", Value::Object(None));
        }
    }

    /// Equipment page: the mouse moving over an element focuses it, with the focus tick a game's
    /// button plays when the mouse enters it (the grid is drawn, not made of the game's buttons).
    /// True when the tick was due.
    fn select_gear_hover(&mut self) -> bool {
        let Some(ctrl) = self.gui_local() else { return false };
        let m = (self.propr(ctrl, "MouseX").as_int(), self.propr(ctrl, "MouseY").as_int());
        if m == self.improved.menu_boxes.select.mouse {
            return false;
        }
        self.improved.menu_boxes.select.mouse = m;
        let under = self.select_gear_under_mouse();
        let Some(nav) = &mut self.improved.menu_boxes.select.nav else { return false };
        let before = nav.chosen.as_ref().map(|_| (nav.gear, nav.gear_shown));
        nav.key(under.map_or(NavKey::HoverNone, NavKey::HoverGear));
        let tick = mouse_gear_tick(before, nav.chosen.as_ref().map(|_| (nav.gear, nav.gear_shown)));
        if tick {
            self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
        }
        tick
    }

    /// A rectangle `r` (pixels) filled with `rgba` (the menu's glass texture drawn opaque).
    pub(crate) fn select_fill(&mut self, page: ObjId, c: ObjId, r: [f32; 4], rgba: [u8; 4]) {
        let Some(glass) = self.gui_local().map(|g| self.propr(g, "FondMenu").clone()) else { return };
        self.set_prop(c, "Style", Value::Byte(1));
        self.select_color(c, rgba);
        self.select_draw_texture(page, c, r, glass);
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// The outline of `r` (pixels), `t` pixels thick, in `rgba`.
    fn select_outline(&mut self, page: ObjId, c: ObjId, r: [f32; 4], t: f32, rgba: [u8; 4]) {
        let [x, y, w, h] = r;
        for side in [[x, y, w, t], [x, y + h - t, w, t], [x, y, t, h], [x + w - t, y, t, h]] {
            self.select_fill(page, c, side, rgba);
        }
    }

    /// The equipment page: each weapon in its cell (unchosen: grey icon on white, thin grey
    /// frame; chosen: icon in colour with a check mark in its corner, on a yellow cell with a black
    /// frame; under the mouse or focused: a thick orange frame around the cell), the
    /// focused weapon's name, how many are chosen, All / None and Start level.
    fn select_paint_equipment(&mut self, page: ObjId, c: ObjId, (rx, ry): (f32, f32), scale: f32, lang: &str) {
        let (ox, oy) = (self.propr(c, "OrgX").as_float(), self.propr(c, "OrgY").as_float());
        let px = |r: [f32; 4]| [r[0] * rx, r[1] * scale * ry, r[2] * rx, r[3] * scale * ry];
        let mut rects: Vec<(Gear, [f32; 4])> = Vec::new();
        for i in 0..WEAPONS.len() {
            let (col, row) = ((i % GRID_COLUMNS) as f32, (i / GRID_COLUMNS) as f32);
            rects.push((Gear::Cell(i), px([GRID_X + col * CELL_PITCH_X, GRID_Y + row * CELL_PITCH_Y, CELL_W, CELL_H])));
        }
        rects.push((Gear::All, px(ALL_BOX)));
        rects.push((Gear::None, px(NONE_BOX)));
        rects.push((Gear::Start, px(START_BOX)));
        // Screen rectangles for the mouse (the canvas origin added); the weapon cells widened
        // over the gaps between them (no spot between two cells where no weapon is under the mouse).
        let gap = px([0.0, 0.0, CELL_PITCH_X - CELL_W, CELL_PITCH_Y - CELL_H]);
        let gap = (gap[2], gap[3]);
        self.improved.menu_boxes.select.gear_rects = rects
            .iter()
            .map(|(g, r)| {
                let r = if matches!(g, Gear::Cell(_)) { cell_hit_rect(*r, gap) } else { *r };
                (*g, [r[0] + ox, r[1] + oy, r[2], r[3]])
            })
            .collect();
        self.select_gear_hover();
        let Some(nav) = self.improved.menu_boxes.select.nav.clone() else { return };
        let icons = self.improved.menu_boxes.select.icons.clone();
        let unit = rx.min(scale * ry).max(0.5);
        for (g, r) in &rects {
            let focus = nav.gear_shown && nav.gear == *g;
            match g {
                Gear::Cell(i) => {
                    let on = nav.weapons & 1 << i != 0;
                    if on {
                        self.select_fill(page, c, *r, SELECTED_FILL);
                        self.select_outline(page, c, *r, (2.0 * unit).round().max(1.0), [0, 0, 0, 255]);
                    } else {
                        self.select_fill(page, c, *r, [255, 255, 255, 255]);
                        self.select_outline(page, c, *r, unit.round().max(1.0), UNSELECTED_FRAME);
                    }
                    if focus {
                        let (frame, t) = hover_frame(*r, unit);
                        self.select_outline(page, c, frame, t, HOVER_FRAME_COLOUR);
                    }
                    if let Some(Some((colour, grey, (w, h)))) = icons.get(*i) {
                        let (w, h) = (w * rx, h * scale * ry);
                        self.set_prop(c, "Style", Value::Byte(5));
                        self.select_color(c, [255, 255, 255, 255]);
                        self.select_draw_texture(page, c, [r[0] + (r[2] - w) / 2.0, r[1] + (r[3] - h) / 2.0, w, h], if on { colour.clone() } else { grey.clone() });
                        self.set_prop(c, "Style", Value::Byte(1));
                    }
                    self.select_color(c, [255, 255, 255, 255]);
                }
                Gear::All => self.select_draw_button(page, c, *r, text(lang, Text::All), focus),
                Gear::None => self.select_draw_button(page, c, *r, text(lang, Text::None), focus),
                Gear::Start => self.select_draw_button(page, c, *r, text(lang, Text::StartLevel), focus),
            }
        }
        // The weapon under the mouse or focused: its name in one line of text, centred over All /
        // None, without a box; nothing when no weapon is shown focused.
        if let (Gear::Cell(i), true) = (nav.gear, nav.gear_shown) {
            let name = self.weapon_label(i);
            let (tw, th) = self.select_text_size(c, &name);
            let centre = (ALL_BOX[0] + NONE_BOX[0] + NONE_BOX[2]) / 2.0;
            let y = (NAME_Y + 15.0) * scale * ry - th / 2.0;
            self.select_draw_text(c, centre * rx - tw / 2.0, y, &name, WEAPON_NAME_COLOUR);
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// After the controls: the levels list's scroll bar.
    fn select_page_after_paint(&mut self, page: ObjId, c: ObjId) {
        let Some(nav) = self.improved.menu_boxes.select.nav.clone() else { return };
        if nav.chosen.is_some() {
            return;
        }
        let scale = self.page_scale_of(page);
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        if let (Some(_), Some(glass)) = (nav.scroll(), self.gui_local().map(|r| self.propr(r, "FondMenu").clone())) {
            let top = ROW_Y * scale;
            let height = ((ROWS - 1) as f32 * ROW_PITCH + ROW_H) * scale;
            let (ox, oy) = (self.propr(c, "OrgX").as_float(), self.propr(c, "OrgY").as_float());
            let track = (top * ry + oy, height * ry);
            self.improved.menu_boxes.select.scroll_rect = Some([SCROLL_X * rx + ox, track.0, SCROLL_W * rx, track.1]);
            let Some((from, shown)) = self.improved.menu_boxes.select.nav.as_ref().and_then(|n| n.scroll()) else { return };
            // The bar in the rows' look (SCROLL_TRACK, SCROLL_THUMB): the styles are set here,
            // the controls drawn just before may leave another one.
            let style = self.prop(c, "Style");
            self.set_prop(c, "Style", Value::Byte(5));
            self.select_color(c, SCROLL_TRACK);
            self.set_prop(c, "bUseBorder", Value::Bool(SCROLL_TRACK_BORDER));
            self.select_draw_texture(page, c, [SCROLL_X * rx, top * ry, SCROLL_W * rx, height * ry], glass.clone());
            self.set_prop(c, "bUseBorder", Value::Bool(false));
            self.set_prop(c, "Style", Value::Byte(1));
            self.select_color(c, SCROLL_THUMB);
            self.set_prop(c, "bUseBorder", Value::Bool(SCROLL_THUMB_BORDER));
            let thumb = (height * shown * ry).max(SCROLL_MIN_THUMB);
            self.select_draw_texture(page, c, [SCROLL_X * rx, (top + height * from) * ry, SCROLL_W * rx, thumb], glass);
            self.set_prop(c, "bUseBorder", Value::Bool(false));
            self.set_prop(c, "Style", style);
        } else {
            self.improved.menu_boxes.select.scroll_rect = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The equipment page ticks on every focus move, its wraps included (Up from the grid's top
    /// row to Start level, Left from a row's first weapon to its last), and when a hidden focus
    /// is shown again. Controls: Enter (a toggle), a key that cannot move and the list page tick
    /// nothing here.
    #[test]
    fn equipment_focus_moves_tick() {
        let mut nav = Nav::new(vec![Start { name: "a".into(), url: "A".into() }], 4);
        nav.key(NavKey::Enter);
        let state = |n: &Nav| n.chosen.as_ref().map(|_| (n.gear, n.gear_shown));
        let mut press = |n: &mut Nav, k: NavKey| {
            let before = state(n);
            n.key(k);
            gear_tick(before, state(n))
        };
        assert!(press(&mut nav, NavKey::Down), "Start level -> the grid's top row (wrap)");
        assert!(press(&mut nav, NavKey::Left), "first weapon -> last of the row (wrap)");
        assert!(press(&mut nav, NavKey::Up), "top row -> Start level (wrap)");
        assert!(!press(&mut nav, NavKey::Left), "Left on Start level: no move, no tick");
        assert!(!press(&mut nav, NavKey::Enter), "Enter: no focus move");
        nav.key(NavKey::HoverNone);
        assert!(press(&mut nav, NavKey::Down), "a hidden focus shown again");
        // The list page (no equipment open): nothing from here, its rows tick themselves.
        assert!(!gear_tick(None, None));
    }

    fn sample() -> Vec<Start> {
        let m = |name: &str, cps: usize| Mission {
            name: name.to_string(),
            url: name.to_string(),
            checkpoints: (1..=cps).map(|i| (format!("{name} {i}"), format!("{name}#cp{i}"))).collect(),
        };
        starts_of(&[m("A", 0), m("B", 2), m("C", 3), m("D", 0), m("E", 1), m("F", 0), m("G", 0), m("H", 2), m("I", 0), m("J", 1)])
    }

    fn names(rows: &[Start]) -> Vec<&str> {
        rows.iter().map(|s| s.name.as_str()).collect()
    }

    /// The bar looks like the page's rows: the track is the half transparent glass of an
    /// unfocused row (as select_draw_button draws it, no border), the thumb the opaque white box
    /// of the focused row (drawn bordered). Negative control: the former black, opaque, bordered
    /// track fails.
    #[test]
    fn scroll_bar_colours() {
        let unfocused_row = |c: [u8; 4], border: bool| c == [255, 255, 255, 128] && !border;
        assert!(unfocused_row(SCROLL_TRACK, SCROLL_TRACK_BORDER));
        assert_eq!((SCROLL_THUMB, SCROLL_THUMB_BORDER), ([255, 255, 255, 255], true));
        assert!(!unfocused_row([0, 0, 0, 255], true));
        assert!(!unfocused_row(SCROLL_TRACK, true));
    }

    /// The scroll bar only shows the place in the list: the mouse on it (the thin bar, or just
    /// beside it) is recognised so that a click there does nothing, whatever the screen scale; a
    /// point on a row, left of the bar's margin, is not on it. Negative control: a bar test
    /// without the margin misses a click just beside the bar.
    #[test]
    fn mouse_on_the_scroll_bar_does_nothing() {
        for k in [1.0f32, 1.5, 2.0, 2.5] {
            let bar = [374.0 * k, 112.0 * k, 7.0 * k, 288.0 * k];
            assert!(on_bar(bar, (377.0 * k, 200.0 * k)), "on the bar ({k})");
            assert!(on_bar(bar, (383.0 * k, 120.0 * k)), "just right of it ({k})");
            assert!(on_bar(bar, (368.0 * k, 390.0 * k)), "just left of it ({k})");
            assert!(!on_bar(bar, (300.0 * k, 200.0 * k)), "on a row ({k})");
            assert!(!on_bar(bar, (377.0 * k, 100.0 * k)), "above the track ({k})");
            assert!(!on_bar(bar, (377.0 * k, 410.0 * k)), "below the track ({k})");
            let thin = |[x, y, w, h]: [f32; 4], (mx, my): (f32, f32)| mx >= x && mx <= x + w && my >= y && my <= y + h;
            assert!(!thin(bar, (368.0 * k, 390.0 * k)), "without the margin a click beside it would reach a row");
        }
        // The list still moves by the wheel and the keys (no drag needed): the wheel keeps the
        // focus on a visible row.
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Wheel(3));
        assert_eq!(n.top, 3);
        assert!((n.top..n.top + 4).contains(&n.focus));
    }

    /// Every focus move ticks: moving to another visible row is a new focus (the row's own
    /// SetFocus); moving past the last visible row scrolls the list under the same visible
    /// row, which is focused again. Negative controls: no move (same row of the list) and a
    /// first focus give no extra tick; a move to another visible row is not focused again.
    #[test]
    fn a_focus_move_that_scrolls_ticks_too() {
        let mut n = Nav::new(sample(), 4);
        let (mut applied, mut applied_row) = (Some(0), Some(0));
        let mut again = 0;
        for _ in 0..10 {
            n.key(NavKey::Down);
            let want = n.focus.checked_sub(n.top);
            if focus_moved_in_place(applied, applied_row, want, n.focus) {
                again += 1;
            }
            (applied, applied_row) = (want, Some(n.focus));
        }
        assert_eq!(n.top, 7, "the list scrolled");
        assert_eq!(again, 7, "each of the 7 moves that scrolled the list ticks");
        assert!(!focus_moved_in_place(Some(3), Some(9), Some(3), 9), "no move");
        assert!(!focus_moved_in_place(None, None, Some(0), 0), "first focus");
        assert!(!focus_moved_in_place(Some(2), Some(5), Some(3), 6), "another visible row");
    }

    #[test]
    fn one_flat_list_in_story_order() {
        let n = Nav::new(sample(), 4);
        assert_eq!(n.starts.len(), 19, "every start and every checkpoint is a row");
        assert_eq!(names(&n.starts[..7]), ["A", "B", "B 1", "B 2", "C", "C 1", "C 2"]);
        assert_eq!(n.starts[2].url, "B#cp1");
        assert_eq!(names(n.visible_rows()), ["A", "B", "B 1", "B 2"]);
    }

    #[test]
    fn enter_and_click_choose_any_row() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        assert_eq!(n.focused().map(|s| s.name.as_str()), Some("B 1"));
        // Right and Left do nothing on the list (no more expanding).
        assert_eq!(n.key(NavKey::Right), Outcome::None);
        assert_eq!(n.key(NavKey::Left), Outcome::None);
        assert!(n.chosen.is_none());
        n.key(NavKey::Enter);
        assert_eq!(n.chosen, Some(("B#cp1".into(), "B 1".into())), "a checkpoint opens the equipment page");
        n.key(NavKey::Back);
        assert_eq!(n.focused().map(|s| s.name.as_str()), Some("B 1"), "back on the same row");
        n.key(NavKey::Up);
        n.key(NavKey::Enter);
        assert_eq!(n.chosen.as_ref().map(|c| c.0.as_str()), Some("B"), "a mission start is chosen at once");
        n.key(NavKey::Back);
        assert_eq!(n.key(NavKey::Back), Outcome::Close, "Escape on the list goes back");
    }

    #[test]
    fn scrolling_follows_the_focus_and_the_wheel() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::End);
        assert_eq!(n.focused().map(|s| s.name.as_str()), Some("J 1"));
        assert_eq!((n.top, n.focus), (15, 18));
        n.key(NavKey::Wheel(-3));
        assert_eq!(n.top, 12);
        assert_eq!(n.focus, 15, "the focus stays on a visible row");
        n.key(NavKey::Wheel(-100));
        assert_eq!(n.top, 0);
        n.key(NavKey::Wheel(100));
        assert_eq!(n.top, 15);
        for _ in 0..5 {
            n.key(NavKey::PageUp);
        }
        assert_eq!((n.top, n.focus), (0, 0));
        assert_eq!(n.scroll(), Some((0.0, 4.0 / 19.0)));
        n.key(NavKey::PageDown);
        assert_eq!((n.top, n.focus), (1, 4));
    }

    #[test]
    fn mouse_hover_and_click() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Wheel(2));
        n.key(NavKey::Hover(1));
        assert_eq!(n.focused().map(|s| s.name.as_str()), Some("B 2"));
        n.key(NavKey::Click(3));
        assert_eq!(n.chosen.as_ref().map(|c| c.0.as_str()), Some("C#cp1"), "one click chooses the row");
        n.key(NavKey::Back);
        assert_eq!(n.key(NavKey::Click(9)), Outcome::None, "past the visible rows: nothing");
        assert!(n.chosen.is_none());
    }

    #[test]
    fn the_mouse_leaving_a_weapon_hides_its_frame_until_it_or_a_key_comes_back() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Enter);
        assert!(n.gear_shown, "the equipment page opens with Start level shown focused");
        n.key(NavKey::HoverGear(Gear::Cell(3)));
        assert_eq!((n.gear, n.gear_shown), (Gear::Cell(3), true));
        n.key(NavKey::HoverNone);
        assert!(!n.gear_shown, "the mouse over nothing: no frame, no name");
        assert_eq!(n.key(NavKey::Enter), Outcome::None, "Enter on a hidden focus only shows it");
        assert_eq!((n.gear, n.gear_shown, n.weapons), (Gear::Cell(3), true, WEAPONS_DEFAULT));
        n.key(NavKey::HoverNone);
        n.key(NavKey::Right);
        assert_eq!((n.gear, n.gear_shown), (Gear::Cell(3), true), "the first arrow shows the focus where it was");
        n.key(NavKey::Right);
        assert_eq!(n.gear, Gear::Cell(4), "the next ones move it");
        n.key(NavKey::HoverNone);
        n.key(NavKey::HoverGear(Gear::All));
        assert_eq!((n.gear, n.gear_shown), (Gear::All, true), "the mouse back over an element takes over");
        n.key(NavKey::HoverNone);
        assert_eq!(n.key(NavKey::ClickGear(Gear::None)), Outcome::WeaponsChanged(0));
        assert!(n.gear_shown);
    }

    #[test]
    fn equipment_grid_toggles_and_starts() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Enter);
        assert_eq!(n.gear, Gear::Start, "Start level is focused first");
        assert_eq!(n.weapons, WEAPONS_DEFAULT);
        n.key(NavKey::Up);
        assert_eq!(n.gear, Gear::All);
        assert_eq!(n.key(NavKey::Enter), Outcome::WeaponsChanged(WEAPONS_ALL));
        n.key(NavKey::Right);
        assert_eq!(n.key(NavKey::Enter), Outcome::WeaponsChanged(0));
        // Up from None: the lowest weapon of None's first column (column 3: the last row is shorter).
        n.key(NavKey::Up);
        assert_eq!(n.gear, Gear::Cell(18));
        n.key(NavKey::Left);
        assert_eq!(n.gear, Gear::Cell(17));
        n.key(NavKey::Home);
        assert_eq!(n.gear, Gear::Cell(0));
        assert_eq!(n.key(NavKey::Enter), Outcome::WeaponsChanged(1));
        n.key(NavKey::Down);
        n.key(NavKey::Right);
        assert_eq!(n.gear, Gear::Cell(6));
        assert_eq!(n.key(NavKey::ClickGear(Gear::Cell(2))), Outcome::WeaponsChanged(0b101));
        n.key(NavKey::HoverGear(Gear::Cell(3)));
        assert_eq!(n.gear, Gear::Cell(3));
        // Down the column to its lowest weapon, then None below it, Start level, and the top again.
        for want in [Gear::Cell(8), Gear::Cell(13), Gear::Cell(18), Gear::None, Gear::Start, Gear::Cell(3)] {
            n.key(NavKey::Down);
            assert_eq!(n.gear, want);
        }
        n.key(NavKey::Up);
        assert_eq!(n.gear, Gear::Start, "up from the top row: the bottom-most control");
        assert_eq!(n.key(NavKey::Enter), Outcome::Start("A".into()));
        assert_eq!(n.key(NavKey::ClickGear(Gear::Start)), Outcome::Start("A".into()));
        n.key(NavKey::Back);
        assert!(n.chosen.is_none());
        assert_eq!(n.focus, 0);
    }

    /// Every element of the equipment page.
    fn every_gear() -> Vec<Gear> {
        (0..WEAPONS.len()).map(Gear::Cell).chain([Gear::All, Gear::None, Gear::Start]).collect()
    }

    #[test]
    fn equipment_arrows_always_land_on_an_element_and_go_round() {
        let n = WEAPONS.len();
        let dirs = [NavKey::Up, NavKey::Down, NavKey::Left, NavKey::Right];
        for g in every_gear() {
            for column in 0..GRID_COLUMNS {
                for k in dirs {
                    let mut c = column;
                    let next = gear_step(g, k, &mut c);
                    assert!(every_gear().contains(&next) && c < GRID_COLUMNS, "{g:?} {k:?}: {next:?}");
                    // No dead end: Up and Down always leave the element.
                    if matches!(k, NavKey::Up | NavKey::Down) {
                        assert_ne!(next, g, "{g:?} {k:?} stays");
                    }
                }
            }
        }
        // Pressing one arrow again and again comes back to where it started.
        for g in every_gear() {
            for k in dirs {
                let mut c = match g { Gear::Cell(i) => i % GRID_COLUMNS, Gear::None => COLUMN_OF_NONE, _ => 0 };
                let mut at = gear_step(g, k, &mut c);
                let mut steps = 1;
                while at != g {
                    at = gear_step(at, k, &mut c);
                    steps += 1;
                    assert!(steps <= n + 4, "{g:?} {k:?} never comes back");
                }
            }
        }
        // Left / Right go round each row, the short last row included.
        let mut c = 0;
        assert_eq!(gear_step(Gear::Cell(4), NavKey::Right, &mut c), Gear::Cell(0));
        assert_eq!(gear_step(Gear::Cell(5), NavKey::Left, &mut c), Gear::Cell(9));
        assert_eq!(gear_step(Gear::Cell(n - 1), NavKey::Right, &mut c), Gear::Cell(n - 1));
        // Up / Down go round a column through All or None and Start level.
        let mut c = 4;
        let mut at = Gear::Cell(4);
        let mut seen = Vec::new();
        for _ in 0..7 {
            at = gear_step(at, NavKey::Down, &mut c);
            seen.push(at);
        }
        assert_eq!(seen, [Gear::Cell(9), Gear::Cell(14), Gear::Cell(19), Gear::None, Gear::Start, Gear::Cell(4), Gear::Cell(9)]);
        let mut c = 0;
        assert_eq!(gear_step(Gear::Cell(20), NavKey::Down, &mut c), Gear::All);
        assert_eq!(gear_step(Gear::All, NavKey::Up, &mut c), Gear::Cell(20));
    }

    #[test]
    fn hover_frame_surrounds_the_cell_in_orange() {
        // 1280 x 720 (unit 1.2) and 1024 x 576 (unit 0.96): 4 and 3 pixels out, 4 and 3 thick.
        let cell = [100.0, 50.0, 64.0, 32.0];
        let (f, t) = hover_frame(cell, 1.2);
        assert_eq!((f, t), ([96.0, 46.0, 72.0, 40.0], 4.0));
        let (f, t) = hover_frame(cell, 0.96);
        assert_eq!((f, t), ([97.0, 47.0, 70.0, 38.0], 3.0));
        // Never thinner than 2 pixels, never over the cell itself.
        let (f, t) = hover_frame(cell, 0.3);
        assert!(t >= 2.0 && f[0] <= cell[0] && f[0] + f[2] >= cell[0] + cell[2]);
        assert_eq!(HOVER_FRAME_COLOUR, [247, 170, 30, 255], "the menus' orange chosen by the owner");
        assert_ne!(HOVER_FRAME_COLOUR, SELECTED_FILL, "distinct from a chosen weapon's yellow");
    }

    #[test]
    fn the_levels_list_goes_round() {
        let mut n = Nav::new(sample(), 4);
        n.key(NavKey::Up);
        assert_eq!((n.focus, n.top), (18, 15), "up from the first row: the last one, the list at its bottom");
        n.key(NavKey::Down);
        assert_eq!((n.focus, n.top), (0, 0), "down from the last row: the first one, the list at its top");
        // Every row by Down, then back to the first; the same by Up.
        for i in 1..=19 {
            n.key(NavKey::Down);
            assert_eq!(n.focus, i % 19);
            assert!(n.focus >= n.top && n.focus < n.top + n.visible);
        }
        for _ in 0..19 {
            n.key(NavKey::Up);
            assert!(n.focus >= n.top && n.focus < n.top + n.visible);
        }
        assert_eq!(n.focus, 0);
        // The wheel at the top goes to the bottom, and at the bottom back to the top.
        n.key(NavKey::Wheel(-1));
        assert_eq!((n.top, n.focus), (15, 18));
        n.key(NavKey::Wheel(1));
        assert_eq!((n.top, n.focus), (0, 0));
        n.key(NavKey::Wheel(1));
        assert_eq!(n.top, 1, "inside the list the wheel scrolls as before");
    }

    #[test]
    fn jones_padding_transparent_above_and_aside_last_row_repeated_below() {
        let mut img = Image { w: 2, h: 2, rgba: vec![255; 16] };
        img.rgba[3] = 0; // top-left texel transparent, bottom row opaque
        let p = pad(&img, 2);
        assert_eq!((p.w, p.h), (6, 6));
        let a = |x: u32, y: u32| p.rgba[((y * p.w + x) * 4 + 3) as usize];
        assert_eq!([a(2, 2), a(3, 2), a(2, 3), a(3, 3)], [0, 255, 255, 255]);
        assert!((0..6).all(|x| a(x, 0) == 0 && a(x, 1) == 0), "transparent above");
        assert!((0..6).all(|y| a(0, y) == 0 && a(1, y) == 0 && a(4, y) == 0 && a(5, y) == 0), "transparent aside");
        assert!([4, 5].iter().all(|&y| a(2, y) == 255 && a(3, y) == 255), "last row repeated below");
    }

    #[test]
    fn jones_tile_ends_on_the_frame_with_her_last_row() {
        // Padded picture 104 x 84 (figure texels 42..61 x 32..81 before the repeated rows).
        let figure = [42, 32, 61, 81];
        for (picture, bottom) in [([45.0, 41.25, 435.0, 491.25], 535.0), ([36.0, 33.0, 348.0, 393.0], 428.0), ([67.5, 49.5, 522.0, 589.5], 642.0)] {
            let (r, uv) = select_level_art::jones_tile(picture, bottom, (104, 84), figure);
            let s = r[3] / uv[3];
            assert!((r[2] / uv[2] - s).abs() < 1e-4, "aspect kept");
            assert_eq!(uv[1] + uv[3], 82.0, "window ends on her last row");
            assert!((r[1] + r[3] - bottom).abs() < 1e-3, "her last row ends on the bottom edge");
            assert!(((figure[3] + 1 - figure[1]) as f32 * s - select_level_art::FIGURE_HEIGHT * picture[3]).abs() < 1e-3);
            assert!((r[0] + (figure[0] as f32 - uv[0]) * s - (picture[0] + picture[2] - select_level_art::FIGURE_OVERLAP * picture[3])).abs() < 1e-3);
            assert_eq!((uv[0], uv[1], uv[2]), (1.0, 1.0, 102.0));
        }
    }

    #[test]
    fn check_mark_is_a_tick() {
        let m = check_mark(32);
        let grey = |x: u32, y: u32| m.rgba[((y * 32 + x) * 4) as usize];
        // Black on both strokes, at their corner and on the frame; white inside elsewhere.
        assert_eq!([grey(9, 20), grey(13, 24), grey(20, 16)], [0; 3]);
        assert_eq!([grey(0, 0), grey(31, 31), grey(0, 15), grey(15, 31)], [0; 4]);
        assert_eq!([grey(5, 5), grey(26, 27), grey(4, 28)], [255; 3]);
        assert!(m.rgba.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
    }

    #[test]
    fn chosen_icon_has_the_mark_in_its_top_right_corner() {
        let icon = Image { w: 64, h: 32, rgba: vec![100; 64 * 32 * 4] };
        let chosen = with_check_mark(&icon);
        assert_eq!((chosen.w, chosen.h), (64, 32));
        let px = |x: u32, y: u32| &chosen.rgba[((y * 64 + x) * 4) as usize..((y * 64 + x) * 4 + 4) as usize];
        // One texel in from the top and right edges (with_check_mark).
        assert_eq!(px(62, 1), [0, 0, 0, 255], "the mark's frame");
        assert_eq!(px(55, 10), [0, 0, 0, 255], "the tick");
        assert_eq!(px(0, 0), [100; 4], "the icon elsewhere");
        assert_eq!(px(63, 0), [100; 4], "the corner texel, outside the mark");
        assert_eq!(px(62, 14), [100; 4], "below the mark");
    }

    #[test]
    fn icons_cover_every_weapon() {
        assert_eq!(ICONS.len(), WEAPONS.len());
        assert!(ICONS.iter().all(|&(w, c)| if w { c < 32 } else { c < 16 }));
    }

    #[test]
    fn missions_from_the_table() {
        if !crate::select_level::available() {
            eprintln!("skipped: {}", crate::select_level::UNAVAILABLE);
            return;
        }
        let m = missions();
        assert_eq!(m.len(), 34);
        assert_eq!(m.iter().map(|m| m.checkpoints.len()).sum::<usize>(), 39);
        // The page's flat list: the 34 starts and the 39 checkpoints, each mission's start first.
        let s = starts();
        assert_eq!(s.len(), 73);
        assert_eq!((s[1].name.as_str(), s[2].name.as_str(), s[2].url.as_str()), ("Brighton Beach 2", "Brighton Beach 3", "Plage01#teleporte_apres_lit"));
        assert_eq!((m[0].name.as_str(), m[0].url.as_str()), ("Brighton Beach 1", "Plage00"));
        assert_eq!(m[1].checkpoints, [("Brighton Beach 3".to_string(), "Plage01#teleporte_apres_lit".to_string())]);
        assert_eq!(m[7].name, "Carrington's cell", "the description's trailing space is dropped");
        assert_eq!(m.last().map(|m| m.url.as_str()), Some("Bateau01"));
        for x in &m {
            assert!(crate::select_level::start_point(&x.url).is_some(), "{}", x.url);
            for (_, url) in &x.checkpoints {
                assert!(crate::select_level::start_point(url).is_some(), "{url}");
            }
        }
    }

    /// Rows drawn focused (white): only one, the controller's focused control.
    fn focused_rows(vm: &Vm, ctrl: ObjId, rows: &[ObjId]) -> (Vec<usize>, Option<usize>) {
        let lit = (0..rows.len()).filter(|&i| vm.propr(rows[i], "bHasFocus").as_bool() || vm.propr(rows[i], "MenuState").as_byte() == 2).collect();
        let held = vm.propr(ctrl, "FocusedControl").as_object().and_then(|f| rows.iter().position(|&r| r == f));
        (lit, held)
    }

    /// The check mark never touches the picture's top or right edge (the wrapping texture's filter
    /// would draw its black frame on the opposite edges), and is still in the top right corner.
    /// Negative control: the mark at the very edge (INSET 0) fails the edge checks.
    #[test]
    fn check_mark_keeps_off_the_edges() {
        for (w, h) in [(32, 32), (64, 32), (16, 16)] {
            let mut img = Image::new(w, h);
            img.rgba.iter_mut().for_each(|v| *v = 0);
            let out = with_check_mark(&img);
            let alpha = |x: u32, y: u32| out.rgba[((y * w + x) * 4 + 3) as usize];
            assert!((0..w).all(|x| alpha(x, 0) == 0), "{w}x{h}: top row");
            assert!((0..h).all(|y| alpha(w - 1, y) == 0), "{w}x{h}: right column");
            assert!(alpha(w - 2, 1) > 0, "{w}x{h}: the mark's corner");
        }
    }

    /// Neighbouring cells' hit rectangles meet: the gap between two cells, across and down, is
    /// always over one of them, and the cell itself stays inside its rectangle.
    #[test]
    fn cell_hit_rects_leave_no_gap() {
        let (w, h, px_, py_) = (CELL_W * 1.7, CELL_H * 1.3, CELL_PITCH_X * 1.7, CELL_PITCH_Y * 1.3);
        let gap = (px_ - w, py_ - h);
        let a = cell_hit_rect([10.0, 20.0, w, h], gap);
        let right = cell_hit_rect([10.0 + px_, 20.0, w, h], gap);
        let below = cell_hit_rect([10.0, 20.0 + py_, w, h], gap);
        assert!((a[0] + a[2] - right[0]).abs() < 1e-3, "across: {a:?} {right:?}");
        assert!((a[1] + a[3] - below[1]).abs() < 1e-3, "down: {a:?} {below:?}");
        assert!(a[0] < 10.0 && a[1] < 20.0 && a[0] + a[2] > 10.0 + w && a[1] + a[3] > 20.0 + h);
        // Negative control: the drawn cells alone leave the gap.
        assert!(10.0 + w < 10.0 + px_);
    }

    #[test]
    fn mouse_gear_tick_rule() {
        let c = |i| Some((Gear::Cell(i), true));
        assert!(mouse_gear_tick(c(1), c(2)));
        assert!(!mouse_gear_tick(c(1), c(1)));
        assert!(!mouse_gear_tick(c(1), Some((Gear::Cell(1), false))), "leaving every element");
        assert!(mouse_gear_tick(Some((Gear::Cell(1), false)), c(1)), "back onto the focused element after nothing");
        assert!(mouse_gear_tick(Some((Gear::Cell(1), false)), c(3)), "from nothing onto another element");
        assert!(!mouse_gear_tick(None, c(1)) && !mouse_gear_tick(c(1), None), "the page opening or closing");
    }
}
