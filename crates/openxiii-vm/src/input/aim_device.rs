//! Aim assist per input device (PC; not in the original PC version, which has no gamepad
//! support): on by default with a gamepad, as on the consoles, off by default with keyboard
//! and mouse, as in the original PC version.
//!
//! The keyboard and mouse value stays where the game keeps it ([Engine.PlayerController]
//! iAutoAimMode of User.ini); the gamepad value is [OpenXIII.Gamepad] AutoAim (absent: 1). The
//! viewer reports each frame the device in use (`set_aim_device`), only once a gamepad has been
//! used: before that, nothing changes. SaveConfig of a controller while the gamepad is in use
//! (the menu's aim assist option) writes the gamepad value and leaves the keyboard one
//! (console.rs, `save_config`).

use crate::value::{ObjId, Value};
use crate::Vm;

/// Section and key of the gamepad value in User.ini.
pub const PAD_SECTION: &str = "OpenXIII.Gamepad";
pub const PAD_KEY: &str = "AutoAim";
/// Section and key of bRotationAssist in User.ini (config of XIIIPlayerController).
pub const ROTATION_SECTION: &str = "XIII.XIIIPlayerController";
pub const ROTATION_KEY: &str = "bRotationAssist";

#[derive(Default)]
pub struct AimDevice {
    /// The gamepad is the device in use.
    pub pad_active: bool,
    /// Player controller the value was last applied to.
    pub pc: Option<ObjId>,
}

impl Vm {
    /// Device in use this frame (`pad`: the gamepad) and the local player's controller: on a
    /// switch to the gamepad (or a new controller while it is in use), the controller takes the
    /// gamepad value; back to keyboard and mouse, the keyboard value.
    pub fn set_aim_device(&mut self, pad: bool, pc: Option<ObjId>) {
        // Lockstep co-op: the device is this machine's player's, whatever controller the window
        // follows (the session's first player's on every machine).
        let pc = if self.lockstep.is_some() { self.lockstep_local_controller().or(pc) } else { pc };
        let switched = pad != self.aim_device.pad_active;
        let new_pc = pc != self.aim_device.pc;
        self.aim_device = AimDevice { pad_active: pad, pc };
        let Some(pc) = pc.filter(|&o| self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "PlayerController")) else { return };
        if !(switched || (pad && new_pc)) {
            return;
        }
        // Lockstep co-op: the values go into this machine's player's next command.
        if self.lockstep.is_some() {
            let v = if pad { self.aim_pad_value() } else { self.aim_keyboard_value() };
            let r = if pad { v != 0 } else { self.rotation_keyboard_value() };
            self.lockstep_local_aim(v, r);
            return;
        }
        if pad {
            self.apply_pad_aim(pc);
            return;
        }
        let v = self.aim_keyboard_value();
        self.set_prop(pc, "iAutoAimMode", Value::Int(v));
        if self.is_a(pc, "XIIIPlayerController") {
            let r = self.rotation_keyboard_value();
            self.set_prop(pc, "bRotationAssist", Value::Bool(r));
        }
    }

    /// Improved build on PC (OpenXIII's rule, not the original's, whose PC version has no
    /// gamepad and lets the keyboard have the aim assist through its options): the aim assist is
    /// the gamepad's only. Called every frame with the device in use: with the gamepad, as
    /// `set_aim_device`; with the keyboard and mouse (also before any gamepad was used, and
    /// with one plugged in but not played), the local player's controller never keeps either help
    /// (iAutoAimMode, bRotationAssist), whatever User.ini, a profile, a menu option or a script
    /// gave it. Lockstep co-op: through this machine's player's next command.
    pub fn set_aim_device_pad_only(&mut self, pad: bool, pc: Option<ObjId>) {
        if pad {
            self.set_aim_device(pad, pc);
            return;
        }
        let pc = if self.lockstep.is_some() { self.lockstep_local_controller().or(pc) } else { pc };
        self.aim_device = AimDevice { pad_active: false, pc };
        let Some(pc) = pc.filter(|&o| self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "PlayerController")) else { return };
        let rotation = self.is_a(pc, "XIIIPlayerController") && self.propr(pc, "bRotationAssist").as_bool();
        if self.propr(pc, "iAutoAimMode").as_int() == 0 && !rotation {
            return;
        }
        if self.lockstep_local_aim(0, false) {
            return;
        }
        self.set_prop(pc, "iAutoAimMode", Value::Int(0));
        if self.is_a(pc, "XIIIPlayerController") {
            self.set_prop(pc, "bRotationAssist", Value::Bool(false));
        }
    }

    /// The gamepad's aim assist on controller `pc`: iAutoAimMode and the view turning towards
    /// the held target (XIIIPlayerController.UpdateRotation's bRotationAssist, which the
    /// consoles' code is written for), both following the gamepad setting: off, neither helps.
    fn apply_pad_aim(&mut self, pc: ObjId) {
        let v = self.aim_pad_value();
        self.set_prop(pc, "iAutoAimMode", Value::Int(v));
        if self.is_a(pc, "XIIIPlayerController") {
            self.set_prop(pc, "bRotationAssist", Value::Bool(v != 0));
        }
    }

    /// The "User" configuration was read again (a profile used, as after the Controls page's
    /// Reset, or a ResetConfig): the controllers took the keyboard values of User.ini, so while
    /// the gamepad is in use its aim assist is applied again at once (before, it came back only
    /// at the next switch of device), and the other local players' at their next frame.
    pub(crate) fn aim_config_reloaded(&mut self) {
        self.other_aim_forget();
        if !self.aim_device.pad_active {
            return;
        }
        // Lockstep co-op: the reload runs in a step on every machine, but only this machine knows
        // its player plays with the gamepad: the gamepad's values go into that player's next
        // command (the same on every machine), not straight onto a controller.
        if self.lockstep.is_some() {
            let v = self.aim_pad_value();
            self.lockstep_local_aim(v, v != 0);
            return;
        }
        let Some(pc) = self.aim_device.pc.filter(|&o| self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "PlayerController")) else { return };
        self.apply_pad_aim(pc);
    }

    /// Gamepad aim assist: [OpenXIII.Gamepad] AutoAim of User.ini, 1 if absent.
    pub fn aim_pad_value(&mut self) -> i32 {
        self.aim_ini(PAD_SECTION, PAD_KEY).unwrap_or(1)
    }

    /// Keyboard and mouse aim assist: [Engine.PlayerController] iAutoAimMode of User.ini, 0 if absent.
    pub fn aim_keyboard_value(&mut self) -> i32 {
        self.aim_ini("Engine.PlayerController", "iAutoAimMode").unwrap_or(0)
    }

    fn aim_ini(&mut self, section: &str, key: &str) -> Option<i32> {
        self.config.as_mut()?.get_string(section, key, "User")?.trim().parse().ok()
    }

    /// Touch controls on Android: the console versions' view turning towards the held target
    /// (XIIIPlayerController's bRotationAssist) on exactly when the aim assist (iAutoAimMode) is,
    /// as the gamepad's setting drives both.
    pub fn rotation_assist_follows_aim(&mut self, pc: Option<ObjId>) {
        let pc = self.aim_local_controller(pc);
        let Some(pc) = pc.filter(|&o| self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "XIIIPlayerController")) else { return };
        let mode = self.propr(pc, "iAutoAimMode").as_int();
        let on = mode != 0;
        if self.propr(pc, "bRotationAssist").as_bool() != on {
            self.aim_local_set(pc, mode, on);
        }
    }

    /// The view turning towards the held target (XIIIPlayerController's bRotationAssist) on.
    pub fn rotation_assist_on(&mut self, pc: Option<ObjId>) {
        let pc = self.aim_local_controller(pc);
        let Some(pc) = pc.filter(|&o| self.objects.get(o).is_some_and(Option::is_some) && self.is_a(o, "XIIIPlayerController")) else { return };
        if !self.propr(pc, "bRotationAssist").as_bool() {
            let mode = self.propr(pc, "iAutoAimMode").as_int();
            self.aim_local_set(pc, mode, true);
        }
    }

    /// The display layer's aim assist switch (Android's Edit panel): every local player's
    /// controller takes `on` (iAutoAimMode), and with `rotation` their bRotationAssist too.
    /// Lockstep co-op: only this machine's player, through their next command.
    pub fn set_local_auto_aim(&mut self, on: bool, rotation: Option<bool>) {
        if self.lockstep.is_some() {
            let Some(pc) = self.lockstep_local_controller() else { return };
            let r = rotation.unwrap_or_else(|| self.propr(pc, "bRotationAssist").as_bool());
            self.lockstep_local_aim(i32::from(on), r);
            return;
        }
        let pcs: Vec<ObjId> = (0..self.objects.len()).filter(|&o| self.objects[o].is_some() && self.is_a(o, "PlayerController")).collect();
        for o in pcs {
            self.set_prop(o, "iAutoAimMode", Value::Int(i32::from(on)));
            if let Some(r) = rotation.filter(|_| self.is_a(o, "XIIIPlayerController")) {
                self.set_prop(o, "bRotationAssist", Value::Bool(r));
            }
        }
    }

    /// The controller of this machine's player: `pc`, or in a lockstep co-op session the
    /// machine's own player's (the window follows the session's first player on every machine).
    fn aim_local_controller(&self, pc: Option<ObjId>) -> Option<ObjId> {
        if self.lockstep.is_some() {
            return self.lockstep_local_controller();
        }
        pc
    }

    /// This machine's player's aim values (iAutoAimMode, bRotationAssist) on controller `pc`;
    /// lockstep co-op: in their next command, as every machine must apply them at the same step.
    fn aim_local_set(&mut self, pc: ObjId, mode: i32, rotation: bool) {
        if self.lockstep_local_aim(mode, rotation) {
            return;
        }
        self.set_prop(pc, "iAutoAimMode", Value::Int(mode));
        self.set_prop(pc, "bRotationAssist", Value::Bool(rotation));
    }

    /// Keyboard and mouse view turning assist: [XIII.XIIIPlayerController] bRotationAssist of
    /// User.ini (False in the PC version's DefUser.ini), false if absent.
    pub fn rotation_keyboard_value(&mut self) -> bool {
        self.config
            .as_mut()
            .and_then(|c| c.get_string(ROTATION_SECTION, ROTATION_KEY, "User"))
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true") || v.trim() == "1")
    }

    /// SaveConfig of `obj` is a controller's while the gamepad is in use (console.rs).
    /// Lockstep co-op: only this machine's player's controller (the other player's device is
    /// their own machine's business).
    pub(crate) fn aim_pad_swap(&self, obj: ObjId) -> bool {
        self.aim_device.pad_active && self.is_a(obj, "PlayerController") && (self.lockstep.is_none() || self.lockstep_local_controller() == Some(obj))
    }
}

