//! Cheats of the test builds (not in the original), behind the `cheats` feature: god mode,
//! infinite ammo, speed ×1/×2/×3 and console aim assist. Android sets them through the
//! Edit panel (touch_controls.rs), the PC through keys F5, F6, F7 and F11 (free in XIII:
//! DefUser.ini binds nothing there). Without the feature, `ACTIVE` is false: no
//! key is taken and nothing is applied, the game is the original's. The effects on the game
//! are in the VM (openxiii-vm cheats.rs, and cheat_speed in the walking physics). The aim
//! assist setting of every build (Android's Edit panel) is in aim_assist.rs.

use std::time::Instant;
use winit::keyboard::KeyCode;

/// True in the "cheats" build (`cheats` feature), false in the normal build.
pub(crate) const ACTIVE: bool = cfg!(feature = "cheats");

/// Display duration of a cheat's message (seconds).
const MESSAGE_S: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cheat {
    God,
    Ammo,
    Speed,
    Aim,
}

/// Cheat bound to a PC key: F5 god mode, F6 infinite ammo, F7 speed, F11 aim assist.
/// None in the normal build (the key goes to the game as in the original).
pub(crate) fn of_key(code: KeyCode) -> Option<Cheat> {
    if !ACTIVE {
        return None;
    }
    match code {
        KeyCode::F5 => Some(Cheat::God),
        KeyCode::F6 => Some(Cheat::Ammo),
        KeyCode::F7 => Some(Cheat::Speed),
        KeyCode::F11 => Some(Cheat::Aim),
        _ => None,
    }
}

/// The keys the game window keeps for itself as cheats, with what each one does. A build
/// without the cheats keeps none: every key goes to the game.
pub(crate) fn tool_keys(cheats: bool) -> Vec<(&'static str, &'static str)> {
    let mut keys = Vec::new();
    if cheats {
        keys.extend([("F5", "god mode"), ("F6", "infinite ammo"), ("F7", "speed x1 / x2 / x3"), ("F11", "aim assist")]);
    }
    keys
}

/// The keys this build keeps for itself (tool_keys of its own features).
pub(crate) fn this_build_tool_keys() -> Vec<(&'static str, &'static str)> {
    tool_keys(ACTIVE)
}

/// Wanted cheats, applied to the player every frame (each map has a new pawn).
#[derive(Default)]
pub(crate) struct Cheats {
    /// Invincibility: the controller's bGodMode.
    pub god: bool,
    /// Infinite ammo: magazine and reserve refilled every frame.
    pub ammo: bool,
    /// Wanted movement speed (1 = normal), applied by Vm::cheat_speed.
    pub speed_value: Option<f32>,
    /// Last message displayed (PC) and its time.
    pub message: Option<(String, Instant)>,
}

impl Cheats {
    /// PC key: toggles the cheat (speed ×1 → ×2 → ×3 → ×1) and returns the message displayed.
    /// Aim assist is set on the game (App::set_aim).
    pub fn toggle(&mut self, t: Cheat) -> String {
        let yes = |b: bool| if b { "on" } else { "off" };
        match t {
            Cheat::God => {
                self.god = !self.god;
                format!("God mode: {}", yes(self.god))
            }
            Cheat::Ammo => {
                self.ammo = !self.ammo;
                format!("Infinite ammo: {}", yes(self.ammo))
            }
            Cheat::Speed => {
                let m = self.speed_value.unwrap_or(1.0) as u32 % 3 + 1;
                self.speed_value = Some(m as f32);
                format!("Speed: x{m}")
            }
            Cheat::Aim => String::new(),
        }
    }
}

impl super::App {
    /// PC cheat key: toggle, then on-screen message.
    pub(crate) fn toggle_cheat(&mut self, t: Cheat) {
        if !ACTIVE {
            return;
        }
        let message = if t == Cheat::Aim {
            let on = !self.game.as_ref().and_then(|g| g.controller.map(|pc| g.vm.propr(pc, "iAutoAimMode").as_int() != 0)).unwrap_or(false);
            self.set_aim(on);
            format!("Aim assist: {}", if on { "on" } else { "off" })
        } else {
            self.cheats.toggle(t)
        };
        println!("cheat: {message}");
        self.cheats.message = Some((message, Instant::now()));
    }

    /// Console aim assist, like GlobalSetProperty: every PlayerController and
    /// their defaults, then User.ini (the console's SET command is refused without cheats).
    pub(crate) fn set_aim(&mut self, on: bool) {
        if !ACTIVE {
            return;
        }
        let Some(g) = &mut self.game else { return };
        let vm = &mut g.vm;
        let pcs: Vec<usize> = (0..vm.objects.len()).filter(|&o| vm.objects[o].is_some() && vm.is_a(o, "PlayerController")).collect();
        for o in pcs {
            vm.set_prop(o, "iAutoAimMode", openxiii_vm::value::Value::Int(i32::from(on)));
            if vm.is_a(o, "XIIIPlayerController") {
                vm.set_prop(o, "bRotationAssist", openxiii_vm::value::Value::Bool(on));
            }
        }
        // The value of the device in use (the VM's aim_device.rs): the gamepad's own while it is
        // the last device used, otherwise the keyboard / touch one.
        let (section, key) = if self.aim_on_pad && self.aim_pad_used {
            (openxiii_vm::aim_device::PAD_SECTION, openxiii_vm::aim_device::PAD_KEY)
        } else {
            ("Engine.PlayerController", "iAutoAimMode")
        };
        if let Some(c) = vm.config.as_mut() {
            c.set_string(section, key, if on { "1" } else { "0" }, "User");
            c.set_string("XIII.XIIIPlayerController", "bRotationAssist", if on { "True" } else { "False" }, "User");
            c.flush();
        }
    }

    /// Wanted cheats applied to the player every frame: god mode on the controller, walking speed
    /// multiplied in the physics (Vm::cheat_speed; GroundSpeed is reset by the script
    /// every frame while armed), ammo full.
    pub(crate) fn apply_cheats(&mut self) {
        if !ACTIVE {
            return;
        }
        let Some(g) = &mut self.game else { return };
        let Some(pc) = g.controller else { return };
        let vm = &mut g.vm;
        vm.cheat_god(pc, self.cheats.god);
        let Some(pawn_value) = vm.propr(pc, "Pawn").as_object().filter(|&p| vm.objects.get(p).is_some_and(|o| o.is_some())) else { return };
        let mult = self.cheats.speed_value.unwrap_or(1.0);
        vm.cheat_speed = (mult != 1.0).then_some((pawn_value, mult));
        if self.cheats.ammo {
            vm.cheat_full_ammo(pawn_value);
        }
    }
}

/// Message of the last cheat, top left, for MESSAGE_S seconds.
pub(crate) fn draw_message(t: &Cheats, g: &mut super::game::Game, quads: &mut Vec<openxiii_vm::canvas::HudDraw>) {
    if let Some((m, when_value)) = &t.message {
        if when_value.elapsed().as_secs_f32() < MESSAGE_S {
            quads.extend(g.vm.free_text("XIIIFonts.PoliceF16", 16.0, 16.0, m, [255, 255, 255, 255]).0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheats build: F5, F6, F7 and F11 are cheats; normal build: no key.
    #[test]
    fn touches_cheats() {
        let touches = [KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F11];
        if cfg!(feature = "cheats") {
            assert_eq!(touches.map(of_key), [Some(Cheat::God), Some(Cheat::Ammo), Some(Cheat::Speed), Some(Cheat::Aim)]);
        } else {
            assert_eq!(touches.map(of_key), [None; 4]);
        }
        assert_eq!(of_key(KeyCode::F9), None);
        assert_eq!(of_key(KeyCode::KeyW), None);
    }

    /// Toggle: god mode and ammo on/off, speed ×1 → ×2 → ×3 → ×1.
    #[test]
    fn toggle_cheats() {
        let mut t = Cheats::default();
        assert_eq!(t.toggle(Cheat::God), "God mode: on");
        assert!(t.god);
        assert_eq!(t.toggle(Cheat::God), "God mode: off");
        assert_eq!(t.toggle(Cheat::Ammo), "Infinite ammo: on");
        assert!(t.ammo);
        assert_eq!(t.toggle(Cheat::Speed), "Speed: x2");
        assert_eq!(t.speed_value, Some(2.0));
        assert_eq!(t.toggle(Cheat::Speed), "Speed: x3");
        assert_eq!(t.toggle(Cheat::Speed), "Speed: x1");
        assert_eq!(t.speed_value, Some(1.0));
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    /// A build without the cheats keeps no key. Negative control: with them, the list finds
    /// every cheat key.
    #[test]
    fn a_release_build_has_no_cheat_key() {
        assert!(tool_keys(false).is_empty());
        let all: Vec<&str> = tool_keys(true).iter().map(|k| k.0).collect();
        for key in ["F5", "F6", "F7", "F11"] {
            assert!(all.contains(&key), "{key} found with the cheats on");
        }
        // This build's own list agrees with its features.
        assert_eq!(this_build_tool_keys().is_empty(), !ACTIVE);
        assert_eq!(of_key(KeyCode::F5).is_some(), ACTIVE);
    }
}
