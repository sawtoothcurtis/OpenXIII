//! Gamepad seen as keyboard and mouse, on PC and Android.
//!
//! PC XIII has no gamepad bindings (UseJoystick=False and no Joy key in DefUser.ini): the
//! DIJOYSTATE given to the VM (pad.rs, set_pad_state) moves nothing in the menus or in game. The
//! combined gamepad (`merged()` of pad.rs / pad_android.rs) is therefore turned into Windows keys
//! and mouse input, sent along the same paths as main.rs's KeyboardInput, MouseInput and
//! MouseMotion: the game keeps its DefUser.ini key bindings.
//!
//! Combined gamepad format: axes lx, ly, rx, ry (−1…1, Y down), triggers lt, rt (0…1); buttons
//! as bits (`B_*`).

// Android reads the gamepad from touch_controls.rs: the PC-only parts are unused there.
#![cfg_attr(target_os = "android", allow(dead_code))]

use openxiii_vm::gamepad_config;

pub const B_A: u32 = 1;
pub const B_B: u32 = 1 << 1;
pub const B_X: u32 = 1 << 2;
pub const B_Y: u32 = 1 << 3;
pub const B_LB: u32 = 1 << 4;
pub const B_RB: u32 = 1 << 5;
pub const B_BACK: u32 = 1 << 6;
pub const B_START: u32 = 1 << 7;
pub const B_THUMB_L: u32 = 1 << 8;
pub const B_THUMB_R: u32 = 1 << 9;
pub const B_HIGH: u32 = 1 << 10;
pub const B_LOW: u32 = 1 << 11;
pub const B_LEFT: u32 = 1 << 12;
pub const B_RIGHT: u32 = 1 << 13;

/// Movement stick threshold for pressing W/S/A/D.
pub const THRESHOLD: f32 = 0.35;

/// Keys, mouse buttons and remainders held by the gamepad (and, on Android, by the touch
/// controls) between two frames.
#[derive(Default)]
pub struct PadKeys {
    /// Windows keys and mouse buttons held pressed by the gamepad, gamepad buttons in the
    /// previous frame.
    pad_touches: Vec<u8>,
    pad_mouse: [bool; 2],
    pad_buttons: u16,
    /// Buttons held when "Select level" had them: ignored until released (B closing
    /// the list must not also send Escape to the menu below).
    pad_ignores: u32,
    /// The gamepad was read as a menu's last frame: the buttons still held when the game
    /// takes it back (B or Start that closed the pause menu, A on "Resume") are ignored until
    /// released (otherwise B, the crouch button in game, made the player crouch).
    was_menu: bool,
    /// W, S, A, D keys pressed by the movement stick.
    wsad: [bool; 4],
    /// Fractional remainder of mouse movements.
    rest: (f32, f32),
    /// GUIController of the last menu opened (faster lookup).
    pub(crate) menu: Option<usize>,
    /// Right stick in game this frame (x right, y down), for the game's joystick path.
    look: (f32, f32),
    /// Improved build: menu directions held by the stick, and their repeats (the VM's
    /// menu_repeat.rs, shared with the other players' gamepads).
    menu_navigation: crate::MenuNavigation,
}

impl super::App {
    /// Windows key as a physical keyboard key (main.rs's KeyboardInput):
    /// keyboard state for all its codes, then WM_KEYDOWN / WM_KEYUP with the generic code and
    /// WM_CHAR for the characters TranslateMessage would produce.
    pub(crate) fn key_windows(&mut self, vk: u8, pressed: bool) {
        // A co-op meeting's waiting sign: the gamepad's back (B, Start) and Android's back give
        // it up, as Escape does (lockstep_play.rs).
        if vk == 0x1B && pressed && self.lockstep_meet_cancel() {
            return;
        }
        let Some(g) = &mut self.game else { return };
        let vks: &[u8] = match vk {
            0xA0 | 0xA1 => &[vk, 0x10],
            0xA2 | 0xA3 => &[vk, 0x11],
            _ => &[vk],
        };
        for &v in vks {
            g.vm.win_input.set_key(v, pressed);
        }
        let generic = *vks.last().unwrap_or(&vk);
        let lparam = 1 | (u32::from(!pressed) << 30) | (u32::from(!pressed) << 31);
        g.vm.win_message(if pressed { 0x100 } else { 0x101 }, u32::from(generic), lparam);
        if pressed {
            let ch = match vk {
                0x0D | 0x1B | 0x20 | 0x08 | 0x30..=0x39 => Some(vk),
                0x41..=0x5A => Some(vk + 32),
                _ => None,
            };
            if let Some(ch) = ch {
                g.vm.win_message(0x102, u32::from(ch), lparam);
            }
        }
    }

    pub(crate) fn button_mouse(&mut self, b: u8, pressed: bool) {
        let Some(g) = &mut self.game else { return };
        g.vm.win_input.active = true;
        g.vm.win_input.push_mouse(12 + u32::from(b), if pressed { 0x80 } else { 0 });
        let msg = [0x201, 0x204, 0x207][usize::from(b)] + u32::from(!pressed);
        let (x, y) = self.cursor_value;
        g.vm.win_message(msg, 0, (y << 16) | x);
    }

    pub(crate) fn mouse_relative(&mut self, dx: f32, dy: f32) {
        let Some(g) = &mut self.game else { return };
        let (rx, ry) = (self.pad_keys.rest.0 + dx, self.pad_keys.rest.1 + dy);
        let (ix, iy) = (rx.trunc() as i32, ry.trunc() as i32);
        self.pad_keys.rest = (rx - ix as f32, ry - iy as f32);
        g.vm.win_input.active = true;
        if ix != 0 {
            g.vm.win_input.push_mouse(0, ix as u32);
        }
        if iy != 0 {
            g.vm.win_input.push_mouse(4, iy as u32);
        }
    }

    /// PC: one frame of the combined gamepad (pad.rs `merged`), `dt` seconds. Menu (open
    /// GUIController page) or mouse not captured: menu keys; otherwise game keys and look.
    #[cfg(not(target_os = "android"))]
    pub(crate) fn pad_keys_frame(&mut self, pad: Option<([f32; 6], u32)>) {
        // Gamepad at rest (or absent) and nothing held: nothing to send, menus are not looked up.
        let idle = pad.is_none_or(|(a, b)| b == 0 && a.iter().all(|&v| v == 0.0));
        let k = &self.pad_keys;
        if idle && k.pad_touches.is_empty() && k.pad_mouse == [false; 2] && k.wsad == [false; 4] && k.pad_buttons == 0 && k.pad_ignores == 0 {
            self.pad_keys.look = (0.0, 0.0);
            self.pad_keys.was_menu = false;
            if let Some(g) = &mut self.game {
                g.vm.look_stick = None;
                g.vm.move_stick = None;
            }
            return;
        }
        let menu = self.game.as_ref().and_then(|g| g.vm.cursor_menu(self.pad_keys.menu));
        self.pad_keys.menu = menu.map(|m| m.0);
        let capture = self.game.as_ref().is_some_and(|g| g.vm.win_input.capture != 0);
        // Split screen: the first player's own menu only (another player's menu also opens a
        // menu stack and releases the mouse, which left the first player with movement only).
        let split = self.game.as_ref().map(|g| (g.vm.split_menus(), g.vm.slot_menu_open(0)));
        let in_menu = first_player_in_menu(split, menu.is_some(), capture);
        self.gamepad(pad, in_menu);
        self.pad_wsad(pad, in_menu, (0.0, 0.0));
        self.look_stick_frame(self.aim_on_pad, in_menu, (0.0, 0.0));
    }

    /// Movement: the touch stick (Android) → W, S, A, D; the gamepad's left stick in game → the
    /// VM's `move_stick`: the keys' range as far as it is pushed (player_input.rs, analog_move),
    /// so a partial push walks, as with the consoles' stick.
    pub(crate) fn pad_wsad(&mut self, pad: Option<([f32; 6], u32)>, menu: bool, stick: (f32, f32)) {
        let (x, y) = stick;
        let pad_stick = pad.filter(|_| !menu).map(|(a, _)| (a[0], a[1]));
        let (move_stick, (px, py)) = menu_stick_route(pad_stick);
        if let Some(g) = &mut self.game {
            g.vm.move_stick = move_stick;
        }
        let wanted = [y < -THRESHOLD || py < -THRESHOLD, y > THRESHOLD || py > THRESHOLD, x < -THRESHOLD || px < -THRESHOLD, x > THRESHOLD || px > THRESHOLD];
        for (i, vk) in [0x57u8, 0x53, 0x41, 0x44].into_iter().enumerate() {
            if wanted[i] != self.pad_keys.wsad[i] {
                self.pad_keys.wsad[i] = wanted[i];
                self.key_windows(vk, wanted[i]);
            }
        }
    }

    /// Gamepad seen as keyboard and mouse. In game: left stick W/S/A/D (`pad_wsad`), right stick
    /// look, RT fire, LT alt fire, A jump, B crouch, X reload, Y grab, LB / RB previous / next
    /// weapon, d-pad left / right item, d-pad up medkit, Back medkit, Start menu, left stick
    /// click walk. In menus: d-pad or left stick = arrows, A Enter, B and Start Escape. An open
    /// "Select level" (and, in the improved build, the Mods page) reads the gamepad itself
    /// (main.rs): the menu behind gets nothing, and buttons held when it closes stay ignored until
    /// released.
    pub(crate) fn gamepad(&mut self, pad: Option<([f32; 6], u32)>, menu: bool) {
        // A co-op meeting's waiting sign (its menus closed): the gamepad as in the menus, its B
        // and Start Escape (`key_windows` gives the meeting up).
        let menu = menu || self.lockstep_meeting.is_some();
        let (axes, raw_value) = pad.unwrap_or(([0.0; 6], 0));
        let page_reads_pad = self.page_reads_gamepad();
        if page_reads_pad {
            self.pad_keys.pad_ignores = raw_value;
        }
        // Improved build: buttons held when a menu gives the gamepad back are ignored until
        // released (the faithful build keeps reading them at once).
        let was_menu = menu_handoff(std::mem::replace(&mut self.pad_keys.was_menu, menu));
        self.pad_keys.pad_ignores = ignored_after_menu(was_menu, menu, raw_value, self.pad_keys.pad_ignores);
        let b = raw_value & !self.pad_keys.pad_ignores;
        let axes = if page_reads_pad { [0.0; 6] } else { axes };
        let mut touches: Vec<u8> = Vec::new();
        let mut mouse = [false; 2];
        // Previous frame's button word (gamepad_config::word: buttons, then the triggers).
        let before_value = self.pad_keys.pad_buttons;
        let mut pad_word = (b & 0x3FFF) as u16;
        let mut look = (0.0, 0.0);
        let mut repeats = [false; 4];
        if menu {
            let (x, y) = (axes[0], axes[1]);
            let stick = self.pad_keys.menu_navigation.directions(x, y);
            let held = [b & B_HIGH != 0 || stick[0], b & B_LOW != 0 || stick[1], b & B_LEFT != 0 || stick[2], b & B_RIGHT != 0 || stick[3]];
            repeats = self.pad_keys.menu_navigation.repeats(held, crate::MenuNavigation::clock());
            for (cond, vk) in [
                (b & B_HIGH != 0 || stick[0], 0x26),
                (b & B_LOW != 0 || stick[1], 0x28),
                (b & B_LEFT != 0 || stick[2], 0x25),
                (b & B_RIGHT != 0 || stick[3], 0x27),
                (b & B_A != 0, 0x0D),
                (b & (B_B | B_START) != 0, 0x1B),
            ] {
                if cond {
                    touches.push(vk);
                }
            }
        } else {
            self.pad_keys.menu_navigation.released(crate::MenuNavigation::clock());
            // Buttons → keys, mouse buttons and wheel notches of the first player's gamepad
            // settings (the VM's gamepad_config.rs; its default is the former fixed table: A
            // Space, B C, X R, Y E, Back and d-pad up Q, Start Escape, left stick click LShift,
            // d-pad left / right F / G, RT / LT left / right mouse button, LB / RB one wheel notch
            // down / up on press).
            let word = gamepad_config::word(b, axes[4], axes[5]);
            let out = self.gamepad_config.players[0].outputs(word, before_value);
            touches = out.keys;
            mouse = out.mouse;
            for notches in out.wheel {
                if let Some(g) = &mut self.game {
                    g.vm.win_input.active = true;
                    g.vm.win_input.push_mouse(8, (notches * 120) as u32);
                }
            }
            pad_word = word;
            // Right stick: look through the game's joystick path (look_stick_frame), as on the
            // consoles.
            look = (axes[2], axes[3]);
        }
        self.pad_keys.look = look;
        self.pad_keys.pad_buttons = pad_word;
        let held_touches = std::mem::take(&mut self.pad_keys.pad_touches);
        for &vk in held_touches.iter().filter(|vk| !touches.contains(vk)) {
            self.key_windows(vk, false);
        }
        for &vk in touches.iter().filter(|vk| !held_touches.contains(vk)) {
            self.key_windows(vk, true);
        }
        // A held direction's repeat: the arrow key released and pressed again (a repeated
        // WM_KEYDOWN of a key already down is not taken by every menu).
        self.repeat_menu_keys(repeats, &touches, &held_touches);
        self.pad_keys.pad_touches = touches;
        for i in 0..2 {
            if mouse[i] != self.pad_keys.pad_mouse[i] {
                self.pad_keys.pad_mouse[i] = mouse[i];
                self.button_mouse(i as u8, mouse[i]);
            }
        }
    }
}

impl super::App {
    /// Look stick given to the game's joystick path (the VM's `look_stick`) for this frame: the
    /// gamepad's right stick, or `touch` (Android's look stick) when it is pushed further; None in
    /// menus and with keyboard and mouse (`active` false), where nothing is read.
    pub(crate) fn look_stick_frame(&mut self, active: bool, menu: bool, touch: (f32, f32)) {
        let pad = self.pad_keys.look;
        let stick = if touch.0.hypot(touch.1) > pad.0.hypot(pad.1) { touch } else { pad };
        if let Some(g) = &mut self.game {
            g.vm.look_stick = (active && !menu).then_some(stick);
        }
    }
}

/// The gamepad is being used this frame: a button pressed or an axis beyond 0.3.
pub fn pad_in_use(pad: Option<([f32; 6], u32)>) -> bool {
    pad.is_some_and(|(axes, b)| b != 0 || axes.iter().any(|a| a.abs() > 0.3))
}

/// The aim assist is the gamepad's only (keyboard and mouse never have it): the improved
/// build's rule on PC. Android's touch controls keep their own switch; the faithful build keeps
/// the original PC game's keyboard option.
pub fn aim_pad_only(improved: bool, android: bool) -> bool {
    improved && !android
}

/// Device whose aim assist applies after this frame (the VM's aim_device.rs), from
/// (`on_pad`, `pad_used`): the gamepad was the last device used, a gamepad has been used at
/// least once. `other`: a key, the mouse or (Android) a finger was used this frame; `pad`: the
/// combined gamepad. The gamepad wins over another device used in the same frame.
pub fn aim_device_next((on_pad, pad_used): (bool, bool), other: bool, pad: Option<([f32; 6], u32)>) -> (bool, bool) {
    if pad_in_use(pad) {
        (true, true)
    } else {
        (on_pad && !other, pad_used)
    }
}

impl super::App {
    /// Aim assist of the device in use (aim_device_next), applied to the local controller once
    /// a gamepad has been used.
    pub(crate) fn aim_device_frame(&mut self, other: bool, pad: Option<([f32; 6], u32)>) {
        let before = (self.aim_on_pad, self.aim_pad_used);
        (self.aim_on_pad, self.aim_pad_used) = aim_device_next(before, other, pad);
        // Improved build on PC: the aim assist is the gamepad's only, so the keyboard and mouse
        // are followed from the first frame (the VM's set_aim_device_pad_only takes any help
        // away from them), not only once a gamepad was used.
        if aim_pad_only_in_this_build(cfg!(target_os = "android")) {
            if let Some(g) = &mut self.game {
                g.vm.set_aim_device_pad_only(self.aim_on_pad && self.aim_pad_used, g.controller);
            }
            if (self.aim_on_pad, self.aim_pad_used) != before {
                println!("Aim assist: {}", if self.aim_on_pad { "gamepad" } else { "none (keyboard and mouse)" });
            }
            return;
        }
        if self.aim_pad_used {
            if let Some(g) = &mut self.game {
                g.vm.set_aim_device(self.aim_on_pad, g.controller);
                // Device change: written to the log with the player's aim assist now in force (on
                // screen only on Android's cheats build: on PC the message got in the way).
                if (self.aim_on_pad, self.aim_pad_used) != before {
                    // (Lockstep co-op: this machine's player's, whose new value comes with their
                    // next command.)
                    let v = match g.vm.lockstep_on() {
                        true => Some(if self.aim_on_pad { g.vm.aim_pad_value() } else { g.vm.aim_keyboard_value() }),
                        false => g.controller.map(|pc| g.vm.propr(pc, "iAutoAimMode").as_int()),
                    };
                    let device = if self.aim_on_pad { "gamepad" } else if cfg!(target_os = "android") { "touch" } else { "keyboard and mouse" };
                    let state = match v {
                        Some(0) => "off",
                        Some(_) => "on",
                        None => "no player",
                    };
                    let message = format!("Aim assist ({device}): {state}");
                    println!("{message}");
                    if aim_message_on_screen(super::cheats::ACTIVE, cfg!(target_os = "android")) {
                        self.cheats.message = Some((message, std::time::Instant::now()));
                    }
                }
            }
        }
    }
}

/// Where the gamepad's left stick goes: the improved build gives it to the VM's `move_stick`
/// (analog walk) and presses no key; the faithful build keeps W / S / A / D past the threshold,
/// with no move stick. Returns (move stick, stick for the keys).
fn stick_route(improved: bool, stick: Option<(f32, f32)>) -> (Option<(f32, f32)>, (f32, f32)) {
    if improved {
        (stick, (0.0, 0.0))
    } else {
        (None, stick.unwrap_or((0.0, 0.0)))
    }
}

#[cfg(test)]
mod stick_route_tests {
    /// Improved: the stick walks (move stick), no key; faithful: the keys, no move stick, as
    /// before. Negative control: the two builds differ.
    #[test]
    fn analog_walk_in_the_improved_build_only() {
        let s = Some((0.3, -0.8));
        assert_eq!(super::stick_route(true, s), (s, (0.0, 0.0)));
        assert_eq!(super::stick_route(false, s), (None, (0.3, -0.8)));
        assert_eq!(super::stick_route(false, None), (None, (0.0, 0.0)));
        assert_ne!(super::stick_route(true, s), super::stick_route(false, s));
    }
}

/// The aim assist message of a device change is shown on screen (cheats build, Android only).
fn aim_message_on_screen(cheats: bool, android: bool) -> bool {
    cheats && android
}

#[cfg(test)]
mod aim_message_tests {
    /// PC: never on screen, cheats or not; Android's cheats build keeps it. Negative control: the
    /// former rule (cheats alone) showed it on PC.
    #[test]
    fn no_aim_assist_message_on_pc() {
        for cheats in [false, true] {
            assert!(!super::aim_message_on_screen(cheats, false));
        }
        assert!(super::aim_message_on_screen(true, true));
        let former = |cheats: bool, _android: bool| cheats;
        assert_ne!(former(true, false), super::aim_message_on_screen(true, false));
    }
}

#[cfg(test)]
mod aim_device_tests {
    use super::aim_device_next;

    /// Varied frames against a model of the rule: the device is the last one used (the
    /// gamepad when both are used in the same frame), and nothing applies before a gamepad.
    #[test]
    fn aim_device_rule() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut checked = 0;
        for _ in 0..50 {
            let mut state = (false, false);
            let mut last_pad: Option<bool> = None;
            for _ in 0..40 {
                let r = next();
                let other = r & 3 == 0;
                let pad = match (r >> 2) % 5 {
                    0 => None,
                    1 => Some(([0.0; 6], 0)),
                    2 => Some(([0.0, 0.0, 0.0, 0.0, 0.0, 0.2], 0)),
                    3 => Some(([0.0, 0.0, -0.8, 0.0, 0.0, 0.0], 0)),
                    _ => Some(([0.0; 6], 1 << ((r >> 8) % 14))),
                };
                let pad_used = matches!((r >> 2) % 5, 3 | 4);
                if pad_used {
                    last_pad = Some(true);
                } else if other && last_pad.is_some() {
                    last_pad = Some(false);
                }
                state = aim_device_next(state, other, pad);
                assert_eq!(state, (last_pad == Some(true), last_pad.is_some()));
                checked += 1;
            }
        }
        assert!(checked >= 100);
    }

    /// Negative check: a rule where a finger or key in the same frame beats the gamepad, or
    /// where an idle gamepad counts as used, gives a different device.
    #[test]
    fn aim_device_rule_negative() {
        let wrong_order = |s: (bool, bool), other: bool, pad| if other { (false, s.1) } else { aim_device_next(s, other, pad) };
        let busy = Some(([0.0; 6], 1));
        assert_ne!(aim_device_next((false, false), true, busy), wrong_order((false, false), true, busy));
        let idle = Some(([0.0, 0.0, 0.0, 0.0, 0.0, 0.2], 0));
        assert_eq!(aim_device_next((false, false), false, idle), (false, false));
    }
}

/// The first player's gamepad drives a menu (menu keys, no look) rather than the game. `split`:
/// (several local players share the screen, the first player's own menu is open), None without
/// a game; then `any_menu` (a menu stack is open) or the mouse not `captured`. In split screen only
/// the first player's own menu counts: another player's menu is theirs alone.
pub(crate) fn first_player_in_menu(split: Option<(bool, bool)>, any_menu: bool, captured: bool) -> bool {
    match split {
        Some((true, own)) => own,
        _ => any_menu || !captured,
    }
}

/// Gamepad buttons kept ignored this frame: the ones `ignores` still holds (released ones are
/// let go), plus every button held when a menu (`was_menu`) gives the gamepad back to the game.
pub(crate) fn ignored_after_menu(was_menu: bool, menu: bool, raw: u32, ignores: u32) -> u32 {
    let ignores = if was_menu && !menu { ignores | raw } else { ignores };
    ignores & raw
}

#[cfg(test)]
mod first_player_menu_tests {
    use super::*;

    /// Split screen: player 2's menu open (a menu stack open, the mouse released) leaves player 1
    /// in game (fire, look, use reach their pawn); player 1's own menu takes their gamepad.
    /// Alone, any menu or a released mouse is the menu, as before. Negative control: the former
    /// rule (any menu, the mouse) put player 1 in the menu when player 2 opened theirs.
    #[test]
    fn another_players_menu_leaves_the_first_player_in_game() {
        for captured in [true, false] {
            assert!(!first_player_in_menu(Some((true, false)), true, captured), "player 2's menu, mouse captured {captured}");
            assert!(first_player_in_menu(Some((true, true)), true, captured), "player 1's own menu");
            assert!(first_player_in_menu(Some((false, false)), true, captured), "alone, a menu");
        }
        assert!(first_player_in_menu(Some((false, false)), false, false), "alone, mouse released");
        assert!(!first_player_in_menu(Some((false, false)), false, true), "alone, in game");
        assert!(first_player_in_menu(None, false, false));
        let former = |any_menu: bool, captured: bool| any_menu || !captured;
        assert_ne!(former(true, false), first_player_in_menu(Some((true, false)), true, false));
    }
}

#[cfg(test)]
mod menu_release_tests {
    use super::*;

    /// B closes the pause menu: still held on the first game frame, it is not the crouch
    /// button until it is released; pressed again afterwards, it is.
    #[test]
    fn button_closing_a_menu_waits_for_its_release() {
        let mut ignores = 0;
        ignores = ignored_after_menu(false, true, B_B, ignores);
        assert_eq!(ignores, 0, "in the menu, B is the menu's");
        ignores = ignored_after_menu(true, false, B_B, ignores);
        assert_eq!(ignores, B_B, "the game takes the gamepad back with B held: ignored");
        ignores = ignored_after_menu(false, false, B_B, ignores);
        assert_eq!(ignores, B_B, "still held: still ignored");
        ignores = ignored_after_menu(false, false, 0, ignores);
        assert_eq!(ignores, 0, "released");
        ignores = ignored_after_menu(false, false, B_B, ignores);
        assert_eq!(ignores, 0, "pressed again in game: the crouch button");
        // Negative control: a button pressed in game, no menu involved, is never ignored.
        assert_eq!(ignored_after_menu(false, false, B_A | B_B, 0), 0);
    }
}

register_viewer_layer!("pad_input");

#[cfg(test)]
mod aim_pad_only_tests {
    /// The keyboard and mouse never have the aim assist in the improved build on PC; Android's
    /// touch controls and the faithful build keep theirs.
    #[test]
    fn pad_only_in_the_improved_pc_build() {
        assert!(super::aim_pad_only(true, false));
        assert!(!super::aim_pad_only(false, false));
        assert!(!super::aim_pad_only(true, true));
    }
}
