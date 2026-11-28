//! When off (the normal case): one boolean read per load, nothing else. Times overlap
//! (reading a texture may cause a package to be read): each column is the time spent in
//! its stage, sub-stages included.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

pub const N: usize = 6;
/// Kinds of load (one "count" column and one "ms" column each).
pub const NAMES: [&str; N] = ["packages_read", "objects_searched", "textures_read", "textures_decoded", "textures_uploaded", "sounds_decoded"];
pub const PACKAGE: usize = 0;
pub const OBJECT: usize = 1;
pub const TEXTURE_READ: usize = 2;
pub const TEXTURE_DECODED: usize = 3;
pub const TEXTURE_SENT: usize = 4;
pub const SOUND: usize = 5;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static COUNT: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static NS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
/// Names of packages read from disk, in order, across all libraries.
pub static READ_PACKAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn read_packages_from(n: usize) -> (usize, Vec<String>) {
    READ_PACKAGES.lock().map(|v| (v.len(), v.get(n..).map(|s| s.to_vec()).unwrap_or_default())).unwrap_or((n, Vec::new()))
}

/// Details (kind, ms, name) of the loads since the last read, at most 256 per frame.
static DETAIL: Mutex<Vec<(usize, f32, String)>> = Mutex::new(Vec::new());

pub fn enable() {
    ACTIVE.store(true, Relaxed);
}

#[inline(always)]
pub fn active_value() -> bool {
    ACTIVE.load(Relaxed)
}

/// Timer for one load: counted when it ends (drop).
pub struct Timer {
    start_value: Option<(Instant, usize)>,
    name_value: Option<String>,
}

impl Timer {
    /// Sets the load's name (kept in the details).
    pub fn name_value(&mut self, f: impl FnOnce() -> String) {
        if self.start_value.is_some() {
            self.name_value = Some(f());
        }
    }
}

#[inline]
pub fn start_value(kind_index: usize) -> Timer {
    Timer { start_value: active_value().then(|| (Instant::now(), kind_index)), name_value: None }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some((t, k)) = self.start_value {
            let ns = t.elapsed().as_nanos() as u64;
            COUNT[k].fetch_add(1, Relaxed);
            NS[k].fetch_add(ns, Relaxed);
            if crate::events::active() {
                const KINDS: [&str; N] = ["package_read", "object_lookup", "texture_read", "texture_decoded", "texture_uploaded", "sound_decoded"];
                let name_value = self.name_value.clone().unwrap_or_default();
                crate::events::log(KINDS[k], || name_value, || format!("ms={:.3}", ns as f64 / 1e6));
            }
            // Object lookups: only the slow ones (0.05 ms and more) are named.
            if let Some(name_value) = self.name_value.take().filter(|_| k != OBJECT || ns >= 50_000) {
                if let Ok(mut d) = DETAIL.lock() {
                    if d.len() < 256 {
                        d.push((k, ns as f32 / 1e6, name_value));
                    }
                }
            }
        }
    }
}

/// Totals since start: (counts, nanoseconds).
pub fn read_from() -> ([u64; N], [u64; N]) {
    (std::array::from_fn(|k| COUNT[k].load(Relaxed)), std::array::from_fn(|k| NS[k].load(Relaxed)))
}

/// Details of the named loads since the previous call.
pub fn take_detail() -> Vec<(usize, f32, String)> {
    DETAIL.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
}
