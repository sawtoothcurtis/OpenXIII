//! Improved build only: the menus' cut-out characters with their edge anti-aliased.
//!
//! The characters standing beside the options pages (Jones on the Video and Difficulty pages,
//! XIII on the Sound and Load pages, the group of the multiplayer rules) are textures with a hard
//! mask, cut in two or three tiles. Enlarged on a large screen, the mask shows as a staircase.
//! Each character is put back together from its tiles, its edge anti-aliased as Jones's on the
//! Select level page (select_level_art::smooth_edge: the shape and the inside are the drawing's
//! own), at the size it is shown at on the screen, then cut back into tiles that replace the
//! game's textures while the game runs (mod_textures). The game's files are not changed, and the
//! pages draw as before: only the texels they read are finer.
//!
//! Always on with Graphics: Enhanced (no setting); Graphics: Original removes the replacements.

use std::sync::Mutex;

use crate::menu_boxes::Image;
use crate::select_level_art as art;
use crate::Vm;

/// A character: its name (cache file), the size of its whole picture and its tiles (texture,
/// place of its top left corner in the picture).
pub struct Figure {
    pub name: &'static str,
    pub size: (u32, u32),
    /// Parts of the whole picture (x0, y0, x1, y1) whose edge keeps its steps, only softened:
    /// the heads (the bun on the Video page).
    pub soft_zones: &'static [[u32; 4]],
    pub parts: &'static [(&'static str, u32, u32)],
}

/// The characters, as their pages lay the tiles out: the body's two tiles one over the other, the
/// rest of the floor shadow left of the lower one.
pub const FIGURES: [Figure; 5] = [
    Figure {
        name: "jones-video",
        size: (512, 512),
        soft_zones: &[[378, 6, 420, 84]],
        parts: &[
            ("XIIIMenuStart.Video_Controls.videojones01A", 256, 0),
            ("XIIIMenuStart.Video_Controls.videojones02A", 256, 256),
            ("XIIIMenuStart.Video_Controls.videojones03A", 0, 256),
        ],
    },
    Figure {
        name: "jones-difficulty",
        size: (512, 512),
        soft_zones: &[[352, 0, 424, 80]],
        parts: &[
            ("XIIIMenuStart.Difficulty.difficultyjones01A", 256, 0),
            ("XIIIMenuStart.Difficulty.difficultyjones02A", 256, 256),
            ("XIIIMenuStart.Difficulty.difficultyjones03A", 0, 384),
        ],
    },
    Figure {
        name: "xiii-sound",
        size: (512, 512),
        soft_zones: &[[364, 0, 418, 66]],
        parts: &[
            ("XIIIMenuStart.audio_controls.sonsXIII01A", 256, 0),
            ("XIIIMenuStart.audio_controls.sonsXIII02A", 256, 256),
            ("XIIIMenuStart.audio_controls.sonsXIII03A", 0, 256),
        ],
    },
    Figure {
        name: "xiii-load",
        size: (256, 384),
        soft_zones: &[[108, 0, 192, 90]],
        parts: &[("XIIIMenuStart.Interface_LoadGame.XIII_buste", 0, 0), ("XIIIMenuStart.Interface_LoadGame.XIII_jambes", 0, 128)],
    },
    Figure {
        name: "multiplayer-rules",
        size: (256, 512),
        soft_zones: &[[48, 0, 92, 64], [100, 84, 132, 128], [146, 38, 186, 90]],
        parts: &[("XIIIMenuStart.Multi_rules.characters01", 0, 0), ("XIIIMenuStart.Multi_rules.characters02", 0, 256)],
    },
];

/// The tiles of `figure` put back together (None when a tile is missing or does not fit).
pub fn assemble(figure: &Figure, mut texture: impl FnMut(&str) -> Option<Image>) -> Option<Image> {
    let mut whole = Image::new(figure.size.0, figure.size.1);
    for &(name, x, y) in figure.parts {
        let tile = texture(name)?;
        if x + tile.w > whole.w || y + tile.h > whole.h || tile.rgba.len() != (tile.w * tile.h * 4) as usize {
            return None;
        }
        let row = (tile.w * 4) as usize;
        for ty in 0..tile.h {
            let (s, d) = ((ty * tile.w * 4) as usize, (((y + ty) * whole.w + x) * 4) as usize);
            whole.rgba[d..d + row].copy_from_slice(&tile.rgba[s..s + row]);
        }
    }
    Some(whole)
}

/// The tiles of `figure` cut from its whole picture enlarged by `scale` / SCALE_ONE (tiles are
/// placed on multiples of SCALE_ONE texels: whole pixels at any enlargement): (texture, image).
pub fn cut(figure: &Figure, whole: &Image, scale: u32, mut texture_size: impl FnMut(&str) -> Option<(u32, u32)>) -> Vec<(&'static str, Image)> {
    let k = |v: u32| v * scale / art::SCALE_ONE;
    figure
        .parts
        .iter()
        .filter_map(|&(name, x, y)| {
            let (w, h) = texture_size(name)?;
            Some((name, whole.region(k(x), k(y), k(w), k(h))))
        })
        .collect()
}

/// Names a texture is drawn under: its full name, and without its group.
fn names(texture: &str) -> Vec<String> {
    let mut out = vec![texture.to_string()];
    let parts: Vec<&str> = texture.split('.').collect();
    if parts.len() == 3 {
        out.push(format!("{}.{}", parts[0], parts[2]));
    }
    out
}

/// Version of the soft zones (in the cache files' names: other zones, other pictures).
const ZONES: u32 = 1;

/// What is in place now for each character: enlargement and variant (None: the game's own
/// textures).
static APPLIED: Mutex<[Option<(u32, bool)>; FIGURES.len()]> = Mutex::new([None; FIGURES.len()]);

/// Pixels of the screen per texel each character's first tile was last drawn at (0: not drawn
/// yet), noted by the canvas.
static SHOWN: Mutex<[f32; FIGURES.len()]> = Mutex::new([0.0; FIGURES.len()]);

/// Improved build: size on the screen (pixels) every XIIIMenuStart texture was last drawn at
/// (lower-case name), for the onomatopoeias prepared at their screen size.
static SHOWN_SIZE: Mutex<Vec<(String, [f32; 2])>> = Mutex::new(Vec::new());

pub(crate) fn note_shown_size(key: &str, w: f32, h: f32) {
    if let Ok(mut all) = SHOWN_SIZE.lock() {
        let lower = key.to_ascii_lowercase();
        match all.iter_mut().find(|(k, _)| *k == lower) {
            Some(e) => e.1 = [w, h],
            None => all.push((lower, [w, h])),
        }
    }
}

/// Size on the screen (pixels) `key` was last drawn at, if drawn.
pub fn shown_size(key: &str) -> Option<[f32; 2]> {
    let key = key.to_ascii_lowercase();
    SHOWN_SIZE.lock().ok()?.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// True for a texture name that may be a character's tile (cheap test for the canvas).
pub(crate) fn maybe_tile(key: &str) -> bool {
    key.len() > 14 && key.as_bytes()[..14].eq_ignore_ascii_case(b"XIIIMenuStart.")
}

/// The canvas drew texture `key` at `pixels_per_texel`: kept when it is a character's first
/// tile (the body).
pub(crate) fn note_shown(key: &str, pixels_per_texel: f32) {
    for (n, figure) in FIGURES.iter().enumerate() {
        if names(figure.parts[0].0).iter().any(|name| name.eq_ignore_ascii_case(key)) {
            if let Ok(mut shown) = SHOWN.lock() {
                shown[n] = pixels_per_texel;
            }
            return;
        }
    }
}

/// Enlargement (in 1 / SCALE_ONE) of the master pictures: the largest size the menus can show
/// them at on this machine's largest screen mode, at most 4 times. Made once; any smaller screen reduces it on the
/// graphics card (bicubic read, mod_textures::set_runtime_fine): nothing is made again when the
/// resolution or the Interface size changes.
pub fn master_scale(display_modes: &[(u32, u32, u32)]) -> u32 {
    let tallest = display_modes.iter().map(|m| m.1).max().filter(|&h| h >= 480).unwrap_or(2160);
    // 1.25 x the 1280 x 720 frame's scale is what a page's 1:1 texel gets; a third more covers
    // the larger Interface sizes.
    let k = (tallest as f32 / 540.0 * art::SCALE_ONE as f32 / 16.0).ceil() as u32 * 16;
    k.clamp(art::SCALE_ONE, 4 * art::SCALE_ONE)
}

/// `master_scale`, or the larger scale of the master pictures already kept on the disk (made
/// when the game was installed, or for a larger screen): a smaller screen reuses them.
pub fn master_scale_kept(display_modes: &[(u32, u32, u32)]) -> u32 {
    static KEPT: Mutex<Option<u32>> = Mutex::new(None);
    let kept = KEPT.lock().map_or(0, |mut k| *k.get_or_insert_with(|| crate::select_level_page::art_cache_dir().map_or(0, |d| kept_scale(&d))));
    master_scale(display_modes).max(kept)
}

/// Cache file of a character's master picture at scale `k`.
fn figure_file(dir: &std::path::Path, figure: &Figure, k: u32, plain: bool) -> std::path::PathBuf {
    dir.join(format!("menu-figure-{}-{}-z{ZONES}.bin", figure.name, art::edge_tag(k, plain)))
}

/// Cache file of Select level's Jones at scale `k`.
fn jones_file(dir: &std::path::Path, k: u32, plain: bool) -> std::path::PathBuf {
    dir.join(format!("select-level-jones-{}.bin", art::edge_tag(k, plain)))
}

/// The largest scale at which every master picture (the characters and Select level's Jones,
/// smooth edge) is in `dir`; 0 when none is complete.
pub fn kept_scale(dir: &std::path::Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut scales: Vec<u32> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix("select-level-jones-q")?;
            rest.strip_suffix("-smooth.bin")?.parse().ok()
        })
        .collect();
    scales.sort_unstable_by(|a, b| b.cmp(a));
    scales
        .into_iter()
        .find(|&k| art::cache_current(&jones_file(dir, k, false)) && FIGURES.iter().all(|f| art::cache_current(&figure_file(dir, f, k, false))))
        .unwrap_or(0)
}

/// Makes every master picture at scale `k` into `dir` (the game being installed: the menus then
/// only read them): the characters, then Select level's Jones; `done` is called after each of
/// the 6 pictures. The pictures of other sizes are removed.
pub fn make_files(dir: &std::path::Path, k: u32, mut read: impl FnMut(&str) -> Option<Image>, mut done: impl FnMut()) {
    for figure in &FIGURES {
        let file = figure_file(dir, figure, k, false);
        let size = (figure.size.0 * k / art::SCALE_ONE, figure.size.1 * k / art::SCALE_ONE);
        if art::cache_read(&file).is_none_or(|i| (i.w, i.h) != size) {
            if let Some(whole) = assemble(figure, &mut read) {
                art::cache_write(&file, &art::smooth_edge(&whole, k, false, figure.soft_zones));
            }
        }
        done();
    }
    if !art::cache_current(&jones_file(dir, k, false)) {
        let _ = art::jones_fine(Some(dir), k, false, &mut read);
    }
    done();
    forget_other_sizes(dir, &art::edge_tag(k, false));
}

/// A master job in the background: its target, and whether it is finished.
static JOB: Mutex<Option<((u32, bool), bool)>> = Mutex::new(None);

impl Vm {
    /// The characters' textures brought to the Graphics setting (every frame of the menus:
    /// nothing is done while it does not change). With Graphics: Enhanced, one master picture
    /// per character is made once in the background (or read from the cache folder) at the
    /// largest size this machine can show; the game's own textures are drawn until it is ready.
    pub(crate) fn menu_figures_apply(&mut self) {
        let wanted = self.character_edge().map(|plain| (master_scale_kept(&self.display_modes), plain));
        {
            let Ok(applied) = APPLIED.lock() else { return };
            if applied.iter().all(|a| *a == wanted) {
                return;
            }
        }
        let Some((k, plain)) = wanted else {
            if let Ok(mut job) = JOB.lock() {
                *job = None;
            }
            for figure in &FIGURES {
                for &(name, _, _) in figure.parts {
                    for n in names(name) {
                        openxiii_assets::mod_textures::remove_runtime(&n);
                    }
                }
            }
            if let Ok(mut applied) = APPLIED.lock() {
                *applied = [None; FIGURES.len()];
            }
            return;
        };
        {
            let Ok(mut job) = JOB.lock() else { return };
            if job.is_some_and(|(target, _)| target == (k, plain)) {
                return;
            }
            *job = Some(((k, plain), false));
        }
        // The game's tiles read here (cheap); the edges made on another thread.
        let inputs: Vec<Option<(Image, Vec<(u32, u32)>)>> = FIGURES
            .iter()
            .map(|figure| {
                let whole = assemble(figure, |p| self.menu_texture_image(p))?;
                let sizes = figure.parts.iter().map(|&(name, _, _)| self.menu_texture_image(name).map(|i| (i.w, i.h))).collect::<Option<Vec<_>>>()?;
                Some((whole, sizes))
            })
            .collect();
        let cache = crate::select_level_page::art_cache_dir();
        let jones_quarters: Vec<Option<Image>> = art::JONES_TEXTURES.iter().map(|p| self.menu_texture_image(p)).collect();
        let math = openxiii_math::Mode::current();
        let spawned = std::thread::Builder::new().name("menu-figures".into()).spawn(move || {
            let _math = math.enter();
            openxiii_package::background::lower_this_thread();
            for (n, (figure, input)) in FIGURES.iter().zip(inputs).enumerate() {
                if JOB.lock().map_or(true, |j| j.map(|(t, _)| t) != Some((k, plain))) {
                    return;
                }
                let Some((whole, sizes)) = input else { continue };
                let file = cache.as_ref().map(|d| figure_file(d, figure, k, plain));
                let size = (figure.size.0 * k / art::SCALE_ONE, figure.size.1 * k / art::SCALE_ONE);
                let fine = file.as_deref().and_then(art::cache_read).filter(|i| (i.w, i.h) == size).unwrap_or_else(|| {
                    let made = art::smooth_edge(&whole, k, plain, figure.soft_zones);
                    if let Some(f) = &file {
                        art::cache_write(f, &made);
                    }
                    made
                });
                let mut size_of = sizes.iter().copied();
                for (name, img) in cut(figure, &fine, k, |_| size_of.next()) {
                    let mips = openxiii_assets::mod_textures::mips(img.w, img.h, img.rgba);
                    for t in names(name) {
                        openxiii_assets::mod_textures::set_runtime_fine(&t, mips.clone());
                    }
                }
                if let Ok(mut applied) = APPLIED.lock() {
                    applied[n] = Some((k, plain));
                }
            }
            // Select level's Jones at the same size, ready before her page opens.
            if let [Some(a), Some(b), Some(c), Some(d)] = &jones_quarters[..] {
                let quarters = [a, b, c, d];
                let _ = art::jones_fine(cache.as_deref(), k, plain, |p| art::JONES_TEXTURES.iter().position(|t| *t == p).map(|i| quarters[i].clone()));
            }
            if let Some(dir) = cache.as_deref() {
                forget_other_sizes(dir, &art::edge_tag(k, plain));
            }
            if let Ok(mut job) = JOB.lock() {
                if job.map(|(t, _)| t) == Some((k, plain)) {
                    *job = Some(((k, plain), true));
                }
            }
        });
        if spawned.is_err() {
            if let Ok(mut job) = JOB.lock() {
                *job = None;
            }
        }
    }
}

/// Removes the enlarged pictures of other sizes from the cache (left by another screen or an
/// older version): only the pictures of this computer's size (`tag`) stay on the disk.
fn forget_other_sizes(dir: &std::path::Path, tag: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mine = format!("-{tag}");
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let sized = name.starts_with("menu-figure-") || name.starts_with("select-level-jones-q");
        if sized && !name.contains(&mine) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(w: u32, h: u32, c: u8) -> Image {
        Image { w, h, rgba: (0..w * h).flat_map(|_| [c, c, c, 255]).collect() }
    }

    #[test]
    fn tiles_are_put_together_and_cut_back() {
        const F: Figure = Figure { name: "t", size: (8, 8), soft_zones: &[], parts: &[("P.G.a", 4, 0), ("P.G.b", 4, 4), ("P.G.c", 0, 6)] };
        let read = |n: &str| match n {
            "P.G.a" => Some(tile(4, 4, 10)),
            "P.G.b" => Some(tile(4, 4, 20)),
            "P.G.c" => Some(tile(4, 2, 30)),
            _ => None,
        };
        let whole = assemble(&F, read).unwrap();
        let at = |x: u32, y: u32| whole.rgba[((y * 8 + x) * 4) as usize];
        assert_eq!((at(4, 0), at(7, 7), at(0, 6), at(0, 0)), (10, 20, 30, 0));
        // Cut back at twice the size from a picture enlarged by repeating texels.
        let mut big = Image::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let (s, d) = ((((y / 2) * 8 + x / 2) * 4) as usize, ((y * 16 + x) * 4) as usize);
                big.rgba[d..d + 4].copy_from_slice(&whole.rgba[s..s + 4]);
            }
        }
        let tiles = cut(&F, &big, 2 * art::SCALE_ONE, |n| read(n).map(|i| (i.w, i.h)));
        assert_eq!(tiles.len(), 3);
        assert_eq!((tiles[2].0, tiles[2].1.w, tiles[2].1.h), ("P.G.c", 8, 4));
        assert!(tiles[2].1.rgba.chunks_exact(4).all(|p| p == [30, 30, 30, 255]));
        assert!(tiles[0].1.rgba.chunks_exact(4).all(|p| p[0] == 10));
        // A missing tile: no picture.
        assert!(assemble(&F, |n| if n == "P.G.b" { None } else { read(n) }).is_none());
        assert_eq!(names("P.G.a"), ["P.G.a", "P.a"]);
        assert_eq!(names("P.a"), ["P.a"]);
        // The game's tiles fit their pictures.
        for f in &FIGURES {
            assert!(f.parts.iter().all(|&(_, x, y)| x < f.size.0 && y < f.size.1), "{}", f.name);
        }
    }
}
