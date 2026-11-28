//! Aim assist, a setting of every build (not a cheat): Android's Edit panel switch, like the
//! consoles' menu.

impl super::App {
    /// Android Edit panel's aim assist switch, like the consoles' menu
    /// (XIIIMenuAdvancedControlsWindow.AutoAimChanged): iAutoAimMode of every PlayerController,
    /// then the value of the device in use in User.ini (aim_device.rs); with the touch controls,
    /// bRotationAssist as well (it follows iAutoAimMode, touch_controls.rs).
    /// A setting in both builds (not a cheat).
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn set_auto_aim(&mut self, on: bool) {
        let pad = self.aim_on_pad && self.aim_pad_used;
        let Some(g) = &mut self.game else { return };
        let vm = &mut g.vm;
        // Every local player's controller (lockstep co-op: this machine's player only, in their
        // next command); touch: the view turning assist follows (touch_controls.rs).
        vm.set_local_auto_aim(on, (!pad).then_some(on));
        let (section, key) = if pad { (openxiii_vm::aim_device::PAD_SECTION, openxiii_vm::aim_device::PAD_KEY) } else { ("Engine.PlayerController", "iAutoAimMode") };
        if let Some(c) = vm.config.as_mut() {
            c.set_string(section, key, if on { "1" } else { "0" }, "User");
            // Touch: the view turning assist follows the switch (touch_controls.rs), kept with it.
            if !pad {
                c.set_string(openxiii_vm::aim_device::ROTATION_SECTION, openxiii_vm::aim_device::ROTATION_KEY, if on { "True" } else { "False" }, "User");
            }
            c.flush();
        }
        println!("Aim assist ({}): {} (Edit panel)", if pad { "gamepad" } else { "touch" }, if on { "on" } else { "off" });
    }
}
