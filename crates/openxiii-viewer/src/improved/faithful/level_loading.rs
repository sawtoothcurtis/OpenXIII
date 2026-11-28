impl App {
    fn note_level_first_frame(&mut self) {}

    fn waiting_for_recap(&self) -> bool {
        false
    }

    fn present_loading_frames(&mut self) {}

    fn begin_loading_recap(&mut self, _req: &openxiii_vm::travel::TravelRequest) -> bool {
        false
    }

    fn start_loading_recap(&mut self, _req: openxiii_vm::travel::TravelRequest, _map: String, _game_dir: &Path) {}

    fn prepare_loading_frame(&mut self) {}

    fn begin_level_hold(&mut self, _anim: &mut Option<openxiii_vm::loading_anim::LoadingAnimation>) {}

    fn take_preloaded_level(&mut self, _map: &str) -> Option<std::thread::JoinHandle<Loaded>> {
        None
    }

    fn preload_step(&mut self) {}
}

impl App {
    fn draw_loading_picture(&mut self) {
        if let Some(r) = self.renderer.as_mut() {
            r.hold_capture = false;
        }
        self.draw_frame_in(0.0, false);
    }
}

fn level_exclusions(game: &Path, map: &str) -> HashSet<String> {
    game::Game::mover_names(game, map).unwrap_or_default()
}

fn load_travel_level(game: &Path, map: &str, exclusions: &HashSet<String>) -> Loaded {
    load(game, map, exclusions)
}

fn refresh_level_graphics(_g: &mut game::Game, _game_dir: &Path) {}
