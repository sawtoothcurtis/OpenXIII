fn menu_consumes_button(_pressed: bool) -> bool {
    true
}

fn play_mods_menu_ticks(_viewer: &mut ViewerState, _game: Option<&mut game::Game>) {}

fn enable_gamepad_vibration(_vm: &mut openxiii_vm::Vm, _gamepad: bool, _wanted: bool) {}

impl App {
    fn mods_key(&mut self, _code: KeyCode, _pressed: bool) -> bool {
        false
    }

    fn mods_mouse_button(&mut self, _button: MouseButton, _state: ElementState) -> bool {
        false
    }

    fn mods_mouse_wheel(&mut self, _z: i32) -> bool {
        false
    }

    fn mods_window_touch(&mut self, _id: u64, _phase: choice_level::Finger, _pos: (f32, f32)) -> bool {
        false
    }

    fn open_requested_gamepad_page(&mut self) {}

    fn start_requested_level(&mut self) {}
}

impl ViewerState {
    fn draw_mods_menu(
        &mut self,
        _g: &mut game::Game,
        _pads: Option<&pad::Pads>,
        _size: winit::dpi::PhysicalSize<u32>,
        _quads: &mut Vec<openxiii_vm::canvas::HudDraw>,
    ) {
    }

    fn route_menu_gamepad(&mut self, choice: &mut choice_level::ChoiceLevel, pads: &pad::Pads, dt: f32) -> (Option<String>, bool) {
        choice.pad(pads.menu(), dt)
    }
}
