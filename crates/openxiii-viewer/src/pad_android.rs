//! Android: gamepads (Bluetooth, USB…) read by the Java activity (GameActivityXiii, gamepad source
//! events) and passed to the VM as pad.rs does on PC with gilrs: same
//! DIJOYSTATE state as an XInput gamepad seen by DirectInput (lX / lY left stick, Y down, lZ
//! combined triggers, lRx / lRy right stick; buttons 1 A … 10 right stick).
//!
//! A gamepad connected or removed during play changes `count`: main.rs's loop
//! then redoes NbJoy (set_pad_count), so the gamepad is seen at any time, not only at
//! launch.

use std::ffi::c_void;
use std::sync::Mutex;

use openxiii_vm::pad::PadState;

/// State of a gamepad received from Java: axes lx, ly, rx, ry (−1…1, Y down), triggers lt, rt
/// (0…1) and buttons (bits of `B_*`).
#[derive(Clone, Copy, Default)]
struct Gamepad {
    axes: [f32; 6],
    buttons_list: u32,
}

const B_A: u32 = 1;
const B_B: u32 = 1 << 1;
const B_X: u32 = 1 << 2;
const B_Y: u32 = 1 << 3;
const B_LB: u32 = 1 << 4;
const B_RB: u32 = 1 << 5;
const B_HIGH: u32 = 1 << 10;
const B_LOW: u32 = 1 << 11;

/// Gamepad slots (at most 4, like EnumJoysticksCallback), None if none.
static GAMEPADS: Mutex<[Option<Gamepad>; 4]> = Mutex::new([None; 4]);

// JNI signature: (JNIEnv*, jclass, slot, connected, lx, ly, rx, ry, lt, rt, buttons).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_openxiii_NativeInput_pad(
    _env: *mut c_void,
    _cls: *mut c_void,
    slot: i32,
    connected: u8,
    lx: f32,
    ly: f32,
    rx: f32,
    ry: f32,
    lt: f32,
    rt: f32,
    buttons: i32,
) {
    let Ok(mut m) = GAMEPADS.lock() else { return };
    let Some(place) = usize::try_from(slot).ok().and_then(|i| m.get_mut(i)) else { return };
    *place = (connected != 0).then_some(Gamepad { axes: [lx, ly, rx, ry, lt, rt], buttons_list: buttons as u32 });
}

/// All gamepads combined (the most deflected axis, buttons of either), None without a
/// gamepad.
pub fn merged() -> Option<([f32; 6], u32)> {
    merged_where(|_| true)
}

/// `merged` of the gamepads whose slot `keep` accepts: touch_controls.rs turns the first
/// player's into keys and a mouse, like the touch controls.
pub fn merged_where(keep: impl Fn(usize) -> bool) -> Option<([f32; 6], u32)> {
    let state_value = GAMEPADS.lock().map(|m| *m).ok()?;
    let mut out: Option<([f32; 6], u32)> = None;
    for m in state_value.into_iter().enumerate().filter(|(i, _)| keep(*i)).filter_map(|(_, m)| m) {
        let (axes, buttons_list) = out.get_or_insert(([0.0; 6], 0));
        for (a, v) in axes.iter_mut().zip(m.axes) {
            if v.abs() > a.abs() {
                *a = v;
            }
        }
        *buttons_list |= m.buttons_list;
    }
    out
}

/// The first player has a gamepad (all of them outside split screen; in split screen the
/// other players' gamepads leave the touch controls to the first player), read by Java to
/// hide or show the touch controls (Gamepads.java).
static FIRST_PLAYER_PAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_first_player_gamepad(on: bool) {
    FIRST_PLAYER_PAD.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_firstPlayerGamepad(_env: *mut c_void, _cls: *mut c_void) -> u8 {
    u8::from(FIRST_PLAYER_PAD.load(std::sync::atomic::Ordering::Relaxed))
}

/// Strength of the two motors (large, small; 0 to 255) of each slot, read by Java (Gamepads.java).
static VIBRATIONS: Mutex<[[u8; 2]; 4]> = Mutex::new([[0; 2]; 4]);

/// Vibration of slot `slot`: large motor × 256 + small motor (0…255 each).
#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_vibration(_env: *mut c_void, _cls: *mut c_void, slot: i32) -> i32 {
    let Ok(v) = VIBRATIONS.lock() else { return 0 };
    usize::try_from(slot).ok().and_then(|i| v.get(i)).map_or(0, |f| i32::from(f[0]) << 8 | i32::from(f[1]))
}

/// Gamepad vibration wanted: yes, unless OXIII_RUMBLE=0 (like the PC's pad.rs).
pub fn vibrations_wanted() -> bool {
    openxiii_package::env::var("OXIII_RUMBLE").map_or(true, |v| v.trim() != "0")
}

/// The game gets its vibration manager in the improved build with a gamepad and vibrations
/// wanted (same rule as the PC's pad.rs).
pub fn vibration_manager_wanted(improved: bool, gamepad: bool, wanted: bool) -> bool {
    improved && gamepad && wanted
}

pub struct Pads {
    /// Local co-op: the d-pad is added to the buttons given to the game (11 to 14), as on PC.
    pub dpad_buttons: bool,
}

impl Pads {
    pub fn new() -> Option<Self> {
        Some(Pads { dpad_buttons: false })
    }

    /// Identity of each gamepad, in order: its slot (Gamepads.java keeps a gamepad in its slot
    /// while it is connected).
    pub fn ids(&self) -> Vec<u64> {
        (0..self.count() as u64).collect()
    }

    /// All gamepads combined (`merged`), as the PC's pad.rs.
    pub fn merged(&self) -> Option<([f32; 6], u32)> {
        merged()
    }

    /// `merged` of the gamepads whose slot (handle) `keep` accepts, as the PC's pad.rs.
    pub fn merged_where(&self, keep: impl Fn(usize) -> bool) -> Option<([f32; 6], u32)> {
        merged_where(keep)
    }

    fn state_value() -> [Option<Gamepad>; 4] {
        GAMEPADS.lock().map(|m| *m).unwrap_or_default()
    }

    /// Motor strength of each force-feedback client, for Java (client k goes to slot k).
    pub fn vibrate(&mut self, forces: &[[f32; 2]; 4]) {
        if let Ok(mut v) = VIBRATIONS.lock() {
            *v = forces.map(|f| f.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8));
        }
    }

    /// Number of gamepads (NbJoy): up to the last one connected.
    pub fn count(&self) -> usize {
        Self::state_value().iter().rposition(Option::is_some).map_or(0, |i| i + 1)
    }

    /// Level selection buttons (choice_level.rs), all gamepads.
    pub fn menu(&self) -> crate::choice_level::ButtonsMenu {
        let mut b = crate::choice_level::ButtonsMenu::default();
        for m in Self::state_value().into_iter().flatten() {
            let y = m.axes[1];
            b.high |= m.buttons_list & B_HIGH != 0 || y < -0.5;
            b.low_value |= m.buttons_list & B_LOW != 0 || y > 0.5;
            b.validate |= m.buttons_list & B_A != 0;
            b.return_value |= m.buttons_list & B_B != 0;
            b.open_flag |= m.buttons_list & B_Y != 0;
            b.weapons |= m.buttons_list & B_X != 0;
            b.previous |= m.buttons_list & B_LB != 0;
            b.next_one |= m.buttons_list & B_RB != 0;
        }
        b
    }

    /// State of each gamepad up to `count` (an empty slot is an idle gamepad).
    pub fn poll(&mut self) -> Vec<PadState> {
        let state_value = Self::state_value();
        let n = self.count();
        let dpad = self.dpad_buttons;
        state_value[..n]
            .iter()
            .map(|m| {
                let m = m.unwrap_or_default();
                let di = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i32;
                let a = m.axes;
                let axes = [di(a[0]), di(a[1]), di(a[4] - a[5]), di(a[2]), di(a[3]), 0];
                let mut buttons = [false; 16];
                for (i, b) in buttons.iter_mut().enumerate().take(if dpad { 14 } else { 10 }) {
                    *b = m.buttons_list & 1 << i != 0;
                }
                PadState { axes, buttons }
            })
            .collect()
    }
}
