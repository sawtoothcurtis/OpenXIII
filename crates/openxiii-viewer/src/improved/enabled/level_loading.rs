/// Improved build: a level's world for a travel, on a worker thread: its movers found first (they are drawn as
/// actors, not as static geometry), then `load`. Both only read the game's files: the game
/// thread keeps drawing the loading picture meanwhile (finding the movers loads every System
/// package into a VM of its own, about half a second, which used to run on the game thread).
fn load_level(game: &Path, map: &str) -> Loaded {
    let exclude = game::Game::mover_names(game, map).unwrap_or_default();
    load(game, map, &exclude)
}

/// Improved build: a map change held by a mod's script (`hold_start()` in its `on("loading")`
/// handler, such as the example mod of the console version's recap bubbles). The map loads in its
/// thread while the mod's loading screen stays on screen, then the level is built and warmed up
/// behind it, and the mod is told (`on("loaded")`). Each new press (key, mouse button, touch or
/// gamepad button) goes to the mod (`on("loading_key")`); once it releases the start the level
/// starts at once if prepared, otherwise the usual loading animation replaces the mod's screen and
/// the level starts by itself when ready (loading_recap::RecapFlow).
struct RecapWait {
    /// The travel, until the level is built (finish_travel).
    req: Option<openxiii_vm::travel::TravelRequest>,
    map: String,
    job: Option<std::thread::JoinHandle<Loaded>>,
    loaded: Option<Loaded>,
    /// Press and preparation state.
    flow: openxiii_vm::loading_recap::RecapFlow,
    /// When the first press came.
    pressed_at: Option<Instant>,
    /// The loading animation took the bubbles' place (a press before the map was read).
    anim_started: bool,
    /// Gamepad buttons down at the previous poll (buttons held from before are not a press).
    pad_down: u32,
    /// Last recap frame drawn: time and whether it had the "ready" line.
    drawn: Option<(Instant, bool)>,
    /// Next frame of the loading animation (after the press, map still loading).
    next_anim: Instant,
}

impl App {
    fn draw_loading_picture(&mut self) {
        if let Some(r) = self.renderer.as_mut() {
            r.hold_capture = true;
        }
        self.draw_frame_in(0.0, true);
    }
}

fn level_exclusions(_game: &Path, _map: &str) {}

fn load_travel_level(game: &Path, map: &str, _exclusions: &()) -> Loaded {
    load_level(game, map)
}

fn refresh_level_graphics(g: &mut game::Game, game_dir: &Path) {
    g.vm.refresh_graphics();
    onomatopoeias::start(game_dir.to_path_buf());
}

impl App {
    /// Improved build: the loading screen a mod holds (black screen, the items its loading
    /// handlers made as the VM's Canvas primitives, no scene), presented once.
    fn recap_frame(&mut self) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) else { return };
        g.vm.mod_loading_frame(size.width.max(1), size.height.max(1));
        let saved = r.only_post_render;
        r.only_post_render = true;
        r.set_hud(&g.vm.hud);
        if let Err(e) = r.render_to_surface(&self.camera, library) {
            eprintln!("render: {e}");
        }
        r.only_post_render = saved;
    }

    /// Improved build: gamepad buttons down now (triggers count as buttons), as bits.
    fn recap_pad_buttons(&mut self) -> u32 {
        #[cfg(not(target_os = "android"))]
        let merged = self.pads.as_mut().and_then(|p| {
            p.poll();
            p.merged()
        });
        #[cfg(target_os = "android")]
        let merged = pad::merged();
        merged.map_or(0, |(axes, bits)| bits | (u32::from(axes[4] > 0.5) << 30) | (u32::from(axes[5] > 0.5) << 31))
    }

    /// Improved build: one step of the wait behind the recap bubbles (called on each redraw while
    /// it lasts, and on a press): load finished, new gamepad press, then what the wait asks
    /// (loading_recap::RecapFlow): the bubbles, the level prepared behind them, the loading
    /// animation after an early press, or the start; then a short sleep so that the wait does not
    /// spin the processor.
    fn recap_step(&mut self) {
        use openxiii_vm::loading_recap::RecapStep;
        let Some(mut w) = self.viewer.recap_wait.take() else { return };
        if w.loaded.is_none() && w.job.as_ref().is_some_and(|j| j.is_finished()) {
            w.loaded = w.job.take().map(|j| j.join().unwrap_or_else(|_| Err("loading: panic".to_string())));
        }
        let pads = self.recap_pad_buttons();
        let mut press = pads & !w.pad_down != 0;
        w.pad_down = pads;
        // Android: the fingers and the touch controls come through NativeInput, not the window.
        #[cfg(target_os = "android")]
        {
            press |= touch_controls::take_recap_press();
        }
        // A press goes to the mod (on("loading_key")), which may release the start.
        if press && !w.flow.pressed && self.game.as_mut().is_some_and(|g| g.vm.mod_loading_key()) {
            w.flow.press();
            w.pressed_at = Some(Instant::now());
        }
        match w.flow.step(w.loaded.is_some()) {
            RecapStep::Start => {
                self.recap_start(&w);
                return;
            }
            RecapStep::PrepareBehindBubbles => {
                // The level built behind the mod's screen (shown again by another thread, so that
                // the screen never waits on one frame), warmed up, the graphics card idle: as a
                // travel without it, but before the release.
                if let Some(r) = self.renderer.as_mut() {
                    r.hold_clear();
                    r.hold_capture = true;
                }
                self.recap_frame();
                // This thread is blocked meanwhile: another one watches the presses, and on one
                // the hold shows the loading animation (made off screen once the hold starts)
                // instead of the bubbles; the level then starts as soon as it is prepared.
                let watch = self.renderer.as_ref().and_then(|r| press_watch::PressWatch::start(r.hold_switch.clone()));
                // (Kept now: the level gets a renderer of its own.)
                let shown_at = self.renderer.as_ref().map(|r| r.hold_switched_at.clone());
                self.recap_prepare(&mut w, true);
                #[allow(unused_mut)]
                let mut pressed_at = watch.and_then(press_watch::PressWatch::stop);
                // The level is ready: the mod is told (on("loaded"): a prompt...), then given
                // the press made meanwhile.
                let map = w.map.clone();
                if let Some(g) = self.game.as_mut() {
                    g.vm.mod_loading_loaded(&map);
                }
                let released = pressed_at.is_some() && self.game.as_mut().is_some_and(|g| g.vm.mod_loading_key());
                if let Some(at) = pressed_at.filter(|_| released) {
                    let shown = shown_at.and_then(|a| a.lock().ok().and_then(|a| *a));
                    match shown {
                        Some(s) => renderer::diag(&format!(
                            "load: press during the preparation, loading animation shown {:.1} ms after it",
                            s.saturating_duration_since(at).as_secs_f64() * 1000.0
                        )),
                        None => renderer::diag("load: press during the preparation, prepared before the loading animation could be shown"),
                    }
                    w.flow.press();
                    w.pressed_at = Some(at);
                    // The same press, read by the window once this thread is free, is not the
                    // game's.
                    self.viewer.recap_swallow = true;
                    self.recap_start(&w);
                    return;
                }
                renderer::diag(&format!(
                    "load: {} prepared behind the mod's loading screen {} ms after the travel began, waiting for its release",
                    w.map,
                    self.travel_started.elapsed().as_millis()
                ));
                // What the mod shows once ready, at once (a press made meanwhile arrives next).
                self.recap_frame();
                w.drawn = Some((Instant::now(), true));
            }
            RecapStep::PrepareThenStart => {
                // Pressed before the map file was read: as a travel without bubbles, the loading
                // animation's frames (kept since the press, a few more made off screen) shown in
                // turn while the level is built and warmed up, then the level starts by itself.
                if let Some(r) = self.renderer.as_mut() {
                    if !w.anim_started {
                        r.hold_clear();
                    }
                    r.hold_capture = true;
                }
                if !w.anim_started {
                    // Read before the first animation step: one frame of it to be kept.
                    let mut anim = self.loading_anim.take();
                    if let Some(a) = anim.as_mut() {
                        self.loading_frame(a);
                    }
                    self.loading_anim = anim;
                }
                self.recap_prepare(&mut w, false);
                self.recap_start(&w);
                return;
            }
            RecapStep::LoadingAnimation => {
                // The usual loading animation until the map file is read, its frames kept for the
                // hold while the level is built.
                let now = Instant::now();
                if !w.anim_started {
                    w.anim_started = true;
                    w.next_anim = now;
                    if let Some(r) = self.renderer.as_mut() {
                        r.hold_clear();
                    }
                }
                if now >= w.next_anim {
                    let mut anim = self.loading_anim.take();
                    if let Some(r) = self.renderer.as_mut() {
                        r.hold_capture = true;
                    }
                    if let Some(a) = anim.as_mut() {
                        self.loading_frame(a);
                        w.next_anim = now + std::time::Duration::from_millis(a.pause_ms());
                    }
                    if let Some(r) = self.renderer.as_mut() {
                        r.hold_capture = false;
                    }
                    self.loading_anim = anim;
                }
            }
            RecapStep::Bubbles { ready } => {
                // Drawn again when the prompt appears and now and then for a resized window.
                if w.drawn.is_none_or(|(t, r)| r != ready || t.elapsed() >= std::time::Duration::from_millis(250)) {
                    self.recap_frame();
                    w.drawn = Some((Instant::now(), ready));
                }
            }
        }
        // (A wait begun meanwhile, by a travel of the level's first step, replaces this one.)
        if self.viewer.recap_wait.is_none() {
            self.viewer.recap_wait = Some(w);
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }

    /// Improved build: the level of the recap's travel built (finish_travel) and warmed up while
    /// the frames kept so far (bubbles or loading animation, `hold_capture` on) are shown again by
    /// another thread; a few more animation frames are made off screen first, as in `travel`:
    /// joining the loading animation's turn, or (`bubbles`) kept apart for a press during the
    /// preparation (Renderer::hold_switch). Its sounds wait for the start (Audio::held): the level
    /// does not run behind the bubbles.
    fn recap_prepare(&mut self, w: &mut RecapWait, bubbles: bool) {
        self.travel_hold = self.renderer.as_mut().and_then(|r| r.hold_start(100));
        if self.travel_hold.is_some() {
            let mut a = self.loading_anim.take();
            if let Some(r) = self.renderer.as_mut() {
                r.warming = true;
                r.hold_capture_alt = bubbles;
            }
            if let Some(a) = a.as_mut() {
                for _ in 0..8 {
                    self.loading_frame(a);
                }
            }
            if let Some(r) = self.renderer.as_mut() {
                r.warming = false;
                r.hold_capture_alt = false;
            }
            self.loading_anim = a;
        }
        if let Some(r) = self.renderer.as_mut() {
            r.hold_capture = false;
        }
        let (Some(req), Some(loaded)) = (w.req.take(), w.loaded.take()) else {
            if let (Some(hold), Some(r)) = (self.travel_hold.take(), self.renderer.as_mut()) {
                r.hold_stop(hold);
            }
            return;
        };
        self.finish_travel(req, w.map.clone(), loaded);
        if let Some(a) = self.audio.as_mut() {
            a.held = true;
        }
        let hold = self.travel_hold.take();
        self.warm_after_travel(hold);
        w.flow.prepared = true;
    }

    /// Improved build: the end of the recap's wait, the level prepared: it runs from the next
    /// frame, its sounds start, the time spent behind the bubbles is not game time.
    fn recap_start(&mut self, w: &RecapWait) {
        if let Some(a) = self.audio.as_mut() {
            if a.held {
                let waiting =
                    self.game.as_ref().and_then(|g| g.vm.audio.as_ref()).map_or(0, |hx| hx.engine.voices.iter().flatten().filter(|v| v.playing).count());
                renderer::diag(&format!("load: {waiting} voices of the level held until the start"));
            }
            a.held = false;
        }
        self.viewer.recap_press = w.pressed_at;
        if let Some(g) = self.game.as_mut() {
            g.vm.mod_loading_end();
        }
        self.run_loop.old_time = counter();
        self.last = Instant::now();
        renderer::diag(&format!("load: {} started after the recap, {} ms after the travel began", w.map, self.travel_started.elapsed().as_millis()));
        if let Some(win) = &self.window {
            win.request_redraw();
        }
    }

    /// Improved build: once everyone confirmed on the co-op character screen, the level it starts
    /// has its packages read on another thread during the confirmation animations (the part of
    /// a travel that needs no game state: about 100 ms of Banque01's 830); `travel` takes them.
    /// A confirmation taken back drops the reading (the thread ends on its own).
    fn preload_step(&mut self) {
        let wanted = self.game.as_ref().and_then(|g| g.vm.character_screen_starting()).map(|url| map_stem(&url));
        match (&self.viewer.preload, wanted) {
            (Some((m, _)), Some(w)) if m.eq_ignore_ascii_case(&w) => {}
            (_, Some(w)) => {
                let dir = self.options.game.clone();
                let stem = w.clone();
                self.viewer.preload = Some((w, std::thread::spawn(move || load_level(&dir, &stem))));
            }
            (Some(_), None) => self.viewer.preload = None,
            (None, None) => {}
        }
    }

    fn note_level_first_frame(&mut self) {
        self.viewer.recap_swallow = false;
        if let Some(t) = self.viewer.recap_press.take() {
            renderer::diag(&format!("load: the level's first frame begins {:.1} ms after the press", t.elapsed().as_secs_f64() * 1000.0));
        }
    }

    fn waiting_for_recap(&self) -> bool {
        self.viewer.recap_wait.is_some()
    }

    fn present_loading_frames(&mut self) {
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            self.draw_frame_in(0.0, true);
        }
    }

    fn begin_loading_recap(&mut self, req: &openxiii_vm::travel::TravelRequest) -> bool {
        // Not for the co-op character screen's map (a stage for the choice, not the level
        // played: its loading screen would come before the screen, then again for the real
        // level).
        let screen = req.url.has_option(openxiii_vm::character_screen::SCREEN_OPTION);
        // Nor for a co-op game over the network: a player waiting for a key on a held screen
        // while the other one already plays (measured: a guest in a window stayed on "press
        // to continue" after joining, the host playing alone).
        let network_coop = req.pending_join || req.url.has_option(openxiii_vm::coop_address::COOP_OPTION);
        let to = map_stem(&req.url);
        let failed = req.url.has_option("failed");
        match self.game.as_mut() {
            Some(g) if !(screen || network_coop) => {
                let held = g.vm.mod_loading_begin(&to, failed);
                if !held {
                    g.vm.mod_loading_end();
                }
                held
            }
            _ => false,
        }
    }

    fn start_loading_recap(&mut self, req: openxiii_vm::travel::TravelRequest, map: String, game_dir: &Path) {
        let (dir, stem) = (game_dir.to_path_buf(), map.clone());
        let job = std::thread::spawn(move || load_level(&dir, &stem));
        let pad_down = self.recap_pad_buttons();
        self.viewer.recap_wait = Some(RecapWait {
            req: Some(req),
            map,
            job: Some(job),
            loaded: None,
            flow: Default::default(),
            pressed_at: None,
            anim_started: false,
            pad_down,
            drawn: None,
            next_anim: Instant::now(),
        });
        self.recap_step();
    }

    fn prepare_loading_frame(&mut self) {
        if let Some(r) = self.renderer.as_mut() {
            r.hold_clear();
            r.hold_capture = true;
        }
    }

    fn begin_level_hold(&mut self, anim: &mut Option<openxiii_vm::loading_anim::LoadingAnimation>) {
        self.travel_hold = self.renderer.as_mut().and_then(|r| r.hold_start(100));
        if self.travel_hold.is_some() {
            if let Some(a) = anim.as_mut() {
                if let Some(r) = self.renderer.as_mut() {
                    r.warming = true;
                }
                for _ in 0..8 {
                    self.loading_frame(a);
                }
                if let Some(r) = self.renderer.as_mut() {
                    r.warming = false;
                }
            }
        }
        if let Some(r) = self.renderer.as_mut() {
            r.hold_capture = false;
        }
    }

    fn take_preloaded_level(&mut self, map: &str) -> Option<std::thread::JoinHandle<Loaded>> {
        self.viewer.preload.take().filter(|(m, _)| m.eq_ignore_ascii_case(map)).map(|(_, job)| job)
    }
}
