//! Mods build only (feature `mods`): the mods' own sounds (`[sounds.<Name>]`, WAV or OGG files),
//! docs/modding/README.md, "Your own sounds".
//!
//! - A new sound "Mod.<Name>" is a plain wave resource of the audio engine, registered when a
//!   level's sounds are loaded, with its own resource and wave ids; `audio_sound_ref` finds it
//!   by name, so it works wherever a sound name is accepted (class values, added actors,
//!   characters, actions).
//! - `replace` makes the waves of a game sound play the mod's samples instead.
//!
//! The audio engine only times and positions the voices; the samples are mixed by the game
//! window, which asks `mod_wave_pcm` for a mod wave before the game's banks. The game's own
//! timing of a replaced sound (its original length) is kept.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use openxiii_assets::audio::Pcm;
use openxiii_assets::hx::Cuuid;
use openxiii_hx::engine::{ResData, WaveInfo};
use openxiii_hx::subsystem::SoundRef;
use openxiii_mods::own_sounds::{decode, OwnSound};

use crate::Vm;

/// A mod sound, decoded.
#[derive(Debug, Clone)]
pub struct OwnLoaded {
    pub def: OwnSound,
    pub pcm: Arc<Pcm>,
    /// Ids of its resource and wave in the audio engine.
    pub resource: Cuuid,
    pub wave: Cuuid,
}

static OWN: OnceLock<Arc<Vec<OwnLoaded>>> = OnceLock::new();
/// Samples of the mods' waves (their own, and the replaced waves of game sounds), by wave.
static PCM: Mutex<Option<HashMap<Cuuid, Arc<Pcm>>>> = Mutex::new(None);

/// Ids of mod sound `i`: a range no game bank uses (the game's ids are random 32-bit pairs
/// drawn by the HX library; 0xFEED… is never drawn for both halves together).
fn ids(i: usize) -> (Cuuid, Cuuid) {
    (Cuuid(0xFEED_0000 | (i as u32 * 2), 0x4D4F_4453), Cuuid(0xFEED_0000 | (i as u32 * 2 + 1), 0x4D4F_4453))
}

/// Decodes the mods' own sounds (once, at startup) and returns one line per problem.
pub fn set_mod_own_sounds(list: Vec<OwnSound>) -> Vec<String> {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for def in list {
        if out.iter().any(|o: &OwnLoaded| o.def.name.eq_ignore_ascii_case(&def.name)) {
            problems.push(format!("warning: sound {} defined twice, the first one is kept ({})", def.sound_name(), def.source));
            continue;
        }
        match decode(&def.file) {
            Ok(d) => {
                let (resource, wave) = ids(out.len());
                let pcm = Arc::new(Pcm { rate: d.rate, channels: d.channels, samples: d.samples });
                remember_pcm(wave, pcm.clone());
                out.push(OwnLoaded { def, pcm, resource, wave });
            }
            Err(e) => problems.push(format!("warning: sound {}: {e}; sound skipped ({})", def.sound_name(), def.source)),
        }
    }
    let _ = OWN.set(Arc::new(out));
    problems
}

pub(crate) fn global_own_sounds() -> Option<Arc<Vec<OwnLoaded>>> {
    OWN.get().cloned().filter(|l| !l.is_empty())
}

fn remember_pcm(wave: Cuuid, pcm: Arc<Pcm>) {
    let mut g = PCM.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(wave, pcm);
}

/// Samples of wave `wave` if it is a mod's (own sound, or a replaced wave of a game sound).
pub fn mod_wave_pcm(wave: Cuuid) -> Option<Arc<Pcm>> {
    PCM.lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(&wave).cloned()
}

impl Vm {
    /// Own sounds of this Vm only (tests): decoded now; problems returned.
    pub fn set_mod_own_sounds_here(&mut self, list: Vec<OwnSound>) -> Vec<String> {
        let mut out = Vec::new();
        let mut problems = Vec::new();
        for (i, def) in list.into_iter().enumerate() {
            match decode(&def.file) {
                Ok(d) => {
                    let (resource, wave) = ids(1000 + i);
                    let pcm = Arc::new(Pcm { rate: d.rate, channels: d.channels, samples: d.samples });
                    remember_pcm(wave, pcm.clone());
                    out.push(OwnLoaded { def, pcm, resource, wave });
                }
                Err(e) => problems.push(e),
            }
        }
        self.mods.own_sounds = Some(Arc::new(out)).filter(|l| !l.is_empty());
        problems
    }

    /// audio_sound_ref hook: a mod sound "Mod.<Name>".
    pub(crate) fn mod_sound_ref(&self, path: &str) -> Option<SoundRef> {
        let name = path.strip_prefix(openxiii_mods::own_sounds::PREFIX)?;
        let own = self.mods.own_sounds.as_ref()?;
        let s = own.iter().find(|o| o.def.name.eq_ignore_ascii_case(name))?;
        Some(SoundRef { path: path.to_string(), name: name.to_string(), cuuid: s.resource, handler: false })
    }

    /// End of a level's sound loading: the mods' own sounds become resources of the audio
    /// engine, and the waves of the game sounds they replace play their samples.
    pub(crate) fn mod_register_own_sounds(&mut self) {
        let Some(own) = self.mods.own_sounds.clone() else { return };
        // A plain sound of the game as the model of the resource (volume, positioned…).
        let template = self.audio.as_ref().and_then(|h| {
            let mut wavs: Vec<(&Cuuid, &ResData)> = h.engine.registry.iter().filter(|(_, r)| matches!(r, ResData::Wav(w) if w.flags & 8 != 0 && w.languages.is_empty())).collect();
            wavs.sort_by_key(|(id, _)| **id);
            wavs.first().and_then(|(_, r)| if let ResData::Wav(w) = r { Some((**w).clone()) } else { None })
        });
        let Some(template) = template else {
            if self.audio.is_some() {
                self.mod_report("warning: own sounds: the level has no plain sound to model them on; they are left out on this level".into());
            }
            return;
        };
        for o in own.iter() {
            if let Some(hx) = self.audio.as_mut() {
                if !hx.engine.registry.contains_key(&o.resource) {
                    let mut w = template.clone();
                    w.cuuid = o.resource;
                    w.name = o.def.sound_name();
                    w.wave = o.wave;
                    w.languages.clear();
                    hx.engine.registry.insert(o.resource, ResData::Wav(Rc::new(w)));
                    let frames = (o.pcm.samples.len() / o.pcm.channels.max(1) as usize) as u64;
                    let info = WaveInfo {
                        cuuid: o.wave,
                        gain: 0.0,
                        streamed: false,
                        frames,
                        rate: o.pcm.rate,
                        markers: Vec::new(),
                        source: None,
                        data_size: (o.pcm.samples.len() * 2) as u32,
                        format_tag: 1,
                        channels: o.pcm.channels,
                    };
                    hx.engine.waves.insert(o.wave, Rc::new(info));
                }
            }
            if let Some(game) = &o.def.replace {
                let game = crate::mod_sounds::object_path(game).to_string();
                if let Err(why) = self.mod_ensure_sound(&game, &format!("sound {} replaced by {}", game, o.def.sound_name())) {
                    self.mod_report(format!("warning: {} replaces {game}: {why} ({})", o.def.sound_name(), o.def.source));
                    continue;
                }
                let Some(id) = self.audio_sound_ref(&game).map(|s| s.cuuid) else { continue };
                let Some(dir) = self.mod_bank_dir() else { continue };
                let waves = crate::mod_sounds::bank_index(&dir).waves_of(id);
                for w in &waves {
                    remember_pcm(*w, o.pcm.clone());
                }
                self.mod_report(format!("{} plays instead of {game} ({} wave(s)) ({})", o.def.sound_name(), waves.len(), o.def.source));
            }
        }
    }
}
