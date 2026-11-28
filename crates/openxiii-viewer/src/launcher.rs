//! Game launched on its own (OpenXIII's XIII.exe opened without arguments, from Explorer): on the first
//! launch, the player points to the original game's XIII.exe; its folder is kept in
//! %APPDATA%\OpenXIII\OpenXIII.ini, next to the player's saves and ini files (the original game
//! folder is only read). Texts in English: this launcher is the one of the released version.

use std::path::{Path, PathBuf};

/// Mark of the playable version (window title, corner of the main menu).
pub const MARK: &str = "OpenXIII 0.1";

/// Folder of OpenXIII's settings and saves.
pub fn folder_data() -> PathBuf {
    // Linux: ~/.local/share/OpenXIII (XDG_DATA_HOME), unless APPDATA is set.
    #[cfg(target_os = "linux")]
    if openxiii_package::env::var_os("APPDATA").is_none() {
        let xdg = openxiii_package::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
        if let Some(d) = xdg.or_else(|| openxiii_package::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))) {
            return d.join("OpenXIII");
        }
    }
    openxiii_package::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir()).join("OpenXIII")
}

/// Linux: original game folder given as the only argument (game folder, its System or its
/// XIII.exe); kept for later launches without an argument.
#[cfg(target_os = "linux")]
pub fn game_gives(path_buf: &Path) -> Option<PathBuf> {
    let dir = game_of_folder(path_buf).or_else(|| game_exe(path_buf))?;
    keep_game(&dir);
    Some(dir)
}

fn file_settings() -> PathBuf {
    folder_data().join("OpenXIII.ini")
}

/// A XIII folder: System\Engine.u, System\XIII.ini and Maps\MapMenu.unr, each name in any case
/// (the GOG release has system\engine.u; Linux and macOS file systems tell the cases apart).
fn valid_game(root: &Path) -> bool {
    use openxiii_package::child_any_case;
    let system = openxiii_package::system_dir(root);
    child_any_case(&system, "Engine.u").is_file() && child_any_case(&system, "XIII.ini").is_file() && child_any_case(&child_any_case(root, "Maps"), "MapMenu.unr").is_file()
}

fn game_kept() -> Option<PathBuf> {
    let text_value = std::fs::read_to_string(file_settings()).ok()?;
    let dir = text_value.lines().find_map(|l| l.trim().strip_prefix("GameDir=")).map(|d| PathBuf::from(d.trim()))?;
    valid_game(&dir).then_some(dir)
}

fn keep_game(dir: &Path) {
    let _ = std::fs::create_dir_all(folder_data());
    let _ = std::fs::write(file_settings(), format!("[OpenXIII]\r\nGameDir={}\r\n", dir.display()));
}

/// The XIII folder containing the chosen exe (System\XIII.exe, or XIII.exe at the root).
#[cfg(target_os = "linux")]
fn game_exe(exe: &Path) -> Option<PathBuf> {
    exe.ancestors().skip(1).take(2).find(|d| valid_game(d)).map(Path::to_path_buf)
}

/// The XIII folder from a chosen folder: the game's own folder, or its System folder (named
/// System, or holding XIII.exe or Engine.u) whose parent is the game. None: not the game.
fn game_of_folder(dir: &Path) -> Option<PathBuf> {
    if valid_game(dir) {
        return Some(dir.to_path_buf());
    }
    let named_system = dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("System"));
    let looks_system = named_system || openxiii_package::child_any_case(dir, "XIII.exe").is_file() || openxiii_package::child_any_case(dir, "Engine.u").is_file();
    dir.parent().filter(|p| looks_system && valid_game(p)).map(Path::to_path_buf)
}

/// Error shown when the chosen folder is not the original game.
#[cfg(windows)]
const NOT_THE_GAME: &str = "This folder does not contain the original XIII game. Choose the XIII folder or its System folder.";

#[cfg(windows)]
pub fn message(title: &str, text_value: &str) {
    let _ = rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title(title).set_description(text_value).set_buttons(rfd::MessageButtons::Ok).show();
}

#[cfg(not(windows))]
pub fn message(title: &str, text_value: &str) {
    eprintln!("{title}: {text_value}");
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
}

/// Shift held at launch: the player picks the original game again.
#[cfg(windows)]
fn shift_held() -> bool {
    const VK_SHIFT: i32 = 0x10;
    unsafe { GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000 != 0 }
}

/// Original game folder: the kept one (unless Shift is held), otherwise asked from the player (None if they
/// give up).
#[cfg(windows)]
pub fn folder_game() -> Option<PathBuf> {
    if !shift_held() {
        if let Some(dir) = game_kept() {
            return Some(dir);
        }
    }
    const HOME_TEXT: &str = "OpenXIII needs the files of the original game XIII (2003), which you must own.\n\n\
             Please select the original game's folder: the XIII folder, or its System folder.\n\n\
             The original game is only read, never modified.";
    // Home screen with the "desktop shortcut" box (unchecked at first); plain dialog without it if
    // the task dialog is not available.
    let shortcut = match crate::desktop_shortcut::ask_with_checkbox(MARK, HOME_TEXT, "Create a desktop shortcut") {
        Some((ok, shortcut)) => {
            if !ok {
                return None;
            }
            shortcut
        }
        None => {
            let home_screen = rfd::MessageDialog::new().set_level(rfd::MessageLevel::Info).set_title(MARK).set_description(HOME_TEXT).set_buttons(rfd::MessageButtons::OkCancel).show();
            if home_screen != rfd::MessageDialogResult::Ok {
                return None;
            }
            false
        }
    };
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    loop {
        // A folder picker (IFileOpenDialog with FOS_PICKFOLDERS); Cancel: None, the game quits.
        let folder = rfd::FileDialog::new().set_title("Select the original game's folder (XIII or XIII\\System)").pick_folder()?;
        if me.as_deref().and_then(Path::parent).is_some_and(|d| folder.canonicalize().ok().as_deref() == Some(d)) {
            message(MARK, "This is OpenXIII's own folder. Choose the XIII folder of the original game, or its System folder.");
            continue;
        }
        match game_of_folder(&folder) {
            Some(dir) => {
                keep_game(&dir);
                if shortcut {
                    crate::desktop_shortcut::create_on_desktop(&dir, &folder_data());
                }
                return Some(dir);
            }
            None => message(MARK, NOT_THE_GAME),

        }
    }
}

#[cfg(not(windows))]
pub fn folder_game() -> Option<PathBuf> {
    let kept = game_kept();
    #[cfg(target_os = "linux")]
    if kept.is_none() {
        message(MARK, "OpenXIII needs the files of the original game XIII (2003), which you must own.\nRun it once with the original game's folder: ./OpenXIII /path/to/XIII");
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake game folder: System\Engine.u, System\XIII.ini, System\XIII.exe and Maps\MapMenu.unr.
    fn fake_game(root: &Path) {
        std::fs::create_dir_all(root.join("System")).unwrap();
        std::fs::create_dir_all(root.join("Maps")).unwrap();
        for f in ["System/Engine.u", "System/XIII.ini", "System/XIII.exe", "Maps/MapMenu.unr"] {
            std::fs::write(root.join(f), b"").unwrap();
        }
    }

    /// The game's folder and its System folder both give the game's folder; another folder, and
    /// a System folder without the game around it, give nothing.
    #[test]
    fn game_folder_from_the_root_or_its_system_folder() {
        let base = std::env::temp_dir().join(format!("oxiii-game-folder-{}", std::process::id()));
        let game = base.join("XIII");
        fake_game(&game);
        assert_eq!(game_of_folder(&game).as_deref(), Some(game.as_path()), "root");
        assert_eq!(game_of_folder(&game.join("System")).as_deref(), Some(game.as_path()), "System");
        assert_eq!(game_of_folder(&game.join("Maps")), None, "another folder of the game");
        let other = base.join("Other");
        std::fs::create_dir_all(other.join("System")).unwrap();
        assert_eq!(game_of_folder(&other), None, "wrong folder");
        assert_eq!(game_of_folder(&other.join("System")), None, "a System folder without the game");
        // Negative control: without Maps\MapMenu.unr the root is not the game.
        std::fs::remove_file(game.join("Maps").join("MapMenu.unr")).unwrap();
        assert_eq!(game_of_folder(&game), None);
        assert_eq!(game_of_folder(&game.join("System")), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The GOG release's names (system\engine.u, lowercase): accepted from the root and from its
    /// system folder, which matters on Linux and macOS where the file system tells cases apart.
    #[test]
    fn game_folder_with_the_gog_release_names() {
        let base = std::env::temp_dir().join(format!("oxiii-game-folder-gog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("XIII");
        std::fs::create_dir_all(game.join("system")).unwrap();
        std::fs::create_dir_all(game.join("Maps")).unwrap();
        for f in ["system/engine.u", "system/XIII.ini", "system/xiii.exe", "Maps/MapMenu.unr"] {
            std::fs::write(game.join(f), b"").unwrap();
        }
        assert_eq!(game_of_folder(&game).as_deref(), Some(game.as_path()), "root");
        assert_eq!(game_of_folder(&game.join("system")).as_deref(), Some(game.as_path()), "system");
        // Negative control: without the engine package the folder is refused.
        std::fs::remove_file(game.join("system").join("engine.u")).unwrap();
        assert_eq!(game_of_folder(&game), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}