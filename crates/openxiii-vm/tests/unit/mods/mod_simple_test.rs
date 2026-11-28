//! Mods build only: the simple character keys (openxiii-mods simple.rs) on the real game:
//! `character = "Jones"` without a [character] table, a fighting soldier with a weapon, a key
//! that spawns a character with simple keys, and `team` / `can_die` / `weapon` on a character
//! the map places (OXIII_GAME = game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// `map` started with the mod written in `toml` (read and resolved as the game does at startup),
/// the player logged in: (vm, player controller, player pawn).
fn start(game: &Path, map: &str, toml: &str) -> (Vm, ObjId, ObjId) {
    let mut f = openxiii_mods::defaults::parse_mod_file(toml, Path::new("test/mod.toml"), "test");
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    let problems = openxiii_mods::simple::resolve(&f.edits, &f.actions, &mut f.characters);
    assert!(problems.is_empty(), "{problems:?}");
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(f.edits);
    vm.set_mod_behaviour_here(f.actions, f.characters);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), map);
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join(format!("{map}.unr"))).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    (vm, pc, pawn)
}

fn ticks(vm: &mut Vm, seconds: f32) {
    for _ in 0..(seconds * 30.0) as usize {
        vm.tick(1.0 / 30.0);
    }
}

fn mesh(vm: &Vm, a: ObjId) -> String {
    crate::mod_character_models::object_name(vm, &vm.native_value(a, Field::Mesh))
        .or_else(|| crate::mod_character_models::object_name(vm, &vm.prop(a, "Mesh")))
        .unwrap_or_default()
}

/// Live characters (Pawns) drawn with `model`.
fn with_look(vm: &Vm, model: &str) -> Vec<ObjId> {
    vm.actors
        .iter()
        .copied()
        .filter(|&a| vm.objects[a].is_some() && vm.is_a(a, "Pawn") && !vm.native_prop(a, Field::DeleteMe).as_bool() && mesh(vm, a).eq_ignore_ascii_case(model))
        .collect()
}

/// Classes of the inventory a pawn carries.
fn inventory(vm: &Vm, pawn: ObjId) -> Vec<String> {
    let mut out = Vec::new();
    let mut k = vm.prop(pawn, "Inventory").as_object();
    while let Some(i) = k {
        if out.len() > 32 {
            break;
        }
        out.push(vm.objects[i].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default());
        k = vm.prop(i, "Inventory").as_object();
    }
    out
}

fn controller(vm: &Vm, pawn: ObjId) -> ObjId {
    vm.native_prop(pawn, Field::Controller).as_object().expect("an AI controller")
}

const PLAGE01: &str = r#"
[[map.Plage01.add]]
character = "Jones"
position = [-470, -860, 1190]

[[map.Plage01.add]]
character = "Spads"
position = [-450, -1100, 1190]
behaviour = "fights"
weapon = "Shotgun"
can_die = false
skin = "XIIIPersos.Skins.SpadsNeigeTex"

[[action]]
key = "K"
spawn = "Carrington"
team = "neutral"
in_front_of_player = 300
"#;

#[test]
fn simple_characters_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Negative control: without the mod, no Jones, Carrington or mod character on the map.
    let (plain, _, _) = start(&game, "Plage01", "");
    assert!(with_look(&plain, "XIIIPersos.JonesM").is_empty());
    assert!(with_look(&plain, "XIIIPersos.CarringtonM").is_empty());
    assert!(plain.mod_characters_alive().is_empty());
    drop(plain);

    let (mut vm, pc, player) = start(&game, "Plage01", PLAGE01);
    // character = "Jones": a calm story character with Jones's look, named Jones, a friend.
    let jones = with_look(&vm, "XIIIPersos.JonesM");
    assert_eq!(jones.len(), 1, "{:?}", vm.mod_messages);
    let jones = jones[0];
    assert!(vm.is_a(jones, "Cine2"), "{}", vm.class_path(vm.objects[jones].as_ref().unwrap().class));
    assert_eq!(vm.prop(jones, "PawnName"), Value::Str("Jones".into()));
    assert_eq!(vm.mod_alliance(controller(&vm, jones), Some(player)), Some(1));
    // The fighter: a soldier with the soldiers' look, an enemy who cannot be killed.
    let spads: Vec<ObjId> = with_look(&vm, "XIIIPersos.SpadsM").into_iter().filter(|&a| vm.mod_characters_alive().iter().any(|(p, _)| *p == a)).collect();
    assert_eq!(spads.len(), 1, "{:?}", vm.mod_messages);
    let spads = spads[0];
    assert!(vm.is_a(spads, "BaseSoldier"));
    // Its skin: on the host, and noted for the network guests (mod_net_skins.rs).
    let skins = format!("{:?}", vm.prop(spads, "Skins")).to_lowercase();
    assert!(skins.contains("spadsneigetex"), "{skins} {:?}", vm.mod_messages);
    assert!(vm.mods.net_skins.get(&spads).is_some_and(|s| s.skins.iter().any(|n| n.to_lowercase().contains("spadsneigetex"))));
    let ai = controller(&vm, spads);
    assert_eq!(vm.mod_alliance(ai, Some(player)), Some(-1));
    assert_eq!(vm.prop(ai, "bGodMode"), Value::Bool(true));
    // Its weapon: InitialInventory, given when its AI starts.
    ticks(&mut vm, 3.0);
    let carried = inventory(&vm, spads);
    assert!(carried.iter().any(|c| c.eq_ignore_ascii_case("XIII.FusilPompe")), "{carried:?} {:?}", vm.mod_messages);
    // The key: Carrington, neutral, with his own voice (CodeMesh 16).
    let (ok, text) = vm.console_command_out(pc, "modkey K");
    assert!(ok, "{text}");
    let carrington = with_look(&vm, "XIIIPersos.CarringtonM");
    assert_eq!(carrington.len(), 1, "{:?}", vm.mod_messages);
    assert_eq!(vm.prop(carrington[0], "CodeMesh"), Value::Int(16));
    assert_eq!(vm.mod_alliance(controller(&vm, carrington[0]), Some(player)), Some(0));
    assert!(!vm.mod_messages.iter().any(|m| m.contains("warning")), "{:?}", vm.mod_messages);
}

#[test]
fn simple_change_keys_on_prock03() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let toml = r#"
[[map.PRock03.change]]
who = "gardien_lostpass"
team = "enemy"
can_die = false
weapon = "Pistol"
name = "Major"
"#;
    // Negative control: the guard is not a mod character without the change.
    let (plain, _, _) = start(&game, "PRock03", "");
    let guard = plain.find_placed("gardien_lostpass", None, 0.0)[0];
    assert!(!plain.mod_characters_alive().iter().any(|(p, _)| *p == guard));
    drop(plain);

    let (mut vm, _, player) = start(&game, "PRock03", toml);
    let guard = vm.find_placed("Major", None, 0.0);
    assert_eq!(guard.len(), 1, "the PawnName is the new name: {:?}", vm.mod_messages);
    let guard = guard[0];
    assert!(vm.mod_characters_alive().iter().any(|(p, _)| *p == guard), "{:?}", vm.mod_messages);
    let ai = controller(&vm, guard);
    assert_eq!(vm.mod_alliance(ai, Some(player)), Some(-1));
    assert_eq!(vm.prop(ai, "bGodMode"), Value::Bool(true));
    ticks(&mut vm, 3.0);
    let carried = inventory(&vm, guard);
    assert!(carried.iter().any(|c| c.eq_ignore_ascii_case("XIII.Beretta")), "{carried:?}");
}
