//! Streamed waves (music, voices): HXAudio's 0x88-byte object (constructor,
//! play, messages, destructor) and its chain:
//!
//! Ubi ADPCM decoder → decoder buffer (2 × 4096 frames, filled
//! by the decoder thread) → marker reader (stops at the callback
//! positions, calls the end of stream) → ring (2 × 4096 frames, filled on each
//! engine update) → streaming DirectSound channel (buffer of two
//! halves of 4096 frames and one look-ahead half, served by the DirectSound thread on each pass: Sleep(20), 31.25 ms in game, see `ds8_period`).
//!
//! 4096 = SoundPCBufSize from XIII.ini; Sleep(20) of the DirectSound thread. At start, the channel plays three halves of silence before the stream.
//!
//! Specific messages:
//! - 0x3EB: end handler; 0x3EC GotoMarker(name); 0x3ED back to the start;
//! - 0x3EE / 0x3F2 SetNextNamedSync/TransMarker(name, handler);
//! - 0x3EF / 0x3F3 SetNextSync/TransMarker(handler);
//! - 0x3F0 / 0x3F4 SetNamedSync/TransMarkers(name, handler) (name prefix);
//! - 0x3F1 / 0x3F5 SetAllSync/TransMarkers(handler).
//!
//! "Sync" callbacks fire when the reader reaches the marker (for seamless chaining
//! through GotoMarker); "transition" callbacks go through a timer set to the time
//! the marker takes to become audible. "Next" callbacks are only used once.

pub use crate::diagnostics::ds8_latency;
use crate::diagnostics::mode_advance;
use crate::engine::{Engine, ObjId, Timer};
use crate::mem::Addr;
use openxiii_assets::ubi_stream::StreamDecoder;
use std::sync::{Arc, Mutex};

/// SoundPCBufSize: half a buffer, in frames. Set when streaming sound starts
/// (called, which also creates the DirectSound thread).
pub const HALF: usize = half_buffer(SOUND_PC_BUF_SIZE) as usize;

/// SoundPCBufSize setting from XIII.ini, 0x1000 by default.
pub const SOUND_PC_BUF_SIZE: u32 = 0x1000;

/// Requests of one decoder-buffer fill after which a stream that still gets nothing is left for
/// the next service (a sound file read normally needs one or two).
const MAX_REFILLS: usize = 4096;

/// Frames of a streamed wave: bytes / block alignment (16 bits, non-zero).
pub fn images_stream(bytes_value: u32, alignment: u16) -> u32 {
    bytes_value / alignment as u32
}

/// Global list of streamed waves: the wave goes to the head; its link (+0x3C) takes the old head only if it is not null
/// (otherwise it keeps what it had). The numbers are those of the waves, 0: empty list.
pub fn push_in_head(head_value: &mut u32, link: &mut u32, wave_value: u32) {
    if *head_value != 0 {
        *link = *head_value;
    }
    *head_value = wave_value;
}

/// CUUID written to a stream: its two 4-byte words, in order ('s byte swap is never enabled).
pub fn write_cuuid(c: openxiii_assets::hx::Cuuid) -> [u8; 8] {
    let mut o = [0u8; 8];
    o[..4].copy_from_slice(&c.0.to_le_bytes());
    o[4..].copy_from_slice(&c.1.to_le_bytes());
    o
}

/// Word of a variable type written to an archive: little-endian, swapped only if 's platform (+0x28) is 1,
/// which never happens on PC (its writer is neither called nor referenced).
pub fn write_word_type(word_value: u32, inverse: bool) -> [u8; 4] {
    if inverse { word_value.to_be_bytes() } else { word_value.to_le_bytes() }
}

/// Boolean variable written to an archive: the word of its type, then the byte of its value.
pub fn write_bool(word_type: u32, value_value: u8, inverse: bool) -> Vec<Vec<u8>> {
    vec![write_word_type(word_type, inverse).to_vec(), vec![value_value]]
}

/// 4-byte word read from an archive by a variable type (integer, flo, kind): little-endian,
/// swapped only if byte +0x2C is 1 (its writer is never called on PC).
pub fn read_word(bytes_value: [u8; 4], inverse: bool) -> u32 {
    if inverse { u32::from_be_bytes(bytes_value) } else { u32::from_le_bytes(bytes_value) }
}

/// Content of a fresh value created by its type (vtable+0 of the types): integer and flo at zero, character
/// at "x", CUUID and object at the null CUUID; kind 5 sets nothing.
pub fn initial_value(kind_value: u32, sub_type: u32) -> Option<Vec<u8>> {
    use crate::interp::{K_CUUID, K_OBJECT, K_SIMPLE, T_CHAR};
    match (kind_value, sub_type) {
        (K_SIMPLE, T_CHAR) => Some(b"x".to_vec()),
        (K_SIMPLE, _) => Some(vec![0; 4]),
        (K_CUUID, _) | (K_OBJECT, _) => Some(vec![0; 8]),
        _ => None,
    }
}

/// String written to an archive (value, variable, type name): 4-byte length
/// (strlen kept on 16 bits, swapped depending on the platform), then its bytes if any. A missing string counts as empty.
pub fn write_string(text_value: &[u8], inverse: bool) -> Vec<Vec<u8>> {
    let n = text_value.len() & 0xFFFF;
    let mut v = vec![write_word_type(n as u32, inverse).to_vec()];
    if n != 0 {
        v.push(text_value[..n].to_vec());
    }
    v
}

/// String value written by its type: kind and subtype of the type (then +0xC), then the string.
pub fn write_value_string(kind_value: u32, sub_type: u32, text_value: &[u8], inverse: bool) -> Vec<Vec<u8>> {
    let mut v = vec![write_word_type(kind_value, inverse).to_vec(), write_word_type(sub_type, inverse).to_vec()];
    v.extend(write_string(text_value, inverse));
    v
}

/// Named type written to an archive (kind 5, object): its kind, then its name (+0xC).
pub fn write_type_named(kind_value: u32, name_value: &[u8], inverse: bool) -> Vec<Vec<u8>> {
    let mut v = vec![write_word_type(kind_value, inverse).to_vec()];
    v.extend(write_string(name_value, inverse));
    v
}

/// CUUID variable written or object variable (the object's CUUID, null CUUID without an object): two words,
/// swapped depending on the platform. then writes the address of its value (+0x2C): a process pointer, meaningless outside
/// it and never read back (no object variable read takes it); it is not reproduced.
pub fn write_cuuid_variable(c: openxiii_assets::hx::Cuuid, inverse: bool) -> Vec<Vec<u8>> {
    vec![write_word_type(c.0, inverse).to_vec(), write_word_type(c.1, inverse).to_vec()]
}

/// Wave written as RIFF by CEdiWaveFile: "RIFF" size "WAVE", "fmt" chunk, "ump3" chunk if present (8 bytes), then
/// "data" (padding byte taken from the tag, "d", if the size is odd) or, without data, an 8-byte "datx" (+8 then +4).
/// No markers (+0x38 null, returns 0). RIFF size rounded up to even.
pub fn write_riff_wave(fmt: &[u8], ump3: Option<&[u8; 8]>, data_value: Option<&[u8]>, datx: (u32, u32)) -> Vec<Vec<u8>> {
    let mut size_value = fmt.len() as u32 + 0xC;
    if ump3.is_some() {
        size_value += 0x10;
    }
    size_value += match data_value {
        Some(d) => d.len() as u32 + 8,
        None => 0xC,
    };
    if size_value & 1 != 0 {
        size_value += 1;
    }
    let block_value = |label: &[u8; 4], n: u32| [label.as_slice(), &n.to_le_bytes()].concat();
    let mut v = vec![[b"RIFF".as_slice(), &size_value.to_le_bytes(), b"WAVE"].concat(), block_value(b"fmt ", fmt.len() as u32), fmt.to_vec()];
    if let Some(u) = ump3 {
        v.push(block_value(b"ump3", 8));
        v.push(u.to_vec());
    }
    match data_value {
        Some(d) => {
            v.push(block_value(b"data", d.len() as u32));
            v.push(d.to_vec());
            if d.len() & 1 != 0 {
                v.push(b"d".to_vec());
            }
        }
        None => {
            v.push(block_value(b"datx", 8));
            v.push(datx.0.to_le_bytes().to_vec());
            v.push(datx.1.to_le_bytes().to_vec());
        }
    }
    v
}

/// CEdiWaveFile::f_vbRead: opens the named file and reads the wave through the object's vtable+4. Returns success and the messages,
/// in order: ("%s is missing" or the second message, the error). Flag +0x25, which would silence the
/// first one, is never set (its writer is never called).
pub fn read_wave_file(name_value: &str, exists: bool, read_value: impl FnOnce() -> bool) -> (bool, Vec<(char, String)>) {
    if !exists {
        let error_value = format!("Error in CEdiWaveFile f_vbRead() : {name_value} can't be opened\r\n");
        return (false, vec![('A', format!("{name_value} is missing\r\n")), ('B', error_value)]);
    }
    if read_value() {
        return (true, Vec::new());
    }
    let error_value = format!("Error in CEdiWaveFile f_vbRead() : {name_value} can't be read from\r\n");
    (false, vec![('B', error_value.clone()), ('A', error_value)])
}

/// Window of a file (start = file position at construction) read: `size_value` × `count_value` bytes, clamped
/// to the whole elements that fit in the window, then to the file; the position only advances by whole elements read. A shared file
/// moved by another window is repositioned before reading (fseek).
#[derive(Debug, Clone, Copy)]
pub struct WindowFile {
    pub start_value: u32,
    pub length: u32,
    pub position: u32,
}

impl WindowFile {
    /// Elements read and their bytes.
    pub fn read_from<'a>(&mut self, file_value: &'a [u8], size_value: u32, count_value: u32) -> (u32, &'a [u8]) {
        let mut n = size_value.wrapping_mul(count_value);
        if n == 0 {
            return (0, &[]);
        }
        if self.position.wrapping_add(n) > self.length {
            let rest_value = self.length.wrapping_sub(self.position);
            n = rest_value - rest_value % size_value;
        }
        let from_time = (self.start_value as usize + self.position as usize).min(file_value.len());
        let available = (file_value.len() - from_time).min(n as usize) as u32;
        let elements = available / size_value;
        self.position += elements * size_value;
        (elements, &file_value[from_time..from_time + (elements * size_value) as usize])
    }
}

/// Half buffer set: the setting if it is a multiple of 256, otherwise rounded up to the next multiple of 256.
pub const fn half_buffer(setting: u32) -> u32 {
    if setting & 0xFF != 0 {
        (setting & !0xFF).wrapping_add(0x100)
    } else {
        setting
    }
}

/// PCM format passed to DirectSound by (16 bytes of WAVEFORMATEX before cbSize): tag 1, channels (low 16 bits), rate,
/// average byte rate = alignment × rate, alignment = (bits / 8) × channels (16 bits), bits.
pub fn format_pcm(channels_list: u32, frequency_value: u32, bits: u8) -> [u8; 16] {
    let alignment = ((bits as u32 >> 3) * (channels_list & 0xFFFF)) & 0xFFFF;
    let mut f = [0u8; 16];
    f[0..2].copy_from_slice(&1u16.to_le_bytes());
    f[2..4].copy_from_slice(&(channels_list as u16).to_le_bytes());
    f[4..8].copy_from_slice(&frequency_value.to_le_bytes());
    f[8..12].copy_from_slice(&alignment.wrapping_mul(frequency_value).to_le_bytes());
    f[12..14].copy_from_slice(&(alignment as u16).to_le_bytes());
    f[14..16].copy_from_slice(&(bits as u16).to_le_bytes());
    f
}
/// Decoder and ring buffers: 2 × 4096 frames.
pub const BUFFER: usize = 2 * HALF;
/// Period of the DirectSound thread, in seconds. Each pass ends with Sleep(20)
/// (20 ms: setting + 0x14 set). The game only raises the Windows timer
/// resolution for Core's calibration (timeBeginPeriod(1), then timeEndPeriod(1)
/// in the same function): in game it stays at 15.625 ms and Sleep(20) lasts two
/// ticks, 31.25 ms. This depends on the system: since Windows 10 2004, timeBeginPeriod only
/// applies to the calling process; before that, another program at 1 ms brought the pass down to
/// 20 ms.
pub const DS8_PERIOD_DEFAULT: f64 = 0.03125;

static DS8_PERIOD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn ds8_period() -> f64 {
    let b = DS8_PERIOD.load(std::sync::atomic::Ordering::Relaxed);
    if b != 0 {
        return f64::from_bits(b);
    }
    let p = DS8_PERIOD_DEFAULT;
    DS8_PERIOD.store(p.to_bits(), std::sync::atomic::Ordering::Relaxed);
    p
}

pub fn set_ds8_period(p: f64) {
    if p > 0.0 && crate::diagnostics::accept_recorded_ds8_period() {
        DS8_PERIOD.store(p.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Delay of the DirectSound play cursor after Play (IDirectSoundBuffer::Play on the
/// pass that starts the channel): the mixer only starts the cursor after about 32 ms (28 to 41
/// ms measured on voice buffers, median 32).
pub const DS8_LATENCY_DEFAULT: f64 = 0.032;

/// Ratio of real time to level time. HXAudio counts in timeGetTime (end of Update,
/// HXUpdate) and the DirectSound thread sleeps in real time, while the level advances on
/// the engine clock (appSeconds, calibrated at startup by Core): in the original, level time runs
/// 0.1 to 0.4 % slower than audio depending on the run (e.g. a 4096-byte music half at 32000 B/s
/// lasts 127.61 ms of level time). In game, audio and level follow the same clock here: 1. A
/// simulation can set another ratio (`set_clock`).
static CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn clock_value() -> f64 {
    let b = CLOCK.load(std::sync::atomic::Ordering::Relaxed);
    if b != 0 {
        return f64::from_bits(b);
    }
    let r = 1.0f64;
    CLOCK.store(r.to_bits(), std::sync::atomic::Ordering::Relaxed);
    r
}

pub fn set_clock(r: f64) {
    if r > 0.0 && crate::diagnostics::accept_recorded_clock_ratio() {
        CLOCK.store(r.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

/// Source of a streamed wave: file, position, channels, allowed duration ("ump3" chunk).
#[derive(Debug, Clone)]
pub struct StreamSource {
    pub path: std::path::PathBuf,
    pub offset: u64,
    pub channels: u16,
    /// Original WAVE format: 1 PCM, 2 Ubi ADPCM.
    pub format_tag: u16,
    pub data_size: u32,
    /// Bit 4 of the wave file: data source kept in memory.
    pub memory: bool,
    /// Archive format bits; the native format passed to the decoder keeps the low byte.
    pub bits: u16,
    pub total: u32,
}

/// Decoder of a wave: reads in 512-frame chunks bounded by the allowed
/// duration, seek, first-read adjustments. Bounded source
/// (read, seek) over file.
pub struct HxDecoder {
    inner: StreamDecoder,
    channels: usize,
    /// + 8: allowed frames; + 0xC: frames returned; + 0x28: first read.
    total: u32,
    decoded: u32,
    first: bool,
}

impl HxDecoder {
    pub fn open(src: &StreamSource) -> Option<HxDecoder> {
        // 0x6EFD0 -> 0x71630 refuses any format other than 16; callers
        // pass the bits as a byte.
        if src.bits as u8 != 16 {
            return None;
        }
        use std::io::{Seek, SeekFrom};
        let mut f = std::fs::File::open(&src.path).ok()?;
        f.seek(SeekFrom::Start(src.offset)).ok()?;
        openxiii_package::events::file_open(&src.path, src.offset);
        let f = openxiii_package::events::LoggedReader::new(f, src.path.display().to_string(), src.offset);
        let channels = src.channels.max(1) as usize;
        Some(HxDecoder {
            inner: StreamDecoder::new(Box::new(std::io::BufReader::with_capacity(0x8000, f)), channels),
            channels,
            total: src.total,
            decoded: 0,
            first: true,
        })
    }

    /// At most `frames` frames into `out`; (frames, end of stream).
    pub fn read(&mut self, out: &mut [i16], frames: usize) -> (usize, Option<bool>) {
        let ch = self.channels;
        let remaining = self.total.wrapping_sub(self.decoded) as usize;
        let n = frames.min(remaining);
        let mut got = 0;
        let mut status = None;
        loop {
            if n <= got {
                break;
            }
            let chunk = if got + 0x200 < n { 0x200 } else { n - got };
            let (r, k) = self.inner.read(&mut out[got * ch..(got + chunk) * ch], chunk);
            self.decoded = self.decoded.wrapping_add(k as u32);
            let done = r == 4 || self.decoded == self.total;
            status = Some(done);
            got += k;
            if done {
                break;
            }
            // Decoder stopped after a seek (+0x50 = 2): the original loops forever.
            if k == 0 {
                break;
            }
        }
        if self.first && got > 0xb3 {
            openxiii_assets::ubi_adpcm::tweaks_first_read(&mut out[..got * ch], ch, got);
        }
        self.first = false;
        (got, status)
    }

    /// + 0xC: frames returned (position in the stream).
    pub fn decoded(&self) -> u32 {
        self.decoded
    }

    pub fn seek(&mut self, frame: u32) {
        self.inner.seek(frame);
        self.decoded = self.inner.position();
        self.first = true;
    }
}

/// What the decoder buffer asks its source for (vtable + 0: read, + 8: seek, + 0xC
/// of object); tests replace it with a test source.
pub trait FrameSource {
    fn read(&mut self, out: &mut [i16], frames: usize) -> (usize, Option<bool>);
    fn decoded(&self) -> u32;
    fn seek(&mut self, frame: u32);
    /// Freeing this decoder leaves nothing for the next one (`left_header`): it is replaced by
    /// the look-ahead thread's decoder, which stands for it.
    fn forget_header(&mut self) {}
}

impl FrameSource for HxDecoder {
    fn read(&mut self, out: &mut [i16], frames: usize) -> (usize, Option<bool>) {
        HxDecoder::read(self, out, frames)
    }
    fn decoded(&self) -> u32 {
        HxDecoder::decoded(self)
    }
    fn seek(&mut self, frame: u32) {
        HxDecoder::seek(self, frame)
    }
    fn forget_header(&mut self) {
        self.inner.leaves_header = false;
    }
}

/// PCM source chosen by (reader).
/// The frame count comes from the bytes of the data chunk, not from the ump3 chunk.
struct PcmDecoder {
    file: std::fs::File,
    base: u64,
    align: u16,
    total: u32,
    decoded: u32,
    size: u32,
    memory: bool,
    /// Native reader position: updated by Seek and, in memory, by Read.
    source_pos: u32,
}

impl PcmDecoder {
    fn open(src: &StreamSource) -> Option<Self> {
        use std::io::{Seek, SeekFrom};
        let mut file = std::fs::File::open(&src.path).ok()?;
        file.seek(SeekFrom::Start(src.offset)).ok()?;
        openxiii_package::events::file_open(&src.path, src.offset);
        let align = ((src.bits as u32 >> 3) * src.channels as u32) as u16;
        Some(Self {
            file, base: src.offset, align, total: src.data_size / align as u32,
            decoded: 0, size: src.data_size, memory: src.memory, source_pos: 0,
        })
    }
}

impl FrameSource for PcmDecoder {
    fn read(&mut self, out: &mut [i16], frames: usize) -> (usize, Option<bool>) {
        use std::io::Read;
        let mut n = frames.min(self.total.wrapping_sub(self.decoded) as usize);
        if self.memory {
            // 0x707E0 only copies whole elements in the in-memory source.
            n = n.min(self.size.wrapping_sub(self.source_pos) as usize / self.align as usize);
        }
        let mut bytes = vec![0u8; n * self.align as usize];
        assert!(bytes.len() <= out.len() * 2);
        let mut read = 0;
        while read < bytes.len() {
            match self.file.read(&mut bytes[read..]) {
                Ok(0) => break,
                Ok(k) => {
                    openxiii_package::events::log("file_read", || format!("PCM stream @{}", self.base), || format!("asked={} read={k}", bytes.len() - read));
                    read += k
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        for (i, &byte) in bytes[..read].iter().enumerate() {
            let mut sample = out[i / 2].to_le_bytes();
            sample[i % 2] = byte;
            out[i / 2] = i16::from_le_bytes(sample);
        }
        if self.memory {
            self.source_pos = self.source_pos.wrapping_add(read as u32);
        }
        let got = read / self.align as usize;
        // 0x7066F: the source may report its end; otherwise +8 grows by the frames read.
        // 0x70C30 does not update +14 of the file source, unlike 0x707E0.
        self.decoded = if self.source_pos >= self.size {
            self.total
        } else {
            self.decoded.wrapping_add(got as u32)
        };
        (got, Some(self.decoded == self.total))
    }

    fn decoded(&self) -> u32 {
        self.decoded
    }

    fn seek(&mut self, frame: u32) {
        use std::io::{Seek, SeekFrom};
        let offset = frame.wrapping_mul(self.align as u32);
        // 0x707C0 accepts the bound in memory; 0x70CD4 refuses it for a file.
        let valid = (offset as i32) >= 0 && if self.memory { offset <= self.size } else { offset < self.size };
        if valid {
            if self.file.seek(SeekFrom::Start(self.base + offset as u64)).is_err() {
                self.decoded = self.total;
                return;
            }
            self.source_pos = offset;
        }
        // Even a refused position returns the old position and not -1: 0x706DF
        // then keeps the requested frame, without moving the underlying source.
        self.decoded = frame;
    }
}

/// Circular buffer of interleaved frames (start, end, read, write, fill), with the
/// stream position of each frame (tracking what is audible; u64::MAX: silence).
#[derive(Debug, Clone)]
pub struct Circular {
    pub buf: Vec<i16>,
    pub pos: Vec<u64>,
    pub channels: usize,
    /// In frames.
    pub read: usize,
    pub write: usize,
    pub fill: usize,
    /// End of the source reached (+0x1C of the decoder buffer, +0x20 of the ring).
    pub end: bool,
    /// Owning object destroyed: the service removes it (+0x1D / +0x21, set).
    pub closed: bool,
}

impl Circular {
    pub fn new(frames: usize, channels: usize) -> Circular {
        Circular { buf: vec![0; frames * channels], pos: vec![u64::MAX; frames], channels, read: 0, write: 0, fill: 0, end: false, closed: false }
    }
    pub fn capacity(&self) -> usize {
        self.buf.len() / self.channels.max(1)
    }
    /// End of one ring refill pass: `got` frames read (at most `chunk`,
    /// the space up to the end being `contig`) for a request of `n`, reader state `st` (1: end).
    /// Returns the next request when the write has reached the end of the buffer: it restarts at the beginning.
    pub fn ring_commit(&mut self, tmp: &[i16], tmp_pos: &[u64], got: usize, st: u8, n: usize, chunk: usize, contig: usize) -> Option<usize> {
        let ch = self.channels;
        let w = self.write;
        self.buf[w * ch..(w + got) * ch].copy_from_slice(&tmp[..got * ch]);
        self.pos[w..w + got].copy_from_slice(&tmp_pos[..got]);
        self.write += got;
        self.end = st == 1;
        self.fill += got;
        if !self.end && contig <= n && chunk == got && self.write == self.capacity() {
            self.write = 0;
            Some(n - got)
        } else {
            None
        }
    }

    /// Reads at most `n` frames.
    pub fn take(&mut self, out: &mut [i16], out_pos: &mut [u64], n: usize) -> usize {
        let n = n.min(self.fill);
        let ch = self.channels;
        let cap = self.capacity();
        for i in 0..n {
            let src = (self.read + i) % cap;
            out[i * ch..(i + 1) * ch].copy_from_slice(&self.buf[src * ch..(src + 1) * ch]);
            out_pos[i] = self.pos[src];
        }
        self.read = (self.read + n) % cap;
        self.fill -= n;
        n
    }
}

/// Marker reader (0x2C bytes): position, frames returned during the current
/// read, next marker watched.
#[derive(Debug, Clone, Default)]
pub struct Splitter {
    /// + 0x14.
    pub pos: u64,
    /// + 0x18: frames returned since the start of the current read.
    pub count: u64,
    /// + 0x20: next callback position (u64::MAX: none); + 0xC: callback armed.
    pub next: u64,
    pub armed: bool,
    /// + 0x10: source (decoder buffer) connected.
    pub source: bool,
    /// + 8: end-of-stream callback connected (the wave).
    pub end_cb: bool,
}

/// How late the game thread must be before the output feeds a channel by itself (seconds).
/// The engine's own update runs every 31.25 ms; a ring holds a quarter of a second.
pub const RESERVE_LATE: f64 = 0.1;

/// What lets the sound output keep a streamed channel fed while the game thread is held up.
///
/// In the original, the rings are refilled by HXAudio's own update thread (ParallelUpdateThread
///HXUpdate), every 31.25 ms whatever the game loop does:
/// a hitch of the game never starves a stream. Here the engine is updated by the game thread;
/// during a hitch longer than the ring, the channel would play silence. With a real sound
/// output only, when the ring runs short and the game thread has not served it for
/// `RESERVE_LATE`, the output reads the next frames itself with a decoder of its own, from
/// the reader's position and no further than the next marker whose handler runs when the
/// reader reaches it, or the end of the stream (what happens there belongs to the game thread:
/// handlers, loops, jumps; the channel is silent from there until it is back). Markers whose
/// handler only runs once they are audible may be passed: the game thread sets their timers
/// when it comes back. The game thread then skips the frames lent this way. Test runs
/// and runs without a sound output never have a reserve.
pub struct Reserve {
    source: StreamSource,
    decoder: Option<Box<dyn FrameSource + Send>>,
    /// Stream position of the frame that follows the last one given to the ring.
    pub next: u64,
    /// Position the output never reads past (next marker handled on reaching it, end of the
    /// stream).
    pub limit: u64,
    /// Last time the game thread served the ring.
    pub served: std::time::Instant,
    /// Frames the output took by itself since then.
    pub lent: u64,
    /// Times the output had to feed the channel by itself, and frames it read in all.
    pub activations: u64,
    pub frames: u64,
    /// The output is feeding the channel by itself (since the game thread's last service).
    active: bool,
    /// The game thread is back from a stall and the ring is not full again yet.
    recovering: bool,
}

impl Reserve {
    pub fn new(source: StreamSource, next: u64, limit: u64) -> Reserve {
        Reserve { source, decoder: None, next, limit, served: std::time::Instant::now(), lent: 0, activations: 0, frames: 0, active: false, recovering: false }
    }

    /// Up to `n` frames from `next` into `out` (interleaved) and their positions; frames read.
    /// Nothing while the game thread served the ring less than `late` seconds ago.
    pub fn take(&mut self, out: &mut [i16], out_pos: &mut [u64], n: usize, late: f64) -> usize {
        if n == 0 || self.next >= self.limit || self.served.elapsed().as_secs_f64() < late {
            return 0;
        }
        if self.decoder.is_none() {
            self.decoder = open_reserve_decoder(&self.source);
        }
        let Some(d) = self.decoder.as_mut() else { return 0 };
        if d.decoded() as u64 != self.next {
            d.seek(self.next as u32);
        }
        let want = n.min((self.limit - self.next) as usize);
        let ch = self.source.channels.max(1) as usize;
        let (got, _) = d.read(&mut out[..want * ch], want);
        for (i, p) in out_pos[..got].iter_mut().enumerate() {
            *p = self.next + i as u64;
        }
        self.next += got as u64;
        self.lent += got as u64;
        self.frames += got as u64;
        if got > 0 && !self.active {
            self.active = true;
            self.activations += 1;
        }
        got
    }
}

impl Clone for Reserve {
    /// A copy starts without a decoder: it opens its own on first use.
    fn clone(&self) -> Reserve {
        Reserve {
            source: self.source.clone(),
            decoder: None,
            next: self.next,
            limit: self.limit,
            served: self.served,
            lent: self.lent,
            activations: self.activations,
            frames: self.frames,
            active: self.active,
            recovering: self.recovering,
        }
    }
}

impl std::fmt::Debug for Reserve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reserve").field("next", &self.next).field("limit", &self.limit).field("lent", &self.lent).field("activations", &self.activations).finish()
    }
}

/// A decoder of the output's own on a stream's source. It is not one of the original's
/// decoders: freeing it leaves nothing behind (`forget_header`).
fn open_reserve_decoder(src: &StreamSource) -> Option<Box<dyn FrameSource + Send>> {
    match src.format_tag {
        1 => PcmDecoder::open(src).map(|d| Box::new(d) as Box<dyn FrameSource + Send>),
        2 => HxDecoder::open(src).map(|mut d| {
            FrameSource::forget_header(&mut d);
            Box::new(d) as Box<dyn FrameSource + Send>
        }),
        _ => None,
    }
}

/// Frames of a stream's source (where its decoder ends).
fn source_frames(src: &StreamSource) -> u64 {
    match src.format_tag {
        1 => {
            let align = ((src.bits as u32 >> 3) * src.channels as u32).max(1);
            (src.data_size / align) as u64
        }
        _ => src.total as u64,
    }
}

/// Frames a channel in follow mode passed over, for the real output (see [`Engine::stream_follow_rate`]).
#[derive(Debug, Clone)]
pub struct Follow {
    pub rate: u32,
    pub frames: std::collections::VecDeque<[f32; 2]>,
    /// Fraction of an output frame left from the last advance.
    carry: f64,
    /// Output side only (what the speakers get, never read by the game): read position between
    /// the first two frames, the output waiting for the frames to reach [`FOLLOW_TARGET`] again
    /// (at the start, or after running dry), and the gain of the short fade back in.
    out_pos: f64,
    starved: bool,
    played: bool,
    fade: f32,
    /// Frames kept ahead, smoothed (the reading speed follows it).
    level: f64,
}

/// Most frames a channel in follow mode keeps for the output (half a second at 48 kHz): older
/// ones are dropped rather than played late.
const FOLLOW_MAX: usize = 24_000;

/// Frames the output keeps ahead in follow mode (200 ms at 44.1 kHz): a hold of the game's steps
/// shorter than this (the other machine's commands a little late on a Wi-Fi link) is not heard,
/// since the steps that catch up afterwards give the missing frames back.
const FOLLOW_TARGET: usize = 8_820;

/// Times a follow-mode output ran dry, and frames it played as silence meanwhile.
static FOLLOW_DRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FOLLOW_SILENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Times a follow-mode output ran dry and frames played as silence since the last call (the
/// counters are taken).
pub fn follow_dry_take() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (FOLLOW_DRY.swap(0, Relaxed), FOLLOW_SILENT.swap(0, Relaxed))
}

fn follow_direct() -> bool {
    {
        false
    }
}

impl Follow {
    fn new(rate: u32) -> Follow {
        Follow { rate, frames: std::collections::VecDeque::new(), carry: 0.0, out_pos: 0.0, starved: true, played: false, fade: 1.0, level: 0.0 }
    }

    /// Real output of a follow-mode channel: `frames` frames read from the frames the game
    /// passed over, a little behind them. The reading speed bends (8 % slower, 5 % faster at most)
    /// to keep about [`FOLLOW_TARGET`] frames ahead; running dry, the output waits for that many
    /// again and fades back in. `draining`: the channel no longer produces (its tail is played
    /// without waiting). Output only: the game's cursor, halves and events are moved by
    /// `advance`, whatever is read here.
    fn render(&mut self, out: &mut Vec<[f32; 2]>, frames: usize, draining: bool) {
        if follow_direct() {
            for _ in 0..frames {
                let s = self.frames.pop_front();
                if s.is_none() && self.played && !draining {
                    FOLLOW_SILENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                self.played |= s.is_some();
                out.push(s.unwrap_or([0.0, 0.0]));
            }
            return;
        }
        let len = self.frames.len();
        // Far too much ahead (a long hold and its catching up): jump back near the target.
        if len > 3 * FOLLOW_TARGET {
            self.frames.drain(..len - FOLLOW_TARGET * 3 / 2);
            self.fade = 0.0;
        }
        let len = self.frames.len();
        // The reading speed follows the frames kept ahead, smoothed over about a third of a
        // second (the game gives them by frames, a slow machine's frames are long): a session
        // that runs a little slower than 60 steps a second (a slow machine) is played a little
        // slower, instead of running dry again and again.
        self.level = if self.played { self.level + (len as f64 - self.level) * 0.02 } else { len as f64 };
        let ratio = (1.0 + (self.level - FOLLOW_TARGET as f64) / FOLLOW_TARGET as f64 * 0.1).clamp(0.92, 1.05);
        for _ in 0..frames {
            if self.starved {
                // (Back after running dry: half the target is enough, the speed rebuilds the rest.)
                let enough = if self.played { FOLLOW_TARGET / 2 } else { FOLLOW_TARGET };
                if self.frames.len() >= enough || (draining && !self.frames.is_empty()) {
                    self.starved = false;
                    self.fade = if self.played { 0.0 } else { 1.0 };
                } else {
                    if self.played && !draining {
                        FOLLOW_SILENT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    out.push([0.0, 0.0]);
                    continue;
                }
            }
            if self.frames.len() < 2 {
                out.push(self.frames.pop_front().map_or([0.0, 0.0], |s| [s[0] * self.fade, s[1] * self.fade]));
                self.out_pos = 0.0;
                self.starved = true;
                if !draining {
                    FOLLOW_DRY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                continue;
            }
            let (a, b) = (self.frames[0], self.frames[1]);
            let t = self.out_pos as f32;
            out.push([(a[0] + (b[0] - a[0]) * t) * self.fade, (a[1] + (b[1] - a[1]) * t) * self.fade]);
            self.played = true;
            // 5 ms back to full volume after a dry spell or a jump.
            self.fade = (self.fade + 1.0 / 220.0).min(1.0);
            self.out_pos += ratio;
            while self.out_pos >= 1.0 && !self.frames.is_empty() {
                self.frames.pop_front();
                self.out_pos -= 1.0;
            }
        }
    }
}

/// Streaming DirectSound channel (CDS8Channel, vtable), served by the DirectSound thread:
/// 6 free, 1 start requested, 2 playing, 3 stop requested (→ 4), 5 immediate stop.
#[derive(Debug, Clone)]
pub struct Ds8Stream {
    pub state: u8,
    /// + 0x50: voice (source) connected; disconnected by the stop (fade).
    pub source: bool,
    /// + 0x5D: pause (halves of silence instead of the ring).
    pub paused: bool,
    /// + 0x10: channel released by the voice (removed when free).
    pub released: bool,
    /// + 0x14 volume (0…1), + 0x18 pitch ratio, + 0x4C pan.
    pub volume: f32,
    pub pitch: f32,
    pub pan: f32,
    /// Wave sample rate.
    pub rate: u32,
    pub channels: usize,
    /// DirectSound buffer of 2 halves, play cursor (frames, fractional).
    pub buffer: Vec<i16>,
    pub buffer_pos: Vec<u64>,
    pub cursor: f64,
    /// + 0x5C: half to write; look-ahead half (+0x1C → +4); frames pending (+0x58).
    pub select: bool,
    pub scratch: Vec<i16>,
    pub scratch_pos: Vec<u64>,
    pub pending: i64,
    /// Notification event (end of a half), reset when read.
    pub event: bool,
    /// Time elapsed since the DirectSound thread's last pass.
    pub poll_clock: f64,
    /// End of playback signalled (callback +0x20 of the channel): the voice is no longer playing.
    pub finished: bool,
    /// Time left before the cursor starts (`ds8_latency`).
    pub delay: f64,
    /// Real sound output only: what the output may read by itself when the game thread is late.
    pub reserve: Option<Reserve>,
    /// Network co-op in lockstep (`Engine::stream_follow_rate`): the game's own pace moves the cursor, and the
    /// frames it passes over wait here, at the output's rate, for the real output to play them.
    pub follow: Option<Follow>,
}

impl Ds8Stream {
    pub fn new(rate: u32, channels: usize) -> Ds8Stream {
        Self::with_follow_rate(rate, channels, 0)
    }

    fn with_follow_rate(rate: u32, channels: usize, follow_rate: u32) -> Ds8Stream {
        let ch = channels.max(1);
        Ds8Stream {
            state: 6,
            source: false,
            paused: false,
            released: false,
            volume: 1.0,
            pitch: 1.0,
            pan: 0.5,
            rate,
            channels: ch,
            buffer: vec![0; 2 * HALF * ch],
            buffer_pos: vec![u64::MAX; 2 * HALF],
            cursor: 0.0,
            select: false,
            scratch: vec![0; HALF * ch],
            scratch_pos: vec![u64::MAX; HALF],
            pending: 0,
            event: false,
            poll_clock: 0.0,
            finished: false,
            delay: 0.0,
            reserve: None,
            follow: match follow_rate {
                0 => None,
                rate => Some(Follow::new(rate)),
            },
        }
    }

    /// DirectSound frequency: wave rate × pitch.
    pub fn frequency(&self) -> f64 {
        frequency_ds(self.rate, self.pitch) as f64
    }

    /// Play: start requested from the DirectSound thread.
    pub fn play(&mut self) {
        self.source = true;
        // Also sets the + 0x5C toggle, whatever the state.
        self.select = true;
        if self.state == 6 {
            self.state = 1;
        }
    }

    /// Stop: the source is disconnected; a start not yet served is cancelled.
    pub fn stop(&mut self) {
        self.source = false;
        if self.state == 1 || self.state == 4 {
            self.state = 6;
            self.finished = true;
        }
    }

    pub fn is_playing(&self) -> bool {
        self.source && self.state != 6
    }

    /// Stream position of the audible frame (None: silence or channel stopped).
    pub fn audible(&self) -> Option<u64> {
        if self.state != 2 {
            return None;
        }
        let p = self.buffer_pos[self.cursor as usize % (2 * HALF)];
        (p != u64::MAX).then_some(p)
    }
}

/// Choice of the DirectSound buffer half to write: the +0x5C toggle is flipped, then corrected
/// by GetCurrentPosition's play and write cursors, in bytes, `half_value` in bytes: both
/// strictly in the second half with the toggle set, it is cleared; both strictly in the
/// first half with the toggle cleared, it is set.
pub fn ds_choose_half(select: bool, play: u32, write: u32, half_value: u32) -> bool {
    let mut s = !select;
    if play > half_value && write > half_value {
        if s {
            s = false;
        }
    } else if !s && play < half_value && write < half_value {
        s = true;
    }
    s
}

/// Ring and channel, shared between the engine (ring refill, on each update) and
/// the audio output (the channel consumes the ring at its own pace).
#[derive(Debug)]
pub struct StreamShared {
    pub ring: Circular,
    pub chan: Ds8Stream,
}

pub type StreamHandle = Arc<Mutex<StreamShared>>;

/// Identity of the native ring: it keeps its reader even if the wave replaces it.
/// `detached` holds the reader moved out of the wave, never a copy of it.
#[derive(Debug)]
pub(crate) struct RingBinding {
    owner: ObjId,
    handle: StreamHandle,
    detached: Option<Splitter>,
}

impl StreamShared {
    /// Read of the ring by the channel: one half, padded with
    /// silence; 0 if the ring is empty and finished.
    fn pull_half(&mut self) -> i64 {
        let ch = self.chan.channels;
        let mut tmp = vec![0i16; HALF * ch];
        let mut tmp_pos = vec![u64::MAX; HALF];
        let mut n = self.ring.take(&mut tmp, &mut tmp_pos, HALF);
        // Ring short of a half with the game thread held up: the output reads on by itself.
        if n < HALF && !self.ring.end && !self.ring.closed && !mutant_reserve() {
            if let Some(r) = self.chan.reserve.as_mut() {
                n += r.take(&mut tmp[n * ch..], &mut tmp_pos[n..], HALF - n, reserve_late());
            }
        }
        let end = self.ring.end && self.ring.fill == 0;
        self.chan.scratch.copy_from_slice(&tmp);
        self.chan.scratch_pos.copy_from_slice(&tmp_pos);
        if end && n == 0 {
            0
        } else {
            HALF as i64
        }
    }

    /// Start prefill: two halves of silence (channel buffer created).
    fn prefill(&mut self) {
        self.chan.scratch.iter_mut().for_each(|s| *s = 0);
        self.chan.buffer.iter_mut().for_each(|s| *s = 0);
        self.chan.scratch_pos.iter_mut().for_each(|s| *s = u64::MAX);
        self.chan.buffer_pos.iter_mut().for_each(|s| *s = u64::MAX);
        self.chan.pending = 2 * HALF as i64;
        self.chan.cursor = 0.0;
        self.chan.event = false;
    }

    /// `refill` with DirectSound's play and write cursors, in bytes.
    pub fn refill_with(&mut self, play: u32, write: u32) {
        let ch = self.chan.channels;
        // Toggle, corrected by the cursors.
        self.chan.select = ds_choose_half(self.chan.select, play, write, (HALF * 2 * ch) as u32);
        let at = if self.chan.select { HALF } else { 0 };
        let scratch = self.chan.scratch.clone();
        self.chan.buffer[at * ch..(at + HALF) * ch].copy_from_slice(&scratch);
        let scratch_pos = self.chan.scratch_pos.clone();
        self.chan.buffer_pos[at..at + HALF].copy_from_slice(&scratch_pos);
        if self.chan.source {
            if !self.chan.paused {
                let n = self.pull_half();
                self.chan.pending += n;
            } else {
                self.chan.scratch.iter_mut().for_each(|s| *s = 0);
                self.chan.scratch_pos.iter_mut().for_each(|s| *s = u64::MAX);
                self.chan.pending += HALF as i64;
            }
        } else {
            self.chan.scratch.iter_mut().for_each(|s| *s = 0);
            self.chan.scratch_pos.iter_mut().for_each(|s| *s = u64::MAX);
        }
    }

    /// DirectSound thread pass over a playing channel: without playback (GetStatus) or start, nothing;
    /// the volume is halved without a source; `first_one`: prefill; otherwise, event signalled:
    /// the wait drops by one half and the next half is written (cursors in bytes); without an event, true. Returns false when
    /// the wait falls to -2 halves or less (the channel ends).
    pub fn turn_state2(&mut self, first_one: bool, plays: bool, flagged: bool, play: u32, write: u32) -> bool {
        if !plays && !first_one {
            return false;
        }
        if !self.chan.source {
            self.chan.volume *= 0.5;
        }
        if first_one {
            self.prefill();
        } else if flagged {
            self.chan.pending -= HALF as i64;
            self.refill_with(play, write);
        } else {
            // Event not signalled: the channel carries.
            return true;
        }
        self.chan.pending > -2 * HALF as i64
    }

    /// One DirectSound thread pass for this channel (`thread_ds_pass`).
    pub fn poll(&mut self) {
        let state_value = self.chan.state;
        let (mut start_ok, mut turn_ok) = (true, true);
        match state_value {
            1 => {
                // Start: buffer created, prefill, looping playback.
                self.prefill();
                self.chan.delay = ds8_latency();
            }
            2 => {
                let flagged = self.chan.event;
                self.chan.event = false;
                let bytes_value = (self.chan.cursor * 2.0 * self.chan.channels as f64) as u32;
                turn_ok = self.turn_state2(false, true, flagged, bytes_value, bytes_value);
            }
            _ => {}
        }
        let _ = &mut start_ok;
        let (fresh_list_new, calls_list, _) = thread_ds_pass(state_value, start_ok, turn_ok, false);
        self.chan.state = fresh_list_new;
        if calls_list.contains(&"cb") {
            self.chan.finished = true;
        }
    }

    /// Advances the channel by `dt` seconds without audio output: the cursor moves through the buffer
    /// (notifications at the end of each half), the DirectSound thread runs on each pass (`ds8_period`).
    pub fn advance(&mut self, dt: f64) {
        let mut left = dt;
        while left > 1e-12 {
            let step = left.min(ds8_period() - self.chan.poll_clock).max(0.0);
            if self.chan.state == 2 {
                let d = self.chan.delay.min(step);
                self.chan.delay -= d;
                let freq = self.chan.frequency();
                match self.chan.follow.as_ref().map(|f| (f.rate, f.carry)) {
                    // Follow mode: the cursor moves output frame by output frame, each frame
                    // kept for the output (the same moves as `render`).
                    Some((rate, carry)) => {
                        let want = (step - d) * f64::from(rate) + carry;
                        let n = want.floor() as usize;
                        for _ in 0..n {
                            let s = self.sample();
                            if let Some(f) = self.chan.follow.as_mut() {
                                f.frames.push_back(s);
                                if f.frames.len() > FOLLOW_MAX {
                                    f.frames.pop_front();
                                }
                            }
                            self.move_cursor(freq / f64::from(rate));
                        }
                        if let Some(f) = self.chan.follow.as_mut() {
                            f.carry = want - n as f64;
                        }
                    }
                    None => self.move_cursor((step - d) * freq),
                }
            }
            self.chan.poll_clock += step;
            left -= step;
            if self.chan.poll_clock >= ds8_period() - 1e-12 {
                self.chan.poll_clock = 0.0;
                self.poll();
            }
        }
    }

    /// Real audio output: `frames` frames at `out_rate` into `out` (left, right, before the
    /// gains), the DirectSound thread running on each output pass (`ds8_period`).
    pub fn render(&mut self, out: &mut Vec<[f32; 2]>, frames: usize, out_rate: u32) {
        // Follow mode: the frames the game passed over, nothing moved here.
        let draining = !self.chan.source || self.chan.state == 6;
        if let Some(f) = self.chan.follow.as_mut() {
            f.render(out, frames, draining);
            return;
        }
        let dt = 1.0 / out_rate as f64;
        for _ in 0..frames {
            if self.chan.state == 2 && self.chan.delay > 0.0 {
                self.chan.delay -= dt;
                out.push([0.0, 0.0]);
            } else if self.chan.state == 2 {
                out.push(self.sample());
                let freq = self.chan.frequency();
                self.move_cursor(freq / out_rate as f64);
            } else {
                out.push([0.0, 0.0]);
            }
            self.chan.poll_clock += dt;
            if self.chan.poll_clock >= ds8_period() {
                self.chan.poll_clock -= ds8_period();
                self.poll();
            }
        }
    }

    fn move_cursor(&mut self, frames: f64) {
        let before = self.chan.cursor;
        let mut after = before + frames;
        let h = HALF as f64;
        // Notifications at frames H − 1 and 2H − 1 (end of each half).
        if (before < h && after >= h) || after >= 2.0 * h {
            self.chan.event = true;
        }
        while after >= 2.0 * h {
            after -= 2.0 * h;
        }
        self.chan.cursor = after;
    }

    /// Frame at the cursor (linear interpolation), left/right channels.
    fn sample(&self) -> [f32; 2] {
        let ch = self.chan.channels;
        let total = 2 * HALF;
        let i = self.chan.cursor as usize % total;
        let j = (i + 1) % total;
        let t = (self.chan.cursor - self.chan.cursor.floor()) as f32;
        let s = |k: usize, c: usize| self.chan.buffer[k * ch + c.min(ch - 1)] as f32;
        [s(i, 0) * (1.0 - t) + s(j, 0) * t, s(i, 1) * (1.0 - t) + s(j, 1) * t]
    }
}

/// `RESERVE_LATE`; the tests decide when the game thread counts as late.
fn reserve_late() -> f64 {
    #[cfg(test)]
    {
        RESERVE_LATE_TEST.with(|l| l.get()).unwrap_or(RESERVE_LATE)
    }
    #[cfg(not(test))]
    {
        RESERVE_LATE
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static RESERVE_LATE_TEST: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// Negative control of the tests (never set in a game): the output's reserve is not used.
fn mutant_reserve() -> bool {
    {
        false
    }
}

/// Callback attached to a marker (0x18-byte record of the + 0x74 list).
#[derive(Debug, Clone)]
pub struct MarkerCb {
    /// Bit 0: transition (when audible); bit 1: once only.
    pub flags: u8,
    pub pos: u32,
    pub name: Vec<u8>,
    /// Handler wrapper (engine.handler_refs).
    pub handler: u32,
}

/// One pass of the DirectSound thread loop over a channel: new state, calls made in order (start,
/// pass with 0, buffer stop, end callback, destruction) and whether the channel is still in the list.
/// A start or pass that fails stops the buffer, returns state 6 and calls the end callback; 3 becomes 4 (buffer
/// stopped), 5 becomes 6 (buffer stopped, callback); a channel at 6 released by the voice is destroyed and removed.
pub fn thread_ds_pass(state_value: u8, start_ok: bool, turn_ok: bool, releases: bool) -> (u8, Vec<&'static str>, bool) {
    match state_value {
        1 if start_ok => (2, vec!["D2A0"], true),
        1 => (6, vec!["D2A0", "stop", "cb"], true),
        2 if turn_ok => (2, vec!["D400:0"], true),
        2 => (6, vec!["D400:0", "stop", "cb"], true),
        3 => (4, vec!["stop"], true),
        5 => (6, vec!["stop", "cb"], true),
        6 if releases => (6, vec!["dtor"], false),
        e => (e, vec![], true),
    }
}

/// Start of a streaming channel, calls made in order: the same-size buffer from the voice's cache is
/// reused (stopped) or a buffer is created (CreateSoundBuffer, QueryInterface, Release); notifications at both ends of the
/// halves (`half_value` in bytes, event `event_value`); first pass (with 1: prefill, two Locks) then
/// looping playback. Returns (success, calls). A first pass that fails (SetFrequency) leaves everything as is.
pub fn ds_start(cache: &mut Vec<u32>, key_tuple: u32, half_value: u32, event_value: u32, first_ok: bool) -> (bool, Vec<String>) {
    let mut calls_list: Vec<String> = Vec::new();
    // The original's log lists the prefill Locks first.
    if let Some(i) = cache.iter().position(|&t| t == key_tuple) {
        cache.remove(i);
        calls_list.push("stop".into());
    } else {
        calls_list.extend(["cree", "qi_buf8", "release"].map(String::from));
    }
    calls_list.push("qi_notify".into());
    calls_list.push(format!("notif2:{}/{}:{}/{}", half_value - 1, event_value, 2 * half_value - 1, event_value));
    calls_list.push("release".into());
    if !first_ok {
        return (false, calls_list);
    }
    let mut v = vec![format!("L0:{half_value}"), format!("L{half_value}:{half_value}")];
    v.extend(calls_list);
    v.push("play0:0:1".into());
    (true, v)
}

/// Start of a static buffer: like `ds_start` without notifications; the reused buffer is set back to `pos`
/// (SetCurrentPosition), a created buffer receives all of `bytes_value`; first pass (with 1: prefill
/// of two halves of `bytes_value`) then playback, looping if asked. Locks are listed first as in the original's
/// log.
pub fn ds_static_start(cache: &mut Vec<u32>, key_tuple: u32, bytes_value: u32, pos: u32, run_loop: bool, first_ok: bool) -> (bool, Vec<String>) {
    let mut locks: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    if let Some(i) = cache.iter().position(|&t| t == key_tuple) {
        cache.remove(i);
        others.push(format!("setpos{pos}"));
    } else {
        others.extend(["cree", "qi_buf8", "release"].map(String::from));
        locks.push(format!("L0:{bytes_value}"));
    }
    if first_ok {
        locks.push(format!("L0:{bytes_value}"));
        locks.push(format!("L{bytes_value}:{bytes_value}"));
        others.push(format!("play0:0:{}", run_loop as u8));
    }
    locks.extend(others);
    (first_ok, locks)
}

/// Insertion after the callbacks with a lower or equal position (sorted list, doubly linked in
/// memory at + 0x74 of the wave).
pub fn cbs_insert(cbs: &mut Vec<MarkerCb>, cb: MarkerCb) {
    let i = cbs.iter().position(|c| c.pos > cb.pos).unwrap_or(cbs.len());
    cbs.insert(i, cb);
}

/// Removes the "once only" callbacks (bit 1), one by one.
pub fn cbs_purge(cbs: &mut Vec<MarkerCb>) {
    cbs.retain(|c| c.flags & 2 == 0);
}

/// The reader watches the first callback with position >= `from`: (next position, armed), or
/// (-1, not armed) without a callback.
pub fn cbs_arm(cbs: &[MarkerCb], from: i32) -> (u32, bool) {
    match cbs_find(cbs, from) {
        Some(i) => (cbs[i].pos, true),
        None => (u32::MAX, false),
    }
}

/// First callback with a greater or equal position (signed comparison, as in the original).
pub fn cbs_find(cbs: &[MarkerCb], pos: i32) -> Option<usize> {
    cbs.iter().position(|c| c.pos as i32 >= pos)
}

/// State of a streamed wave (fields + 0x44 … + 0x84 of the 0x88-byte object).
pub struct StreamState {
    /// + 0x6C: stream position outside playback.
    pub position: u64,
    /// + 0x84: decoder and its buffer; absent if the file is missing.
    pub decoder: Option<Box<dyn FrameSource>>,
    /// The wave's source, for the output's reserve (`Reserve`).
    pub source: Option<StreamSource>,
    pub decbuf: Circular,
    /// + 0x20 of the decoder buffer: seek to perform before the next read.
    pub pending_seek: Option<u32>,
    /// + 0x78: marker reader (created by playback).
    pub reader: Option<Splitter>,
    /// + 0x7C / + 0x20 / + 0x1C: ring and channel.
    pub shared: Option<StreamHandle>,
    /// + 0x74: callbacks sorted by position.
    pub cbs: Vec<MarkerCb>,
    /// + 0x48: timers of the transition callbacks (slots reused).
    pub timers: Vec<Timer>,
    /// + 0x70: end handler (message 0x3EB).
    pub end_handler: u32,
    pub rate: u32,
    pub channels: usize,
    /// Decoder in the decoder thread's list: a newly created decoder goes into the waiting list,
    /// merged at the start of the thread's next pass.
    pub decoder_registered: bool,
    /// First fill of the decoded buffer done ahead of time by the "stream" thread (`advance_value`), and the address
    /// of the decoder opened here (the result is only used if it is still that one).
    advance_value: Option<(std::sync::mpsc::Receiver<Option<Advance>>, usize)>,
}

/// First fill of a Ubi ADPCM stream's decoder buffer, done ahead of time by a separate thread
/// (on a fresh decoder): the logic no longer decodes the first
/// BUFFER frames on the decoder thread's first pass (a noticeable cost when a new music track
/// starts). The thread opens its own decoder on the same source and does exactly the same read (one
/// request of BUFFER frames into a fresh buffer, first-read adjustments included); the decoder
/// thread's pass then takes its frames, its read state and the decoder itself, which continues from
/// where it is. If the first read is not that one (seek requested, buffer already touched, decoder
/// replaced), the result is discarded and decoding happens as before. The thread waits on its
/// channel (no busy loop).
pub struct Advance {
    decoder_value: HxDecoder,
    images: Vec<i16>,
    value_read: (usize, Option<bool>),
}

type RequestAdvance = (StreamSource, usize, std::sync::mpsc::Sender<Option<Advance>>);

/// First read of a fresh decoder: that of the look-ahead thread as well as that of the decoder pass.
pub fn first_read(src: &StreamSource, frames: usize) -> Option<Advance> {
    let mut decoder_value = HxDecoder::open(src)?;
    // Not a decoder of the original until the stream adopts it (`left_header`).
    decoder_value.inner.leaves_header = false;
    let mut images = vec![0i16; frames * decoder_value.channels];
    let value_read = decoder_value.read(&mut images, frames);
    Some(Advance { decoder_value, images, value_read })
}

fn request_advance(src: &StreamSource, frames: usize) -> Option<std::sync::mpsc::Receiver<Option<Advance>>> {
    static THREAD: std::sync::LazyLock<Option<std::sync::Mutex<std::sync::mpsc::Sender<RequestAdvance>>>> = std::sync::LazyLock::new(|| {
        let (tx, rx) = std::sync::mpsc::channel::<RequestAdvance>();
        std::thread::Builder::new()
            .name("stream".into())
            .spawn(move || {
                while let Ok((src, frames, response)) = rx.recv() {
                    let _ = response.send(first_read(&src, frames));
                }
            })
            .ok()?;
        Some(std::sync::Mutex::new(tx))
    });
    // In tests, the decoding stays on the test's own thread (per-thread test state).
    if cfg!(test) {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(first_read(src, frames));
        return Some(rx);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    THREAD.as_ref()?.lock().ok()?.send((src.clone(), frames, tx)).ok()?;
    Some(rx)
}

impl std::fmt::Debug for StreamState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamState").field("position", &self.position).field("reader", &self.reader).field("cbs", &self.cbs.len()).finish()
    }
}

impl StreamState {
    pub fn new(src: Option<&StreamSource>, rate: u32) -> StreamState {
        let channels = src.map_or(1, |s| s.channels.max(1) as usize);
        let decoder = src.and_then(|s| match s.format_tag {
            1 => PcmDecoder::open(s).map(|d| Box::new(d) as Box<dyn FrameSource>),
            2 => HxDecoder::open(s).map(|d| Box::new(d) as Box<dyn FrameSource>),
            _ => None,
        });
        // Ubi ADPCM decoder opened: its first fill is requested from the look-ahead thread.
        let advance_value = match (src, decoder.as_ref()) {
            (Some(s), Some(d)) if s.format_tag == 2 && mode_advance().is_some() => {
                request_advance(s, BUFFER).map(|rx| (rx, d.as_ref() as *const dyn FrameSource as *const () as usize))
            }
            _ => None,
        };
        StreamState {
            advance_value,
            position: 0,
            decoder,
            source: src.cloned(),
            decbuf: Circular::new(BUFFER, channels),
            pending_seek: None,
            reader: None,
            shared: None,
            cbs: Vec::new(),
            timers: Vec::new(),
            end_handler: 0,
            rate,
            channels,
            decoder_registered: false,
        }
    }

    /// Decoder thread service: the buffer is filled.
    pub fn service_decoder(&mut self) {
        if self.decbuf.closed || self.decbuf.end || self.decbuf.fill >= self.decbuf.capacity() {
            return;
        }
        let space = self.decbuf.capacity() - self.decbuf.fill;
        self.decbuf_fill(space);
    }

    /// Fills up to `n` frames. The engine asks again after wrapping around the end of the ring
    /// buffer; that request is a loop here, bounded, so a damaged stream whose decoder never
    /// gives anything cannot exhaust the stack or hang the game.
    fn decbuf_fill(&mut self, mut n: usize) {
        for _ in 0..MAX_REFILLS {
            match self.decbuf_fill_step(n) {
                Some(rest) => n = rest,
                None => return,
            }
        }
    }

    /// One request of `decbuf_fill`: Some(frames still to ask for) when the engine asks again.
    fn decbuf_fill_step(&mut self, n: usize) -> Option<usize> {
        if n == 0 {
            return None;
        }
        let cap = self.decbuf.capacity();
        let contig = cap - self.decbuf.write;
        let chunk = n.min(contig);
        let ch = self.channels;
        let search = self.pending_seek.is_some();
        if let Some(t) = self.pending_seek.take() {
            if let Some(d) = self.decoder.as_mut() {
                d.seek(t);
            }
        }
        let w = self.decbuf.write;
        // First fill decoded ahead of time (`Advance`): only for the expected read.
        if let Some((rx, address)) = self.advance_value.take() {
            let same_decoder = self.decoder.as_ref().is_some_and(|d| d.as_ref() as *const dyn FrameSource as *const () as usize == address && d.decoded() == 0);
            let planned = !search && same_decoder && w == 0 && self.decbuf.fill == 0 && chunk == BUFFER && cap == BUFFER;
            if planned {
                if let Ok(Some(a)) = rx.recv() {
                    if a.images.len() == chunk * ch {
                        let (got, status) = a.value_read;
                        self.decbuf.buf[..chunk * ch].copy_from_slice(&a.images);
                        for i in 0..got {
                            self.decbuf.pos[i] = i as u64;
                        }
                        // The adopted decoder takes the place of the stream's own: only it is the
                        // original's decoder, whose freeing leaves its header (`left_header`).
                        let mut adopted = a.decoder_value;
                        adopted.inner.leaves_header = true;
                        if let Some(own) = self.decoder.as_mut() {
                            own.forget_header();
                        }
                        self.decoder = Some(Box::new(adopted));
                        self.decbuf.write += got;
                        self.decbuf.end = status == Some(true);
                        self.decbuf.fill += got;
                        if !self.decbuf.end && contig <= n {
                            self.decbuf.write = 0;
                            return Some(n - got);
                        }
                        return None;
                    }
                }
            }
        }
        let (got, status) = match self.decoder.as_mut() {
            Some(d) => {
                let p0 = d.decoded() as u64;
                let r = d.read(&mut self.decbuf.buf[w * ch..(w + chunk) * ch], chunk);
                for i in 0..r.0 {
                    self.decbuf.pos[w + i] = p0 + i as u64;
                }
                r
            }
            None => (0, Some(true)),
        };
        self.decbuf.write += got;
        self.decbuf.end = status == Some(true);
        self.decbuf.fill += got;
        if !self.decbuf.end && contig <= n {
            self.decbuf.write = 0;
            return Some(n - got);
        }
        None
    }

    /// Read of the decoder buffer by the reader: (frames, state 0/1 end/2 empty).
    fn decbuf_read(&mut self, out: &mut [i16], out_pos: &mut [u64], n: usize) -> (usize, u8) {
        let got = self.decbuf.take(out, out_pos, n);
        let status = if !self.decbuf.end {
            if self.decbuf.fill == 0 {
                2
            } else {
                0
            }
        } else if self.decbuf.fill == 0 {
            1
        } else {
            0
        };
        (got, status)
    }

    /// Seek of the decoder buffer: emptied, seek on the next fill.
    fn decbuf_seek(&mut self, frame: u32) {
        self.decbuf.read = 0;
        self.decbuf.write = 0;
        self.decbuf.fill = 0;
        self.decbuf.end = false;
        self.pending_seek = Some(frame);
    }
}

impl Engine {
    fn ring_key(&self, obj: ObjId) -> Option<u32> {
        let handle = self.stream(obj)?.shared.as_ref()?;
        self.ring_bindings.iter().find_map(|(&key, b)| Arc::ptr_eq(handle, &b.handle).then_some(key))
    }

    fn register_ring(&mut self, obj: ObjId) -> Option<u32> {
        if let Some(key) = self.ring_key(obj) { return Some(key); }
        let handle = self.stream(obj)?.shared.clone()?;
        let mut key = obj;
        if self.ring_bindings.contains_key(&key) {
            key = u32::MAX;
            while self.ring_bindings.contains_key(&key) { key -= 1; }
        }
        self.ring_bindings.insert(key, RingBinding { owner: obj, handle, detached: None });
        Some(key)
    }

    fn ensure_ring_bindings(&mut self) {
        for key in self.ring_order.clone() {
            if !self.ring_bindings.contains_key(&key) { self.register_ring(key); }
        }
    }

    fn ring_handle(&self, key: u32) -> Option<StreamHandle> {
        self.ring_bindings.get(&key).map(|b| b.handle.clone())
            .or_else(|| self.stream(key).and_then(|s| s.shared.clone()))
    }

    fn ring_owner(&self, key: u32) -> ObjId {
        self.ring_bindings.get(&key).map_or(key, |b| b.owner)
    }

    fn ring_reader(&self, key: u32) -> Option<&Splitter> {
        if let Some(reader) = self.ring_bindings.get(&key).and_then(|b| b.detached.as_ref()) { return Some(reader); }
        self.stream(self.ring_owner(key))?.reader.as_ref()
    }

    fn ring_reader_mut(&mut self, key: u32) -> Option<&mut Splitter> {
        if self.ring_bindings.get(&key).is_some_and(|b| b.detached.is_some()) {
            return self.ring_bindings.get_mut(&key)?.detached.as_mut();
        }
        let owner = self.ring_owner(key);
        self.stream_mut(owner)?.reader.as_mut()
    }

    /// Callback of a string handler (wrapper vtable + 8): the parameter
    /// is the marker name string.
    pub(crate) fn call_string_handler(&mut self, handler: u32, name: &[u8]) {
        if !matches!(self.handler_refs.get(handler as usize), Some(Some(_))) {
            return;
        }
        let s = self.mem.new_string(name);
        let params = self.mem.alloc_with(&s.to_le_bytes());
        self.handler_call(handler, params);
        self.mem.release(params);
        self.mem.delete_string(s);
    }

    pub(crate) fn handler_addref(&mut self, handler: u32) {
        if let Some(Some(h)) = self.handler_refs.get(handler as usize).copied() {
            if h.native == 0 {
                self.send(h.owner, 0x3ea, 0);
            }
        }
    }

    pub(crate) fn stream(&self, obj: ObjId) -> Option<&StreamState> {
        self.wave_obj(obj)?.stream.as_deref()
    }

    pub(crate) fn stream_mut(&mut self, obj: ObjId) -> Option<&mut StreamState> {
        self.wave_obj_mut(obj)?.stream.as_deref_mut()
    }

    /// Wave markers (position, name), sorted.
    fn markers(&self, obj: ObjId) -> Vec<(u32, Vec<u8>)> {
        self.wave_obj(obj).and_then(|w| w.wave.as_ref()).map(|w| w.markers.clone()).unwrap_or_default()
    }

    /// Stream position: the reader's during playback, otherwise + 0x6C.
    // Reader position in frames.
    fn stream_pos(&self, obj: ObjId) -> u64 {
        self.stream(obj).map_or(0, |s| s.reader.as_ref().map_or(s.position, |r| r.pos))
    }

    /// Insertion after the callbacks with a lower or equal position.
    fn stream_insert(&mut self, obj: ObjId, cb: MarkerCb) {
        if let Some(st) = self.stream_mut(obj) {
            cbs_insert(&mut st.cbs, cb);
        }
    }

    /// New callback (the record keeps a reference to the handler).
    fn stream_add(&mut self, obj: ObjId, pos: u32, handler: u32, name: Vec<u8>, flags: u8) {
        self.handler_addref(handler);
        self.stream_insert(obj, MarkerCb { flags, pos, name, handler });
    }

    /// The reader watches the first callback with position ≥ `from`.
    fn stream_arm(&mut self, obj: ObjId, from: u64) {
        let Some(st) = self.stream_mut(obj) else { return };
        let (next, armed) = cbs_arm(&st.cbs, from as i32);
        if let Some(r) = st.reader.as_mut() {
            r.next = if armed { next as u64 } else { u64::MAX };
            r.armed = armed;
        }
    }

    /// Messages specific to streamed waves; false if the message goes through the common handling
    pub(crate) fn stream_message(&mut self, obj: ObjId, msg: u32, args: Addr) -> bool {
        if self.stream(obj).is_none() {
            return false;
        }
        // Pushed arguments: string (push.r, its value) then handler wrapper (push.h,
        // which has already taken a reference).
        let name_arg = |e: &Engine| e.mem.string(e.mem.u32(args));
        match msg {
            1 => {
                // Stop: end handler and timers dropped, then the
                // common handling stops the voice.
                let end = self.stream_mut(obj).map(|st| std::mem::take(&mut st.end_handler)).unwrap_or(0);
                if end != 0 {
                    self.handler_release(end);
                }
                let n = self.stream(obj).map_or(0, |s| s.timers.len());
                for i in 0..n {
                    self.stream_timer_stop(obj, i);
                }
                false
            }
            2 => {
                let n = self.stream(obj).map_or(0, |s| s.timers.len());
                for i in 0..n {
                    self.stream_timer_pause(obj, i);
                }
                false
            }
            3 => {
                let n = self.stream(obj).map_or(0, |s| s.timers.len());
                for i in 0..n {
                    self.stream_timer_resume(obj, i);
                }
                false
            }
            4 => {
                // Voice playing, or a pending transition callback.
                let playing = self.stream_is_playing(obj);
                self.holder_set(args, playing);
                true
            }
            0x3eb => {
                let h = self.mem.u32(args);
                let old = self.stream_mut(obj).map(|st| std::mem::replace(&mut st.end_handler, h)).unwrap_or(0);
                if old != 0 {
                    self.handler_release(old);
                }
                true
            }
            0x3ec => {
                // GotoMarker(name): first marker with this name.
                let name = name_arg(self);
                if let Some(&(pos, _)) = self.markers(obj).iter().find(|(_, n)| *n == name) {
                    self.stream_drop_once(obj);
                    self.stream_seek(obj, pos as u64);
                }
                true
            }
            0x3ed => {
                self.stream_drop_once(obj);
                self.stream_seek(obj, 0);
                true
            }
            0x3ee..=0x3f5 => {
                let trans = matches!(msg, 0x3f2..=0x3f5) as u8;
                let current = self.stream_pos(obj);
                let markers = self.markers(obj);
                let prefix = |name: &[u8], n: &[u8]| {
                    let len = name.len() & 0xffff;
                    n.len() >= len && n[..len] == name[..len]
                };
                let named = matches!(msg, 0x3ee | 0x3f0 | 0x3f2 | 0x3f4);
                let handler = if named { self.mem.u32(args + 4) } else { self.mem.u32(args) };
                if handler != 0 {
                    match msg {
                        0x3ee | 0x3f2 => {
                            let name = name_arg(self);
                            if let Some((pos, n)) = markers.iter().find(|(p, n)| *p as u64 > current && prefix(&name, n)) {
                                self.stream_add(obj, *pos, handler, n.clone(), trans | 2);
                            }
                        }
                        0x3ef | 0x3f3 => {
                            if let Some((pos, n)) = markers.iter().find(|(p, _)| *p as u64 > current) {
                                self.stream_add(obj, *pos, handler, n.clone(), trans | 2);
                            }
                        }
                        0x3f0 | 0x3f4 => {
                            let name = name_arg(self);
                            for (pos, n) in markers.iter().filter(|(_, n)| prefix(&name, n)) {
                                self.stream_add(obj, *pos, handler, n.clone(), trans);
                            }
                        }
                        _ => {
                            for (pos, n) in &markers {
                                self.stream_add(obj, *pos, handler, n.clone(), trans);
                            }
                        }
                    }
                    // The caller's reference (push.h) is released: each callback keeps its own.
                    self.handler_release(handler);
                }
                // Watching from the reader's current position.
                if self.stream(obj).is_some_and(|s| s.reader.is_some()) {
                    let at = self.stream_pos(obj);
                    self.stream_arm(obj, at);
                }
                true
            }
            // The reader and the channel move to the destination.
            0x3ea => {
                self.reserve_forget(obj);
                let destination = self.mem.u32(args);
                if self.stream(destination).is_none() {
                    return true;
                }
                self.ensure_ring_bindings();
                let source_key = self.register_ring(obj);
                if destination != obj {
                    if let Some(old_key) = self.register_ring(destination) {
                        let old_reader = self.stream_mut(destination).unwrap().reader.take();
                        self.ring_bindings.get_mut(&old_key).unwrap().detached = old_reader;
                    }
                }
                let (pan, pitch, volume, fx, voice) = {
                    let wave = self.wave_obj_mut(obj).unwrap();
                    (wave.pan, wave.pitch, wave.volume, wave.fx, wave.voice.take())
                };
                let (reader, shared) = {
                    let source = self.stream_mut(obj).unwrap();
                    (source.reader.take(), source.shared.take())
                };
                let wave = self.wave_obj_mut(destination).unwrap();
                (wave.pan, wave.pitch, wave.volume, wave.fx, wave.voice) = (pan, pitch, volume, fx, voice);
                let target = wave.stream.as_deref_mut().unwrap();
                target.shared = shared;
                target.reader = reader;
                if let Some(reader) = target.reader.as_mut() {
                    // Init/Seek do not empty the buffer and do not reset count to zero.
                    reader.source = target.decoder.is_some();
                    reader.pos = target.position;
                    reader.end_cb = true;
                }
                let position = target.position;
                self.stream_arm(destination, position);
                // The order refers to the ring, not to the wave's replaceable fields.
                if let Some(key) = source_key {
                    self.ring_bindings.get_mut(&key).unwrap().owner = destination;
                }
                if destination == obj {
                    self.wave_obj_mut(obj).unwrap().voice = None;
                    let source = self.stream_mut(obj).unwrap();
                    let reader = source.reader.take();
                    source.shared = None;
                    if let Some(key) = source_key {
                        self.ring_bindings.get_mut(&key).unwrap().detached = reader;
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub(crate) fn stream_is_playing(&self, obj: ObjId) -> bool {
        let Some(st) = self.stream(obj) else { return false };
        if let Some(h) = &st.shared {
            if h.lock().map(|s| s.chan.is_playing()).unwrap_or(false) {
                return true;
            }
        }
        st.timers.iter().any(|t| t.handler != 0)
    }

    /// "once" callbacks are forgotten (without releasing their handler,
    /// as in the original).
    fn stream_drop_once(&mut self, obj: ObjId) {
        if let Some(st) = self.stream_mut(obj) {
            cbs_purge(&mut st.cbs);
        }
    }

    /// The reader and the decoder restart from `pos`, then the callbacks at that
    /// position fire immediately.
    pub(crate) fn stream_seek(&mut self, obj: ObjId, pos: u64) {
        self.reserve_forget(obj);
        let Some(st) = self.stream_mut(obj) else { return };
        if let Some(r) = st.reader.as_mut() {
            r.pos = pos;
        }
        st.position = pos;
        st.decbuf_seek(pos as u32);
        self.stream_dispatch(obj);
    }

    /// Callbacks at the reader's position; sync ones immediately, transition ones
    /// through a timer (time until audible). Watching resumes right after.
    pub(crate) fn stream_dispatch(&mut self, obj: ObjId) {
        let pos = self.stream_pos(obj);
        let Some(st) = self.stream(obj) else { return };
        let reading = st.reader.is_some();
        let first = st.cbs.iter().position(|c| c.pos as u64 >= pos);
        let run: Vec<MarkerCb> = match first {
            Some(i) if st.cbs[i].pos as u64 == pos => {
                let n = st.cbs[i..].iter().take_while(|c| c.pos as u64 == pos).count();
                self.stream_mut(obj).map(|s| s.cbs.drain(i..i + n).collect()).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        for cb in &run {
            if cb.flags & 1 == 0 {
                self.call_string_handler(cb.handler, &cb.name);
                if cb.flags & 2 != 0 {
                    self.handler_release(cb.handler);
                }
            } else if reading {
                // Delay: ring + 2 × 4096 + frames already returned by the current read.
                let (ring_fill, count, rate) = {
                    let st = self.stream(obj).unwrap();
                    let fill = st.shared.as_ref().and_then(|h| h.lock().ok().map(|s| s.ring.fill)).unwrap_or(0) as u64;
                    (fill, st.reader.as_ref().map_or(0, |r| r.count), st.rate.max(1) as u64)
                };
                let delay = ((ring_fill + BUFFER as u64 + count) * 1000 / rate) as u32;
                if cb.flags & 2 == 0 {
                    self.handler_addref(cb.handler);
                }
                self.stream_timer_add(obj, delay, cb.handler, &cb.name);
            }
        }
        for cb in run {
            if cb.flags & 2 == 0 {
                self.stream_insert(obj, cb);
            }
        }
        if reading {
            let at = self.stream_pos(obj);
            self.stream_arm(obj, at + 1);
        }
    }

    /// Play: voice, ring and reader created, connected, channel started.
    pub(crate) fn stream_play(&mut self, obj: ObjId) {
        let (rate, ch, pos) = match self.stream(obj) {
            Some(st) => (st.rate, st.channels, st.position),
            None => return,
        };
        let pitch = self.wave_obj(obj).map_or(1.0, |w| w.pitch);
        let handle: StreamHandle = Arc::new(Mutex::new(StreamShared { ring: Circular::new(BUFFER, ch), chan: Ds8Stream::with_follow_rate(rate, ch, self.stream_follow_rate) }));
        if let Some(st) = self.stream_mut(obj) {
            if let Some(old) = st.shared.take() {
                if let Ok(mut s) = old.lock() {
                    s.ring.closed = true;
                    s.chan.stop();
                    s.chan.released = true;
                }
            }
            st.reader = Some(Splitter { pos, count: 0, next: u64::MAX, armed: false, source: st.decoder.is_some(), end_cb: true });
            st.shared = Some(handle.clone());
        }
        self.stream_arm(obj, pos);
        if let Some(v) = self.wave_obj(obj).and_then(|w| w.voice) {
            self.voice_set(v, |x| {
                x.playing = true;
                x.paused = false;
                x.stream = Some(handle.clone());
                x.starts = x.starts.wrapping_add(1);
            });
            let gain = self.voices.get(v as usize).and_then(|x| x.as_ref()).map_or(1.0, |x| x.gain);
            if let Ok(mut s) = handle.lock() {
                s.chan.volume = gain;
                s.chan.pitch = pitch;
                s.chan.play();
            }
        }
        if let Some(key) = self.register_ring(obj) { self.ring_order.insert(0, key); }
    }

    /// Stop of a streamed wave's voice (message 1): source disconnected (fade
    /// of the end of the buffer), channel released.
    pub(crate) fn stream_stop_voice(&mut self, obj: ObjId) {
        if let Some(h) = self.stream(obj).and_then(|s| s.shared.clone()) {
            if let Ok(mut s) = h.lock() {
                s.chan.stop();
                s.chan.released = true;
            }
        }
    }

    pub(crate) fn stream_pause_voice(&mut self, obj: ObjId, paused: bool) {
        if let Some(h) = self.stream(obj).and_then(|s| s.shared.clone()) {
            if let Ok(mut s) = h.lock() {
                s.chan.paused = paused;
            }
        }
    }

    /// Destruction: timers, callback and end handlers released, ring
    /// and buffer closed.
    pub(crate) fn stream_destroy(&mut self, obj: ObjId) {
        let n = self.stream(obj).map_or(0, |s| s.timers.len());
        for i in 0..n {
            self.stream_timer_stop(obj, i);
        }
        let (end, cbs, shared) = match self.stream_mut(obj) {
            Some(st) => (std::mem::take(&mut st.end_handler), std::mem::take(&mut st.cbs), st.shared.take()),
            None => return,
        };
        if end != 0 {
            self.handler_release(end);
        }
        for c in cbs {
            self.handler_release(c.handler);
        }
        let removed = shared.as_ref().and_then(|h| self.ring_bindings.iter().find_map(|(&key, b)| Arc::ptr_eq(h, &b.handle).then_some(key)));
        if let Some(h) = shared {
            if let Ok(mut s) = h.lock() {
                s.ring.closed = true;
                s.chan.stop();
                s.chan.released = true;
            }
        }
        if let Some(st) = self.stream_mut(obj) {
            st.decbuf.closed = true;
            st.reader = None;
        }
        // After a transfer, a key numerically equal to obj may belong to another wave.
        if let Some(key) = removed {
            self.ring_order.retain(|&o| o != key);
            self.ring_bindings.remove(&key);
        }
    }

    /// Marker reader: `n` frames into `out`; (frames, state 0 / 1 end / 2 empty).
    fn splitter_read(&mut self, obj: ObjId, out: &mut [i16], out_pos: &mut [u64], n: usize) -> (usize, u8) {
        let key = self.ring_key(obj).unwrap_or(obj);
        self.splitter_read_ring(key, out, out_pos, n)
    }

    fn splitter_read_ring(&mut self, key: u32, out: &mut [i16], out_pos: &mut [u64], n: usize) -> (usize, u8) {
        let obj = self.ring_owner(key);
        let ch = self.stream(obj).map_or(1, |s| s.channels);
        let Some(r) = self.ring_reader(key).cloned() else { return (0, 1) };
        if r.armed && r.next >= r.pos && r.next < r.pos + n as u64 {
            // Marker within the request: read up to it.
            let k = (r.next - r.pos) as usize;
            let (got, st) = if r.source {
                let s = self.stream_mut(obj).unwrap();
                s.decbuf_read(out, out_pos, k)
            } else {
                (0, 1)
            };
            if let Some(rd) = self.ring_reader_mut(key) {
                rd.pos += got as u64;
                rd.count += got as u64;
            }
            if st == 2 {
                // Empty buffer: the original loses the frames read (the reader has moved on).
                if let Some(rd) = self.ring_reader_mut(key) {
                    rd.count = 0;
                }
                return (0, 2);
            }
            if k > got {
                if let Some(rd) = self.ring_reader_mut(key) {
                    rd.count = 0;
                }
                return (got, 0);
            }
            // Marker reached (flag 1).
            self.stream_dispatch(obj);
            if self.ring_reader(key).is_none() {
                return (got, 0);
            }
            let (more, st2) = self.splitter_read_ring(key, &mut out[got * ch..], &mut out_pos[got..], n - got);
            if let Some(rd) = self.ring_reader_mut(key) {
                rd.count = 0;
            }
            return (got + more, st2);
        }
        let (got, st) = if r.source {
            let s = self.stream_mut(obj).unwrap();
            s.decbuf_read(out, out_pos, n)
        } else {
            (0, 1)
        };
        if let Some(rd) = self.ring_reader_mut(key) {
            rd.pos += got as u64;
            rd.count += got as u64;
        }
        if r.end_cb && st == 1 {
            // End of stream (flag 0): end handler, then loop if the wave
            // loops and the handler did not move the read position.
            let before = self.ring_reader(key).map_or(0, |r| r.pos);
            self.stream_end(obj);
            let moved = self.ring_reader(key).is_none_or(|rd| rd.pos != before);
            if !moved {
                if let Some(rd) = self.ring_reader_mut(key) {
                    rd.count = 0;
                }
                return (got, st);
            }
            let (more, st2) = if got < n { self.splitter_read_ring(key, &mut out[got * ch..], &mut out_pos[got..], n - got) } else { (0, 0) };
            if let Some(rd) = self.ring_reader_mut(key) {
                rd.count = 0;
            }
            return (got + more, st2);
        }
        if let Some(rd) = self.ring_reader_mut(key) {
            rd.count = 0;
        }
        (got, st)
    }

    /// Flag 0: end of stream.
    fn stream_end(&mut self, obj: ObjId) {
        let pos = self.stream_pos(obj);
        let end = self.stream(obj).map_or(0, |s| s.end_handler);
        if end != 0 {
            // The wrapper's vtable+8, called with (wave, 0): no parameters.
            self.handler_call(end, 0);
        }
        let looping = self.wave_obj(obj).is_some_and(|w| w.res.flags & 1 != 0);
        if looping && self.stream(obj).is_some_and(|s| s.reader.is_some()) && self.stream_pos(obj) == pos {
            self.stream_seek(obj, 0);
        }
    }

    /// Service of a wave's ring: refilled from the reader.
    fn ring_service(&mut self, obj: ObjId) -> bool {
        let Some(h) = self.ring_handle(obj) else { return false };
        let (closed, end, cap, fill) = match h.lock() {
            Ok(s) => (s.ring.closed, s.ring.end, s.ring.capacity(), s.ring.fill),
            Err(_) => return false,
        };
        if closed {
            return false;
        }
        // Real sound output: frames the output read by itself while this thread was held up
        // are skipped here, then the reserve is told where the reader stands.
        if self.external_audio {
            self.reserve_settle(obj, &h);
        }
        if !end && fill < cap {
            self.ring_fill(obj, &h, cap - fill);
        }
        if self.external_audio {
            self.reserve_recover(obj, &h);
            self.reserve_publish(obj, &h);
        }
        true
    }

    /// The frames lent by the output's reserve are passed over: the markers among them (only
    /// ones handled once audible) are served in order, then the reader and the decoder go on
    /// from the frame that follows.
    pub(crate) fn reserve_settle(&mut self, key: u32, h: &StreamHandle) {
        let lent = match h.lock() {
            Ok(mut s) => match s.chan.reserve.as_mut() {
                Some(r) => {
                    // Back from a stall, whether the output had to read by itself or not.
                    r.recovering |= r.lent > 0 || r.served.elapsed().as_secs_f64() >= reserve_late();
                    r.served = std::time::Instant::now();
                    r.active = false;
                    std::mem::take(&mut r.lent)
                }
                None => 0,
            },
            Err(_) => 0,
        };
        if lent == 0 {
            return;
        }
        let owner = self.ring_owner(key);
        let Some(pos) = self.ring_reader(key).map(|r| r.pos + lent) else { return };
        let attached = self.ring_bindings.get(&key).is_none_or(|b| b.detached.is_none());
        // Markers passed by the output: reached now, one after the other.
        for _ in 0..4096 {
            let Some(at) = self.ring_reader(key).filter(|r| attached && r.armed && r.next < pos).map(|r| r.next) else { break };
            if let Some(r) = self.ring_reader_mut(key) {
                r.pos = at;
            }
            self.stream_dispatch(owner);
            if self.ring_reader(key).is_none_or(|r| r.pos != at) {
                // A handler moved the reader or ended the stream: nothing left to pass over.
                return;
            }
        }
        if let Some(r) = self.ring_reader_mut(key) {
            r.pos = pos;
        }
        if let Some(st) = self.stream_mut(owner) {
            st.position = pos;
            st.decbuf_seek(pos as u32);
            // Frames for this very service: the decoder thread's next pass would come a frame
            // later.
            if st.decoder_registered {
                st.service_decoder();
            }
        }
        if attached {
            self.stream_arm(owner, pos);
        }
    }

    /// Back from a stall the ring is empty: a jump made by the program in this very service (a
    /// marker reached at once) would leave the channel without frames until the decoder
    /// thread's next pass, a wait a full ring normally covers. Until the ring is full again, the
    /// decoder is served at once after such a jump and the ring filled from it.
    fn reserve_recover(&mut self, key: u32, h: &StreamHandle) {
        let (recovering, end, cap, fill) = match h.lock() {
            Ok(mut s) => {
                let full = s.ring.fill >= s.ring.capacity();
                let (end, cap, fill) = (s.ring.end, s.ring.capacity(), s.ring.fill);
                match s.chan.reserve.as_mut() {
                    Some(r) => {
                        if full {
                            r.recovering = false;
                        }
                        (r.recovering, end, cap, fill)
                    }
                    None => (false, end, cap, fill),
                }
            }
            Err(_) => return,
        };
        if !recovering || end || fill >= cap {
            return;
        }
        let owner = self.ring_owner(key);
        let served = match self.stream_mut(owner) {
            Some(st) if st.pending_seek.is_some() && st.decoder_registered => {
                st.service_decoder();
                true
            }
            _ => false,
        };
        if served {
            self.ring_fill(key, h, cap - fill);
        }
    }

    /// The reader is about to be moved (jump to a marker, back to the start): what the output
    /// read by itself came before the jump and is not passed over; nothing more is lent until
    /// the ring's next service.
    fn reserve_forget(&mut self, obj: ObjId) {
        if !self.external_audio {
            return;
        }
        if let Some(h) = self.stream(obj).and_then(|s| s.shared.clone()) {
            if let Ok(mut s) = h.lock() {
                if let Some(r) = s.chan.reserve.as_mut() {
                    r.lent = 0;
                    r.limit = r.next;
                    r.active = false;
                    r.served = std::time::Instant::now();
                }
            }
        }
    }

    /// Where the reader stands and how far the output's reserve may read by itself.
    pub(crate) fn reserve_publish(&mut self, key: u32, h: &StreamHandle) {
        let owner = self.ring_owner(key);
        let detached = self.ring_bindings.get(&key).is_some_and(|b| b.detached.is_some());
        let state = match (self.ring_reader(key), self.stream(owner)) {
            (Some(r), Some(st)) if !detached && r.source && st.decoder.is_some() && st.pending_seek.is_none() => {
                // First marker from the reader's position whose handler runs on reaching it.
                let stop = st.cbs.iter().find(|c| c.pos as u64 >= r.pos && c.flags & 1 == 0).map_or(u64::MAX, |c| c.pos as u64);
                st.source.as_ref().map(|src| (r.pos, stop.min(source_frames(src)), src.clone()))
            }
            _ => None,
        };
        let Ok(mut s) = h.lock() else { return };
        match state {
            Some((next, limit, src)) => match s.chan.reserve.as_mut() {
                Some(r) if r.lent == 0 => {
                    if r.source.path != src.path || r.source.offset != src.offset {
                        r.source = src;
                        r.decoder = None;
                    }
                    r.next = next;
                    r.limit = limit.max(next);
                }
                Some(_) => {}
                None => s.chan.reserve = Some(Reserve::new(src, next, limit.max(next))),
            },
            // Nothing the output could read by itself (reader moved away, jump pending).
            None => {
                if let Some(r) = s.chan.reserve.as_mut() {
                    if r.lent == 0 {
                        r.limit = r.next;
                    }
                }
            }
        }
    }

    fn ring_fill(&mut self, obj: ObjId, h: &StreamHandle, mut n: usize) {
        loop {
            let (write, cap, ch) = match h.lock() {
                Ok(s) => (s.ring.write, s.ring.capacity(), s.ring.channels),
                Err(_) => return,
            };
            let contig = cap - write;
            let chunk = n.min(contig);
            let mut tmp = vec![0i16; chunk * ch];
            let mut tmp_pos = vec![u64::MAX; chunk];
            let (got, st) = self.splitter_read_ring(obj, &mut tmp, &mut tmp_pos, chunk);
            let Ok(mut s) = h.lock() else { return };
            match s.ring.ring_commit(&tmp, &tmp_pos, got, st, n, chunk, contig) {
                Some(continued) => n = continued,
                None => return,
            }
        }
    }

    /// Decoder thread passes due in the interval (every `ds8_period`): true if there is one.
    fn decoder_passes_due(&mut self) -> bool {
        if self.decoder_clock < ds8_period() - 1e-12 {
            return false;
        }
        while self.decoder_clock >= ds8_period() - 1e-12 {
            self.decoder_clock -= ds8_period();
        }
        true
    }

    /// One decoder thread pass: every decoder in its list is served.
    fn serve_decoders(&mut self, objs: &[ObjId]) {
        for &o in objs {
            if let Some(st) = self.stream_mut(o) {
                if st.decoder_registered {
                    st.service_decoder();
                }
            }
        }
    }

    /// Decoder and DirectSound threads (simulated, without real audio output) over the interval
    /// before this update: they run during the frame, before HXAudio's Update, which
    /// therefore sees from its start the finished channels (end callback +0x20) and the halves served.
    pub fn stream_threads(&mut self, dt: f64) {
        let objs: Vec<ObjId> = (0..self.object_count() as u32).filter(|&o| self.stream(o).is_some()).collect();
        self.decoder_clock += dt;
        if self.decoder_passes_due() {
            self.serve_decoders(&objs);
        }
        // Decoders created since the last update (started by this frame's scripts,
        // after the passes it covers) join the list for the next pass.
        for &o in &objs {
            if let Some(st) = self.stream_mut(o) {
                st.decoder_registered = true;
            }
        }
        if !self.external_audio {
            // A single DirectSound thread: every channel runs on the same passes, a started channel
            // waits for the next one.
            let clock = self.ds8_clock.clamp(0.0, ds8_period() - 1e-9);
            let mut channels: Vec<StreamHandle> = self.ring_bindings.values().map(|b| b.handle.clone()).collect();
            for &o in &objs {
                if let Some(h) = self.stream(o).and_then(|s| s.shared.clone()) {
                    if !channels.iter().any(|old| Arc::ptr_eq(old, &h)) { channels.push(h); }
                }
            }
            for h in channels {
                {
                    if let Ok(mut s) = h.lock() {
                        s.chan.poll_clock = clock;
                        s.advance(dt);
                    }
                }
            }
            let mut c = clock + dt;
            while c >= ds8_period() - 1e-12 {
                c -= ds8_period();
            }
            self.ds8_clock = c.max(0.0);
        }
        // End of the channel's playback: the voice is no longer playing.
        for voice in self.voices.iter_mut().flatten() {
            let finished = voice.stream.as_ref().and_then(|h| h.lock().ok().map(|s| s.chan.finished)).unwrap_or(false);
            if finished { voice.playing = false; }
        }
    }

    /// Update of the streamed waves (start): rings, voice settings.
    /// Every streamed wave is served, like the thread's ring list (add).
    pub(crate) fn streams_update(&mut self, _dt: f64) {
        let objs: Vec<ObjId> = (0..self.object_count() as u32).filter(|&o| self.stream(o).is_some()).collect();
        self.ensure_ring_bindings();
        let order = self.ring_order.clone();
        let mut keep = Vec::new();
        for o in order {
            if self.ring_service(o) {
                keep.push(o);
            } else {
                self.ring_bindings.remove(&o);
            }
        }
        self.ring_order = keep;
        // Voice settings (volume, pitch, pan) to the channel.
        for &o in &objs {
            let Some(v) = self.wave_obj(o).and_then(|w| w.voice) else { continue };
            let Some(x) = self.voices.get(v as usize).and_then(|x| x.as_ref()) else { continue };
            let (gain, pitch, pan) = (x.gain, x.pitch, x.pan);
            if let Some(h) = self.stream(o).and_then(|s| s.shared.clone()) {
                if let Ok(mut s) = h.lock() {
                    if s.chan.source {
                        s.chan.volume = gain;
                        s.chan.pitch = pitch;
                        s.chan.pan = pan;
                    }
                }
            }
        }
    }

    // ----- timers of the transition callbacks (slots + 0x48) -----

    /// One-shot timer of `delay` ms, free slot reused.
    fn stream_timer_add(&mut self, obj: ObjId, delay: u32, handler: u32, name: &[u8]) {
        let now = self.time_ms;
        let s = self.mem.new_string(name);
        let params = self.mem.alloc_with(&s.to_le_bytes());
        let Some(st) = self.stream_mut(obj) else { return };
        let slot = match st.timers.iter().position(|t| t.handler == 0) {
            Some(i) => i,
            None => {
                st.timers.push(Timer { params: 0, entry: 0, repeat: false, delay: 0, range: 0, start: 0, handler: 0 });
                st.timers.len() - 1
            }
        };
        st.timers[slot] = Timer { params, entry: 0, repeat: false, delay, range: 0, start: now, handler };
        let entry = self.timer_list_add(obj, slot as u32, delay, 0);
        if let Some(t) = self.stream_mut(obj).and_then(|st| st.timers.get_mut(slot)) {
            t.entry = entry;
        }
    }

    /// Stops a timer and releases its handler.
    fn stream_timer_stop(&mut self, obj: ObjId, slot: usize) {
        let Some(t) = self.stream_mut(obj).and_then(|st| st.timers.get_mut(slot)) else { return };
        if t.handler == 0 {
            return;
        }
        let (entry, handler, params) = (t.entry, t.handler, t.params);
        t.entry = 0;
        t.handler = 0;
        t.params = 0;
        if entry != 0 {
            self.timer_entry_kill_id(entry);
        }
        self.free_string_params(params);
        self.handler_release(handler);
    }

    fn stream_timer_pause(&mut self, obj: ObjId, slot: usize) {
        let now = self.time_ms;
        let Some(t) = self.stream_mut(obj).and_then(|st| st.timers.get_mut(slot)) else { return };
        if t.handler == 0 || t.entry == 0 {
            return;
        }
        if !t.repeat {
            let elapsed = now.wrapping_sub(t.start);
            t.delay = if elapsed < t.delay { t.delay - elapsed } else { 1 };
            t.start = t.start.wrapping_add(elapsed);
        }
        let entry = t.entry;
        t.entry = 0;
        self.timer_entry_kill_id(entry);
    }

    fn stream_timer_resume(&mut self, obj: ObjId, slot: usize) {
        let Some(t) = self.stream(obj).and_then(|st| st.timers.get(slot)).cloned() else { return };
        if t.handler != 0 && t.entry == 0 {
            let e = self.timer_list_add(obj, slot as u32, t.delay, t.range);
            if let Some(t) = self.stream_mut(obj).and_then(|st| st.timers.get_mut(slot)) {
                t.entry = e;
            }
        }
    }

    /// Callback of a transition timer: one-shot, handler released afterwards.
    pub(crate) fn stream_timer_fire(&mut self, obj: ObjId, slot: usize) {
        let Some(t) = self.stream(obj).and_then(|st| st.timers.get(slot)).cloned() else { return };
        if !matches!(self.handler_refs.get(t.handler as usize), Some(Some(_))) {
            return;
        }
        if t.entry != 0 {
            self.timer_entry_kill_id(t.entry);
        }
        if let Some(x) = self.stream_mut(obj).and_then(|st| st.timers.get_mut(slot)) {
            x.entry = 0;
            x.handler = 0;
            x.params = 0;
        }
        self.handler_call(t.handler, t.params);
        self.free_string_params(t.params);
        self.handler_release(t.handler);
    }

    fn free_string_params(&mut self, params: Addr) {
        if params != 0 {
            let s = self.mem.u32(params);
            self.mem.release(params);
            self.mem.delete_string(s);
        }
    }
}

/// Frequency passed to SetFrequency: FILD of the rate, FMUL by the pitch on the
/// 24-bit FPU (an f32 product), _ftol (truncation, low word), then clamped as unsigned to
/// [100, 100000] (so a negative pitch gives 100000).
pub fn frequency_ds(rate: u32, pitch: f32) -> u32 {
    let f = (rate as f32 * pitch).trunc();
    // _ftol outside the i64 range: 0x8000000000000000, low word zero.
    let v = if f.is_finite() && f.abs() < 9.2e18 { f as i64 as u32 } else { 0 };
    v.clamp(100, 100_000)
}

#[cfg(test)]
mod follow_tests {
    use super::*;

    /// Follow mode's output: the game's frames come by steps of 735 (1/60 s), the output reads
    /// blocks of 256. A hold of the steps shorter than the frames kept ahead (50 ms, then the
    /// steps catch up) is not heard: no frame of silence once the sound started, nothing lost.
    /// Negative control: the former output (frames read as they come) plays silence there.
    #[test]
    fn a_short_hold_is_not_heard() {
        fn run(direct: bool) -> (usize, usize) {
            let mut f = Follow::new(44_100);
            let mut out = Vec::new();
            let mut made = 0usize;
            let mut silent = 0usize;
            let mut started = false;
            let mut t = 0.0f64;
            let next = 1.0f32;
            // 3 s of output; the steps stop from 1.0 s to 1.05 s, then catch up at once.
            while t < 3.0 {
                let due = if (1.0..1.05).contains(&t) { made } else { (t * 44_100.0) as usize + 735 };
                while made < due {
                    for _ in 0..735 {
                        f.frames.push_back([next, next]);
                    }
                    made += 735;
                }
                out.clear();
                if direct {
                    for _ in 0..256 {
                        out.push(f.frames.pop_front().unwrap_or([0.0, 0.0]));
                    }
                } else {
                    f.render(&mut out, 256, false);
                }
                for s in &out {
                    if s[0] != 0.0 {
                        started = true;
                    } else if started {
                        silent += 1;
                    }
                }
                t += 256.0 / 44_100.0;
            }
            (silent, f.frames.len())
        }
        let (silent, left) = run(false);
        assert_eq!(silent, 0, "silence after the hold");
        assert!(left < 3 * FOLLOW_TARGET, "{left} frames left behind");
        let (silent, _) = run(true);
        assert!(silent > 0, "the former output heard no hold");
    }
}