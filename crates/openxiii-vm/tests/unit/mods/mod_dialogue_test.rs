//! Mods build only: lines said by characters (`[[map.<Map>.say]]`), on the real Plage01
//! (OXIII_GAME = original game folder; skipped when it is not set), with the example mod
//! tests/fixtures/mods/visitor-with-voice: Carrington (a character of Hual02) added and calm, a line of
//! the game said by a character of the map (Pam), two of his own lines of Hual02 said by him, on
//! an event of the map and near the player. Then any line and any sound of any level on Plage01,
//! and a line of the mod's own (its sound written by the test).

use std::path::{Path, PathBuf};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/visitor-with-voice/mod.toml")
}

/// Plage01 started with mod file `path` (text `text`), the player logged in, with sound.
fn plage01(game: &Path, path: &Path, text: &str) -> (Vm, ObjId) {
    let mut file = openxiii_mods::defaults::parse_mod_file(text, path, "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    // As the game does at startup: look names become characters (simple.rs).
    let problems = openxiii_mods::simple::resolve(&file.edits, &file.actions, &mut file.characters);
    assert!(problems.is_empty(), "{problems:?}");
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(file.edits);
    vm.set_mod_behaviour_here(file.actions, file.characters);
    let problems = vm.set_mod_own_sounds_here(file.sounds);
    assert!(problems.is_empty(), "{problems:?}");
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    vm.audio_init(game, "Plage01");
    vm.audio_start_level();
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    (vm, pawn)
}

fn said(vm: &Vm, what: &str) -> usize {
    vm.mod_messages.iter().filter(|m| m.contains(what) && !m.starts_with("warning")).count()
}

fn put(vm: &mut Vm, pawn: ObjId, at: [f32; 3]) {
    let mut args = [Value::vector(at[0], at[1], at[2])];
    let _ = crate::natives::call(vm, "SetLocation", pawn, &mut args, &[true]);
}

/// Ticks until `what` was said (at most `seconds`), the player kept at `at` if given.
fn tick_until(vm: &mut Vm, pawn: ObjId, at: Option<[f32; 3]>, what: &str, seconds: u32) -> bool {
    for frame in 0..(seconds * 30) {
        if let (Some(p), 0) = (at, frame % 15) {
            put(vm, pawn, p);
        }
        vm.tick(1.0 / 30.0);
        if said(vm, what) > 0 {
            return true;
        }
    }
    false
}

/// The text of the dialogue line the HUD shows (HudDlg.MyMessage.StringMessage).
fn subtitle(vm: &mut Vm) -> String {
    let Some(pc) = vm.actors.iter().copied().find(|&a| vm.is_a(a, "PlayerController")) else { return String::new() };
    let Some(hud) = vm.prop(pc, "myHUD").as_object() else { return String::new() };
    let Some(dialog) = vm.prop(hud, "HudDlg").as_object() else { return String::new() };
    let Value::Struct(name, fields) = vm.prop(dialog, "MyMessage") else { return String::new() };
    let Some(i) = vm.struct_def(name.as_str()).and_then(|d| d.index.get("stringmessage").copied()) else { return String::new() };
    match fields.into_vec().get(i) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

#[test]
fn example_lines_and_calm_carrington() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let path = example();
    let (mut vm, pawn) = plage01(&game, &path, &std::fs::read_to_string(&path).unwrap());
    let carrington = vm.mods.live.keys().copied().next().unwrap_or_else(|| panic!("Carrington not added\n{}", vm.mod_messages.join("\n")));
    assert_eq!(vm.prop(carrington, "Mesh"), Value::Asset("XIIIPersos.CarringtonM".into()));
    // His voice comes from the player's game files: Hual02's bank, loaded with the level.
    assert!(vm.mod_banks.iter().any(|(b, _, _)| b.eq_ignore_ascii_case("Hual02")), "{}", vm.mod_messages.join("\n"));
    let at = vm.native_value(carrington, Field::Location).as_vector();
    let start = vm.native_value(pawn, Field::Location).as_vector();
    let far = [start[0] - 2000.0, start[1], start[2]];
    // Janet (PawnName Pam, a character of the map) asks about him when the level starts.
    // Subtitles come in the player's language ("Who's that?" in English).
    let who = vm.mod_game_line_text("Plage01_Pam_03").expect("Janet's line");
    let ok = tick_until(&mut vm, pawn, Some(far), &format!("say Plage01_Pam_03 by Pam: \"{who}\""), 10);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    assert_eq!(subtitle(&mut vm), who);
    assert_eq!(said(&vm, "by Carrington:"), 0, "far away, Carrington says nothing\n{}", vm.mod_messages.join("\n"));
    // Near him during the opening cinematic (its comic frame; without a window the scene
    // never ends): XIII is not walking there, nothing is said. The test then ends the scene as
    // its last step does.
    let level = vm.level.unwrap();
    let near = [at[0] - 150.0, at[1], at[2]];
    assert!(vm.prop(level, "bCineFrame").as_bool(), "the opening cinematic is on");
    assert!(!tick_until(&mut vm, pawn, Some(near), "by Carrington:", 4), "{}", vm.mod_messages.join("\n"));
    // The end of the opening scene (TriggerEvent('Objectif91'), as the scene's last step) with
    // XIII near him: his line of Hual02 for the event, then his other line (near), one after
    // the other.
    vm.set_prop(level, "bCineFrame", Value::Bool(false));
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    let best = vm.mod_game_line_text("Hual02_Carrington_06").expect("Carrington's line");
    let ok = tick_until(&mut vm, pawn, Some(near), &format!("say Hual02_Carrington_06 by Carrington: \"{best}\""), 20);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    assert_eq!(subtitle(&mut vm), best);
    assert_eq!(said(&vm, "say Hual02_Carrington_05"), 0, "the second line waits for the first one");
    let believe = vm.mod_game_line_text("Hual02_Carrington_05").expect("Carrington's other line");
    let ok = tick_until(&mut vm, pawn, Some(near), &format!("say Hual02_Carrington_05 by Carrington: \"{believe}\""), 20);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    assert_eq!(subtitle(&mut vm), believe);
    // once = true: coming back, or the event again, says nothing again.
    tick_until(&mut vm, pawn, Some(far), "-", 3);
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    tick_until(&mut vm, pawn, Some(near), "-", 8);
    assert_eq!(said(&vm, "say Hual02_Carrington_06 by Carrington: "), 1);
    assert_eq!(said(&vm, "say Hual02_Carrington_05 by Carrington: "), 1);
    let log = vm.mod_messages.join("\n");
    assert!(!log.contains("warning"), "{log}");
    // Calm all along: alive, never targeting the player.
    assert!(vm.prop(carrington, "Health").as_int() > 0, "{log}");
    if let Some(c) = vm.native_prop(carrington, Field::Controller).as_object() {
        assert_ne!(vm.prop(c, "Enemy").as_object(), Some(pawn), "{log}");
        let state = vm.objects[c].as_ref().and_then(|o| o.state.clone()).unwrap_or_default();
        eprintln!("Carrington's controller: {} in state {state}", vm.objects[c].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default());
    }
}

/// Waves of the voices the audio engine is playing.
fn playing_waves(vm: &Vm) -> Vec<openxiii_assets::hx::Cuuid> {
    vm.audio.as_ref().map(|h| h.engine.voices.iter().flatten().map(|v| v.wave).collect()).unwrap_or_default()
}

/// Sound `path` of the game is in a loaded bank.
fn loaded(vm: &mut Vm, path: &str) -> bool {
    let id = vm.audio_sound_ref(path).expect("sound of the game").cuuid;
    vm.audio.as_ref().is_some_and(|h| h.engine.registry.contains_key(&id))
}

/// A voice of the audio engine plays game sound `path`: its resource, one it uses, or one of its
/// waves.
fn playing(vm: &mut Vm, path: &str) -> bool {
    let ids = ids_of(vm, path);
    assert!(ids.len() > 1, "no wave for {path}");
    playing_ids(vm, &ids)
}

/// The resources game sound `path` uses (itself included) and its waves.
fn ids_of(vm: &mut Vm, path: &str) -> std::collections::HashSet<openxiii_assets::hx::Cuuid> {
    let id = vm.audio_sound_ref(path).unwrap().cuuid;
    let index = crate::mod_sounds::bank_index(&vm.mod_bank_dir().unwrap());
    let mut ids = std::collections::HashSet::new();
    let mut todo = vec![id];
    while let Some(i) = todo.pop() {
        if ids.insert(i) {
            todo.extend(index.entries.get(&i).map(|(_, l)| l.clone()).unwrap_or_default());
        }
    }
    ids
}

/// A voice of the audio engine plays one of `ids`.
fn playing_ids(vm: &Vm, ids: &std::collections::HashSet<openxiii_assets::hx::Cuuid>) -> bool {
    let voices: Vec<_> = vm.audio.as_ref().map(|h| h.engine.voices.iter().flatten().map(|v| (v.res, v.wave)).collect()).unwrap_or_default();
    voices.iter().any(|(r, w)| ids.contains(r) || ids.contains(w))
}

/// One of `ids` plays within two seconds of audio, the listener at `at`.
fn heard_ids(vm: &mut Vm, ids: &std::collections::HashSet<openxiii_assets::hx::Cuuid>, at: [f32; 3]) -> bool {
    for _ in 0..40 {
        vm.audio.as_mut().unwrap().update_micro(0, at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        if playing_ids(vm, ids) {
            return true;
        }
        vm.audio_tick(0.05);
    }
    playing_ids(vm, ids)
}

/// Game sound `path` starts playing within two seconds of audio (a voice starts after the
/// sound driver's latency, a program's wave may come after a delay).
fn heard(vm: &mut Vm, path: &str) -> bool {
    for _ in 0..40 {
        if playing(vm, path) {
            return true;
        }
        vm.audio_tick(0.05);
    }
    playing(vm, path)
}

/// Resources and waves of the voices the audio engine is playing.
fn playing_voices(vm: &Vm) -> Vec<(openxiii_assets::hx::Cuuid, openxiii_assets::hx::Cuuid)> {
    vm.audio.as_ref().map(|h| h.engine.voices.iter().flatten().map(|v| (v.res, v.wave)).collect()).unwrap_or_default()
}

/// A mono 16-bit WAV file of `n` samples.
fn write_wav(path: &Path, rate: u32, n: usize) {
    let data: Vec<u8> = (0..n).flat_map(|i| ((i as i16).wrapping_mul(300)).to_le_bytes()).collect();
    let mut f = Vec::new();
    f.extend(b"RIFF");
    f.extend((36 + data.len() as u32).to_le_bytes());
    f.extend(b"WAVEfmt ");
    f.extend(16u32.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.extend(rate.to_le_bytes());
    f.extend((rate * 2).to_le_bytes());
    f.extend(2u16.to_le_bytes());
    f.extend(16u16.to_le_bytes());
    f.extend(b"data");
    f.extend((data.len() as u32).to_le_bytes());
    f.extend(data);
    std::fs::write(path, f).unwrap();
}

const SPADS_LINE: &str = "SPADS01_McCall_00";
const SPADS_VOICE: &str = "SPADS01Voices.SPADS01_McCall_00";
/// A helicopter of Kello01b (ambience): only Kello01b's bank holds it (sounds/XIIISound.md).
const HELICOPTER: &str = "XIIISound.Vehicles.KelloHelicoLoop1";
/// The sniper rifle's shot (a weapon of later levels, not on Plage01).
const SNIPER: &str = "XIIISound.Guns__SnipFire.SnipFire__hSnipFire";

/// Any line and any sound of any level, on Plage01, from the game's own files: a line of SPADS01
/// (voice and subtitle), a sound of Kello01b that is not a dialogue (with the mod's subtitle), a weapon's shot on a key, and
/// the mod's own line (its sound, a WAV file of the mod). The banks come when the level starts,
/// and each voice is heard.
#[test]
fn any_line_and_any_sound_of_any_level() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Without mods, none of these sounds is on Plage01.
    let (mut plain, _) = plage01(&game, Path::new("m/mod.toml"), "");
    assert!(!loaded(&mut plain, SPADS_VOICE), "the SPADS01 line is already on Plage01: the test proves nothing");
    assert!(!loaded(&mut plain, HELICOPTER), "the Kello01b helicopter is already on Plage01: the test proves nothing");
    assert!(!loaded(&mut plain, SNIPER), "the sniper shot is already on Plage01: the test proves nothing");
    assert!(plain.mod_banks.is_empty());
    drop(plain);

    let dir = std::env::temp_dir().join(format!("oxiii-any-line-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_wav(&dir.join("my_line.wav"), 22050, 22050);
    let text = format!(
        "[sounds.MyLine]\nfile = \"my_line.wav\"\n\n\
         [[action]]\nkey = \"J\"\nsound = \"{SNIPER}\"\n\n\
         [[map.Plage01.say]]\nby = \"Pam\"\nline = \"{SPADS_LINE}\"\nwhen = \"start\"\n\n\
         [[map.Plage01.say]]\nby = \"Pam\"\ntext = \"A helicopter, far away.\"\nsound = \"{HELICOPTER}\"\nwhen = \"start\"\n\n\
         [[map.Plage01.say]]\nby = \"Pam\"\ntext = \"My own line.\"\nsound = \"Mod.MyLine\"\nwhen = \"start\"\n"
    );
    let (mut vm, pawn) = plage01(&game, &dir.join("mod.toml"), &text);
    let messages = vm.mod_messages.join("\n");
    eprintln!("{messages}");
    // The banks of the two other levels are loaded with the level.
    for bank in ["SPADS01", "Kello01b"] {
        assert!(vm.mod_banks.iter().any(|(b, _, _)| b.eq_ignore_ascii_case(bank)), "bank {bank} not loaded\n{messages}");
    }
    assert!(loaded(&mut vm, SPADS_VOICE) && loaded(&mut vm, HELICOPTER), "{messages}");
    // XIII next to Janet: her voice is heard where she is.
    let pam = vm.actors.iter().copied().find(|&a| vm.is_a(a, "Pawn") && vm.prop(a, "PawnName").as_name().eq_ignore_ascii_case("Pam")).expect("Janet");
    let far = {
        let [x, y, z] = vm.native_value(pam, Field::Location).as_vector();
        [x - 100.0, y, z]
    };
    // Without a window there is no viewport: the listener (the window's microphone) is put
    // where XIII stands, as the game's view puts it. Sounds played by Janet are then heard
    // as in the game, from where she is.
    vm.audio.as_mut().unwrap().update_micro(0, far, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    // The SPADS01 line: McCall's voice and his subtitle, said by Janet.
    let mccall = vm.mod_game_line_text(SPADS_LINE).expect("McCall's line");
    let ok = tick_until(&mut vm, pawn, Some(far), &format!("say {SPADS_LINE} by Pam: \"{mccall}\""), 10);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    let voice = heard(&mut vm, SPADS_VOICE);
    let id = vm.audio_sound_ref(SPADS_VOICE).unwrap().cuuid;
    let index = crate::mod_sounds::bank_index(&vm.mod_bank_dir().unwrap());
    let about: Vec<_> = playing_voices(&vm).iter().map(|(r, _)| (*r, index.entries.get(r).map(|e| e.0.clone()))).collect();
    assert!(voice, "the SPADS01 voice is not playing: {about:?}; sound {id:?} = {:?}", index.entries.get(&id));
    assert_eq!(subtitle(&mut vm), mccall);
    // The Kello01b helicopter, a sound of the game by its name, with the mod's subtitle.
    let ok = tick_until(&mut vm, pawn, Some(far), &format!("say {HELICOPTER} by Pam: \"A helicopter, far away.\""), 30);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    assert!(heard(&mut vm, HELICOPTER), "the Kello01b helicopter is not playing: {:?}", playing_voices(&vm));
    // The mod's own line: its subtitle and its own sound.
    let ok = tick_until(&mut vm, pawn, Some(far), "say Mod.MyLine by Pam: \"My own line.\"", 30);
    assert!(ok, "{}", vm.mod_messages.join("\n"));
    vm.audio_tick(0.05);
    let wave = vm.mods.own_sounds.as_ref().unwrap().iter().find(|o| o.def.name == "MyLine").unwrap().wave;
    assert!(playing_waves(&vm).contains(&wave), "Mod.MyLine is not playing: {:?}", playing_waves(&vm));
    assert_eq!(subtitle(&mut vm), "My own line.");
    // A key bound to a sound of another level: the sniper rifle's shot, its bank loaded with
    // the level.
    assert!(loaded(&mut vm, SNIPER), "{}", vm.mod_messages.join("\n"));
    let pc = vm.native_prop(pawn, Field::Controller).as_object().unwrap();
    assert!(vm.mod_key(pc, "J"));
    assert!(heard(&mut vm, SNIPER), "the sniper shot is not playing: {:?}", playing_voices(&vm));
    let log = vm.mod_messages.join("\n");
    assert!(!log.contains("warning"), "{log}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `[map.<Map>.replace_sounds]`: an ambient sound of Plage01 (an actor's AmbientSound) replaced
/// by a sound of another level (Kello01b's helicopter): the new one is heard where the old one
/// was, and the old one is not.
#[test]
fn ambient_sound_replaced_by_another_levels_sound() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Without mods: an ambient actor of Plage01 whose sound is heard next to it.
    let (mut plain, _) = plage01(&game, Path::new("m/mod.toml"), "");
    let ambient: Vec<(ObjId, String)> = plain
        .actors
        .iter()
        .copied()
        .filter_map(|a| match plain.prop(a, "AmbientSound") {
            Value::Asset(p) => Some((a, p)),
            _ => None,
        })
        .collect();
    assert!(!ambient.is_empty(), "no ambient sound on Plage01");
    let mut chosen = None;
    for (a, sound) in &ambient {
        let at = plain.native_value(*a, Field::Location).as_vector();
        let ids = ids_of(&mut plain, sound);
        if heard_ids(&mut plain, &ids, at) {
            chosen = Some((plain.objects[*a].as_ref().unwrap().name.clone(), sound.clone(), at, ids));
            break;
        }
    }
    let (actor, sound, at, old) = chosen.unwrap_or_else(|| panic!("no ambient sound heard on Plage01: {ambient:?}"));
    eprintln!("{actor}: {sound}");
    assert!(!loaded(&mut plain, HELICOPTER));
    drop(plain);

    let text = format!("[map.Plage01.replace_sounds]\n\"{sound}\" = \"{HELICOPTER}\"\n");
    let (mut vm, _) = plage01(&game, Path::new("m/mod.toml"), &text);
    let log = vm.mod_messages.join("\n");
    assert!(log.contains(&format!("Plage01: sound {sound} plays as {HELICOPTER}")), "{log}");
    assert!(vm.mod_banks.iter().any(|(b, _, _)| b.eq_ignore_ascii_case("Kello01b")), "{log}");
    let new = ids_of(&mut vm, HELICOPTER);
    assert!(heard_ids(&mut vm, &new, at), "the helicopter is not heard at {actor}: {:?}\n{log}", playing_voices(&vm));
    assert!(!playing_ids(&vm, &old), "the old ambient sound still plays at {actor}");
    assert!(!log.contains("warning"), "{log}");

    // A wrong entry is reported, the file's other entries are kept.
    let file = openxiii_mods::defaults::parse_mod_file("[map.Plage01.replace_sounds]\n\"NotASound\" = \"XIIISound.A.B\"\n", Path::new("m/mod.toml"), "test");
    assert_eq!(file.problems.len(), 1, "{:?}", file.problems);
}

/// A line whose speaker is not on the map: a warning, nothing said.
#[test]
fn unknown_speaker_is_reported() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[[map.Plage01.say]]\nby = \"Nobody\"\ntext = \"Hello\"\nwhen = \"start\"\n";
    let (mut vm, pawn) = plage01(&game, Path::new("m/mod.toml"), text);
    tick_until(&mut vm, pawn, None, "-", 8);
    let log = vm.mod_messages.join("\n");
    assert!(log.contains("warning: Plage01: say \"Hello\" by Nobody: nobody named Nobody on this map"), "{log}");
    assert_eq!(said(&vm, "say \"Hello\""), 0, "{log}");
}
