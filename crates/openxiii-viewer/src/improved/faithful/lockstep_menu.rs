impl crate::App {
    fn set_lockstep_notice(&mut self, _why: &str) {}

    /// The waiting sign drawn over the picture (screen pixels of a `w` x `h` view): while meeting
    /// the other machine, and while a step waits for it.
    pub(crate) fn lockstep_sign(&mut self, _w: f32, _h: f32) -> Vec<openxiii_vm::canvas::HudDraw> {
        Vec::new()
    }

    pub(crate) fn lockstep_notice_frame(&mut self) {}

    pub(crate) fn lockstep_pad_buttons(&mut self) -> u32 {
        0
    }

    fn note_departing_player(&mut self, _by: Option<usize>, _local: Option<usize>) {}
}

fn character_selection_travel(_url: &mut openxiii_vm::travel::Url) -> bool {
    false
}

fn local_graphics_profile_value() -> &'static str {
    "0"
}

/// The faithful build's host is not listed on the Join page (it has none).
fn list_coop_host(_meeting: &mut Meeting, _vm: &mut Vm, _map: &str) {}
