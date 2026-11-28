fn start_capture_level(g: &mut game::Game) {
    if let Some(url) = g.vm.take_select_level_start() {
        println!("select level page {url}: {}", g.vm.from_menu_main(&url));
    }
}

fn scale_menu_art(r: &mut Renderer, vm: &openxiii_vm::Vm) {
    let (w, h) = r.screen_size();
    r.set_onomatopoeia_scale(vm.window_menu_frame(w as i32, h as i32).map_or(1.0, |f| f.scale));
}

fn draw_capture_mods_menu(g: &mut game::Game, w: u32, h: u32, quads: &mut Vec<openxiii_vm::canvas::HudDraw>) {
    match g.vm.take_menu_action() {
        Some(openxiii_vm::menu_boxes::MenuAction::Mods) => {
            let mut page = mods_page::ModsPage::default();
            page.open();
            // Laid out on the menus' page and placed like them, as in the window.
            let screen = g.vm.menu_screen(w.max(1), h.max(1));
            let start = quads.len();
            page.draw(&mut g.vm, screen.size.0, screen.size.1, false, quads);
            screen.place(&mut quads[start..]);
        }
        None => {}
    }
}

fn use_menu_master_textures() -> bool {
    openxiii_vm::video_settings::graphics_enhanced_local() && onomatopoeias::enabled()
}

fn apply_configured_render_quality(r: &mut Renderer, config: &mut openxiii_vm::config::ConfigCache) {
    let quality = if openxiii_vm::video_settings::graphics_enhanced_local() {
        render_quality::RenderQuality::from_config(config)
    } else {
        render_quality::RenderQuality::original()
    };
    r.set_render_quality(quality);
}
