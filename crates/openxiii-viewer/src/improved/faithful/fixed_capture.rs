fn start_capture_level(_g: &mut game::Game) {}

fn scale_menu_art(_r: &mut Renderer, _vm: &openxiii_vm::Vm) {}

fn draw_capture_mods_menu(_g: &mut game::Game, _w: u32, _h: u32, _quads: &mut Vec<openxiii_vm::canvas::HudDraw>) {}

fn apply_configured_render_quality(r: &mut Renderer, _config: &mut openxiii_vm::config::ConfigCache) {
    r.set_render_quality(render_quality::RenderQuality::original());
}
