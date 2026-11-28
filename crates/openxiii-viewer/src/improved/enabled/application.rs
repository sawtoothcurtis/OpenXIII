#[path = "../../mods_page.rs"]
mod mods_page;

#[path = "../../press_watch.rs"]
mod press_watch;

#[path = "../../onomatopoeias.rs"]
mod onomatopoeias;

#[path = "../../install_pictures.rs"]
mod install_pictures;

#[derive(Default)]
struct ViewerState {
    /// Why a network game could not start or ended, to tell on the menus (lockstep_play.rs).
    lockstep_notice: Option<openxiii_vm::added_text::Text>,
    /// Improved build: map loading behind the recap bubbles, until a new press.
    recap_wait: Option<RecapWait>,
    /// Improved build, Android and Linux: the menus' pictures still to make when the game is
    /// installed (true until the window is open), then being made behind a "Loading..." screen
    /// while the game waits (install_pictures.rs).
    #[cfg(not(windows))]
    install_wanted: bool,
    #[cfg(not(windows))]
    installing: Option<install_pictures::Running>,
    /// Improved build: the packages of the level the co-op character screen starts, read on
    /// another thread from the moment everyone confirmed (map stem and the reading), taken by
    /// `travel` for that map; dropped when a confirmation is taken back.
    preload: Option<(String, std::thread::JoinHandle<Loaded>)>,
    /// Improved build: the press that ended the recap's wait, until the level's first frame
    /// (its delay in the log).
    recap_press: Option<Instant>,
    /// Improved build: the level started on a press read during its preparation; the window's
    /// own copy of that press, read afterwards, is dropped until the level's first frame.
    recap_swallow: bool,
    /// Improved build: the Mods page of the main menu.
    mods_page: mods_page::ModsPage,
}

impl ViewerState {
    fn new(_options: &Options) -> Self {
        Self {
            #[cfg(not(windows))]
            install_wanted: _options.standalone && _options.screenshot.is_none(),
            ..Self::default()
        }
    }
}

fn player_files_folder() -> PathBuf {
    launcher::folder_data()
}

fn prepare_menu_art(game: &Path) {
    onomatopoeias::start(game.to_path_buf());
}

fn character_selection_active(g: &game::Game) -> bool {
    g.vm.character_screen_on()
}

fn configure_lockstep_play() {
    openxiii_vm::lockstep::set_wanted(true);
}

fn configure_first_launch(options: &Options, root: &Path) {
    if options.standalone && !root.join("save").join("System").join("XIII.ini").exists() {
        #[cfg(windows)]
        let native = native_display_size();
        #[cfg(not(windows))]
        let native: Option<(u32, u32)> = None;
        match native {
            Some((w, h)) => println!("First launch: Graphics Enhanced, fullscreen at the screen's native size {w}x{h}"),
            None => println!("First launch: Graphics Enhanced, fullscreen at the screen's native size (from the window)"),
        }
        openxiii_vm::video_settings::set_first_launch(native);
    }
}

fn configure_level_menu(options: &Options) {
    // The Load page's Select level entry is a game feature of the improved build (not a cheat).
    openxiii_vm::menu_boxes::set_select_level_entry(options.standalone || options.screenshot.is_some());
    // The Select level page's pictures are kept between runs of the standalone game.
    openxiii_vm::select_level_page::set_art_cache(options.standalone.then(|| launcher::folder_data().join("cache")));
}

fn configure_coop_menu() {
    let coop = mods_page::coop_enabled(openxiii_mods::mods_folder().as_deref());
    openxiii_vm::menu_boxes::set_coop_box(coop);
    mods_page::note_coop_at_start(coop);
}

fn configure_join_file(options: &Options) {
    openxiii_vm::coop_page::set_join_file(options.standalone.then(|| player_files_folder().join("CoopJoin.ini")));
    // The Join page's list tells the games this build could join (lockstep_find.rs).
    let game = options.game.clone();
    openxiii_vm::lockstep_find::set_own_game(Box::new(move || crate::lockstep_play::own_game(&game)));
}

fn prepare_installed_pictures(options: &Options) {
    #[cfg(windows)]
    if options.standalone && options.screenshot.is_none() {
        install_pictures::prepare(&options.game, &launcher::folder_data().join("cache"));
    }
    #[cfg(not(windows))]
    if options.standalone && options.screenshot.is_none() {
        onomatopoeias::hold();
    }
}

fn configure_character_screen(options: &Options) {
    openxiii_vm::character_page::set_choice_file(options.standalone.then(|| player_files_folder().join("CoopCharacters.ini")));
    let places = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("CharacterScreen.ini")));
    openxiii_vm::character_screen::set_place_file(places);
}

include!("window.rs");
include!("level_loading.rs");
include!("menu_overlay.rs");
include!("fixed_capture.rs");
include!("menu_navigation.rs");
