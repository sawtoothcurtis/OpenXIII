//! The faithful network driver uses the shared LOGIN option and the original packet handling.
use crate::travel::Url;
use crate::value::ObjId;
use crate::Vm;

// In this edition the original field lists are the complete public network types.
macro_rules! with_edition_fields {
    ($($tokens:tt)*) => { $($tokens)* };
}
pub(crate) use with_edition_fields;

pub(crate) fn network_presentation_enhanced(_: crate::video_settings::SessionGraphics) -> bool {
    false
}

impl Vm {
    pub(crate) fn lan_host_tick_rate(&self, original: f32) -> f32 {
        original
    }

    pub(crate) fn lan_client_tick_rate(&self, fallback: i32) -> f32 {
        fallback as f32
    }

    pub(crate) fn net_host_rate_tick(&mut self) {}

    pub(crate) fn receive_host_rate(&mut self, _text: &str) -> bool {
        false
    }

    pub(crate) fn replicate_move_floor(&self, b: f32) -> f32 {
        b
    }

    pub(crate) fn receive_aim_update(&mut self, _connection: usize, _text: &str) -> bool {
        false
    }

    pub(crate) fn apply_network_login(&mut self, _connection: usize, pc: ObjId, url: &Url) {
        self.aim_client(pc, url);
    }

    pub(crate) fn login_pad_aim(&mut self) -> Option<i32> {
        None
    }

    pub(crate) fn remember_login_aim(&mut self, _mode: i32) {}

    pub(crate) fn add_network_login_options(&mut self, _url: &mut Url) {}

    pub(crate) fn net_client_aim_tick(&mut self) {}

    pub(crate) fn host_left_message(&self) -> String {
        String::new()
    }

    /// No Co-op page in this edition: no co-op name, the chat keeps the game's player names.
    pub fn coop_chat_name(&mut self) -> String {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_aim_updates_to_original_control_dispatch() {
        let mut vm = Vm::new();
        assert!(!vm.receive_aim_update(0, "OXIIIAUTOAIM V=1"));
    }

    #[test]
    fn leaves_host_rate_text_and_move_floor_to_the_original() {
        let mut vm = Vm::new();
        assert!(!vm.receive_host_rate("OXIIIHOSTRATE R=144"));
        assert_eq!(vm.replicate_move_floor(0.011), 0.011);
        assert_eq!(vm.lan_host_tick_rate(90.0), 90.0);
    }
}
