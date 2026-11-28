//! Original initialization, travel and save behavior at the edition extension points.

use crate::{config::ConfigCache, travel::TravelRequest, Vm};
use std::path::Path;

pub(crate) fn apply_first_launch(_: &mut ConfigCache) -> bool {
    false
}

pub(crate) fn window_size_kept(_: i32, _: i32, _: (i32, i32)) -> bool {
    false
}

pub(crate) fn prefetch_menu_pictures(_: &Path) {}
pub(crate) fn character_screen_client_travel(_: &mut Vm) {}

pub(crate) fn character_screen_travel(_: &mut Vm, request: Option<TravelRequest>) -> Option<TravelRequest> {
    request
}

pub(crate) fn erase_coop_slot(_: &mut Vm) {}
pub(crate) fn write_coop_slot(_: &mut Vm, _: i32) {}
pub(crate) fn read_coop_slot(_: &mut Vm) {}

#[test]
fn saved_window_sizes_use_the_original_resolution_table() {
    assert!(!window_size_kept(1024, 576, (1920, 1080)));
}
