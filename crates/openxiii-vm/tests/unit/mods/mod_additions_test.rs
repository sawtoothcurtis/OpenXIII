//! Mods build only: what a mod adds is added beside the game's own things, on the real Plage01
//! (OXIII_GAME = original game folder; skipped when it is not set). The example Blaster
//! (examples/mods/blaster-gun, a new weapon made from the .44) is picked up with and without a 9mm.

use std::path::{Path, PathBuf};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn blaster_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/blaster-gun/mod.toml")
}

/// Plage01 started with the mod files `texts` (path, text), the player logged in: (vm, pawn).
fn plage01(game: &Path, texts: &[(PathBuf, String)]) -> (Vm, ObjId) {
    let (mut classes, mut overrides, mut edits, mut actions, mut characters) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (path, text) in texts {
        let file = openxiii_mods::defaults::parse_mod_file(text, path, "test");
        assert!(file.problems.is_empty(), "{:?}", file.problems);
        classes.extend(file.new_classes);
        overrides.extend(file.overrides);
        edits.extend(file.edits);
        actions.extend(file.actions);
        characters.extend(file.characters);
    }
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(classes);
    vm.set_mod_defaults_here(overrides);
    vm.set_mod_edits_here(edits);
    vm.set_mod_behaviour_here(actions, characters);
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

fn class_path(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default()
}

/// The pawn's inventory: (object, class, ItemName, InventoryGroup).
fn inventory(vm: &Vm, pawn: ObjId) -> Vec<(ObjId, String, String, String)> {
    let mut out = Vec::new();
    let mut item = vm.prop(pawn, "Inventory").as_object();
    while let Some(i) = item {
        out.push((i, class_path(vm, i), format!("{}", vm.prop(i, "ItemName")), format!("{}", vm.prop(i, "InventoryGroup"))));
        item = vm.prop(i, "Inventory").as_object();
        if out.len() > 64 {
            break;
        }
    }
    out
}

/// The player stands on the Blaster's pickup until it is gone (at most 20 s).
fn walk_onto_blaster(vm: &mut Vm, pawn: ObjId) -> bool {
    let Some(pick) = vm.actors.iter().copied().find(|&a| class_path(vm, a).eq_ignore_ascii_case("Mod.BlasterPick")) else { return false };
    let at = vm.native_value(pick, Field::Location).as_vector();
    for frame in 0..(20 * 30) {
        if frame % 30 == 0 {
            let mut args = [Value::vector(at[0], at[1], at[2] + 40.0)];
            let _ = crate::natives::call(vm, "SetLocation", pawn, &mut args, &[true]);
        }
        vm.tick(1.0 / 30.0);
        if vm.objects[pick].is_none() || vm.native_prop(pick, Field::DeleteMe).as_bool() || vm.objects[pick].as_ref().is_some_and(|o| o.state.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("Sleeping"))) {
            return true;
        }
    }
    false
}

#[test]
fn blaster_is_a_weapon_of_its_own() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let blaster = (blaster_file(), std::fs::read_to_string(blaster_file()).unwrap());
    // The player also carries the throwing knives, whose slot is 1 (Weapon.uc's default).
    let nine = (PathBuf::from("start9/mod.toml"), "[map.Plage01.start]\nweapons = [\"XIII.Beretta\", \"XIII.TKnife\"]\n".to_string());
    let knife_only = (PathBuf::from("knife/mod.toml"), "[map.Plage01.start]\nweapons = [\"XIII.TKnife\"]\n".to_string());
    for with_9mm in [false, true] {
        let texts = vec![blaster.clone(), if with_9mm { nine.clone() } else { knife_only.clone() }];
        let (mut vm, pawn) = plage01(&game, &texts);
        let before = inventory(&vm, pawn);
        let picked = walk_onto_blaster(&mut vm, pawn);
        let after = inventory(&vm, pawn);
        eprintln!("9mm before: {with_9mm}; picked: {picked}\n before: {before:?}\n after: {after:?}");
        assert!(picked);
        // The Blaster is a weapon of its own, with its own ammo, beside the 9mm and the knives.
        let find = |c: &str| after.iter().find(|x| x.1.eq_ignore_ascii_case(c)).cloned();
        let (blaster, _, name, slot) = find("Mod.Blaster").expect("Blaster in the inventory");
        assert_eq!(name, format!("{}", Value::Str("Blaster".into())));
        assert!(find("Mod.BlasterAmmo").is_some());
        assert_eq!(find("XIII.Beretta").is_some(), with_9mm);
        let (knife, _, _, knife_slot) = find("XIII.TKnife").expect("knives");
        // Its own slot: the first free one (18); the knives keep 1, the 9mm 2.
        assert_eq!((slot.as_str(), knife_slot.as_str()), ("18", "1"), "{after:?}");
        let log = vm.mod_messages.join("\n");
        assert!(log.contains("Mod.Blaster: weapon slot 18 (InventoryGroup) of its own"), "{log}");
        // Chosen with its slot (SwitchWeapon → WeaponChange) and with the wheel (NextWeapon);
        // the knives with theirs.
        let first = vm.prop(pawn, "Inventory").as_object().unwrap();
        assert_eq!(vm.call_named(first, "WeaponChange", vec![Value::Byte(18)]).unwrap(), Value::Object(Some(blaster)));
        assert_eq!(vm.call_named(first, "WeaponChange", vec![Value::Byte(1)]).unwrap(), Value::Object(Some(knife)));
        let fists = find("XIII.Fists").unwrap().0;
        assert_eq!(vm.call_named(first, "NextWeapon", vec![Value::Object(None), Value::Object(Some(fists))]).unwrap(), Value::Object(Some(knife)));
        let last = if with_9mm { find("XIII.Beretta").unwrap().0 } else { knife };
        assert_eq!(vm.call_named(first, "NextWeapon", vec![Value::Object(None), Value::Object(Some(last))]).unwrap(), Value::Object(Some(blaster)));
        if with_9mm {
            let beretta = find("XIII.Beretta").unwrap().0;
            assert_eq!(vm.call_named(first, "WeaponChange", vec![Value::Byte(2)]).unwrap(), Value::Object(Some(beretta)));
        }
        // Its icon in the weapon list is chosen by weapon: the knives and the 9mm keep the sheet's
        // tiles; outside the pickup message (HudMessage), no tile of the sheet is replaced (the
        // items' icons share its places).
        let sheet = Value::Asset("XIIIMenu.HUD.Weapon1Icons".into());
        assert!(vm.mod_weapon_icon(knife, &sheet).is_none());
        assert!(vm.mod_hud_tile(&sheet, 128.0, 128.0, 64.0, 32.0).is_none(), "tile 18 drawn outside HudMessage");
    }
}

/// A new weapon with a HudIcon: the HUD draws that texture for it.
#[test]
fn new_weapon_hud_icon() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[classes.Gun2]\nparent = \"XIII.magnum\"\n[class.\"Mod.Gun2\"]\nHudIcon = \"XIIIMenu.HUD.MireBeretta\"\n[map.Plage01.start]\nweapons = [\"Mod.Gun2\"]\n";
    let (mut vm, pawn) = plage01(&game, &[(PathBuf::from("gun2/mod.toml"), text.to_string())]);
    let inv = inventory(&vm, pawn);
    let gun = inv.iter().find(|x| x.1 == "Mod.Gun2").unwrap_or_else(|| panic!("{inv:?}\n{:?}", vm.mod_messages));
    assert_eq!(gun.3, "18");
    let sheet = Value::Asset("XIIIMenu.HUD.Weapon1Icons".into());
    let (icon, uv) = vm.mod_weapon_icon(gun.0, &sheet).expect("icon");
    assert_eq!(icon, Value::Asset("XIIIMenu.HUD.MireBeretta".into()));
    assert!(uv[2] > 0.0 && uv[3] > 0.0, "{uv:?}");
    assert!(vm.mod_messages.iter().any(|m| m.starts_with("Mod.Gun2.HudIcon = ")), "{:?}", vm.mod_messages);
}

/// A character a mod adds without a look of its own is drawn as XIII (BaseSoldier's default
/// mesh): one warning; with a soldier's mesh, none.
#[test]
fn added_character_look() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let soldier = |props: &str| format!("[[map.Plage01.add]]\nclass = \"XIDPawn.BaseSoldier\"\nposition = [-242, -808, 1121]\nyaw = 180\n{props}");
    let (vm, _) = plage01(&game, &[(PathBuf::from("a/mod.toml"), soldier(""))]);
    let log = vm.mod_messages.join("\n");
    assert!(log.contains("it is drawn with XIII's own look (XIDPawn.BaseSoldier)"), "{log}");
    let (vm, _) = plage01(&game, &[(PathBuf::from("a/mod.toml"), soldier("properties = { Mesh = \"XIIIPersos.SpadsM\" }\n"))]);
    let log = vm.mod_messages.join("\n");
    assert!(!log.contains("XIII's own look"), "{log}");
    let added = vm.actors.iter().copied().filter(|&a| class_path(&vm, a) == "XIDPawn.BaseSoldier").find(|&a| vm.prop(a, "Mesh") == Value::Asset("XIIIPersos.SpadsM".into()));
    assert!(added.is_some(), "{log}");
}

