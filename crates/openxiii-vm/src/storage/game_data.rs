//! Data files recorded from, or derived from, the original game (crates/openxiii-vm/game_data):
//! the original's object index order, the Select level table and the texture and shader prewarm
//! lists.
//!
//! A file `rel` (for example `object_order/sounds_pass1.csv`) is looked up, in this order:
//! 1. `$OXIII_DATA/rel` when the OXIII_DATA environment variable names a folder;
//! 2. `data/rel` next to the executable;
//! 3. the copy embedded at build time (crates/openxiii-vm/game_data).
//! 4. development builds only: the source tree's copy (development runs and tests).
//!
//! Each file is read once and kept for the whole run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

include!(concat!(env!("OUT_DIR"), "/game_data.rs"));

fn candidates(rel: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = openxiii_package::env::var_os("OXIII_DATA").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(d).join(rel));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        out.push(dir.join("data").join(rel));
    }
    out
}

/// Contents of the optional data file `rel` (path relative to the data folder, with `/`), or None
/// when no copy is found.
pub fn file(rel: &str) -> Option<&'static str> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<&'static str>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = cache.get(rel) {
        return *v;
    }
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let found = candidates(rel)
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok().map(leak))
        .or_else(|| EMBEDDED.iter().find(|(n, _)| *n == rel).map(|(_, t)| *t));
    cache.insert(rel.to_string(), found);
    found
}

/// Like [`file`], for tests: None (with a message saying what is missing) lets the caller skip.
#[cfg(test)]
pub(crate) fn test_file(rel: &str) -> Option<&'static str> {
    let f = file(rel);
    if f.is_none() {
        eprintln!("skipped: optional game data `{rel}` not found (set OXIII_DATA to a folder that contains it)");
    }
    f
}
