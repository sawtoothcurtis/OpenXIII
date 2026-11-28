//! oxiii server <game> <URL> [seconds] [-port=N]: dedicated server like "UCC server URL"
//! (UServerCommandlet::Main), with no window and no local player.
//!
//! The engine has no UClient (GIsClient = 0): UGameEngine::Init loads the command-line URL
//! without going through Entry, LoadMap always listens (NM_DedicatedServer), UGameEngine::Tick
//! skips everything related to viewports. The FPU stays in 53-bit mode (no Direct3D device).
//! The loop follows real time: Tick(real frame duration), then wait until
//! 1 / GetMaxTickRate since the start of the frame.

use openxiii_vm::server::{console_key, quit_key, ucc_log_path, ServerLoop, UccLog};
use std::path::Path;

/// Arguments after the game folder: the URL, an optional duration (OpenXIII-specific, for
/// testing; without it the server runs until the process is stopped) and the UCC
/// options (-port=N…), which make up appCmdLine.
pub fn server(game: &Path, args: &[String]) -> Result<(), String> {
    // The VM recurses deeply (travel scripts with connected players): 256 MiB stack, as in oxiii loop.
    let (game, args) = (game.to_path_buf(), args.to_vec());
    std::thread::Builder::new().stack_size(256 << 20).spawn(move || server_thread(&game, &args)).map_err(|e| e.to_string())?.join().unwrap_or_else(|e| std::panic::resume_unwind(e))
}

fn server_thread(game: &Path, args: &[String]) -> Result<(), String> {
    let seconds = args.get(1).and_then(|s| s.parse::<f64>().ok());
    let line: Vec<&str> = args.iter().enumerate().filter(|&(i, _)| !(i == 1 && seconds.is_some())).map(|(_, a)| a.as_str()).collect();
    let line = line.join(" ");
    // SAFETY: before any threads (InitBase reads -PORT= from appCmdLine).
    unsafe { std::env::set_var("OXIII_COMMAND_LINE", &line) };
    let mut vm = crate::load_vm(game)?;
    vm.ipdrv.dedicated = true;
    // A dedicated server really listens (load_vm puts sockets offline unless OXIII_NETWORK=1).
    vm.ipdrv.offline = false;
    // UCC.exe log: System\UCC.log (LOG=, ABSLOG=), without the events of [Core.System] Suppress=.
    let base = format!("{}\\", openxiii_package::system_dir(&game).display());
    let suppress = vm.config.as_mut().and_then(|c| c.multi_find("Core.System", "Suppress", "System")).unwrap_or_default();
    let mut record_log = UccLog::new(ucc_log_path(&line, &base), suppress);
    openxiii_vm::net::set_fpu53(true);
    // UGameEngine::Init: first word of appCmdLine (after SERVER), otherwise
    // DefaultLocalMap; a word starting with "-" also gives DefaultLocalMap. FURL(Base of the
    // [DefaultPlayer] options, text, TRAVEL_Partial).
    let mut tokens = line.split_whitespace();
    let mut first = tokens.next();
    if first.is_some_and(|t| t.eq_ignore_ascii_case("SERVER")) {
        first = tokens.next();
    }
    let text = match first.filter(|t| !t.starts_with('-')) {
        Some(t) => t.to_string(),
        None => vm.config.as_mut().and_then(|c| c.get_string("URL", "LocalMap", "System")).unwrap_or_else(|| "Index.unr".into()),
    };
    let url = openxiii_vm::travel::start_url(&openxiii_package::system_dir(&game), &text);
    if url.map.is_empty() {
        return Err(format!("invalid URL: {text}"));
    }
    load_map_url(&mut vm, game, &url, &mut record_log)?;
    // UServerCommandlet::Main (openxiii_vm::server): GIsRunning, OldTime = SecondStartTime = appSeconds.
    let start = std::time::Instant::now();
    let app_seconds = || start.elapsed().as_secs_f64();
    let mut run_loop = ServerLoop::new(app_seconds());
    loop {
        let new_time = app_seconds();
        let old_time = run_loop.old_time;
        // GEngine->Tick((float)(NewTime − OldTime)).
        let delta = run_loop.delta(new_time);
        vm.tick(delta);
        let travel = vm.engine_travel_tick(delta);
        output(&mut vm, &mut record_log);
        // UGameEngine::Tick → Browse → LoadMap: the level being left is removed (driver destroyed, connections
        // closed by engine_travel_tick), the next one loaded, listened on and started as at startup.
        if let Some(req) = travel {
            out!("server travel to {}", req.url.string());
            vm.unload_level();
            load_map_url(&mut vm, game, &req.url, &mut record_log)?;
        }
        // OldTime, TickCount and CurrentTickRate (UEngine +0xB8, read by GETCURRENTTICKRATE).
        run_loop.after_tick(new_time);
        vm.engine.tick_rate.current = run_loop.current_tick_rate;
        // Wait until 1 / MaxTickRate since the start of the frame, appSleep:
        // Sleep(_ftol(seconds × 1000)).
        if let Some(sleep) = ServerLoop::sleep_seconds(vm.net_max_tick_rate(), app_seconds(), new_time) {
            std::thread::sleep(std::time::Duration::from_millis((sleep as f64 * 1000.0) as u32 as u64));
        }
        // The Q (or q) key in the console stops the server (GIsRequestingExit), like
        // EXIT or QUIT issued by an administrator.
        if quit_key(console_key()) || vm.quit_requested {
            break;
        }
        if seconds.is_some_and(|s| new_time >= s) {
            break;
        }
    }
    let conns = vm.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len());
    out!("server: {:.1} s, {:.1} frames/s, {conns} connection(s), {} actors", app_seconds(), run_loop.current_tick_rate, vm.actors.len());
    record_log.close();
    Ok(())
}

/// UGameEngine::LoadMap for a dedicated server: map, collision, GameInfo (DefaultServerGame), ULevel::Listen
/// (no UClient), InitGame, BeginPlay.
fn load_map_url(vm: &mut openxiii_vm::Vm, game: &Path, url: &openxiii_vm::travel::Url, record_log: &mut UccLog) -> Result<(), String> {
    vm.url = url.clone();
    let map_file = openxiii_vm::travel::map_file(url);
    let map = map_file.rsplit_once('.').map_or(map_file.clone(), |(m, _)| m.to_string());
    let map_path = game.join("Maps").join(&map_file);
    let actors = vm.load_map(&map_path).map_err(|e| e.to_string())?;
    out!("{map}: {} actors loaded in the VM", actors.len());
    crate::prepare_collisions(vm, game, &map, &map_path)?;
    let game_class = vm.game_class_for(url);
    if let Err(e) = vm.start_game(&game_class, &url.options_string()) {
        out!("GameInfo: {e}");
    }
    vm.begin_play();
    output(vm, record_log);
    let Some(port) = vm.ipdrv.net_driver.as_ref().map(|d| d.socket.1) else {
        return Err("the server is not listening (ULevel::Listen failed)".into());
    };
    // OpenXIII-specific: addresses to give to players (one line per connected network adapter).
    out!("server ready on map {map}, port {port} (UDP), LAN beacon on port 8777");
    for (a, _) in openxiii_vm::ipdrv::locals_addresses() {
        out!("  address to type to join: {a}:{port}");
    }
    Ok(())
}

/// VM messages go to the UCC.log file then to the console (the commandlet's LogToStdout).
fn output(vm: &mut openxiii_vm::Vm, record_log: &mut UccLog) {
    for l in &vm.log {
        record_log.serialize_line(l);
    }
    // "Title: …" is UCC's console title (not a log line): written every frame,
    // it flooded the screen; the window title takes it, with no line.
    vm.log.retain(|l| match l.strip_prefix("Title: ") {
        Some(t) => {
            title_console(t);
            false
        }
        None => true,
    });
    crate::print_vm_output(vm);
}

/// SetConsoleTitleW, only when the title changes.
fn title_console(t: &str) {
    static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let Ok(mut d) = LAST.lock() else { return };
    if *d == t {
        return;
    }
    *d = t.to_string();
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn SetConsoleTitleW(title: *const u16) -> i32;
        }
        let w: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: null-terminated string, alive during the call.
        unsafe { SetConsoleTitleW(w.as_ptr()) };
    }
}
