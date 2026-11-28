//! Improved build only: which way a two-player split screen is cut (the Co-op page's "Split"
//! setting). The original game always stacks two views (UClient::ActiveViewports: top
//! and bottom halves); on a very wide screen (21:9, 32:9) each half would be a thin strip, so the
//! improved build puts the two views side by side there.
//!
//! - Auto (default): top / bottom up to about 16:9 (16:10, 16:9 and narrower), side by side on
//!   wider screens ([`AUTO_SIDE_BY_SIDE_ASPECT`]).
//! - Horizontal: always top / bottom (a horizontal line between the views), as the original.
//! - Vertical: always side by side (a vertical line between the views).
//!
//! Only two views change: three and four players keep the original's quarters. With Graphics:
//! Original (video_settings.rs) the views are always stacked, as in the original game.
//! The value is kept with the Co-op page's other values (coop_page.rs, "SplitLayout=").

use std::sync::atomic::{AtomicU8, Ordering};

/// The setting's values, in the order the page cycles through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitLayout {
    Auto,
    Horizontal,
    Vertical,
}

impl SplitLayout {
    pub const ALL: [SplitLayout; 3] = [SplitLayout::Auto, SplitLayout::Horizontal, SplitLayout::Vertical];

    /// Text kept in the file.
    pub fn key_text(self) -> &'static str {
        match self {
            SplitLayout::Auto => "Auto",
            SplitLayout::Horizontal => "Horizontal",
            SplitLayout::Vertical => "Vertical",
        }
    }

    /// Value read from the file: Auto for anything unknown.
    pub fn from_text(s: &str) -> SplitLayout {
        let s = s.trim();
        SplitLayout::ALL.into_iter().find(|v| v.key_text().eq_ignore_ascii_case(s)).unwrap_or(SplitLayout::Auto)
    }

    fn index(self) -> u8 {
        match self {
            SplitLayout::Auto => 0,
            SplitLayout::Horizontal => 1,
            SplitLayout::Vertical => 2,
        }
    }

    /// The next (or previous, `step` < 0) value, wrapping.
    pub fn step(self, step: isize) -> SplitLayout {
        let n = SplitLayout::ALL.len() as isize;
        SplitLayout::ALL[(self.index() as isize + step).rem_euclid(n) as usize]
    }

    /// Are two views side by side in a `width` × `height` client with this setting?
    pub fn side_by_side(self, width: i32, height: i32) -> bool {
        match self {
            SplitLayout::Horizontal => false,
            SplitLayout::Vertical => true,
            SplitLayout::Auto => width > 0 && height > 0 && width as f32 / height as f32 > AUTO_SIDE_BY_SIDE_ASPECT,
        }
    }
}

/// Auto: screens wider than this (width / height) get their views side by side. 16:9 (1.78) and
/// 16:10 stay stacked; 21:9 (2.33 to 2.39) and 32:9 (3.56) are cut down the middle.
pub const AUTO_SIDE_BY_SIDE_ASPECT: f32 = 2.0;

/// The setting, process-wide (read by the client's layout, which has no VM at hand).
static LAYOUT: AtomicU8 = AtomicU8::new(0);

/// The current setting.
pub fn split_layout() -> SplitLayout {
    SplitLayout::ALL[LAYOUT.load(Ordering::Relaxed) as usize % 3]
}

/// Sets the setting (the Co-op page, the kept value at startup).
pub fn set_split_layout(v: SplitLayout) {
    LAYOUT.store(v.index(), Ordering::Relaxed);
}

/// Two views side by side in a `width` × `height` client? Never in the faithful build, with
/// Graphics: Original, or in this crate's tests (they compare the original's layout).
pub use crate::improved::visual_effects::side_by_side;

/// A two-view layout fraction (x, y, width, height of the client) turned from stacked to side by
/// side: the top half becomes the left one, the bottom half the right one.
pub fn side_by_side_fraction(f: [f32; 4]) -> [f32; 4] {
    [f[1], f[0], f[3], f[2]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_cuts_wide_screens_down_the_middle() {
        let a = SplitLayout::Auto;
        assert!(!a.side_by_side(1920, 1080), "16:9 stacked");
        assert!(!a.side_by_side(1920, 1200), "16:10 stacked");
        assert!(!a.side_by_side(1024, 768), "4:3 stacked");
        assert!(a.side_by_side(2560, 1080), "21:9 side by side");
        assert!(a.side_by_side(3440, 1440), "21:9 side by side");
        assert!(a.side_by_side(5120, 1440), "32:9 side by side");
        assert!(!SplitLayout::Horizontal.side_by_side(5120, 1440));
        assert!(SplitLayout::Vertical.side_by_side(1920, 1080));
        assert!(!a.side_by_side(0, 0));
    }

    #[test]
    fn values_cycle_and_read_back() {
        assert_eq!(SplitLayout::Auto.step(1), SplitLayout::Horizontal);
        assert_eq!(SplitLayout::Vertical.step(1), SplitLayout::Auto);
        assert_eq!(SplitLayout::Auto.step(-1), SplitLayout::Vertical);
        for v in SplitLayout::ALL {
            assert_eq!(SplitLayout::from_text(v.key_text()), v);
        }
        assert_eq!(SplitLayout::from_text("vertical "), SplitLayout::Vertical);
        assert_eq!(SplitLayout::from_text("?"), SplitLayout::Auto);
    }

    #[test]
    fn top_half_goes_left_bottom_half_right() {
        assert_eq!(side_by_side_fraction([0.0, 0.0, 1.0, 0.5]), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(side_by_side_fraction([0.0, 0.5, 1.0, 0.5]), [0.5, 0.0, 0.5, 1.0]);
    }
}
