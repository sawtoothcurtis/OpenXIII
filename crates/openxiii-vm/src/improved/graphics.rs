//! Video preferences used by the improved edition.

use crate::{video_settings, Vm};
use std::sync::atomic::Ordering;

/// V-sync of Graphics: Enhanced when its VSync key is absent (first launch included).
pub(crate) const VSYNC_DEFAULT: video_settings::VsyncMode = video_settings::VsyncMode::Classic;
pub(crate) fn vsync_ini(enhanced: bool) -> (&'static str, &'static str) {
    if enhanced {
        (video_settings::SECTION, video_settings::KEY_VSYNC)
    } else {
        (video_settings::ORIGINAL_VSYNC_SECTION, video_settings::KEY_ORIGINAL_VSYNC)
    }
}
pub(crate) const ANISOTROPY_DEFAULT: u32 = if cfg!(target_os = "android") { 4 } else { 16 };

pub(crate) fn local() -> bool {
    video_settings::graphics_override().unwrap_or_else(|| video_settings::ENHANCED.load(Ordering::Relaxed))
}

pub(crate) fn effective(session: video_settings::SessionGraphics) -> bool {
    if let Some(on) = video_settings::graphics_override() {
        return on;
    }
    if let Some(on) = session.host {
        return on;
    }
    video_settings::ENHANCED.load(Ordering::Relaxed)
}

pub(crate) fn refresh(on: bool, session: video_settings::SessionGraphics) {
    if on != effective(session) {
        video_settings::set_graphics_enhanced(on);
    }
}

impl Vm {
    /// The edge of the menus' characters (select_level_art::smooth_edge): None the game's own
    /// pictures (Graphics: Original), otherwise Some(false), the slopes found again from the steps
    /// (heads keep the simple variant, menu_figures.rs). No setting: a CharacterEdges key left in
    /// XIII.ini by an older build is ignored.
    pub(crate) fn character_edge(&self) -> Option<bool> {
        self.engine.client.session_graphics.enhanced().then_some(false)
    }
}
