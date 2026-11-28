fn menu_stick_route(stick: Option<(f32, f32)>) -> (Option<(f32, f32)>, (f32, f32)) {
    stick_route(true, stick)
}

fn menu_handoff(previous: bool) -> bool {
    previous
}

impl super::App {
    fn page_reads_gamepad(&self) -> bool {
        self.choice.unlocked || self.viewer.mods_page.open
    }

    fn repeat_menu_keys(&mut self, repeats: [bool; 4], touches: &[u8], held_touches: &[u8]) {
        for (i, vk) in [0x26u8, 0x28, 0x25, 0x27].into_iter().enumerate() {
            if repeats[i] && touches.contains(&vk) && held_touches.contains(&vk) {
                self.key_windows(vk, false);
                self.key_windows(vk, true);
            }
        }
    }
}

/// Improved build: on PC the aim assist is the gamepad's only (pad_keys.rs aim_pad_only).
fn aim_pad_only_in_this_build(android: bool) -> bool {
    aim_pad_only(true, android)
}
