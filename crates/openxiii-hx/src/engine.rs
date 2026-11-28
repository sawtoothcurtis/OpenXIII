//! Core of HXAudio.dll's "usnd" sound engine: bank resources, sound objects (waves,
//! programs, actors), variables, timers and driver voices.
//!
//! Main correspondences (HXAudio.dll):
//! - `CPCWavResData` vtable+0x1C: creation of the wave object (empty 0x44, normal
//!   0x4C, streamed 0x88); wave messages; volumes and pitches per slot;
//!   driver gain = exp(0.15 × dB).
//! - `CProgramResObj`: creation, messages, running a handler,
//!   attach blocks.
//! - `CActorResObj`: creation, messages, broadcast,
//!   application of the actor's settings; context `CActorProgramContext`
//!   (attachments, cleanup, destruction).
//! - Timers: (creation), (stop), global list.

use crate::diagnostics::{static_at_turns, static_delay};
use crate::interp::{Ctx, K_CUUID, K_OBJECT, K_SIMPLE, T_BOOL, T_FLOAT, T_INT, T_STRING};
use crate::mem::{Addr, Mem};
use crate::program::Program;
use crate::rand::MsvcRand;
use crate::stream::{StreamHandle, StreamSource, StreamState};
use openxiii_assets::hx::Cuuid;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

pub type ObjId = u32;
pub type VarId = u32;
pub type ActorId = u32;
pub type VoiceId = u32;

/// `CPCWavResData`: simple sound (reference to a wave, volume and options).
#[derive(Debug, Clone)]
pub struct WavRes {
    pub cuuid: Cuuid,
    pub version: u32,
    pub name: String,
    /// + 0x20: volume in dB (HX convention).
    pub volume: f32,
    /// + 0x50.
    pub x50: f32,
    /// + 0x24: starting pitch.
    pub pitch: f32,
    /// + 0x54: 1 loop, 2 per-language variants, 4 stoppable, 8 positioned (message 0x1E),
    /// 0x10 pan, 0x20 Doppler pitch, 0x40 voice (PlaySound).
    pub flags: u8,
    /// + 0x28: wave (`CPCWaveFileIdObj`).
    pub wave: Cuuid,
    pub x18: u32,
    pub x1c: bool,
    /// Per-language variants: (ASCII code "fr  "…, wave).
    pub languages: Vec<(u32, Cuuid)>,
}

impl WavRes {
    /// Wave chosen at playback (+0x28): with variants, the one for the language,
    /// otherwise the start of the variant tree (the smallest language: "de" for "ru" or
    /// "pl"); a duplicate key keeps its first wave.
    pub fn wave_for(&self, language: u32) -> Cuuid {
        if self.flags & 2 == 0 {
            return self.wave;
        }
        let v = self.languages.iter().find(|(l, _)| *l == language).or_else(|| self.languages.iter().min_by_key(|(l, _)| *l));
        v.map_or(self.wave, |x| x.1)
    }
}

/// Wave (`CPCWaveFileIdObj`): what the driver needs to play it.
#[derive(Debug, Clone)]
pub struct WaveInfo {
    pub cuuid: Cuuid,
    /// + 0x10 of the `CWaveFile` object: own gain in dB.
    pub gain: f32,
    /// Streamed (music, ambiences): 0x88 object with markers.
    pub streamed: bool,
    pub frames: u64,
    pub rate: u32,
    /// Wave markers (position in frames, name), sorted.
    pub markers: Vec<(u32, Vec<u8>)>,
    /// Streamed waves: file and data position.
    pub source: Option<StreamSource>,
    /// Data size (+0x08 of the `CWaveFile`), format (+0x40: tag, channels).
    pub data_size: u32,
    pub format_tag: u16,
    pub channels: u16,
}

/// Default attenuation of a CActorResData (+0x64, copied by constructor, which
/// the static initialiser sets at load time: byte 0, 100, 10, −100).
pub const ROLLOFF_DEFAULT: [f32; 4] = [0.0, 100.0, 10.0, -100.0];

/// Actor resource (`CActorResData`), made by the subsystem.
#[derive(Debug, Clone)]
pub struct ActorRes {
    pub name: String,
    /// + 0x20: actor positioned in space.
    pub is3d: bool,
    /// + 0x64…0x70: attenuation (SetRolloff).
    pub rolloff: [f32; 4],
}

/// Class in the HX type registry: name, C++ object size, version, parent class, factory (instantiable class).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeHx {
    pub name_value: &'static str,
    pub size_value: u32,
    pub version: u32,
    pub parent_value: Option<&'static str>,
    pub factory: bool,
}

const fn type_hx_(name_value: &'static str, size_value: u32, version: u32, parent_value: Option<&'static str>, factory: bool) -> TypeHx {
    TypeHx { name_value, size_value, version, parent_value, factory }
}

/// HXAudio's type registry, filled when the DLL loads by ten static initialisers (record, inserted
/// by into the tree of singleton, built). CreateObject
/// looks up a bank entry's class name in it (strcmp: case-sensitive).
pub const TYPES_HX: [TypeHx; 10] = [
    type_hx_("CPCWaveFileIdObj", 0x34, 3, Some("CWaveFileIdObj"), true),
    type_hx_("toto", 0, 0, None, false),
    type_hx_("CIdObj", 0xC, 1, Some("toto"), false),
    type_hx_("CRefIdObj", 0x10, 1, Some("CIdObj"), false),
    type_hx_("CWaveFileIdObj", 0x30, 3, Some("CRefIdObj"), true),
    type_hx_("CResData", 0x20, 2, Some("CRefIdObj"), false),
    type_hx_("CActorResData", 0x78, 2, Some("CResData"), true),
    type_hx_("CWavResData", 0x5C, 5, Some("CResData"), false),
    type_hx_("CProgramResData", 0x148, 0xB, Some("CResData"), true),
    type_hx_("CPCWavResData", 0x64, 3, Some("CWavResData"), true),
];

/// Lookup: the class with this exact name.
pub fn type_hx(name_value: &str) -> Option<&'static TypeHx> {
    TYPES_HX.iter().find(|t| t.name_value == name_value)
}

#[derive(Debug, Clone)]
pub enum ResData {
    Wav(Rc<WavRes>),
    Program(Rc<Program>),
    Actor(Rc<ActorRes>),
}

/// Write of a variable watched by the subsystem: its listener number, the actor name and
/// type (ActorName, ActorType) and the variable's value, copied at write time.
#[derive(Debug, Clone)]
pub struct HookEvent {
    pub code: u32,
    pub actor: Vec<u8>,
    pub kind: i32,
    pub text: Vec<u8>,
    pub float: f32,
    pub int: u32,
}

/// Listener of a variable (targets of ` `).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hook {
    /// Pitch (0) or volume (1) modifier of an actor.
    Modifier { actor: ObjId, volume: bool },
    /// Subsystem listener (number specific to `subsystem`).
    Subsystem(u32),
}

/// Initialisation value of a variable (content +8 of the type's value): word (integer, float, boolean, character, kind 5), text,
/// CUUID, object already found in the registry.
#[derive(Debug, Clone)]
pub enum InitVar {
    Word(u32),
    Text(Vec<u8>),
    Cuuid([u32; 2]),
    ObjectVar(Option<ObjId>),
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: Vec<u8>,
    pub kind: u32,
    pub ty: u32,
    /// + 0x2C: value (integer, float, boolean, character or object).
    pub value: u32,
    pub cuuid: [u32; 2],
    pub text: Vec<u8>,
    pub hooks: Vec<Hook>,
}

/// Volume (dB, summed) or pitch (multiplied) slots of a wave.
#[derive(Debug)]
pub struct WaveObj {
    pub res: Rc<WavRes>,
    pub wave: Option<Rc<WaveInfo>>,
    pub streamed: bool,
    /// + 0x0C: pan, + 0x10: resulting pitch, + 0x14: resulting volume,
    /// + 0x18: fourth 2D setting.
    pub pan: f32,
    pub pitch: f32,
    pub volume: f32,
    pub fx: f32,
    /// + 0x1C: driver voice.
    pub voice: Option<VoiceId>,
    pub volume_slots: [f32; 3],
    pub volume_count: u16,
    pub pitch_slots: [f32; 3],
    pub pitch_count: u16,
    /// + 0x42: reserved slots (bits 0..2 volume, 8..10 pitch).
    pub slot_bits: u16,
    /// + 0x48: end handler (SetEndNotif).
    pub end_notif: u32,
    /// Streamed wave: decoder, markers and callbacks (0x88-byte object).
    pub stream: Option<Box<crate::stream::StreamState>>,
}

/// Timer of a program (0x1C-byte structure in node + 0x14).
#[derive(Debug, Clone)]
pub struct Timer {
    pub params: Addr,
    /// Entry in the global list (0 = paused or stopped).
    pub entry: u32,
    pub repeat: bool,
    pub delay: u32,
    pub range: u32,
    pub start: u32,
    /// Handler called (wrapper), 0 if stopped.
    pub handler: u32,
}

#[derive(Debug, Clone)]
pub struct ProgObj {
    pub prog: Rc<Program>,
    /// + 0x34: object variables.
    pub locals: Addr,
    /// + 0x4C, + 0x50, + 0x54, + 0x58.
    pub valid: bool,
    pub held: i32,
    pub played: bool,
    pub actor: ObjId,
    /// + 0x0C: timers by number, next number.
    pub next_timer: u32,
    pub timers: BTreeMap<u32, Timer>,
    /// Handler wrappers (`CHdlrMgr`), one per named handler.
    pub handler_refs: Vec<u32>,
}

/// Attachment of an object to an actor (0x28-byte entry).
#[derive(Debug, Clone)]
pub struct Entry {
    pub obj: ObjId,
    /// + 0x04: global sequence number.
    pub serial: u32,
    pub res2d: bool,
    pub playable: bool,
    pub follows: bool,
    pub positioned: bool,
    pub single_2d: bool,
    pub intercept: bool,
    pub closing: bool,
    /// + 0x10 / + 0x14: the caller's key (Unreal Sound object).
    pub key: u64,
    /// Sound channel (subsystem key): released when the entry disappears.
    pub channel: u32,
    /// + 0x20: owning program.
    pub owner: ObjId,
    /// + 0x24: references.
    pub refs: i32,
}

/// Pitch or volume modifier (`CActorModifiers`).
#[derive(Debug, Clone)]
pub struct Modifier {
    pub base: f32,
    pub value: f32,
    pub vars: Vec<(VarId, f32)>,
}

#[derive(Debug, Clone)]
pub struct ActorObj {
    pub res: Rc<ActorRes>,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub orient: [f32; 6],
    pub pitch_mod: Modifier,
    pub rolloff: [f32; 4],
    /// + 0x80: pan, pitch, volume, fourth setting computed by the listener.
    pub params2d: [f32; 4],
    pub dirty: bool,
    pub is3d: bool,
    pub name: String,
    pub params_valid: bool,
    pub volume_mod: Modifier,
    pub entries: Vec<Entry>,
    pub vars: BTreeMap<Vec<u8>, VarId>,
    /// Pan coefficient in force when the actor is updated.
    pub pan_coeff: f32,
}

#[derive(Debug)]
pub enum ObjKind {
    /// Missing wave or wave without a driver (0x44 object: answers nothing).
    Silent(Rc<WavRes>),
    Wave(Box<WaveObj>),
    Program(Box<ProgObj>),
    Actor(Box<ActorObj>),
}

#[derive(Debug)]
pub struct Object {
    pub res: Cuuid,
    pub kind: ObjKind,
}

/// Wrapper of a named handler (vtable).
#[derive(Debug, Clone, Copy)]
pub struct HandlerRef {
    pub owner: ObjId,
    pub index: u32,
    /// Native subsystem handler (code, 0 for a program handler): static objects
    /// whose reference has no effect (Pause, Unpause).
    pub native: u32,
}

/// Entry of the global timer list (0x20 bytes).
#[derive(Debug, Clone, Copy)]
pub struct TimerEntry {
    pub id: u32,
    pub prog: ObjId,
    pub timer: u32,
    pub due: u32,
    pub delay: u32,
    pub range: u32,
    pub killed: bool,
}

/// Driver voice: what a mixer has to play.
#[derive(Debug, Clone)]
pub struct Voice {
    pub res: Cuuid,
    pub wave: Cuuid,
    pub looping: bool,
    pub playing: bool,
    pub paused: bool,
    /// Linear gain, pitch ratio, pan (0..1) and fourth setting.
    pub gain: f32,
    pub pitch: f32,
    pub pan: f32,
    pub fx: f32,
    /// Position in frames of the wave.
    pub position: f64,
    pub frames: u64,
    pub rate: u32,
    /// Incremented on each start (the mixer restarts from the beginning).
    pub starts: u32,
    /// Unique voice number (slots are reused).
    pub serial: u64,
    /// Streamed waves: ring and DirectSound channel, shared with the audio output.
    pub stream: Option<StreamHandle>,
    /// Seconds before the DirectSound cursor of a static buffer starts after Play: the
    /// dsound.dll mixer, as for streaming channels (`stream::ds8_latency`).
    pub wait_delay: f64,
    /// Static buffer: time of the Play requested by the game thread (channel state 1), then start of
    /// playback (pass of the DirectSound thread that launches it, state 2, plus `wait_delay`). See
    /// `advance_static_voices`.
    pub launches: f64,
    pub start_value: Option<f64>,
    /// Time (seconds, `time_ms` scale) of the last requested pause: a started channel goes
    /// to 3 (pause requested) then to 4 on the DirectSound thread's next pass.
    pub pause_launches: f64,
    /// Stop requested at this time on a started channel (state 5: 2 and 3 → 5): the voice
    /// still answers "playing" and the end callback waits for the DirectSound thread's next pass
    /// (6, buffer stop, callback).
    pub stop_turn: Option<f64>,
}

/// End callback of a driver stop (`Engine::voice_stop`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EndStop {
    NoSource,
    Immediately,
    /// On the DirectSound thread pass after this time (state 5 → 6).
    AtTurn(f64),
}

pub struct Engine {
    pub mem: Mem,
    pub registry: HashMap<Cuuid, ResData>,
    pub waves: HashMap<Cuuid, Rc<WaveInfo>>,
    /// Loaded banks, in order: name, resources and waves they added.
    pub banks: Vec<(String, Vec<Cuuid>, Vec<Cuuid>)>,
    pub unhandled: std::collections::BTreeMap<(&'static str, u32), u32>,
    objects: Vec<Option<Object>>,
    free_objects: Vec<usize>,
    vars: Vec<Option<Var>>,
    pub globals: BTreeMap<Vec<u8>, VarId>,
    pub(crate) handler_refs: Vec<Option<HandlerRef>>,
    pub(crate) timer_list: Vec<TimerEntry>,
    timer_serial: u32,
    /// Listeners (player camera).
    pub listeners: Vec<crate::listener::Listener>,
    pub voices: Vec<Option<Voice>>,
    voice_serial: u64,
    /// Bits +0x0C/+0x0D left by the entries returned to the pool.
    entry_pool: Vec<(bool, bool)>,
    entry_serial: u32,
    pub rand_main: MsvcRand,
    /// Tests: messages sent (object, message, argument block) and calls by name on the actor, recorded when the list exists.
    /// Tests: stack of the last nested context (call of a program function).
    #[cfg(test)]
    pub(crate) last_frame: Addr,
    /// Tests: events of a program object (handler run, attach, init and end blocks).
    #[cfg(test)]
    pub(crate) record_log: Option<Vec<String>>,
    #[cfg(test)]
    pub(crate) sends: Option<(Vec<(ObjId, u32, Addr, Vec<u8>, Addr)>, Vec<(ObjId, Vec<u8>, Addr)>)>,

    /// First CUUID of the process (byte at 1); it does
    /// srand(rdtsc >> 32) on the calling thread.
    pub first_cuuid: bool,
    /// Seed given to that srand (a given one, or the host's in a lockstep session);
    /// it wins over `srand_clock`.
    pub srand_first_cuuid: Option<u32>,
    /// That srand reads the clock, as in the original (the game); off, the sequence continues
    /// (tests and tools, which stay reproducible).
    pub srand_clock: bool,
    pub time_ms: u32,
    /// DirectSound thread: time elapsed since its last pass. A single thread serves
    /// every channel.
    pub ds8_clock: f64,
    /// End callbacks of voices stopped in state 5 then released (message 1): (stop time,
    /// wave), served on the DirectSound thread's next pass as for a kept voice.
    pub fins_deferred: Vec<(f64, ObjId)>,
    /// Tests: end callbacks of static waves (wave, `time_ms`).
    #[cfg(test)]
    pub(crate) fins_noted: Vec<(ObjId, u32)>,
    /// Voice language (4-byte ASCII code, "fr  ").
    pub language: u32,
    pub logs: Vec<String>,
    /// Sounds playing per channel (UHXAUDIOSubsystem + 0xB8 + 8 × channel); decremented when
    /// each entry carrying the channel key is destroyed.
    pub channel_counts: [i32; 8],
    /// Writes of variables watched by the subsystem, with what the listener copies of them
    /// at write time.
    pub hook_events: Vec<HookEvent>,
    /// Calls (listeners notified of a write), even without a listener: recorded for tests.
    pub notes_write: u32,
    /// ActorName and ActorType global variables, read by the
    /// subsystem's listeners.
    pub hook_actor_vars: Option<(VarId, VarId)>,
    /// Rings of the streamed waves, in service order.
    pub(crate) ring_order: Vec<ObjId>,
    pub(crate) ring_bindings: BTreeMap<u32, crate::stream::RingBinding>,
    /// Native handlers already created (code → wrapper) and calls received (codes, in order).
    native_handlers: BTreeMap<u32, u32>,
    pub native_calls: Vec<u32>,
    /// A real audio output consumes the streaming channels (otherwise the engine simulates the
    /// DirectSound thread at the game's pace).
    pub external_audio: bool,
    /// In lockstep, the game advances streams and the device plays the frames it produced.
    /// Zero lets the audio output drive new stream channels normally.
    pub stream_follow_rate: u32,
    /// Time elapsed since the decoder thread's last pass.
    pub decoder_clock: f64,
    /// In game: real time of the last Update, to date a creation made later in the
    /// frame (timeGetTime at the time of the call). None: `time_ms` only (simulations).
    pub update_real: Option<std::time::Instant>,
    trace_calls: bool,
    /// Improved build's co-op: waves played by an actor (key: HX actor) in place of others,
    /// so that a player who is not XIII does not speak with his voice. Empty in the game.
    pub wave_swaps: HashMap<ObjId, Rc<WaveSwaps>>,
    /// Set while a sound announcing a death is started (its swaps take the death waves).
    pub swap_death: bool,
}

/// Waves an actor plays instead of others (`Engine::wave_swaps`): for each replaced wave
/// resource, the waves to draw from (none: silence), and on a death the death waves.
#[derive(Debug, Default)]
pub struct WaveSwaps {
    pub waves: HashMap<Cuuid, Vec<Cuuid>>,
    pub death: HashMap<Cuuid, Vec<Cuuid>>,
    /// Own draw (the engine's random sequence stays the game's).
    pub draw: std::cell::Cell<u32>,
    /// The actor was killed: its shouts from then on are death shouts (a program may start
    /// its wave after the call that announced the death).
    pub dead: std::cell::Cell<bool>,
    /// Whose voice this is (the co-op character and timbre), to rebuild it for another one.
    pub tag: String,
    /// The sound that tells a hit, whose third parameter says the hit kills (the pawn's
    /// hHitSound: PlaySound(hHitSound, SoundType, Damage, bIsDead)). Other sounds of the
    /// pawn have other parameters.
    pub hit_sound: Option<Cuuid>,
}

impl WaveSwaps {
    /// The resource to create in place of `id`: Some(None) for silence, None when `id` is not
    /// replaced.
    pub fn pick(&self, id: Cuuid, death: bool) -> Option<Option<Cuuid>> {
        if death {
            self.dead.set(true);
        }
        let death = death || self.dead.get();
        let list = death.then(|| self.death.get(&id)).flatten().or_else(|| self.waves.get(&id))?;
        if list.is_empty() {
            return Some(None);
        }
        let x = self.draw.get().wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.draw.set(x);
        Some(Some(list[(x >> 16) as usize % list.len()]))
    }
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            mem: Mem::new(),
            registry: HashMap::new(),
            waves: HashMap::new(),
            banks: Vec::new(),
            unhandled: std::collections::BTreeMap::new(),
            objects: vec![None],
            free_objects: Vec::new(),
            vars: vec![None],
            globals: BTreeMap::new(),
            handler_refs: vec![None],
            timer_list: Vec::new(),
            timer_serial: 0,
            listeners: Vec::new(),
            voices: vec![None],
            voice_serial: 0,
            entry_pool: Vec::new(),
            entry_serial: 0,
            rand_main: MsvcRand::new(1),
            #[cfg(test)]
            last_frame: 0,
            #[cfg(test)]
            record_log: None,
            #[cfg(test)]
            sends: None,

            first_cuuid: true,
            srand_first_cuuid: None,
            srand_clock: false,
            time_ms: 0,
            ds8_clock: 0.0,
            fins_deferred: Vec::new(),
            #[cfg(test)]
            fins_noted: Vec::new(),
            language: u32::from_be_bytes(*b"fr  "),
            logs: Vec::new(),
            channel_counts: [0; 8],
            hook_events: Vec::new(),
            notes_write: 0,
            hook_actor_vars: None,
            ring_order: Vec::new(),
            ring_bindings: BTreeMap::new(),
            native_handlers: BTreeMap::new(),
            native_calls: Vec::new(),
            external_audio: false,
            stream_follow_rate: 0,
            decoder_clock: 0.0,
            update_real: None,
            wave_swaps: HashMap::new(),
            swap_death: false,
            trace_calls: crate::diagnostics::trace_calls(),
        }
    }

    pub fn log(&mut self, text: String) {
        self.logs.push(text);
    }

    /// msvcr70's rand() on the game thread's sequence;
    #[track_caller]
    pub fn rand(&mut self) -> i32 {
        self.rand_main.next()
    }

    /// Draw of a new object's CUUID: srand on the very
    /// first call, then one rand().
    #[track_caller]
    pub fn cuuid_rand(&mut self) -> i32 {
        self.cuuid_rand_with(crate::rand::clock_srand_seed)
    }

    /// [`Engine::cuuid_rand`] with `clock` as the time-stamp counter's high dword.
    #[track_caller]
    pub(crate) fn cuuid_rand_with(&mut self, clock: impl FnOnce() -> u32) -> i32 {
        if self.first_cuuid {
            self.first_cuuid = false;
            // The given seed, else the clock's when `srand_clock` is on, else none (the sequence
            // continues).
            if let Some(s) = self.srand_first_cuuid.or_else(|| self.srand_clock.then(clock)) {
                self.rand_main.seed = s;
            }
        }
        self.rand()
    }

    pub fn time_ms(&self) -> u32 {
        self.time_ms
    }

    // ----- resources -----

    /// Adds the resources of a `.hxc` bank (waves, simple sounds, programs).
    /// `wave_len` gives (frames, rate) of a wave of the bank, to know when it ends.
    /// `bank_path` locates the external files (.hsc) of streamed waves.
    pub fn add_bank(
        &mut self,
        data: &[u8],
        bank: &openxiii_assets::hx::HxBank,
        wave_len: &dyn Fn(&openxiii_assets::hx::Wave) -> (u64, u32),
        bank_path: Option<&std::path::Path>,
    ) {
        for w in &bank.waves {
            let (frames, rate) = wave_len(w);
            let gain = wave_gain(data, w.cuuid, bank);
            let streamed = w.mode & 2 != 0;
            // Native RIFF labels stay Latin-1 bytes.
            let markers = w.markers.iter().map(|(p, n)| (*p, n.chars().map(|c| c as u8).collect())).collect();
            let data_size = match &w.location {
                openxiii_assets::hx::Location::Internal { size, .. } | openxiii_assets::hx::Location::External { size, .. } => *size,
            };
            let format_tag = match w.codec {
                openxiii_assets::hx::Codec::Pcm16 => 1,
                openxiii_assets::hx::Codec::UbiAdpcm => 2,
                openxiii_assets::hx::Codec::Psx => 3,
                openxiii_assets::hx::Codec::Dsp => 4,
                openxiii_assets::hx::Codec::XboxImaAdpcm => 5,
                openxiii_assets::hx::Codec::Mp3 => 0x55,
                openxiii_assets::hx::Codec::Other(n) => n,
            };
            let source = if streamed { stream_source(bank_path, w, frames, format_tag) } else { None };
            let channels = w.channels;
            self.waves
                .entry(w.cuuid)
                .or_insert_with(|| Rc::new(WaveInfo { cuuid: w.cuuid, gain, streamed, frames, rate, markers, source, data_size, format_tag, channels }));
        }
        for e in &bank.entries {
            // CreateObject: a class unknown to the registry or without a factory creates nothing.
            if self.registry.contains_key(&e.cuuid) || !type_hx(&e.class).is_some_and(|t| t.factory) {
                continue;
            }
            match e.class.as_str() {
                "CPCWavResData" => {
                    if let Some(w) = parse_wavres(data, e.offset as usize) {
                        self.registry.insert(e.cuuid, ResData::Wav(Rc::new(w)));
                    }
                }
                "CProgramResData" => {
                    if let Ok(p) = Program::parse(data, e.offset as usize) {
                        self.registry.insert(e.cuuid, ResData::Program(Rc::new(p)));
                    }
                }
                _ => {}
            }
        }
    }

    /// Actor resource made by the subsystem (`CActorResData`).
    pub fn add_actor_res(&mut self, id: Cuuid, res: ActorRes) {
        self.registry.insert(id, ResData::Actor(Rc::new(res)));
    }

    // ----- objects -----

    pub fn object(&self, id: ObjId) -> Option<&Object> {
        self.objects.get(id as usize).and_then(|o| o.as_ref())
    }
    pub fn object_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        self.objects.get_mut(id as usize).and_then(|o| o.as_mut())
    }

    pub(crate) fn insert_object(&mut self, o: Object) -> ObjId {
        match self.free_objects.pop() {
            Some(i) => {
                self.objects[i] = Some(o);
                i as u32
            }
            None => {
                self.objects.push(Some(o));
                (self.objects.len() - 1) as u32
            }
        }
    }

    pub(crate) fn object_count(&self) -> usize {
        self.objects.len()
    }
    pub(crate) fn wave_obj(&self, id: ObjId) -> Option<&WaveObj> {
        self.wave(id)
    }
    pub(crate) fn wave_obj_mut(&mut self, id: ObjId) -> Option<&mut WaveObj> {
        self.wave_mut(id)
    }
    fn wave(&self, id: ObjId) -> Option<&WaveObj> {
        match &self.object(id)?.kind {
            ObjKind::Wave(w) => Some(w),
            _ => None,
        }
    }
    fn wave_mut(&mut self, id: ObjId) -> Option<&mut WaveObj> {
        match &mut self.object_mut(id)?.kind {
            ObjKind::Wave(w) => Some(w),
            _ => None,
        }
    }
    pub fn program_obj(&self, id: ObjId) -> Option<&ProgObj> {
        match &self.object(id)?.kind {
            ObjKind::Program(p) => Some(p),
            _ => None,
        }
    }
    fn program_obj_mut(&mut self, id: ObjId) -> Option<&mut ProgObj> {
        match &mut self.object_mut(id)?.kind {
            ObjKind::Program(p) => Some(p),
            _ => None,
        }
    }
    pub fn actor(&self, id: ObjId) -> Option<&ActorObj> {
        match &self.object(id)?.kind {
            ObjKind::Actor(a) => Some(a),
            _ => None,
        }
    }
    pub fn actor_mut(&mut self, id: ObjId) -> Option<&mut ActorObj> {
        match &mut self.object_mut(id)?.kind {
            ObjKind::Actor(a) => Some(a),
            _ => None,
        }
    }

    /// Creates the object of a resource (`CResData` vtable+0x1C).
    pub fn create_object(&mut self, id: Cuuid) -> Option<ObjId> {
        let res = self.registry.get(&id)?.clone();
        match res {
            ResData::Wav(w) => Some(self.create_wave(id, w)),
            ResData::Program(p) => Some(self.create_program(id, p)),
            ResData::Actor(a) => Some(self.create_actor(id, a)),
        }
    }

    /// `create_object` for an object that will play on HX actor `actor`: a wave that actor
    /// swaps (`wave_swaps`) is created as its replacement, or silent.
    pub fn create_object_for(&mut self, actor: ObjId, id: Cuuid) -> Option<ObjId> {
        if let Some(swaps) = self.wave_swaps.get(&actor) {
            if let Some(choice) = swaps.pick(id, self.swap_death) {
                let wanted = choice.filter(|c| matches!(self.registry.get(c), Some(ResData::Wav(_))));
                match wanted {
                    Some(c) => return self.create_object(c),
                    None => {
                        let Some(ResData::Wav(w)) = self.registry.get(&id).cloned() else { return self.create_object(id) };
                        return Some(self.insert_object(Object { res: id, kind: ObjKind::Silent(w) }));
                    }
                }
            }
        }
        self.create_object(id)
    }

    /// Length in seconds of sound resource `id` when it is a single wave of a loaded bank
    /// (None for a program or an unloaded wave). Used by the mods build for subtitles.
    pub fn sound_seconds(&self, id: Cuuid) -> Option<f32> {
        let Some(ResData::Wav(w)) = self.registry.get(&id) else { return None };
        let wave = self.wave_of(w)?;
        (wave.rate > 0).then(|| wave.frames as f32 / wave.rate as f32)
    }

    /// Choice of a sound's wave: the variant for the current language if the sound has one.
    pub(crate) fn wave_of(&self, w: &WavRes) -> Option<Rc<WaveInfo>> {
        self.waves.get(&w.wave_for(self.language)).cloned()
    }

    fn create_wave(&mut self, id: Cuuid, res: Rc<WavRes>) -> ObjId {
        let Some(wave) = self.wave_of(&res) else {
            return self.insert_object(Object { res: id, kind: ObjKind::Silent(res) });
        };
        let mut w = WaveObj {
            res: res.clone(),
            wave: Some(wave.clone()),
            streamed: wave.streamed,
            pan: 0.5,
            pitch: 1.0,
            volume: 0.0,
            fx: 1.0,
            voice: None,
            volume_slots: [0.0; 3],
            volume_count: 1,
            pitch_slots: [1.0; 3],
            pitch_count: 1,
            slot_bits: 0x101,
            end_notif: 0,
            stream: wave.streamed.then(|| Box::new(StreamState::new(wave.source.as_ref(), wave.rate))),
        };
        // Driver voice, volume, starting pitch; looping according to flag 1.
        let voice = self.voice_create(&res, &wave);
        w.voice = Some(voice);
        let obj = self.insert_object(Object { res: id, kind: ObjKind::Wave(Box::new(w)) });
        let v = self.wave(obj).map_or(0.0, |w| w.volume);
        self.wave_apply_volume(obj, v);
        if let Some(voice) = self.voices.get_mut(voice as usize).and_then(|v| v.as_mut()) {
            voice.pitch = res.pitch;
        }
        obj
    }

    fn create_program(&mut self, id: Cuuid, prog: Rc<Program>) -> ObjId {
        // Object variables, handlers.
        let locals = self.mem.alloc(prog.locals_size() as usize);
        let p = ProgObj {
            prog: prog.clone(),
            locals,
            valid: true,
            held: 0,
            played: false,
            actor: 0,
            next_timer: 1,
            timers: BTreeMap::new(),
            handler_refs: Vec::new(),
        };
        let obj = self.insert_object(Object { res: id, kind: ObjKind::Program(Box::new(p)) });
        let refs: Vec<u32> = (0..prog.handlers.len())
            .map(|i| {
                self.handler_refs.push(Some(HandlerRef { owner: obj, index: i as u32, native: 0 }));
                (self.handler_refs.len() - 1) as u32
            })
            .collect();
        if let Some(p) = self.program_obj_mut(obj) {
            p.handler_refs = refs;
        }
        obj
    }

    fn create_actor(&mut self, id: Cuuid, res: Rc<ActorRes>) -> ObjId {
        let a = ActorObj {
            res: res.clone(),
            pos: [0.0; 3],
            vel: [0.0; 3],
            orient: [0.0; 6],
            pitch_mod: Modifier { base: 1.0, value: 1.0, vars: Vec::new() },
            rolloff: res.rolloff,
            params2d: [0.5, 1.0, 0.0, 0.5],
            dirty: false,
            is3d: res.is3d,
            name: String::new(),
            params_valid: false,
            volume_mod: Modifier { base: 0.0, value: 0.0, vars: Vec::new() },
            entries: Vec::new(),
            vars: BTreeMap::new(),
            pan_coeff: 1.0,
        };
        self.insert_object(Object { res: id, kind: ObjKind::Actor(Box::new(a)) })
    }

    /// Destroys an object (vtable+0 with deletion).
    pub fn destroy_object(&mut self, id: ObjId) {
        // Streamed wave: callbacks, timers, ring and buffers first.
        if self.stream(id).is_some() {
            self.stream_destroy(id);
        }
        // Program still valid: end block first (destructor).
        if self.program_obj(id).is_some_and(|p| p.valid) {
            self.program_message(id, 0x3ee, 0);
        }
        let Some(o) = self.objects.get_mut(id as usize).and_then(|o| o.take()) else {
            return;
        };
        self.free_objects.push(id as usize);
        if !self.wave_swaps.is_empty() {
            self.wave_swaps.remove(&id);
        }
        match o.kind {
            ObjKind::Wave(w) => {
                self.fins_deferred.retain(|&(_, o)| o != id);
                // Releases the voice (the end callback fires if it was playing).
                if let Some(v) = w.voice {
                    let was_playing = self.voice_playing(v);
                    self.voice_release(v);
                    if was_playing && w.end_notif != 0 && !w.streamed {
                        self.handler_call(w.end_notif, 0);
                    }
                }
            }
            ObjKind::Program(p) => {
                for (_, t) in p.timers {
                    if t.entry != 0 {
                        self.timer_entry_kill_id(t.entry);
                    }
                }
                for h in p.handler_refs {
                    if let Some(slot) = self.handler_refs.get_mut(h as usize) {
                        *slot = None;
                    }
                }
                self.mem.release(p.locals);
            }
            ObjKind::Actor(a) => {
                let ids: Vec<ObjId> = a.entries.iter().map(|e| e.obj).collect();
                for e in ids {
                    self.destroy_object(e);
                }
            }
            ObjKind::Silent(_) => {}
        }
    }

    // ----- messages -----

    /// Message `msg` to object `obj` (vtable+4), arguments at address `args`.
    pub fn send(&mut self, obj: ObjId, msg: u32, args: Addr) {
        #[cfg(test)]
        if self.sends.is_some() {
            // Named messages (0x3E9, 0x3EF): name and parameters of the {name, parameters} block; other blocks: address only.
            let (name_value, params) = if msg == 0x3e9 || msg == 0x3ef {
                let (mut name_value, mut a) = (Vec::new(), self.mem.u32(args));
                while self.mem.u8(a) != 0 {
                    name_value.push(self.mem.u8(a));
                    a += 1;
                }
                (name_value, self.mem.u32(args + 4))
            } else {
                (Vec::new(), 0)
            };
            if let Some(l) = self.sends.as_mut() {
                l.0.push((obj, msg, args, name_value, params));
            }
        }
        let kind = match self.object(obj).map(|o| &o.kind) {
            Some(ObjKind::Wave(_)) => 1,
            Some(ObjKind::Program(_)) => 2,
            Some(ObjKind::Actor(_)) => 3,
            _ => 0,
        };
        match kind {
            1 => self.wave_message(obj, msg, args),
            2 => self.program_message(obj, msg, args),
            3 => self.actor_message(obj, msg, args),
            _ => {}
        }
    }

    /// Message carrying a handler name: arguments {name, parameters, 0}.
    pub fn send_named(&mut self, obj: ObjId, msg: u32, name: &[u8], params: Addr) {
        let mut text = name.to_vec();
        text.push(0);
        let name_addr = self.mem.alloc_with(&text);
        let args = self.mem.alloc(12);
        self.mem.set_u32(args, name_addr);
        self.mem.set_u32(args + 4, params);
        self.send(obj, msg, args);
        self.mem.release(args);
        self.mem.release(name_addr);
    }

    /// Send with a small temporary argument area.
    pub fn send_with(&mut self, obj: ObjId, msg: u32, words: &[u32]) -> Vec<u32> {
        let mut bytes = Vec::with_capacity(words.len() * 4);
        for w in words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        let args = self.mem.alloc_with(&bytes);
        self.send(obj, msg, args);
        let out = (0..words.len()).map(|i| self.mem.u32(args + 4 * i as u32)).collect();
        self.mem.release(args);
        out
    }

    /// "Is it…" query: boolean answer in a holder {1, &value}.
    pub fn ask(&mut self, obj: ObjId, msg: u32) -> bool {
        let holder = self.mem.alloc(12);
        self.mem.set_u32(holder, 1);
        self.mem.set_u32(holder + 4, holder + 8);
        self.send(obj, msg, holder);
        let r = self.holder_get(holder);
        self.mem.release(holder);
        r
    }

    /// Does program `owner` have a standard handler for `msg`? (message 1000).
    /// Arguments: {&msg, &holder}, holder {1, &result}.
    fn owner_handles(&mut self, owner: ObjId, msg: u32) -> bool {
        let block = self.mem.alloc(24);
        self.mem.set_u32(block, block + 8);
        self.mem.set_u32(block + 4, block + 12);
        self.mem.set_u32(block + 8, msg);
        self.mem.set_u32(block + 12, 1);
        self.mem.set_u32(block + 16, block + 20);
        self.send(owner, 1000, block);
        let r = self.holder_get(block + 12);
        self.mem.release(block);
        r
    }

    /// Writes a boolean into a result holder.
    pub fn holder_set(&mut self, holder: Addr, value: bool) {
        match self.mem.u32(holder) {
            1 => {
                let p = self.mem.u32(holder + 4);
                self.mem.set_u32(p, value as u32);
            }
            2 => {
                let var = self.mem.u32(holder + 4);
                // Only the low byte is written.
                let prior = self.var_u32(var);
                self.set_var_u32(var, (prior & !0xff) | value as u32);
                self.var_write_notify(var);
            }
            _ => {}
        }
    }

    /// Reads a result holder.
    pub fn holder_get(&mut self, holder: Addr) -> bool {
        match self.mem.u32(holder) {
            1 => {
                let p = self.mem.u32(holder + 4);
                self.mem.u32(p) != 0
            }
            2 => {
                let var = self.mem.u32(holder + 4);
                self.var_read_notify(var);
                self.var_u32(var) as u8 != 0
            }
            _ => false,
        }
    }

    // ----- waves -----

    fn wave_message(&mut self, obj: ObjId, msg: u32, args: Addr) {
        let Some(w) = self.wave(obj) else { return };
        let flags = w.res.flags;
        let voice = w.voice;
        let streamed = w.streamed;
        // Streamed waves: markers, transitions and callbacks.
        if streamed && self.stream_message(obj, msg, args) {
            return;
        }
        match msg {
            0 => {
                if streamed {
                    self.stream_play(obj);
                } else if let Some(v) = voice {
                    self.voice_play(v);
                }
            }
            1 => {
                if flags & 4 != 0 {
                    if let Some(v) = voice {
                        if streamed {
                            self.stream_stop_voice(obj);
                        }
                        let end_value = self.voice_stop(v);
                        if !streamed {
                            match end_value {
                                EndStop::Immediately => self.static_end_notify(obj),
                                EndStop::AtTurn(t) => self.fins_deferred.push((t, obj)),
                                EndStop::NoSource => {}
                            }
                        }
                        self.voice_release(v);
                        if let Some(w) = self.wave_mut(obj) {
                            w.voice = None;
                        }
                    }
                }
            }
            2 => {
                if let Some(v) = voice {
                    if flags & 4 == 0 {
                        if streamed {
                            self.stream_stop_voice(obj);
                        }
                        if self.voice_stop(v) == EndStop::Immediately && !streamed {
                            self.static_end_notify(obj);
                        }
                    } else if streamed {
                        self.stream_pause_voice(obj, true);
                        self.voice_pause(v, true);
                    } else {
                        self.voice_pause(v, true);
                    }
                }
            }
            3 => {
                if let Some(v) = voice {
                    if streamed {
                        self.stream_pause_voice(obj, false);
                    }
                    self.voice_pause(v, false);
                }
            }
            4 => {
                let playing = voice.is_some_and(|v| self.voice_is_playing(v));
                self.holder_set(args, playing);
            }
            6 => {
                let v = self.mem.f32(args);
                self.wave_set_volume(obj, 0, v);
            }
            7 => {
                let v = self.wave(obj).map_or(0.0, |w| w.volume_slots[0]);
                self.mem.set_f32(args, v);
            }
            8 => {
                let slot = self.wave_alloc_slot(obj, false);
                self.mem.set_u32(args, slot);
            }
            9 => {
                let i = self.mem.u32(args) as i32;
                self.wave_free_slot(obj, false, i);
            }
            10 => {
                let i = self.mem.u32(args) as i32;
                let v = self.mem.f32(args + 4);
                self.wave_set_volume(obj, i, v);
            }
            0xc => {
                let v = self.mem.f32(args);
                self.wave_set_pitch(obj, 0, v);
            }
            0xd => {
                let v = self.wave(obj).map_or(1.0, |w| w.pitch_slots[0]);
                self.mem.set_f32(args, v);
            }
            0xe => {
                let slot = self.wave_alloc_slot(obj, true);
                self.mem.set_u32(args, slot);
            }
            0xf => {
                let i = self.mem.u32(args) as i32;
                self.wave_free_slot(obj, true, i);
            }
            0x10 => {
                let i = self.mem.u32(args) as i32;
                let v = self.mem.f32(args + 4);
                self.wave_set_pitch(obj, i, v);
            }
            0x12 => {
                let v = self.mem.f32(args);
                if let Some(w) = self.wave_mut(obj) {
                    w.pan = v;
                }
                if let Some(vo) = voice {
                    if flags & 0x10 != 0 {
                        self.voice_set(vo, |x| x.pan = v);
                    }
                }
            }
            0x13 => {
                let v = self.wave(obj).map_or(0.5, |w| w.pan);
                self.mem.set_f32(args, v);
            }
            0x14 => {
                let v = self.mem.f32(args);
                if let Some(w) = self.wave_mut(obj) {
                    w.fx = v;
                }
                if let Some(vo) = voice {
                    self.voice_set(vo, |x| x.fx = v);
                }
            }
            0x15 => {
                let v = self.wave(obj).map_or(1.0, |w| w.fx);
                self.mem.set_f32(args, v);
            }
            0x16 => {
                // 2D settings: pan, pitch, volume, fourth.
                let p = [self.mem.f32(args), self.mem.f32(args + 4), self.mem.f32(args + 8), self.mem.f32(args + 12)];
                if let Some(w) = self.wave_mut(obj) {
                    w.pan = p[0];
                    w.pitch = p[1];
                    w.volume = p[2];
                    w.fx = p[3];
                }
                if let Some(vo) = voice {
                    self.wave_set_volume(obj, 1, p[2]);
                    self.wave_set_pitch(obj, 1, p[1]);
                    if flags & 0x10 != 0 {
                        self.voice_set(vo, |x| {
                            x.pan = p[0];
                            x.fx = p[3];
                        });
                    }
                }
            }
            0x1e => self.holder_set(args, flags >> 3 & 1 != 0),
            0x1f => self.holder_set(args, false),
            0x3eb => {
                let v = self.mem.u32(args);
                if let Some(w) = self.wave_mut(obj) {
                    w.end_notif = v;
                }
            }
            _ => *self.unhandled.entry(("wave", msg)).or_default() += 1,
        }
    }

    /// Volume of a slot: clamped to 0 dB at the top, −100 dB at the bottom.
    fn wave_set_volume(&mut self, obj: ObjId, slot: i32, v: f32) {
        let Some(w) = self.wave_mut(obj) else { return };
        let v = if v > 0.0 {
            0.0
        } else if v < -100.0 {
            -100.0
        } else {
            v
        };
        if let Some(s) = w.volume_slots.get_mut(slot as usize) {
            *s = v;
        }
        self.wave_recalc_volume(obj);
    }

    /// Pitch of a slot: slot 1 (Doppler) only with 0x20.
    fn wave_set_pitch(&mut self, obj: ObjId, slot: i32, v: f32) {
        let Some(w) = self.wave_mut(obj) else { return };
        if w.res.flags & 0x20 != 0 || slot != 1 {
            if let Some(s) = w.pitch_slots.get_mut(slot as usize) {
                *s = v;
            }
            self.wave_recalc_pitch(obj);
        }
    }

    fn wave_alloc_slot(&mut self, obj: ObjId, pitch: bool) -> u32 {
        let Some(w) = self.wave_mut(obj) else {
            return 0xffff;
        };
        for i in 0..3u32 {
            let bit = if pitch { 0x100 << i } else { 1 << i } as u16;
            if w.slot_bits & bit == 0 {
                w.slot_bits |= bit;
                if pitch {
                    w.pitch_slots[i as usize] = 1.0;
                    if w.pitch_count as u32 <= i {
                        w.pitch_count = i as u16 + 1;
                    }
                } else {
                    w.volume_slots[i as usize] = 0.0;
                    if w.volume_count as u32 <= i {
                        w.volume_count = i as u16 + 1;
                    }
                }
                return i;
            }
        }
        0xffff
    }

    fn wave_free_slot(&mut self, obj: ObjId, pitch: bool, i: i32) {
        let Some(w) = self.wave_mut(obj) else { return };
        let bit = if pitch { (0x100u32 << (i & 0x1f)) as u16 } else { (1u32 << (i & 0x1f)) as u16 };
        w.slot_bits &= !bit;
        let recalc;
        if pitch {
            if i == w.pitch_count as i32 - 1 {
                w.pitch_count -= 1;
            }
            let s = w.pitch_slots.get_mut(i as usize);
            recalc = match s {
                Some(s) if *s != 1.0 => {
                    *s = 1.0;
                    true
                }
                _ => false,
            };
        } else {
            if i == w.volume_count as i32 - 1 {
                w.volume_count -= 1;
            }
            let s = w.volume_slots.get_mut(i as usize);
            recalc = match s {
                Some(s) if *s != 0.0 => {
                    *s = 0.0;
                    true
                }
                _ => false,
            };
        }
        if recalc {
            if pitch {
                self.wave_recalc_pitch(obj);
            } else {
                self.wave_recalc_volume(obj);
            }
        }
    }

    /// Sum of the reserved volume slots.
    fn wave_recalc_volume(&mut self, obj: ObjId) {
        let Some(w) = self.wave_mut(obj) else { return };
        let mut v = w.volume_slots[0];
        for i in 1..w.volume_count as usize {
            if i < 3 && w.slot_bits & (1 << i) != 0 {
                v += w.volume_slots[i];
            }
        }
        if v > 0.0 {
            v = 0.0;
        }
        if v < -100.0 {
            v = -100.0;
        }
        self.wave_apply_volume(obj, v);
    }

    /// Resulting volume sent to the driver: gain = e^(0.15 × (wave + sound + object)).
    fn wave_apply_volume(&mut self, obj: ObjId, v: f32) {
        let Some(w) = self.wave_mut(obj) else { return };
        let v = if v > 0.0 {
            0.0
        } else if v < -100.0 {
            -100.0
        } else {
            v
        };
        w.volume = v;
        let Some(voice) = w.voice else { return };
        // (wave + sound) + object, × 0.15 (double constant), then e^x through FLDL2E / F2XM1 / FSCALE,
        // on the game thread's 24-bit FPU.
        let wave_gain = w.wave.as_ref().map_or(0.0, |x| x.gain);
        let sum = (wave_gain + w.res.volume) + v;
        let gain = crate::listener::exp_x87((sum as f64 * 0.15) as f32);
        // Gain clamped to [0, 1] by the driver; NaN passes through unchanged.
        let gain = if gain < 0.0 { 0.0 } else if gain > 1.0 { 1.0 } else { gain };
        self.voice_set(voice, |x| x.gain = gain);
    }

    /// Product of the reserved pitch slots.
    fn wave_recalc_pitch(&mut self, obj: ObjId) {
        let Some(w) = self.wave_mut(obj) else { return };
        let mut p = w.pitch_slots[0];
        for i in 1..w.pitch_count as usize {
            if i < 3 && w.slot_bits & (0x100 << i) != 0 {
                p *= w.pitch_slots[i];
            }
        }
        w.pitch = p;
        if let Some(voice) = w.voice {
            self.voice_set(voice, |x| x.pitch = p);
        }
    }

    // ----- driver voices -----
    // HXAudio's DirectSound driver (vtable): creation, play,
    // stop (end callback set), pause and resume,
    // gain clamped to [0, 1], voice released.

    fn voice_create(&mut self, res: &WavRes, wave: &WaveInfo) -> VoiceId {
        let v = Voice {
            res: res.cuuid,
            wave: wave.cuuid,
            looping: res.flags & 1 != 0,
            playing: false,
            paused: false,
            gain: 1.0,
            pitch: 1.0,
            pan: 0.5,
            fx: 1.0,
            position: 0.0,
            frames: wave.frames,
            rate: wave.rate,
            starts: 0,
            serial: {
                self.voice_serial += 1;
                self.voice_serial
            },
            stream: None,
            wait_delay: 0.0,
            launches: 0.0,
            start_value: None,
            pause_launches: 0.0,
            stop_turn: None,
        };
        match self.voices.iter().skip(1).position(|v| v.is_none()) {
            Some(i) => {
                self.voices[i + 1] = Some(v);
                (i + 1) as u32
            }
            None => {
                self.voices.push(Some(v));
                (self.voices.len() - 1) as u32
            }
        }
    }
    pub(crate) fn voice_set(&mut self, v: VoiceId, f: impl FnOnce(&mut Voice)) {
        if let Some(Some(x)) = self.voices.get_mut(v as usize) {
            f(x);
        }
    }
    fn voice_play(&mut self, v: VoiceId) {
        // Playback is only accepted from state 6 (stopped); otherwise the driver logs "f_bPlay failed" and
        // touches nothing (no restart from the beginning of a voice that is playing or paused).
        if self.voices.get(v as usize).and_then(|x| x.as_ref()).is_some_and(|x| x.playing) {
            return;
        }
        let launches = self.time_ms as f64 / 1000.0;
        self.voice_set(v, |x| {
            x.playing = true;
            x.paused = false;
            x.position = 0.0;
            x.wait_delay = static_delay();
            x.launches = launches;
            x.start_value = None;
            x.starts = x.starts.wrapping_add(1);
        });
    }

    fn voice_playing(&self, v: VoiceId) -> bool {
        self.voices.get(v as usize).and_then(|x| x.as_ref()).is_some_and(|x| x.playing)
    }

    /// Driver end callback (set by the static wave's message 0x3EB): fired when a played voice is stopped,
    /// released or finishes; it calls the wave's handler wrapper (vtable + 8, without parameters).
    fn static_end_notify(&mut self, obj: ObjId) {
        #[cfg(test)]
        self.fins_noted.push((obj, self.time_ms));
        let h = self.wave(obj).map_or(0, |w| w.end_notif);
        if h != 0 {
            self.handler_call(h, 0);
        }
    }

    /// First DirectSound thread pass strictly after time `t` (seconds, `time_ms` scale): a Play or a pause
    /// made at the exact time of a pass waits for the next one.
    fn turn_ds_after(&self, t: f64) -> f64 {
        let period = crate::stream::ds8_period();
        let last_turn = self.time_ms as f64 / 1000.0 - self.ds8_clock;
        last_turn + (((t - last_turn) / period + 1e-9).floor() + 1.0) * period
    }

    /// Channel state of a static buffer in the original (+4 of CDS8Channel): 1 play requested, 2 started, 3 pause requested,
    /// 4 paused, 5 stop requested, 6 stopped. The DirectSound thread does 1 → 2, 3 → 4 and 5 → 6 on its pass.
    fn state_static_channel(&self, v: VoiceId) -> u8 {
        let Some(x) = self.voices.get(v as usize).and_then(|x| x.as_ref()) else { return 6 };
        let now = self.time_ms as f64 / 1000.0;
        if !x.playing {
            6
        } else if x.stop_turn.is_some() {
            5
        } else if x.paused {
            // Pause of a started channel: 3 until the next pass; of a channel not yet started: 4 immediately.
            let launches_before = self.turn_ds_after(x.launches) <= x.pause_launches + 1e-9;
            if launches_before && self.turn_ds_after(x.pause_launches) > now + 1e-9 {
                3
            } else {
                4
            }
        } else if self.turn_ds_after(x.launches) <= now + 1e-9 {
            2
        } else {
            1
        }
    }

    /// Driver stop and release (same transitions): 6 nothing; 1 and 4 → 6 and end callback immediately; 2 and
    /// 3 → 5: the voice still plays and the callback waits for the DirectSound thread's next pass (`stop_turn`, `fins_deferred`).
    fn voice_stop(&mut self, v: VoiceId) -> EndStop {
        let Some(x) = self.voices.get(v as usize).and_then(|x| x.as_ref()) else { return EndStop::NoSource };
        if !x.playing || x.stop_turn.is_some() {
            return EndStop::NoSource;
        }
        let prior = false;
        if !prior && x.stream.is_none() && static_at_turns() && !self.external_audio && matches!(self.state_static_channel(v), 2 | 3) {
            let t = self.time_ms as f64 / 1000.0;
            self.voice_set(v, |x| x.stop_turn = Some(t));
            return EndStop::AtTurn(t);
        }
        self.voice_set(v, |x| x.playing = false);
        EndStop::Immediately
    }
    /// Pause (state 1 → 4, 2 → 3) and resume (4 → 1, 3 → 2): no effect on a stopped voice or one not
    /// in the source state (failing CAS), including a voice with a stop requested (5).
    /// Resume of a paused voice (4 → 1): restarted on the next pass, like a Play.
    fn voice_pause(&mut self, v: VoiceId, paused: bool) {
        let t = self.time_ms as f64 / 1000.0;
        let state_value = self.state_static_channel(v);
        let turns = static_at_turns() && !self.external_audio;
        self.voice_set(v, |x| {
            if x.playing && x.stop_turn.is_none() && x.paused != paused {
                x.paused = paused;
                if paused {
                    x.pause_launches = t;
                } else if state_value == 4 && turns && x.stream.is_none() {
                    x.launches = t;
                    x.start_value = None;
                }
            }
        });
    }
    fn voice_release(&mut self, v: VoiceId) {
        if let Some(slot) = self.voices.get_mut(v as usize) {
            *slot = None;
        }
    }
    pub fn voice_is_playing(&self, v: VoiceId) -> bool {
        match self.voices.get(v as usize).and_then(|x| x.as_ref()) {
            Some(x) => match &x.stream {
                Some(h) => h.lock().map(|s| s.chan.is_playing()).unwrap_or(false),
                // Stop requested: the voice plays until the DirectSound thread's next pass.
                None if x.stop_turn.is_some() => x.playing,
                None => x.playing && (x.looping || x.frames == 0 || x.position < x.frames as f64),
            },
            None => false,
        }
    }

    /// Streamed waves in the engine update: decoders, channels and rings (and
    /// HXAudio's threads).
    pub fn advance_voices(&mut self, dt: f64) {
        self.streams_update(dt);
    }

    /// Static buffers (CDS8Channel channels, vtable): the game thread's Play (state 1) waits for
    /// the DirectSound thread's next pass (state 2: IDirectSoundBuffer::Play, `stream::ds8_period`), and the
    /// cursor starts `wait_delay` later (dsound.dll mixer). The end is the cursor
    /// seen at the Update; in the original it is state 6 set on the DirectSound thread's next pass.
    /// Measured state-2 times fall on the grid
    /// of passes (±2 ms), not the state-1 times. Without waiting for the pass, a voice could end 16 ms early,
    /// its HX actor would be destroyed and the next sound would create another one (one extra CUUID and
    /// rand() draw). The position follows the pitch.
    pub fn advance_static_voices(&mut self, dt: f64) {
        let at_turns = static_at_turns() && !self.external_audio;
        let now = self.time_ms as f64 / 1000.0;
        let period = crate::stream::ds8_period();
        let last_turn = now - self.ds8_clock;
        // First DirectSound thread pass at time `t` or later.
        let turn_after = |t: f64| last_turn + ((t - last_turn) / period - 1e-9).ceil() * period;
        let mut finished_list: Vec<VoiceId> = Vec::new();
        for (vi, x) in self.voices.iter_mut().enumerate().filter_map(|(i, v)| v.as_mut().map(|v| (i as VoiceId, v))) {
            if x.stream.is_some() {
                continue;
            }
            if !at_turns {
                if x.playing && !x.paused {
                    let wait_value = x.wait_delay.min(dt);
                    x.wait_delay -= wait_value;
                    x.position += (dt - wait_value) * x.rate as f64 * x.pitch.max(0.0) as f64;
                    if x.looping && x.frames > 0 {
                        x.position %= x.frames as f64;
                    } else if x.position >= x.frames as f64 {
                        x.playing = false;
                        finished_list.push(vi);
                    }
                }
                continue;
            }
            // Stop requested (state 5): stopped on the DirectSound thread's next pass, which then calls the end callback.
            if let Some(t) = x.stop_turn {
                if turn_after(t) <= now + 1e-9 {
                    x.playing = false;
                    x.stop_turn = None;
                    finished_list.push(vi);
                }
                continue;
            }
            if !x.playing || x.paused {
                continue;
            }
            // Playback started on the pass after the Play, cursor `wait_delay` later.
            let start_value = match x.start_value {
                Some(d) => d,
                None => {
                    let t = turn_after(x.launches);
                    if t > now + 1e-9 {
                        continue;
                    }
                    let d = t + x.wait_delay;
                    x.start_value = Some(d);
                    d
                }
            };
            let from_time = (now - dt).max(start_value);
            if now > from_time {
                x.position += (now - from_time) * x.rate as f64 * x.pitch.max(0.0) as f64;
            }
            let done_at = x.position >= x.frames as f64;
            if x.looping && x.frames > 0 {
                x.position %= x.frames as f64;
            } else if done_at {
                // End: read at the cursor (the game thread's IDirectSoundBuffer::GetStatus), not on a pass.
                x.playing = false;
                finished_list.push(vi);
            }
        }
        // End callbacks of voices released during their stop (state 5): on the pass after the stop.
        let (due_list, remaining_items): (Vec<_>, Vec<_>) = std::mem::take(&mut self.fins_deferred).into_iter().partition(|&(t, _)| turn_after(t) <= now + 1e-9);
        self.fins_deferred = remaining_items;
        for (_, o) in due_list {
            if self.wave(o).is_some() {
                self.static_end_notify(o);
            }
        }
        // End callback of the static waves whose voice just finished.
        for v in finished_list {
            let obj = self.objects.iter().position(|o| matches!(o.as_ref().map(|o| &o.kind), Some(ObjKind::Wave(w)) if w.voice == Some(v) && !w.streamed));
            if let Some(o) = obj {
                self.static_end_notify(o as ObjId);
            }
        }
    }

    // ----- programs -----

    pub(crate) fn program_held(&mut self, obj: ObjId, delta: i32) {
        if let Some(p) = self.program_obj_mut(obj) {
            p.held += delta;
        }
    }

    /// Wrapper of handler `name` of program `owner` (0 if missing).
    pub(crate) fn handler_ref(&mut self, owner: ObjId, name: &[u8]) -> u32 {
        let Some(p) = self.program_obj(owner) else {
            return 0;
        };
        match p.prog.handler_index(name) {
            Some(i) => p.handler_refs.get(i).copied().unwrap_or(0),
            None => 0,
        }
    }

    fn ctx_for(&mut self, obj: ObjId, start: u32, len: u32, stack: u32, params: Addr) -> Option<Ctx> {
        let p = self.program_obj(obj)?;
        let prog = p.prog.clone();
        let locals = p.locals;
        let actor = p.actor;
        let frame = self.mem.alloc(stack as usize);
        Some(Ctx {
            prog,
            owner: obj,
            actor,
            locals,
            code_base: start,
            pc: 0,
            end: len,
            sp: 0,
            frame,
            frame_size: stack,
            params,
            argbase: 0,
            globals: actor != 0,
            actor_vars: actor != 0,
        })
    }

    fn run_ctx(&mut self, mut ctx: Ctx) {
        self.run(&mut ctx);
        self.mem.release(ctx.frame);
    }

    /// Runs handler `index` with parameters `params`.
    pub fn exec_handler(&mut self, obj: ObjId, index: u32, params: Addr) {
        let Some(p) = self.program_obj(obj) else {
            return;
        };
        let Some(h) = p.prog.handlers.get(index as usize).cloned() else {
            return;
        };
        #[cfg(test)]
        if let Some(j) = self.record_log.as_mut() {
            j.push(format!("{obj}.H{index}"));
        }
        if self.trace_calls {
            let id = self.object(obj).map(|o| o.res);
            let line = format!("{:.2} s: handler {} (program {obj}, {id:?})", self.time_ms as f64 / 1000.0, h.name);
            self.logs.push(line);
        }
        if let Some(ctx) = self.ctx_for(obj, h.start, h.len, h.stack, params) {
            self.run_ctx(ctx);
        }
    }

    /// Runs a header block; returns false if missing (offset −1).
    fn run_block(&mut self, obj: ObjId, (start, len): (u32, u32), stack: u32) -> bool {
        if start == u32::MAX {
            return len == 0;
        }
        if let Some(ctx) = self.ctx_for(obj, start, len, stack, 0) {
            self.run_ctx(ctx);
        }
        true
    }

    fn program_message(&mut self, obj: ObjId, msg: u32, args: Addr) {
        let Some(p) = self.program_obj(obj) else {
            return;
        };
        let prog = p.prog.clone();
        let std = |id: usize| prog.std_handler.get(id).copied().flatten();
        match msg {
            0 | 1 | 6 | 0xc => {
                if let Some(h) = std(msg as usize) {
                    self.exec_handler(obj, h, args);
                }
                if msg == 0 {
                    if let Some(p) = self.program_obj_mut(obj) {
                        p.played = true;
                    }
                }
            }
            2 | 3 => {
                let ids: Vec<u32> = self.program_obj(obj).map(|p| p.timers.keys().copied().collect()).unwrap_or_default();
                for id in ids {
                    if msg == 2 {
                        self.timer_pause(obj, id);
                    } else {
                        self.timer_resume(obj, id);
                    }
                }
                if let Some(h) = std(msg as usize) {
                    self.exec_handler(obj, h, args);
                }
            }
            4 => {
                let Some(p) = self.program_obj(obj) else {
                    return;
                };
                let (valid, locals) = (p.valid, p.locals);
                if !valid {
                    self.holder_set(args, false);
                    return;
                }
                match prog.play_mode & 7 {
                    0 => {
                        if let Some(h) = std(4) {
                            self.exec_handler(obj, h, args);
                        } else {
                            self.holder_set(args, true);
                        }
                    }
                    1 => self.holder_set(args, true),
                    2 => {
                        self.holder_set(args, false);
                        for &off in &prog.watched {
                            let sub = self.mem.u32(locals + off as u32);
                            if sub != 0 {
                                self.send(sub, 4, args);
                                if self.holder_get(args) {
                                    return;
                                }
                            }
                        }
                        let any_timer = self.program_obj(obj).is_some_and(|p| p.timers.values().any(|t| t.handler != 0));
                        if any_timer {
                            self.holder_set(args, true);
                        }
                    }
                    3 => {
                        let played = self.program_obj(obj).is_some_and(|p| p.played);
                        self.holder_set(args, !played);
                    }
                    4 => self.holder_set(args, false),
                    // Modes 5 to 7: no case 's switch, the holder is not touched.
                    _ => {}
                }
            }
            5 | 0x3e9 | 0x3ef => {
                // {name, parameters}.
                let name_addr = self.mem.u32(args);
                let params = self.mem.u32(args + 4);
                let name = self.c_string(name_addr);
                if let Some(i) = prog.handler_index(&name) {
                    self.exec_handler(obj, i as u32, params);
                }
            }
            0x16 => {
                if let Some(h) = std(0x16) {
                    self.exec_handler(obj, h, args);
                    return;
                }
                if let Some(h) = std(6) {
                    self.exec_handler(obj, h, args + 8);
                }
                if let Some(h) = std(0xc) {
                    self.exec_handler(obj, h, args + 4);
                }
                if let Some(h) = std(0x12) {
                    self.exec_handler(obj, h, args);
                }
            }
            0x20 => {
                // Attachment to an actor.
                #[cfg(test)]
                if let Some(j) = self.record_log.as_mut() {
                    j.push(format!("{obj}.A"));
                }
                let actor = self.mem.u32(args);
                if let Some(p) = self.program_obj_mut(obj) {
                    p.actor = actor;
                    p.valid = true;
                    p.played = false;
                }
                let (start, len) = prog.attach_block();
                if !prog.code.is_empty() && (start != u32::MAX || len == 0) {
                    // Variable 0 of the object receives the object itself.
                    let locals = self.program_obj(obj).map_or(0, |p| p.locals);
                    self.mem.set_u32(locals, obj);
                    if start != u32::MAX {
                        if let Some(ctx) = self.ctx_for(obj, start, len, prog.stack_attach(), 0) {
                            self.run_ctx(ctx);
                        }
                    }
                }
                #[cfg(test)]
                if let Some(j) = self.record_log.as_mut() {
                    j.push(format!("{obj}.I"));
                }
                self.run_block(obj, prog.init_block(), prog.stack_init());
            }
            1000 => {
                // Does the program handle message *args[0]?
                let id = self.mem.u32(self.mem.u32(args));
                let holder = self.mem.u32(args + 4);
                let has = (id as usize) < 0x21 && std(id as usize).is_some();
                self.holder_set(holder, has);
            }
            0x3ea => {
                let actor = self.program_obj(obj).map_or(0, |p| p.actor);
                self.actor_addref(actor, obj);
            }
            0x3eb => {
                let actor = self.program_obj(obj).map_or(0, |p| p.actor);
                self.actor_release(actor, obj);
            }
            0x3ec => {
                let out = self.mem.u32(args + 4);
                let delay = self.mem.u32(args + 8);
                let handler = self.mem.u32(args + 12);
                let id = if handler == 0 { 0xffff } else { self.timer_add(obj, delay, 0, handler) };
                self.mem.set_u32(out, id);
            }
            0x3ed => {
                let id = self.mem.u32(args);
                self.timer_kill(obj, id);
            }
            0x3ee => {
                // Destruction: end block.
                #[cfg(test)]
                if let Some(j) = self.record_log.as_mut() {
                    j.push(format!("{obj}.T"));
                }
                let ids: Vec<u32> = self.program_obj(obj).map(|p| p.timers.keys().copied().collect()).unwrap_or_default();
                for id in ids {
                    self.timer_stop(obj, id);
                }
                // Without code (fails) nothing changes; otherwise variable 0 of the object is set to
                // zero before the end block, and the object is no longer valid if the block could be set (a missing
                // block is only set with zero length).
                if !prog.code.is_empty() {
                    let locals = self.program_obj(obj).map_or(0, |p| p.locals);
                    self.mem.set_u32(locals, 0);
                    if self.run_block(obj, prog.term_block(), prog.stack_term()) {
                        if let Some(p) = self.program_obj_mut(obj) {
                            p.valid = false;
                        }
                    }
                }
            }
            0x3f0 => {
                let out = self.mem.u32(args + 4);
                let lo = self.mem.u32(args + 8);
                let hi = self.mem.u32(args + 12);
                let range = if lo < hi { hi - lo } else { 0 };
                let handler = self.mem.u32(args + 16);
                let id = if handler == 0 { 0xffff } else { self.timer_add(obj, lo, range, handler) };
                self.mem.set_u32(out, id);
            }
            _ => *self.unhandled.entry(("program", msg)).or_default() += 1,
        }
    }

    fn c_string(&self, addr: Addr) -> Vec<u8> {
        let mut out = Vec::new();
        let mut a = addr;
        loop {
            let b = self.mem.u8(a);
            if b == 0 || out.len() > 0xffff {
                break;
            }
            out.push(b);
            a += 1;
        }
        out
    }

    // ----- timers -----

    /// Timer repeated every `delay` + random(`range`) ms.
    fn timer_add(&mut self, prog: ObjId, delay: u32, range: u32, handler: u32) -> u32 {
        let now = self.time_ms;
        let Some(p) = self.program_obj_mut(prog) else {
            return 0xffff;
        };
        let id = p.next_timer;
        if p.timers.get(&id).is_some_and(|t| t.handler != 0) {
            return id;
        }
        let t = Timer { params: 0, entry: 0, repeat: true, delay, range, start: now, handler };
        p.timers.insert(id, t);
        let next = p.timers.keys().next_back().copied().unwrap_or(0) + 1;
        p.next_timer = next;
        let entry = self.timer_list_add(prog, id, delay, range);
        if let Some(t) = self.program_obj_mut(prog).and_then(|p| p.timers.get_mut(&id)) {
            t.entry = entry;
        }
        id
    }

    /// timeGetTime at the time of a call: in game, the Update's time plus the real time elapsed
    /// since; otherwise (simulations, no record of this time) that of the Update.
    fn time_call(&self) -> u32 {
        match self.update_real {
            Some(i) => self.time_ms.wrapping_add(i.elapsed().as_millis() as u32),
            None => self.time_ms,
        }
    }

    /// Entry of the global list: due = now + delay + random.
    pub(crate) fn timer_list_add(&mut self, prog: ObjId, timer: u32, delay: u32, range: u32) -> u32 {
        // timeGetTime() read at the time of the call, not the time of the last Update:
        // a timer created during the game's Tick (PlaySound starting a program) starts from
        // the real time of that call (several ms after the Update).
        let r = self.rand() as i64;
        // rand() × n / 0x7FFF on 64 bits.
        let extra = (r * range as i64 / 0x7fff) as u32;
        let now = self.time_call();
        let due = extra.wrapping_add(now).wrapping_add(delay);
        self.timer_serial = self.timer_serial.wrapping_add(1).max(1);
        let id = self.timer_serial;
        // New entries go to the head of the list.
        self.timer_list.insert(0, TimerEntry {
            id,
            prog,
            timer,
            due,
            delay,
            range,
            killed: false,
        });
        id
    }

    pub(crate) fn timer_entry_kill_id(&mut self, id: u32) {
        if let Some(e) = self.timer_list.iter_mut().find(|e| e.id == id) {
            e.killed = true;
        }
    }

    /// Stops a timer and releases its handler.
    fn timer_stop(&mut self, prog: ObjId, id: u32) {
        let Some(t) = self.program_obj_mut(prog).and_then(|p| p.timers.get_mut(&id)) else {
            return;
        };
        if t.handler == 0 {
            return;
        }
        let entry = t.entry;
        let handler = t.handler;
        t.entry = 0;
        t.handler = 0;
        if entry != 0 {
            self.timer_entry_kill_id(entry);
        }
        self.handler_release(handler);
    }

    /// KillTimer: stops and removes the timer.
    fn timer_kill(&mut self, prog: ObjId, id: u32) {
        self.timer_stop(prog, id);
        if let Some(p) = self.program_obj_mut(prog) {
            p.timers.remove(&id);
        }
    }

    /// Pause: keeps the remaining time of a non-repeating timer.
    fn timer_pause(&mut self, prog: ObjId, id: u32) {
        let now = self.time_ms;
        let Some(t) = self.program_obj_mut(prog).and_then(|p| p.timers.get_mut(&id)) else {
            return;
        };
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

    /// Resume.
    fn timer_resume(&mut self, prog: ObjId, id: u32) {
        let Some(t) = self.program_obj(prog).and_then(|p| p.timers.get(&id)).cloned() else {
            return;
        };
        if t.handler != 0 && t.entry == 0 {
            let e = self.timer_list_add(prog, id, t.delay, t.range);
            if let Some(t) = self.program_obj_mut(prog).and_then(|p| p.timers.get_mut(&id)) {
                t.entry = e;
            }
        }
    }

    /// Release of a handler wrapper (vtable+0: message 0x3EB to the owner).
    pub(crate) fn handler_release(&mut self, handler: u32) {
        if let Some(Some(h)) = self.handler_refs.get(handler as usize).copied() {
            if h.native == 0 {
                self.send(h.owner, 0x3eb, 0);
            }
        }
    }

    /// Wrapper of a native subsystem handler (created once per code).
    pub fn native_handler(&mut self, code: u32) -> u32 {
        if let Some(&h) = self.native_handlers.get(&code) {
            return h;
        }
        self.handler_refs.push(Some(HandlerRef { owner: 0, index: 0, native: code }));
        let h = (self.handler_refs.len() - 1) as u32;
        self.native_handlers.insert(code, h);
        h
    }

    /// Call of a wrapper (vtable+8): program, or native code recorded for the subsystem.
    pub(crate) fn handler_call(&mut self, handler: u32, params: Addr) {
        let Some(Some(h)) = self.handler_refs.get(handler as usize).copied() else { return };
        if h.native != 0 {
            self.native_calls.push(h.native);
        } else {
            self.exec_handler(h.owner, h.index, params);
        }
    }

    /// Timer state (debugging).
    pub fn debug_timers(&self) -> String {
        let list: Vec<String> =
            self.timer_list.iter().map(|e| format!("#{} prog {} t{} due {} {}", e.id, e.prog, e.timer, e.due, if e.killed { "killed" } else { "" })).collect();
        list.join(" | ")
    }

    /// Pass over the timer list: entries added during the pass
    /// (at the head) wait for the next pass; the due time advances from the old one.
    pub fn tick_timers(&mut self) {
        let now = self.time_ms;
        let order: Vec<u32> = self.timer_list.iter().map(|e| e.id).collect();
        for &id in &order {
            let Some(i) = self.timer_list.iter().position(|e| e.id == id) else {
                continue;
            };
            let e = self.timer_list[i];
            if e.killed {
                self.timer_list.remove(i);
                continue;
            }
            let part = e.due <= now;
            if part {
                self.timer_fire(e.prog, e.timer);
                let r = self.rand() as i64;
                let extra = (r * e.range as i64 / 0x7fff) as u32;
                if let Some(x) = self.timer_list.iter_mut().find(|x| x.id == id) {
                    let step_size = extra.wrapping_add(x.delay);
                    x.due = x.due.wrapping_add(step_size);
                }
            }
        }
    }

    /// Timer callback (wrapper vtable+8 =).
    fn timer_fire(&mut self, prog: ObjId, id: u32) {
        if self.stream(prog).is_some() {
            self.stream_timer_fire(prog, id as usize);
            return;
        }
        let Some(t) = self.program_obj(prog).and_then(|p| p.timers.get(&id)).cloned() else {
            return;
        };
        let Some(Some(h)) = self.handler_refs.get(t.handler as usize).copied() else {
            return;
        };
        if !t.repeat {
            if t.entry != 0 {
                self.timer_entry_kill_id(t.entry);
            }
            if let Some(t) = self.program_obj_mut(prog).and_then(|p| p.timers.get_mut(&id)) {
                t.entry = 0;
                t.handler = 0;
            }
            self.exec_handler(h.owner, h.index, t.params);
            self.handler_release(t.handler);
        } else {
            self.exec_handler(h.owner, h.index, t.params);
        }
    }

    // ----- variables -----

    pub fn new_var(&mut self, name: &[u8], kind: u32, ty: u32) -> VarId {
        self.vars.push(Some(Var { name: name.to_vec(), kind, ty, value: 0, cuuid: [0; 2], text: Vec::new(), hooks: Vec::new() }));
        (self.vars.len() - 1) as u32
    }

    /// Variable created by its type with an optional initialisation value (vtable+0xC of the types): for simple ones, the value is copied then
    /// the listeners notified — integer, flo, string, boolean, character
    /// (byte); CUUID, object (registry object found by CUUID, otherwise null) and kind 5 (16-bit
    /// word) without notifying anyone.
    pub fn new_var_init(&mut self, name: &[u8], kind: u32, ty: u32, init: Option<InitVar>) -> VarId {
        let v = self.new_var(name, kind, ty);
        match init {
            None => {}
            Some(InitVar::Word(x)) if kind == K_SIMPLE => {
                self.set_var_u32(v, if ty == T_BOOL || ty == crate::interp::T_CHAR { x & 0xFF } else { x });
                self.var_changed(v);
            }
            Some(InitVar::Word(x)) => self.set_var_u32(v, x & 0xFFFF),
            Some(InitVar::Text(t)) => {
                self.set_var_string(v, &t);
                self.var_changed(v);
            }
            Some(InitVar::Cuuid(c)) => self.set_var_cuuid(v, c),
            Some(InitVar::ObjectVar(o)) => self.set_var_u32(v, o.unwrap_or(0)),
        }
        v
    }

    /// Creates a global variable (`CGlobalVarAccess` vtable+4).
    pub fn create_global(&mut self, name: &str, kind: u32, ty: u32) -> VarId {
        if let Some(&v) = self.globals.get(name.as_bytes()) {
            return v;
        }
        let v = self.new_var(name.as_bytes(), kind, ty);
        self.globals.insert(name.as_bytes().to_vec(), v);
        v
    }

    /// Creates an actor variable (`CActorProgramContext` vtable CVarAccess+4).
    pub fn create_actor_var(&mut self, actor: ObjId, name: &str, kind: u32, ty: u32) -> VarId {
        if let Some(&v) = self.actor(actor).and_then(|a| a.vars.get(name.as_bytes())) {
            return v;
        }
        let v = self.new_var(name.as_bytes(), kind, ty);
        if let Some(a) = self.actor_mut(actor) {
            a.vars.insert(name.as_bytes().to_vec(), v);
        }
        v
    }

    /// CGlobalVarTable::Remove: the variable with this name leaves the global array and is destroyed; false if it is not there.
    pub fn delete_global(&mut self, name: &str) -> bool {
        match self.globals.remove(name.as_bytes()) {
            Some(v) => {
                if let Some(slot) = self.vars.get_mut(v as usize) {
                    *slot = None;
                }
                true
            }
            None => false,
        }
    }

    pub(crate) fn global_var(&mut self, name: &[u8]) -> VarId {
        self.globals.get(name).copied().unwrap_or(0)
    }

    pub(crate) fn actor_var(&mut self, actor: ObjId, name: &[u8]) -> VarId {
        self.actor(actor).and_then(|a| a.vars.get(name).copied()).unwrap_or(0)
    }

    fn var(&self, v: VarId) -> Option<&Var> {
        self.vars.get(v as usize).and_then(|x| x.as_ref())
    }
    fn var_mut(&mut self, v: VarId) -> Option<&mut Var> {
        self.vars.get_mut(v as usize).and_then(|x| x.as_mut())
    }
    pub fn var_kind(&self, v: VarId) -> u32 {
        self.var(v).map_or(u32::MAX, |x| x.kind)
    }
    pub fn var_type(&self, v: VarId) -> u32 {
        self.var(v).map_or(u32::MAX, |x| x.ty)
    }
    pub fn var_u32(&self, v: VarId) -> u32 {
        self.var(v).map_or(0, |x| x.value)
    }
    pub fn var_f32(&self, v: VarId) -> f32 {
        f32::from_bits(self.var_u32(v))
    }
    pub fn var_cuuid(&self, v: VarId) -> [u32; 2] {
        self.var(v).map_or([0; 2], |x| x.cuuid)
    }
    pub fn var_string(&self, v: VarId) -> Vec<u8> {
        self.var(v).map_or_else(Vec::new, |x| x.text.clone())
    }
    pub fn set_var_u32(&mut self, v: VarId, value: u32) {
        if let Some(x) = self.var_mut(v) {
            x.value = value;
        }
    }
    pub fn set_var_f32(&mut self, v: VarId, value: f32) {
        self.set_var_u32(v, value.to_bits());
    }
    pub fn set_var_cuuid(&mut self, v: VarId, value: [u32; 2]) {
        if let Some(x) = self.var_mut(v) {
            x.cuuid = value;
        }
    }
    pub fn set_var_string(&mut self, v: VarId, text: &[u8]) {
        if let Some(x) = self.var_mut(v) {
            x.text = text.to_vec();
        }
    }
    pub fn add_hook(&mut self, v: VarId, hook: Hook) {
        if let Some(x) = self.var_mut(v) {
            if !x.hooks.contains(&hook) {
                x.hooks.push(hook);
            }
        }
    }

    /// Notifies the listeners of a variable modified by the subsystem.
    pub fn var_changed(&mut self, v: VarId) {
        self.var_write_notify(v);
    }

    /// Read: the listeners have nothing to do here in XIII.
    pub(crate) fn var_read_notify(&mut self, _v: VarId) {}

    /// Write: notifies the listeners.
    pub(crate) fn var_write_notify(&mut self, v: VarId) {
        self.notes_write += 1;
        let hooks = self.var(v).map(|x| x.hooks.clone()).unwrap_or_default();
        for h in hooks {
            match h {
                Hook::Modifier { actor, volume } => self.modifier_recalc(actor, volume),
                Hook::Subsystem(code) => {
                    let (actor, kind) = match self.hook_actor_vars {
                        Some((name, kind)) => (self.var_string(name), self.var_u32(kind) as i32),
                        None => (Vec::new(), 0),
                    };
                    let ev = HookEvent { code, actor, kind, text: self.var_string(v), float: self.var_f32(v), int: self.var_u32(v) };
                    self.hook_events.push(ev);
                }
            }
        }
    }

    // ----- actors -----

    /// Value of a modifier: pitch = base × variables, volume = base + variables,
    /// then broadcast SetPitchEx/SetVolumeEx slot 2.
    pub fn modifier_recalc(&mut self, actor: ObjId, volume: bool) {
        let Some(a) = self.actor(actor) else { return };
        let m = if volume { &a.volume_mod } else { &a.pitch_mod };
        let mut value = m.base;
        // Variables are in a std::map ordered by CVar address: creation order
        // here (the heap places them at increasing addresses at startup).
        let mut vars: Vec<VarId> = m.vars.iter().map(|&(v, _)| v).collect();
        vars.sort_unstable();
        for var in vars {
            let x = self.var_f32(var);
            value = if volume { x + value } else { x * value };
        }
        if let Some(a) = self.actor_mut(actor) {
            if volume {
                a.volume_mod.value = value;
            } else {
                a.pitch_mod.value = value;
            }
        }
        let msg = if volume { 10 } else { 0x10 };
        self.send_with(actor, msg, &[2, value.to_bits()]);
    }

    /// Removes a variable from a modifier (CActorModifiers vtable+4:; vtable+0x10: which also removes the modifier from
    /// the variable's listeners), then recomputes the value.
    pub fn modifier_remove(&mut self, actor: ObjId, volume: bool, var: VarId, of_listeners: bool) {
        if of_listeners {
            if let Some(x) = self.var_mut(var) {
                x.hooks.retain(|h| *h != Hook::Modifier { actor, volume });
            }
        }
        if let Some(a) = self.actor_mut(actor) {
            let m = if volume { &mut a.volume_mod } else { &mut a.pitch_mod };
            m.vars.retain(|(v, _)| *v != var);
        }
        self.modifier_recalc(actor, volume);
    }

    /// Adds a variable to a modifier (vtable+0xC).
    pub fn modifier_add(&mut self, actor: ObjId, volume: bool, var: VarId, factor: f32) {
        self.add_hook(var, Hook::Modifier { actor, volume });
        if let Some(a) = self.actor_mut(actor) {
            let m = if volume { &mut a.volume_mod } else { &mut a.pitch_mod };
            if !m.vars.iter().any(|(v, _)| *v == var) {
                m.vars.push((var, factor));
            }
        }
        self.modifier_recalc(actor, volume);
    }

    fn actor_message(&mut self, actor: ObjId, msg: u32, args: Addr) {
        match msg {
            0x18 => {
                let p = [self.mem.f32(args), self.mem.f32(args + 4), self.mem.f32(args + 8)];
                self.actor_set_position(actor, p);
            }
            6 => {
                let v = self.mem.f32(args);
                if let Some(a) = self.actor_mut(actor) {
                    a.volume_mod.base = v;
                }
                self.modifier_recalc(actor, true);
            }
            7 => {
                let v = self.actor(actor).map_or(0.0, |a| a.volume_mod.base);
                self.mem.set_f32(args, v);
            }
            0x44c => {
                let r = [self.mem.f32(args), self.mem.f32(args + 4), self.mem.f32(args + 8), self.mem.f32(args + 12)];
                if let Some(a) = self.actor_mut(actor) {
                    a.rolloff = r;
                }
            }
            0x1a => {
                let v = [self.mem.f32(args), self.mem.f32(args + 4), self.mem.f32(args + 8)];
                if let Some(a) = self.actor_mut(actor) {
                    a.vel = v;
                    a.dirty = true;
                }
            }
            0x44d => self.actor_kill(actor),
            _ => self.actor_broadcast(actor, msg, args),
        }
    }

    /// SetPosition (vtable+8).
    pub fn actor_set_position(&mut self, actor: ObjId, p: [f32; 3]) {
        if let Some(a) = self.actor_mut(actor) {
            a.pos = p;
            a.dirty = true;
        }
    }

    /// Broadcast of a message to the actor's objects.
    fn actor_broadcast(&mut self, actor: ObjId, msg: u32, args: Addr) {
        let entries: Vec<(ObjId, ObjId, bool)> =
            self.actor(actor).map(|a| a.entries.iter().map(|e| (e.obj, e.owner, e.intercept)).collect()).unwrap_or_default();
        for (obj, owner, intercept) in entries {
            if owner != 0 && intercept && self.owner_handles(owner, msg) {
                continue;
            }
            self.send(obj, msg, args);
        }
    }

    fn entry_index(&self, actor: ObjId, obj: ObjId) -> Option<usize> {
        self.actor(actor)?.entries.iter().position(|e| e.obj == obj)
    }

    pub(crate) fn actor_addref(&mut self, actor: ObjId, obj: ObjId) {
        if let Some(i) = self.entry_index(actor, obj) {
            if let Some(a) = self.actor_mut(actor) {
                a.entries[i].refs += 1;
            }
        }
    }

    pub(crate) fn actor_release(&mut self, actor: ObjId, obj: ObjId) {
        if let Some(i) = self.entry_index(actor, obj) {
            if let Some(a) = self.actor_mut(actor) {
                a.entries[i].refs -= 1;
            }
        }
    }

    /// Call by name on the actor (vtable+0x10 → actor object vtable+0x10 → message 0x3E9).
    pub(crate) fn actor_call(&mut self, actor: ObjId, name: &[u8], args: Addr) {
        #[cfg(test)]
        if let Some(l) = self.sends.as_mut() {
            l.1.push((actor, name.to_vec(), args));
        }
        let mut text = name.to_vec();
        text.push(0);
        let name_addr = self.mem.alloc_with(&text);
        let block = self.mem.alloc(8);
        self.mem.set_u32(block, name_addr);
        self.mem.set_u32(block + 4, args);
        self.actor_broadcast(actor, 0x3e9, block);
        self.mem.release(block);
        self.mem.release(name_addr);
    }

    /// New entry.
    fn new_entry(&mut self, obj: ObjId, owner: ObjId, key: u64, channel: u32) -> Entry {
        let (single_2d, intercept) = self.entry_pool.pop().unwrap_or((true, false));
        let serial = self.entry_serial;
        self.entry_serial = self.entry_serial.wrapping_add(1);
        let mut e = Entry {
            obj,
            serial,
            res2d: true,
            playable: true,
            follows: false,
            positioned: true,
            single_2d,
            intercept,
            closing: false,
            key,
            channel,
            owner,
            refs: 0,
        };
        // Class traits: waves and programs are "Playable" and "Res2D".
        let (playable, res2d) = match self.object(obj).map(|o| &o.kind) {
            Some(ObjKind::Wave(_)) | Some(ObjKind::Silent(_)) | Some(ObjKind::Program(_)) => (true, true),
            Some(ObjKind::Actor(_)) => (true, true),
            None => (false, false),
        };
        e.res2d = res2d;
        e.playable = playable;
        e.follows = self.ask(obj, 0x1f);
        e.positioned = self.ask(obj, 0x1e);
        if owner != 0 {
            let owner_is_program = self.program_obj(owner).is_some();
            e.intercept = owner_is_program;
            if owner_is_program {
                e.single_2d = true;
                for m in [6, 0xc, 0x12, 0x16] {
                    if self.owner_handles(owner, m) {
                        e.single_2d = false;
                    }
                }
            }
        }
        e
    }

    /// Attaches an object created by the subsystem (vtable+0).
    pub fn actor_attach_top(&mut self, actor: ObjId, obj: ObjId, key: u64, channel: u32, hold: bool) {
        let e = self.new_entry(obj, 0, key, channel);
        let Some(a) = self.actor_mut(actor) else {
            return;
        };
        a.entries.insert(0, e);
        if hold {
            a.entries[0].refs += 1;
        }
        self.after_attach(actor, obj);
    }

    /// Attaches an object created by a program (vtable+4).
    pub(crate) fn actor_attach(&mut self, actor: ObjId, obj: ObjId, owner: ObjId, hold: bool) {
        let Some(owner_entry) = self.entry_index(actor, owner) else {
            return;
        };
        let (owner_obj, key, channel) = {
            let a = self.actor(actor).unwrap();
            let o = &a.entries[owner_entry];
            (o.obj, o.key, o.channel)
        };
        let e = self.new_entry(obj, owner_obj, key, channel);
        let Some(a) = self.actor_mut(actor) else {
            return;
        };
        a.entries.insert(0, e);
        if hold {
            a.entries[0].refs += 1;
        }
        self.after_attach(actor, obj);
    }

    fn after_attach(&mut self, actor: ObjId, obj: ObjId) {
        match self.object(obj).map(|o| &o.kind) {
            Some(ObjKind::Program(_)) => {
                self.send_with(obj, 0x20, &[actor]);
            }
            Some(ObjKind::Wave(_)) => {
                // Slots 1 (attenuation) and 2 (actor modifiers).
                self.send_with(obj, 8, &[0]);
                self.send_with(obj, 8, &[0]);
                self.send_with(obj, 0xe, &[0]);
                self.send_with(obj, 0xe, &[0]);
            }
            _ => {}
        }
        if let Some(i) = self.entry_index(actor, obj) {
            self.apply_actor(actor, i, true);
        }
    }

    /// 2D parameters of the actor: computed by the listener if needed.
    pub fn actor_params2d(&mut self, actor: ObjId) -> [f32; 4] {
        let Some(a) = self.actor(actor) else {
            return [0.5, 1.0, 0.0, 0.5];
        };
        if !a.params_valid {
            let (pos, rolloff, vel, pan) = (a.pos, a.rolloff, a.vel, a.pan_coeff);
            let p = crate::listener::params(&self.listeners, pos, vel, rolloff, pan);
            if let Some(a) = self.actor_mut(actor) {
                a.params2d = p;
                a.params_valid = true;
            }
        }
        self.actor(actor).map_or([0.5, 1.0, 0.0, 0.5], |a| a.params2d)
    }

    /// Send to an entry's object, unless the owning program handles the message.
    fn entry_send(&mut self, actor: ObjId, index: usize, msg: u32, words: &[u32]) {
        let Some(e) = self.actor(actor).and_then(|a| a.entries.get(index)).cloned() else {
            return;
        };
        if e.owner != 0 && e.intercept && self.owner_handles(e.owner, msg) {
            return;
        }
        self.send_with(e.obj, msg, words);
    }

    /// Applies the actor's settings to an entry.
    pub fn apply_actor(&mut self, actor: ObjId, index: usize, volume_pitch: bool) {
        let Some(e) = self.actor(actor).and_then(|a| a.entries.get(index)).cloned() else {
            return;
        };
        let (flight, pitch, is3d, pos) = {
            let a = self.actor(actor).unwrap();
            (a.volume_mod.value, a.pitch_mod.value, a.is3d, a.pos)
        };
        if volume_pitch && e.playable {
            self.entry_send(actor, index, 10, &[2, flight.to_bits()]);
            self.entry_send(actor, index, 0x10, &[2, pitch.to_bits()]);
        }
        if !is3d || !e.positioned {
            return;
        }
        if e.follows {
            self.entry_send(actor, index, 0x18, &[pos[0].to_bits(), pos[1].to_bits(), pos[2].to_bits()]);
            return;
        }
        if !e.playable {
            return;
        }
        let p = self.actor_params2d(actor);
        if e.single_2d {
            self.entry_send(actor, index, 0x16, &[p[0].to_bits(), p[1].to_bits(), p[2].to_bits(), p[3].to_bits()]);
        } else {
            self.entry_send(actor, index, 0x12, &[p[0].to_bits()]);
            self.entry_send(actor, index, 0x14, &[p[3].to_bits()]);
            self.entry_send(actor, index, 10, &[1, p[2].to_bits()]);
            self.entry_send(actor, index, 0x10, &[1, p[1].to_bits()]);
        }
    }

    /// Cleanup of finished objects without references.
    pub fn actor_cleanup(&mut self, actor: ObjId) {
        let entries: Vec<Entry> = self.actor(actor).map(|a| a.entries.clone()).unwrap_or_default();
        let mut dead = Vec::new();
        for e in &entries {
            if e.refs == 0 && e.res2d {
                let skip = e.owner != 0 && e.intercept && self.owner_handles(e.owner, 4);
                let playing = if skip { false } else { self.ask(e.obj, 4) };
                if !playing {
                    dead.push(e.obj);
                }
            }
        }
        for obj in dead {
            let Some(a) = self.actor_mut(actor) else {
                return;
            };
            let Some(i) = a.entries.iter().position(|x| x.obj == obj) else {
                continue;
            };
            let e = a.entries.remove(i);
            for x in a.entries.iter_mut() {
                if x.owner == obj {
                    x.owner = 0;
                }
            }
            self.drop_entry(&e);
            self.destroy_object(obj);
        }
    }

    /// End of an entry: pool and key release.
    fn drop_entry(&mut self, e: &Entry) {
        self.entry_pool.push((e.single_2d, e.intercept));
        if e.channel != 0 {
            if let Some(c) = self.channel_counts.get_mut(e.channel as usize) {
                *c -= 1;
            }
        }
    }

    /// Objects of the actor attached with key `key` (vtable+0x20).
    pub fn actor_find_key(&self, actor: ObjId, key: u64) -> Vec<ObjId> {
        self.actor(actor).map(|a| a.entries.iter().filter(|e| e.key == key).map(|e| e.obj).collect()).unwrap_or_default()
    }

    /// Objects attached to the actor (vtable+0xC).
    pub fn actor_objects(&self, actor: ObjId) -> Vec<ObjId> {
        self.actor(actor).map(|a| a.entries.iter().map(|e| e.obj).collect()).unwrap_or_default()
    }

    /// Update of an actor: cleanup then 3D settings.
    pub fn actor_update(&mut self, actor: ObjId) {
        self.actor_cleanup(actor);
        let is3d = self.actor(actor).is_some_and(|a| a.is3d);
        if is3d {
            if let Some(a) = self.actor_mut(actor) {
                a.params_valid = false;
            }
            let n = self.actor(actor).map_or(0, |a| a.entries.len());
            for i in 0..n {
                self.apply_actor(actor, i, false);
            }
            if let Some(a) = self.actor_mut(actor) {
                a.dirty = false;
            }
        }
    }

    /// Number of attached objects (vtable+0xC).
    pub fn actor_count(&self, actor: ObjId) -> usize {
        self.actor(actor).map_or(0, |a| a.entries.len())
    }

    /// Kill (0x44D): destroys the programs, removes what is no longer held.
    pub fn actor_kill(&mut self, actor: ObjId) {
        let entries: Vec<ObjId> = self.actor(actor).map(|a| a.entries.iter().map(|e| e.obj).collect()).unwrap_or_default();
        for obj in &entries {
            if self.program_obj(*obj).is_some() {
                self.send(*obj, 0x3ee, 0);
            }
        }
        // Passes over the list as long as its head exists and is not closing; each pass removes the entries
        // without references (stopping at the first closing entry), and when a pass removes no more
        // (same number of held entries as the previous one, 10000 at the start) the references of the remaining entries are
        // set to zero: everything goes on the next pass.
        let mut before_value = 10000usize;
        loop {
            let Some(a) = self.actor(actor) else {
                return;
            };
            if a.entries.first().is_none_or(|e| e.closing) {
                break;
            }
            let mut held_value = 0usize;
            let mut i = 0;
            loop {
                let Some(a) = self.actor_mut(actor) else {
                    return;
                };
                let Some(e) = a.entries.get(i) else { break };
                if e.closing {
                    break;
                }
                if e.refs == 0 {
                    let e = a.entries.remove(i);
                    self.drop_entry(&e);
                    self.destroy_object(e.obj);
                } else {
                    held_value += 1;
                    i += 1;
                }
            }
            if held_value == before_value {
                if let Some(a) = self.actor_mut(actor) {
                    for e in a.entries.iter_mut().take_while(|e| !e.closing) {
                        e.refs = 0;
                    }
                }
            }
            before_value = held_value;
        }
        let rest: Vec<ObjId> = self.actor(actor).map(|a| a.entries.iter().map(|e| e.obj).collect()).unwrap_or_default();
        for obj in rest {
            self.send_with(obj, 0x20, &[actor]);
        }
    }
}

/// Reads a `CPCWavResData`.
pub fn parse_wavres(data: &[u8], offset: usize) -> Option<WavRes> {
    let mut p = offset;
    let u32_at = |p: usize| -> Option<u32> { data.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) };
    let class_len = u32_at(p)? as usize;
    p += 4 + class_len;
    let cuuid = Cuuid(u32_at(p)?, u32_at(p + 4)?);
    p += 8;
    let version = u32_at(p)?;
    p += 4;
    let mut name = String::new();
    if version > 1 {
        let n = u32_at(p)? as usize;
        name = data.get(p + 4..p + 4 + n)?.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        p += 4 + n;
    }
    let volume = f32::from_bits(u32_at(p)?);
    let x50 = f32::from_bits(u32_at(p + 4)?);
    let pitch = f32::from_bits(u32_at(p + 8)?);
    p += 12;
    let mut flags = if version >= 3 {
        let f = *data.get(p)?;
        p += 1;
        f
    } else {
        // CWavResData::Read: old four-byte booleans.
        // Constructor sets flags 0x1C before they are read.
        let mut f = 0x1c;
        if u32_at(p)? != 0 {
            f |= 1;
        }
        p += 4;
        if version < 2 {
            for _ in 0..5 {
                let _ = u32_at(p)?;
                p += 4;
            }
        } else {
            for bit in [8, 4] {
                if u32_at(p)? != 0 {
                    f |= bit;
                } else {
                    f &= !bit;
                }
                p += 4;
            }
        }
        (f | 0x10) & !0x20
    };
    let wave = Cuuid(u32_at(p)?, u32_at(p + 4)?);
    p += 8;
    // Before version 3, the presence of languages follows the CUUID.
    if version < 3 {
        if u32_at(p)? != 0 {
            flags |= 2;
        } else {
            flags &= !2;
        }
        p += 4;
    }
    let x18 = if version >= 4 {
        let v = u32_at(p)?;
        p += 4;
        v
    } else {
        0
    };
    let x1c = if version >= 5 {
        let v = u32_at(p)? != 0;
        p += 4;
        v
    } else {
        false
    };
    let mut languages = Vec::new();
    if flags & 2 != 0 {
        let n = u32_at(p)?;
        p += 4;
        for _ in 0..n {
            languages.push((u32_at(p)?, Cuuid(u32_at(p + 4)?, u32_at(p + 8)?)));
            p += 12;
        }
    }
    Some(WavRes { cuuid, version, name, volume, x50, pitch, flags, wave, x18, x1c, languages })
}

/// Own gain of a wave (word following the version of a `CPCWaveFileIdObj`).
fn wave_gain(data: &[u8], id: Cuuid, bank: &openxiii_assets::hx::HxBank) -> f32 {
    let Some(e) = bank.entries.iter().find(|e| e.cuuid == id) else {
        return 0.0;
    };
    let p = e.offset as usize;
    let Some(n) = data.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else {
        return 0.0;
    };
    let at = p + 4 + n + 8 + 4;
    // A positive gain is reset to 0 on read (0 < gain → gain = 0).
    let g = data.get(at..at + 4).map_or(0.0, |b| f32::from_le_bytes(b.try_into().unwrap()));
    if g > 0.0 {
        0.0
    } else {
        g
    }
}

#[allow(dead_code)]
const UNUSED: [u32; 5] = [K_CUUID, K_OBJECT, K_SIMPLE, T_BOOL, T_STRING + T_FLOAT + T_INT];

/// Source of a streamed wave: external file (looked up case-insensitively next to
/// the bank), position, channels, allowed duration ("ump3" chunk, otherwise the wave's duration).
// Path of the external stream file relative to the bank.
#[path = "stream_path.rs"]
mod stream_path;

fn stream_source(bank_path: Option<&std::path::Path>, w: &openxiii_assets::hx::Wave, frames: u64, format_tag: u16) -> Option<StreamSource> {
    let dir = bank_path?.parent()?;
    let (path, offset) = match &w.location {
        openxiii_assets::hx::Location::External { resource, offset, .. } => {
            let direct = stream_path::resolve(bank_path?, resource)?;
            let name = direct.file_name()?.to_string_lossy().into_owned();
            let path = if direct.exists() {
                direct
            } else {
                std::fs::read_dir(direct.parent().unwrap_or(dir))
                    .ok()?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(&name)))?
            };
            (path, *offset as u64)
        }
        openxiii_assets::hx::Location::Internal { offset, .. } => (bank_path?.to_path_buf(), *offset as u64),
    };
    let total = w.ump3.map_or(frames as u32, |u| u.1);
    let data_size = match w.location {
        openxiii_assets::hx::Location::Internal { size, .. } | openxiii_assets::hx::Location::External { size, .. } => size,
    };
    Some(StreamSource { path, offset, channels: w.channels, format_tag, data_size, memory: w.mode & 4 != 0, bits: w.bits, total })
}

