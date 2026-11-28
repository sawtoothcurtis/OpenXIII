//! DirectInput gamepads of the viewports (WinDrv.dll): UWindowsClient::EnumJoysticksCallback
//! (at most 4 gamepads, NbJoy, axis range −32768…32767 through EnumAxesCallback), AttribPadToViewport and the gamepad part of
//! UWindowsViewport::UpdateInput, called during the Tick of the viewport's player.
//!
//! The VM reads no device: the display layer gives the DIJOYSTATE of each gamepad
//! (`set_pad_state`).

use crate::value::ObjId;
use crate::Vm;

/// DIJOYSTATE read by IDirectInputDevice8::GetDeviceState (0x110 bytes): axes lX, lY, lZ, lRx,
/// lRy, lRz (−32768…32767) and buttons rgbButtons (bit 0x80: pressed), only the first 16.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PadState {
    pub axes: [i32; 6],
    pub buttons: [bool; 16],
}

/// Time read by WinDrv's function (seconds of the processor counter) assumed for the
/// axis duration: a machine switched on for more than 1000 s.
const PAD_TIME_MACHINE: f64 = 3600.0;

/// Rounding of an x87 operation at 24-bit precision (the game's control word).
fn x87_24(x: f64) -> f64 {
    f64::from(x as f32)
}

/// WinDrv's time function: (high × 2³² + low) × GSecondsPerCycle, each operation rounded
/// to 24 bits.
#[cfg(test)]
pub(crate) fn pad_time(tsc: u64, seconds_by_cycle: f64) -> f64 {
    // The low word is loaded signed then corrected by 2³²: this addition is already rounded to 24 bits.
    let low_value = f64::from(tsc as u32 as i32);
    let low_value = if low_value < 0.0 { x87_24(low_value + 4294967296.0) } else { low_value };
    let sum_value = x87_24((tsc >> 32) as f64 * 4294967296.0 + low_value);
    x87_24(sum_value * seconds_by_cycle)
}

/// clamp((time − last) × 0.001, 0, 1) as a single-precision float (UpdateInput: subtraction
/// and product on 24 bits, bounds through function).
/// Duration given to the gamepad axes in UpdateInput (see update_input_pad).
pub(crate) fn pad_look_delta() -> f32 {
    pad_delta(PAD_TIME_MACHINE, 0.0)
}

pub(crate) fn pad_delta(time_value: f64, last_one: f64) -> f32 {
    (x87_24(x87_24(time_value - last_one) * 0.001) as f32).clamp(0.0, 1.0)
}

/// One event of the gamepad part of UpdateInput.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PadEvent {
    /// CauseInputEvent(key, action, value).
    Process { key: u8, action: u8, value: f32 },
    /// UInput::DirectAxis (vtable +0x7C): key, value × 1/32768, frame duration.
    Axis { key: u8, value: f32, delta: f32 },
}

/// WinDrv state shared by the gamepads.
#[derive(Debug, Clone, Default)]
pub struct Pads {
    /// aJoyHandle: state of each enumerated gamepad (None: null handle).
    pub states: [Option<PadState>; 4],
    /// NbJoy.
    pub count: i32,
    /// Last values of X, −Y, Z and −Rz, globals (shared by all
    /// viewports in the original code).
    pub(crate) previous: [f32; 4],
}

/// Axis range set by EnumAxesCallback: DIPROP_RANGE from −32768 to 32767.
pub(crate) const AXIS_MIN: i32 = -32768;
pub(crate) const AXIS_MAX: i32 = 32767;
/// 1 / 32768.
const AXIS_SCALE: f32 = 1.0 / 32768.0;
/// Thresholds of the direction events.
const HIGH: f32 = 16384.0;
const LOW: f32 = -16384.0;

impl Vm {
    /// State of gamepad `index` (enumeration order) for the next frame.
    pub fn set_pad_state(&mut self, index: usize, state: Option<PadState>) {
        if let Some(s) = self.pads.states.get_mut(index) {
            *s = state;
        }
    }

    /// Gamepads found at startup (EnumDevices at the client's Init): NbJoy and the handles.
    pub fn set_pad_count(&mut self, count: usize) {
        self.pads.count = count.min(4) as i32;
        for (i, s) in self.pads.states.iter_mut().enumerate() {
            if i >= count {
                *s = None;
            } else if s.is_none() {
                *s = Some(PadState::default());
            }
        }
        self.attrib_pad_to_viewport();
    }

    /// UWindowsClient::AttribPadToViewport: the non-null handles, in order,
    /// to the viewports (Viewport+0x1C4). Only the served viewport is written; the others, including
    /// those in reserve, keep their old handle. `viewport_pads` is indexed by the viewport's slot.
    pub(crate) fn attrib_pad_to_viewport(&mut self) {
        let slots = self.engine.client.viewports.clone();
        let Some(&last_one) = slots.iter().max() else { return };
        if self.engine.client.viewport_pads.len() <= usize::from(last_one) {
            self.engine.client.viewport_pads.resize(usize::from(last_one) + 1, None);
        }
        // Local split screen (split_versus.rs): each player keeps their own gamepad.
        if self.split_pads() {
            self.split_attrib_pads(&slots);
            return;
        }
        let mut served = 0;
        for h in 0..4 {
            if served >= slots.len() {
                break;
            }
            if self.pads.states[h].is_some() {
                self.engine.client.viewport_pads[usize::from(slots[served])] = Some(h);
                served += 1;
            }
        }
    }

    /// UWindowsViewport::UpdateInput, gamepad part: with UseJoystick and a gamepad
    /// for the viewport, buttons 1 to 16 to IK_Joy1…16 (press / release), axes through
    /// UInput::DirectAxis (X, −Y, Z, R, U, V × 1/32768), then, from the second frame on, the
    /// direction events when X, −Y, Z or −Rz crosses ±16384. `pc`: the viewport's player,
    /// whose UInput is the current one (input_sys).
    /// Returns true when the gamepad was read: its buttons (200…215) and its axes are then marked as handled
    /// for the keyboard reading that follows (windrv_input.rs).
    pub(crate) fn update_input_pad(&mut self, pc: ObjId, dt: f32) -> bool {
        // Look stick of a gamepad or of the touch controls (not in the original PC version).
        if !(self.local_split() && self.coop.players.first().is_some_and(|&first| first != pc)) {
            self.look_stick_axes(pc);
        }
        let Some(slot) = self.viewport_slot_of(pc) else { return false };
        if !self.engine.client.viewports.contains(&slot) {
            return false;
        }
        let Some(handle) = self.engine.client.viewport_pads.get(usize::from(slot)).copied().flatten() else { return false };
        // Local co-op (coop.rs): the gamepad of a player other than the first plays the keys.
        if self.local_split() && self.coop.players.first().is_some_and(|&first| first != pc) {
            if let Some(state) = self.pads.states[handle] {
                self.coop_pad_input(pc, state);
            }
            return false;
        }
        let use_joystick = self
            .config
            .as_mut()
            .and_then(|c| c.get_string("WinDrv.WindowsClient", "UseJoystick", "System"))
            .is_some_and(|v| v.eq_ignore_ascii_case("true") || v.trim() == "1");
        if !use_joystick {
            return false;
        }
        let Some(state) = self.pads.states[handle] else { return false };
        // Duration passed to the axes: (time − viewport+0x7C) × 0.001, clamped to [0, 1]. No DLL code writes
        // viewport+0x7C (it stays at 0) and the time is that of the processor counter in seconds: the value
        // has nothing to do with the frame duration and is 1 as soon as the machine has been on for 1000 s.
        let _ = dt;
        let delta = pad_delta(PAD_TIME_MACHINE, 0.0);
        let downs: Vec<bool> = (200..216u8).map(|key| self.input_key_down(key)).collect();
        let events = self.pad_events(slot, state, delta, &|key| downs[usize::from(key - 200)]);
        for e in events {
            match e {
                PadEvent::Process { key, action, value } => {
                    self.input_process(pc, key, action, value);
                }
                PadEvent::Axis { key, value, delta } => self.input_direct_axis(pc, key, value, delta),
            }
        }
        true
    }

    /// The events of the gamepad part of UpdateInput, in the original's order: buttons
    /// (CauseInputEvent press / release), axes (UInput::DirectAxis, vtable +0x7C), then, from the
    /// viewport's second frame on, the direction events when X, −Y, Z or −Rz crosses ±16384. Updates
    /// the last values (WinDrv globals) and the viewport's "already seen" memory.
    pub(crate) fn pad_events(&mut self, slot: u8, state: PadState, delta: f32, key_down: &dyn Fn(u8) -> bool) -> Vec<PadEvent> {
        let mut out = Vec::new();
        for i in 0..16u8 {
            let key = 200 + i;
            let down = key_down(key);
            if !down && state.buttons[i as usize] {
                out.push(PadEvent::Process { key, action: crate::player_input::IST_PRESS, value: 0.0 });
            } else if down && !state.buttons[i as usize] {
                out.push(PadEvent::Process { key, action: crate::player_input::IST_RELEASE, value: 0.0 });
            }
        }
        let [lx, ly, lz, lrx, lry, lrz] = state.axes;
        for (key, v) in [(0xe0u8, lx), (0xe1, -ly), (0xea, lz), (0xe3, lrx), (0xe8, lry), (0xe9, lrz)] {
            out.push(PadEvent::Axis { key, value: v as f32 * AXIS_SCALE, delta });
        }
        let first = !self.pad_seen.contains(&slot);
        let values = [lx as f32, -ly as f32, lz as f32, -lrz as f32];
        if !first {
            for (j, key) in [0xe0u8, 0xe1, 0xe8, 0xe9].into_iter().enumerate() {
                let (prev, v) = (self.pads.previous[j], values[j]);
                if (HIGH < prev && v < HIGH) || (prev < LOW && LOW < v) {
                    out.push(PadEvent::Process { key, action: crate::player_input::IST_RELEASE, value: v });
                } else if (prev < HIGH && HIGH < v) || (LOW < prev && v < LOW) {
                    out.push(PadEvent::Process { key, action: crate::player_input::IST_PRESS, value: v });
                }
            }
        }
        self.pads.previous = values;
        self.pad_seen.insert(slot);
        out
    }
}

