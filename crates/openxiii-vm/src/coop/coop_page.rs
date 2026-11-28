//! Improved build only: the Co-op page (opened by the main menu's Co-op box) and the
//! "Players on this screen" setting of the multiplayer pages.
//!
//! The Co-op page is a real menu page of the game (an XIIIWindowMainMenu, built like the Select
//! level page: white page, picture panel, title label, rows in the Load page's save slot style,
//! the game's own help bar). It has two steps:
//! 1. how to play together: Split-screen, Host LAN game, Join LAN game;
//! 2. which game (not to join: the host chooses): New game, Continue, Load, Select level, each
//!    the menu's own flow (the new-game video, the Continue page, the Load page, the Select level
//!    page), opened over this page.
//!
//! While the network co-op is not offered ([`LAN_COOP_READY`]), the first step is left out: the
//! page opens on the split-screen game step.
//!
//! What is chosen is the co-op session of [`crate::local_play`]: set when a way to play together
//! is chosen, its start when a game is chosen, cleared by Escape back to the first step and when
//! the page is left. Escape goes back one step; on the first step it closes the page.
//!
//! A step to choose the characters will come after the game is chosen (and directly after Join):
//! [`Nav::after_game_choice`] is where it goes in.
//!
//! The multiplayer pages Host LAN and Bot challenge get a "Players on this screen" control of
//! their own style (XIIIValueControl), kept in [`crate::local_play::LocalPlay`]. The Join LAN page
//! has the address box in its place (multi_join.rs): joining with two players on this screen
//! would need one connection per player.

use crate::added_text::{text, Text};
use crate::local_play::{CoopMode, CoopSession, CoopStart};
use crate::menu_boxes::Image;
use crate::split_layout::SplitLayout;
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// The co-op campaign over the local network (coop_address.rs, played in lockstep: lockstep.rs)
/// is offered on the page. While it is not, the page has no step to choose how to play together:
/// it opens on the split-screen game step, and Escape there closes it. The network co-op code
/// stays in the game, out of the menus.
pub const LAN_COOP_READY: bool = false;

/// File keeping the last host address typed to join (set by the game window; None: not kept).
static JOIN_FILE: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Where the last host address typed on the page is kept between runs.
pub fn set_join_file(path: Option<std::path::PathBuf>) {
    if let Ok(mut f) = JOIN_FILE.lock() {
        *f = path;
    }
    // The split setting kept from the last time (Auto the first time).
    crate::split_layout::set_split_layout(last_split_layout());
}

/// How the split screen is cut, kept ("SplitLayout=..." in the join file), Auto without one.
pub fn last_split_layout() -> SplitLayout {
    let path = JOIN_FILE.lock().ok().and_then(|f| f.clone());
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("SplitLayout=").map(SplitLayout::from_text)))
        .unwrap_or(SplitLayout::Auto)
}

/// Last host address kept ("Address=..." in the join file), empty without one.
pub fn last_join_address() -> String {
    let path = JOIN_FILE.lock().ok().and_then(|f| f.clone());
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("Address=").map(|a| a.trim().to_string())))
        .unwrap_or_default()
}

fn keep_join_address(address: &str) {
    keep_page_value("Address", address);
}

/// Characters the co-op name box takes, and its longest name.
const NAME_CHARS: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-_. '!?àâäçéèêëîïôöùûüÀÂÄÇÉÈÊËÎÏÔÖÙÛÜ";
const NAME_MAX: i32 = 16;

/// The name kept for `kept` (the Co-op page's box), else the multiplayer name (`multi`).
pub fn coop_name_from(kept: Option<String>, multi: Option<String>) -> String {
    kept.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).or_else(|| multi.map(|m| m.trim().to_string()).filter(|m| !m.is_empty())).unwrap_or_default()
}

/// Extra enemies per extra player kept ("ExtraEnemies=..." in the join file), 0 without one.
pub fn last_extra_enemies() -> u32 {
    let path = JOIN_FILE.lock().ok().and_then(|f| f.clone());
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("ExtraEnemies=").and_then(|v| v.trim().parse::<u32>().ok())))
        .filter(|v| EXTRA_ENEMIES.contains(v))
        .unwrap_or(0)
}

/// One of the page's kept values ("Key=..." in the join file), None without one (or without a file).
pub(crate) fn page_value(key: &str) -> Option<String> {
    let path = JOIN_FILE.lock().ok().and_then(|f| f.clone())?;
    let prefix = format!("{key}=");
    std::fs::read_to_string(path).ok()?.lines().find_map(|l| l.trim().strip_prefix(prefix.as_str()).map(|a| a.trim().to_string()))
}

/// One of the page's kept values set in the join file, the others kept.
pub(crate) fn keep_page_value(key: &str, value: &str) {
    let Some(path) = JOIN_FILE.lock().ok().and_then(|f| f.clone()) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::write(path, page_values_text(&old, key, value));
}

/// The join file's text with `key` set to `value` (other lines kept, the section first).
fn page_values_text(old: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut out = String::from("[CoopJoin]\r\n");
    for l in old.lines().map(str::trim).filter(|l| !l.is_empty() && !l.eq_ignore_ascii_case("[CoopJoin]") && !l.starts_with(&prefix)) {
        out.push_str(l);
        out.push_str("\r\n");
    }
    out.push_str(&format!("{prefix}{value}\r\n"));
    out
}

/// Values of the extra enemies setting (percent more per extra player): none, +50 %, +100 %.
pub const EXTRA_ENEMIES: [u32; 3] = [0, 50, 100];

/// Characters the address box takes: an IPv4 address or a host name, and a port after ':'.
const ADDRESS_CHARS: &str = "0123456789.:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-_";
/// Longest address the box takes.
const ADDRESS_MAX: i32 = 40;
/// Multiplayer page whose games have a second player on the same screen (split_versus.rs) by
/// its own travel URL: the offline one, Bot challenge.
pub const LOCAL_PLAYERS_PAGE: &str = "XIIIMenuBotChallengeSetupClient";
/// Hosting a LAN game also has them (split_versus.rs: a listen server's players on this screen);
/// its URL stays the page's own (there NP is the most players of the game).
pub const LOCAL_PLAYERS_HOST_PAGE: &str = "XIIIMenuMultiLANHost";

/// The setting can be changed on this multiplayer page.
pub fn local_players_ready(class: &str) -> bool {
    class.eq_ignore_ascii_case(LOCAL_PLAYERS_PAGE) || class.eq_ignore_ascii_case(LOCAL_PLAYERS_HOST_PAGE)
}

/// The travel URL of the Bot challenge page (XIIIMenuBotChallengeSetupClient.StartPressed, built
/// for one player) made the split-screen setup's (XIIIMenuSplitSetupClient.StartPressed) for
/// `players` players on this screen: `NP` is their number and `NBots` is kept at most the map's
/// ideal player count (`IP`) minus them, as that page does. The bots' teams of the default set
/// are the same on both pages for two players (1, 0, 1, 0), and the players' teams default to 0
/// and 1. Any other option is left as it is.
pub fn split_travel_url(url: &str, players: u8) -> String {
    let mut parts: Vec<String> = url.split('?').map(str::to_string).collect();
    let value = |parts: &[String], key: &str| -> Option<i32> {
        parts.iter().skip(1).find_map(|p| p.split_once('=').filter(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v.trim().parse().ok()))
    };
    let ideal = value(&parts, "IP");
    let n = i32::from(players.max(1));
    let mut has_np = false;
    let mut has_bots = false;
    for p in parts.iter_mut().skip(1) {
        let Some((k, v)) = p.split_once('=') else { continue };
        if k.eq_ignore_ascii_case("NP") {
            *p = format!("{k}={n}");
            has_np = true;
        } else if k.eq_ignore_ascii_case("NBots") {
            let bots = v.trim().parse::<i32>().unwrap_or(0);
            let bots = match ideal {
                Some(ip) => bots.min(ip - n).max(0),
                None => bots.max(0),
            };
            *p = format!("{k}={bots}");
            has_bots = true;
        }
    }
    if !has_np {
        parts.push(format!("NP={n}"));
    }
    if !has_bots {
        // Always given in split screen: the map's default counts one human only.
        parts.push(format!("NBots={}", ideal.map_or(0, |ip| (ip - n).max(0))));
    }
    parts.join("?")
}

/// An entry of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    SplitScreen,
    LanHost,
    LanJoin,
    NewGame,
    Continue,
    Load,
    SelectLevel,
    /// Joining: the host's address (the game's own text box, XIIIEditCtrl).
    Address,
    /// Joining: go.
    JoinNow,
    /// Improved build, choosing the game: extra enemies per extra player (a value changed with
    /// left / right, not a choice).
    ExtraEnemies,
    /// Improved build, choosing a split-screen game: how the screen is cut (split_layout.rs), a
    /// value changed like the one above.
    SplitLayout,
    /// The name this player has in the LAN co-op chat (a text box, like the address).
    CoopName,
    /// Joining: a co-op game found on the network (lockstep_find.rs), by its place in the list.
    Found(u8),
}

/// Joining: the co-op games found on the network shown at most (under the address box).
pub const FOUND_ROWS: usize = 5;
/// Joining: the search for the games on the network that could not start is tried again after
/// this long.
const FINDER_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
/// The join step: the address box, its Join row, then the games found.
const JOIN_ENTRIES: [Entry; 2 + FOUND_ROWS] = [Entry::Address, Entry::JoinNow, Entry::Found(0), Entry::Found(1), Entry::Found(2), Entry::Found(3), Entry::Found(4)];

/// The game step of a split-screen session: the extra enemies, then how the screen is cut.
const GAME_SPLIT_ENTRIES: [Entry; 6] = [Entry::NewGame, Entry::Continue, Entry::Load, Entry::SelectLevel, Entry::ExtraEnemies, Entry::SplitLayout];
/// The game step of a LAN host, its setting first. A network co-op game is played in lockstep
/// (lockstep.rs): one player on each PC, two in all.
const GAME_HOST_ENTRIES: [Entry; 5] = [Entry::ExtraEnemies, Entry::NewGame, Entry::Continue, Entry::Load, Entry::SelectLevel];

/// The page's steps: how to play together, then which game (or, to join, the host's address).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Mode,
    Game,
    Join,
}

impl Step {
    pub fn entries(self) -> &'static [Entry] {
        match self {
            Step::Mode => &[Entry::SplitScreen, Entry::LanHost, Entry::LanJoin, Entry::CoopName],
            Step::Game => &[Entry::NewGame, Entry::Continue, Entry::Load, Entry::SelectLevel, Entry::ExtraEnemies],
            Step::Join => &[Entry::Address, Entry::JoinNow],
        }
    }
}

/// Input of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavKey {
    Up,
    Down,
    Home,
    End,
    Enter,
    Back,
    Left,
    Right,
    /// The mouse moved over this row.
    Hover(usize),
    /// Click on this row.
    Click(usize),
}

/// What an input asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// A way to play together was chosen (the page shows the next step, or plays at once).
    Mode(CoopMode),
    /// Back to the first step: the session is cleared.
    BackToMode,
    /// Leave the page (Escape on the first step): the session is cleared.
    Close,
    /// Everything is chosen: start this game (None: join a host).
    Play(Option<CoopStart>),
    /// The extra enemies setting changed to this percentage.
    ExtraEnemies(u32),
    /// The split setting changed to this value.
    SplitLayout(SplitLayout),
    /// Join the game found at this place of the list.
    JoinFound(usize),
}

/// Navigation of the page, without the VM.
#[derive(Clone, Debug)]
pub struct Nav {
    pub step: Step,
    pub focus: usize,
    /// Entries shown greyed (their game mode or page does not exist): never focused or chosen.
    pub greyed: Vec<Entry>,
    pub mode: Option<CoopMode>,
    /// Extra enemies per extra player, in percent (one of EXTRA_ENEMIES).
    pub extra_enemies: u32,
    /// How the split screen is cut.
    pub split_layout: SplitLayout,
    /// Joining: the games found on the network shown (at most FOUND_ROWS).
    pub found: usize,
    /// The page has its first step (how to play together); without it, split screen only.
    pub mode_step: bool,
}

impl Nav {
    pub fn new(greyed: Vec<Entry>) -> Nav {
        let mut n = Nav { step: Step::Mode, focus: 0, greyed, mode: None, extra_enemies: 0, split_layout: SplitLayout::Auto, found: 0, mode_step: true };
        n.focus = n.first_enabled();
        n
    }

    /// The page without its first step: a split-screen session, on its game step.
    pub fn split_only(greyed: Vec<Entry>) -> Nav {
        let mut n = Nav::new(greyed);
        n.mode_step = false;
        n.step = Step::Game;
        n.mode = Some(CoopMode::SplitScreen);
        n.focus = n.first_choice();
        n
    }

    pub fn entries(&self) -> &'static [Entry] {
        match (self.step, self.mode) {
            (Step::Game, Some(CoopMode::SplitScreen)) => &GAME_SPLIT_ENTRIES,
            (Step::Game, Some(CoopMode::LanHost)) => &GAME_HOST_ENTRIES,
            (Step::Join, _) => &JOIN_ENTRIES[..2 + self.found.min(FOUND_ROWS)],
            _ => self.step.entries(),
        }
    }

    /// Joining: the games found on the network now, by whether this build can join each one
    /// (the others are shown greyed). The focus stays on its row while there is one.
    pub fn set_found(&mut self, joinable: &[bool]) {
        self.found = joinable.len().min(FOUND_ROWS);
        self.greyed.retain(|e| !matches!(e, Entry::Found(_)));
        self.greyed.extend(joinable.iter().take(FOUND_ROWS).enumerate().filter(|(_, &ok)| !ok).map(|(i, _)| Entry::Found(i as u8)));
        if self.step == Step::Join {
            let n = self.entries().len();
            if self.focus >= n || !self.enabled(self.entries()[self.focus]) {
                self.focus = 1.min(n - 1);
            }
        }
    }

    pub fn enabled(&self, e: Entry) -> bool {
        !self.greyed.contains(&e)
    }

    pub fn focused(&self) -> Option<Entry> {
        self.entries().get(self.focus).copied()
    }

    fn first_enabled(&self) -> usize {
        self.entries().iter().position(|&e| self.enabled(e)).unwrap_or(0)
    }

    /// Where a step starts: on its first game (New game) when it has one, the settings above it
    /// being changed only when wanted; otherwise on its first enabled row.
    fn first_choice(&self) -> usize {
        self.entries().iter().position(|&e| e == Entry::NewGame && self.enabled(e)).unwrap_or_else(|| self.first_enabled())
    }

    /// Rows of the step that are settings shown above its games (a LAN host's).
    pub fn settings_on_top(&self) -> usize {
        let e = self.entries();
        match e.iter().position(|&x| x == Entry::NewGame) {
            Some(i) if i > 0 => i,
            _ => 0,
        }
    }

    /// Next enabled row from `from` in direction `step` (+1 / -1), wrapping like the game's menus.
    fn next_enabled(&self, from: usize, step: isize) -> usize {
        let n = self.entries().len() as isize;
        let mut i = from as isize;
        for _ in 0..n {
            i = (i + step).rem_euclid(n);
            if self.enabled(self.entries()[i as usize]) {
                return i as usize;
            }
        }
        from
    }

    /// Enter or a click on an enabled row.
    fn choose(&mut self, i: usize) -> Outcome {
        let Some(&e) = self.entries().get(i).filter(|&&e| self.enabled(e)) else { return Outcome::None };
        self.focus = i;
        let start = match e {
            Entry::SplitScreen | Entry::LanHost | Entry::LanJoin => {
                let m = match e {
                    Entry::SplitScreen => CoopMode::SplitScreen,
                    Entry::LanHost => CoopMode::LanHost,
                    _ => CoopMode::LanJoin,
                };
                self.mode = Some(m);
                // Joining: the host chooses the game, this page asks for the host's address.
                self.step = if m == CoopMode::LanJoin { Step::Join } else { Step::Game };
                self.focus = self.first_choice();
                return Outcome::Mode(m);
            }
            // The text boxes take Enter themselves (end of typing).
            Entry::Address | Entry::CoopName => return Outcome::None,
            Entry::JoinNow => return self.after_game_choice(None),
            Entry::Found(k) => return Outcome::JoinFound(usize::from(k)),
            // Enter or a click on the setting: its next value.
            Entry::ExtraEnemies => return self.change_extra_enemies(1),
            Entry::SplitLayout => return self.change_split_layout(1),
            Entry::NewGame => CoopStart::NewGame,
            Entry::Continue => CoopStart::Continue,
            Entry::Load => CoopStart::Load,
            Entry::SelectLevel => CoopStart::SelectLevel,
        };
        self.after_game_choice(Some(start))
    }

    /// The extra enemies setting moved by step values (wrapping).
    fn change_extra_enemies(&mut self, step: isize) -> Outcome {
        let n = EXTRA_ENEMIES.len() as isize;
        let i = EXTRA_ENEMIES.iter().position(|&v| v == self.extra_enemies).unwrap_or(0) as isize;
        self.extra_enemies = EXTRA_ENEMIES[(i + step).rem_euclid(n) as usize];
        Outcome::ExtraEnemies(self.extra_enemies)
    }

    /// The split setting moved by step values (wrapping).
    fn change_split_layout(&mut self, step: isize) -> Outcome {
        self.split_layout = self.split_layout.step(step);
        Outcome::SplitLayout(self.split_layout)
    }

    /// The game is chosen (None: joining). The step to choose the characters goes in here: it
    /// will show its own rows and return `Outcome::Play` once both players have chosen.
    pub fn after_game_choice(&mut self, start: Option<CoopStart>) -> Outcome {
        Outcome::Play(start)
    }

    pub fn key(&mut self, k: NavKey) -> Outcome {
        let n = self.entries().len();
        match k {
            NavKey::Up => self.focus = self.next_enabled(self.focus, -1),
            NavKey::Down => self.focus = self.next_enabled(self.focus, 1),
            NavKey::Home => self.focus = self.first_enabled(),
            NavKey::End => self.focus = (0..n).rev().find(|&i| self.enabled(self.entries()[i])).unwrap_or(self.focus),
            NavKey::Hover(i) => {
                if self.entries().get(i).is_some_and(|&e| self.enabled(e)) {
                    self.focus = i;
                }
            }
            NavKey::Enter => return self.choose(self.focus),
            NavKey::Left | NavKey::Right if self.focused() == Some(Entry::ExtraEnemies) => {
                return self.change_extra_enemies(if k == NavKey::Left { -1 } else { 1 });
            }
            NavKey::Left | NavKey::Right if self.focused() == Some(Entry::SplitLayout) => {
                return self.change_split_layout(if k == NavKey::Left { -1 } else { 1 });
            }
            NavKey::Left | NavKey::Right => {}
            NavKey::Click(i) => return self.choose(i),
            NavKey::Back => {
                return match self.step {
                    Step::Mode => Outcome::Close,
                    // No first step to go back to.
                    _ if !self.mode_step => Outcome::Close,
                    Step::Game | Step::Join => {
                        // Back on the way to play that was chosen.
                        let row = match self.mode {
                            Some(CoopMode::LanHost) => 1,
                            Some(CoopMode::LanJoin) => 2,
                            _ => 0,
                        };
                        self.step = Step::Mode;
                        self.mode = None;
                        self.focus = row;
                        Outcome::BackToMode
                    }
                };
            }
        }
        Outcome::None
    }
}

// Layout, in the page's 640 x 480 units (heights multiplied by fScaleTo): a black picture area
// ending above the help bar, the game's press-start picture at its right (the characters, the
// logo left out), the title over its top edge, the entries as bands over the black at its left.
const AREA: [f32; 4] = [24.0, 56.0, 592.0, 380.0];
const TITLE: [f32; 4] = [235.0, 40.0, 170.0, 40.0];
/// Part of the press-start picture shown, from its left edge (a fraction of its width): up to a
/// little past XIII's pistol, the XIII logo at its bottom right left out.
const PICTURE_RIGHT: f32 = 0.665;
/// Width of the picture's left edge faded into black (a fraction of the shown width).
const PICTURE_FADE: f32 = 0.25;
/// Bands: left, top of the first, width, height and gap between them (left of the characters).
const BAND_X: f32 = 66.0;
const BAND_Y: f32 = 150.0;
const BAND_W: f32 = 300.0;
const BAND_H: f32 = 30.0;
const BAND_GAP: f32 = 8.0;
/// The address box keeps room for its title and an address.
const ADDRESS_W: f32 = 320.0;
const ROWS: usize = 8;
/// Hosting a LAN game (its step's own layout, under the lines giving the address): left of the
/// lines and of the hint, centre of each line, top of the first band, gap between bands, extra
/// gap between the settings and the games, height of a band.
const HOST_TEXT_X: f32 = 48.0;
const HOST_LINES_Y: [f32; 3] = [92.0, 110.0, 130.0];
const HOST_LINE_GAP: f32 = 13.0;
const HOST_BAND_Y: f32 = 158.0;
const HOST_BAND_PITCH: f32 = 29.0;
const HOST_SETTINGS_GAP: f32 = 8.0;
const HOST_BAND_H: f32 = 25.0;
/// Centre of the hint's first line (it ends above the area's bottom edge).
const HOST_HINT_Y: f32 = 404.0;
/// Width the lines and the hint wrap at (left of the characters' faces).
const HOST_TEXT_W: f32 = 290.0;
/// Joining: centres of the two lines above the address box.
const JOIN_LINES_Y: [f32; 2] = [104.0, 123.0];
/// Light grey of the second lines and hints.
const HINT_TEXT: [u8; 4] = [175, 175, 175, 255];
const ROW_INSET: f32 = 10.0;
/// Text of a greyed entry, and of its hint.
const GREYED_TEXT: [u8; 4] = [120, 120, 120, 255];
/// Text of the other bands over the black area.
const BAND_TEXT: [u8; 4] = [235, 235, 235, 255];

/// The press-start picture: the four textures the game's XIIIMenuPressStart draws in a 2 x 2 grid.
const PRESS_START: [&str; 4] = ["XIIIMenuStart.presstart01", "XIIIMenuStart.presstart02", "XIIIMenuStart.presstart03", "XIIIMenuStart.presstart04"];
/// Texture name of the picture (a number added for each one registered).
const PRESS_START_TEXTURE: &str = "Mod.CoopPage.PressStart";

/// Row `i`'s rectangle (640 x 480 units, unscaled heights): x, y, width, height.
fn row_rect(i: usize) -> [f32; 4] {
    [BAND_X, BAND_Y + i as f32 * (BAND_H + BAND_GAP), BAND_W, BAND_H]
}

/// Row `i`'s rectangle in the step in front: a LAN host's game step has its own layout (lower,
/// tighter bands, its settings apart from its games).
fn step_row_rect(nav: &Nav, i: usize) -> [f32; 4] {
    if nav.step == Step::Game && nav.mode == Some(CoopMode::LanHost) {
        let gap = if i >= nav.settings_on_top() { HOST_SETTINGS_GAP } else { 0.0 };
        [BAND_X, HOST_BAND_Y + i as f32 * HOST_BAND_PITCH + gap, BAND_W, HOST_BAND_H]
    } else {
        row_rect(i)
    }
}

/// `text` cut into lines no wider than `width` pixels, at spaces (`size` gives a text's width).
fn wrap_words(text: &str, width: f32, mut size: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let tried = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && size(&tried) > width {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = tried;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The Co-op page in front: its object, rows, navigation and picture.
#[derive(Default)]
pub struct CoopPage {
    pending: bool,
    pub(crate) page: Option<ObjId>,
    rows: Vec<ObjId>,
    /// The rows' own text colour (given back when a row is no longer greyed).
    row_text: Option<Value>,
    pub nav: Option<Nav>,
    applied: Option<usize>,
    background_serial: u32,
    /// The press-start picture (texture, size in pixels).
    press_start: Option<(Value, (u32, u32))>,
    /// The host's address box (XIIIEditCtrl), shown in place of the first row to join.
    address: Option<ObjId>,
    /// The co-op name box (XIIIEditCtrl), shown in place of its row on the first step, and the
    /// name last kept from it.
    name: Option<ObjId>,
    name_kept: Option<String>,
    /// This computer's addresses on the local network (coop_address::lan_addresses), found when
    /// hosting is chosen.
    lan_addresses: Vec<String>,
    /// Joining: the search for the co-op games on the network while the join step is shown, and
    /// what it found (lockstep_find.rs).
    finder: Option<crate::lockstep_find::Finder>,
    found: Vec<crate::lockstep_find::Found>,
    /// When the search last failed to start (tried again a few seconds later, said once).
    finder_failed: Option<std::time::Instant>,
}

impl CoopPage {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending
    }
}

/// Classes of the multiplayer pages that get the setting, and where it goes (640 x 480 units,
/// top multiplied by fScaleTo): x, y, width, height.
const PLAYERS_SETTING: [(&str, [f32; 4]); 2] = [
    ("XIIIMenuMultiLANHost", [344.0, 22.0, 262.0, 28.0]),
    ("XIIIMenuBotChallengeSetupClient", [288.0, 60.0, 300.0, 28.0]),
];

/// Narrowest setting box whose label also takes the "(coming soon)" hint.
const HINT_MIN_WIDTH: f32 = 300.0;

/// Where a multiplayer page gets the setting (None for any other class).
pub(crate) fn players_setting_rect(class: &str) -> Option<[f32; 4]> {
    PLAYERS_SETTING.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)).map(|(_, r)| *r)
}

/// Functions hooked on the multiplayer pages: on the page where the setting can be changed, also
/// its keys (the arrows on the setting) and its start (the split-screen travel URL).
pub(crate) fn players_setting_hooked(class: &str, name: &str) -> bool {
    players_setting_rect(class).is_some()
        && (matches!(name, "Created" | "Paint") || (local_players_ready(class) && matches!(name, "InternalOnKeyEvent" | "StartPressed")))
}

impl Vm {
    /// The Co-op page in front.
    pub fn coop_page_in_front(&self) -> Option<ObjId> {
        let page = self.improved.menu_boxes.coop_page.page?;
        let active = self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object())?;
        (active == page && self.objects.get(page).is_some_and(|o| o.is_some())).then_some(page)
    }

    /// Entries greyed on this computer.
    fn coop_greyed(&self) -> Vec<Entry> {
        let mut g = Vec::new();
        if !LAN_COOP_READY {
            g.extend([Entry::LanHost, Entry::LanJoin, Entry::CoopName]);
        }
        if !crate::select_level::available() {
            g.push(Entry::SelectLevel);
        }
        g
    }

    /// Opens the Co-op page over the main menu. False without a menu.
    pub fn open_coop_page(&mut self) -> bool {
        if self.coop_page_in_front().is_some() {
            return true;
        }
        let (Some(root), Some(class)) = (self.gui_local(), self.class("XIIIWindowMainMenu")) else { return false };
        self.change_local_play(|p| p.coop = None);
        self.improved.menu_boxes.coop_page.pending = true;
        let opened = self.call_named(root, "OpenMenuWithClass", vec![Value::Class(Some(class))]).map(|v| v.as_bool()).unwrap_or(false);
        self.improved.menu_boxes.coop_page.pending = false;
        let page = self.propr(root, "ActivePage").as_object();
        if !opened || page.is_none() || page != self.improved.menu_boxes.coop_page.page {
            self.warn("Co-op: page not opened".to_string());
            return false;
        }
        true
    }

    /// Hooks of the page's XIIIWindow functions (menu_boxes_call), as the Select level page's.
    #[allow(clippy::type_complexity)]
    pub(crate) fn coop_page_call(
        &mut self,
        f: usize,
        this: ObjId,
        name: &str,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        match name {
            "Created" if self.improved.menu_boxes.coop_page.pending && self.is_a(this, "XIIIWindowMainMenu") => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.keep_out_of_saved_menu_stack(this);
                    self.coop_page_created(this);
                }
                Ok(r)
            }
            _ if Some(this) != self.improved.menu_boxes.coop_page.page => Err(args),
            "Paint" | "AfterPaint" => {
                let canvas = args.first().cloned().flatten().and_then(|v| v.as_object());
                let r = self.call_inner(f, this, args);
                if let Some(c) = canvas {
                    if name == "Paint" {
                        self.coop_page_paint(this, c);
                    } else {
                        self.coop_page_after_paint(this, c);
                    }
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
                // Improved menus (menu_mouse.rs): Enter, Backspace, Escape and the click pressed act
                // here without XIIIWindow.InternalOnKeyEvent, as the game's pages handle their
                // press and return before it: its MnValid / MnAnnul come with the key's release
                // only, once (they came with the press too).
                if state == 1 && matches!(key, 0x0D | 0x08 | 0x1B | 0x01) && crate::menu_mouse::enabled(self.engine.client.session_graphics) {
                    self.coop_page_key(this, key);
                    return Ok(Ok((Value::Bool(true), outs)));
                }
                // The page's own key event first: the menu sounds, F4.
                let r = self.call_inner(f, this, args);
                if key == 0x73 {
                    return Ok(r);
                }
                if state == 1 {
                    self.coop_page_key(this, key);
                }
                Ok(Ok((Value::Bool(true), outs)))
            }
            _ => Err(args),
        }
    }

    fn coop_page_created(&mut self, page: ObjId) {
        let greyed = self.coop_greyed();
        let p = &mut self.improved.menu_boxes.coop_page;
        p.page = Some(page);
        p.rows.clear();
        p.row_text = None;
        p.applied = None;
        p.press_start = None;
        p.address = None;
        p.name = None;
        p.name_kept = None;
        p.finder = None;
        p.found.clear();
        p.finder_failed = None;
        p.nav = Some(if LAN_COOP_READY { Nav::new(greyed) } else { Nav::split_only(greyed) });
        // Without the first step, the split-screen session is chosen as the page opens.
        if !LAN_COOP_READY {
            self.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: None }));
        }
        // The extra enemies kept from the last time (none the first time).
        let extra = last_extra_enemies();
        if let Some(nav) = &mut self.improved.menu_boxes.coop_page.nav {
            nav.extra_enemies = extra;
        }
        self.set_coop_extra_enemies(extra);
        let split = last_split_layout();
        if let Some(nav) = &mut self.improved.menu_boxes.coop_page.nav {
            nav.split_layout = split;
        }
        crate::split_layout::set_split_layout(split);
        let scale = self.page_scale_of(page);
        self.set_prop(page, "bShowBCK", Value::Bool(true));
        self.set_prop(page, "bShowSEL", Value::Bool(true));
        if let Some(class) = self.class("XIIIButton") {
            for i in 0..ROWS {
                let [x, y, w, h] = row_rect(i);
                let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y * scale), Value::Float(w), Value::Float(h * scale)];
                let Some(row) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { continue };
                if let Value::Array(mut controls) = self.prop(page, "Controls") {
                    controls.push(Value::Object(Some(row)));
                    self.set_prop(page, "Controls", Value::Array(controls));
                }
                if self.improved.menu_boxes.coop_page.row_text.is_none() {
                    self.improved.menu_boxes.coop_page.row_text = Some(self.propr(row, "TextColor").clone());
                }
                self.improved.menu_boxes.coop_page.rows.push(row);
            }
        }
        // The host's address box, over the first row, as the game's own Host LAN page has one
        // for the server name (XIIIEditCtrl: typed text, Backspace, Enter ends the typing).
        if let Some(class) = self.class("XIIIEditCtrl") {
            let [x, y, _, h] = row_rect(0);
            let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y * scale), Value::Float(ADDRESS_W), Value::Float(h * scale)];
            if let Some(edit) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) {
                if let Value::Array(mut controls) = self.prop(page, "Controls") {
                    controls.push(Value::Object(Some(edit)));
                    self.set_prop(page, "Controls", Value::Array(controls));
                }
                let title = self.added_text(Text::CoopAddressBox).to_string();
                self.set_prop(edit, "TitleText", Value::Str(title));
                self.set_prop(edit, "bCalculateSize", Value::Bool(false));
                self.set_prop(edit, "FirstBoxWidth", Value::Float(130.0));
                self.set_prop(edit, "AllowedCharSet", Value::Str(ADDRESS_CHARS.to_string()));
                self.set_prop(edit, "MaxWidth", Value::Int(ADDRESS_MAX));
                let _ = self.call_named(edit, "SetText", vec![Value::Str(last_join_address())]);
                self.set_prop(edit, "bVisible", Value::Bool(false));
                self.set_prop(edit, "bNeverFocus", Value::Bool(true));
                self.improved.menu_boxes.coop_page.address = Some(edit);
            }
        }
        // The co-op name box, in its row of the first step.
        if let Some(class) = self.class("XIIIEditCtrl") {
            let row = Step::Mode.entries().iter().position(|&e| e == Entry::CoopName).unwrap_or(0);
            let [x, y, _, h] = row_rect(row);
            let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y * scale), Value::Float(ADDRESS_W), Value::Float(h * scale)];
            if let Some(edit) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) {
                if let Value::Array(mut controls) = self.prop(page, "Controls") {
                    controls.push(Value::Object(Some(edit)));
                    self.set_prop(page, "Controls", Value::Array(controls));
                }
                let title = self.added_text(Text::CoopName).to_string();
                self.set_prop(edit, "TitleText", Value::Str(title));
                self.set_prop(edit, "bCalculateSize", Value::Bool(false));
                self.set_prop(edit, "FirstBoxWidth", Value::Float(130.0));
                self.set_prop(edit, "AllowedCharSet", Value::Str(NAME_CHARS.to_string()));
                self.set_prop(edit, "MaxWidth", Value::Int(NAME_MAX));
                let name = self.coop_chat_name();
                let _ = self.call_named(edit, "SetText", vec![Value::Str(name.clone())]);
                self.improved.menu_boxes.coop_page.name = Some(edit);
                self.improved.menu_boxes.coop_page.name_kept = Some(name);
            }
        }
    }

    /// This player's name in the LAN co-op chat: the one typed on the Co-op page, else their
    /// multiplayer name (the multiplayer pages' MyName, else Name, of [DefaultPlayer]).
    pub fn coop_chat_name(&mut self) -> String {
        let multi = self.config.as_mut().and_then(|c| {
            c.get_string("DefaultPlayer", "MyName", "User").filter(|n| !n.trim().is_empty()).or_else(|| c.get_string("DefaultPlayer", "Name", "User"))
        });
        coop_name_from(page_value("CoopName"), multi)
    }

    /// The control showing row `i` of the step in front: the text box of a text entry (the
    /// address, the co-op name), otherwise the row's button.
    fn coop_row_control(&self, i: usize) -> Option<ObjId> {
        let p = &self.improved.menu_boxes.coop_page;
        match p.nav.as_ref().and_then(|n| n.entries().get(i).copied()) {
            Some(Entry::Address) => p.address,
            Some(Entry::CoopName) => p.name,
            _ => p.rows.get(i).copied(),
        }
    }

    /// The press-start picture, put together once from the game's own four textures (read from the
    /// player's installed game, as its press-start page draws them), up to PICTURE_RIGHT of its width.
    fn coop_press_start(&mut self) -> Option<(Value, (u32, u32))> {
        if let Some(p) = &self.improved.menu_boxes.coop_page.press_start {
            return Some(p.clone());
        }
        let tiles: Vec<Image> = PRESS_START.iter().map(|p| self.menu_texture_image(p)).collect::<Option<Vec<_>>>()?;
        let (tw, th) = (tiles[0].w, tiles[0].h);
        let (full_w, h) = (tw + tiles[1].w, th + tiles[2].h);
        let w = ((full_w as f32 * PICTURE_RIGHT).round() as u32).clamp(1, full_w);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for (k, t) in tiles.iter().enumerate() {
            let (ox, oy) = ((k as u32 % 2) * tw, (k as u32 / 2) * th);
            for y in 0..t.h.min(h - oy) {
                for x in 0..t.w.min(w.saturating_sub(ox)) {
                    let s = ((y * t.w + x) * 4) as usize;
                    let d = (((oy + y) * w + ox + x) * 4) as usize;
                    rgba[d..d + 4].copy_from_slice(&t.rgba[s..s + 4]);
                }
            }
        }
        // The tiles' edge pixels (a thin light line where they meet once put together) take their
        // neighbours' colour, across the joins of the columns and of the rows.
        let px = |rgba: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * w + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        if tw >= 2 && tw + 1 < w {
            for y in 0..h {
                let (left, right) = (px(&rgba, tw - 2, y), px(&rgba, tw + 1, y));
                for (x, v) in [(tw - 1, left), (tw, right)] {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&v);
                }
            }
        }
        if th >= 2 && th + 1 < h {
            for x in 0..w {
                let (up, down) = (px(&rgba, x, th - 2), px(&rgba, x, th + 1));
                for (y, v) in [(th - 1, up), (th, down)] {
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i..i + 4].copy_from_slice(&v);
                }
            }
        }
        // Its left edge fades into the black of the page's area (its dark background is not quite
        // black: the edge showed as a line).
        let fade = (w as f32 * PICTURE_FADE).round().max(1.0) as u32;
        for y in 0..h {
            for x in 0..fade.min(w) {
                let k = x as f32 / fade as f32;
                let i = ((y * w + x) * 4) as usize;
                for c in &mut rgba[i..i + 3] {
                    *c = (*c as f32 * k).round() as u8;
                }
            }
        }
        // Its last two columns black: the drawn picture's left edge samples them (the texture
        // wraps), which showed them as a light line in the black area.
        for y in 0..h {
            for x in w.saturating_sub(2)..w {
                let i = ((y * w + x) * 4) as usize;
                rgba[i..i + 3].fill(0);
            }
        }
        self.improved.menu_boxes.coop_page.background_serial += 1;
        let name = format!("{PRESS_START_TEXTURE}{}", self.improved.menu_boxes.coop_page.background_serial);
        let value = Self::menu_register(&name, Image { w, h, rgba });
        self.improved.menu_boxes.coop_page.press_start = Some((value.clone(), (w, h)));
        Some((value, (w, h)))
    }

    fn coop_page_key(&mut self, page: ObjId, key: i32) {
        let rows = self.improved.menu_boxes.coop_page.rows.clone();
        let k = match key {
            0x26 => NavKey::Up,
            0x28 => NavKey::Down,
            0x24 | 0x21 => NavKey::Home,
            0x23 | 0x22 => NavKey::End,
            0x0D => NavKey::Enter,
            0x1B | 0x08 => NavKey::Back,
            0x25 => NavKey::Left,
            0x27 => NavKey::Right,
            // IK_LeftMouse: a click on the row under the cursor (none elsewhere on the page).
            0x01 => {
                let shown: Vec<ObjId> = (0..rows.len()).filter_map(|i| self.coop_row_control(i)).collect();
                match shown.iter().position(|&r| self.propr(r, "bVisible").as_bool() && self.call_named(r, "IsInBounds", Vec::new()).is_ok_and(|v| v.as_bool()))
                {
                    Some(v) => NavKey::Click(v),
                    None => return,
                }
            }
            _ => return,
        };
        let Some(nav) = &mut self.improved.menu_boxes.coop_page.nav else { return };
        let out = nav.key(k);
        match out {
            Outcome::None => {}
            Outcome::Mode(mode) => {
                self.change_local_play(|p| p.coop = Some(CoopSession { mode, start: None }));
                // Hosting: one player on this PC (lockstep, two in all), and the addresses to give.
                if mode == CoopMode::LanHost {
                    self.set_menu_local_players(1);
                    self.coop_page_find_addresses();
                }
            }
            Outcome::BackToMode => self.change_local_play(|p| p.coop = None),
            Outcome::Close => {
                self.change_local_play(|p| p.coop = None);
                if let Some(root) = self.propr(page, "myRoot").as_object() {
                    // The main menu's OnReOpen (hMainPage): its arrival hit alone in the improved
                    // menus, the main menu's loop having gone on under the page (menu_sounds.rs).
                    if crate::menu_mouse::enabled(self.engine.client.session_graphics) {
                        self.with_main_arrival_only(|vm| {
                            let _ = vm.call_named(root, "CloseMenu", vec![Value::Bool(true)]);
                        });
                    } else {
                        let _ = self.call_named(root, "CloseMenu", vec![Value::Bool(true)]);
                    }
                }
                // Back on the main menu with the Co-op box focused (nothing had the focus, and the
                // arrows then did nothing): given at the menu's next AfterPaint (menu_boxes.rs).
                self.improved.menu_boxes.refocus_coop = true;
            }
            Outcome::Play(start) => self.coop_page_play(start),
            // A game of the list: joined as its address typed in the box would be.
            Outcome::JoinFound(k) => {
                let addr = self.improved.menu_boxes.coop_page.found.get(k).map(|f| crate::coop_address::address_text(*f.addr.ip(), Some(f.addr.port())));
                if let (Some(addr), Some(edit)) = (addr, self.improved.menu_boxes.coop_page.address) {
                    self.log.push(format!("Log: Co-op: joining the game found at {addr}"));
                    let _ = self.call_named(edit, "SetText", vec![Value::Str(addr)]);
                    self.coop_page_play(None);
                }
            }
            Outcome::ExtraEnemies(percent) => {
                self.set_coop_extra_enemies(percent);
                keep_page_value("ExtraEnemies", &percent.to_string());
            }
            Outcome::SplitLayout(v) => {
                crate::split_layout::set_split_layout(v);
                keep_page_value("SplitLayout", v.key_text());
            }
        }
        // Only while the page is still in front: a page opened over it (Continue, Load, Select
        // level) keeps its own focus (taking it back left that page without one, ignoring keys).
        if self.coop_page_in_front().is_some() {
            self.coop_page_sync_focus();
        }
    }

    /// This computer's addresses on the local network, as the other player types them.
    fn coop_page_find_addresses(&mut self) {
        let found: Vec<String> = crate::coop_address::lan_addresses().into_iter().map(|a| crate::coop_address::address_text(a, None)).collect();
        self.log.push(format!("Log: Co-op: this computer's addresses {found:?}"));
        self.improved.menu_boxes.coop_page.lan_addresses = found;
    }

    /// Everything is chosen: the session gets its start, then the menu's own flow runs.
    fn coop_page_play(&mut self, start: Option<CoopStart>) {
        let mode = self.improved.menu_boxes.coop_page.nav.as_ref().and_then(|n| n.mode);
        let Some(mode) = mode else { return };
        self.change_local_play(|p| p.coop = Some(CoopSession { mode, start }));
        let Some(root) = self.gui_local() else { return };
        match start {
            // Joining (coop_address.rs): the address typed in the box, kept for the next time.
            None => {
                let address = self.improved.menu_boxes.coop_page.address.map(|e| self.propr(e, "TextStr").as_name().trim().to_string()).unwrap_or_default();
                if address.is_empty() || !self.coop_join(&address) {
                    // Nothing to join yet: the box keeps the focus.
                    if let Some(nav) = &mut self.improved.menu_boxes.coop_page.nav {
                        nav.focus = 0;
                    }
                    return;
                }
                keep_join_address(&address);
                let _ = self.call_named(root, "CloseAll", vec![Value::Bool(true)]);
            }
            Some(CoopStart::NewGame) if mode == CoopMode::LanHost && crate::lockstep::wanted() => {
                // Hosting over the network: the first level is asked for at once (XIIIMenu's
                // EndOfVideo), its video played on both machines once they met (the window's
                // lockstep_play.rs), so that the player who joins sees it too.
                let _ = self.call_named(root, "CloseAll", vec![Value::Bool(true)]);
                self.coop_host_new_game();
            }
            Some(CoopStart::NewGame) => {
                // The main menu's New game: its video, then the first level. The page closes first
                // so that the video plays over the main menu as from the New game box.
                let _ = self.call_named(root, "CloseMenu", vec![Value::Bool(true)]);
                let menu = self.improved.menu_boxes.page.filter(|&m| self.objects.get(m).is_some_and(|o| o.is_some()));
                if let Some(menu) = menu {
                    if let Some(new) = self.propr(menu, "NewButton").as_object() {
                        let _ = self.call_named(menu, "InternalOnClick", vec![Value::Object(Some(new))]);
                    }
                }
            }
            Some(CoopStart::Continue) => {
                let _ = self.call_named(root, "OpenMenu", vec![Value::Str("XIDInterf.XIIIMenuContinue".to_string())]);
            }
            Some(CoopStart::Load) => {
                let _ = self.call_named(root, "OpenMenu", vec![Value::Str("XIDInterf.XIIIMenuLoadGameWindow".to_string())]);
            }
            Some(CoopStart::SelectLevel) => {
                self.open_select_level_page();
            }
        }
    }

    /// The host's New game over the network (lockstep co-op): the first level without the main
    /// menu's video ("Plage00?OXIIIIntro"); the window plays the video on both machines once they
    /// met, then loads the level.
    pub fn coop_host_new_game(&mut self) {
        let url = format!("Plage00?{}", crate::coop_address::INTRO_OPTION);
        self.log.push(format!("Log: Co-op: new game {url}, its video once both players are here"));
        self.client_travel = Some((url, crate::travel::TRAVEL_ABSOLUTE, false));
    }

    /// The focused row of the navigation given the input focus.
    fn coop_page_sync_focus(&mut self) {
        let Some(nav) = &self.improved.menu_boxes.coop_page.nav else { return };
        let want = Some(nav.focus).filter(|v| *v < self.improved.menu_boxes.coop_page.rows.len());
        if let Some(v) = want {
            let Some(row) = self.coop_row_control(v) else { return };
            if !self.propr(row, "bHasFocus").as_bool() {
                let _ = self.call_named(row, "SetFocus", vec![Value::Object(None)]);
            }
        }
        self.improved.menu_boxes.coop_page.applied = want;
    }

    /// Name of an entry in the game's language (the game's own words for its own flows).
    fn coop_entry_label(&mut self, e: Entry, lang: &str) -> String {
        let own = |vm: &mut Vm, key: &str| vm.original_text("XIIIMenu", key, "XIDInterf");
        match e {
            Entry::SplitScreen => text(lang, Text::SplitScreen).to_string(),
            Entry::LanHost => text(lang, Text::HostLanGame).to_string(),
            Entry::LanJoin => text(lang, Text::JoinLanGame).to_string(),
            Entry::NewGame => own(self, "NewGameText").unwrap_or_else(|| text(lang, Text::NewGame).to_string()),
            Entry::Continue => own(self, "ContinueText").unwrap_or_else(|| text(lang, Text::Continue).to_string()),
            Entry::Load => own(self, "LoadGameText").unwrap_or_else(|| text(lang, Text::LoadGame).to_string()),
            Entry::SelectLevel => text(lang, Text::SelectLevel).to_string(),
            Entry::Address => text(lang, Text::CoopAddressBox).to_string(),
            Entry::CoopName => text(lang, Text::CoopName).to_string(),
            Entry::JoinNow => text(lang, Text::CoopJoinNow).to_string(),
            Entry::ExtraEnemies => {
                let value = match self.coop_extra_enemies() {
                    0 => text(lang, Text::ExtraEnemiesNone).to_string(),
                    v => format!("+{v} %"),
                };
                format!("{}   < {value} >", text(lang, if self.coop_page_hosting() { Text::ExtraEnemiesPerPlayer } else { Text::ExtraEnemies }))
            }
            Entry::SplitLayout => {
                let value = match crate::split_layout::split_layout() {
                    SplitLayout::Auto => Text::SplitLayoutAuto,
                    SplitLayout::Horizontal => Text::SplitLayoutHorizontal,
                    SplitLayout::Vertical => Text::SplitLayoutVertical,
                };
                let name = if self.coop_page_hosting() { Text::ScreenSplit } else { Text::SplitLayout };
                format!("{}   < {} >", text(lang, name), text(lang, value))
            }
            // "Name - Map (address)", and why it cannot be joined.
            Entry::Found(k) => match self.improved.menu_boxes.coop_page.found.get(usize::from(k)) {
                Some(f) => {
                    let addr = crate::coop_address::address_text(*f.addr.ip(), Some(f.addr.port()));
                    let line = format!("{} - {} ({addr})", f.listing.name, f.listing.map);
                    if f.same_game { line } else { format!("{line} {}", text(lang, Text::CoopFoundOtherVersion)) }
                }
                None => String::new(),
            },
        }
    }

    /// Joining: the search for the games on the network goes on while the join step is shown
    /// (stopped otherwise), and the list follows what it found.
    fn coop_page_find_games(&mut self, join: bool, in_front: bool) {
        let p = &mut self.improved.menu_boxes.coop_page;
        if !join {
            p.finder_failed = None;
            if p.finder.take().is_some() {
                p.found.clear();
                if let Some(nav) = &mut p.nav {
                    nav.set_found(&[]);
                }
            }
            return;
        }
        // Under a message box: the search waits (kept, not started again).
        if !in_front {
            return;
        }
        if p.finder.is_none() {
            if p.finder_failed.is_some_and(|t| t.elapsed() < FINDER_RETRY) {
                return;
            }
            let first_failure = p.finder_failed.is_none();
            match crate::lockstep_find::Finder::new(crate::coop_address::DEFAULT_PORT, crate::lockstep_find::own_game()) {
                Ok(f) => {
                    p.finder = Some(f);
                    p.finder_failed = None;
                }
                Err(e) => {
                    p.finder_failed = Some(std::time::Instant::now());
                    if first_failure {
                        self.warn(format!("Co-op: no search for the games on the network: {e}"));
                    }
                    return;
                }
            }
        }
        let p = &mut self.improved.menu_boxes.coop_page;
        let Some(finder) = p.finder.as_mut() else { return };
        let before: Vec<_> = p.found.iter().map(|f| f.addr).collect();
        p.found = finder.poll().to_vec();
        let joinable: Vec<bool> = p.found.iter().map(|f| f.same_game).collect();
        if let Some(nav) = &mut p.nav {
            nav.set_found(&joinable);
        }
        let now: Vec<_> = p.found.iter().map(|f| f.addr).collect();
        if now != before {
            self.log.push(format!("Log: Co-op: games found on the network {now:?}"));
        }
    }

    /// The page shows the LAN host's game step.
    fn coop_page_hosting(&self) -> bool {
        self.improved.menu_boxes.coop_page.nav.as_ref().is_some_and(|n| n.step == Step::Game && n.mode == Some(CoopMode::LanHost))
    }

    /// After the page's Paint (white page): the black picture area with the press-start picture at
    /// its right, the title, then the rows laid out (place, texts, greyed ones, focus) before they
    /// draw themselves.
    fn coop_page_paint(&mut self, page: ObjId, c: ObjId) {
        let scale = self.page_scale_of(page);
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        let lang = self.text_language().to_string();
        self.select_color(c, [255, 255, 255, 255]);
        self.set_prop(c, "Style", Value::Byte(1));
        // The picture area, black, and the picture at its right, as high as the area, its
        // proportions kept (what does not fit on the left stays black).
        let [ax, ay, aw, ah] = [AREA[0], AREA[1] * scale, AREA[2], AREA[3] * scale];
        self.select_fill(page, c, [ax * rx, ay * ry, aw * rx, ah * ry], [0, 0, 0, 255]);
        if let Some((pic, (w, h))) = self.coop_press_start() {
            let k = (ah / h as f32).min(aw / w as f32);
            let (pw, ph) = (w as f32 * k, h as f32 * k);
            self.select_color(c, [255, 255, 255, 255]);
            // No border: its left side showed as a line in the middle of the black area.
            self.set_prop(c, "bUseBorder", Value::Bool(false));
            self.select_draw_texture(page, c, [(ax + aw - pw) * rx, (ay + ah - ph) * ry, pw * rx, ph * ry], pic);
        }
        let Some(nav) = self.improved.menu_boxes.coop_page.nav.clone() else { return };
        self.select_draw_label(page, c, TITLE, text(&lang, Text::MenuCoop), (rx, ry), true);

        let rows = self.improved.menu_boxes.coop_page.rows.clone();
        let join = nav.step == Step::Join;
        let in_front = self.coop_page_in_front().is_some();
        self.coop_page_find_games(join, in_front);
        let Some(nav) = self.improved.menu_boxes.coop_page.nav.clone() else { return };
        if let Some(edit) = self.improved.menu_boxes.coop_page.address {
            self.set_prop(edit, "bVisible", Value::Bool(join));
            self.set_prop(edit, "bNeverFocus", Value::Bool(!join));
        }
        let name_row = nav.entries().iter().position(|&e| e == Entry::CoopName);
        if let Some(edit) = self.improved.menu_boxes.coop_page.name {
            self.set_prop(edit, "bVisible", Value::Bool(name_row.is_some()));
            self.set_prop(edit, "bNeverFocus", Value::Bool(name_row.is_none()));
            // The name typed is kept as it changes (for this page and the network games).
            let typed = self.propr(edit, "TextStr").as_name().trim().to_string();
            if self.improved.menu_boxes.coop_page.name_kept.as_deref() != Some(typed.as_str()) {
                keep_page_value("CoopName", &typed);
                self.improved.menu_boxes.coop_page.name_kept = Some(typed);
            }
            if let Some(i) = name_row {
                let focus = Some(i) == self.improved.menu_boxes.coop_page.applied;
                self.set_prop(edit, "bUseBorder", Value::Bool(focus));
            }
        }
        // A focus moved by the mouse becomes the navigation's.
        let shown: Vec<Option<ObjId>> = (0..rows.len()).map(|i| self.coop_row_control(i)).collect();
        let focused = shown.iter().position(|r| r.is_some_and(|r| self.propr(r, "bHasFocus").as_bool() && self.propr(r, "bVisible").as_bool()));
        if focused.is_some() && focused != self.improved.menu_boxes.coop_page.applied {
            if let (Some(v), Some(nav)) = (focused, &mut self.improved.menu_boxes.coop_page.nav) {
                nav.key(NavKey::Hover(v));
            }
        }
        // Only while the page is in front: under a message box (a reason after a network game,
        // the host-left box) taking the focus back to a row made the box's buttons ignore the
        // mouse (a click only moves the focus when the control under it is not the focused one).
        if self.coop_page_in_front().is_some() {
            self.coop_page_sync_focus();
        }
        let entries = nav.entries();
        for (i, &row) in rows.iter().enumerate() {
            // Each row where the step in front lays it out.
            let [x, y, w, h] = step_row_rect(&nav, i);
            self.set_prop(row, "WinLeft", Value::Float(x / 640.0));
            self.set_prop(row, "WinTop", Value::Float(y * scale / 480.0));
            self.set_prop(row, "WinWidth", Value::Float(w / 640.0));
            self.set_prop(row, "WinHeight", Value::Float(h * scale / 480.0));
            let entry = entries.get(i).copied();
            let shown = entry.is_some();
            let enabled = entry.is_some_and(|e| nav.enabled(e));
            let label = match entry {
                Some(e) => self.coop_entry_label(e, &lang),
                None => String::new(),
            };
            // A text entry's row is its box (the address, the co-op name).
            let shown = shown && !matches!(entry, Some(Entry::Address | Entry::CoopName));
            self.set_prop(row, "bVisible", Value::Bool(shown));
            self.set_prop(row, "bNeverFocus", Value::Bool(!enabled || !shown));
            self.set_prop(row, "Text", Value::Str(label));
            self.set_prop(row, "TextAlign", Value::Byte(0));
            self.set_prop(row, "TextX", Value::Int((ROW_INSET * rx).round() as i32));
            let focus = Some(i) == self.improved.menu_boxes.coop_page.applied;
            // Light text on the bands over the black area, the menu's own colour on the
            // focused one (its white box).
            if !enabled {
                self.set_prop(row, "TextColor", crate::canvas::color_value(GREYED_TEXT));
            } else if focus {
                if let Some(normal) = self.improved.menu_boxes.coop_page.row_text.clone() {
                    self.set_prop(row, "TextColor", normal);
                }
            } else {
                self.set_prop(row, "TextColor", crate::canvas::color_value(BAND_TEXT));
            }
            self.set_prop(row, "bUseBorder", Value::Bool(focus));
        }
        if let (true, Some(edit)) = (join, self.improved.menu_boxes.coop_page.address) {
            let focus = Some(0) == self.improved.menu_boxes.coop_page.applied;
            self.set_prop(edit, "bUseBorder", Value::Bool(focus));
        }
    }

    /// After the rows: the hint on each greyed one, at its right end.
    fn coop_page_after_paint(&mut self, page: ObjId, c: ObjId) {
        let Some(nav) = self.improved.menu_boxes.coop_page.nav.clone() else { return };
        let scale = self.page_scale_of(page);
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        if nav.step == Step::Game && nav.mode == Some(CoopMode::LanHost) {
            self.coop_page_host_texts(c, scale, (rx, ry));
        }
        if nav.step == Step::Join {
            self.coop_page_join_texts(c, scale, (rx, ry));
        }
        let hint = self.added_text(Text::ComingSoon);
        for (i, &e) in nav.entries().iter().enumerate() {
            // A game found greyed says why in its own text.
            if nav.enabled(e) || matches!(e, Entry::Found(_)) {
                continue;
            }
            let (tw, th) = self.select_text_size(c, hint);
            let [x, y, w, h] = step_row_rect(&nav, i);
            let y = (y + h / 2.0) * scale * ry - th / 2.0;
            self.select_draw_text(c, (x + w - ROW_INSET) * rx - tw, y, hint, GREYED_TEXT);
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// A text drawn left-aligned at `x`, centred on `y` (units, `y` unscaled), in the canvas font
    /// in front.
    fn coop_page_line(&mut self, c: ObjId, x: f32, y: f32, scale: f32, (rx, ry): (f32, f32), t: &str, rgba: [u8; 4]) {
        let (_, th) = self.select_text_size(c, t);
        self.select_draw_text(c, x * rx, y * scale * ry - th / 2.0, t, rgba);
    }

    /// Lines of `t` wrapped at HOST_TEXT_W, drawn in the canvas's small font from `y` down, one
    /// HOST_LINE_GAP apart.
    fn coop_page_small_lines(&mut self, c: ObjId, y: f32, scale: f32, (rx, ry): (f32, f32), t: &str, rgba: [u8; 4]) {
        let font = self.prop(c, "Font");
        let small = self.prop(c, "SmallFont");
        if !matches!(small, Value::Object(None) | Value::Void) {
            self.set_prop(c, "Font", small);
        }
        let lines = wrap_words(t, HOST_TEXT_W * rx, |s| self.select_text_size(c, s).0);
        for (k, l) in lines.iter().enumerate() {
            self.coop_page_line(c, HOST_TEXT_X, y + k as f32 * HOST_LINE_GAP, scale, (rx, ry), l, rgba);
        }
        self.set_prop(c, "Font", font);
    }

    /// Hosting a LAN game: this computer's address on top (the others under it), how the other
    /// player joins, and the hint at the bottom of the area.
    fn coop_page_host_texts(&mut self, c: ObjId, scale: f32, r: (f32, f32)) {
        let addresses = self.improved.menu_boxes.coop_page.lan_addresses.clone();
        let lang = self.text_language().to_string();
        let (first, mut next) = match addresses.split_first() {
            Some((main, others)) => {
                let line = crate::added_text::fill(text(&lang, Text::LanHostAddress), &[main]);
                self.coop_page_line(c, HOST_TEXT_X, HOST_LINES_Y[0], scale, r, &line, BAND_TEXT);
                if others.is_empty() {
                    (true, HOST_LINES_Y[1])
                } else {
                    let line = crate::added_text::fill(text(&lang, Text::LanHostOtherAddresses), &[&others.join(", ")]);
                    self.coop_page_small_lines(c, HOST_LINES_Y[1], scale, r, &line, HINT_TEXT);
                    (true, HOST_LINES_Y[2])
                }
            }
            None => {
                self.coop_page_line(c, HOST_TEXT_X, HOST_LINES_Y[0], scale, r, text(&lang, Text::LanHostNoAddress), BAND_TEXT);
                (false, HOST_LINES_Y[1])
            }
        };
        if first {
            let how = crate::added_text::fill(text(&lang, Text::LanHostHowToJoin), &[text(&lang, Text::MenuCoop), text(&lang, Text::JoinLanGame)]);
            self.coop_page_small_lines(c, next, scale, r, &how, HINT_TEXT);
            next += HOST_LINE_GAP;
        }
        let _ = next;
        self.coop_page_small_lines(c, HOST_HINT_Y, scale, r, text(&lang, Text::LanHostHint), HINT_TEXT);
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// Joining: what to type and when, above the address box, and an example in the empty box.
    fn coop_page_join_texts(&mut self, c: ObjId, scale: f32, r: (f32, f32)) {
        let lang = self.text_language().to_string();
        self.coop_page_line(c, HOST_TEXT_X, JOIN_LINES_Y[0], scale, r, text(&lang, Text::LanJoinIntro), BAND_TEXT);
        self.coop_page_small_lines(c, JOIN_LINES_Y[1], scale, r, text(&lang, Text::LanJoinHint), HINT_TEXT);
        // No game found yet: the search goes on, said where the first one will be.
        if self.improved.menu_boxes.coop_page.found.is_empty() {
            let [x, y, _, h] = row_rect(2);
            self.coop_page_small_lines_at(c, x + ROW_INSET, y + h / 2.0, scale, r, text(&lang, Text::CoopFoundSearching), HINT_TEXT);
        }
    }

    /// One line of `t` in the canvas's small font, left-aligned at `x`, centred on `y`.
    fn coop_page_small_lines_at(&mut self, c: ObjId, x: f32, y: f32, scale: f32, r: (f32, f32), t: &str, rgba: [u8; 4]) {
        let font = self.prop(c, "Font");
        let small = self.prop(c, "SmallFont");
        if !matches!(small, Value::Object(None) | Value::Void) {
            self.set_prop(c, "Font", small);
        }
        self.coop_page_line(c, x, y, scale, r, t, rgba);
        self.set_prop(c, "Font", font);
    }

    /// Hooks of the multiplayer pages (menu_boxes_call): Created adds the setting, Paint keeps it
    /// in step with the state (value, greyed with its hint).
    #[allow(clippy::type_complexity)]
    pub(crate) fn players_setting_call(
        &mut self,
        f: usize,
        this: ObjId,
        class: &str,
        name: &str,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        let Some(rect) = players_setting_rect(class) else { return Err(args) };
        match name {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.players_setting_created(this, rect);
                }
                Ok(r)
            }
            "Paint" => {
                let Some(control) = self.improved.menu_boxes.players_setting.filter(|(p, _)| *p == this).map(|(_, c)| c) else { return Err(args) };
                let ready = local_players_ready(class);
                self.players_setting_refresh(this, control, rect, ready);
                // The page draws its highlight band at 91 + 45 x (index of the focused control);
                // the setting is the last of the Controls but sits above the first row: drawn as
                // if no control were focused (index -1), the band falls on the setting.
                let focused = self.propr(this, "FocusedControl").as_object();
                let on_setting = ready && focused == Some(control);
                if on_setting {
                    self.set_prop(this, "FocusedControl", Value::Object(None));
                }
                let r = self.call_inner(f, this, args);
                if on_setting && self.propr(this, "FocusedControl").as_object().is_none() {
                    self.set_prop(this, "FocusedControl", Value::Object(Some(control)));
                }
                Ok(r)
            }
            "InternalOnKeyEvent" => {
                let control = self.improved.menu_boxes.players_setting.filter(|(p, _)| *p == this).map(|(_, c)| c);
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                let focused = self.propr(this, "FocusedControl").as_object();
                // Left / Right on the setting (also a click on its arrows, which XIIIValueControl
                // turns into those keys); press or repeat, as the page's own controls.
                if control.is_some() && focused == control && matches!(state, 1 | 2) && matches!(key, 0x25 | 0x27) {
                    let n = self.local_play().local_players;
                    let n = if key == 0x25 { n.saturating_sub(1) } else { n + 1 };
                    self.set_menu_local_players(n);
                    let outs = self.functions[f]
                        .params
                        .iter()
                        .enumerate()
                        .map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Byte(0))) } else { None })
                        .collect();
                    return Ok(Ok((Value::Bool(true), outs)));
                }
                Err(args)
            }
            "StartPressed" => {
                let r = self.call_inner(f, this, args);
                let n = self.local_play().local_players;
                // Bot challenge only: a hosted game's players on this screen come from the setting
                // itself (split_versus.rs), its URL is the page's.
                if n > 1 && class.eq_ignore_ascii_case(LOCAL_PLAYERS_PAGE) {
                    if let Some((url, _, _)) = self.client_travel.as_mut() {
                        *url = split_travel_url(url, n);
                        let url = url.clone();
                        self.log.push(format!("Log: Split screen: {n} players on this screen, travelling to {url}"));
                    }
                }
                Ok(r)
            }
            _ => Err(args),
        }
    }

    fn players_setting_created(&mut self, page: ObjId, [x, y, w, h]: [f32; 4]) {
        let Some(class) = self.class("XIIIValueControl") else { return };
        let scale = self.page_scale_of(page);
        let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y * scale), Value::Float(w), Value::Float(h)];
        let Some(control) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { return };
        // Last of the page's Controls (drawn with them); never focused while greyed, so the page's
        // own key handling (Up / Down through Controls, the highlight band drawn at the focused
        // control's index) stays the original's.
        if let Value::Array(mut controls) = self.prop(page, "Controls") {
            controls.push(Value::Object(Some(control)));
            self.set_prop(page, "Controls", Value::Array(controls));
        }
        // The label, with the hint while greyed ("Players on this screen (coming soon)") where it
        // fits: next to the Host LAN page's title the box is too narrow for it (the text ran
        // past the box), the grey alone says it.
        let mut label = self.added_text(Text::PlayersOnThisScreen).to_string();
        let ready = self.objects.get(page).and_then(|o| o.as_ref()).is_some_and(|o| local_players_ready(&self.classes[o.class].name));
        if !ready && w >= HINT_MIN_WIDTH {
            label = format!("{label} ({})", self.added_text(Text::ComingSoon));
        }
        self.set_prop(control, "Text", Value::Str(label));
        self.set_prop(control, "bCalculateSize", Value::Bool(false));
        let _ = self.call_named(control, "SetRange", vec![Value::Int(1), Value::Int(i32::from(crate::local_play::MAX_LOCAL_PLAYERS))]);
        self.improved.menu_boxes.players_setting = Some((page, control));
        self.players_setting_refresh(page, control, [x, y, w, h], ready);
    }

    /// The control shows the state's value; greyed (never focused) while nothing reads it. The
    /// label box takes all but VALUE_BOX units of the width (FirstBoxWidth is in pixels; the value
    /// box is drawn 64 pixels narrower than what is left).
    fn players_setting_refresh(&mut self, page: ObjId, control: ObjId, [_, _, w, _]: [f32; 4], ready: bool) {
        const VALUE_BOX: f32 = 80.0;
        let rx = self.propr(page, "fRatioX").as_float().max(0.01);
        self.set_prop(control, "FirstBoxWidth", Value::Float((w - VALUE_BOX) * rx));
        // Greyed: 1, the only number these games have.
        let n = if ready { self.local_play().local_players } else { 1 };
        let _ = self.call_named(control, "SetValue", vec![Value::Int(i32::from(n))]);
        if !ready {
            self.set_prop(control, "bNeverFocus", Value::Bool(true));
            self.set_prop(control, "bAcceptsInput", Value::Bool(false));
            self.set_prop(control, "bShowLeftArrow", Value::Bool(false));
            self.set_prop(control, "bShowRightArrow", Value::Bool(false));
            self.set_prop(control, "TextColor", crate::canvas::color_value(GREYED_TEXT));
        }
    }

    /// The multiplayer pages' setting changed (arrows on the control of the Bot challenge page).
    /// Only the menus' state: the engine's own setting (split_versus.rs, `Vm::set_local_players`)
    /// is given this value by the game window before a travel.
    pub fn set_menu_local_players(&mut self, n: u8) {
        self.change_local_play(|p| p.set_local_players(n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav() -> Nav {
        Nav::new(vec![Entry::LanHost, Entry::LanJoin, Entry::CoopName])
    }

    fn host_nav() -> Nav {
        let mut n = Nav::new(Vec::new());
        n.focus = 1;
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::LanHost));
        n
    }

    #[test]
    fn split_only_opens_on_the_split_game_and_escape_closes() {
        let mut n = Nav::split_only(Vec::new());
        assert_eq!((n.step, n.focused(), n.mode), (Step::Game, Some(Entry::NewGame), Some(CoopMode::SplitScreen)));
        assert!(!n.entries().iter().any(|e| matches!(e, Entry::LanHost | Entry::LanJoin | Entry::CoopName)));
        assert_eq!(n.key(NavKey::Back), Outcome::Close);
        let mut n = Nav::split_only(Vec::new());
        assert_eq!(n.key(NavKey::Enter), Outcome::Play(Some(CoopStart::NewGame)));
    }

    #[test]
    fn hosting_starts_on_new_game_under_its_settings() {
        let mut n = host_nav();
        assert_eq!(n.entries(), &GAME_HOST_ENTRIES[..]);
        assert_eq!((n.step, n.focused(), n.settings_on_top()), (Step::Game, Some(Entry::NewGame), 1));
        // The one setting of a lockstep game: the extra enemies (no players on this PC to choose).
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::ExtraEnemies));
        assert_eq!(n.key(NavKey::Right), Outcome::ExtraEnemies(50));
        // Up from the first row wraps to the last game.
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::SelectLevel));
        n.key(NavKey::Down);
        assert_eq!(n.focused(), Some(Entry::ExtraEnemies));
    }

    #[test]
    fn hosting_plays_and_goes_back() {
        let mut n = host_nav();
        assert_eq!(n.key(NavKey::Enter), Outcome::Play(Some(CoopStart::NewGame)));
        n.key(NavKey::Down);
        assert_eq!(n.key(NavKey::Enter), Outcome::Play(Some(CoopStart::Continue)));
        // A click on a game row plays it; on a setting it steps it.
        assert_eq!(n.key(NavKey::Click(4)), Outcome::Play(Some(CoopStart::SelectLevel)));
        assert_eq!(n.key(NavKey::Click(0)), Outcome::ExtraEnemies(50));
        assert_eq!(n.key(NavKey::Back), Outcome::BackToMode);
        assert_eq!((n.step, n.focused()), (Step::Mode, Some(Entry::LanHost)));
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::LanHost));
        assert_eq!((n.entries(), n.focused()), (&GAME_HOST_ENTRIES[..], Some(Entry::NewGame)));
    }

    #[test]
    fn host_layout_separates_settings_from_games() {
        let n = host_nav();
        let r = |n: &Nav, i: usize| step_row_rect(n, i);
        assert_eq!(r(&n, 0)[1], HOST_BAND_Y);
        assert_eq!(r(&n, 1)[1] - r(&n, 0)[1], HOST_BAND_PITCH + HOST_SETTINGS_GAP);
        assert_eq!(r(&n, 2)[1] - r(&n, 1)[1], HOST_BAND_PITCH);
        // Every row above the hint, inside the area.
        let last = r(&n, ROWS - 1);
        assert!(last[1] + last[3] < HOST_HINT_Y - HOST_LINE_GAP / 2.0 && HOST_HINT_Y + HOST_LINE_GAP < AREA[1] + AREA[3]);
        // The split-screen step keeps the page's own layout.
        let mut s = Nav::new(Vec::new());
        s.key(NavKey::Enter);
        assert_eq!(step_row_rect(&s, 2), row_rect(2));
    }

    #[test]
    fn words_wrap_at_the_width() {
        let size = |s: &str| s.chars().count() as f32;
        assert_eq!(wrap_words("aa bb cc dd", 5.0, size), vec!["aa bb", "cc dd"]);
        assert_eq!(wrap_words("aaaaaaa b", 5.0, size), vec!["aaaaaaa", "b"]);
        assert_eq!(wrap_words("  ", 5.0, size), Vec::<String>::new());
    }

    #[test]
    fn first_step_then_second_then_play() {
        let mut n = nav();
        assert_eq!((n.step, n.focused()), (Step::Mode, Some(Entry::SplitScreen)));
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::SplitScreen));
        assert_eq!((n.step, n.focused(), n.mode), (Step::Game, Some(Entry::NewGame), Some(CoopMode::SplitScreen)));
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        n.key(NavKey::Down);
        assert_eq!(n.focused(), Some(Entry::SelectLevel));
        assert_eq!(n.key(NavKey::Enter), Outcome::Play(Some(CoopStart::SelectLevel)));
        n.key(NavKey::Down);
        assert_eq!(n.focused(), Some(Entry::ExtraEnemies));
        n.key(NavKey::Down);
        assert_eq!(n.focused(), Some(Entry::SplitLayout));
        n.key(NavKey::Down);
        assert_eq!(n.focused(), Some(Entry::NewGame), "Down wraps to the top, as the game's menus");
        assert_eq!(n.key(NavKey::Click(1)), Outcome::Play(Some(CoopStart::Continue)));
        assert_eq!(n.key(NavKey::Click(2)), Outcome::Play(Some(CoopStart::Load)));
        assert_eq!(n.key(NavKey::Click(0)), Outcome::Play(Some(CoopStart::NewGame)));
        assert_eq!(n.key(NavKey::Click(9)), Outcome::None, "past the rows: nothing");
    }

    #[test]
    fn extra_enemies_band_on_the_game_step_only_and_left_right_change_it() {
        let mut n = nav();
        assert!(!n.entries().contains(&Entry::ExtraEnemies), "not on the first step");
        n.key(NavKey::Enter);
        assert!(n.entries().contains(&Entry::ExtraEnemies));
        n.key(NavKey::End);
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::ExtraEnemies));
        assert_eq!(n.key(NavKey::Right), Outcome::ExtraEnemies(50));
        assert_eq!(n.key(NavKey::Right), Outcome::ExtraEnemies(100));
        assert_eq!(n.key(NavKey::Right), Outcome::ExtraEnemies(0), "wraps");
        assert_eq!(n.key(NavKey::Left), Outcome::ExtraEnemies(100));
        assert_eq!(n.key(NavKey::Enter), Outcome::ExtraEnemies(0), "Enter takes the next value, it starts nothing");
        n.key(NavKey::Home);
        assert_eq!(n.key(NavKey::Right), Outcome::None, "left / right do nothing on the other rows");
        // Joining: the host decides, no band.
        let mut j = Nav::new(Vec::new());
        j.key(NavKey::Down);
        j.key(NavKey::Down);
        j.key(NavKey::Enter);
        assert!(!j.entries().contains(&Entry::ExtraEnemies));
    }

    #[test]
    fn the_page_values_file_keeps_the_other_value() {
        // The co-op name: the one typed, else the multiplayer name, else nothing.
        assert_eq!(coop_name_from(Some(" Kim ".into()), Some("XIII".into())), "Kim");
        assert_eq!(coop_name_from(Some("  ".into()), Some("XIII".into())), "XIII");
        assert_eq!(coop_name_from(None, Some(" ".into())), "");
        let t = page_values_text("", "Address", "192.0.2.5");
        assert_eq!(t, "[CoopJoin]\r\nAddress=192.0.2.5\r\n");
        let t = page_values_text(&t, "ExtraEnemies", "50");
        assert_eq!(t, "[CoopJoin]\r\nAddress=192.0.2.5\r\nExtraEnemies=50\r\n");
        let t = page_values_text(&t, "Address", "192.0.2.10");
        assert_eq!(t, "[CoopJoin]\r\nExtraEnemies=50\r\nAddress=192.0.2.10\r\n");
    }
    #[test]
    fn escape_goes_back_one_step_then_closes() {
        let mut n = nav();
        n.key(NavKey::Enter);
        assert_eq!(n.key(NavKey::Back), Outcome::BackToMode);
        assert_eq!((n.step, n.focused(), n.mode), (Step::Mode, Some(Entry::SplitScreen), None));
        assert_eq!(n.key(NavKey::Back), Outcome::Close);
    }

    #[test]
    fn greyed_entries_are_never_focused_or_chosen() {
        let mut n = nav();
        for k in [NavKey::Down, NavKey::Up, NavKey::End, NavKey::Home, NavKey::Hover(1), NavKey::Hover(2)] {
            n.key(k);
            assert_eq!(n.focused(), Some(Entry::SplitScreen), "{k:?}");
        }
        assert_eq!(n.key(NavKey::Click(1)), Outcome::None);
        assert_eq!(n.key(NavKey::Click(2)), Outcome::None);
        assert_eq!((n.step, n.mode), (Step::Mode, None));
        // Every entry greyed on the second step but one: the focus stays there.
        let mut n = Nav::new(vec![Entry::NewGame, Entry::Continue, Entry::Load]);
        n.key(NavKey::Enter);
        assert_eq!(n.focused(), Some(Entry::SelectLevel));
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::SplitLayout), "the settings below, wrapping");
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::ExtraEnemies));
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::SelectLevel));
    }

    #[test]
    fn split_band_only_for_a_split_screen_game_and_left_right_change_it() {
        let mut n = Nav::new(Vec::new());
        assert!(!n.entries().contains(&Entry::SplitLayout), "not on the first step");
        n.key(NavKey::Enter);
        assert_eq!(n.mode, Some(CoopMode::SplitScreen));
        n.key(NavKey::End);
        assert_eq!(n.focused(), Some(Entry::SplitLayout));
        assert_eq!(n.key(NavKey::Right), Outcome::SplitLayout(SplitLayout::Horizontal));
        assert_eq!(n.key(NavKey::Right), Outcome::SplitLayout(SplitLayout::Vertical));
        assert_eq!(n.key(NavKey::Right), Outcome::SplitLayout(SplitLayout::Auto), "wraps");
        assert_eq!(n.key(NavKey::Left), Outcome::SplitLayout(SplitLayout::Vertical));
        assert_eq!(n.key(NavKey::Enter), Outcome::SplitLayout(SplitLayout::Auto), "Enter takes the next value");
        // A LAN game: one view per computer, no band.
        let mut h = Nav::new(Vec::new());
        h.key(NavKey::Down);
        h.key(NavKey::Enter);
        assert_eq!(h.mode, Some(CoopMode::LanHost));
        assert!(!h.entries().contains(&Entry::SplitLayout));
        assert!(h.entries().contains(&Entry::ExtraEnemies));
    }

    #[test]
    fn host_chooses_a_game_and_join_asks_for_the_address() {
        let mut n = Nav::new(Vec::new());
        n.key(NavKey::Down);
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::LanHost));
        assert_eq!((n.step, n.focused()), (Step::Game, Some(Entry::NewGame)));
        assert_eq!(n.key(NavKey::Back), Outcome::BackToMode);
        assert_eq!(n.focused(), Some(Entry::LanHost), "back on the way chosen");
        n.key(NavKey::Down);
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::LanJoin));
        assert_eq!((n.step, n.focused(), n.mode), (Step::Join, Some(Entry::Address), Some(CoopMode::LanJoin)));
        assert_eq!(n.key(NavKey::Enter), Outcome::None, "Enter in the box ends the typing only");
        n.key(NavKey::Down);
        assert_eq!(n.key(NavKey::Enter), Outcome::Play(None), "the host chooses the game");
        assert_eq!(n.key(NavKey::Click(1)), Outcome::Play(None));
        assert_eq!(n.key(NavKey::Back), Outcome::BackToMode);
        assert_eq!((n.step, n.focused()), (Step::Mode, Some(Entry::LanJoin)));
    }

    /// The games found on the network come under the address box and its Join row: one is
    /// joined with Enter or a click, one of another version is greyed and never chosen, and the
    /// focus goes back to Join when its game leaves the list.
    #[test]
    fn join_lists_the_games_found() {
        let mut n = Nav::new(Vec::new());
        n.focus = 2;
        assert_eq!(n.key(NavKey::Enter), Outcome::Mode(CoopMode::LanJoin));
        assert_eq!(n.entries(), &[Entry::Address, Entry::JoinNow], "nothing found yet");
        n.set_found(&[true, false, true]);
        assert_eq!(n.entries(), &[Entry::Address, Entry::JoinNow, Entry::Found(0), Entry::Found(1), Entry::Found(2)]);
        n.key(NavKey::End);
        assert_eq!(n.focused(), Some(Entry::Found(2)));
        assert_eq!(n.key(NavKey::Enter), Outcome::JoinFound(2));
        n.key(NavKey::Up);
        assert_eq!(n.focused(), Some(Entry::Found(0)), "the other version is skipped");
        assert_eq!(n.key(NavKey::Click(1 + 2)), Outcome::None, "greyed: not chosen");
        assert_eq!(n.key(NavKey::Click(2)), Outcome::JoinFound(0));
        n.key(NavKey::End);
        n.set_found(&[true]);
        assert_eq!(n.focused(), Some(Entry::JoinNow), "its game left the list");
        n.set_found(&[true; 9]);
        assert_eq!(n.entries().len(), 2 + FOUND_ROWS, "at most FOUND_ROWS games");
        // Leaving the step: the list is not shown on the first step.
        assert_eq!(n.key(NavKey::Back), Outcome::BackToMode);
        assert!(!n.entries().contains(&Entry::Found(0)));
    }

    #[test]
    fn join_address_kept_between_runs() {
        let dir = std::env::temp_dir().join(format!("oxiii-coop-join-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        set_join_file(Some(dir.join("CoopJoin.ini")));
        assert_eq!(last_join_address(), "", "nothing kept yet");
        keep_join_address("192.0.2.20:7778");
        assert_eq!(last_join_address(), "192.0.2.20:7778");
        keep_join_address("127.0.0.1");
        assert_eq!(last_join_address(), "127.0.0.1");
        set_join_file(None);
        assert_eq!(last_join_address(), "", "not kept without a file");
        let _ = std::fs::remove_dir_all(&dir);
        // The box takes an address with its port and a host name.
        assert!("192.0.2.20:7778".chars().chain("my-pc".chars()).all(|c| ADDRESS_CHARS.contains(c)));
    }

    #[test]
    fn the_session_follows_the_page() {
        let mut vm = Vm::default();
        // What coop_page_key does with each outcome, without a menu.
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: None }));
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::SplitScreen, start: Some(CoopStart::Load) }));
        assert_eq!(vm.take_local_play_change().and_then(|p| p.coop).and_then(|s| s.start), Some(CoopStart::Load));
        vm.set_menu_local_players(2);
        assert_eq!(vm.local_play().local_players, 2, "2 is accepted and kept");
        vm.set_menu_local_players(3);
        assert_eq!(vm.local_play().local_players, 2);
    }

    #[test]
    fn settings_only_on_the_two_pages() {
        // The Join LAN page has the address box in its place (multi_join.rs).
        assert!(players_setting_rect("XIIIMenuMultiLANJoin").is_none());
        for c in ["XIIIMenuMultiLANHost", "XIIIMenuBotChallengeSetupClient"] {
            let r = players_setting_rect(c).unwrap();
            assert!(r[0] >= 0.0 && r[0] + r[2] <= 640.0 && r[1] >= 0.0 && r[1] + r[3] <= 450.0, "{c}: inside the page, above the help bar");
            assert!(players_setting_hooked(c, "Created") && players_setting_hooked(c, "Paint"));
            assert!(!players_setting_hooked(c, "AfterPaint"));
            // Keys and start only where the setting can be changed (the offline page, hosting).
            let ready = local_players_ready(c);
            assert!(ready, "{c}");
            assert_eq!(players_setting_hooked(c, "InternalOnKeyEvent"), ready, "{c}");
            assert_eq!(players_setting_hooked(c, "StartPressed"), ready, "{c}");
        }
        assert!(players_setting_rect("XIIIMenuMultiplayer").is_none());
    }

    #[test]
    fn bot_challenge_url_made_the_split_setups() {
        // Default bot set (NbBots 0): NBots = IP - NP, one bot fewer for the second player.
        let one = "DM_Banque?Game=XIIIMP.XIIIMPGameInfo?IP=6?NP=1?FF=0?FR=20?TI=10?NBots=5?TB0=1?NB0=1?SK=x?Name=XIII";
        assert_eq!(
            split_travel_url(one, 2),
            "DM_Banque?Game=XIIIMP.XIIIMPGameInfo?IP=6?NP=2?FF=0?FR=20?TI=10?NBots=4?TB0=1?NB0=1?SK=x?Name=XIII"
        );
        // Bots chosen: kept when they fit, else IP - NP (NbBots = Min(NbBots, IdealPC - N)).
        assert!(split_travel_url("M?IP=8?NP=1?NBots=3", 2).ends_with("?NP=2?NBots=3"));
        assert!(split_travel_url("M?IP=4?NP=1?NBots=3", 2).ends_with("?NP=2?NBots=2"));
        assert!(split_travel_url("M?IP=2?NP=1?NBots=1", 2).ends_with("?NP=2?NBots=0"), "never below 0");
        // Missing options are added: NBots always given in split screen.
        assert_eq!(split_travel_url("M?Game=G?IP=6", 2), "M?Game=G?IP=6?NP=2?NBots=4");
        // One player: the page's own URL.
        assert_eq!(split_travel_url(one, 1), one);
        // Over the page's ranges: NP and NBots always as the split setup computes them.
        for ip in 2..=16 {
            for bots in 0..=16 {
                for n in 1..=2u8 {
                    let url = split_travel_url(&format!("Map?IP={ip}?NP=1?NBots={bots}"), n);
                    let want = bots.min(ip - i32::from(n)).max(0);
                    assert_eq!(url, format!("Map?IP={ip}?NP={n}?NBots={want}"));
                }
            }
        }
    }

    #[test]
    fn setting_changed_offline_and_hosting_only() {
        let mut vm = Vm::default();
        vm.set_menu_local_players(2);
        assert_eq!(vm.local_play().local_players, 2);
        assert!(local_players_ready("xiiimenubotchallengesetupclient"));
        assert!(local_players_ready("XIIIMenuMultiLANHost") && !local_players_ready("XIIIMenuMultiLANJoin"));
    }
}
