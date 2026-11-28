//! Mods build only: map selectors ("*", "solo", "multi", "Hual*", a map name) and their
//! priority, on the real Plage01 (OXIII_GAME = original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// Plage01 started with the mod file `text`, the player logged in: (vm, pawn).
fn plage01(game: &Path, text: &str) -> (Vm, ObjId) {
    let file = openxiii_mods::defaults::parse_mod_file(text, Path::new("test/mod.toml"), "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_defaults_here(file.overrides);
    vm.set_mod_edits_here(file.edits);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    (vm, pawn)
}

/// Health of the soldiers placed in the map (not the scripted Cine2 characters).
fn soldier_health(vm: &Vm) -> Vec<Value> {
    vm.actors
        .iter()
        .filter(|&&a| vm.objects[a].as_ref().is_some_and(|o| vm.classes[o.class].name == "BaseSoldier"))
        .map(|&a| vm.prop(a, "Health"))
        .collect()
}

fn has(vm: &Vm, pawn: ObjId, class: &str) -> bool {
    let mut item = vm.prop(pawn, "Inventory").as_object();
    while let Some(i) = item {
        if vm.objects[i].as_ref().is_some_and(|o| vm.class_path(o.class).eq_ignore_ascii_case(class)) {
            return true;
        }
        item = vm.prop(i, "Inventory").as_object();
    }
    false
}

fn count(vm: &Vm, class: &str) -> usize {
    vm.actors.iter().filter(|&&a| vm.objects[a].as_ref().is_some_and(|o| vm.class_path(o.class).eq_ignore_ascii_case(class)) && !vm.native_prop(a, Field::DeleteMe).as_bool()).count()
}

#[test]
fn selectors_and_their_priority_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let table = |sel: &str, health: i32| format!("[map.\"{sel}\".class.\"XIDPawn.BaseSoldier\"]\nHealth = {health}\n");
    // Negative check: a selector for other maps changes nothing.
    let (plain, _) = plage01(&game, "");
    let base = soldier_health(&plain);
    assert!(!base.is_empty());
    let (other, _) = plage01(&game, &format!("{}{}", table("multi", 5), table("Hual*", 6)));
    assert_eq!(soldier_health(&other), base);

    // Each selector for Plage01, then a more precise one on top: the most precise wins.
    let mut text = String::new();
    for (sel, health) in [("*", 111), ("solo", 222), ("P*", 333), ("Plage*", 444), ("Plage01", 555)] {
        text.push_str(&table(sel, health));
        let (vm, _) = plage01(&game, &text);
        let h = soldier_health(&vm);
        assert!(h.iter().all(|v| *v == Value::Int(health)), "{sel}: {h:?}");
    }
    // The order of the tables in the file does not matter.
    let reversed = format!("{}{}{}", table("Plage01", 555), table("solo", 222), table("*", 111));
    assert!(soldier_health(&plage01(&game, &reversed).0).iter().all(|v| *v == Value::Int(555)));

    // Start weapons and replacements.
    let text = "[map.\"solo\".start]\nweapons = [\"XIII.Beretta\"]\n[map.\"multi\".start]\nweapons = [\"XIII.m16\"]\n\
                [map.\"*\".replace]\n\"XIII.MedPick\" = \"XIII.MagnumPick\"\n[map.Plage01.replace]\n\"XIII.MedPick\" = \"XIII.FusilChassePick\"\n";
    let medkits = count(&plain, "XIII.MedPick");
    assert!(medkits > 0);
    let (vm, pawn) = plage01(&game, text);
    assert!(has(&vm, pawn, "XIII.Beretta") && !has(&vm, pawn, "XIII.m16"));
    assert_eq!(count(&vm, "XIII.MedPick"), 0);
    assert_eq!(count(&vm, "XIII.MagnumPick"), count(&plain, "XIII.MagnumPick"));
    assert_eq!(count(&vm, "XIII.FusilChassePick"), count(&plain, "XIII.FusilChassePick") + medkits);
}

/// Banque01 started from save point `portal` ("" for the map's start) with the mod file `text`:
/// (vm, pawn).
fn banque01(game: &Path, text: &str, portal: &str) -> (Vm, ObjId) {
    banque01_cleaned(game, text, portal, false)
}

/// banque01(), and with `clean` the map entry's save point destroyed before the player arrives,
/// as a loaded checkpoint does (XIIISaveGameTrigger.CleanMap, ActorsToDestroy).
fn banque01_cleaned(game: &Path, text: &str, portal: &str, clean: bool) -> (Vm, ObjId) {
    let file = openxiii_mods::defaults::parse_mod_file(text, Path::new("test/mod.toml"), "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_defaults_here(file.overrides);
    vm.set_mod_edits_here(file.edits);
    vm.load_dir(&game.join("System")).unwrap();
    let mut url = crate::travel::start_url(&game.join("System"), "Banque01");
    url.portal = portal.to_string();
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Banque01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    if clean {
        let entry: Vec<ObjId> = vm.actors.iter().copied().filter(|&a| vm.is_a(a, "XIIISaveGameTrigger") && format!("{}", vm.prop(a, "TeleporterName")).contains("PlayerStart")).collect();
        assert_eq!(entry.len(), 1);
        let _ = vm.destroy_actor(entry[0]);
    }
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    (vm, pawn)
}

fn names(vm: &Vm) -> Vec<String> {
    vm.actors.iter().filter(|&&a| !vm.native_prop(a, Field::DeleteMe).as_bool()).filter_map(|&a| vm.objects[a].as_ref().map(|o| o.name.to_lowercase())).collect()
}

/// Checkpoint numbers of Banque01 (maps.md: 1 its start, 2 Teleporter_bricole, 3
/// Teleporter_otage), and changes made from a checkpoint or at one only.
#[test]
fn checkpoint_conditions_on_banque01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let (plain, _) = banque01(&game, "", "");
    assert_eq!(plain.mod_checkpoint_here(), 1);
    let pickup = plain
        .actors
        .iter()
        .copied()
        .find(|&a| plain.is_a(a, "Pickup") && !plain.propr(a, "bStatic").as_bool() && !plain.propr(a, "bNoDelete").as_bool())
        .and_then(|a| plain.objects[a].as_ref().map(|o| o.name.clone()))
        .expect("a pickup in Banque01");
    let text = format!(
        "[[map.Banque01.remove]]\nname = \"{pickup}\"\nfrom_checkpoint = 2\n\n[map.Banque01.checkpoint.3.start]\nweapons = [\"XIII.m16\"]\n\n[map.\"solo\".checkpoint.2.start]\nweapons = [\"XIII.MiniUzi\"]\n"
    );
    let mut seen = Vec::new();
    for (portal, number) in [("", 1), ("PlayerStart", 1), ("Teleporter_bricole", 2), ("teleporter_otage", 3)] {
        let (vm, pawn) = banque01(&game, &text, portal);
        assert_eq!(vm.mod_checkpoint_here(), number, "{portal}");
        let removed = !names(&vm).contains(&pickup.to_lowercase());
        let (m16, uzi) = (has(&vm, pawn, "XIII.m16"), has(&vm, pawn, "XIII.MiniUzi"));
        seen.push((portal, removed, m16, uzi));
        assert_eq!(removed, number >= 2, "{portal}: {pickup} removed {removed}");
        assert_eq!(m16, number == 3, "{portal}: m16");
        assert_eq!(uzi, number == 2, "{portal}: mini Uzi");
    }
    eprintln!("Banque01 (portal, removed, M16, Uzi): {seen:?}");
    // A loaded checkpoint destroys save points before the start weapons are given: the
    // number read when the level started still holds.
    let (vm, pawn) = banque01_cleaned(&game, &text, "Teleporter_otage", true);
    assert_eq!(vm.mod_checkpoint_here(), 3);
    assert!(has(&vm, pawn, "XIII.m16"), "M16 not given at checkpoint 3 after CleanMap");
}
