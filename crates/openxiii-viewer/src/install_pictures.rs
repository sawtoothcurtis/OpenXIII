//! Improved build, the game a player starts: the menus' improved pictures made once, when the
//! game is installed (its first launch), behind a "Loading..." screen: the video frames of the
//! added menus (Mods, Co-op, Select level's background), Select level's Jones, the menus'
//! characters with their smooth edge and the onomatopoeias. They are kept compressed in the data
//! folder's cache and only read afterwards; they are made again when one is missing or when a
//! larger screen needs larger master pictures. The game's frames then never wait for them.
//!
//! On a computer the progress is a small window before the game's (desktop_shortcut.rs); on
//! Android, where nothing can be shown before the game's window, the game's window shows it
//! (`Running`), the game starting once it is finished.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use openxiii_vm::menu_boxes::Image;

/// Every mode of the primary screen (width, height, bits), as the game window lists them.
#[cfg(windows)]
fn screen_modes() -> Vec<(u32, u32, u32)> {
    #[link(name = "user32")]
    extern "system" {
        fn EnumDisplaySettingsW(device: *const u16, mode: u32, devmode: *mut u8) -> i32;
    }
    let mut out = Vec::new();
    for i in 0.. {
        // DEVMODEW (220 bytes): dmSize at 68, dmBitsPerPel at 168, dmPelsWidth at 172,
        // dmPelsHeight at 176 (words 17, 42, 43, 44).
        let mut devmode = [0u32; 55];
        devmode[17] = 220;
        // SAFETY: a zeroed DEVMODEW of the size it declares; a null device is the primary display.
        if unsafe { EnumDisplaySettingsW(std::ptr::null(), i, devmode.as_mut_ptr().cast()) } == 0 {
            break;
        }
        out.push((devmode[43], devmode[44], devmode[42]));
    }
    out
}

/// What is to be made in `cache`: the video frames, whether the master pictures (at scale `k`)
/// and the onomatopoeias are.
struct Plan {
    game: PathBuf,
    cache: PathBuf,
    k: u32,
    frames: Vec<(PathBuf, u32)>,
    masters: bool,
    onomatopoeias: bool,
}

impl Plan {
    /// What is missing; None when everything is there. `video_dir`: the folder of the videos,
    /// as the game names it (the frames being made are known by that path).
    fn new(game: &Path, video_dir: &Path, cache: &Path, display_modes: &[(u32, u32, u32)]) -> Option<Plan> {
        let k = openxiii_vm::menu_figures::master_scale(display_modes);
        let frames: Vec<_> = openxiii_vm::menu_boxes::video_frames(video_dir).into_iter().filter(|(v, i)| v.is_file() && !openxiii_vm::still_frames::kept(v, *i)).collect();
        let masters = openxiii_vm::menu_figures::kept_scale(cache) < k;
        let onomatopoeias = !cache.join("onomatopoeias.bin").is_file();
        (!frames.is_empty() || masters || onomatopoeias).then(|| Plan { game: game.to_path_buf(), cache: cache.to_path_buf(), k, frames, masters, onomatopoeias })
    }

    fn steps(&self) -> u32 {
        self.frames.len() as u32 + if self.masters { 6 } else { 0 } + u32::from(self.onomatopoeias)
    }

    /// Makes everything, `step` called after each step.
    fn run(&self, step: &dyn Fn()) {
        let started = std::time::Instant::now();
        let mark = |what: &str, t: &mut std::time::Instant| {
            crate::renderer::diag(&format!("Loading: {what} in {:.2} s", t.elapsed().as_secs_f64()));
            *t = std::time::Instant::now();
        };
        let mut t = std::time::Instant::now();
        // The video frames decoded by ffmpeg (one core each) while the other pictures are made.
        for (video, index) in &self.frames {
            openxiii_vm::still_frames::request(video, *index);
        }
        if self.onomatopoeias {
            crate::onomatopoeias::prepare(&self.game);
            step();
            mark("onomatopoeias", &mut t);
        }
        if self.masters {
            if let Ok(mut library) = openxiii_package::library::Library::new(&self.game) {
                let read = |path: &str| {
                    let (loaded, index) = library.resolve(path)?;
                    let t = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).ok()?;
                    let (w, h, rgba) = t.d3d_mips().into_iter().next()?;
                    Some(Image { w, h, rgba })
                };
                openxiii_vm::menu_figures::make_files(&self.cache, self.k, read, step);
            }
            mark(&format!("characters and Jones (scale {})", self.k), &mut t);
        }
        for (video, index) in &self.frames {
            let _ = openxiii_vm::still_frames::wait(video, *index, std::time::Duration::from_secs(600));
            step();
            mark(&format!("video frame {}", video.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default()), &mut t);
        }
        crate::renderer::diag(&format!("Menus' pictures prepared in {:.1} s", started.elapsed().as_secs_f64()));
    }
}

/// Makes what is missing in `cache` (nothing when everything is there), showing a progress
/// window while it works (a computer, before the game's window opens).
#[cfg(windows)]
pub fn prepare(game: &Path, cache: &Path) {
    let Some(plan) = Plan::new(game, &game.join("Video"), cache, &screen_modes()) else { return };
    crate::desktop_shortcut::with_progress(crate::launcher::MARK, "Loading...", plan.steps(), |step: &dyn Fn(&str)| plan.run(&|| step("")));
}

/// The making going on behind the game window's "Loading..." screen (Android).
#[cfg_attr(windows, allow(dead_code))]
pub struct Running {
    steps: u32,
    done: Arc<AtomicU32>,
    finished: Arc<AtomicBool>,
}

impl Running {
    /// Starts making what is missing on another thread; None when everything is there.
    /// `display_modes`: the screen's (the master pictures' size).
    pub fn start(game: &Path, video_dir: &Path, cache: &Path, display_modes: &[(u32, u32, u32)]) -> Option<Running> {
        let plan = Plan::new(game, video_dir, cache, display_modes)?;
        let running = Running { steps: plan.steps(), done: Arc::default(), finished: Arc::default() };
        let (done, finished) = (running.done.clone(), running.finished.clone());
        let spawned = std::thread::Builder::new().name("install-pictures".into()).spawn(move || {
            plan.run(&|| {
                done.fetch_add(1, Ordering::AcqRel);
            });
            finished.store(true, Ordering::Release);
        });
        spawned.is_ok().then_some(running)
    }

    /// Share done, from 0 to 100.
    pub fn percent(&self) -> u32 {
        self.done.load(Ordering::Acquire).min(self.steps) * 100 / self.steps.max(1)
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}
