//! Faithful local play. Unit tests can still exercise the shared campaign and split-screen code.
use crate::{
    value::{ObjId, Value},
    Vm,
};

pub(crate) const CAMPAIGN_AVAILABLE: bool = cfg!(test);
pub(crate) const MAX_PLAYERS: u8 = if cfg!(test) { crate::split_versus::MAX_LOCAL_PLAYERS } else { 1 };

pub(crate) fn character_preview_url(_: &Vm) -> bool {
    false
}

impl Vm {
    /// The original has no co-op page to go back to.
    pub(crate) fn reopen_coop_after_split_screen(&mut self) {}

    pub(crate) fn update_local_characters(&mut self, _: f32) {}

    pub(crate) fn versus_menu_one_player(&mut self) -> Option<(ObjId, Value)> {
        None
    }
}
