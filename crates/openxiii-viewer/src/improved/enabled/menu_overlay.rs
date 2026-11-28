fn menu_consumes_button(pressed: bool) -> bool {
    pressed
}

fn play_mods_menu_ticks(viewer: &mut ViewerState, game: Option<&mut game::Game>) {
    let ticks = viewer.mods_page.take_ticks();
    if let Some(g) = game {
        for _ in 0..ticks {
            g.vm.play_menu_sound(openxiii_vm::menu_sounds::MenuSound::Tick);
        }
    }
}

fn enable_gamepad_vibration(vm: &mut openxiii_vm::Vm, gamepad: bool, wanted: bool) {
    if pad::vibration_manager_wanted(true, gamepad, wanted) {
        vm.enable_vibrations();
    }
}

impl App {
    fn mods_key(&mut self, code: KeyCode, pressed: bool) -> bool {
        if self.viewer.mods_page.open && pressed {
            let key = match code {
                KeyCode::ArrowUp => Some(mods_page::Key::Up),
                KeyCode::ArrowDown => Some(mods_page::Key::Down),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Some(mods_page::Key::Toggle),
                KeyCode::Escape | KeyCode::Backspace => Some(mods_page::Key::Back),
                _ => None,
            };
            if let Some(k) = key {
                self.viewer.mods_page.key(k);
            }
            return true;
        }
        false
    }

    fn mods_mouse_button(&mut self, button: MouseButton, state: ElementState) -> bool {
        if button == MouseButton::Left && self.viewer.mods_page.click(state == ElementState::Pressed) {
            return true;
        }
        if self.viewer.mods_page.open && state == ElementState::Pressed {
            return true;
        }
        false
    }

    fn mods_mouse_wheel(&mut self, z: i32) -> bool {
        if self.viewer.mods_page.open {
            self.viewer.mods_page.wheel(z as f32 / 120.0);
            return true;
        }
        false
    }

    fn mods_window_touch(&mut self, id: u64, phase: choice_level::Finger, pos: (f32, f32)) -> bool {
        if self.viewer.mods_page.touch(id, phase, {
            // The finger on the page the Mods page is laid out on (gui.rs, MenuScreen).
            let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
            self.game.as_ref().map_or(pos, |g| g.vm.menu_screen(size.width.max(1), size.height.max(1)).to_page(pos))
        }) {
            return true;
        }
        false
    }

    fn open_requested_gamepad_page(&mut self) {
        if let Some(slot) = self.game.as_mut().and_then(|g| g.vm.take_gamepad_settings_request()) {
            self.open_gamepad_settings_slot(slot);
        }
    }

    fn start_requested_level(&mut self) {
        if let Some(url) = self.game.as_mut().and_then(|g| g.vm.take_select_level_start()) {
            self.leave_to(&url);
        }
    }
}

impl ViewerState {
    fn draw_mods_menu(
        &mut self,
        g: &mut game::Game,
        pads: Option<&pad::Pads>,
        size: winit::dpi::PhysicalSize<u32>,
        quads: &mut Vec<openxiii_vm::canvas::HudDraw>,
    ) {
        match g.vm.take_menu_action() {
            Some(openxiii_vm::menu_boxes::MenuAction::Mods) => self.mods_page.open(),
            None => {}
        }
        let gamepad = pads.is_some_and(|p| p.count() > 0);
        // Laid out on the menus' page and placed like them (gui.rs, MenuScreen).
        let screen = g.vm.menu_screen(size.width.max(1), size.height.max(1));
        let start = quads.len();
        self.mods_page.draw(&mut g.vm, screen.size.0, screen.size.1, gamepad, quads);
        screen.place(&mut quads[start..]);
        // Full-screen page: the menu behind is not hovered while it is open.
        g.vm.set_menu_overlay(self.mods_page.open);
    }

    fn route_menu_gamepad(&mut self, choice: &mut choice_level::ChoiceLevel, pads: &pad::Pads, dt: f32) -> (Option<String>, bool) {
        match self.mods_page.pad(pads.menu()) {
            true if self.mods_page.open => (None, true),
            mods_mask => {
                let (url, mask_value) = choice.pad(pads.menu(), dt);
                (url, mask_value || mods_mask)
            }
        }
    }
}
