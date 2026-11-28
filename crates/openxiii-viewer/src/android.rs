//! Android: android_main of libopenxiii.so (crate openxiii-android, whose root is this
//! main.rs), started by the Java activity org.openxiii.GameActivityXiii (GameActivity).
//!
//! The Java launcher has already chosen the original game folder on the phone and written it to
//! <filesDir>/OpenXIII/OpenXIII.ini (GameDir=…), like XIII.exe's launcher in
//! %APPDATA%\OpenXIII: APPDATA is therefore the app's filesDir, where the player's saves
//! and ini files also go (launcher.rs).

use winit::event_loop::EventLoop;
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::EventLoopBuilderExtAndroid;

#[no_mangle]
fn android_main(app: AndroidApp) {
    let data_value = app.internal_data_path().unwrap_or_else(|| "/data/local/tmp".into());
    // Test builds: the log also goes, with its timings, to a file the tester can send back.
    let timed = cfg!(feature = "test-env-file").then(|| std::path::Path::new("/sdcard/Download/OpenXIII-log.txt"));
    super::android_log::redirect(&data_value.join("OpenXIII").join("OpenXIII.log"), timed);
    std::env::set_var("APPDATA", &data_value);
    ffmpeg_from_apk();
    #[cfg(feature = "mods")]
    mods_folder_from_launcher(&data_value);
    println!("OpenXIII: data in {}", data_value.display());
    super::start_app(move || EventLoop::builder().with_android_app(app).build().expect("event loop"));
    // End of the game (Quit): the activity closes with the process.
    std::process::exit(0);
}

/// Aim assist of the console versions (iAutoAimMode of [Engine.PlayerController], aiming at the
/// best pawn in the cone: aim.rs; bRotationAssist of [XIII.XIIIPlayerController], the view that
/// turns towards the held target when moving forward or strafing): enabled once in the player's User.ini, created from the
/// game's DefUser.ini if it does not exist yet. The marker keeps the player's choice if they turn it off
/// later in the options.
pub fn enable_aim_assist(game: &std::path::Path, player_value: &std::path::Path) {
    // Third marker: the builds before it could leave the touch aim assist at 0 while it seemed on
    // (the view assist was forced on); updating to this build switches it on once more.
    let marker = player_value.join("OpenXIII-aim-assist3");
    if marker.exists() {
        return;
    }
    let user = player_value.join("User.ini");
    let source = if user.is_file() { user.clone() } else { openxiii_package::system_dir(&game).join("DefUser.ini") };
    let Ok(bytes_value) = std::fs::read(&source) else { return };
    // Ini in Latin-1, as ConfigCache::flush_file writes them.
    let text_value: String = bytes_value.iter().map(|&b| b as char).collect();
    let mut ini = openxiii_vm::config::ConfigFile::parse(&text_value);
    ini.set("Engine.PlayerController", "iAutoAimMode", "1");
    ini.set("XIII.XIIIPlayerController", "bRotationAssist", "True");
    let output: Vec<u8> = ini.to_text().chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
    let _ = std::fs::create_dir_all(player_value);
    if std::fs::write(&user, output).is_ok() {
        let _ = std::fs::write(&marker, b"1");
        println!("OpenXIII: aim assist turned on in {}", user.display());
    }
}

/// .bik videos (openxiii_vm::video): no ffmpeg on the phone, the APK ships its own as
/// libffmpeg.so next to libopenxiii.so, in nativeLibraryDir (extractNativeLibs: the only folder
/// of the app from which Android allows running a program). This folder is that of libopenxiii.so,
/// read from /proc/self/maps. Without this file (or with OXIII_FFMPEG already set), nothing changes.
fn ffmpeg_from_apk() {
    if openxiii_package::env::var_os("OXIII_FFMPEG").is_some() {
        return;
    }
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else { return };
    let Some(lib) = maps.lines().filter_map(|l| l.split_whitespace().nth(5)).find(|p| p.ends_with("/libopenxiii.so")) else { return };
    let ffmpeg = std::path::Path::new(lib).with_file_name("libffmpeg.so");
    if ffmpeg.is_file() {
        println!("OpenXIII: ffmpeg {}", ffmpeg.display());
        std::env::set_var("OXIII_FFMPEG", &ffmpeg);
    }
}

/// Mods build: the Mods folder the Java launcher made in shared storage (ModsDir= line of
/// OpenXIII.ini), where mods are copied from a computer over USB, as next to XIII.exe on PC.
#[cfg(feature = "mods")]
fn mods_folder_from_launcher(data_value: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(data_value.join("OpenXIII").join("OpenXIII.ini")) else { return };
    if let Some(dir) = text.lines().find_map(|l| l.trim().strip_prefix("ModsDir=")).map(str::trim).filter(|d| !d.is_empty()) {
        println!("OpenXIII: mods in {dir}");
        openxiii_mods::set_default_folder(std::path::PathBuf::from(dir));
    }
}
