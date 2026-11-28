fn apply_fullscreen_fallback(_window: &Window, _vm: &mut openxiii_vm::Vm, _x: u32, _y: u32, _borderless: bool) {}

fn present_after_resize(_r: &mut Renderer, _game_loaded: bool, _size: winit::dpi::PhysicalSize<u32>) {}

fn present_before_mode_switch(_g: &game::Game, _renderer: Option<&mut Renderer>) {}

fn first_launch_dimensions(_native: Option<(u32, u32)>, _first_launch: &mut Option<(u32, u32)>, _fs: bool, x: i32, y: i32) -> (i32, i32) {
    (x, y)
}

impl App {
    #[cfg(not(windows))]
    fn start_install_pictures(&mut self, _window: &Window) {}

    fn recap_consumes_event(&mut self, _event: &WindowEvent) -> bool {
        false
    }

    fn draw_loading_screen(&mut self) -> bool {
        false
    }

    fn leave_screen_clean(&mut self) {}
}
