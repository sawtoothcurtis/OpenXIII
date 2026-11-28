//! "Select level" of the standalone game (OpenXIII addition; not in the original game): an entry at the
//! bottom left of the main menu opens the list of campaign maps in story
//! order, each followed by its checkpoints. Mouse, keyboard, gamepad and touch. As in the
//! coop mod's selector, test weapons (Beretta and M16 by default, or any of the
//! game's) are given with full ammo, for the maps that expect the weapons of the
//! previous mission.
//!
//! A checkpoint is an XIIISaveGameTrigger of the map that has a TeleporterName. Each choice
//! (checkpoint or map start) starts like loading a real save at that point
//! (openxiii_vm::select_level: record written as DoSave and SaveAtCheckpoint do, then
//! LaunchFromCheckpoint), with the inventory the story guarantees at that point.
//! Improved build: the game's own Select level page (the VM's select_level_page.rs) replaces the
//! list drawn here; L and gamepad Y open it, and the checked weapons are shared with it.
//!
//! The table below has 34 maps in story order and 39 checkpoints (one per Teleporter, sorted
//! by description).

use openxiii_vm::canvas::HudDraw;
use openxiii_vm::Vm;

/// A campaign map: file name, description of its start, checkpoints (Tag of the
/// Teleporter, Tag of the XIIISaveGameTrigger, its SoundToLaunch music, description).
pub struct LevelEntry {
    pub map_value: &'static str,
    pub name_value: &'static str,
    pub points: &'static [(&'static str, &'static str, &'static str, &'static str)],
}

pub const LEVELS: &[LevelEntry] = &[
    LevelEntry { map_value: "Plage00", name_value: "Brighton Beach 1", points: &[] },
    LevelEntry { map_value: "Plage01", name_value: "Brighton Beach 2", points: &[("teleporte_apres_lit", "CP1", "XIIIsound.Music__Plage01.Plage01__hMusicInit2", "Brighton Beach 3")] },
    LevelEntry { map_value: "Banque01", name_value: "Winslow Bank", points: &[("Teleporter_bricole", "savegame_bricole", "XIIIsound.Music__Banque01.Banque01__hMusicInit1", "Winslow Bank 1"), ("Teleporter_otage", "savegame_otage", "XIIIsound.Music__Banque01.Banque01__hMusicInit2", "Winslow Bank 2")] },
    LevelEntry { map_value: "Amos01", name_value: "FBI", points: &[("Sauvegarde1_teleporter", "Sauvegarde1", "XIIIsound.Music__Amos01.Amos01__hMusicInitPostCine", "FBI 1"), ("Sauvegarde2_teleporter", "Sauvegarde2", "XIIIsound.Music__Amos01.Amos01__hMusicInitEtage", "FBI 2")] },
    LevelEntry {
        map_value: "Toits01",
        name_value: "Major Jones",
        points: &[("Toits_CheckPointbis", "Toit_save1bis", "XIIIsound.Music__Toits01.Toits01__hMusicInit1", "Major Jones 1"), ("PlayerStartToit1", "Toit_save2", "XIIIsound.Music__Toits01.Toits01__hMusicInit2", "Major Jones 2"), ("PlayerStartToit2", "Toit_save3", "XIIIsound.Music__Toits01.Toits01__hMusicInit3", "Major Jones 3")],
    },
    LevelEntry { map_value: "Hual01a", name_value: "Emerald Base bridge", points: &[] },
    LevelEntry { map_value: "Hual01b", name_value: "Emerald Base roof", points: &[] },
    LevelEntry {
        map_value: "Hual02",
        name_value: "Carrington's cell",
        points: &[("teleporter1", "XIIISaveGameTrigger1", "XIIIsound.Music__Hualpar2.Hualpar2__hMusicInit2", "Carrington's cell 1"), ("teleporter3", "XIIISaveGameTrigger3", "XIIIsound.Music__Hualpar2.Hualpar2__hMusicInit3", "Carrington's cell 2"), ("teleporter2", "XIIISaveGameTrigger2", "XIIIsound.Music__Hualpar2.Hualpar2__hMusicInit4", "Carrington's cell 3")],
    },
    LevelEntry { map_value: "Hual04a", name_value: "Cable car station", points: &[] },
    LevelEntry { map_value: "Hual04c", name_value: "Cable car", points: &[("CheckPointHual04c", "firstsave", "XIIIsound.Music__Hual04c.Hual04c__hMusicInit1", "Cable car 1")] },
    LevelEntry { map_value: "Kello01a", name_value: "Kellownee Lake", points: &[("CheckPointkello01a", "saveafterzelack", "", "Kellownee Lake 1")] },
    LevelEntry { map_value: "Kello01b", name_value: "Kellownee hideout", points: &[("Checkpoint", "CP1", "XIIIsound.Music__Kello01b.Kello01b__hMusicInit2", "Kellownee hideout 1")] },
    LevelEntry {
        map_value: "PRock01a",
        name_value: "Plain Rock 1",
        points: &[("checkpoint_apres_cine", "CP1", "XIIIsound.Music__PRock01a.PRock01a__hMusicInit1", "Plain Rock 2"), ("checkpoint2_droit", "CP2", "XIIIsound.Music__PRock01a.PRock01a__hMusicInit2", "Plain Rock 3"), ("checkpoint2_gauche", "None", "XIIIsound.Music__PRock01a.PRock01a__hMusicInit2", "Plain Rock 4")],
    },
    LevelEntry { map_value: "PRock03", name_value: "Doc Johansson", points: &[("Teleporter_Jo", "SaveGame_Jo", "XIIIsound.Music__PRock03.PRock03__hMusicInit2", "Doc Johansson 1")] },
    LevelEntry { map_value: "PRock04a", name_value: "Canyon 1", points: &[("DEnis", "XIIISaveGameTrigger9845", "XIIIsound.Music__PRock04a.PRock04a__hMusicInit2", "Canyon 2"), ("KArine", "XIIISaveGameTrigger98765", "XIIIsound.Music__PRock04a.PRock04a__hMusicInit2", "Canyon 3")] },
    LevelEntry { map_value: "PRock04b", name_value: "Canyon 4", points: &[("save_01", "XIIISaveGameTrigger1", "XIIIsound.Music__PRock04b.PRock04b__hMusicInit2", "Canyon 5"), ("save_02", "XIIISaveGameTrigger", "XIIIsound.Music__PRock04b.PRock04b__hMusicInit2", "Canyon 6")] },
    LevelEntry { map_value: "SPADS01", name_value: "Sewage", points: &[] },
    LevelEntry { map_value: "SPADS02a", name_value: "SPADS camp 1", points: &[("slave1Spads2a", "XIIISaveGameTrigger", "XIIIsound.Music__Spads02a.Spads02a__hMusicInit1", "SPADS camp 2"), ("slave2Spads2a", "XIIISaveGameTrigger", "XIIIsound.Music__Spads02a.Spads02a__hMusicInit2", "SPADS camp 3")] },
    LevelEntry { map_value: "SPADS02b", name_value: "McCall", points: &[("Spads02bSlave1", "None", "XIIIsound.Music__Spads02b.Spads02b__hMusicInit1", "McCall 1")] },
    LevelEntry { map_value: "Base01", name_value: "Submarine base", points: &[("Base01Save1", "Base01Save1", "XIIIsound.Music__Base01.Base01__hMusicInit2", "Submarine base 1")] },
    LevelEntry { map_value: "SMarin01", name_value: "Submarine 1", points: &[("SM01_teleporter1", "SaveGameSM01", "XIIIsound.Music__SM01.SM01__hMusicInit2", "Submarine 2")] },
    LevelEntry { map_value: "SMarin02", name_value: "Submarine 3", points: &[("SM02_teleport1", "SM02_save2", "XIIIsound.Music__SM02.SM02__hMusicInit2", "Submarine 4")] },
    LevelEntry { map_value: "USA01", name_value: "Sabotage", points: &[("jez_one", "XIIISaveGameTrigger", "", "Sabotage 1")] },
    LevelEntry { map_value: "USA02", name_value: "Quay 33", points: &[("helicosave", "XIIISaveGameTrigger", "XIIIsound.Music__USA02.USA02__hMusicInit1", "Quay 33-1")] },
    LevelEntry { map_value: "Palace01", name_value: "Bristol Suites Hotel", points: &[("Palace01Save1", "Palace01Save1", "XIIIsound.Music__Palace01.Palace01__hMusicInit2", "Bristol Suites Hotel 1")] },
    LevelEntry { map_value: "Sanc01", name_value: "Sanctuary garden", points: &[("Checkpoint", "CP1", "XIIIsound.Music__Sanc01.Sanc01__hMusicInit2", "Sanctuary garden 1")] },
    LevelEntry { map_value: "Sanc02a", name_value: "Sanctuary hall", points: &[("Sanc02aSave_01", "XIIISaveGameTrigger", "XIIIsound.Music__Sanc02a.Sanc02a__hMusicInit1", "Sanctuary hall 1")] },
    LevelEntry { map_value: "Sanc02b", name_value: "Sanctuary crypt", points: &[("karineallegro", "antibu", "XIIIsound.Music__Sanc02b.Sanc02b__hMusicInit1", "Sanctuary crypt 1")] },
    LevelEntry { map_value: "Sanc03", name_value: "Sanctuary cliff", points: &[("Kallegro", "XIIISaveGameTrigger654", "XIIIsound.Music__Sanc03.Sanc03__hMusicInit2", "Sanctuary cliff 1")] },
    LevelEntry { map_value: "SSH101a", name_value: "SSH1 base admission", points: &[("Sauvegarde2", "XIIISaveGameTrigger2", "XIIIsound.Music__SSH101a.SSH101a__hMusicInit2", "SSH1 base admission 1"), ("Sauvegarde3", "XIIISaveGameTrigger4", "XIIIsound.Music__SSH101a.SSH101a__hMusicInit3", "SSH1 base admission 2")] },
    LevelEntry { map_value: "SSH101b", name_value: "SSH1 trap", points: &[("SSH101bSave1", "XIIISaveGameTrigger", "XIIIsound.Music__SSH101b.SSH101b__hMusicInit2", "SSH1 trap 1")] },
    LevelEntry { map_value: "SSH101c", name_value: "Total Red", points: &[] },
    LevelEntry { map_value: "SSH102b", name_value: "SSH1 final", points: &[("Teleporter_StopBomb", "SaveGame_StopBomb", "XIIIsound.Music__SSH102b.SSH102b__hMusicInit2", "SSH1 final 1")] },
    LevelEntry { map_value: "Bateau01", name_value: "Lady Bee", points: &[] },
];

/// One line of the list: a map's start or one of its checkpoints.
struct Line {
    level_value: usize,
    point: Option<usize>,
}

impl Line {
    fn url(&self) -> String {
        let n = &LEVELS[self.level_value];
        match self.point {
            Some(p) => format!("{}#{}", n.map_value, n.points[p].0),
            None => n.map_value.to_string(),
        }
    }
}

fn lines_list() -> Vec<Line> {
    let mut out = Vec::new();
    for (i, n) in LEVELS.iter().enumerate() {
        out.push(Line { level_value: i, point: None });
        out.extend((0..n.points.len()).map(|p| Line { level_value: i, point: Some(p) }));
    }
    out
}

pub use openxiii_vm::select_level::{WEAPONS, WEAPONS_DEFAULT};

/// The test equipment chosen for a start on map `target` is dropped when `loaded` is the main
/// menu's map: that start was given up before its level (the co-op character screen left with
/// Back, or a game quit), and a later game on the same map must start with its own inventory.
pub fn equipment_dropped_on(target: &str, loaded: &str) -> bool {
    let loaded = loaded.to_ascii_lowercase();
    loaded.starts_with("mapmenu") && !target.to_ascii_lowercase().starts_with("mapmenu")
}

/// The equipment chosen for a start on map `target` is given now, on map `map` (`screen`: the
/// co-op character screen is on). Never on the character screen: it is played on Hual02, the
/// map of the "Carrington's cell" start, and the equipment given there was lost with its game
/// (in a network co-op game, the host gave it there and the guest gave it in the level: both
/// games differed from the start).
pub fn equipment_due(target: &str, map: &str, screen: bool) -> bool {
    (!screen || mutant_equipment_on_screen()) && map.eq_ignore_ascii_case(target)
}

#[cfg(test)]
fn mutant_equipment_on_screen() -> bool {
    false
}
#[cfg(not(test))]
fn mutant_equipment_on_screen() -> bool {
    false
}

/// What becomes of a Select level start's equipment once a map is loaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadoutStep {
    /// Kept for later (the co-op character screen, on the way to its level).
    Keep,
    /// Its level again (first arrival, Retry, a death's restart): given again.
    Give,
    /// Gone: the main menu, a save loaded, or another level (the next one reached by finishing
    /// it carries the game's own inventory, as in the game).
    Drop,
}

/// The equipment chosen for a start on map `target`, once map `loaded` is loaded (`screen`: it is
/// the co-op character screen; `save_loaded`: a save slot was read since the start was chosen).
pub fn loadout_after_travel(target: &str, loaded: &str, screen: bool, save_loaded: bool) -> LoadoutStep {
    if save_loaded || equipment_dropped_on(target, loaded) {
        LoadoutStep::Drop
    } else if screen {
        LoadoutStep::Keep
    } else if loaded.eq_ignore_ascii_case(target) {
        LoadoutStep::Give
    } else {
        LoadoutStep::Drop
    }
}

/// A Select level start's equipment as a lockstep session holds it: the same on every machine,
/// from the host's start (`host_weapons`, the map and the classes the host sent): the equipment
/// to give at the level's first arrival and the loadout given again on a Retry of that level;
/// the save slots read count from the session's start (0). A guest used to have only the first:
/// after a Retry the host gave the equipment again and the guest did not, and both games differed.
pub fn session_equipment<T: Clone>(host_weapons: Option<T>) -> (Option<T>, Option<T>, u32) {
    let loadout = if cfg!(test) && crate::choice_level::mutant_host_only_loadout() { None } else { host_weapons.clone() };
    (host_weapons, loadout, 0)
}

#[cfg(test)]
fn mutant_host_only_loadout() -> bool {
    false
}
#[cfg(not(test))]
fn mutant_host_only_loadout() -> bool {
    false
}

/// List keys (keyboard or gamepad).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    High,
    Low,
    PageHigh,
    PageLow,
    Start,
    EndEntry,
    LevelPrevious,
    LevelNext,
    Weapons,
    Validate,
    Return,
}

/// Phase of a finger on the touch screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Finger {
    Pose,
    Moves,
    Raised,
    Cancels,
}

/// Gamepad buttons read for the menu (all gamepads together).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ButtonsMenu {
    pub high: bool,
    pub low_value: bool,
    pub validate: bool,
    pub return_value: bool,
    pub open_flag: bool,
    pub weapons: bool,
    pub previous: bool,
    pub next_one: bool,
}

impl ButtonsMenu {
    fn none(&self) -> bool {
        *self == ButtonsMenu::default()
    }
}

/// Layout of the last frame (for aiming with the mouse or finger).
#[derive(Clone, Copy, Default)]
struct Set {
    input_value: [f32; 4],
    return_value: [f32; 4],
    weapons: [f32; 4],
    list_value: [f32; 4],
    line_h: f32,
    visible: usize,
}

fn inside(r: [f32; 4], (x, y): (f32, f32)) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

/// Current press (mouse or finger): starting point, first visible line at press time, drag.
struct Press {
    start_point: (f32, f32),
    high: usize,
    slides: bool,
}

const FONT: &str = "XIIIFonts.PoliceF16";
// Colours B, G, R, A (HudDraw).
const WHITE: [u8; 4] = [255, 255, 255, 255];
const GREY: [u8; 4] = [175, 175, 175, 255];
const GREY_DARK: [u8; 4] = [120, 120, 120, 230];
const BACKGROUND: [u8; 4] = [8, 8, 8, 215];
const HIGHLIGHTS: [u8; 4] = [30, 120, 190, 210];
const FRAME: [u8; 4] = [60, 60, 60, 230];

/// Pages of the list: the maps, or the test weapons.
const LEVELS_P: usize = 0;
const WEAPONS_P: usize = 1;

pub struct ChoiceLevel {
    /// List opened over the main menu.
    pub unlocked: bool,
    /// Entry displayed in the last frame (main menu in front).
    menu: bool,
    /// Displayed page (LEVELS_P, WEAPONS_P), selected line and first visible line of each.
    page: usize,
    choice: [usize; 2],
    high: [usize; 2],
    set: Set,
    press: Option<Press>,
    /// Menu cursor position in the last frame, for hovering.
    mouse: (f32, f32),
    /// Gamepads: previous state (edges), held direction and repeat delay.
    pad_before: ButtonsMenu,
    held: i32,
    repeat: f32,
    /// Gamepads hidden from the game: list open, then until every button is released.
    pad_mask: bool,
    /// A mouse press taken by the menu: its release is taken too.
    click_taken: bool,
    /// Finger followed since a press taken by the menu.
    finger: Option<u64>,
    /// Checked test weapons (one bit per line of WEAPONS): kept while the game runs, back to
    /// WEAPONS_DEFAULT (Beretta and M16) at each launch (the owner's choice, never on disk).
    weapons: u32,
    /// Improved build: L or gamepad Y asked for the game's Select level page (opened at the next
    /// frame, which has the VM).
    want_page: bool,
}

impl Default for ChoiceLevel {
    fn default() -> Self {
        ChoiceLevel {
            unlocked: false,
            menu: false,
            page: LEVELS_P,
            choice: [0; 2],
            high: [0; 2],
            set: Set::default(),
            press: None,
            mouse: (0.0, 0.0),
            pad_before: ButtonsMenu::default(),
            held: 0,
            repeat: 0.0,
            pad_mask: false,
            click_taken: false,
            finger: None,
            weapons: WEAPONS_DEFAULT,
            want_page: false,
        }
    }
}

/// "SELECT LEVEL" entry shown on the main menu: "cheats" build only. In the
/// normal build (without the `cheats` feature), no entry, no L / T keys, no list: everything goes
/// through `menu`, false.
fn input_visible(menu_main: bool) -> bool {
    super::cheats::ACTIVE && menu_main
}

impl ChoiceLevel {
    /// The entry is displayed (standalone game's main menu in front).
    pub fn active_value(&self) -> bool {
        self.menu
    }

    /// Classes to give at the chosen start: each checked weapon then its ammo (no duplicates).
    pub fn classes_weapons(&self) -> Vec<&'static str> {
        openxiii_vm::select_level::weapon_classes(self.weapons)
    }

    /// Outside the standalone game's main menu: neither entry nor list.
    pub fn hide(&mut self) {
        self.menu = false;
        self.close();
    }

    fn lines_n(&self) -> usize {
        if self.page == WEAPONS_P {
            WEAPONS.len() + 2
        } else {
            LEVELS.iter().map(|n| 1 + n.points.len()).sum()
        }
    }

    fn open_flag(&mut self) {
        if !openxiii_vm::select_level::available() {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("{}", openxiii_vm::select_level::UNAVAILABLE);
            }
            return;
        }
        // Improved build: the game's own Select level page (the VM's select_level_page.rs) replaces
        // the list drawn here.
        if self.request_level_page() { return; }
        self.unlocked = true;
        self.page = LEVELS_P;
        self.press = None;
        self.pad_mask = true;
    }

    fn close(&mut self) {
        self.unlocked = false;
        self.press = None;
        self.finger = None;
    }

    fn show_choice(&mut self) {
        let (p, n) = (self.page, self.set.visible.max(1));
        if self.choice[p] < self.high[p] {
            self.high[p] = self.choice[p];
        } else if self.choice[p] >= self.high[p] + n {
            self.high[p] = self.choice[p] + 1 - n;
        }
        self.high[p] = self.high[p].min(self.lines_n().saturating_sub(n));
    }

    /// Selected line: a map starts; on the weapons page, "All", "None" or a weapon
    /// checked or unchecked.
    fn validate(&mut self) -> Option<String> {
        let i = self.choice[self.page];
        if self.page == WEAPONS_P {
            self.weapons = match i {
                0 => (1 << WEAPONS.len()) - 1,
                1 => 0,
                _ => self.weapons ^ 1 << (i - 2),
            };
            return None;
        }
        self.close();
        lines_list().get(i).map(Line::url)
    }

    /// Weapons page opened, or back to the maps.
    fn page_weapons(&mut self, open_flag: bool) {
        self.page = if open_flag { WEAPONS_P } else { LEVELS_P };
        self.press = None;
    }

    /// Keyboard or gamepad key, list open. Returns the URL to load.
    pub fn key_col(&mut self, t: Key) -> Option<String> {
        let (p, n) = (self.page, self.lines_n());
        let page = self.set.visible.max(1);
        let c = self.choice[p];
        match t {
            Key::High => self.choice[p] = c.saturating_sub(1),
            Key::Low => self.choice[p] = (c + 1).min(n - 1),
            Key::PageHigh => self.choice[p] = c.saturating_sub(page),
            Key::PageLow => self.choice[p] = (c + page).min(n - 1),
            Key::Start => self.choice[p] = 0,
            Key::EndEntry => self.choice[p] = n - 1,
            Key::LevelPrevious | Key::LevelNext if p == LEVELS_P => {
                let everything = lines_list();
                let current_value = everything[c].level_value;
                let target_value = match t {
                    Key::LevelPrevious if everything[c].point.is_some() => current_value,
                    Key::LevelPrevious => current_value.saturating_sub(1),
                    _ => current_value + 1,
                };
                self.choice[p] = everything.iter().position(|l| l.level_value == target_value).unwrap_or(c);
            }
            Key::LevelPrevious | Key::LevelNext => {}
            // Changing page keeps the position of each.
            Key::Weapons => {
                self.page_weapons(p == LEVELS_P);
                return None;
            }
            Key::Validate => return self.validate(),
            Key::Return if p == WEAPONS_P => {
                self.page_weapons(false);
                return None;
            }
            Key::Return => self.close(),
        }
        self.show_choice();
        None
    }

    /// Keyboard key, main menu in front: L opens the list. True if it is taken.
    pub fn key_menu(&mut self) -> bool {
        if self.menu && !self.unlocked {
            self.open_flag();
            return true;
        }
        false
    }

    /// Press (mouse or finger) at (x, y), or at the menu cursor's position. True if the menu takes it.
    pub fn press(&mut self, pos: Option<(f32, f32)>) -> bool {
        let pos = pos.unwrap_or(self.mouse);
        if !self.menu {
            return false;
        }
        if self.unlocked || inside(self.set.input_value, pos) {
            self.press = Some(Press { start_point: pos, high: self.high[self.page], slides: false });
            return true;
        }
        false
    }

    /// Finger (or held mouse) moved: scrolls the list by dragging.
    pub fn moves(&mut self, pos: (f32, f32)) {
        let h = self.set.line_h.max(1.0);
        let n = self.lines_n().saturating_sub(self.set.visible.max(1));
        let unlocked = self.unlocked;
        if let Some(a) = &mut self.press {
            let dy = pos.1 - a.start_point.1;
            if !a.slides && (dy.abs() > h * 0.5 || (pos.0 - a.start_point.0).abs() > h * 0.5) {
                a.slides = true;
            }
            if a.slides && unlocked {
                self.high[self.page] = (a.high as f32 - dy / h).round().clamp(0.0, n as f32) as usize;
            }
        }
    }

    /// Release (mouse or finger). Returns the URL to load if a map is chosen.
    pub fn released(&mut self, pos: Option<(f32, f32)>) -> Option<String> {
        let pos = pos.unwrap_or(self.mouse);
        let a = self.press.take()?;
        if a.slides {
            return None;
        }
        if !self.unlocked {
            if inside(self.set.input_value, pos) {
                self.open_flag();
            }
            return None;
        }
        if inside(self.set.return_value, pos) {
            return self.key_col(Key::Return);
        }
        if self.page == LEVELS_P && inside(self.set.weapons, pos) {
            return self.key_col(Key::Weapons);
        }
        let l = self.set.list_value;
        if inside(l, pos) {
            let i = self.high[self.page] + ((pos.1 - l[1]) / self.set.line_h.max(1.0)) as usize;
            if i < self.lines_n() && i < self.high[self.page] + self.set.visible {
                self.choice[self.page] = i;
                return self.validate();
            }
        }
        None
    }

    /// Mouse button: press taken by the menu? (the release follows the same path).
    pub fn click(&mut self, pressed: bool) -> (bool, Option<String>) {
        if pressed {
            self.click_taken = self.press(None);
            (self.click_taken, None)
        } else if std::mem::take(&mut self.click_taken) {
            (true, self.released(None))
        } else {
            (false, None)
        }
    }

    /// This finger is followed by the menu (placed on the entry or on the list).
    pub fn follows(&self, id: u64) -> bool {
        self.finger == Some(id)
    }

    /// Touch screen: a finger placed on the entry or on the list is followed until it lifts.
    pub fn finger(&mut self, id: u64, phase: Finger, pos: (f32, f32)) -> Option<String> {
        match phase {
            Finger::Pose if self.finger.is_none() => {
                if self.press(Some(pos)) {
                    self.finger = Some(id);
                }
                None
            }
            Finger::Moves if self.finger == Some(id) => {
                self.moves(pos);
                None
            }
            Finger::Raised if self.finger == Some(id) => {
                self.finger = None;
                self.released(Some(pos))
            }
            Finger::Cancels if self.finger == Some(id) => {
                self.finger = None;
                self.press = None;
                None
            }
            _ => None,
        }
    }

    /// Mouse wheel, list open: three lines per step.
    pub fn wheel(&mut self, notches: f32) {
        let n = self.lines_n().saturating_sub(self.set.visible.max(1)) as i32;
        let step_size = -(notches * 3.0).round() as i32;
        let p = self.page;
        self.high[p] = (self.high[p] as i32 + step_size).clamp(0, n) as usize;
    }

    /// Gamepads, main menu in front: Y opens; list open, d-pad or stick to
    /// choose (repeated when held), LB / RB from one map to another, X for weapons, A to
    /// start or check, B to go back. Returns the URL to load, and whether the gamepads are hidden
    /// from the game this frame.
    pub fn pad(&mut self, b: ButtonsMenu, dt: f32) -> (Option<String>, bool) {
        let before_value = std::mem::replace(&mut self.pad_before, b);
        let front = |n: bool, a: bool| n && !a;
        let mut url = None;
        if !self.menu {
            self.pad_mask = false;
            return (None, false);
        }
        if !self.unlocked {
            if front(b.open_flag, before_value.open_flag) {
                self.open_flag();
            }
        } else {
            if front(b.validate, before_value.validate) {
                url = self.key_col(Key::Validate);
            } else if front(b.return_value, before_value.return_value) {
                self.key_col(Key::Return);
            } else if front(b.weapons, before_value.weapons) {
                self.key_col(Key::Weapons);
            } else if front(b.previous, before_value.previous) {
                self.key_col(Key::LevelPrevious);
            } else if front(b.next_one, before_value.next_one) {
                self.key_col(Key::LevelNext);
            }
            let dir = i32::from(b.low_value) - i32::from(b.high);
            if dir == 0 {
                self.held = 0;
            } else if dir != self.held {
                self.held = dir;
                self.repeat = 0.4;
                self.key_col(if dir < 0 { Key::High } else { Key::Low });
            } else {
                self.repeat -= dt;
                if self.repeat <= 0.0 {
                    self.repeat = 0.08;
                    self.key_col(if dir < 0 { Key::High } else { Key::Low });
                }
            }
        }
        if !self.unlocked && b.none() {
            self.pad_mask = false;
        }
        (url, self.pad_mask)
    }

    /// Summary of the checked weapons for the maps page.
    fn resume_weapons(&self) -> String {
        let names_list: Vec<&str> = WEAPONS.iter().enumerate().filter(|(i, _)| self.weapons & 1 << i != 0).map(|(_, a)| a.name_value).collect();
        match names_list.len() {
            0 => "none".to_string(),
            n if n == WEAPONS.len() => "all weapons, full ammo".to_string(),
            1..=3 => format!("{}, full ammo", names_list.join(", ")),
            n => format!("{n} weapons, full ammo"),
        }
    }

    /// Draws the entry and, when open, the list, on a `w` × `h` image, if the main menu
    /// is in front. `gamepad`: a gamepad is connected (the help says so).
    pub fn draw_fn(&mut self, vm: &mut Vm, w: u32, h: u32, gamepad: bool, quads: &mut Vec<HudDraw>) {
        let front = vm.menu_main().is_some();
        // Improved build: the game's Select level page (opened by the Load page's entry, or by L /
        // gamepad Y here) shares the checked weapons with this file.
        self.update_level_page(vm, front);
        self.menu = input_visible(front);
        if !self.menu {
            self.close();
            return;
        }
        let (wf, hf) = (w as f32, h as f32);
        let s = (h / 480).max(1) as f32;
        let cursor_value = vm.cursor_menu_draw(w);
        let mouse = cursor_value.as_ref().map_or(self.mouse, |c| c.0);
        let moves_list = mouse != self.mouse;
        self.mouse = mouse;
        // Held mouse: the list follows as under a finger.
        if self.click_taken && moves_list {
            self.moves(mouse);
        }
        let (_, th) = text_value(vm, 0.0, 0.0, 1.0, "Ag", WHITE, &mut Vec::new());
        let th = th.max(8.0) * s;
        let margin = 12.0 * s;

        // Entry, bottom left (the version mark is bottom right).
        let help = if gamepad { "  (Y)" } else { "  [L]" };
        let (ew, _) = text_value(vm, 0.0, 0.0, s, &format!("SELECT LEVEL{help}"), WHITE, &mut Vec::new());
        let ey = hf - th - 8.0 * s;
        self.set.input_value = [0.0, ey - 6.0 * s, margin + ew + 12.0 * s, hf];
        // Improved build: the Load page's Select level entry replaces this one (L and Y still
        // open the game's page).
        if self.hide_replaced_level_entry() {
            return;
        }
        if !self.unlocked {
            let hover = inside(self.set.input_value, mouse);
            quads.push(Vm::rectangle_free(margin - 6.0 * s, ey - 4.0 * s, margin + ew + 6.0 * s, ey + th + 2.0 * s, if hover { HIGHLIGHTS } else { BACKGROUND }));
            // Greyed out without its optional data table (the entry then only explains why).
            let color_value = if openxiii_vm::select_level::available() { WHITE } else { GREY };
            text_value(vm, margin, ey, s, "SELECT LEVEL", color_value, quads);
            let (tw, _) = text_value(vm, 0.0, 0.0, s, "SELECT LEVEL", WHITE, &mut Vec::new());
            text_value(vm, margin + tw, ey, s, help, GREY, quads);
            return;
        }

        // Open list: background, title, Back button, weapons line, lines, help.
        let p = self.page;
        quads.push(Vm::rectangle_free(0.0, 0.0, wf, hf, BACKGROUND));
        text_value(vm, margin, margin, s, if p == WEAPONS_P { "TEST WEAPONS AND ITEMS" } else { "SELECT LEVEL" }, WHITE, quads);
        let back = if gamepad { "BACK (B)" } else { "BACK [Esc]" };
        let (bw, _) = text_value(vm, 0.0, 0.0, s, back, WHITE, &mut Vec::new());
        self.set.return_value = [wf - margin - bw - 12.0 * s, 0.0, wf, margin + th + 10.0 * s];
        let hover_return = inside(self.set.return_value, mouse);
        quads.push(Vm::rectangle_free(wf - margin - bw - 6.0 * s, margin - 4.0 * s, wf - margin + 6.0 * s, margin + th + 4.0 * s, if hover_return { HIGHLIGHTS } else { FRAME }));
        text_value(vm, wf - margin - bw, margin, s, back, WHITE, quads);

        // Below the title: the test weapons (maps page), or the explanation (weapons page).
        let ay = margin + th + 12.0 * s;
        if p == LEVELS_P {
            let key_weapons = if gamepad { "(X)" } else { "[T]" };
            let line_weapons = format!("Test weapons: {}   {key_weapons}", self.resume_weapons());
            let (aw, _) = text_value(vm, 0.0, 0.0, s, &line_weapons, WHITE, &mut Vec::new());
            self.set.weapons = [margin - 6.0 * s, ay - 4.0 * s, margin + aw + 6.0 * s, ay + th + 4.0 * s];
            quads.push(Vm::rectangle_free(self.set.weapons[0], self.set.weapons[1], self.set.weapons[2], self.set.weapons[3], if inside(self.set.weapons, mouse) { HIGHLIGHTS } else { FRAME }));
            text_value(vm, margin, ay, s, &line_weapons, WHITE, quads);
        } else {
            self.set.weapons = [0.0; 4];
            text_value(vm, margin, ay, s, "Given with full ammo when the chosen level starts.", GREY, quads);
        }

        let line_h = (th * 1.6).round();
        let high_list = ay + th + 14.0 * s;
        let low_list = hf - th - 2.0 * margin;
        let visible = (((low_list - high_list) / line_h).floor() as usize).max(1);
        self.set.list_value = [0.0, high_list, wf, high_list + visible as f32 * line_h];
        self.set.line_h = line_h;
        self.set.visible = visible;
        let total = self.lines_n();
        self.high[p] = self.high[p].min(total.saturating_sub(visible));
        // Mouse hover (not during a drag): the line under the cursor becomes the selection.
        if moves_list && self.press.is_none() && inside(self.set.list_value, mouse) {
            let i = self.high[p] + ((mouse.1 - high_list) / line_h) as usize;
            if i < total {
                self.choice[p] = i;
            }
        }
        let everything = lines_list();
        for k in (self.high[p]..total).take(visible) {
            let y = high_list + (k - self.high[p]) as f32 * line_h;
            let ty = y + (line_h - th) / 2.0;
            if k == self.choice[p] {
                quads.push(Vm::rectangle_free(margin - 6.0 * s, y, wf - margin + 6.0 * s, y + line_h, HIGHLIGHTS));
            }
            if p == WEAPONS_P {
                match k {
                    0 => text_value(vm, margin, ty, s, "All weapons", WHITE, quads),
                    1 => text_value(vm, margin, ty, s, "No weapon", WHITE, quads),
                    _ => {
                        let checked = if self.weapons & 1 << (k - 2) != 0 { "[X]" } else { "[ ]" };
                        text_value(vm, margin, ty, s, &format!("{checked} {}", WEAPONS[k - 2].name_value), WHITE, quads)
                    }
                };
                continue;
            }
            let l = &everything[k];
            let n = &LEVELS[l.level_value];
            match l.point {
                None => {
                    text_value(vm, margin, ty, s, n.name_value, WHITE, quads);
                    let (cw, _) = text_value(vm, 0.0, 0.0, s, n.map_value, GREY, &mut Vec::new());
                    text_value(vm, wf - margin - cw, ty, s, n.map_value, GREY, quads);
                }
                Some(i) => {
                    text_value(vm, margin + 32.0 * s, ty, s, &format!("Checkpoint: {}", n.points[i].3), GREY, quads);
                }
            }
        }
        // Scroll bar.
        let (lh, lb) = (high_list, high_list + visible as f32 * line_h);
        let y1 = lh + (lb - lh) * self.high[p] as f32 / total as f32;
        let y2 = lh + (lb - lh) * ((self.high[p] + visible) as f32 / total as f32).min(1.0);
        quads.push(Vm::rectangle_free(wf - 4.0 * s, y1, wf - 1.0 * s, y2, GREY_DARK));
        let help = match (p == WEAPONS_P, gamepad) {
            (false, true) => "A: start    B: back    X: test weapons    LB / RB: previous / next level",
            (false, false) => "Click or Enter: start    Esc: back    T: test weapons    Wheel: scroll    Left / Right: previous / next level",
            (true, true) => "A: check / uncheck    B or X: back to levels",
            (true, false) => "Click or Enter: check / uncheck    Esc or T: back to levels",
        };
        text_value(vm, margin, hf - th - margin, s, help, GREY_DARK, quads);
        if let Some((_, Some(tile))) = cursor_value {
            quads.push(tile);
        }
    }
}

/// Text in the menu font at (x, y), scaled `s` times (page tiles with point filtering).
/// Also returns its scaled width and height.
fn text_value(vm: &mut Vm, x: f32, y: f32, s: f32, t: &str, color: [u8; 4], quads: &mut Vec<HudDraw>) -> (f32, f32) {
    let (cases, tw, th) = vm.free_text(FONT, 0.0, 0.0, t, color);
    quads.extend(cases.into_iter().map(|d| match d {
        HudDraw::Tile { x1, y1, x2, y2, uv, material, color, point, enlarged, glyph } => HudDraw::Tile { x1: x + x1 * s, y1: y + y1 * s, x2: x + x2 * s, y2: y + y2 * s, uv, material, color, point, enlarged: enlarged || s > 1.0, glyph },
        other_one => other_one,
    }));
    (tw * s, th * s)
}

#[cfg(test)]
mod tests {
    /// Lockstep co-op, a Select level start then Retry of its level: the host (its own page's
    /// start) and the guest (the host's start received at the meeting) hold the same equipment
    /// and give it again on the Retry, both.
    #[test]
    fn session_equipment_is_the_same_on_both_machines() {
        use super::{loadout_after_travel, session_equipment, LoadoutStep};
        let sent = ("Hual01a".to_string(), vec!["XIII.M16", "XIII.Beretta"]);
        // The host's own state after its Select level page (leave_to), the guest's from the meeting.
        let host = session_equipment(Some(sent.clone()));
        let guest = session_equipment(Some(sent.clone()));
        assert_eq!(host, guest, "the same session state");
        // Each machine, after the Retry's reload of the level (no save slot read since the start).
        let step = |state: &(Option<(String, Vec<&str>)>, Option<(String, Vec<&str>)>, u32), reads: u32| {
            state.1.as_ref().map(|(target, _)| loadout_after_travel(target, "Hual01a", false, reads != state.2))
        };
        assert_eq!(step(&host, 0), Some(LoadoutStep::Give));
        assert_eq!(step(&guest, 0), Some(LoadoutStep::Give), "the guest gives it again too");
    }

    use super::*;

    /// "Carrington's cell" starts on Hual02, the character screen's map: the equipment waits for
    /// the level and is never given on the screen (a network co-op game parted at its first steps:
    /// the host gave it on its screen, the guest in the level).
    #[test]
    fn select_level_equipment_waits_past_the_character_screen() {
        assert!(!equipment_due("Hual02", "Hual02", true), "not on the character screen");
        assert!(equipment_due("Hual02", "hual02", false), "in the level");
        assert!(equipment_due("Plage01", "Plage01", false));
        assert!(!equipment_due("Plage01", "Hual02", true));
        assert!(!equipment_due("Plage01", "Plage00", false), "another map");
    }

    #[test]
    fn select_level_equipment_is_dropped_back_on_the_main_menu_only() {
        // A start given up on the character screen (back to MapMenu): the next New game on
        // Plage00 must not receive the equipment chosen for that start.
        assert!(equipment_dropped_on("Plage00", "MapMenu"));
        assert!(equipment_dropped_on("Banque01", "mapmenu"));
        // On the way to its level (the character screen's map) and on its level: kept.
        assert!(!equipment_dropped_on("Plage00", "Hual02"));
        assert!(!equipment_dropped_on("Plage00", "Plage00"));
    }

    #[test]
    fn select_level_equipment_lasts_the_level_run() {
        use LoadoutStep::*;
        // First arrival, Retry and a death's restart reload the same level: given each time.
        assert_eq!(loadout_after_travel("Hual01a", "Hual01a", false, false), Give);
        assert_eq!(loadout_after_travel("Hual01a", "hual01a", false, false), Give);
        // Co-op: the character screen's map on the way: kept.
        assert_eq!(loadout_after_travel("Hual01a", "Hual02", true, false), Keep);
        // Back on the main menu (New game, Continue, Load start from there): dropped (b86ba5884).
        assert_eq!(loadout_after_travel("Plage00", "MapMenu", false, false), Drop);
        assert_eq!(loadout_after_travel("Plage00", "MapMenu", true, false), Drop);
        // A save loaded from the in-game menu, even of the same level: dropped.
        assert_eq!(loadout_after_travel("Hual01a", "Hual01a", false, true), Drop);
        // The next level reached by finishing it: the game's own inventory.
        assert_eq!(loadout_after_travel("Hual01a", "Hual01b", false, false), Drop);
    }

    #[test]
    fn table_levels() {
        if !openxiii_vm::select_level::available() { eprintln!("skipped: {}", openxiii_vm::select_level::UNAVAILABLE); return; }
        assert_eq!(LEVELS.len(), 34);
        assert_eq!(LEVELS.iter().map(|n| n.points.len()).sum::<usize>(), 39);
        let l = lines_list();
        assert_eq!(l[0].url(), "Plage00");
        assert_eq!(l[2].url(), "Plage01#teleporte_apres_lit");
        assert_eq!(l.last().unwrap().url(), "Bateau01");
        // Each line has its start in the start table (select_level), with the same
        // Tag, music and description as the menu list.
        for x in &l {
            let d = openxiii_vm::select_level::start_point(&x.url()).unwrap_or_else(|| panic!("{} without a start", x.url()));
            if let Some(p) = x.point {
                let (_, tag, sound_value, desc) = LEVELS[x.level_value].points[p];
                assert_eq!((d.tag.as_str(), d.sound_value.as_str(), d.description.as_str()), (tag, sound_value, desc), "{}", x.url());
            }
        }
    }

    register_viewer_layer!("level_menu_tests");

    /// Normal build: the main menu has no "SELECT LEVEL" entry (hence no L or T).
    #[test]
    fn input_by_version() {
        assert_eq!(input_visible(true), cfg!(feature = "cheats"));
        assert!(!input_visible(false));
    }

}

register_viewer_layer!("level_menu");
