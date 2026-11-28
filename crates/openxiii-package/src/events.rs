//! Kinds: file opened / read, package read, object looked up, texture read / decoded / uploaded,
//! sound decoded, GPU resource created (wgpu-hal hook), markers. Kept in memory, written by
//! `finish` at the end of the run. Off (always in release builds): one atomic read per event
//! site, nothing else.

mod imp {
    pub fn init() -> bool {
        false
    }
    #[inline(always)]
    pub fn active() -> bool {
        false
    }
    #[inline(always)]
    pub fn log(_kind: &'static str, _name: impl FnOnce() -> String, _detail: impl FnOnce() -> String) {}
    pub fn set_frame(_n: u32) {}
    pub fn first_normal_frame() {}
    pub fn loading_again(_map: &str) {}
    pub fn window_frame(_n: u32) -> bool {
        false
    }
    pub fn finish() {}
}

pub use imp::*;

/// A file read whole: "file_read" with its size.
pub fn file_read(path: &std::path::Path, bytes: usize) {
    if active() {
        log("file_read", || path.display().to_string(), || format!("pos=0 asked={bytes} read={bytes}"));
    }
}

/// A file opened for streaming: "file_open".
pub fn file_open(path: &std::path::Path, offset: u64) {
    if active() {
        log("file_open", || path.display().to_string(), || format!("offset={offset}"));
    }
}

/// Reader that reports each read ("file_read", name given at creation): music and voice streams.
pub struct LoggedReader<R> {
    inner: R,
    name: String,
    pos: u64,
}

impl<R> LoggedReader<R> {
    pub fn new(inner: R, name: String, pos: u64) -> Self {
        Self { inner, name, pos }
    }
}

impl<R: std::io::Seek> std::io::Seek for LoggedReader<R> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let r = self.inner.seek(to);
        if let Ok(p) = r {
            self.pos = p;
        }
        r
    }
}

impl<R: std::io::Read> std::io::Read for LoggedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let r = self.inner.read(buf);
        if let Ok(k) = r {
            if active() {
                let pos = self.pos;
                log("file_read", || self.name.clone(), || format!("pos={pos} asked={} read={k}", buf.len()));
            }
            self.pos += k as u64;
        }
        r
    }
}
