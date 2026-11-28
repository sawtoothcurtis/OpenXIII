//! A mod's own sounds: `[sounds.<Name>]` tables, read from WAV or OGG files of the mod folder.
//!
//! ```toml
//! [sounds.Beep]                 # a new sound, named "Mod.Beep" wherever a sound is accepted
//! file = "sounds/beep.wav"      # WAV (PCM 8/16/24/32-bit or float) or OGG Vorbis
//!
//! [sounds.Pistol]
//! file = "sounds/pistol.ogg"
//! replace = "XIIIsound.Guns__9mmFire.9mmFire__h9mmFire"   # also plays instead of this game sound
//! ```

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// Prefix of the names of the mods' own sounds.
pub const PREFIX: &str = "Mod.";

/// A `[sounds.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnSound {
    /// Name as written ("Beep"); the sound is "Mod.Beep".
    pub name: String,
    /// Audio file, inside the mod folder.
    pub file: PathBuf,
    /// Game sound this one plays instead of, if any.
    pub replace: Option<String>,
    pub source: Source,
}

impl OwnSound {
    /// Full sound name: "Mod.<Name>".
    pub fn sound_name(&self) -> String {
        format!("{PREFIX}{}", self.name)
    }
}

/// Decoded samples: interleaved signed 16-bit.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

pub(crate) type SoundTables = BTreeMap<String, Spanned<toml::Table>>;

/// Reads the `[sounds.<Name>]` tables of a file of the mod in folder `mod_dir`. A wrong table
/// gives an error (with its line) and is skipped.
pub(crate) fn read(text: &str, file: &Path, mod_name: &str, mod_dir: &Path, tables: SoundTables, out: &mut Vec<OwnSound>, problems: &mut Vec<Problem>) {
    for (name, t) in tables {
        let line = line_of(text, t.span().start);
        let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: format!("sounds.{name}: {m}; sound skipped") };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            problems.push(err("the name may only use letters, digits, _ and -".into()));
            continue;
        }
        let t = t.get_ref();
        if let Some(k) = t.keys().find(|k| !["file", "replace"].contains(&k.as_str())) {
            problems.push(err(format!("unknown key {k} (keys: file, replace)")));
            continue;
        }
        let Some(toml::Value::String(f)) = t.get("file") else {
            problems.push(err("file = \"path inside the mod folder\" is missing".into()));
            continue;
        };
        let rel = Path::new(f);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            problems.push(err(format!("file {f:?} must be inside the mod folder")));
            continue;
        }
        let replace = match t.get("replace") {
            None => None,
            Some(toml::Value::String(r)) if r.contains('.') => Some(r.trim().to_string()),
            Some(v) => {
                problems.push(err(format!("replace = {v} must be a game sound \"Package.Group.Name\"")));
                continue;
            }
        };
        out.push(OwnSound { name, file: crate::file_in_mod(mod_dir, f), replace, source: Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line } });
    }
}

/// Decoded samples of an OGG file at most (10 minutes of 48 kHz stereo): a small compressed file
/// can decode to far more samples than its size, so the decoding stops there.
pub const MAX_OGG_SAMPLES: usize = 48_000 * 2 * 600;

/// Adds `packet` to `samples` unless that makes more than `max` samples (false then, nothing added).
pub(crate) fn push_capped(samples: &mut Vec<i16>, packet: Vec<i16>, max: usize) -> bool {
    if samples.len() + packet.len() > max {
        return false;
    }
    samples.extend(packet);
    true
}

/// Decodes a WAV or OGG Vorbis file into 16-bit samples.
pub fn decode(path: &Path) -> Result<Decoded, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "wav" => decode_wav(path),
        "ogg" => decode_ogg(path),
        _ => Err(format!("{}: only .wav and .ogg files are supported", path.display())),
    }
}

fn decode_wav(path: &Path) -> Result<Decoded, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = r.spec();
    let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => r.samples::<f32>().map(|s| s.map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)).collect::<Result<_, _>>(),
        (hound::SampleFormat::Int, 8) => r.samples::<i8>().map(|s| s.map(|v| (v as i16) << 8)).collect::<Result<_, _>>(),
        (hound::SampleFormat::Int, 16) => r.samples::<i16>().collect::<Result<_, _>>(),
        // 9 to 15 bits are scaled up, more than 16 down.
        (hound::SampleFormat::Int, b) => r.samples::<i32>().map(|s| s.map(|v| if b >= 16 { (v >> (b as u32 - 16)) as i16 } else { (v << (16 - b as u32)) as i16 })).collect::<Result<_, _>>(),
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    if spec.channels == 0 || spec.channels > 2 {
        return Err(format!("{}: {} channels, only mono and stereo are supported", path.display(), spec.channels));
    }
    Ok(Decoded { rate: spec.sample_rate, channels: spec.channels, samples })
}

fn decode_ogg(path: &Path) -> Result<Decoded, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut r = lewton::inside_ogg::OggStreamReader::new(f).map_err(|e| format!("{}: {e}", path.display()))?;
    let (rate, channels) = (r.ident_hdr.audio_sample_rate, r.ident_hdr.audio_channels as u16);
    if channels == 0 || channels > 2 {
        return Err(format!("{}: {channels} channels, only mono and stereo are supported", path.display()));
    }
    let mut samples = Vec::new();
    while let Some(packet) = r.read_dec_packet_itl().map_err(|e| format!("{}: {e}", path.display()))? {
        if !push_capped(&mut samples, packet, MAX_OGG_SAMPLES) {
            return Err(format!("{}: longer than {MAX_OGG_SAMPLES} samples (10 minutes of 48 kHz stereo)", path.display()));
        }
    }
    Ok(Decoded { rate, channels, samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::parse_mod_file;

    #[test]
    fn tables_and_errors() {
        let text = "[sounds.Beep]\nfile = \"sounds/beep.wav\"\n\n[sounds.Pistol]\nfile = \"pistol.ogg\"\nreplace = \"XIIIsound.Guns__9mmFire.9mmFire__h9mmFire\"\n\n[sounds.Bad]\nfile = \"../outside.wav\"\n\n[sounds.Worse]\nreplace = \"XIIIsound.A.B\"\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert_eq!(f.sounds.len(), 2, "{:?}", f.sounds);
        let beep = f.sounds.iter().find(|s| s.name == "Beep").unwrap();
        assert_eq!(beep.sound_name(), "Mod.Beep");
        assert_eq!(beep.file, Path::new("m").join("sounds/beep.wav"));
        assert_eq!(beep.replace, None);
        assert_eq!(beep.source.line, 1);
        let pistol = f.sounds.iter().find(|s| s.name == "Pistol").unwrap();
        assert_eq!(pistol.replace.as_deref(), Some("XIIIsound.Guns__9mmFire.9mmFire__h9mmFire"));
        let all: Vec<String> = f.problems.iter().map(|p| p.to_string()).collect();
        assert!(all.iter().any(|p| p.contains("(line 8)") && p.contains("inside the mod folder")), "{all:?}");
        assert!(all.iter().any(|p| p.contains("(line 11)") && p.contains("file = ")), "{all:?}");
    }

    #[test]
    fn decode_wav_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tone.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 22050, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..2205 {
            w.write_sample(((i as f32 * 0.1).dsin() * 10000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        let d = decode(&path).unwrap();
        assert_eq!((d.rate, d.channels, d.samples.len()), (22050, 1, 2205));
        assert_eq!(d.samples[10], ((10.0f32 * 0.1).dsin() * 10000.0) as i16);
        let bad = tmp.path().join("noise.mp3");
        std::fs::write(&bad, b"x").unwrap();
        assert!(decode(&bad).unwrap_err().contains("only .wav and .ogg"));
        let broken = tmp.path().join("broken.wav");
        std::fs::write(&broken, b"RIFF....").unwrap();
        assert!(decode(&broken).is_err());
    }
}
