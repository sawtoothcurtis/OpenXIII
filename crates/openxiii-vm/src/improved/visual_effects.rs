//! Visual fixes governed by the effective graphics setting.
pub fn wide_display(graphics: crate::video_settings::SessionGraphics) -> bool {
    if cfg!(test) || !graphics.enhanced() {
        return false;
    }
    true
}

pub fn side_by_side(width: i32, height: i32, graphics: crate::video_settings::SessionGraphics) -> bool {
    !cfg!(test) && graphics.enhanced() && crate::split_layout::split_layout().side_by_side(width, height)
}

pub(crate) fn repair_focus_hints(graphics: crate::video_settings::SessionGraphics) -> bool {
    graphics.enhanced()
}

pub(crate) fn angle_effects(graphics: crate::video_settings::SessionGraphics) -> bool {
    let on = !cfg!(test) && graphics.enhanced();
    on
}
