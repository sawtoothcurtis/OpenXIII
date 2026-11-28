//! Saves: USaveGameDevice and its PC version UPCSaveGameDevice
//! A queue of operations requested by the menus (Request*), run
//! one per frame by Tick, whose completion the menus read (Is*Finished). Profiles "Name.pro":
//! 10 slots of 0x800 bytes then the player's configuration (compressed User.ini) at
//! 0x5000. Checkpoints (SaveAtCheckpoint): URL, exported inventory (ExportTravel) and
//! music state (UAudioSaver), compressed with zlib.
//!
//! The profile folder is OpenXIII's own ("save" next to "game"): the original game
//! is never modified; its profiles are copied there at the first launch.

use crate::value::{ObjId, Value};
use crate::Vm;
#[cfg(test)]
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};

/// eOperationToExecute.
pub const OP_CREATE_PROFILE: usize = 1;
pub const OP_USE_PROFILE: usize = 2;
pub const OP_CHECK_CARD: usize = 3;
pub const OP_GET_PROFILE_LIST: usize = 4;
pub const OP_IS_SLOT_EMPTY: usize = 5;
pub const OP_GET_DESCRIPTION: usize = 6;
pub const OP_GET_DATE: usize = 7;
pub const OP_ERASE_SLOT: usize = 8;
pub const OP_READ_SLOT: usize = 9;
pub const OP_WRITE_SLOT: usize = 10;
pub const OP_READ_USER_CONFIG: usize = 11;
pub const OP_WRITE_USER_CONFIG: usize = 12;
pub const OP_LOAD_CHECKPOINT: usize = 13;

const SLOT_SIZE: u64 = 0x800;
const USER_CONFIG_OFFSET: u64 = 0x5000;
const USER_CONFIG_SIZE: usize = 0x1400;

/// UAudioSaver: music started last (AActor::execPlayMusic) and MusicVars, given back to the
/// next map by LoadMap when `flag` is set (loading a checkpoint).
#[derive(Debug, Clone)]
pub struct AudioSaver {
    pub flag: i32,
    pub name: String,
    pub params: [i32; 5],
    pub vars: Vec<(String, i32, u8)>,
}

impl Default for AudioSaver {
    fn default() -> Self {
        AudioSaver { flag: 0, name: String::new(), params: [-1; 5], vars: Vec::new() }
    }
}

/// Engine state that survives the map change (UGameEngine subsystems).
#[derive(Debug, Default, Clone)]
pub struct EngineState {
    pub save: Option<SaveDevice>,
    pub audio_saver: AudioSaver,
    /// GConfig: the configuration files modified in game (SET, profiles) stay from one map
    /// to the next.
    pub config: Option<crate::config::ConfigCache>,
    /// VideoPlayerDevice (UPCVideoPlayerDevice): a video continues during a travel.
    pub video: crate::video::VideoDevice,
    /// UWindowsClient: active viewports and the number wanted for the next map (split screen).
    pub client: crate::client::ClientState,
    /// UClass::ClassUnique of the script classes (by lowercase name): the classes stay
    /// loaded from one map to the next, the names of created objects continue their numbering.
    pub class_unique: std::collections::HashMap<String, u16>,
    /// Objects loaded by the emulated linker (load_order.rs) that survive the travel.
    pub load_order: Option<crate::load_order::LoadOrder>,
    /// UEngine::CurrentTickRate (+0xB8) and its measurement by Engine.dll's main loop.
    pub tick_rate: TickRate,
    /// Mods build only: the scripts' memory, each mod's `save` table (mod_script_save.rs).
    #[cfg(feature = "mods")]
    pub mod_script_save: crate::mod_script_save::Saves,
    /// Mods build only: the memory of the checkpoint being loaded, for the next map's scripts.
    #[cfg(feature = "mods")]
    pub mod_script_save_restore: Option<crate::mod_script_save::Saves>,
}

/// Measurement: at each frame, one more frame; as soon as a real second (appSeconds) has passed
/// since the last measurement, CurrentTickRate = frames / seconds elapsed (float), then the count restarts.
#[derive(Debug, Default, Clone)]
pub struct TickRate {
    pub current: f32,
    frames: i32,
    last: Option<f64>,
}

impl TickRate {
    /// A frame finished at real time `now` (seconds since the process started).
    pub fn frame(&mut self, now: f64) {
        let last = *self.last.get_or_insert(now);
        self.frames += 1;
        if now > last + 1.0 {
            // fsub then fdivp under the game FPU's 24-bit precision.
            self.current = self.frames as f32 / (now - last) as f32;
            self.frames = 0;
            self.last = Some(now);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SaveDevice {
    /// GSys->SavePath.
    pub dir: PathBuf,
    /// Slots read so far (Continue, Load): the viewer drops the Select level equipment when a
    /// save is loaded (choice_level.rs).
    pub slot_reads: u32,
    /// Pending operations (at most 16) and their parameter (+0x110, +0x150).
    pending: Vec<(usize, i32)>,
    /// +0x64: operation finished; +0xD8: return code.
    finished: [bool; 14],
    ret: [i32; 14],
    /// +0x9C: parameter of the current operation (slot).
    param: i32,
    /// +0xA0: profile.
    pub profile: String,
    /// +0xAC: description read or to write.
    desc: String,
    /// +0xB8…+0xBF: date of the slot (year, month, day, hour, minute).
    date: (i32, [u8; 4]),
    /// +0xC0: profiles found.
    profiles: Vec<String>,
    /// +0xCC.
    slot_empty: i32,
    /// +0xD0 / +0xD4: parameters of LaunchFromCheckpoint.
    load_mode: u8,
    load_from_start: i32,
    /// +0x30 / +0x3C: last checkpoint and its description; +0x48 / +0x54: the one at the
    /// start of the map.
    pub checkpoint: Vec<u8>,
    pub checkpoint_desc: String,
    pub start: Vec<u8>,
    pub start_desc: String,
    /// +0x60: RequestWriteSlot(…, bAtLastCheckpoint).
    at_last_checkpoint: bool,
    /// Mods build only: the scripts' memory saved with the last checkpoint and with the start
    /// of the map (mod_script_save.rs).
    #[cfg(feature = "mods")]
    pub(crate) mod_save_checkpoint: crate::mod_script_save::Saves,
    #[cfg(feature = "mods")]
    pub(crate) mod_save_start: crate::mod_script_save::Saves,
    /// +0x194: profile file.
    path: String,
    /// Step of OperationReadSlot (reading, then loading on the next frame).
    read_stage: u8,
}

// ---- Archives (FArchive) ----

fn put_compact(out: &mut Vec<u8>, v: i32) {
    let mut a = v.unsigned_abs();
    let mut b0 = if v < 0 { 0x80u8 } else { 0 };
    if a < 0x40 {
        b0 |= a as u8;
        out.push(b0);
        return;
    }
    b0 |= (a & 0x3f) as u8 | 0x40;
    out.push(b0);
    a >>= 6;
    loop {
        if a < 0x80 {
            out.push(a as u8);
            return;
        }
        out.push((a & 0x7f) as u8 | 0x80);
        a >>= 7;
    }
}

/// operator<<(FArchive&, FString&): length with the final zero (0 for an empty string).
fn put_string(out: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        put_compact(out, 0);
        return;
    }
    let bytes: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect();
    put_compact(out, bytes.len() as i32 + 1);
    out.extend_from_slice(&bytes);
    out.push(0);
}

/// operator<<(FArchive&, TArray<TMap<FString, FString>::TPair>&) when writing: compact count, then the key and the
/// value of each pair (TravelInfo of SaveAtCheckpoint).
fn put_pairs(out: &mut Vec<u8>, pairs: &[(String, String)]) {
    put_compact(out, pairs.len() as i32);
    for (k, v) in pairs {
        put_string(out, k);
        put_string(out, v);
    }
}

/// TArray<BYTE>: compact count then the bytes.
fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_compact(out, b.len() as i32);
    out.extend_from_slice(b);
}

struct In<'a>(openxiii_package::Reader<'a>);

impl<'a> In<'a> {
    fn new(data: &'a [u8]) -> Self {
        In(openxiii_package::Reader::new(data, 0))
    }
    fn byte(&mut self) -> Option<u8> {
        self.0.u8().ok()
    }
    fn int(&mut self) -> Option<i32> {
        self.0.i32().ok()
    }
    fn compact(&mut self) -> Option<i32> {
        self.0.compact().ok()
    }
    /// operator<<(FArchive&, FString&) when reading: length with the final zero, then the bytes as
    /// they are (a zero in the middle stays in the string, which `*FString` will cut at the first zero);
    /// negative length: 16-bit characters.
    fn string(&mut self) -> Option<String> {
        let len = self.compact()?;
        if len == 0 {
            return Some(String::new());
        }
        if len > 0 {
            let b = self.0.take(len as usize).ok()?;
            Some(b[..b.len() - 1].iter().map(|&c| c as char).collect())
        } else {
            let b = self.0.take((len.unsigned_abs() as usize).checked_mul(2)?).ok()?;
            let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units[..units.len() - 1]))
        }
    }
    fn slot_description(&mut self) -> Option<(u8, String)> {
        Some((self.byte()?, self.string()?))
    }

    fn slot_date(&mut self) -> Option<(i32, [u8; 4])> {
        Some((self.int()?, self.0.take(4).ok()?.try_into().ok()?))
    }

    /// When reading: compact count (Empty), then for each pair AddZeroed, key and value.
    fn pairs(&mut self) -> Vec<(String, String)> {
        let n = self.compact().unwrap_or(0);
        let mut info = Vec::new();
        for _ in 0..n.max(0) {
            // A damaged count stops at the end of the data instead of filling memory with empty pairs.
            let (Some(k), Some(v)) = (self.string(), self.string()) else { break };
            info.push((k, v));
        }
        info
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.compact()?;
        self.0.take(usize::try_from(n).ok()?).ok().map(|b| b.to_vec())
    }
}

/// UAudioSaver's MusicVars array when reading: compact count, then
/// name (FString) and value of each element; the type is not read back.
fn read_vars_music(r: &mut In, previous: &[u8]) -> Vec<(String, i32, u8)> {
    let mut vars = Vec::new();
    let nv = r.compact().unwrap_or(0);
    for i in 0..nv.max(0) as usize {
        // Same as `pairs`: stop at the end of the data.
        let Some(name) = r.string() else { break };
        let v = r.int().unwrap_or(0);
        vars.push((name, v, previous.get(i).copied().unwrap_or(0)));
    }
    vars
}

/// UAudioSaver and element: the type stays in memory.
fn put_audio_saver(out: &mut Vec<u8>, s: &AudioSaver) {
    put_string(out, &s.name);
    for p in s.params { out.extend_from_slice(&p.to_le_bytes()); }
    put_compact(out, s.vars.len() as i32);
    for (name, value, _) in &s.vars {
        put_string(out, name);
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// compress (zlib) with the original size in front: Engine's compress (compress2 at level
/// Z_DEFAULT_COMPRESSION) replaced by flate2, identical output byte for byte.
fn pack(raw: &[u8], header: u32) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(raw);
    let body = e.finish().unwrap_or_default();
    let mut out = header.to_le_bytes().to_vec();
    out.extend(body);
    out
}

/// uncompress (zlib 1.1.3: crate::zlib11): the original size in front is used as
/// capacity; None if the stream is damaged, incomplete or larger than the capacity, or asks for a
/// dictionary (only Z_STREAM_END is a success).
fn unpack(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 4 {
        return None;
    }
    // zlib cannot expand data more than 1032 times: a larger size in front comes from a damaged
    // file and only the bound is reserved (same result, without a multi-gigabyte allocation).
    let size = (u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize).min((data.len() - 4).saturating_mul(1032) + 64);
    let mut out = Vec::new();
    out.try_reserve_exact(size).ok()?;
    out.resize(size, 0);
    let (code, len) = crate::zlib11::uncompress(&mut out, &data[4..]);
    if code != crate::zlib11::Z_OK {
        return None;
    }
    out.truncate(len);
    Some(out)
}

/// SaveAtCheckpoint, raw content: URL of the map ("map#portal" if a portal is given), TravelInfo
/// (one pair: player name, travel text), then UAudioSaver.
pub(crate) fn checkpoint_raw(map: &str, portal: &str, name: &str, text: &str, saver: &AudioSaver) -> Vec<u8> {
    let mut raw = Vec::new();
    if portal.is_empty() {
        put_string(&mut raw, map);
    } else {
        put_string(&mut raw, &format!("{map}#{portal}"));
    }
    put_pairs(&mut raw, &[(name.to_string(), text.to_string())]);
    put_audio_saver(&mut raw, saver);
    raw
}

/// End of SaveAtCheckpoint: compress into a buffer the size of the raw data; if the compressed data does not fit
/// (Z_BUF_ERROR), nothing is stored and the function returns false. Otherwise the checkpoint (+0x30, raw size in front) and its
/// description (+0x3C), and also the starting point (+0x48, +0x54) for an empty portal, "PlayerStart" (appStricmp)
/// or a starting point still empty.
pub(crate) fn store_checkpoint(d: &mut SaveDevice, raw: &[u8], portal: &str, desc: &str) -> bool {
    let blob = pack(raw, raw.len() as u32);
    if blob.len() - 4 > raw.len() {
        return false;
    }
    d.checkpoint = blob.clone();
    d.checkpoint_desc = desc.to_string();
    if portal.is_empty() || portal.eq_ignore_ascii_case("PlayerStart") || d.start.is_empty() {
        d.start = blob;
        d.start_desc = desc.to_string();
    }
    true
}

/// Decompressed checkpoint (format of SaveAtCheckpoint): URL "map#portal",
/// player, travel text, then the UAudioSaver part as is.
pub struct CheckpointData {
    pub url: String,
    pub player_value: String,
    pub text_value: String,
    pub music_value: Vec<u8>,
}

/// Reads a checkpoint (USaveGameDevice +0x30, .chk file of a campaign).
pub fn read_checkpoint(data: &[u8]) -> Option<CheckpointData> {
    let raw = unpack(data)?;
    let mut r = In::new(&raw);
    let url = r.string()?;
    let n = r.compact()?;
    let (mut player_value, mut text_value) = (String::new(), String::new());
    for _ in 0..n.max(0) {
        player_value = r.string()?;
        text_value = r.string()?;
    }
    let music_value = raw[r.0.pos..].to_vec();
    Some(CheckpointData { url, player_value, text_value, music_value })
}

/// Writes a checkpoint like SaveAtCheckpoint: URL, 1, player, text, then the music
/// (without a template: empty UAudioSaver, name "", parameters at −1, no MusicVar).
pub fn write_checkpoint(url: &str, player_value: &str, text_value: &str, music_value: Option<&[u8]>) -> Vec<u8> {
    let mut raw = Vec::new();
    put_string(&mut raw, url);
    put_compact(&mut raw, 1);
    put_string(&mut raw, player_value);
    put_string(&mut raw, text_value);
    match music_value {
        Some(m) => raw.extend_from_slice(m),
        None => {
            let s = AudioSaver::default();
            put_string(&mut raw, &s.name);
            for p in s.params {
                raw.extend_from_slice(&p.to_le_bytes());
            }
            put_compact(&mut raw, 0);
        }
    }
    pack(&raw, raw.len() as u32)
}

fn latin1(b: &[u8]) -> String {
    b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect()
}

/// Profile folder set by the game launched alone (%APPDATA%\OpenXIII\save); without it, "save"
/// next to the game folder.
static FOLDER_SAVES: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Sets the profile folder, before the game is loaded.
pub fn set_folder_saves(dir: PathBuf) {
    let _ = FOLDER_SAVES.set(dir);
}

pub(crate) fn folder_saves() -> Option<PathBuf> {
    FOLDER_SAVES.get().cloned()
}

/// Android: folder of the markers for the aim assist switched on by default (iAutoAimMode), one per profile.
static AIM_ASSIST_MARKERS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Android: the first time a profile is used, its User.ini gets iAutoAimMode=1 and
/// bRotationAssist=True (the console versions' aim assist); afterwards the player's choice in the options is kept.
pub fn set_aim_assist_markers(dir: PathBuf) {
    let _ = AIM_ASSIST_MARKERS.set(dir);
}

impl SaveDevice {
    /// UPCSaveGameDevice: Init (USaveGameDevice::Init); the folder receives the
    /// original game's profiles at the first launch.
    pub fn new(dir: PathBuf, original: Option<PathBuf>) -> Self {
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
            if let Some(o) = original {
                if let Ok(rd) = std::fs::read_dir(o) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pro")) {
                            if let Some(n) = p.file_name() {
                                let _ = std::fs::copy(&p, dir.join(n));
                            }
                        }
                    }
                }
            }
        }
        SaveDevice { dir, finished: [true; 14], ..Default::default() }
    }

    /// Read the entire stored checkpoint before replacing either in-memory copy.
    fn restore_slot_data(&mut self, data: &[u8]) -> Option<()> {
        let mut reader = In::new(data);
        if reader.byte()? == 0 {
            return None;
        }
        let (version, description) = reader.slot_description()?;
        let date = if version > 2 { reader.slot_date()? } else { self.date };
        let checkpoint = reader.bytes()?;
        self.start_desc.clone_from(&description);
        self.checkpoint_desc = description;
        self.date = date;
        self.start.clone_from(&checkpoint);
        self.checkpoint = checkpoint;
        Some(())
    }

    /// USaveGameDevice::GetCurrentOperation: the first pending operation.
    fn current(&self) -> usize {
        self.pending.first().map_or(0, |p| p.0)
    }

    /// USaveGameDevice::AddOperation.
    fn add_operation(&mut self, op: usize, param: i32) {
        if self.pending.len() >= 16 {
            return;
        }
        self.pending.push((op, param));
        self.finished[op] = false;
        if self.pending.len() == 1 {
            self.param = param;
            self.ret[op] = 0;
        }
    }

    /// USaveGameDevice::FinishCurrentOperation.
    fn finish(&mut self, op: usize) {
        if self.pending.is_empty() || self.pending[0].0 != op {
            return;
        }
        self.finished[op] = true;
        self.pending.remove(0);
        if let Some(&(next, p)) = self.pending.first() {
            self.ret[next] = 0;
            self.param = p;
        }
    }

    fn fail(&mut self, code: i32) {
        let op = self.current();
        self.ret[op] = code;
        self.finish(op);
    }

    fn profile_path(&self) -> String {
        self.dir.join(format!("{}.pro", self.profile)).to_string_lossy().into_owned()
    }

    // ---- Menu requests (vtable +0x78…+0xE4) ----

    /// RequestCreateProfile: operation 1, profile name (+0xA0).
    pub fn request_create_profile(&mut self, name: &str) -> bool {
        self.add_operation(OP_CREATE_PROFILE, 0);
        self.profile = name.to_string();
        true
    }
    /// IsCreateProfileFinished.
    pub fn is_create_profile_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_CREATE_PROFILE, code)
    }
    /// Lockstep co-op: the session's profile (the host's) in use, its files the session's
    /// (session_disk.rs).
    pub fn place_session_profile(&mut self, name: &str) {
        self.profile = name.to_string();
        self.path = self.profile_path();
    }
    /// The profile in use of another device (a lockstep session's new VM keeps this machine's
    /// player's profile: each machine writes its own saves).
    pub fn take_profile_of(&mut self, other: &SaveDevice) {
        self.profile.clone_from(&other.profile);
        self.path.clone_from(&other.path);
    }
    /// RequestUseProfile: operation 2.
    pub fn request_use_profile(&mut self, name: &str) -> bool {
        self.add_operation(OP_USE_PROFILE, 0);
        self.profile = name.to_string();
        true
    }
    /// IsUseProfileFinished.
    pub fn is_use_profile_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_USE_PROFILE, code)
    }
    /// RequestGetProfileList: operation 4.
    pub fn request_get_profile_list(&mut self) -> bool {
        self.add_operation(OP_GET_PROFILE_LIST, 0);
        true
    }
    /// IsGetProfileListFinished.
    pub fn is_get_profile_list_finished(&mut self, code: &mut i32, list: &mut Vec<String>) -> bool {
        if !self.finished[OP_GET_PROFILE_LIST] {
            return false;
        }
        *code = self.ret[OP_GET_PROFILE_LIST];
        *list = std::mem::take(&mut self.profiles);
        self.finish(OP_GET_PROFILE_LIST);
        true
    }
    /// DoesMemoryCardIncludeThisProfile: never finished on PC (no operation 3).
    pub fn does_memory_card_include_this_profile(&mut self, code: &mut i32) -> bool {
        if !self.finished[OP_CHECK_CARD] {
            *code = -1;
            return false;
        }
        *code = 1;
        let r = self.ret[OP_CHECK_CARD] > 0;
        self.finish(OP_CHECK_CARD);
        r
    }
    /// StartNewCheckOnMemoryCardForThisProfile.
    pub fn start_new_check(&mut self, code: &mut i32, name: &str) -> bool {
        if !self.pending.iter().any(|p| p.0 == OP_CHECK_CARD) {
            self.add_operation(OP_CHECK_CARD, 0);
            self.profile = name.to_string();
        }
        *code = 1;
        true
    }
    fn slot_request(&mut self, op: usize, slot: i32) -> bool {
        if !(0..10).contains(&slot) {
            return false;
        }
        self.add_operation(op, slot);
        true
    }
    /// RequestIsSlotEmpty: slot in 0…GetMaxNumberOfSavingSlots (10, UPCSaveGameDevice), operation 5, +0xCC = 0.
    pub fn request_is_slot_empty(&mut self, slot: i32) -> bool {
        if !self.slot_request(OP_IS_SLOT_EMPTY, slot) {
            return false;
        }
        self.slot_empty = 0;
        true
    }
    /// IsSlotEmptyFinished.
    pub fn is_slot_empty_finished(&mut self, code: &mut i32, empty: &mut i32) -> bool {
        if !self.finished[OP_IS_SLOT_EMPTY] {
            return false;
        }
        *code = self.ret[OP_IS_SLOT_EMPTY];
        *empty = self.slot_empty;
        self.finish(OP_IS_SLOT_EMPTY);
        true
    }
    /// RequestGetSlotContentDescription: operation 6.
    pub fn request_get_description(&mut self, slot: i32) -> bool {
        self.slot_request(OP_GET_DESCRIPTION, slot)
    }
    /// IsGetSlotContentDescriptionFinished: description (+0xAC) returned then cleared.
    pub fn is_get_description_finished(&mut self, code: &mut i32, desc: &mut String) -> bool {
        if !self.finished[OP_GET_DESCRIPTION] {
            return false;
        }
        *code = self.ret[OP_GET_DESCRIPTION];
        *desc = std::mem::take(&mut self.desc);
        self.finish(OP_GET_DESCRIPTION);
        true
    }
    /// RequestGetSlotContentDateAndTime: operation 7.
    pub fn request_get_date(&mut self, slot: i32) -> bool {
        self.slot_request(OP_GET_DATE, slot)
    }
    /// IsGetSlotContentDateAndTimeFinished: date (+0xB8…+0xBF) if the code is positive.
    pub fn is_get_date_finished(&mut self, code: &mut i32, date: &mut (i32, [u8; 4])) -> bool {
        if !self.finished[OP_GET_DATE] {
            return false;
        }
        *code = self.ret[OP_GET_DATE];
        if *code >= 0 {
            *date = self.date;
        }
        self.finish(OP_GET_DATE);
        true
    }
    /// RequestEraseSlot: operation 8.
    pub fn request_erase_slot(&mut self, slot: i32) -> bool {
        self.slot_request(OP_ERASE_SLOT, slot)
    }
    /// IsEraseSlotFinished.
    pub fn is_erase_slot_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_ERASE_SLOT, code)
    }
    /// RequestWriteSlot.
    pub fn request_write_slot(&mut self, slot: i32, desc: &str, at_last_checkpoint: bool) -> bool {
        if !(0..10).contains(&slot) {
            return false;
        }
        self.desc = desc.to_string();
        self.at_last_checkpoint = at_last_checkpoint;
        self.add_operation(OP_WRITE_SLOT, slot);
        true
    }
    /// IsWriteSlotFinished.
    pub fn is_write_slot_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_WRITE_SLOT, code)
    }
    /// RequestReadSlot: operation 9.
    pub fn request_read_slot(&mut self, slot: i32) -> bool {
        self.slot_request(OP_READ_SLOT, slot)
    }
    /// IsReadSlotFinished: the operation only ends here on error (the
    /// loading ends it otherwise).
    pub fn is_read_slot_finished(&mut self, code: &mut i32) -> bool {
        if !self.finished[OP_READ_SLOT] {
            return false;
        }
        *code = self.ret[OP_READ_SLOT];
        if *code < 0 {
            self.finish(OP_READ_SLOT);
        }
        true
    }
    /// RequestReadUserConfig: operation 11.
    pub fn request_read_user_config(&mut self) -> bool {
        self.add_operation(OP_READ_USER_CONFIG, 0);
        true
    }
    /// IsReadUserConfigFinished.
    pub fn is_read_user_config_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_READ_USER_CONFIG, code)
    }
    /// RequestWriteUserConfig: operation 12.
    pub fn request_write_user_config(&mut self) -> bool {
        self.add_operation(OP_WRITE_USER_CONFIG, 0);
        true
    }
    /// IsWriteUserConfigFinished.
    pub fn is_write_user_config_finished(&mut self, code: &mut i32) -> bool {
        self.is_finished(OP_WRITE_USER_CONFIG, code)
    }
    /// A profile file is open (its configuration can be written: RequestWriteUserConfig).
    pub fn has_profile(&self) -> bool {
        !self.path.is_empty()
    }
    /// IsMemoryCardReady: no operation in progress.
    pub fn is_memory_card_ready(&self) -> bool {
        self.pending.is_empty() || self.pending[0].0 == 0
    }
    /// IsThereACheckpoint: checkpoint in memory (+0x30) not empty.
    pub fn is_there_a_checkpoint(&self) -> bool {
        !self.checkpoint.is_empty()
    }
    /// LaunchFromCheckpoint.
    pub fn launch_from_checkpoint(&mut self, from_start: i32, mode: u8) -> bool {
        self.add_operation(OP_LOAD_CHECKPOINT, 0);
        self.load_mode = mode;
        self.load_from_start = from_start;
        true
    }

    fn is_finished(&mut self, op: usize, code: &mut i32) -> bool {
        if !self.finished[op] {
            return false;
        }
        *code = self.ret[op];
        self.finish(op);
        true
    }

    // ---- Profile files (UPCSaveGameDevice::Operation*) ----

    /// GFileManager->CreateFileWriter(Path, FILEWRITE_Append): file created if missing, kept
    /// otherwise (session_disk.rs: the session's files in a lockstep session), then `data` at `offset`.
    fn append_at(&self, files: &crate::session_disk::SessionFiles, offset: u64, data: &[u8]) -> bool {
        files.write_at(Path::new(&self.path), offset, data, true)
    }

    fn read_at(&self, files: &crate::session_disk::SessionFiles, offset: u64, len: usize) -> Option<Vec<u8>> {
        files.read_at(Path::new(&self.path), offset, len)
    }

    fn write_at(&self, files: &crate::session_disk::SessionFiles, offset: u64, data: &[u8]) -> bool {
        files.write_at(Path::new(&self.path), offset, data, false)
    }
}

impl Vm {
    /// The save device (USaveGameDevice::Instance).
    pub fn save_device(&mut self) -> Option<&mut SaveDevice> {
        self.engine.save.as_mut()
    }

    /// UPCSaveGameDevice::Tick, at the end of UGameEngine::Tick: the operation at the head of
    /// the queue. Loading a checkpoint returns the travel to do.
    pub(crate) fn save_tick(&mut self) -> Option<crate::travel::TravelRequest> {
        let dev = self.engine.save.as_mut()?;
        let op = dev.current();
        if op == OP_READ_SLOT {
            return self.op_read_slot();
        }
        if op == OP_LOAD_CHECKPOINT {
            let (from, mode) = (dev.load_from_start, dev.load_mode);
            let req = self.load_from_checkpoint(from, mode);
            if let Some(d) = self.engine.save.as_mut() {
                d.finish(OP_LOAD_CHECKPOINT);
            }
            return req;
        }
        if op == 0 || dev.finished[op] {
            return None;
        }
        match op {
            OP_CREATE_PROFILE => self.op_create_profile(),
            OP_USE_PROFILE => self.op_use_profile(),
            OP_GET_PROFILE_LIST => self.op_get_profile_list(),
            OP_IS_SLOT_EMPTY => self.op_is_slot_empty(),
            OP_GET_DESCRIPTION => self.op_get_description(),
            OP_GET_DATE => self.op_get_date(),
            OP_ERASE_SLOT => self.op_erase_slot(),
            OP_WRITE_SLOT => self.op_write_slot(),
            OP_READ_USER_CONFIG => self.op_read_user_config(),
            OP_WRITE_USER_CONFIG => self.op_write_user_config(),
            _ => {}
        }
        None
    }

    fn dev(&mut self) -> &mut SaveDevice {
        self.engine.save.as_mut().expect("save device")
    }

    pub(crate) fn active_save_slot(&mut self) -> (String, i32) {
        let device = self.dev();
        (device.path.clone(), device.param)
    }

    /// OperationCreateProfile: file with 10 empty slots and the configuration
    /// area, then the current configuration.
    fn op_create_profile(&mut self) {
        let session_files = self.session.files.clone();
        let d = self.dev();
        d.path = d.profile_path();
        if !session_files.active() {
            let _ = std::fs::create_dir_all(&d.dir);
        }
        let ok = session_files.write(Path::new(&d.path), &vec![0u8; SLOT_SIZE as usize * 10 + USER_CONFIG_SIZE]);
        if !ok {
            self.dev().fail(-1);
            return;
        }
        self.op_write_user_config_inner();
        let op = self.dev().current();
        self.dev().finish(op);
    }

    /// OperationUseProfile.
    fn op_use_profile(&mut self) {
        let session_files = self.session.files.clone();
        let d = self.dev();
        d.path = d.profile_path();
        let size = session_files.len(Path::new(&d.path));
        if size < 1 {
            d.fail(-1);
            return;
        }
        self.op_read_user_config();
    }

    /// OperationGetProfileList: "*.pro" of the folder, without the extension.
    fn op_get_profile_list(&mut self) {
        let session_files = self.session.files.clone();
        let d = self.dev();
        let mut files = Vec::new();
        for n in session_files.names(&d.dir) {
            if Path::new(&n).extension().is_some_and(|x| x.eq_ignore_ascii_case("pro")) {
                files.push(n);
            }
        }
        // FindFiles("*.pro") (FindFirstFile/FindNextFile) returns the files in the order of the
        // NTFS index: full names compared in uppercase ("PLAYER-RUN.PRO" before
        // "PLAYER.PRO", '-' < '.'), before Left(InStr(".pro")) removes the extension.
        files.sort_by_key(|n| n.to_uppercase());
        let names: Vec<String> = files.iter().map(|n| n.find(".pro").map_or(String::new(), |i| n[..i].to_string())).collect();
        d.profiles = names;
        let op = d.current();
        d.finish(op);
    }

    fn slot_offset(&mut self) -> u64 {
        (self.dev().param as u64) << 11
    }

    /// OperationIsSlotEmpty.
    fn op_is_slot_empty(&mut self) {
        let session_files = self.session.files.clone();
        let off = self.slot_offset();
        let d = self.dev();
        match d.read_at(&session_files, off, 1) {
            None => d.fail(-1),
            Some(b) => {
                // The byte read, 1 if missing: the slot is empty if it is not 1.
                d.slot_empty = (b.first().copied().unwrap_or(1) != 1) as i32;
                let op = d.current();
                d.finish(op);
            }
        }
    }

    /// OperationGetSlotContentDescription.
    fn op_get_description(&mut self) {
        let session_files = self.session.files.clone();
        let off = self.slot_offset();
        let d = self.dev();
        let Some(b) = d.read_at(&session_files, off, SLOT_SIZE as usize) else {
            d.fail(-1);
            return;
        };
        let mut r = In::new(&b);
        if r.byte().unwrap_or(0) == 0 {
            d.fail(-2);
            return;
        }
        let Some((_, description)) = r.slot_description() else {
            d.fail(-1);
            return;
        };
        d.desc = description;
        let op = d.current();
        d.finish(op);
    }

    /// OperationGetSlotContentDateAndTime: the date of a slot (version > 2).
    fn op_get_date(&mut self) {
        let session_files = self.session.files.clone();
        let off = self.slot_offset();
        let d = self.dev();
        let Some(b) = d.read_at(&session_files, off, SLOT_SIZE as usize) else {
            d.fail(-1);
            return;
        };
        let mut r = In::new(&b);
        if r.byte().unwrap_or(0) == 0 {
            d.fail(-2);
            return;
        }
        let Some((version, description)) = r.slot_description() else {
            d.fail(-1);
            return;
        };
        let date = if version > 2 {
            let Some(date) = r.slot_date() else {
                d.fail(-1);
                return;
            };
            date
        } else {
            (0, [0; 4])
        };
        d.desc = description;
        d.date = date;
        let op = d.current();
        d.finish(op);
    }

    /// OperationEraseSlot.
    fn op_erase_slot(&mut self) {
        let session_files = self.session.files.clone();
        let off = self.slot_offset();
        let d = self.dev();
        if !d.write_at(&session_files, off, &[0u8; SLOT_SIZE as usize]) {
            d.fail(-1);
            return;
        }
        let op = d.current();
        d.finish(op);
        // Mods build only: the scripts' memory of the slot (mod_script_save.rs).
        #[cfg(feature = "mods")]
        {
            let (path, slot) = self.active_save_slot();
            self.mod_script_save_erase_slot(&path, slot);
        }
        crate::improved::storage::erase_coop_slot(self);
    }

    /// OperationWriteSlot: CreateFileWriter(FILEWRITE_Append) first (failure: code
    /// -1, nothing written or logged, the date stays), then at the slot: occupied, version 3,
    /// description, date, checkpoint; log of the size written, close.
    fn op_write_slot(&mut self) {
        let session_files = self.session.files.clone();
        let off = self.slot_offset();
        let d = self.dev();
        // CreateFileWriter(FILEWRITE_Append) fails: nothing written.
        if !d.append_at(&session_files, 0, &[]) {
            d.fail(-1);
            return;
        }
        let desc = if !d.desc.is_empty() {
            d.desc.clone()
        } else if d.at_last_checkpoint {
            d.checkpoint_desc.clone()
        } else {
            d.start_desc.clone()
        };
        let mut out = vec![1u8, 3];
        put_string(&mut out, &desc);
        // Lockstep co-op: the session's clock (tick.rs), the same on every machine.
        let session = self.lockstep.as_ref().map(|l| l.step).zip(self.session.clock);
        let d = self.dev();
        let now = match session {
            Some((step, base)) => {
                let t = crate::lockstep::level_date(Some(step), Some(base));
                (t[0], t[1] as u8, t[3] as u8, t[4] as u8, t[5] as u8)
            }
            None => local_time(),
        };
        d.date = (now.0, [now.1, now.2, now.3, now.4]);
        out.extend_from_slice(&now.0.to_le_bytes());
        out.extend_from_slice(&[now.1, now.2, now.3, now.4]);
        let data = if d.at_last_checkpoint { d.checkpoint.clone() } else { d.start.clone() };
        put_bytes(&mut out, &data);
        let param = d.param;
        #[cfg(feature = "mods")]
        let (profile, at_last) = (d.path.clone(), d.at_last_checkpoint);
        // FArchiveFileWriter's write errors are not checked.
        let _ = self.dev().append_at(&session_files, off, &out);
        // GLog; beyond SLOT_SIZE, between two pairs of OVERFLOW lines.
        if out.len() > SLOT_SIZE as usize {
            let message = format!("While Saving in slot {param}, we saved {} bytes where {} only could be written", out.len(), SLOT_SIZE);
            self.log_overflow(&message, "SLOT_SIZE");
        } else {
            self.log.push(format!("Log: Data saved in slot {param} took {} bytes where {} could be used", out.len(), SLOT_SIZE));
        }
        // Mods build only: the scripts' memory, in a file of its own (mod_script_save.rs).
        #[cfg(feature = "mods")]
        self.mod_script_save_write_slot(&profile, param, at_last);
        crate::improved::storage::write_coop_slot(self, param);
        let d = self.dev();
        let op = d.current();
        d.finish(op);
    }

    /// OperationReadSlot: reading of a slot's checkpoint, then on the
    /// next frame LoadFromCheckpoint.
    fn op_read_slot(&mut self) -> Option<crate::travel::TravelRequest> {
        let session_files = self.session.files.clone();
        let stage = self.dev().read_stage;
        if stage != 0 {
            if stage == 1 {
                let mode = self.dev().load_mode;
                let req = self.load_from_checkpoint(0, mode);
                let d = self.dev();
                let op = d.current();
                d.finish(op);
                d.read_stage = 0;
                return req;
            }
            return None;
        }
        let off = self.slot_offset();
        let d = self.dev();
        let Some(b) = d.read_at(&session_files, off, SLOT_SIZE as usize) else {
            d.fail(-1);
            return None;
        };
        if d.restore_slot_data(&b).is_none() {
            d.fail(-1);
            return None;
        }
        d.finished[OP_READ_SLOT] = true;
        d.read_stage += 1;
        d.slot_reads = d.slot_reads.wrapping_add(1);
        // Mods build only: the scripts' memory of the slot (mod_script_save.rs).
        #[cfg(feature = "mods")]
        {
            let (path, slot) = self.active_save_slot();
            self.mod_script_save_read_slot(&path, slot);
        }
        crate::improved::storage::read_coop_slot(self);
        None
    }

    /// OperationReadUserConfig.
    fn op_read_user_config(&mut self) {
        let session_files = self.session.files.clone();
        let d = self.dev();
        let Some(mut data) = d.read_at(&session_files, USER_CONFIG_OFFSET, USER_CONFIG_SIZE).and_then(|b| In::new(&b).bytes()) else {
            d.fail(-1);
            return;
        };
        self.set_user_config(&mut data);
        let op = self.dev().current();
        self.dev().finish(op);
    }

    /// OperationWriteUserConfig.
    /// Overflow of a slot or of the configuration: two OVERFLOW lines, the message,
    /// "Please increase the size of … !", two OVERFLOW lines.
    fn log_overflow(&mut self, message: &str, size: &str) {
        let overflow = format!("Log: {}", ["OVERFLOW WARNING"; 6].join(" "));
        self.log.push(overflow.clone());
        self.log.push(overflow.clone());
        self.log.push(format!("Log: {message}"));
        self.log.push(format!("Log: Please increase the size of {size} !"));
        self.log.push(overflow.clone());
        self.log.push(overflow);
    }

    fn op_write_user_config(&mut self) {
        self.op_write_user_config_inner();
        let op = self.dev().current();
        self.dev().finish(op);
    }

    fn op_write_user_config_inner(&mut self) {
        let session_files = self.session.files.clone();
        let data = self.get_user_config();
        let mut out = Vec::new();
        put_bytes(&mut out, &data);
        // GLog, as for a slot.
        if out.len() <= USER_CONFIG_SIZE {
            self.log.push(format!("Log: User config took {} bytes where {} could be used", out.len(), USER_CONFIG_SIZE));
        } else {
            let message = format!("While Saving user config, we saved {} bytes where {} only could be written", out.len(), USER_CONFIG_SIZE);
            self.log_overflow(&message, "USER_CONFIG_SIZE");
        }
        let d = self.dev();
        if !d.write_at(&session_files, USER_CONFIG_OFFSET, &out) {
            d.fail(-1);
        }
    }

    /// USaveGameDevice::GetUserConfig: compressed User.ini text.
    fn get_user_config(&mut self) -> Vec<u8> {
        let text = self.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
        if text.is_empty() {
            return Vec::new();
        }
        let bytes: Vec<u8> = text.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect();
        // Original compress receives only Len+1 destination bytes (call).
        let data = pack(&bytes, bytes.len() as u32 + 1);
        if data.len() - 4 > bytes.len() + 1 { Vec::new() } else { data }
    }

    /// USaveGameDevice::SetUserConfig: User.ini replaced, player name, "User"
    /// configuration values read again, key bindings and PlayerName of the PlayerReplicationInfos.
    fn set_user_config(&mut self, data: &mut Vec<u8>) {
        let Some(mut raw) = unpack(data) else { return };
        let text = latin1(&raw);
        // FArray assignment retains its allocated (header) length, not actual zlib length.
        raw.resize(u32::from_le_bytes(data[..4].try_into().unwrap()) as usize, 0);
        *data = raw;
        if let Some(c) = self.config.as_mut() {
            c.replace_file("User", &text);
        }
        // GConfig vtable +0xC is GetString(FString&), not SetString.
        let profile = self.config.as_mut().and_then(|c| c.get_string("DefaultPlayer", "Name", "User")).unwrap_or_default();
        self.dev().profile = profile.clone();
        if let Some(dir) = AIM_ASSIST_MARKERS.get() {
            // Third marker: the builds before it could leave a profile at 0 with the touch switch
            // showing it on (the view assist was forced on); once more, every profile starts on.
            let marker = dir.join(format!("OpenXIII-aim-assist3-{profile}"));
            if !marker.exists() {
                if let Some(c) = self.config.as_mut() {
                    c.set_string("Engine.PlayerController", "iAutoAimMode", "1", "User");
                    c.set_string("XIII.XIIIPlayerController", "bRotationAssist", "True", "User");
                }
                // The profile's own copy too, read back the next time it is loaded.
                self.dev().request_write_user_config();
                let _ = std::fs::create_dir_all(dir);
                let _ = std::fs::write(&marker, b"1");
                println!("OpenXIII: aim assist turned on for profile {profile}");
            }
        }
        // Each "User" configuration class (ConfigName +0x98), in GObjObjects order:
        // LoadConfig of the default values of its subclasses and of their objects.
        for class in 0..self.classes.len() {
            if self.classes[class].config_name.eq_ignore_ascii_case("User") {
                self.reload_class_config(class);
            }
        }
        // Not in the original: the gamepad's aim assist, which the reload replaced by the
        // keyboard values, comes back at once (aim_device.rs).
        self.aim_config_reloaded();
        let user = self.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
        let bindings = crate::input::Bindings::parse(&user);
        // UInput::ReInit (+0x98) resets every local input, including its input properties.
        let inputs: Vec<_> = self.input_sys.take().into_iter()
            .chain(std::mem::take(&mut self.other_inputs)).collect();
        for old in inputs {
            let pc = old.pc;
            let primary = self.input_sys.replace(old);
            self.reset_input();
            self.input_sys = primary;
            self.input_init(pc, &bindings);
        }
        // Original FObjectIterator includes live PRI outside the level's actor array.
        // Rust CDOs have object slots, while native defaults are outside GObjObjects.
        let taken: Vec<ObjId> = self.objects.iter().enumerate()
            .filter(|(id, object)| object.is_some() && !self.is_cdo(*id) && self.is_a(*id, "PlayerReplicationInfo"))
            .map(|(id, _)| id).collect();
        for p in taken {
            self.set_prop(p, "PlayerName", Value::Str(profile.clone()));
        }
    }

    /// USaveGameDevice::SaveAtCheckpoint: URL (map#portal), the player's
    /// inventory and music, compressed; also the map's starting point for an empty portal
    /// or "PlayerStart".
    pub(crate) fn save_at_checkpoint(&mut self, portal: &str, desc: &str) -> bool {
        if self.engine.save.is_none() {
            return false;
        }
        // Lockstep co-op: the first player's (the host's), on every machine, as split screen saves
        // its first player: "this machine's player" would make each machine's checkpoint its own.
        let pc = match self.lockstep.is_some() {
            true => self.coop.players.first().copied().filter(|&p| self.alive(p)),
            false => self.local_viewport().and_then(|p| self.propr(p, "Actor").as_object()),
        };
        let Some(pc) = pc else { return false };
        let mut text = String::new();
        if let Some(pawn) = self.propr(pc, "Pawn").as_object() {
            self.export_travel(pawn, &mut text);
            let mut inv = self.propr(pawn, "Inventory").as_object();
            let mut guard = 0;
            while let (Some(i), true) = (inv, guard < 256) {
                self.export_travel(i, &mut text);
                inv = self.propr(i, "Inventory").as_object();
                guard += 1;
            }
        }
        let name = self.propr(pc, "PlayerReplicationInfo").as_object().map(|r| self.propr(r, "PlayerName").as_name()).unwrap_or_default();
        let map = self.url.map.clone();
        self.audio_saver_get_vars();
        let s = self.engine.audio_saver.clone();
        let raw = checkpoint_raw(&map, portal, &name, &text, &s);
        // Mods build only: whether the starting point is stored too (store_checkpoint).
        #[cfg(feature = "mods")]
        let start_too = portal.is_empty() || portal.eq_ignore_ascii_case("PlayerStart") || self.engine.save.as_ref().is_none_or(|d| d.start.is_empty());
        let stored = store_checkpoint(self.dev(), &raw, portal, desc);
        // Mods build only: the scripts' memory goes with the checkpoint (mod_script_save.rs).
        #[cfg(feature = "mods")]
        if stored {
            self.mod_script_save_checkpoint(start_too);
        }
        stored
    }

    /// Debugging: the Trigger(Other, EventInstigator) event of actor `name`, the instigator being
    /// the player's pawn (triggers an XIIISaveGameTrigger without walking into it).
    pub fn debug_trigger(&mut self, name: &str, pawn: ObjId) -> bool {
        let Some(a) = self.actors.iter().copied().find(|&a| self.objects[a].as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(name))) else { return false };
        self.event_pub(a, "Trigger", vec![Value::Object(Some(pawn)), Value::Object(Some(pawn))]);
        true
    }

    /// UAudioSaver::GetHXMusicVars: MusicVars of the map, values of the HX sound.
    pub(crate) fn audio_saver_get_vars(&mut self) {
        let vars: Vec<(String, i32, u8)> = self.music_vars();
        let hx = self.audio.as_ref();
        let vars = vars.into_iter().map(|(n, _, t)| match hx {
            Some(h) => {
                let value = h.get_music_var(&n, t);
                (n, value, t)
            }
            None => (n, 0, t),
        });
        self.engine.audio_saver.vars = vars.collect();
    }

    /// USaveGameDevice::LoadFromCheckpoint: checkpoint (or the one at the start of
    /// the map), music given back by the loaded map, absolute travel to its URL. The
    /// UAudioSaver flag stays set during Browse and drops afterwards (`checkpoint_browse_done`).
    fn load_from_checkpoint(&mut self, from_start: i32, _mode: u8) -> Option<crate::travel::TravelRequest> {
        // Mods build only: the scripts' memory of that checkpoint (mod_script_save.rs).
        #[cfg(feature = "mods")]
        self.mod_script_save_load(from_start != 0);
        let d = self.dev();
        if from_start != 0 {
            d.checkpoint = d.start.clone();
        }
        let raw = unpack(&d.checkpoint)?;
        let mut r = In::new(&raw);
        let url = r.string()?;
        let info = r.pairs();
        let mut saver = AudioSaver { name: r.string().unwrap_or_default(), ..Default::default() };
        for p in saver.params.iter_mut() {
            *p = r.int().unwrap_or(-1);
        }
        // Reading: FArray::Empty(0x14, n) then Add element by element, name and
        // value only. The type (+0x10) is not read back: the memory reallocated at the same
        // size keeps that of the previous element at the same rank (0 beyond).
        let previous: Vec<u8> = self.engine.audio_saver.vars.iter().map(|v| v.2).collect();
        saver.vars = read_vars_music(&mut r, &previous);
        // Flag set, music name cleared (FString = "") before Browse.
        saver.flag = 1;
        saver.name.clear();
        self.engine.audio_saver = saver;
        let req = self.browse_request_pub(&url, crate::travel::TRAVEL_ABSOLUTE, info);
        Some(req)
    }

    /// UGameEngine::LoadMap: music of a checkpoint given back to the map
    /// (LevelInfo.InitMusic loaded by its name, InitMusicParameters, MusicVars).
    pub(crate) fn apply_audio_saver(&mut self) {
        if self.engine.audio_saver.flag == 0 {
            return;
        }
        let Some(level) = self.level else { return };
        let s = self.engine.audio_saver.clone();
        // StaticLoadObject(USound, NULL, name): nothing for an empty name.
        let music = if s.name.is_empty() { Value::Object(None) } else { Value::Asset(s.name.clone()) };
        self.set_prop(level, "InitMusic", music);
        self.set_prop(level, "InitMusicParameters", Value::Array(s.params.iter().map(|&p| Value::Int(p)).collect()));
        let vars: Vec<Value> =
            s.vars.iter().map(|(n, v, t)| Value::Struct("levelinfo.musicvar".into(), vec![Value::Str(n.clone()), Value::Int(*v), Value::Byte(*t)].into())).collect();
        self.set_prop(level, "MusicVars", Value::Array(vars));
    }

    /// USaveGameDevice::LoadFromCheckpoint: Browse done (map loaded, InitGame,
    /// BeginPlay, PlayMusicInit), the UAudioSaver flag drops.
    pub fn checkpoint_browse_done(&mut self) {
        self.engine.audio_saver.flag = 0;
    }

    /// AActor::execPlayMusic: name of the music kept by UAudioSaver
    /// ("Package.Group.", the sound's name and the parameters only if the flag is set).
    pub(crate) fn audio_saver_play_music(&mut self, sound: &str, params: [i32; 5]) {
        let parts: Vec<&str> = sound.split('.').collect();
        let s = &mut self.engine.audio_saver;
        if parts.len() >= 2 {
            let outer = parts[parts.len() - 2];
            if parts.len() >= 3 {
                s.name = format!("{}.", parts[parts.len() - 3]);
            }
            s.name.push_str(outer);
            s.name.push('.');
        }
        if s.flag != 0 {
            s.name.push_str(parts.last().copied().unwrap_or(""));
            s.params = params;
        }
    }
}

/// appSystemTime: local (year, month, day, hour, minute).
fn local_time() -> (i32, u8, u8, u8, u8) {
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    (t.year(), t.month() as u8, t.day() as u8, t.hour() as u8, t.minute() as u8)
}

#[allow(dead_code)]
fn utc_time() -> (i32, u8, u8, u8, u8) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    // Universal time (no time zone without a dependency): days since 1970.
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, m) = ((rem / 3600) as u8, ((rem % 3600) / 60) as u8);
    // H. Hinnant's civil-from-days algorithm.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if mo <= 2 { y + 1 } else { y };
    (y as i32, mo, d, h, m)
}

#[cfg(test)]
mod slot_input_tests {
    use super::*;

    #[test]
    fn failed_profile_read_leaves_the_next_operation_queued() {
        let mut vm = Vm::new();
        let mut device = SaveDevice::default();
        device.add_operation(OP_READ_USER_CONFIG, 0);
        device.add_operation(OP_CHECK_CARD, 0);
        vm.engine.save = Some(device);
        vm.op_read_user_config();
        let device = vm.dev();
        assert_eq!(device.ret[OP_READ_USER_CONFIG], -1);
        assert_eq!(device.current(), OP_CHECK_CARD);
        assert!(!device.finished[OP_CHECK_CARD]);
    }

    #[test]
    fn truncated_slot_keeps_the_previous_checkpoint() {
        let mut device = SaveDevice::default();
        let mut data = vec![1, 3];
        put_string(&mut data, "checkpoint");
        data.extend_from_slice(&2003i32.to_le_bytes());
        data.extend_from_slice(&[10, 23, 12, 7]);
        put_bytes(&mut data, &[3, 1, 4]);
        for end in 0..data.len() {
            assert!(device.restore_slot_data(&data[..end]).is_none());
            assert!(device.start_desc.is_empty() && device.checkpoint_desc.is_empty());
            assert!(device.start.is_empty() && device.checkpoint.is_empty());
            assert_eq!(device.date, (0, [0; 4]));
        }
        assert!(device.restore_slot_data(&data).is_some());
        assert_eq!(device.start_desc, "checkpoint");
        assert_eq!(device.checkpoint_desc, device.start_desc);
        assert_eq!(device.checkpoint, [3, 1, 4]);
        assert_eq!(device.start, device.checkpoint);
        assert_eq!(device.date, (2003, [10, 23, 12, 7]));

        // Version 2 predates the timestamp. A valid empty checkpoint is still allowed.
        assert!(device.restore_slot_data(&[1, 2, 0, 0]).is_some());
        assert_eq!(device.date, (2003, [10, 23, 12, 7]));
        assert!(device.checkpoint.is_empty());
        assert!(device.restore_slot_data(&[1, 2, 0, 0x81]).is_none());
    }
}
