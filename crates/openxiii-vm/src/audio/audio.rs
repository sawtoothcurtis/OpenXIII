//! Link between the virtual machine and the HX audio subsystem (openxiii-hx): Engine.dll's sound
//! natives call UHXAUDIOSubsystem with the actor, the Sound object and their
//! parameters; in return the subsystem reads the actors' properties (position, velocity,
//! attenuation) and notifies the script (OnoStartNotif, VoiceEndNotif…).

use crate::canvas::AudioCall;
use crate::types::Type;
use crate::value::Value;
use crate::{ObjId, Vm};
use openxiii_assets::hx::Cuuid;
use openxiii_hx::subsystem::{ActorState, HxAudio, Notify, OnoSet, NPCStep, ProbabilitySound, SoundRef, World, XiiiStep};
use openxiii_package::ObjectRef;

/// The game world as the subsystem sees it.
struct VmWorld<'a> {
    vm: &'a mut Vm,
}

impl World for VmWorld<'_> {
    fn actor(&mut self, id: u32) -> Option<ActorState> {
        self.vm.audio_actor_state(id as ObjId)
    }
    fn find_actor(&mut self, name: &str) -> Option<u32> {
        self.vm.actors.iter().copied().find(|&a| self.vm.objects[a].as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(name))).map(|a| a as u32)
    }
    fn sound_by_cuuid(&mut self, id: Cuuid) -> Option<SoundRef> {
        self.vm.audio_sound_by_cuuid(id)
    }
    fn set_left_foot(&mut self, id: u32, left: bool) {
        self.vm.set_prop(id as ObjId, "bLeftFoot", Value::Bool(left));
    }
    fn music_var(&mut self, index: usize) -> Option<(String, i32, u8)> {
        self.vm.music_vars().get(index).cloned()
    }
}

/// Lazy traversal of `Vm::order_sounds` by `audio_sound_by_cuuid`: the first `read_count` paths
/// already read (kept in `audio_sounds`), and for each CUUID seen, the rank of its first Sound.
#[derive(Debug, Default, Clone)]
pub struct IndexSounds {
    pub(crate) order_len: usize,
    pub(crate) read_count: usize,
    pub(crate) first_list: std::collections::HashMap<Cuuid, usize>,
}

impl Vm {
    /// LevelInfo.MusicVars: (name, value, type).
    pub fn music_vars(&self) -> Vec<(String, i32, u8)> {
        let Some(level) = self.level else { return Vec::new() };
        match self.prop(level, "MusicVars") {
            Value::Array(items) => items
                .iter()
                .map(|item| match item {
                    Value::Struct(_, f) => {
                        (f.first().map(|x| x.as_name()).unwrap_or_default(), f.get(1).map_or(0, |x| x.as_int()), f.get(2).map_or(0, |x| x.as_byte()))
                    }
                    _ => (String::new(), 0, 0),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn set_music_var_value(&mut self, index: usize, value: i32) {
        let Some(level) = self.level else { return };
        if let Value::Array(mut items) = self.prop(level, "MusicVars") {
            if let Some(Value::Struct(_, f)) = items.get_mut(index) {
                if f.len() > 1 {
                    f[1] = Value::Int(value);
                }
            }
            self.set_prop(level, "MusicVars", Value::Array(items));
        }
    }

    /// State of an actor for HXAudio.
    pub fn audio_actor_state(&mut self, a: ObjId) -> Option<ActorState> {
        let o = self.objects.get(a)?.as_ref()?;
        let name = o.name.clone();
        let f = |vm: &Vm, n: &str| vm.propr(a, n).as_float();
        let b = |vm: &Vm, n: &str| vm.propr(a, n).as_bool();
        let ambient = match self.prop(a, "AmbientSound") {
            Value::Asset(p) => self.audio_sound_ref(&p),
            _ => None,
        };
        // Mover.SoundLocationOffset (0x254), read by HXAudio for an AMover only
        // (CanHearSound, Update).
        let mover_offset = self.is_a(a, "Mover").then(|| self.propr(a, "SoundLocationOffset").as_vector());
        Some(ActorState {
            id: a as u32,
            name,
            location: self.propr(a, "Location").as_vector(),
            velocity: self.propr(a, "Velocity").as_vector(),
            has_position: b(self, "bHasPosition"),
            has_rolloff: b(self, "bHasRollOff"),
            underwater: b(self, "bUnderwater"),
            saturation_distance: f(self, "SaturationDistance"),
            stabilisation_distance: f(self, "StabilisationDistance"),
            stabilisation_volume: f(self, "StabilisationVolume"),
            voices_saturation_distance: f(self, "VoicesSaturationDistance"),
            voices_stabilisation_distance: f(self, "VoicesStabilisationDistance"),
            voices_stabilisation_volume: f(self, "VoicesStabilisationVolume"),
            pan_coeff: f(self, "PanCoeff"),
            mover_offset,
            left_foot: b(self, "bLeftFoot"),
            ambient_sound: ambient,
        })
    }

    /// Sound object "Package.Group.Name": CUUID of its HX resource and nature (USound::PostLoad: UuidToQword, handler if "__" after the first character).
    pub fn audio_sound_ref(&mut self, path: &str) -> Option<SoundRef> {
        // Mods build only: a sound of the game replaced on this map (`[map.<Map>.replace_sounds]`).
        #[cfg(feature = "mods")]
        if let Some(to) = self.mods.sound_swaps.get(&path.to_ascii_lowercase()).cloned() {
            return self.audio_sound_ref_named(&to);
        }
        self.audio_sound_ref_named(path)
    }

    /// `audio_sound_ref` without the mods' replacements.
    fn audio_sound_ref_named(&mut self, path: &str) -> Option<SoundRef> {
        if let Some(r) = crate::improved::presentation::menu_sound(self, path) {
            return r;
        }
        // Mods build only: a mod's own sound "Mod.<Name>" (mod_own_sounds.rs).
        #[cfg(feature = "mods")]
        if let Some(s) = self.mod_sound_ref(path) {
            return Some(s);
        }
        if let Some(s) = self.audio_sounds.get(path) {
            return s.clone();
        }
        let r = (|| {
            let library = self.library.as_mut()?;
            let (loaded, index) = library.resolve(path)?;
            if loaded.package.export_class(index) != "Sound" {
                return None;
            }
            let (_, id) = openxiii_assets::hx::sound_reference(&loaded.package, &loaded.data, index).ok()?;
            let name = path.rsplit('.').next().unwrap_or(path).to_string();
            let handler = sound_handler(&name);
            Some(SoundRef { path: path.to_string(), name, cuuid: id?, handler })
        })();
        self.audio_sounds.insert(path.to_string(), r.clone());
        r
    }

    /// GetSound / GetHandlerSounds: the first Sound with this CUUID in the subsystem's array, in GObjObjects index order
    /// (FillSoundsArray, records of `order_original.rs`); without a record, among the sounds already seen, in alphabetical order of the paths.
    fn audio_sound_by_cuuid(&mut self, id: Cuuid) -> Option<SoundRef> {
        // First Sound with this CUUID in `order_sounds`. The traversal stays lazy and in order:
        // each path is read (`audio_sound_ref`, which keeps it in `audio_sounds`) in the same call
        // as before; only the part already read is no longer read again or copied at each call (a missing
        // CUUID used to cost a full traversal with two string copies per sound).
        let found = self.index_sounds_read(Some(id));
        let r = match found {
            Some(i) => self.audio_sounds.get(&self.order_sounds[i]).cloned().flatten(),
            None => self.audio_sounds.values().flatten().filter(|s| s.cuuid == id).min_by(|a, b| a.path.cmp(&b.path)).cloned(),
        };
        r
    }

    /// Rank of the first Sound of `target_value` in `order_sounds`, reading the rest of the list in
    /// order up to it; without a target, the whole list is read.
    fn index_sounds_read(&mut self, target_value: Option<Cuuid>) -> Option<usize> {
        if self.index_sounds.order_len != self.order_sounds.len() {
            self.index_sounds = IndexSounds { order_len: self.order_sounds.len(), ..Default::default() };
        }
        if let Some(&i) = target_value.and_then(|id| self.index_sounds.first_list.get(&id)) {
            return Some(i);
        }
        while self.index_sounds.read_count < self.order_sounds.len() {
            let i = self.index_sounds.read_count;
            self.index_sounds.read_count += 1;
            let path_buf = std::mem::take(&mut self.order_sounds[i]);
            let s = self.audio_sound_ref(&path_buf);
            self.order_sounds[i] = path_buf;
            if let Some(s) = s {
                self.index_sounds.first_list.entry(s.cuuid).or_insert(i);
                if Some(s.cuuid) == target_value {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Start of the level: the whole list of the map's Sounds read in advance (what the first
    /// GetSound of a missing CUUID does). Reading a Sound only depends on the game's files
    /// (`audio_sound_ref`, library cache): the lookups return the same sounds.
    pub fn audio_read_order_sounds(&mut self) {
        self.index_sounds_read(None);
    }

    /// Raw property (array, structure) of a package object, and its package for the references.
    fn audio_raw(&mut self, path: &str, prop: &str) -> Option<(Vec<u8>, String, Vec<String>)> {
        let library = self.library.as_mut()?;
        let (loaded, index) = library.resolve(path)?;
        let data = loaded.package.object_data(&loaded.data, index).ok()?;
        let raw = match data.get(&loaded.package, prop)? {
            openxiii_package::Value::Raw(b) => b.clone(),
            _ => return None,
        };
        let _ = &raw;
        // Paths of all the package's objects, to translate the compact indices.
        let pkg = loaded.name.clone();
        let paths: Vec<String> = Vec::new();
        Some((raw, pkg, paths))
    }

    /// Class-specific data (after the properties) of a sound object.
    fn audio_native(&mut self, path: &str) -> Option<Vec<u8>> {
        let library = self.library.as_mut()?;
        let (loaded, index) = library.resolve(path)?;
        let data = loaded.package.object_data(&loaded.data, index).ok()?;
        let e = &loaded.package.exports[index];
        let end = e.serial_offset as usize + e.serial_size as usize;
        loaded.data.get(data.end..end).map(|b| b.to_vec())
    }

    /// Compact object index → path, in the package that contains `owner`.
    fn audio_object_path(&mut self, owner: &str, index: i32) -> Option<String> {
        let library = self.library.as_mut()?;
        let (loaded, _) = library.resolve(owner)?;
        match ObjectRef::from(index) {
            ObjectRef::None => None,
            ObjectRef::Export(_) => Some(format!("{}.{}", loaded.name, loaded.package.object_path(index.into()))),
            _ => Some(loaded.package.object_path(index.into())),
        }
    }

    fn audio_sound_prop(&mut self, owner: &str, prop: &str) -> Option<SoundRef> {
        match self.asset_prop(owner, prop)? {
            Value::Asset(p) => self.audio_sound_ref(&p),
            _ => None,
        }
    }

    /// Array of ProbaSound {Proba: int, Sound: object} (serialization order).
    fn audio_probability_array(&mut self, owner: &str, prop: &str) -> Vec<ProbabilitySound> {
        let Some((raw, _, _)) = self.audio_raw(owner, prop) else {
            return Vec::new();
        };
        let mut r = openxiii_package::Reader::new(&raw, 0);
        let Ok(count) = r.compact() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for _ in 0..count.max(0) {
            let (Ok(probability), Ok(obj)) = (r.u32(), r.compact()) else {
                break;
            };
            let sound = self.audio_object_path(owner, obj).and_then(|p| self.audio_sound_ref(&p));
            out.push(ProbabilitySound { sound, probability: probability as i32 });
        }
        out
    }

    fn audio_sound_array(&mut self, owner: &str, prop: &str) -> Vec<Option<SoundRef>> {
        let Some((raw, _, _)) = self.audio_raw(owner, prop) else {
            return Vec::new();
        };
        let mut r = openxiii_package::Reader::new(&raw, 0);
        let Ok(count) = r.compact() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for _ in 0..count.max(0) {
            let Ok(obj) = r.compact() else { break };
            out.push(self.audio_object_path(owner, obj).and_then(|p| self.audio_sound_ref(&p)));
        }
        out
    }

    /// USndXIIIStep::Serialize (after USndStep::Serialize): EndStep,
    /// Stumble, LeftSteps, RightSteps, OnosLand, OnosRun and their sums (LicenseeVer ≥ 0x28).
    fn audio_xiii_step(&mut self, path: &str) -> Option<XiiiStep> {
        let f = |vm: &mut Vm, n: &str| vm.asset_prop(path, n).map_or(0.0, |v| v.as_float());
        let i = |vm: &mut Vm, n: &str| vm.asset_prop(path, n).map_or(0, |v| v.as_int());
        let stumble = {
            let raw = self.audio_raw(path, "Stumble");
            match raw {
                Some((raw, _, _)) => {
                    let mut r = openxiii_package::Reader::new(&raw, 0);
                    match (r.u32(), r.compact()) {
                        (Ok(p), Ok(o)) => ProbabilitySound { probability: p as i32, sound: self.audio_object_path(path, o).and_then(|x| self.audio_sound_ref(&x)) },
                        _ => ProbabilitySound { probability: 0, sound: None },
                    }
                }
                None => ProbabilitySound { probability: 0, sound: None },
            }
        };
        Some(XiiiStep {
            land_sound: self.audio_sound_prop(path, "LandSound"),
            pitch_min: f(self, "PitchMin"),
            pitch_max: f(self, "PitchMax"),
            end_step: self.audio_sound_prop(path, "EndStep"),
            stumble,
            left_steps: self.audio_probability_array(path, "LeftSteps"),
            right_steps: self.audio_probability_array(path, "RightSteps"),
            onos_land: self.audio_probability_array(path, "OnosLand"),
            onos_land_probability_sum: i(self, "OnosLandProbaSum"),
            onos_run: self.audio_probability_array(path, "OnosRun"),
            onos_run_probability_sum: i(self, "OnosRunProbaSum"),
            silent_volume: f(self, "SilentVolume"),
            walk_volume: f(self, "WalkVolume"),
        })
    }

    /// USndPNJStep::Serialize: LeftSteps, RightSteps.
    fn audio_npc_step(&mut self, path: &str) -> Option<NPCStep> {
        Some(NPCStep {
            land_sound: self.audio_sound_prop(path, "LandSound"),
            pitch_min: self.asset_prop(path, "PitchMin").map_or(0.0, |v| v.as_float()),
            pitch_max: self.asset_prop(path, "PitchMax").map_or(0.0, |v| v.as_float()),
            left_steps: self.audio_sound_array(path, "LeftSteps"),
            right_steps: self.audio_sound_array(path, "RightSteps"),
        })
    }

    /// UPNJOno / UDeathOno: array of FTimbre (three groups of four sounds) read by
    /// USndOno::Serialize (operator<<(FTimbre): the
    /// twelve references in order), after the properties. The packages' CodeMesh property
    /// keeps a copy of it in which each FTimbre is reversed (hPNJDetect: mesh 0's shout is
    /// at rank 11 there); the engine replaces it with this data.
    pub(crate) fn audio_ono(&mut self, path: &str) -> Option<OnoSet> {
        let class = self.library.as_mut().and_then(|library| library.resolve(path))
            .map(|(loaded, index)| loaded.package.export_class(index).to_string());
        let is_npc = class.as_deref().is_some_and(|name| {
            name.eq_ignore_ascii_case("PNJOno") || self.class(name).zip(self.class("PNJOno"))
                .is_some_and(|(child, parent)| self.is_child_of(child, parent))
        });
        let mut probability = self.asset_prop(path, "Proba").map_or(100, |v| v.as_int());
        let mut timbres = Vec::new();
        if let Some(raw) = self.audio_native(path) {
            let mut r = openxiii_package::Reader::new(&raw, 0);
            if let Ok(count) = r.compact() {
                for _ in 0..count.max(0) {
                    let mut t: [Option<SoundRef>; 12] = Default::default();
                    for slot in t.iter_mut() {
                        let Ok(o) = r.compact() else { break };
                        *slot = self.audio_object_path(path, o).and_then(|p| self.audio_sound_ref(&p));
                    }
                    timbres.push(t);
                }
                // UPNJOno::Serialize replaces the property with
                // the four native bytes after the timbres. DeathOno does not read it.
                if is_npc {
                    if let Ok(native_probability) = r.i32() { probability = native_probability; }
                }
            }
        }
        Some(OnoSet { timbres, probability })
    }

    /// Runs a sound call from the script (like Engine.dll's natives, synchronously).
    /// Call to the audio subsystem; the result is only used by PlayStrVoice (false without output).
    pub fn audio_dispatch(&mut self, call: AudioCall) -> bool {
        // Local co-op (coop.rs): the intro's sounds are played once for the team.
        let call = if self.coop.on {
            match self.coop_intro_sound(call) {
                Some(call) => call,
                None => return true,
            }
        } else {
            call
        };
        let what = String::new();
        let Some(mut hx) = self.audio.take() else {
            let str_voice = matches!(call, AudioCall::StrVoice { .. });
            self.sounds.push(call);
            return !str_voice;
        };
        // msvcr70's rand(): a single sequence per thread, shared by appRand (Core) and HXAudio.
        hx.engine.rand_main.seed = self.seed;
        let mut result = true;
        match call {
            AudioCall::Play { actor, rolloff, sound, slot, params, check_handler, first_free } => {
                if let Some(s) = self.audio_sound_ref(&sound) {
                    let a = actor.and_then(|x| self.audio_actor_state(x));
                    let b = rolloff.and_then(|x| self.audio_actor_state(x));
                    if (first_free && params.first() == Some(&-1)) || params.is_empty() || (check_handler && !s.handler) {
                        hx.play_sound(a.as_ref(), b.as_ref(), &s, slot);
                    } else {
                        let list: Vec<i32> = params.iter().copied().take_while(|&p| p != -1).collect();
                        hx.play_sound_params(a.as_ref(), b.as_ref(), &s, slot, &list);
                    }
                }
            }
            AudioCall::Stop { actor, sound, kinds, notify } => {
                let s = sound.and_then(|p| self.audio_sound_ref(&p));
                hx.stop_sound(actor.map(|a| a as u32), s.as_ref(), kinds, notify);
            }
            AudioCall::XiiiStep { actor, step, speed, end_step, silent } => {
                if let (Some(a), Some(st)) = (self.audio_actor_state(actor), self.audio_xiii_step(&step)) {
                    let mut world = VmWorld { vm: self };
                    hx.play_snd_xiii_step(&mut world, &a, &st, speed, end_step, silent);
                }
            }
            AudioCall::NPCStep { actor, step, speed, category, silent } => {
                if let (Some(a), Some(st)) = (self.audio_actor_state(actor), self.audio_npc_step(&step)) {
                    let mut world = VmWorld { vm: self };
                    hx.play_snd_npc_step(&mut world, &a, &st, speed, category, silent);
                }
            }
            AudioCall::Ono { actor, ono, code_mesh, timbre, death } => {
                if let (Some(a), Some(o)) = (self.audio_actor_state(actor), self.audio_ono(&ono)) {
                    hx.play_snd_ono(&a, &o, code_mesh, timbre, death);
                }
            }
            AudioCall::Pause { kinds } => hx.pause_sound(kinds, false),
            AudioCall::Resume { kinds } => hx.resume_sound(kinds, false),
            AudioCall::MusicPosition { pos } => hx.set_music_position(pos),
            // AActor::execSetVolume: SetVolumeLine(0xFFF, volume).
            AudioCall::Volume { kinds, volume } => hx.set_volume_line_kinds(kinds, volume),
            AudioCall::MusicCounter { index, delta, name, ty, value, alert } => {
                // ALevelInfo::execIncAttente / execDecAttaque…: the value has already been changed (natives.rs).
                // The diagnostic reads the LevelInfo's current descriptions, even after they are changed.
                if hx.log_ono {
                    hx.music_debug_vars = self.music_vars().into_iter().take(3).collect();
                }
                hx.set_music_var(&name, value, ty);
                if hx.log_ono {
                    // The native getters rewrite Value in the LevelInfo's three FMusicVars.
                    for (index, (_, value, _)) in hx.music_debug_vars.iter().enumerate() {
                        self.set_music_var_value(index, *value);
                    }
                }
                let effect_value = effect_counter(index, delta, value, alert, hx.attack_wait);
                if let Some(state_value) = effect_value.call_state {
                    hx.call_music_state(state_value, Some((name, ty)));
                }
                if effect_value.wait_value {
                    hx.attack_wait = true;
                    {
                        hx.attack_wait_start = hx.time;
                    }
                }
            }
            AudioCall::StrVoice { actor, name, rolloff } => {
                // AActor::execPlayStrVoice: the speaking actor defaults to the caller.
                let speaker = self.audio_actor_state(rolloff.unwrap_or(actor));
                result = hx.play_str_voice(actor as u32, speaker.as_ref(), &name);
            }
        }
        let notes = std::mem::take(&mut hx.notifications);
        self.seed_hx(hx.engine.rand_main.seed, &what);
        self.audio = Some(hx);
        self.audio_notify(notes);
        result
    }

    /// The actor and the script event of each notification of the audio subsystem.
    /// AActor::OnoStartNotif: eventOnoStartNotif.
    /// AActor::VoiceEndNotif / VoicePauseNotif / VoiceUnPauseNotif call eventEndOfVoice, eventPauseVoice and eventUnPauseVoice; BeginBeginFlashNotif,
    /// EndBeginFlashNotif, BeginEndFlashNotif, EndEndFlashNotif the event with the same name without "Notif".
    pub(crate) fn notify_event(n: &Notify) -> (u32, &'static str) {
        match *n {
            Notify::OnoStart(a) => (a, "OnoStartNotif"),
            Notify::VoiceEnd(a) => (a, "EndOfVoice"),
            Notify::VoicePause(a) => (a, "PauseVoice"),
            Notify::VoiceUnPause(a) => (a, "UnPauseVoice"),
            Notify::BeginBeginFlash(a) => (a, "BeginBeginFlash"),
            Notify::EndBeginFlash(a) => (a, "EndBeginFlash"),
            Notify::BeginEndFlash(a) => (a, "BeginEndFlash"),
            Notify::EndEndFlash(a) => (a, "EndEndFlash"),
        }
    }

    pub(crate) fn audio_notify(&mut self, notes: Vec<Notify>) {
        for n in notes {
            let (who, event) = Self::notify_event(&n);
            let who = who as ObjId;
            if who < self.objects.len() && self.objects[who].is_some() {
                self.event_pub(who, event, vec![]);
            }
        }
    }

    /// UGameEngine::LoadMap after the players: Audio->CleanUp (the
    /// old map's bank is unloaded), then SetViewport for
    /// each viewport of the client: one microphone each, bHasRollOff and bHasPosition lowered on
    /// Viewport->Actor and its Pawn.
    pub fn audio_set_viewports(&mut self) {
        if self.audio.is_none() {
            return;
        }
        if let Some(hx) = self.audio.as_mut() {
            hx.engine.rand_main.seed = self.seed;
            hx.clean_up();
            let seed = hx.engine.rand_main.seed;
            self.seed_hx(seed, "audio.rs:406");
        }
        self.audio_viewports();
    }

    /// Audio->SetViewport of each viewport of the client (LoadMap, Init).
    fn audio_viewports(&mut self) {
        for (i, v) in self.active_viewport_objects().into_iter().enumerate() {
            if let Some(hx) = self.audio.as_mut() {
                hx.set_viewport(i);
            }
            let Some(pc) = self.propr(v, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { continue };
            for a in [Some(pc), self.propr(pc, "Pawn").as_object()].into_iter().flatten() {
                self.set_prop(a, "bHasRollOff", Value::Bool(false));
                self.set_prop(a, "bHasPosition", Value::Bool(false));
            }
        }
    }

    /// HXAudio global variables (music variables: Music.Flash…), which carry over from one map to the next:
    /// (name, value) of each one.
    pub fn audio_globals(&self) -> Vec<(String, u32)> {
        let Some(hx) = self.audio.as_ref() else { return Vec::new() };
        hx.engine.globals.iter().map(|(k, &v)| (String::from_utf8_lossy(k).into_owned(), hx.engine.var_u32(v))).collect()
    }

    /// Entry of UGameEngine::LoadMap: Audio->CleanUp when Audio exists, before destroying
    /// the viewports' actors and spawning the players. Without a microphone until the SetViewport at the end of LoadMap,
    /// the weapon selection sound that AcceptInventory plays (Weapon.BringUp → PlaySelect) does not come out and creates no
    /// HX actor.
    pub fn audio_start_load_map(&mut self) {
        self.audio_clean_up();
    }

    /// AActor::execKillAllSounds → CleanUp: general pause, microphones
    /// destroyed (no more Update until the next SetViewport), map bank unloaded.
    pub fn audio_clean_up(&mut self) {
        // Mods build only: the banks loaded for the mods leave with the map's (mod_sounds.rs).
        #[cfg(feature = "mods")]
        self.mod_unload_banks();
        if let Some(hx) = self.audio.as_mut() {
            hx.engine.rand_main.seed = self.seed;
            hx.clean_up();
            let seed = hx.engine.rand_main.seed;
            self.seed_hx(seed, "audio.rs:431");
        }
    }

    /// Split-screen microphones (UGameEngine::Tick, before GLevel->Tick):
    /// for each viewport of the client, an FCameraSceneNode on Viewport->Actor
    /// (Location, Rotation, FovAngle, without PlayerCalcView), then Audio->UpdateMicro(Viewport,
    /// Frame) unless the controller's state is called "CameraView".
    /// Size of each viewport: its rectangle in the window of the last Draw.
    pub fn audio_listeners_from_viewports(&mut self) {
        let client = self.client_size;
        if self.audio.is_none() {
            return;
        }
        let slots = self.engine.client.viewports.clone();
        // Co-op with one picture shown (a cinematic in full screen): only that player is heard.
        // Every microphone listens from the shown viewport, so that the hidden players' places
        // (a parked player left on Plage00's boat, the other player at SPADS) no longer keep
        // sounds loud or lean them to one side. The original's split screen (menus) is untouched.
        let shown = if self.coop.on && slots.len() > 1 { self.engine.client.extended.filter(|s| slots.contains(s)) } else { None };
        for (i, slot) in slots.into_iter().enumerate() {
            let slot = shown.unwrap_or(slot);
            let Some(v) = self.viewport_object(slot) else { continue };
            let Some(pc) = self.propr(v, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) else { continue };
            let rect = self.engine.client.viewport_rect(slot, client);
            let size = (rect[2].max(1) as u32, rect[3].max(1) as u32);
            let mut location = self.propr(pc, "Location").as_vector();
            let mut rotation = self.propr(pc, "Rotation").as_rotator();
            if let Some(camera) = crate::improved::presentation::character_camera(self, slot) {
                (location, rotation) = camera;
            }
            let fov = self.propr(pc, "FovAngle").as_float();
            let node = self.camera_scene(Some(pc), Some(pc), location, rotation, fov, size);
            let state = self.state_chain_names(pc).into_iter().next().unwrap_or_default();
            if state.eq_ignore_ascii_case("CameraView") {
                continue;
            }
            self.audio_update_micro(i, &node);
        }
    }

    /// UHXAUDIOSubsystem::UpdateMicro, listener frame: P0, P1, P2 =
    /// Deproject of (0, 0, 0, 1), (0, −1, 0, 1) and (1, 0, 0, 1); U = P1 − P0, W = P2 − P0,
    /// aim = W × U, U and aim normalized (nothing below
    /// 1e-8, otherwise × appRSqrt), up = −U.
    fn audio_update_micro(&mut self, view: usize, node: &crate::visibility::SceneNode) {
        let p0 = node.deproject_plane([0.0, 0.0, 0.0, 1.0]);
        let p1 = node.deproject_plane([0.0, -1.0, 0.0, 1.0]);
        let p2 = node.deproject_plane([1.0, 0.0, 0.0, 1.0]);
        // Frame computed as the machine code does (openxiii_hx::listener::frame_micro, tested).
        let (aim, high) = openxiii_hx::listener::frame_micro(p0, p1, p2);
        if let Some(hx) = self.audio.as_mut() {
            hx.update_micro(view, node.origin, aim, high);
        }
    }

    /// HXAudio clock (seconds) and forcing for the next update.
    pub fn audio_time(&self) -> Option<f64> {
        self.audio.as_ref().map(|hx| hx.time)
    }

    pub fn audio_set_time(&mut self, t: f64) {
        if let Some(hx) = self.audio.as_mut() {
            hx.time_impose = Some(t);
        }
    }

    /// In game, real clock: HX timer creations are dated at the moment of the call.
    pub fn audio_set_real_time(&mut self, yes: bool) {
        if let Some(hx) = self.audio.as_mut() {
            hx.real_time = yes;
        }
    }

    /// Sound update every frame (UHXAUDIOSubsystem::Update).
    pub fn audio_tick(&mut self, dt: f32) {
        // Calls left pending for lack of a subsystem.
        let Some(mut hx) = self.audio.take() else {
            return;
        };
        // The Update notifications (pause, resume and end of the HF voice, Flash) are delivered
        // at their place, with the subsystem put back: a sound started by the event goes out in the same
        // update, like HXAudio's direct calls (see update_stage). msvcr70's rand():
        // a single sequence per thread, shared by appRand (Core) and HXAudio.
        let mut stage = 0;
        loop {
            hx.engine.rand_main.seed = self.seed;
            let more = {
                let mut world = VmWorld { vm: self };
                hx.update_stage(&mut world, dt as f64, &mut stage)
            };
            let notes = std::mem::take(&mut hx.notifications);
            self.seed_hx(hx.engine.rand_main.seed, "audio.rs:539");
            self.audio = Some(hx);
            self.audio_notify(notes);
            if !more {
                break;
            }
            let Some(h) = self.audio.take() else { return };
            hx = h;
        }
    }

    pub fn audio_set_ds8_phase(&mut self, phase: f64) {
        if let Some(hx) = self.audio.as_mut() {
            hx.engine.ds8_clock = phase;
        }
    }

    /// HXAudio clock (timeGetTime, in seconds since the start of the map): the one seen by
    /// the code called during the level tick (timers created by the script's sounds).
    pub fn audio_set_clock(&mut self, t: f64) {
        if let Some(hx) = self.audio.as_mut() {
            hx.time = t;
            hx.engine.time_ms = (t * 1000.0 + 1e-6) as u32;
        }
    }

    /// Start of a map (LoadSounds + ParseAmbientActors, then PlayMusicInit). LoadSounds is already
    /// done when LoadMap has called it (later maps, `audio_load_sounds`).
    pub fn audio_start_level(&mut self) {
        if !self.audio_sounds_loaded {
            self.audio_load_sounds();
        }
        self.audio_sounds_loaded = false;
        let Some(mut hx) = self.audio.take() else {
            return;
        };
        hx.engine.rand_main.seed = self.seed;
        let title = self.level.map(|l| self.propr(l, "Title").as_name()).unwrap_or_default();
        let init_music = self.level.and_then(|l| match self.prop(l, "InitMusic") {
            Value::Asset(p) => self.audio_sound_ref(&p),
            _ => None,
        });
        // UGameEngine::Browse: PlayMusicInit right after the map is loaded.
        hx.play_music_init(&title, init_music);
        self.seed_hx(hx.engine.rand_main.seed, "audio.rs:588");
        self.audio = Some(hx);
    }

    /// Audio->LoadSounds (vtable +0xE8): called by UGameEngine::LoadMap
    /// before "End LoadMap" when Audio exists (later maps), otherwise by Init
    /// after the first map. Ends with ParseAmbientActors.
    pub fn audio_load_sounds(&mut self) {
        let Some(mut hx) = self.audio.take() else {
            return;
        };
        self.audio_sounds_loaded = true;
        // msvcr70's rand(): a single sequence per thread, shared by appRand (Core) and HXAudio.
        hx.engine.rand_main.seed = self.seed;
        let level = self.level;
        let init_music = level.and_then(|l| match self.prop(l, "InitMusic") {
            Value::Asset(p) => self.audio_sound_ref(&p),
            _ => None,
        });
        // LoadSounds: music counters reset to zero, HX variables created; the
        // start handler is called "<SoundProject>.Music__<Title>.<Title>__hMusicInit"
        // (Hual02 uses Hualpar2's).
        let title = level.map(|l| self.propr(l, "Title").as_name()).unwrap_or_default();
        let vars: Vec<(String, i32, u8)> = self.music_vars().into_iter().map(|(n, _, t)| (n, 0, t)).collect();
        for i in 0..vars.len() {
            self.set_music_var_value(i, 0);
        }
        let fallback = openxiii_hx::subsystem::music_init_name(&title, &hx.config.sound_project);
        let music_init = self.audio_sound_ref(&fallback);
        let flash = level.and_then(|l| self.propr(l, "FlashManager").as_object()).map(|a| a as u32);
        hx.load_sounds(&title, &vars, init_music, music_init, flash);
        // End of LoadMapSounds: physical memory taken by the map's bank,
        // difference of two GlobalMemoryStatus calls (dwTotalPhys − dwAvailPhys) in KB. For a
        // process without LARGEADDRESSAWARE, Windows caps these fields at 2 GB as soon as the machine
        // has more memory: the difference is 0 (the original's logs always show "0 Kb").
        self.log.push(format!("Log: SOUND : mem used for map {title} = 0 Kb"));
        let actors: Vec<ActorState> = self.actors.clone().into_iter().filter_map(|a| self.audio_actor_state(a)).collect();
        hx.parse_ambient_actors(&actors);
        self.seed_hx(hx.engine.rand_main.seed, "audio.rs:637");
        self.audio = Some(hx);
        self.audio_read_order_sounds();
        // Mods build only: the sound banks the mods need on this level (mod_sounds.rs).
        #[cfg(feature = "mods")]
        self.mod_level_sounds();
    }

    /// UObject::LoadConfig of the default values of UHXAUDIOSubsystem, called by UClass::Register: starting from
    /// zero, each StaticConstructor property present in [HXAudio.HXAudioSubsystem] of the "User" file is
    /// imported by ImportText (without flags); a missing key leaves the value.
    pub(crate) fn hx_load_config(&mut self) -> openxiii_hx::subsystem::Config {
        use openxiii_hx::subsystem::{FieldConfig, Config, PROPERTIES_CONFIG};
        let mut config = Config::zero();
        for (name_value, _, _, _, _) in PROPERTIES_CONFIG {
            let Some(text_value) = self.config.as_mut().and_then(|c| c.get_string("HXAudio.HXAudioSubsystem", name_value, "User")) else {
                continue;
            };
            let chars: Vec<char> = text_value.chars().collect();
            let (ty, mut v) = match config.field_value(name_value) {
                Some(FieldConfig::BoolVar(b)) => (Type::Bool, Value::Bool(*b)),
                Some(FieldConfig::FloatVar(f)) => (Type::Float, Value::Float(*f)),
                Some(FieldConfig::Integer(i)) => (Type::Int, Value::Int(*i)),
                Some(FieldConfig::StringVar(s)) => (Type::Str, Value::Str(s.clone())),
                None => continue,
            };
            self.import_text(&ty, None, &chars, 0, 0, &mut v);
            match (config.field_value(name_value), v) {
                (Some(FieldConfig::BoolVar(b)), Value::Bool(x)) => *b = x,
                (Some(FieldConfig::FloatVar(f)), Value::Float(x)) => *f = x,
                (Some(FieldConfig::Integer(i)), Value::Int(x)) => *i = x,
                (Some(FieldConfig::StringVar(s)), Value::Str(x)) => *s = x,
                _ => {}
            }
        }
        config
    }

    /// ULevel::DestroyActor → UHXAUDIOSubsystem::NoteDestroy.
    pub fn audio_note_destroy(&mut self, a: ObjId) {
        if let Some(hx) = self.audio.as_mut() {
            hx.note_destroy(a as u32);
        }
    }

    /// HX audio subsystem, created once with the "Fix" project: UGameEngine::Init
    /// calls InitAudio after the first map is loaded (Browse, hence its
    /// PreBeginPlay) and the player of the first viewport, then SetViewport (vtable +0x74) for
    /// each viewport of the client, without CleanUp: LoadMap found Audio null. It survives
    /// travels as in the engine, the bank of each map being loaded by LoadSounds
    /// (`audio_start_level`) and unloaded by CleanUp (`audio_set_viewports`).
    pub fn audio_init(&mut self, game: &std::path::Path, _map: &str) {
        let _math = self.session.math.enter();
        if self.audio.is_some() {
            return;
        }
        // Config properties of UHXAUDIOSubsystem ([HXAudio.HXAudioSubsystem] of User.ini), loaded by UClass::Register.
        let mut config = if self.config.is_some() { self.hx_load_config() } else { openxiii_hx::subsystem::Config::default() };
        // OXIII_SOUND_LANGUAGE: forced SoundLanguage.
        if let Some(v) = crate::env_once!("OXIII_SOUND_LANGUAGE") {
            config.sound_language = v.to_string();
        }
        // The special actors (Music, HFVoice…) that the subsystem creates draw their CUUID from
        // the game thread's sequence: the first one does 's srand.
        let mut engine = openxiii_hx::Engine::new();
        if self.lockstep_on() { engine.stream_follow_rate = crate::lockstep::FOLLOW_RATE; }
        engine.rand_main.seed = self.seed;
        engine.srand_clock = self.hx_srand_clock;
        engine.srand_first_cuuid = self.hx_srand_given;
        let mut hx = HxAudio::new(engine, config);
        // Init: version of the HarmonX library (0x01180002), as
        // NAME_Init (0x2FA).
        self.log.push("Init: HarmonX version : 1.24.2".into());
        hx.init_project(game);
        self.seed_hx(hx.engine.rand_main.seed, "audio.rs:679");
        self.audio = Some(hx);
        self.audio_viewports();
    }
}

/// USound::PostLoad: byte +0x29 (handler) = the name cut at the first "__"
/// (FString::Left of InStr, empty without "__" or if it is at the start) is not empty.
pub(crate) fn sound_handler(name: &str) -> bool {
    name.find("__").is_some_and(|i| i > 0)
}

/// What ALevelInfo::execIncAttaque and execDecAttaque do with the audio subsystem once
/// MusicVars[2] is changed and the counter notified (vtable +0xC4): as long as no wait (+0x2C) is set, an attack
/// counter that goes to 1 calls state 1 (+0xCC); one that drops back to 0 calls state 0 if nobody is alert any more
/// (MusicVars[1] zero), otherwise sets the wait (+0x2C = 1, time at +0x30). The other counters do nothing more.
pub(crate) struct EffectCounter {
    pub call_state: Option<i32>,
    pub wait_value: bool,
}

pub(crate) fn effect_counter(index: usize, delta: i32, value: i32, alert: i32, attack_wait: bool) -> EffectCounter {
    let mut e = EffectCounter { call_state: None, wait_value: false };
    if index == 2 && !attack_wait {
        if delta > 0 && value == 1 {
            e.call_state = Some(1);
        } else if delta < 0 && value == 0 {
            if alert == 0 {
                e.call_state = Some(0);
            } else {
                e.wait_value = true;
            }
        }
    }
    e
}

