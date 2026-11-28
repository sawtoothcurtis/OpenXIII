//! Mods build only: keys bound to mod actions and mod characters, on the real Plage01
//! (OXIII_GAME = original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::behaviour::{ActionKind, CharacterDef, ModAction, StateLook, Team};
use openxiii_mods::placement::Source;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn source() -> Source {
    Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }
}

fn spawn_action(key: &str, what: &str, distance: f32) -> ModAction {
    ModAction { key: key.into(), kind: ActionKind::Spawn { what: what.into(), distance }, source: source() }
}

/// A soldier kept in a calm state of its AI, with a looping idle animation.
fn guard(name: &str, team: Team, can_die: bool) -> CharacterDef {
    CharacterDef {
        name: name.into(),
        base: "XIDPawn.BaseSoldier".into(),
        team: Some(team),
        allowed_states: vec!["ResteSurPlace".into()],
        can_die,
        death_animation: Some("DeathDos".into()),
        properties: vec![],
        states: vec![StateLook { state: "ResteSurPlace".into(), animation: Some("WaitNeutre".into()), sound: None, looping: true }],
        source: source(),
        keys: Default::default(),
        like: None,
    }
}

/// Plage01 started and the player logged in: (vm, player controller, player pawn).
fn plage01(game: &Path, actions: Vec<ModAction>, characters: Vec<CharacterDef>) -> (Vm, ObjId, ObjId) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    if !actions.is_empty() || !characters.is_empty() {
        vm.set_mod_behaviour_here(actions, characters);
    }
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    (vm, pc, pawn)
}

fn live_actors(vm: &Vm) -> usize {
    vm.actors.iter().filter(|&&a| vm.objects[a].is_some() && !vm.native_prop(a, Field::DeleteMe).as_bool()).count()
}

fn state(vm: &Vm, o: ObjId) -> String {
    vm.objects[o].as_ref().and_then(|x| x.state.clone()).unwrap_or_default()
}

/// Sequence playing on channel 0 of `actor`, and whether it loops.
fn animation(vm: &Vm, actor: ObjId) -> (String, bool) {
    vm.mesh_insts.get(&actor).map(|i| (i.active_state(0).0, i.in_loop(0))).unwrap_or_default()
}

fn shoot(vm: &mut Vm, target: ObjId, by: ObjId, damage: i32) {
    let dt = vm.class("XIII.DTGunned").map(|c| Value::Class(Some(c))).unwrap_or(Value::Class(None));
    let at = vm.native_value(target, Field::Location);
    vm.event_pub(target, "TakeDamage", vec![Value::Int(damage), Value::Object(Some(by)), at, Value::vector(0.0, 0.0, 0.0), dt]);
}

fn ticks(vm: &mut Vm, seconds: f32) {
    for _ in 0..(seconds * 30.0) as usize {
        vm.tick(1.0 / 30.0);
    }
}

#[test]
fn keys_and_characters_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Without mod actions: the key does nothing, the level does not change.
    let (mut plain, plain_pc, _) = plage01(&game, Vec::new(), Vec::new());
    let before = live_actors(&plain);
    assert!(!plain.mod_key(plain_pc, "F8"));
    assert_eq!(live_actors(&plain), before);
    assert!(plain.mod_characters_alive().is_empty());

    let actions = vec![spawn_action("F8", "Guard", 300.0), spawn_action("F10", "Statue", 300.0)];
    let characters = vec![guard("Guard", Team::Enemy, true), guard("Statue", Team::Neutral, false)];
    let (mut vm, pc, player) = plage01(&game, actions, characters);
    assert_eq!(live_actors(&vm), before, "the mod actions must not change the level by themselves");

    // F8: the guard appears 300 units in front of the player, on the ground, facing him.
    let (ok, text) = vm.console_command_out(pc, "modkey F8");
    assert!(ok, "{text}");
    let alive = vm.mod_characters_alive();
    assert_eq!(alive.len(), 1, "{:?}", vm.mod_messages);
    let (guard_pawn, guard_ai) = alive[0];
    let guard_ai = guard_ai.expect("the guard has no AI controller");
    let [px, py, _] = vm.native_value(player, Field::Location).as_vector();
    let [gx, gy, _] = vm.native_value(guard_pawn, Field::Location).as_vector();
    let distance = ((gx - px).powi(2) + (gy - py).powi(2)).sqrt();
    assert!((distance - 300.0).abs() < 40.0, "distance {distance}");
    let yaw = vm.native_value(pc, Field::Rotation).as_rotator()[1] as f32 * std::f32::consts::TAU / 65536.0;
    let ahead = (gx - px) * yaw.cos() + (gy - py) * yaw.sin();
    assert!(ahead > 250.0, "not in front of the player: {ahead}");
    let facing = vm.native_value(guard_pawn, Field::Rotation).as_rotator()[1];
    let expected = (vm.native_value(pc, Field::Rotation).as_rotator()[1] + 32768).rem_euclid(65536);
    assert!((facing - expected).rem_euclid(65536).min((expected - facing).rem_euclid(65536)) < 2000, "facing {facing}, expected {expected}");

    // The player shoots the enemy guard (lightly): its AI wants to fight back, tries its attack
    // states, and stays in ResteSurPlace with its looping animation.
    ticks(&mut vm, 3.0);
    shoot(&mut vm, guard_pawn, player, 5);
    ticks(&mut vm, 5.0);
    assert!(vm.mod_redirects > 0, "the AI never tried to leave its allowed state");
    assert_eq!(vm.prop(guard_ai, "Enemy").as_object(), Some(player), "the guard should have the player as its enemy");
    assert!(state(&vm, guard_ai).eq_ignore_ascii_case("ResteSurPlace"), "state {}", state(&vm, guard_ai));
    // Over two seconds, the looping animation is on nearly all the time (a cross-fade of a few
    // frames can follow each restart of the state by the AI).
    let mut on = 0;
    let mut seen = Vec::new();
    for _ in 0..60 {
        vm.tick(1.0 / 30.0);
        let (anim, looping) = animation(&vm, guard_pawn);
        if anim.eq_ignore_ascii_case("WaitNeutre") && looping {
            on += 1;
        } else if seen.len() < 5 {
            seen.push((anim, looping));
        }
    }
    assert!(on >= 50, "looping animation on {on}/60 frames, otherwise {seen:?}");
    assert_eq!(vm.alliance_level(guard_ai, Some(player)), -1);

    // Shot: it dies with its death animation.
    shoot(&mut vm, guard_pawn, player, 1000);
    ticks(&mut vm, 0.2);
    assert!(vm.prop(guard_pawn, "Health").as_int() <= 0);
    assert!(state(&vm, guard_pawn).eq_ignore_ascii_case("Dying"), "pawn state {}", state(&vm, guard_pawn));
    let (anim, _) = animation(&vm, guard_pawn);
    assert!(anim.eq_ignore_ascii_case("DeathDos"), "death animation {anim}");

    // F10: a neutral character that cannot die.
    assert!(vm.mod_key(pc, "F10"));
    let statue = vm.mod_characters_alive().into_iter().find(|&(p, _)| p != guard_pawn).expect("no second character");
    assert_eq!(vm.alliance_level(statue.1.unwrap(), Some(player)), 0);
    shoot(&mut vm, statue.0, player, 1000);
    ticks(&mut vm, 0.5);
    assert!(vm.prop(statue.0, "Health").as_int() > 0, "the statue died");
    assert!(!state(&vm, statue.0).eq_ignore_ascii_case("Dying"));

    // Unknown names: a warning, nothing created.
    vm.set_mod_behaviour_here(vec![spawn_action("F11", "Nobody", 100.0)], vec![]);
    assert!(vm.mod_key(pc, "F11"));
    assert!(vm.mod_messages.iter().any(|m| m.contains("unknown class Nobody (no [character] of that name either)")), "{:?}", vm.mod_messages);
}

/// `remove` of a mods character: Destroyed sends its controller to another state, which must
/// not be kept in its allowed state (nor its look played again on the pawn being removed).
/// Negative control: removing it while it is still listed as a mods character redirects that
/// state, as before the fix.
#[test]
fn removed_character_gets_no_look_again() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let actions = vec![spawn_action("F8", "Guard", 300.0)];
    let (mut vm, pc, _) = plage01(&game, actions.clone(), vec![guard("Guard", Team::Neutral, true)]);
    assert!(vm.mod_key(pc, "F8"));
    ticks(&mut vm, 1.0);
    let (pawn, _) = vm.mod_characters_alive()[0];
    let before = vm.mod_redirects;
    vm.mod_script_remove(pawn).unwrap();
    assert_eq!(vm.mod_redirects, before, "{:?}", vm.mod_messages);
    assert!(vm.mod_characters_alive().is_empty());
    ticks(&mut vm, 0.5);

    // Negative control: destroyed while still a mods character.
    let (mut vm, pc, _) = plage01(&game, actions, vec![guard("Guard", Team::Neutral, true)]);
    assert!(vm.mod_key(pc, "F8"));
    ticks(&mut vm, 1.0);
    let (pawn, _) = vm.mod_characters_alive()[0];
    let before = vm.mod_redirects;
    assert!(vm.destroy_actor(pawn).unwrap());
    assert!(vm.mod_redirects > before, "the negative control saw no redirect: {:?}", vm.mod_messages);
}

/// The singing soldier example (examples/mods/singing-soldier) on Plage01: U makes one singer
/// appear. He loops the soldier's singing animation ("gichante") with his whole body: the
/// waiting pose XIIIPawn.PlayWaiting blends over his upper body (channel 12, from 'X Spine') at
/// each loop stays off, as on SPADS02a's singer (a Cine2 looping it from its script). And the
/// song plays once, also when his AI asks for another state, which is kept in ResteSurPlace:
/// asked for again, the state he is in does not start the song a second time over the first.
#[test]
fn singing_spads_example_sings_once_with_his_whole_body() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/singing-soldier/mod.toml");
    let file = openxiii_mods::defaults::parse_mod_file(&std::fs::read_to_string(&toml).unwrap(), &toml, "singing-soldier");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let (mut vm, pc, player) = plage01(&game, file.actions, file.characters);
    vm.audio_init(&game, "Plage01");
    vm.audio_start_level();
    let level = vm.level.unwrap();
    vm.set_prop(level, "bCineFrame", Value::Bool(false));
    let song = "SPADS02aVoices.SPADS02a_SPADS2a_00";
    assert!(vm.mod_key(pc, "U"), "{:?}", vm.mod_messages);
    let res = vm.audio_sound_ref(song).expect("no song").cuuid;
    let alive = vm.mod_characters_alive();
    assert_eq!(alive.len(), 1);
    let (singer, ai) = alive[0];
    let ai = ai.expect("the singer has no AI controller");
    let (mut most, mut singing, mut upper_body_waiting) = (0usize, 0, 0);
    let frames = 600;
    for i in 0..frames {
        // His AI reacts: a state he may not go to (kept in ResteSurPlace), then a light shot.
        if i == 60 {
            vm.goto_state(ai, "Patrouille", "").unwrap();
        }
        if i == 120 {
            shoot(&mut vm, singer, player, 5);
        }
        let at = vm.native_value(player, Field::Location).as_vector();
        vm.audio.as_mut().unwrap().update_micro(0, at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        vm.tick(1.0 / 30.0);
        vm.audio_tick(1.0 / 30.0);
        let songs = vm.audio.as_ref().unwrap().engine.voices.iter().flatten().filter(|v| v.res == res && v.playing).count();
        most = most.max(songs);
        let (anim, looping) = animation(&vm, singer);
        if anim.eq_ignore_ascii_case("gichante") && looping {
            singing += 1;
            let layer = vm.mesh_insts.get(&singer).and_then(|m| m.channels.get(12)).map_or(0.0, |c| c.blend_alpha);
            if layer > 0.0 {
                upper_body_waiting += 1;
            }
        }
    }
    assert!(vm.mod_redirects >= 2, "the AI never asked for another state: {:?}", vm.mod_messages);
    assert!(vm.prop(singer, "Health").as_int() > 0);
    assert_eq!(most, 1, "the song plays {most} times at once ({:?})", vm.mod_messages);
    assert!(singing >= frames - 10, "singing animation on {singing}/{frames} frames");
    assert_eq!(upper_body_waiting, 0, "the upper body is in the waiting pose on {upper_body_waiting}/{frames} frames");
}
