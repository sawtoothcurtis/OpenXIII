//! Menu behavior for the faithful edition.

use crate::{gui::MenuFrame, ObjId, Vm};

pub(crate) fn character_screen_open(_: &Vm) -> bool {
    false
}

pub(crate) fn begin_wide_menu(_: &mut Vm, _: &MenuFrame, _: f32, whole_window: bool) -> bool {
    whole_window
}

pub(crate) fn end_wide_menu(_: &mut Vm, _: usize) {}

pub(crate) fn prepare_menu_figures(_: &mut Vm) {}

pub(crate) fn update_menu_hover(_: &mut Vm, _: ObjId) {}

pub(crate) fn help_bar_frame(_: &mut Vm, _: ObjId, _: bool) {}

pub(crate) fn cursor_range(_: &Vm, width: i32) -> (i32, i32) {
    (0, width)
}

pub(crate) fn help_bar_touch(_: &mut Vm, _: ObjId, _: f32, _: f32) -> Option<Option<u8>> {
    None
}

pub(crate) fn help_bar_lift(_: &mut Vm) {}

pub(crate) fn menu_part_at(_: &Vm, _: f32, _: f32) -> Option<usize> {
    None
}

pub(crate) fn is_list_row(_: &Vm, _: ObjId) -> bool {
    false
}

pub(crate) fn overlay_open(_: &Vm) -> bool {
    false
}

pub(crate) fn mouse_key_event(_: &mut Vm, _: ObjId, _: u8, _: u8, _: f32) -> Option<bool> {
    None
}

pub(crate) fn menu_key_event(_: &mut Vm, _: ObjId, _: u8, _: u8, _: bool) -> Option<bool> {
    None
}

pub(crate) fn track_cursor_motion(_: &mut Vm, _: ObjId, _: (i32, i32), _: (i32, i32)) {}
