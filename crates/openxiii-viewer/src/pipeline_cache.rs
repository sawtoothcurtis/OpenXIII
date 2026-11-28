//! The graphics driver's compiled pipelines kept between runs (Vulkan: Android phones, Linux).
//!
//! A level's prewarm creates a few hundred pipelines before its first frame. A phone's driver
//! compiles each of them again at every launch, a long black screen before the menu; with the
//! driver's cache read back from a file, it only finds them again. The other backends keep
//! their own caches (or none) and are left alone: nothing is opened there.
//!
//! The file lives in the cache folder next to the player's settings, named after the adapter;
//! wgpu and the driver check it against the adapter and the driver's version, and start from
//! an empty cache when it does not match.

use std::path::PathBuf;
use std::sync::Mutex;

struct Open {
    cache: wgpu::PipelineCache,
    file: PathBuf,
    /// Size of the data last written (a driver's cache only grows).
    saved: usize,
}

static FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);
static OPEN: Mutex<Option<Open>> = Mutex::new(None);

/// Folder of the cache file (the game a player starts); None: no cache is kept.
pub fn set_folder(folder: Option<PathBuf>) {
    if let Ok(mut f) = FOLDER.lock() {
        *f = folder;
    }
}

/// The device feature to ask for, when a cache is kept and the adapter has one.
pub fn wanted_features(adapter: &wgpu::Adapter) -> wgpu::Features {
    let kept = FOLDER.lock().is_ok_and(|f| f.is_some());
    if kept && wgpu::util::pipeline_cache_key(&adapter.get_info()).is_some() {
        adapter.features() & wgpu::Features::PIPELINE_CACHE
    } else {
        wgpu::Features::empty()
    }
}

/// Opens the cache of a new device from its file (an empty cache without one); the pipelines
/// created from now on use it.
pub fn open(device: &wgpu::Device, info: &wgpu::AdapterInfo) {
    let Ok(mut open) = OPEN.lock() else { return };
    *open = None;
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return;
    }
    let (Some(folder), Some(key)) = (FOLDER.lock().ok().and_then(|f| f.clone()), wgpu::util::pipeline_cache_key(info)) else { return };
    let file = folder.join(format!("{key}.bin"));
    let data = std::fs::read(&file).ok();
    // SAFETY: `data` is what `get_data` returned on an earlier run, written whole (temporary
    // file, then renamed); wgpu and the driver validate its header against this adapter and
    // fall back to an empty cache.
    let cache = unsafe { device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("pipelines"), data: data.as_deref(), fallback: true }) };
    crate::renderer::diag(&format!("pipeline cache: {} bytes read", data.as_ref().map_or(0, Vec::len)));
    *open = Some(Open { cache, file, saved: data.map_or(0, |d| d.len()) });
}

/// The open cache, for a pipeline descriptor.
pub fn get() -> Option<wgpu::PipelineCache> {
    OPEN.lock().ok()?.as_ref().map(|o| o.cache.clone())
}

/// Writes the cache to its file when it has grown (after a batch of new pipelines), on
/// another thread.
pub fn save() {
    let Ok(mut open) = OPEN.lock() else { return };
    let Some(o) = open.as_mut() else { return };
    let Some(data) = o.cache.get_data().filter(|d| d.len() != o.saved) else { return };
    o.saved = data.len();
    let file = o.file.clone();
    let _ = std::thread::Builder::new().name("pipeline-cache".into()).spawn(move || {
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, &file).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    });
}
