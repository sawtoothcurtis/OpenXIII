//! Original PC gamepad input uses movement keys and has no menu direction repeat.
use crate::{pad::PadState, value::ObjId, Vm};

pub(crate) fn movement_keys(x: i32, y: i32) -> [bool; 4] {
    const THRESHOLD: i32 = 11469;
    [y < -THRESHOLD, y > THRESHOLD, x < -THRESHOLD, x > THRESHOLD]
}

impl Vm {
    pub(crate) fn menu_stick_directions(&mut self, _: usize, pad: &PadState, _: bool) -> ([bool; 4], [bool; 4]) {
        let [x, y, ..] = pad.axes;
        ([y < -16384, y > 16384, x < -16384, x > 16384], [false; 4])
    }

    pub(crate) fn record_movement_stick(&mut self, _: ObjId, _: (f32, f32)) {}

    pub(crate) fn analog_player_movement(&self, _: ObjId) -> Option<(f32, f32)> {
        None
    }
}
