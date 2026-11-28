//! Improved build only: the page of the co-op character screen (character_screen.rs) and the
//! screen's place in the co-op flow.
//!
//! The screen is the last step before a co-op game starts. The game's own travel to the game
//! (New game after its video, Continue, Load, Select level, or joining a host) is held
//! ([`Held`]) and the screen's map is loaded instead; when every player on this computer has
//! confirmed, the held travel goes as it was asked (a checkpoint's music state given back), with
//! the choices kept in the co-op session ([`crate::local_play::LocalPlay::characters`]). Going
//! back from the screen loads the main menu again and opens the Co-op page on its second step.
//!
//! Each player has their own page, drawn over their own view (one half of the screen each in
//! split screen): the roster on the left (moving in it shows the character at once), the
//! character's name, a Variant button when they have more than one, and Confirm. The first
//! player uses the keyboard and mouse, the second their gamepad (coop.rs gives its d-pad, A and
//! B to their own menu as arrows, Enter and Escape).

use crate::added_text::{fill, text, Text};
use crate::character_screen::{Choice, PanelPicture, ROSTER, SCREEN_OPTION, SPLITTER};
use crate::local_play::{CoopMode, CoopSession, CoopStart};
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// What the mouse can click on a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    /// The arrows of the chosen line: the previous and the next look.
    VariantPrev,
    Variant,
    Confirm,
}

/// Half the height of a character of the roster (Unreal units), for the camera's field of view.
pub const CHARACTER_HALF_HEIGHT: f32 = 90.0;

/// The game's travel held while the screen is shown: a travel to a map with the checkpoint music
/// state it carried (UAudioSaver, used by the next map's load: given back just before the travel
/// goes), or a client's travel to a host (joining over the network: the engine connects first).
pub enum Held {
    Request { request: crate::travel::TravelRequest, audio: crate::save::AudioSaver },
    Client { url: String, travel_type: u8, items: bool },
}

/// File keeping each player's last choice between runs (set by the game window; None: not kept).
static CHOICE_FILE: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Where the players' last choices are kept between runs.
pub fn set_choice_file(path: Option<std::path::PathBuf>) {
    if let Ok(mut f) = CHOICE_FILE.lock() {
        *f = path;
    }
}

/// Each player's last choice ("Player1=Jones", "Player2=Killer.3"), XIII without one.
pub fn last_choices() -> [Choice; 2] {
    let path = CHOICE_FILE.lock().ok().and_then(|f| f.clone());
    let text = path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    read_choices(&text)
}

fn read_choices(text: &str) -> [Choice; 2] {
    let mut out = [Choice::default(); 2];
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let i = match k.trim().to_ascii_lowercase().as_str() {
            "player1" => 0,
            "player2" => 1,
            _ => continue,
        };
        if let Some(c) = Choice::from_code(v) {
            out[i] = c;
        }
    }
    out
}

/// The characters last chosen in co-op, when they were ever kept (None: never chosen).
pub fn remembered_choices() -> Option<[Choice; 2]> {
    let path = CHOICE_FILE.lock().ok().and_then(|f| f.clone())?;
    std::fs::read_to_string(path).ok().map(|t| read_choices(&t))
}

fn keep_choices(choices: [Choice; 2]) {
    let Some(path) = CHOICE_FILE.lock().ok().and_then(|f| f.clone()) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("[CoopCharacters]\r\nPlayer1={}\r\nPlayer2={}\r\n", choices[0].code(), choices[1].code()));
}

/// Input of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Back,
    /// Click on a row of the roster.
    Pick(usize),
    /// Click on the chosen line's arrows (previous, next variant).
    VariantPrev,
    Variant,
    /// Click on Confirm.
    Confirm,
}

/// What an input asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOutcome {
    None,
    /// Show this character (moved in the roster or changed variant).
    Show(Choice),
    /// The player confirmed, or took the confirmation back.
    Confirmed(bool),
    /// Leave the screen (the first player's Escape before confirming).
    Leave,
}

/// One player's page, without the VM: their choice, confirmation and the variant last shown of
/// each character (moving back to a character shows that variant again).
#[derive(Clone, Debug, PartialEq)]
pub struct PageState {
    pub choice: Choice,
    pub confirmed: bool,
    pub variants: Vec<usize>,
}

impl PageState {
    pub fn new(choice: Choice) -> PageState {
        let mut variants = crate::character_screen::default_variants();
        if let Some(v) = variants.get_mut(choice.character) {
            *v = choice.variant;
        }
        PageState { choice, confirmed: false, variants }
    }

    fn show(&mut self, character: usize) -> PageOutcome {
        let character = character % ROSTER.len();
        let variant = self.variants[character].min(ROSTER[character].variants.len() - 1);
        self.choice = Choice { character, variant };
        PageOutcome::Show(self.choice)
    }

    fn cycle_variant(&mut self, step: isize) -> PageOutcome {
        let n = ROSTER[self.choice.character].variants.len();
        if n < 2 {
            return PageOutcome::None;
        }
        // In the shown order (character_screen::variant_order), not the list's.
        let order = crate::character_screen::variant_order(self.choice.character);
        let at = order.iter().position(|&v| v == self.choice.variant).unwrap_or(0);
        let v = order[(at as isize + step).rem_euclid(n as isize) as usize];
        self.variants[self.choice.character] = v;
        self.choice.variant = v;
        PageOutcome::Show(self.choice)
    }

    /// One input. Back is the same for every player: a ready player takes their confirmation
    /// back first; a choosing player leaves the screen (both players back to the Co-op page).
    /// Enter (or A) while ready does nothing, so that a second press is not a surprise.
    pub fn key(&mut self, k: PageKey) -> PageOutcome {
        if self.confirmed {
            return match k {
                PageKey::Back | PageKey::Confirm => {
                    self.confirmed = false;
                    PageOutcome::Confirmed(false)
                }
                _ => PageOutcome::None,
            };
        }
        let n = ROSTER.len();
        match k {
            PageKey::Up => self.show(self.choice.character + n - 1),
            PageKey::Down => self.show(self.choice.character + 1),
            PageKey::Pick(i) if i < n => self.show(i),
            PageKey::Pick(_) => PageOutcome::None,
            PageKey::Left | PageKey::VariantPrev => self.cycle_variant(-1),
            PageKey::Right | PageKey::Variant => self.cycle_variant(1),
            PageKey::Enter | PageKey::Confirm => {
                self.confirmed = true;
                PageOutcome::Confirmed(true)
            }
            PageKey::Back => PageOutcome::Leave,
        }
    }
}

/// Name of a roster character in language `lang`.
pub fn character_name(lang: &str, character: usize) -> &'static str {
    let Some(c) = ROSTER.get(character) else { return "" };
    match c.id {
        "Mongoose" => text(lang, Text::NameMongoose),
        "Kim" => text(lang, Text::NameKim),
        "Admiral" => text(lang, Text::NameAdmiral),
        "FBI" => text(lang, Text::NameFbi),
        "Killer" => text(lang, Text::NameKiller),
        "Conspirator" => text(lang, Text::NameConspirator),
        "Policeman" => text(lang, Text::NamePoliceman),
        "Spads" => "SPADS",
        id => id,
    }
}

impl Vm {
    /// Size of `t` on the page: the canvas font's, times the page's text scale.
    fn page_text_size(&mut self, c: ObjId, t: &str) -> (f32, f32) {
        let k = self.improved.character_screen.text_scale;
        let (w, h) = self.select_text_size(c, t);
        (w * k, h * k)
    }

    /// `t` at (x, y) on the page in the canvas font, scaled by the page's text scale (as is at 1).
    fn page_text(&mut self, c: ObjId, x: f32, y: f32, t: &str, rgba: [u8; 4]) {
        let k = self.improved.character_screen.text_scale;
        if (k - 1.0).abs() < 0.01 {
            self.select_draw_text(c, x, y, t, rgba);
            return;
        }
        let Some(font) = crate::Vm::font_path(&self.propr(c, "Font")) else { return };
        let (tiles, _, _) = self.free_text(&font, 0.0, 0.0, t, rgba);
        for tile in tiles {
            if let crate::canvas::HudDraw::Tile { x1, y1, x2, y2, uv, material, color, .. } = tile {
                self.hud.push(crate::canvas::HudDraw::Tile { x1: x + x1 * k, y1: y + y1 * k, x2: x + x2 * k, y2: y + y2 * k, uv, material, color, point: k > 1.0 && crate::gui::enlarged_text_bicubic(self.engine.client.session_graphics), enlarged: k > 1.0, glyph: true });
            }
        }
    }
}

/// A key event of the page that is a mouse click converted by the menu: UGUIController's
/// NativeKeyEvent looks under the cursor and passes the click on as no key (0), release only.
pub fn converted_click(key: i32, state: i32) -> bool {
    key == 0 && state == 3
}

/// The area the page lays itself out in: the player's whole view (the canvas's SizeX, SizeY),
/// or the menus' 640 x 480 scaled by their ratios when the view's size is not known.
/// A key the character screen takes and drops before the menus see it: IK_RightMouse, which
/// UGUIController::NativeKeyEvent turns into Backspace (back). Escape, Backspace
/// and the gamepad's B still go back.
pub fn swallowed_on_screen(key: u8) -> bool {
    key == 0x02
}

pub fn page_area((size_x, size_y): (f32, f32), (rx, ry): (f32, f32)) -> (f32, f32) {
    if size_x >= 1.0 && size_y >= 1.0 {
        (size_x, size_y)
    } else {
        (640.0 * rx, 480.0 * ry)
    }
}

/// The label of roster row `character` showing `variant`: the person's name, or for a group of
/// different people (character_screen::PEOPLE_GROUPS) the one shown ("Gros flic", "Tueur 2").
pub fn row_label(lang: &str, character: usize, variant: usize) -> &'static str {
    if crate::character_screen::variants_are_people(character) {
        if let Some(v) = ROSTER[character].variants.get(variant) {
            return crate::added_text::variant_name(lang, v.name);
        }
    }
    character_name(lang, character)
}

/// The items of a hint line (separated by three spaces in every language); with `single`, the
/// second item (the variant keys) is left out, the character having no other variant.
pub fn hint_items(line: &str, single: bool) -> Vec<&str> {
    let items: Vec<&str> = line.split("   ").map(str::trim).filter(|t| !t.is_empty()).collect();
    if single && items.len() > 1 {
        return vec![items[0]];
    }
    items
}

/// Items laid out on lines no wider than `max` (an item wider than `max` is a line alone).
pub fn hint_lines(items: &[&str], widths: &[f32], sep: f32, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0.0;
    for (t, &tw) in items.iter().zip(widths) {
        if !current.is_empty() && current_w + sep + tw <= max {
            current.push_str("   ");
            current.push_str(t);
            current_w += sep + tw;
        } else {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            current.push_str(t);
            current_w = tw;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Where a page's list starts showing rows so that the chosen one is in view, near the middle.
pub fn list_start(chosen: usize, count: usize, visible: usize) -> usize {
    if visible >= count {
        return 0;
    }
    chosen.saturating_sub(visible / 2).min(count - visible)
}

impl Vm {
    /// Opens player `p`'s page on the GUI controller of their viewport.
    pub(crate) fn character_page_open(&mut self, p: usize, pc: ObjId) {
        let Some(viewport) = self.prop(pc, "Player").as_object() else { return };
        let root = match self.prop(viewport, "LocalInteractions") {
            Value::Array(list) => {
                list.iter().filter_map(Value::as_object).find(|&o| self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "GUIController"))
            }
            _ => None,
        };
        let (Some(root), Some(class)) = (root, self.class("XIIIWindowMainMenu")) else {
            self.log.push(format!("Log: Character screen: no menu for player {}", p + 1));
            return;
        };
        self.improved.character_screen.opening = Some(p);
        let opened = self.call_named(root, "OpenMenuWithClass", vec![Value::Class(Some(class))]).is_ok_and(|v| v.as_bool());
        self.improved.character_screen.opening = None;
        if !opened || self.improved.character_screen.pages.get(p).copied().flatten().is_none() {
            self.log.push(format!("Log: Character screen: page of player {} not opened", p + 1));
        }
    }

    /// The player whose page this is.
    pub(crate) fn character_page_of(&self, page: ObjId) -> Option<usize> {
        self.improved.character_screen.pages.iter().position(|&p| p == Some(page))
    }

    /// Hooks of the pages' XIIIWindow functions (menu_boxes_call).
    #[allow(clippy::type_complexity)]
    pub(crate) fn character_page_call(
        &mut self,
        f: usize,
        this: ObjId,
        name: &str,
        args: Vec<Option<Value>>,
    ) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        match name {
            "Created" if self.improved.character_screen.opening.is_some() && self.is_a(this, "XIIIWindowMainMenu") => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.keep_out_of_saved_menu_stack(this);
                    if let Some(p) = self.improved.character_screen.opening {
                        if let Some(slot) = self.improved.character_screen.pages.get_mut(p) {
                            *slot = Some(this);
                        }
                    }
                    // No page background, as the in-game menu (XIIIMenuInGame: Background=None): the
                    // view stays visible.
                    self.set_prop(this, "Background", Value::Object(None));
                    // The game's help bar: choose, back.
                    // The page draws its own hints for each player's device: no help bar of the
                    // menu under them (on this page only).
                    self.set_prop(this, "bShowSEL", Value::Bool(false));
                    self.set_prop(this, "bShowBCK", Value::Bool(false));
                    self.set_prop(this, "bDisplayBar", Value::Bool(false));
                }
                Ok(r)
            }
            _ => {
                let Some(p) = self.character_page_of(this) else { return Err(args) };
                match name {
                    // Not the page's own Paint (its white page would hide the view): only ours.
                    "Paint" => {
                        let canvas = args.first().cloned().flatten().and_then(|v| v.as_object());
                        if let Some(c) = canvas {
                            self.character_page_paint(p, this, c);
                        }
                        let outs = self.functions[f].params.iter().map(|_| None).collect();
                        Ok(Ok((Value::Void, outs)))
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
                        // The page's own key event first (the menu sounds).
                        let r = self.call_inner(f, this, args);
                        // F4, and the mouse's moves (IK_MouseX, IK_MouseY, IK_MouseW): as the page
                        // answers, so that the menu's cursor follows the mouse (taking them stopped
                        // the cursor at 0, 0 and every click missed).
                        if key == 0x73 || matches!(key, 0xe4..=0xe6) {
                            // The mouse moves: the first player's cursor shows.
                            if p == 0 && key != 0x73 {
                                self.improved.character_screen.mouse_in_use = true;
                            }
                            return Ok(r);
                        }
                        // Press, and repeat for the arrows (held down).
                        if state == 1 || (state == 2 && matches!(key, 0x25..=0x28)) {
                            self.character_page_key(p, key);
                        }
                        // A click while the cursor is over something else than the focused control:
                        // UGUIController::NativeKeyEvent looks under the cursor and
                        // passes the click on as no key (0), its release only. It is the click.
                        if converted_click(key, state) {
                            self.character_page_key(p, 0x01);
                        }
                        Ok(Ok((Value::Bool(true), outs)))
                    }
                    _ => Err(args),
                }
            }
        }
    }

    fn page_state(&self, p: usize) -> PageState {
        let s = &self.improved.character_screen;
        PageState {
            choice: s.shown.get(p).copied().unwrap_or_default(),
            confirmed: s.confirmed.get(p).copied().unwrap_or(false),
            variants: s.variants.get(p).cloned().unwrap_or_else(crate::character_screen::default_variants),
        }
    }

    fn character_page_key(&mut self, p: usize, key: i32) {
        // The first player's menu cursor shows while the mouse is in use (click, wheel), not
        // after a key or a gamepad press (character_screen_cursors).
        if p == 0 {
            self.improved.character_screen.mouse_in_use = matches!(key, 0x01 | 0xEC | 0xED);
        }
        let k = match key {
            0x26 | 0xEC => PageKey::Up,
            0x28 | 0xED => PageKey::Down,
            0x25 => PageKey::Left,
            0x27 => PageKey::Right,
            0x0D => PageKey::Enter,
            0x1B | 0x08 => PageKey::Back,
            // IK_LeftMouse: what is under the cursor (only the first player has a mouse).
            0x01 => match self.character_page_hit(p) {
                Some(Hit::Row(i)) => PageKey::Pick(i),
                Some(Hit::VariantPrev) => PageKey::VariantPrev,
                Some(Hit::Variant) => PageKey::Variant,
                Some(Hit::Confirm) => PageKey::Confirm,
                None => return,
            },
            _ => return,
        };
        let mut st = self.page_state(p);
        let out = st.key(k);
        if let Some(v) = self.improved.character_screen.variants.get_mut(p) {
            *v = st.variants;
        }
        match out {
            PageOutcome::None => {}
            PageOutcome::Show(choice) => {
                // Another character or variant: the game's focus tick, as a button's MouseEnter
                // plays it (this page is drawn here, not made of the game's buttons).
                self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
                let started = std::time::Instant::now();
                self.screen_show(p, choice);
            }
            PageOutcome::Confirmed(on) => self.character_page_confirm(p, on),
            PageOutcome::Leave => {
                self.log.push(format!("Log: Character screen: player {} goes back to the Co-op page", p + 1));
                // The game's back sound (MnAnnul), as its pages play it (nothing else does here).
                self.play_menu_sound(crate::menu_sounds::MenuSound::Back);
                self.improved.character_screen.back = true;
            }
        }
    }

    /// Player `p` confirms their choice or takes it back: their character plays its confirmation
    /// (or its pose again). Once everyone confirmed, the game starts when every confirmation
    /// animation has finished, after [`crate::character_screen::CONFIRM_START_DELAY`] at most
    /// (character_screen_frame); taking it back before stops that.
    pub(crate) fn character_page_confirm(&mut self, p: usize, on: bool) {
        if let Some(c) = self.improved.character_screen.confirmed.get_mut(p) {
            *c = on;
        }
        // The sounds of the game's buttons: validated (MnValid), or cancelled (MnAnnul).
        self.play_menu_sound(if on { crate::menu_sounds::MenuSound::Enter } else { crate::menu_sounds::MenuSound::Back });
        let code = self.improved.character_screen.shown.get(p).map(|c| c.code()).unwrap_or_default();
        self.log.push(format!("Log: Character screen: player {} {} {code}", p + 1, if on { "confirms" } else { "takes back" }));
        self.screen_confirm(p, on);
        // The arms of the confirmed character made in the background from now: the confirmation
        // animation (and the loading that follows) hides the time it takes.
        #[cfg(feature = "mods")]
        if on {
            if let Some(name) = self.improved.character_screen.shown.get(p).and_then(|c| c.variant_data()).and_then(|v| v.arms) {
                crate::coop_looks::prefetch_arms(self, name);
            }
        }
        if on && self.improved.character_screen.confirmed.iter().all(|&c| c) {
            let at = crate::character_screen::start_time(self.improved.character_screen.clock);
            self.improved.character_screen.finish_at = Some(at);
            self.log.push(format!("Log: Character screen: everyone confirmed, the game starts once the confirmations have finished, in {:.1} s at most", crate::character_screen::CONFIRM_START_DELAY));
        } else if !on {
            self.improved.character_screen.finish_at = None;
        }
    }

    /// The level the co-op character screen will start once every confirmation animation has
    /// finished: its held travel's URL, from the moment everyone confirmed (None before, after a
    /// confirmation was taken back, or for a network guest's held travel). The game window reads
    /// it to read that map's packages meanwhile (main.rs, preload).
    pub fn character_screen_starting(&self) -> Option<crate::travel::Url> {
        self.improved.character_screen.finish_at?;
        match self.improved.character_screen.held.as_ref()? {
            crate::character_page::Held::Request { request, .. } => Some(request.url.clone()),
            crate::character_page::Held::Client { .. } => None,
        }
    }

    /// What is under the first player's mouse on page `p`.
    fn character_page_hit(&self, p: usize) -> Option<Hit> {
        let root = self
            .improved.character_screen
            .pages
            .get(p)
            .copied()
            .flatten()
            .and_then(|page| self.propr(page, "Controller").as_object().or_else(|| self.propr(page, "myRoot").as_object()))?;
        let (x, y) = (self.propr(root, "MouseX").as_int() as f32, self.propr(root, "MouseY").as_int() as f32);
        self.improved.character_screen.hits.get(p)?.iter().find(|(r, _)| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]).map(|(_, h)| *h)
    }

    /// Everyone on this computer confirmed: the choices go into the co-op session (and are kept
    /// for the next time), their arms are made, and the held travel goes.
    pub(crate) fn character_screen_finish(&mut self) {
        let shown = self.improved.character_screen.shown.clone();
        let mut choices = last_choices();
        for (i, c) in shown.iter().enumerate().take(2) {
            choices[i] = *c;
        }
        self.coop_take_characters(choices, shown.len());
        self.log.push(format!("Log: Character screen: everyone ready ({})", shown.iter().map(|c| c.code()).collect::<Vec<_>>().join(", ")));
        self.improved.character_screen.go = true;
    }

    /// The co-op game's characters: kept for the next time, given to the co-op session, the
    /// first `players` players' arms made.
    fn coop_take_characters(&mut self, choices: [Choice; 2], players: usize) {
        keep_choices(choices);
        let codes = choices.map(|c| (c.character as u8, c.variant as u8));
        self.change_local_play(|lp| lp.characters = Some(codes));
        #[cfg(not(feature = "mods"))]
        let _ = players;
        #[cfg(feature = "mods")]
        for (i, c) in choices.iter().enumerate().take(players) {
            if let Some(name) = c.variant_data().and_then(|v| v.arms) {
                let started = std::time::Instant::now();
                match crate::coop_looks::ensure_arms(self, name) {
                    Ok(()) => self.log.push(format!(
                        "Log: Character screen: player {} arms {name} ready in {:.0} ms",
                        i + 1,
                        started.elapsed().as_secs_f64() * 1000.0
                    )),
                    Err(e) => self.log.push(format!("Log: Character screen: player {} arms {name}: {e}", i + 1)),
                }
            }
        }
    }

    /// Continue or Load from the Co-op page: the characters of the slot just read (coop_save.rs),
    /// otherwise the ones last chosen in co-op, are taken again and the travel goes without the
    /// screen. False when there are none (the screen is shown, as for a new game).
    fn coop_resume_characters(&mut self, r: &crate::travel::TravelRequest) -> bool {
        let slot = self.improved.character_screen.slot_characters.take();
        let lp = self.local_play();
        let resuming = lp.coop.is_some_and(|s| s.mode != CoopMode::LanJoin && matches!(s.start, Some(CoopStart::Continue | CoopStart::Load)));
        if !resuming || !self.character_screen_wanted(r) {
            return false;
        }
        let (choices, from) = match (slot.flatten(), remembered_choices()) {
            (Some(c), _) => (c, "the save"),
            (None, Some(c)) => (c, "the last co-op game"),
            (None, None) => return false,
        };
        // Both players' arms, as the screen makes them (the session may still count one player
        // on this screen at this point).
        self.coop_take_characters(choices, 2);
        self.log.push(format!(
            "Log: Character screen: skipped, {} keeps {}, {} from {from}",
            r.url.string(),
            choices[0].code(),
            choices[1].code()
        ));
        true
    }

    /// UGameEngine::Tick, the travel of this frame (if any): held for the screen when it is the
    /// co-op game's start, and the held one once the screen is done.
    pub(crate) fn character_screen_travel(&mut self, req: Option<crate::travel::TravelRequest>) -> Option<crate::travel::TravelRequest> {
        let s = &mut self.improved.character_screen;
        if s.on && matches!(s.held, Some(Held::Request { .. })) && std::mem::take(&mut s.go) {
            if let Some(Held::Request { request, audio }) = s.held.take() {
                self.engine.audio_saver = audio;
                self.character_pages_close();
                self.log.push(format!("Log: Character screen: on to {}", request.url.string()));
                return Some(request);
            }
        }
        if s.on && s.held.is_none() && std::mem::take(&mut s.go) {
            // Screen opened without a game to go to (testing): it stays.
            self.log.push("Log: Character screen: nothing to go to".to_string());
            return req;
        }
        if s.on && std::mem::take(&mut s.back) {
            s.held = None;
            self.character_pages_close();
            let mode = self.local_play().coop.map(|c| c.mode);
            self.improved.character_screen.reopen_coop = mode;
            return Some(self.browse_request_pub("MapMenu", crate::travel::TRAVEL_ABSOLUTE, Vec::new()));
        }
        let r = req?;
        if self.coop_resume_characters(&r) || !self.character_screen_wanted(&r) {
            return Some(r);
        }
        let map = crate::character_screen::places().map;
        self.log.push(format!("Log: Character screen: {} held, choosing the characters on {map}", r.url.string()));
        // The checkpoint's music state goes with the held travel (the screen's map must not take it).
        let audio = self.engine.audio_saver.clone();
        self.engine.audio_saver.flag = 0;
        self.improved.character_screen.held = Some(Held::Request { request: r, audio });
        Some(self.browse_request_pub(&format!("{map}?{SCREEN_OPTION}"), crate::travel::TRAVEL_ABSOLUTE, Vec::new()))
    }

    /// Leaving the screen: every player's page is closed (CloseAll, as
    /// UGameEngine::GUICleanup does for every viewport). The travel closes the first player's
    /// menus only: a page left open in another player's view kept their gamepad in its menu
    /// (coop_pad_input) for the whole game.
    fn character_pages_close(&mut self) {
        let pages = self.improved.character_screen.pages.clone();
        for page in pages.into_iter().flatten() {
            let Some(root) = self.propr(page, "Controller").as_object().filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()) && self.is_a(c, "GUIController")) else {
                continue;
            };
            let _ = self.call_named(root, "CloseAll", vec![Value::Bool(true)]);
        }
    }

    /// UGameEngine::Tick, before the client travel of this frame is taken: joining a host
    /// (network co-op) waits for the character screen, and goes once it is done. The engine would
    /// connect first and load the host's map: the address is held instead.
    pub(crate) fn character_screen_client_travel(&mut self) {
        let s = &mut self.improved.character_screen;
        if s.on && matches!(s.held, Some(Held::Client { .. })) && std::mem::take(&mut s.go) {
            if let Some(Held::Client { url, travel_type, items }) = s.held.take() {
                self.log.push(format!("Log: Character screen: on to {url}"));
                self.client_travel = Some((url, travel_type, items));
            }
            return;
        }
        let Some((url, travel_type, items)) = self.client_travel.clone() else { return };
        let parsed = crate::travel::Url::parse(Some(&self.url), &url, travel_type);
        let lp = self.local_play();
        let skip = false;
        let joining = !skip && lp.coop.is_some_and(|c| c.mode == CoopMode::LanJoin);
        if !joining || lp.characters.is_some() || self.improved.character_screen.on || parsed.host.is_empty() || parsed.has_option("failed") {
            return;
        }
        let map = crate::character_screen::places().map;
        self.log.push(format!("Log: Character screen: joining {url} held, choosing the character on {map}"));
        self.improved.character_screen.held = Some(Held::Client { url, travel_type, items });
        self.client_travel = Some((format!("{map}?{SCREEN_OPTION}"), crate::travel::TRAVEL_ABSOLUTE, false));
    }

    /// A travel that starts a co-op game whose characters are not chosen yet.
    fn character_screen_wanted(&self, r: &crate::travel::TravelRequest) -> bool {
        let lp = self.local_play();
        let Some(session) = lp.coop else { return false };
        if lp.characters.is_some() || r.pending_join || self.improved.character_screen.on {
            return false;
        }
        // A way of starting the game was chosen (joining: the host's address is the travel).
        if session.start.is_none() && session.mode != CoopMode::LanJoin {
            return false;
        }
        let map = r.url.map.to_ascii_lowercase();
        if map.starts_with("mapmenu") || r.url.has_option(SCREEN_OPTION) || r.url.has_option("failed") {
            return false;
        }
        // Joining: only the travel to the host.
        if session.mode == CoopMode::LanJoin && r.url.host.is_empty() {
            return false;
        }
        true
    }

    /// The main menu is shown again after going back from the screen: its Co-op page, on the
    /// step after the way to play together (or the host's address to join).
    pub(crate) fn character_screen_reopen_coop(&mut self) {
        let Some(mode) = self.improved.character_screen.reopen_coop.take() else { return };
        if !self.open_coop_page() {
            return;
        }
        self.change_local_play(|lp| lp.coop = Some(CoopSession { mode, start: None }));
        if let Some(nav) = self.improved.menu_boxes.coop_page.nav.as_mut() {
            nav.mode = Some(mode);
            nav.step = if mode == CoopMode::LanJoin { crate::coop_page::Step::Join } else { crate::coop_page::Step::Game };
            nav.focus = 0;
        }
    }

    /// Player `p`'s page, built as the game's own menu pages are: on the white page, a block of
    /// two comic panels with a thin black ink border, and the help bar under them across the
    /// block. The left panel is a picture of the game (CharacterScreen.ini's Picture) carrying a
    /// tag with the player across its top-left corner, the roster as the game's glass rows
    /// (the chosen one opaque; "<  Name : look  >" when that character has several looks, a small
    /// arrow at the right of the others that have several), a triangle above or below when the
    /// list goes on, and the Valider button. The right panel is the live view with the character
    /// standing in it. The help bar gives the keys of the player's own device. Where the block
    /// is, its size and the panels' shares come from the place file's layout ([`Layout`]).
    fn character_page_paint(&mut self, p: usize, page: ObjId, c: ObjId) {
        // The page covers the player's whole view, whatever area the menus are given (a network
        // guest's menus may be in a centred 4:3 frame): the canvas's origin and clip are the view's
        // own while the page is drawn, so that its panels and the scene drawn in the live-view
        // panel (client.rs) are in the same place.
        let (size_x, size_y) = (self.prop(c, "SizeX").as_int() as f32, self.prop(c, "SizeY").as_int() as f32);
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        let view = page_area((size_x, size_y), (rx, ry));
        let saved = ["OrgX", "OrgY", "ClipX", "ClipY"].map(|k| (k, self.prop(c, k)));
        for (k, v) in [("OrgX", 0.0), ("OrgY", 0.0), ("ClipX", view.0), ("ClipY", view.1)] {
            self.set_prop(c, k, Value::Float(v));
        }
        self.character_page_paint_in(p, page, c, view);
        for (k, v) in saved {
            self.set_prop(c, k, v);
        }
    }

    fn character_page_paint_in(&mut self, p: usize, page: ObjId, c: ObjId, (w, h): (f32, f32)) {
        let (ox, oy) = (0.0, 0.0);
        let ry = h / 480.0;
        // The page is one picture made for 1280 x 720 (a half of it in split screen), scaled as a
        // whole: its text too, at the height the menu font has there (the game's font keeps its
        // pixel size whatever the window, so on a smaller window the text would crowd out the rows).
        let players = self.improved.character_screen.pages.len();
        let layout = self.character_layout();
        // The reference text height and margin (TextSize, Margin).
        let t0 = layout.text;
        let reference_h = if players > 1 { 360.0 } else { 720.0 };
        // A compact block: its 1280-wide reference size made as large as the view allows, with a
        // margin of Margin of the view's height and width around it (the bar included), then
        // multiplied by BlockScale.
        let block_ref = (layout.aspect > 0.0).then(|| {
            let bw = layout.width * 1280.0;
            (bw, bw / layout.aspect)
        });
        // Two players side by side (client.rs): a title across the top of both halves, with room
        // kept for it above the blocks (TITLE_ROOM rows of the tag's height).
        let titled = players > 1 && self.engine.client.character_screen_side_by_side;
        const TITLE_ROOM: f32 = 2.4;
        let scale = match block_ref {
            Some((bw, bh)) => {
                let tag = t0 + 14.0;
                let title = if titled { tag * TITLE_ROOM } else { 0.0 };
                ((h * (1.0 - 2.0 * layout.margin)) / (bh + tag * 0.5 + title)).min((w * (1.0 - 2.0 * layout.margin)) / (bw * (1.0 + 2.0 * layout.bar))) * layout.scale
            }
            None => (h / reference_h).min(w / 1280.0),
        };
        self.improved.character_screen.text_scale = 1.0;
        let (_, font_h) = self.page_text_size(c, "Xg");
        self.improved.character_screen.text_scale = if font_h > 0.0 { (t0 * scale / font_h).clamp(0.3, 3.0) } else { 1.0 };
        let lang = self.text_language().to_string();
        let st = self.page_state(p);
        let mut hits = Vec::new();
        self.set_prop(c, "Style", Value::Byte(1));
        let (_, th) = self.page_text_size(c, "Xg");
        // Rows, gaps and margins in the reference's proportions to the text.
        let k = th / t0;
        let row = (th + 10.0 * k).max(12.0 * k);
        let gap = (row * 0.25).round().max(2.0);
        let m = (h * layout.margin).round();
        let (aw, _) = self.page_text_size(c, "<");
        // The roster's widest line (the chosen line of a character with several looks carries
        // its look's name and both arrows): the list panel is never narrower.
        // Every row's possible label (a person's name, or each person of a group).
        let mut lines = Vec::new();
        for i in 0..ROSTER.len() {
            for variant in 0..ROSTER[i].variants.len() {
                lines.push(row_label(&lang, i, variant).to_string());
            }
        }
        let widest = lines.iter().map(|t| self.page_text_size(c, t).0).fold(0.0f32, f32::max);
        let need_rows_w = widest + 2.0 * (aw + 18.0 * k) + 8.0 * k;
        // The block: centred, its height from its width and the widest proportion, leaving room
        // above for the tag that sits across the panel's corner.
        let tag_h = row + 4.0 * k;
        let title_room = if titled { (tag_h * TITLE_ROOM).round() } else { 0.0 };
        let room_h = h - 2.0 * m - tag_h * 0.5 - title_room;
        let (bw, bh) = match block_ref {
            Some((rw, rh)) => ((rw * scale).min(w - 2.0 * m), (rh * scale).min(room_h)),
            None => ((layout.width * w).min(w - 2.0 * m), room_h),
        };
        let bx = ((w - bw) / 2.0).round();
        let by = ((h - bh - title_room) / 2.0 + title_room + tag_h * 0.25).round();
        let over = (layout.bar * bw).round().min(bx - 2.0).max(0.0);
        let bar = [bx - over, by + bh - row, bw + 2.0 * over, row];
        let panels_h = bar[1] - gap * 1.5 - by;
        let pg = (layout.gap * bw).round().max(4.0);
        let list_w = (layout.list * bw).max(need_rows_w / 0.88).min(bw * 0.72).round();
        let list = [bx, by, list_w, panels_h];
        let extra = (layout.view_extra.get(p).copied().unwrap_or(0.0) * bw).round();
        let view = [bx + list_w + pg, by, bw - list_w - pg + extra, panels_h];
        // The white page, with a hole for the live view only.
        self.character_page_white(page, c, [0.0, 0.0, view[0], h]);
        self.character_page_white(page, c, [view[0] + view[2], 0.0, w - view[0] - view[2], h]);
        self.character_page_white(page, c, [view[0], 0.0, view[2], view[1]]);
        self.character_page_white(page, c, [view[0], view[1] + view[3], view[2], h - view[1] - view[3]]);
        // The left panel's picture, then both panels' ink borders.
        self.character_page_background(page, c, list, layout.picture);
        // The ink of the game's comic panels, thicker at higher resolutions.
        let ink = ((2.0 * ry).round().max(2.0) * layout.ink).max(1.0);
        self.character_page_frame(page, c, list, ink);
        self.character_page_frame(page, c, view, ink);
        self.character_screen_window(p, view, layout.room);
        // OutfitLabel=small: the outfit of a person with several, in small discreet letters (the
        // blue of the game's pick-up messages) at the bottom of the live view. Off by default.
        let outfit_label = self.improved.character_screen.places.as_ref().is_some_and(|pl| pl.outfit_label);
        if outfit_label && ROSTER[st.choice.character].variants.len() > 1 && !crate::character_screen::variants_are_people(st.choice.character) {
            if let Some(v) = ROSTER[st.choice.character].variants.get(st.choice.variant) {
                let outfit = crate::added_text::variant_name(&lang, v.name);
                let inset = (view[2] * 0.04).round().max(6.0);
                self.page_text(c, view[0] + inset, view[1] + view[3] - inset - th, outfit, [120, 190, 255, 255]);
            }
        }
        // The tag across the left panel's top-left corner: the player (the page's title alone),
        // and Ready once confirmed.
        let who = if players > 1 { fill(text(&lang, Text::CharacterPlayer), &[&(p + 1).to_string()]) } else { text(&lang, Text::CharacterTitle).to_string() };
        let tag = if st.confirmed { format!("{who} - {}", text(&lang, Text::CharacterReady)) } else { who };
        let (tw, _) = self.page_text_size(c, &tag);
        let tag_r = [(list[0] - (w * 0.012).round()).max(2.0), list[1] - (tag_h * 0.5).round(), tw + 28.0 * k, tag_h];
        let tag_colour = if st.confirmed { [0, 110, 0, 255] } else { [0, 0, 0, 255] };
        self.character_page_tag(page, c, tag_r, &tag, tag_colour);
        if titled {
            self.character_page_title(page, c, p, w, tag_r[1], &lang);
        }
        // The rows, the triangles and Valider, centred in the panel under the tag.
        let rows_w = (list[2] * 0.86).max(need_rows_w).min(list[2] - 8.0 * k);
        let rows_x = (list[0] + (list[2] - rows_w) / 2.0).round();
        let confirm_w = (rows_w * 0.6).round();
        let top = tag_r[1] + tag_r[3] + gap;
        let bottom = list[1] + list[3] - gap * 1.5;
        let n = ROSTER.len();
        // The rows, their gaps and the triangles lower together (down to the text's height and a
        // little) when the panel is low, so that the layout's number of rows fits when it can.
        let want = layout.rows.min(n).max(1);
        // Everything in proportion to the text (k), so that the same rows fit at every size.
        let sizes = |line: f32| {
            let g = (line * 0.2).max(1.0);
            let tri = (line * 0.45).max(6.0 * k);
            (g, tri, 2.0 * (tri + g) + 2.0 * g + line)
        };
        let fits = |line: f32, k: usize| {
            let (g, _, fixed) = sizes(line);
            fixed + k as f32 * (line + g) - g <= bottom - top
        };
        let smallest = (th + 4.0 * k).min(row);
        let mut line = row;
        while line > smallest && !fits(line, want) {
            line = (line - 0.5).max(smallest);
        }
        let (gap, tri, fixed) = sizes(line);
        let visible = (1..=want).rev().find(|&k| fits(line, k)).unwrap_or(1);
        let content = fixed + visible as f32 * (line + gap) - gap;
        let mut y = (top + ((bottom - top - content) / 2.0).max(0.0)).round();
        let start = list_start(st.choice.character, n, visible);
        let cx = rows_x + rows_w / 2.0;
        if start > 0 {
            self.character_page_triangle(page, c, cx, y + tri / 2.0, tri, Arrow::Up);
            hits.push(([rows_x + ox, y + oy, rows_w, tri + gap], Hit::Row(start - 1)));
        }
        y += tri + gap;
        for i in start..(start + visible).min(n) {
            let r = [rows_x, y, rows_w, line];
            let chosen = i == st.choice.character;
            let shown_variant = if chosen { st.choice.variant } else { st.variants.get(i).copied().unwrap_or(0) };
            let name = row_label(&lang, i, shown_variant);
            let looks = ROSTER[i].variants.len();
            if chosen && looks > 1 {
                // The chosen line with several looks: the selector's arrows at both ends (a click
                // on either changes it); its name, or the person of the group shown.
                self.character_page_box(page, c, r, name, true, true);
                self.page_text(c, r[0] + 8.0 * k, r[1] + (r[3] - th) / 2.0, "<", [0, 0, 0, 255]);
                self.page_text(c, r[0] + r[2] - 8.0 * k - aw, r[1] + (r[3] - th) / 2.0, ">", [0, 0, 0, 255]);
                if !st.confirmed {
                    let side = (r[2] * 0.18).max(aw + 16.0 * k);
                    hits.push(([r[0] + ox, r[1] + oy, side, r[3]], Hit::VariantPrev));
                    hits.push(([r[0] + r[2] - side + ox, r[1] + oy, side, r[3]], Hit::Variant));
                }
            } else {
                self.character_page_box(page, c, r, name, chosen, true);
                if looks > 1 {
                    // Several looks: the chosen line's arrows at both ends, fainter, seen before
                    // choosing.
                    let faint = [96, 96, 96, 255];
                    self.page_text(c, r[0] + 8.0 * k, r[1] + (r[3] - th) / 2.0, "<", faint);
                    self.page_text(c, r[0] + r[2] - 8.0 * k - aw, r[1] + (r[3] - th) / 2.0, ">", faint);
                }
            }
            hits.push(([r[0] + ox, r[1] + oy, r[2], r[3]], Hit::Row(i)));
            y += line + gap;
        }
        if start + visible < n {
            self.character_page_triangle(page, c, cx, y + tri / 2.0, tri, Arrow::Down);
            hits.push(([rows_x + ox, y + oy, rows_w, tri + gap], Hit::Row(start + visible)));
        }
        y += tri + gap * 2.0;
        let confirm_label = if st.confirmed { text(&lang, Text::CharacterReady) } else { text(&lang, Text::CharacterConfirm) };
        let ready = [(cx - confirm_w / 2.0).round(), y, confirm_w, line];
        // The first player's mouse (or finger) over Valider: the focused look of the game's
        // buttons, and their focus tick as it comes over it (a button's MouseEnter).
        let hovered = p == 0 && self.improved.character_screen.mouse_in_use && self.character_page_hit(p) == Some(Hit::Confirm);
        if p == 0 {
            if hovered && !self.improved.character_screen.confirm_hovered {
                self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
            }
            self.improved.character_screen.confirm_hovered = hovered;
        }
        self.character_page_box(page, c, ready, confirm_label, st.confirmed || hovered, true);
        hits.push(([ready[0] + ox, ready[1] + oy, ready[2], ready[3]], Hit::Confirm));
        // The help bar under the panels, across the block: back on the left, the moves in the
        // middle, ready on the right, for the player's own device (left/right only with several
        // looks).
        let pad = p == 1 || (players == 1 && self.aim_device.pad_active);
        let (move_hint, act_hint) = match (pad, st.confirmed) {
            (false, false) => (text(&lang, Text::HintKeysMove), text(&lang, Text::HintKeysAct)),
            (false, true) => ("", text(&lang, Text::HintKeysReady)),
            (true, false) => (text(&lang, Text::HintPadMove), text(&lang, Text::HintPadAct)),
            (true, true) => ("", text(&lang, Text::HintPadReady)),
        };
        let single = ROSTER[st.choice.character].variants.len() < 2;
        let moves = hint_items(move_hint, single).join("   ");
        let acts = hint_items(act_hint, false);
        self.character_page_tag(page, c, bar, "", [0, 0, 0, 255]);
        let grey = [110, 110, 110, 255];
        let ty = bar[1] + (bar[3] - th) / 2.0;
        let (left, right) = match acts.as_slice() {
            [ready, back] => (*back, *ready),
            [one] => (*one, ""),
            _ => ("", ""),
        };
        self.page_text(c, bar[0] + 10.0 * k, ty, left, grey);
        let (rw, _) = self.page_text_size(c, right);
        self.page_text(c, bar[0] + bar[2] - 10.0 * k - rw, ty, right, grey);
        let (mw, _) = self.page_text_size(c, &moves);
        let (lw, _) = self.page_text_size(c, left);
        if mw + lw + rw + 48.0 * k < bar[2] {
            self.page_text(c, bar[0] + (bar[2] - mw) / 2.0, ty, &moves, grey);
        }
        self.select_color(c, [255, 255, 255, 255]);
        if let Some(hit) = self.improved.character_screen.hits.get_mut(p) {
            *hit = hits;
        }
    }

    /// A white tag of the game's look with a thin black border and a drop shadow (the player's
    /// tag, the help bar), its text left aligned in `rgba`. `r` in pixels.
    fn character_page_tag(&mut self, page: ObjId, c: ObjId, r: [f32; 4], label: &str, rgba: [u8; 4]) {
        let s = (r[3] * 0.12).round().max(2.0);
        self.character_page_fill(page, c, [r[0] + s, r[1] + s, r[2], r[3]], [0, 0, 0, 255], false);
        self.character_page_fill(page, c, r, [255, 255, 255, 255], true);
        if !label.is_empty() {
            let (_, th) = self.page_text_size(c, label);
            let k = th / self.character_layout().text;
            self.page_text(c, r[0] + 14.0 * k, r[1] + (r[3] - th) / 2.0, label, rgba);
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// Side by side: the screen's title, one tag of the game's look in larger letters, centred
    /// on the line between the two halves, in the room above the player tags (`above`, the top
    /// of the tags). Each half is `w` wide and draws the part of the tag that falls in it (page
    /// 0's half on the left, page 1's on the right), so that both parts meet.
    fn character_page_title(&mut self, page: ObjId, c: ObjId, p: usize, w: f32, above: f32, lang: &str) {
        let label = text(lang, Text::CharacterSelection);
        let normal = self.improved.character_screen.text_scale;
        self.improved.character_screen.text_scale = normal * 1.35;
        let (tw, th) = self.page_text_size(c, label);
        let k = th / self.character_layout().text;
        let box_h = (th + 12.0 * k).round();
        let box_w = (tw + 40.0 * k).round();
        // The middle of the screen in this half's pixels.
        let middle = if p == 0 { w } else { 0.0 };
        // Just above the player tags (the title and the blocks are centred together).
        let r = [(middle - box_w / 2.0).round(), (above - box_h - (box_h * 0.6).round()).max(2.0), box_w, box_h];
        // The tag's look (character_page_tag: drop shadow, white, thin black border), drawn by
        // hand and cut at the half's edges: the canvas would push a rectangle crossing an edge
        // inside it, its border along the middle line.
        let cut = |x: f32, y: f32, rw: f32, rh: f32| -> Option<[f32; 4]> {
            let (x0, x1) = (x.max(0.0), (x + rw).min(w));
            (x1 > x0 && rh > 0.0).then_some([x0, y, x1 - x0, rh])
        };
        let s = (r[3] * 0.12).round().max(2.0);
        let t = (r[3] * 0.04).round().max(1.0);
        let parts = [
            (cut(r[0] + s, r[1] + s, r[2], r[3]), [0, 0, 0, 255]),
            (cut(r[0], r[1], r[2], r[3]), [255, 255, 255, 255]),
            (cut(r[0], r[1], r[2], t), [0, 0, 0, 255]),
            (cut(r[0], r[1] + r[3] - t, r[2], t), [0, 0, 0, 255]),
            (cut(r[0], r[1], t, r[3]), [0, 0, 0, 255]),
            (cut(r[0] + r[2] - t, r[1], t, r[3]), [0, 0, 0, 255]),
        ];
        for (part, rgba) in parts {
            if let Some(part) = part {
                self.character_page_fill(page, c, part, rgba, false);
            }
        }
        self.page_text(c, r[0] + (r[2] - tw) / 2.0, r[1] + (r[3] - th) / 2.0, label, [0, 0, 0, 255]);
        self.select_color(c, [255, 255, 255, 255]);
        self.improved.character_screen.text_scale = normal;
    }

    /// A small triangle of the menu (white with a black outline) centred on (`cx`, `cy`), `size`
    /// pixels across, pointing up or down (the list goes on).
    fn character_page_triangle(&mut self, page: ObjId, c: ObjId, cx: f32, cy: f32, size: f32, arrow: Arrow) {
        let Some(white) = self.character_page_white_texture(page) else { return };
        self.set_prop(c, "Style", Value::Byte(1));
        let layers: &[(f32, [u8; 4])] = match arrow {
            _ => &[(2.0, [0, 0, 0, 255]), (0.0, [255, 255, 255, 255])],
        };
        for &(grow, rgba) in layers {
            self.select_color(c, rgba);
            let s = (size + 2.0 * grow).round().max(2.0);
            let n = (s / 2.0).ceil() as i32;
            for k in 0..n {
                // Strip k from the tip: its length grows to the base.
                let len = (s * (k + 1) as f32 / n as f32).round().max(1.0);
                let r = match arrow {
                    Arrow::Up => [cx - len / 2.0, cy - n as f32 / 2.0 + k as f32, len, 1.0],
                    Arrow::Down => [cx - len / 2.0, cy + n as f32 / 2.0 - 1.0 - k as f32, len, 1.0],
                };
                self.select_draw_texture(page, c, r, white.clone());
            }
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// The left panel's background in `r`: the game's multi_splitter picture (the multiplayer
    /// rules page's), cropped to the panel's shape, or the Co-op box's frame of the game's video.
    fn character_page_background(&mut self, page: ObjId, c: ObjId, r: [f32; 4], picture: PanelPicture) {
        match picture {
            PanelPicture::Coop => {
                if let Some(pic) = self.character_page_picture(r[2] / r[3].max(1.0)) {
                    self.select_color(c, [255, 255, 255, 255]);
                    self.select_draw_texture(page, c, r, pic);
                }
            }
            PanelPicture::Splitter => {
                let tex = match self.improved.character_screen.splitter.clone() {
                    Some(t) => t,
                    None => {
                        let class = self.class("Texture").map(|c| Value::Class(Some(c))).unwrap_or(Value::Class(None));
                        let Some(t) = self.call_named(page, "DynamicLoadObject", vec![Value::Str(SPLITTER.to_string()), class]).ok().filter(|v| v.as_bool()) else { return };
                        self.improved.character_screen.splitter = Some(t.clone());
                        t
                    }
                };
                let size = |vm: &Self, k: &str| tex.as_object().map_or(0.0, |o| vm.propr(o, k).as_int() as f32);
                let (uw, vh) = (size(self, "USize"), size(self, "VSize"));
                let (uw, vh) = if uw > 0.0 && vh > 0.0 { (uw, vh) } else { (256.0, 256.0) };
                let uv = crop_uv(uw, vh, r[2] / r[3].max(1.0));
                self.select_color(c, [255, 255, 255, 255]);
                self.set_prop(c, "Style", Value::Byte(1));
                self.set_prop(c, "CurX", Value::Float(r[0]));
                self.set_prop(c, "CurY", Value::Float(r[1]));
                let mut args = vec![tex, Value::Float(r[2]), Value::Float(r[3]), Value::Float(uv[0]), Value::Float(uv[1]), Value::Float(uv[2]), Value::Float(uv[3])];
                let _ = self.canvas_native("DrawTile", c, &mut args, &[true; 7]);
            }
        }
    }

    /// The left panel's picture: the Co-op box's frame of the game's video (made in the
    /// background, still_frames.rs), cropped to the panel's `aspect`; None until it is ready.
    fn character_page_picture(&mut self, aspect: f32) -> Option<Value> {
        if let Some((a, pic)) = self.improved.character_screen.picture.clone() {
            if (a - aspect).abs() < 0.02 {
                return Some(pic);
            }
        }
        let (file, frame) = (crate::menu_boxes::COOP_VIDEO, crate::menu_boxes::COOP_FRAME);
        let video = self.engine.video.dir.join(format!("{file}.bik"));
        crate::still_frames::request(&video, frame);
        let f = crate::still_frames::get(&video, frame)?;
        let img = crate::menu_boxes::Image { w: f.w, h: f.h, rgba: f.rgba.clone() };
        let fitted = crate::menu_boxes::crop_to_aspect(&img, aspect, crate::menu_boxes::Crop { cx: 0.5, cy: 0.5, h: 1.0 });
        self.improved.character_screen.picture_serial += 1;
        let name = format!("Mod.CharacterScreen.Picture{}", self.improved.character_screen.picture_serial);
        let pic = Self::menu_register(&name, fitted);
        self.improved.character_screen.picture = Some((aspect, pic.clone()));
        Some(pic)
    }

    /// The white page (the menu's own white texture), opaque. `r` in pixels.
    fn character_page_white(&mut self, page: ObjId, c: ObjId, r: [f32; 4]) {
        if r[2] <= 0.0 || r[3] <= 0.0 {
            return;
        }
        let Some(white) = self.character_page_white_texture(page) else { return };
        self.set_prop(c, "Style", Value::Byte(1));
        self.select_color(c, [255, 255, 255, 255]);
        self.select_draw_texture(page, c, r, white);
    }

    /// The comic panel's black frame around `r`, `t` pixels thick.
    fn character_page_frame(&mut self, page: ObjId, c: ObjId, r: [f32; 4], t: f32) {
        let Some(white) = self.character_page_white_texture(page) else { return };
        self.set_prop(c, "Style", Value::Byte(1));
        self.select_color(c, [0, 0, 0, 255]);
        for e in [[r[0] - t, r[1] - t, r[2] + 2.0 * t, t], [r[0] - t, r[1] + r[3], r[2] + 2.0 * t, t], [r[0] - t, r[1], t, r[3]], [r[0] + r[2], r[1], t, r[3]]]
        {
            self.select_draw_texture(page, c, e, white.clone());
        }
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// XIIIMenuStart.menublanc (the GUI controller's first DefaultPens), loaded once.
    fn character_page_white_texture(&mut self, page: ObjId) -> Option<Value> {
        if let Some(t) = self.improved.character_screen.white.clone() {
            return Some(t);
        }
        let class = self.class("Texture").map(|c| Value::Class(Some(c))).unwrap_or(Value::Class(None));
        let t = self.call_named(page, "DynamicLoadObject", vec![Value::Str("XIIIMenuStart.menublanc".to_string()), class]).ok().filter(|v| v.as_bool())?;
        self.improved.character_screen.white = Some(t.clone());
        Some(t)
    }

    /// Player `p`'s live-view panel `win` (pixels within their viewport): the scene is drawn
    /// there, with the recorded view's height of picture (character_screen.rs).
    fn character_screen_window(&mut self, p: usize, win: [f32; 4], room: f32) {
        if let Some(panel) = self.improved.character_screen.panels.get_mut(p) {
            *panel = Some((win, room));
        }
    }

    /// A box of colour `rgba` (the menu's white texture tinted), with the canvas's black border.
    fn character_page_fill(&mut self, page: ObjId, c: ObjId, r: [f32; 4], rgba: [u8; 4], border: bool) {
        let Some(white) = self.character_page_white_texture(page) else { return };
        self.set_prop(c, "Style", Value::Byte(1));
        self.select_color(c, rgba);
        self.set_prop(c, "bUseBorder", Value::Bool(border));
        self.select_draw_texture(page, c, r, white);
        self.set_prop(c, "bUseBorder", Value::Bool(false));
        self.select_color(c, [255, 255, 255, 255]);
    }

    /// A box of the page in the game's look (glass, opaque with a black border when chosen),
    /// its text in black (left aligned in the roster, centred on the buttons). `r` in pixels.
    fn character_page_box(&mut self, page: ObjId, c: ObjId, r: [f32; 4], label: &str, focus: bool, centred: bool) {
        // The game's glass rows (XIIIButton: FondMenu half transparent, opaque with a black border
        // when chosen), on the picture panel.
        if let Some(glass) = self.gui_local().map(|g| self.propr(g, "FondMenu").clone()) {
            self.set_prop(c, "Style", Value::Byte(5));
            self.select_color(c, [255, 255, 255, if focus { 255 } else { 150 }]);
            self.set_prop(c, "bUseBorder", Value::Bool(focus));
            self.select_draw_texture(page, c, r, glass);
            self.set_prop(c, "bUseBorder", Value::Bool(false));
            self.set_prop(c, "Style", Value::Byte(1));
        }
        let (tw, th) = self.page_text_size(c, label);
        let k = th / self.character_layout().text;
        let x = if centred { r[0] + (r[2] - tw) / 2.0 } else { r[0] + 8.0 * k };
        self.page_text(c, x, r[1] + (r[3] - th) / 2.0, label, [0, 0, 0, 255]);
    }
}

/// The page's small triangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arrow {
    Up,
    Down,
}

/// The part of a `w` x `h` texture (u, v, width, height in texels) that fills a panel of shape
/// `aspect` (width / height): all of its width or height, centred.
pub fn crop_uv(w: f32, h: f32, aspect: f32) -> [f32; 4] {
    if w / h > aspect {
        let cw = h * aspect;
        [(w - cw) / 2.0, 0.0, cw, h]
    } else {
        let ch = w / aspect.max(0.01);
        [0.0, (h - ch) / 2.0, w, ch]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_reach_the_page() {
        assert!(converted_click(0, 3), "the menu's converted click (its release)");
        assert!(!converted_click(0, 1) && !converted_click(0x0D, 3) && !converted_click(0x01, 3));
        // The right button does nothing; Escape, Backspace and the gamepad's back stay.
        assert!(swallowed_on_screen(0x02));
        assert!(![0x01, 0x08, 0x1B, 0xC8, 0x0D].into_iter().any(swallowed_on_screen));
        // A click on a row picks it, on Valider confirms (the page's keys).
        let mut st = PageState::new(Choice::default());
        assert_eq!(st.key(PageKey::Pick(2)), PageOutcome::Show(Choice { character: 2, variant: crate::character_screen::default_variants()[2] }));
        assert_eq!(st.key(PageKey::Confirm), PageOutcome::Confirmed(true));
    }

    #[test]
    fn the_page_covers_the_whole_view() {
        // A 16:9 view whose menus are framed 4:3 (a network guest): the page still takes the view.
        assert_eq!(page_area((1024.0, 576.0), (1.2, 1.2)), (1024.0, 576.0));
        assert_eq!(page_area((1280.0, 720.0), (1.5, 1.5)), (1280.0, 720.0));
        assert_eq!(page_area((800.0, 600.0), (1.25, 1.25)), (800.0, 600.0));
        // A half of a split screen.
        assert_eq!(page_area((1280.0, 360.0), (2.0, 0.75)), (1280.0, 360.0));
        // Unknown size: the menus' own area.
        assert_eq!(page_area((0.0, 0.0), (2.0, 1.5)), (1280.0, 720.0));
    }

    #[test]
    fn the_panel_picture_is_cropped_to_the_panel() {
        assert_eq!(crop_uv(256.0, 256.0, 2.0), [0.0, 64.0, 256.0, 128.0]);
        assert_eq!(crop_uv(256.0, 256.0, 0.5), [64.0, 0.0, 128.0, 256.0]);
        assert_eq!(crop_uv(256.0, 128.0, 2.0), [0.0, 0.0, 256.0, 128.0]);
    }

    #[test]
    fn hints_without_the_variant_keys_and_on_narrow_lines() {
        let fr = "Haut/Bas : personnage   Gauche/Droite : variante";
        assert_eq!(hint_items(fr, false), vec!["Haut/Bas : personnage", "Gauche/Droite : variante"]);
        assert_eq!(hint_items(fr, true), vec!["Haut/Bas : personnage"], "one variant: no left/right");
        assert_eq!(hint_items("Entrée : prêt   Échap : retour", false).len(), 2, "the action line keeps both");
        for lang in crate::added_text::LANGUAGES {
            for t in [Text::HintKeysMove, Text::HintPadMove] {
                assert_eq!(hint_items(text(lang, t), false).len(), 2, "{lang} {t:?}: two items");
            }
        }
        let items = ["aaaa", "bb", "cccccc"];
        let widths = [40.0, 20.0, 60.0];
        assert_eq!(hint_lines(&items, &widths, 10.0, 70.0), vec!["aaaa   bb".to_string(), "cccccc".to_string()]);
        assert_eq!(hint_lines(&items, &widths, 10.0, 1000.0).len(), 1);
        assert_eq!(hint_lines(&items, &widths, 10.0, 10.0).len(), 3, "an item wider than the line is a line alone");
    }

    #[test]
    fn moving_shows_each_character_with_its_last_variant() {
        let mut st = PageState::new(Choice::default());
        // Jones shows her default outfit (her camouflage, character_screen::VARIANT_ORDER).
        let jones_default = crate::character_screen::default_variants()[1];
        assert_eq!(st.key(PageKey::Down), PageOutcome::Show(Choice { character: 1, variant: jones_default }));
        assert_eq!(st.key(PageKey::Up), PageOutcome::Show(Choice::default()));
        assert_eq!(st.key(PageKey::Up), PageOutcome::Show(Choice { character: ROSTER.len() - 1, variant: 0 }), "wraps to the bottom");
        let killer = ROSTER.iter().position(|c| c.id == "Killer").unwrap();
        st.key(PageKey::Pick(killer));
        assert_eq!(st.key(PageKey::Right), PageOutcome::Show(Choice { character: killer, variant: 1 }));
        assert_eq!(st.key(PageKey::Variant), PageOutcome::Show(Choice { character: killer, variant: 2 }));
        st.key(PageKey::Down);
        st.key(PageKey::Up);
        assert_eq!(st.choice, Choice { character: killer, variant: 2 }, "the killer chosen before");
        assert_eq!(st.key(PageKey::Left), PageOutcome::Show(Choice { character: killer, variant: 1 }));
        st.key(PageKey::Left);
        assert_eq!(st.key(PageKey::Left), PageOutcome::Show(Choice { character: killer, variant: 3 }), "wraps");
        // One look (Carrington): the variant keys do nothing.
        st.key(PageKey::Pick(2));
        assert_eq!(st.key(PageKey::Right), PageOutcome::None);
        assert_eq!(st.key(PageKey::Pick(99)), PageOutcome::None);
    }

    #[test]
    fn ready_is_taken_back_first_then_back_leaves_for_any_player() {
        let mut st = PageState::new(Choice { character: 2, variant: 0 });
        assert_eq!(st.key(PageKey::Enter), PageOutcome::Confirmed(true));
        assert_eq!(st.key(PageKey::Down), PageOutcome::None, "ready: the list stays");
        assert_eq!(st.key(PageKey::Enter), PageOutcome::None, "ready: a second press is not a surprise");
        assert_eq!(st.key(PageKey::Back), PageOutcome::Confirmed(false), "Back takes ready back first");
        assert_eq!(st.key(PageKey::Back), PageOutcome::Leave, "then leaves, whoever presses it");
        assert_eq!(st.key(PageKey::Confirm), PageOutcome::Confirmed(true));
        assert_eq!(st.key(PageKey::Confirm), PageOutcome::Confirmed(false), "a click on Ready takes it back");
    }

    #[test]
    fn the_list_keeps_the_chosen_row_in_view() {
        assert_eq!(list_start(0, 16, 20), 0);
        assert_eq!(list_start(0, 16, 7), 0);
        assert_eq!(list_start(8, 16, 7), 5);
        assert_eq!(list_start(15, 16, 7), 9);
        for chosen in 0..16 {
            for visible in 3..16 {
                let s = list_start(chosen, 16, visible);
                assert!(s <= chosen && chosen < s + visible && s + visible <= 16, "{chosen} {visible}");
            }
        }
    }

    #[test]
    fn last_choices_read_back() {
        let c = read_choices("[CoopCharacters]\r\nPlayer1=Jones\r\nPlayer2=Killer.3\r\nPlayer3=Kim\r\n");
        assert_eq!(c[0].code(), "Jones");
        assert_eq!(c[1].code(), "Killer.3");
        assert_eq!(read_choices("Player1=Nobody"), [Choice::default(); 2]);
    }

    #[test]
    fn names_in_every_language() {
        for lang in crate::added_text::LANGUAGES {
            for i in 0..ROSTER.len() {
                assert!(!character_name(lang, i).is_empty());
            }
        }
        assert_eq!(character_name("frt", 3), "La Mangouste");
    }
}
