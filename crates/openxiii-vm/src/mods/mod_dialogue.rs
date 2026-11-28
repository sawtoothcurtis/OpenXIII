//! Mods build only: lines said by characters, `[[map.<Map>.say]]` (openxiii_mods::dialogue).
//!
//! A line is said as the game's DialogueManager (xidcine) says one: its voice is played by the
//! speaker, and its subtitle goes to the HUD as an XIIIDialogMessage (switch 0, the speaker as
//! OptionalObject), kept on screen for the voice's length plus one second (three seconds when
//! the length is unknown). Lines never speak over each other: a line triggered while another
//! is being said (or while a line of the game's own dialogues is) waits. The player coming
//! near a speaker counts only when the player has control: not during the game's cinematics.
//!
//! A line of the game is named as DialogueManager.Speak names its voice ("Hual02_Carrington_04");
//! its voice is the sound of that name in the level's Voices package (loaded with its bank if
//! needed, as the other sounds of mods), its subtitle the sentence of that speaker and index in
//! the level's localization file, in the player's language.

use std::collections::VecDeque;

use openxiii_mods::dialogue::{Say, When};
use openxiii_mods::Edit;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Class of the hidden actor that hears an event of the map for a line (`when = "event"`): its
/// Trigger only sets bEnabled (LineOfSightTrigger.Trigger), which the line then clears.
const LISTENER_CLASS: &str = "Engine.LineOfSightTrigger";

/// Seconds a subtitle stays when the voice's length is unknown (DialogueManager.Speak: 2 + 1).
const UNKNOWN_LENGTH: f32 = 3.0;

#[derive(Debug, Clone)]
pub(crate) struct LiveSay {
    pub say: Say,
    pub source: String,
    /// Speaker found on the map (checked again when it is gone).
    pub speaker: Option<ObjId>,
    /// `when = "event"`: the actor that hears it.
    pub listener: Option<ObjId>,
    /// Said (or waiting to be said) at least once.
    pub done: bool,
    /// `when = "near"`: the player is within the distance.
    pub inside: bool,
    /// The speaker could not be found: said once in the log.
    pub warned: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModDialogues {
    pub says: Vec<LiveSay>,
    /// Lines triggered and waiting: (index in `says`, the speaker, time it may start).
    pub queue: VecDeque<(usize, ObjId, f32)>,
    /// Level time at which the line being said ends.
    pub busy_until: f32,
}

impl Vm {
    pub(crate) fn mod_level_time(&self) -> f32 {
        self.level.map_or(self.time, |l| self.prop(l, "TimeSeconds").as_float())
    }

    /// The local player's controller: the one of the window, or else the level's first one
    /// (a game without a window).
    pub(crate) fn mod_local_pc(&self) -> Option<ObjId> {
        self.local_viewport()
            .and_then(|v| self.propr(v, "Actor").as_object())
            .or_else(|| self.actors.iter().copied().find(|&a| self.is_a(a, "PlayerController") && self.mod_alive(a)))
    }

    pub(crate) fn mod_player_pawn(&self) -> Option<ObjId> {
        let pc = self.mod_local_pc()?;
        self.native_prop(pc, Field::Pawn).as_object()
    }

    pub(crate) fn mod_alive(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some())
            && !self.native_prop(a, Field::DeleteMe).as_bool()
            && (!self.is_a(a, "Pawn") || self.prop(a, "Health").as_int() > 0)
    }

    /// End of the level start-up (after the mods' additions): the lines of this map.
    pub(crate) fn mod_dialogues_start(&mut self) {
        self.mods.dialogues = ModDialogues::default();
        if self.mod_net_client() {
            return;
        }
        let map = self.mod_map_name();
        let mut says = Vec::new();
        for e in self.mod_edits_here() {
            let Edit::Say(say) = &e.edit else { continue };
            let mut live =
                LiveSay { say: (**say).clone(), source: e.source.to_string(), speaker: None, listener: None, done: false, inside: false, warned: false };
            if let When::Event { name } = &say.when {
                live.listener = self.mod_event_listener(name);
                if live.listener.is_none() {
                    self.mod_report(format!("warning: {map}: {}: event {name} cannot be heard ({})", say.describe(), e.source));
                }
            }
            // The voice's bank is loaded now, with the level, rather than when it is said (by
            // mod_dialogue_sounds when the level's sounds come after this, as they usually do).
            if let Some(path) = self.mod_line_sound(&say).filter(|_| self.audio.is_some()) {
                if let Err(why) = self.mod_ensure_sound(&path, &format!("{map}: {}", say.describe())) {
                    self.mod_report(format!("warning: {map}: {}: {why} ({})", say.describe(), e.source));
                }
            }
            says.push(live);
        }
        self.mods.dialogues.says = says;
    }

    /// When the level's sounds are loaded (after the mods' additions, hence after
    /// mod_dialogues_start, which had no audio engine yet): the banks of the lines' voices.
    pub(crate) fn mod_dialogue_sounds(&mut self) {
        let map = self.mod_map_name();
        let says: Vec<LiveSay> = self.mods.dialogues.says.clone();
        for s in says {
            if let Some(path) = self.mod_line_sound(&s.say) {
                if let Err(why) = self.mod_ensure_sound(&path, &format!("{map}: {}", s.say.describe())) {
                    self.mod_report(format!("warning: {map}: {}: {why} ({})", s.say.describe(), s.source));
                }
            }
        }
    }

    /// A hidden actor tagged `event`: the map's TriggerEvent(event) calls its Trigger.
    pub(crate) fn mod_event_listener(&mut self, event: &str) -> Option<ObjId> {
        let class = self.class(LISTENER_CLASS)?;
        let at = self
            .mod_player_pawn()
            .or_else(|| self.actors.first().copied())
            .map(|a| self.native_value(a, Field::Location))
            .unwrap_or(Value::vector(0.0, 0.0, 0.0));
        let tag = Value::Name(crate::value::NameStruct::from_time(event));
        let a = self.spawn_actor(class, Value::Object(None), Some(tag), at, Value::rotator(0, 0, 0), Value::Object(None), true).ok().flatten()?;
        self.set_prop(a, "bEnabled", Value::Bool(false));
        Some(a)
    }

    /// Who says the line: a mod character of that name on the map, or else a character of the
    /// map whose name, Tag or PawnName is `by`.
    fn mod_find_speaker(&self, by: &str) -> Option<ObjId> {
        if let Some(b) = self.mods.behaviour.as_ref() {
            let mut live: Vec<ObjId> = self
                .mods.live
                .values()
                .filter(|l| b.characters.get(l.def).is_some_and(|d| d.name.eq_ignore_ascii_case(by)))
                .map(|l| l.pawn)
                .filter(|&p| self.mod_alive(p))
                .collect();
            live.sort();
            if let Some(&p) = live.first() {
                return Some(p);
            }
        }
        let matches = |a: ObjId| {
            let o = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            o.eq_ignore_ascii_case(by)
                || self.prop(a, "Tag").as_name().eq_ignore_ascii_case(by)
                || (self.is_a(a, "Pawn") && self.prop(a, "PawnName").as_name().eq_ignore_ascii_case(by))
        };
        // Characters first (a Tag may be shared by several actors).
        let found: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.mod_alive(a) && matches(a)).collect();
        found.iter().copied().find(|&a| self.is_a(a, "Pawn")).or_else(|| found.first().copied())
    }

    /// Each tick: triggers, then the next line waiting if none is being said.
    pub(crate) fn mod_dialogues_tick(&mut self) {
        let now = self.mod_level_time();
        let player = self.mod_player_pawn();
        let map = self.mod_map_name();
        // A cinematic (Level.bCineFrame, the comic frame): the player does not walk anywhere.
        let cinematic = self.level.is_some_and(|l| self.prop(l, "bCineFrame").as_bool());
        let mut d = std::mem::take(&mut self.mods.dialogues);
        for (i, s) in d.says.iter_mut().enumerate() {
            if s.say.once && s.done {
                continue;
            }
            if s.speaker.is_none_or(|a| !self.mod_alive(a)) {
                s.speaker = self.mod_find_speaker(&s.say.by);
            }
            let fire = match &s.say.when {
                When::Start => true,
                When::Near { distance } => {
                    let near = match (player, s.speaker) {
                        (Some(p), Some(a)) if !cinematic => {
                            let [px, py, pz] = self.native_value(p, Field::Location).as_vector();
                            let [x, y, z] = self.native_value(a, Field::Location).as_vector();
                            ((px - x).powi(2) + (py - y).powi(2) + (pz - z).powi(2)).sqrt() <= *distance
                        }
                        _ => false,
                    };
                    let enter = near && !s.inside;
                    s.inside = near;
                    enter
                }
                When::Event { .. } => match s.listener {
                    Some(l) if self.objects.get(l).is_some_and(|o| o.is_some()) && self.prop(l, "bEnabled").as_bool() => {
                        self.set_prop(l, "bEnabled", Value::Bool(false));
                        true
                    }
                    _ => false,
                },
            };
            if !fire {
                continue;
            }
            match s.speaker {
                Some(a) => {
                    s.done = true;
                    d.queue.push_back((i, a, now + s.say.delay));
                }
                None if matches!(s.say.when, When::Start) && !s.warned => {
                    // The level's characters may come a little later (spawned by its scripts).
                    s.warned = now > 5.0;
                    if s.warned {
                        s.done = true;
                        self.mod_report(format!("warning: {map}: {}: nobody named {} on this map ({})", s.say.describe(), s.say.by, s.source));
                    }
                }
                None if !s.warned => {
                    s.warned = true;
                    self.mod_report(format!("warning: {map}: {}: nobody named {} on this map ({})", s.say.describe(), s.say.by, s.source));
                }
                None => {}
            }
        }
        // DialogueManager.WarnIfAVoiceIsAlreadyRunning: nothing over a line of the game's own
        // dialogues either.
        let free = now >= d.busy_until && !d.queue.is_empty() && !self.mod_game_dialogue_running();
        let next = free.then(|| d.queue.front().copied()).flatten().filter(|&(_, _, at)| now >= at);
        if let Some((i, speaker, _)) = next {
            d.queue.pop_front();
            let say = d.says[i].clone();
            self.mods.dialogues = d;
            let seconds = self.mod_say_line(speaker, &say);
            self.mods.dialogues.busy_until = now + seconds;
            return;
        }
        self.mods.dialogues = d;
    }

    /// A DialogueManager of the level is saying a line (bSpeaking).
    fn mod_game_dialogue_running(&self) -> bool {
        self.actors.iter().any(|&a| self.is_a(a, "DialogueManager") && self.propr(a, "bSpeaking").as_bool())
    }

    /// Says one line now: its voice and its subtitle. The seconds it lasts.
    fn mod_say_line(&mut self, speaker: ObjId, s: &LiveSay) -> f32 {
        let map = self.mod_map_name();
        let what = s.say.describe();
        if !self.mod_alive(speaker) {
            self.mod_report(format!("warning: {map}: {what}: {} died before saying it ({})", s.say.by, s.source));
            return 0.0;
        }
        let (sound, mut text) = match &s.say.line {
            Some(line) => {
                let sound = self.mod_line_sound(&s.say);
                if sound.is_none() {
                    self.mod_report(format!("warning: {map}: {what}: the game has no voice {line} (see dialogues.md) ({})", s.source));
                }
                let text = s.say.text.clone().or_else(|| self.mod_game_line_text(line));
                if text.is_none() {
                    self.mod_report(format!("warning: {map}: {what}: no subtitle for {line} in the game's texts ({})", s.source));
                }
                (sound, text)
            }
            None => (s.say.sound.clone(), s.say.text.clone()),
        };
        let mut seconds = None;
        if let (Some(line), Some(path)) = (&s.say.line, &sound) {
            // A line of the game is played as DialogueManager plays one: PlayStrVoice(name,
            // speaker), on the speaker's voice slot. A line of another level comes from a bank
            // loaded for the mods: its name is added to the map's spoken lines first.
            let why = format!("{map}: {what}");
            let ready = self.mod_ensure_sound(path, &why).is_ok() && self.audio.as_mut().is_some_and(|h| h.add_str_voice(line));
            if ready && self.audio_dispatch(crate::canvas::AudioCall::StrVoice { actor: speaker, name: line.clone(), rolloff: None }) {
                seconds = self.audio.as_ref().map(|h| h.wave_duration(line)).filter(|&d| d > 0.0).or_else(|| self.mod_sound_seconds(path));
            } else {
                self.mod_report(format!("warning: {map}: {what}: its voice could not be played ({})", s.source));
            }
        } else if let Some(path) = &sound {
            match self.mod_play_sound(speaker, path, &format!("{map}: {what}")) {
                Ok(()) => {
                    seconds = self.mod_sound_seconds(path);
                }
                Err(why) => self.mod_report(format!("warning: {map}: {what}: {why} ({})", s.source)),
            }
        }
        // DialogueManager.Speak: GetWaveDuration + 1, or 3 seconds.
        let seconds = seconds.map_or(UNKNOWN_LENGTH, |s| s + 1.0);
        if let Some(t) = text.take().filter(|t| !t.trim().is_empty()) {
            self.mod_subtitle(speaker, &t, seconds);
            self.mod_report(format!("{map}: {what}: \"{t}\" ({})", s.source));
        } else {
            self.mod_report(format!("{map}: {what} ({})", s.source));
        }
        seconds
    }

    /// The voice of a line: the game line's sound in its level's Voices package, or the
    /// mod's `sound`.
    fn mod_line_sound(&mut self, say: &Say) -> Option<String> {
        let Some(line) = &say.line else { return say.sound.clone() };
        let (level, _, _) = openxiii_mods::dialogue::parse_line_name(line)?;
        let path = format!("{level}Voices.{line}");
        self.audio_sound_ref(&path).map(|_| path)
    }

    /// Length of sound `path`: a mod's own sound (its decoded samples), or a single wave of a
    /// loaded bank.
    pub(crate) fn mod_sound_seconds(&mut self, path: &str) -> Option<f32> {
        if let Some(name) = path.strip_prefix(openxiii_mods::own_sounds::PREFIX) {
            let own = self.mods.own_sounds.as_ref()?.iter().find(|o| o.def.name.eq_ignore_ascii_case(name))?;
            let per_second = own.pcm.rate as f32 * own.pcm.channels.max(1) as f32;
            return (per_second > 0.0).then(|| own.pcm.samples.len() as f32 / per_second);
        }
        let id = self.audio_sound_ref(crate::mod_sounds::object_path(path))?.cuuid;
        self.audio.as_ref()?.engine.sound_seconds(id).filter(|&s| s > 0.0)
    }

    /// Subtitle of game line `name` in the player's language (the English text if that
    /// language has no file for the level).
    pub(crate) fn mod_game_line_text(&mut self, name: &str) -> Option<String> {
        let (level, _, _) = openxiii_mods::dialogue::parse_line_name(name)?;
        let config = self.config.as_mut()?;
        let language = config.language.clone();
        for lang in [language.as_str(), "int"] {
            let text = config.file_text(&format!("{}.{lang}", level.to_ascii_lowercase()));
            if let Some(t) = openxiii_mods::dialogue::game_line_text(&text, name) {
                return Some(t);
            }
        }
        None
    }

    /// The HUD shows `text` as a dialogue line of `speaker` for `seconds`
    /// (PC.myHUD.LocalizedMessage(Class'XIIIDialogMessage', 0, None, None, Speaker, Text), then
    /// HudDlg.MyMessage.EndOfLife and Lifetime as DialogueManager.Speak sets them).
    fn mod_subtitle(&mut self, speaker: ObjId, text: &str, seconds: f32) {
        let Some(pc) = self.mod_local_pc() else { return };
        let Some(hud) = self.prop(pc, "myHUD").as_object() else { return };
        let Some(message) = self.class("XIII.XIIIDialogMessage") else { return };
        let Some(f) = self.find_function(hud, "LocalizedMessage", false) else { return };
        let args =
            vec![Some(Value::Class(Some(message))), Some(Value::Int(0)), None, None, Some(Value::Object(Some(speaker))), Some(Value::Str(text.to_string()))];
        if let Err(e) = self.call(f, hud, args) {
            self.mod_report(format!("warning: subtitle \"{text}\": {e}"));
            return;
        }
        let Some(dialog) = self.prop(hud, "HudDlg").as_object() else { return };
        let now = self.mod_level_time();
        if let Value::Struct(name, fields) = self.prop(dialog, "MyMessage") {
            let Some(index) = self.struct_def(name.as_str()).map(|d| d.index.clone()) else { return };
            let mut fields = fields.into_vec();
            for (field, value) in [("endoflife", now + seconds), ("lifetime", seconds)] {
                if let Some(&i) = index.get(field) {
                    fields[i] = Value::Float(value);
                }
            }
            self.set_prop(dialog, "MyMessage", Value::Struct(name, crate::value::Fields::new(fields)));
        }
    }
}

/// One line of the game's dialogues, as `line = "…"` names it.
#[derive(Debug, Clone)]
pub struct DialogueLine {
    /// Level of the line (its Voices package and localization file).
    pub level: String,
    /// "Plage01_Jones_01".
    pub name: String,
    pub speaker: String,
    /// The level's Voices package has its voice.
    pub voice: bool,
    pub subtitle: String,
}

impl Vm {
    /// The lines of the DialogueManagers of each level's localization file in language `lang`
    /// ("int", "frt"…; English when the level has no file in that language), with the speaker
    /// and whether the level's Voices package has its voice.
    pub(crate) fn dialogue_lines(&mut self, lang: &str) -> Result<Vec<DialogueLine>, String> {
        let game = self.system_dir.as_ref().and_then(|d| d.parent()).map(std::path::Path::to_path_buf).ok_or("no game folder")?;
        let system = self.system_dir.clone().ok_or("no System folder")?;
        let mut levels: Vec<String> = std::fs::read_dir(game.join("Sounds"))
            .map_err(|e| format!("{}: {e}", game.join("Sounds").display()))?
            .filter_map(Result::ok)
            .filter_map(|e| e.path().file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter_map(|s| s.strip_suffix("Voices").map(str::to_string))
            .collect();
        levels.sort_by_key(|s| s.to_lowercase());
        levels.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut out = Vec::new();
        for level in &levels {
            let read =
                |ext: &str| std::fs::read(system.join(format!("{}.{ext}", level.to_ascii_lowercase()))).ok().map(|b| crate::config::decode_file_text(&b));
            let text = read(&lang.to_ascii_lowercase()).or_else(|| read("int")).unwrap_or_default();
            let lines = openxiii_mods::dialogue::game_lines(&text);
            if lines.is_empty() {
                continue;
            }
            let package = format!("{level}Voices");
            let voices: Vec<String> = match self.library.as_mut().and_then(|lib| lib.get(&package)) {
                Some(p) => (0..p.package.exports.len())
                    .filter(|&i| p.package.export_class(i) == "Sound")
                    .map(|i| p.package.object_name(openxiii_package::ObjectRef::Export(i)).to_string())
                    .collect(),
                None => Vec::new(),
            };
            for l in lines {
                let Some((_, speaker, index)) = openxiii_mods::dialogue::parse_line_name(&l.name) else { continue };
                // The level's name as the map file writes it, the speaker as the voice does.
                let fallback = format!("{level}_{speaker}_{index:02}");
                let voice = voices.iter().find(|v| v.eq_ignore_ascii_case(&l.name));
                let name = voice.cloned().unwrap_or(fallback);
                let speaker = openxiii_mods::dialogue::parse_line_name(&name).map_or(speaker.to_string(), |(_, s, _)| s.to_string());
                out.push(DialogueLine { level: level.clone(), name, speaker, voice: voice.is_some(), subtitle: l.text.replace(['\r', '\n'], " ") });
            }
        }
        Ok(out)
    }

    /// dialogues.md: the lines of the game a mod can name in `line = "…"`, map by map: the
    /// lines of the DialogueManagers of each level's localization file (English), with the
    /// speaker and whether the level's Voices package has its voice.
    pub fn dialogues_reference(&mut self) -> Result<String, String> {
        let lines = self.dialogue_lines("int")?;
        let mut out = String::from("# Dialogue lines of the game\n\n");
        out.push_str("All the names a mod can use: [names/](names/README.md).\n\n");
        out.push_str("Generated by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`)\n");
        out.push_str("from the game's own files. A mod names a line in `line = \"…\"` of a `[[map.<Map>.say]]` entry\n");
        out.push_str("(see [make-a-character-talk.md](make-a-character-talk.md)): any character can say any of them, on any map. The\n");
        out.push_str("subtitle is shown in the player's language; here, the English text. Lines without a voice show\n");
        out.push_str("their subtitle only (XIII's thoughts, for example).\n\n");
        let mut level = String::new();
        for l in &lines {
            if l.level != level {
                if !level.is_empty() {
                    out.push('\n');
                }
                level = l.level.clone();
                out.push_str(&format!("## {level}\n\n| Line | Speaker | Voice | Subtitle |\n|---|---|---|---|\n"));
            }
            out.push_str(&format!("| {} | {} | {} | {} |\n", l.name, l.speaker, if l.voice { "yes" } else { "no" }, l.subtitle.replace('|', r"\|")));
        }
        if !lines.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{} lines.\n", lines.len()));
        Ok(out)
    }
}
