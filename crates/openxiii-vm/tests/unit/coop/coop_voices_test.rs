//! Improved build with mods: a co-op player's character voice, on the real Plage01
//! (OXIII_GAME = original game folder; skipped when it is not set).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use openxiii_hx::engine::ResData;
use openxiii_hx::subsystem::SLOT_EFFECT;

use crate::character_screen::{Choice, ROSTER};
use crate::value::ObjId;
use crate::{NativeField as Field, Vm};

const HIT: &str = "XIIIsound.XIIIPerso__XIIIPaf.XIIIPaf__hXIIIPaf";
const LADDER: &str = "XIIIsound.SpecActions__LadderClimb.LadderClimb__hXIIIClimbLadder";
const JUMP: &str = "XIIIsound.XIIIPerso__XIIIJump.XIIIJump__hXIIIJump";

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn plage01(game: &Path) -> (Vm, ObjId) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
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

fn choice(id: &str) -> Choice {
    Choice { character: ROSTER.iter().position(|c| c.id == id).unwrap(), variant: 0 }
}

/// The waves heard when `sound` plays eight times on `pawn` with `params` (the game draws
/// whether and which grunt plays), half a second apart.
fn heard(vm: &mut Vm, pawn: ObjId, sound: &str, params: &[i32]) -> BTreeSet<String> {
    // The previous sounds end first (a death shout lasts a second).
    for _ in 0..40 {
        vm.audio_tick(0.05);
    }
    let mut out = BTreeSet::new();
    for _ in 0..8 {
        out.extend(heard_once(vm, pawn, sound, params));
    }
    out
}

fn heard_once(vm: &mut Vm, pawn: ObjId, sound: &str, params: &[i32]) -> BTreeSet<String> {
    let s = vm.audio_sound_ref(sound).expect("sound of the game");
    let state = vm.audio_actor_state(pawn).unwrap();
    let at = state.location;
    let mut out = BTreeSet::new();
    let hx = vm.audio.as_mut().unwrap();
    hx.update_micro(0, [at[0] + 10.0, at[1], at[2]], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    if params.is_empty() {
        hx.play_sound(Some(&state), None, &s, SLOT_EFFECT);
    } else {
        hx.play_sound_params(Some(&state), None, &s, SLOT_EFFECT, params);
    }
    for _ in 0..10 {
        let hx = vm.audio.as_ref().unwrap();
        for v in hx.engine.voices.iter().flatten().filter(|v| v.playing) {
            if let Some(ResData::Wav(w)) = hx.engine.registry.get(&v.res) {
                if out.insert(w.name.clone()) {
                    eprintln!("  heard {} ({:.2} s)", w.name, v.frames as f64 / v.rate.max(1) as f64);
                }
            }
        }
        vm.audio_tick(0.05);
    }
    out
}

fn any(set: &BTreeSet<String>, prefix: &str) -> bool {
    set.iter().any(|n| n.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()))
}

#[test]
fn a_player_who_is_not_xiii_shouts_with_his_characters_voice() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // XIII: his own grunt with the bullet's impact, his jump effort.
    let (mut vm, pawn) = plage01(&game);
    let xiii = heard(&mut vm, pawn, HIT, &[4, 30, 0]);
    assert!(any(&xiii, "OnoPafBull") && any(&xiii, "XIIIPafBull"), "{xiii:?}");
    let xiii_jump = heard(&mut vm, pawn, JUMP, &[]);
    assert!(any(&xiii_jump, "OnoJump") && any(&xiii_jump, "XIIIJump1"), "{xiii_jump:?}");
    // Choosing XIII changes nothing.
    vm.coop_voice(pawn, Choice::default());
    assert!(vm.audio.as_ref().unwrap().actor_wave_swaps.is_empty());

    // Jones: her hurt shouts (another level's bank), her death shout, no voice on the jump;
    // the impact and the jump's effort noise stay.
    let (mut vm, pawn) = plage01(&game);
    vm.coop_voice(pawn, choice("Jones"));
    eprintln!("{}", vm.log.iter().filter(|l| l.contains("Co-op")).cloned().collect::<Vec<_>>().join("\n"));
    // Other sounds of the pawn whose third parameter is set (a ladder climbed silently,
    // bSilent) say nothing of a death.
    heard_once(&mut vm, pawn, LADDER, &[300, 0, 1]);
    let hurt = heard(&mut vm, pawn, HIT, &[4, 30, 0]);
    assert!(any(&hurt, "Hurt_Jones") && any(&hurt, "XIIIPafBull") && !any(&hurt, "OnoPafBull"), "{hurt:?}");
    let fist = heard(&mut vm, pawn, HIT, &[8, 30, 0]);
    assert!(any(&fist, "Hurt_Jones") && !any(&fist, "OnoHurt_XIII"), "{fist:?}");
    let jump = heard(&mut vm, pawn, JUMP, &[]);
    assert!(any(&jump, "XIIIJump1") && !any(&jump, "OnoJump"), "{jump:?}");
    // Once killed, her death shout (and no more hurt shouts).
    let death = heard(&mut vm, pawn, HIT, &[4, 100, 1]);
    assert!(any(&death, "Dth1_Jones") && !any(&death, "Hurt_Jones"), "{death:?}");
    // Alive again in the same pawn (Health > 0 on the next frame): hurt shouts again.
    vm.coop_voice(pawn, choice("Jones"));
    let again = heard(&mut vm, pawn, HIT, &[4, 30, 0]);
    assert!(any(&again, "Hurt_Jones") && !any(&again, "Dth1_Jones"), "{again:?}");

    // Kim: the shouts the game has for her but never plays.
    let (mut vm, pawn) = plage01(&game);
    vm.coop_voice(pawn, choice("Kim"));
    let kim = heard(&mut vm, pawn, HIT, &[4, 30, 0]);
    assert!(any(&kim, "Hurt_Kim") && !any(&kim, "OnoPafBull"), "{kim:?}");

    // A soldier: his timbre's shouts.
    let (mut vm, pawn) = plage01(&game);
    vm.coop_voice(pawn, choice("FBI"));
    let fbi = heard(&mut vm, pawn, HIT, &[4, 30, 0]);
    assert!(any(&fbi, "Hurt_FBI") && !any(&fbi, "OnoPafBull"), "{fbi:?}");
}

#[test]
fn every_character_gets_hurt_and_death_shouts() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let (mut vm, _) = plage01(&game);
    for (c, ch) in ROSTER.iter().enumerate().skip(1) {
        for v in 0..ch.variants.len() {
            let choice = Choice { character: c, variant: v };
            let swaps = vm.coop_voice_swaps_for_test(choice, 0);
            let (hurt, death) = (swaps.0, swaps.1);
            assert!(hurt > 0 && death > 0, "{}: {hurt} hurt, {death} death", choice.code());
        }
    }
}

/// The name the game's shout files give each character's voice (`Hurt_<Name>_00`,
/// `Dth1_<Name>_00`), by roster id and variant; `{}` is the timbre (1 to 3).
fn shout_name(id: &str, variant: usize, timbre: i32) -> String {
    let t = timbre + 1;
    match id {
        "Jones" => "Jones".into(),
        "Carrington" => "Carrington".into(),
        "Mongoose" => "Mangoose".into(),
        "Kim" => "Kim".into(),
        "Amos" => "Amos".into(),
        "McCall" => "MacCall".into(),
        "Sheridan" => "Sheridan".into(),
        "Standwell" => "Standwell".into(),
        "Admiral" => "Pacha".into(),
        "Spads" => format!("SPADS{t}"),
        "GI" => format!("GI{t}"),
        "FBI" => format!("FBI{t}"),
        // One voice each, no timbres; the fourth look (Tueur5M) has the fourth killer's voice.
        "Killer" => format!("Killer{}", variant + 1),
        "Conspirator" => format!("KKK{t}"),
        "Policeman" => format!("Screw{t}"),
        _ => panic!("{id}: no expected voice"),
    }
}

#[test]
fn every_character_shouts_with_its_own_voice_in_every_timbre() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let (mut vm, pawn) = plage01(&game);
    for (c, ch) in ROSTER.iter().enumerate() {
        for v in 0..ch.variants.len() {
            let choice = Choice { character: c, variant: v };
            if ch.id == "XIII" {
                // XIII, in every outfit, keeps his own voice: nothing swapped.
                vm.coop_voice(pawn, choice);
                assert!(vm.audio.as_ref().unwrap().actor_wave_swaps.is_empty(), "{}: XIII's voice swapped", choice.code());
                continue;
            }
            for timbre in 0..3 {
                let (hurt, death) = vm.coop_voice_paths_for_test(choice, timbre);
                let name = shout_name(ch.id, v, timbre).to_ascii_lowercase();
                eprintln!("{} timbre {timbre}: hurt {hurt:?}, death {death:?}", choice.code());
                let last = |p: &String| p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                assert!(!hurt.is_empty() && !death.is_empty(), "{} timbre {timbre}: hurt {hurt:?}, death {death:?}", choice.code());
                assert!(hurt.iter().all(|p| last(p).starts_with(&format!("hurt_{name}_"))), "{} timbre {timbre}: hurt {hurt:?}", choice.code());
                assert!(death.iter().all(|p| last(p).starts_with(&format!("dth1_{name}_"))), "{} timbre {timbre}: death {death:?}", choice.code());
            }
        }
    }
}
