//! Video preferences of the original renderer.

use crate::video_settings;

pub(crate) const VSYNC_DEFAULT: video_settings::VsyncMode = video_settings::VsyncMode::Classic;
pub(crate) fn vsync_ini(_: bool) -> (&'static str, &'static str) {
    (video_settings::ORIGINAL_VSYNC_SECTION, video_settings::KEY_ORIGINAL_VSYNC)
}
pub(crate) const ANISOTROPY_DEFAULT: u32 = 1;

pub(crate) fn local() -> bool {
    false
}
pub(crate) fn effective(_: video_settings::SessionGraphics) -> bool {
    false
}
pub(crate) fn refresh(_: bool, _: video_settings::SessionGraphics) {}
