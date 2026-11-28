fn menu_stick_route(stick: Option<(f32, f32)>) -> (Option<(f32, f32)>, (f32, f32)) {
    stick_route(false, stick)
}

fn menu_handoff(_previous: bool) -> bool {
    false
}

impl super::App {
    fn page_reads_gamepad(&self) -> bool {
        self.choice.unlocked
    }

    fn repeat_menu_keys(&mut self, _repeats: [bool; 4], _touches: &[u8], _held_touches: &[u8]) {}
}

/// Faithful build: the original PC game's keyboard aim option (pad_keys.rs aim_pad_only).
fn aim_pad_only_in_this_build(android: bool) -> bool {
    aim_pad_only(false, android)
}
