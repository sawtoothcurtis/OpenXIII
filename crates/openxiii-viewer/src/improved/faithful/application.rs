struct ViewerState;

impl ViewerState {
    fn new(_options: &Options) -> Self {
        Self
    }
}

fn prepare_menu_art(_game: &Path) {}

fn character_selection_active(_g: &game::Game) -> bool {
    false
}

fn configure_lockstep_play() {
    openxiii_vm::lockstep::set_wanted(false);
}

fn configure_first_launch(_options: &Options, _root: &Path) {}

fn configure_level_menu(_options: &Options) {}

fn configure_coop_menu() {}

fn configure_join_file(_options: &Options) {}

fn prepare_installed_pictures(_options: &Options) {}

fn configure_character_screen(_options: &Options) {}

include!("window.rs");
include!("level_loading.rs");
include!("menu_overlay.rs");
include!("fixed_capture.rs");
include!("menu_navigation.rs");
