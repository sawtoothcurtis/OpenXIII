//! oxiii-view: real-time viewer of a XIII map.
//!
//! Usage: oxiii-view <game> <map> [--screenshot output.png] [--camera x,y,z,yaw,pitch]
//!         [--size WxH] [--fov horizontal angle]
//! Controls: ZQSD/WASD to move, hold right click + mouse to look,
//! Shift to go faster, Escape to quit.

// Windowed program (like XIII.exe): launched by a tool, it does not open a console;
// launched from a console, it writes its log there (AttachConsole of the parent).
#![cfg_attr(windows, windows_subsystem = "windows")]

#[macro_use]
mod improved;
register_viewer_layer!("application");
mod audio;
mod audio_output;
mod choice_level;
mod lockstep_play;
mod d3d11_marker;
mod gamepad_layout;
mod gamepad_page;
mod d3d;
#[cfg(windows)]
mod desktop_shortcut;
mod split_projectors;
mod game;
mod game_log;
mod launcher;
mod lights_scene;
mod gamma;
// Android: gamepads read by the Java activity (pad_android.rs), gilrs sees nothing there.
#[cfg_attr(target_os = "android", path = "pad_android.rs")]
mod pad;
mod pipeline_cache;
mod native_mode;

static PLAY_AFTER_NEXT_FRAME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

#[cfg(windows)]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod prop_geometry;
mod render_quality;
mod foreground;
mod modal_loop;
mod renderer;
mod screen_fx;
mod sprites;
mod pad_keys;
#[cfg(target_os = "android")]
mod touch_controls;
mod aim_assist;
mod cheats;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod android_log;
mod video_audio;

use openxiii_package::library::Library;
use openxiii_package::Package;
use renderer::{Camera, Renderer};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

struct Options {
    local_players: Option<u8>,
    game: PathBuf,
    map: String,
    screenshot: Option<PathBuf>,
    camera: Option<Camera>,
    /// "Play" mode: game logic active and XIII's pawn moving.
    play: bool,
    /// --gamepad-settings: the gamepad settings page opens over the first menu page (testing
    /// until the options menu has its entry).
    gamepad_settings: bool,
    /// Seconds simulated before a capture in play mode.
    seconds: f32,
    /// Size of the capture (1280 × 720 by default).
    size: (u32, u32),
    /// Forced horizontal field of view (FovAngle), in degrees.
    fov: Option<f32>,
    /// XIII.exe's startup videos (ubi, alien, Nvidia): never by default (no startup
    /// screen), only with --intro (window or capture).
    intro: Option<bool>,
    /// --fullscreen: start like UWindowsClient (fullscreen if StartupFullscreen); without
    /// it, always a window (tests, tools). The game's launcher passes it.
    fullscreen: bool,
    /// Launched alone, without arguments (launcher.rs): "Select level" on the menu, saves in
    /// `save_root`.
    standalone: bool,
    /// Folder that receives "save"; without it, the one in the game folder.
    save_root: Option<PathBuf>,
}

fn parse_args() -> Result<Options, String> {
    // Android: no command line (the Java launcher keeps the game folder in OpenXIII.ini).
    let args: Vec<String> = if cfg!(target_os = "android") { Vec::new() } else { std::env::args().skip(1).collect() };
    // XIII.exe opened from Explorer: the full game, like the game's launcher.
    // Linux: a single argument = the original game folder (remembered afterwards), same full game.
    #[cfg(target_os = "linux")]
    let game_linux = if args.len() == 1 && !args[0].starts_with('-') {
        match launcher::game_gives(Path::new(&args[0])) {
            Some(d) => Some(d),
            None => {
                launcher::message(launcher::MARK, "This does not look like the original XIII game folder (System/Engine.u, System/XIII.ini or Maps/MapMenu.unr is missing).");
                std::process::exit(1)
            }
        }
    } else {
        None
    };
    #[cfg(not(target_os = "linux"))]
    let game_linux: Option<PathBuf> = None;
    // --gamepad-settings alone: the full game too, with the gamepad settings page opened.
    let gamepad_settings = args.iter().any(|a| a == "--gamepad-settings");
    if args.iter().all(|a| a == "--gamepad-settings") || game_linux.is_some() {
        // Released game only: says when a newer official version exists, before the game starts.
        #[cfg(not(target_os = "android"))]
        openxiii_update::at_launch(&launcher::folder_data(), launcher::MARK);
        let Some(game) = game_linux.or_else(launcher::folder_game) else { std::process::exit(0) };
        return Ok(Options {
            local_players: None,
            game,
            map: "MapMenu".into(),
            screenshot: None,
            camera: None,
            play: true,
            gamepad_settings,
            seconds: 3.0,
            size: (1280, 720),
            fov: None,
            // Straight to the menu, without the startup videos.
            intro: Some(false),
            // OXIII_WINDOW=1: windowed (testing, LAN launchers); otherwise like StartupFullscreen.
            fullscreen: openxiii_package::env::var_os("OXIII_WINDOW").is_none(),
            standalone: true,
            save_root: Some(launcher::folder_data()),
        });
    }
    if args.len() < 2 {
        return Err("usage: oxiii-view <game> <map> [--screenshot output.png] [--camera x,y,z,yaw,pitch]".into());
    }
    let mut options = Options {
        local_players: None,
        game: args[0].clone().into(),
        map: args[1].clone(),
        screenshot: None,
        camera: None,
        play: false,
        gamepad_settings: false,
        seconds: 3.0,
        size: (1280, 720),
        fov: None,
        intro: None,
        fullscreen: false,
        standalone: false,
        save_root: None,
    };
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" => {
                options.screenshot = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--camera" => {
                let v: Vec<f32> = args.get(i + 1).ok_or("--camera without a value")?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 5 {
                    return Err("--camera expects x,y,z,yaw,pitch".into());
                }
                options.camera = Some(Camera { position: [v[0], v[1], v[2]], yaw: v[3], pitch: v[4], roll: 0.0 });
                i += 1;
            }
            "--play" => options.play = true,
            "--gamepad-settings" => options.gamepad_settings = true,
            "--windowed" => options.fullscreen = false,
            "--fullscreen" => options.fullscreen = true,
            "--intro" => options.intro = Some(true),
            "--no-intro" => options.intro = Some(false),
            "--size" => {
                let v: Vec<u32> = args.get(i + 1).ok_or("--size without a value")?.split('x').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() != 2 || v[0] == 0 || v[1] == 0 {
                    return Err("--size expects WxH".into());
                }
                options.size = (v[0], v[1]);
                i += 1;
            }
            // Players on this screen for the multiplayer games (improved build: split screen at 2).
            "--local-players" => {
                let n: u8 = args.get(i + 1).and_then(|s| s.parse().ok()).filter(|n| (1..=4).contains(n)).ok_or("--local-players expects 1 to 4")?;
                options.local_players = Some(n);
                i += 1;
            }
            "--fov" => {
                options.fov = Some(args.get(i + 1).and_then(|s| s.parse().ok()).ok_or("--fov expects an angle")?);
                i += 1;
            }
            "--seconds" => {
                options.seconds = args.get(i + 1).and_then(|s| s.parse().ok()).ok_or("--seconds expects a number")?;
                i += 1;
            }
            // Mods build only: turns mods off, read by start_mods.
            #[cfg(feature = "mods")]
            "--no-mods" => {}
            other =>return Err(format!("unknown option: {other}")),
        }
        i += 1;
    }
    Ok(options)
}

fn load(game: &Path, map: &str, exclude: &std::collections::HashSet<String>) -> Result<(openxiii_assets::world::World, Library), String> {
    let path = openxiii_package::library::map_path(game, map);
    openxiii_package::events::log("map_start", || map.to_string(), String::new);
    let data = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    openxiii_package::events::file_read(&path, data.len());
    let package = Package::parse(&data).map_err(|e| e.to_string())?;
    let mut library = Library::new(game).map_err(|e| e.to_string())?;
    // Improved build: the menus' onomatopoeias redrawn on other cores while the map loads.
    prepare_menu_art(game);
    library.insert(map, path, data, package);
    let world = openxiii_assets::world::build_world_excluding(&mut library, map, exclude).ok_or("unreadable map")?;
    openxiii_package::events::log("map_end", || map.to_string(), String::new);
    Ok((world, library))
}

/// Travel during a capture (UGameEngine::Browse then LoadMap): geometry, packages and logic of the
/// URL's map, the inventory and the engine's subsystems carried over.
fn headless_travel(
    game_dir: &Path,
    req: openxiii_vm::travel::TravelRequest,
    bindings: &openxiii_vm::input::Bindings,
    vm: &mut openxiii_vm::Vm,
) -> Option<(openxiii_assets::world::World, Library, game::Game)> {
    let _math = vm.session.math.enter();
    let map = map_stem(&req.url);
    let exclude = game::Game::mover_names(game_dir, &map).unwrap_or_default();
    let (world, mut library) = match load(game_dir, &map, &exclude) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("travel to {map}: {e}");
            return None;
        }
    };
    let name = req.url.get_option("NAME=").map(str::to_string);
    let info = req.info.iter().find(|(n, _)| Some(n) == name.as_ref()).or(req.info.first()).map(|(_, t)| t.clone());
    // UGameEngine::LoadMap: the level being left is removed from the VM, which loads the next one.
    let mut reuse = std::mem::take(vm);
    reuse.unload_level();
    // Players on this screen chosen in the menus, given to the engine as the window does (App::travel).
    let players = reuse.local_play().local_players;
    reuse.set_local_players(players);
    let mut req = req;
    if let Some(e) = req.engine.as_mut() {
        e.client.local_players = reuse.engine.client.local_players;
    }
    match game::Game::new(
        game_dir,
        &req.url,
        world.collision.clone(),
        world.collision_tags.clone(),
        world.collision_materials.clone(),
        &mut library,
        bindings,
        info.as_deref(),
        req.engine,
        Some(reuse),
        Default::default(),
    ) {
        Ok(g) => Some((world, library, g)),
        Err(e) => {
            eprintln!("travel to {map}: {e}");
            None
        }
    }
}

/// Initial camera: the player start, otherwise above the center of the map.
fn start_camera(world: &openxiii_assets::world::World) -> Camera {
    if let Some((p, yaw)) = world.player_start {
        return Camera { position: [p[0], p[1], p[2] + 60.0], yaw: yaw as f32 * 360.0 / 65536.0, pitch: 0.0, roll: 0.0 };
    }
    let c = [0, 1, 2].map(|k| (world.bounds_min[k] + world.bounds_max[k]) / 2.0);
    Camera { position: [c[0], c[1], world.bounds_max[2]], yaw: 0.0, pitch: -45.0, roll: 0.0 }
}

struct App {
    options: Options,
    world: Option<openxiii_assets::world::World>,
    library: Option<Library>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    camera: Camera,
    keys: HashSet<KeyCode>,
    looking: bool,
    last: Instant,
    /// XIII.exe's MainLoop (openxiii_vm::main_loop): frame counter, seconds and frames
    /// counted for CurrentTickRate, foreground state of the previous frame.
    run_loop: openxiii_vm::main_loop::ClientLoop,
    /// Window in the foreground (WM_SETFOCUS / WM_KILLFOCUS).
    focused: bool,
    /// The window is hidden by other windows (winit's Occluded).
    occluded: bool,
    /// Lockstep co-op: a frame run without drawing while the window is minimized or hidden, so
    /// that the session keeps stepping, sending and receiving (`about_to_wait`).
    background_frame: bool,
    background_last: Option<Instant>,
    game: Option<game::Game>,
    /// Gamepads enumerated at startup (gilrs), given to the VM like DirectInput.
    pads: Option<pad::Pads>,
    characters: Option<game::Characters>,
    /// Viewport frame counter (shadow scheduling, AShadowProjector::UpdateShadow).
    shadow_frame: u32,
    audio: Option<audio::Audio>,
    /// Travel requested by the game (end of level, ?restart), done after the frame.
    pending_travel: Option<openxiii_vm::travel::TravelRequest>,
    /// Network co-op in lockstep (lockstep_play.rs): the session, the real time not yet
    /// simulated (seconds), this machine's player's view of the last step and its screen effects.
    lockstep: Option<lockstep_play::Runner>,
    lockstep_time: f64,
    /// This frame is not drawn (a late machine's time goes to the steps), and the frames left
    /// undrawn in a row (lockstep_play.rs, lockstep_steps).
    lockstep_skip_draw: bool,
    lockstep_undrawn: u32,
    lockstep_view: Option<(openxiii_vm::client::ViewportFrame, Vec<openxiii_vm::screen_fx::FxPass>)>,
    /// What drawing this machine's view left in the VM (the actors drawn with their owner
    /// tests, the display list…), put back while `lockstep_view` is presented.
    lockstep_output: Option<openxiii_vm::client::ViewOutput>,
    /// The steps `lockstep_view` and `lockstep_output` were made at (a frame shows the view's,
    /// with scene lists made for it at present when the output is another step's).
    lockstep_view_step: u64,
    lockstep_output_step: u64,
    /// The camera of this machine's view at the step before `lockstep_view_step` (step, view
    /// actor, location, rotation): a frame draws the world between the last two steps, its camera
    /// between theirs.
    lockstep_view_prev: Option<(u64, Option<openxiii_vm::value::ObjId>, [f32; 3], [i32; 3])>,
    /// The other players' views of the last step (player rank, viewport, what each drew, its
    /// view), for the per-view work split screen does at present (shadows).
    lockstep_others: Vec<(usize, u8, openxiii_vm::client::ViewOutput, Option<(Option<openxiii_vm::value::ObjId>, [f32; 3], [i32; 3])>)>,
    /// Shadow work of the views after this machine's one, drawn with the next frame.
    lockstep_shadow_after: Vec<renderer::ShadowRender>,
    /// What the window's title says about the session ("" for nothing).
    lockstep_note: String,
    /// The next travel starts a lockstep session: its map gets a new VM.
    lockstep_fresh: bool,
    /// A lockstep meeting under way (lockstep_play.rs).
    lockstep_meeting: Option<Box<lockstep_play::MeetState>>,
    /// Lockstep co-op, a new game: the session met, its first level waiting for the end of the
    /// New game video (lockstep_intro_poll); the gamepad buttons down at the last look.
    lockstep_intro: Option<Box<(lockstep_play::Runner, openxiii_vm::travel::TravelRequest)>>,
    lockstep_intro_pad: u32,
    /// Lockstep co-op: the loading picture kept after a travel until the other machine is in
    /// the level too, and the session's first step there (lockstep_hold_frame).
    lockstep_hold: Option<(renderer::PresentHold, u64)>,
    /// Frames of the level drawn off screen behind that picture (lockstep_play.rs).
    lockstep_hold_warm: u32,
    /// The start state of the next session (travel items, music, checkpoints), as the host has it.
    lockstep_start_state: Option<Vec<(String, Vec<u8>)>>,
    /// The Select level equipment is being given at a step (lockstep sessions give it only then).
    lockstep_equipment_now: bool,
    /// A lockstep session ended: the next travel (to the menus) starts a new VM with this
    /// machine's own settings.
    lockstep_restore: bool,
    /// Lockstep co-op: the menu pages this machine's menus kept when the session began (its root
    /// window's SaveMenuStack), given back to the menus' new VM after the session.
    lockstep_menus: Option<Vec<String>>,
    viewer: ViewerState,
    lockstep_host_started: bool,
    /// The waiting sign of this frame for the single view (meeting).
    lockstep_sign_now: Vec<openxiii_vm::canvas::HudDraw>,
    /// UEngLoadingAnimationThread (one object for the whole session) and the rand() of its thread.
    loading_anim: Option<openxiii_vm::loading_anim::LoadingAnimation>,
    /// Improved build: the loading animation shown again by another thread while the rest of
    /// the travel blocks (started in `travel` once the map file is read, ended by `travel_held`).
    travel_hold: Option<renderer::PresentHold>,
    /// When the current travel started (its load time in the log).
    travel_started: Instant,
    /// Sound of the videos (binkw32 output, kept from one map to the next).
    video_sound: Option<video_audio::VideoSound>,
    /// Last Present of the video player, which stays on screen as long as the game is not drawn.
    video_show: Option<openxiii_vm::video::VideoPresent>,
    /// Startup videos still to play, and whether one is in progress.
    intro: Vec<&'static str>,
    intro_playing: bool,
    /// Display modes of the screen (width, height, bits per pixel).
    display_modes: Vec<(u32, u32, u32)>,
    /// The screen's native resolution, read once before any change of display mode (native_mode.rs).
    screen_native: Option<(u32, u32)>,
    started: Instant,
    /// Alt pressed (GetKeyState(VK_MENU) of ViewportWndProc).
    alt_down: bool,
    /// Mouse capture applied to the Windows cursor (sync_capture).
    capture_view: Option<u32>,
    /// Last cursor position in the window (WM_MOUSEMOVE).
    cursor_value: (u32, u32),
    /// Size of the exclusive fullscreen left when focus was lost, restored on return.
    full_screen_suspended: Option<(u32, u32)>,
    perf: (u32, f32, f32, f32, Option<Instant>),
    /// Android: touch controls (touch_controls.rs).
    #[cfg(target_os = "android")]
    touch_controls: touch_controls::TouchControls,
    /// Gamepad seen as keyboard and mouse (pad_keys.rs).
    pad_keys: pad_keys::PadKeys,
    /// The player last used the gamepad (aim assist of the gamepad, the VM's aim_device.rs),
    /// and a gamepad has been used at least once (before that, nothing changes).
    aim_on_pad: bool,
    aim_pad_used: bool,
    /// Cheats of the test builds (cheats.rs); never set in the normal build.
    cheats: cheats::Cheats,
    perf_steps: [f64; 9],
    /// "Select level" of the main menu of the standalone game.
    choice: choice_level::ChoiceLevel,
    /// Gamepad settings of the local players (the VM's gamepad_config.rs) and their page.
    gamepad_config: openxiii_vm::gamepad_config::GamepadConfig,
    /// Vibration just switched on in a player's gamepad settings: that player (0…) and the seconds
    /// of the short feedback rumble left.
    rumble_test: Option<(usize, f32)>,
    gamepad_page: gamepad_page::GamepadPage,
    /// Split screen: the pages of the players after the first, each in its own view.
    other_gamepad_pages: [gamepad_page::GamepadPage; openxiii_vm::gamepad_config::PLAYERS - 1],
    /// Android: the first player's gamepad settings page had their gamepad in the last frame
    /// (touch_controls.rs then gives the game that gamepad at rest).
    #[cfg(target_os = "android")]
    first_page_has_pad: bool,
    /// Test equipment to give on this map, once its starting inventory has been given.
    weapons_for: Option<(String, Vec<&'static str>)>,
    /// The Select level start's equipment for the whole run of its level (Retry, a death's
    /// restart), and the save device's slot reads when it was chosen.
    loadout: Option<(String, Vec<&'static str>)>,
    slot_reads_seen: u32,
    /// What the players chose in the menus (co-op session, players on this screen): the reference
    /// copy, given to each map's VM (openxiii_vm::local_play).
    local_play: openxiii_vm::local_play::LocalPlay,
}

/// Improved build: key and mouse releases still reach the game while a page opened over the main
/// menu (Select level, Mods) is open: the menu box that opened it saw the press.

/// Loop counter (rdtsc in the original, low word only): nanoseconds since launch, on 32 bits.
fn counter() -> u32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u32
}

/// GSecondsPerCycle of the counter above.
const SECONDS_BY_CYCLE: f64 = 1e-9;

/// XIII.exe, after the engine is initialized and the first map is
/// loaded: Open, Play then Tick as long as the status is not 0, for each video, without handling
/// the window's messages (keys wait until the end). Order of the original XIII.log.
const INTRO_VIDEOS: [&str; 3] = ["ubi", "alien", "Nvidia"];

/// One step of the startup videos: `true` as long as they occupy the loop.
fn intro_step(vm: &mut openxiii_vm::Vm, intro: &mut Vec<&'static str>, playing: &mut bool, dt: f32) -> bool {
    if vm.video_status() == 0 {
        if intro.is_empty() {
            *playing = false;
            return false;
        }
        let name = intro.remove(0);
        // A missing video would leave the status at 1 (Play) and the original loop would spin
        // forever: we move on to the next one.
        if !vm.video_open_play(name) {
            vm.engine.video.status = 0;
            return true;
        }
        *playing = true;
    }
    vm.video_tick_only(dt);
    true
}

/// Key bindings: User.ini if it exists (the player's settings), otherwise DefUser.ini.
fn load_bindings(game: &Path) -> openxiii_vm::input::Bindings {
    let user_ini = openxiii_package::system_dir(&game).join("User.ini");
    let ini_path = if user_ini.exists() { user_ini } else { openxiii_package::system_dir(&game).join("DefUser.ini") };
    openxiii_vm::input::Bindings::load(&ini_path)
}

/// Result of the background map load.
type Loaded = Result<(openxiii_assets::world::World, Library), String>;

/// Map name of a URL, without extension (map files, sound banks).
fn map_stem(url: &openxiii_vm::travel::Url) -> String {
    let file = openxiii_vm::travel::map_file(url);
    file.rsplit_once('.').map_or(file.clone(), |(m, _)| m.to_string())
}

/// EInputKey codes (the Windows virtual key codes) of a physical key, as
/// with a QWERTY keyboard: WinDrv (UWindowsViewport::UpdateInput) reads GetKeyState for each
/// code, so Shift, Ctrl and Alt also give their left or right variant (LShift, RControl…).
fn vk_codes(code: KeyCode) -> Vec<u8> {
    use KeyCode::*;
    let name = format!("{code:?}");
    let single = |p: &str| name.strip_prefix(p).filter(|r| r.len() == 1).map(|r| r.as_bytes()[0]);
    if let Some(c) = single("Key").or_else(|| single("Digit")) {
        return vec![c];
    }
    if let Some(k) = name.strip_prefix("Numpad").and_then(|r| r.parse::<u8>().ok()) {
        return vec![0x60 + k];
    }
    if let Some(k) = name.strip_prefix('F').and_then(|r| r.parse::<u8>().ok()).filter(|k| (1..=24).contains(k)) {
        return vec![0x6f + k];
    }
    match code {
        ShiftLeft => vec![0x10, 0xa0],
        ShiftRight => vec![0x10, 0xa1],
        ControlLeft => vec![0x11, 0xa2],
        ControlRight => vec![0x11, 0xa3],
        AltLeft => vec![0x12, 0xa4],
        AltRight => vec![0x12, 0xa5],
        Space => vec![0x20],
        Enter | NumpadEnter => vec![0x0d],
        // VK_ESCAPE: without it, Escape never reached the game (neither menu nor video skip).
        Escape => vec![0x1b],
        Backspace => vec![0x08],
        Tab => vec![0x09],
        ArrowLeft => vec![0x25],
        ArrowUp => vec![0x26],
        ArrowRight => vec![0x27],
        ArrowDown => vec![0x28],
        PageUp => vec![0x21],
        PageDown => vec![0x22],
        End => vec![0x23],
        Home => vec![0x24],
        Insert => vec![0x2d],
        Delete => vec![0x2e],
        CapsLock => vec![0x14],
        NumLock => vec![0x90],
        ScrollLock => vec![0x91],
        Pause => vec![0x13],
        NumpadMultiply => vec![0x6a],
        NumpadAdd => vec![0x6b],
        NumpadSubtract => vec![0x6d],
        NumpadDecimal => vec![0x6e],
        NumpadDivide => vec![0x6f],
        Semicolon => vec![0xba],
        Equal => vec![0xbb],
        Comma => vec![0xbc],
        Minus => vec![0xbd],
        Period => vec![0xbe],
        Slash => vec![0xbf],
        Backquote => vec![0xc0],
        BracketLeft => vec![0xdb],
        Backslash => vec![0xdc],
        BracketRight => vec![0xdd],
        Quote => vec![0xde],
        _ => vec![],
    }
}

/// Exclusive fullscreen in the screen's x × y video mode (32 bits, the highest refresh rate),
/// like the game's D3D device; false if the screen does not have this mode. `borderless`
/// ([OpenXIII.Video] FullscreenMode=Borderless, not in the original): a borderless window
/// covering the monitor at the desktop's resolution instead, whatever x × y.
fn set_fullscreen(window: &Window, x: u32, y: u32, borderless: bool) -> bool {
    let Some(monitor) = window.current_monitor() else { return false };
    if borderless {
        window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(monitor))));
        return true;
    }
    let mode = monitor
        .video_modes()
        .filter(|m| m.size().width == x && m.size().height == y)
        .max_by_key(|m| (m.bit_depth() == 32, m.refresh_rate_millihertz()));
    match mode {
        Some(m) => {
            window.set_fullscreen(Some(winit::window::Fullscreen::Exclusive(m)));
            true
        }
        None => false,
    }
}

/// UWindowsViewport::ToggleFullscreen: in fullscreen, EndFullscreen and
/// back to the WindowedViewportX/Y window; otherwise fullscreen at FullscreenViewportX/Y.
fn toggle_fullscreen(vm: &mut openxiii_vm::Vm, window: &Window) {
    let size = |vm: &mut openxiii_vm::Vm, kx: &str, ky: &str| {
        let c = vm.config.as_mut()?;
        let x: u32 = c.get_string("WinDrv.WindowsClient", kx, "System")?.trim().parse().ok()?;
        let y: u32 = c.get_string("WinDrv.WindowsClient", ky, "System")?.trim().parse().ok()?;
        Some((x, y))
    };
    // UWindowsViewport::ToggleFullscreen: leaving fullscreen (windowed ResizeViewport:
    // SetMouseCapture(0, 0, 0), SetDrag(0)) then SetMouseCapture(1, 1, 0); entering: fullscreen ResizeViewport,
    // SetDrag(1), SetMouseCapture(1, 1, 0).
    if vm.fullscreen {
        window.set_fullscreen(None);
        vm.fullscreen = false;
        if let Some((x, y)) = size(vm, "WindowedViewportX", "WindowedViewportY") {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(x, y));
        }
        vm.win_set_mouse_capture(0, 0, 0);
        vm.win_set_drag(0);
        vm.win_set_mouse_capture(1, 1, 0);
    } else if let Some((x, y)) = size(vm, "FullscreenViewportX", "FullscreenViewportY") {
        let (x, y) = vm.fullscreen_size((x, y));
        let borderless = vm.borderless_fullscreen();
        vm.fullscreen = set_fullscreen(window, x, y, borderless);
        // Improved build: a screen without the x × y video mode gets borderless fullscreen
        // instead, as the Video menu does (otherwise Alt+Enter did nothing). The faithful build
        // keeps the original's ToggleFullscreen, which does nothing then.
        apply_fullscreen_fallback(window, vm, x, y, borderless);
        if vm.fullscreen {
            vm.win_set_drag(1);
            vm.win_set_mouse_capture(1, 1, 0);
        }
    }
    // UWindowsViewport::TryRenderDevice (called by ToggleFullscreen and EndFullscreen): with GIsRunning,
    // Actor->GetLevel()->DetailChange(RenDev->HighDetailActors) on the current map.
    let high = vm.high_detail_actors();
    vm.detail_change(high);
}

/// Display mode chosen in the Video menu (not in the original): a window of `size`, a
/// borderless fullscreen window, or exclusive fullscreen in the `size` video mode (borderless if
/// the monitor does not have it). Mouse capture and drag as ToggleFullscreen.
fn apply_display_mode(vm: &mut openxiii_vm::Vm, window: &Window, mode: openxiii_vm::video_settings::FullscreenMode, size: (u32, u32)) {
    use openxiii_vm::video_settings::FullscreenMode;
    match mode {
        FullscreenMode::Windowed => {
            window.set_fullscreen(None);
            vm.fullscreen = false;
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1));
            vm.win_set_mouse_capture(0, 0, 0);
            vm.win_set_drag(0);
            vm.win_set_mouse_capture(1, 1, 0);
        }
        FullscreenMode::Borderless | FullscreenMode::Exclusive => {
            let exclusive = mode == FullscreenMode::Exclusive && set_fullscreen(window, size.0, size.1, false);
            vm.fullscreen = exclusive || set_fullscreen(window, size.0, size.1, true);
            if vm.fullscreen {
                vm.win_set_drag(1);
                vm.win_set_mouse_capture(1, 1, 0);
            }
        }
    }
    // As ToggleFullscreen (TryRenderDevice): DetailChange on the current map.
    let high = vm.high_detail_actors();
    vm.detail_change(high);
}

/// Windows cursor according to the viewport's mouse capture (SetMouseCapture: hidden and confined when captured,
/// shown otherwise).
fn sync_capture(window: &Window, vm: &openxiii_vm::Vm, seen_flag: &mut Option<u32>) {
    let capture = vm.win_input.capture;
    if *seen_flag == Some(capture) {
        return;
    }
    *seen_flag = Some(capture);
    if capture != 0 {
        let grabbed = window.set_cursor_grab(winit::window::CursorGrabMode::Locked).or_else(|_| window.set_cursor_grab(winit::window::CursorGrabMode::Confined));
        if grabbed.is_ok() {
            window.set_cursor_visible(false);
        }
    } else {
        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
        window.set_cursor_visible(true);
    }
}

/// Unreal rotator → degrees in [-180, 180].
/// UGameEngine::Draw of a split-screen viewport into `target`: camera, HUD and visibility
/// of its player (VM), then the rendering stages of `draw_frame` for this view and the scene in
/// the viewport's rectangle. The view's camera, if there is one.
#[allow(clippy::too_many_arguments)]
/// The improved build's co-op character screen is shown (its scene goes into a panel of the
/// page, drawn per view even for one player); never in the other builds.
fn character_screen_on(g: &game::Game) -> bool {
    character_selection_active(g)
}

fn draw_viewport_view(
    r: &mut Renderer,
    g: &mut game::Game,
    library: &mut Library,
    characters: Option<&game::Characters>,
    slot: u8,
    client: (u32, u32),
    shadow_frame: u32,
    target: &wgpu::TextureView,
    extra: &[openxiii_vm::canvas::HudDraw],
) -> Option<Camera> {
    let f = g.draw_viewport(slot, client)?;
    draw_viewport_frame(r, g, library, characters, &f, slot, shadow_frame, target, extra)
}

/// Rendering of a viewport whose UGameEngine::Draw was done by the VM (`f`): see
/// `draw_viewport_view`.
fn draw_viewport_frame(
    r: &mut Renderer,
    g: &mut game::Game,
    library: &mut Library,
    characters: Option<&game::Characters>,
    f: &openxiii_vm::client::ViewportFrame,
    slot: u8,
    shadow_frame: u32,
    target: &wgpu::TextureView,
    extra: &[openxiii_vm::canvas::HudDraw],
) -> Option<Camera> {
    r.consume_engine_flush(&mut g.vm);
    let (_, location, rotation) = f.view?;
    let camera = Camera { position: location, yaw: degrees(rotation[1]), pitch: degrees(rotation[0]), roll: degrees(rotation[2]) };
    r.fov = Some(f.fov);
    r.lens = f.lens;
    // Local split screen: the weapon keeps the height it has in the whole window (old co-op mod).
    // Side by side (split_layout.rs: a view narrower than the window, as high as it), the weapon
    // as in a solo window of the view's shape (Hor+ keeps a 4:3 window's height of view).
    r.overlay_aspect_cap = g.vm.local_split().then(|| {
        let view_aspect = f.rect[2] as f32 / f.rect[3].max(1) as f32;
        if view_aspect < r.window_aspect() { openxiii_vm::coop::REFERENCE_ASPECT } else { r.window_aspect() }
    });
    r.only_post_render = g.vm.only_post_render;
    r.gamma = crate::gamma::applies(g.vm.fullscreen, g.vm.engine.client.session_graphics).then_some(g.vm.client_color);
    // Render textures (URenderTargetMaterial::Update during PostRender): rendered after the
    // characters are updated below, the scene in this frame's state like the engine.
    let targets = g.render_targets();
    // Viewport canvas: its primitives in window pixels (origin of the rectangle).
    let (ox, oy) = (f.rect[0] as f32, f.rect[1] as f32);
    let shift = |p: [f32; 2]| [p[0] + ox, p[1] + oy];
    let mut hud: Vec<openxiii_vm::canvas::HudDraw> = f
        .hud
        .iter()
        .cloned()
        .map(|mut d| {
            use openxiii_vm::canvas::HudDraw;
            match &mut d {
                HudDraw::Tile { x1, y1, x2, y2, .. } => {
                    [*x1, *y1] = shift([*x1, *y1]);
                    [*x2, *y2] = shift([*x2, *y2]);
                }
                HudDraw::Triangle { p, .. } => *p = p.map(shift),
                HudDraw::Line { from, to, .. } => {
                    *from = shift(*from);
                    *to = shift(*to);
                }
                HudDraw::Portal { rect, .. } => {
                    rect[0] += ox;
                    rect[1] += oy;
                }
            }
            d
        })
        .collect();
    // Pages of the game window over this view (the gamepad settings page), in window pixels.
    hud.extend_from_slice(extra);
    r.set_hud(&hud);
    // Apply2DEffect per viewport: the effects' state is that of FD3DRenderInterface, shared.
    // Lockstep co-op: the effects of the view were advanced by the step (lockstep_steps).
    r.fx_passes = match lockstep_play::LOCKSTEP_FX.with(|l| l.borrow_mut().take()) {
        Some(fx) => fx,
        None => g.vm.screen_fx_frame(f.player, Some(location)),
    };
    r.cartoon_flash = g.vm.screen_fx.cartoon_flash;
    r.vignette = g.vm.vignette(f.player);
    r.flash = g.flash_tile(f.player);
    r.clear_volume = g.vm.view_volume_clear(f.player);
    r.volume_fog = g.vm.view_volume_fog(f.player);
    r.color_filter = f.filter.map(|c| [c[2], c[1], c[0], c[3]]);
    if let Some(c) = characters {
        let (weather, c2w) = g.weather(camera.position, camera.yaw, camera.pitch);
        r.view_size = Some([f.rect[2].max(1) as f32, f.rect[3].max(1) as f32]);
        r.set_weather(weather.as_ref(), &c2w);
        r.view_size = None;
        let verts = g.character_frame(c);
        g.character_cartoon_in(c, &verts, r.cartoon_mut());
        let light_views = g.scene_light_leaves(c);
        r.set_scene_light_leaves(light_views);
        r.set_dyn_lights(&g.dyn_light_records());
        r.set_prop_lights(g.prop_lights(c, &verts));
        r.set_hidden_owners(g.hidden_baked());
        r.set_texture_replacements(&g.vm.texture_replacements);
        if let Some(w) = g.replaced_world(library, false) {
            r.set_replaced(w.as_ref());
        }
        r.update_sine(library, Some(&g.sine_drawn()));
        r.set_bsp_visible(g.bsp_visible());
        let mut jobs = lockstep_play::LOCKSTEP_SHADOW_JOBS.with(|l| std::mem::take(&mut *l.borrow_mut()));
        let preview = lockstep_play::LOCKSTEP_STEP_SCENE.with(|l| l.borrow_mut().take()).map(|step| g.vm.swap_view_output(step));
        jobs.extend(g.update_shadows(c, camera.position, &|k| r.cartoon_dir(k), shadow_frame, Some(slot)));
        if let Some(p) = preview {
            let step = g.vm.swap_view_output(p);
            lockstep_play::LOCKSTEP_STEP_SCENE.with(|l| *l.borrow_mut() = Some(step));
        }
        if !jobs.is_empty() {
            g.for_each_shadow_mesh(c, &jobs, |first, mesh, id| r.set_shadow_vertices_id(first, mesh, id));
        }
        r.set_shadow_jobs(jobs);
        r.update_dynamic(&verts, &g.first_person(c));
        r.set_ignore_vignette(&g.ignore_vignette(c));
        r.set_vignette_owners(g.vignette_baked());
        r.set_translucent_keys(g.translucent_keys(c));
        let (sprites, meshes) = g.particle_sprites_for(&camera, f.player.or(f.view.and_then(|v| v.0)));
        let model = r.model();
        let mut projectors = g.projector_draws(&camera, model.as_deref(), f.fov);
        // Improved build: the character screen's other players' shadows are not this view's.
        projectors.retain(|d| !f.hidden_projectors.contains(&d.info.projector));
        r.set_trails(g.trail_batches(&camera));
        r.set_particles(&sprites, &meshes, library);
        r.set_bsp_projectors(&projectors, library);
    }
    render_targets(r, g, characters, &targets, library);
    // The scene in its own rectangle when it has one (the character screen's live-view panel).
    let rect = f.rect.map(|k| k as f32);
    r.draw_view_in(target, &camera, rect, f.scene.map(|s| s.map(|k| k as f32)), library);
    r.lens = [0.0; 2];
    r.in_panel = false;
    Some(camera)
}

/// Views of the frame's render textures (URenderTargetMaterial::Update), each with its own
/// display list: the characters that only they draw (target_rendered) for the duration of
/// their rendering, then those of the main view. The player's pawn seen by a comic panel's camera
/// (Plage00: the first guard's fall) therefore does not appear in the first-person view.
fn render_targets(r: &mut Renderer, g: &mut game::Game, characters: Option<&game::Characters>, targets: &[(String, Vec<renderer::TargetView>)], library: &mut Library) {
    if targets.is_empty() {
        return;
    }
    let t0 = std::time::Instant::now();
    let targets_list = characters.and_then(|c| Some((c, g.character_frame_targets(c)?)));
    let t1 = std::time::Instant::now();
    if let Some((c, verts)) = &targets_list {
        r.use_target_vertices(true);
        r.update_dynamic(verts, &g.first_person(c));
    }
    let t2 = std::time::Instant::now();
    for (key, views) in targets {
        r.render_target(key, views, library);
    }
    let t3 = std::time::Instant::now();
    if let Some((c, _)) = targets_list {
        r.use_target_vertices(false);
        let verts = g.character_frame(c);
        r.update_dynamic(&verts, &g.first_person(c));
    }
    timer_targets([t0, t1, t2, t3, std::time::Instant::now()], targets.iter().map(|(_, v)| v.len()).sum());
}

fn timer_targets(t: [std::time::Instant; 5], views_list: usize) {
    static TOTAL: std::sync::Mutex<([f64; 4], u32, usize)> = std::sync::Mutex::new(([0.0; 4], 0, 0));
    return;
}

fn degrees(r: i32) -> f32 {
    let r = r.rem_euclid(65536);
    (if r > 32768 { r - 65536 } else { r }) as f32 * 360.0 / 65536.0
}

/// Adapts the OpenWindow boundaries to the existing Winit/Renderer blocks.
/// No device is announced before the real Ok(Renderer).
struct InitialWindowHost<'a> {
    event_loop: &'a ActiveEventLoop,
    title: String,
    allow_fullscreen: bool,
    /// Fullscreen is borderless ([OpenXIII.Video] FullscreenMode).
    borderless: bool,
    /// Improved build, Graphics: Original with a Borderless key: fullscreen is exclusive at the
    /// original's resolution (Vm::fullscreen_size), whatever size the keys kept from Enhanced.
    original_fullscreen: bool,
    requested_size: (i32, i32),
    color: [f32; 3],
    world: &'a openxiii_assets::world::World,
    library: &'a mut Library,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    fullscreen: bool,
    display_modes: Vec<(u32, u32, u32)>,
    screen_native: Option<(u32, u32)>,
    /// First launch: the native size render_device opened the fullscreen at, to write into the ini.
    first_launch_size: Option<(u32, u32)>,
    last_error: Option<String>,
    /// spawn_play_actors has already published this log before resumed; no duplicate.
    opened_log_already_emitted: bool,
}

impl InitialWindowHost<'_> {
    fn create_window(&mut self, x: i32, y: i32) {
        // OXIII_WINDOW=WxH (set by a launcher): forced window size.
        let (x, y) = openxiii_package::env::var("OXIII_WINDOW")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))))
            .unwrap_or((x, y));
        let attributes = Window::default_attributes().with_title(&self.title);
        // the exe's icon (build.rs, group 1) for the window and the taskbar
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::{IconExtWindows, WindowAttributesExtWindows};
            match winit::window::Icon::from_resource(1, None) {
                Ok(icon) => attributes.with_window_icon(Some(icon.clone())).with_taskbar_icon(Some(icon)),
                Err(_) => attributes,
            }
        };
        let attributes = if x > 0 && y > 0 {
            attributes.with_inner_size(winit::dpi::PhysicalSize::new(x as u32, y as u32))
        } else {
            attributes.with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
        };
        self.window = Some(Arc::new(self.event_loop.create_window(attributes).expect("window")));
        // Dragging or resizing the window does not stop the frames (nor a session's steps).
        #[cfg(windows)]
        if let Some(w) = &self.window {
            modal_loop::keep_frames_while_dragged(w);
        }
    }

    fn render_device(&mut self, fs: bool, x: i32, y: i32) -> u32 {
        let window = self.window.as_ref().expect("window created before the device");
        let x = if x == -1 { self.requested_size.0 } else { x };
        let y = if y == -1 { self.requested_size.1 } else { y };
        self.fullscreen = false;
        if let Some(m) = window.current_monitor().or_else(|| self.event_loop.primary_monitor()) {
            let mut modes: Vec<(u32, u32, u32, u32)> = m.video_modes().map(|v| (v.size().width, v.size().height, v.bit_depth() as u32, v.refresh_rate_millihertz())).collect();
            modes.sort();
            self.display_modes = modes.into_iter().map(|(w, h, b, _)| (w, h, b)).collect();
            // Read once, before the game changes the display mode (its fallback is the desktop's size).
            if self.screen_native.is_none() {
                self.screen_native = native_mode::of_monitor(&m);
            }
        }
        // Android: no display modes (the game image is the surface's size, scaled up by the
        // screen); those a monitor of the screen's size would list (video_settings::android_modes).
        #[cfg(target_os = "android")]
        if let Some(screen) = touch_controls::screen_size() {
            self.display_modes = openxiii_vm::video_settings::android_modes(screen);
            self.screen_native = Some(screen);
        }
        // First launch whose native size was unknown at load (Linux, Android): fullscreen at the
        // screen's native size, written into the ini once the window is open (resumed). Android
        // too: the screen's size, the largest of the Video menu's list, as on PC (never the
        // activity's first 600-line image).
        let (x, y) = first_launch_dimensions(self.screen_native, &mut self.first_launch_size, fs, x, y);
        // Android: the game image at the resolution the game opens with (its ini's).
        #[cfg(target_os = "android")]
        if x > 0 && y > 0 {
            if let Some(size) = openxiii_vm::video_settings::android_surface_for(&self.display_modes, (x as u32, y as u32)) {
                touch_controls::want_surface(size);
                // The activity reads that size on its own thread and resizes the surface. The
                // device is created once it has (two seconds at most): created on the activity's
                // first image, it was configured again a moment later, the menu's first second
                // flickering.
                let asked = Instant::now();
                while asked.elapsed() < std::time::Duration::from_secs(2) && { let s = window.inner_size(); (s.width, s.height) != size } {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                let got = window.inner_size();
                println!("Android: surface {}x{} after {} ms", got.width, got.height, asked.elapsed().as_millis());
            }
        }
        // new_windowed: every Err return comes before Self::build(..., library).
        // A failure therefore does not modify Library; Window/World/Library stay the same on fallback.
        // The local surface/adapter/device are released normally by Rust on Err.
        // Exclusive fullscreen only after the surface is created: once the display mode has
        // changed (winit's ChangeDisplaySettings), DXGI refuses any swap chain for the window
        // (DXGI_ERROR_INVALID_CALL, Direct3D 11 as well as DirectX 12); a chain already created follows the
        // mode change through ResizeBuffers. Same order as ToggleFullscreen during a game.
        match pollster::block_on(Renderer::new_windowed(window.clone(), self.world, self.library)) {
            Ok(mut renderer) => {
                let (x, y) = (x.max(1) as u32, y.max(1) as u32);
                let (x, y) = if self.original_fullscreen { openxiii_vm::video_settings::original_fullscreen_size(&self.display_modes, (x, y)) } else { (x, y) };
                self.fullscreen = fs && self.allow_fullscreen && set_fullscreen(window, x, y, self.borderless);
                if self.fullscreen {
                    let size_value = window.inner_size();
                    renderer.resize(size_value.width, size_value.height);
                }
                self.renderer = Some(renderer);
                1
            }
            Err(error) => { self.last_error = Some(error); 0 }
        }
    }

    fn call(&mut self, call: openxiii_vm::resize::OpenHostCall) -> Result<u32, String> {
        use openxiii_vm::resize::{OpenCall, OpenHostCall};
        match call {
            OpenHostCall::CreateWindow { width, height, .. } => { self.create_window(width, height); Ok(0) }
            OpenHostCall::Call(OpenCall::RenderDevice(_, x, y, fs)) => Ok(self.render_device(fs != 0, x, y)),
            OpenHostCall::Call(OpenCall::RenderDeviceOpen) => {
                // UObject vtable metadata: +84 is UpdateGamma, not InitInput.
                let renderer = self.renderer.as_mut().ok_or("device missing")?;
                renderer.gamma = self.fullscreen.then_some(self.color);
                Ok(0)
            }
            OpenHostCall::Call(OpenCall::Viewport90) => {
                // UObject vtable metadata: +90 is UpdateWindowFrame.
                self.window.as_ref().ok_or("window missing")?.set_title(&self.title);
                Ok(0)
            }
            OpenHostCall::Call(OpenCall::Log(text)) => {
                if text == "Opened viewport" && self.opened_log_already_emitted {
                    self.opened_log_already_emitted = false;
                } else {
                    println!("Log: {text}");
                }
                Ok(0)
            }
            OpenHostCall::Call(OpenCall::FailAssert(expression, _)) => Err(self.last_error.take().unwrap_or_else(|| format!("OpenWindow: {expression}"))),
            // The interactive producer never issues a temporary viewport request.
            OpenHostCall::Call(OpenCall::Malloc(_)) => Err("temporary viewport without a memory host".into()),
        }
    }
}

impl ApplicationHandler for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _math = self.game_math().enter();
        self.run_background_lockstep(event_loop);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let _math = self.game_math().enter();
        if let Some(window) = &self.window {
            // Android: back in the foreground, surface rebuilt on the new native window.
            if let Some(r) = &mut self.renderer {
                if let Err(e) = r.recreate_surface(window.clone()) {
                    eprintln!("error: surface: {e}");
                }
            }
            // A network co-op session: the other machine is told this one is back.
            #[cfg(target_os = "android")]
            if self.lockstep.is_some() || self.lockstep_intro.is_some() {
                self.lockstep_paused(false);
            }
            return;
        }
        let world = self.world.take().expect("world");
        let mut library = self.library.take().expect("library");
        let has_window = self.window.is_some();
        let has_renderer = self.renderer.is_some();
        let opened_log_already_emitted = self.game.as_ref().is_some_and(|g| g.vm.engine.client.initial_window_request().is_some());
        let mut host = InitialWindowHost {
            event_loop, title: if self.options.standalone { launcher::MARK.to_string() } else { format!("OpenXIII — {}", self.options.map) },
            allow_fullscreen: self.options.fullscreen,
            borderless: self.game.as_mut().is_some_and(|g| g.vm.borderless_fullscreen()),
            original_fullscreen: self.game.as_mut().is_some_and(|g| g.vm.original_fullscreen()),
            requested_size: self.game.as_ref().map_or((0, 0), |g| g.vm.engine.client.initial_window_request().map_or(g.vm.viewport, |request| request.requested_size)),
            color: self.game.as_ref().map_or([0.5, 0.5, 1.0], |g| g.vm.client_color),
            world: &world, library: &mut library,
            window: self.window.take(), renderer: self.renderer.take(), fullscreen: false,
            display_modes: Vec::new(), screen_native: self.screen_native, first_launch_size: None, last_error: None, opened_log_already_emitted,
        };
        let result = if let Some(g) = &mut self.game {
            match g.vm.open_initial_window(has_window, has_renderer, &mut |call| {
                let device_call = matches!(&call, openxiii_vm::resize::OpenHostCall::Call(openxiii_vm::resize::OpenCall::RenderDevice(..)));
                let result = host.call(call)?;
                let accepted_size = if device_call && result != 0 {
                    host.window.as_ref().map(|window| { let size = window.inner_size(); (size.width as i32, size.height as i32) })
                } else { None };
                Ok::<_, String>(openxiii_vm::client::InitialWindowReply { result, accepted_size })
            }) {
                Ok(true) => Ok(()),
                Ok(false) => {
                    // Existing path without an initial request (VM already loaded).
                    let (x, y) = g.vm.viewport;
                    host.create_window(x, y);
                    if host.render_device(g.vm.fullscreen, x, y) != 0 { Ok(()) }
                    else { Err(host.last_error.take().unwrap_or_else(|| "device missing".into())) }
                }
                Err(error) => Err(error),
            }
        } else {
            // Visit mode: same default size and no fullscreen, as before.
            host.create_window(0, 0);
            if host.render_device(false, 0, 0) != 0 { Ok(()) }
            else { Err(host.last_error.take().unwrap_or_else(|| "device missing".into())) }
        };
        self.display_modes = std::mem::take(&mut host.display_modes);
        self.screen_native = host.screen_native;
        if let Some(g) = &mut self.game {
            if let (Some((w, h)), Some(c)) = (host.first_launch_size, g.vm.config.as_mut()) {
                println!("First launch: fullscreen at the screen's native size {w}x{h}");
                openxiii_vm::video_settings::write_first_launch_size(c, w, h);
            }
            g.vm.fullscreen = host.fullscreen;
            g.vm.display_modes = self.display_modes.clone();
            g.vm.screen_native = self.screen_native;
        }
        let window = host.window.take();
        let renderer = host.renderer.take();
        drop(host); // releases the Library borrow before the characters are initialized.
        let renderer = match result { Ok(()) => renderer.ok_or_else(|| "device missing".to_string()), Err(error) => Err(error) };
        match renderer {
            Ok(mut r) => {
                if let Some(g) = &mut self.game {
                    let chars = g.characters(&mut library);
                    let (indices, batches, owners) = g.character_layout(&chars);
                    r.set_dynamic(&g.character_frame(&chars), &indices, &batches, &owners, &mut library);
                    self.characters = Some(chars);
                }
                self.renderer = Some(r);
            }
            Err(e) => {
                eprintln!("error: {e}");
                event_loop.exit();
            }
        }
        self.library = Some(library);
        let Some(window) = window else { return };
        if let Some(g) = &mut self.game {
            // Play mode: mouse captured in fullscreen (ResizeViewport: SetMouseCapture(1, 1, 0)); in a window,
            // on the first key or button press (ViewportWndProc).
            if g.vm.fullscreen {
                g.vm.win_set_drag(1);
                g.vm.win_set_mouse_capture(1, 1, 0);
            }
            g.vm.win_input.active = true;
            sync_capture(&window, &g.vm, &mut self.capture_view);
            self.looking = true;
        }
        // Android and Linux: the menus' pictures still to make are made now, at the size of
        // this screen, behind a "Loading..." screen (the game waits for them).
        #[cfg(not(windows))]
        self.start_install_pictures(&window);
        // The window, shown before the device and the game were made, takes the foreground now
        // (foreground.rs).
        foreground::bring_to_front(&window);
        self.window = Some(window);
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let _math = self.game_math().enter();
        // Improved build: behind the recap bubbles, a new press (not a key repeat, not a key
        // already down when the window got the focus) ends the wait and is not given to the game.
        // Alt, and any key pressed while Alt is held (Alt+Enter, Alt+F4, Alt+Tab), are window
        // shortcuts: they never end the recap wait and always reach the handling below.
        if self.recap_consumes_event(&event) { return; }
        match event {
            // Alt+F4 or the window's close button: the screen is left as by the "Quit" menu.
            WindowEvent::CloseRequested => {
                if self.game.is_some() {
                    self.leave_screen_clean();
                }
                event_loop.exit();
            }
            // Focus lost (Alt+Tab, Alt+Win): mouse released and exclusive fullscreen left
            // (window minimized), like the game when the application is deactivated; restored on
            // return. Without this, the locked cursor and exclusive video mode blocked Windows.
            WindowEvent::Focused(focused) if self.game.is_some() => {
                self.focused = focused;
                if focused {
                    // Window minimized: winit no longer redraws, whereas MainLoop was still running (OldTime tracked
                    // on each iteration, without Tick); the first frame on return only counts its own time.
                    self.run_loop.old_time = counter();
                }
                if let (Some(w), Some(g)) = (&self.window, &mut self.game) {
                    // WM_SETFOCUS / WM_KILLFOCUS of ViewportWndProc (windrv_input.rs).
                    g.vm.win_focus(focused);
                    if focused {
                        // WM_SIZE after restoring: ToggleFullscreen (+0x1B0), so SetMouseCapture(1, 1, 0).
                        if let Some((x, y)) = self.full_screen_suspended.take() {
                            // The window is restored first: entered while still minimized,
                            // exclusive fullscreen kept the title bar and remembered the minimized
                            // placement, so the next Alt+Enter minimized the game.
                            if w.is_minimized() == Some(true) {
                                w.set_minimized(false);
                            }
                            if set_fullscreen(w, x, y, false) {
                                g.vm.fullscreen = true;
                                g.vm.win_set_drag(1);
                                g.vm.win_set_mouse_capture(1, 1, 0);
                            }
                        }
                    } else {
                        self.alt_down = false;
                        // A borderless fullscreen window stays (no video mode to give back).
                        if g.vm.fullscreen && !g.vm.borderless_fullscreen() {
                            let size_value = w.inner_size();
                            self.full_screen_suspended = Some((size_value.width, size_value.height));
                            w.set_fullscreen(None);
                            w.set_minimized(true);
                            g.vm.fullscreen = false;
                        }
                    }
                }
            }
            WindowEvent::Focused(focused) => self.focused = focused,
            WindowEvent::Occluded(occluded) => self.occluded = occluded,
            // WM_MOUSEMOVE: position of the Windows mouse (UGameEngine::MousePosition), read by the menus when
            // the mouse is not captured (bWindowsMouseAvailable).
            WindowEvent::CursorMoved { position, .. } if self.game.is_some() => {
                self.cursor_value = (position.x.clamp(0.0, 65535.0) as u32, position.y.clamp(0.0, 65535.0) as u32);
                if let Some(g) = &mut self.game {
                    let (x, y) = self.cursor_value;
                    g.vm.win_message(0x200, 0, (y << 16) | x);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                    // Improved build: the new buffers shown black for about 100 ms at a steady
                    // rate before the menu comes back (a variable refresh rate screen showed the
                    // old frame through the change).
                    present_after_resize(r, self.game.is_some(), size);
                }
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                // A real key: back to keyboard and mouse (aim_on_pad).
                if !is_synthetic && event.state == ElementState::Pressed {
                    self.aim_on_pad = false;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    // Visit: Escape quits. In game, Escape opens the menu as in the game.
                    if code == KeyCode::Escape && self.game.is_none() {
                        event_loop.exit();
                    }
                    let pressed = event.state == ElementState::Pressed;
                    // A met co-op session's New game video: its skip keys.
                    if self.lockstep_intro_key(code, pressed) {
                        return;
                    }
                    // A co-op meeting's waiting sign: Escape gives it up (lockstep_play.rs).
                    if pressed && code == KeyCode::Escape && self.lockstep_meet_cancel() {
                        return;
                    }
                    if matches!(code, KeyCode::AltLeft | KeyCode::AltRight) {
                        self.alt_down = pressed;
                    }
                    // UWindowsViewport::ViewportWndProc: Alt + Enter toggles
                    // between fullscreen and window (ToggleFullscreen) and is not
                    // passed to the game.
                    if pressed && self.alt_down && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
                        if let (Some(g), Some(w)) = (&mut self.game, &self.window) {
                            toggle_fullscreen(&mut g.vm, w);
                        }
                        return;
                    }
                    // Gamepad settings page open: the keys go to it (releases still reach the
                    // game).
                    if self.gamepad_page_keyboard(code, pressed) {
                        return;
                    }
                    // "Select level" (choice_level.rs): list open, all keys go to it; on the
                    // main menu, L opens it.
                    if self.choice.unlocked {
                        if pressed {
                            let key_col = match code {
                                KeyCode::ArrowUp => Some(choice_level::Key::High),
                                KeyCode::ArrowDown => Some(choice_level::Key::Low),
                                KeyCode::PageUp => Some(choice_level::Key::PageHigh),
                                KeyCode::PageDown => Some(choice_level::Key::PageLow),
                                KeyCode::Home => Some(choice_level::Key::Start),
                                KeyCode::End => Some(choice_level::Key::EndEntry),
                                KeyCode::ArrowLeft => Some(choice_level::Key::LevelPrevious),
                                KeyCode::ArrowRight => Some(choice_level::Key::LevelNext),
                                KeyCode::KeyT => Some(choice_level::Key::Weapons),
                                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Some(choice_level::Key::Validate),
                                KeyCode::Escape | KeyCode::Backspace => Some(choice_level::Key::Return),
                                _ => None,
                            };
                            if let Some(url) = key_col.and_then(|t| self.choice.key_col(t)) {
                                self.leave_to(&url);
                            }
                        }
                        // Improved build: releases still reach the game (the list may have been
                        // opened by a menu box on a key press the game saw).
                        if menu_consumes_button(pressed) {
                            return;
                        }
                    }
                    if pressed && !event.repeat && code == KeyCode::KeyL && self.choice.key_menu() {
                        return;
                    }
                    // Improved build: Mods page open, the keys go to it (releases still reach
                    // the game).
                    if self.mods_key(code, pressed) { return; }
                    // Mods build only: a key bound to a mod action, if neither the viewer nor
                    // the player's own bindings use it.
                    #[cfg(feature = "mods")]
                    if let (Some(name), Some(g)) = (mod_key_name(code), self.game.as_mut()) {
                        if g.vm.mod_action_keys().iter().any(|k| k.eq_ignore_ascii_case(&name)) && mod_key_free(&name, g) {
                            if pressed && !event.repeat {
                                if let Some(pc) = g.controller {
                                    g.vm.mod_key(pc, &name);
                                }
                            }
                            return;
                        }
                    }
                    // "Cheats" build: F5, F6, F7 and F11 (cheats.rs); nothing in the normal build.
                    if let Some(t) = cheats::of_key(code) {
                        if pressed && !event.repeat && self.game.is_some() {
                            self.toggle_cheat(t);
                        }
                        return;
                    }
                    if let Some(g) = &mut self.game {
                        // Play mode: keys go to the window as on Windows. The keyboard
                        // state (GetKeyState) changes for all the key's codes, then
                        // ViewportWndProc receives WM_KEYDOWN / WM_KEYUP with the generic code (Shift,
                        // Ctrl, Alt: VK_SHIFT…), WM_SYSKEYDOWN / WM_SYSKEYUP with Alt (bit 29 of
                        // lParam); auto-repeat sends WM_KEYDOWN again. UpdateInput's keyboard
                        // scan then releases the keys that Windows no longer sees
                        // pressed (windrv_input.rs).
                        let vks = vk_codes(code);
                        for &vk in &vks {
                            g.vm.win_input.set_key(vk, pressed);
                        }
                        // Keys pressed or released outside the window, reported on focus
                        // change: Windows does not send them as messages, only its keyboard state changes.
                        if is_synthetic {
                            return;
                        }
                        let alt = self.alt_down || (!pressed && matches!(code, KeyCode::AltLeft | KeyCode::AltRight));
                        let lparam = 1 | (u32::from(alt) << 29) | (u32::from(event.repeat || !pressed) << 30) | (u32::from(!pressed) << 31);
                        let msg = match (pressed, alt) {
                            (true, false) => 0x100,
                            (false, false) => 0x101,
                            (true, true) => 0x104,
                            (false, true) => 0x105,
                        };
                        let mut action = openxiii_vm::windrv_input::WinAction::Nothing;
                        if let Some(&vk) = vks.first() {
                            action = g.vm.win_message(msg, u32::from(vk), lparam);
                        }
                        // WM_CHAR / WM_SYSCHAR (TranslateMessage): typed character to UEngine::Key.
                        if pressed {
                            if let Some(text) = &event.text {
                                for ch in text.chars().filter(|&c| (c as u32) < 256) {
                                    g.vm.win_message(if alt { 0x106 } else { 0x102 }, ch as u32, lparam);
                                }
                            }
                        }
                        match action {
                            openxiii_vm::windrv_input::WinAction::Closing => {
                                self.leave_screen_clean();
                                event_loop.exit();
                            }
                            openxiii_vm::windrv_input::WinAction::FullToggleScreen => {
                                if let Some(w) = &self.window {
                                    toggle_fullscreen(&mut g.vm, w);
                                }
                            }
                            openxiii_vm::windrv_input::WinAction::Nothing => {}
                        }
                    } else if pressed {
                        self.keys.insert(code);
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            // Visit mode: right click to look. Play mode: DirectInput mouse
            // buttons (IK_LeftMouse, IK_RightMouse, IK_MiddleMouse).
            WindowEvent::MouseInput { state, button, .. } if self.game.is_some() => {
                if state == ElementState::Pressed {
                    self.aim_on_pad = false;
                }
                // Gamepad settings page open: the left button is its own, the others do nothing.
                if button == MouseButton::Left && self.gamepad_page_click(state == ElementState::Pressed) {
                    return;
                }
                if self.gamepad_page.open && state == ElementState::Pressed {
                    return;
                }
                // DIMOFS_BUTTON0…2 (dwOfs 12, 13, 14), bit 0x80 when pressed.
                // Improved build: Mods page open, the left button is its own (a release whose
                // press it did not see reaches the game).
                if self.mods_mouse_button(button, state) { return; }
                if button == MouseButton::Left {
                    let (taken, url) = self.choice.click(state == ElementState::Pressed);
                    if let Some(url) = url {
                        self.leave_to(&url);
                    }
                    if taken {
                        return;
                    }
                }
                if self.choice.unlocked && menu_consumes_button(state == ElementState::Pressed) {
                    return;
                }
                let ofs = match button {
                    MouseButton::Left => 12,
                    MouseButton::Right => 13,
                    MouseButton::Middle => 14,
                    _ => return,
                };
                if let Some(g) = &mut self.game {
                    let pressed = state == ElementState::Pressed;
                    g.vm.win_input.active = true;
                    g.vm.win_input.push_mouse(ofs, if pressed { 0x80 } else { 0 });
                    // WM_LBUTTONDOWN…WM_MBUTTONUP to the window: capture is taken on the first press.
                    let msg = [0x201, 0x204, 0x207][(ofs - 12) as usize] + u32::from(!pressed);
                    let (x, y) = self.cursor_value;
                    g.vm.win_message(msg, 0, (y << 16) | x);
                }
            }
            // Wheel: DIMOFS_Z (dwOfs 8), 120 per notch; UpdateInput turns it into the IK_MouseW axis then
            // MouseWheelUp or MouseWheelDown pressed and released.
            WindowEvent::MouseWheel { delta, .. } if self.game.is_some() => {
                self.aim_on_pad = false;
                let z = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 120.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => p.y as i32,
                };
                if self.gamepad_page_wheel(z as f32 / 120.0) {
                    return;
                }
                if self.choice.unlocked {
                    self.choice.wheel(z as f32 / 120.0);
                    return;
                }
                if self.mods_mouse_wheel(z) { return; }
                if let Some(g) = &mut self.game {
                    g.vm.win_input.active = true;
                    g.vm.win_input.push_mouse(8, z as u32);
                }
            }
            // Touch screen: only for "Select level" (choice_level.rs) and the Mods page.
            WindowEvent::Touch(t) if self.game.is_some() => {
                let pos = (t.location.x as f32, t.location.y as f32);
                let phase = match t.phase {
                    winit::event::TouchPhase::Started => choice_level::Finger::Pose,
                    winit::event::TouchPhase::Moved => choice_level::Finger::Moves,
                    winit::event::TouchPhase::Ended => choice_level::Finger::Raised,
                    winit::event::TouchPhase::Cancelled => choice_level::Finger::Cancels,
                };
                // Improved build: the Mods page open takes the fingers.
                if self.mods_window_touch(t.id, phase, pos) { return; }
                if let Some(url) = self.choice.finger(t.id, phase, pos) {
                    self.leave_to(&url);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => {
                self.looking = state == ElementState::Pressed;
            }
            // Improved build: waiting behind the recap bubbles, the game does not run.
            WindowEvent::RedrawRequested if self.draw_loading_screen() => {}
            WindowEvent::RedrawRequested => {
                self.note_level_first_frame();
                // The Java loading screen goes once the game has drawn a frame of its own (the
                // first one prepares the level's textures and pipelines: seconds on a phone).
                #[cfg(target_os = "android")]
                touch_controls::game_frame();
                if let (Some(w), Some(g)) = (&self.window, &self.game) {
                    sync_capture(w, &g.vm, &mut self.capture_view);
                }
                // Real elapsed time (XIII.exe's MainLoop, unbounded): ULevel::Tick clamps it to [0.0005; 0.4] s.
                let dt = self.run_loop.delta(counter(), SECONDS_BY_CYCLE);
                #[cfg(target_os = "android")]
                self.apply_touch_controls(self.last.elapsed().as_secs_f32());
                #[cfg(not(target_os = "android"))]
                self.apply_cheats();
                self.last = Instant::now();
                // In single player (AllowBeingNotTicked: neither a pending level nor a network driver), the world only
                // advances if the window is in the foreground and not minimized; sound is paused when the
                // foreground is lost, resumed on return. The startup videos run before MainLoop.
                // Lockstep co-op: a session is a network game for this rule (the other machine needs
                // this one's steps): it keeps running without the foreground, sound included.
                let in_session = self.lockstep.is_some();
                let tick = match &mut self.game {
                    Some(g) => {
                        let network = g.vm.ipdrv.net_driver.is_some() || in_session;
                        let focused = self.focused;
                        let (focused, reduced) = (focused, self.window.as_ref().and_then(|w| w.is_minimized()).unwrap_or(false));
                        let f = self.run_loop.frame(false, || openxiii_vm::main_loop::allow_being_not_ticked(false, network), || focused && !reduced, g.vm.audio.is_some());
                        if let Some(hx) = g.vm.audio.as_mut() {
                            if f.pause_sound {
                                hx.pause_sound(0xFFF, false);
                            }
                            if f.resume_sound {
                                hx.resume_sound(0xFFF, false);
                            }
                        }
                        f.tick
                    }
                    None => {
                        true
                    }
                };
                let intro = match &mut self.game {
                    Some(g) => (self.intro_playing || !self.intro.is_empty()) && intro_step(&mut g.vm, &mut self.intro, &mut self.intro_playing, dt),
                    None => false,
                };
                let t_update = Instant::now();
                // The PC's real time for HXAudio (timeGetTime), from its current clock.
                // Lockstep co-op: the sound's clock is the session's steps, never the PC's.
                let lockstep_session = self.lockstep.is_some();
                if let Some(g) = &mut self.game {
                    if g.clock_hx.is_none() && !lockstep_session {
                        g.clock_hx = g.vm.audio_time().map(|t| (Instant::now(), t));
                        // Lockstep co-op: the sound's clock is the session's steps, not the PC's.
                        if g.clock_hx.is_some() && !g.vm.lockstep_on() {
                            g.vm.audio_set_real_time(true);
                        }
                    }
                }
                if !intro && tick {
                    self.update(dt);
                }
                // UPCVideoPlayerDevice: what the player displayed and its sound; UGameEngine::Draw
                // skipped during playback, the screen keeps the video's last frame.
                if let Some(g) = &mut self.game {
                    if let Some(p) = g.vm.engine.video.presented.take() {
                        self.video_show = Some(p);
                    }
                }
                self.apply_video_audio();
                if let Some(g) = &mut self.game {
                    if intro || g.vm.engine.video.draw_skipped {
                        g.vm.sounds.clear();
                        if let (Some(a), Some(hx)) = (&mut self.audio, g.vm.audio.as_mut()) {
                            a.sync(&mut hx.engine);
                        }
                        if let (Some(r), Some(library)) = (&mut self.renderer, &mut self.library) {
                            r.consume_engine_flush(&mut g.vm);
                            r.set_video(Some(self.video_show.as_ref().unwrap_or(&openxiii_vm::video::VideoPresent::Black)));
                            if let Err(e) = r.render_to_surface(&self.camera, library) {
                                eprintln!("render: {e}");
                            }
                        }
                        if let Some(req) = self.pending_travel.take() {
                            // XIIIPlayerController.PlayingVideo: ServerTravel when the video has
                            // finished (GetStatus 0). UGameEngine::Draw is no longer skipped: LoadMap
                            // does PaintProgress (the HUD's NextLoadScreen, XIII logo),
                            // then the loading animation, without the video's frame.
                            let playing = self.game.as_ref().is_some_and(|g| g.vm.video_status() == 1);
                            self.video_show = None;
                            if let Some(r) = &mut self.renderer {
                                r.set_video(None);
                            }
                            if !playing {
                                self.paint_progress();
                            }
                            self.travel_held(req);
                        }
                        if self.game.as_ref().is_some_and(|g| g.vm.quit_requested) {
                            self.leave_screen_clean();
                            event_loop.exit();
                            return;
                        }
                        if intro {
                            // MainLoop starts after the startup videos: OldTime taken again at their end.
                            // During the videos, the player's Tick receives all the real time since the
                            // start of the previous frame, drawing and screen wait included (otherwise the
                            // video plays in slow motion, more and more so in fullscreen).
                            let start_value = self.run_loop.new_time;
                            self.run_loop = openxiii_vm::main_loop::ClientLoop::new(counter(), self.run_loop.had_focus);
                            if self.intro_playing || !self.intro.is_empty() {
                                self.run_loop.old_time = start_value;
                            }
                        } else {
                            self.end_image();
                        }
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    self.video_show = None;
                    if let Some(r) = &mut self.renderer {
                        r.set_video(None);
                    }
                }
                // Travel started in this frame: UGameEngine::Tick does LoadMap before Client->Tick,
                // the old level is only drawn by PaintProgress.
                // Lockstep co-op: a late machine leaves this frame undrawn (lockstep_steps), the
                // sounds of its steps still sent.
                let skip_draw = std::mem::take(&mut self.lockstep_skip_draw) && self.lockstep.is_some();
                if skip_draw && tick && self.pending_travel.is_none() && !self.background_frame {
                    if let Some(g) = self.game.as_mut() {
                        g.vm.sounds.clear();
                        if let (Some(a), Some(hx)) = (&mut self.audio, g.vm.audio.as_mut()) {
                            a.sync(&mut hx.engine);
                        }
                    }
                }
                if tick && self.pending_travel.is_none() && !self.background_frame && !skip_draw {
                    // Lockstep co-op: what drawing draws at random is this machine's own.
                    if let Some(g) = self.game.as_mut().filter(|_| self.lockstep.is_some()) {
                        g.vm.lockstep_draw_begin();
                    }
                    self.draw_frame(dt);
                    if let Some(g) = self.game.as_mut() {
                        g.vm.lockstep_draw_end();
                    }
                }
                self.end_image();
                if let Some(req) = self.pending_travel.take() {
                    self.paint_progress();
                    self.travel_held(req);
                }
                // SETRES (video options): new window size.
                if let (Some(g), Some(w)) = (&mut self.game, &self.window) {
                    if std::mem::take(&mut g.vm.pending_toggle_fullscreen) {
                        toggle_fullscreen(&mut g.vm, w);
                    }
                    // Display mode chosen in the Video menu (improved build), with the size the
                    // page's SETRES asked for.
                    // Improved build: black for about 100 ms before the window or display mode
                    // changes, so that a variable refresh rate screen holds black, not the menu,
                    // while the change blocks.
                    present_before_mode_switch(g, self.renderer.as_mut());
                    // Android: the surface takes the chosen resolution (GameActivityXiii.java), as
                    // a monitor's display mode would.
                    #[cfg(target_os = "android")]
                    if let Some((x, y)) = g.vm.pending_resize.take() {
                        if let Some(size) = openxiii_vm::video_settings::android_surface_for(&g.vm.display_modes, (x, y)) {
                            touch_controls::want_surface(size);
                        }
                    }
                    if let Some(mode) = g.vm.pending_display_mode.take() {
                        let size = g.vm.pending_resize.take().unwrap_or_else(|| {
                            let s = w.inner_size();
                            (s.width, s.height)
                        });
                        let size = if mode == openxiii_vm::video_settings::FullscreenMode::Exclusive { g.vm.fullscreen_size(size) } else { size };
                        apply_display_mode(&mut g.vm, w, mode, size);
                    }
                    if let Some((x, y)) = g.vm.pending_resize.take() {
                        let borderless = g.vm.borderless_fullscreen();
                        if !(g.vm.fullscreen && (borderless || set_fullscreen(w, x, y, false))) {
                            let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(x, y));
                        }
                    }
                }
                // QUIT / EXIT ("Quit" menu): appRequestExit.
                if self.game.as_ref().is_some_and(|g| g.vm.quit_requested) {
                    self.leave_screen_clean();
                    event_loop.exit();
                    return;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }

    /// Android: moved to the background, the native surface is about to disappear; the ini files are written
    /// (the system may stop the game without notice afterwards).
    #[cfg(target_os = "android")]
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(r) = &mut self.renderer {
            r.drop_surface();
        }
        // A network co-op session: the game loop stops while in the background; the other
        // machine is told, so that it waits for this one instead of ending the session after
        // its usual timeout (the player came back to the menus).
        if self.lockstep.is_some() || self.lockstep_intro.is_some() {
            self.lockstep_paused(true);
        }
        if let Some(c) = self.game.as_mut().and_then(|g| g.vm.config.as_mut()) {
            c.flush();
        }
    }

    /// appPreExit → GConfig->Exit (FConfigCacheIni::Flush): the changed ini files are written.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // The window goes at once (exclusive fullscreen given back first): what follows, and the
        // end of the process, never leave it on the screen looking frozen.
        if let Some(w) = &self.window {
            if w.fullscreen().is_some() {
                w.set_fullscreen(None);
            }
            w.set_visible(false);
        }
        // Lockstep co-op: the game window closed during a session: the other machine is told
        // (it says the other player left, not that the connection was lost).
        if self.lockstep.is_some() {
            self.lockstep_end("this machine's game was closed");
        }
        if let Some(c) = self.game.as_mut().and_then(|g| g.vm.config.as_mut()) {
            c.flush();
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            // The mouse really moved (not a stray pixel): back to keyboard and mouse (aim_on_pad).
            if delta.0.hypot(delta.1) >= 2.0 {
                self.aim_on_pad = false;
            }
            // Play mode: raw mouse movements, like the DIMOFS_X (dwOfs 0) and
            // DIMOFS_Y (dwOfs 4) entries of the DirectInput buffer, one per moving axis; UpdateInput turns them into
            // IK_MouseX = dx and IK_MouseY = −dy.
            // Improved build: behind the recap bubbles the level does not run yet, the mouse
            // does not turn its view.
        if self.waiting_for_recap() { return; }
            if let Some(g) = &mut self.game {
                let (dx, dy) = (delta.0 as i32, delta.1 as i32);
                g.vm.win_input.active = true;
                if dx != 0 {
                    g.vm.win_input.push_mouse(0, dx as u32);
                }
                if dy != 0 {
                    g.vm.win_input.push_mouse(4, dy as u32);
                }
                return;
            }
            if self.looking {
                self.camera.yaw += delta.0 as f32 * 0.15;
                self.camera.pitch = (self.camera.pitch - delta.1 as f32 * 0.15).clamp(-89.0, 89.0);
            }
        }
    }
}

impl App {

    /// The travel, with the loading picture shown again at a steady rate while it loads
    /// (improved build, Renderer::hold_start): a variable refresh rate screen never waits on
    /// one frame for the whole load.
    fn travel_held(&mut self, mut req: openxiii_vm::travel::TravelRequest) {
        // The network co-op session is met here, then played in lockstep.
        if openxiii_vm::lockstep::wanted() && !self.lockstep_travel(&mut req) {
            return;
        }
        // A loading picture still kept for the other machine: the new travel shows its own.
        self.lockstep_reveal();
        self.travel(req);
        let hold = self.travel_hold.take();
        self.warm_after_travel(hold);
    }

    /// After a travel (its map built while `hold` shows the loading picture again): the level
    /// warmed up off screen, the graphics card idle, then the hold ends. Also the end of a map
    /// load behind the recap bubbles (recap_step).
    fn warm_after_travel(&mut self, hold: Option<renderer::PresentHold>) {
        let hold_used = hold.is_some();
        // Behind the loading picture, the new level drawn a few times off screen: its textures,
        // pipelines and targets are made before the first frame the player sees.
        if hold.is_some() {
            let started = Instant::now();
            // The level's first logic step, without game time (its first-step work: actors
            // starting, sounds, the HUD), then the drawings.
            let before = self.lockstep.as_ref().map(|r| r.step);
            self.update(0.0);
            if let (Some(b), Some(r)) = (before, self.lockstep.as_ref()) {
                eprintln!("lockstep: warm-up after travel ran steps {b}..{} (time left {:.4} s)", r.step, self.lockstep_time);
            }
            // The Select level start's equipment (its weapons and their sounds) given here too.
            self.give_start_equipment();
            // Its weapons are only given once MapInfo has played its first frame (bFirstTime,
            // after some game time): their sound banks are loaded here, so that the frame that
            // gives them does not load them.
            self.preload_start_equipment();
            let first_step = started.elapsed().as_millis();
            if let Some(r) = self.renderer.as_mut() {
                r.warming = true;
            }
            for _ in 0..3 {
                self.draw_frame(0.0);
            }
            if let Some(r) = self.renderer.as_mut() {
                r.warming = false;
            }
            renderer::diag(&format!("load: level warmed up behind the loading picture in {} ms (first logic step {first_step} ms)", started.elapsed().as_millis()));
        }
        if let (Some(hold), Some(r)) = (hold, self.renderer.as_mut()) {
            // The graphics card finishes the warm-up's uploads and drawings while the loading
            // picture still turns: the first frame of the level does not wait for them.
            let gpu = Instant::now();
            // Polled without blocking: the picture's thread keeps presenting meanwhile.
            r.wait_idle_sharing(std::time::Duration::from_secs(2));
            renderer::diag(&format!("load: graphics card idle after {} ms", gpu.elapsed().as_millis()));
            // Network co-op in lockstep: the picture stays until the other machine is in the
            // level too (lockstep_hold_frame), so that both players see it at the same time.
            match self.lockstep.as_ref().filter(|l| l.local_player().is_some()).map(|l| l.step) {
                Some(first) => {
                    self.lockstep_hold = Some((hold, first));
                    self.lockstep_hold_warm = 0;
                }
                None => r.hold_stop(hold),
            }
        }
        // Without the held loading picture (faithful build), the level's first frame is its
        // warm-up frame (the original's precache frame after LoadMap): the play starts with the
        // frame after it.
        if hold_used {
            openxiii_package::events::first_normal_frame();
        } else {
            PLAY_AFTER_NEXT_FRAME.store(2, std::sync::atomic::Ordering::Relaxed);
            // The original does not present its precache frame: the loading screen stays up.
            if let Some(r) = self.renderer.as_mut() {
                r.hide_next_frame = true;
            }
        }
        self.last = Instant::now();
    }

    /// The sound banks of the Select level equipment still to give (improved build with mods).
    fn preload_start_equipment(&mut self) {
        #[cfg(feature = "mods")]
        if let (Some((_, classes)), Some(game)) = (&self.weapons_for, &mut self.game) {
            game.vm.load_class_sounds_named(classes);
        }
    }

    /// Test equipment for the chosen start, on its map, after the starting inventory.
    fn give_start_equipment(&mut self) {
        // Lockstep co-op: only at a step (lockstep_steps), the same on every machine.
        if self.lockstep.is_some() && !self.lockstep_equipment_now {
            return;
        }
        if let (Some((map_value, classes)), Some(game)) = (&self.weapons_for, &mut self.game) {
            if choice_level::equipment_due(map_value, &self.options.map, character_screen_on(game)) && game.vm.equipment_test(classes).is_some() {
                // Improved build: the test weapons keep their sounds on maps whose bank lacks them.
                #[cfg(feature = "mods")]
                game.vm.load_class_sounds_named(classes);
                self.weapons_for = None;
            }
        }
    }

    /// UGameEngine::Draw of the frame: controller camera, HUD, scene.
    fn draw_frame(&mut self, dt: f32) {
        self.draw_frame_in(dt, false);
    }

    /// `whole`: one view over the whole window even in split screen (the loading screen).
    fn draw_frame_in(&mut self, dt: f32, whole: bool) {
        // Lockstep meeting: the waiting sign over the picture.
        if self.lockstep_meeting.is_some() {
            let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
            self.lockstep_sign_now = self.lockstep_sign(size.width.max(1) as f32, size.height.max(1) as f32);
        }
        {
            static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            // The level loaded before the window opened: it plays once its first frame is drawn.
            // After a travel without the held loading picture: once its warm-up frame is drawn.
            let after_travel = PLAY_AFTER_NEXT_FRAME.load(std::sync::atomic::Ordering::Relaxed);
            if after_travel > 0 {
                PLAY_AFTER_NEXT_FRAME.store(after_travel - 1, std::sync::atomic::Ordering::Relaxed);
            }
            if n == 2 || after_travel == 1 {
                openxiii_package::events::first_normal_frame();
            }
            if openxiii_package::events::window_frame(n) {
                std::process::exit(0);
            }
        }
        if let (Some(r), Some(g)) = (&mut self.renderer, &mut self.game) {
            r.consume_engine_flush(&mut g.vm);
            apply_render_quality(r, &mut g.vm);
            // What the level references, prepared with the scenery prewarm of the first frame.
            if r.wants_precache_roots() {
                r.set_precache_roots(g.precache_roots());
            }
            if r.wants_replaced_reserve() {
                if let Some(library) = &mut self.library {
                    r.reserve_replaced(g.replaced_capacity(library).as_ref());
                }
            }
        }
        // Split screen (several active viewports): one UGameEngine::Draw per viewport.
        // The co-op character screen too (improved build): its scene goes into a panel of the page.
        if !whole && self.game.as_ref().is_some_and(|g| g.active_viewports().len() > 1 || character_screen_on(g)) {
            self.draw_frame_views(dt);
            return;
        }
        // One view on the whole window: no split-screen cap left from a co-op session (it also
        // turns Hor+ off, Renderer::view_widen).
        if let Some(r) = &mut self.renderer {
            r.overlay_aspect_cap = None;
        }
        let mut step_value = Instant::now();
        let mut mark_step = |perf: &mut [f64; 9], k: usize| {
            let now = Instant::now();
            perf[k] += (now - step_value).as_secs_f64();
            step_value = now;
        };
        // UGameEngine::Draw: controller camera (PlayerCalcView), then the HUD (PostRender,
        // which places the held weapon), then the scene with the actors up to date.
        if let Some(g) = &mut self.game {
            if let Some((location, rotation)) = g.calc_view() {
                self.camera = Camera { position: location, yaw: degrees(rotation[1]), pitch: degrees(rotation[0]), roll: degrees(rotation[2]) };
            }
        }
        let mut pending_targets = Vec::new();
        if let (Some(r), Some(g)) = (&mut self.renderer, &mut self.game) {
            let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
            r.fov = g.fov();
            r.only_post_render = g.vm.only_post_render;
            r.gamma = crate::gamma::applies(g.vm.fullscreen, g.vm.engine.client.session_graphics).then_some(g.vm.client_color);
            // FCameraSceneNode::Render: PreRender, scene (visibility), HUD.
            let fov = g.fov().unwrap_or(85.0);
            let cam = self.camera;
            let (mut quads, targets) = g.render_frame(size.width.max(1), size.height.max(1), cam.position, cam.yaw, cam.pitch, fov);
            // Standalone game: "Select level" on top of the main menu (choice_level.rs).
            // No more "OpenXIII 0.1" text on the menu.
            if self.options.standalone && self.options.map.eq_ignore_ascii_case("MapMenu") {
                let gamepad = self.pads.as_ref().is_some_and(|p| p.count() > 0);
                self.choice.draw_fn(&mut g.vm, size.width, size.height, gamepad, &mut quads);
            } else {
                self.choice.hide();
            }
            // Improved build: the main menu's Mods box and the Load page's Select level entry
            // (the VM's menu_boxes.rs), and the Mods page over the menu.
            // The Co-op box opens the VM's own Co-op page (coop_page.rs).
            self.viewer.draw_mods_menu(g, self.pads.as_ref(), size, &mut quads);
            // Gamepad settings page over the menu (gamepad_page.rs).
            if self.gamepad_page.open {
                let gamepad = self.pads.as_ref().is_some_and(|p| p.count() > 0);
                let config = self.gamepad_config;
                // Laid out on the menus' screen and placed like them (gui.rs, MenuScreen).
                let screen = g.vm.menu_screen(size.width.max(1), size.height.max(1));
                self.gamepad_page.draw_on_screen(&mut g.vm, &config, screen, gamepad, &mut quads);
            }
            cheats::draw_message(&self.cheats, g, &mut quads);
            quads.extend(std::mem::take(&mut self.lockstep_sign_now));
            r.consume_engine_flush(&mut g.vm);
            // URenderTargetMaterial::Update renders the scene at PostRender time,
            // characters in this frame's pose: rendered further down, after they are updated
            // (otherwise a comic panel showed the previous frame, without a character that just arrived).
            pending_targets = targets;
            r.set_hud(&quads);
            r.fx_passes = g.screen_fx();
    r.cartoon_flash = g.vm.screen_fx.cartoon_flash;
            r.cartoon_flash = g.vm.screen_fx.cartoon_flash;
            r.vignette = g.vignette();
            r.flash = g.flash_tile(g.controller);
            r.clear_volume = g.vm.view_volume_clear(g.controller);
            r.volume_fog = g.vm.view_volume_fog(g.controller);
            r.color_filter = g.color_filter;
            g.vm.sounds.clear();
            if let (Some(a), Some(hx)) = (&mut self.audio, g.vm.audio.as_mut()) {
                a.sync(&mut hx.engine);
            }
        }
        mark_step(&mut self.perf_steps, 0);
        // Actors created or destroyed since the previous frame: list of drawn meshes
        // rebuilt (the engine walks its actors every frame).
        let mut baked_moved = None;
        if let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) {
            // Actors of the fixed geometry moved: once for the signature, the changed meshes
            // and the frame's hidden actors.
            let t_a = std::time::Instant::now();
            let moved = g.baked_moved_image();
            let t_b = std::time::Instant::now();
            g.list_characters(r, library, &mut self.characters, &moved);
            let t_c = std::time::Instant::now();
            // StaticMeshes changed by the script (Amos01: DisplayScreen).
            let replaces = g.replaced_world_with(library, false, &moved);
            let t_d = std::time::Instant::now();
            let to_replace = replaces.is_some();
            if let Some(w) = replaces {
                r.set_replaced(w.as_ref());
            }
            let t_e = std::time::Instant::now();
            if game::dbg_reason() && (t_e - t_a).as_secs_f64() > 0.001 {
                let ms = |x: std::time::Instant, y: std::time::Instant| (y - x).as_secs_f64() * 1000.0;
                renderer::diag(&format!("list reason t={:.3} actor list step: moved {:.3}, character list {:.3}, replaced scenery {:.3} (redone: {to_replace}), set_replaced {:.3} ms", game::dbg_time(), ms(t_a, t_b), ms(t_b, t_c), ms(t_c, t_d), ms(t_d, t_e)));
            }
            baked_moved = Some(moved);
        }
        mark_step(&mut self.perf_steps, 1);
        if let (Some(r), Some(g), Some(c)) = (&mut self.renderer, &mut self.game, &self.characters) {
            let fov = g.fov().unwrap_or(85.0);
            let (weather, c2w) = g.weather(self.camera.position, self.camera.yaw, self.camera.pitch);
            r.set_weather(weather.as_ref(), &c2w);
            let verts = g.character_frame(c);
            g.character_cartoon_in(c, &verts, r.cartoon_mut());
            let light_views = g.scene_light_leaves(c);
            r.set_scene_light_leaves(light_views);
            r.set_dyn_lights(&g.dyn_light_records());
            r.set_prop_lights(g.prop_lights(c, &verts));
            r.set_hidden_owners(match &baked_moved {
                Some(moved) => g.hidden_baked_with(moved),
                None => g.hidden_baked(),
            });
            r.set_texture_replacements(&g.vm.texture_replacements);
            mark_step(&mut self.perf_steps, 2);
            self.shadow_frame = self.shadow_frame.wrapping_add(1);
            let jobs = g.update_shadows(c, self.camera.position, &|k| r.cartoon_dir(k), self.shadow_frame, None);
            if !jobs.is_empty() {
                g.for_each_shadow_mesh(c, &jobs, |first, mesh, id| r.set_shadow_vertices_id(first, mesh, id));
            }
            r.set_shadow_jobs(jobs);
            mark_step(&mut self.perf_steps, 3);
            r.update_dynamic(&verts, &g.first_person(c));
            r.set_ignore_vignette(&g.ignore_vignette(c));
            r.set_vignette_owners(g.vignette_baked());
            r.set_translucent_keys(g.translucent_keys(c));
            mark_step(&mut self.perf_steps, 4);
            let (sprites, meshes) = g.particle_sprites(&self.camera);
            let model = r.model();
            let projectors = g.projector_draws(&self.camera, model.as_deref(), fov);
            r.set_trails(g.trail_batches(&self.camera));
            if let Some(library) = self.library.as_mut() {
                r.set_particles(&sprites, &meshes, library);
                r.set_bsp_projectors(&projectors, library);
            }
        }
        mark_step(&mut self.perf_steps, 5);
        if let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) {
            render_targets(r, g, self.characters.as_ref(), &pending_targets, library);
        }
        mark_step(&mut self.perf_steps, 6);
        if let (Some(r), Some(library)) = (&mut self.renderer, &mut self.library) {
            // Engine time for the texture modifiers (GEngine->CurrentFrameCount).
            r.frame.count = r.frame.count.wrapping_add(1);
            r.frame.delta = dt;
            let drawn = self.game.as_ref().map(|g| g.sine_drawn());
            r.update_sine(library, drawn.as_deref());
            r.set_bsp_visible(self.game.as_ref().and_then(|g| g.bsp_visible()));
            mark_step(&mut self.perf_steps, 7);
            if let Err(e) = r.render_to_surface(&self.camera, library) {
                eprintln!("render: {e}");
            }
        }
        mark_step(&mut self.perf_steps, 8);
    }

    /// Split-screen frame: UWindowsClient::Tick draws each active viewport
    /// (UGameEngine::Draw: PlayerCalcView, HUD, visibility of its player) in its rectangle,
    /// with the same stages as `draw_frame` per view. Screen effects (Apply2DEffect) and
    /// DrawPortal are not handled here yet.
    fn draw_frame_views(&mut self, dt: f32) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let client = (size.width.max(1), size.height.max(1));
        let Some(slots) = self.game.as_ref().map(|g| g.active_viewports()) else { return };
        // Lockstep co-op: only this machine's player's view, made by the last step.
        let slots = if self.lockstep.is_some() { slots.into_iter().take(0).collect::<Vec<u8>>() } else { slots };
        let lockstep_view = if self.lockstep.is_some() { self.lockstep_view.clone() } else { None };
        // Actors created or destroyed: list of drawn meshes rebuilt, once per frame.
        if let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) {
            let moved = g.baked_moved_image();
            g.list_characters(r, library, &mut self.characters, &moved);
        }
        let Some(r) = self.renderer.as_mut() else { return };
        r.frame.count = r.frame.count.wrapping_add(1);
        r.frame.delta = dt;
        let (frame, target) = match r.begin_frame() {
            Ok(Some(f)) => f,
            Ok(None) => return,
            Err(e) => {
                eprintln!("render: {e}");
                return;
            }
        };
        self.shadow_frame = self.shadow_frame.wrapping_add(1);
        // Lockstep meeting under way (the co-op character screen once confirmed): its waiting
        // sign, in window pixels, over the last view drawn.
        let mut meet_sign = std::mem::take(&mut self.lockstep_sign_now);
        let last = slots.len().saturating_sub(1);
        for (n, slot) in slots.into_iter().enumerate() {
            let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) else { break };
            // This view's player's gamepad settings page (gamepad_page.rs).
            let mut extra = Vec::new();
            let page = match usize::from(slot) {
                0 => Some(&mut self.gamepad_page),
                n => self.other_gamepad_pages.get_mut(n - 1),
            };
            if let Some(page) = page.filter(|p| p.open) {
                // The players after the first drive their page with their own gamepad.
                #[cfg(not(target_os = "android"))]
                let gamepad = slot > 0 || self.pads.as_ref().is_some_and(|p| p.count() > 0);
                #[cfg(target_os = "android")]
                let gamepad = true;
                let rect = g.vm.view_rect_of_slot(usize::from(slot), (client.0 as i32, client.1 as i32)).map(|v| v as f32);
                page.draw(&mut g.vm, &self.gamepad_config, client.0, rect, gamepad, &mut extra);
            }
            if n == last {
                extra.append(&mut meet_sign);
            }
            if let Some(camera) = draw_viewport_view(r, g, library, self.characters.as_ref(), slot, client, self.shadow_frame, &target, &extra) {
                self.camera = camera;
            }
        }
        self.draw_lockstep_view(lockstep_view, client, &target);
        if let (Some(g), Some(r)) = (&mut self.game, &mut self.renderer) {
            g.vm.sounds.clear();
            if let (Some(a), Some(hx)) = (&mut self.audio, g.vm.audio.as_mut()) {
                a.sync(&mut hx.engine);
            }
            r.present(frame);
        }
    }

    /// UGameEngine::PaintProgress, called by LoadMap before loading the next
    /// map: one frame of the old map with LevelAction = LEVACT_Loading (the HUD draws
    /// the loading screen), Pauser cleared.
    fn paint_progress(&mut self) {
        openxiii_package::events::loading_again("(loading screen)");
        let level = match &mut self.game {
            Some(g) => g.vm.level,
            None => return,
        };
        let Some(level) = level else { return };
        if let Some(g) = &mut self.game {
            g.vm.set_prop(level, "LevelAction", openxiii_vm::value::Value::Byte(1));
            g.vm.set_prop(level, "Pauser", openxiii_vm::value::Value::Object(None));
        }
        // Improved build: the loading screen is the whole window's, in split screen and when
        // leaving the co-op character screen too (one picture, not one per player's view).
        self.draw_loading_picture();
        // Improved build: the loading picture presented a few more times before the load blocks
        // the loop. Some display paths (driver frame generation, overlays) show a frame made
        // between the last two presented ones: with a single present the screen kept that mix of
        // the page left and the loading picture for the whole load.
        // Spaced in time (an interpolator follows the presentation timing), about 120 ms in all;
        // every travel goes through here (level to level, quitting to the main menu, split screen).
        self.present_loading_frames();
        if let Some(r) = self.renderer.as_mut() {
            r.hold_capture = false;
        }
        if let Some(g) = &mut self.game {
            g.vm.set_prop(level, "LevelAction", openxiii_vm::value::Value::Byte(0));
        }
    }

    /// One frame of the loading animation (UEngLoadingAnimationThread::RenderAnimation):
    /// black screen, the VM's Canvas primitives, no scene.
    fn loading_frame(&mut self, anim: &mut openxiii_vm::loading_anim::LoadingAnimation) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let (Some(r), Some(g), Some(library)) = (&mut self.renderer, &mut self.game, &mut self.library) else { return };
        g.vm.loading_animation_frame(anim, size.width.max(1), size.height.max(1));
        let saved = r.only_post_render;
        r.only_post_render = true;
        r.set_hud(&g.vm.hud);
        if let Err(e) = r.render_to_surface(&self.camera, library) {
            eprintln!("render: {e}");
        }
        r.only_post_render = saved;
    }

    /// UGameEngine::Browse then LoadMap: new map (geometry, rendering, logic, sound); the
    /// player takes back the inventory exported by the previous map.
    fn game_math(&self) -> openxiii_math::Mode {
        if self.lockstep.is_some() { return openxiii_math::Mode::Portable; }
        self.game.as_ref().map(|g| g.vm.session.math).unwrap_or_default()
    }

    fn travel(&mut self, mut req: openxiii_vm::travel::TravelRequest) {
        let _math = self.game_math().enter();
        // Improved build: the mods' scripts hear the change of map first (on("map_end"),
        // on("loading")), the old level still there; one may hold the start behind its own
        // loading screen (hold_start()).
        let held = self.begin_loading_recap(&req);
        openxiii_package::events::loading_again(req.url.map.trim_end_matches(".unr"));
        // Rest of LoadMap after PaintProgress (drawn by the caller with the old controller and
        // its pawn): the viewports' actors destroyed, the engine handed over to the travel.
        if req.engine.is_none() {
            if let Some(g) = &mut self.game {
                g.vm.engine_load_map_continued(&mut req);
            }
        }
        self.travel_started = Instant::now();
        let game_dir = self.options.game.clone();
        let map = map_stem(&req.url);
        println!("travel to {}", req.url.string());
        // Faithful build: the level's movers found here, on the game thread, as before (the
        // improved build finds them on the worker thread, load_level).
        let exclusions = level_exclusions(&game_dir, &map);
        // UWinLoadingAnimationThreadLauncher: during LoadMap, the loading animation is
        // redrawn every 75 to 124 ms (loading_anim.rs).
        // The animation thread has its own rand() (multithreaded msvcr70: _tiddata->_holdrand,
        // 1 in a new thread); the thread and the object last for the whole session.
        if self.loading_anim.is_none() {
            self.loading_anim = self.game.as_mut().map(|g| openxiii_vm::loading_anim::LoadingAnimation::new(&mut g.vm, 1));
        }
        // A held start: the mod's loading screen stays while the map loads in the background and
        // until the mod releases it (recap_step); the event loop keeps running meanwhile.
        if held { self.start_loading_recap(req, map, &game_dir); return; }
        let mut anim = self.loading_anim.take();
        // Improved build: the animation's frames are kept, so that its turn goes on once the
        // rest of the travel blocks this thread (Renderer::hold_start).
        self.prepare_loading_frame();
        // Improved build: the packages already read since everyone confirmed on the co-op
        // character screen (preload_step), when they are this map's.
        let preloaded = self.take_preloaded_level(&map);
        let loaded = if let Some(job) = preloaded {
            renderer::diag(&format!("load: {map}'s packages read during the confirmations ({})", if job.is_finished() { "ready" } else { "still reading" }));
            while !job.is_finished() {
                let Some(a) = anim.as_mut() else { break };
                self.loading_frame(a);
                let until = Instant::now() + std::time::Duration::from_millis(a.pause_ms());
                while !job.is_finished() && Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            renderer::diag(&format!("load: waited {} ms for the preloaded packages", self.travel_started.elapsed().as_millis()));
            job.join().unwrap_or_else(|_| Err("loading: panic".to_string()))
        } else { std::thread::scope(|scope| {
                let job = scope.spawn(openxiii_math::inherit(|| load_travel_level(&game_dir, &map, &exclusions)));
            while !job.is_finished() {
                let Some(a) = anim.as_mut() else { break };
                self.loading_frame(a);
                let pause = a.pause_ms();
                let until = Instant::now() + std::time::Duration::from_millis(pause);
                while !job.is_finished() && Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            job.join().unwrap_or_else(|_| Err("loading: panic".to_string()))
        }) };
        // Improved build: the rest of the travel (the level built, its actors spawned) blocks
        // this thread: the animation's kept frames are shown in turn by another thread, a few
        // more made off screen to give it its whole turn. Ended by `travel_held`.
        self.begin_level_hold(&mut anim);
        self.loading_anim = anim;
        self.finish_travel(req, map, loaded);
    }

    /// End of UGameEngine::LoadMap once the map is read: the new level, its rendering and sound.
    fn finish_travel(&mut self, mut req: openxiii_vm::travel::TravelRequest, map: String, loaded: Loaded) {
        let game_dir = self.options.game.clone();
        let (world, mut library) = match loaded {
            Ok(x) => x,
            Err(e) => {
                eprintln!("travel to {map}: {e}");
                return;
            }
        };
        let name = req.url.get_option("NAME=").map(str::to_string);
        // ULevel::SpawnPlayActor: the inventory of the player with this name, otherwise the first one (single game).
        let info = req.info.iter().find(|(n, _)| Some(n) == name.as_ref()).or(req.info.first()).map(|(_, t)| t.clone());
        // Lockstep co-op, the session's start map: a new VM, as a game started on that map, so
        // that both machines begin from the same state whatever they did before (menus, maps).
        let fresh = std::mem::take(&mut self.lockstep_fresh);
        // After a lockstep session: back to the menus in a new VM with this machine's own
        // settings and language (the session's VM had the host's, written nowhere).
        let restore = std::mem::take(&mut self.lockstep_restore) && !fresh && self.lockstep.is_none();
        // This machine's player's profile in use (its saves), kept by the new VM.
        let kept_save = if fresh || restore { req.engine.as_ref().and_then(|e| e.save.clone()) } else { None };
        // The window's own input state (WinDrv: the mouse capture taken, the keys held) goes on
        // in the new VM, as the window does: a new one had no capture, so the menus after a
        // session dropped the mouse (moves and buttons) until a key was pressed.
        let kept_win = if fresh || restore { self.game.as_ref().map(|g| g.vm.win_input.carried()) } else { None };
        // So does the window's mode: a new VM said "windowed" (the Video page's Display) for a
        // window in fullscreen, at a session's start and back in the menus after it.
        let kept_fullscreen = if fresh || restore { self.game.as_ref().map(|g| g.vm.fullscreen) } else { None };
        // (After a session the engine handed over is kept, as on any return to the menus: its
        // client and menus' state, so that the menus go on without the launch's profile page.)
        if fresh {
            req.engine = None;
        }
        let start_state = if fresh { self.lockstep_start_state.take() } else { None };
        if fresh {
            // (Kept when the meeting began too: a later page may have saved an empty list.)
            if let Some(m) = self.game.as_ref().and_then(|g| g.vm.saved_menu_stack()) {
                self.lockstep_menus = Some(m);
            }
        }
        let bindings = load_bindings(&game_dir);
        // UGameEngine::LoadMap: the level being left is removed from the VM, which loads the next one.
        if let Some(p) = self.game.as_mut().and_then(|g| g.vm.take_local_play_change()) {
            self.local_play = p;
        }
        let local_play = self.local_play;
        let fresh_vm = fresh.then(|| {
            let mut vm = openxiii_vm::Vm::default();
            if let Some(session) = self.lockstep.as_ref().and_then(|r| r.session.as_ref()) { vm.session = session.clone(); }
            if let Err(e) = vm.load_dir(&openxiii_package::system_dir(&game_dir)) {
                eprintln!("lockstep: {e}");
            }
            vm.init_engine_config();
            // The session's settings (the host's files), written nowhere.
            if let Some(c) = vm.config.as_mut() {
                c.read_only = true;
            }
            vm.set_local_play(local_play);
            vm.set_local_players(local_play.local_players);
            // Each player's own settings from the first step of the map on.
            if let Some(r) = self.lockstep.as_ref() {
                vm.lockstep_start(r.sizes.clone());
                vm.lockstep_set_profiles(r.profiles.clone());
            }
            // The host's start: its music before the map loads.
            if let Some(e) = start_state.as_ref() {
                vm.lockstep_apply_start_entries(e);
            }
            let late = false;
            if !late {
                vm.audio_init(&game_dir, &map);
            }
            vm
        });
        let restored_vm = restore.then(|| {
            let mut vm = openxiii_vm::Vm::default();
            // Back to the menus: a new VM, seeded like the game's launch.
            game::launch_seed(&mut vm);
            if let Err(e) = vm.load_dir(&openxiii_package::system_dir(&game_dir)) {
                eprintln!("lockstep: {e}");
            }
            vm.init_engine_config();
            let mut lp = local_play;
            lp.coop = None;
            lp.local_players = 1;
            lp.characters = None;
            vm.set_local_play(lp);
            vm.set_local_players(1);
            // The menus come back on the pages left for the session, as on any return to the
            // menus (XIIIRootWindow's SaveMenuStack), not on the start pages.
            vm.lockstep_menus = self.lockstep_menus.take();
            eprintln!("lockstep: back to the menus with this machine's own settings");
            vm
        });
        if restore {
            let mut lp = self.local_play;
            lp.coop = None;
            lp.local_players = 1;
            lp.characters = None;
            self.local_play = lp;
        }
        let mut reuse = if fresh { fresh_vm } else if restore { restored_vm } else { self.game.as_mut().map(|g| {
            let mut vm = std::mem::take(&mut g.vm);
            vm.unload_level();
            vm.set_local_play(local_play);
            // Players on this screen chosen in the menus, given to the engine's split-screen
            // multiplayer setting (split_versus.rs) before the multiplayer map logs its players in.
            vm.set_local_players(local_play.local_players);
            // The engine handed over to the travel replaces the VM's (Game::new): it carries the
            // setting too.
            if let Some(e) = req.engine.as_mut() {
                e.client.local_players = vm.engine.client.local_players;
            }
            vm
        }) };
        // After a session the engine handed over is the session's: its configuration (the host's
        // language and User.ini, read only) would replace the new VM's (Game::new). The new VM's
        // own configuration (this machine's settings and language) goes with the engine instead.
        if restore {
            if let (Some(e), Some(vm)) = (req.engine.as_mut(), reuse.as_mut()) {
                if vm.config.is_some() {
                    e.config = vm.config.take();
                }
            }
            let keep = false;
            if let Some(e) = req.engine.as_mut().filter(|_| !keep) {
                e.client.lockstep_sizes = None;
                e.client.local_players = 1;
                e.client.wanted = 1;
            }
        }
        let mut g = match game::Game::new(
            &game_dir,
            &req.url,
            world.collision.clone(),
            world.collision_tags.clone(),
            world.collision_materials.clone(),
            &mut library,
            &bindings,
            info.as_deref(),
            req.engine,
            reuse,
            Default::default(),
        ) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("travel to {map}: {e}");
                return;
            }
        };
        // Lockstep co-op: this machine's profile and the host's checkpoints in the new VM's save
        // device (its own saves, a later Retry).
        if let (Some(d), Some(old)) = (g.vm.engine.save.as_mut(), kept_save.as_ref()) {
            d.take_profile_of(old);
        }
        if let Some(w) = kept_win {
            g.vm.win_input = w;
        }
        if let Some(f) = kept_fullscreen {
            g.vm.fullscreen = f;
        }
        if let Some(e) = start_state.as_ref() {
            let saves: Vec<(String, Vec<u8>)> = e.iter().filter(|x| x.0.starts_with("start/save_")).cloned().collect();
            g.vm.lockstep_apply_start_entries(&saves);
        }
        g.vm.display_modes = self.display_modes.clone();
        g.vm.screen_native = self.screen_native;
        refresh_level_graphics(&mut g, &game_dir);
        let Some(window) = self.window.clone() else { return };
        // Same surface and same device (Renderer::rebuild): the loading screen presented
        // by paint_progress stays displayed during loading, like the original. Recreating the
        // surface left the window white for one to two seconds.
        let prior = self.renderer.take();
        self.characters = None;
        self.game = None;
        let fresh_list_new = match prior {
            Some(r) => Ok(r.rebuild(&world, &mut library)),
            None => pollster::block_on(Renderer::new_windowed(window.clone(), &world, &mut library)),
        };
        match fresh_list_new {
            Ok(mut r) => {
                let chars = g.characters(&mut library);
                let (indices, batches, owners) = g.character_layout(&chars);
                r.set_dynamic(&g.character_frame(&chars), &indices, &batches, &owners, &mut library);
                self.characters = Some(chars);
                self.renderer = Some(r);
            }
            Err(e) => {
                eprintln!("render of {map}: {e}");
                return;
            }
        }
        self.library = Some(library);
        self.game = Some(g);
        if openxiii_package::env::var("OXIII_MUTE").is_err() {
            self.audio = audio::Audio::new(&game_dir, &map);
        }
        println!("travel to {map}: {:.0} ms", self.travel_started.elapsed().as_secs_f64() * 1000.0);
        renderer::diag(&format!("load: travel to {map} took {:.0} ms", self.travel_started.elapsed().as_secs_f64() * 1000.0));
        window.set_title(&if self.options.standalone { launcher::MARK.to_string() } else { format!("OpenXIII — {map}") });
        // The Select level equipment belongs to the start chosen on its page: back on the main
        // menu before its level was reached (the co-op character screen left with Back), it is
        // dropped, so that a later game starting on the same map does not receive it.
        if self.weapons_for.as_ref().is_some_and(|(target, _)| choice_level::equipment_dropped_on(target, &map)) {
            self.weapons_for = None;
        }
        // The equipment lasts the run of its level: given again when the same level is loaded
        // again (Retry, a death's restart), dropped on the main menu, when a save is loaded or
        // on another level.
        let reads = self.game.as_mut().and_then(|g| g.vm.save_device().map(|d| d.slot_reads)).unwrap_or(0);
        let save_loaded = std::mem::replace(&mut self.slot_reads_seen, reads) != reads;
        let screen = self.game.as_ref().is_some_and(character_screen_on);
        if let Some((target, classes)) = &self.loadout {
            match choice_level::loadout_after_travel(target, &map, screen, save_loaded) {
                choice_level::LoadoutStep::Keep => {}
                choice_level::LoadoutStep::Give => self.weapons_for = Some((target.clone(), classes.clone())),
                choice_level::LoadoutStep::Drop => {
                    self.loadout = None;
                    self.weapons_for = None;
                }
            }
        }
        self.options.map = map;
        self.last = Instant::now();
    }

    /// End of a MainLoop iteration: CurrentTickRate (UEngine +0xB8, frames / cumulative seconds once one
    /// second has passed), then EnforceTickRate: appSleep until 1 / GetMaxTickRate (120 in single player) since the start
    /// of the frame. Only for the windowed game: captures keep their fixed step.
    fn end_image(&mut self) {
        let Some(g) = &mut self.game else { return };
        // Not in the original: the refresh rate of the window's screen, read again each second
        // (window moved to another screen, display mode changed); an enhanced LAN host draws at it.
        thread_local!(static DISPLAY_READ: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) });
        if DISPLAY_READ.get().is_none_or(|t| t.elapsed().as_secs_f32() >= 1.0) {
            DISPLAY_READ.set(Some(Instant::now()));
            let millihertz = self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz());
            g.vm.engine.client.display_hz = millihertz.map(|m| (m + 500) / 1000).filter(|&hz| hz > 0);
        }
        if let Some(r) = self.run_loop.after_tick() {
            g.vm.engine.tick_rate.current = r;
        }
        if let Some(s) = self.run_loop.sleep_seconds(g.vm.net_max_tick_rate(), counter(), SECONDS_BY_CYCLE) {
            // appSleep: Sleep(_ftol(seconds × 1000)).
            std::thread::sleep(std::time::Duration::from_millis((s as f64 * 1000.0) as u32 as u64));
        }
    }

    /// Start chosen in "Select level": like loading a save from that point.
    fn leave_to(&mut self, url: &str) {
        if let Some(g) = &mut self.game {
            if g.vm.from_menu_main(url) {
                let map_value = url.split('#').next().unwrap_or(url).to_string();
                let classes = self.choice.classes_weapons();
                self.weapons_for = (!classes.is_empty()).then_some((map_value, classes));
                self.loadout = self.weapons_for.clone();
                self.slot_reads_seen = g.vm.save_device().map_or(0, |d| d.slot_reads);
            } else {
                eprintln!("Select level: {url} could not start");
            }
        }
    }

    fn update(&mut self, dt: f32) {
        // A lockstep meeting under way goes on (lockstep_play.rs).
        if self.lockstep_meeting.is_some() {
            self.lockstep_meet_poll();
        }
        // A met session's New game video (lockstep_play.rs).
        if self.lockstep_intro.is_some() {
            self.lockstep_intro_poll();
        }
        // Why a network game stopped, told on the menus.
        self.lockstep_notice_frame();
        // The menu pages kept when the menus are left (read only), for the menus after a
        // lockstep session.
        if self.lockstep.is_none() {
            if let Some(g) = self.game.as_mut() {
                g.vm.note_menus_left();
            }
        }
        self.preload_step();
        let mut pad_url = None;
        let mut pad_states = None;
        if let (Some(pads), Some(game)) = (&mut self.pads, &mut self.game) {
            // Gamepads plugged or removed (pad.rs), then NbJoy (a new map has a new VM).
            pads.dpad_buttons = game.vm.local_split();
            #[allow(unused_mut)]
            let mut states = pads.poll();
            // Split screen: each player keeps their gamepad when another is plugged or removed
            // (the test gamepad has an identity of its own).
            let mut ids = pads.ids();
            ids.resize(states.len(), u64::MAX);
            game.vm.set_pad_ids(&ids);
            if game.vm.pads.count != states.len() as i32 {
                println!("gamepads: {}", states.len());
                game.vm.set_pad_count(states.len());
            }
            pad_states = Some(states);
        }
        // PC: the gamepad seen as keyboard and mouse (pad_keys.rs; Android does it with its touch
        // controls), before "Select level" reads it so that a button closing it stays ignored.
        // Improved build: the Controls page's Gamepad entry (the VM's controls_entry.rs).
        self.open_requested_gamepad_page();
        // --gamepad-settings (testing): the page opens over the first menu page.
        if self.options.gamepad_settings && self.game.as_ref().is_some_and(|g| g.vm.gui_page_active()) {
            self.options.gamepad_settings = false;
            self.open_gamepad_settings(0);
        }
        // Player 1's gamepad settings for the look stick (gamepad_config.rs).
        self.gamepad_look_tune();
        let mut page_has_pad = false;
        // The gamepads the first player's page reads (None: all of them, one player here).
        let mut page_pads: Option<Vec<usize>> = None;
        if pad_states.is_some() {
            // Local split screen: only the first player's gamepad (the VM's split_versus.rs); with one
            // player on this machine (solo, network co-op), every gamepad.
            let mine = self.game.as_ref().and_then(|g| g.vm.first_player_pads());
            let all = self.pads.as_ref().and_then(|p| p.merged());
            // In split screen the first player's page reads the first player's gamepads only.
            let split = self.game.as_ref().is_some_and(|g| g.vm.split_menus());
            let all = if split { self.first_player_pads() } else { all };
            if split {
                page_pads = Some(mine.clone().unwrap_or_default());
            }
            // The gamepad settings page reads every gamepad itself: the game sees them at rest.
            page_has_pad = self.gamepad_page_pad(all, dt);
            let merged = self.pads.as_ref().and_then(|p| p.merged_where(|i| mine.as_ref().is_none_or(|m| m.contains(&i))));
            let merged = if page_has_pad { Some(([0.0; 6], 0)) } else { merged };
            // Aim assist of the device in use (the VM's aim_device.rs), once a gamepad has been
            // used; keys and mouse clear aim_on_pad in their events.
            // Lockstep co-op: the gamepad reaches the game in this machine's player's commands
            // (the VM's lockstep.rs, read by coop_pad_input on every machine), never also as keys
            // here: only its device switches are followed.
            #[cfg(not(target_os = "android"))]
            {
                self.aim_device_frame(false, merged);
                let lockstep = self.game.as_ref().is_some_and(|g| g.vm.lockstep_on());
                self.pad_keys_frame(if lockstep { None } else { merged });
            }
            // Android: touch_controls.rs gives the first player's gamepad to the game with the
            // touch controls, at the start of the next frame.
            #[cfg(target_os = "android")]
            {
                let _ = merged;
                self.first_page_has_pad = page_has_pad;
            }
        }
        // Split screen: the pages of the other players, each with its own player's gamepad.
        // No gamepad support at all: the keyboard and mouse's aim assist is still followed.
        #[cfg(not(target_os = "android"))]
        if pad_states.is_none() {
            self.aim_device_frame(false, None);
        }
        // (Lockstep co-op: no other player on this machine.)
        let lockstep = self.game.as_ref().is_some_and(|g| g.vm.lockstep_on());
        let pads_of_pages = match &pad_states {
            Some(states) if !lockstep => self.other_gamepad_pages_pad(states, dt),
            _ => Vec::new(),
        };
        // Improved build: the Mods page's focus moves (keys, gamepad, mouse) play the game's
        // focus tick, as the game's own buttons do.
        play_mods_menu_ticks(&mut self.viewer, self.game.as_mut());
        if let (Some(pads), Some(game), Some(states)) = (&mut self.pads, &mut self.game, pad_states) {
            // "Select level" open: the gamepads belong to it, the game sees them at rest.
            // Improved build: the Mods page open has the gamepads (Select level then sees none).
            let (url, mask_value) = self.viewer.route_menu_gamepad(&mut self.choice, pads, dt);
            // The first player's gamepad settings page has its gamepads: the game sees them at
            // rest; in split screen only the first player's, the other players keep theirs.
            let page_masks = |i: usize| page_has_pad && page_pads.as_ref().is_none_or(|m| m.contains(&i));
            for (i, state) in states.into_iter().enumerate() {
                game.vm.set_pad_state(i, Some(if mask_value || page_masks(i) || pads_of_pages.contains(&i) { Default::default() } else { state }));
            }
            pad_url = url;
            let test_pads = false;
            enable_gamepad_vibration(&mut game.vm, pads.count() > 0 || test_pads, pad::vibrations_wanted());
            // Vibration just switched on in the gamepad settings: a short rumble of that player's
            // gamepad, also before the game's vibrations start; one frame of rest after it.
            let test = self.rumble_test;
            if let Some((_, left)) = &mut self.rumble_test {
                *left -= dt;
            }
            let test_ended = test.is_some_and(|t| t.1 <= 0.0);
            if test_ended {
                self.rumble_test = None;
            }
            let ticked = game.vm.vibrations_tick(dt).map(|f| game.vm.coop_vibrations_by_pad(f));
            let ticked = if test.is_some() { Some(ticked.unwrap_or([[0.0; 2]; 4])) } else { ticked };
            if let Some(forces) = ticked {
                // Vibration turned off in a player's gamepad settings: that player's gamepad
                // stays still.
                let players = self.gamepad_config.players;
                let forces: [[f32; 2]; 4] = std::array::from_fn(|h| {
                    let p = game.vm.gamepad_player(h);
                    gamepad_page::rumble_force(forces[h], players[p].vibration, p, test)
                });
                pads.vibrate(&forces);
            }
        }
        if let Some(url) = pad_url {
            self.leave_to(&url);
        }
        // The menus' choices, kept beyond this map's VM.
        if let Some(p) = self.game.as_mut().and_then(|g| g.vm.take_local_play_change()) {
            self.local_play = p;
        }
        // Improved build: Start level on the game's Select level page.
        self.start_requested_level();
        self.give_start_equipment();
        if let Some(game) = &mut self.game {
            // Network co-op in lockstep: fixed steps of the session instead of the frame's time.
            if self.lockstep.is_some() {
                self.lockstep_steps(dt);
                return;
            }
            game.update(dt);
            let travel = game.vm.engine_travel_tick_start(dt);
            game_log::write_value(&mut game.vm.log);
            if let Some(req) = travel {
                self.pending_travel = Some(req);
            }
            return;
        }
        let dt = dt.min(0.1);
        let speed = if self.keys.contains(&KeyCode::ShiftLeft) { 2400.0 } else { 600.0 } * dt;
        let (sy, cy) = self.camera.yaw.to_radians().sin_cos();
        let (sp, cp) = self.camera.pitch.to_radians().sin_cos();
        let forward = [cp * cy, cp * sy, sp];
        // Unreal is left-handed: the camera's right is (-sin yaw, cos yaw).
        let right = [-sy, cy, 0.0];
        let mut move_by = |dir: [f32; 3], amount: f32| {
            for k in 0..3 {
                self.camera.position[k] += dir[k] * amount;
            }
        };
        // Physical keys: W/Z forward, S back, A/Q left, D right, E/Space up, C down.
        if self.keys.contains(&KeyCode::KeyW) {
            move_by(forward, speed);
        }
        if self.keys.contains(&KeyCode::KeyS) {
            move_by(forward, -speed);
        }
        if self.keys.contains(&KeyCode::KeyA) {
            move_by(right, -speed);
        }
        if self.keys.contains(&KeyCode::KeyD) {
            move_by(right, speed);
        }
        if self.keys.contains(&KeyCode::Space) {
            move_by([0.0, 0.0, 1.0], speed);
        }
        if self.keys.contains(&KeyCode::KeyC) {
            move_by([0.0, 0.0, 1.0], -speed);
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn AttachConsole(process: u32) -> i32;
}

#[allow(dead_code)]
/// OXIII_LOG_WGPU=1: warnings and errors from wgpu (and its backends, Direct3D 11
/// included) in the diagnostics; =2 adds the info messages.
fn log_wgpu() {
    struct RecordLog(log::LevelFilter);
    impl log::Log for RecordLog {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                renderer::diag(&format!("wgpu {} {}: {}", r.level(), r.target(), r.args()));
            }
        }
        fn flush(&self) {}
    }
    let level_value = match openxiii_package::env::var("OXIII_LOG_WGPU").as_deref() {
        Ok("1") => log::LevelFilter::Warn,
        Ok("2") => log::LevelFilter::Info,
        _ => return,
    };
    if log::set_logger(Box::leak(Box::new(RecordLog(level_value)))).is_ok() {
        log::set_max_level(level_value);
    }
}

/// Mods build on a computer: whether the game writes the names catalog by itself at startup:
/// only the game a player starts (not a screenshot run or a test run), with mods on.
#[cfg(all(feature = "mods", not(target_os = "android")))]
fn auto_catalog_wanted(standalone: bool, capture: bool, mods_off: bool) -> bool {
    standalone && !capture && !mods_off
}

#[cfg(all(test, feature = "mods", not(target_os = "android")))]
mod auto_catalog_tests {
    /// Only the game a player starts writes the catalog by itself: never a screenshot run or a
    /// test run (not standalone), nor a game started with --no-mods.
    #[test]
    fn only_the_game_a_player_starts() {
        assert!(super::auto_catalog_wanted(true, false, false));
        assert!(!super::auto_catalog_wanted(false, false, false));
        assert!(!super::auto_catalog_wanted(true, true, false));
        assert!(!super::auto_catalog_wanted(true, false, true));
    }
}

/// Mods build: `--mods-catalog [folder]` (start_app): writes "Modding catalog.html" of `game`
/// into `folder`, or next to the Mods folder. Returns the exit code.
#[cfg(all(feature = "mods", not(target_os = "android")))]
fn write_mods_catalog(game: &Path, folder: Option<PathBuf>, into_repository: bool) -> i32 {
    // The lists made from the game's files never go into a repository's docs by mistake.
    if let Some(f) = folder.as_ref().filter(|f| openxiii_vm::mod_catalog::inside_repository_docs(f)) {
        if !into_repository {
            eprintln!("mods catalog: {} is inside a repository's docs; the lists made from the game's files are not written there (add --into-repository to do it anyway)", f.display());
            return 2;
        }
    }
    let file = match folder {
        Some(f) => f.join(openxiii_vm::mod_catalog::FILE_NAME),
        None => match openxiii_vm::mod_catalog::default_file() {
            Some(f) => f,
            None => {
                eprintln!("mods catalog: no Mods folder");
                return 1;
            }
        },
    };
    println!("writing the modding catalog of {} to {}", game.display(), file.display());
    match openxiii_vm::Vm::write_catalog(game, &file, &|step| println!("  {step}")) {
        Ok(w) => {
            println!("written: {} ({} KB in {:.1} s)", w.file.display(), w.bytes / 1024, w.seconds);
            0
        }
        Err(e) => {
            eprintln!("mods catalog: {e}");
            1
        }
    }
}

/// Mods build: discovers the mods (unless --no-mods or OXIII_NO_MODS=1), logs them and hands
/// their class default values to the VM (before any game script is loaded).
#[cfg(feature = "mods")]
fn start_mods(args: &[String], game: &std::path::Path) {
    openxiii_vm::mod_setup::start_mods(args, Some(game), &mut |line| println!("{line}"));
}

/// Mods build: name of a key a mod action can use ("F8", "K", "7", "NumPad3", "PageUp"…).
#[cfg(feature = "mods")]
fn mod_key_name(code: KeyCode) -> Option<String> {
    let s = format!("{code:?}");
    let name = if let Some(l) = s.strip_prefix("Key") {
        l.to_string()
    } else if let Some(d) = s.strip_prefix("Digit") {
        d.to_string()
    } else if let Some(d) = s.strip_prefix("Numpad").filter(|d| d.len() == 1) {
        format!("NumPad{d}")
    } else if (s.starts_with('F') && s[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n))) || ["Insert", "Delete", "Home", "End", "PageUp", "PageDown"].contains(&s.as_str()) {
        s
    } else {
        return None;
    };
    Some(name)
}

/// Mods build: true if no one else uses key `name`: not a key the game window keeps for itself
/// (cheats::tool_keys of this build) nor a key the player's bindings (User.ini) give to the game. Otherwise the
/// mod action is not run, and a warning says so once.
#[cfg(feature = "mods")]
fn mod_key_free(name: &str, g: &mut game::Game) -> bool {
    static WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let viewer = cheats::this_build_tool_keys().iter().any(|(k, _)| k.eq_ignore_ascii_case(name));
    let user = g.vm.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
    let bound = user.lines().filter_map(|l| l.split_once('=')).find(|(k, v)| k.trim().eq_ignore_ascii_case(name) && !v.trim().is_empty()).map(|(_, v)| v.trim().to_string());
    let why = match (viewer, bound) {
        (true, _) => format!("key {name} is used by the game window"),
        (false, Some(b)) => format!("key {name} is bound by the game to \"{b}\" in the player's settings"),
        (false, None) => return true,
    };
    let mut warned = WARNED.lock().unwrap_or_else(|e| e.into_inner());
    if !warned.iter().any(|w| w == name) {
        warned.push(name.to_string());
        let line = format!("mods: warning: {why}: the mod action on {name} is not run (choose another key)");
        openxiii_mods::log::note(&line);
        println!("{line}");
    }
    false
}

fn main() {
    start_app(|| EventLoop::new().expect("event loop"));
}

/// The game, with the platform's event loop: winit's on PC, that of
/// the Android activity for android_main (crate openxiii-android).
pub(crate) fn start_app(run_loop: impl FnOnce() -> EventLoop<()>) {
    log_wgpu();
    // ATTACH_PARENT_PROCESS: the log goes to the console that launched the program, if there is
    // one; redirected outputs (pipes, files) stay as they are.
    #[cfg(windows)]
    unsafe {
        AttachConsole(u32::MAX);
    }
    // The window keeps the focus through long frames and loads (foreground.rs).
    foreground::keep_focus_through_stalls();
    // Android: no command line (the Java launcher keeps the game folder in OpenXIII.ini).
    let args: Vec<String> = if cfg!(target_os = "android") { Vec::new() } else { std::env::args().skip(1).collect() };
    let _ = &args;
    if openxiii_package::events::init() {
    }
    // Mods build: `XIII.exe --mods-catalog [folder]` (the game the launcher remembers) or
    // `oxiii-view <game> --mods-catalog [folder]` writes the names catalog and stops.
    #[cfg(all(feature = "mods", not(target_os = "android")))]
    {
        let at = args.iter().position(|a| a == "--mods-catalog").filter(|&i| i <= 1);
        if let Some(i) = at {
            let game = if i == 1 { Some(PathBuf::from(&args[0])) } else { launcher::folder_game() };
            let Some(game) = game else { std::process::exit(1) };
            let folder = args.get(i + 1).filter(|a| !a.starts_with("--")).map(PathBuf::from);
            std::process::exit(write_mods_catalog(&game, folder, args.iter().any(|a| a == "--into-repository")));
        }
    }
    let mut options = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // Improved build: the Co-op page's network co-op is played in lockstep (lockstep_play.rs).
    configure_lockstep_play();
    let startup = game::StartupOptions {
        local_players: options.local_players,
        ..Default::default()
    };
    let _math = startup.math.enter();
    // Played game (not a capture): the ini files the game writes (video options, keys…) go
    // into "save\System" next to the game folder, like the profiles, and are read from there at startup.
    if options.play && options.screenshot.is_none() {
        let root = options.save_root.clone().unwrap_or_else(|| options.game.parent().unwrap_or(&options.game).to_path_buf());
        openxiii_vm::config::set_folder_player(root.join("save").join("System"));
        // Improved build, the game a player starts for the first time (no XIII.ini of their
        // own yet): the original's display defaults, fullscreen at the screen's native size,
        // Graphics: Enhanced. Windows knows the native size now; Linux and Android once the
        // window is open (render_device).
        configure_first_launch(&options, &root);
        // Aim assist switched on by default on Android (both builds, a setting of the Edit panel).
        #[cfg(target_os = "android")]
        {
            android::enable_aim_assist(&options.game, &root.join("save").join("System"));
            openxiii_vm::save::set_aim_assist_markers(root.join("save").join("System"));
        }
        openxiii_vm::save::set_folder_saves(root.join("save"));
    } else if let Some(d) = openxiii_package::env::var_os("OXIII_PLAYER_FOLDER") {
        // Testing: captures with the ini files of a player folder (read before the game's), like a played game.
        openxiii_vm::config::set_folder_player(std::path::PathBuf::from(d));
    }
    // Standalone game: the graphics driver's compiled pipelines are kept between runs.
    if options.standalone && options.screenshot.is_none() {
        pipeline_cache::set_folder(Some(launcher::folder_data().join("cache")));
    }
    // Standalone game: a panic is displayed (no console).
    if options.standalone {
        std::panic::set_hook(Box::new(|info| {
            // Also in the log (error output), so that a failure on another machine can be read afterwards.
            eprintln!("OpenXIII stopped because of an error: {info}");
            if openxiii_package::env::var_os("RUST_BACKTRACE").is_some() {
                eprintln!("{}", std::backtrace::Backtrace::force_capture());
            }
            launcher::message(launcher::MARK, &format!("OpenXIII stopped because of an error:\n\n{info}"))
        }));
    }
    // Panic: the pending log is written first (it comes before the message, as before).
    let next_one = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        game_log::empty();
        next_one(info)
    }));
    #[cfg(feature = "mods")]
    start_mods(&args, &options.game);
    // Improved build: the Load page gets a Select level entry where the level list exists
    // (cheats build, standalone game or capture).
    configure_level_menu(&options);
    // Mods build on a computer, the game a player starts: the names catalog and the lists of
    // names, written in the background when missing or out of date (mod_catalog.rs).
    #[cfg(all(feature = "mods", not(target_os = "android")))]
    if auto_catalog_wanted(options.standalone, options.screenshot.is_some(), openxiii_mods::disabled(&args)) {
        openxiii_vm::mod_catalog::refresh_in_background(options.game.clone());
    }
    // Co-op in the menus unless unchecked on the Mods page (read at startup, as the mods).
    configure_coop_menu();
    // The Co-op page's last host address, kept between runs of the standalone game.
    configure_join_file(&options);
    // The menus' improved pictures made when the game is installed (and only if one is missing
    // afterwards), with a progress window: the menus then only read them.
    prepare_installed_pictures(&options);
    // The co-op character screen: each player's last choice, and its places (camera and spot of
    // each player), which this text file can change without rebuilding (character_screen.rs).
    configure_character_screen(&options);
    let started = Instant::now();
    // In play mode, Movers (doors…) are animated separately: they are removed from the fixed geometry.
    // Start URL: options from [DefaultPlayer], map and portal ("Hual02#Teleporter2").
    let url = openxiii_vm::travel::start_url(&openxiii_package::system_dir(&options.game), &options.map);
    options.map = map_stem(&url);
    let exclude = if options.play { game::Game::mover_names(&options.game, &options.map).unwrap_or_default() } else { Default::default() };
    let (mut world, mut library) = match load(&options.game, &options.map, &exclude) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "{}: {} vertices, {} triangles, {} materials, loaded in {:.2} s",
        options.map,
        world.vertices.len(),
        world.indices.len() / 3,
        world.batches.len(),
        started.elapsed().as_secs_f32()
    );
    let mut camera = options.camera.unwrap_or_else(|| start_camera(&world));
    println!("camera: {:.0},{:.0},{:.0},{:.0},{:.0}", camera.position[0], camera.position[1], camera.position[2], camera.yaw, camera.pitch);
    let bindings = load_bindings(&options.game);
    let mut game = None;
    if options.play {
        match game::Game::new(
            &options.game,
            &url,
            world.collision.clone(),
            world.collision_tags.clone(),
            world.collision_materials.clone(),
            &mut library,
            &bindings,
            None,
            None,
            None,
            startup,
        ) {
            Ok(g) => {
                println!("play mode: map logic active, XIII's pawn ready");
                // Improved build: the profile's Graphics setting known, the menus'
                // onomatopoeias redrawn on other cores while the first frames are prepared.
        let mut g = g;
        refresh_level_graphics(&mut g, &options.game);
                // Mods build only: classes named by mods that are not loaded.
                #[cfg(feature = "mods")]
                let mut g = g;
                #[cfg(feature = "mods")]
                g.vm.report_mod_classes();
                game = Some(g);
            }
            Err(e) => eprintln!("play mode unavailable: {e}"),
        }
    }
    let mut video_show: Option<openxiii_vm::video::VideoPresent> = None;
    // Apply2DEffect passes of the last simulated frame.
    let mut fx_passes = Vec::new();
    if let (Some(g), Some(_)) = (&mut game, &options.screenshot) {
        // Capture in play mode: simulation at 30 frames per second, keys through the engine's
        // input, then the controller's camera (PlayerCalcView).
        let hz = 30.0f32;
        let steps = (options.seconds * hz) as usize;
        // Actors drawn every frame (LastRenderTime), as in the window.
        let mut sim_chars = g.characters(&mut library);
        let mut intro: Vec<&'static str> = if options.intro == Some(true) { INTRO_VIDEOS.to_vec() } else { Vec::new() };
        let mut intro_playing = false;
        // Real network game (driver with a real socket, OXIII_NETWORK): the capture follows
        // real time like oxiii simulate, otherwise the client would finish its seconds long before
        // the host, which would see its port closed (ICMP); without network, it goes as fast as it can.
        let wall = std::time::Instant::now();
        let mut sim_clock = 0.0f64;
        let paced = false;
        for step in 0..steps {
            openxiii_package::events::set_frame(step as u32);
            if step == 1 {
                openxiii_package::events::first_normal_frame();
            }
            if paced || g.vm.ipdrv.net_driver.as_ref().is_some_and(|d| d.socket.0.is_some()) {
                sim_clock += 1.0 / hz as f64;
                let ahead = sim_clock - wall.elapsed().as_secs_f64();
                if ahead > 0.0 {
                    std::thread::sleep(std::time::Duration::from_secs_f64(ahead));
                }
            } else {
                sim_clock = wall.elapsed().as_secs_f64();
            }
            if (intro_playing || !intro.is_empty()) && intro_step(&mut g.vm, &mut intro, &mut intro_playing, 1.0 / hz) {
                if let Some(p) = g.vm.engine.video.presented.take() {
                    video_show = Some(p);
                }
                continue;
            }
            start_capture_level(g);
            {
                g.update(1.0 / hz);
            }
            let travel = g.vm.engine_travel_tick(1.0 / hz);
            // Log of the end of the frame (EndMap, destruction of the viewports' actors on travel).
            // Capture (tool): written before the traces this loop prints itself (camera,
            // SUIVI, damage), in the same order as before the log thread.
            game_log::write_value(&mut g.vm.log);
            game_log::empty();
            if let Some(p) = g.vm.engine.video.presented.take() {
                video_show = Some(p);
            }
            g.vm.engine.video.audio.clear();
            if let Some(req) = travel {
                // Travel during the capture: the next map is loaded as in the window.
                println!("travel to {}", req.url.string());
                match headless_travel(&options.game, req, &bindings, &mut g.vm) {
                    Some((w2, l2, g2)) => {
                        world = w2;
                        library = l2;
                        *g = g2;
                        sim_chars = g.characters(&mut library);
                        continue;
                    }
                    None => break,
                }
            }
            // UGameEngine::Draw skipped during a video.
            if g.vm.engine.video.draw_skipped {
                // No Apply2DEffect for this frame: nothing to redraw in capture.
                fx_passes.clear();
                continue;
            }
            video_show = None;
            // Split screen: the real frame of every viewport (UGameEngine::Draw of each), so that
            // every player's camera and first-person models follow as in the window.
            let split = g.active_viewports().len() > 1 || character_screen_on(g);
            let view = if split {
                lockstep_play::draw_capture_views(
                    g, options.size,
                );
                None
            } else {
                g.calc_view()
            };
            // FCameraSceneNode::Render every frame: PreRender, scene (LastRenderTime of the
            // actors seen), then the HUD (its effects advance with it).
            g.vm.display_window = (options.size.0 as i32, options.size.1 as i32);
            if !split {
            let frame = g.vm.hud_begin(None, options.size.0, options.size.1);
            if sim_chars.signature() != g.characters_signature() {
                sim_chars = g.characters(&mut library);
            }
            if let Some((location, rotation)) = view {
                let fov = g.fov().unwrap_or(85.0);
                g.mark_rendered(&sim_chars, location, degrees(rotation[1]), degrees(rotation[0]), fov);
            }
            g.camera_filter();
            let precaching = frame.is_some_and(|frame| g.vm.hud_end(frame));
            // UGameEngine::Draw: second precache frame (Vm::precache_wanted).
            if g.vm.precache_wanted(precaching) {
                g.vm.precache_render = true;
                let frame = g.vm.hud_begin(None, options.size.0, options.size.1);
                if let Some((location, rotation)) = view {
                    let fov = g.fov().unwrap_or(85.0);
                    g.mark_rendered(&sim_chars, location, degrees(rotation[1]), degrees(rotation[0]), fov);
                }
                g.camera_filter();
                if let Some(frame) = frame {
                    let _ = g.vm.hud_end(frame);
                }
                g.vm.precache_render = false;
            }
            }
            // Apply2DEffect on every drawn frame (the intensities rise with it).
            fx_passes = g.screen_fx();
            let now = (step + 1) as f32 / hz;
            game::TIME_CAPTURE.store(now.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
        openxiii_package::events::finish();
        if let (Some((location, rotation)), None) = (g.calc_view(), options.camera) {
            camera = Camera { position: location, yaw: degrees(rotation[1]), pitch: degrees(rotation[0]), roll: degrees(rotation[2]) };
            println!("player camera after {} s: {:?} (yaw {:.1}°, pitch {:.1}°)", options.seconds, location, camera.yaw, camera.pitch);
        }
    }
    // Mods build only, OXIII_MODS_DUMP=1: objects whose class has mod values, after the simulation.
    #[cfg(feature = "mods")]
    if let (Some(g), Ok("1")) = (&game, openxiii_package::env::var("OXIII_MODS_DUMP").as_deref()) {
        game_log::empty();
        if let Some(w) = g.pawn.and_then(|p| match g.vm.prop(p, "Weapon") {
            openxiii_vm::value::Value::Object(Some(w)) => Some(w),
            _ => None,
        }) {
            let name = g.vm.objects[w].as_ref().map_or("?".to_string(), |o| o.name.clone());
            let show = |o: usize, props: &[&str]| props.iter().map(|p| format!("{p}={}", g.vm.prop(o, p))).collect::<Vec<_>>().join(", ");
            println!("mods dump: weapon in the player's hands: {name}: {}", show(w, &["ReloadCount", "PickupAmmoCount", "ShotTime", "TraceAccuracy"]));
            if let openxiii_vm::value::Value::Object(Some(a)) = g.vm.prop(w, "AmmoType") {
                println!("mods dump: its ammo: {}", show(a, &["AmmoAmount", "MaxAmmo", "SRDamages", "LRDamages"]));
            }
        }
        for line in g.vm.mod_dump() {
            println!("mods dump: {line}");
        }
        let live = g.vm.actors.iter().filter(|&&a| g.vm.objects[a].as_ref().is_some_and(|o| !o.name.starts_with("Default__"))).count();
        println!("mods dump: {live} actors in the level");
        // Every character (Pawn) alive, with its health.
        for (id, o) in g.vm.objects.iter().enumerate() {
            let Some(o) = o else { continue };
            if !o.name.starts_with("Default__") && g.vm.is_a(id, "Pawn") {
                let ai = g.vm.native_prop(id, openxiii_vm::NativeField::Controller).as_object();
                let ai_state = ai.and_then(|c| g.vm.objects[c].as_ref().and_then(|x| x.state.clone())).unwrap_or_default();
                println!("mods dump: character {} ({}): Health={}, AI state {ai_state}", o.name, g.vm.classes[o.class].name, g.vm.prop(id, "Health"));
            }
        }
    }
    if let Some(path) = &options.screenshot {
        let result = pollster::block_on(async {
            let (w, h) = options.size;
            let mut r = Renderer::new_headless(w, h, &world, &mut library).await?;
            capture_shot(&mut r, game.as_mut(), &mut library, &world, camera, &options, &mut fx_passes, &video_show, Some(path)).await
        });
        game_log::empty();
        match result {
            Ok(()) => println!("capture written: {}", path.display()),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let event_loop = run_loop();
    // The checked test weapons start at Beretta and M16 at each launch (kept while the game runs).
    let choice = choice_level::ChoiceLevel::default();
    let weapons_for = None;
    let viewer = ViewerState::new(&options);
    let mut app = App {
        viewer,
        options,
        world: Some(world),
        library: Some(library),
        window: None,
        renderer: None,
        camera,
        keys: HashSet::new(),
        looking: false,
        last: Instant::now(),
        run_loop: openxiii_vm::main_loop::ClientLoop::new(counter(), true),
        focused: true,
        occluded: false,
        background_frame: false,
        background_last: None,
        game,
        pads: pad::Pads::new(),
        choice,
        gamepad_config: openxiii_vm::config::folder_player().map(|d| openxiii_vm::gamepad_config::GamepadConfig::load(&d)).unwrap_or_default(),
        rumble_test: None,
        gamepad_page: Default::default(),
        other_gamepad_pages: Default::default(),
        #[cfg(target_os = "android")]
        first_page_has_pad: false,
        weapons_for,
        loadout: None,
        slot_reads_seen: 0,
        local_play: Default::default(),
        characters: None,
        shadow_frame: 0,
        audio: None,
        pending_travel: None,
        lockstep: None,
        lockstep_time: 0.0,
        lockstep_skip_draw: false,
        lockstep_undrawn: 0,
        lockstep_view: None,
        lockstep_output: None,
        lockstep_others: Vec::new(),
        lockstep_shadow_after: Vec::new(),
        lockstep_note: String::new(),
        lockstep_fresh: false,
        lockstep_view_step: u64::MAX,
        lockstep_output_step: u64::MAX,
        lockstep_view_prev: None,
        lockstep_meeting: None,
        lockstep_intro: None,
        lockstep_intro_pad: 0,
        lockstep_hold: None,
        lockstep_hold_warm: 0,
        lockstep_sign_now: Vec::new(),
        lockstep_start_state: None,
        lockstep_equipment_now: false,
        lockstep_host_started: false,
        lockstep_restore: false,
        lockstep_menus: None,
        loading_anim: None,
        travel_hold: None,
        travel_started: Instant::now(),
        video_sound: None,
        video_show: None,
        intro: Vec::new(),
        intro_playing: false,
        display_modes: Vec::new(),
        screen_native: None,
        started: { let t = Instant::now(); let _ = game::START.set(t); t },
        alt_down: false,
        capture_view: None,
        cursor_value: (0, 0),
        full_screen_suspended: None,
        perf: (0, 0.0, 0.0, 0.0, None),
        #[cfg(target_os = "android")]
        touch_controls: Default::default(),
        pad_keys: Default::default(),
        aim_on_pad: false,
        aim_pad_used: false,
        cheats: Default::default(),
        perf_steps: [0.0; 9],
    };
    if app.game.is_some() && app.options.intro == Some(true) {
        app.intro = INTRO_VIDEOS.to_vec();
    }
    if app.game.is_some() && openxiii_package::env::var("OXIII_MUTE").is_err() {
        app.audio = audio::Audio::new(&app.options.game, &app.options.map);
    }
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("error: {e}");
    }
    // The game ends normally (window closed, Quit): an unfinished Direct3D 11 start of this
    // process is not a failure (d3d11_marker.rs; a crash or a kill never gets here).
    #[cfg(all(windows, feature = "dx11"))]
    d3d11_marker::clean_exit();
    game_log::empty();
    // The ini files are written (exiting), the log emptied: the process ends here. Freeing the
    // loaded game piece by piece took more than a minute after Alt+F4 or "Quit", with nothing
    // left to save.
    end_process(app);
}

/// Output flushed, then the process ends without dropping `app` (nothing in it writes a file
/// when dropped; the system frees the memory, threads and sockets at once).
fn end_process(app: App) -> ! {
    use std::io::Write;
    std::mem::forget(app);
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(0)
}

/// [OpenXIII.Video] settings of the ini (render scale, MSAA, anisotropy), applied when they change;
/// what the device accepts is handed to the VM for the Video menu's lists. Graphics: Original
/// (video_settings::graphics_enhanced) draws as the original: 100 %, no MSAA, no anisotropy.
fn apply_render_quality(r: &mut Renderer, vm: &mut openxiii_vm::Vm) {
    r.session_graphics = vm.engine.client.session_graphics;
    #[cfg(feature = "mods")]
    r.use_mod_surface_effects(vm.mod_surface_effects());
    vm.refresh_graphics();
    // V-sync: the Video menu's mode (ini), or OXIII_VSYNC when set.
    vm.refresh_vsync();
    r.set_vsync(openxiii_vm::video_settings::vsync_mode());
    // Improved build: the menu's scale, the menus' onomatopoeias are prepared at.
    scale_menu_art(r, vm);
    if let Some(c) = vm.config.as_mut() {
        apply_configured_render_quality(r, c);
    }
    r.set_shadow_quality(vm.shadow_quality());
    if r.fit_targets() {
        vm.invalidate_shadow_targets();
    }
    let caps = openxiii_vm::video_settings::VideoCaps {
        msaa: r.supported_msaa_counts().to_vec(),
        max_render_scale: r.max_render_scale_percent(),
        max_anisotropy: if r.anisotropy_supported() { 16 } else { 1 },
    };
    if vm.video_caps != caps {
        vm.video_caps = caps;
    }
}

async fn capture_shot(
    r: &mut Renderer,
    mut game: Option<&mut game::Game>,
    library: &mut Library,
    world: &openxiii_assets::world::World,
    camera: Camera,
    options: &Options,
    fx_passes: &mut Vec<openxiii_vm::screen_fx::FxPass>,
    video_show: &Option<openxiii_vm::video::VideoPresent>,
    path: Option<&Path>,
) -> Result<(), String> {
    let _ = world;
    match game.as_deref_mut() {
        Some(g) => apply_render_quality(r, &mut g.vm),
        // Map viewer: the same settings, read from the ini files.
        None => r.set_render_quality(render_quality::RenderQuality::from_config(&mut openxiii_vm::config::ConfigCache::new(options.game.join("System")))),
    }
    if game.is_none() {
        r.fit_targets();
    }
    let (w, h) = options.size;
    // Split screen: each viewport in its rectangle (like draw_frame_views).
    if let Some(g) = game.as_deref_mut().filter(|g| g.active_viewports().len() > 1 || character_screen_on(g)) {
        let chars = g.characters(library);
        let (indices, batches, owners) = g.character_layout(&chars);
        r.set_dynamic(&g.character_frame(&chars), &indices, &batches, &owners, library);
        let target = r.offscreen_target();
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        r.clear_target(&view);
        for slot in g.active_viewports() {
            #[allow(unused_mut)]
            let mut extra = Vec::new();
            draw_viewport_view(r, g, library, Some(&chars), slot, (w, h), 1, &view, &extra);
        }
        return match path {
            Some(path) => r.save_png(&target, path).await,
            None => Ok(()),
        };
    }
    if let Some(g) = game.as_deref_mut() {
        let chars = g.characters(library);
        let (indices, batches, owners) = g.character_layout(&chars);
        r.set_dynamic(&g.character_frame(&chars), &indices, &batches, &owners, library);
        let verts = g.character_frame(&chars);
        g.character_cartoon_in(&chars, &verts, r.cartoon_mut());
        let light_views = g.scene_light_leaves(&chars);
        r.set_scene_light_leaves(light_views);
        r.set_dyn_lights(&g.dyn_light_records());
        r.set_prop_lights(g.prop_lights(&chars, &verts));
        r.set_hidden_owners(g.hidden_baked());
        r.set_texture_replacements(&g.vm.texture_replacements);
        if let Some(w) = g.replaced_world(library, true) {
            r.set_replaced(w.as_ref());
        }
        // --camera: the forced view is not that of the game's scene, all visible
        // sections ripple.
        let drawn = options.camera.is_none().then(|| g.sine_drawn());
        r.update_sine(library, drawn.as_deref());
        // Scenery cut by the player's view (not with --camera, forced view).
        r.set_bsp_visible(options.camera.is_none().then(|| g.bsp_visible()).flatten());
        // Shadows: the cartoon direction comes from a first computation of the passes (previous
        // frame in the window).
        r.prepare_cartoon_dirs(library, &camera);
        let jobs = g.update_shadows(&chars, camera.position, &|k| r.cartoon_dir(k), 1, None);
        if !jobs.is_empty() {
            g.for_each_shadow_mesh(&chars, &jobs, |first, mesh, id| r.set_shadow_vertices_id(first, mesh, id));
        }
        r.set_shadow_jobs(jobs);
        r.update_dynamic(&verts, &g.first_person(&chars));
        r.set_ignore_vignette(&g.ignore_vignette(&chars));
        r.set_vignette_owners(g.vignette_baked());
        r.set_translucent_keys(g.translucent_keys(&chars));
        let (sprites, meshes) = g.particle_sprites(&camera);
        r.set_trails(g.trail_batches(&camera));
        r.set_particles(&sprites, &meshes, library);
        let model = r.model();
        let projectors = g.projector_draws(&camera, model.as_deref(), g.fov().unwrap_or(85.0));
        r.set_bsp_projectors(&projectors, library);
        r.fov = g.fov();
        r.only_post_render = g.vm.only_post_render;
        r.gamma = crate::gamma::applies(g.vm.fullscreen, g.vm.engine.client.session_graphics).then_some(g.vm.client_color);
        let (mut quads, targets) = g.hud(w, h);
        // Improved build: a click on the main menu's Mods box or the Load page's Select level
        // entry during the capture opens its page, as in the window.
        draw_capture_mods_menu(g, w, h, &mut quads);
        // Each render texture with the characters of its own list (target_rendered),
        // as in the window: otherwise a comic panel could be empty in capture.
        render_targets(r, g, Some(&chars), &targets, library);
        {
            r.set_hud(&quads);
        }
        r.fx_passes = fx_passes.clone();
        r.cartoon_flash = g.vm.screen_fx.cartoon_flash;
        r.vignette = g.vignette();
        r.flash = g.flash_tile(g.controller);
        r.clear_volume = g.vm.view_volume_clear(g.controller);
        r.volume_fog = g.vm.view_volume_fog(g.controller);
        r.color_filter = g.color_filter;
        let (weather, c2w) = g.weather(camera.position, camera.yaw, camera.pitch);
        r.set_weather(weather.as_ref(), &c2w);
    }
    if options.fov.is_some() {
        r.fov = options.fov;
    }
    // Capture during a video: the player's screen.
    if video_show.is_some() || game.as_deref().is_some_and(|g| g.vm.engine.video.draw_skipped) {
        r.set_video(Some(video_show.as_ref().unwrap_or(&openxiii_vm::video::VideoPresent::Black)));
    }
    match path {
        Some(path) => r.screenshot(&camera, path, library).await,
        None => {
            r.render_offscreen(&camera, library);
            Ok(())
        }
    }
}

impl App {
    /// The video's sound commands of the frame (UPCVideoPlayerDevice's Bink sound) applied to
    /// the video sound output. With no video open in the VM any more, a sound still playing
    /// is stopped: its Stop went with a VM replaced by a travel (a network session's start or
    /// end), and the video's sound went on into the level and the menus.
    pub(crate) fn apply_video_audio(&mut self) {
        let Some(g) = &mut self.game else { return };
        let calls = std::mem::take(&mut g.vm.engine.video.audio);
        let open = g.vm.engine.video.is_open();
        if !calls.is_empty() {
            if self.video_sound.is_none() && openxiii_package::env::var("OXIII_MUTE").is_err() {
                self.video_sound = video_audio::VideoSound::new();
            }
            if let Some(v) = &mut self.video_sound {
                v.apply(calls);
            }
        }
        if !open {
            if let Some(v) = &mut self.video_sound {
                v.stop_if_playing();
            }
        }
    }
}
