//! Menu behavior for the improved edition.

use crate::gui::{IST_PRESS, IST_RELEASE};
use crate::{gui::MenuFrame, ObjId, Vm};

/// The in-game menus' Video line, whatever Graphics is: with Original it is the way back to
/// Enhanced without leaving the game (pause_video.rs).
pub(crate) fn offer_pause_video(vm: &mut Vm, page: ObjId, multiplayer: bool) {
    if multiplayer {
        vm.pause_video_multi_created(page);
    } else {
        vm.pause_video_created(page);
    }
}

pub(crate) fn character_screen_open(vm: &Vm) -> bool {
    vm.character_screen_on()
}

pub(crate) fn begin_wide_menu(vm: &mut Vm, frame: &MenuFrame, canvas_width: f32, _: bool) -> bool {
    vm.wide_menu_begin(frame, canvas_width);
    true
}

pub(crate) fn end_wide_menu(vm: &mut Vm, start: usize) {
    vm.wide_menu_end(start);
}

pub(crate) fn prepare_menu_figures(vm: &mut Vm) {
    vm.menu_figures_apply();
    vm.select_page_jones_fit();
}

pub(crate) fn update_menu_hover(vm: &mut Vm, ctrl: ObjId) {
    vm.menu_mouse_settle(ctrl);
    vm.help_bar_hover(ctrl);
}

pub(crate) fn help_bar_frame(vm: &mut Vm, ctrl: ObjId, drawn: bool) {
    vm.help_bar_frame(ctrl, drawn);
}

pub(crate) fn cursor_range(vm: &Vm, width: i32) -> (i32, i32) {
    vm.help_bar_cursor_range(width)
}

pub(crate) fn help_bar_touch(vm: &mut Vm, ctrl: ObjId, x: f32, y: f32) -> Option<Option<u8>> {
    vm.help_bar_touch_key(ctrl, x, y)
}

pub(crate) fn help_bar_lift(vm: &mut Vm) {
    vm.help_bar_untouch();
}

pub(crate) fn menu_part_at(vm: &Vm, x: f32, y: f32) -> Option<usize> {
    vm.select_gear_at(x, y)
}

pub(crate) fn is_list_row(vm: &Vm, control: ObjId) -> bool {
    vm.improved.menu_boxes.select.is_row(control)
}

pub(crate) fn overlay_open(vm: &Vm) -> bool {
    vm.menu_overlay_open()
}

pub(crate) fn mouse_key_event(vm: &mut Vm, ctrl: ObjId, key: u8, action: u8, delta: f32) -> Option<bool> {
    if vm.gof_pub(ctrl, "ViewportOwner").is_some() && vm.gof_pub(ctrl, "ViewportOwner") == vm.local_viewport() {
        if let Some(r) = vm.menu_mouse_key(ctrl, key, action, delta) {
            return Some(r);
        }
        if key == 0x01 {
            if action == IST_PRESS {
                return Some(vm.gui_controller_key_event_in(ctrl, key, action, delta, true));
            }
            if action == IST_RELEASE && vm.improved.menu_mouse.click.is_some() {
                let r = vm.gui_controller_key_event_in(ctrl, key, action, delta, false);
                vm.menu_mouse_click_end(ctrl, key, action);
                return Some(r);
            }
        }
    }
    None
}

pub(crate) fn menu_key_event(vm: &mut Vm, ctrl: ObjId, key: u8, action: u8, click_sound: bool) -> Option<bool> {
    if let Some(result) = vm.help_bar_click_key(ctrl, key, action) {
        return Some(result);
    }
    if click_sound {
        vm.menu_mouse_click_begin(ctrl, key, action);
    }
    // Escape on a box whose only button is Cancel (the multiplayer Join page's "Connecting"
    // box) is that Cancel, as Backspace is in XIIIMsgBox.InternalOnKeyEvent: in the original,
    // XIIIWindow.InternalOnKeyEvent eats Escape there and the box can only be left by
    // Backspace or its button.
    if key == 0x1B && action == IST_PRESS && escape_cancels_box(vm, ctrl) {
        return Some(vm.gui_controller_key_event_in(ctrl, 0x08, action, 0.0, false));
    }
    (vm.character_screen_on() && crate::character_page::swallowed_on_screen(key)).then_some(true)
}

/// The controller's active page is an XIIIMsgBox with a single button, its Cancel button.
fn escape_cancels_box(vm: &Vm, ctrl: ObjId) -> bool {
    let Some(page) = vm.gof_pub(ctrl, "ActivePage").filter(|&p| vm.is_a(p, "XIIIMsgBox")) else { return false };
    let cancel = vm.propr(page, "CancelButton").as_object();
    let buttons: Vec<Option<ObjId>> = match vm.propr(page, "Buttons") {
        crate::value::Value::Array(b) => b.iter().map(crate::value::Value::as_object).collect(),
        _ => Vec::new(),
    };
    cancel.is_some() && buttons == [cancel]
}

/// On a scaled menu the raw mouse delta may land on a different control than the Windows cursor.
pub(crate) fn track_cursor_motion(vm: &mut Vm, ctrl: ObjId, old: (i32, i32), new: (i32, i32)) {
    if old == new || !vm.engine.client.session_graphics.enhanced() || vm.gof_pub(ctrl, "ActivePage").is_none() {
        return;
    }
    let dragging = vm.gof_pub(ctrl, "ActiveControl").is_some_and(|a| {
        vm.propr(a, "MenuState").as_byte() == crate::gui::MSAT_PRESSED && vm.propr(a, "bCaptureMouse").as_bool()
    });
    if !dragging {
        vm.gui_look_under_cursor(ctrl, (new.0 - old.0) as f32, (new.1 - old.1) as f32);
    }
}
