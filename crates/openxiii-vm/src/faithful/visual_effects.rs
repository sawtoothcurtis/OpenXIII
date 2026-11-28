//! Original visual behavior. Unit-test overrides exercise the shared implementations.
pub fn wide_display(_: crate::video_settings::SessionGraphics) -> bool {
    false
}

pub fn side_by_side(_: i32, _: i32, _: crate::video_settings::SessionGraphics) -> bool {
    false
}

pub(crate) fn repair_focus_hints(_: crate::video_settings::SessionGraphics) -> bool {
    cfg!(test)
}

pub(crate) fn angle_effects(_: crate::video_settings::SessionGraphics) -> bool {
    false
}
