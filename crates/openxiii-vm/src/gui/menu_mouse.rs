//! Improved build only (Graphics: Enhanced, PC): the menus follow the device last used.
//!
//! Entering a page: GUIPage.InitComponent and GUIController.CloseMenu call the page's
//! FocusFirst(None, true), which focuses the first control, whose XIIIGUIBaseButton.MouseEnter
//! (OnActivate) plays MnCurseur. With the mouse, the control under the cursor then took the
//! focus with a second tick, after a frame showing the first control lit. When the menu's last
//! input was the mouse (a move, a button, the wheel), that first focus is silent and, before the
//! page is drawn, the control under the cursor takes it with one tick (or no control keeps it
//! when the cursor is over none, and the tick the first focus would have made is played then, or
//! the help bar side's own tick when the cursor is on one, so that entering or leaving a page
//! still ticks once, as in the original); the first arrow, Tab or Enter on a page left without focus
//! gives the focus to the first control, as on entering the page with the keys. With the keys or
//! a gamepad last used, nothing changes.
//!
//! Clicks: in the original a left click on a button is key 0x01 for the page, and only Enter's
//! and Backspace's key events play XIIIWindow.InternalOnKeyEvent's MnValid / MnAnnul (on their
//! release, for the page then active: any of its help bar's validating entries, or Return /
//! Cancel / Back / Delete). Here a click that validates a control plays, at the button's release,
//! Enter's MnValid under Enter's condition. A click on a page's Back or Cancel button stays
//! silent, as Escape, as does a click that only gives the focus and a click on the help bar
//! (help_bar_click.rs: Enter's own sound on its right side, Escape's silence on its left).

use crate::value::{ObjId, Value};
use crate::Vm;

/// The menu's last input device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Device {
    /// Keyboard or gamepad (the original's behaviour).
    #[default]
    Keys,
    Mouse,
}

/// The device a GUIController key event comes from: None for events that tell nothing
/// (releases, holds, a still axis).
pub(crate) fn device_of(key: u8, action: u8, delta: f32) -> Option<Device> {
    match key {
        // IK_LeftMouse, IK_RightMouse, IK_MiddleMouse, wheel.
        0x01 | 0x02 | 0x04 | 0xec | 0xed if action == 1 => Some(Device::Mouse),
        // IK_MouseX, IK_MouseY: a real move.
        0xe4 | 0xe5 if delta != 0.0 => Some(Device::Mouse),
        0x01 | 0x02 | 0x04 | 0xe4 | 0xe5 | 0xe6 | 0xec | 0xed => None,
        // Gamepad sticks (IK_JoyX / IK_JoyY): only a push beyond half.
        0xe0 | 0xe1 => (delta.abs() > 0.5).then_some(Device::Keys),
        _ if action == 1 => Some(Device::Keys),
        _ => None,
    }
}

/// Keys that bring the focus back to the first control of a page left without focus: the arrows,
/// Tab, Enter and the gamepad's directions and A (before GUIController's mapping).
pub(crate) fn refocus_key(key: u8) -> bool {
    matches!(key, 0x25..=0x28 | 0x09 | 0x0d | 0xd4..=0xd7 | 0xca | 0xe0 | 0xe1)
}

/// A click's sound to play at the button's release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClickSound {
    /// Enter's MnValid.
    Enter,
}

/// The sound a click pressed on a control will play: none on a page's Back or Cancel button
/// (silent, as Escape), Enter's otherwise.
pub(crate) fn click_sound(back_button: bool) -> Option<ClickSound> {
    (!back_button).then_some(ClickSound::Enter)
}

/// The menus' mouse state.
#[derive(Clone, Debug, Default)]
pub(crate) struct MenuMouse {
    pub(crate) last: Device,
    /// A page entered with the mouse: its focus to settle before it is drawn.
    pub(crate) entered: Option<ObjId>,
    /// FocusFirst of a page entered with the mouse in progress: its MnCurseur is not played.
    pub(crate) silent: bool,
    /// The page left without focus by the mouse.
    pub(crate) unfocused: Option<ObjId>,
    /// A click pressed on a control: the sound for its release.
    pub(crate) click: Option<ClickSound>,
    /// The first focus's MnCurseur of a page entered with the mouse was not played.
    pub(crate) swallowed: bool,
    /// That tick to play once the help bar of the page is known (help_bar_click.rs), unless the
    /// cursor stands on one of its sides, whose own tick then stands for it.
    pub(crate) page_tick: bool,
}

/// The menus' mouse rules: improved build, Graphics: Enhanced, not on Android.
pub(crate) use crate::improved::input::menu_mouse_enabled as enabled;

const ENTER_FLAGS: [&str; 8] = ["bShowSEL", "bShowRUN", "bShowNXT", "bShowACC", "bShowSCH", "bShowEDT", "bShowCHO", "bShowSAV"];

/// Whether XIIIWindow.InternalOnKeyEvent plays its sound for `sound` on a page whose flags are
/// read by `flag` (and whose video plays or not).
pub(crate) fn key_plays(sound: ClickSound, playing_video: bool, flag: impl Fn(&str) -> bool) -> bool {
    !playing_video
        && match sound {
            ClickSound::Enter => ENTER_FLAGS.iter().any(|f| flag(f)),
        }
}

impl Vm {
    fn menu_mouse_local(&self, ctrl: ObjId) -> bool {
        enabled(self.engine.client.session_graphics) && self.gui_local() == Some(ctrl) && !self.gui_without_mouse(ctrl)
    }

    /// GUIController.NativeKeyEvent, before anything: the device noted; a key that brings the
    /// focus back on a page left without it. Some(taken) when the event is used here.
    pub(crate) fn menu_mouse_key(&mut self, ctrl: ObjId, key: u8, action: u8, delta: f32) -> Option<bool> {
        if !self.menu_mouse_local(ctrl) {
            return None;
        }
        // The help bar's key sent for a click stays the mouse's.
        let from_bar = action == 1 && self.improved.help_bar.injected == Some(key);
        if from_bar {
            self.improved.help_bar.injected = None;
        } else if let Some(d) = device_of(key, action, delta) {
            self.improved.menu_mouse.last = d;
        }
        let page = self.propr(ctrl, "ActivePage").as_object();
        if self.improved.menu_mouse.unfocused.is_some() && self.improved.menu_mouse.unfocused != page {
            self.improved.menu_mouse.unfocused = None;
        }
        if action == 1 && refocus_key(key) && page.is_some() && self.improved.menu_mouse.unfocused == page && page.is_some_and(|p| self.propr(p, "FocusedControl").as_object().is_none()) {
            self.improved.menu_mouse.unfocused = None;
            let p = page?;
            let _ = self.call_named(p, "FocusFirst", vec![Value::Object(None), Value::Bool(true)]);
            return Some(true);
        }
        None
    }

    /// Before a script call (Vm::call): a page's FocusFirst(None, true) with the mouse last used
    /// is followed (menu_mouse_focus_first_end after it).
    pub(crate) fn menu_mouse_focus_first(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        let r = self.menu_mouse_focus_first_in(f, this, args);
        r
    }

    fn menu_mouse_focus_first_in(&mut self, f: crate::types::FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if self.improved.menu_mouse.silent || self.improved.menu_mouse.last != Device::Mouse || self.functions[f].name != "FocusFirst" || !enabled(self.engine.client.session_graphics) {
            return false;
        }
        if args.first().and_then(|a| a.as_ref()).and_then(|a| a.as_object()).is_some() || !self.is_a(this, "GUIPage") {
            return false;
        }
        let ctrl = self.propr(this, "Controller").as_object();
        if ctrl.is_none() || ctrl != self.gui_local() || ctrl.is_some_and(|c| self.gui_without_mouse(c)) {
            return false;
        }

        if self.improved.character_screen.on {
            return false;
        }
        self.improved.menu_mouse.silent = true;
        self.improved.menu_mouse.swallowed = false;
        self.improved.menu_mouse.page_tick = false;
        true
    }

    pub(crate) fn menu_mouse_focus_first_end(&mut self, this: ObjId) {
        self.improved.menu_mouse.silent = false;
        self.improved.menu_mouse.entered = Some(this);
    }

    /// PlayMenu of a sound (natives.rs): false when it is the first focus's tick of a page entered
    /// with the mouse (noted, for menu_mouse_settle).
    pub(crate) fn menu_mouse_plays(&mut self, sound: &str) -> bool {
        let swallow = self.improved.menu_mouse.silent && sound.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("MnCurseur"));
        self.improved.menu_mouse.swallowed |= swallow;
        !swallow
    }

    /// GUIController.NativePreRender, once the cursor's place is known: the focus of a page entered
    /// with the mouse goes to the control under the cursor (one tick), or to none.
    pub(crate) fn menu_mouse_settle(&mut self, ctrl: ObjId) {
        let Some(page) = self.improved.menu_mouse.entered else { return };
        if !self.menu_mouse_local(ctrl) {
            return;
        }
        self.improved.menu_mouse.entered = None;
        let swallowed = std::mem::take(&mut self.improved.menu_mouse.swallowed);
        if self.propr(ctrl, "ActivePage").as_object() != Some(page) || self.improved.menu_mouse.last != Device::Mouse {
            return;
        }

        if self.menu_overlay_open() {
            return;
        }
        let (mx, my) = (self.propr(ctrl, "MouseX").as_int() as f32, self.propr(ctrl, "MouseY").as_int() as f32);
        // The page itself under the cursor: no control.
        let under = self.gui_controller_under_cursor(ctrl, mx, my).filter(|&u| u != page);
        // The page's focused control (GUIMultiComponent.FocusedControl; the controller's may be
        // none).
        let focused = self.propr(page, "FocusedControl").as_object().or_else(|| self.propr(ctrl, "FocusedControl").as_object());
        match under {
            // The first control itself: its tick, silent at the focus, played once.
            Some(u) if Some(u) == focused => {
                self.play_menu_sound(crate::menu_sounds::MenuSound::Tick);
                self.set_prop(ctrl, "ActiveControl", Value::Object(Some(u)));
            }
            // Another control: the hover's own path (watched, focus, MouseEnter's tick).
            Some(_) => self.gui_look_under_cursor(ctrl, 0.0, 0.0),
            None => {
                // The control alone loses its focus (GUIComponent.LoseFocus would blur the page
                // too, whose keys then went nowhere): blurry, no focused control.
                if let Some(f) = focused {
                    self.gui_event_pub(f, "MenuStateChange", vec![Value::Byte(0)]);
                    self.set_prop(page, "FocusedControl", Value::Object(None));
                    self.set_prop(ctrl, "FocusedControl", Value::Object(None));
                }
                self.set_prop(ctrl, "ActiveControl", Value::Object(None));
                self.improved.menu_mouse.unfocused = Some(page);
                // The page changed with the cursor over no control: the first focus's tick, once
                // (help_bar_hover, which knows whether the cursor is on a side of the help bar).
                self.improved.menu_mouse.page_tick = swallowed;
            }
        }
    }

    /// GUIController.NativeKeyEvent, left mouse button: on the press, the sound its release will
    /// play when it reaches a control (not a click that only gives the focus, nor a raw-key page).
    pub(crate) fn menu_mouse_click_begin(&mut self, ctrl: ObjId, key: u8, action: u8) {
        if key != 0x01 || !self.menu_mouse_local(ctrl) {
            return;
        }
        if action != 1 {
            return;
        }
        self.improved.menu_mouse.click = None;
        let Some(page) = self.propr(ctrl, "ActivePage").as_object() else { return };
        if self.propr(page, "bNeedRawKey").as_bool() || !self.is_a(page, "XIIIWindow") {
            return;
        }
        let active = self.propr(ctrl, "ActiveControl").as_object();
        if active.is_none() || active != self.propr(ctrl, "FocusedControl").as_object() {
            return;
        }
        let back = ["BackButton", "CancelButton"].iter().any(|n| self.propr(page, n).as_object().is_some() && self.propr(page, n).as_object() == active);
        self.improved.menu_mouse.click = click_sound(back);
    }

    /// After the left mouse button's release reached the menus: the click's sound, under the
    /// key's own condition, for the page then active.
    pub(crate) fn menu_mouse_click_end(&mut self, ctrl: ObjId, key: u8, action: u8) {
        if key != 0x01 || action != 3 {
            return;
        }
        let Some(sound) = self.improved.menu_mouse.click.take() else { return };
        if !self.menu_mouse_local(ctrl) {
            return;
        }
        let Some(page) = self.propr(ctrl, "ActivePage").as_object() else { return };
        if !self.is_a(page, "XIIIWindow") {
            return;
        }
        let playing = self.propr(page, "bPlayingVideo").as_bool();
        if key_plays(sound, playing, |f| self.propr(page, f).as_bool()) {
            self.play_menu_sound(match sound {
                ClickSound::Enter => crate::menu_sounds::MenuSound::Enter,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_follows_the_input() {
        assert_eq!(device_of(0x01, 1, 0.0), Some(Device::Mouse));
        assert_eq!(device_of(0xe4, 4, 3.0), Some(Device::Mouse));
        assert_eq!(device_of(0xec, 1, 0.0), Some(Device::Mouse));
        // A still mouse axis, a release: nothing said.
        assert_eq!(device_of(0xe4, 4, 0.0), None);
        assert_eq!(device_of(0x01, 3, 0.0), None);
        // Keyboard and gamepad presses.
        assert_eq!(device_of(0x28, 1, 0.0), Some(Device::Keys));
        assert_eq!(device_of(0x0d, 1, 0.0), Some(Device::Keys));
        assert_eq!(device_of(0xd4, 1, 0.0), Some(Device::Keys));
        assert_eq!(device_of(0xe1, 4, 0.9), Some(Device::Keys));
        // Negative control: a stick at rest, a key release say nothing.
        assert_eq!(device_of(0xe1, 4, 0.1), None);
        assert_eq!(device_of(0x28, 3, 0.0), None);
    }

    #[test]
    fn refocus_keys_are_navigation_keys() {
        for k in [0x25, 0x26, 0x27, 0x28, 0x09, 0x0d, 0xd4, 0xd7] {
            assert!(refocus_key(k), "{k:x}");
        }
        for k in [0x1b, 0x08, 0x01, 0x41, 0x20] {
            assert!(!refocus_key(k), "{k:x}");
        }
    }

    #[test]
    fn click_sounds_mirror_the_keys() {
        let flags = |on: &'static [&'static str]| move |f: &str| on.contains(&f);
        assert!(key_plays(ClickSound::Enter, false, flags(&["bShowSEL", "bShowBCK"])));
        assert!(key_plays(ClickSound::Enter, false, flags(&["bShowSAV"])));
        // Nothing during a video; nothing without a validating entry.
        assert!(!key_plays(ClickSound::Enter, true, flags(&["bShowSEL"])));
        assert!(!key_plays(ClickSound::Enter, false, flags(&["bShowBCK"])));
        assert!(!key_plays(ClickSound::Enter, false, flags(&[])));
    }

    /// A click on Back or Cancel is silent (as Escape); other clicks keep Enter's sound.
    #[test]
    fn back_clicks_are_silent() {
        assert_eq!(click_sound(true), None);
        // Negative control: a validating button still has its sound.
        assert_eq!(click_sound(false), Some(ClickSound::Enter));
    }

    /// Only the first focus's MnCurseur of a page entered with the mouse is silent.
    #[test]
    fn only_the_entry_tick_is_silent() {
        let mut vm = Vm::new();
        for s in ["XIIIsound.Interface.MnCurseur", "XIIIsound.Interface.MnValid", "XIIIsound.Interface__AmbianceMenu.AmbianceMenu__hMainPage"] {
            assert!(vm.menu_mouse_plays(s), "{s}");
        }
        assert!(!vm.improved.menu_mouse.swallowed);
        vm.improved.menu_mouse.silent = true;
        assert!(!vm.menu_mouse_plays("XIIIsound.Interface.MnCurseur"));
        assert!(vm.improved.menu_mouse.swallowed, "the swallowed tick is noted for the settle");
        // Negative control: the other sounds still play during that focus.
        assert!(vm.menu_mouse_plays("XIIIsound.Interface.MnValid"));
        assert!(vm.menu_mouse_plays("XIIIsound.Interface__AmbianceMenu.AmbianceMenu__hMainPage"));
    }

}
