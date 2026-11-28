//! New version notice of the released game: at launch, before the game window opens, OpenXIII asks
//! GitHub which release is the latest and, when it is newer than this build, says so in a small system
//! window: "Open the download page", "Later" or "Don't ask again". Nothing is downloaded or installed:
//! the release page opens in the player's browser. Nothing here runs inside the game, which stays unchanged.
//!
//! One request per launch, to github.com only, with nothing sent about the player or the machine (no
//! telemetry). "Don't ask again" writes `Auto=false` in update.ini, next to OpenXIII.ini. Android has
//! the same notice in its launcher (android/app/src/main/java/org/openxiii/UpdateNotice.java).

use std::path::{Path, PathBuf};
use std::process::Command;

/// The official repository.
pub const REPOSITORY: &str = "https://github.com/sawtoothcurtis/OpenXIII";

/// Version of this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// "0.2.0" -> [0, 2, 0] (one to four numbers).
pub fn parse_version(v: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = v.split('.').map(|p| if p.is_empty() || p.len() > 9 { None } else { p.parse().ok() }).collect();
    parts.filter(|p| (1..=4).contains(&p.len()))
}

/// `candidate` is a later version than `current` (0.2 > 0.1.9, 0.2 == 0.2.0).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_version(candidate), parse_version(current)) else { return false };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Version of the release a ".../releases/latest" redirect points to (".../releases/tag/v0.2.0").
pub fn version_of_redirect(location: &str, repository: &str) -> Option<String> {
    let tag = location.trim().strip_prefix(repository)?.strip_prefix("/releases/tag/")?;
    let v = tag.strip_prefix('v').unwrap_or(tag);
    parse_version(v).map(|_| v.to_string())
}

/// The page of a release (built from the version, never taken from the answer as is).
pub fn release_page(version: &str) -> String {
    format!("{REPOSITORY}/releases/tag/v{version}")
}

// ---------------------------------------------------------------------------------------------
// Settings

fn settings_file(data: &Path) -> PathBuf {
    data.join("update.ini")
}

/// update.ini in the settings folder: "Auto=false" never looks.
pub fn enabled(data: &Path) -> bool {
    let text = std::fs::read_to_string(settings_file(data)).unwrap_or_default();
    !text.lines().any(|l| l.trim().eq_ignore_ascii_case("Auto=false"))
}

pub fn set_enabled(data: &Path, on: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(data)?;
    std::fs::write(settings_file(data), format!("; OpenXIII says at launch when a new version exists (true) or never (false).\r\nAuto={on}\r\n"))
}

// ---------------------------------------------------------------------------------------------
// The question to GitHub (the system's curl: built into Windows 10 and later, present on Linux)

fn command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window.
        c.creation_flags(0x0800_0000);
    }
    c
}

fn curl() -> Command {
    #[cfg(windows)]
    {
        // The system's own curl (Windows 10 1803 and later); plain "curl.exe" from the search path
        // when neither SystemRoot nor windir is set.
        match std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")) {
            Some(root) => command(&PathBuf::from(root).join("System32").join("curl.exe").to_string_lossy()),
            None => command("curl.exe"),
        }
    }
    #[cfg(not(windows))]
    {
        command(if Path::new("/usr/bin/curl").is_file() { "/usr/bin/curl" } else { "curl" })
    }
}

/// Version of the latest release of `repository`: where ".../releases/latest" redirects to (no
/// download, nothing but the address is read).
#[doc(hidden)]
pub fn latest_in(repository: &str) -> Result<String, String> {
    let out = curl()
        .args(["--silent", "--show-error", "--head", "--proto", "=https", "--connect-timeout", "4", "--max-time", "8", "--output"])
        .arg(if cfg!(windows) { "NUL" } else { "/dev/null" })
        .args(["--write-out", "%{redirect_url}"])
        .arg(format!("{repository}/releases/latest"))
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl could not be started: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let location = String::from_utf8_lossy(&out.stdout);
    version_of_redirect(&location, repository).ok_or_else(|| "no release yet".into())
}

/// The latest release's version, if it is newer than this build.
pub fn newer() -> Result<Option<String>, String> {
    let v = latest_in(REPOSITORY)?;
    Ok(is_newer(&v, VERSION).then_some(v))
}

/// Opens `url` in the player's browser.
fn open_page(url: &str) {
    #[cfg(windows)]
    let started = command("explorer.exe").arg(url).spawn();
    #[cfg(not(windows))]
    let started = command("xdg-open").arg(url).spawn();
    if started.is_err() {
        eprintln!("The new version is at {url}");
    }
}

// ---------------------------------------------------------------------------------------------
// The window at launch

/// What the player answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Open,
    Later,
    Never,
}

const OPEN: &str = "Open the download page";
const LATER: &str = "Later";
const NEVER: &str = "Don't ask again";

#[cfg(windows)]
mod windows_dialog;

#[cfg(windows)]
fn ask(title: &str, heading: &str, text: &str) -> Answer {
    windows_dialog::ask(title, heading, text, OPEN, LATER, NEVER)
}

/// Linux: zenity's window when it is installed, otherwise a line in the terminal.
#[cfg(not(windows))]
fn ask(title: &str, heading: &str, text: &str) -> Answer {
    let text = format!("{heading}\n\n{text}");
    let zenity = Command::new("zenity")
        .args([
            "--question",
            "--no-wrap",
            &format!("--title={title}"),
            &format!("--text={text}"),
            &format!("--ok-label={OPEN}"),
            &format!("--cancel-label={LATER}"),
        ])
        .arg(format!("--extra-button={NEVER}"))
        .output();
    match zenity {
        Ok(out) => match out.status.code() {
            Some(0) => Answer::Open,
            _ if String::from_utf8_lossy(&out.stdout).trim() == NEVER => Answer::Never,
            _ => Answer::Later,
        },
        Err(_) => {
            eprintln!("{title}: {text}");
            Answer::Later
        }
    }
}

/// At launch of the released game, before its window: unless the player turned it off, says when a
/// newer version exists. Returns when the game should start; "Open the download page" opens it and
/// ends the program instead (the player is updating).
/// `data`: the folder of OpenXIII's settings; `title`: the window title.
pub fn at_launch(data: &Path, title: &str) {
    if !enabled(data) {
        return;
    }
    let version = match newer() {
        Ok(Some(v)) => v,
        Ok(None) => return eprintln!("update: this is the latest version ({VERSION})"),
        Err(e) => return eprintln!("update: could not look for a new version: {e}"),
    };
    let heading = format!("OpenXIII {version} is available");
    let text = format!("This is version {VERSION}. The new version can be downloaded from the official OpenXIII page on GitHub.");
    match ask(title, &heading, &text) {
        Answer::Open => {
            open_page(&release_page(&version));
            std::process::exit(0)
        }
        Answer::Later => {}
        Answer::Never => {
            let _ = set_enabled(data, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10", "0.9.9"));
        assert!(is_newer("1", "0.99"));
        assert!(!is_newer("0.2", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0-beta", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(parse_version("1.2.3.4.5").is_none());
    }

    #[test]
    fn redirects() {
        let r = REPOSITORY;
        assert_eq!(version_of_redirect(&format!("{r}/releases/tag/v0.2.0"), r).as_deref(), Some("0.2.0"));
        assert_eq!(version_of_redirect(&format!("{r}/releases/tag/0.3\r\n"), r).as_deref(), Some("0.3"));
        // No release yet: GitHub sends back to the releases list.
        assert_eq!(version_of_redirect(&format!("{r}/releases"), r), None);
        assert_eq!(version_of_redirect("https://example.com/x/releases/tag/v9.0", r), None);
        assert_eq!(version_of_redirect(&format!("{r}/releases/tag/v1.0/../../evil"), r), None);
        assert_eq!(version_of_redirect("", r), None);
        assert_eq!(release_page("0.2.0"), format!("{r}/releases/tag/v0.2.0"));
    }

    #[test]
    fn settings() {
        let dir = std::env::temp_dir().join(format!("openxiii-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(enabled(&dir));
        set_enabled(&dir, false).unwrap();
        assert!(!enabled(&dir));
        set_enabled(&dir, true).unwrap();
        assert!(enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
