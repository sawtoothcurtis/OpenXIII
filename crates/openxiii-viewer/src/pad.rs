//! PC gamepads read by gilrs and passed to the VM as WinDrv saw them (DirectInput,
//! the VM's pad.rs): enumeration (UWindowsClient::EnumJoysticksCallback,
//! at most 4), then the DIJOYSTATE state of each gamepad every frame.
//!
//! An XInput gamepad seen by DirectInput (HID compatibility driver): lX / lY left stick
//! (Y down), lZ combined triggers (left towards +, right towards −), lRx / lRy right stick
//! (Y down), lRz zero; buttons 1 A, 2 B, 3 X, 4 Y, 5 LB, 6 RB, 7 Back, 8 Start, 9 left
//! stick, 10 right stick; the d-pad is the POV hat, which XIII does not read.
//!
//! Hot plugging: the list is redone at each `poll` (gamepads still connected kept in their
//! order, new ones appended); main.rs's loop then redoes NbJoy (set_pad_count) when the count
//! changes, like on Android.

use gilrs::{Axis, Button, Gilrs};
use openxiii_vm::pad::PadState;

use crate::pad_keys::{B_A, B_B, B_BACK, B_HIGH, B_LB, B_LEFT, B_LOW, B_RB, B_RIGHT, B_START, B_THUMB_L, B_THUMB_R, B_X, B_Y};

/// At most 4 gamepads (EnumJoysticksCallback).
const MAX_PADS: usize = 4;
/// Stick dead zone of the combined gamepad (`merged`).
const DEAD_ZONE: f32 = 0.25;
/// Trigger dead zone of the combined gamepad.
const TRIGGER_DEAD_ZONE: f32 = 0.1;
/// Seconds between two attempts when gilrs could not start.
const RETRY_SECONDS: u64 = 2;

pub struct Pads {
    gilrs: Option<Gilrs>,
    /// Last failed gilrs start.
    last_try: std::time::Instant,
    /// Connected gamepads, in order (aJoyHandle).
    list: Vec<Pad>,
    /// Local co-op: the d-pad is added to the buttons given to the game (11 to 14), for the
    /// players whose gamepad the game reads itself. Otherwise the buttons are the original ten.
    pub dpad_buttons: bool,
}

struct Pad {
    id: gilrs::GamepadId,
    /// Vibration, created on first request (None: no force feedback).
    engines: Option<Engines>,
    /// No force feedback (creation refused once: no further attempts).
    without_vibrations: bool,
}

/// The two motors of a gamepad: one permanent effect per motor, whose strength (gain) is set.
/// Dropping it frees the effects.
struct Engines {
    big: gilrs::ff::Effect,
    small: gilrs::ff::Effect,
    force: [f32; 2],
    plays: bool,
}

/// Gamepad vibration wanted: yes, unless OXIII_RUMBLE=0. The scripts' bUseRumble option
/// (User.ini, [XIII.XIIIPlayerController], true by default) also turns it off.
/// The game gets its vibration manager (the consoles' UForceFeedbackManager): improved build only,
/// with a gamepad and vibrations wanted. The PC original never creates it (no ForceFeedbackManager:
/// RumbleFX does nothing), so the faithful build has none.
pub fn vibration_manager_wanted(improved: bool, gamepad: bool, wanted: bool) -> bool {
    improved && gamepad && wanted
}

pub fn vibrations_wanted() -> bool {
    openxiii_package::env::var("OXIII_RUMBLE").map_or(true, |v| v.trim() != "0")
}

impl Engines {
    fn new(gilrs: &mut Gilrs, id: gilrs::GamepadId) -> Option<Self> {
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
        if !gilrs.gamepad(id).is_ff_supported() {
            return None;
        }
        let permanent = |kind| BaseEffect {
            kind,
            scheduling: Replay { after: Ticks::from_ms(0), play_for: Ticks::from_ms(1000), with_delay: Ticks::from_ms(0) },
            envelope: Default::default(),
        };
        let mut effect_value = |kind| EffectBuilder::new().add_effect(permanent(kind)).gamepads(&[id]).gain(0.0).finish(gilrs).ok();
        let big = effect_value(BaseEffectType::Strong { magnitude: u16::MAX })?;
        let small = effect_value(BaseEffectType::Weak { magnitude: u16::MAX })?;
        eprintln!("gamepad: rumble ready");
        Some(Engines { big, small, force: [0.0; 2], plays: false })
    }

    fn set(&mut self, force: [f32; 2]) {
        if force == self.force {
            return;
        }
        self.force = force;
        let gains = self.big.set_gain(force[0]).and(self.small.set_gain(force[1]));
        let plays = force != [0.0; 2];
        let mut r = Ok(());
        if plays != self.plays {
            self.plays = plays;
            r = if plays { self.big.play().and(self.small.play()) } else { self.big.stop().and(self.small.stop()) };
        }
        let report = gains.is_err() || r.is_err();
        if report {
            eprintln!("vibrations {:.2} {:.2}: {gains:?} {r:?}", force[0], force[1]);
        }
    }
}

/// New gamepad order: those of `current` still in `connected`, in their order, then the
/// other connected ones in their order, at most `max`.
fn pad_order<T: PartialEq + Copy>(current: &[T], connected: &[T], max: usize) -> Vec<T> {
    let mut out: Vec<T> = current.iter().copied().filter(|id| connected.contains(id)).collect();
    for &id in connected {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out.truncate(max);
    out
}

impl Pads {
    /// Enumeration at client startup: the connected gamepads, at most 4. Without gilrs, a new
    /// attempt is made every 2 seconds.
    pub fn new() -> Option<Self> {
        let mut pads = Pads { gilrs: None, last_try: std::time::Instant::now(), list: Vec::new(), dpad_buttons: false };
        match Gilrs::new() {
            Ok(mut gilrs) => {
                // Windows.Gaming.Input (gilrs) sees no gamepad right after launch: they
                // arrive as "Connected" events a few milliseconds later. We
                // wait for them at most half a second, and 100 ms after the last arrival.
                let start_value = std::time::Instant::now();
                let mut last_value: Option<std::time::Instant> = None;
                while start_value.elapsed().as_millis() < 500 && last_value.is_none_or(|d| d.elapsed().as_millis() < 100) {
                    while let Some(e) = gilrs.next_event() {
                        if matches!(e.event, gilrs::EventType::Connected) {
                            last_value = Some(std::time::Instant::now());
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                pads.gilrs = Some(gilrs);
                pads.refresh();
            }
            Err(e) => eprintln!("gamepads: gilrs unavailable ({e}), new attempt every {RETRY_SECONDS} s"),
        }
        eprintln!("gamepads: {}", pads.list.len());
        Some(pads)
    }

    /// Pending events, then the list of connected gamepads (kept order, new ones at the end).
    fn refresh(&mut self) {
        if self.gilrs.is_none() {
            if self.last_try.elapsed().as_secs() < RETRY_SECONDS {
                return;
            }
            self.last_try = std::time::Instant::now();
            self.gilrs = Gilrs::new().ok();
            if self.gilrs.is_some() {
                eprintln!("gamepads: gilrs started");
            }
        }
        let Some(gilrs) = &mut self.gilrs else { return };
        let mut plugged = false;
        while let Some(e) = gilrs.next_event() {
            plugged |= matches!(e.event, gilrs::EventType::Connected | gilrs::EventType::Disconnected);
        }
        let connected: Vec<gilrs::GamepadId> = gilrs.gamepads().filter(|(_, g)| g.is_connected()).map(|(id, _)| id).collect();
        let current: Vec<gilrs::GamepadId> = self.list.iter().map(|p| p.id).collect();
        let order = pad_order(&current, &connected, MAX_PADS);
        if order == current {
            return;
        }
        // A removed gamepad drops its vibration effects.
        let mut old = std::mem::take(&mut self.list);
        self.list = order
            .into_iter()
            .map(|id| match old.iter().position(|p| p.id == id) {
                Some(i) => old.swap_remove(i),
                None => Pad { id, engines: None, without_vibrations: false },
            })
            .collect();
        drop(old);
        if plugged {
            eprintln!("gamepads: {} connected", self.list.len());
        }
        let mut line = format!("gamepads seen: {}", self.list.len());
        for (k, p) in self.list.iter().enumerate() {
            let g = gilrs.gamepad(p.id);
            let uuid: String = g.uuid().iter().map(|b| format!("{b:02x}")).collect();
            line.push_str(&format!(" | {k}: \"{}\" (os \"{}\", id {}, uuid {uuid}, mapping {:?})", g.name(), g.os_name(), usize::from(p.id), g.mapping_source()));
        }
        crate::renderer::diag(&line);
    }

    /// Strength of the two motors (large, small; 0 to 1) of each force-feedback client: client k
    /// goes to the k-th gamepad.
    pub fn vibrate(&mut self, forces: &[[f32; 2]; 4]) {
        let Some(gilrs) = &mut self.gilrs else { return };
        for (k, pad) in self.list.iter_mut().enumerate() {
            let f = forces[k];
            if pad.engines.is_none() {
                if f == [0.0; 2] || pad.without_vibrations {
                    continue;
                }
                pad.engines = Engines::new(gilrs, pad.id);
                if pad.engines.is_none() {
                    eprintln!("gamepad {k}: no rumble");
                    pad.without_vibrations = true;
                    continue;
                }
            }
            if let Some(m) = pad.engines.as_mut() {
                m.set(f);
            }
        }
    }

    /// Identity of each gamepad, in order (stays the same while it is plugged in).
    pub fn ids(&self) -> Vec<u64> {
        self.list.iter().map(|p| usize::from(p.id) as u64).collect()
    }

    /// Number of gamepads (NbJoy).
    pub fn count(&self) -> usize {
        self.list.len()
    }

    fn connected(&self) -> impl Iterator<Item = gilrs::Gamepad<'_>> {
        self.gilrs.iter().flat_map(|gilrs| self.list.iter().map(move |p| gilrs.gamepad(p.id))).filter(|g| g.is_connected())
    }

    /// Level selection buttons (choice_level.rs), all gamepads: d-pad or left stick
    /// up / down, A, B, X, Y, LB, RB. Read after `poll`.
    pub fn menu(&self) -> crate::choice_level::ButtonsMenu {
        let mut b = crate::choice_level::ButtonsMenu::default();
        for g in self.connected() {
            let y = g.axis_data(Axis::LeftStickY).map_or(0.0, |d| d.value());
            b.high |= g.is_pressed(Button::DPadUp) || y > 0.5;
            b.low_value |= g.is_pressed(Button::DPadDown) || y < -0.5;
            b.validate |= g.is_pressed(Button::South);
            b.return_value |= g.is_pressed(Button::East);
            b.open_flag |= g.is_pressed(Button::North);
            b.weapons |= g.is_pressed(Button::West);
            b.previous |= g.is_pressed(Button::LeftTrigger);
            b.next_one |= g.is_pressed(Button::RightTrigger);
        }
        b
    }

    /// All gamepads combined (the most deflected axis, buttons of either), None without a
    /// gamepad, in pad_android.rs's format: axes lx, ly, rx, ry (−1…1, Y down, dead zone), lt,
    /// rt (0…1); buttons as pad_keys.rs's bits. Read after `poll`.
    pub fn merged(&self) -> Option<([f32; 6], u32)> {
        self.merged_where(|_| true)
    }

    /// `merged` of the gamepads whose number (handle) `keep` accepts.
    pub fn merged_where(&self, keep: impl Fn(usize) -> bool) -> Option<([f32; 6], u32)> {
        let mut out: Option<([f32; 6], u32)> = None;
        let Some(gilrs) = &self.gilrs else { return None };
        let kept = self.list.iter().enumerate().filter(|(i, _)| keep(*i)).map(|(_, p)| gilrs.gamepad(p.id));
        for g in kept.filter(|g| g.is_connected()) {
            let axis = |a: Axis| g.axis_data(a).map_or(0.0, |d| d.value()).clamp(-1.0, 1.0);
            let trigger = |b: Button| g.button_data(b).map_or(0.0, |d| d.value()).clamp(0.0, 1.0);
            let stick = |x: f32, y: f32| if x.hypot(y) < DEAD_ZONE { (0.0, 0.0) } else { (x, y) };
            let (lx, ly) = stick(axis(Axis::LeftStickX), -axis(Axis::LeftStickY));
            let (rx, ry) = stick(axis(Axis::RightStickX), -axis(Axis::RightStickY));
            let t = |v: f32| if v < TRIGGER_DEAD_ZONE { 0.0 } else { v };
            let pad_axes = [lx, ly, rx, ry, t(trigger(Button::LeftTrigger2)), t(trigger(Button::RightTrigger2))];
            let mut bits = 0;
            for (b, bit) in [
                (Button::South, B_A),
                (Button::East, B_B),
                (Button::West, B_X),
                (Button::North, B_Y),
                (Button::LeftTrigger, B_LB),
                (Button::RightTrigger, B_RB),
                (Button::Select, B_BACK),
                (Button::Start, B_START),
                (Button::LeftThumb, B_THUMB_L),
                (Button::RightThumb, B_THUMB_R),
                (Button::DPadUp, B_HIGH),
                (Button::DPadDown, B_LOW),
                (Button::DPadLeft, B_LEFT),
                (Button::DPadRight, B_RIGHT),
            ] {
                if g.is_pressed(b) {
                    bits |= bit;
                }
            }
            let (axes, buttons_list) = out.get_or_insert(([0.0; 6], 0));
            for (a, v) in axes.iter_mut().zip(pad_axes) {
                if v.abs() > a.abs() {
                    *a = v;
                }
            }
            *buttons_list |= bits;
        }
        out
    }

    /// Reads the pending events (gamepads plugged or removed) then the state of each gamepad.
    pub fn poll(&mut self) -> Vec<PadState> {
        self.refresh();
        let Some(gilrs) = &self.gilrs else { return Vec::new() };
        let dpad = self.dpad_buttons;
        self.list
            .iter()
            .map(|p| {
                let g = gilrs.gamepad(p.id);
                let axis = |a: Axis| g.axis_data(a).map_or(0.0, |d| d.value());
                let trigger = |b: Button| g.button_data(b).map_or(0.0, |d| d.value());
                let di = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i32;
                let axes = [
                    di(axis(Axis::LeftStickX)),
                    di(-axis(Axis::LeftStickY)),
                    di(trigger(Button::LeftTrigger2) - trigger(Button::RightTrigger2)),
                    di(axis(Axis::RightStickX)),
                    di(-axis(Axis::RightStickY)),
                    0,
                ];
                let order = [
                    Button::South,
                    Button::East,
                    Button::West,
                    Button::North,
                    Button::LeftTrigger,
                    Button::RightTrigger,
                    Button::Select,
                    Button::Start,
                    Button::LeftThumb,
                    Button::RightThumb,
                    Button::DPadUp,
                    Button::DPadDown,
                    Button::DPadLeft,
                    Button::DPadRight,
                ];
                let mut buttons = [false; 16];
                for (i, b) in order.into_iter().enumerate().filter(|&(i, _)| dpad || i < 10) {
                    buttons[i] = g.is_connected() && g.is_pressed(b);
                }
                PadState { axes, buttons }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::pad_order;

    #[test]
    fn order_kept_added_removed_max() {
        // Nothing then a first gamepad.
        assert_eq!(pad_order::<u32>(&[], &[], 4), Vec::<u32>::new());
        assert_eq!(pad_order(&[], &[7], 4), vec![7]);
        // Kept order even if the connected list is sorted differently; new ones at the end.
        assert_eq!(pad_order(&[5, 2], &[1, 2, 5], 4), vec![5, 2, 1]);
        // Removal: the others keep their order.
        assert_eq!(pad_order(&[5, 2, 1], &[1, 5], 4), vec![5, 1]);
        // Removal then plugged again: back at the end.
        assert_eq!(pad_order(&[5, 1], &[1, 2, 5], 4), vec![5, 1, 2]);
        // At most 4.
        assert_eq!(pad_order(&[], &[1, 2, 3, 4, 5, 6], 4), vec![1, 2, 3, 4]);
        assert_eq!(pad_order(&[6, 5, 4, 3], &[1, 2, 3, 4, 5, 6], 4), vec![6, 5, 4, 3]);
        // A fifth gamepad takes the place of one removed.
        assert_eq!(pad_order(&[6, 5, 4, 3], &[1, 3, 4, 5], 4), vec![5, 4, 3, 1]);
        // Everything removed.
        assert_eq!(pad_order(&[1, 2], &[], 4), Vec::<u32>::new());
    }

    /// Random sequences: never more than the maximum, no duplicates, only connected
    /// gamepads, and the relative order of the kept ones never changes.
    #[test]
    fn order_random_sequences() {
        let mut seed: u32 = 0x9E37_79B9;
        let mut draw = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % n
        };
        let mut current: Vec<u32> = Vec::new();
        for _ in 0..2000 {
            let connected: Vec<u32> = (0..8).filter(|_| draw(3) != 0).collect();
            let next = pad_order(&current, &connected, 4);
            assert!(next.len() <= 4);
            assert!(next.iter().all(|id| connected.contains(id)));
            let mut sorted = next.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), next.len());
            let kept: Vec<u32> = current.iter().copied().filter(|id| connected.contains(id)).take(4).collect();
            assert_eq!(&next[..kept.len()], &kept[..]);
            assert_eq!(next.len(), connected.len().min(4));
            current = next;
        }
    }
}

#[cfg(test)]
mod faithful_rumble_tests {
    /// The faithful build never creates the vibration manager, even with a gamepad (the PC
    /// original has none). Negative control: the improved build with a gamepad does; without a
    /// gamepad or with vibrations off, neither does.
    #[test]
    fn the_faithful_build_has_no_vibration_manager() {
        assert!(!super::vibration_manager_wanted(false, true, true));
        assert!(super::vibration_manager_wanted(true, true, true));
        assert!(!super::vibration_manager_wanted(true, false, true));
        assert!(!super::vibration_manager_wanted(true, true, false));
    }
}
