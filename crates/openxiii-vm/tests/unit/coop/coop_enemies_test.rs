//! Co-op extra enemies (coop_enemies.rs) on the real maps (OXIII_GAME = original game folder;
//! skipped when it is not set).

use std::path::{Path, PathBuf};

use crate::value::{ObjId, Value};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// `map` started with co-op on or off and the extra enemies setting `percent`.
fn start(game: &Path, map: &str, coop: bool, percent: Option<u32>) -> Vm {
    start_screen(game, map, coop, percent, false)
}

/// start(), and with `screen` the map opened as the co-op character screen's backdrop
/// (character_screen.rs: "Map?OXIIIChoose", set as character_screen_from_url does).
fn start_screen(game: &Path, map: &str, coop: bool, percent: Option<u32>, screen: bool) -> Vm {
    start_with(game, map, coop, percent, screen, false)
}

/// start_screen(), and with `loud` the copies spawned with their Touch events (test control).
fn start_with(game: &Path, map: &str, coop: bool, percent: Option<u32>, screen: bool, loud: bool) -> Vm {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), map);
    vm.url = url.clone();
    vm.set_coop(coop);
    if let Some(p) = percent {
        vm.set_coop_extra_enemies(p);
    }
    vm.coop.enemies.loud_spawns = loud;
    vm.load_map(&game.join("Maps").join(format!("{map}.unr"))).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    #[cfg(feature = "improved")]
    {
        vm.improved.character_screen.on = screen;
    }
    #[cfg(not(feature = "improved"))]
    let _ = screen;
    vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
    vm
}

fn live(vm: &Vm) -> Vec<ObjId> {
    vm.actors.iter().copied().filter(|&a| vm.objects[a].is_some() && !vm.prop(a, "bDeleteMe").as_bool()).collect()
}

/// Every property of the soldiers and generators, in map order.
fn enemy_state(vm: &Vm) -> Vec<(String, Vec<Value>)> {
    live(vm)
        .into_iter()
        .filter(|&a| vm.is_a(a, "BaseSoldier") || vm.is_a(a, "gennmi"))
        .map(|a| {
            let o = vm.objects[a].as_ref().unwrap();
            (o.name.clone(), o.props.iter().cloned().collect())
        })
        .collect()
}

fn class_name(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().map(|o| vm.classes[o.class].name.clone()).unwrap_or_default()
}

#[test]
fn solo_and_setting_zero_change_nothing() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    for map in ["Plage01", "Hual01a"] {
        // Solo with the setting at 100: the setting is never read.
        let (plain, set) = (start(&game, map, false, None), start(&game, map, false, Some(100)));
        assert_eq!(live(&plain).len(), live(&set).len(), "{map} solo");
        assert_eq!(enemy_state(&plain), enemy_state(&set), "{map} solo");
        assert!(set.coop.enemies.groups.is_empty() && set.coop.enemies.generators.is_empty());
        // Co-op with the setting at 0.
        let (coop, zero) = (start(&game, map, true, None), start(&game, map, true, Some(0)));
        assert_eq!(live(&coop).len(), live(&zero).len(), "{map} co-op 0");
        assert_eq!(enemy_state(&coop), enemy_state(&zero), "{map} co-op 0");
        // Negative control: co-op at 100 does change them.
        let full = start(&game, map, true, Some(100));
        assert!(live(&full).len() > live(&coop).len() || !full.coop.enemies.generators.is_empty(), "{map} co-op 100");
        assert_ne!(enemy_state(&coop), enemy_state(&full), "{map} co-op 100");
    }
}

#[test]
fn copies_generators_and_paired_deaths() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    for map in ["Plage01", "Hual01a", "PRock01a"] {
        let base = start(&game, map, true, Some(0));
        let half = start(&game, map, true, Some(50));
        let mut full = start(&game, map, true, Some(100));
        let (h, f) = (half.coop_enemies_report().clone(), full.coop_enemies_report().clone());
        eprintln!("{map} 50 %: {h:?}");
        eprintln!("{map} 100 %: {f:?}");
        assert_eq!(f.eligible, h.eligible);
        assert_eq!(f.wanted, f.eligible);
        assert_eq!(h.wanted, f.eligible / 2);
        assert_eq!(h.generated_after - h.generated_before, h.generated_before / 2, "{map} 50 % generators");
        assert_eq!(f.generated_after, 2 * f.generated_before, "{map} 100 % generators");
        assert_eq!(f.made + f.no_room + f.failed.iter().map(|x| x.1).sum::<usize>(), f.wanted);
        // One actor more per copy, plus its controller (and the controller's own spawns).
        assert!(live(&full).len() >= live(&base).len() + 2 * f.made);
        // Copies: same class, Tag, Event, ORDER, health, network numbers as their original; no
        // story item in their starting inventory.
        let (weapon, ammo) = (full.class("Weapon").unwrap(), full.class("Ammunition").unwrap());
        for g in full.coop.enemies.groups.iter().filter(|g| g.placed) {
            let original = g.members[0];
            for &c in &g.members[1..] {
                assert_eq!(class_name(&full, c), "BaseSoldier");
                for p in ["Tag", "Event", "ORDER", "Health", "NumReseauPropre", "NumReseauAttaque", "Skill", "Alliance", "Mesh"] {
                    assert_eq!(full.prop(c, p), full.prop(original, p), "{map} {p}");
                }
                assert!(full.prop(c, "Controller").as_object().is_some_and(|ai| full.is_a(ai, "IAController")));
                let Value::Array(items) = full.prop(c, "InitialInventory") else { panic!() };
                for i in items {
                    let Value::Struct(_, fields) = i else { continue };
                    if let Some(class) = fields[0].as_class() {
                        assert!(full.is_child_of(class, weapon) || full.is_child_of(class, ammo));
                    }
                }
            }
        }
        // Generators: finite ones doubled, unlimited ones untouched.
        for a in live(&base).into_iter().filter(|&a| base.is_a(a, "gennmi")) {
            let (before, after) = (base.prop(a, "NbNMIMax").as_int(), full.prop(a, "NbNMIMax").as_int());
            if full.coop.enemies.generators.contains_key(&a) {
                assert_eq!(after, before * 2, "{map}");
            } else {
                assert_eq!(after, before, "{map}");
            }
        }
        // Paired death: the original dies first, its Event moves to the copy, which fires it.
        let named = |v: &Value| !v.name_ref().is_empty() && !v.name_ref().eq_ignore_ascii_case("None");
        let pair = full.coop.enemies.groups.iter().find(|g| g.placed && g.members.len() == 2 && named(&full.prop(g.members[0], "Event"))).map(|g| (g.members[0], g.members[1]));
        if let Some((original, copy)) = pair {
            let event = full.prop(original, "Event");
            let dt = full.class("DTGunned").or_else(|| full.class("DTBladeCut")).unwrap();
            let died = |vm: &mut Vm, who: ObjId| {
                vm.set_prop(who, "Health", Value::Int(0));
                vm.call_named(who, "Died", vec![Value::Object(None), Value::Class(Some(dt)), Value::vector(0.0, 0.0, 0.0)]).unwrap();
            };
            died(&mut full, original);
            assert!(!named(&full.prop(original, "Event")), "{map}: the original's Event fired early");
            assert_eq!(full.prop(copy, "Event"), event, "{map}");
            died(&mut full, copy);
            assert_eq!(full.prop(copy, "Event"), event, "{map}: the last one keeps it and fires it");
            // Negative control: with the setting at 0 the same soldier keeps its Event to the end.
            let mut zero = start(&game, map, true, Some(0));
            let same = live(&zero).into_iter().find(|&a| zero.objects[a].as_ref().unwrap().name == full.objects[original].as_ref().unwrap().name).unwrap();
            died(&mut zero, same);
            assert_eq!(zero.prop(same, "Event"), event, "{map}");
        } else {
            eprintln!("{map}: no placed pair with a death Event");
        }
    }
}

/// Improved build only (the character screen exists there alone).
#[cfg(feature = "improved")]
#[test]
fn character_screen_backdrop_gets_no_extra_enemies() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let plain = start_screen(&game, "Hual02", true, Some(0), true);
    let play_zero = start_screen(&game, "Hual02", true, Some(0), false);
    for percent in [50, 100] {
        let screen = start_screen(&game, "Hual02", true, Some(percent), true);
        assert_eq!(live(&screen).len(), live(&plain).len(), "{percent} %");
        assert_eq!(enemy_state(&screen), enemy_state(&plain), "{percent} %");
        assert!(screen.coop.enemies.groups.is_empty() && screen.coop.enemies.generators.is_empty());
        // Negative control: the same map opened for play does get them.
        let play = start_screen(&game, "Hual02", true, Some(percent), false);
        assert_ne!(enemy_state(&play), enemy_state(&play_zero), "{percent} % in play");
        assert!(!play.coop.enemies.generators.is_empty() || !play.coop.enemies.groups.is_empty());
    }
}

/// Extras' looks (PRock04b, many looks) and the dialogue rule (Hual01a): looks come from the map's own eligible
/// enemies; scripted dialogue speakers are not copied; drawing a look never touches the game's
/// random stream.
#[test]
fn extra_looks_and_dialogue_speakers() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let vm = start(&game, "PRock04b", true, Some(100));
    let keys: std::collections::HashSet<String> = vm.coop.enemies.donors.iter().map(|d| d.look.key()).collect();
    assert!(keys.len() >= 2, "{keys:?}");
    let mut changed = 0;
    for g in vm.coop.enemies.groups.iter().filter(|g| g.placed) {
        let (original, look) = (g.members[0], |a: ObjId| crate::coop_enemies::Look { mesh: vm.prop(a, "Mesh"), skins: match vm.prop(a, "Skins") { Value::Array(v) => v, _ => Vec::new() }, code_mesh: 0, timbre: None }.key());
        for &c in &g.members[1..] {
            assert!(keys.contains(&look(c)) || look(c) == look(original), "{} not a look of the map", look(c));
            changed += usize::from(look(c) != look(original));
            // Everything else stays the original's.
            for p in ["Tag", "Event", "ORDER", "Health", "Skill", "NumReseauPropre", "SpecificAnimations", "InitialInventory"] {
                assert_eq!(vm.prop(c, p), vm.prop(original, p), "{p}");
            }
        }
    }
    assert!(changed > 0, "no extra got another look");
    // Scripted dialogue: BaseSoldier1 speaks in a DialogueManager and gets no copy; BaseSoldier3
    // (no dialogue) does (negative control).
    let mut vm = start(&game, "Hual01a", true, Some(100));
    let named = |vm: &Vm, n: &str| live(vm).into_iter().find(|&a| vm.objects[a].as_ref().unwrap().name == n).unwrap();
    let (speaker, plain) = (named(&vm, "BaseSoldier1"), named(&vm, "BaseSoldier3"));
    assert!(!vm.coop.enemies.member.contains_key(&speaker));
    assert!(vm.coop.enemies.member.contains_key(&plain));
    // The game's random stream is not drawn by the looks.
    let seed = vm.seed;
    let first = vm.enemies_extra_look(plain, false, 5, &[]);
    assert_eq!(vm.seed, seed);
    assert_eq!(vm.enemies_extra_look(plain, false, 5, &[]), first);
}

/// A copy born inside a volume gets no Touch event (its original, placed in it, got none at
/// the start either), but is in contact with it: DetectionVolume.Touch, which reads
/// Other.Controller, never runs for a copy without its controller (PRock04b, 100 %).
#[test]
fn copies_born_in_volumes_touch_quietly() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let detection = |vm: &Vm| vm.warnings.iter().chain(vm.log.iter()).filter(|l| l.contains("DetectionVolume") && l.contains("Accessed None")).count();
    let quiet = start(&game, "PRock04b", true, Some(100));
    assert!(quiet.coop_enemies_report().made > 0);
    assert_eq!(detection(&quiet), 0);
    // In contact all the same: some copy stands in a DetectionVolume.
    let in_volume = quiet.coop.enemies.groups.iter().filter(|g| g.placed).flat_map(|g| g.members[1..].to_vec()).any(|c| match quiet.prop(c, "Touching") {
        Value::Array(t) => t.iter().filter_map(Value::as_object).any(|o| quiet.is_a(o, "DetectionVolume")),
        _ => false,
    });
    assert!(in_volume);
    // Negative control: spawned with their Touch events, the warnings come back.
    let loud = start_with(&game, "PRock04b", true, Some(100), false, true);
    assert!(detection(&loud) > 0);
    // And none at 0 %.
    assert_eq!(detection(&start(&game, "PRock04b", true, Some(0))), 0);
}
