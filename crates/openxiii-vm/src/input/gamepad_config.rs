//! Gamepad settings of each local player (not in the original PC version, which has no gamepad
//! bindings): the action of each button, vibration, aim assist, vertical inversion of the look
//! stick and look speed. Saved in their own file ([`FILE_NAME`], next to User.ini), one section
//! per player; the game's own ini files are not touched.
//!
//! The gamepad plays the keyboard and mouse bindings of DefUser.ini (the viewer's pad_keys.rs): an
//! action is the key or mouse input it sends. The default settings are exactly the fixed table
//! the gamepad used before these settings existed, so a player without a settings file plays as
//! before.
//!
//! The settings page shows the console versions' controls picture (XIIIMenuControlsWindow,
//! XIIIMenuStart.controlsPS2_1…4), so the buttons are also named after the PlayStation 2 gamepad.
//! The look speed is the console slider of XIIIMenuAdvancedControlsWindow (0…10, fLookSpeed =
//! 0.7 + value / 10, 3 = 1.0), applied to the look stick alone.

use std::path::Path;

/// Local players with their own gamepad settings (split screen and co-op go up to four).
pub const PLAYERS: usize = 4;
/// Settings file, in the player's ini folder (save\System next to the game).
pub const FILE_NAME: &str = "Gamepad.ini";
/// Look speed of the console slider at fLookSpeed 1.0 (DefUser.ini): the default.
pub const LOOK_SPEED_DEFAULT: u8 = 3;
/// Highest value of the console slider (XIIIMenuAdvancedControlsWindow: SetRange(0, 10, 1)).
pub const LOOK_SPEED_MAX: u8 = 10;
/// A trigger pressed further than this is a pressed button (pad_keys.rs's fire threshold).
pub const TRIGGER_PRESSED: f32 = 0.5;

/// A button of the gamepad. The order is the bit order of the combined gamepad word (the
/// viewer's pad_keys.rs `B_*` bits, then the two triggers as bits 14 and 15).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    LeftBumper,
    RightBumper,
    Back,
    Start,
    LeftThumb,
    RightThumb,
    Up,
    Down,
    Left,
    Right,
    LeftTrigger,
    RightTrigger,
}

/// Every button, in bit order.
pub const BUTTONS: [Button; 16] = [
    Button::A,
    Button::B,
    Button::X,
    Button::Y,
    Button::LeftBumper,
    Button::RightBumper,
    Button::Back,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::LeftTrigger,
    Button::RightTrigger,
];

impl Button {
    pub fn index(self) -> usize {
        BUTTONS.iter().position(|&b| b == self).unwrap_or(0)
    }

    pub fn bit(self) -> u16 {
        1 << self.index()
    }

    /// Key of the settings file.
    pub fn key(self) -> &'static str {
        [
            "A",
            "B",
            "X",
            "Y",
            "LeftBumper",
            "RightBumper",
            "Back",
            "Start",
            "LeftThumb",
            "RightThumb",
            "DPadUp",
            "DPadDown",
            "DPadLeft",
            "DPadRight",
            "LeftTrigger",
            "RightTrigger",
        ][self.index()]
    }

    /// Name of the button on the PlayStation 2 gamepad of the picture (the console page's
    /// sButtonsP names, shortened).
    pub fn ps2_name(self) -> &'static str {
        [
            "X", "O", "[]", "/\\", "L1", "R1", "SELECT", "START", "L3", "R3", "+", "+", "+", "+", "L2", "R2",
        ][self.index()]
    }

    /// The Start button stays the menu button: it cannot be given another action (without it a
    /// gamepad could no longer open the menu).
    pub fn locked(self) -> bool {
        self == Button::Start
    }
}

/// An action of a button: the keyboard or mouse input of DefUser.ini that it plays.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Action {
    None,
    Jump,
    Crouch,
    Reload,
    Grab,
    QuickHeal,
    Menu,
    Walk,
    PrevItem,
    NextItem,
    Fire,
    AltFire,
    PrevWeapon,
    NextWeapon,
}

/// Every action, in the order in which their keys are pressed during a frame (the order of the
/// gamepad's former fixed table) and offered on the page.
pub const ACTIONS: [Action; 14] = [
    Action::Jump,
    Action::Crouch,
    Action::Reload,
    Action::Grab,
    Action::QuickHeal,
    Action::Menu,
    Action::Walk,
    Action::PrevItem,
    Action::NextItem,
    Action::Fire,
    Action::AltFire,
    Action::PrevWeapon,
    Action::NextWeapon,
    Action::None,
];

/// What an action sends to the game.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    /// Windows virtual key held while the button is held.
    Key(u8),
    /// Mouse button (0 left, 1 right) held while the button is held.
    Mouse(u8),
    /// One wheel notch (−1 down, 1 up) when the button is pressed.
    Wheel(i32),
    Nothing,
}

impl Action {
    /// The key or mouse input of DefUser.ini's binding for this action.
    pub fn output(self) -> Output {
        match self {
            Action::Jump => Output::Key(0x20),       // Space=Jump
            Action::Crouch => Output::Key(0x43),     // C=Duck
            Action::Reload => Output::Key(0x52),     // R=Reload
            Action::Grab => Output::Key(0x45),       // E=Grab
            Action::QuickHeal => Output::Key(0x51),  // Q=QuickHeal
            Action::Menu => Output::Key(0x1B),       // Escape=Showmenu
            Action::Walk => Output::Key(0xA0),       // LShift=Walking
            Action::PrevItem => Output::Key(0x46),   // F=InventoryPrevious
            Action::NextItem => Output::Key(0x47),   // G=InventoryNext
            Action::Fire => Output::Mouse(0),        // LeftMouse=Fire
            Action::AltFire => Output::Mouse(1),     // RightMouse=AltFire
            Action::PrevWeapon => Output::Wheel(-1), // MouseWheelDown=PrevWeapon
            Action::NextWeapon => Output::Wheel(1),  // MouseWheelUp=NextWeapon
            Action::None => Output::Nothing,
        }
    }

    /// Value in the settings file (the game's command name).
    pub fn key(self) -> &'static str {
        match self {
            Action::None => "None",
            Action::Jump => "Jump",
            Action::Crouch => "Duck",
            Action::Reload => "Reload",
            Action::Grab => "Grab",
            Action::QuickHeal => "QuickHeal",
            Action::Menu => "ShowMenu",
            Action::Walk => "Walking",
            Action::PrevItem => "InventoryPrevious",
            Action::NextItem => "InventoryNext",
            Action::Fire => "Fire",
            Action::AltFire => "AltFire",
            Action::PrevWeapon => "PrevWeapon",
            Action::NextWeapon => "NextWeapon",
        }
    }

    pub fn from_key(s: &str) -> Option<Action> {
        ACTIONS.into_iter().find(|a| a.key().eq_ignore_ascii_case(s.trim()))
    }

    /// The name of the action in the game's own PC key settings (XIDInterf localization:
    /// section, key), when the game has one.
    pub fn game_label(self) -> Option<(&'static str, &'static str)> {
        const MOVE: &str = "XIIIMenuInputMoveClient";
        const SHOOT: &str = "XIIIMenuInputShootClient";
        const OTHER: &str = "XIIIMenuInputOtherClient";
        Some(match self {
            Action::Jump => (MOVE, "LabelList[4]"),
            Action::Crouch => (MOVE, "LabelList[5]"),
            Action::Walk => (MOVE, "LabelList[6]"),
            Action::Fire => (SHOOT, "LabelList[0]"),
            Action::AltFire => (SHOOT, "LabelList[1]"),
            Action::Reload => (SHOOT, "LabelList[2]"),
            Action::NextWeapon => (SHOOT, "LabelList[3]"),
            Action::PrevWeapon => (SHOOT, "LabelList[4]"),
            Action::Grab => (OTHER, "LabelList[0]"),
            Action::PrevItem => (OTHER, "LabelList[1]"),
            Action::NextItem => (OTHER, "LabelList[2]"),
            Action::QuickHeal => (OTHER, "LabelList[6]"),
            Action::Menu | Action::None => return None,
        })
    }

    /// English name, when the game has none in its language.
    pub fn english(self) -> &'static str {
        match self {
            Action::None => "-",
            Action::Jump => "Jump",
            Action::Crouch => "Crouch",
            Action::Reload => "Reload",
            Action::Grab => "Grab / use",
            Action::QuickHeal => "Quick heal",
            Action::Menu => "Menu",
            Action::Walk => "Walk",
            Action::PrevItem => "Previous item",
            Action::NextItem => "Next item",
            Action::Fire => "Fire",
            Action::AltFire => "Alt fire",
            Action::PrevWeapon => "Previous weapon",
            Action::NextWeapon => "Next weapon",
        }
    }
}

/// Inputs of one frame of a player's gamepad: keys held (in press order), mouse buttons held,
/// wheel notches sent this frame.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Outputs {
    pub keys: Vec<u8>,
    pub mouse: [bool; 2],
    pub wheel: Vec<i32>,
}

/// Gamepad settings of one player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerPad {
    /// Action of each button, in [`BUTTONS`] order.
    pub actions: [Action; 16],
    pub vibration: bool,
    /// Aim assist with the gamepad. Player 1's lives in User.ini ([OpenXIII.Gamepad] AutoAim,
    /// aim_device.rs), the value the game already uses: the file keeps it for the other players.
    pub aim_assist: bool,
    pub invert_look: bool,
    /// Console slider value, 0…[`LOOK_SPEED_MAX`].
    pub look_speed: u8,
}

impl Default for PlayerPad {
    /// The gamepad's former fixed table (pad_keys.rs): A jump, B crouch, X reload, Y grab, LB /
    /// RB previous / next weapon, Back and d-pad up quick heal, Start menu, left stick click walk,
    /// d-pad left / right previous / next item, left trigger alt fire, right trigger fire.
    fn default() -> Self {
        let mut actions = [Action::None; 16];
        for (b, a) in [
            (Button::A, Action::Jump),
            (Button::B, Action::Crouch),
            (Button::X, Action::Reload),
            (Button::Y, Action::Grab),
            (Button::LeftBumper, Action::PrevWeapon),
            (Button::RightBumper, Action::NextWeapon),
            (Button::Back, Action::QuickHeal),
            (Button::Start, Action::Menu),
            (Button::LeftThumb, Action::Walk),
            (Button::Up, Action::QuickHeal),
            (Button::Left, Action::PrevItem),
            (Button::Right, Action::NextItem),
            (Button::LeftTrigger, Action::AltFire),
            (Button::RightTrigger, Action::Fire),
        ] {
            actions[b.index()] = a;
        }
        PlayerPad { actions, vibration: true, aim_assist: true, invert_look: false, look_speed: LOOK_SPEED_DEFAULT }
    }
}

impl PlayerPad {
    pub fn action(&self, b: Button) -> Action {
        self.actions[b.index()]
    }

    /// Remapping: `from`'s action goes to `to`, and `to`'s action to `from` (a conflict is an
    /// exchange). Nothing happens with the locked Start button; false then.
    pub fn swap(&mut self, from: Button, to: Button) -> bool {
        if from.locked() || to.locked() {
            return false;
        }
        self.actions.swap(from.index(), to.index());
        true
    }

    /// Gives button `b` action `a` (not the menu, and nothing on the locked Start button): false
    /// when nothing changed.
    pub fn set(&mut self, b: Button, a: Action) -> bool {
        if b.locked() || a == Action::Menu || self.action(b) == a {
            return false;
        }
        self.actions[b.index()] = a;
        true
    }

    /// Next (`step` 1) or previous (−1) action of button `b`, the menu excepted (it stays on
    /// Start). Another button may then have the same action (as Back and d-pad up by default).
    pub fn cycle(&mut self, b: Button, step: i32) -> bool {
        if b.locked() {
            return false;
        }
        let list: Vec<Action> = ACTIONS.into_iter().filter(|&a| a != Action::Menu).collect();
        let i = list.iter().position(|&a| a == self.action(b)).unwrap_or(list.len() - 1) as i32;
        let n = list.len() as i32;
        self.actions[b.index()] = list[(i + step).rem_euclid(n) as usize];
        true
    }

    /// Settings of the look stick for the VM (`look_stick_tune`): look speed factor (fLookSpeed
    /// of the console slider, exactly 1.0 at the default) and vertical inversion.
    pub fn look_tune(&self) -> (f32, bool) {
        (f32::from(7 + self.look_speed.min(LOOK_SPEED_MAX)) / 10.0, self.invert_look)
    }

    /// Inputs of a frame: `now` and `before` are this frame's and the previous frame's button
    /// words ([`word`]). Keys in [`ACTIONS`] order, wheel notches in [`BUTTONS`] order.
    pub fn outputs(&self, now: u16, before: u16) -> Outputs {
        let mut out = Outputs::default();
        let held = |a: Action| BUTTONS.iter().any(|&b| now & b.bit() != 0 && self.action(b) == a);
        for a in ACTIONS {
            match a.output() {
                Output::Key(vk) if held(a) && !out.keys.contains(&vk) => out.keys.push(vk),
                Output::Mouse(m) if held(a) => out.mouse[usize::from(m)] = true,
                _ => {}
            }
        }
        for b in BUTTONS {
            if now & b.bit() != 0 && before & b.bit() == 0 {
                if let Output::Wheel(n) = self.action(b).output() {
                    out.wheel.push(n);
                }
            }
        }
        out
    }
}

/// Button word of a frame: the combined gamepad's button bits (bits 0…13) and the triggers
/// pressed past [`TRIGGER_PRESSED`] (bits 14 and 15). `lt`, `rt`: 0…1.
pub fn word(buttons: u32, lt: f32, rt: f32) -> u16 {
    (buttons & 0x3FFF) as u16 | (u16::from(lt > TRIGGER_PRESSED) << 14) | (u16::from(rt > TRIGGER_PRESSED) << 15)
}

/// Button word ([`word`] layout) and left stick (x right, y down, −1…1) of a gamepad state as the
/// game reads it (DIJOYSTATE: buttons in the combined word's order, triggers on the shared Z axis).
pub fn state_word(state: &crate::pad::PadState) -> (u16, (f32, f32)) {
    const TRIGGER: i32 = 16384;
    let mut w: u16 = (0..14).filter(|&i| state.buttons[i]).fold(0, |w, i| w | 1 << i);
    if state.axes[2] > TRIGGER {
        w |= Button::LeftTrigger.bit();
    }
    if state.axes[2] < -TRIGGER {
        w |= Button::RightTrigger.bit();
    }
    (w, (state.axes[0] as f32 / 32767.0, state.axes[1] as f32 / 32767.0))
}

/// The settings of the local players.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GamepadConfig {
    pub players: [PlayerPad; PLAYERS],
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl GamepadConfig {
    /// Settings file text. Unknown sections, keys or values are ignored (the default stays); the
    /// Start button keeps the menu.
    pub fn parse(text: &str) -> GamepadConfig {
        let mut c = GamepadConfig::default();
        let mut player: Option<usize> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                player = name.trim().strip_prefix("Player").and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=PLAYERS).contains(n)).map(|n| n - 1);
                continue;
            }
            let (Some(p), Some((k, v))) = (player, line.split_once('=')) else { continue };
            let pad = &mut c.players[p];
            let k = k.trim();
            if let Some(b) = BUTTONS.into_iter().find(|b| b.key().eq_ignore_ascii_case(k)) {
                if let Some(a) = Action::from_key(v).filter(|&a| !b.locked() && a != Action::Menu) {
                    pad.actions[b.index()] = a;
                }
            } else if k.eq_ignore_ascii_case("Vibration") {
                pad.vibration = parse_bool(v).unwrap_or(pad.vibration);
            } else if k.eq_ignore_ascii_case("AimAssist") {
                pad.aim_assist = parse_bool(v).unwrap_or(pad.aim_assist);
            } else if k.eq_ignore_ascii_case("InvertLook") {
                pad.invert_look = parse_bool(v).unwrap_or(pad.invert_look);
            } else if k.eq_ignore_ascii_case("LookSpeed") {
                if let Ok(n) = v.trim().parse::<u8>() {
                    pad.look_speed = n.min(LOOK_SPEED_MAX);
                }
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("; OpenXIII gamepad settings, one section per local player.\r\n");
        for (i, p) in self.players.iter().enumerate() {
            s.push_str(&format!("[Player{}]\r\n", i + 1));
            for b in BUTTONS {
                s.push_str(&format!("{}={}\r\n", b.key(), p.action(b).key()));
            }
            let yes = |v: bool| if v { "True" } else { "False" };
            s.push_str(&format!("Vibration={}\r\n", yes(p.vibration)));
            // Player 1's aim assist is User.ini's (aim_device.rs).
            if i > 0 {
                s.push_str(&format!("AimAssist={}\r\n", yes(p.aim_assist)));
            }
            s.push_str(&format!("InvertLook={}\r\nLookSpeed={}\r\n\r\n", yes(p.invert_look), p.look_speed));
        }
        s
    }

    /// Settings of folder `dir` (the default when the file is missing or unreadable).
    pub fn load(dir: &Path) -> GamepadConfig {
        std::fs::read(dir.join(FILE_NAME)).map(|b| GamepadConfig::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(FILE_NAME), self.to_text())
    }
}

impl crate::Vm {
    /// A page of the game's menus is open (the local GUIController's ActivePage): the gamepad
    /// settings page is drawn over it.
    pub fn gui_page_active(&self) -> bool {
        self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object()).is_some_and(|p| self.objects.get(p).is_some_and(|x| x.is_some()))
    }

    /// A text of the installed game's localization ("Package.<language>", [section] key), if
    /// present: the gamepad settings page uses the game's own words (in both builds).
    pub fn game_text(&mut self, section: &str, key: &str, package: &str) -> Option<String> {
        self.config.as_mut()?.localize2(section, key, package).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// Language code of the game's localization files ("int", "frt", "det", "itt", "est").
    pub fn game_language(&self) -> String {
        self.config.as_ref().map_or_else(|| "int".to_string(), |c| c.language.clone())
    }

    /// Local player (0 = first) whose view reads gamepad `handle` (the client's viewport_pads);
    /// the first player when no view reads it (one player: every gamepad is the first player's).
    pub fn gamepad_player(&self, handle: usize) -> usize {
        self.engine.client.viewport_pads.iter().position(|&h| h == Some(handle)).map_or(0, |slot| self.player_of_slot(slot))
    }

    /// Local player (0 = first, the rank of their gamepad settings) whose view is viewport
    /// `slot`. Viewport numbers are not ranks (the second player's view can be viewport 3):
    /// the rank is the player's place among the local players; 0 without one.
    pub fn player_of_slot(&self, slot: usize) -> usize {
        self.local_controller_of_slot(slot).and_then(|pc| self.coop.players.iter().position(|&p| p == pc)).filter(|&r| r < PLAYERS).unwrap_or(0)
    }

    /// Player 1's aim assist with the gamepad: User.ini's [OpenXIII.Gamepad] AutoAim
    /// (aim_device.rs), on by default.
    pub fn gamepad_aim_assist(&mut self) -> bool {
        self.aim_pad_value() != 0
    }

    /// Sets player 1's aim assist with the gamepad (User.ini, written now) and, while the
    /// gamepad is the device in use, the controller's iAutoAimMode.
    pub fn set_gamepad_aim_assist(&mut self, on: bool, pc: Option<crate::value::ObjId>) {
        let Some(c) = self.config.as_mut() else { return };
        c.set_string(crate::aim_device::PAD_SECTION, crate::aim_device::PAD_KEY, if on { "1" } else { "0" }, "User");
        c.flush_file(false, Some("User"));
        // Lockstep co-op: this machine's player's values, through their next command.
        if self.lockstep.is_some() {
            if self.aim_device.pad_active {
                self.lockstep_local_aim(i32::from(on), on);
            }
            return;
        }
        if let Some(pc) = pc.filter(|&o| self.aim_device.pad_active && self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "PlayerController")) {
            self.set_prop(pc, "iAutoAimMode", crate::value::Value::Int(i32::from(on)));
            if self.is_a(pc, "XIIIPlayerController") {
                self.set_prop(pc, "bRotationAssist", crate::value::Value::Bool(on));
            }
        }
    }
}

/// Aim assist of the local players after the first (split screen and co-op). Their controllers
/// take the value of their own gamepad settings ([`PlayerPad::aim_assist`]); User.ini keeps
/// the first player's values only.
#[derive(Default, Debug)]
pub struct OtherAim {
    /// Controller and value last applied, by player rank (0, the first player, stays empty).
    applied: [Option<(crate::value::ObjId, bool)>; PLAYERS],
    /// Aim assist that a SaveConfig of such a controller wanted to keep: (player rank, on).
    saved: Vec<(usize, bool)>,
}

impl crate::Vm {
    /// One frame of the gamepad of local player `rank` (1…, not the first): their controller
    /// takes their aim assist (iAutoAimMode) and the gamepad's view turning assist
    /// (bRotationAssist), as the first player's does with a gamepad (aim_device.rs). Applied
    /// only when the controller or the setting changes: a script's own value in between (as
    /// Prock03_GenEffects' pause of the aim assist) is left alone.
    pub(crate) fn other_pad_aim(&mut self, pc: crate::value::ObjId, rank: usize, on: bool) {
        if rank == 0 || rank >= PLAYERS || self.other_aim.applied[rank] == Some((pc, on)) {
            return;
        }
        if !(self.objects.get(pc).is_some_and(Option::is_some) && self.is_a(pc, "PlayerController")) {
            return;
        }
        self.other_aim.applied[rank] = Some((pc, on));
        self.set_prop(pc, "iAutoAimMode", crate::value::Value::Int(i32::from(on)));
        if self.is_a(pc, "XIIIPlayerController") {
            self.set_prop(pc, "bRotationAssist", crate::value::Value::Bool(on));
        }
    }

    /// The other local players' aim assist is applied again at their next frame (the "User"
    /// configuration was read again: aim_device.rs, `aim_config_reloaded`).
    pub(crate) fn other_aim_forget(&mut self) {
        self.other_aim.applied = Default::default();
    }

    /// Rank of the local player (1…) whose controller is `obj`, when it is not the first's: its
    /// aim values are its gamepad's, not User.ini's (console.rs, `save_config`).
    pub(crate) fn other_pad_controller(&self, obj: crate::value::ObjId) -> Option<usize> {
        (1..PLAYERS).find(|&r| self.other_aim.applied[r].is_some_and(|(pc, _)| pc == obj))
    }

    /// SaveConfig of the controller of local player `rank` (1…): the aim assist it holds goes to
    /// that player's gamepad settings (taken by the game window, `take_other_aim_saved`).
    pub(crate) fn other_aim_saved(&mut self, rank: usize, on: bool) {
        if let Some(a) = self.other_aim.applied.get_mut(rank).and_then(Option::as_mut) {
            a.1 = on;
            // The view turning assist follows the saved setting, as `other_pad_aim` applies it.
            let pc = a.0;
            if self.objects.get(pc).is_some_and(Option::is_some) && self.is_a(pc, "XIIIPlayerController") {
                self.set_prop(pc, "bRotationAssist", crate::value::Value::Bool(on));
            }
        }
        self.other_aim.saved.retain(|s| s.0 != rank);
        self.other_aim.saved.push((rank, on));
    }

    /// Aim assist values of the players after the first saved by the game since the last call:
    /// (player rank, on), for the settings file.
    pub fn take_other_aim_saved(&mut self) -> Vec<(usize, bool)> {
        std::mem::take(&mut self.other_aim.saved)
    }

    /// Controller of the local player whose view is viewport `slot` (0 = the first).
    pub fn local_controller_of_slot(&self, slot: usize) -> Option<crate::value::ObjId> {
        self.coop.players.iter().copied().find(|&pc| self.objects.get(pc).is_some_and(Option::is_some) && self.viewport_slot_of(pc).map(usize::from) == Some(slot))
    }

    /// Several local players share the screen: each one's menu is drawn in their own view.
    pub fn split_menus(&self) -> bool {
        self.local_split() && self.coop_local_players() > 1
    }

    /// A page of the game's menus is open in the view of the player of viewport `slot` (their
    /// own GUIController). Without split screen, the local player's.
    pub fn gui_page_active_slot(&self, slot: usize) -> bool {
        if !self.split_menus() {
            return slot == 0 && self.gui_page_active();
        }
        let Some(pc) = self.local_controller_of_slot(slot) else { return false };
        let Some(viewport) = self.prop(pc, "Player").as_object().filter(|&v| self.objects.get(v).is_some_and(Option::is_some)) else { return false };
        let crate::value::Value::Array(list) = self.prop(viewport, "LocalInteractions") else { return false };
        list.iter().filter_map(crate::value::Value::as_object).any(|c| {
            self.objects.get(c).is_some_and(Option::is_some)
                && self.is_a(c, "GUIController")
                && self.prop(c, "ActivePage").as_object().is_some_and(|p| self.objects.get(p).is_some_and(Option::is_some))
        })
    }

    /// Split screen: the player of viewport `slot` has a menu of their own open (its menu stack),
    /// the rule each player's gamepad follows (coop.rs, `coop_menu_open`).
    pub fn slot_menu_open(&self, slot: usize) -> bool {
        // The failure menu in the whole window is every player's (coop_end.rs).
        self.local_controller_of_slot(slot).is_some_and(|pc| self.coop_menu_open(pc) || self.coop_end_menu_open(pc))
    }

    /// State of the gamepad read by viewport `slot` this frame, if any.
    pub fn slot_pad_state(&self, slot: usize) -> Option<crate::pad::PadState> {
        let handle = self.engine.client.viewport_pads.get(slot).copied().flatten()?;
        self.pads.states.get(handle).copied().flatten()
    }

    /// Gamepad handle read by viewport `slot` (split screen), if any.
    pub fn slot_pad_handle(&self, slot: usize) -> Option<usize> {
        self.engine.client.viewport_pads.get(slot).copied().flatten()
    }

    /// Rectangle (x, y, width, height) of the view of the player of viewport `slot` in a window
    /// of size `client`: the whole window without split screen.
    pub fn view_rect_of_slot(&self, slot: usize, client: (i32, i32)) -> [i32; 4] {
        match self.local_controller_of_slot(slot).filter(|_| self.split_menus()) {
            Some(pc) => self.viewport_rect_of(pc, client),
            None => [0, 0, client.0, client.1],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gamepad's fixed table before these settings (pad_keys.rs, `gamepad` in game), kept
    /// here as the reference: keys in table order, mouse [RT, LT], wheel on LB / RB press.
    fn former_table(b: u32, before: u32, lt: f32, rt: f32) -> Outputs {
        const B: [u32; 14] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
        let mut keys = Vec::new();
        for (mask, vk) in [(B[0], 0x20), (B[1], 0x43), (B[2], 0x52), (B[3], 0x45), (B[6] | B[10], 0x51), (B[7], 0x1B), (B[8], 0xA0), (B[12], 0x46), (B[13], 0x47)] {
            if b & mask != 0 && !keys.contains(&vk) {
                keys.push(vk);
            }
        }
        let mut wheel = Vec::new();
        for (mask, n) in [(B[4], -1), (B[5], 1)] {
            if b & mask != 0 && before & mask == 0 {
                wheel.push(n);
            }
        }
        Outputs { keys, mouse: [rt > 0.5, lt > 0.5], wheel }
    }

    struct Rng(u32);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0
        }
        fn trigger(&mut self) -> f32 {
            [0.0, 0.3, 0.5, 0.51, 1.0][(self.next() % 5) as usize]
        }
    }

    /// The default settings give exactly the former fixed table, frame after frame (varied
    /// buttons, triggers around the threshold, presses and releases).
    #[test]
    fn default_is_former_table() {
        let pad = PlayerPad::default();
        let mut r = Rng(0x5EED_1234);
        let (mut before, mut before_word) = (0u32, 0u16);
        for _ in 0..2000 {
            let b = match r.next() % 4 {
                0 => 0,
                1 => 1 << (r.next() % 14),
                _ => r.next() & 0x3FFF,
            };
            let (lt, rt) = (r.trigger(), r.trigger());
            let w = word(b, lt, rt);
            assert_eq!(pad.outputs(w, before_word), former_table(b, before, lt, rt), "buttons {b:#x} lt {lt} rt {rt}");
            (before, before_word) = (b, w);
        }
    }

    /// Negative check: a single remapped button gives other inputs than the former table.
    #[test]
    fn remapped_differs_from_former_table() {
        let mut pad = PlayerPad::default();
        assert!(pad.swap(Button::A, Button::B));
        let w = word(1, 0.0, 0.0);
        assert_ne!(pad.outputs(w, 0), former_table(1, 0, 0.0, 0.0));
        assert_eq!(pad.outputs(w, 0).keys, vec![0x43]);
        // Fire on a bumper: held, not a wheel notch.
        let mut pad = PlayerPad::default();
        assert!(pad.swap(Button::RightTrigger, Button::RightBumper));
        let out = pad.outputs(Button::RightBumper.bit(), 0);
        assert_eq!(out.mouse, [true, false]);
        assert!(out.wheel.is_empty());
        let out = pad.outputs(Button::RightTrigger.bit(), 0);
        assert_eq!(out.wheel, vec![1]);
        assert!(pad.outputs(Button::RightTrigger.bit(), Button::RightTrigger.bit()).wheel.is_empty());
    }

    #[test]
    fn swap_and_lock() {
        let mut r = Rng(42);
        for _ in 0..300 {
            let mut pad = PlayerPad::default();
            let (x, y) = (BUTTONS[(r.next() % 16) as usize], BUTTONS[(r.next() % 16) as usize]);
            let (ax, ay) = (pad.action(x), pad.action(y));
            let done = pad.swap(x, y);
            assert_eq!(done, !x.locked() && !y.locked());
            if done {
                assert_eq!((pad.action(x), pad.action(y)), (ay, ax));
            } else {
                assert_eq!(pad, PlayerPad::default());
            }
            assert_eq!(pad.action(Button::Start), Action::Menu);
        }
        let mut pad = PlayerPad::default();
        assert!(!pad.cycle(Button::Start, 1));
        for _ in 0..40 {
            pad.cycle(Button::A, 1);
            assert_ne!(pad.action(Button::A), Action::Menu);
        }
        let mut back = PlayerPad::default();
        back.cycle(Button::X, 1);
        back.cycle(Button::X, -1);
        assert_eq!(back, PlayerPad::default());
    }

    #[test]
    fn look_tune_default_is_neutral() {
        assert_eq!(PlayerPad::default().look_tune(), (1.0, false));
        let mut p = PlayerPad::default();
        for v in 0..=LOOK_SPEED_MAX {
            p.look_speed = v;
            assert!((p.look_tune().0 - (0.7 + f32::from(v) / 10.0)).abs() < 1e-6);
        }
    }

    /// A file written by a build that had stick swapping (SwapSticks) or a layout line: those keys
    /// are skipped, the other lines still apply, and they are not written back.
    #[test]
    fn swap_sticks_and_layout_lines_are_ignored() {
        let text = "[Player1]\r\nSwapSticks=True\r\nLayout=GoofyXIII\r\nVibration=False\r\n[Player2]\r\nSwapSticks=True\r\n";
        let c = GamepadConfig::parse(text);
        let mut want = GamepadConfig::default();
        want.players[0].vibration = false;
        assert_eq!(c, want);
        let out = c.to_text();
        assert!(!out.contains("SwapSticks") && !out.contains("Layout"));
        // Negative control: the Vibration line between them is read.
        assert_ne!(c, GamepadConfig::default());
    }

    /// Random settings survive the file; garbage leaves the defaults.
    #[test]
    fn file_round_trip() {
        let mut r = Rng(7);
        for _ in 0..200 {
            let mut c = GamepadConfig::default();
            for (i, p) in c.players.iter_mut().enumerate() {
                for b in BUTTONS {
                    if !b.locked() {
                        let list: Vec<Action> = ACTIONS.into_iter().filter(|&a| a != Action::Menu).collect();
                        p.actions[b.index()] = list[(r.next() as usize) % list.len()];
                    }
                }
                p.vibration = r.next() & 1 == 0;
                p.aim_assist = if i == 0 { true } else { r.next() & 1 == 0 };
                p.invert_look = r.next() & 1 == 0;
                p.look_speed = (r.next() % 11) as u8;
            }
            assert_eq!(GamepadConfig::parse(&c.to_text()), c);
        }
        let junk = "[Player1]\nA=Teleport\nStart=Jump\nLookSpeed=99\nVibration=maybe\n[Player9]\nA=Duck\n[Other]\nB=Jump\n";
        let mut want = GamepadConfig::default();
        want.players[0].look_speed = LOOK_SPEED_MAX;
        assert_eq!(GamepadConfig::parse(junk), want);
        assert_eq!(GamepadConfig::parse(""), GamepadConfig::default());
    }
}

