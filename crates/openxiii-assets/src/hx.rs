//! Ubisoft HX audio banks (HXAudio engine): `.hxc` (index + headers, internal
//! sounds) and `.hsc` (external streams). Little-endian on PC.
//!
//! Layout (format documented by vgmstream, `meta/ubi_hx.c`):
//! - u32 index offset; at that offset: "INDX", u32 type, u32 count.
//! - Index entry: class (u32 + text), CUUID (2 × u32), header offset and size,
//!   unknown u32, links (u32 n + n × CUUID), languages (u32 n + n × 16 bytes).
//! - Header: class, CUUID, then class-specific data.
//!   `CPCWavResData` carries the sound's name and a link to a `CPCWaveFileIdObj`,
//!   which contains a pseudo-RIFF ("fmt", internal "data" or external "datx").
//!
//! HXAudio: reading of CPCWaveFileIdObj (internal samples, on
//! demand), of CWaveFileIdObj, of the CUUID, of
//! the identifier and of a CResData.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug)]
pub struct HxError(pub String);

impl fmt::Display for HxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HxError {}

type Result<T> = std::result::Result<T, HxError>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(HxError(msg.into()))
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        match self.data.get(self.pos..self.pos.saturating_add(n)) {
            Some(b) => {
                self.pos += n;
                Ok(b)
            }
            None => err(format!("end of data at 0x{:X} ({n} bytes)", self.pos)),
        }
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        if len > 0x1000 {
            return err(format!("string too long at 0x{:X}", self.pos - 4));
        }
        let b = self.bytes(len)?;
        Ok(b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect())
    }
    fn cuuid(&mut self) -> Result<Cuuid> {
        Ok(Cuuid(self.u32()?, self.u32()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cuuid(pub u32, pub u32);

impl fmt::Display for Cuuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}-{:08x}", self.0, self.1)
    }
}

#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub class: String,
    pub cuuid: Cuuid,
    pub offset: u32,
    pub size: u32,
    pub links: Vec<Cuuid>,
    pub languages: Vec<[u32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Codec {
    Pcm16,
    UbiAdpcm,
    Psx,
    Dsp,
    XboxImaAdpcm,
    Mp3,
    Other(u16),
}

impl From<u16> for Codec {
    fn from(v: u16) -> Self {
        match v {
            1 => Codec::Pcm16,
            2 => Codec::UbiAdpcm,
            3 => Codec::Psx,
            4 => Codec::Dsp,
            5 => Codec::XboxImaAdpcm,
            0x55 => Codec::Mp3,
            n => Codec::Other(n),
        }
    }
}

/// Location of a sound's samples.
#[derive(Debug, Clone)]
pub enum Location {
    /// Inside the .hxc itself (absolute position, size).
    Internal { offset: u32, size: u32 },
    /// In an external file (often the .hsc of the same name).
    External { resource: String, offset: u32, size: u32 },
}

#[derive(Debug, Clone)]
pub struct Wave {
    pub cuuid: Cuuid,
    /// Name taken from the `CPCWavResData` that points to this wave (if found).
    pub name: Option<String>,
    /// CUUID of that `CPCWavResData` (referenced by the Sound objects of the .uax files).
    pub resource: Option<Cuuid>,
    /// Language of the variant ("fr", "en", "de", "es", "it"), if dubbed.
    pub language: Option<String>,
    pub codec: Codec,
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u16,
    pub bits: u16,
    pub mode: u8,
    pub location: Location,
    /// Wave markers ("cue" and "labl" chunks): (position in frames, name), sorted by position.
    pub markers: Vec<(u32, String)>,
    /// "ump3" chunk (Ubi ADPCM): the second word is the duration in frames that HXAudio's
    /// streaming decoder allows itself to read.
    pub ump3: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct HxBank {
    pub index_type: u32,
    pub entries: Vec<IndexEntry>,
    pub waves: Vec<Wave>,
    /// Entries whose header could not be read (class, message).
    pub failures: Vec<(String, String)>,
}

impl HxBank {
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::parse_inner(data, None)
    }

    /// Also reads the external WAVE headers of non-streamed waves
    /// (CPCWaveFileIdObj::Read). Their format comes from the file,
    /// not from the index's RIFF. `parse` alone has no access to the disk for this.
    pub fn parse_with_path(data: &[u8], bank_path: &std::path::Path) -> Result<Self> {
        Self::parse_inner(data, Some(bank_path))
    }

    fn parse_inner(data: &[u8], bank_path: Option<&std::path::Path>) -> Result<Self> {
        let mut c = Cursor { data, pos: 0 };
        let index = c.u32()? as usize;
        c.pos = index;
        if c.bytes(4)? != b"INDX" {
            return err(format!("\"INDX\" index missing at 0x{index:X}"));
        }
        let index_type = c.u32()?;
        let count = c.u32()?;
        if index_type == 1 {
            return err("index of type 1 (without links) not supported");
        }
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let class = c.string()?;
            let cuuid = c.cuuid()?;
            let offset = c.u32()?;
            let size = c.u32()?;
            let _unknown = c.u32()?;
            let links = (0..c.u32()?).map(|_| c.cuuid()).collect::<Result<Vec<_>>>()?;
            let languages = (0..c.u32()?).map(|_| Ok([c.u32()?, c.u32()?, c.u32()?, c.u32()?])).collect::<Result<Vec<_>>>()?;
            entries.push(IndexEntry { class, cuuid, offset, size, links, languages });
        }

        // Names: each WavResData names the waves it points to.
        let mut names: HashMap<Cuuid, (String, Cuuid, Option<String>)> = HashMap::new();
        for e in entries.iter().filter(|e| e.class.ends_with("WavResData")) {
            if let Ok(name) = wavres_name(data, e) {
                // Direct links, then per-language variants: ASCII u32 code ("fr  "), ?, CUUID.
                let direct = e.links.iter().map(|l| (*l, None));
                let langs = e.languages.iter().map(|l| (Cuuid(l[2], l[3]), Some(language_code(l[0]))));
                for (link, lang) in direct.chain(langs) {
                    names.entry(link).or_insert_with(|| (name.clone(), e.cuuid, lang));
                }
            }
        }

        let mut waves = Vec::new();
        let mut failures = Vec::new();
        for e in entries.iter().filter(|e| e.class.ends_with("WaveFileIdObj")) {
            match parse_wave(data, e, bank_path) {
                Ok(mut w) => {
                    if let Some((name, resource, lang)) = names.get(&e.cuuid) {
                        w.name = Some(name.clone());
                        w.resource = Some(*resource);
                        w.language = lang.clone();
                    }
                    waves.push(w);
                }
                Err(x) => failures.push((e.class.clone(), x.0)),
            }
        }
        Ok(HxBank { index_type, entries, waves, failures })
    }
}

fn language_code(v: u32) -> String {
    v.to_be_bytes().iter().filter(|b| b.is_ascii_alphanumeric()).map(|&b| b as char).collect()
}

fn wavres_name(data: &[u8], e: &IndexEntry) -> Result<String> {
    let mut c = Cursor { data, pos: e.offset as usize };
    c.string()?;
    c.cuuid()?;
    c.u32()?; // flags
    c.string()
}

fn parse_wave(data: &[u8], e: &IndexEntry, bank_path: Option<&std::path::Path>) -> Result<Wave> {
    let mut c = Cursor { data, pos: e.offset as usize };
    let class = c.string()?;
    let cuuid = c.cuuid()?;
    if class != "CPCWaveFileIdObj" {
        return err(format!("wave class not supported: {class}"));
    }
    let version = c.u32()?;
    let _gain = c.u32()?;
    // Before v3 two BOOL32, then a bit byte.
    let mode = if version < 3 {
        u8::from(c.u32()? != 0) | (u8::from(c.u32()? != 0) << 1)
    } else {
        c.u8()?
    };
    let adjust = if mode & 8 != 0 { c.u32()? } else { 0 };
    let resource = if mode & (1 | 8) != 0 { Some(c.string()?) } else { None };
    if mode & 8 == 0 && mode & 1 != 0 && version < 2 {
        c.u32()?; // old offset read then reset to zero.
    }
    // Without the stream bit, the reader opens the external WAVE
    // from its start and actually reads its format and data position.
    let external_data = if mode & (1 | 2 | 8) == 1 {
        let bank_path = bank_path.ok_or_else(|| HxError("external WAVE wave: bank path required".into()))?;
        let resource = resource.as_deref().unwrap();
        let name = std::path::Path::new(resource).file_name().ok_or_else(|| HxError("external WAVE name missing".into()))?;
        let path = bank_path.with_file_name(name);
        let bytes = std::fs::read(&path).map_err(|e| HxError(format!("{}: {e}", path.display())))?;
        openxiii_package::events::file_read(&path, bytes.len());
        Some(bytes)
    } else {
        None
    };
    if let Some(bytes) = external_data.as_deref() {
        c = Cursor { data: bytes, pos: 0 };
    }
    let source_len = c.data.len();
    let riff = c.pos;
    if c.bytes(4)? != b"RIFF" {
        return err(format!("RIFF missing at 0x{riff:X}"));
    }
    let riff_end = riff + 8 + c.u32()? as usize;
    c.bytes(4)?; // "WAVE"
    let (mut format, mut data_chunk, mut datx) = (None, None, None);
    let mut cues: Vec<(u32, u32)> = Vec::new();
    let mut labels: HashMap<u32, String> = HashMap::new();
    let mut ump3 = None;
    while c.pos + 8 <= riff_end.min(source_len) {
        let id: [u8; 4] = c.bytes(4)?.try_into().unwrap();
        let size = c.u32()? as usize;
        let body = c.pos;
        match &id {
            b"fmt " => {
                let codec = c.u16()?;
                let channels = c.u16()?;
                let rate = c.u32()?;
                let _bytes_per_sec = c.u32()?;
                let align = c.u16()?;
                let bits = c.u16()?;
                format = Some((codec, channels, rate, align, bits));
            }
            b"data" => {
                // Internal: size sometimes inconsistent, clamped to the end of the RIFF.
                let size = if size == 0 || body + size > riff_end { riff_end - body } else { size };
                data_chunk = Some((body as u32, size as u32));
            }
            b"datx" => datx = Some((c.u32()?, c.u32()? + adjust)),
            b"ump3" if size >= 8 => ump3 = Some((c.u32()?, c.u32()?)),
            b"cue " => {
                // The marker uses dwSampleOffset, the last word
                // of the cue, not dwPosition (second word).
                let n = c.u32()?;
                for _ in 0..n {
                    let id = c.u32()?;
                    c.u32()?; // dwPosition
                    c.bytes(12)?; // fccChunk, dwChunkStart, dwBlockStart
                    let pos = c.u32()?;
                    cues.push((id, pos));
                }
            }
            b"LIST" => {
                // "adtl": "labl" labels (marker identifier, text).
                if c.bytes(4)? == b"adtl" {
                    let end = body + size;
                    while c.pos + 8 <= end {
                        let sub: [u8; 4] = c.bytes(4)?.try_into().unwrap();
                        let len = c.u32()? as usize;
                        let start = c.pos;
                        if &sub == b"labl" && len >= 4 {
                            let id = c.u32()?;
                            let text = c.bytes(len - 4)?;
                            // Latin-1 non-breaking space -> space.
                            labels.insert(id, text.iter().take_while(|&&b| b != 0).map(|&b| if b == 0xa0 { ' ' } else { b as char }).collect());
                        }
                        c.pos = start + len + (len & 1);
                    }
                }
            }
            _ => {}
        }
        c.pos = body + size + (size & 1);
    }
    let (codec, channels, sample_rate, block_align, bits) = format.ok_or_else(|| HxError("\"fmt\" chunk missing".into()))?;
    let location = match (resource, datx, data_chunk) {
        (Some(resource), Some((size, offset)), _) => Location::External { resource, offset, size },
        (Some(resource), None, Some((offset, size))) if mode & 2 == 0 => Location::External { resource, offset, size },
        (None, _, Some((offset, size))) => Location::Internal { offset, size },
        _ => return err("\"data\"/\"datx\" chunk missing"),
    };
    let mut markers: Vec<(u32, String)> = cues.into_iter().map(|(id, pos)| (pos, labels.remove(&id).unwrap_or_default())).collect();
    markers.sort_by_key(|m| m.0);
    Ok(Wave { cuuid, name: None, resource: None, language: None, codec: codec.into(), channels, sample_rate, block_align, bits, mode, location, markers, ump3 })
}

/// Writes 16-bit PCM samples to a WAV file.
pub fn write_wav(path: &std::path::Path, channels: u16, rate: u32, pcm: &[u8]) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    std::fs::write(path, out)
}

/// USound::UuidToQword: 16 hex digits read from the first 18 bytes of the
/// text "XXXXXXXX-XXXX-XXXX" (positions 8 and 13 skipped, whatever they contain); a byte that
/// is not a digit (signed bytes: '0'-'9', 'a'-'f', 'A'-'F') still shifts by 4 bits without
/// adding anything. Past the text: zero bytes.
pub fn uuid_to_qword(t: &[u8]) -> Cuuid {
    let mut q: u64 = 0;
    for i in 0..0x12usize {
        if i == 8 || i == 13 {
            continue;
        }
        let c = t.get(i).copied().unwrap_or(0) as i8;
        q <<= 4;
        q = q.wrapping_add(match c {
            0x30..=0x39 => (c - 0x30) as u64,
            0x61..=0x66 => (c - 0x57) as u64,
            0x41..=0x46 => (c - 0x37) as u64,
            _ => 0,
        });
    }
    Cuuid((q >> 32) as u32, q as u32)
}

/// Sound object of a package (.uax): file type ("UHX") and identifier of the
/// HX resource, written as text "XXXXXXXX-XXXX-XXXX" (CUUID of the `CPCWavResData`); USound::Serialize
pub fn sound_reference(
    package: &openxiii_package::Package,
    data: &[u8],
    index: usize,
) -> std::result::Result<(String, Option<Cuuid>), openxiii_package::Error> {
    let object = package.object_data(data, index)?;
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = openxiii_package::Reader::new(&data[..end], object.end);
    let file_type = package.name(r.compact()?).to_string();
    let _skip = r.u32()?;
    let len = r.compact()?.max(0) as usize;
    let text = r.take(len)?;
    let id = (text.len() >= 18).then(|| uuid_to_qword(text));
    Ok((file_type, id))
}

