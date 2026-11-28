#[cfg(windows)]
/// The primary screen's current mode (its native size on a desktop), in real pixels whatever
/// the display scaling: EnumDisplaySettingsW(ENUM_CURRENT_SETTINGS).
fn native_display_size() -> Option<(u32, u32)> {
    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplaySettingsW(device: *const u16, mode: u32, devmode: *mut u8) -> i32;
    }
    // DEVMODEW (220 bytes): dmSize at 68, dmPelsWidth at 172, dmPelsHeight at 176.
    // Words, for the structure's alignment: dmSize is the low half of word 17.
    let mut devmode = [0u32; 55];
    devmode[17] = 220;
    // SAFETY: a zeroed DEVMODEW of the size it declares; a null device is the primary display.
    let ok = unsafe { EnumDisplaySettingsW(std::ptr::null(), u32::MAX, devmode.as_mut_ptr().cast()) };
    let (w, h) = (devmode[43], devmode[44]);
    (ok != 0 && w > 0 && h > 0).then_some((w, h))
}

fn apply_fullscreen_fallback(window: &Window, vm: &mut openxiii_vm::Vm, x: u32, y: u32, borderless: bool) {
    if !vm.fullscreen && !borderless {
        vm.fullscreen = set_fullscreen(window, x, y, true);
    }
}

fn present_after_resize(r: &mut Renderer, game_loaded: bool, size: winit::dpi::PhysicalSize<u32>) {
    if game_loaded && size.width > 0 && size.height > 0 {
        for _ in 0..6 {
            if let Ok(Some((frame, _view))) = r.begin_frame() {
                r.present(frame);
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
    }
}

fn present_before_mode_switch(g: &game::Game, renderer: Option<&mut Renderer>) {
    if g.vm.pending_display_mode.is_some() || g.vm.pending_resize.is_some() {
        if let Some(r) = renderer {
            for _ in 0..6 {
                if let Ok(Some((frame, _view))) = r.begin_frame() {
                    r.present(frame);
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
    }
}

impl App {
    #[cfg(not(windows))]
    /// Improved build, Android and Linux: one line in a bubble on a black screen (the menus'
    /// pictures being made on a first start), presented once.
    fn text_frame(&mut self, text: &str) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) else { return };
        g.vm.loading_text_frame(text, size.width.max(1), size.height.max(1));
        let saved = r.only_post_render;
        r.only_post_render = true;
        r.set_hud(&g.vm.hud);
        if let Err(e) = r.render_to_surface(&self.camera, library) {
            eprintln!("render: {e}");
        }
        r.only_post_render = saved;
    }

    #[cfg(not(windows))]
    fn start_install_pictures(&mut self, window: &Window) {
        if std::mem::take(&mut self.viewer.install_wanted) {
            let video_dir = self.game.as_ref().map_or_else(|| self.options.game.join("Video"), |g| g.vm.engine.video.dir.clone());
            self.viewer.installing =
                install_pictures::Running::start(&self.options.game, &video_dir, &launcher::folder_data().join("cache"), &self.display_modes);
            if self.viewer.installing.is_none() {
                onomatopoeias::release();
            }
            window.request_redraw();
        }
    }

    fn recap_consumes_event(&mut self, event: &WindowEvent) -> bool {
        let alt_shortcut = matches!(&event, WindowEvent::KeyboardInput { event, .. }
            if self.alt_down || matches!(event.physical_key, PhysicalKey::Code(KeyCode::AltLeft | KeyCode::AltRight)));
        if self.viewer.recap_swallow
            && !alt_shortcut
            && match &event {
                WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
                WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
                _ => false,
            }
        {
            return true;
        }
        if let Some(w) = self.viewer.recap_wait.as_mut().filter(|_| !alt_shortcut) {
            let press = match &event {
                WindowEvent::KeyboardInput { event, is_synthetic, .. } => event.state == ElementState::Pressed && !event.repeat && !is_synthetic,
                WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
                WindowEvent::Touch(t) => t.phase == winit::event::TouchPhase::Started,
                _ => false,
            };
            if press {
                // The mod decides (on("loading_key")); released, at once: the level starts if
                // prepared, the loading animation replaces the mod's screen otherwise.
                if !w.flow.pressed && self.game.as_mut().is_some_and(|g| g.vm.mod_loading_key()) {
                    w.flow.press();
                    w.pressed_at.get_or_insert_with(Instant::now);
                }
                self.recap_step();
                return true;
            }
        }
        false
    }

    fn draw_loading_screen(&mut self) -> bool {
        if self.viewer.recap_wait.is_some() {
            self.recap_step();
            return true;
        }
        #[cfg(not(windows))]
        if self.viewer.installing.is_some() {
            let (percent, finished) = self.viewer.installing.as_ref().map_or((100, true), |r| (r.percent(), r.finished()));
            if finished {
                self.viewer.installing = None;
                self.last = Instant::now();
            } else {
                // Android: the same percent on the Java loading screen laid over the game.
                #[cfg(target_os = "android")]
                crate::touch_controls::set_loading(percent.min(100) as i32);
                self.text_frame(&format!("Loading... {percent} %"));
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            return true;
        }
        false
    }

    /// Improved build, on quitting: a few black frames, spaced over several screen refreshes,
    /// then the window hidden before the swap chain goes. In borderless and exclusive fullscreen
    /// the last frame (the "Quit?" box) otherwise stayed on the screen, dirty, while the display
    /// went back to the desktop.
    fn leave_screen_clean(&mut self) {
        // About 300 ms of black at a steady rate: a screen with a variable refresh rate
        // (G-SYNC) settles on it instead of holding the last menu frame.
        if let Some(r) = self.renderer.as_mut() {
            for _ in 0..18 {
                if let Ok(Some((frame, _view))) = r.begin_frame() {
                    r.present(frame);
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
        if let Some(w) = &self.window {
            w.set_visible(false);
        }
    }
}

fn first_launch_dimensions(native: Option<(u32, u32)>, first_launch: &mut Option<(u32, u32)>, fs: bool, x: i32, y: i32) -> (i32, i32) {
    match native {
        Some((w, h)) if fs && openxiii_vm::video_settings::take_first_launch_size_pending() => {
            #[cfg(target_os = "android")]
            let (w, h) = openxiii_vm::video_settings::android_first_launch_size((w, h));
            *first_launch = Some((w, h));
            (w as i32, h as i32)
        }
        _ => (x, y),
    }
}
