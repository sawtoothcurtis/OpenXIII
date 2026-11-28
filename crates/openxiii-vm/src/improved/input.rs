//! Improved controller navigation and analog movement.
use crate::{pad::PadState, value::ObjId, Vm};

pub(crate) fn movement_keys(_: i32, _: i32) -> [bool; 4] {
    // The native input receives these axes through analog_player_movement.
    [false; 4]
}

fn mouse_navigation(graphics: crate::video_settings::SessionGraphics) -> bool {
    !cfg!(target_os = "android") && graphics.enhanced()
}

pub(crate) fn menu_mouse_enabled(graphics: crate::video_settings::SessionGraphics) -> bool {
    mouse_navigation(graphics)
}

pub(crate) fn help_bar_mouse_enabled(graphics: crate::video_settings::SessionGraphics) -> bool {
    mouse_navigation(graphics)
}

impl Vm {
    pub(crate) fn menu_stick_directions(&mut self, rank: usize, pad: &PadState, menu: bool) -> ([bool; 4], [bool; 4]) {
        let (ax, bt) = (&pad.axes, &pad.buttons);
        if self.improved.pad_menu_repeat.len() <= rank {
            self.improved.pad_menu_repeat.resize(rank + 1, Default::default());
        }
        let held_before = self.improved.pad_menu_repeat[rank].0;
        let stick = crate::menu_repeat::stick_directions(ax[0] as f32 / 32768.0, ax[1] as f32 / 32768.0, if menu { held_before } else { [false; 4] });
        let dirs = [bt[10] || stick[0], bt[11] || stick[1], bt[12] || stick[2], bt[13] || stick[3]];
        let dirs = if menu { dirs } else { [false; 4] };
        // Lockstep co-op: the session's time, the same on every machine.
        let now = self.lockstep_clock().unwrap_or_else(crate::menu_repeat::clock);
        let entry = &mut self.improved.pad_menu_repeat[rank];
        entry.0 = if menu { stick } else { [false; 4] };
        (stick, entry.1.update(dirs, now))
    }

    pub(crate) fn record_movement_stick(&mut self, pc: ObjId, stick: (f32, f32)) {
        match self.move_sticks.iter_mut().find(|m| m.0 == pc) {
            Some(m) => m.1 = stick,
            None => self.move_sticks.push((pc, stick)),
        }
    }

    pub(crate) fn analog_player_movement(&self, pc: ObjId) -> Option<(f32, f32)> {
        self.move_stick_of(pc).map(|(x, y)| crate::player_input::analog_move(x, y))
    }
}
