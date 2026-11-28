//! Network bit streams (Core.dll): FBitWriter and FBitReader, bytes read and written least
//! significant bit first (GShift[i] = 1 << i).
//!
//! FBitWriter::WriteInt / SerializeInt: at most appCeilLogTwo(Max) bits,
//! from the least to the most significant bit, while written value + mask < Max. FBitReader::SerializeInt
//! (ReadInt): the same loop when reading. An overflow sets ArIsError
//! (+0x28) without writing anything. Bit copies: appBitsCpy.

/// appCeilLogTwo: smallest n with 2^n ≥ x (0 for 0 and 1).
pub fn ceil_log_two(x: u32) -> u32 {
    if x <= 1 {
        0
    } else {
        32 - (x - 1).leading_zeros()
    }
}

#[derive(Debug, Clone)]
pub struct BitWriter {
    pub buffer: Vec<u8>,
    /// Bits written (+0x44) and maximum size (+0x48).
    pub num: usize,
    pub max: usize,
    pub error: bool,
}

/// FBitWriterMark: ArIsError (+0x28) and bit count (+0x44) of an FBitWriter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mark {
    pub error: bool,
    pub num: usize,
}

impl BitWriter {
    /// FBitWriter(MaxBits): buffer of (MaxBits + 7) / 8 zeroed bytes.
    pub fn new(max_bits: usize) -> Self {
        BitWriter { buffer: vec![0; max_bits.div_ceil(8)], num: 0, max: max_bits, error: false }
    }

    /// FBitWriter::WriteBit.
    pub fn write_bit(&mut self, bit: bool) {
        if self.num + 1 > self.max {
            self.error = true;
            return;
        }
        if bit {
            self.buffer[self.num >> 3] |= 1 << (self.num & 7);
        }
        self.num += 1;
    }

    pub fn write_int(&mut self, value: u32, max: u32) {
        if self.num + ceil_log_two(max) as usize > self.max {
            self.error = true;
            return;
        }
        if max <= 1 {
            return;
        }
        let mut new_value: u32 = 0;
        let mut mask: u32 = 1;
        while mask != 0 {
            if value & mask != 0 {
                let b = &mut self.buffer[self.num >> 3];
                *b = b.wrapping_add(1 << (self.num & 7));
                new_value = new_value.wrapping_add(mask);
            }
            mask = mask.wrapping_mul(2);
            self.num += 1;
            if max <= mask.wrapping_add(new_value) {
                return;
            }
        }
    }

    /// FBitWriter::SerializeBits: `bits` bits of `data`, least significant bit first.
    pub fn write_bits(&mut self, data: &[u8], bits: usize) {
        if self.num + bits > self.max {
            self.error = true;
            return;
        }
        for i in 0..bits {
            if data[i >> 3] & (1 << (i & 7)) != 0 {
                self.buffer[self.num >> 3] |= 1 << (self.num & 7);
            }
            self.num += 1;
        }
    }

    /// FBitWriter::Serialize: whole bytes.
    pub fn write_bytes(&mut self, data: &[u8]) {
        self.write_bits(data, data.len() * 8);
    }

    /// FBitWriter::GetNumBytes.
    pub fn num_bytes(&self) -> usize {
        self.num.div_ceil(8)
    }

    /// FBitWriterMark(FBitWriter&).
    pub fn mark(&self) -> Mark {
        Mark { error: self.error, num: self.num }
    }

    /// FBitWriterMark::Pop: back to the mark (bits and ArIsError), the rest reset
    /// to zero.
    pub fn pop(&mut self, mark: Mark) {
        let num = mark.num;
        if num & 7 != 0 {
            self.buffer[num >> 3] &= (1u8 << (num & 7)) - 1;
        }
        let (start, end) = (num.div_ceil(8), self.num.div_ceil(8));
        if start < end {
            self.buffer[start..end].fill(0);
        }
        self.error = mark.error;
        self.num = num;
    }

    pub fn data(&self) -> &[u8] {
        &self.buffer[..self.num_bytes()]
    }
}

#[derive(Debug, Clone)]
pub struct BitReader {
    pub buffer: Vec<u8>,
    /// Bits available (+0x44) and position (+0x48).
    pub num: usize,
    pub pos: usize,
    pub error: bool,
    /// ArIsCriticalError (+0x2C): string longer than ArMaxSerializeSize.
    pub critical: bool,
    /// ArMaxSerializeSize (+0x30): 0 (no limit), 10000 for an FInBunch.
    pub max_serialize: usize,
}

impl BitReader {
    /// FBitReader(Src, CountBits).
    pub fn new(data: &[u8], bits: usize) -> Self {
        let mut buffer = data[..bits.div_ceil(8).min(data.len())].to_vec();
        buffer.resize(bits.div_ceil(8), 0);
        BitReader { buffer, num: bits, pos: 0, error: false, critical: false, max_serialize: 0 }
    }

    /// FBitReader::AtEnd.
    pub fn at_end(&self) -> bool {
        self.error || self.pos == self.num
    }

    /// FBitReader::ReadBit.
    pub fn read_bit(&mut self) -> bool {
        if self.pos + 1 > self.num {
            self.error = true;
            return false;
        }
        let bit = self.buffer[self.pos >> 3] & (1 << (self.pos & 7)) != 0;
        self.pos += 1;
        bit
    }

    pub fn read_int(&mut self, max: u32) -> u32 {
        let mut value: u32 = 0;
        if max <= 1 {
            return 0;
        }
        let mut mask: u32 = 1;
        while mask != 0 {
            if self.num <= self.pos {
                self.error = true;
                return value;
            }
            if self.buffer[self.pos >> 3] & (1 << (self.pos & 7)) != 0 {
                value |= mask;
            }
            mask = mask.wrapping_mul(2);
            self.pos += 1;
            if max <= value.wrapping_add(mask) {
                return value;
            }
        }
        value
    }

    /// FBitReader::SerializeBits: `bits` bits into a zeroed buffer.
    pub fn read_bits(&mut self, bits: usize) -> Vec<u8> {
        let mut out = vec![0u8; bits.div_ceil(8)];
        if self.pos + bits > self.num {
            self.error = true;
            return out;
        }
        for i in 0..bits {
            if self.buffer[self.pos >> 3] & (1 << (self.pos & 7)) != 0 {
                out[i >> 3] |= 1 << (i & 7);
            }
            self.pos += 1;
        }
        out
    }

    /// FBitReader::Serialize.
    pub fn read_bytes(&mut self, n: usize) -> Vec<u8> {
        self.read_bits(n * 8)
    }

    /// FBitReader::SetData: the next `bits` bits of `src` become this stream.
    pub fn sub_reader(&mut self, bits: usize) -> BitReader {
        let data = self.read_bits(bits);
        BitReader::new(&data, bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_round_trip() {
        let mut w = BitWriter::new(256);
        for (v, m) in [(0u32, 0x4000u32), (5, 0x3FF), (1023, 1024), (7, 8), (300, 0x4000)] {
            w.write_int(v, m);
        }
        w.write_bit(true);
        let mut r = BitReader::new(&w.buffer, w.num);
        for (v, m) in [(0u32, 0x4000u32), (5, 0x3FF), (1023, 1024), (7, 8), (300, 0x4000)] {
            assert_eq!(r.read_int(m), v);
        }
        assert!(r.read_bit());
        assert_eq!(ceil_log_two(0x4000), 14);
        assert_eq!(ceil_log_two(0x3FF), 10);
    }
}
