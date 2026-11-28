//! HXAudio's streaming Ubi ADPCM decoder (0xC4-byte object: read, seek,
//! position), over a byte reader with no size limit (
//! reading continues past the wave, up to the end of the .hsc file).
//!
//! Differences from a full decode (`ubi_adpcm::decode`, which follows vgmstream), all from the original:
//! - the channel state is only read on the first frame after the header (start or seek);
//!   the state blocks of later frames are read but ignored, the decoder carries on;
//! - during the call that loads this state, the first frame of each decoded sub-frame takes,
//!   channel by channel, the value stored at +8 of the state;
//! - a seek re-reads the header, restarts from the frame (even sub-frame) containing the target
//!   and skips the frames before it;
//! - in stereo, the codes of a sub-frame beyond the last group of 8 are expanded with
//!   channel 0's state, and joint stereo processes the whole group (buffer leftovers included).

use crate::ubi_adpcm::{unpack_codes, Channel};
use std::io::{Read, Seek, SeekFrom};

/// Byte source of the stream (.hsc file opened at the wave's position).
pub trait ByteSource: Read + Seek + Send {}
impl<T: Read + Seek + Send> ByteSource for T {}

/// Read results (return codes).
pub const READ_OK: i32 = 0;
pub const READ_END: i32 = 4;

thread_local! {
    /// Header words (+4 … +0x33) left in the memory of the last decoder freed on this thread.
    static LEFT_HEADER: std::cell::Cell<[u32; 12]> = const { std::cell::Cell::new([0; 12]) };
}

/// The 0xC4-byte object comes from operator new (msvcr70) and its constructor
/// does not clear the header words (+4 … +0x33): until the first read loads the header, they
/// hold what the heap block held, in practice the header of the last decoder freed (same block
/// size, reused first by the heap). Only position() reads them before that: a seek
/// before the first read (a checkpoint whose music jumps at once) gets its target back when
/// that old header is long enough, 0xFFFE0000 otherwise (fresh, zeroed memory).
pub fn left_header() -> [u32; 12] {
    LEFT_HEADER.with(|h| h.get())
}

/// Sets the header words the next decoder created on this thread starts with (`left_header`).
pub fn set_left_header(h: [u32; 12]) {
    LEFT_HEADER.with(|c| c.set(h));
}

pub struct StreamDecoder {
    /// False for a decoder the original does not have at that time (look-ahead thread not
    /// adopted, sound output's cache): freeing it leaves nothing behind (`left_header`).
    pub leaves_header: bool,
    src: Box<dyn ByteSource>,
    /// Byte reader: stream position at its creation (+4) and relative position (+0xC).
    base: u64,
    pos: u64,
    /// Raw 0x30-byte header (+4 … +0x33).
    hdr: [u32; 12],
    /// +0x78: header read; +0x79: channel state loaded from the header.
    header: bool,
    loaded: bool,
    /// +0x7C: sub-frames decoded; +0x80: sub-frames read in the current frame.
    subframe: u32,
    in_frame: u32,
    /// +0x56: channels of the format (initialisation).
    channels: usize,
    states: Vec<Channel>,
    /// +0x44: read buffer, reused (an unaligned sub-frame reads past the bytes read).
    raw: Vec<u8>,
    /// +0x4C / +0x48: decoded frames pending, interleaved.
    out: Vec<i16>,
    avail: usize,
    /// +0x5C: outputs of a stereo sub-frame by groups (channel 0 ×4, channel 1 ×4), kept.
    work: Vec<i16>,
    /// +0x50: 2 = end or error (final).
    status: u32,
    /// +0xC0: end of file reached while reading the last sub-frame (4).
    end: u32,
    /// +0xB8 / +0xBC: pending seek (target), then frames to skip.
    seek_pending: bool,
    skip: u32,
    /// +0xB4.
    out_pos: u32,
}

impl StreamDecoder {
    /// (16-bit format, `channels` channels).
    pub fn new(mut src: Box<dyn ByteSource>, channels: usize) -> StreamDecoder {
        let base = src.stream_position().unwrap_or(0);
        StreamDecoder {
            leaves_header: true,
            src,
            base,
            pos: 0,
            hdr: left_header(),
            header: false,
            loaded: false,
            subframe: 0,
            in_frame: 0,
            channels: channels.max(1),
            states: vec![Channel::initial(); channels.max(1)],
            raw: Vec::new(),
            out: Vec::new(),
            avail: 0,
            work: Vec::new(),
            status: 0,
            end: 0,
            seek_pending: false,
            skip: 0,
            out_pos: 0,
        }
    }

    /// The next read re-reads the header then starts from `frame`.
    pub fn seek(&mut self, frame: u32) {
        self.byte_seek(0);
        self.seek_pending = true;
        self.skip = frame;
        self.header = false;
        self.avail = 0;
        self.out_pos = frame;
        self.end = 0;
    }

    /// Position after a seek (0xFFFE0000 if the decoder is stopped).
    pub fn position(&self) -> u32 {
        if self.status == 2 || (self.channels as u32 * 2).wrapping_mul(self.hdr[1]) < self.out_pos {
            0xfffe_0000
        } else {
            self.out_pos
        }
    }

    fn byte_seek(&mut self, pos: u64) {
        let _ = self.src.seek(SeekFrom::Start(self.base + pos));
        self.pos = pos;
    }

    /// Reads `n` bytes into the +0x44 buffer; 4 if the read is short (end of
    /// file), otherwise 5 or 4 depending on the reader's remainder.
    fn read_bytes(&mut self, n: usize) -> i32 {
        if self.raw.len() < n {
            self.raw.resize(n, 0);
        }
        let mut got = 0;
        while got < n {
            match self.src.read(&mut self.raw[got..n]) {
                Ok(0) | Err(_) => break,
                Ok(k) => got += k,
            }
        }
        self.pos += got as u64;
        if got < n {
            // Zeroes n − read bytes starting at n − read (not at read).
            let start = n - got;
            let end = (start + (n - got)).min(self.raw.len());
            self.raw[start..end].fill(0);
            return 4;
        }
        // Reader remainder (limit 0xFFFFFFFF − position) compared as signed with 5000: always
        // negative, hence 5 ("near the end") on every complete read.
        if (u32::MAX.wrapping_sub(self.pos as u32) as i32) > 0x1388 {
            0
        } else if u32::MAX != self.pos as u32 {
            5
        } else {
            4
        }
    }

    fn h_channels(&self) -> u32 {
        self.hdr[11]
    }

    /// Up to `request` interleaved frames into `dst`; (code, frames returned).
    pub fn read(&mut self, dst: &mut [i16], request: usize) -> (i32, usize) {
        let mut first_state: Option<Vec<i32>> = None;
        let mut need = request;
        if self.status == 2 || self.status == 1 {
            return (self.status as i32, 0);
        }
        let mut local30: i32 = -1;
        let mut local24: i32 = -1;
        if !self.header {
            let st = self.read_bytes(0x30);
            local30 = st;
            if st != 0 && st != 5 {
                return (st, 0);
            }
            for i in 0..12 {
                self.hdr[i] = u32::from_le_bytes(self.raw[i * 4..i * 4 + 4].try_into().unwrap());
            }
            if self.hdr[0] != 8 {
                self.status = 2;
                return (2, 0);
            }
            local24 = local30;
            // States reset (no step of 5), read buffer of 2 bytes per code.
            self.states = vec![Channel::initial(); self.channels];
            let size = self.hdr[4] as usize * 2;
            if self.raw.len() < size {
                self.raw.resize(size, 0);
            }
            self.header = true;
            self.in_frame = self.hdr[5];
            let ch = self.h_channels();
            let ok = match ch {
                1 | 2 => true,
                4 | 6 => self.hdr[10] == 1 && self.hdr[7] == 0,
                _ => false,
            };
            if !ok {
                return (-1, 0);
            }
            self.loaded = false;
        }
        let mut ivar9 = local30;
        if self.seek_pending {
            let ch = self.h_channels();
            let cps = self.hdr[4];
            let bits = self.hdr[9];
            let spf = self.hdr[5].max(1);
            let target = self.skip;
            let mut sf = (target.wrapping_mul(ch) as u64 / cps.max(1) as u64) as u32;
            if sf & 1 != 0 {
                sf -= 1;
            }
            self.subframe = sf;
            let mut off = (((bits * cps) >> 3) + 1) * sf + 0x30 + (sf / spf) * ch * 0x34;
            if self.hdr[2] <= sf {
                self.subframe = self.hdr[2].wrapping_sub(2);
            }
            if sf % spf != 0 {
                off = off.wrapping_add((-1i32 - ((bits * cps) >> 3) as i32) as u32);
            }
            self.skip = target.wrapping_sub(cps * sf / ch.max(1));
            self.byte_seek(off as u64);
            self.avail = 0;
            self.seek_pending = false;
            ivar9 = local24;
        }
        let nch = self.channels;
        if self.avail < need {
            let mut ivar8 = local30;
            if self.avail != 0 {
                let n = self.avail;
                dst[..n * nch].copy_from_slice(&self.out[..n * nch]);
                if self.end == 4 {
                    self.status = 2;
                }
                need -= n;
                self.out_pos = self.out_pos.wrapping_add(n as u32);
                self.avail = 0;
            }
            while need != 0 && self.status != 2 {
                local30 = ivar8;
                let spf = self.hdr[5];
                if self.in_frame == spf {
                    let ch = self.h_channels() as usize;
                    let st = self.read_bytes(ch * 0x34);
                    local30 = st;
                    if ivar8 == 5 && local30 != 4 {
                        local30 = 5;
                    }
                    if local30 != 0 && local30 != 5 {
                        return (local30, 0);
                    }
                    if !self.loaded {
                        if self.hdr[10] == 1 || self.hdr[10] == 3 {
                            self.states = (0..ch).map(|c| Channel::read(&self.raw[c * 0x34..])).collect();
                        }
                        first_state = Some((0..ch.min(self.states.len())).map(|c| self.states[c].next1()).collect());
                    }
                    self.loaded = true;
                    self.in_frame = 0;
                }
                self.subframe = self.subframe.wrapping_add(1);
                self.in_frame += 1;
                let sf = self.subframe;
                let ch = self.h_channels().max(1);
                let codes;
                if sf < self.hdr[2] {
                    let size = ((self.hdr[4] * self.hdr[9]) >> 3) as usize + 1;
                    ivar8 = self.read_bytes(size);
                    if local30 == 5 && ivar8 != 4 {
                        ivar8 = 5;
                    }
                    if ivar8 != 0 && ivar8 != 5 {
                        self.in_frame -= 1;
                        self.subframe -= 1;
                        return (ivar8, 0);
                    }
                    self.avail = (self.hdr[4] / ch) as usize;
                    codes = self.hdr[4] as usize;
                } else {
                    if sf != self.hdr[2] {
                        self.status = 2;
                        return (-1, 0);
                    }
                    let size = ((self.hdr[9] * self.hdr[3]) >> 3) as usize + 1;
                    ivar8 = self.read_bytes(size);
                    if local30 == 5 && ivar8 != 4 {
                        ivar8 = 5;
                    }
                    if ivar8 == 6 {
                        self.subframe -= 1;
                        self.in_frame -= 1;
                        return (6, 0);
                    }
                    if ivar8 == 2 {
                        self.status = 2;
                        return (2, request - need);
                    }
                    self.avail = (self.hdr[3] / ch) as usize;
                    codes = self.hdr[3] as usize;
                }
                self.decode_subframe(codes);
                if let Some(f) = &first_state {
                    for (c, v) in f.iter().enumerate().take(nch) {
                        self.out[c] = *v as i16;
                    }
                }
                let avail = self.avail as i64;
                let skip = self.skip as i64;
                let left = avail - skip;
                ivar9 = ivar8;
                if left < 0 {
                    self.avail = 0;
                    self.skip = (skip - avail) as u32;
                } else {
                    let left = left as usize;
                    let start = request - need;
                    let skip = skip as usize;
                    let rest;
                    if left < need {
                        dst[start * nch..(start + left) * nch].copy_from_slice(&self.out[skip * nch..(skip + left) * nch]);
                        need -= left;
                        self.out_pos = self.out_pos.wrapping_add(left as u32);
                        self.skip = 0;
                        rest = 0;
                    } else {
                        dst[start * nch..(start + need) * nch].copy_from_slice(&self.out[skip * nch..(skip + need) * nch]);
                        self.out_pos = self.out_pos.wrapping_add((skip + need) as u32);
                        let from = (skip + need) * nch;
                        let to = self.avail * nch;
                        self.out.copy_within(from..to, 0);
                        rest = self.avail - (skip + need);
                        need = 0;
                        self.skip = 0;
                    }
                    self.avail = rest;
                    if ivar8 == 4 {
                        if rest == 0 {
                            self.status = 2;
                        } else {
                            self.end = 4;
                            self.subframe -= 1;
                            self.in_frame = 0;
                            ivar9 = 0;
                        }
                    }
                }
            }
        } else {
            let n = need;
            dst[..n * nch].copy_from_slice(&self.out[..n * nch]);
            self.out_pos = self.out_pos.wrapping_add(n as u32);
            self.out.copy_within(n * nch..self.avail * nch, 0);
            self.avail -= n;
            need = 0;
            ivar9 = if self.avail == 0 && self.end == 4 { 4 } else { 0 };
        }
        let got = request - need;
        if self.status != 2 && got != 0 {
            if ivar9 == 4 {
                self.status = 2;
            }
            return (ivar9, got);
        }
        self.status = 2;
        (4, got)
    }

    /// (6-bit codes, mode 3; 4-bit, mode 1): one sub-frame into `out`. Unpacking
    /// (6 bits) and (4 bits), expansion (6 bits) and (4 bits).
    fn decode_subframe(&mut self, codes: usize) {
        // The 4-bit routines are chosen when the mode (+0x28 of
        // the header) is 1, the 6-bit ones otherwise, whatever the code size.
        let bits = if self.hdr[10] == 1 { 4 } else { 6 };
        let nch = self.h_channels() as usize;
        let list = unpack_codes(&self.raw, codes, bits);
        let frames = codes / nch.max(1);
        let need = (frames + 4) * nch.max(1);
        if self.out.len() < need {
            self.out.resize(need, 0);
        }
        match nch {
            2 => {
                if self.work.len() < codes * 2 + 8 {
                    self.work.resize(codes * 2 + 8, 0);
                }
                let groups = codes & !7;
                for g in (0..groups).step_by(8) {
                    for k in 0..4 {
                        self.work[g + k] = self.states[0].expand(list[g + 2 * k], bits);
                    }
                    for k in 0..4 {
                        self.work[g + 4 + k] = self.states[1].expand(list[g + 2 * k + 1], bits);
                    }
                }
                for (k, &c) in list.iter().enumerate().skip(groups) {
                    self.work[k] = self.states[0].expand(c, bits);
                }
                if self.hdr[7] == 1 {
                    // Joint stereo in groups of 8: saturated sum and difference.
                    let mut g = 0;
                    while g < codes {
                        for k in 0..4 {
                            let a = self.work[g + k] as i32;
                            let b = self.work[g + 4 + k] as i32;
                            let f = g / 2 + k;
                            if f * 2 + 1 < self.out.len() {
                                self.out[f * 2] = (a + b).clamp(-32768, 32767) as i16;
                                self.out[f * 2 + 1] = (a - b).clamp(-32768, 32767) as i16;
                            }
                        }
                        g += 8;
                    }
                } else {
                    for f in 0..frames {
                        let g = f / 4 * 8;
                        self.out[f * 2] = self.work[g + f % 4];
                        self.out[f * 2 + 1] = self.work[g + 4 + f % 4];
                    }
                }
            }
            _ => {
                // Expands the codes of a mono sub-block in pairs (odd count minus 1); the remaining code
                // is only expanded if the loop did not run, i.e. a sub-block of a single code. Otherwise the last frame
                // keeps what the buffer held (the previous sub-block at the same place).
                for (k, &c) in list.iter().enumerate().take(if codes == 1 { 1 } else { codes & !1 }) {
                    let s = self.states[0].expand(c, bits);
                    if k < self.out.len() {
                        self.out[k] = s;
                    }
                }
            }
        }
    }
}

impl Drop for StreamDecoder {
    /// operator delete: the block keeps the header words for the next decoder.
    fn drop(&mut self) {
        if self.leaves_header {
            set_left_header(self.hdr);
        }
    }
}
