fn target_scale(_world_h: u32, _max_texture: u32) -> u32 {
    1
}

fn fx_scene_side(_world_h: u32, _max_texture: u32) -> u32 {
    crate::screen_fx::SIDE
}

fn shadow_target_size(_world_h: u32, _max_texture: u32, _quality: u32) -> u32 {
    SHADOW_TARGET_SIZE
}

#[cfg(test)]
#[test]
fn render_textures_follow_the_world_height_only_in_the_improved_build() {
    let sides: Vec<u32> = [480, 512, 720, 1080, 2160].iter().map(|&h| fx_scene_side(h, 16384)).collect();
    assert!(sides.iter().all(|&s| s == crate::screen_fx::SIDE));
    let scales: Vec<u32> = [480, 600, 720, 1080, 1440, 2160, 4320].iter().map(|&h| target_scale(h, 16384)).collect();
    assert!(scales.iter().all(|&s| s == 1));
}

#[cfg(test)]
#[test]
fn shadow_targets_follow_the_world_height_only_in_the_improved_build() {
    let sizes: Vec<u32> = [480, 600, 720, 1080, 1440, 1620, 2160, 3240, 8640].iter().map(|&h| shadow_target_size(h, 16384, 0)).collect();
    assert!(sizes.iter().all(|&s| s == SHADOW_TARGET_SIZE));
    assert_eq!(shadow_target_size(2160, 16384, 2048), SHADOW_TARGET_SIZE);
}

fn held_frame_usage() -> wgpu::TextureUsages {
    wgpu::TextureUsages::empty()
}

fn prepare_canvas_textures() {}

fn resolve_canvas_textures(_pass: &mut openxiii_assets::material::Pass) {}

fn sample_enlarged_text(_point: bool, _glyph: bool, _enlarged: bool, _: openxiii_vm::video_settings::SessionGraphics) -> bool {
    false
}

fn set_canvas_filter(_uniform: &mut PassUniform, _pass: &openxiii_assets::material::Pass, _point: bool, _text_bicubic: bool) {}

fn configured_shadow_target_size(world_h: u32, max_texture: u32, quality: u32) -> u32 {
    shadow_target_size(world_h, max_texture, quality)
}

struct RenderTargetSizes;

impl RenderTargetSizes {
    fn local(_: &Renderer) -> Self {
        Self
    }
    fn panel_scale(&self, world_h: u32, max_texture: u32) -> u32 {
        target_scale(world_h, max_texture)
    }
    fn effects_side(&self, world_h: u32, max_texture: u32) -> u32 {
        fx_scene_side(world_h, max_texture)
    }
}

/// The original's gamma ramp: full screen only, as Direct3D 8 does (gamma.rs).
pub(crate) fn gamma_in_window(_graphics: openxiii_vm::video_settings::SessionGraphics) -> bool {
    false
}
