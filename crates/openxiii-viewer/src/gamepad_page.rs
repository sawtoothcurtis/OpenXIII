//! Gamepad settings page (not in the original PC version): the console versions' controls picture
//! (XIIIMenuControlsWindow: XIIIMenuStart.controlsPS2_1…4, four 320 × 240 quarters of the 640 ×
//! 480 menu) with, at the end of each line of the picture, the action of that button for the
//! chosen player; and the console's advanced page (XIIIMenuAdvancedControlsWindow: vibration, aim
//! assist, inverted look, look speed 0…10). The settings are the VM's gamepad_config.rs, saved in
//! their own file at each change.
//!
//! Remapping: choose a button and press the gamepad button that should do its action (the two
//! actions are exchanged; Start cancels), or choose its action with left / right. Keyboard,
//! mouse and gamepad.
//!
//! Directions follow the screen (spatial navigation over the boxes as drawn): Up / Down to the
//! box above / below, Left / Right along the bottom row; on a line with a value (a button's
//! action, the player, an advanced setting) Left / Right change that value.
//!
//! Opened over a menu of the game (`App::open_gamepad_settings`); it closes with that menu. In
//! split screen each player opens their own page from their own menu (the in-game menu's
//! Controls, then Gamepad): it is drawn in their view, shows their settings only and follows
//! their gamepad only (`App::open_gamepad_settings_slot`).

use openxiii_vm::canvas::HudDraw;
use openxiii_vm::gamepad_config::{Action, Button, GamepadConfig, PlayerPad, LOOK_SPEED_MAX, PLAYERS};
use openxiii_vm::Vm;

const FONT: &str = "XIIIFonts.PoliceF16";
const SMALL_FONT: &str = "XIIIFonts.XIIIConsoleFont";
/// The console page's four quarters (sBackground(0…3)).
const PICTURE: [&str; 4] = [
    "XIIIMenuStart.Control_Console.controlsps2_1",
    "XIIIMenuStart.Control_Console.controlsps2_2",
    "XIIIMenuStart.Control_Console.controlsps2_3",
    "XIIIMenuStart.Control_Console.controlsps2_4",
];
const ARROW_LEFT: &str = "XIIIMenuStart.Control_Console.fleche_gauche";
const ARROW_RIGHT: &str = "XIIIMenuStart.Control_Console.fleche_droite";
const ARROW_UP: &str = "XIIIMenuStart.Control_Console.fleche_top";
const ARROW_DOWN: &str = "XIIIMenuStart.Control_Console.fleche_down";
// Colours B, G, R, A (HudDraw).
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const GREY: [u8; 4] = [110, 110, 110, 255];
const LIGHT: [u8; 4] = [225, 225, 225, 255];
const RED: [u8; 4] = [0, 0, 200, 255];
const DIM: [u8; 4] = [0, 0, 0, 160];
const DARK: [u8; 4] = [40, 40, 40, 255];
/// Length of the rumble that answers vibration being switched on, and its strength (both
/// motors, 0 to 1).
pub(crate) const RUMBLE_TEST_SECONDS: f32 = 0.35;
pub(crate) const RUMBLE_TEST_FORCE: [f32; 2] = [0.6, 0.6];

/// The motors of gamepad handle `h` this frame: the game's `wanted`, raised to the feedback
/// rumble while it lasts for the handle's player (`test`: player and seconds left), and zero
/// when that player turned vibration off.
pub(crate) fn rumble_force(wanted: [f32; 2], vibration_on: bool, player: usize, test: Option<(usize, f32)>) -> [f32; 2] {
    if !vibration_on {
        return [0.0; 2];
    }
    match test {
        Some((p, left)) if p == player && left > 0.0 => [wanted[0].max(RUMBLE_TEST_FORCE[0]), wanted[1].max(RUMBLE_TEST_FORCE[1])],
        _ => wanted,
    }
}

use crate::gamepad_layout::{button_name, neighbour, neighbour_across, towards, wrap, Dir, Layout, Rect, Texts, LABELS};

/// The bottom row (advanced, default values, back). On the buttons page Left / Right move
/// everywhere; on the advanced page they change the focused setting, and move along this row
/// only.
fn in_bar(item: Item) -> bool {
    matches!(item, Item::Advanced | Item::Defaults | Item::Back)
}

/// Gamepad buttons that drive the page while a button is edited (confirm, cancel, choose the
/// action): never taken as the new button of the action.
fn drives_the_edit(b: Button) -> bool {
    matches!(b, Button::A | Button::B | Button::Start | Button::Up | Button::Down | Button::Left | Button::Right)
}

/// A button being edited (the red "?"): Left / Right choose its action, Jump (A, Enter)
/// confirms, Back (B, Esc) cancels, another gamepad button exchanges the two directly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Edit {
    button: Button,
    /// The action chosen so far (None: still the "?").
    choice: Option<Action>,
}

/// The box of `item` in layout `l`: a button's whole line (name and action), a box, a row of the
/// advanced page.
fn item_rect(l: &Layout, item: Item) -> Option<Rect> {
    let union = |a: Rect, b: Rect| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
    match item {
        Item::Button(b) => l.rows.iter().find(|r| LABELS[r.index].0 == b).map(|r| r.name.map_or(r.value, |n| union(n, r.value))),
        Item::Player => Some(l.player),
        Item::Advanced => Some(l.advanced),
        Item::Defaults => Some(l.defaults),
        Item::Back => Some(l.back),
        Item::Vibration => l.adv_rows.first().copied(),
        Item::AimAssist => l.adv_rows.get(1).copied(),
        Item::InvertLook => l.adv_rows.get(2).copied(),
        Item::LookSpeed => l.adv_rows.get(3).copied(),
    }
}

/// The page at 640 × 480 with English stand-ins measured roughly: the directions before the
/// first frame (and in the tests without a window).
fn stand_in_layout() -> Layout {
    let pad = PlayerPad::default();
    let t = Texts {
        title: "Controls".into(),
        player: "Player 1".into(),
        labels: LABELS.iter().map(|(b, ..)| pad.action(*b).english().to_string()).collect(),
        advanced: "Advanced".into(),
        defaults: "Default values".into(),
        back: "BACK (B)".into(),
        help: String::new(),
        advanced_title: "Advanced".into(),
        advanced_rows: ["Vibration", "Aim assist", "Invert look", "Look speed"].iter().map(|s| s.to_string()).collect(),
        values: vec!["Yes".into(), "No".into()],
    };
    let measure = |small: bool, s: &str| {
        let n = s.chars().count() as f32;
        if small {
            (n * 9.0, 13.0)
        } else {
            (n * 13.0, 18.0)
        }
    };
    Layout::new(640.0, 480.0, &t, &measure)
}

/// Something that can have the focus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Button(Button),
    Player,
    Advanced,
    Defaults,
    Back,
    Vibration,
    AimAssist,
    InvertLook,
    LookSpeed,
}

/// Focus order of the buttons page: the picture's labels from top left to bottom right (Start,
/// which stays the menu button, is not chosen), then the bottom row.
fn main_items() -> Vec<Item> {
    let mut v: Vec<Item> = LABELS.iter().map(|l| l.0).filter(|b| !b.locked()).map(Item::Button).collect();
    v.extend([Item::Player, Item::Advanced, Item::Defaults, Item::Back]);
    v
}

const ADVANCED_ITEMS: [Item; 5] = [Item::Vibration, Item::AimAssist, Item::InvertLook, Item::LookSpeed, Item::Back];

/// Keys of the page (keyboard, gamepad or test scripts).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,
}

impl Key {
    pub fn from_name(s: &str) -> Option<Key> {
        Some(match s {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "enter" => Key::Enter,
            "back" => Key::Back,
            _ => return None,
        })
    }
}

/// What the page asks of the App after a key: settings changed (to apply and save).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Changed {
    pub settings: bool,
    /// Player 1's aim assist changed (it lives in User.ini).
    pub aim_assist: bool,
    /// Vibration was just switched on for this player (0…): a short rumble of their gamepad
    /// shows it works.
    pub rumble: Option<usize>,
}

/// Page state.
#[derive(Default)]
pub struct GamepadPage {
    pub open: bool,
    /// Player whose settings are shown (0…3).
    pub player: usize,
    /// View (viewport) the page belongs to: its menu, its rectangle, its gamepad (0 = the first
    /// player's, or the whole window without split screen).
    pub slot: usize,
    /// The page of one player in split screen: the player cannot be changed.
    locked: bool,
    advanced: bool,
    focus: usize,
    /// Button being edited (its action chosen, or a gamepad press awaited).
    editing: Option<Edit>,
    /// Last change, shown on the help line: text, seconds left.
    note: Option<(String, f32)>,
    /// Save error.
    pub error: Option<String>,
    /// Gamepad: previous frame's button word and menu directions.
    pad_before: u16,
    dirs_before: [bool; 4],
    /// Improved build: the stick's held directions and the repeats of held directions, as in
    /// the other menus (the VM's menu_repeat.rs).
    menu_navigation: crate::MenuNavigation,
    /// Buttons held when the page opened or closed: ignored until released.
    pad_ignores: u16,
    /// Mouse areas of the last frame, in screen pixels.
    areas: Vec<(Item, [f32; 4])>,
    /// Layout of the last frame: the boxes the directions move between.
    layout: Option<Layout>,
    /// Box of the bottom row that last had the focus: Down from above comes back to it.
    last_bar: Option<Item>,
    /// What had the focus before Down went to the bottom row: Up comes back to it.
    last_above: Option<Item>,
    /// Slider of the look speed: left, right (pixels).
    slider: [f32; 2],
    mouse: (f32, f32),
    click_taken: bool,
    /// The box (and page) that had the focus at the last frame drawn: the focus tick.
    ticked_focus: Option<(bool, Item)>,
}

fn inside(r: [f32; 4], (x, y): (f32, f32)) -> bool {
    x >= r[0] && x < r[2] && y >= r[1] && y < r[3]
}

/// Texts the game does not have, in its five languages (int, frt, det, itt, est).
#[derive(Clone, Copy)]
enum Txt {
    Defaults,
    Sensitivity,
    EditKeys,
    EditPad,
    HelpKeys,
    HelpPad,
    Swapped,
    OtherPlayers,
    CannotWrite,
    Menu,
}

fn txt(lang: &str, t: Txt) -> &'static str {
    let i = ["int", "frt", "det", "itt", "est"].iter().position(|l| l.eq_ignore_ascii_case(lang)).unwrap_or(0);
    let row: [&str; 5] = match t {
        Txt::Defaults => ["Default values", "Valeurs par défaut", "Standardwerte", "Valori predefiniti", "Valores por defecto"],
        Txt::Sensitivity => ["Sensitivity", "Sensibilité", "Empfindlichkeit", "Sensibilità", "Sensibilidad"],
        Txt::EditKeys => [
            "Left / Right: choose the action   Enter: confirm   Esc: cancel",
            "Gauche / Droite : choisir l'action   Entrée : valider   Échap : annuler",
            "Links / Rechts: Aktion wählen   Eingabe: bestätigen   Esc: abbrechen",
            "Sinistra / Destra: scegli l'azione   Invio: conferma   Esc: annulla",
            "Izquierda / Derecha: elige la acción   Intro: confirmar   Esc: cancelar",
        ],
        Txt::EditPad => [
            "Left / Right: choose the action   A: confirm   B: cancel   Other button: exchange",
            "Gauche / Droite : choisir l'action   A : valider   B : annuler   Autre bouton : échanger",
            "Links / Rechts: Aktion wählen   A: bestätigen   B: abbrechen   Andere Taste: tauschen",
            "Sinistra / Destra: scegli l'azione   A: conferma   B: annulla   Altro pulsante: scambia",
            "Izquierda / Derecha: elige la acción   A: confirmar   B: cancelar   Otro botón: intercambiar",
        ],
        Txt::HelpKeys => [
            "Enter: change the action   Arrows: move   Esc: back",
            "Entrée : changer l'action   Flèches : se déplacer   Échap : retour",
            "Eingabe: Aktion ändern   Pfeiltasten: bewegen   Esc: zurück",
            "Invio: cambia l'azione   Frecce: spostati   Esc: indietro",
            "Intro: cambiar la acción   Flechas: moverse   Esc: volver",
        ],
        Txt::HelpPad => [
            "A: change the action   B: back",
            "A : changer l'action   B : retour",
            "A: Aktion ändern   B: zurück",
            "A: cambia l'azione   B: indietro",
            "A: cambiar la acción   B: volver",
        ],
        Txt::Swapped => ["Exchanged: {0} / {1}", "Échange : {0} / {1}", "Getauscht: {0} / {1}", "Scambiati: {0} / {1}", "Intercambiadas: {0} / {1}"],
        Txt::OtherPlayers => [
            "Used by this player's gamepad in split screen and co-op",
            "Utilisés par la manette de ce joueur en écran partagé et en co-op",
            "Für das Gamepad dieses Spielers im geteilten Bildschirm und Koop",
            "Usate dal controller di questo giocatore in schermo condiviso e co-op",
            "Usados por el mando de este jugador en pantalla dividida y cooperativo",
        ],
        Txt::CannotWrite => ["Cannot write {0}: {1}", "Impossible d'écrire {0} : {1}", "{0} kann nicht geschrieben werden: {1}", "Impossibile scrivere {0}: {1}", "No se puede escribir {0}: {1}"],
        Txt::Menu => ["Menu", "Menu", "Menü", "Menu", "Menú"],
    };
    row[i]
}

/// `t` with `{0}`, `{1}`… replaced by `args`.
fn fill(t: &str, args: &[&str]) -> String {
    let mut s = t.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

/// The bottom row's "default values" box.
pub(crate) fn defaults_text(lang: &str) -> &'static str {
    txt(lang, Txt::Defaults)
}

/// The help line of the buttons page, for a player with a gamepad or on the keyboard.
pub(crate) fn help_text(lang: &str, gamepad: bool) -> &'static str {
    txt(lang, if gamepad { Txt::HelpPad } else { Txt::HelpKeys })
}

/// Name of the look speed row of the advanced page.
#[cfg(test)]
pub(crate) fn sensitivity_text(lang: &str) -> &'static str {
    txt(lang, Txt::Sensitivity)
}

/// The help line while a button is edited.
#[cfg(test)]
pub(crate) fn edit_help_text(lang: &str, gamepad: bool) -> &'static str {
    txt(lang, if gamepad { Txt::EditPad } else { Txt::EditKeys })
}

/// Name of an action the game has no word for (testing the layout without a game).
#[cfg(test)]
pub(crate) fn txt_for(lang: &str, a: Action) -> String {
    if a == Action::Menu {
        txt(lang, Txt::Menu).to_string()
    } else {
        a.english().to_string()
    }
}

/// Name of an action in the game's language: the game's own PC key settings label when it has one.
pub fn action_label(vm: &mut Vm, a: Action) -> String {
    match a.game_label() {
        Some((section, key)) => vm.game_text(section, key, "XIDInterf").unwrap_or_else(|| a.english().to_string()),
        None if a == Action::Menu => txt(&vm.game_language(), Txt::Menu).to_string(),
        None => a.english().to_string(),
    }
}

impl GamepadPage {
    /// The focus tick of the game's menus (XIIIGUIBaseButton.MouseEnter plays MnCurseur when a
    /// button takes the focus): true once per frame when the focus is on another box than at the
    /// last frame, whatever moved it (mouse, keys, gamepad), the advanced page's boxes included.
    /// The page opening ticks nothing, nor a focus that stays (one tick per move, never two).
    #[allow(dead_code)]
    fn focus_tick(&mut self) -> bool {
        let now = (self.advanced, self.focused());
        let moved = self.ticked_focus.is_some_and(|f| f != now);
        self.ticked_focus = Some(now);
        moved
    }

    /// Opens the page on player `player`'s buttons. `held`: the gamepad buttons held now
    /// (ignored until released).
    pub fn open(&mut self, player: usize, held: u16) {
        // Also what this page read itself at its last frame (it reads its gamepad even closed):
        // the button that chose the Gamepad entry is never a press on the page, whichever
        // gamepad source gave it.
        let held = held | self.pad_before;
        *self = GamepadPage { open: true, player: player.min(PLAYERS - 1), pad_ignores: held, pad_before: held, mouse: self.mouse, ..Default::default() };
    }

    /// Opens the page of player `player` (0 = first), whose view is viewport `slot`, in split
    /// screen: their settings only.
    pub fn open_in(&mut self, slot: usize, player: usize, held: u16) {
        self.open(player, held);
        self.slot = slot;
        self.locked = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.editing = None;
        self.pad_ignores = self.pad_before;
    }

    fn items(&self) -> Vec<Item> {
        if self.advanced {
            ADVANCED_ITEMS.to_vec()
        } else if self.locked {
            main_items().into_iter().filter(|&i| i != Item::Player).collect()
        } else {
            main_items()
        }
    }

    pub fn focused(&self) -> Item {
        let items = self.items();
        items[self.focus.min(items.len() - 1)]
    }

    /// The button being edited (its red "?").
    pub fn waiting(&self) -> Option<Button> {
        self.editing.map(|e| e.button)
    }

    /// The action shown on the edited button: None while it is still the "?".
    fn edit_choice(&self) -> Option<Action> {
        self.editing.and_then(|e| e.choice)
    }

    fn focus_on(&mut self, item: Item) {
        if let Some(i) = self.items().iter().position(|&x| x == item) {
            self.focus = i;
            if in_bar(item) {
                self.last_bar = Some(item);
            }
        }
    }

    /// The focusable boxes of the page now, as last drawn.
    fn nav_boxes(&self) -> Vec<(Item, Rect)> {
        let fallback;
        let l = match &self.layout {
            Some(l) => l,
            None => {
                fallback = stand_in_layout();
                &fallback
            }
        };
        self.items().into_iter().filter_map(|i| item_rect(l, i).map(|r| (i, r))).collect()
    }

    /// Left / Right move the focus (true), or change the focused setting (the advanced page,
    /// away from its bottom row).
    fn moves_across(&self) -> bool {
        !self.advanced || in_bar(self.focused())
    }

    /// Moves the focus to the box on the screen in direction `dir` (spatial navigation, by the
    /// boxes' places on the screen, not their order): on the buttons page in every direction
    /// between all the boxes, Left / Right staying on the bottom row once there; on the advanced
    /// page Up / Down, and Left / Right along the bottom row only. At an edge it wraps to the far end of the row or column, as the weapon menu
    /// does. Between the bottom row and what is above it, the focus comes back where it was.
    /// False when it cannot move.
    fn step(&mut self, dir: Dir) -> bool {
        let boxes = self.nav_boxes();
        let from_item = self.focused();
        let Some(from) = boxes.iter().position(|b| b.0 == from_item) else { return false };
        let rects: Vec<Rect> = boxes.iter().map(|b| b.1).collect();
        let across = matches!(dir, Dir::Left | Dir::Right);
        // Left / Right along the bottom row stay on it (wrapping at its ends); from any other box
        // they reach every box.
        let in_row = self.advanced || in_bar(from_item);
        let allowed = |i: usize| !across || !in_row || in_bar(boxes[i].0);
        let near = if across { neighbour_across(&rects, from, dir, &allowed) } else { neighbour(&rects, from, dir, &allowed) };
        let Some(mut to) = near.or_else(|| wrap(&rects, from, dir, &allowed)) else { return false };
        if across {
            // Moved along the bottom row: Up now goes to what is above the new box.
            self.last_above = None;
        } else {
            let remembered = match (in_bar(from_item), in_bar(boxes[to].0)) {
                (false, true) => self.last_bar,
                (true, false) => self.last_above,
                _ => None,
            };
            if let Some(m) = remembered.and_then(|m| boxes.iter().position(|b| b.0 == m)) {
                if m != from && towards(rects[from], rects[m], dir).is_some() {
                    to = m;
                }
            }
            if in_bar(boxes[to].0) && !in_bar(from_item) {
                self.last_above = Some(from_item);
            }
        }
        self.focus_on(boxes[to].0);
        true
    }

    /// A key of the page. Returns what changed in `config`.
    pub fn key(&mut self, k: Key, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> Changed {
        if self.editing.is_some() {
            return self.act(k, config, lang, labels);
        }
        // Directions: the box that way on the screen (Left / Right change a setting of the
        // advanced page instead).
        let dir = match k {
            Key::Up => Some(Dir::Up),
            Key::Down => Some(Dir::Down),
            Key::Left if self.moves_across() => Some(Dir::Left),
            Key::Right if self.moves_across() => Some(Dir::Right),
            _ => None,
        };
        if let Some(d) = dir {
            self.step(d);
            return Changed::default();
        }
        self.act(k, config, lang, labels)
    }

    /// A key that is not a move of the focus: on the edited button, or on the focused box.
    fn act(&mut self, k: Key, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> Changed {
        let mut changed = Changed::default();
        let pad = &mut config.players[self.player];
        if let Some(e) = &mut self.editing {
            match k {
                Key::Left | Key::Right => {
                    // The next / previous action, shown in place of the "?" (saved on confirm).
                    let mut trial = *pad;
                    if let Some(a) = e.choice {
                        trial.set(e.button, a);
                    }
                    trial.cycle(e.button, if k == Key::Left { -1 } else { 1 });
                    e.choice = Some(trial.action(e.button));
                }
                Key::Enter => {
                    if let Some(a) = e.choice {
                        changed.settings = pad.set(e.button, a);
                    }
                    self.editing = None;
                }
                Key::Back => self.editing = None,
                Key::Up | Key::Down => {}
            }
            return changed;
        }
        match (k, self.focused()) {
            (Key::Back, _) if self.advanced => {
                self.advanced = false;
                self.focus_on(Item::Advanced);
            }
            (Key::Back, _) | (Key::Enter, Item::Back) => self.close(),
            (Key::Enter, Item::Button(b)) if !b.locked() => self.editing = Some(Edit { button: b, choice: None }),
            (Key::Left | Key::Right | Key::Enter, Item::Player) if !self.locked => {
                let step = if k == Key::Left { PLAYERS - 1 } else { 1 };
                self.player = (self.player + step) % PLAYERS;
            }
            (Key::Enter, Item::Advanced) => {
                self.advanced = true;
                self.focus = 0;
            }
            (Key::Enter, Item::Defaults) => {
                *pad = PlayerPad::default();
                changed = Changed { settings: true, aim_assist: self.player == 0, ..Default::default() };
                self.note = Some((txt(lang, Txt::Defaults).to_string(), 3.0));
            }
            (Key::Left | Key::Right | Key::Enter, Item::Vibration) => {
                pad.vibration = !pad.vibration;
                changed.settings = true;
                changed.rumble = pad.vibration.then_some(self.player);
            }
            (Key::Left | Key::Right | Key::Enter, Item::AimAssist) => {
                pad.aim_assist = !pad.aim_assist;
                changed = Changed { settings: true, aim_assist: self.player == 0, ..Default::default() };
            }
            (Key::Left | Key::Right | Key::Enter, Item::InvertLook) => {
                pad.invert_look = !pad.invert_look;
                changed.settings = true;
            }
            (Key::Left, Item::LookSpeed) if pad.look_speed > 0 => {
                pad.look_speed -= 1;
                changed.settings = true;
            }
            (Key::Right | Key::Enter, Item::LookSpeed) if pad.look_speed < LOOK_SPEED_MAX => {
                pad.look_speed += 1;
                changed.settings = true;
            }
            _ => {}
        }
        let _ = labels;
        changed
    }

    /// The pressed gamepad button `b` while a button is edited: the edited button's action (the
    /// one chosen, if any) goes to `b` and `b`'s to the edited button.
    fn answer(&mut self, b: Button, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> Changed {
        let Some(Edit { button: from, choice }) = self.editing.take() else { return Changed::default() };
        if b.locked() || b == from {
            return Changed::default();
        }
        let pad = &mut config.players[self.player];
        if let Some(a) = choice {
            pad.set(from, a);
        }
        let (a, other) = (pad.action(from), pad.action(b));
        if !pad.swap(from, b) {
            return Changed::default();
        }
        if other != a {
            self.note = Some((fill(txt(lang, Txt::Swapped), &[&labels(a), &labels(other)]), 4.0));
        }
        // The focus follows the action.
        self.focus_on(Item::Button(b));
        Changed { settings: true, ..Default::default() }
    }

    /// One frame of the gamepad: `word` the button word (gamepad_config::word), `stick` the left
    /// stick (x right, y down). Returns true when the page has the gamepad (the game must see it
    /// at rest), and what changed.
    #[allow(clippy::too_many_arguments)]
    pub fn pad(&mut self, word: u16, stick: (f32, f32), dt: f32, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> (bool, Changed) {
        let now = crate::MenuNavigation::clock();
        self.pad_at(word, stick, dt, now, config, lang, labels)
    }

    /// [`Self::pad`] at `now` (seconds of a steady clock: the repeats of held directions).
    #[allow(clippy::too_many_arguments)]
    fn pad_at(&mut self, word: u16, stick: (f32, f32), dt: f32, now: f64, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> (bool, Changed) {
        self.pad_ignores &= word;
        let before = std::mem::replace(&mut self.pad_before, word);
        if let Some((_, t)) = &mut self.note {
            *t -= dt;
            if *t <= 0.0 {
                self.note = None;
            }
        }
        if !self.open {
            // Closed: no direction is held for the menu, nothing repeats.
            self.menu_navigation.released(now);
            return (self.pad_ignores != 0, Changed::default());
        }
        let pressed = word & !before & !self.pad_ignores;
        let mut changed = Changed::default();
        // A button edited: another gamepad button pressed takes its action at once (exchange).
        if self.editing.is_some() {
            if let Some(p) = openxiii_vm::gamepad_config::BUTTONS.into_iter().find(|&p| pressed & p.bit() != 0 && !drives_the_edit(p)) {
                let ch = self.answer(p, config, lang, labels);
                self.dirs_before = [false; 4];
                return (true, ch);
            }
        }
        let bit = |b: Button| word & b.bit() != 0;
        let stick_dirs = self.menu_navigation.directions(stick.0, stick.1);
        let dirs = [bit(Button::Up) || stick_dirs[0], bit(Button::Down) || stick_dirs[1], bit(Button::Left) || stick_dirs[2], bit(Button::Right) || stick_dirs[3]];
        let before_dirs = std::mem::replace(&mut self.dirs_before, dirs);
        // Improved build: a held direction repeats as in the other menus (directions only: A, B
        // and Start never repeat).
        let repeats = self.menu_navigation.repeats(dirs, now);
        for (i, k) in [Key::Up, Key::Down, Key::Left, Key::Right].into_iter().enumerate() {
            if (dirs[i] && !before_dirs[i]) || repeats[i] {
                changed = self.merge(changed, k, config, lang, labels);
            }
        }
        if pressed & Button::A.bit() != 0 {
            changed = self.merge(changed, Key::Enter, config, lang, labels);
        } else if pressed & (Button::B.bit() | Button::Start.bit()) != 0 {
            changed = self.merge(changed, Key::Back, config, lang, labels);
        }
        (true, changed)
    }

    fn merge(&mut self, c: Changed, k: Key, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> Changed {
        let d = self.key(k, config, lang, labels);
        Changed { settings: c.settings || d.settings, aim_assist: c.aim_assist || d.aim_assist, rumble: c.rumble.or(d.rumble) }
    }

    /// Left mouse button: true when the page takes it.
    pub fn click(&mut self, pressed: bool, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> (bool, Changed) {
        if !self.open {
            return (std::mem::take(&mut self.click_taken) && !pressed, Changed::default());
        }
        if pressed {
            self.click_taken = true;
            return (true, Changed::default());
        }
        if !std::mem::take(&mut self.click_taken) {
            return (false, Changed::default());
        }
        let Some(&(item, r)) = self.areas.iter().find(|(_, r)| inside(*r, self.mouse)) else { return (true, Changed::default()) };
        if let Some(e) = self.editing {
            // A click on the edited button confirms it; anywhere else cancels.
            let k = if item == Item::Button(e.button) { Key::Enter } else { Key::Back };
            return (true, self.act(k, config, lang, labels));
        }
        self.focus_on(item);
        if item == Item::LookSpeed && self.slider[1] > self.slider[0] && self.mouse.0 >= self.slider[0] - 8.0 {
            let pad = &mut config.players[self.player];
            let f = ((self.mouse.0 - self.slider[0]) / (self.slider[1] - self.slider[0])).clamp(0.0, 1.0);
            pad.look_speed = (f * f32::from(LOOK_SPEED_MAX)).round() as u8;
            return (true, Changed { settings: true, ..Default::default() });
        }
        // The player box: its left half goes back one player.
        let k = if item == Item::Player && self.mouse.0 < (r[0] + r[2]) / 2.0 { Key::Left } else { Key::Enter };
        (true, self.act(k, config, lang, labels))
    }

    /// Mouse wheel: the previous / next action of the edited button, or the previous / next
    /// value of the setting under the mouse.
    pub fn wheel(&mut self, notches: f32, config: &mut GamepadConfig, lang: &str, labels: &dyn Fn(Action) -> String) -> Changed {
        let k = if notches > 0.0 { Key::Right } else { Key::Left };
        if self.editing.is_some() {
            return self.act(k, config, lang, labels);
        }
        let Some(&(item, _)) = self.areas.iter().find(|(_, r)| inside(*r, self.mouse)) else { return Changed::default() };
        if matches!(item, Item::Back | Item::Advanced | Item::Defaults | Item::Button(_)) {
            return Changed::default();
        }
        self.focus_on(item);
        self.act(k, config, lang, labels)
    }

    /// Draws the page over the menu of its view, in `rect` (x, y, width, height, window pixels)
    /// of a window `screen_w` wide. `gamepad`: the page's player has a gamepad (help text).
    pub fn draw(&mut self, vm: &mut Vm, config: &GamepadConfig, screen_w: u32, rect: [f32; 4], gamepad: bool, quads: &mut Vec<HudDraw>) {
        if !self.open {
            return;
        }
        // Over a page of the game's menus only (this view's): it closes with them.
        if !vm.gui_page_active_slot(self.slot) {
            self.close();
            return;
        }
        let [x0, y0, rw, rh] = rect;
        // The mouse is the first player's (the window's). Mouse areas stay in the page's own
        // pixels, the mouse is brought into them.
        let cursor = if self.slot == 0 { vm.cursor_menu_draw(screen_w) } else { None };
        let mouse = cursor.as_ref().map_or(self.mouse, |c| (c.0 .0 - x0, c.0 .1 - y0));
        let mut local = Vec::new();
        // In a split screen player's view during a level, the game stays visible around the
        // page, as behind that player's other in-game menu pages.
        let backdrop = !vm.split_menus() || vm.url.map.to_ascii_lowercase().starts_with("mapmenu");
        self.draw_local(vm, config, rw.max(1.0), rh.max(1.0), gamepad, mouse, backdrop, &mut local);
        quads.extend(local.into_iter().map(|d| shifted(d, x0, y0)));
        if let Some((_, Some(tile))) = cursor {
            quads.push(tile);
        }
    }

    /// Draws the page over the whole window's menus, by the menus' rule (gui.rs, MenuScreen):
    /// laid out on their screen and placed like them (improved build), or over the whole window.
    pub fn draw_on_screen(&mut self, vm: &mut Vm, config: &GamepadConfig, screen: openxiii_vm::gui::MenuScreen, gamepad: bool, quads: &mut Vec<HudDraw>) {
        let (w, h) = screen.size;
        if screen.frame.is_none() {
            self.draw(vm, config, w, [0.0, 0.0, w as f32, h as f32], gamepad, quads);
            return;
        }
        if !self.open {
            return;
        }
        if !vm.gui_page_active_slot(self.slot) {
            self.close();
            return;
        }
        let start = quads.len();
        // The mouse on the menus' screen (the page's own pixels there).
        let cursor = if self.slot == 0 { vm.cursor_menu_page(w) } else { None };
        let mouse = cursor.as_ref().map_or(self.mouse, |c| c.0);
        self.draw_local(vm, config, w.max(1) as f32, h.max(1) as f32, gamepad, mouse, true, quads);
        if let Some((_, Some(tile))) = cursor {
            quads.push(tile);
        }
        screen.place(&mut quads[start..]);
    }

    /// The page's texts now.
    fn texts(&self, vm: &mut Vm, config: &GamepadConfig, gamepad: bool) -> Texts {
        let lang = vm.game_language();
        let pad = config.players[self.player];
        let labels = LABELS
            .iter()
            .map(|(b, ..)| match (self.waiting() == Some(*b), self.edit_choice()) {
                (true, None) => "?".to_string(),
                (true, Some(a)) => action_label(vm, a),
                (false, _) => action_label(vm, pad.action(*b)),
            })
            .collect();
        let word = vm.game_text("XIIIMenuPlayersSetupClient", "PlayerText", "XIDInterf").unwrap_or_else(|| "Player".into());
        let back = vm.game_text("XIIIWindow", "BackText", "XIDInterf").unwrap_or_else(|| "Back".into()).to_uppercase();
        let help = if let Some(e) = &self.error {
            e.clone()
        } else if self.editing.is_some() {
            txt(&lang, if gamepad { Txt::EditPad } else { Txt::EditKeys }).to_string()
        } else if let Some((n, _)) = &self.note {
            n.clone()
        } else if self.player > 0 && !self.advanced && !self.locked {
            txt(&lang, Txt::OtherPlayers).to_string()
        } else if !self.advanced {
            help_text(&lang, gamepad).to_string()
        } else {
            String::new()
        };
        let adv = |vm: &mut Vm, key: &str| vm.game_text("XIIIMenuAdvancedControlsWindow", key, "XIDInterf").unwrap_or_else(|| key.trim_end_matches("Text").to_string()).trim().to_string();
        Texts {
            title: vm.game_text("XIIIMenuControlsWindow", "TitleText", "XIDInterf").unwrap_or_else(|| "Controls".into()),
            player: format!("{word} {}", self.player + 1),
            labels,
            advanced: vm.game_text("XIIIMenuControlsWindow", "AdvancedText", "XIDInterf").unwrap_or_else(|| "Advanced".into()),
            defaults: defaults_text(&lang).to_string(),
            back: if gamepad { format!("{back} (B)") } else { format!("{back} [Esc]") },
            help,
            advanced_title: adv(vm, "TitleText"),
            // The look speed row is named "Sensitivity", clearer than the game's own label.
            advanced_rows: ["RumbleText", "AutoAimText", "InvPadText"].iter().map(|k| adv(vm, k)).chain(std::iter::once(txt(&lang, Txt::Sensitivity).to_string())).collect(),
            values: vec![
                vm.game_text("XIIICheckBoxControl", "sYes", "XIDInterf").unwrap_or_else(|| "Yes".into()),
                vm.game_text("XIIICheckBoxControl", "sNo", "XIDInterf").unwrap_or_else(|| "No".into()),
            ],
        }
    }

    /// The page in its own pixels (0, 0)–(w, h).
    #[allow(clippy::too_many_arguments)]
    fn draw_local(&mut self, vm: &mut Vm, config: &GamepadConfig, wf: f32, hf: f32, gamepad: bool, mouse: (f32, f32), backdrop: bool, quads: &mut Vec<HudDraw>) {
        let moved = mouse != self.mouse;
        self.mouse = mouse;
        let pad = config.players[self.player];
        self.areas.clear();
        let t = self.texts(vm, config, gamepad);
        let l = {
            let cell = std::cell::RefCell::new(&mut *vm);
            let measure = |small: bool, s: &str| text(&mut cell.borrow_mut(), if small { SMALL_FONT } else { FONT }, 0.0, 0.0, 1.0, s, WHITE, &mut Vec::new());
            Layout::new(wf, hf, &t, &measure)
        };
        let s = l.s;
        self.layout = Some(l.clone());

        if backdrop {
            quads.push(Vm::rectangle_free(0.0, 0.0, wf, hf, BLACK));
        }
        if l.list {
            quads.push(Vm::rectangle_free(l.page[0], l.page[1], l.page[2], l.page[3], DARK));
        }
        let [px0, py0, px1, py1] = l.picture;
        // The list's advanced page covers the middle: no picture peeking out behind it.
        if px1 > px0 && !(l.list && self.advanced) {
            let (qw, qh) = ((px1 - px0) / 2.0, (py1 - py0) / 2.0);
            for (k, tex) in PICTURE.iter().enumerate() {
                let (x, y) = (px0 + (k % 2) as f32 * qw, py0 + (k / 2) as f32 * qh);
                if let Some(t) = vm.texture_free(tex, x, y, x + qw, y + qh, WHITE) {
                    quads.push(t);
                }
            }
        }

        // Title tab.
        bordered(quads, l.title, WHITE);
        let shown = fit(vm, FONT, s, &t.title, l.title[2] - l.title[0] - 8.0 * s);
        let (tw, th) = text(vm, FONT, 0.0, 0.0, s, &shown, BLACK, &mut Vec::new());
        text(vm, FONT, (l.title[0] + l.title[2] - tw) / 2.0, (l.title[1] + l.title[3] - th) / 2.0, s, &shown, BLACK, quads);

        // Player box, top right (a plain label on a player's own page, and on the advanced
        // page, whose keys do not reach it).
        let focus = self.focused();
        if self.locked || self.advanced {
            bordered(quads, l.player, WHITE);
            let shown = fit(vm, FONT, s, &t.player, l.player[2] - l.player[0] - 8.0);
            let (pw, ph) = text(vm, FONT, 0.0, 0.0, s, &shown, BLACK, &mut Vec::new());
            text(vm, FONT, (l.player[0] + l.player[2] - pw) / 2.0, (l.player[1] + l.player[3] - ph) / 2.0, s, &shown, BLACK, quads);
        } else {
            self.item_box(vm, quads, Item::Player, l.player, &t.player, focus == Item::Player, s, true);
        }

        if self.advanced {
            self.draw_advanced(vm, &pad, &l, &t, quads);
        } else {
            // The buttons: on the picture, or a list (name, action).
            let ls = l.label_s;
            for row in &l.rows {
                let b = LABELS[row.index].0;
                let a = pad.action(b);
                let waiting = self.waiting() == Some(b);
                let item = Item::Button(b);
                let focused = !b.locked() && focus == item;
                let (fill_c, ink) = if waiting {
                    (RED, WHITE)
                } else if focused {
                    (BLACK, WHITE)
                } else if b.locked() {
                    (LIGHT, GREY)
                } else {
                    (WHITE, if a == Action::None { GREY } else { BLACK })
                };
                if let Some(n) = row.name {
                    quads.push(Vm::rectangle_free(n[0], n[1], n[2], n[3], if focused { RED } else { LIGHT }));
                    let arrow = match b {
                        Button::Up => Some(ARROW_UP),
                        Button::Down => Some(ARROW_DOWN),
                        Button::Left => Some(ARROW_LEFT),
                        Button::Right => Some(ARROW_RIGHT),
                        _ => None,
                    };
                    match arrow {
                        Some(tex) => {
                            let side = (n[3] - n[1] - 2.0).max(1.0);
                            let cx = (n[0] + n[2]) / 2.0;
                            quads.push(Vm::rectangle_free(cx - side / 2.0, n[1] + 1.0, cx + side / 2.0, n[3] - 1.0, BLACK));
                            if let Some(t) = vm.texture_free(tex, cx - side / 2.0, n[1] + 1.0, cx + side / 2.0, n[3] - 1.0, WHITE) {
                                quads.push(t);
                            }
                        }
                        None => {
                            let name = button_name(b);
                            let (nw, nh) = text(vm, SMALL_FONT, 0.0, 0.0, ls, name, BLACK, &mut Vec::new());
                            text(vm, SMALL_FONT, (n[0] + n[2] - nw) / 2.0, (n[1] + n[3] - nh) / 2.0, ls, name, if focused { WHITE } else { BLACK }, quads);
                        }
                    }
                }
                let r = row.value;
                let (_, lh) = text(vm, SMALL_FONT, 0.0, 0.0, ls, &row.shown, BLACK, &mut Vec::new());
                quads.push(Vm::rectangle_free(r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0, BLACK));
                quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], fill_c));
                text(vm, SMALL_FONT, r[0] + 4.0 * ls, r[1] + ((r[3] - r[1]) - lh) / 2.0, ls, &row.shown, ink, quads);
                if !b.locked() {
                    let hit = row.name.map_or(r, |n| [n[0], r[1], r[2], r[3]]);
                    self.areas.push((item, hit));
                }
            }
            // Bottom row.
            self.item_box(vm, quads, Item::Advanced, l.advanced, &t.advanced, focus == Item::Advanced, s, false);
            self.item_box(vm, quads, Item::Defaults, l.defaults, &t.defaults, focus == Item::Defaults, s, false);
        }

        // Back tab, bottom right.
        self.item_box(vm, quads, Item::Back, l.back, &t.back, focus == Item::Back, s, false);

        // Help line (or the remap in progress, a change, an error).
        if !l.help_shown.is_empty() {
            let colour = if self.error.is_some() || self.editing.is_some() {
                RED
            } else if self.note.is_some() {
                BLACK
            } else {
                GREY
            };
            let r = l.help;
            quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], WHITE));
            let (lw, lh) = text(vm, SMALL_FONT, 0.0, 0.0, l.label_s, &l.help_shown, BLACK, &mut Vec::new());
            text(vm, SMALL_FONT, (r[0] + r[2] - lw) / 2.0, r[1] + (r[3] - r[1] - lh) / 2.0, l.label_s, &l.help_shown, colour, quads);
        }

        // The mouse moves the focus.
        if moved && self.editing.is_none() {
            if let Some(&(item, _)) = self.areas.iter().find(|(_, r)| inside(*r, mouse)) {
                self.focus_on(item);
            }
        }
        // Improved build: the focus tick of the game's menus (MnCurseur).
        self.play_focus_sound(vm);
    }

    /// The advanced page (XIIIMenuAdvancedControlsWindow): a white box over the dimmed picture,
    /// one row for each setting.
    fn draw_advanced(&mut self, vm: &mut Vm, pad: &PlayerPad, l: &Layout, t: &Texts, quads: &mut Vec<HudDraw>) {
        let s = l.s;
        quads.push(Vm::rectangle_free(l.page[0], l.page[1], l.page[2], l.page[3], DIM));
        bordered(quads, l.adv_box, WHITE);
        bordered(quads, l.adv_title, WHITE);
        let shown = fit(vm, FONT, s, &t.advanced_title, l.adv_title[2] - l.adv_title[0] - 8.0 * s);
        let (tw, th) = text(vm, FONT, 0.0, 0.0, s, &shown, BLACK, &mut Vec::new());
        text(vm, FONT, (l.adv_title[0] + l.adv_title[2] - tw) / 2.0, (l.adv_title[1] + l.adv_title[3] - th) / 2.0, s, &shown, BLACK, quads);
        let (yes, no) = (t.values[0].clone(), t.values[1].clone());
        let focus = self.focused();
        let ah = 20.0 * l.adv_row_height() / 30.0;
        for (k, (item, on)) in [(Item::Vibration, Some(pad.vibration)), (Item::AimAssist, Some(pad.aim_assist)), (Item::InvertLook, Some(pad.invert_look)), (Item::LookSpeed, None)].into_iter().enumerate() {
            let row = l.adv_rows[k];
            let yc = (row[1] + row[3]) / 2.0;
            let focused = focus == item;
            if focused {
                quads.push(Vm::rectangle_free(row[0], row[1], row[2], row[3], LIGHT));
            }
            let label = &t.advanced_rows[k];
            let (_, lh) = text(vm, FONT, 0.0, 0.0, s, label, BLACK, &mut Vec::new());
            text(vm, FONT, l.adv_label_x, yc - lh / 2.0, s, label, BLACK, quads);
            match on {
                Some(v) => {
                    let value = if v { &yes } else { &no };
                    let (vw, _) = text(vm, FONT, 0.0, 0.0, s, value, BLACK, &mut Vec::new());
                    let x = l.adv_value_x;
                    if focused {
                        arrows(vm, quads, x - vw / 2.0 - ah - 6.0 * s, x + vw / 2.0 + 6.0 * s, yc, ah);
                    }
                    text(vm, FONT, x - vw / 2.0, yc - lh / 2.0, s, value, BLACK, quads);
                }
                None => {
                    // Slider: 11 notches.
                    let [x0, x1] = l.slider;
                    self.slider = [x0, x1];
                    let u = ah / 20.0;
                    quads.push(Vm::rectangle_free(x0, yc - u, x1, yc + u, BLACK));
                    for i in 0..=LOOK_SPEED_MAX {
                        let x = x0 + (x1 - x0) * f32::from(i) / f32::from(LOOK_SPEED_MAX);
                        quads.push(Vm::rectangle_free(x - 0.5 * u, yc - 4.0 * u, x + 0.5 * u, yc + 4.0 * u, BLACK));
                    }
                    let x = x0 + (x1 - x0) * f32::from(pad.look_speed) / f32::from(LOOK_SPEED_MAX);
                    quads.push(Vm::rectangle_free(x - 4.0 * u, yc - 8.0 * u, x + 4.0 * u, yc + 8.0 * u, if focused { RED } else { BLACK }));
                    let n = pad.look_speed.to_string();
                    let (_, nh) = text(vm, SMALL_FONT, 0.0, 0.0, s, &n, BLACK, &mut Vec::new());
                    text(vm, SMALL_FONT, x1 + 6.0 * u, yc - nh / 2.0, s, &n, BLACK, quads);
                }
            }
            self.areas.push((item, row));
        }
    }

    /// A box that can have the focus (black when it has it), with arrows when `arrows_too`.
    #[allow(clippy::too_many_arguments)]
    fn item_box(&mut self, vm: &mut Vm, quads: &mut Vec<HudDraw>, item: Item, r: [f32; 4], label: &str, focused: bool, s: f32, arrows_too: bool) {
        let (fill_c, ink) = if focused { (BLACK, WHITE) } else if inside(r, self.mouse) { (LIGHT, BLACK) } else { (WHITE, BLACK) };
        quads.push(Vm::rectangle_free(r[0] - 2.0, r[1] - 2.0, r[2] + 2.0, r[3] + 2.0, BLACK));
        quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], fill_c));
        let room = (r[2] - r[0]) - if arrows_too { 50.0 * (r[3] - r[1]) / 32.0 } else { 10.0 };
        let shown = fit(vm, FONT, s, label, room);
        let (lw, lh) = text(vm, FONT, 0.0, 0.0, s, &shown, ink, &mut Vec::new());
        let cx = (r[0] + r[2]) / 2.0;
        text(vm, FONT, cx - lw / 2.0, (r[1] + r[3] - lh) / 2.0, s, &shown, ink, quads);
        if arrows_too {
            let ry = (r[3] - r[1]) / 32.0;
            arrows(vm, quads, r[0] + 4.0 * ry, r[2] - 24.0 * ry, (r[1] + r[3]) / 2.0, 20.0 * ry);
        }
        self.areas.push((item, r));
    }
}

/// The console page's left and right arrows (fleche_gauche / fleche_droite), squares of side
/// `side` at x `left` and `right`, centred on `yc`.
fn arrows(vm: &mut Vm, quads: &mut Vec<HudDraw>, left: f32, right: f32, yc: f32, side: f32) {
    for (tex, x, alt) in [(ARROW_LEFT, left, "<"), (ARROW_RIGHT, right, ">")] {
        let r = [x, yc - side / 2.0, x + side, yc + side / 2.0];
        match vm.texture_free(tex, r[0], r[1], r[2], r[3], WHITE) {
            Some(t) => {
                quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], BLACK));
                quads.push(t);
            }
            None => {
                text(vm, FONT, r[0] + 4.0, r[1], 1.0, alt, GREY, quads);
            }
        }
    }
}

/// `d` moved by (`dx`, `dy`) pixels.
fn shifted(mut d: HudDraw, dx: f32, dy: f32) -> HudDraw {
    match &mut d {
        HudDraw::Tile { x1, y1, x2, y2, .. } => {
            *x1 += dx;
            *x2 += dx;
            *y1 += dy;
            *y2 += dy;
        }
        HudDraw::Triangle { p, .. } => {
            for v in p.iter_mut() {
                v[0] += dx;
                v[1] += dy;
            }
        }
        HudDraw::Line { from, to, .. } => {
            from[0] += dx;
            from[1] += dy;
            to[0] += dx;
            to[1] += dy;
        }
        HudDraw::Portal { rect, .. } => {
            rect[0] += dx;
            rect[1] += dy;
        }
    }
    d
}

fn bordered(quads: &mut Vec<HudDraw>, r: [f32; 4], c: [u8; 4]) {
    quads.push(Vm::rectangle_free(r[0] - 2.0, r[1] - 2.0, r[2] + 2.0, r[3] + 2.0, BLACK));
    quads.push(Vm::rectangle_free(r[0], r[1], r[2], r[3], c));
}

#[allow(clippy::too_many_arguments)]
fn text(vm: &mut Vm, font: &str, x: f32, y: f32, s: f32, t: &str, color: [u8; 4], quads: &mut Vec<HudDraw>) -> (f32, f32) {
    let (mut cases, mut tw, mut th) = vm.free_text(font, 0.0, 0.0, t, color);
    if cases.is_empty() && font != FONT && !t.is_empty() {
        (cases, tw, th) = vm.free_text(FONT, 0.0, 0.0, t, color);
    }
    quads.extend(cases.into_iter().map(|d| match d {
        HudDraw::Tile { x1, y1, x2, y2, uv, material, color, point, enlarged, glyph } => HudDraw::Tile { x1: x + x1 * s, y1: y + y1 * s, x2: x + x2 * s, y2: y + y2 * s, uv, material, color, point: point && s >= 1.0, enlarged: enlarged || s > 1.0, glyph },
        other => other,
    }));
    (tw * s, th * s)
}

/// `t` cut with "..." to fit in `width` pixels.
fn fit(vm: &mut Vm, font: &str, s: f32, t: &str, width: f32) -> String {
    let width_of = |vm: &mut Vm, t: &str| text(vm, font, 0.0, 0.0, s, t, WHITE, &mut Vec::new()).0;
    if width_of(vm, t) <= width {
        return t.to_string();
    }
    let mut cut: String = t.to_string();
    while !cut.is_empty() {
        cut.pop();
        let c = format!("{}...", cut.trim_end());
        if width_of(vm, &c) <= width {
            return c;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(a: Action) -> String {
        a.english().to_string()
    }

    fn page() -> (GamepadPage, GamepadConfig) {
        let mut p = GamepadPage::default();
        p.open(0, 0);
        (p, GamepadConfig::default())
    }

    /// Remapping with the gamepad: A on Jump's button (A) shows the "?", then the X button: jump
    /// and reload are exchanged; Start and B cancel; Left / Right in the "?" choose the action
    /// (nothing saved before A confirms); waiting long changes nothing.
    #[test]
    fn remap_by_press_exchanges() {
        let (mut p, mut c) = page();
        p.focus_on(Item::Button(Button::A));
        p.key(Key::Enter, &mut c, "int", &labels);
        assert_eq!(p.waiting(), Some(Button::A));
        // A still held from the key: no press yet.
        let (taken, ch) = p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(taken && !ch.settings);
        let (_, ch) = p.pad(Button::X.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(ch.settings);
        assert_eq!((c.players[0].action(Button::A), c.players[0].action(Button::X)), (Action::Reload, Action::Jump));
        assert_eq!(p.waiting(), None);
        assert_eq!(p.focused(), Item::Button(Button::X));
        // Start cancels, and so does B.
        for cancel in [Button::Start, Button::B] {
            p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
            p.key(Key::Enter, &mut c, "int", &labels);
            let before = c;
            p.pad(cancel.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            assert_eq!((p.waiting(), c), (None, before), "{cancel:?}");
            assert!(p.open);
        }
        // No time limit: the "?" stays until confirmed or cancelled.
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.key(Key::Enter, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.0), 60.0, &mut c, "int", &labels);
        assert_eq!(p.waiting(), Some(Button::X));
        // Right twice chooses the action two further on, shown but not saved; B cancels it.
        let mut expected = c.players[0];
        expected.cycle(Button::X, 1);
        expected.cycle(Button::X, 1);
        for _ in 0..2 {
            p.pad(Button::Right.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        }
        assert_eq!(p.edit_choice(), Some(expected.action(Button::X)));
        assert_ne!(c.players[0], expected, "nothing saved before the confirm");
        p.pad(Button::B.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert_ne!(c.players[0], expected);
        // Again, confirmed by A this time.
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.key(Key::Enter, &mut c, "int", &labels);
        p.key(Key::Right, &mut c, "int", &labels);
        p.key(Key::Right, &mut c, "int", &labels);
        let (_, ch) = p.pad(Button::A.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(ch.settings && p.waiting().is_none());
        assert_eq!(c.players[0], expected);
        assert_eq!(p.focused(), Item::Button(Button::X), "Left / Right in the \"?\" never moved the focus");
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        // Defaults bring everything back.
        p.focus_on(Item::Defaults);
        let ch = p.key(Key::Enter, &mut c, "int", &labels);
        assert!(ch.settings && ch.aim_assist);
        assert_eq!(c, GamepadConfig::default());
    }

    /// The focus tick (MnCurseur, once per frame drawn): the page opening ticks nothing; a move
    /// of the focus by the gamepad, the keys or the mouse ticks once; a frame where the focus
    /// stays ticks nothing; the advanced page ticks as it moves too, its values changed by Left /
    /// Right tick nothing. Negative control: a tick on every frame, or none, fails.
    #[test]
    fn focus_ticks_once_per_move() {
        let (mut p, mut c) = page();
        assert!(!p.focus_tick(), "the page opening");
        assert!(!p.focus_tick(), "nothing moved");
        // The gamepad.
        p.pad(Button::Down.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(p.focus_tick(), "the d-pad moved the focus");
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(!p.focus_tick(), "released: no second tick");
        // The keys.
        p.key(Key::Down, &mut c, "int", &labels);
        assert!(p.focus_tick(), "a key moved the focus");
        assert!(!p.focus_tick());
        // The mouse (draw_local's focus_on under the cursor), onto another box then the same.
        p.focus_on(Item::Defaults);
        assert!(p.focus_tick(), "the mouse onto another box");
        p.focus_on(Item::Defaults);
        assert!(!p.focus_tick(), "the mouse inside the same box");
        // The advanced page: entering it moves the focus to its first row.
        p.focus_on(Item::Advanced);
        assert!(p.focus_tick());
        p.key(Key::Enter, &mut c, "int", &labels);
        assert!(p.advanced);
        assert!(p.focus_tick(), "the advanced page's first row");
        p.key(Key::Right, &mut c, "int", &labels);
        assert!(!p.focus_tick(), "a value changed, the focus stays");
        p.key(Key::Down, &mut c, "int", &labels);
        assert!(p.focus_tick(), "down the advanced page");
    }

    /// Navigation with the gamepad: d-pad / stick move the focus, A enters the advanced page,
    /// left / right change its values, B goes back to the buttons page then closes the page.
    #[test]
    fn pad_navigation() {
        let (mut p, mut c) = page();
        let n = main_items().len();
        for _ in 0..n {
            if in_bar(p.focused()) {
                break;
            }
            p.pad(Button::Down.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        }
        // Down goes down the column to the bottom row.
        assert!(in_bar(p.focused()), "{:?}", p.focused());
        let row = p.focused();
        p.pad(Button::Right.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(in_bar(p.focused()) && p.focused() != row, "Right along the bottom row");
        assert_eq!(c, GamepadConfig::default());
        p.focus_on(Item::Advanced);
        p.pad(Button::A.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(p.advanced);
        assert_eq!(p.focused(), Item::Vibration);
        p.pad(0, (0.6, 0.0), 0.016, &mut c, "int", &labels);
        assert!(!c.players[0].vibration);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.7), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.7), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(0, (0.0, 0.7), 0.016, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::LookSpeed);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(Button::Right.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert_eq!(c.players[0].look_speed, 4);
        p.pad(Button::B.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(!p.advanced && p.open);
        p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        p.pad(Button::B.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(!p.open);
        // Still held after closing: the game must not see it.
        let (taken, _) = p.pad(Button::B.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(taken);
        let (taken, _) = p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
        assert!(!taken);
    }

    register_viewer_layer!("gamepad_page_tests");

    /// The A press that chose the Gamepad entry, still held when the page opens, is not a
    /// press on the page (it would start a remap of the first line, and the next direction
    /// would exchange two buttons), even when the opener was given no held buttons. Negative
    /// control: a page that forgot its last frame starts the remap.
    #[test]
    fn opening_press_is_not_a_page_press() {
        for forget in [false, true] {
            let mut p = GamepadPage::default();
            let mut c = GamepadConfig::default();
            // Closed, the page reads the gamepad: A pressed in the menu.
            p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
            p.pad(Button::A.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            if forget {
                p.pad_before = 0;
            }
            p.open(0, 0);
            for _ in 0..10 {
                p.pad(Button::A.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            }
            p.pad(0, (0.0, 0.0), 0.016, &mut c, "int", &labels);
            let first = p.focused();
            p.pad(Button::Right.bit(), (0.0, 0.0), 0.016, &mut c, "int", &labels);
            assert_eq!(c, GamepadConfig::default());
            if forget {
                assert!(p.waiting().is_some(), "negative control: the opening press started an edit");
                assert_eq!(p.focused(), first);
            } else {
                assert_eq!(p.waiting(), None);
                // Right only moved the focus.
                assert_ne!(p.focused(), first);
            }
        }
    }

    /// Problems of a navigation `step` (from box, direction → box) over `boxes`: a move to a box
    /// that is not on that side of the screen, a box out of reach, a bottom row not walked with
    /// Left / Right. `across(i)`: Left / Right move from box `i` (otherwise they change a value).
    fn nav_problems(list: bool, boxes: &[(Item, Rect)], step: &dyn Fn(usize, Dir) -> Option<usize>, across: &dyn Fn(usize) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        let n = boxes.len();
        let mut edges = vec![Vec::new(); n];
        for from in 0..n {
            for d in crate::gamepad_layout::DIRS {
                if matches!(d, Dir::Left | Dir::Right) && !across(from) {
                    continue;
                }
                let vertical = matches!(d, Dir::Up | Dir::Down);
                // Left / Right from the bottom row stay on it; elsewhere they reach any box.
                let reachable = |j: usize| j != from && (vertical || if in_bar(boxes[from].0) { in_bar(boxes[j].0) } else { across(j) });
                let Some(to) = step(from, d) else {
                    // At an edge the focus wraps: it stays only where nothing else can be reached.
                    if (0..n).any(reachable) {
                        out.push(format!("{:?} {d:?} goes nowhere", boxes[from].0));
                    }
                    continue;
                };
                // That way on the screen: Left / Right, a box entirely on that side.
                let side = |j: usize, d: Dir| if vertical { towards(boxes[from].1, boxes[j].1, d).is_some() } else { crate::gamepad_layout::on_side(boxes[from].1, boxes[j].1, d) };
                let that_way = |j: usize| side(j, d);
                let wrapped_ok = !(0..n).any(|j| reachable(j) && that_way(j)) && side(to, crate::gamepad_layout::opposite(d));
                if to == from || (!that_way(to) && !wrapped_ok) {
                    out.push(format!("{:?} {d:?} -> {:?} is not that way: {:?} -> {:?}", boxes[from].0, boxes[to].0, boxes[from].1, boxes[to].1));
                }
                // Left / Right go to a box of the same row on that side whenever there is one (the
                // box drawn to the right, not one below that sticks out a little further).
                let beside = |j: usize| crate::gamepad_layout::beside(boxes[from].1, boxes[j].1, d);
                if !vertical && !beside(to) && (0..n).any(|j| reachable(j) && beside(j)) {
                    out.push(format!("{:?} {d:?} -> {:?} leaves its row", boxes[from].0, boxes[to].0));
                }
                // In the list, Up / Down from a button's line never go to a line of another
                // column while a box lies straight that way (Down at the foot of a short column
                // goes to the bottom row below it).
                let in_line = |j: usize| {
                    let (a, b) = (boxes[from].1, boxes[j].1);
                    a[0] < b[2] && b[0] < a[2] && towards(a, b, d).is_some()
                };
                if list && vertical && matches!((boxes[from].0, boxes[to].0), (Item::Button(_), Item::Button(_))) && !in_line(to) && (0..n).any(|j| j != from && in_line(j)) {
                    out.push(format!("{:?} {d:?} -> {:?} leaves its column", boxes[from].0, boxes[to].0));
                }
                edges[from].push(to);
            }
        }
        for start in 0..n {
            let mut seen = vec![false; n];
            let mut todo = vec![start];
            seen[start] = true;
            while let Some(i) = todo.pop() {
                for &j in &edges[i] {
                    if !seen[j] {
                        seen[j] = true;
                        todo.push(j);
                    }
                }
            }
            if let Some(j) = (0..n).find(|&j| !seen[j]) {
                out.push(format!("{:?} out of reach from {:?}", boxes[j].0, boxes[start].0));
                break;
            }
        }
        // The bottom row, left to right, walked with Right then back with Left.
        let mut bar: Vec<usize> = (0..n).filter(|&i| in_bar(boxes[i].0)).collect();
        bar.sort_by(|&a, &b| boxes[a].1[0].total_cmp(&boxes[b].1[0]));
        for w in bar.windows(2) {
            if step(w[0], Dir::Right) != Some(w[1]) || step(w[1], Dir::Left) != Some(w[0]) {
                out.push(format!("bottom row: {:?} and {:?} are not neighbours across", boxes[w[0]].0, boxes[w[1]].0));
            }
        }
        out
    }

    /// The page in an area: its boxes, and the stateless step of its navigation.
    fn nav_of(p: &GamepadPage) -> Vec<(Item, Rect)> {
        p.nav_boxes()
    }

    fn stateless_step(p: &GamepadPage, boxes: &[(Item, Rect)], from: usize, d: Dir) -> Option<usize> {
        let mut q = GamepadPage { open: true, player: p.player, locked: p.locked, advanced: p.advanced, layout: p.layout.clone(), ..Default::default() };
        q.focus_on(boxes[from].0);
        q.last_bar = None;
        q.last_above = None;
        let mut c = GamepadConfig::default();
        let before = c;
        q.key(match d { Dir::Up => Key::Up, Dir::Down => Key::Down, Dir::Left => Key::Left, Dir::Right => Key::Right }, &mut c, "int", &labels);
        assert_eq!(c, before, "a move changed a setting");
        let to = q.focused();
        let i = boxes.iter().position(|b| b.0 == to)?;
        (i != from).then_some(i)
    }

    /// Spatial navigation in every layout: for each area size, language, page (buttons or
    /// advanced) and kind of page (window's or a split screen player's), each direction from
    /// each box leads to a box lying that way on the screen (the real rectangles), every box can
    /// be reached from every other, and the bottom row is walked with Left / Right. Also with
    /// the focus memory, by a long walk of random keys.
    #[test]
    fn directions_follow_the_screen() {
        use crate::gamepad_layout::tests::{texts, Fonts, LANGS, SIZES};
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let mut checked = 0;
        let mut moves = 0;
        let mut seed = 0x1234_5678u32;
        for lang in LANGS {
            let t = texts(lang);
            for (w, h) in SIZES {
                let l = Layout::new(w, h, &t, &m);
                for (advanced, locked) in [(false, false), (false, true), (true, false), (true, true)] {
                    let p = GamepadPage { open: true, advanced, locked, layout: Some(l.clone()), ..Default::default() };
                    let boxes = nav_of(&p);
                    assert_eq!(boxes.len(), p.items().len(), "{lang} {w}x{h}: every item has a box");
                    let problems = nav_problems(l.list, &boxes, &|f, d| stateless_step(&p, &boxes, f, d), &|i| !advanced || in_bar(boxes[i].0));
                    assert!(problems.is_empty(), "{lang} {w}x{h} list {} advanced {advanced} locked {locked}: {problems:#?}", l.list);
                    // The focus memory: random keys, each move still goes that way.
                    let mut q = GamepadPage { open: true, advanced, locked, layout: Some(l.clone()), ..Default::default() };
                    let mut c = GamepadConfig::default();
                    for _ in 0..400 {
                        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                        let d = crate::gamepad_layout::DIRS[(seed >> 16) as usize % 4];
                        let from = q.focused();
                        if matches!(d, Dir::Left | Dir::Right) && advanced && !in_bar(from) {
                            continue;
                        }
                        let k = match d { Dir::Up => Key::Up, Dir::Down => Key::Down, Dir::Left => Key::Left, Dir::Right => Key::Right };
                        q.key(k, &mut c, lang, &labels);
                        let to = q.focused();
                        if to != from {
                            let r = |i: Item| boxes.iter().find(|b| b.0 == i).unwrap().1;
                            let wrapped = towards(r(from), r(to), crate::gamepad_layout::opposite(d)).is_some();
                            assert!(towards(r(from), r(to), d).is_some() || wrapped, "{lang} {w}x{h}: {from:?} {d:?} -> {to:?} with memory");
                            moves += 1;
                        }
                    }
                    assert_eq!(c, GamepadConfig::default(), "moving changed no setting");
                    checked += 1;
                }
            }
        }
        eprintln!("gamepad page navigation: {checked} pages checked, {moves} remembered moves");
        assert_eq!(checked, LANGS.len() * SIZES.len() * 4);
    }

    /// Negative controls: the former navigation (Up / Down through the list in its order,
    /// wrapping) and a navigation with Up and Down crossed are both caught.
    #[test]
    fn wrong_directions_are_seen() {
        let p = GamepadPage { open: true, layout: Some(stand_in_layout()), ..Default::default() };
        let boxes = nav_of(&p);
        let list = p.layout.as_ref().is_some_and(|l| l.list);
        let n = boxes.len();
        let former = |f: usize, d: Dir| match d {
            Dir::Up => Some((f + n - 1) % n),
            Dir::Down => Some((f + 1) % n),
            _ => None,
        };
        let problems = nav_problems(list, &boxes, &former, &|i| in_bar(boxes[i].0));
        assert!(problems.iter().any(|s| s.contains("not that way")), "{problems:#?}");
        assert!(problems.iter().any(|s| s.contains("bottom row")), "{problems:#?}");
        let crossed = |f: usize, d: Dir| {
            let d = match d {
                Dir::Up => Dir::Down,
                Dir::Down => Dir::Up,
                d => d,
            };
            stateless_step(&p, &boxes, f, d)
        };
        assert!(!nav_problems(list, &boxes, &crossed, &|i| in_bar(boxes[i].0)).is_empty());
        // And the real one is clean.
        assert!(nav_problems(list, &boxes, &|f, d| stateless_step(&p, &boxes, f, d), &|i| in_bar(boxes[i].0)).is_empty());
    }

    /// Negative control of the column rule: the first version of the navigation (an offset across
    /// counted a quarter) sends Down at the foot of the list's short last column to a button of
    /// the column beside it, which the check sees.
    #[test]
    fn nearest_by_score_alone_is_seen() {
        // The lengths of the columns come from the game's fonts: without them (wide estimates)
        // no list has the short last column this control needs.
        if openxiii_package::env::var_os("OXIII_GAME").is_none() {
            eprintln!("OXIII_GAME not set: nearest_by_score_alone_is_seen skipped");
            return;
        }
        use crate::gamepad_layout::tests::{texts, Fonts, SIZES};
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let mut seen = Vec::new();
        // The help lines the page shows, and the former page's French one: they change the list's
        // scale and so the lengths of its columns.
        let helps = |lang: &str| [edit_help_text(lang, true).to_string(), help_text(lang, true).to_string(), "A : appuyer sur le nouveau bouton   Gauche / Droite : choisir l'action   B : retour".to_string()];
        for (lang, help, (w, h)) in crate::gamepad_layout::tests::LANGS.iter().flat_map(|l| helps(l).into_iter().flat_map(move |hl| SIZES.iter().map(move |s| (*l, hl.clone(), *s)))) {
            let mut t = texts(lang);
            t.help = help;
            let l = Layout::new(w, h, &t, &m);
            let p = GamepadPage { open: true, locked: true, layout: Some(l.clone()), ..Default::default() };
            let boxes = nav_of(&p);
            let rects: Vec<Rect> = boxes.iter().map(|b| b.1).collect();
            let by_score = |f: usize, d: Dir| {
                let mut best: Option<(f32, usize)> = None;
                for (i, &c) in rects.iter().enumerate() {
                    let ok = i != f && (matches!(d, Dir::Up | Dir::Down) || in_bar(boxes[i].0));
                    if let Some((along, gap, off)) = towards(rects[f], c, d).filter(|_| ok) {
                        let s = along + 4.0 * gap + 0.25 * off;
                        if best.is_none_or(|(b, _)| s < b) {
                            best = Some((s, i));
                        }
                    }
                }
                best.map(|b| b.1)
            };
            let problems = nav_problems(l.list, &boxes, &by_score, &|i| in_bar(boxes[i].0));
            if problems.iter().any(|s| s.contains("leaves its column")) {
                seen.push((w, h, l.list));
            }
        }
        assert!(seen.iter().any(|s| s.2), "the former rule leaves the column in a list: {seen:?}");
    }

    /// On the bottom row, Left / Right go from box to box and Up / Down leave
    /// it; Down comes back to the bottom row's box last used, Up to the button left from.
    #[test]
    fn bottom_row_is_walked_across() {
        let (mut p, mut c) = page();
        p.layout = Some(stand_in_layout());
        p.focus_on(Item::Advanced);
        p.key(Key::Right, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Defaults);
        p.key(Key::Right, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Back);
        p.key(Key::Right, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Advanced, "Right at the right end wraps to the left end");
        p.key(Key::Left, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Back, "and Left at the left end to the right end");
        p.key(Key::Left, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Defaults);
        // Up leaves the row, Down comes back to Defaults, Up again to the same button.
        p.key(Key::Up, &mut c, "int", &labels);
        let above = p.focused();
        assert!(matches!(above, Item::Button(_)), "{above:?}");
        p.key(Key::Down, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Defaults);
        p.key(Key::Up, &mut c, "int", &labels);
        assert_eq!(p.focused(), above);
        // From the first column down to the bottom row, then Right to Back: Up goes to the
        // right column (above Back), not back to the left column.
        p.focus_on(Item::Button(Button::LeftBumper));
        p.key(Key::Down, &mut c, "int", &labels);
        assert!(in_bar(p.focused()));
        p.focus_on(Item::Defaults);
        p.key(Key::Right, &mut c, "int", &labels);
        assert_eq!(p.focused(), Item::Back);
        p.key(Key::Up, &mut c, "int", &labels);
        let boxes = p.nav_boxes();
        let r = |i: Item| boxes.iter().find(|b| b.0 == i).unwrap().1;
        assert_ne!(p.focused(), Item::Button(Button::LeftBumper));
        assert!(towards(r(Item::Back), r(p.focused()), Dir::Up).is_some());
        assert!(crate::gamepad_layout::overlap([r(Item::Back)[0], 0.0, r(Item::Back)[2], 1e6], [r(p.focused())[0], 0.0, r(p.focused())[2], 1e6]), "{:?} above Back", p.focused());
        assert_eq!(c, GamepadConfig::default());
        // Left / Right on a button move the focus to the box beside it on the screen, and never
        // change its action.
        p.focus_on(Item::Button(Button::X));
        p.key(Key::Right, &mut c, "int", &labels);
        assert_ne!(p.focused(), Item::Button(Button::X));
        assert_eq!(c, GamepadConfig::default());
    }

    /// Negative control of the row rule: Left / Right by the nearest-box score alone (the rule of
    /// Up / Down) leave the row on the picture, e.g. from the top left label to the small label
    /// under it, which the check sees.
    #[test]
    fn nearest_box_across_is_seen() {
        use crate::gamepad_layout::tests::{texts, Fonts, LANGS, SIZES};
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let mut seen = 0;
        for lang in LANGS {
            let t = texts(lang);
            for (w, h) in SIZES {
                let l = Layout::new(w, h, &t, &m);
                let p = GamepadPage { open: true, layout: Some(l.clone()), ..Default::default() };
                let boxes = nav_of(&p);
                let rects: Vec<Rect> = boxes.iter().map(|b| b.1).collect();
                let plain = |f: usize, d: Dir| match d {
                    Dir::Left | Dir::Right if !in_bar(boxes[f].0) => neighbour(&rects, f, d, &|_| true).or_else(|| wrap(&rects, f, d, &|_| true)),
                    _ => stateless_step(&p, &boxes, f, d),
                };
                if nav_problems(l.list, &boxes, &plain, &|_| true).iter().any(|s| s.contains("leaves its row")) {
                    seen += 1;
                }
            }
        }
        assert!(seen > 0, "the nearest-box rule never leaves a row");
    }

    /// Wrapping at every edge of the buttons page, as in the weapon menu: from the box farthest
    /// in a direction (the plain nearest-box rule finds nothing beyond it) that direction goes to
    /// a box at the other end.
    #[test]
    fn edges_wrap_around() {
        let p = GamepadPage { open: true, layout: Some(stand_in_layout()), ..Default::default() };
        let boxes = nav_of(&p);
        let rects: Vec<Rect> = boxes.iter().map(|b| b.1).collect();
        let mut wraps = 0;
        for from in 0..boxes.len() {
            for d in crate::gamepad_layout::DIRS {
                if neighbour(&rects, from, d, &|_| true).is_some() {
                    continue;
                }
                let to = stateless_step(&p, &boxes, from, d).unwrap_or_else(|| panic!("{:?} {d:?} stays", boxes[from].0));
                assert!(towards(rects[from], rects[to], crate::gamepad_layout::opposite(d)).is_some(), "{:?} {d:?} -> {:?}", boxes[from].0, boxes[to].0);
                wraps += 1;
            }
        }
        assert!(wraps >= 4, "every side has an edge: {wraps}");
        // Top to bottom and bottom to top.
        let top = (0..rects.len()).min_by(|&a, &b| rects[a][1].total_cmp(&rects[b][1])).unwrap();
        let to = stateless_step(&p, &boxes, top, Dir::Up).unwrap();
        assert!(rects[to][1] > rects[top][3], "Up from the top goes to the bottom: {:?}", boxes[to].0);
        let back = boxes.iter().position(|b| b.0 == Item::Back).unwrap();
        let to = stateless_step(&p, &boxes, back, Dir::Down).unwrap();
        assert!(rects[to][3] < rects[back][1], "Down from the bottom goes to the top: {:?}", boxes[to].0);
    }

    /// Vibration switched on asks for a short rumble of that player's gamepad, switched off
    /// asks for none; the rumble raises only that player's motors, while it lasts, and never
    /// those of a player who turned vibration off.
    #[test]
    fn vibration_on_rumbles_once() {
        let (mut p, mut c) = page();
        p.advanced = true;
        p.focus_on(Item::Vibration);
        let off = p.key(Key::Enter, &mut c, "int", &labels);
        assert_eq!((c.players[0].vibration, off.rumble), (false, None));
        let on = p.key(Key::Right, &mut c, "int", &labels);
        assert_eq!((c.players[0].vibration, on.rumble), (true, Some(0)));
        let test = Some((0, RUMBLE_TEST_SECONDS));
        assert_eq!(rumble_force([0.0; 2], true, 0, test), RUMBLE_TEST_FORCE);
        assert_eq!(rumble_force([0.9, 0.1], true, 0, test), [0.9, RUMBLE_TEST_FORCE[1]]);
        assert_eq!(rumble_force([0.2; 2], true, 1, test), [0.2; 2], "another player's gamepad");
        assert_eq!(rumble_force([0.2; 2], true, 0, Some((0, 0.0))), [0.2; 2], "over");
        assert_eq!(rumble_force([0.2; 2], false, 0, test), [0.0; 2], "vibration off");
    }

    /// Players: each has its own settings.
    #[test]
    fn players_are_separate() {
        let (mut p, mut c) = page();
        p.focus_on(Item::Player);
        p.key(Key::Enter, &mut c, "int", &labels);
        assert_eq!(p.player, 1);
        p.focus_on(Item::Button(Button::X));
        p.key(Key::Enter, &mut c, "int", &labels);
        p.key(Key::Right, &mut c, "int", &labels);
        p.key(Key::Enter, &mut c, "int", &labels);
        assert_eq!(c.players[0], PlayerPad::default());
        assert_ne!(c.players[1], PlayerPad::default());
        p.focus_on(Item::Player);
        p.key(Key::Enter, &mut c, "int", &labels);
        p.key(Key::Enter, &mut c, "int", &labels);
        assert_eq!(p.player, 3 % PLAYERS);
    }
}

impl super::App {
    /// Opens the gamepad settings page on player `player`'s settings (0 = player 1), over the
    /// game's menu page now open (main menu or in-game menu). The entry of the options menu calls
    /// this; nothing happens without an open menu page.
    pub(crate) fn open_gamepad_settings(&mut self, player: usize) {
        let held = self.pads_word();
        // Player 1's aim assist lives in User.ini (aim_device.rs).
        if let Some(g) = &mut self.game {
            if !g.vm.gui_page_active() {
                return;
            }
            self.gamepad_config.players[0].aim_assist = g.vm.gamepad_aim_assist();
        }
        // One local player (solo, network games): their own settings only, the player cannot be
        // changed (split screen gives each player their own page, `open_gamepad_settings_slot`).
        self.gamepad_page.open_in(0, player, held);
    }

    /// Opens the page of the player of view `slot` (the Gamepad entry of that player's own menu):
    /// in split screen, in their view with their settings only, driven by their gamepad;
    /// otherwise as `open_gamepad_settings`.
    pub(crate) fn open_gamepad_settings_slot(&mut self, slot: usize) {
        let Some(g) = &mut self.game else { return };
        if !g.vm.split_menus() {
            return self.open_gamepad_settings(0);
        }
        if slot >= PLAYERS || !g.vm.gui_page_active_slot(slot) {
            return;
        }
        self.gamepad_config.players[0].aim_assist = g.vm.gamepad_aim_assist();
        let held = if slot == 0 { self.first_pads_word() } else { self.game.as_ref().and_then(|g| g.vm.slot_pad_state(slot)).map_or(0, |s| openxiii_vm::gamepad_config::state_word(&s).0) };
        let player = self.game.as_ref().map_or(0, |g| g.vm.player_of_slot(slot));
        match slot {
            0 => self.gamepad_page.open_in(0, player, held),
            n => self.other_gamepad_pages[n - 1].open_in(n, player, held),
        }
    }

    /// Gamepads of the first player: all of them, except in split screen those of the other
    /// players on this machine (the VM's split_versus.rs attribution).
    pub(crate) fn first_player_pads(&self) -> Option<([f32; 6], u32)> {
        let mine = self.game.as_ref().and_then(|g| g.vm.first_player_pads());
        self.pads.as_ref().and_then(|p| p.merged_where(|i| mine.as_ref().is_none_or(|m| m.contains(&i))))
    }

    fn first_pads_word(&self) -> u16 {
        self.first_player_pads().map_or(0, |(a, b)| openxiii_vm::gamepad_config::word(b, a[4], a[5]))
    }

    /// One frame of the pages of the players after the first (split screen), each with the
    /// state of its own player's gamepad (`states`, by gamepad handle). Returns the handles of
    /// the gamepads a page has: the game sees them at rest.
    pub(crate) fn other_gamepad_pages_pad(&mut self, states: &[openxiii_vm::pad::PadState], dt: f32) -> Vec<usize> {
        let mut taken = Vec::new();
        for slot in 1..PLAYERS {
            let Some(handle) = self.game.as_ref().and_then(|g| g.vm.slot_pad_handle(slot)) else { continue };
            let (word, stick) = states.get(handle).map_or((0, (0.0, 0.0)), openxiii_vm::gamepad_config::state_word);
            let lang = self.gamepad_lang();
            let labels = if self.other_gamepad_pages[slot - 1].open { Some(self.gamepad_labels()) } else { None };
            let labels: &dyn Fn(Action) -> String = match &labels {
                Some(l) => l,
                None => &|a: Action| a.english().to_string(),
            };
            let (t, ch) = self.other_gamepad_pages[slot - 1].pad(word, stick, dt, &mut self.gamepad_config, &lang, labels);
            self.gamepad_page_apply(ch, slot);
            if t {
                taken.push(handle);
            }
        }
        taken
    }

    /// Button word of the combined gamepad now (0 without one).
    fn pads_word(&self) -> u16 {
        if let Some((a, b)) = self.pads.as_ref().and_then(|p| p.merged()) {
            return openxiii_vm::gamepad_config::word(b, a[4], a[5]);
        }
        0
    }

    fn gamepad_labels(&mut self) -> impl Fn(Action) -> String {
        let mut names: Vec<(Action, String)> = Vec::new();
        if let Some(g) = &mut self.game {
            for a in openxiii_vm::gamepad_config::ACTIONS {
                names.push((a, action_label(&mut g.vm, a)));
            }
        }
        move |a: Action| names.iter().find(|n| n.0 == a).map_or_else(|| a.english().to_string(), |n| n.1.clone())
    }

    fn gamepad_lang(&self) -> String {
        self.game.as_ref().map_or_else(|| "int".to_string(), |g| g.vm.game_language())
    }

    /// A change of the page: saved at once in the settings file (player folder), player 1's aim
    /// assist in User.ini.
    pub(crate) fn gamepad_page_apply(&mut self, ch: Changed, slot: usize) {
        if let Some(player) = ch.rumble {
            self.rumble_test = Some((player, RUMBLE_TEST_SECONDS));
        }
        if ch.aim_assist {
            if let Some(g) = &mut self.game {
                let on = self.gamepad_config.players[0].aim_assist;
                g.vm.set_gamepad_aim_assist(on, g.controller);
            }
        }
        if ch.settings {
            let error = openxiii_vm::config::folder_player().and_then(|dir| self.gamepad_config.save(&dir).err()).map(|e| {
                let lang = self.gamepad_lang();
                fill(txt(&lang, Txt::CannotWrite), &[openxiii_vm::gamepad_config::FILE_NAME, &e.to_string()])
            });
            match slot {
                0 => self.gamepad_page.error = error,
                n => self.other_gamepad_pages[(n - 1).min(PLAYERS - 2)].error = error,
            }
        }
    }

    /// A key of the page (keyboard or test script).
    pub(crate) fn gamepad_page_key(&mut self, k: Key) {
        let labels = self.gamepad_labels();
        let lang = self.gamepad_lang();
        let ch = self.gamepad_page.key(k, &mut self.gamepad_config, &lang, &labels);
        self.gamepad_page_apply(ch, 0);
    }

    /// Keyboard key while the page is open: true when taken (presses only; releases still
    /// reach the game).
    pub(crate) fn gamepad_page_keyboard(&mut self, code: winit::keyboard::KeyCode, pressed: bool) -> bool {
        use winit::keyboard::KeyCode;
        if !self.gamepad_page.open {
            return false;
        }
        if !pressed {
            return false;
        }
        let k = match code {
            KeyCode::ArrowUp => Some(Key::Up),
            KeyCode::ArrowDown => Some(Key::Down),
            KeyCode::ArrowLeft => Some(Key::Left),
            KeyCode::ArrowRight => Some(Key::Right),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Some(Key::Enter),
            KeyCode::Escape | KeyCode::Backspace => Some(Key::Back),
            _ => None,
        };
        if let Some(k) = k {
            self.gamepad_page_key(k);
        }
        true
    }

    /// Left mouse button: true when the page takes it.
    pub(crate) fn gamepad_page_click(&mut self, pressed: bool) -> bool {
        let labels = self.gamepad_labels();
        let lang = self.gamepad_lang();
        let (taken, ch) = self.gamepad_page.click(pressed, &mut self.gamepad_config, &lang, &labels);
        self.gamepad_page_apply(ch, 0);
        taken
    }

    /// Wheel: true when the page takes it.
    pub(crate) fn gamepad_page_wheel(&mut self, notches: f32) -> bool {
        if !self.gamepad_page.open {
            return false;
        }
        let labels = self.gamepad_labels();
        let lang = self.gamepad_lang();
        let ch = self.gamepad_page.wheel(notches, &mut self.gamepad_config, &lang, &labels);
        self.gamepad_page_apply(ch, 0);
        true
    }

    /// One frame of the combined gamepad for the page: true when the page has it (the game and
    /// the other pages see it at rest).
    pub(crate) fn gamepad_page_pad(&mut self, pad: Option<([f32; 6], u32)>, dt: f32) -> bool {
        let (axes, b) = pad.unwrap_or(([0.0; 6], 0));
        let word = openxiii_vm::gamepad_config::word(b, axes[4], axes[5]);
        let lang = self.gamepad_lang();
        let labels = if self.gamepad_page.open { Some(self.gamepad_labels()) } else { None };
        let labels: &dyn Fn(Action) -> String = match &labels {
            Some(l) => l,
            None => &|a: Action| a.english().to_string(),
        };
        let (taken, ch) = self.gamepad_page.pad(word, (axes[0], axes[1]), dt, &mut self.gamepad_config, &lang, labels);
        self.gamepad_page_apply(ch, 0);
        taken
    }

    /// The gamepad settings for the game this frame: player 1's look stick (look speed,
    /// inversion), and every player's settings for local multiplayer.
    pub(crate) fn gamepad_look_tune(&mut self) {
        let tune = self.gamepad_config.players[0].look_tune();
        // The game saved the aim assist of a player after the first (the VM's console.rs
        // SaveConfig): it goes to that player's settings.
        let saved = self.game.as_mut().map(|g| g.vm.take_other_aim_saved()).unwrap_or_default();
        let mut changed = false;
        for (rank, on) in saved {
            if let Some(p) = self.gamepad_config.players.get_mut(rank) {
                changed |= std::mem::replace(&mut p.aim_assist, on) != on;
            }
        }
        if changed {
            self.gamepad_page_apply(Changed { settings: true, ..Default::default() }, 0);
        }
        if let Some(g) = &mut self.game {
            g.vm.look_stick_tune = tune;
            g.vm.gamepad_players = self.gamepad_config.players;
        }
    }
}

register_viewer_layer!("gamepad_page");
