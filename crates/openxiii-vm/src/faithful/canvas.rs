//! Canvas extensions in the original edition.
use crate::{ObjId, Vm};

impl Vm {
    #[inline]
    pub(crate) fn menu_tile_style(&self, _material: &str, style: u8, _xl: f32, _yl: f32, _ul: f32) -> u8 {
        style
    }

    #[inline]
    pub(crate) fn begin_message_bands(&self) {}
    #[inline]
    pub(crate) fn finish_message_bands(&mut self, _start: ()) {}

    #[inline]
    pub(crate) fn begin_canvas_text(&mut self, _canvas: ObjId) {}
    #[inline]
    pub(crate) fn finish_canvas_text(&mut self, _canvas: ObjId, _note: ()) {}

    #[inline]
    pub(crate) fn reset_loading_origin(&mut self, _canvas: ObjId) {}
    #[inline]
    pub(crate) fn coop_view_separator(&mut self, _pc: ObjId, _hud: ObjId, _canvas: ObjId) {}

    #[inline]
    pub(crate) fn hud_scale_begin(&mut self, _canvas: ObjId) {}
    #[inline]
    pub(crate) fn hud_scale_end(&mut self, _canvas: ObjId, _scale: ()) {}
}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn menus_keep_the_original_viewport_in_low_wide_windows() {
        let mut vm = Vm::new();
        for size in [(1024, 576), (854, 480), (1024, 768)] {
            vm.viewport = size;
            let scale = vm.hud_scale_begin(0);
            vm.hud_scale_end(0, scale);
            assert_eq!(vm.viewport, size);
        }
    }
}
