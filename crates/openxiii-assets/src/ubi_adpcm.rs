//! "Ubisoft ADPCM" decoder (codec 2 of the HX banks: XIII's voices and music).
//!
//! Rust port of vgmstream's decoder (`src/coding/ubi_adpcm_decoder.c`,
//! ISC licence), itself based on the reverse engineering by Zench (decubisnd), sigsegv,
//! bnnm and Nicknine. The integer arithmetic reproduces the C code exactly,
//! 16-bit truncations included.
//!
//! Stream: 0x30-byte header, then frames: 0x34-byte state per channel, then
//! two sub-frames of 4- or 6-bit codes (each followed by a padding byte).

use std::fmt;

#[derive(Debug)]
pub struct AdpcmError(pub String);

impl fmt::Display for AdpcmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AdpcmError {}

const CODES_PER_SUBFRAME_MAX: u32 = 1536;
const FRAME_SIZE_MAX: usize = 0x34 * 2 + (1536 * 6 / 8 + 1) * 2;

const ADPCM6_TABLE1: [i32; 64] = [
    -100000000, -369, -245, -133, -33, 56, 135, 207, 275, 338, 395, 448, 499, 548, 593, 635, 676, 717, 755, 791, 825, 858, 889, 919, 948, 975, 1003, 1029,
    1054, 1078, 1103, 1132, 1800, 1800, 1800, 2048, 3072, 4096, 5000, 5056, 5184, 5240, 6144, 6880, 9624, 12880, 14952, 18040, 20480, 22920, 25600, 28040,
    32560, 35840, 40960, 45832, 51200, 56320, 63488, 67704, 75776, 89088, 102400, 0,
];

const ADPCM6_TABLE2: [i32; 64] = [
    1800, 1800, 1800, 2048, 3072, 4096, 5000, 5056, 5184, 5240, 6144, 6880, 9624, 12880, 14952, 18040, 20480, 22920, 25600, 28040, 32560, 35840, 40960, 45832,
    51200, 56320, 63488, 67704, 75776, 89088, 102400, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 5, 6, 6, 6, 7,
];

const ADPCM4_TABLE1: [i32; 16] =
    [-100000000, 8, 269, 425, 545, 645, 745, 850, -1082465976, 1058977874, 1068540887, 1072986849, 1075167887, 1076761723, 1078439444, 1203982336];

const ADPCM4_TABLE2: [i32; 16] = [-1536, 2314, 5243, 8192, 14336, 25354, 45445, 143626, 0, 0, 0, 1, 1, 1, 3, 7];

const DELTA_TABLE: [i32; 66] = [
    1024, 1031, 1053, 1076, 1099, 1123, 1148, 1172, 1198, 1224, 1251, 1278, 1306, 1334, 1363, 1393, 1423, 1454, 1485, 1518, 1551, 1584, 1619, 1654, 1690, 1726,
    1764, 1802, 1841, 1881, 1922, 1964, 2007, -1024, -1031, -1053, -1076, -1099, -1123, -1148, -1172, -1198, -1224, -1251, -1278, -1306, -1334, -1363, -1393,
    -1423, -1454, -1485, -1518, -1551, -1584, -1619, -1654, -1690, -1726, -1764, -1802, -1841, -1881, -1922, -1964, -2007,
];

#[derive(Debug, Clone)]
pub struct Header {
    pub sample_count: u32,
    pub subframe_count: u32,
    pub codes_per_subframe_last: u32,
    pub codes_per_subframe: u32,
    pub subframes_per_frame: u32,
    pub bits_per_sample: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Channel {
    step1: i32,
    next1: i32,
    next2: i32,
    coef1: i16,
    coef2: i16,
    mod1: i16,
    mod2: i16,
    mod3: i16,
    mod4: i16,
    hist1: i16,
    hist2: i16,
    delta1: i16,
    delta2: i16,
    delta3: i16,
    delta4: i16,
    delta5: i16,
}

fn u32_value(d: &[u8], o: usize) -> u32 {
    d.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}

fn i16_value(d: &[u8], o: usize) -> i16 {
    d.get(o..o + 2).map_or(0, |b| i16::from_le_bytes(b.try_into().unwrap()))
}

fn clamp16(v: i32) -> i32 {
    v.clamp(-32768, 32767)
}

fn sign(v: i32) -> i32 {
    if v < 0 {
        -1
    } else {
        1
    }
}

fn absmax16(v: i16, max: i16) -> i16 {
    if v < 0 {
        if v < -max {
            -max
        } else {
            v
        }
    } else if v > max {
        max
    } else {
        v
    }
}

impl Channel {
    /// Initial state of HXAudio's streaming decoder: all zero, no step of 5.
    pub(crate) fn initial() -> Self {
        Channel { step1: 5, ..Default::default() }
    }

    /// State field +8 (next delta, or first frame in 6-bit mode).
    pub(crate) fn next1(&self) -> i32 {
        self.next1
    }

    pub(crate) fn read(d: &[u8]) -> Self {
        Channel {
            step1: u32_value(d, 0x04) as i32,
            next1: u32_value(d, 0x08) as i32,
            next2: u32_value(d, 0x0C) as i32,
            coef1: i16_value(d, 0x10),
            coef2: i16_value(d, 0x12),
            mod1: i16_value(d, 0x18),
            mod2: i16_value(d, 0x1A),
            mod3: i16_value(d, 0x1C),
            mod4: i16_value(d, 0x1E),
            hist1: i16_value(d, 0x20),
            hist2: i16_value(d, 0x22),
            delta1: i16_value(d, 0x28),
            delta2: i16_value(d, 0x2A),
            delta3: i16_value(d, 0x2C),
            delta4: i16_value(d, 0x2E),
            delta5: i16_value(d, 0x30),
        }
    }

    /// Part shared by the 4- and 6-bit modes: new step and delta. The step is the low word of +4
    /// taken as signed (pmaddwd by 246); the delta shift is a 64-bit psllq of the integer shift
    /// in 6-bit mode (zero from 32 on), a shl masked to 5 bits in 4-bit mode.
    fn step(&self, code_signed: i32, table1: &[i32], table2: &[i32], six: bool) -> (i32, i32) {
        let index = code_signed.unsigned_abs() as usize;
        let step_next = table1[index].wrapping_add(self.step1);
        let step = ((self.step1 as i16 as i32 * 246).wrapping_add(table2[index]) >> 8).clamp(271, 2560);
        let mut delta = 0;
        if ((step_next as u32 & 0xFFFF_FF00).wrapping_sub(1)) & 0x8000_0000 == 0 {
            let delta_index = ((step_next >> 3) & 0x1F) as usize + if code_signed < 0 { 33 } else { 0 };
            let d = DELTA_TABLE[delta_index];
            let shift = ((step_next as u32 & 0xFFFF_FF00) as i32 >> 8) as u32;
            delta = if six {
                (if shift < 32 { (d as u32) << shift } else { 0 }) as i32 >> 10
            } else {
                d.wrapping_shl(shift) >> 10
            };
        }
        (step, delta)
    }

    fn expand6(&mut self, code: u8) -> i16 {
        let (step, delta) = self.step(code as i32 - 31, &ADPCM6_TABLE1, &ADPCM6_TABLE2, true);
        let sample = (delta + self.delta1 as i32 + self.hist1 as i32) as i16;
        self.hist1 = sample;
        self.step1 = step;
        self.delta1 = delta as i16;
        sample
    }

    fn expand4(&mut self, code: u8) -> i16 {
        let (step, delta0) = self.step(code as i32 - 7, &ADPCM4_TABLE1, &ADPCM4_TABLE2, false);
        // pmaddwd of the modulators by the deltas, paddd of the two halves, psrad 10: 32-bit word
        // (kept as is at +8).
        let next0 = (self.mod1 as i32 * self.delta1 as i32 + self.mod2 as i32 * self.delta2 as i32)
            .wrapping_add(self.mod3 as i32 * self.delta3 as i32 + self.mod4 as i32 * self.delta4 as i32)
            >> 10;
        let predicted = (self.coef1 as i32 * self.hist1 as i32 + self.coef2 as i32 * self.hist2 as i32) >> 10;
        let sample = (delta0 + next0 + predicted) as i16;

        let mut coef1_next = self.coef1 as i32 * 255;
        let mut coef2_next = self.coef2 as i32 * 254;
        let delta0 = delta0 as i16 as i32;
        // 16-bit sum (paddw) for the zero test and the sign.
        let sum_value = (delta0 as i16).wrapping_add(next0 as i16) as i32;
        if sum_value != 0 {
            // packssdw of the +8/+0xC deltas then paddsw with the deltas.
            let sign1 = sign(sum_value) * sign(self.delta1.saturating_add(clamp16(self.next1) as i16) as i32);
            let sign2 = sign(sum_value) * sign(self.delta2.saturating_add(clamp16(self.next2) as i16) as i32);
            let mut coef_delta = ((((sign1 * 3072) + coef1_next) >> 6) & !0x3) as i16 as i32;
            coef_delta = clamp16(clamp16(coef_delta + 30719) - 30719);
            coef_delta = clamp16(clamp16(coef_delta - 30720) + 30720);
            coef_delta = ((sign2 * 1024) as i16 as i32 - (sign1 * coef_delta) as i16 as i32) * 2;
            coef1_next += sign1 * 3072;
            coef2_next += coef_delta;
        }

        self.hist2 = self.hist1;
        self.hist1 = sample;
        // coef2 saturated to ±0x300 (paddsw/psubsw), coef1 saturated at the top to
        // 0x3C0 − coef2 before saturation (taken unsigned, words of the 32-bit register): bounds ±0x300 and ±(0x3C0 − coef2).
        let (mut c1, mut c2) = ((coef1_next >> 8) as i16, (coef2_next >> 8) as i16);
        let ecx = 0x3C0i32.wrapping_sub(c2 as u16 as i32);
        c2 = c2.saturating_add(0x7CFF).saturating_sub(0x7CFF);
        c2 = c2.saturating_add(-0x7D00).saturating_sub(-0x7D00);
        let edx = 0x7FFFi32.wrapping_sub(ecx) as u32;
        let (lo, hi) = (edx as u16 as i16, (edx >> 16) as u16 as i16);
        c1 = c1.saturating_add(lo).saturating_sub(lo);
        c2 = c2.saturating_add(hi).saturating_sub(hi);
        self.coef2 = absmax16(c2, 768);
        let bound = 0x3C0 - self.coef2 as i32;
        self.coef1 = if (c1 as i32).abs() > bound { (if c1 >= 0 { bound } else { -bound }) as i16 } else { c1 };
        self.next2 = self.next1;
        self.next1 = next0;
        self.step1 = step;
        self.delta5 = self.delta4;
        self.delta4 = self.delta3;
        self.delta3 = self.delta2;
        self.delta2 = self.delta1;
        self.delta1 = delta0 as i16;
        let d1 = sign(self.delta1 as i32);
        // pmullw (product truncated to 16 bits) then paddsw (saturated sum) and psraw 8.
        let module = |m: i16, d: i16| (m.wrapping_mul(255).saturating_add((2048 * d1 * sign(d as i32)) as i16)) >> 8;
        self.mod4 = module(self.mod4, self.delta5);
        self.mod3 = module(self.mod3, self.delta4);
        self.mod2 = module(self.mod2, self.delta3);
        self.mod1 = module(self.mod1, self.delta2);
        sample
    }

    pub(crate) fn expand(&mut self, code: u8, bps: u32) -> i16 {
        if bps == 6 {
            self.expand6(code)
        } else {
            self.expand4(code)
        }
    }
}

/// 4- or 6-bit codes packed into little-endian u32 words, most significant bits first.
pub(crate) fn unpack_codes(data: &[u8], count: usize, bps: u32) -> Vec<u8> {
    let mask = if bps == 6 { 0x3F } else { 0x0F };
    let (mut pos, mut bits, mut input) = (0usize, 0u32, 0u64);
    (0..count)
        .map(|_| {
            if bits < bps {
                input = (input << 32) | u32_value(data, pos) as u64;
                pos += 4;
                bits += 32;
            }
            bits -= bps;
            ((input >> bits) & mask) as u8
        })
        .collect()
}

pub fn read_header(data: &[u8]) -> Result<Header, AdpcmError> {
    if data.len() < 0x30 || u32_value(data, 0) != 8 {
        return Err(AdpcmError("invalid Ubi ADPCM header".into()));
    }
    let h = Header {
        sample_count: u32_value(data, 0x04),
        subframe_count: u32_value(data, 0x08),
        codes_per_subframe_last: u32_value(data, 0x0C),
        codes_per_subframe: u32_value(data, 0x10),
        subframes_per_frame: u32_value(data, 0x14),
        bits_per_sample: u32_value(data, 0x24),
        channels: u32_value(data, 0x2C),
    };
    let ok = h.codes_per_subframe <= CODES_PER_SUBFRAME_MAX
        && h.codes_per_subframe_last <= CODES_PER_SUBFRAME_MAX
        && (h.codes_per_subframe | h.codes_per_subframe_last) != 0
        && h.subframes_per_frame == 2
        && matches!(h.bits_per_sample, 4 | 6)
        && matches!(h.channels, 1 | 2);
    if !ok {
        return Err(AdpcmError(format!("Ubi ADPCM parameters not supported: {h:?}")));
    }
    Ok(h)
}

/// Adjustments of the HX decoder's first read after creation or seek (16-bit format), on `frames`
/// interleaved frames: frame 179 smoothed, then, beyond 485 frames read, the zero frames from 1 to 484 and a few fixed frames
/// replaced by the average of their neighbours. Nothing below 180 frames. Streams go through this (openxiii-hx, stream.rs), and so do
/// internal waves: fills their CWaveData through the same reader. A neighbour beyond the frames read (exactly 180 or 486
/// frames) is read by the original outside the data; here it is 0.
pub fn tweaks_first_read(s: &mut [i16], ch: usize, frames: usize) {
    if frames <= 0xb3 {
        return;
    }
    let value_read = |s: &[i16], i: usize| -> i32 { s.get(i).copied().unwrap_or(0) as i32 };
    let avg = |a: i32, b: i32| ((a + b) / 2) as i16;
    if ch == 1 {
        s[179] = avg(value_read(s, 180), value_read(s, 178));
        if frames > 0x1e5 {
            for i in 1..0x1e5 {
                if s[i] == 0 {
                    s[i] = avg(value_read(s, i - 1), value_read(s, i + 1));
                }
            }
            for i in [61, 151, 270, 385, 397, 428, 485] {
                s[i] = avg(value_read(s, i + 1), value_read(s, i - 1));
            }
        }
    } else {
        for c in 0..2 {
            s[179 * 2 + c] = avg(value_read(s, 180 * 2 + c), value_read(s, 178 * 2 + c));
        }
        if frames > 0x1e5 {
            for f in 1..0x1e5 {
                if s[f * 2] == 0 || s[f * 2 + 1] == 0 {
                    s[f * 2] = avg(value_read(s, (f - 1) * 2), value_read(s, (f + 1) * 2));
                    s[f * 2 + 1] = avg(value_read(s, (f - 1) * 2 + 1), value_read(s, (f + 1) * 2 + 1));
                }
            }
            for f in [61, 144, 151, 270, 385, 397, 428, 485] {
                for c in 0..2 {
                    s[f * 2 + c] = avg(value_read(s, (f + 1) * 2 + c), value_read(s, (f - 1) * 2 + c));
                }
            }
        }
    }
}

/// Decodes a whole stream to interleaved 16-bit PCM.
pub fn decode(data: &[u8]) -> Result<(Header, Vec<i16>), AdpcmError> {
    let h = read_header(data)?;
    let (bps, channels) = (h.bits_per_sample, h.channels as usize);
    let mut samples = Vec::with_capacity(h.sample_count as usize);
    let mut offset = 0x30;
    let mut subframe = 0;
    let mut state = [Channel::default(), Channel::default()];
    let mut frame = vec![0u8; FRAME_SIZE_MAX + 8];
    while subframe < h.subframe_count && offset < data.len() {
        let (count_a, count_b) = if subframe + 1 == h.subframe_count {
            (h.codes_per_subframe_last, 0)
        } else if subframe + 2 == h.subframe_count {
            (h.codes_per_subframe, h.codes_per_subframe_last)
        } else {
            (h.codes_per_subframe, h.codes_per_subframe)
        };
        let size = |count: u32| {
            if count == 0 {
                0
            } else {
                (bps * count / 8) as usize + 1
            }
        };
        let (size_a, size_b) = (size(count_a), size(count_b));
        let setup = 0x34 * channels;
        // As in vgmstream, the frame buffer is not cleared: the unaligned end
        // of the last sub-frame reads leftovers of the previous frame (identical output).
        let available = data.len().min(offset + setup + size_a + size_b) - offset;
        frame[..available].copy_from_slice(&data[offset..offset + available]);
        for (ch, s) in state.iter_mut().enumerate().take(channels) {
            *s = Channel::read(&frame[ch * 0x34..]);
        }
        for (start, count) in [(setup, count_a), (setup + size_a, count_b)] {
            let codes = unpack_codes(&frame[start..], count as usize, bps);
            if channels == 1 {
                samples.extend(codes.iter().map(|&c| state[0].expand(c, bps)));
            } else {
                for group in codes.chunks(8) {
                    let mut g = [0u8; 8];
                    g[..group.len()].copy_from_slice(group);
                    let mut s = [0i32; 8];
                    for k in 0..4 {
                        s[k] = state[0].expand(g[2 * k], bps) as i32;
                        s[k + 4] = state[1].expand(g[2 * k + 1], bps) as i32;
                    }
                    // Joint stereo: sum and difference.
                    let joint = [s[0] + s[4], s[0] - s[4], s[1] + s[5], s[1] - s[5], s[2] + s[6], s[2] - s[6], s[3] + s[7], s[3] - s[7]];
                    samples.extend(joint.iter().take(group.len()).map(|&v| clamp16(v) as i16));
                }
            }
        }
        offset += setup + size_a + size_b;
        subframe += 2;
    }
    samples.truncate(h.sample_count as usize);
    Ok((h, samples))
}

