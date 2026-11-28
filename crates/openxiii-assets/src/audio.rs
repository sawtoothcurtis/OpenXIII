//! Game sounds ready to play: a map's HX banks (and the shared Fix bank), PCM samples
//! decoded on demand, package Sound objects resolved to their HX resource.

use crate::hx::{Codec, Cuuid, HxBank, Location};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Interleaved 16-bit PCM samples.
#[derive(Debug, Clone)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

struct BankFile {
    path: PathBuf,
    data: Vec<u8>,
    bank: HxBank,
}

pub struct SoundSet {
    banks: Vec<BankFile>,
    /// Resource (CPCWavResData) -> (bank, waves: (index, language)).
    resources: HashMap<Cuuid, (usize, Vec<(usize, Option<String>)>)>,
    /// Sound program (CProgramResData) -> resources it uses.
    programs: HashMap<Cuuid, Vec<Cuuid>>,
    cache: HashMap<(Cuuid, usize), Option<std::sync::Arc<Pcm>>>,
    /// Wave (`CPCWaveFileIdObj`) → (bank, wave index).
    wave_index: HashMap<Cuuid, (usize, usize)>,
    /// Preferred voice language ("fr", "en"…).
    pub language: String,
}

impl SoundSet {
    /// Loads banks `names` (without extension) from `<game>/Sounds/PC`.
    pub fn load(game: &Path, names: &[&str]) -> SoundSet {
        let dir = game.join("Sounds").join("PC");
        let mut set = SoundSet {
            banks: Vec::new(),
            resources: HashMap::new(),
            programs: HashMap::new(),
            cache: HashMap::new(),
            wave_index: HashMap::new(),
            language: "fr".into(),
        };
        for name in names {
            let path = dir.join(format!("{name}.hxc"));
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            let Ok(bank) = HxBank::parse_with_path(&data, &path) else {
                continue;
            };
            let b = set.banks.len();
            for (i, w) in bank.waves.iter().enumerate() {
                set.wave_index.entry(w.cuuid).or_insert((b, i));
                if let Some(r) = w.resource {
                    let entry = set.resources.entry(r).or_insert((b, Vec::new()));
                    if entry.0 == b {
                        entry.1.push((i, w.language.clone()));
                    }
                }
            }
            for e in bank.entries.iter().filter(|e| e.class == "CProgramResData") {
                set.programs.entry(e.cuuid).or_insert_with(|| e.links.clone());
            }
            set.banks.push(BankFile { path, data, bank });
        }
        set
    }

    pub fn len(&self) -> usize {
        self.resources.len() + self.programs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of samples (frames × channels) of a wave, from its header, without decoding it.
    pub fn wave_samples(&self, wave: Cuuid) -> Option<u64> {
        let &(b, i) = self.wave_index.get(&wave)?;
        let bank = &self.banks[b];
        let w = &bank.bank.waves[i];
        Some(wave_length(&bank.path, &bank.data, w).0 * w.channels.max(1) as u64)
    }

    /// Wave decoded elsewhere (prewarming): stored in the `pcm_wave` cache if not already there.
    pub fn insert_wave(&mut self, wave: Cuuid, pcm: Option<std::sync::Arc<Pcm>>) {
        self.cache.entry((wave, usize::MAX)).or_insert(pcm);
    }

    /// Wave already in the `pcm_wave` cache (decoded or missing).
    pub fn has_wave(&self, wave: Cuuid) -> bool {
        self.cache.contains_key(&(wave, usize::MAX))
    }

    /// Waves of the loaded banks (CUUID of `CPCWaveFileIdObj`), in a stable order.
    pub fn waves(&self) -> Vec<Cuuid> {
        let mut v: Vec<(usize, usize, Cuuid)> = self.wave_index.iter().map(|(c, &(b, i))| (b, i, *c)).collect();
        v.sort_unstable_by_key(|e| (e.0, e.1));
        v.into_iter().map(|e| e.2).collect()
    }

    /// Samples of sound `id` (resource or program). For a program, `choice` picks among
    /// the sounds it uses (approximation: the HX program interpreter is not written yet).
    pub fn pcm(&mut self, id: Cuuid, choice: usize) -> Option<std::sync::Arc<Pcm>> {
        let resource = match self.programs.get(&id) {
            Some(links) => {
                let usable: Vec<Cuuid> = links.iter().copied().filter(|l| self.resources.contains_key(l)).collect();
                if usable.is_empty() {
                    return None;
                }
                usable[choice % usable.len()]
            }
            None => id,
        };
        if let Some(p) = self.cache.get(&(resource, 0)) {
            return p.clone();
        }
        let pcm = self.decode(resource).map(std::sync::Arc::new);
        self.cache.insert((resource, 0), pcm.clone());
        pcm
    }

    /// Samples of a wave (`CPCWaveFileIdObj`) by its CUUID.
    pub fn pcm_wave(&mut self, wave: Cuuid) -> Option<std::sync::Arc<Pcm>> {
        let key = (wave, usize::MAX);
        if let Some(p) = self.cache.get(&key) {
            return p.clone();
        }
        let pcm = self.wave_index.get(&wave).copied().and_then(|(b, i)| self.decode_wave(b, i)).map(std::sync::Arc::new);
        self.cache.insert(key, pcm.clone());
        pcm
    }

    fn decode(&self, resource: Cuuid) -> Option<Pcm> {
        let (b, waves) = self.resources.get(&resource)?;
        // Variant in the desired language, otherwise without language, otherwise the first.
        let pick =
            waves.iter().find(|(_, l)| l.as_deref() == Some(self.language.as_str())).or_else(|| waves.iter().find(|(_, l)| l.is_none())).or(waves.first())?;
        self.decode_wave(*b, pick.0)
    }

    fn decode_wave(&self, b: usize, index: usize) -> Option<Pcm> {
        let bank = &self.banks[b];
        let mut ch = openxiii_package::loadings::start_value(openxiii_package::loadings::SOUND);
        ch.name_value(|| format!("{}#{index}", bank.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        let w = &bank.bank.waves[index];
        let bytes: Vec<u8> = match &w.location {
            Location::Internal { offset, size } => bank.data.get(*offset as usize..(*offset + *size) as usize)?.to_vec(),
            Location::External { resource, offset, size } => {
                use std::io::{Read, Seek, SeekFrom};
                let name = Path::new(resource).file_name()?;
                let mut f = std::fs::File::open(bank.path.parent()?.join(name)).ok()?;
                f.seek(SeekFrom::Start(*offset as u64)).ok()?;
                let mut buf = vec![0u8; *size as usize];
                f.read_exact(&mut buf).ok()?;
                buf
            }
        };
        match w.codec {
            Codec::Pcm16 => Some(Pcm {
                rate: w.sample_rate,
                channels: w.channels.max(1),
                samples: bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
            }),
            Codec::UbiAdpcm => {
                // Fills the wave through HX's stream reader: ump3 block duration,
                // 512-frame chunks and the first-read adjustments.
                let h = crate::ubi_adpcm::read_header(&bytes).ok()?;
                let ch = w.channels.max(1) as usize;
                let frames = w.ump3.map_or(h.sample_count as usize / h.channels.max(1) as usize, |u| u.1 as usize);
                let mut decoder = crate::ubi_stream::StreamDecoder::new(Box::new(std::io::Cursor::new(bytes)), ch);
                // Decoded for the sound output, cached, at a time of its own: not one of the
                // logic's decoders, so freeing it leaves nothing for them (`left_header`).
                decoder.leaves_header = false;
                let mut samples = vec![0; frames * ch];
                let mut got = 0;
                while got < frames {
                    let chunk = (frames - got).min(0x200);
                    let (r, k) = decoder.read(&mut samples[got * ch..(got + chunk) * ch], chunk);
                    got += k;
                    if r == 4 || k == 0 {
                        break;
                    }
                }
                crate::ubi_adpcm::tweaks_first_read(&mut samples, ch, got);
                Some(Pcm { rate: w.sample_rate, channels: ch as u16, samples })
            }
            _ => None,
        }
    }
}

/// Duration of a wave in frames and its sample rate, without decoding it (Ubi ADPCM header read if needed
/// from the external file next to bank `bank_path`).
pub fn wave_length(bank_path: &Path, data: &[u8], w: &crate::hx::Wave) -> (u64, u32) {
    let channels = w.channels.max(1) as u64;
    let size = match &w.location {
        Location::Internal { size, .. } | Location::External { size, .. } => *size as u64,
    };
    match w.codec {
        Codec::Pcm16 => (size / (2 * channels), w.sample_rate),
        Codec::UbiAdpcm => {
            let head: Option<Vec<u8>> = match &w.location {
                Location::Internal { offset, .. } => data.get(*offset as usize..*offset as usize + 0x30).map(|b| b.to_vec()),
                Location::External { resource, offset, .. } => (|| {
                    use std::io::{Read, Seek, SeekFrom};
                    let name = Path::new(resource).file_name()?;
                    let mut f = std::fs::File::open(bank_path.parent()?.join(name)).ok()?;
                    f.seek(SeekFrom::Start(*offset as u64)).ok()?;
                    let mut buf = vec![0u8; 0x30];
                    f.read_exact(&mut buf).ok()?;
                    Some(buf)
                })(),
            };
            match head.and_then(|h| crate::ubi_adpcm::read_header(&h).ok()) {
                Some(h) => (h.sample_count as u64 / h.channels.max(1) as u64, w.sample_rate),
                None => (0, w.sample_rate),
            }
        }
        _ => (0, w.sample_rate),
    }
}

/// CUUID of the HX resource of a Sound object ("Package.Group.Sound").
pub fn sound_id(library: &mut openxiii_package::library::Library, path: &str) -> Option<Cuuid> {
    let (loaded, index) = library.resolve(path)?;
    if loaded.package.export_class(index) != "Sound" {
        return None;
    }
    crate::hx::sound_reference(&loaded.package, &loaded.data, index).ok()?.1
}
