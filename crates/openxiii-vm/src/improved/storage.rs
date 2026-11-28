//! Edition-specific behavior during initialization, travel and save operations.

use crate::{travel::TravelRequest, Vm};
use std::path::Path;

pub(crate) use crate::video_settings::apply_first_launch;

pub(crate) fn prefetch_menu_pictures(video_dir: &Path) {
    if crate::select_level_page::art_cache_set() {
        crate::menu_boxes::prefetch_pictures(video_dir);
    }
}

pub(crate) fn character_screen_client_travel(vm: &mut Vm) {
    vm.character_screen_client_travel();
}

pub(crate) fn character_screen_travel(vm: &mut Vm, request: Option<TravelRequest>) -> Option<TravelRequest> {
    vm.character_screen_travel(request)
}

pub(crate) fn erase_coop_slot(vm: &mut Vm) {
    let (profile, slot) = vm.active_save_slot();
    vm.coop_save_erase_slot(&profile, slot);
}

pub(crate) fn write_coop_slot(vm: &mut Vm, slot: i32) {
    let (profile, _) = vm.active_save_slot();
    vm.coop_save_write_slot(&profile, slot);
}

pub(crate) fn read_coop_slot(vm: &mut Vm) {
    let (profile, slot) = vm.active_save_slot();
    vm.coop_save_read_slot(&profile, slot);
}

/// Improved build: a saved window size kept as it is at start (UWindowsViewport::OpenWindow
/// otherwise brings it to a standard 4:3 resolution): at least 640 x 480, and within the desktop
/// when its size is known.
pub(crate) fn window_size_kept(w: i32, h: i32, desktop: (i32, i32)) -> bool {
    w >= 640 && h >= 480 && (desktop.0 <= 0 || desktop.1 <= 0 || (w <= desktop.0 && h <= desktop.1))
}

#[cfg(test)]
#[test]
fn saved_window_sizes_kept_when_they_fit() {
    assert!(window_size_kept(1024, 576, (1920, 1080)));
    assert!(window_size_kept(854, 480, (1920, 1080)));
    assert!(window_size_kept(1920, 1080, (1920, 1080)));
    // Too small, or larger than the desktop (a 1280 x 960 window on a 1366 x 768 laptop): the
    // original's standard resolution instead.
    assert!(!window_size_kept(640, 400, (1920, 1080)));
    assert!(!window_size_kept(1280, 960, (1366, 768)));
    // Desktop not known yet: kept.
    assert!(window_size_kept(1024, 576, (0, 0)));
}
