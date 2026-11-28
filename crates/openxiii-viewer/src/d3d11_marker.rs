//! Direct3D 11 attempt markers (Windows, dx11 feature): a game that stops while Direct3D 11 is
//! being started in a window (a driver crash) must not stop the same way at the next start, so
//! each attempt is recorded until the window's start is finished, and the next starts fall back
//! to Vulkan or OpenGL after such a failure.
//!
//! In %LOCALAPPDATA%\OpenXIII, `d3d11-test-<pid>.txt` is an attempt in progress by process `pid`:
//! written before opening the adapter, deleted once the window's start is finished (surface,
//! renderer, pipeline precompilation) or when Direct3D 11 is refused.
//!
//! At each start, a file whose process no longer runs is a failure: Direct3D 11 is skipped, as
//! long as that file stays (OXIII_RENDERER=dx11 tries again; a finished start then removes the
//! files of ended processes). The files of running processes (another game starting at the same
//! time) are left alone. The single `d3d11-test.txt` of earlier versions, without a process, is
//! deleted without being counted.

use std::path::{Path, PathBuf};

const PREFIX: &str = "d3d11-test-";
const LEGACY: &str = "d3d11-test.txt";

/// What the attempt files found at start mean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// Direct3D 11 is skipped this time: an attempt's process ended during it.
    pub skip: bool,
    /// Files whose name carries no process, to delete.
    pub unreadable: Vec<PathBuf>,
}

/// The decision from the attempt files `markers` (path, process id read from the name; None:
/// unreadable), `alive` telling whether a process still runs and this process' id `me`. Pure: no
/// file or process is touched.
pub fn decide(markers: &[(PathBuf, Option<u32>)], alive: &dyn Fn(u32) -> bool, me: u32) -> Decision {
    let mut skip = false;
    let mut unreadable = Vec::new();
    for (path, pid) in markers {
        match pid {
            // Our own (a second window in the same process) or another game still starting.
            Some(p) if *p == me || alive(*p) => {}
            // A process that ended during its attempt.
            Some(_) => skip = true,
            None => unreadable.push(path.clone()),
        }
    }
    Decision { skip, unreadable }
}

/// Process id carried by an attempt file's name (`d3d11-test-<pid>.txt`).
pub fn pid_of(name: &str) -> Option<u32> {
    name.strip_prefix(PREFIX)?.strip_suffix(".txt")?.parse().ok()
}

/// Folder of the files (%LOCALAPPDATA%\OpenXIII).
pub fn folder() -> Option<PathBuf> {
    openxiii_package::env::var_os("LOCALAPPDATA").map(|d| Path::new(&d).join("OpenXIII"))
}

/// The attempt files in `dir`, with their process ids.
fn markers_in(dir: &Path) -> Vec<(PathBuf, Option<u32>)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(PREFIX).then(|| (e.path(), pid_of(&name)))
        })
        .collect()
}

fn own_file(dir: &Path) -> PathBuf {
    dir.join(format!("{PREFIX}{}.txt", std::process::id()))
}

/// At start, before an attempt in a window: true when Direct3D 11 is to be skipped (an attempt's
/// process ended during it). The earlier versions' single file and unreadable names are removed.
pub fn should_skip_in(dir: &Path, alive: &dyn Fn(u32) -> bool) -> bool {
    let _ = std::fs::remove_file(dir.join(LEGACY));
    let d = decide(&markers_in(dir), alive, std::process::id());
    for f in &d.unreadable {
        let _ = std::fs::remove_file(f);
    }
    d.skip
}

/// Start of this process' attempt on adapter `name`.
pub fn begin_in(dir: &Path, name: &str) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(own_file(dir), name);
}

/// End of this process' attempt: the window's start finished (`finished`: the files of ended
/// processes are removed too, Direct3D 11 works again) or Direct3D 11 was refused.
pub fn end_in(dir: &Path, finished: bool, alive: &dyn Fn(u32) -> bool) {
    let _ = std::fs::remove_file(own_file(dir));
    if finished {
        for (f, pid) in markers_in(dir) {
            if pid.is_some_and(|p| !alive(p)) {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

/// The game ends normally (its window closed, Quit in the menus) while its start may not be
/// finished: its own attempt is not a failure, its file goes. Called only on the clean way out of
/// the event loop: a crash, a panic or a killed process never gets here, so their file stays.
pub fn clean_exit_in(dir: &Path) {
    let _ = std::fs::remove_file(own_file(dir));
}

/// `clean_exit_in` in %LOCALAPPDATA%\OpenXIII.
pub fn clean_exit() {
    if let Some(d) = folder() {
        clean_exit_in(&d);
    }
}

/// Process `pid` still runs (Windows: OpenProcess and GetExitCodeProcess; elsewhere, assumed).
pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn GetExitCodeProcess(process: *mut std::ffi::c_void, code: *mut u32) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: plain Win32 calls on a handle this function opens and closes.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pid: u32) -> (PathBuf, Option<u32>) {
        (PathBuf::from(format!("{PREFIX}{pid}.txt")), Some(pid))
    }

    #[test]
    fn names() {
        assert_eq!(pid_of("d3d11-test-1234.txt"), Some(1234));
        assert_eq!(pid_of("d3d11-test.txt"), None);
        assert_eq!(pid_of("d3d11-test-x.txt"), None);
    }

    #[test]
    fn decisions() {
        let dead = |_: u32| false;
        let alive = |_: u32| true;
        // Nothing: Direct3D 11.
        assert!(!decide(&[], &dead, 1).skip);
        // Another game starting at the same time: ignored.
        assert!(!decide(&[m(7)], &alive, 1).skip);
        // Our own file: ignored.
        assert!(!decide(&[m(1)], &dead, 1).skip);
        // One process that ended during its attempt: skipped at once.
        assert!(decide(&[m(7)], &dead, 1).skip);
        // Mixed: the ended one decides.
        assert!(decide(&[m(7), m(9)], &|p| p == 7, 1).skip);
        assert!(!decide(&[m(7), m(9)], &|p| p == 7 || p == 9, 1).skip);
        // A file without a process: removed, not a failure.
        let d = decide(&[(PathBuf::from("d3d11-test-x.txt"), None)], &dead, 1);
        assert_eq!((d.skip, d.unreadable.len()), (false, 1));
    }

    /// The files themselves: an ended attempt keeps Direct3D 11 skipped until a finished start;
    /// a running game's attempt is left alone.
    #[test]
    fn files_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxiii-d3d11-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dead = |_: u32| false;
        // An earlier version's single file: removed, not a failure.
        std::fs::write(dir.join(LEGACY), "x").unwrap();
        assert!(!should_skip_in(&dir, &dead));
        assert!(!dir.join(LEGACY).exists());
        // A running game's attempt: left alone, Direct3D 11 kept.
        std::fs::write(dir.join("d3d11-test-4000000002.txt"), "gpu").unwrap();
        assert!(!should_skip_in(&dir, &|p| p == 4000000002));
        assert!(dir.join("d3d11-test-4000000002.txt").exists());
        // It ends without finishing its start: skipped, and again at the next start.
        assert!(should_skip_in(&dir, &dead));
        assert!(should_skip_in(&dir, &dead));
        // A refused attempt of this process removes only its own file: still skipped.
        begin_in(&dir, "gpu");
        end_in(&dir, false, &dead);
        assert!(!own_file(&dir).exists());
        assert!(should_skip_in(&dir, &dead));
        // A finished start (forced Direct3D 11) removes the ended processes' files.
        begin_in(&dir, "gpu");
        end_in(&dir, true, &dead);
        assert!(!own_file(&dir).exists() && !dir.join("d3d11-test-4000000002.txt").exists());
        assert!(!should_skip_in(&dir, &dead));
        // Negative control: with every process taken as running, an attempt file never skips.
        std::fs::write(dir.join("d3d11-test-4000000003.txt"), "gpu").unwrap();
        assert!(!should_skip_in(&dir, &|_| true));
        assert!(should_skip_in(&dir, &dead));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Closed during its start: a clean exit removes the attempt, an ended process that did not
    /// pass there (killed, crashed) keeps Direct3D 11 skipped.
    #[test]
    fn clean_exit_during_start() {
        let dir = std::env::temp_dir().join(format!("oxiii-d3d11-exit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dead = |_: u32| false;
        // The window closed in its first second: begun, never finished, then a clean exit.
        begin_in(&dir, "gpu");
        clean_exit_in(&dir);
        assert!(!own_file(&dir).exists());
        assert!(!should_skip_in(&dir, &dead), "a clean exit is not a failure");
        // A real process that ends without a clean exit (an abort or a kill): its file stays and
        // the next start skips Direct3D 11.
        let child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" }).args(if cfg!(windows) { &["/c", "exit", "3"][..] } else { &[][..] }).spawn().unwrap();
        let pid = child.id();
        std::fs::write(dir.join(format!("{PREFIX}{pid}.txt")), "gpu").unwrap();
        let mut child = child;
        let _ = child.wait();
        if cfg!(windows) {
            assert!(should_skip_in(&dir, &process_alive), "an ended attempt without a clean exit still counts");
        }
        // This process' clean exit leaves the other process' file alone.
        clean_exit_in(&dir);
        assert!(dir.join(format!("{PREFIX}{pid}.txt")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn this_process_is_alive() {
        assert!(process_alive(std::process::id()));
        if cfg!(windows) {
            assert!(!process_alive(4_000_000_001));
        }
    }
}
