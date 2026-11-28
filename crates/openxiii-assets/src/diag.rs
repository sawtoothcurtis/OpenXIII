//! Optional diagnostics of openxiii-assets. None of them is built here: each one answers
//! "off" as a constant.

pub(crate) fn lightmap_surface() -> Option<usize> {
    None
}

pub(crate) fn static_mesh_actor() -> Option<&'static str> {
    None
}

pub(crate) fn texture_replacements() -> bool {
    false
}

pub(crate) fn check_materials() -> bool {
    false
}
