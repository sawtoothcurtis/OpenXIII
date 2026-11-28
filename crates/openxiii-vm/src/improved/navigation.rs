//! Edition extensions for console commands, level selection, and transitions.

use crate::{ObjId, Vm};

pub(crate) fn start_url(vm: &mut Vm, url: &str) -> String {
    vm.multi_join_start_url(url)
}

pub(crate) fn local_players_command(vm: &mut Vm, word: &str, rest: &str) -> Option<String> {
    vm.local_players_exec(word, rest)
}

pub(crate) fn resolution_offered(vm: &Vm, w: i32, h: i32) -> bool {
    let (cw, ch) = vm.viewport_size_x_y();
    crate::video_settings::resolutions_with_current(vm.menu_resolutions(), (cw.max(0) as u32, ch.max(0) as u32))
        .contains(&(w as u32, h as u32))
}

/// Graphics: Original with a Borderless key: fullscreen is exclusive at the original's
/// resolution (Vm::fullscreen_size).
pub(crate) fn original_fullscreen(vm: &mut Vm) -> bool {
    vm.borderless_key() && !vm.graphics_setting()
}

/// The Video page's Enter with Borderless chosen: its SETRES changes neither the window nor the
/// saved sizes (video_menu.rs, keep_sizes).
pub(crate) fn setres_keeps_sizes(vm: &Vm) -> bool {
    vm.improved.video_menu.keep_sizes
}

pub(crate) fn level_selection_page(vm: &Vm) -> Option<ObjId> {
    vm.menu_main().or_else(|| vm.load_page_in_front()).or_else(|| vm.select_level_page_in_front())
}

pub(crate) fn menu_sound(vm: &mut Vm, native: &str, sound: String) -> Option<String> {
    if native == "PlayMenu" {
        if !vm.menu_mouse_plays(&sound) {
            return None;
        }
        if vm.improved.menu_boxes.arrival_only && crate::menu_sounds::is_main_page(&sound) && vm.engine.client.session_graphics.enhanced() {
            return Some(crate::menu_sounds::MAIN_ARRIVAL.to_owned());
        }
    }
    Some(sound)
}

/// Improved build: Cine2.InitCheckPoint places a cinematic pawn on its checkpoint's navigation
/// point, which stands about 100 units above the floor, so the pawn falls when the level is
/// restarted there (Toits01, "Major Jones 3"). A walking pawn that collides with the world is
/// swept down onto the floor instead; with no floor within reach it stays where it was placed.
pub(crate) fn checkpoint_cine_to_floor(vm: &mut Vm, a: ObjId) {
    use crate::NativeField as Field;
    if !vm.engine.client.session_graphics.enhanced() {
        return;
    }
    // The native itself is on top of the script stack, its caller just below.
    let Some(&caller) = vm.stack.len().checked_sub(2).and_then(|i| vm.stack.get(i)) else { return };
    let func = &vm.functions[caller];
    if !func.name.eq_ignore_ascii_case("InitCheckPoint") || !vm.classes[func.owner].name.eq_ignore_ascii_case("Cine2") {
        return;
    }
    if vm.native_prop(a, Field::Physics).as_byte() != 1 || !vm.native_prop(a, Field::CollideWorld).as_bool() {
        return;
    }
    let placed = vm.native_prop(a, Field::Location).as_vector();
    let rot = vm.native_prop(a, Field::Rotation).as_rotator();
    let mut hit = crate::physics::CheckResult::new(1.0);
    vm.move_actor(a, [0.0, 0.0, -256.0], rot, &mut hit, false, true, false, false);
    if hit.time >= 1.0 {
        vm.far_move_actor(a, placed, false, false, false);
    }
}
