//! Improved build only: still frames of the game's videos for the added menu pictures (Mods box,
//! Co-op box, Select level background).
//!
//! The videos have a single key frame (frame 0), so a frame deep in a video costs decoding every
//! frame before it (up to 2,001 for Cine14). That work is done once, off the main thread: ffmpeg
//! decodes up to the frame and keeps it with its `trim` filter (always built into the ffmpeg
//! program, so the small bundled ffmpeg and the Android one have it), in YUV 4:2:0; the
//! conversion to RGBA is done here, so every platform gets the same pixels. The frame is then
//! kept in memory and in the cache folder next to OpenXIII's settings (never in the game folder),
//! checked on later runs against the video's size and modification time and the format version.
//!
//! `get` never blocks: it returns None until the frame is ready, and the menus show a
//! placeholder until then.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Version of the cache file format and of the conversion (a change makes the files again).
const VERSION: u32 = 2;

/// A decoded frame: width, height, RGBA rows from the top.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Where a frame came from and how long it took (made or read back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Decoded,
    Cache,
}

#[derive(Default)]
enum Slot {
    #[default]
    Pending,
    Done(Option<Arc<Frame>>, Source, Duration),
}

#[derive(Default)]
struct State {
    cache_dir: Option<PathBuf>,
    slots: HashMap<(PathBuf, u32), Slot>,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static S: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Folder where frames are kept between runs (None: made again each run).
pub fn set_cache_dir(dir: Option<PathBuf>) {
    if let Ok(mut s) = state().0.lock() {
        s.cache_dir = dir;
    }
}

/// Starts making frame `index` of `video` in the background (nothing if it is already made or
/// being made).
pub fn request(video: &Path, index: u32) {
    let key = (video.to_path_buf(), index);
    let cache_dir = {
        let Ok(mut s) = state().0.lock() else { return };
        if s.slots.contains_key(&key) {
            return;
        }
        s.slots.insert(key.clone(), Slot::Pending);
        s.cache_dir.clone()
    };
    let spawned = std::thread::Builder::new().name("still-frame".into()).spawn(move || {
        openxiii_package::background::lower_this_thread();
        let started = Instant::now();
        let (frame, source) = make(&key.0, key.1, cache_dir.as_deref());
        let (lock, ready) = state();
        if let Ok(mut s) = lock.lock() {
            s.slots.insert(key, Slot::Done(frame.map(Arc::new), source, started.elapsed()));
        }
        ready.notify_all();
    });
    if spawned.is_err() {
        if let Ok(mut s) = state().0.lock() {
            s.slots.remove(&(video.to_path_buf(), index));
        }
    }
}

/// Frame `index` of `video` if it is ready (starting it otherwise). Never blocks.
pub fn get(video: &Path, index: u32) -> Option<Arc<Frame>> {
    request(video, index);
    let s = state().0.lock().ok()?;
    match s.slots.get(&(video.to_path_buf(), index)) {
        Some(Slot::Done(f, ..)) => f.clone(),
        _ => None,
    }
}

/// True once frame `index` of `video` is made (found or not).
pub fn is_done(video: &Path, index: u32) -> bool {
    state().0.lock().is_ok_and(|s| matches!(s.slots.get(&(video.to_path_buf(), index)), Some(Slot::Done(..))))
}

/// Waits for frame `index` of `video` at most `timeout` (captures and tests).
pub fn wait(video: &Path, index: u32, timeout: Duration) -> Option<Arc<Frame>> {
    request(video, index);
    let (lock, ready) = state();
    let key = (video.to_path_buf(), index);
    let end = Instant::now() + timeout;
    let mut s = lock.lock().ok()?;
    loop {
        if let Some(Slot::Done(f, ..)) = s.slots.get(&key) {
            return f.clone();
        }
        let left = end.checked_duration_since(Instant::now())?;
        s = ready.wait_timeout(s, left).ok()?.0;
    }
}

/// How frame `index` of `video` was obtained and how long it took (once it is done).
pub fn timing(video: &Path, index: u32) -> Option<(Source, Duration, bool)> {
    let s = state().0.lock().ok()?;
    match s.slots.get(&(video.to_path_buf(), index))? {
        Slot::Done(f, source, d) => Some((*source, *d, f.is_some())),
        Slot::Pending => None,
    }
}

/// Identity of the source video: size and modification time (seconds, nanoseconds).
fn stamp(video: &Path) -> Option<(u64, u64, u32)> {
    let m = std::fs::metadata(video).ok()?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((m.len(), t.as_secs(), t.subsec_nanos()))
}

/// True when frame `index` of `video` is in the cache folder, up to date with the video.
pub fn kept(video: &Path, index: u32) -> bool {
    let dir = state().0.lock().ok().and_then(|s| s.cache_dir.clone());
    match (dir, stamp(video)) {
        (Some(dir), Some(stamp)) => cache_read(&cache_file(&dir, video, index), stamp, index).is_some(),
        _ => false,
    }
}

fn cache_file(dir: &Path, video: &Path, index: u32) -> PathBuf {
    let stem = video.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    dir.join(format!("frame-{stem}-{index}.bin"))
}

fn make(video: &Path, index: u32, cache_dir: Option<&Path>) -> (Option<Frame>, Source) {
    let Some(stamp) = stamp(video) else { return (None, Source::Decoded) };
    let file = cache_dir.map(|d| cache_file(d, video, index));
    if let Some(f) = file.as_deref().and_then(|f| cache_read(f, stamp, index)) {
        return (Some(f), Source::Cache);
    }
    let frame = decode(video, index);
    if let (Some(path), Some(f)) = (file.as_deref(), frame.as_ref()) {
        cache_write(path, stamp, index, f);
    }
    (frame, Source::Decoded)
}

/// "OXVF", version, video size (u64), modification time (u64 seconds, u32 nanoseconds), frame
/// index, width, height (u32 little-endian), then RGBA.
const HEADER: usize = 4 + 4 + 8 + 8 + 4 + 4 + 4 + 4;

fn cache_read(path: &Path, (len, secs, nanos): (u64, u64, u32), index: u32) -> Option<Frame> {
    let data = std::fs::read(path).ok()?;
    let u32_at = |o: usize| data.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let u64_at = |o: usize| data.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    let ok = data.get(..4)? == b"OXVF" && u32_at(4)? == VERSION && u64_at(8)? == len && u64_at(16)? == secs && u32_at(24)? == nanos && u32_at(28)? == index;
    if !ok {
        return None;
    }
    let (w, h) = (u32_at(32)?, u32_at(36)?);
    let rgba = crate::select_level_art::unpack(data.get(HEADER..)?, w as usize * h as usize * 4)?;
    (w > 0).then_some(Frame { w, h, rgba })
}

fn cache_write(path: &Path, (len, secs, nanos): (u64, u64, u32), index: u32, f: &Frame) {
    let mut data = Vec::with_capacity(HEADER + f.rgba.len());
    data.extend_from_slice(b"OXVF");
    data.extend_from_slice(&VERSION.to_le_bytes());
    data.extend_from_slice(&len.to_le_bytes());
    data.extend_from_slice(&secs.to_le_bytes());
    data.extend_from_slice(&nanos.to_le_bytes());
    for v in [index, f.w, f.h] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    data.extend_from_slice(&crate::select_level_art::pack(&f.rgba));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Frame `index` decoded by ffmpeg (YUV 4:2:0), converted to RGBA.
fn decode(video: &Path, index: u32) -> Option<Frame> {
    let header = crate::video::BinkHeader::read(&read_head(video)?)?;
    if index >= header.frames || header.width == 0 || header.height == 0 {
        return None;
    }
    let trim = format!("trim=start_frame={index}");
    let mut command = crate::video::command(&[
        "-v",
        "error",
        "-i",
        video.to_str()?,
        "-map",
        "0:v:0",
        "-vf",
        &trim,
        "-frames:v",
        "1",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "yuv420p",
        "-",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW and BELOW_NORMAL_PRIORITY_CLASS: decoded in the time the game leaves.
        command.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    let out = command.stdout(std::process::Stdio::piped()).output().ok()?;
    yuv420_to_rgba(header.width, header.height, &out.stdout)
}

/// The start of a video file, enough for its header and the tracks' table.
fn read_head(video: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(video).ok()?.take(4096).read_to_end(&mut head).ok()?;
    Some(head)
}

/// Planar YUV 4:2:0 (BT.601, limited range: Bink's colours) to RGBA; each chroma sample covers
/// a 2 × 2 block. None when `yuv` is shorter than one frame.
pub fn yuv420_to_rgba(w: u32, h: u32, yuv: &[u8]) -> Option<Frame> {
    let (w, h) = (w as usize, h as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let (ys, cs) = (w * h, cw * ch);
    if yuv.len() < ys + 2 * cs {
        return None;
    }
    let (y_plane, u_plane, v_plane) = (&yuv[..ys], &yuv[ys..ys + cs], &yuv[ys + cs..ys + 2 * cs]);
    let mut rgba = vec![255u8; ys * 4];
    // Fixed point, 16 fractional bits: R = 1.164 (Y − 16) + 1.596 (V − 128), G = 1.164 (Y − 16)
    // − 0.392 (U − 128) − 0.813 (V − 128), B = 1.164 (Y − 16) + 2.017 (U − 128).
    const K: i32 = 76309;
    let clamp = |v: i32| ((v + 32768) >> 16).clamp(0, 255) as u8;
    for y in 0..h {
        for x in 0..w {
            let c = (y / 2) * cw + x / 2;
            let l = K * (y_plane[y * w + x] as i32 - 16);
            let (u, v) = (u_plane[c] as i32 - 128, v_plane[c] as i32 - 128);
            let o = (y * w + x) * 4;
            rgba[o] = clamp(l + 104597 * v);
            rgba[o + 1] = clamp(l - 25675 * u - 53279 * v);
            rgba[o + 2] = clamp(l + 132201 * u);
        }
    }
    Some(Frame { w: w as u32, h: h as u32, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_and_primaries_convert_as_bt601() {
        // 2 × 2: one chroma sample. Y 16 / 235 with neutral chroma = black / white.
        let f = yuv420_to_rgba(2, 2, &[16, 235, 126, 16, 128, 128]).unwrap();
        assert_eq!(&f.rgba[0..4], &[0, 0, 0, 255]);
        assert_eq!(&f.rgba[4..8], &[255, 255, 255, 255]);
        assert_eq!(&f.rgba[8..11], &[128, 128, 128]);
        // Pure red in BT.601 limited range: Y 81, U 90, V 240.
        let r = yuv420_to_rgba(2, 2, &[81, 81, 81, 81, 90, 240]).unwrap();
        assert!(r.rgba[0] >= 253 && r.rgba[1] <= 2 && r.rgba[2] <= 2, "{:?}", &r.rgba[..4]);
        // Too short: nothing.
        assert!(yuv420_to_rgba(2, 2, &[0; 5]).is_none());
        // Odd sizes round the chroma planes up.
        assert!(yuv420_to_rgba(3, 3, &[0; 9 + 4 + 4]).is_some());
    }

    #[test]
    fn cache_file_is_checked_against_the_video() {
        let dir = std::env::temp_dir().join(format!("oxiii-still-{}", std::process::id()));
        let video = dir.join("Clip.bik");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&video, b"not a video").unwrap();
        let stamp = stamp(&video).unwrap();
        let path = cache_file(&dir, &video, 7);
        let f = Frame { w: 2, h: 1, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        cache_write(&path, stamp, 7, &f);
        assert_eq!(cache_read(&path, stamp, 7), Some(Frame { w: 2, h: 1, rgba: f.rgba.clone() }));
        // Another frame index, another size or date, another version: not used.
        assert!(cache_read(&path, stamp, 8).is_none());
        assert!(cache_read(&path, (stamp.0 + 1, stamp.1, stamp.2), 7).is_none());
        assert!(cache_read(&path, (stamp.0, stamp.1 + 1, stamp.2), 7).is_none());
        let mut data = std::fs::read(&path).unwrap();
        data[4] ^= 0xFF;
        std::fs::write(&path, &data).unwrap();
        assert!(cache_read(&path, stamp, 7).is_none());
        // A video that is not a Bink file: done, no frame, and nothing waits for it.
        assert!(wait(&video, 7, Duration::from_secs(5)).is_none());
        assert!(is_done(&video, 7));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the player's videos (OXIII_GAME): the three menu frames are made in the background,
    /// then read back from the cache; timings printed. Skipped without the game or ffmpeg.
    #[test]
    fn menu_frames_cold_then_cached() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from) else { return };
        let videos = game.join("Video");
        let wanted =
            [("Cine01", crate::menu_boxes::MODS_FRAME), ("Cine09", crate::menu_boxes::COOP_FRAME), ("Cine14", crate::select_level_art::BACKGROUND_FRAME)];
        if wanted.iter().any(|(f, _)| !videos.join(format!("{f}.bik")).is_file()) {
            return;
        }
        // `make` directly (the shared slots and cache folder are left to the other tests, which
        // ask for the same frames at the same time): the three in parallel, then from the cache.
        let cache = std::env::temp_dir().join(format!("oxiii-still-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let run = |label: &str| {
            let started = Instant::now();
            let out: Vec<_> = std::thread::scope(|scope| {
                let jobs: Vec<_> = wanted
                    .iter()
                    .map(|(f, n)| {
                        let (video, cache) = (videos.join(format!("{f}.bik")), cache.clone());
                        scope.spawn(move || {
                            let t = Instant::now();
                            let (frame, source) = make(&video, *n, Some(&cache));
                            (frame, source, t.elapsed())
                        })
                    })
                    .collect();
                jobs.into_iter().map(|j| j.join().unwrap()).collect()
            });
            for ((f, n), (_, source, d)) in wanted.iter().zip(&out) {
                eprintln!("{f} frame {n}: {source:?} in {} ms", d.as_millis());
            }
            eprintln!("all three, in parallel ({label}): {} ms", started.elapsed().as_millis());
            out
        };
        let cold = run("cold");
        if cold.iter().any(|(f, ..)| f.is_none()) {
            eprintln!("no ffmpeg: nothing to check");
            let _ = std::fs::remove_dir_all(&cache);
            return;
        }
        assert!(cold.iter().all(|(_, s, _)| *s == Source::Decoded));
        let cached = run("cached");
        for (c, k) in cold.iter().zip(&cached) {
            assert_eq!(k.1, Source::Cache);
            assert_eq!(k.0, c.0);
        }
        let _ = std::fs::remove_dir_all(&cache);
    }
}
