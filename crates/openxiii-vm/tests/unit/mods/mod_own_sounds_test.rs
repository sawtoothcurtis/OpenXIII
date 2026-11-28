//! Mods build only: the mods' own sounds, on the real Plage01 with its sounds (OXIII_GAME =
//! original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::behaviour::{ActionKind, ModAction};
use openxiii_mods::own_sounds::OwnSound;
use openxiii_mods::placement::Source;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

const PISTOL: &str = "XIIIsound.Guns__9mmFire.9mmFire__h9mmFire";

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn source() -> Source {
    Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }
}

/// A mono 16-bit WAV file of `n` samples.
fn write_wav(path: &Path, rate: u32, n: usize, step: i16) {
    let data: Vec<u8> = (0..n).flat_map(|i| ((i as i16).wrapping_mul(step)).to_le_bytes()).collect();
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

fn plage01(game: &Path, own: Vec<OwnSound>) -> (Vm, ObjId, ObjId, Vec<String>) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    let problems = vm.set_mod_own_sounds_here(own);
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
    (vm, pc, pawn, problems)
}

/// Waves of the voices the audio engine is playing.
fn playing_waves(vm: &Vm) -> Vec<openxiii_assets::hx::Cuuid> {
    vm.audio.as_ref().map(|h| h.engine.voices.iter().flatten().map(|v| v.wave).collect()).unwrap_or_default()
}

#[test]
fn own_sounds_new_and_replacing() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = std::env::temp_dir().join(format!("oxiii-own-sounds-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_wav(&dir.join("beep.wav"), 22050, 4410, 300);
    write_wav(&dir.join("pistol.wav"), 44100, 8820, 7);
    let own = vec![
        OwnSound { name: "Beep".into(), file: dir.join("beep.wav"), replace: None, source: source() },
        OwnSound { name: "Pistol".into(), file: dir.join("pistol.wav"), replace: Some(PISTOL.into()), source: source() },
        OwnSound { name: "Missing".into(), file: dir.join("missing.wav"), replace: None, source: source() },
    ];
    // Without own sounds, "Mod.Beep" is nothing.
    let (mut plain, _, _, _) = plage01(&game, Vec::new());
    assert!(plain.audio_sound_ref("Mod.Beep").is_none());

    let (mut vm, pc, _, problems) = plage01(&game, own);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("missing.wav"), "{problems:?}");

    // The new sound is a resource of the audio engine, and a key plays it.
    let beep = vm.audio_sound_ref("Mod.Beep").expect("Mod.Beep");
    let wavs = vm.audio.as_ref().unwrap().engine.registry.values().filter(|r| matches!(r, openxiii_hx::engine::ResData::Wav(_))).count();
    assert!(vm.audio.as_ref().unwrap().engine.registry.contains_key(&beep.cuuid), "not registered; {wavs} sounds in the engine; messages {:?}", vm.mod_messages);
    let wave = vm.mods.own_sounds.as_ref().unwrap().iter().find(|o| o.def.name == "Beep").unwrap().wave;
    let pcm = crate::mod_own_sounds::mod_wave_pcm(wave).expect("samples of Mod.Beep");
    assert_eq!((pcm.rate, pcm.channels, pcm.samples.len()), (22050, 1, 4410));
    assert_eq!(pcm.samples[3], 900);
    vm.set_mod_behaviour_here(vec![ModAction { key: "J".into(), kind: ActionKind::Sound { sound: "Mod.Beep".into() }, source: source() }], vec![]);
    assert!(vm.mod_key(pc, "J"));
    vm.audio_tick(0.05);
    assert!(playing_waves(&vm).contains(&wave), "Mod.Beep is not playing: {:?}", playing_waves(&vm));

    // The 9mm's firing sound plays the mod's samples: every wave it can play gives them.
    let id = vm.audio_sound_ref(PISTOL).unwrap().cuuid;
    let dir_banks = vm.mod_bank_dir().unwrap();
    let waves = crate::mod_sounds::bank_index(&dir_banks).waves_of(id);
    assert!(!waves.is_empty(), "no wave found for {PISTOL}");
    for w in &waves {
        let p = crate::mod_own_sounds::mod_wave_pcm(*w).expect("replaced wave");
        assert_eq!((p.rate, p.samples.len()), (44100, 8820));
    }
    let mut args = [Value::Asset(PISTOL.into())];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    crate::natives::call(&mut vm, "PlaySound", pawn, &mut args, &[true]).unwrap();
    vm.audio_tick(0.05);
    let now = playing_waves(&vm);
    assert!(now.iter().any(|w| waves.contains(w)), "the pistol sound did not start one of its waves: {now:?} / {waves:?}");
    assert!(vm.mod_messages.iter().any(|m| m.contains("Mod.Pistol plays instead of")), "{:?}", vm.mod_messages);
    let _ = std::fs::remove_dir_all(&dir);
}
