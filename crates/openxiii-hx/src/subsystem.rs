//! Unreal's audio subsystem for XIII (`UHXAUDIOSubsystem`, HXAudio.dll): connects the
//! game's actors and Sound objects to the HX engine.
//!
//! - One HX actor (`CActorResObj`) per game actor and per category: sound effects (`SoundEffect`,
//!   several per actor depending on attenuation), voice, "HF" voice; plus fixed actors
//!   (Music, HFVoice, StrAmb, UnderWater, Menu, Ryan, MicroCanon).
//! - A simple sound is created, attached to the actor and played; a "Programme__hName" sound runs
//!   handler hName of the program, a single program instance per actor.
//! - Footsteps (`PlaySndXIIIStep` / `PlaySndPNJStep`) and shouts (`PlaySndPNJOno` / `PlaySndDeathOno`)
//!   pick their sounds at random from the footstep and onomatopoeia objects.
//! - `update` follows the game's actors (position, velocity), cleans up finished sounds and destroys
//!   empty HX actors.

use crate::engine::{ActorRes, Engine, Hook, ObjId, VarId};
use crate::interp::{K_SIMPLE, T_BOOL, T_FLOAT, T_INT, T_STRING};
use crate::listener::Listener;
use openxiii_assets::hx::Cuuid;
use std::collections::HashMap;

/// Categories ("slots") of UHXAUDIOSubsystem::GetHXActor.
pub const SLOT_EFFECT: i32 = 1;
pub const SLOT_VOICE: i32 = 2;
pub const SLOT_HFVOICE: i32 = 4;
/// Codes of the native handlers of HF voice markers.
const NATIVE_HF_PAUSE: u32 = 1;
const NATIVE_HF_UNPAUSE: u32 = 2;
pub const SLOT_STRAMB: i32 = 8;
pub const SLOT_MUSIC: i32 = 0x10;
pub const SLOT_MENU: i32 = 0x20;
pub const SLOT_RYAN: i32 = 0x40;
pub const SLOT_MICRO_BARREL: i32 = 0x80;
pub const SLOT_UNDERWATER: i32 = 0x100;
pub const SLOT_EFFECT_SINGLE: i32 = 0x200;

/// `[HXAudio.HXAudioSubsystem]` settings (User.ini, values from DefUser.ini).
#[derive(Debug, Clone)]
pub struct Config {
    pub master_volume: f32,
    pub music_light_volume: f32,
    pub music_normal_volume: f32,
    pub music_slider_pos: i32,
    pub sd_volume: f32,
    /// + 0x134: minimum interval between two launches of the same sound (CheckLaunchTime).
    pub min_check_time: f32,
    /// + 400 / + 0x194: speeds below which footsteps are silent / played at "walk" volume.
    pub walk_speed: f32,
    pub run_speed: f32,
    pub sound_mode: i32,
    /// + 0x118: SoundProject, prefix of the starting music handlers.
    pub sound_project: String,
    /// + 0x154: SoundLanguage ([HXAudio.HXAudioSubsystem] of User.ini), language of the spoken lines.
    pub sound_language: String,
    /// + 0x184: Superviser (mask 2), copied into `HxAudio::log_ono` (diagnostic logs). In the original, it also hooks
    /// Init and Update to Ubisoft's debug supervisor (Cosy network link, no effect on the game).
    pub supervise: bool,
    /// + 0x13C: CheckLoading.
    pub check_loading: bool,
    /// + 0x18C: EditorVisuVolume.
    pub editor_view_volume: f32,
    /// + 0x230: LevelBanks.
    pub level_banks: bool,
}

impl Default for Config {
    fn default() -> Self {
        // Absent from the game's .ini files, the other values stay at zero as in the original.
        Config {
            master_volume: 0.0,
            music_light_volume: -6.0,
            music_normal_volume: 0.0,
            music_slider_pos: 2,
            sd_volume: 0.0,
            min_check_time: 0.0,
            walk_speed: 0.0,
            run_speed: 0.0,
            sound_mode: 1,
            sound_project: "XIIISound".to_string(),
            sound_language: "French".to_string(),
            supervise: false,
            check_loading: false,
            editor_view_volume: 0.0,
            level_banks: true,
        }
    }
}

/// Configuration properties created by UHXAUDIOSubsystem::StaticConstructor (IMPLEMENT_CLASS), in
/// creation order: name, kind (b boolean, f float, i integer, s string), offset in the object, flags (CPF_Config, plus
/// CPF_NeedCtorLink for strings), category. "User" configuration (ClassConfigName), section [HXAudio.HXAudioSubsystem].
pub const PROPERTIES_CONFIG: [(&str, u8, u32, u32, &str); 15] = [
    ("Superviser", b'b', 0x184, 0x4000, "HXAUDIO"),
    ("CheckLoading", b'b', 0x13C, 0x4000, "HXAUDIO"),
    ("MinCheckTime", b'f', 0x134, 0x4000, "HXAUDIO"),
    ("SoundMode", b'i', 0x160, 0x4000, "HXAUDIO"),
    ("MasterVolume", b'f', 0x164, 0x4000, "HXAUDIO"),
    ("MusicLightVolume", b'f', 0x170, 0x4000, "HXAUDIO"),
    ("MusicNormalVolume", b'f', 0x174, 0x4000, "HXAUDIO"),
    ("MusicSliderPos", b'i', 0x178, 0x4000, "HXAUDIO"),
    ("SDVolume", b'f', 0x17C, 0x4000, "HXAUDIO"),
    ("SoundProject", b's', 0x118, 0x404000, "Client"),
    ("SoundLanguage", b's', 0x154, 0x404000, "Client"),
    ("WalkSpeed", b'f', 0x190, 0x4000, "HXAUDIO"),
    ("RunSpeed", b'f', 0x194, 0x4000, "HXAUDIO"),
    ("EditorVisuVolume", b'f', 0x18C, 0x4000, "HXAUDIO"),
    ("LevelBanks", b'b', 0x230, 0x4000, "HXAUDIO"),
];

/// `Config` field of a `PROPERTIES_CONFIG` property.
pub enum FieldConfig<'a> {
    BoolVar(&'a mut bool),
    FloatVar(&'a mut f32),
    Integer(&'a mut i32),
    StringVar(&'a mut String),
}

impl Config {
    /// Class values before UObject::LoadConfig (UClass::Register): all zero, the C++ constructor sets no setting.
    pub fn zero() -> Config {
        Config {
            master_volume: 0.0,
            music_light_volume: 0.0,
            music_normal_volume: 0.0,
            music_slider_pos: 0,
            sd_volume: 0.0,
            min_check_time: 0.0,
            walk_speed: 0.0,
            run_speed: 0.0,
            sound_mode: 0,
            sound_project: String::new(),
            sound_language: String::new(),
            supervise: false,
            check_loading: false,
            editor_view_volume: 0.0,
            level_banks: false,
        }
    }

    /// Field of property `name_value` of `PROPERTIES_CONFIG` (case-insensitive, like an FName).
    pub fn field_value(&mut self, name_value: &str) -> Option<FieldConfig<'_>> {
        Some(match name_value.to_ascii_lowercase().as_str() {
            "superviser" => FieldConfig::BoolVar(&mut self.supervise),
            "checkloading" => FieldConfig::BoolVar(&mut self.check_loading),
            "minchecktime" => FieldConfig::FloatVar(&mut self.min_check_time),
            "soundmode" => FieldConfig::Integer(&mut self.sound_mode),
            "mastervolume" => FieldConfig::FloatVar(&mut self.master_volume),
            "musiclightvolume" => FieldConfig::FloatVar(&mut self.music_light_volume),
            "musicnormalvolume" => FieldConfig::FloatVar(&mut self.music_normal_volume),
            "musicsliderpos" => FieldConfig::Integer(&mut self.music_slider_pos),
            "sdvolume" => FieldConfig::FloatVar(&mut self.sd_volume),
            "soundproject" => FieldConfig::StringVar(&mut self.sound_project),
            "soundlanguage" => FieldConfig::StringVar(&mut self.sound_language),
            "walkspeed" => FieldConfig::FloatVar(&mut self.walk_speed),
            "runspeed" => FieldConfig::FloatVar(&mut self.run_speed),
            "editorvisuvolume" => FieldConfig::FloatVar(&mut self.editor_view_volume),
            "levelbanks" => FieldConfig::BoolVar(&mut self.level_banks),
            _ => return None,
        })
    }
}

/// What HXAudio reads from a game actor.
#[derive(Debug, Clone, Default)]
pub struct ActorState {
    /// Game-side identifier of the actor (table key).
    pub id: u32,
    pub name: String,
    pub location: [f32; 3],
    pub velocity: [f32; 3],
    pub has_position: bool,
    pub has_rolloff: bool,
    pub underwater: bool,
    pub saturation_distance: f32,
    pub stabilisation_distance: f32,
    pub stabilisation_volume: f32,
    pub voices_saturation_distance: f32,
    pub voices_stabilisation_distance: f32,
    pub voices_stabilisation_volume: f32,
    pub pan_coeff: f32,
    /// Offset of a Mover (its sound position = Location + offset).
    pub mover_offset: Option<[f32; 3]>,
    pub left_foot: bool,
    pub ambient_sound: Option<SoundRef>,
}

impl ActorState {
    fn sound_position(&self) -> [f32; 3] {
        match self.mover_offset {
            Some(o) => [self.location[0] + o[0], self.location[1] + o[1], self.location[2] + o[2]],
            None => self.location,
        }
    }
    /// Actor with position and attenuation (bHasPosition and bHasRollOff).
    fn positioned(&self) -> bool {
        self.has_position && self.has_rolloff
    }
}

/// Game Sound object: path, name, CUUID of the resource and kind (handler if the name
/// contains "__", USound::PostLoad).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRef {
    pub path: String,
    pub name: String,
    pub cuuid: Cuuid,
    pub handler: bool,
}

impl SoundRef {
    /// Handler name: what follows "__" in the name.
    pub fn handler_name(&self) -> String {
        match self.name.find("__") {
            Some(i) => self.name[i + 2..].to_string(),
            None => String::new(),
        }
    }
}

/// Weighted sound (`ProbaSound`: {Sound, Proba}).
#[derive(Debug, Clone)]
pub struct ProbabilitySound {
    pub sound: Option<SoundRef>,
    pub probability: i32,
}

/// `USndXIIIStep` (player footsteps).
#[derive(Debug, Clone)]
pub struct XiiiStep {
    pub land_sound: Option<SoundRef>,
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub end_step: Option<SoundRef>,
    pub stumble: ProbabilitySound,
    pub left_steps: Vec<ProbabilitySound>,
    pub right_steps: Vec<ProbabilitySound>,
    pub onos_land: Vec<ProbabilitySound>,
    pub onos_land_probability_sum: i32,
    pub onos_run: Vec<ProbabilitySound>,
    pub onos_run_probability_sum: i32,
    pub silent_volume: f32,
    pub walk_volume: f32,
}

/// `USndPNJStep` (character footsteps): one sound per footstep category.
#[derive(Debug, Clone)]
pub struct NPCStep {
    pub land_sound: Option<SoundRef>,
    pub pitch_min: f32,
    pub pitch_max: f32,
    pub left_steps: Vec<Option<SoundRef>>,
    pub right_steps: Vec<Option<SoundRef>>,
}

/// `UPNJOno` / `UDeathOno`: per mesh, three timbres of four sounds.
#[derive(Debug, Clone)]
pub struct OnoSet {
    pub timbres: Vec<[Option<SoundRef>; 12]>,
    /// + 0x34 (shouts only): probability in percent.
    pub probability: i32,
}

/// Events to send back to the game (AActor::OnoStartNotif, VoiceEndNotif…).
#[derive(Debug, Clone, PartialEq)]
pub enum Notify {
    OnoStart(u32),
    VoiceEnd(u32),
    VoicePause(u32),
    VoiceUnPause(u32),
    BeginBeginFlash(u32),
    EndBeginFlash(u32),
    BeginEndFlash(u32),
    EndEndFlash(u32),
}

/// What the subsystem asks of the game.
pub trait World {
    fn actor(&mut self, id: u32) -> Option<ActorState>;
    /// Game actor named `name` (StaticFindObject).
    fn find_actor(&mut self, name: &str) -> Option<u32>;
    /// Sound object whose resource has CUUID `id`.
    fn sound_by_cuuid(&mut self, id: Cuuid) -> Option<SoundRef>;
    /// Writes the actor's bLeftFoot (toggled on each step).
    fn set_left_foot(&mut self, id: u32, left: bool);
    /// LevelInfo.MusicVars[index]: (name, value, type 0 = integer, 1 = boolean).
    fn music_var(&mut self, index: usize) -> Option<(String, i32, u8)>;
}

/// A USound's own data (+ 0x28, + 0x58, + 0x64, + 0x68).
#[derive(Debug, Clone)]
struct Slot {
    res: Option<Cuuid>,
    key: u64,
    channel: u32,
    last_launch: f64,
}

/// Current "HF" voice (structure + 0xAC).
#[derive(Debug, Clone, Default)]
pub(crate) struct HfVoice {
    pub(crate) active: bool,
    pause: bool,
    unpause: bool,
    shared: bool,
    unreal: u32,
    actor: ObjId,
    notify: u32,
}

/// Micro (a view's listener): previous position and time for the velocity.
#[derive(Debug, Clone, Default)]
pub(crate) struct Micro {
    pub(crate) listener: Listener,
    last_time: f64,
}

pub struct HxAudio {
    pub engine: Engine,
    pub config: Config,
    /// + 0x184: log of shout launches, separate from the volume settings.
    pub log_ono: bool,
    slots: HashMap<String, Slot>,
    program_keys: HashMap<Cuuid, u64>,
    /// Keys of the sounds launched by CUUID (table + 0x68).
    cuuid_keys: HashMap<Cuuid, u64>,
    /// The map's spoken lines by name (tables + 0x88 and + 0x68 of LoadMapSounds): resource,
    /// key, channel.
    str_sounds: HashMap<String, (Cuuid, u64, u32)>,
    /// Game folder and bank of the loaded map (LoadMapSounds).
    game_dir: std::path::PathBuf,
    map_bank: Option<String>,
    next_key: u64,
    /// Improved build's co-op: the waves each Unreal actor (a player who is not XIII) plays
    /// in place of XIII's voice, given to that actor's HX actors (`Engine::wave_swaps`).
    pub actor_wave_swaps: HashMap<u32, std::rc::Rc<crate::engine::WaveSwaps>>,
    pub(crate) effects: Vec<(u32, ObjId)>,
    pub(crate) effects_single: Vec<(u32, ObjId)>,
    voices: Vec<(u32, ObjId)>,
    pub(crate) hf: HfVoice,
    orphans: Vec<ObjId>,
    pub(crate) ambient: Vec<u32>,
    ambient_playing: Vec<(u32, String)>,
    pub music: ObjId,
    pub hfvoice_actor: ObjId,
    pub stramb: ObjId,
    pub underwater: ObjId,
    pub menu: ObjId,
    pub ryan: ObjId,
    pub micro_barrel: ObjId,
    var_actor_type: VarId,
    var_actor_name: VarId,
    var_master: VarId,
    /// gglobalvolume: volume line 0xFFF (SetVolumeLine), separate from the .ini's MasterVolume setting (`Config::master_volume`, +0x164).
    pub(crate) gglobalvolume: f32,
    var_sound_design: VarId,
    var_global_sd: VarId,
    channel_max: [i32; 8],
    pub(crate) micros: Vec<Micro>,
    /// Creation rank of each micro in `micros` (order of the listeners in the engine's pool).
    rank_micros: Vec<u64>,
    /// Listeners of the micros replaced by SetViewport: the original does not destroy the old micro (replaces the
    /// table's pointer), its listener stays in the pool without being updated any more, even after ResetMicros, which
    /// only destroys the table's micros. With their creation rank.
    lost_listeners: Vec<(u64, Listener)>,
    next_micro: u64,
    music_key: u64,
    music_init: Option<SoundRef>,
    /// + 0x21C: names of the MusicVars variables of the last LoadSounds; the next one removes them from the global variable array.
    music_var_names: Vec<String>,
    /// LevelInfo.MusicVars[0..3]: current descriptions for the +184 diagnostic.
    pub music_debug_vars: Vec<(String, i32, u8)>,
    /// + 0x2C / + 0x30: end of attack while enemies remain on alert; the infiltration
    /// music waits WaitingTime seconds (ALevelInfo::execDecAttaque, Update).
    pub attack_wait: bool,
    pub attack_wait_start: f64,
    /// Start of the wait requested by the script (execDecAttaque): taken at the time of the next
    /// Update (`wait_to_place`), like the measurement done in Update.
    pub wait_to_place: bool,
    /// + 0x138: paused categories (PauseSound); global pause (counter) and resume
    /// deferred by two updates.
    paused_kinds: i32,
    /// + 0x130: toggled by the ASTAT AUDIO command (Exec); HXAudio never reads it back (only the object's copy and
    /// operator=, dead code, copy it).
    pub astat_audio: bool,
    pub(crate) system_pause: i32,
    pub(crate) system_resume_delay: i32,
    flash_manager: Option<u32>,
    flash_value: i32,
    /// Block of the HF voice for the current Update (voice active when it started).
    hf_block: bool,
    next_actor_res: u32,
    /// Deferred StopSound "All": actor (+0xA4, none for "All") and categories (+0xA8).
    pub(crate) pending_stop: Option<(Option<u32>, i32)>,
    /// rand() of the ParallelUpdateThread thread (msvcr70 seed 1 for a new thread).
    pub rand_thread: crate::rand::MsvcRand,
    /// Time in seconds (game clock).
    pub time: f64,
    pub time_level: f64,
    /// In game (real clock): the time of each Update is recorded to date the timer creations
    /// made later in the frame (`Engine::time_call`).
    pub real_time: bool,
    /// HXAudio clock imposed for the next update (timeGetTime recorded from
    /// the original, in seconds on the `time` scale).
    pub time_impose: Option<f64>,
    dt_current: f64,
    /// Free play (no imposed time): the update thread (ParallelUpdateThread) runs as in the original,
    /// every 31.25 ms (Sleep(30): two Windows timer ticks), HXUpdate only updating the engine if more than
    /// 30 ms have elapsed since its last update. `thread_last`: timeGetTime of that last update; `thread_t`: next pass of the thread
    /// (seconds of `time`); `engine_t`: time of the engine's last advance.
    free: bool,
    thread_last: u32,
    thread_t: f64,
    engine_t: f64,
    /// Minimum audible volume (+ 0x38 = −40 dB) and for voices (+ 0x3C = −15 dB).
    min_volume: f32,
    min_voice_volume: f32,
    pub notifications: Vec<Notify>,
}

pub(crate) const HOOK_CHANGE_VOLUME: u32 = 1;
pub(crate) const HOOK_PLAY_SOUND: u32 = 2;
pub(crate) const HOOK_STOP_SOUND: u32 = 3;
pub(crate) const HOOK_CALL_HANDLER: u32 = 4;
const HOOK_CALL_MUSIC_HANDLER: u32 = 5;
const HOOK_FLASH: u32 = 6;

impl HxAudio {
    /// UHXAUDIOSubsystem::Init: global variables and fixed actors.
    pub fn new(mut engine: Engine, config: Config) -> HxAudio {
        let mut s = HxAudio {
            engine: Engine::new(),
            log_ono: config.supervise,
            config,
            slots: HashMap::new(),
            program_keys: HashMap::new(),
            cuuid_keys: HashMap::new(),
            str_sounds: HashMap::new(),
            game_dir: std::path::PathBuf::new(),
            map_bank: None,
            next_key: 1,
            actor_wave_swaps: HashMap::new(),
            effects: Vec::new(),
            effects_single: Vec::new(),
            voices: Vec::new(),
            hf: HfVoice::default(),
            orphans: Vec::new(),
            ambient: Vec::new(),
            ambient_playing: Vec::new(),
            music: 0,
            hfvoice_actor: 0,
            stramb: 0,
            underwater: 0,
            menu: 0,
            ryan: 0,
            micro_barrel: 0,
            var_actor_type: 0,
            var_actor_name: 0,
            var_master: 0,
            gglobalvolume: 0.0,
            var_sound_design: 0,
            var_global_sd: 0,
            channel_max: [0, 6, 6, 6, 6, 6, 6, 0],
            micros: Vec::new(),
            rank_micros: Vec::new(),
            lost_listeners: Vec::new(),
            next_micro: 0,
            music_key: 0,
            music_init: None,
            music_var_names: Vec::new(),
            music_debug_vars: Vec::new(),
            attack_wait: false,
            attack_wait_start: 0.0,
            wait_to_place: false,
            paused_kinds: 0,
            astat_audio: false,
            system_pause: 0,
            system_resume_delay: 0,
            flash_manager: None,
            flash_value: 0,
            hf_block: false,
            next_actor_res: 1,
            pending_stop: None,
            rand_thread: crate::rand::MsvcRand::new(1),
            time: 0.0,
            time_level: 0.0,
            real_time: false,
            time_impose: None,
            dt_current: 0.0,
            free: false,
            thread_last: 0,
            thread_t: 0.0,
            engine_t: 0.0,
            min_volume: -40.0,
            min_voice_volume: -15.0,
            notifications: Vec::new(),
        };
        let e = &mut engine;
        let v = e.create_global("ChangeVolume", K_SIMPLE, T_FLOAT);
        e.set_var_f32(v, 0.0);
        e.add_hook(v, Hook::Subsystem(HOOK_CHANGE_VOLUME));
        s.var_actor_type = e.create_global("ActorType", K_SIMPLE, T_INT);
        s.var_actor_name = e.create_global("ActorName", K_SIMPLE, T_STRING);
        e.hook_actor_vars = Some((s.var_actor_name, s.var_actor_type));
        for (name, hook) in
            [("PlaySound", HOOK_PLAY_SOUND), ("StopSound", HOOK_STOP_SOUND), ("CallHandler", HOOK_CALL_HANDLER), ("CallMusicHandler", HOOK_CALL_MUSIC_HANDLER)]
        {
            let v = e.create_global(name, K_SIMPLE, T_STRING);
            e.add_hook(v, Hook::Subsystem(hook));
        }
        e.create_global("iEffect", K_SIMPLE, T_INT);
        s.var_global_sd = e.create_global("GlobalSDFlag", K_SIMPLE, T_INT);
        s.engine = engine;
        s.music = s.special_actor("Music");
        let flash = s.engine.create_actor_var(s.music, "Flash", K_SIMPLE, T_INT);
        s.engine.add_hook(flash, Hook::Subsystem(HOOK_FLASH));
        let wait = s.engine.create_actor_var(s.music, "WaitingTime", K_SIMPLE, T_FLOAT);
        let _ = wait;
        s.engine.create_actor_var(s.music, "bMusicOff", K_SIMPLE, T_BOOL);
        s.hfvoice_actor = s.special_actor("HFVoice");
        s.stramb = s.special_actor("StrAmb");
        let flash = s.engine.create_actor_var(s.stramb, "Flash", K_SIMPLE, T_INT);
        s.engine.add_hook(flash, Hook::Subsystem(HOOK_FLASH));
        s.underwater = s.special_actor("UnderWater");
        s.menu = s.special_actor("Menu");
        s.ryan = s.special_actor("Ryan");
        s.micro_barrel = s.special_actor("MicroCanon");
        s.var_master = s.engine.create_global("Master", K_SIMPLE, T_FLOAT);
        let master = s.config.master_volume;
        s.gglobalvolume = master;
        s.engine.set_var_f32(s.var_master, master);
        s.engine.var_changed(s.var_master);
        for a in [s.stramb, s.music, s.hfvoice_actor, s.menu, s.ryan, s.micro_barrel, s.underwater] {
            s.engine.modifier_add(a, true, s.var_master, 1.0);
        }
        s.var_sound_design = s.engine.create_global("SoundDesign", K_SIMPLE, T_FLOAT);
        let sd = s.config.sd_volume;
        s.engine.set_var_f32(s.var_sound_design, sd);
        s.engine.var_changed(s.var_sound_design);
        for a in [s.music, s.stramb] {
            s.engine.modifier_add(a, true, s.var_sound_design, 1.0);
        }
        // The "SoundEffect" and "Voice" lines that GetHXActor and SetVolumeLine look for are
        // created neither by Init nor by any game bank.
        let pos = s.config.music_slider_pos;
        s.set_music_position(pos);
        s
    }

    /// SetMusicPosition (vtable + 0xBC): menu music slider; 0 turns the
    /// music off (bMusicOff of the Music actor). The line volume requested for 0x10 (MusicLight /
    /// MusicNormalVolume) has no effect: SetVolumeLine only handles 0xFFF.
    pub fn set_music_position(&mut self, pos: i32) {
        let Some(&v) = self.engine.actor(self.music).and_then(|a| a.vars.get(b"bMusicOff".as_slice())) else {
            return;
        };
        let off = match pos {
            0 => true,
            1 | 2 => false,
            _ => return,
        };
        self.engine.set_var_u32(v, off as u32);
        self.engine.var_changed(v);
    }

    /// Resource of a new HX actor: its CIdObj receives a fresh CUUID (rdtsc,
    /// one rand() of the game thread, the system time; srand(rdtsc) the very first time). The identifier
    /// stays a counter here, but the draw is made.
    fn new_actor_res(&mut self, name: &str, is3d: bool) -> Cuuid {
        let _ = self.engine.cuuid_rand();
        let id = Cuuid(0xac70_0000, self.next_actor_res);
        self.next_actor_res += 1;
        // Default attenuation of a CActorResData (FD0 = type 0, 100, 10, −100); GetHXActor
        // replaces it with message 0x44C, fixed actors (Music, Menu…) keep it.
        self.engine.add_actor_res(id, ActorRes { name: name.to_string(), is3d, rolloff: crate::engine::ROLLOFF_DEFAULT });
        id
    }

    fn special_actor(&mut self, name: &str) -> ObjId {
        let id = self.new_actor_res(name, false);
        self.engine.create_object(id).unwrap_or(0)
    }

    /// Shared "Fix" bank, loaded once with the subsystem (game project), and game
    /// folder for the map banks.
    pub fn init_project(&mut self, game: &std::path::Path) -> Vec<String> {
        self.game_dir = game.to_path_buf();
        self.engine.load_banks(game, &["Fix"])
    }

    /// Banks of a map without going through LoadSounds (tools): Fix then the map.
    pub fn load_map(&mut self, game: &std::path::Path, map: &str) -> Vec<String> {
        let mut loaded = self.init_project(game);
        loaded.extend(self.load_map_sounds(map));
        loaded
    }

    /// LoadMapSounds: "..\Sounds\PC\<Title>.hxc",
    /// so the level's title and not the map file name (SMarin01 → SM01); the
    /// bank's spoken lines (+0x54, +0x88) become the names for PlayStrVoice.
    pub fn load_map_sounds(&mut self, title: &str) -> Vec<String> {
        self.unload_map_sounds();
        // (language manager, GetLanguageId()) before loading.
        self.engine.language = language_id(&self.config.sound_language);
        let game = self.game_dir.clone();
        let (mut loaded, mut direct) = self.engine.load_banks_direct(&game, &[title]);
        // The same loader tries defaultmulti.hxc
        // if the requested index could not be loaded.
        if loaded.is_empty() {
            (loaded, direct) = self.engine.load_banks_direct(&game, &["defaultmulti"]);
        }
        if loaded.is_empty() {
            return loaded;
        }
        // Unloading applies to the bank actually loaded;
        // the line prefix below stays the requested title.
        self.map_bank = loaded.first().cloned();
        // Spoken lines: bank resources whose name starts with the title ("SM01_Kim_00").
        let prefix = title.to_ascii_lowercase();
        // Skips the insertion if the resource already exists,
        // notably as a program dependency. The loader keeps
        // the CUUID order and its ten frontiers to tell these creations apart.
        let mut list: Vec<(String, Cuuid, u32)> = self
            .engine
            .registry
            .iter()
            .filter(|(id, _)| direct.contains(id))
            .filter_map(|(id, r)| match r {
                crate::engine::ResData::Wav(w) if w.name.to_ascii_lowercase().starts_with(&prefix) => Some((w.name.to_ascii_lowercase(), *id, w.x18)),
                _ => None,
            })
            .collect();
        list.sort();
        for (name, id, channel) in list {
            let key = self.next_key;
            self.next_key += 1;
            self.str_sounds.insert(name, (id, key, channel));
        }
        loaded
    }

    /// Mods build only: spoken line `name` of a bank loaded for the mods (another level's line),
    /// made playable by PlayStrVoice as the map's own lines are, until the map's sounds are
    /// unloaded. False if no loaded resource has that name.
    pub fn add_str_voice(&mut self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        if self.str_sounds.contains_key(&lower) {
            return true;
        }
        let mut found: Vec<(Cuuid, u32)> = self
            .engine
            .registry
            .iter()
            .filter_map(|(id, r)| match r {
                crate::engine::ResData::Wav(w) if w.name.eq_ignore_ascii_case(name) => Some((*id, w.x18)),
                _ => None,
            })
            .collect();
        found.sort();
        let Some(&(id, channel)) = found.first() else { return false };
        let key = self.next_key;
        self.next_key += 1;
        self.str_sounds.insert(lower, (id, key, channel));
        true
    }

    /// UnLoadMapSounds: the map's bank unloaded, spoken lines forgotten.
    fn unload_map_sounds(&mut self) {
        if let Some(b) = self.map_bank.take() {
            self.engine.unload_bank(&b);
        }
        self.str_sounds.clear();
    }

    /// CleanUp (vtable + 0xF8): global pause (PauseSound(0xFFF, 1)), ResetMicros,
    /// ambient actors (+0x204) forgotten, orphans (+0x210) and lists (+0x198, +0x1AC)
    /// destroyed, HF voice stopped (destroyed if not shared), DeleteStrUserData then
    /// UnLoadMapSounds. Called by UGameEngine::LoadMap and KillAllSounds.
    pub fn clean_up(&mut self) {
        self.actor_wave_swaps.clear();
        self.engine.wave_swaps.clear();
        self.pause_sound(0xfff, true);
        self.reset_micros();
        self.ambient.clear();
        self.ambient_playing.clear();
        for hx in std::mem::take(&mut self.orphans) {
            self.kill_actor(hx);
        }
        for (_, hx) in std::mem::take(&mut self.effects).into_iter().chain(std::mem::take(&mut self.effects_single)).chain(std::mem::take(&mut self.voices)) {
            self.kill_actor(hx);
        }
        if self.hf.active {
            self.hf.active = false;
            if !self.hf.shared && self.hf.actor != 0 {
                self.kill_actor(self.hf.actor);
                self.hf.actor = 0;
            }
        }
        self.hf.shared = false;
        self.unload_map_sounds();
    }

    /// ForceSystemResume: global pause counter and deferred resume reset to
    /// zero, PrivateResumeSound(0xFFF, 0).
    fn force_system_resume(&mut self) {
        self.system_pause = 0;
        self.system_resume_delay = 0;
        self.private_resume(0xfff, false);
    }

    /// GetWaveDuration: duration of a named spoken line, in seconds, computed from the
    /// data size as for 16-bit PCM (× 2.5 for Ubi ADPCM); −1 if unknown.
    pub fn wave_duration(&self, name: &str) -> f32 {
        let Some(&(res, _, _)) = self.str_sounds.get(&name.to_ascii_lowercase()) else {
            return -1.0;
        };
        let Some(crate::engine::ResData::Wav(w)) = self.engine.registry.get(&res) else {
            return -1.0;
        };
        let Some(info) = self.engine.wave_of(w) else {
            return -1.0;
        };
        let size = info.data_size as f32;
        let rate2 = (info.rate << 1) as f32;
        let mut d = size / rate2 / info.channels as f32;
        if info.format_tag == 2 {
            d *= 2.5;
        }
        d
    }

    /// WaveHasPosition: the spoken line is positioned in space (flag 8).
    pub fn wave_has_position(&self, name: &str) -> bool {
        // Without a micro (FArray::Num of +0xFC) the answer is no, whether the bank is loaded or not.
        if self.micros.is_empty() {
            return false;
        }
        let Some(&(res, _, _)) = self.str_sounds.get(&name.to_ascii_lowercase()) else {
            return false;
        };
        match self.engine.registry.get(&res) {
            Some(crate::engine::ResData::Wav(w)) => w.flags & 8 != 0,
            _ => false,
        }
    }

    /// PlayStrVoice (vtable + 0x8C): a dialogue line by name, on the actor's HF
    /// voice (or on the shared HFVoice actor for sounds marked 0x40), with the
    /// Pause / Unpause markers; `notify` receives the voice's notifications.
    pub fn play_str_voice(&mut self, notify: u32, actor: Option<&ActorState>, name: &str) -> bool {
        // Without a micro, nothing is started and the return value is 0.
        if self.micros.is_empty() {
            return false;
        }
        let Some(&(res, key, channel)) = self.str_sounds.get(&name.to_ascii_lowercase()) else {
            return false;
        };
        let flags = match self.engine.registry.get(&res) {
            Some(crate::engine::ResData::Wav(w)) => w.flags,
            _ => 0,
        };
        let hx = if flags & 0x40 == 0 {
            self.get_hx_actor(actor, actor, SLOT_HFVOICE)
        } else {
            self.hf.shared = true;
            self.hf.unreal = actor.map_or(0, |a| a.id);
            self.hf.actor = self.hfvoice_actor;
            Some(self.hfvoice_actor).filter(|&a| a != 0)
        };
        let Some(hx) = hx else { return false };
        self.hf.notify = notify;
        let Some(obj) = self.engine.create_object(res) else { return false };
        self.engine.actor_attach_top(hx, obj, key, channel, false);
        self.hf.active = true;
        self.register_hf_markers(obj);
        self.engine.send(obj, 0, 0);
        true
    }

    // ----- Sound objects -----

    /// RegisterSound / LoadID / CreateUserData: resource,
    /// key and channel of a USound (data built here on first request, not all at once).
    fn slot(&mut self, s: &SoundRef) -> Slot {
        if let Some(x) = self.slots.get(&s.path) {
            return x.clone();
        }
        let res = self.engine.registry.contains_key(&s.cuuid).then_some(s.cuuid);
        let channel = match res.and_then(|id| self.engine.registry.get(&id)) {
            Some(crate::engine::ResData::Wav(w)) => w.x18,
            Some(crate::engine::ResData::Program(p)) => p.x18,
            _ => 0,
        };
        // Handler sounds of the same program share their key (CreateUserData).
        let key = if s.handler {
            let next = &mut self.next_key;
            *self.program_keys.entry(s.cuuid).or_insert_with(|| {
                let k = *next;
                *next += 1;
                k
            })
        } else {
            let k = self.next_key;
            self.next_key += 1;
            k
        };
        let slot = Slot { res, key, channel, last_launch: f64::NEG_INFINITY };
        self.slots.insert(s.path.clone(), slot.clone());
        slot
    }

    /// Channel full (ChannelIsFull).
    fn channel_is_full(&mut self, s: &SoundRef) -> bool {
        let channel = self.slot(s).channel;
        // Channel -1 (two waves of the banks): the count and maximum read at + 0xB0 and + 0xB4 are the size and capacity of the
        // dependency array (0x15E and 0x15E, Init): the channel is always full and the sound never plays.
        if channel == u32::MAX {
            return true;
        }
        let c = channel as usize;
        c != 0 && c < 8 && self.channel_max[c] <= self.engine.channel_counts[c]
    }

    /// UpdateChannel: one more sound on its channel.
    fn update_channel(&mut self, s: &SoundRef) {
        let c = self.slot(s).channel as usize;
        if c != 0 && c < 8 {
            self.engine.channel_counts[c] += 1;
        }
    }

    /// CheckLaunchTime: resources marked at + 0x1C are not relaunched too quickly.
    fn check_launch_time(&mut self, s: &SoundRef) -> bool {
        let check = match self.engine.registry.get(&s.cuuid) {
            Some(crate::engine::ResData::Wav(w)) => w.x1c,
            Some(crate::engine::ResData::Program(p)) => p.x1c,
            _ => false,
        };
        if !check {
            return true;
        }
        let now = self.time;
        let min = self.config.min_check_time as f64;
        let Some(slot) = self.slots.get_mut(&s.path) else {
            return true;
        };
        // FSUB in 24-bit precision, then strictly ordered acceptance.
        let elapsed = crate::listener::sum24(now, -slot.last_launch);
        if !(elapsed > min) {
            return false;
        }
        slot.last_launch = now;
        true
    }

    /// Distance to the nearest micro: micro − Location, minus a Mover's offset, the
    /// first one or a strictly smaller one kept. `xzy` chooses the order of the sum: CanHearSound
    /// (length) computes (X² + Y²) + Z², Update's ambient loop
    /// computes (X² + Z²) + Y².
    fn nearest_micro_offset(&self, pos: [f32; 3], offset: Option<[f32; 3]>, xzy: bool) -> Option<f32> {
        let mut best: Option<f32> = None;
        for m in &self.micros {
            let mut d = vec_sub(m.listener.pos, pos);
            if let Some(o) = offset {
                d = vec_sub(d, o);
            }
            let dist = if xzy {
                ((d[0] * d[0] + d[2] * d[2]) + d[1] * d[1]).sqrt()
            } else {
                ((d[0] * d[0] + d[1] * d[1]) + d[2] * d[2]).sqrt()
            };
            if best.is_none_or(|b| dist < b) {
                best = Some(dist);
            }
        }
        best
    }

    /// LaunchHandler: micro − Location without a Mover offset,
    /// length (X² + Y²) + Z².
    fn nearest_micro(&self, pos: [f32; 3]) -> Option<f32> {
        self.nearest_micro_offset(pos, None, false)
    }

    /// CanHearSound.
    pub fn can_hear_sound(&mut self, a: &ActorState, s: &SoundRef) -> bool {
        if !a.positioned() {
            return true;
        }
        let channel = self.slot(s).channel;
        if channel == 5 {
            if self.min_voice_volume <= a.stabilisation_volume {
                return true;
            }
        } else if self.min_volume < a.stabilisation_volume {
            return true;
        }
        let level = match self.engine.registry.get(&s.cuuid) {
            Some(crate::engine::ResData::Wav(w)) => {
                let gain = self.engine.waves.get(&w.wave).map_or(0.0, |x| x.gain);
                gain + w.volume
            }
            _ => 0.0,
        };
        if self.min_volume <= level {
            // Without a micro, the distance stays 0, compared with StabilisationDistance
            let d = self.nearest_micro_offset(a.location, a.mover_offset, false).unwrap_or(0.0);
            if d < a.stabilisation_distance {
                return true;
            }
        }
        false
    }

    /// LaunchHandler: a "mode 2" program does not start out of range.
    fn launch_handler(&mut self, s: &SoundRef, a: Option<&ActorState>, b: Option<&ActorState>, slot: i32) -> bool {
        let mode = match self.engine.registry.get(&s.cuuid) {
            Some(crate::engine::ResData::Program(p)) => p.play_mode & 7,
            _ => return true,
        };
        if mode != 2 {
            return true;
        }
        match slot {
            SLOT_EFFECT | SLOT_VOICE | SLOT_HFVOICE => {
                let Some(a) = a else { return false };
                let b = b.unwrap_or(a);
                if a.positioned() && b.stabilisation_volume <= self.min_volume {
                    let d = self.nearest_micro(a.location).unwrap_or(0.0);
                    if b.stabilisation_distance <= d {
                        return false;
                    }
                }
                true
            }
            SLOT_STRAMB | SLOT_MUSIC | SLOT_MENU | SLOT_RYAN | SLOT_MICRO_BARREL | SLOT_UNDERWATER => true,
            _ => false,
        }
    }

    // ----- HX actors -----

    /// GetHXActor: HX actor of actor `a` for category `slot`, with the attenuation
    /// coming from `b`.
    pub fn get_hx_actor(&mut self, a: Option<&ActorState>, b: Option<&ActorState>, slot: i32) -> Option<ObjId> {
        {
            let r = self.get_hx_actor_without_trace(a, b, slot);
            self.give_wave_swaps(a, slot, r);
            r
        }
    }

    /// The co-op swaps of Unreal actor `a` given to its HX actor `hx` (not to the shared ones).
    fn give_wave_swaps(&mut self, a: Option<&ActorState>, slot: i32, hx: Option<ObjId>) {
        if self.actor_wave_swaps.is_empty() {
            return;
        }
        let (Some(a), Some(hx)) = (a, hx) else { return };
        if !matches!(slot, SLOT_EFFECT | SLOT_EFFECT_SINGLE | SLOT_VOICE) {
            return;
        }
        match self.actor_wave_swaps.get(&a.id) {
            Some(s) => {
                self.engine.wave_swaps.insert(hx, s.clone());
            }
            None => {
                self.engine.wave_swaps.remove(&hx);
            }
        }
    }

    fn get_hx_actor_without_trace(&mut self, a: Option<&ActorState>, b: Option<&ActorState>, slot: i32) -> Option<ObjId> {
        let fixed = match slot {
            SLOT_MENU => Some(self.menu),
            SLOT_STRAMB => Some(self.stramb),
            SLOT_MUSIC => Some(self.music),
            SLOT_UNDERWATER => Some(self.underwater),
            SLOT_RYAN => Some(self.ryan),
            SLOT_MICRO_BARREL => Some(self.micro_barrel),
            _ => None,
        };
        if fixed.is_some() {
            return fixed;
        }
        let line = match slot {
            SLOT_EFFECT | SLOT_EFFECT_SINGLE => "SoundEffect",
            SLOT_VOICE | SLOT_HFVOICE => "Voice",
            _ => return None,
        };
        if slot == SLOT_HFVOICE && self.hf.active {
            let prev = self.hf.unreal;
            // StopSound(HF actor, 0, 4, 1): the previous speaker receives the
            // end of voice (VoiceEndNotif).
            self.stop_sound(Some(prev), None, SLOT_HFVOICE, true);
        }
        let a = a?;
        let b = b.unwrap_or(a);
        // Attenuation: [disabled, stabilisation distance, saturation distance, far volume].
        let mut rolloff = [0.0f32; 4];
        if !a.positioned() {
            rolloff[0] = f32::from_bits(1);
        } else if slot == SLOT_VOICE || slot == SLOT_HFVOICE {
            rolloff = [0.0, b.voices_stabilisation_distance, b.voices_saturation_distance, b.voices_stabilisation_volume];
        } else {
            rolloff = [0.0, b.stabilisation_distance, b.saturation_distance, b.stabilisation_volume];
        }
        match slot {
            SLOT_EFFECT => {
                for &(u, hx) in &self.effects {
                    if u == a.id {
                        if let Some(x) = self.engine.actor(hx) {
                            if x.rolloff[1] == rolloff[1] && x.rolloff[2] == rolloff[2] && x.rolloff[3] == rolloff[3] {
                                return Some(hx);
                            }
                        }
                    }
                }
            }
            SLOT_EFFECT_SINGLE => {
                if let Some(&(_, hx)) = self.effects_single.iter().find(|(u, _)| *u == a.id) {
                    return Some(hx);
                }
            }
            SLOT_VOICE => {
                if let Some(&(_, hx)) = self.voices.iter().find(|(u, _)| *u == a.id) {
                    return Some(hx);
                }
            }
            _ => {}
        }
        let res = self.new_actor_res(&a.name, a.has_position);
        let hx = self.engine.create_object(res)?;
        if !a.has_position {
            self.engine.send_with(hx, 0x44c, &rolloff.map(f32::to_bits));
        } else {
            if self.micros.len() <= 1 {
                if let Some(x) = self.engine.actor_mut(hx) {
                    x.pan_coeff = a.pan_coeff;
                }
            }
            self.engine.actor_set_position(hx, a.sound_position());
            self.engine.send_with(hx, 0x1a, &a.velocity.map(f32::to_bits));
            self.engine.actor_update(hx);
            self.engine.send_with(hx, 0x44c, &rolloff.map(f32::to_bits));
        }
        if let Some(&v) = self.engine.globals.get(line.as_bytes()) {
            self.engine.modifier_add(hx, true, v, 1.0);
        }
        if slot != SLOT_EFFECT_SINGLE && self.var_sound_design != 0 {
            self.engine.modifier_add(hx, true, self.var_sound_design, 1.0);
        }
        if self.var_master != 0 {
            self.engine.modifier_add(hx, true, self.var_master, 1.0);
        }
        let owner = self.engine.create_actor_var(hx, "OwnerName", K_SIMPLE, T_STRING);
        self.engine.set_var_string(owner, a.name.as_bytes());
        self.engine.var_changed(owner);
        match slot {
            SLOT_EFFECT => self.effects.push((a.id, hx)),
            SLOT_VOICE => self.voices.push((a.id, hx)),
            // 0x200: list +0x1AC is never filled (GetHXActor only stores channels 1
            // and 2 there); each call therefore creates an actor and draws a CUUID.
            SLOT_EFFECT_SINGLE => {}
            SLOT_HFVOICE => {
                self.hf.unreal = a.id;
                self.hf.actor = hx;
            }
            _ => {}
        }
        Some(hx)
    }

    // ----- launching -----

    /// "Pause" and "Unpause" markers of an HF voice (PlaySound / PlayStrVoice): native transition
    /// callbacks (message 0x3F4), which set flags + 4 / + 8 of the HF voice.
    fn register_hf_markers(&mut self, obj: ObjId) {
        for (name, code) in [(b"Pause".as_slice(), NATIVE_HF_PAUSE), (b"Unpause".as_slice(), NATIVE_HF_UNPAUSE)] {
            let s = self.engine.mem.new_string(name);
            let h = self.engine.native_handler(code);
            let mut args = s.to_le_bytes().to_vec();
            args.extend_from_slice(&h.to_le_bytes());
            let block = self.engine.mem.alloc_with(&args);
            self.engine.send(obj, 0x3f4, block);
            self.engine.mem.release(block);
            self.engine.mem.delete_string(s);
        }
    }

    /// PlaySound without parameters (vtable + 0xA0).
    pub fn play_sound(&mut self, a: Option<&ActorState>, b: Option<&ActorState>, s: &SoundRef, slot: i32) -> bool {
        // Without a micro, nothing is played.
        if self.micros.is_empty() {
            return false;
        }
        let info = self.slot(s);
        if info.res.is_none() {
            return false;
        }
        if self.channel_is_full(s) {
            return true;
        }
        if !s.handler {
            let mut slot = slot;
            if let Some(crate::engine::ResData::Wav(w)) = self.engine.registry.get(&s.cuuid) {
                if w.flags & 0x40 != 0 {
                    slot = SLOT_MENU;
                }
            }
            // CanHearSound of a null actor returns 0.
            let hear = a.is_some_and(|a| self.can_hear_sound(a, s));
            if hear {
                let Some(hx) = self.get_hx_actor(a, b, slot) else {
                    return true;
                };
                let Some(obj) = self.engine.create_object_for(hx, s.cuuid) else {
                    return true;
                };
                self.engine.actor_attach_top(hx, obj, info.key, info.channel, false);
                if slot == SLOT_HFVOICE {
                    self.hf.active = true;
                    self.register_hf_markers(obj);
                }
                if self.check_launch_time(s) {
                    self.engine.send(obj, 0, 0);
                    self.update_channel(s);
                }
            }
            true
        } else {
            if !self.launch_handler(s, a, b, slot) {
                return true;
            }
            let name = s.handler_name();
            match self.get_hx_actor(a, b, slot) {
                Some(hx) => self.exec_handler(hx, s, &name, &[]),
                None => false,
            }
        }
    }

    /// PlaySound with integer parameters (vtable + 0x9C): only for
    /// handler sounds (otherwise like `play_sound`).
    pub fn play_sound_params(&mut self, a: Option<&ActorState>, b: Option<&ActorState>, s: &SoundRef, slot: i32, params: &[i32]) -> bool {
        // Without a micro, nothing; a sound without a handler returns without playing anything
        // (byte + 0x29 zero).
        if self.micros.is_empty() {
            return false;
        }
        let info = self.slot(s);
        // 0x26B1B..0x26B54: resource, channel, then handler kind.
        if info.res.is_none() {
            return false;
        }
        if self.channel_is_full(s) {
            return true;
        }
        if !s.handler {
            return false;
        }
        if !self.launch_handler(s, a, b, slot) {
            return true;
        }
        let name = s.handler_name();
        match self.get_hx_actor(a, b, slot) {
            Some(hx) => {
                // Co-op swaps: PlaySound(hHitSound, SoundType, Damage, bIsDead) of a dying
                // player takes the character's death shouts.
                let hit = self.engine.wave_swaps.get(&hx).filter(|w| w.hit_sound == Some(s.cuuid)).cloned();
                let death = hit.is_some() && params.get(2).is_some_and(|&d| d != 0);
                if let (Some(w), true) = (&hit, death) {
                    w.dead.set(true);
                }
                self.engine.swap_death = death;
                let r = self.exec_handler(hx, s, &name, params);
                self.engine.swap_death = false;
                r
            }
            None => false,
        }
    }

    /// ExecHandler: the actor's program for this key (created if needed), then the
    /// named handler with the parameters.
    fn exec_handler(&mut self, hx: ObjId, s: &SoundRef, name: &str, params: &[i32]) -> bool {
        let info = self.slot(s);
        exec_handler_logic(&mut ExecVm { hx: self, actor_value: hx, s, info, name, params })
    }

    /// Stop (vtable + 0xA4). `kinds`: category mask.
    pub fn stop_sound(&mut self, actor: Option<u32>, s: Option<&SoundRef>, kinds: i32, notify: bool) {
        match (s, actor) {
            (None, None) => {
                if kinds & SLOT_EFFECT != 0 {
                    for (_, hx) in std::mem::take(&mut self.effects).into_iter().chain(std::mem::take(&mut self.effects_single)) {
                        self.kill_actor(hx);
                    }
                    self.ambient_playing.clear();
                }
                if kinds & SLOT_VOICE != 0 {
                    for (_, hx) in std::mem::take(&mut self.voices) {
                        self.kill_actor(hx);
                    }
                }
                if kinds & SLOT_HFVOICE != 0 && self.hf.active {
                    self.end_hf_voice(notify);
                }
                for (bit, a) in [
                    (SLOT_STRAMB, self.stramb),
                    (SLOT_UNDERWATER, self.underwater),
                    (SLOT_MUSIC, self.music),
                    (SLOT_MENU, self.menu),
                    (SLOT_RYAN, self.ryan),
                    (SLOT_MICRO_BARREL, self.micro_barrel),
                ] {
                    if kinds & bit != 0 {
                        self.engine.actor_kill(a);
                    }
                }
            }
            (None, Some(u)) => {
                if kinds & SLOT_EFFECT != 0 {
                    let hxs: Vec<ObjId> = self.effects.iter().filter(|(x, _)| *x == u).map(|(_, h)| *h).collect();
                    self.effects.retain(|(x, _)| *x != u);
                    // The actor only leaves the ambient table (+0x1F0) if it had an effect HX actor
                    // +0x198 or a unique one +0x1AC.
                    let mut sort = !hxs.is_empty();
                    for hx in hxs {
                        self.kill_actor(hx);
                    }
                    if let Some(i) = self.effects_single.iter().position(|(x, _)| *x == u) {
                        let (_, hx) = self.effects_single.remove(i);
                        self.kill_actor(hx);
                        sort = true;
                    }
                    if sort {
                        self.ambient_playing.retain(|(x, _)| *x != u);
                    }
                }
                if kinds & SLOT_VOICE != 0 {
                    if let Some(i) = self.voices.iter().position(|(x, _)| *x == u) {
                        let (_, hx) = self.voices.remove(i);
                        self.engine.actor_update(hx);
                        self.kill_actor(hx);
                    }
                }
                if kinds & SLOT_HFVOICE != 0 && self.hf.active && self.hf.unreal == u {
                    self.end_hf_voice(notify);
                }
            }
            (Some(s), actor) => {
                let key = self.slot(s).key;
                let mut targets: Vec<ObjId> = Vec::new();
                let pick = |list: &Vec<(u32, ObjId)>| -> Vec<ObjId> { list.iter().filter(|(x, _)| actor.is_none_or(|u| *x == u)).map(|(_, h)| *h).collect() };
                if kinds & SLOT_EFFECT != 0 {
                    targets.extend(pick(&self.effects));
                    targets.extend(pick(&self.effects_single));
                }
                if kinds & SLOT_VOICE != 0 {
                    targets.extend(pick(&self.voices));
                }
                if actor.is_none() {
                    for (bit, a) in [
                        (SLOT_STRAMB, self.stramb),
                        (SLOT_UNDERWATER, self.underwater),
                        (SLOT_MUSIC, self.music),
                        (SLOT_MENU, self.menu),
                        (SLOT_RYAN, self.ryan),
                        (SLOT_MICRO_BARREL, self.micro_barrel),
                    ] {
                        if kinds & bit != 0 {
                            targets.push(a);
                        }
                    }
                }
                // The actor whose AmbientSound is this sound leaves the table of started ambients
                // if it has an effect (+0x198) or unique (+0x1AC) HX actor: the distance may restart it (Update).
                if kinds & SLOT_EFFECT != 0 {
                    if let Some(u) = actor {
                        if self.effects.iter().chain(self.effects_single.iter()).any(|(x, _)| *x == u) {
                            self.ambient_playing.retain(|(x, p)| !(*x == u && *p == s.path));
                        }
                    } else {
                        // Without an actor, each effect key leaves the ambients, even if
                        // FindKey does not find this sound.
                        let effects = &self.effects;
                        let single = &self.effects_single;
                        self.ambient_playing.retain(|(u, _)| !effects.iter().chain(single).any(|(x, _)| x == u));
                    }
                }
                for hx in targets {
                    for obj in self.engine.actor_find_key(hx, key) {
                        self.engine.send(obj, 1, 0);
                    }
                }
                if kinds & SLOT_HFVOICE != 0 && self.hf.active && actor.is_none_or(|u| u == self.hf.unreal) {
                    let objs = self.engine.actor_find_key(self.hf.actor, key);
                    if !objs.is_empty() {
                        for obj in objs {
                            self.engine.send(obj, 1, 0);
                        }
                        self.end_hf_voice(notify);
                    }
                }
            }
        }
    }

    fn end_hf_voice(&mut self, notify: bool) {
        self.hf.active = false;
        self.hf.pause = false;
        self.hf.unpause = false;
        if notify {
            self.notifications.push(Notify::VoiceEnd(self.hf.notify));
        }
        if !self.hf.shared {
            let hx = self.hf.actor;
            self.kill_actor(hx);
        } else {
            self.engine.actor_kill(self.hfvoice_actor);
            self.hf.shared = false;
        }
    }

    /// Kill then destruction of an HX actor and its resource.
    fn kill_actor(&mut self, hx: ObjId) {
        self.engine.actor_kill(hx);
        let res = self.engine.object(hx).map(|o| o.res);
        self.engine.destroy_object(hx);
        if let Some(r) = res {
            self.engine.registry.remove(&r);
        }
    }

    /// NoteDestroy: the game actor disappears; its unstoppable sounds finish
    /// on their own ("orphan" actors).
    pub fn note_destroy(&mut self, u: u32) {
        let handle = |s: &mut HxAudio, hx: ObjId| -> bool {
            s.engine.send(hx, 1, 0);
            s.engine.actor_update(hx);
            let keep = s.engine.actor_objects(hx).iter().any(|&o| match s.engine.object(o).map(|x| &x.kind) {
                Some(crate::engine::ObjKind::Wave(w)) => w.res.flags & 4 == 0,
                Some(crate::engine::ObjKind::Silent(w)) => w.flags & 4 == 0,
                _ => false,
            });
            if keep {
                s.orphans.push(hx);
            }
            keep
        };
        let hxs: Vec<ObjId> = self.effects.iter().filter(|(x, _)| *x == u).map(|(_, h)| *h).collect();
        for hx in hxs {
            if handle(self, hx) {
                self.effects.retain(|&(x, h)| !(x == u && h == hx));
            }
        }
        if let Some(&(_, hx)) = self.effects_single.iter().find(|(x, _)| *x == u) {
            if handle(self, hx) {
                self.effects_single.retain(|(x, _)| *x != u);
            }
        }
        if let Some(&(_, hx)) = self.voices.iter().find(|(x, _)| *x == u) {
            if handle(self, hx) {
                self.voices.retain(|(x, _)| *x != u);
            }
        }
        if self.hf.active && self.hf.unreal == u {
            let hx = self.hf.actor;
            self.engine.send(hx, 1, 0);
            self.engine.actor_update(hx);
            let keep = self.engine.actor_objects(hx).iter().any(|&o| match self.engine.object(o).map(|x| &x.kind) {
                Some(crate::engine::ObjKind::Wave(w)) => w.res.flags & 4 == 0,
                _ => false,
            });
            if keep {
                if !self.hf.shared {
                    self.orphans.push(hx);
                }
                self.hf.active = false;
                self.hf.pause = false;
                self.hf.unpause = false;
            }
        }
        self.ambient.retain(|&x| x != u);
        self.stop_sound(Some(u), None, 0xfff, false);
    }

    // ----- footsteps and shouts -----

    /// HXAudio's weighted draw: r = rand × sum / 0x7FFF, then accumulate while it stays < r.
    fn weighted(&mut self, list: &[ProbabilitySound], base: i32) -> Option<usize> {
        let sum = list.iter().fold(base, |acc, p| acc.wrapping_add(p.probability));
        let r = (self.engine.rand().wrapping_mul(sum)) / 0x7fff;
        let (mut i, mut acc) = (0usize, 0i32);
        if 0 < r {
            loop {
                if list.len() <= i {
                    break;
                }
                acc = acc.wrapping_add(list[i].probability);
                i += 1;
                if acc >= r {
                    break;
                }
            }
        }
        (i < list.len()).then_some(i)
    }

    /// Creates, attaches and plays a sound on HX actor `hx`; returns the object.
    fn start_on(&mut self, hx: ObjId, s: &SoundRef) -> Option<ObjId> {
        let info = self.slot(s);
        info.res?;
        let obj = self.engine.create_object_for(hx, s.cuuid)?;
        self.engine.actor_attach_top(hx, obj, info.key, info.channel, false);
        Some(obj)
    }

    /// PlaySndXIIIStep.
    pub fn play_snd_xiii_step(&mut self, world: &mut dyn World, a: &ActorState, step: &XiiiStep, speed: f32, end_step: i32, silent: bool) -> bool {
        if self.micros.is_empty() {
            return false;
        }
        let mut volume = 0.0f32;
        let sound: Option<SoundRef>;
        if speed >= 0.0 {
            if end_step != -2 {
                let left = !a.left_foot;
                world.set_left_foot(a.id, left);
                if speed < self.config.walk_speed {
                    return true;
                }
                if speed < self.config.run_speed {
                    volume = step.walk_volume;
                }
                let list = if left { &step.left_steps } else { &step.right_steps };
                let list = list.clone();
                sound = match self.weighted(&list, step.stumble.probability) {
                    Some(i) => list[i].sound.clone(),
                    None => step.stumble.sound.clone(),
                };
            } else {
                sound = step.end_step.clone();
                self.play_ono_pick(a, &step.onos_run, step.onos_run_probability_sum);
            }
        } else {
            sound = step.land_sound.clone();
            self.play_ono_pick(a, &step.onos_land, step.onos_land_probability_sum);
        }
        let Some(sound) = sound else { return false };
        let Some(hx) = self.get_hx_actor(Some(a), Some(a), SLOT_EFFECT) else {
            return false;
        };
        let Some(obj) = self.start_on(hx, &sound) else {
            return false;
        };
        self.log_step(hx, &sound, false);
        if silent {
            volume += step.silent_volume;
        }
        self.engine.send_with(obj, 6, &[volume.to_bits()]);
        let r = self.engine.rand();
        // ((max − min) × rand) × (1/32767) + min, each operation rounded to 24 bits (the game's FPU).
        let pitch = ((step.pitch_max - step.pitch_min) * r as f32) * f32::from_bits(0x38000100) + step.pitch_min;
        self.engine.send_with(obj, 0xc, &[pitch.to_bits()]);
        self.engine.send(obj, 0, 0);
        true
    }

    /// +184 diagnostic of footsteps: USound names and resource of the HX actor (20525/20B04/20CB4/20F00).
    fn log_step(&mut self, hx: ObjId, sound: &SoundRef, ono: bool) {
        if self.log_ono {
            let actor_name = self.engine.actor(hx).map_or("", |x| x.res.name.as_str());
            let kind = if ono { "ono" } else { "footstep" };
            self.engine.logs.push(format!("SOUND : Play {kind} {} on actor {actor_name}", sound.name));
        }
    }

    /// Running or landing onomatopoeia, on the voice actor.
    fn play_ono_pick(&mut self, a: &ActorState, list: &[ProbabilitySound], sum: i32) {
        let r = (self.engine.rand().wrapping_mul(sum)) / 0x7fff;
        let (mut i, mut acc) = (0usize, 0i32);
        if 0 < r {
            loop {
                if list.len() <= i {
                    break;
                }
                acc = acc.wrapping_add(list[i].probability);
                i += 1;
                if acc >= r {
                    break;
                }
            }
        }
        let Some(Some(s)) = list.get(i).map(|p| p.sound.clone()) else {
            return;
        };
        let Some(hx) = self.get_hx_actor(Some(a), Some(a), SLOT_VOICE) else {
            return;
        };
        if let Some(obj) = self.start_on(hx, &s) {
            self.log_step(hx, &s, true);
            self.engine.send(obj, 0, 0);
        }
    }

    /// PlaySndPNJStep.
    pub fn play_snd_npc_step(&mut self, world: &mut dyn World, a: &ActorState, step: &NPCStep, speed: f32, category: i32, _silent: bool) -> bool {
        if self.micros.is_empty() || !(0..5).contains(&category) {
            return false;
        }
        let sound = if speed >= 0.0 {
            let left = !a.left_foot;
            world.set_left_foot(a.id, left);
            let list = if left { &step.left_steps } else { &step.right_steps };
            list.get(category as usize).cloned().flatten()
        } else {
            step.land_sound.clone()
        };
        let Some(sound) = sound else { return false };
        if !self.can_hear_sound(a, &sound) {
            return true;
        }
        let Some(hx) = self.get_hx_actor(Some(a), Some(a), SLOT_EFFECT) else {
            return false;
        };
        let Some(obj) = self.start_on(hx, &sound) else {
            return false;
        };
        self.log_step(hx, &sound, false);
        let r = self.engine.rand();
        // ((max − min) × rand) × (1/32767) + min, each operation rounded to 24 bits (the game's FPU).
        let pitch = ((step.pitch_max - step.pitch_min) * r as f32) * f32::from_bits(0x38000100) + step.pitch_min;
        self.engine.send_with(obj, 0xc, &[pitch.to_bits()]);
        self.engine.send(obj, 0, 0);
        true
    }

    /// PlaySndPNJOno and PlaySndDeathOno.
    pub fn play_snd_ono(&mut self, a: &ActorState, ono: &OnoSet, code_mesh: i32, timbre: i32, death: bool) -> bool {
        if self.micros.is_empty() || timbre > 2 || code_mesh > 0x25 {
            return false;
        }
        if code_mesh < 0 || ono.timbres.len() <= code_mesh as usize {
            return true;
        }
        if !death && ono.probability < (self.engine.rand() * 100) / 0x7fff {
            return true;
        }
        let t = &ono.timbres[code_mesh as usize];
        // Timbre 0, 1, otherwise the third, negatives included.
        let group = match timbre {
            0 => 0,
            1 => 4,
            _ => 8,
        };
        let pick = (self.engine.rand() * 4) / 0x7fff;
        let mut chosen = None;
        for k in pick.clamp(0, 4) as usize..5 {
            if k == 4 {
                chosen = t[group].clone();
                break;
            }
            if let Some(s) = &t[group + k] {
                chosen = Some(s.clone());
                break;
            }
        }
        let Some(s) = chosen else { return false };
        if self.slot(&s).res.is_none() {
            return false;
        }
        if death {
            // PlaySndDeathOno: neither ChannelIsFull nor voice playing. The actor's HF voice is
            // stopped (vtable+0xA4, with the end notification), then each object of its voice
            // actor (message 1); GetHXActor then reuses this actor
            // if it exists.
            if self.hf.active && self.hf.unreal == a.id {
                self.stop_sound(Some(a.id), None, SLOT_HFVOICE, true);   // end notification requested (push 1)
            }
            if let Some(&(_, hx)) = self.voices.iter().find(|(x, _)| *x == a.id) {
                for o in self.engine.actor_objects(hx) {
                    self.engine.send(o, 1, 0);
                }
            }
        } else {
            if self.channel_is_full(&s) || (self.hf.active && self.hf.unreal == a.id) {
                return true;
            }
            if let Some(&(_, hx)) = self.voices.iter().find(|(x, _)| *x == a.id) {
                if self.engine.actor_count(hx) != 0 {
                    return true;
                }
            }
        }
        let Some(hx) = self.get_hx_actor(Some(a), Some(a), SLOT_VOICE) else {
            return false;
        };
        let Some(obj) = self.start_on(hx, &s) else {
            return false;
        };
        if self.log_ono {
            let actor_name = self.engine.actor(hx).map_or("", |x| x.res.name.as_str());
            let frame = 0;
            // Mirrors the log text of the original HXAudio.dll (French in the original).
            let message = format!("PNJ ono lancé SOUND : Play onomatopee {} on actor {} à la trame {}", s.name, actor_name, frame);
            self.engine.logs.push(message);
        }
        if self.check_launch_time(&s) {
            self.engine.send(obj, 0, 0);
            self.update_channel(&s);
            self.notifications.push(Notify::OnoStart(a.id));
        }
        true
    }

    // ----- music -----

    /// LoadSounds: reset for a new map, the map's music variables,
    /// starting music.
    pub fn load_sounds(
        &mut self,
        title: &str,
        music_vars: &[(String, i32, u8)],
        init_music: Option<SoundRef>,
        music_init: Option<SoundRef>,
        flash_manager: Option<u32>,
    ) {
        // ForceSystemResume then two engine updates, orphans stopped and destroyed,
        // two more updates.
        self.force_system_resume();
        self.engine_update(0.0);
        self.engine_update(0.0);
        for hx in std::mem::take(&mut self.orphans) {
            self.engine.send(hx, 1, 0);
            self.engine.actor_update(hx);
            self.kill_actor(hx);
        }
        self.engine_update(0.0);
        self.engine_update(0.0);
        for a in [self.stramb, self.underwater, self.music, self.menu, self.ryan, self.micro_barrel] {
            self.engine.send_with(a, 6, &[0f32.to_bits()]);
        }
        // FillSoundsArray (USound data rebuilt), DeleteStrUserData,
        // LoadMapSounds(title),
        // deferred stop and HF voice reset.
        self.slots.clear();
        self.load_map_sounds(title);
        self.pending_stop = None;
        self.hf = HfVoice::default();
        let sd = self.config.sd_volume;
        self.engine.set_var_f32(self.var_sound_design, sd);
        self.engine.var_changed(self.var_sound_design);
        self.engine.set_var_u32(self.var_global_sd, 0);
        self.engine.var_changed(self.var_global_sd);
        self.post_edit_change();
        self.flash_manager = flash_manager;
        self.flash_value = 0;
        // The previous level's variables leave the global array, then the list is emptied.
        for name in std::mem::take(&mut self.music_var_names) {
            self.engine.delete_global(&name);
        }
        self.music_debug_vars = music_vars.iter().take(3).map(|(name, value, ty)| (name.clone(), *value, *ty)).collect();
        for (name, value, ty) in music_vars {
            self.music_var_names.push(name.clone());
            // Types 0 (integer) and 1 (boolean) only: another type creates nothing.
            let v = match ty {
                0 => self.engine.create_global(name, K_SIMPLE, T_INT),
                1 => self.engine.create_global(name, K_SIMPLE, T_BOOL),
                _ => continue,
            };
            // LoadSounds resets the value of each MusicVars entry to 0 before copying them.
            let _ = value;
            let x = 0u32;
            self.engine.set_var_u32(v, x);
            self.engine.var_changed(v);
        }
        if let Some(&w) = self.engine.actor(self.music).and_then(|a| a.vars.get(b"WaitingTime".as_slice())) {
            self.engine.set_var_f32(w, 4.0);
            self.engine.var_changed(w);
        }
        self.attack_wait = false;
        // InitMusic is registered (RegisterSound) but PlayMusicInit reads it again from the ALevelInfo; only the starting music found by name is kept.
        let _ = init_music;
        self.music_init = music_init;
    }

    /// PlayMusicInit, called by UGameEngine::Browse after loading the
    /// map: outside the menu ("MapMenu"), LevelInfo.InitMusic (read at call time) or, failing that, the
    /// handler "Music__<map>.<map>__hMusicInit" found by LoadSounds, on the Music
    /// actor; its key designates the music.
    /// Starting handler found by name by LoadSounds ("…__hMusicInit"), played by PlayMusicInit without InitMusic.
    pub fn music_start(&self) -> Option<&SoundRef> {
        self.music_init.as_ref()
    }

    pub fn play_music_init(&mut self, title: &str, init_music: Option<SoundRef>) {
        if title.eq_ignore_ascii_case("MapMenu") {
            return;
        }
        if let Some(s) = init_music.or_else(|| self.music_init.clone()) {
            self.play_sound(None, None, &s, SLOT_MUSIC);
            self.music_key = self.slot(&s).key;
        }
    }

    /// CallMusicHandler(int) (vtable + 0xCC): on the music program, 0 starts
    /// "hInfiltration", 1 starts "hAttaque" with the HX value of MusicVars[2] (NbAttaque).
    pub fn call_music_state(&mut self, which: i32, attack: Option<(String, u8)>) {
        let objs = self.engine.actor_find_key(self.music, self.music_key);
        let Some(prog) = objs.into_iter().find(|&o| self.engine.program_obj(o).is_some()) else {
            return;
        };
        let Some((name, with_attack)) = state_music(which) else { return };
        let params: Vec<i32> = if with_attack {
            vec![attack.map_or(-1, |(name, ty)| self.get_music_var(&name, ty))]
        } else {
            Vec::new()
        };
        let mut bytes = Vec::new();
        for p in &params {
            bytes.extend_from_slice(&p.to_le_bytes());
        }
        let block = if params.is_empty() { 0 } else { self.engine.mem.alloc_with(&bytes) };
        self.engine.send_named(prog, 5, name, block);
        if block != 0 {
            self.engine.mem.release(block);
        }
    }

    /// GetMusicVarValue: value of the HX global variable, −1 if it does not exist.
    pub fn get_music_var(&self, name: &str, ty: u8) -> i32 {
        let Some(&v) = self.engine.globals.get(name.as_bytes()) else {
            return -1;
        };
        match ty {
            0 => self.engine.var_u32(v) as i32,
            // Type 1: low byte of the variable (movzx eax, al).
            1 => (self.engine.var_u32(v) & 0xFF) as i32,
            _ => -1,
        }
    }

    /// PauseSound (vtable + 0xA8): 0xFFF = global pause (counter), otherwise the
    /// requested categories (PrivatePauseSound).
    pub fn pause_sound(&mut self, kinds: i32, system: bool) {
        if kinds == 0xfff {
            if self.system_pause == 0 {
                self.private_pause(0xfff, true);
            }
            self.system_pause += 1;
            self.system_resume_delay = 0;
        } else {
            self.private_pause(kinds, system);
        }
    }

    /// ResumeSound (vtable + 0xAC): the global resume waits for two updates.
    /// SystemPause: engine update, global pause of every category (PrivatePauseSound), update.
    pub fn system_pause(&mut self) {
        self.engine_update(0.0);
        self.private_pause(0xfff, true);
        self.engine_update(0.0);
    }

    /// SystemResume: like SystemPause with PrivateResumeSound.
    pub fn system_resume(&mut self) {
        self.engine_update(0.0);
        self.private_resume(0xfff, true);
        self.engine_update(0.0);
    }

    pub fn resume_sound(&mut self, kinds: i32, system: bool) {
        if kinds == 0xfff {
            if self.system_pause > 0 {
                self.system_pause -= 1;
                if self.system_pause == 0 {
                    self.system_resume_delay = 2;
                }
            }
        } else {
            self.private_resume(kinds, system);
        }
    }

    /// HX actors targeted by a category mask (PrivatePauseSound / PrivateResumeSound);
    /// `orphans`: with the orphan sounds (the global resume forgets them).
    fn kind_actors(&self, kinds: i32, orphans: bool) -> Vec<ObjId> {
        let mut out = Vec::new();
        for (bit, a) in [(8, self.stramb), (0x100, self.underwater), (0x10, self.music), (0x20, self.menu), (0x40, self.ryan), (0x80, self.micro_barrel)] {
            if kinds & bit != 0 {
                out.push(a);
            }
        }
        // HF voice: the actor is only targeted if the voice is active (test of the first word).
        if kinds & 4 != 0 && self.hf.active && self.hf.actor != 0 {
            out.push(self.hf.actor);
        }
        if kinds & 2 != 0 {
            out.extend(self.voices.iter().map(|&(_, hx)| hx));
        }
        if kinds & 1 != 0 {
            out.extend(self.effects.iter().map(|&(_, hx)| hx));
            if orphans {
                out.extend(self.orphans.iter().copied());
            }
            out.extend(self.effects_single.iter().map(|&(_, hx)| hx));
        }
        out
    }

    /// PrivatePauseSound: Pause message (2) to the actors of the categories.
    fn private_pause(&mut self, kinds: i32, system: bool) {
        let kinds = if system {
            0xfff
        } else {
            self.paused_kinds |= kinds;
            kinds
        };
        for a in self.kind_actors(kinds, true) {
            self.engine.send(a, 2, 0);
        }
    }

    /// UHXAUDIOSubsystem::Exec, console commands that affect the sound state: ASTAT AUDIO toggles + 0x130;
    /// PAUSESOUNDS / UNPAUSESOUNDS send Pause (2) / Resume (3) to each HX actor of the effect sounds (+ 0x198) then of the unique sounds
    /// (+ 0x1AC), without changing the paused categories or the global pause. True if the command is handled. The other commands
    /// (DS8_INIT, IMPORT_WAVE, CHANGE_PROPERTIES, GET_PROPERTIES, DELETE_BANK, PLAY_STOP, LOAD_*, ADD_BNH, GET_*) are editor and
    /// profiling tools: not reimplemented, they return false like an unknown command.
    pub fn exec(&mut self, command: &str) -> bool {
        if let Some(rest) = parse_command(command, "ASTAT") {
            if parse_command(rest, "AUDIO").is_none() {
                return false;
            }
            self.astat_audio = !self.astat_audio;
            return true;
        }
        let msg = if parse_command(command, "PAUSESOUNDS").is_some() {
            2
        } else if parse_command(command, "UNPAUSESOUNDS").is_some() {
            3
        } else {
            return false;
        };
        let actors_list: Vec<ObjId> = self.effects.iter().chain(&self.effects_single).map(|&(_, hx)| hx).collect();
        for a in actors_list {
            self.engine.send(a, msg, 0);
        }
        true
    }

    /// PrivateResumeSound: Resume message (3); the global resume spares the
    /// categories paused one by one.
    fn private_resume(&mut self, kinds: i32, system: bool) {
        if !system {
            for a in self.kind_actors(kinds, true) {
                self.engine.send(a, 3, 0);
            }
            self.paused_kinds &= kinds ^ 0xfff;
        } else {
            for a in self.kind_actors(self.paused_kinds ^ 0xfff, false) {
                self.engine.send(a, 3, 0);
            }
        }
    }

    /// CallMusicHandler(text): "hName ( n, a, b )" (read_call_music) on the music
    /// program.
    pub fn call_music_handler(&mut self, text: &str) {
        let objs = self.engine.actor_find_key(self.music, self.music_key);
        let Some(prog) = objs.into_iter().find(|&o| self.engine.program_obj(o).is_some()) else {
            return;
        };
        let Some((name, _, vals)) = read_call_music(text.as_bytes()) else { return };
        let params: Vec<i32> = vals.iter().map(|v| v.unwrap_or(0)).collect();
        let mut bytes = Vec::new();
        for p in &params {
            bytes.extend_from_slice(&p.to_le_bytes());
        }
        let block = if params.is_empty() { 0 } else { self.engine.mem.alloc_with(&bytes) };
        self.engine.send_named(prog, 5, &name, block);
        if block != 0 {
            self.engine.mem.release(block);
        }
    }

    /// SetMusicVarValue.
    pub fn set_music_var(&mut self, name: &str, value: i32, ty: u8) {
        if let Some(&v) = self.engine.globals.get(name.as_bytes()) {
            let changed = match ty {
                0 => { self.engine.set_var_u32(v, value as u32); true }
                1 => {
                    let prior = self.engine.var_u32(v);
                    self.engine.set_var_u32(v, (prior & !0xFF) | (value != 0) as u32);
                    true
                }
                _ => false,
            };
            if changed { self.engine.var_changed(v); }
        }
        // +184: the log is also produced for a missing variable or an unhandled type.
        if self.log_ono {
            let mut read = |i: usize| {
                let Some((name, value, ty)) = self.music_debug_vars.get(i).cloned() else { return -1; };
                // GetMusicVarValue only handles 0/1; otherwise it keeps the description's value.
                let result = if ty <= 1 { self.get_music_var(&name, ty) } else { value };
                self.music_debug_vars[i].1 = result;
                result
            };
            // Native order of the three reads: indices 2, 0, 1.
            let (attack, wait_value, alert) = (read(2), read(0), read(1));
            self.engine.logs.push(format!("NbAttente = {wait_value}   NbAlerte = {alert}   NbAttaque = {attack}"));
        }
    }

    /// UHXAUDIOSubsystem::SetVolumeLine (vtable + 0xB8): only line 0xFFF has an effect,
    /// on gglobalvolume and the "Master" global variable.
    pub fn set_volume_line_kinds(&mut self, kinds: i32, value: f32) {
        if kinds == 0xfff {
            self.gglobalvolume = value;
            if self.var_master != 0 {
                self.engine.set_var_f32(self.var_master, value);
                self.engine.var_changed(self.var_master);
            }
        }
    }

    /// PostEditChange (vtable + 0x4C, called by LoadSounds after SoundDesign and GlobalSDFlag): the .ini settings are clamped to
    /// their bounds (mode 0 to 1, music position 0 to 2, volumes from -100 to 0 dB; zero, positive or negative, and a NaN become +0) and applied: SetSoundMode (no effect),
    /// SetMusicPosition, then SetVolumeLine(0xFFF, MasterVolume) which resets the "Master" variable and gglobalvolume to the .ini setting.
    pub fn post_edit_change(&mut self) {
        fn bound(v: f32) -> f32 {
            if v < -100.0 {
                -100.0
            } else if v >= 0.0 || v.is_nan() {
                0.0
            } else {
                v
            }
        }
        self.config.sound_mode = self.config.sound_mode.clamp(0, 1);
        self.config.master_volume = bound(self.config.master_volume);
        let master = self.config.master_volume;
        self.config.music_slider_pos = self.config.music_slider_pos.clamp(0, 2);
        self.config.music_light_volume = bound(self.config.music_light_volume);
        self.config.music_normal_volume = bound(self.config.music_normal_volume);
        self.set_music_position(self.config.music_slider_pos);
        self.config.sd_volume = bound(self.config.sd_volume);
        self.set_volume_line_kinds(0xfff, master);
    }

    // ----- update -----

    /// SetViewport (vtable + 0x74) for viewport `view`: fresh micro (
    /// HX listener created, current time, zero previous position) stored under this viewport
    /// (TMap::Set, the old one replaced). bHasRollOff / bHasPosition of the controller and of
    /// its pawn are cleared by the caller.
    pub fn set_viewport(&mut self, view: usize) {
        let micro = Micro { listener: Listener::default(), last_time: self.time };
        let rank = self.next_micro;
        self.next_micro += 1;
        self.rank_micros.resize(self.micros.len(), 0);
        if view < self.micros.len() {
            self.lost_listeners.push((self.rank_micros[view], self.micros[view].listener));
            self.micros[view] = micro;
            self.rank_micros[view] = rank;
        } else {
            self.micros.push(micro);
            self.rank_micros.push(rank);
        }
        self.publish_listeners();
    }

    /// The engine's listener pool: those of the table's micros and those of the lost micros, in creation order.
    fn publish_listeners(&mut self) {
        let mut all_list: Vec<(u64, Listener)> = self.lost_listeners.clone();
        all_list.extend(self.micros.iter().enumerate().map(|(i, m)| (self.rank_micros.get(i).copied().unwrap_or(0), m.listener)));
        all_list.sort_by_key(|x| x.0);
        self.engine.listeners = all_list.into_iter().map(|x| x.1).collect();
    }

    /// Listener of micro `view` in the engine's pool (its rank among the lost listeners and the table's micros).
    pub(crate) fn listener_micro(&self, view: usize) -> Option<&Listener> {
        let rank = *self.rank_micros.get(view)?;
        let before_value = self.lost_listeners.iter().filter(|p| p.0 < rank).count() + self.rank_micros.iter().filter(|&&r| r < rank).count();
        self.engine.listeners.get(before_value)
    }

    /// The table's micros (one per viewport set by SetViewport).
    pub fn count_micros(&self) -> usize {
        self.micros.len()
    }

    /// ResetMicros: StopSound(0, 0, 0xFFF, 0), the table's micros destroyed (lost listeners remain).
    pub fn reset_micros(&mut self) {
        self.stop_sound(None, None, 0xfff, false);
        self.micros.clear();
        self.rank_micros.clear();
        self.publish_listeners();
    }

    /// UpdateMicro: listener of the view (position, look, up); the velocity comes
    /// from the movement since the previous update.
    pub fn update_micro(&mut self, view: usize, pos: [f32; 3], front: [f32; 3], up: [f32; 3]) {
        // Micro looked up by viewport: none, nothing to do.
        if view >= self.micros.len() {
            return;
        }
        // The original timestamps the micro with RDTSC × GSecondsPerCycle (micro_time); we use the HX
        // clock. 24-bit computation as in the machine code (micro_speed). The velocity is
        // only used by Doppler, which no sound of the banks enables (flag 0x20).
        let now = self.time;
        let m = &mut self.micros[view];
        let old = m.listener.pos;
        m.listener.vel = crate::listener::micro_speed(now, &mut m.last_time, pos, old);
        m.listener.pos = pos;
        m.listener.front = front;
        m.listener.up = up;
        self.publish_listeners();
    }

    /// Update, called every frame with the elapsed time; notifications
    /// accumulate in `notifications` (see `update_stage` to deliver them in place).
    pub fn update(&mut self, world: &mut dyn World, dt: f64) {
        let mut stage = 0;
        while self.update_stage(world, dt, &mut stage) {}
    }

    /// Staged Update: HXAudio calls AActor::VoicePauseNotif, VoiceUnPauseNotif,
    /// VoiceEndNotif and the *FlashNotif in the middle of Update, before
    /// the end of attack, the orphans and the engine update. Each call stops just
    /// after a notification (true: the caller delivers it then calls again with the same `stage`);
    /// false when Update is finished.
    pub fn update_stage(&mut self, world: &mut dyn World, dt: f64, stage: &mut u8) -> bool {
        // Real frame time (see `clock_value`); a simulation can impose timeGetTime.
        if *stage == 0 {
            {
                self.free = self.time_impose.is_none() && crate::diagnostics::use_audio_thread();
            }
            self.dt_current = match self.time_impose.take() {
                Some(t) => (t - self.time).max(0.0),
                None => dt * crate::stream::clock_value(),
            };
        }
        let dt = self.dt_current;
        loop {
            match *stage {
                0 => {
                    self.time += dt;
                    if self.wait_to_place {
                        self.attack_wait_start = self.time;
                        self.wait_to_place = false;
                    }
                    // timeGetTime is a whole number of milliseconds; an imposed time
                    // (difference of two timeGetTime values, in seconds) can fall just below the
                    // millisecond (7.567999… for 7568): truncation would put it 1 ms early.
                    self.engine.time_ms = (self.time * 1000.0 + 1e-6) as u32;
                    self.engine.update_real = self.real_time.then(std::time::Instant::now);
                    // Decoder and DirectSound threads during the elapsed frame.
                    self.engine.stream_threads(dt);
                    self.engine.advance_static_voices(dt);
                    if self.free {
                        self.passes_thread();
                    }
                    // Without a micro (table +0xFC empty), Update does nothing; the engine
                    // then advances through ParallelUpdateThread (HXUpdate),
                    // on its own thread and hence its own rand() sequence.
                    if self.micros.is_empty() {
                        std::mem::swap(&mut self.engine.rand_main, &mut self.rand_thread);
                        if !self.free {
                            self.engine_update(dt);
                        }
                        std::mem::swap(&mut self.engine.rand_main, &mut self.rand_thread);
                        *stage = 6;
                        return false;
                    }
                    // Deferred stop "All" (+0xA4 actor, +0xA8 categories) set by the
                    // watched variables: StopSound(actor, 0, categories, 1).
                    if let Some((actor, kinds)) = self.pending_stop.take() {
                        self.stop_sound(actor, None, kinds, true);
                    }
                    self.update_sources(world, dt);
                    // HF voice block, decided once on +0xAC->active.
                    self.hf_block = self.hf.active;
                    *stage = 1;
                }
                1 => {
                    *stage = 2;
                    if self.hf_block && self.hf.pause {
                        self.hf.pause = false;
                        self.notifications.push(Notify::VoicePause(self.hf.notify));
                        return true;
                    }
                }
                2 => {
                    *stage = 3;
                    if self.hf_block && self.hf.unpause {
                        self.hf.unpause = false;
                        self.notifications.push(Notify::VoiceUnPause(self.hf.notify));
                        return true;
                    }
                }
                3 => {
                    *stage = 4;
                    if self.hf_block && self.update_hf(world) {
                        return true;
                    }
                }
                4 => {
                    *stage = 5;
                    if self.update_flash() {
                        return true;
                    }
                }
                5 => {
                    self.update_end(world, dt);
                    *stage = 6;
                    return false;
                }
                _ => return false,
            }
        }
    }

    /// Start of Update: ambient sounds, special actors, effect and voice lists.
    fn update_sources(&mut self, world: &mut dyn World, _dt: f64) {
        // Ambient sounds of positioned actors: triggered according to the distance to the micro.
        let ambient = self.ambient.clone();
        for u in ambient {
            let Some(a) = world.actor(u) else { continue };
            // An actor without AmbientSound is not skipped: PlaySound(actor, actor, null) plays
            // nothing but the actor enters the table, and when far away, StopSound(actor, null)
            // stops all its effect sounds.
            let sound = a.ambient_sound.clone();
            let d = self.nearest_micro_offset(a.location, a.mover_offset, true).unwrap_or(0.0);
            let playing = self.ambient_playing.iter().any(|(x, _)| *x == u);
            if d < a.stabilisation_distance || self.min_volume < a.stabilisation_volume {
                if !playing {
                    // Channel 0x200 if bUnderwater (bit 8 of 0x34), otherwise 1.
                    let slot = if a.underwater { SLOT_EFFECT_SINGLE } else { SLOT_EFFECT };
                    if let Some(sound) = &sound {
                        self.play_sound(Some(&a), Some(&a), sound, slot);
                    }
                    self.ambient_playing.push((u, sound.as_ref().map_or(String::new(), |s| s.path.clone())));
                }
            } else if playing && !(a.stabilisation_volume < self.min_volume) == (a.stabilisation_volume == self.min_volume) {
                self.stop_sound(Some(u), sound.as_ref(), SLOT_EFFECT, true);
            }
        }
        for a in [self.music, self.stramb, self.underwater, self.menu, self.ryan, self.micro_barrel] {
            self.engine.actor_update(a);
        }
        for list in 0..3 {
            let entries: Vec<(u32, ObjId)> = match list {
                0 => self.effects.clone(),
                1 => self.effects_single.clone(),
                _ => self.voices.clone(),
            };
            for (u, hx) in entries {
                if let Some(a) = world.actor(u) {
                    if a.has_position {
                        if self.micros.len() <= 1 {
                            if let Some(x) = self.engine.actor_mut(hx) {
                                x.pan_coeff = a.pan_coeff;
                            }
                        }
                        self.engine.actor_set_position(hx, a.sound_position());
                        self.engine.send_with(hx, 0x1a, &a.velocity.map(f32::to_bits));
                    }
                }
                self.engine.actor_update(hx);
                if self.engine.actor_count(hx) == 0 {
                    match list {
                        0 => self.effects.retain(|&(x, h)| !(x == u && h == hx)),
                        1 => self.effects_single.retain(|&(x, h)| !(x == u && h == hx)),
                        _ => self.voices.retain(|&(x, h)| !(x == u && h == hx)),
                    }
                    self.kill_actor(hx);
                }
            }
        }
    }

    /// HF voice: the actor's position if it is not shared, update;
    /// when finished, VoiceEndNotif, actor destroyed (or released if shared), pause
    /// and resume flags reset. True if VoiceEnd must be delivered.
    fn update_hf(&mut self, world: &mut dyn World) -> bool {
        if self.hf.actor == 0 {
            return false;
        }
        let hx = self.hf.actor;
        if !self.hf.shared {
            if let Some(a) = world.actor(self.hf.unreal) {
                if a.has_position {
                    if self.micros.len() <= 1 {
                        if let Some(x) = self.engine.actor_mut(hx) {
                            x.pan_coeff = a.pan_coeff;
                        }
                    }
                    self.engine.actor_set_position(hx, a.sound_position());
                }
            }
        }
        self.engine.actor_update(hx);
        if self.engine.actor_count(hx) != 0 {
            return false;
        }
        self.hf.active = false;
        self.notifications.push(Notify::VoiceEnd(self.hf.notify));
        if !self.hf.shared {
            self.kill_actor(hx);
            self.hf.actor = 0;
        } else {
            self.hf.shared = false;
        }
        self.hf.pause = false;
        self.hf.unpause = false;
        true
    }

    /// FlashManager notification (+0x9C) according to the Flash variable (+0xA0), reset
    fn update_flash(&mut self) -> bool {
        let Some(m) = self.flash_manager else { return false };
        let n = match self.flash_value {
            1 => Notify::BeginBeginFlash(m),
            2 => Notify::EndBeginFlash(m),
            3 => Notify::BeginEndFlash(m),
            4 => Notify::EndEndFlash(m),
            0 => return false,
            _ => {
                self.flash_value = 0;
                return false;
            }
        };
        self.notifications.push(n);
        self.flash_value = 0;
        true
    }

    /// End of Update: end of attack, orphans, engine,
    /// watched variables (after the engine), deferred resume.
    fn update_end(&mut self, world: &mut dyn World, dt: f64) {
        // End of attack: the infiltration music waits WaitingTime seconds while
        // enemies remain on alert (MusicVars[1] = NbAlerte, MusicVars[2] = NbAttaque).
        if self.attack_wait {
            let elapsed = (self.time - self.attack_wait_start) as f32;
            let wait = self.engine.actor(self.music).and_then(|a| a.vars.get(b"WaitingTime".as_slice()).copied()).map_or(0.0, |v| self.engine.var_f32(v));
            let attack = world.music_var(2).map_or(0, |v| v.1);
            let alert = world.music_var(1).map_or(0, |v| v.1);
            let mut waiting = false;
            if attack == 0 {
                if alert != 0 && elapsed < wait {
                    waiting = true;
                } else {
                    let var = world.music_var(2).map(|v| (v.0, v.2));
                    self.call_music_state(0, var);
                    self.attack_wait = false;
                }
            }
            if !waiting && wait <= elapsed {
                self.attack_wait = false;
            }
        }
        let orphans = std::mem::take(&mut self.orphans);
        for hx in orphans {
            self.engine.actor_update(hx);
            if self.engine.actor_count(hx) == 0 {
                self.engine.actor_update(hx);
                self.kill_actor(hx);
            } else {
                self.orphans.push(hx);
            }
        }
        if self.free {
            // End of Update: the engine advances by the time elapsed since the last update (the thread's included).
            let elapsed_value = (self.time - self.engine_t).max(0.0);
            self.engine_t = self.time;
            self.engine_update(elapsed_value);
        } else {
            self.engine_update(dt);
        }
        self.process_hooks(world);
        if self.system_resume_delay > 0 {
            self.system_resume_delay -= 1;
            if self.system_resume_delay == 0 {
                self.private_resume(0xfff, true);
            }
        }
    }

    /// Passes of the update thread since the last frame, on its rand() sequence: on each pass (every 31.25 ms), HXUpdate updates the
    /// engine if more than 30 ms have elapsed since its last update (`update_thread`), at the time of its pass.
    fn passes_thread(&mut self) {
        const PERIOD: f64 = 0.03125;
        if self.thread_t == 0.0 {
            self.thread_t = self.time - self.dt_current + PERIOD;
            self.engine_t = self.time - self.dt_current;
        }
        while self.thread_t <= self.time {
            let t_ms = (self.thread_t * 1000.0 + 1e-6) as u32;
            if update_thread(t_ms, &mut self.thread_last) {
                let elapsed_value = (self.thread_t - self.engine_t).max(0.0);
                self.engine_t = self.thread_t;
                let saved = self.engine.time_ms;
                self.engine.time_ms = t_ms;
                std::mem::swap(&mut self.engine.rand_main, &mut self.rand_thread);
                self.engine_update(elapsed_value);
                std::mem::swap(&mut self.engine.rand_main, &mut self.rand_thread);
                self.engine.time_ms = saved;
            }
            self.thread_t += PERIOD;
        }
    }

    /// HX engine update: voices, timers, and native callbacks of the
    /// Pause / Unpause markers of voices, seen on the next update.
    fn engine_update(&mut self, dt: f64) {
        self.engine.advance_voices(dt);
        self.engine.tick_timers();
        for code in std::mem::take(&mut self.engine.native_calls) {
            match code {
                NATIVE_HF_PAUSE => self.hf.pause = true,
                NATIVE_HF_UNPAUSE => self.hf.unpause = true,
                _ => {}
            }
        }
    }

    /// Watched variables written by programs. Each listener copies on write the
    /// actor name (ActorName), its type (ActorType) and the value into an 8-slot table
    /// (handlers, sounds, stops, volumes; table
    /// full: the write is lost); CallMusicHandler and the flash
    /// act immediately., at the end of Update, then empties the four tables in
    /// this order: handlers, sounds, stops, volumes.
    pub(crate) fn process_hooks(&mut self, world: &mut dyn World) {
        let events = std::mem::take(&mut self.engine.hook_events);
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let mut handlers: Vec<(String, i32, String)> = Vec::new();
        let mut plays: Vec<(String, i32, Option<Cuuid>)> = Vec::new();
        let mut stops: Vec<(String, i32, String)> = Vec::new();
        let mut volumes: Vec<(String, i32, f32)> = Vec::new();
        for e in events {
            let actor = text(&e.actor);
            match e.code {
                HOOK_CALL_HANDLER => store(&mut handlers, |h| h.0.is_empty(), (actor, e.kind, text(&e.text))),
                HOOK_PLAY_SOUND => store(&mut plays, |p| p.2.is_none_or(|c| c.1 == 0), (actor, e.kind, parse_cuuid(&text(&e.text)))),
                HOOK_STOP_SOUND => store(&mut stops, |s| s.0.is_empty() && s.2.is_empty(), (actor, e.kind, text(&e.text))),
                HOOK_CHANGE_VOLUME => store(&mut volumes, |v| v.0.is_empty() && v.1 == -1, (actor, e.kind, e.float)),
                HOOK_CALL_MUSIC_HANDLER => self.call_music_handler(&text(&e.text)),
                HOOK_FLASH => flash_write(self.flash_manager, &mut self.flash_value, e.int as i32),
                _ => {}
            }
        }
        // Handlers: TestHandler (vtable + 0xB0).
        for (actor, kind, call) in handlers {
            if actor.is_empty() {
                break;
            }
            let a = world.find_actor(&actor).and_then(|id| world.actor(id));
            self.test_handler(world, &call, kind, a.as_ref());
        }
        for (actor, kind, sound) in plays {
            let Some(sound) = sound.filter(|c| c.1 != 0) else { break };
            // GetSound (vtable + 0x88, which skips handler sounds: byte + 0x29): a game Sound object has this CUUID → PlaySound; otherwise the
            // resource is created directly with a key specific to the CUUID.
            let a = world.find_actor(&actor).and_then(|id| world.actor(id));
            match world.sound_by_cuuid(sound).filter(|s| !s.handler) {
                Some(s) => {
                    self.play_sound(a.as_ref(), a.as_ref(), &s, kind);
                }
                None => {
                    let Some(hx) = self.get_hx_actor(a.as_ref(), a.as_ref(), kind) else {
                        continue;
                    };
                    let channel = match self.engine.registry.get(&sound) {
                        Some(crate::engine::ResData::Wav(w)) => w.x18,
                        Some(crate::engine::ResData::Program(p)) => p.x18,
                        _ => continue,
                    };
                    let key = {
                        let next = &mut self.next_key;
                        *self.cuuid_keys.entry(sound).or_insert_with(|| {
                            let k = *next;
                            *next += 1;
                            k
                        })
                    };
                    if let Some(obj) = self.engine.create_object(sound) {
                        self.engine.actor_attach_top(hx, obj, key, channel, false);
                        self.engine.send(obj, 0, 0);
                    }
                }
            }
        }
        for (actor, kind, sound) in stops {
            if actor.is_empty() && sound.is_empty() {
                break;
            }
            // "All" for the sound → deferred stop at the start of the next
            // Update; actor "All" with a specific sound → nothing.
            if actor.eq_ignore_ascii_case("All") {
                if sound.eq_ignore_ascii_case("All") {
                    self.pending_stop = Some((None, kind));
                }
                continue;
            }
            let id = world.find_actor(&actor);
            let a = id.and_then(|id| world.actor(id));
            // GetHXActor, the actor possibly null (special actors).
            let Some(hx) = self.get_hx_actor(a.as_ref(), a.as_ref(), kind) else { continue };
            if sound.eq_ignore_ascii_case("All") {
                self.pending_stop = Some((id, kind));
                continue;
            }
            // Sound key: Sound + 0x64 (GetSound), otherwise that of the direct
            // resource (table + 0x68); its objects on this HX actor are stopped
            // (then + 4 of each).
            let Some(cuuid) = parse_cuuid(&sound) else { continue };
            let key = match world.sound_by_cuuid(cuuid).filter(|s| !s.handler) {
                Some(s) => self.slot(&s).key,
                None => match self.cuuid_keys.get(&cuuid) {
                    Some(&k) => k,
                    None => continue,
                },
            };
            for obj in self.engine.actor_find_key(hx, key) {
                self.engine.send(obj, 1, 0);
            }
        }
        for (actor, kind, volume) in volumes {
            if actor.is_empty() && kind == -1 {
                break;
            }
            let a = world.find_actor(&actor).and_then(|id| world.actor(id));
            // a volume (dB) above 0 is brought back to 0, below -100 to -100 (the log records it);
            // a NaN passes through unchanged.
            let volume = if volume > 0.0 {
                0.0
            } else if volume < -100.0 {
                -100.0
            } else {
                volume
            };
            if let Some(hx) = self.get_hx_actor(a.as_ref(), a.as_ref(), kind) {
                self.engine.send_with(hx, 6, &[volume.to_bits()]);
            }
        }
    }

    /// TestHandler: "<CUUID> : <handler> <c> <n>, <a1>, … )" (read_test_handler).
    /// HX actor: that of the type (GetHXActor, special actors included); first Sound with a
    /// handler for this CUUID (GetHandlerSounds), then ExecHandler.
    fn test_handler(&mut self, world: &mut dyn World, call: &str, kind: i32, a: Option<&ActorState>) {
        let Some((id, name, n, vals)) = read_test_handler(call.as_bytes()) else { return };
        // malloc(n × 4): a negative n is not allocated (out-of-game case); arguments without
        // a comma read stay undefined in the original (0 here).
        let params: Vec<i32> = (0..n.max(0) as usize).map(|k| vals.get(k).copied().flatten().unwrap_or(0)).collect();
        let hx = self.get_hx_actor(a, a, kind);
        let Some(cuuid) = parse_cuuid_bytes(&id) else { return };
        let Some(s) = world.sound_by_cuuid(cuuid).filter(|s| s.handler) else { return };
        if let Some(hx) = hx {
            self.exec_handler(hx, &s, &String::from_utf8_lossy(&name), &params);
        }
    }

    /// The map's actors with an ambient sound (ParseAmbientActors).
    /// Mods build only: the ambient sound of actor `a` started again as ParseAmbientActors starts
    /// it (a sound replaced by a mod, whose bank came after the level's).
    pub fn restart_ambient_actor(&mut self, a: &ActorState) {
        let Some(s) = &a.ambient_sound else { return };
        if !a.positioned() {
            let slot = if a.underwater { SLOT_EFFECT_SINGLE } else { SLOT_EFFECT };
            self.play_sound(Some(a), Some(a), s, slot);
        }
    }

    pub fn parse_ambient_actors(&mut self, actors: &[ActorState]) {
        self.ambient.clear();
        for a in actors {
            let Some(s) = &a.ambient_sound else { continue };
            if !a.positioned() {
                let slot = if a.underwater { SLOT_EFFECT_SINGLE } else { SLOT_EFFECT };
                self.play_sound(Some(a), Some(a), s, slot);
            } else {
                self.ambient.push(a.id);
            }
        }
    }
}

/// GetLanguageId: prefix of SoundLanguage, case-insensitive (English, French, German,
/// Spanish, Italian, Russian, Polish) → code "en  ", "fr  "…; otherwise (empty included) English.
/// Name of the starting music handler that LoadSounds loads through `StaticLoadObject`: Hualpar2's for Hual02,
/// otherwise "<SoundProject>.Music__<Title>.<Title>__hMusicInit".
pub fn music_init_name(title: &str, project: &str) -> String {
    if title.eq_ignore_ascii_case("Hual02") {
        "XIIISound.Music__Hualpar2.Hualpar2__hMusicInit".to_string()
    } else {
        format!("{project}.Music__{title}.{title}__hMusicInit")
    }
}

pub fn language_id(language: &str) -> u32 {
    let starts = |p: &str| language.len() >= p.len() && language[..p.len()].eq_ignore_ascii_case(p);
    let code: &[u8; 4] = if starts("English") {
        b"en  "
    } else if starts("French") {
        b"fr  "
    } else if starts("German") {
        b"de  "
    } else if starts("Spanish") {
        b"es  "
    } else if starts("Italian") {
        b"it  "
    } else if starts("Russian") {
        b"ru  "
    } else if starts("Polish") {
        b"pl  "
    } else {
        b"en  "
    };
    u32::from_be_bytes(*code)
}

/// Whitespace for sscanf (msvcr70's isspace).
fn white(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// sscanf's "%i" (msvcr70): whitespace skipped, sign, "0x" hexadecimal, "0" octal, otherwise
/// decimal, on 32 bits without saturation; None when no digit is read.
fn read_i(t: &[u8]) -> Option<i32> {
    let mut i = 0;
    while t.get(i).is_some_and(|&c| white(c)) {
        i += 1;
    }
    let neg = match t.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut base = 10;
    let mut value_read = false;
    if t.get(i) == Some(&b'0') {
        value_read = true;
        i += 1;
        base = 8;
        if matches!(t.get(i), Some(b'x' | b'X')) {
            i += 1;
            base = 16;
        }
    }
    let mut v: u32 = 0;
    while let Some(d) = t.get(i).and_then(|&c| (c as char).to_digit(base)) {
        v = v.wrapping_mul(base).wrapping_add(d);
        value_read = true;
        i += 1;
    }
    value_read.then_some(if neg { v.wrapping_neg() } else { v } as i32)
}

/// What ExecHandler asks of the engine: implemented by the game (`ExecVm`) and by the test
/// (the original's log).
pub(crate) trait WorldExec {
    /// Micro table (+0xFC) non-empty.
    fn a_of_micros(&self) -> bool;
    /// Objects of the HX actor for the sound's key (+0xD0, vtable + 0x20): the first program.
    fn program_existing(&mut self) -> Option<ObjId>;
    /// Creation through the sound's resource (+0x58, vtable + 0x1C); None without a resource or if null.
    fn creates_program(&mut self) -> Option<ObjId>;
    /// Attach to the actor (+0xD0, vtable + 0: object, key, -1, 0, 0).
    fn attach(&mut self, p: ObjId);
    /// CheckLaunchTime.
    fn launch_allowed(&mut self) -> bool;
    /// Message 5 to the program (name and arguments).
    fn sends(&mut self, p: ObjId);
    /// UpdateChannel.
    fn sets_to_day_channel(&mut self);
}

/// ExecHandler: without a micro, nothing; the actor's program for
/// the sound's key, created and attached if needed (failure: 0); launch refused: 1 without sending; otherwise
/// message 5 then UpdateChannel.
pub(crate) fn exec_handler_logic(m: &mut impl WorldExec) -> bool {
    if !m.a_of_micros() {
        return false;
    }
    let prog = match m.program_existing() {
        Some(p) => p,
        None => {
            let Some(p) = m.creates_program() else {
                return false;
            };
            m.attach(p);
            p
        }
    };
    if !m.launch_allowed() {
        return true;
    }
    m.sends(prog);
    m.sets_to_day_channel();
    true
}

struct ExecVm<'a> {
    hx: &'a mut HxAudio,
    actor_value: ObjId,
    s: &'a SoundRef,
    info: Slot,
    name: &'a str,
    params: &'a [i32],
}

impl WorldExec for ExecVm<'_> {
    fn a_of_micros(&self) -> bool {
        !self.hx.micros.is_empty()
    }
    fn program_existing(&mut self) -> Option<ObjId> {
        let e = &self.hx.engine;
        e.actor_find_key(self.actor_value, self.info.key).into_iter().find(|&o| e.program_obj(o).is_some())
    }
    fn creates_program(&mut self) -> Option<ObjId> {
        self.hx.engine.create_object(self.s.cuuid)
    }
    fn attach(&mut self, p: ObjId) {
        self.hx.engine.actor_attach_top(self.actor_value, p, self.info.key, self.info.channel, false);
    }
    fn launch_allowed(&mut self) -> bool {
        self.hx.check_launch_time(self.s)
    }
    fn sends(&mut self, p: ObjId) {
        let mut bytes = Vec::new();
        for v in self.params {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let e = &mut self.hx.engine;
        let block = if self.params.is_empty() { 0 } else { e.mem.alloc_with(&bytes) };
        e.send_named(p, 5, self.name.as_bytes(), block);
        if block != 0 {
            e.mem.release(block);
        }
    }
    fn sets_to_day_channel(&mut self) {
        self.hx.update_channel(self.s);
    }
}

/// Handler of CallMusicHandler(int): 0 "hInfiltration" without
/// an argument, 1 "hAttaque" with the value of MusicVars[MUSIC_VAR_ATTACK] (vtable + 0xC8 on
/// MusicVars + 0x28), other: nothing.
pub fn state_music(which: i32) -> Option<(&'static [u8], bool)> {
    match which {
        0 => Some((b"hInfiltration", false)),
        1 => Some((b"hAttaque", true)),
        _ => None,
    }
}

/// Index of NbAttaque in LevelInfo.MusicVars (20-byte elements).
pub const MUSIC_VAR_ATTACK: usize = 2;

/// Text of CallMusicHandler: sscanf("%s %c %i", handler, c, n); if n is not
/// zero (or not read: undefined in the original, taken as non-zero), one "%i" after each comma of the
/// text, spaces skipped, up to ")".
pub fn read_call_music(t: &[u8]) -> Option<(Vec<u8>, Option<i32>, Vec<Option<i32>>)> {
    let mut i = 0;
    while t.get(i).is_some_and(|&c| white(c)) {
        i += 1;
    }
    let d = i;
    while t.get(i).is_some_and(|&c| !white(c)) {
        i += 1;
    }
    let name = t[d..i].to_vec();
    if name.is_empty() {
        return None;
    }
    while t.get(i).is_some_and(|&c| white(c)) {
        i += 1;
    }
    let n = if i < t.len() { read_i(&t[i + 1..]) } else { None };
    let mut vals = Vec::new();
    if n != Some(0) {
        let mut i = 0;
        while i < t.len() && t[i] != b')' {
            if t[i] == b',' {
                i += 1;
                while t.get(i) == Some(&b' ') {
                    i += 1;
                }
                vals.push(read_i(&t[i.min(t.len())..]));
            }
            i += 1;
        }
    }
    Some((name, n, vals))
}

/// Text of TestHandler: sscanf("%s : %s %c %i", CUUID, handler, c, n) (n is
/// 2 if sscanf stops earlier), then, if n is not zero, one "%i" after each comma of the whole
/// text (spaces skipped) up to the character before ")". None if the
/// CUUID, the ":" or the handler is missing (the original then starts from an undefined buffer).
pub fn read_test_handler(t: &[u8]) -> Option<(Vec<u8>, Vec<u8>, i32, Vec<Option<i32>>)> {
    let mut i = 0;
    let skips = |i: &mut usize| {
        while t.get(*i).is_some_and(|&c| white(c)) {
            *i += 1;
        }
    };
    let word_value = |i: &mut usize| {
        let d = *i;
        while t.get(*i).is_some_and(|&c| !white(c)) {
            *i += 1;
        }
        t[d..*i].to_vec()
    };
    skips(&mut i);
    let id = word_value(&mut i);
    if id.is_empty() {
        return None;
    }
    skips(&mut i);
    if t.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    skips(&mut i);
    let name = word_value(&mut i);
    if name.is_empty() {
        return None;
    }
    skips(&mut i);
    let mut n = 2;
    if i < t.len() {
        i += 1;
        skips(&mut i);
        if let Some(v) = read_i(&t[i..]) {
            n = v;
        }
    }
    let mut vals = Vec::new();
    if n != 0 && t.first() != Some(&b')') {
        let mut i = 0;
        loop {
            if t.get(i) == Some(&b',') {
                i += 1;
                while t.get(i) == Some(&b' ') {
                    i += 1;
                }
                vals.push(read_i(&t[i.min(t.len())..]));
            }
            i += 1;
            if i >= t.len() || t[i] == b')' {
                break;
            }
        }
    }
    Some((id, name, n, vals))
}

/// "5B038131-BFC6-6DE1" → CUUID (USound::UuidToQword: hexadecimal digits, dashes skipped).
/// HXAudio has its own: (digits).
/// ParseCommand: the word (case-insensitive) followed by something other than a letter or digit; the rest, whitespace skipped.
pub fn parse_command<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let t = s.trim_start_matches([' ', '\t']);
    let (b, w) = (t.as_bytes(), word.as_bytes());
    if b.len() < w.len() || !b[..w.len()].eq_ignore_ascii_case(w) {
        return None;
    }
    let rest = &t[w.len()..];
    if rest.as_bytes().first().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(rest.trim_start_matches([' ', '\t']))
}

pub fn parse_cuuid(text: &str) -> Option<Cuuid> {
    parse_cuuid_bytes(text.as_bytes())
}

/// CUUID read from text: dashes at 8 and 13, byte 18 zero ("XXXXXXXX-XXXX-XXXX"), each piece read; the second word is (2nd piece << 16) + 3rd.
/// CUUID::ToText: "%08X-%04X-%04X" of the first word then of the two halves of the second.
pub fn text_cuuid(c: Cuuid) -> String {
    format!("{:08X}-{:04X}-{:04X}", c.0, c.1 >> 16, c.1 & 0xFFFF)
}

pub fn parse_cuuid_bytes(t: &[u8]) -> Option<Cuuid> {
    let at = |i: usize| t.get(i).copied().unwrap_or(0);
    // Past the text, its terminator (and, beyond, memory: zero here).
    if at(8) != b'-' || at(13) != b'-' || at(18) != 0 {
        return None;
    }
    let a = hex_hx(&t[0..]);
    let b = hex_hx(&t[9..]);
    let c = hex_hx(&t[14..]);
    Some(Cuuid(a, (b << 16).wrapping_add(c)))
}

/// Hexadecimal number up to the first class-0 character (table,
/// indexed by the signed byte): digits and letters A-F / a-f count, classes above 3
/// are skipped; any other ASCII character (dash, terminator) stops. Bytes ≥ 0x80 read the
/// 512 bytes before the table: 0xBB, 0xBC, 0xBD are classes 1, 2, 3 there, 0xC1 and 0xFF
/// class 0, the others skipped.
fn hex_hx(t: &[u8]) -> u32 {
    let mut v = 0u32;
    for i in 0.. {
        let b = t.get(i).copied().unwrap_or(0);
        let c = b as i8 as i32 as u32;
        let class_value = match b {
            b'0'..=b'9' => 1,
            b'A'..=b'F' => 2,
            b'a'..=b'f' => 3,
            0xbb => 1,
            0xbc => 2,
            0xbd => 3,
            0xc1 | 0xff => 0,
            0x80..=0xff => 4,
            _ => 0,
        };
        match class_value {
            0 => break,
            1 => v = (v << 4).wrapping_add(c).wrapping_sub(0x30),
            2 => v = (v << 4).wrapping_add(c).wrapping_sub(0x37),
            3 => v = (v << 4).wrapping_add(c).wrapping_sub(0x57),
            _ => {}
        }
    }
    v
}

/// Slot of a listener table (8 slots): the first free one, otherwise a new one; table
/// full, the write is lost. A slot is free when its fields equal those of an emptied
/// slot, so a write that has them (empty name…) is taken over by the next one.
/// HXUpdate (called by the update thread every 30 ms): the engine update only happens if more than 30 ms
/// (unsigned comparison of timeGetTime) have elapsed since the last one, whose time is only taken if it happens. This rule
/// is not wired in free play.
pub fn update_thread(now_value: u32, last_value: &mut u32) -> bool {
    if now_value.wrapping_sub(*last_value) > 0x1e {
        *last_value = now_value;
        true
    } else {
        false
    }
}

/// Loop of the update thread (ParallelUpdateThread): while the flag is set, HXUpdate then Sleep(30).
pub fn loop_thread(active_value: &mut dyn FnMut() -> bool, update_value: &mut dyn FnMut(), sleep_value: &mut dyn FnMut(u32)) {
    if !active_value() {
        return;
    }
    loop {
        update_value();
        sleep_value(30);
        if !active_value() {
            break;
        }
    }
}

/// Maximum number of USounds in the subsystem's array (0x1388, FillSoundsArray).
pub const MAX_SOUNDS: usize = 5000;

/// FillSoundsArray: the USound array receives, in GObjObjects index order, the objects that are Sounds
/// (`Some(true)`), at most MAX_SOUNDS; empty slots and other objects are skipped. Returns the indices kept.
pub fn fill_array_sounds(cases: &[Option<bool>]) -> Vec<usize> {
    cases.iter().enumerate().filter(|(_, c)| **c == Some(true)).map(|(i, _)| i).take(MAX_SOUNDS).collect()
}

/// Difference of two vectors: `a − b` component by component.
fn vec_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Flash listener: the variable's value is only written if the flash handler exists.
fn flash_write(handler_value: Option<u32>, value_value: &mut i32, read_one: i32) {
    if handler_value.is_some() {
        *value_value = read_one;
    }
}

fn store<T>(table: &mut Vec<T>, free: impl Fn(&T) -> bool, e: T) {
    if let Some(i) = table.iter().position(free) {
        table[i] = e;
    } else if table.len() < 8 {
        table.push(e);
    }
}

