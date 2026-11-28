//! Audio output: the HX engine's voices (openxiii-hx) are played as HXAudio's DirectSound 8 driver
//! sets them (CDS8Channel): volume and pan through HXAudio's
//! 128-step tables (DirectSound hundredths of dB), frequency = wave rate
//! × pitch, clamped to 100…100,000 Hz.

use openxiii_assets::audio::{Pcm, SoundSet};
use openxiii_hx::engine::{Engine, Voice};
use openxiii_hx::stream::StreamHandle;
use rodio::dynamic_mixer::{DynamicMixer, DynamicMixerController};
use crate::audio_output::Output;
use rodio::Source;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Mixer output rate.
const OUT_RATE: u32 = 44100;

/// DirectSound volume (hundredths of dB) for a 0…1 gain in 128 steps (read).
const VOLUME: [i32; 128] = [
    -10000, -4216, -3632, -3284, -3035, -2841, -2682, -2547, -2431, -2327, -2235, -2152, -2075, -2005, -1940, -1880, -1823, -1770, -1720, -1672, -1627, -1584,
    -1543, -1504, -1467, -1431, -1396, -1363, -1331, -1300, -1271, -1242, -1214, -1187, -1160, -1135, -1110, -1086, -1063, -1040, -1017, -996, -974, -954,
    -934, -914, -894, -875, -857, -839, -821, -804, -786, -770, -753, -737, -721, -706, -690, -675, -660, -646, -632, -618, -604, -590, -577, -563, -550, -537,
    -525, -512, -500, -488, -476, -464, -452, -441, -429, -418, -407, -396, -385, -375, -364, -354, -343, -333, -323, -313, -303, -294, -284, -274, -265, -256,
    -247, -237, -228, -219, -211, -202, -193, -185, -176, -168, -159, -151, -143, -135, -127, -119, -111, -103, -95, -87, -80, -72, -65, -57, -50, -43, -35,
    -28, -21, -14, -7, 0,
];

/// DirectSound pan (hundredths of dB) for 0 (left) … 1 (right) in 129 steps (
/// read, applied to the buffer).
const PAN: [i32; 129] = [
    -9999, -8801, -7414, -6603, -6027, -5580, -5214, -4904, -4636, -4398, -4186, -3993, -3817, -3654, -3503, -3362, -3230, -3105, -2987, -2875, -2769, -2667,
    -2569, -2476, -2386, -2299, -2215, -2134, -2056, -1979, -1905, -1833, -1763, -1694, -1627, -1562, -1497, -1435, -1373, -1312, -1253, -1194, -1137, -1080,
    -1024, -968, -914, -860, -806, -754, -701, -649, -598, -547, -496, -445, -395, -345, -296, -246, -197, -147, -98, -49, 0, 49, 98, 147, 197, 246, 296, 345,
    395, 445, 496, 547, 598, 649, 701, 754, 806, 860, 914, 968, 1024, 1080, 1137, 1194, 1253, 1312, 1373, 1435, 1497, 1562, 1627, 1694, 1763, 1833, 1905, 1979,
    2056, 2134, 2215, 2299, 2386, 2476, 2569, 2667, 2769, 2875, 2987, 3105, 3230, 3362, 3503, 3654, 3817, 3993, 4186, 4398, 4636, 4904, 5214, 5580, 6027, 6603,
    7414, 8801, 9999,
];

fn db100(v: i32) -> f32 {
    if v <= -10000 {
        0.0
    } else {
        10f32.powf(v as f32 / 2000.0)
    }
}

/// DirectSound volume of a 0…1 gain (VOLUME[ftol(gain × 127)]); outside 0…1 the original reads past the table, clamped here.
fn ds_volume(gain: f32) -> i32 {
    VOLUME[((gain * 127.0) as i32).clamp(0, 127) as usize]
}

/// Left/right gains of a 0…1 volume and a 0…1 pan (HXAudio's DirectSound tables).
fn ds_gains(volume: f32, pan: f32) -> [f32; 2] {
    let g = volume.clamp(0.0, 1.0);
    let amp = db100(ds_volume(g));
    let pan = ds_pan(pan);
    let left = if pan > 0 { db100(-pan) } else { 1.0 };
    let right = if pan < 0 { db100(pan) } else { 1.0 };
    [amp * left, amp * right]
}

/// DirectSound pan of a 0…1 pan (CDS8Channel: PAN[-ftol(pan × -128)]); outside 0…1 the original reads past the table, clamped here.
fn ds_pan(pan: f32) -> i32 {
    PAN[((pan * 128.0) as i32).clamp(0, 128) as usize]
}

/// Left/right gains and frequency of a voice, as DirectSound would apply them.
fn voice_params(v: &Voice) -> ([f32; 2], f64) {
    let freq = ((v.rate as f64 * v.pitch as f64) as i64).clamp(100, 100_000) as f64;
    (ds_gains(v.gain, v.pan), freq)
}

/// Streamed voices: the engine's DirectSound channel (two-half buffer, thread every 20 ms)
/// is played at the output's pace, which consumes the ring filled by the engine.
struct StreamMix {
    handle: StreamHandle,
    block: Vec<[f32; 2]>,
    at: usize,
    channel: usize,
    gains: [f32; 2],
}

impl Iterator for StreamMix {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        if self.channel == 0 && self.at >= self.block.len() {
            let mut s = self.handle.lock().ok()?;
            if s.chan.released && s.chan.state == 6 {
                return None;
            }
            self.block.clear();
            self.at = 0;
            s.render(&mut self.block, 256, OUT_RATE);
            self.gains = ds_gains(s.chan.volume, s.chan.pan);
        }
        let v = self.block[self.at][self.channel] * self.gains[self.channel];
        if self.channel == 1 {
            self.at += 1;
        }
        self.channel ^= 1;
        Some(v.clamp(-32768.0, 32767.0) as i16)
    }
}

impl Source for StreamMix {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        OUT_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Settings shared between the game and the output thread.
#[derive(Debug, Clone)]
struct Control {
    gains: [f32; 2],
    freq: f64,
    playing: bool,
    paused: bool,
    looping: bool,
    restart: bool,
    stop: bool,
}

/// Frames computed per `Mix` block: the settings (Control) are read once per block
/// (2.9 ms), under a single lock, instead of once per frame (44,100 locks and copies per
/// second per voice). Each frame is computed as before. Mixing stays asynchronous relative
/// to the game, like DirectSound's: nothing in the game (HX state, randomness) depends on it.
const BLOCK_MIX: usize = 128;

struct Mix {
    pcm: Arc<Pcm>,
    ctl: Arc<Mutex<Control>>,
    pos: f64,
    /// Samples of the current block (left, right interleaved) and read position.
    block_value: Vec<i16>,
    at: usize,
}

impl Mix {
    /// Next block; false when the voice is stopped (end of the source).
    #[inline(never)]
    fn fill(&mut self) -> bool {
        let c = {
            let Ok(mut x) = self.ctl.lock() else { return false };
            let c = x.clone();
            if c.restart {
                x.restart = false;
            }
            c
        };
        if c.stop {
            return false;
        }
        if c.restart {
            self.pos = 0.0;
        }
        self.block_value.clear();
        self.at = 0;
        let ch = self.pcm.channels.max(1) as usize;
        let frames = self.pcm.samples.len() / ch;
        let cl = |v: f32| v.clamp(-32768.0, 32767.0) as i16;
        for _ in 0..BLOCK_MIX {
            if !c.playing || c.paused || frames == 0 {
                self.block_value.extend_from_slice(&[0, 0]);
                continue;
            }
            if self.pos >= frames as f64 {
                if c.looping {
                    self.pos %= frames as f64;
                } else {
                    self.block_value.extend_from_slice(&[0, 0]);
                    continue;
                }
            }
            // Linear interpolation between two frames.
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            let j = if i + 1 < frames {
                i + 1
            } else if c.looping {
                0
            } else {
                i
            };
            let s = |k: usize, c: usize| self.pcm.samples[k * ch + c.min(ch - 1)] as f32;
            let l = s(i, 0) * (1.0 - t) + s(j, 0) * t;
            let r = s(i, 1) * (1.0 - t) + s(j, 1) * t;
            self.block_value.push(cl(l * c.gains[0]));
            self.block_value.push(cl(r * c.gains[1]));
            self.pos += c.freq / OUT_RATE as f64;
        }
        true
    }
}

impl Iterator for Mix {
    type Item = i16;
    #[inline]
    fn next(&mut self) -> Option<i16> {
        if self.at >= self.block_value.len() && !self.fill() {
            return None;
        }
        let v = self.block_value[self.at];
        self.at += 1;
        Some(v)
    }
}

impl Source for Mix {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        OUT_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Output limiter threshold (full scale = 1).
const LIMIT: f32 = 0.98;

/// Limiter release: time (s) for the gain to return to 1 after a peak.
const LIMIT_RETURN: f32 = 0.15;

/// Peaks of the sum of voices, recorded by the output thread for the log.
#[derive(Default)]
struct Peaks {
    /// Largest absolute value of the sum (bits of an f32) since the last report.
    max: AtomicU32,
    /// Frames whose sum exceeded full scale since the last report.
    frames_list: AtomicU32,
}

/// Central mix: all game voices are summed here (44.1 kHz stereo), then the
/// sum goes through a limiter before output. Without it, the sum of several shots exceeds
/// full scale and Android's output clips it hard; on PC, the Windows mixer
/// already limits the sum. A single conversion to the phone's rate instead of
/// one per voice.
struct Central {
    mix: DynamicMixer<f32>,
    /// Current frame (left, right) and channel to return.
    frame: [f32; 2],
    channel_value: usize,
    /// Peak envelope (instant attack, release in LIMIT_RETURN).
    env: f32,
    return_value: f32,
    limiter: bool,
    peaks: Arc<Peaks>,
}

impl Central {
    fn new(mix: DynamicMixer<f32>, limiter: bool, peaks: Arc<Peaks>) -> Central {
        let return_value = (-1.0 / (LIMIT_RETURN * OUT_RATE as f32)).exp();
        Central { mix, frame: [0.0; 2], channel_value: 0, env: 0.0, return_value, limiter, peaks }
    }
}

impl Iterator for Central {
    type Item = f32;
    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.channel_value == 0 {
            // Without voices, the mixer stops (None): silence, the output stays open.
            let l = self.mix.next().unwrap_or(0.0);
            let r = self.mix.next().unwrap_or(0.0);
            let peak = l.abs().max(r.abs());
            if peak > 1.0 {
                self.peaks.frames_list.fetch_add(1, Ordering::Relaxed);
            }
            if peak > f32::from_bits(self.peaks.max.load(Ordering::Relaxed)) {
                self.peaks.max.store(peak.to_bits(), Ordering::Relaxed);
            }
            self.env = peak.max(self.env * self.return_value);
            let g = if self.limiter && self.env > LIMIT { LIMIT / self.env } else { 1.0 };
            self.frame = [l * g, r * g];
        }
        let v = self.frame[self.channel_value];
        self.channel_value ^= 1;
        Some(v)
    }
}

impl Source for Central {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        OUT_RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Central mix: None = voice by voice to rodio (PC default); Some(limiter).
/// Android: with limiter. OXIII_SOUND_LIMITER=1: with limiter, =0: voice by voice.
fn limiter_value() -> Option<bool> {
    match openxiii_package::env::var("OXIII_SOUND_LIMITER").ok().as_deref() {
        Some("0") => None,
        Some(_) => Some(true),
        None => cfg!(target_os = "android").then_some(true),
    }
}

struct Playing {
    ctl: Arc<Mutex<Control>>,
    starts: u32,
}

pub struct Audio {
    /// Output that follows the system's device (reopened when headphones are unplugged, etc.).
    output: Output,
    /// Central mix with limiter (Android): voices are added to it instead of to the output.
    central: Option<Arc<DynamicMixerController<f32>>>,
    peaks: Arc<Peaks>,
    /// Last peak report in the log.
    record: std::time::Instant,
    set: SoundSet,
    /// Game folder and banks whose waves `set` holds (those of the HX engine: Fix, then the
    /// bank that LoadSounds loads by the level's title).
    game: std::path::PathBuf,
    banks: Vec<String>,
    playing: HashMap<u64, Playing>,
    /// Streaming channels being played (by address of the shared channel).
    streams: HashMap<usize, StreamHandle>,
    /// Bank waves decoded ahead of time by a background thread (prewarming), received here
    /// before each update.
    warmup_value: Option<std::sync::mpsc::Receiver<(openxiii_assets::hx::Cuuid, Option<Arc<Pcm>>)>>,
    /// Improved build: the level is prepared behind the loading recap and does not run yet: the
    /// banks are loaded, but its voices start only once this is false again.
    pub held: bool,
}

/// Largest wave decoded ahead (samples): beyond that (music), decoded on first
/// play as before.
const WARMUP_MAX: u64 = 4 << 20;

fn warmup() -> Option<&'static str> {
    Some("1")
}

/// Background thread: same banks, same waves, same decoding as `SoundSet::pcm_wave`.
fn warm_up(game: std::path::PathBuf, names: Vec<String>) -> std::sync::mpsc::Receiver<(openxiii_assets::hx::Cuuid, Option<Arc<Pcm>>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("sounds".into()).spawn(move || {
        let names_list: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut set = SoundSet::load(&game, &names_list);
        for w in set.waves() {
            if set.wave_samples(w).is_none_or(|n| n > WARMUP_MAX) {
                continue;
            }
            let pcm = set.pcm_wave(w);
            if tx.send((w, pcm)).is_err() {
                return;
            }
        }
    });
    rx
}

impl Audio {
    /// Opens the audio output and loads the waves of the map's banks and of Fix.
    pub fn new(game: &std::path::Path, _map: &str) -> Option<Audio> {
        let output = Output::new()?;
        let peaks = Arc::new(Peaks::default());
        let central = match limiter_value() {
            Some(limit) => {
                let (ctl, mix) = rodio::dynamic_mixer::mixer::<f32>(2, OUT_RATE);
                output.play_raw(Central::new(mix, limit, peaks.clone()));
                println!("sound: central mix{}", if limit { " with limiter" } else { " without limiter" });
                Some(ctl)
            }
            None => None,
        };
        let set = SoundSet::load(game, &[]);
        Some(Audio {
            output,
            central,
            peaks,
            record: std::time::Instant::now(),
            set,
            game: game.to_path_buf(),
            banks: Vec::new(),
            playing: HashMap::new(),
            streams: HashMap::new(),
            warmup_value: None,
            held: false,
        })
    }

    /// Plays a voice: into the central mix if it exists, otherwise directly on the output.
    fn play<S: Source<Item = f32> + Send + 'static>(&self, s: S) {
        match &self.central {
            Some(c) => c.add(s),
            None => {
                self.output.play_raw(s);
            }
        }
    }

    /// Log of the sum's peaks (at most every 5 s, only if there was an overflow).
    fn record_peaks(&mut self) {
        if self.record.elapsed().as_secs_f32() < 5.0 {
            return;
        }
        self.record = std::time::Instant::now();
        let frames_list = self.peaks.frames_list.swap(0, Ordering::Relaxed);
        let max = f32::from_bits(self.peaks.max.swap(0, Ordering::Relaxed));
        if frames_list > 0 {
            println!("sound: sum beyond full scale on {frames_list} frames (peak {max:.2})");
        }
    }

    /// Forwards the engine's voices to the mixer (new ones, settings, stops). Streamed
    /// voices play from their channel until it stops (fade after the stop included).
    pub fn sync(&mut self, engine: &mut Engine) {
        // Lockstep co-op (stream follow mode): the game's pace moves the channels.
        engine.external_audio = engine.stream_follow_rate == 0;
        self.record_peaks();
        // Banks compared without copying them (every frame); copied only if they changed.
        let changed = engine.banks.len() != self.banks.len() || engine.banks.iter().zip(&self.banks).any(|(b, n)| b.0 != *n);
        if changed {
            let banks: Vec<String> = engine.banks.iter().map(|b| b.0.clone()).collect();
            // Banks of the map and of Fix as in SoundSet (the map first for duplicates).
            let mut names: Vec<&str> = banks.iter().map(String::as_str).filter(|n| !n.eq_ignore_ascii_case("Fix")).collect();
            names.push("Fix");
            self.set = SoundSet::load(&self.game, &names);
            println!("sound: {} resources in the banks {}", self.set.len(), names.join(", "));
            self.warmup_value = warmup().map(|_| warm_up(self.game.clone(), names.iter().map(|n| n.to_string()).collect()));
            self.banks = banks;
        }
        // Waves already decoded by the background thread.
        if let Some(rx) = &self.warmup_value {
            while let Ok((w, pcm)) = rx.try_recv() {
                self.set.insert_wave(w, pcm);
            }
        }
        if self.held {
            return;
        }
        let mut alive = std::collections::HashSet::new();
        for v in engine.voices.iter().flatten() {
            if let Some(h) = &v.stream {
                let key = std::sync::Arc::as_ptr(h) as usize;
                if !self.streams.contains_key(&key) {
                    let mix = StreamMix { handle: h.clone(), block: Vec::new(), at: 0, channel: 0, gains: [0.0; 2] };
                    self.play(mix.convert_samples());
                    self.streams.insert(key, h.clone());
                }
                continue;
            }
            alive.insert(v.serial);
            let (gains, freq) = voice_params(v);
            match self.playing.get_mut(&v.serial) {
                Some(p) => {
                    if let Ok(mut c) = p.ctl.lock() {
                        c.gains = gains;
                        c.freq = freq;
                        c.playing = v.playing;
                        c.paused = v.paused;
                        c.looping = v.looping;
                        if v.starts != p.starts {
                            c.restart = true;
                        }
                    }
                    p.starts = v.starts;
                }
                None if v.playing => {
                    // Mods build only: a mod's samples for this wave (own sound or replaced game sound).
                    #[cfg(feature = "mods")]
                    let pcm = openxiii_vm::mod_own_sounds::mod_wave_pcm(v.wave).or_else(|| self.set.pcm_wave(v.wave));
                    #[cfg(not(feature = "mods"))]
                    let pcm = self.set.pcm_wave(v.wave);
                    let Some(pcm) = pcm else {
                        continue;
                    };
                    let ctl = Arc::new(Mutex::new(Control { gains, freq, playing: true, paused: v.paused, looping: v.looping, restart: false, stop: false }));
                    let mix = Mix { pcm, ctl: ctl.clone(), pos: 0.0, block_value: Vec::with_capacity(2 * BLOCK_MIX), at: 0 };
                    self.play(mix.convert_samples());
                    self.playing.insert(v.serial, Playing { ctl, starts: v.starts });
                }
                None => {}
            }
        }
        self.streams.retain(|_, h| h.lock().map(|s| !(s.chan.released && s.chan.state == 6)).unwrap_or(false));
        self.playing.retain(|serial, p| {
            if alive.contains(serial) {
                true
            } else {
                if let Ok(mut c) = p.ctl.lock() {
                    c.stop = true;
                }
                false
            }
        });
    }
}

