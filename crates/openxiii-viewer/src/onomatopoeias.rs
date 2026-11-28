//! Improved build: the menus' onomatopoeias (the words shown over the menu boxes on hover,
//! XIIIMenuStart) with smooth outlines, with Graphics: Enhanced only. The 15 pictures are
//! redrawn once per run on other cores while the game loads its first map (the black screen
//! before the main menu): ScaleFX 3x, SMAA, blacks boosted. They stay in memory (no file is
//! written). When the canvas draws one, the renderer reduces it with a Lanczos-3 filter to the
//! exact size it is shown at (the VM notes that size), with the transparent outline slightly
//! tightened, and uploads that single level (no blurry mip level); a new screen size replaces
//! the previous textures. ScaleFX and SMAA are under the MIT license (see their files).

#[path = "onomatopoeias/scalefx.rs"]
mod scalefx;
#[path = "onomatopoeias/smaa.rs"]
mod smaa;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

/// The onomatopoeias of the menus (texture names, without package and group).
const NAMES: [&str; 15] = [
    "Bang",
    "Crash",
    "clapclap",
    "ksshss",
    "loadgameSlam",
    "multiplayerBam",
    "newgameWoowoo",
    "newgameksshhh",
    "onlineBaomm",
    "optionBrrrr",
    "optionDifficultyBrrrr",
    "optionSonBrrrr",
    "optionSonBuzzz",
    "optionSonWoooo",
    "optionVideoTiptap",
];

/// Final sizes (menu units, times the menu's scale on the screen) the menus' pages draw each
/// onomatopoeia at once its pop-in zoom ends, as the game's menu scripts do; one onomatopoeia can
/// end at two sizes on different pages. optionSonBrrrr was not seen drawn: optionBrrrr's.
pub const SIZES: &[(&str, u32, u32)] = &[
    ("Bang", 96, 64),
    ("Bang", 113, 63),
    ("clapclap", 128, 64),
    ("clapclap", 158, 76),
    ("Crash", 142, 64),
    ("Crash", 158, 76),
    ("ksshss", 158, 76),
    ("ksshss", 193, 96),
    ("loadgameSlam", 160, 74),
    ("multiplayerBam", 100, 100),
    ("newgameksshhh", 223, 63),
    ("newgameWoowoo", 223, 73),
    ("onlineBaomm", 224, 64),
    ("optionBrrrr", 193, 96),
    ("optionDifficultyBrrrr", 248, 76),
    ("optionSonBrrrr", 193, 96),
    ("optionSonBuzzz", 120, 40),
    ("optionSonBuzzz", 158, 76),
    ("optionSonWoooo", 237, 126),
    ("optionVideoTiptap", 158, 71),
];

/// Black boost: brightness (brightest channel) below BLACK_LOW goes to black, above BLACK_HIGH
/// it is unchanged, smoothstep between; hue and alpha kept.
const BLACK_LOW: f32 = 0.10;
const BLACK_HIGH: f32 = 0.35;
/// Outline tightening: alpha contrast around 0.5 after the reduction.
const ALPHA_CONTRAST: f32 = 1.35;
/// Enlargement made by ScaleFX.
const SCALE: usize = 3;

/// A treated picture at 3x: width, height, straight-alpha RGBA8.
pub struct Base {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
struct State {
    /// The game folder (pictures read from its packages).
    game: Option<PathBuf>,
    started: bool,
    done: bool,
    /// By lower-case texture name (without package and group: the canvas may draw a texture
    /// under its full path or without its group).
    bases: HashMap<String, Arc<Base>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static DONE: Condvar = Condvar::new();

/// Always on in a released game (Graphics: Enhanced decides).
pub fn enabled() -> bool {
    true
}

/// In use now: Graphics: Enhanced and not turned off.
pub fn active() -> bool {
    crate::use_menu_master_textures()
}

/// `path` is one of the menus' onomatopoeias (XIIIMenuStart.[Group.]Name).
pub fn is_onomatopoeia(path: &str) -> bool {
    (path.split('.').next().is_some_and(|p| p.eq_ignore_ascii_case("XIIIMenuStart"))
        && path.rsplit('.').next().is_some_and(|n| NAMES.iter().any(|o| o.eq_ignore_ascii_case(n))))
        || is_cine_bang(path)
}

/// The white BANG the controls' Shoot page draws over its picture (XIIICine.effets.Bang, one of
/// the comic panels' effects): treated like the menus' own onomatopoeias when the canvas draws it.
fn is_cine_bang(path: &str) -> bool {
    path.split('.').next().is_some_and(|p| p.eq_ignore_ascii_case("XIIICine")) && path.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("Bang"))
}

/// Key of the white BANG (the menus have their own "Bang").
const CINE_BANG: &str = "xiiicine-bang";

/// Starts redrawing the 15 pictures on another thread from the game folder (once per run, when
/// in use: with Graphics: Original it starts only when Enhanced is chosen). The renderer waits
/// for it only if it needs one before it is done.
pub fn start(game: PathBuf) {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = st.get_or_insert_with(State::default);
    st.game = Some(game);
    spawn_if_needed(st);
}

/// The installation is about to make the masters (`prepare`): the game does not start its own
/// redrawing meanwhile.
static HELD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// See HELD.
#[allow(dead_code)]
pub fn hold() {
    HELD.store(true, std::sync::atomic::Ordering::Release);
}

/// The installation will not make them: the game redraws them when it needs them.
#[allow(dead_code)]
pub fn release() {
    HELD.store(false, std::sync::atomic::Ordering::Release);
}

fn spawn_if_needed(st: &mut State) {
    if st.started || !active() || HELD.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let Some(game) = st.game.clone() else { return };
    st.started = true;
    let math = openxiii_math::Mode::current();
    let spawned = std::thread::Builder::new().name("onomatopoeias".into()).spawn(move || {
        let _math = math.enter();
        openxiii_package::background::lower_this_thread();
        let bases = load_or_make(&game);
        let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let st = st.get_or_insert_with(State::default);
        st.bases = bases;
        st.done = true;
        DONE.notify_all();
    });
    if spawned.is_err() {
        st.done = true;
    }
}

/// The masters, kept on the disk from an earlier run (or the installation) when the game's
/// pictures are the same; otherwise redrawn, then kept. They are given to the renderer; the
/// redrawn pictures are returned (none when read from the disk).
fn load_or_make(game: &std::path::Path) -> HashMap<String, Arc<Base>> {
    let sources = read_all(game);
    let print = fingerprint(&sources);
    let file = openxiii_vm::select_level_page::art_cache_dir().map(|d| d.join("onomatopoeias.bin"));
    let mut bases = HashMap::new();
    let masters = match file.as_deref().and_then(|f| cache_read(f, print)) {
        Some(masters) => masters,
        None => {
            bases = redraw_all(sources);
            let masters = make_masters(&bases);
            if let Some(f) = &file {
                cache_write(f, print, &masters);
            }
            masters
        }
    };
    for (key, mips) in masters {
        openxiii_assets::mod_textures::set_runtime_fine(&master_texture(&key), mips);
    }
    bases
}

/// The game being installed: the masters made now (or found on the disk) and given to the
/// renderer, so that nothing is made while the game runs.
pub fn prepare(game: &std::path::Path) {
    let bases = load_or_make(game);
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = st.get_or_insert_with(State::default);
    st.game = Some(game.to_path_buf());
    st.bases = bases;
    st.started = true;
    st.done = true;
    DONE.notify_all();
    release();
}

/// Texture name of an onomatopoeia's master picture.
fn master_texture(key: &str) -> String {
    format!("Mod.Onomatopoeia.{key}")
}

/// One master picture per onomatopoeia: the redrawing itself (3 times the texture, its own
/// proportions: pages draw the same picture at other sizes and shapes than on hover, as the
/// controls pages' corner picture), with its smaller levels each reduced from the redrawing
/// (Lanczos). The graphics card reads it with the bicubic filter at any screen size
/// (mod_textures fine textures), so nothing is made when the resolution changes and the menu
/// never waits.
fn make_masters(bases: &HashMap<String, Arc<Base>>) -> Vec<(String, Mips)> {
    let mut out = Vec::new();
    for (key, base) in bases {
        let (w, h) = (base.w, base.h);
        let mut mips = Vec::new();
        let mut l = 0;
        loop {
            let (lw, lh) = ((w >> l).max(1), (h >> l).max(1));
            mips.extend(at_size(base, lw, lh));
            if lw == 1 && lh == 1 {
                break;
            }
            l += 1;
        }
        out.push((key.clone(), mips));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

type Mips = openxiii_assets::mod_textures::Mips;

/// The game's pictures the masters are made from: name, size and RGBA of each (FNV-1a 64, the
/// same on every computer and version of the compiler).
fn fingerprint(sources: &[(String, usize, usize, Vec<u8>)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for (key, w, hh, rgba) in sources {
        eat(key.as_bytes());
        eat(&(*w as u64).to_le_bytes());
        eat(&(*hh as u64).to_le_bytes());
        eat(rgba);
    }
    h
}

/// Version of the masters' cache file and of the redrawing (a change makes them again).
const CACHE_VERSION: u32 = 1;

/// Reads `n` bytes at `*at` and moves past them.
fn take<'a>(data: &'a [u8], at: &mut usize, n: usize) -> Option<&'a [u8]> {
    let s = data.get(*at..at.checked_add(n)?)?;
    *at += n;
    Some(s)
}

fn take_u32(data: &[u8], at: &mut usize) -> Option<u32> {
    take(data, at, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Cache file: "OXON", version, fingerprint of the game's pictures, number of masters, then for
/// each its name (length, bytes) and levels (count, then width, height, compressed length and
/// the RGBA compressed). Numbers little-endian (u32, the fingerprint u64).
fn cache_read(path: &std::path::Path, print: u64) -> Option<Vec<(String, Mips)>> {
    let data = std::fs::read(path).ok()?;
    let at = &mut 0usize;
    if take(&data, at, 4)? != b"OXON" || take_u32(&data, at)? != CACHE_VERSION {
        return None;
    }
    if u64::from_le_bytes(take(&data, at, 8)?.try_into().ok()?) != print {
        return None;
    }
    let count = take_u32(&data, at)?;
    let mut out = Vec::new();
    for _ in 0..count {
        let n = take_u32(&data, at)? as usize;
        let key = String::from_utf8(take(&data, at, n)?.to_vec()).ok()?;
        let levels = take_u32(&data, at)?;
        let mut mips = Vec::new();
        for _ in 0..levels {
            let (w, h, len) = (take_u32(&data, at)?, take_u32(&data, at)?, take_u32(&data, at)? as usize);
            let rgba = openxiii_vm::select_level_art::unpack(take(&data, at, len)?, w as usize * h as usize * 4)?;
            mips.push((w, h, rgba));
        }
        out.push((key, mips));
    }
    Some(out)
}

fn cache_write(path: &std::path::Path, print: u64, masters: &[(String, Mips)]) {
    let mut data = Vec::new();
    data.extend_from_slice(b"OXON");
    data.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    data.extend_from_slice(&print.to_le_bytes());
    data.extend_from_slice(&(masters.len() as u32).to_le_bytes());
    for (key, mips) in masters {
        data.extend_from_slice(&(key.len() as u32).to_le_bytes());
        data.extend_from_slice(key.as_bytes());
        data.extend_from_slice(&(mips.len() as u32).to_le_bytes());
        for (w, h, rgba) in mips {
            let packed = openxiii_vm::select_level_art::pack(rgba);
            for v in [*w, *h, packed.len() as u32] {
                data.extend_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(&packed);
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Starts the redrawing if it has not started yet and Graphics: Enhanced is now on (a released
/// game reads the setting with the player's settings, after the start): called by the canvas
/// every frame of the menus (cheap once started).
pub fn kick() {
    if !active() {
        return;
    }
    if let Ok(mut st) = STATE.lock() {
        if let Some(s) = st.as_mut() {
            spawn_if_needed(s);
        }
    }
}

/// The master texture drawn for onomatopoeia `path`, once it is ready (never waits).
pub fn master_for(path: &str) -> Option<String> {
    if !is_onomatopoeia(path) || !active() {
        return None;
    }
    let name = master_texture(&name_key(path));
    openxiii_assets::mod_textures::is_runtime(&name).then_some(name)
}

/// The treated 3x picture of `path`, waiting for the redrawing if it is still running.
pub fn base(path: &str) -> Option<Arc<Base>> {
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = st.as_mut() {
        spawn_if_needed(s);
    }
    loop {
        match st.as_ref() {
            None => return None,
            Some(s) if !s.started => return None,
            Some(s) if s.done => return s.bases.get(&name_key(path)).cloned(),
            Some(_) => st = DONE.wait(st).unwrap_or_else(|e| e.into_inner()),
        }
    }
}

/// Every onomatopoeia of XIIIMenuStart redrawn.
#[cfg(test)]
fn make_all(game: &std::path::Path) -> HashMap<String, Arc<Base>> {
    redraw_all(read_all(game))
}

/// The pictures read by `read_all`, redrawn, by name.
fn redraw_all(sources: Vec<(String, usize, usize, Vec<u8>)>) -> HashMap<String, Arc<Base>> {
    sources.into_iter().filter_map(|(key, w, h, rgba)| Some((key, Arc::new(redraw(w, h, &rgba)?)))).collect()
}

/// The onomatopoeias' pictures in the game (first level), by name, in the order found (its own
/// copy of the packages: the game's library stays with the main thread).
fn read_all(game: &std::path::Path) -> Vec<(String, usize, usize, Vec<u8>)> {
    let mut out = Vec::new();
    let Ok(mut library) = openxiii_package::library::Library::new(game) else { return out };
    let Some(loaded) = library.get("XIIIMenuStart") else { return out };
    let package = &loaded.package;
    let found: Vec<(String, usize)> = (0..package.exports.len())
        .filter(|&i| package.export_class(i) == "Texture")
        .map(|i| (format!("XIIIMenuStart.{}", package.object_path(openxiii_package::ObjectRef::from((i + 1) as i32))), i))
        .filter(|(path, _)| is_onomatopoeia(path))
        .collect();
    for (path, i) in found {
        let Ok(texture) = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, i) else { continue };
        let mips = texture.d3d_mips();
        let Some((w, h, rgba)) = mips.into_iter().next() else { continue };
        out.push((name_key(&path), w as usize, h as usize, rgba));
    }
    // The white BANG of the controls' Shoot page.
    if let Some((loaded, i)) = library.resolve("XIIICine.effets.Bang") {
        if let Ok(texture) = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, i) {
            if let Some((w, h, rgba)) = texture.d3d_mips().into_iter().next() {
                out.push((CINE_BANG.to_string(), w as usize, h as usize, rgba));
            }
        }
    }
    out
}

/// Lower-case texture name, without package and group.
fn name_key(path: &str) -> String {
    if is_cine_bang(path) {
        return CINE_BANG.to_string();
    }
    path.rsplit('.').next().unwrap_or(path).to_ascii_lowercase()
}

/// ScaleFX 3x, SMAA, blacks boosted.
fn redraw(w: usize, h: usize, rgba: &[u8]) -> Option<Base> {
    if w == 0 || h == 0 || rgba.len() != w * h * 4 || w.max(h) * SCALE > 4096 {
        return None;
    }
    let (bw, bh) = (w * SCALE, h * SCALE);
    let big = scalefx::scalefx3x(w, h, rgba);
    let filled = fill_transparent(bw, bh, &big);
    let mut big = smaa::antialias(bw, bh, &big, &filled);
    boost_blacks(&mut big);
    Some(Base { w: bw, h: bh, rgba: big })
}

/// `base` reduced to (`dw`, `dh`) (premultiplied Lanczos-3), outline tightened: one level.
pub fn at_size(base: &Base, dw: usize, dh: usize) -> Vec<(u32, u32, Vec<u8>)> {
    let pm: Vec<[f32; 4]> = base
        .rgba
        .chunks_exact(4)
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
        })
        .collect();
    let small = if (dw, dh) == (base.w, base.h) { pm } else { resample(base.w, base.h, &pm, dw, dh) };
    let out = small
        .iter()
        .flat_map(|p| {
            let a = p[3].clamp(0.0, 1.0);
            if a <= 0.0 {
                return [0, 0, 0, 0];
            }
            let c = |v: f32| ((v.clamp(0.0, a) / a) * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            let at = ((a - 0.5) * ALPHA_CONTRAST + 0.5).clamp(0.0, 1.0);
            [c(p[0]), c(p[1]), c(p[2]), (at * 255.0 + 0.5) as u8]
        })
        .collect();
    vec![(dw as u32, dh as u32, out)]
}

/// Rows 0..h computed on up to 4 threads, in order (each call of `f` makes one row).
pub(crate) fn par_rows<T: Send>(h: usize, f: impl Fn(usize) -> Vec<T> + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(4).min(h.max(1));
    if threads <= 1 || h < 16 {
        return (0..h).flat_map(&f).collect();
    }
    let chunk = h.div_ceil(threads);
    let f = &f;
    let priority = openxiii_package::background::current();
    std::thread::scope(|s| {
        let parts: Vec<_> = (0..threads)
            .map(|t| {
                let math = openxiii_math::Mode::current();
                s.spawn(move || {
                    let _math = math.enter();
                    openxiii_package::background::take(priority);
                    (t * chunk..((t + 1) * chunk).min(h)).flat_map(f).collect::<Vec<T>>()
                })
            })
            .collect();
        parts.into_iter().flat_map(|p| p.join().expect("onomatopoeia thread")).collect()
    })
}

/// Colour of the texels of a straight-alpha RGBA8 image, the transparent ones (alpha 0) filled
/// with the average of their visible neighbours, grown a few texels: filters that read the
/// colour then bring no hidden background colour into the outline.
pub(crate) fn fill_transparent(w: usize, h: usize, rgba: &[u8]) -> Vec<[f32; 3]> {
    let mut rgb: Vec<[f32; 3]> = rgba.chunks_exact(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
    let mut known: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] > 0).collect();
    // The texels still to fill; each pass reads the state before it and writes after it, as a
    // pass over copies of the whole picture does.
    let mut todo: Vec<usize> = (0..w * h).filter(|&i| !known[i]).collect();
    for _ in 0..8 {
        let mut fresh: Vec<(usize, [f32; 3])> = Vec::new();
        let mut rest: Vec<usize> = Vec::with_capacity(todo.len());
        for &i in &todo {
            let (x, y) = (i % w, i / w);
            let mut sum = [0f32; 3];
            let mut n = 0.0;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if known[j] {
                    for c in 0..3 {
                        sum[c] += rgb[j][c];
                    }
                    n += 1.0;
                }
            }
            if n > 0.0 {
                fresh.push((i, sum.map(|v| v / n)));
            } else {
                rest.push(i);
            }
        }
        if fresh.is_empty() {
            break;
        }
        for (i, c) in fresh {
            rgb[i] = c;
            known[i] = true;
        }
        todo = rest;
    }
    rgb
}

/// `fill_transparent` as passes over the whole picture (the tests check the list of texels to
/// fill gives the same colours).
#[cfg(test)]
fn fill_transparent_ref(w: usize, h: usize, rgba: &[u8]) -> Vec<[f32; 3]> {
    let mut rgb: Vec<[f32; 3]> = rgba.chunks_exact(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
    let mut known: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] > 0).collect();
    for _ in 0..8 {
        let (prev_rgb, prev_known) = (rgb.clone(), known.clone());
        let mut changed = false;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if prev_known[i] {
                    continue;
                }
                let mut sum = [0f32; 3];
                let mut n = 0.0;
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if prev_known[j] {
                        for c in 0..3 {
                            sum[c] += prev_rgb[j][c];
                        }
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    rgb[i] = sum.map(|v| v / n);
                    known[i] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    rgb
}

/// Blacks boosted on a straight-alpha RGBA8 image (dark grey outlines become black). The
/// brightness is the brightest channel, so saturated colours (dark in luminance) stay.
fn boost_blacks(rgba: &mut [u8]) {
    for p in rgba.chunks_exact_mut(4) {
        let l = p[0].max(p[1]).max(p[2]) as f32 / 255.0;
        if l >= BLACK_HIGH {
            continue;
        }
        let t = ((l - BLACK_LOW) / (BLACK_HIGH - BLACK_LOW)).clamp(0.0, 1.0);
        let k = t * t * (3.0 - 2.0 * t);
        for c in &mut p[..3] {
            *c = (*c as f32 * k + 0.5) as u8;
        }
    }
}

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-6 {
        1.0
    } else if x < 3.0 {
        let px = std::f32::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    } else {
        0.0
    }
}

/// Weights of a Lanczos-3 resampling of `n` texels to `m` (widened when reducing, so it
/// averages over the reduced area): first source texel (may be outside, clamped when read) and
/// the weights from it.
fn taps(n: usize, m: usize) -> Vec<(i64, Vec<f32>)> {
    let ratio = n as f32 / m as f32;
    let support = 3.0 * ratio.max(1.0);
    let scale = 1.0 / ratio.max(1.0);
    (0..m)
        .map(|d| {
            let c = (d as f32 + 0.5) * ratio - 0.5;
            let first = (c - support).floor() as i64;
            let last = (c + support).ceil() as i64;
            let mut w: Vec<f32> = (first..=last).map(|s| lanczos3((s as f32 - c) * scale)).collect();
            let sum: f32 = w.iter().sum();
            w.iter_mut().for_each(|v| *v /= sum);
            (first, w)
        })
        .collect()
}

/// Lanczos-3 resampling of premultiplied RGBA from (w, h) to (dw, dh), edges clamped.
fn resample(w: usize, h: usize, img: &[[f32; 4]], dw: usize, dh: usize) -> Vec<[f32; 4]> {
    let (tx, ty) = (taps(w, dw), taps(h, dh));
    let at = |i: i64, n: usize| i.clamp(0, n as i64 - 1) as usize;
    let mid: Vec<[f32; 4]> = par_rows(h, |y| {
        tx.iter()
            .map(|(first, wt)| {
                let mut acc = [0f32; 4];
                for (k, wk) in wt.iter().enumerate() {
                    let p = img[y * w + at(first + k as i64, w)];
                    for c in 0..4 {
                        acc[c] += p[c] * wk;
                    }
                }
                acc
            })
            .collect()
    });
    par_rows(dh, |y| {
        let (first, wt) = &ty[y];
        (0..dw)
            .map(|x| {
                let mut acc = [0f32; 4];
                for (k, wk) in wt.iter().enumerate() {
                    let p = mid[at(first + k as i64, h) * dw + x];
                    for c in 0..4 {
                        acc[c] += p[c] * wk;
                    }
                }
                acc
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With OXIII_GAME: the filling by lists of texels gives the same colours, to the bit, as
    /// passes over the whole picture, on every redrawn picture.
    #[test]
    fn filling_is_the_same_as_full_passes() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME") else { return };
        let mut library = openxiii_package::library::Library::new(std::path::Path::new(&game)).unwrap();
        let loaded = library.get("XIIIMenuStart").unwrap();
        let mut seen = 0;
        for i in 0..loaded.package.exports.len() {
            if loaded.package.export_class(i) != "Texture" {
                continue;
            }
            let path = format!("XIIIMenuStart.{}", loaded.package.object_path(openxiii_package::ObjectRef::from((i + 1) as i32)));
            if !is_onomatopoeia(&path) {
                continue;
            }
            let texture = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, i).unwrap();
            let (w, h, rgba) = texture.d3d_mips().into_iter().next().unwrap();
            let (bw, bh) = (w as usize * SCALE, h as usize * SCALE);
            let big = scalefx::scalefx3x(w as usize, h as usize, &rgba);
            let (new, old) = (fill_transparent(bw, bh, &big), fill_transparent_ref(bw, bh, &big));
            assert!(old.iter().zip(&new).all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits)), "{path}: not the same filling");
            seen += 1;
        }
        assert!(seen >= 10, "{seen} onomatopoeias");
    }

    /// With OXIII_GAME: the 15 pictures read from the game and redrawn, with the time it takes.
    #[test]
    fn redraw_from_the_game() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME") else { return };
        let t0 = std::time::Instant::now();
        let all = make_all(std::path::Path::new(&game));
        let bytes: usize = all.values().map(|b| b.rgba.len()).sum();
        eprintln!("{} pictures in {:.0} ms, {:.1} MB", all.len(), t0.elapsed().as_secs_f64() * 1000.0, bytes as f64 / 1e6);
        assert_eq!(all.len(), NAMES.len() + 1, "{:?}", all.keys().collect::<Vec<_>>());
    }

    #[test]
    fn onomatopoeia_names() {
        assert!(is_onomatopoeia("XIIIMenuStart.Controle_PC.Bang"));
        assert!(is_onomatopoeia("xiiimenustart.interface_home.newgameWoowoo"));
        assert!(is_onomatopoeia("XIIIMenuStart.optionSonBuzzz"));
        assert!(!is_onomatopoeia("XIIIMenuStart.Profile.Profilefond1"));
        assert!(is_onomatopoeia("XIIICine.effets.Bang"));
        assert!(!is_onomatopoeia("XIIICine.effets.Ta"));
        assert!(!is_onomatopoeia("XIIIMenu.HUD.Bang"));
    }

    #[test]
    fn black_boost() {
        let mut px = [40u8, 40, 40, 200, 70, 70, 70, 255, 200, 50, 50, 128];
        boost_blacks(&mut px);
        assert!(px[0] < 15 && px[3] == 200, "{px:?}");
        assert!(px[4] < 70 && px[4] > 30, "{px:?}");
        assert_eq!(&px[8..], &[200, 50, 50, 128]);
    }

    #[test]
    fn flat_picture_stays_flat_at_screen_size() {
        let rgba: Vec<u8> = std::iter::repeat([200u8, 100, 50, 255]).take(16 * 8).flatten().collect();
        let base = redraw(16, 8, &rgba).unwrap();
        assert_eq!((base.w, base.h), (48, 24));
        let m = at_size(&base, 21, 11);
        assert_eq!((m.len(), m[0].0, m[0].1), (1, 21, 11));
        for p in m[0].2.chunks(4) {
            assert!(p[0].abs_diff(200) <= 1 && p[1].abs_diff(100) <= 1 && p[2].abs_diff(50) <= 1 && p[3] == 255, "{p:?}");
        }
    }

    #[test]
    fn smooth_outline_without_dark_halo() {
        // White opaque disc on a fully transparent black background, shown 1.5 times its size.
        let n = 16i32;
        let mut rgba = vec![0u8; (n * n * 4) as usize];
        for y in 0..n {
            for x in 0..n {
                if (x - 8) * (x - 8) + (y - 8) * (y - 8) <= 30 {
                    rgba[((y * n + x) * 4) as usize..((y * n + x) * 4 + 4) as usize].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        let base = redraw(16, 16, &rgba).unwrap();
        let m = at_size(&base, 24, 24);
        let mut soft = 0;
        for p in m[0].2.chunks(4) {
            if p[3] > 0 {
                assert!(p[0] >= 240 && p[1] >= 240 && p[2] >= 240, "{p:?}");
                soft += usize::from(p[3] < 255);
            }
        }
        assert!(soft > 0, "no smoothed texel on the outline");
    }
}
