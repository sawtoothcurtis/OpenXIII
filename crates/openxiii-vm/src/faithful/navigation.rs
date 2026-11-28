//! Original console commands, level selection, and transitions.

use crate::{ObjId, Vm};

pub(crate) fn start_url(_: &mut Vm, url: &str) -> String {
    url.to_owned()
}

pub(crate) fn local_players_command(_: &mut Vm, _: &str, _: &str) -> Option<String> {
    None
}

pub(crate) fn resolution_offered(_: &Vm, _: i32, _: i32) -> bool {
    false
}

pub(crate) fn original_fullscreen(_: &mut Vm) -> bool {
    false
}

pub(crate) fn setres_keeps_sizes(_: &Vm) -> bool {
    false
}

pub(crate) fn level_selection_page(vm: &Vm) -> Option<ObjId> {
    vm.menu_main()
}

pub(crate) fn menu_sound(_: &mut Vm, _: &str, sound: String) -> Option<String> {
    Some(sound)
}

pub(crate) fn checkpoint_cine_to_floor(_: &mut Vm, _: ObjId) {}
