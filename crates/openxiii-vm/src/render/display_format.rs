//! Screens that are not 4:3 (improved build only, never in the faithful build). The original game
//! only knows 4:3 modes (UD3DRenderDevice::GetAvailableRes, see `available_res`): its projection
//! keeps the player's FovAngle across the width, so a wider window cuts the top and
//! the bottom of the view, and its menus are laid out on a 640 × 480 grid stretched to the window.
//!
//! - 3D view, "Hor+": the vertical field of view is the one a 4:3 window of the same height would
//!   have, the extra width shows more on the sides. Narrower than 4:3 (portrait phone), nothing
//!   changes: the width keeps FovAngle and the height grows. Only the projection changes (render,
//!   visibility, WorldToScreen); FovAngle and every other value of the game stay as they are.
//! - Menus: drawn in the largest 4:3 frame centred in the window (gui.rs, `menu_frame`).
//! - HUD: already anchored to the margins with fixed pixel sizes; the parts the scripts scale by
//!   ClipX / 640 or by the window's ratio are laid out as on a 4:3 screen of the same height.
//!
//! In a 4:3 window every function here returns "no change", so the output stays identical.

/// Is the wide-screen display active? Faithful build: never; improved build with the Video
/// menu's Graphics: Original (the original's look) neither. Also off in this crate's unit tests:
/// the tests compare the menus and the view with the original's, which only knows 4:3.
#[allow(unused_imports)]
use openxiii_math::SimMath as _;
pub use crate::improved::visual_effects::wide_display as active;

/// Hor+ factor of a `width` × `height` window: the tangent of the horizontal half angle
/// (FovAngle / 2) is multiplied by it, which keeps the vertical field of view of a 4:3 window of
/// the same height. `None` when the display is off or the window is 4:3 or narrower.
pub fn hor_plus(width: u32, height: u32, graphics: crate::video_settings::SessionGraphics) -> Option<f32> {
    if !active(graphics) {
        return None;
    }
    hor_plus_factor(width, height)
}

/// `hor_plus` without the build check: window ratio / (4 / 3), `None` up to 4:3.
pub fn hor_plus_factor(width: u32, height: u32) -> Option<f32> {
    if width == 0 || height == 0 || u64::from(width) * 3 <= u64::from(height) * 4 {
        return None;
    }
    Some(width as f32 / height as f32 * 0.75)
}

/// Horizontal half angle (radians) widened by `hor_plus` for this window; the angle itself when
/// nothing changes (bit for bit).
pub fn widen_half(half: f32, window: (u32, u32), graphics: crate::video_settings::SessionGraphics) -> f32 {
    match hor_plus(window.0, window.1, graphics) {
        Some(k) => ((half as f64).dtan() * k as f64).datan() as f32,
        None => half,
    }
}

/// Horizontal field of view in degrees widened by `hor_plus`; `fov` itself when nothing changes.
pub fn widen_fov(fov: f32, window: (u32, u32), graphics: crate::video_settings::SessionGraphics) -> f32 {
    match hor_plus(window.0, window.1, graphics) {
        Some(_) => widen_half(fov.to_radians() * 0.5, window, graphics).to_degrees() * 2.0,
        None => fov,
    }
}

/// Ratio of a view of `view` pixels as the same layout would have in a 4:3 window of the same
/// height (a split-screen half of a 16:9 window → the half of a 4:3 window). The view's own ratio
/// when nothing changes.
pub fn ratio_as_43(view: (f32, f32), window: (u32, u32), graphics: crate::video_settings::SessionGraphics) -> f32 {
    let ratio = view.0 / view.1.max(1.0);
    match hor_plus(window.0, window.1, graphics) {
        Some(k) => ratio / k,
        None => ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_of_common_screens() {
        assert_eq!(hor_plus_factor(1280, 960), None);
        assert_eq!(hor_plus_factor(640, 480), None);
        assert_eq!(hor_plus_factor(1080, 2400), None);
        assert!((hor_plus_factor(1920, 1080).unwrap() - 4.0 / 3.0).abs() < 1e-6);
        assert!((hor_plus_factor(1920, 1200).unwrap() - 1.2).abs() < 1e-6);
        assert!((hor_plus_factor(2560, 1080).unwrap() - 1.777_777_8).abs() < 1e-6);
    }

    #[test]
    fn vertical_field_kept() {
        // 85° across a 4:3 window: vertical tangent tan(42.5°) / (4/3). In 16:9 with the factor,
        // the vertical tangent is the same.
        let t = 42.5f64.to_radians().dtan();
        let k = hor_plus_factor(1920, 1080).unwrap() as f64;
        let vertical_16_9 = t * k / (1920.0 / 1080.0);
        assert!((vertical_16_9 - t * 0.75).abs() < 1e-6);
    }

    #[test]
    fn faithful_build_never_changes() {
        // These crate tests are built without the improved feature unless asked for.
        if !cfg!(feature = "improved") {
            assert_eq!(hor_plus(1920, 1080, Default::default()), None);
            assert_eq!(widen_fov(85.0, (1920, 1080), Default::default()).to_bits(), 85f32.to_bits());
        }
        assert_eq!(widen_half(0.7, (1280, 960), Default::default()).to_bits(), 0.7f32.to_bits());
    }
}
