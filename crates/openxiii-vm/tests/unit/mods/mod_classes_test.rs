//! Mods build only: the mods' new classes ("Mod.<Name>") on the real Plage01 (OXIII_GAME =
//! original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::new_classes::NewClass;
use openxiii_mods::placement::{ActorValue, Edit, MapEdit, Source};
use openxiii_mods::{DefaultOverride, ModValue};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn source() -> Source {
    Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }
}

fn new_class(name: &str, parent: &str) -> NewClass {
    NewClass { name: name.into(), parent: parent.into(), source: source() }
}

fn value(class: &str, property: &str, value: ModValue) -> DefaultOverride {
    DefaultOverride { class: class.into(), property: property.into(), index: None, value, mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1, map: None, apply_to_placed: false }
}

/// Plage01 started with these new classes, class values and map changes: (vm, pawn).
fn plage01(game: &Path, classes: Vec<NewClass>, values: Vec<DefaultOverride>, edits: Vec<MapEdit>) -> (Vm, ObjId) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(classes);
    let shot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/blaster-gun/sounds/blaster-shot.wav");
    let problems = vm.set_mod_own_sounds_here(vec![openxiii_mods::own_sounds::OwnSound { name: "TestShot".into(), file: shot, replace: None, source: source() }]);
    assert!(problems.is_empty(), "{problems:?}");
    vm.set_mod_defaults_here(values);
    vm.set_mod_edits_here(edits);
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

fn cdo_value(vm: &mut Vm, class: &str, property: &str) -> Value {
    let c = vm.class(class).unwrap_or_else(|| panic!("class {class}"));
    vm.create_cdo(c);
    vm.prop(vm.classes[c].cdo, property)
}

#[test]
fn new_weapon_class_behaves_as_its_parent_with_its_own_values() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: skipped");
        return;
    };
    let classes = vec![
        new_class("TestPistol", "XIII.Beretta"),
        new_class("TestPistolPick", "XIII.BerettaPick"),
        // Refused: unknown parent, name of a game class.
        new_class("NoParent", "XIII.NoSuchClass"),
        new_class("Magnum", "XIII.Beretta"),
    ];
    let values = vec![
        // On the parent: inherited by the new class.
        value("XIII.Beretta", "ShotTime", ModValue::Float(0.25)),
        // Its own values.
        value("Mod.TestPistol", "ReloadCount", ModValue::Int(21)),
        // Weapon.GiveAmmo keeps at most PickupAmmoCount in the magazine.
        value("Mod.TestPistol", "PickupAmmoCount", ModValue::Int(21)),
        value("Mod.TestPistol", "ItemName", ModValue::Str("Test pistol".into())),
        value("Mod.TestPistol", "PickupClassName", ModValue::Str("Mod.TestPistolPick".into())),
        // A mod's own sound in a class value.
        value("Mod.TestPistol", "hFireSound", ModValue::Str("Mod.TestShot".into())),
        value("Mod.TestPistolPick", "InventoryType", ModValue::Str("Mod.TestPistol".into())),
    ];
    let edits = vec![MapEdit {
        map: "Plage01".into(),
        edit: Edit::Add { class: "Mod.TestPistolPick".into(), position: [-6643.0, -11524.0, 690.0], yaw: None, properties: vec![ActorValue { property: "PickupMessage".into(), index: None, value: ModValue::Str("Test".into()) }] },
        source: source(), checkpoint: Default::default(),
    }];
    let (mut vm, pawn) = plage01(&game, classes, values, edits);
    let messages = vm.mod_messages.join("\n");

    // A subclass of the Beretta named "Mod.TestPistol".
    let c = vm.class("Mod.TestPistol").unwrap_or_else(|| panic!("no class\n{messages}"));
    let beretta = vm.class("XIII.Beretta").unwrap();
    assert!(vm.is_child_of(c, beretta));
    assert_eq!(vm.class_path(c), "Mod.TestPistol");
    assert!(vm.class("Mod.NoParent").is_none());
    assert!(messages.contains("new class Mod.NoParent: XIII.NoSuchClass is not a class"), "{messages}");
    assert!(messages.contains("the game already has a class named Magnum"), "{messages}");
    assert!(vm.class("Magnum").is_some_and(|m| vm.class_path(m).eq_ignore_ascii_case("XIII.Magnum")));

    // Its values: its own, the parent's mod value, the rest from the parent.
    assert_eq!(cdo_value(&mut vm, "Mod.TestPistol", "ReloadCount"), Value::Byte(21));
    assert_eq!(cdo_value(&mut vm, "Mod.TestPistol", "ShotTime"), Value::Float(0.25));
    assert_eq!(cdo_value(&mut vm, "Mod.TestPistol", "ItemName"), Value::Str("Test pistol".into()));
    assert_eq!(cdo_value(&mut vm, "Mod.TestPistol", "hFireSound"), Value::Asset("Mod.TestShot".into()), "{messages}");
    for p in ["hReloadSound", "MeshName", "AmmoName", "AttachmentClass", "WeaponMode"] {
        assert_eq!(cdo_value(&mut vm, "Mod.TestPistol", p), cdo_value(&mut vm, "XIII.Beretta", p), "{p}");
    }
    assert_eq!(cdo_value(&mut vm, "XIII.Beretta", "ReloadCount"), Value::Byte(13));

    // The pickup placed by the map change carries the new weapon.
    let pick_class = vm.class("Mod.TestPistolPick").unwrap();
    let pick = vm.actors.clone().into_iter().find(|&a| vm.objects[a].as_ref().is_some_and(|o| o.class == pick_class)).unwrap_or_else(|| panic!("no pickup\n{messages}"));
    assert_eq!(vm.prop(pick, "InventoryType"), Value::Class(Some(c)));

    // The game's own GiveWeapon finds the class by its name and the weapon runs the Beretta's scripts.
    let f = vm.find_function(pawn, "GiveWeapon", false).expect("GiveWeapon");
    vm.call(f, pawn, vec![Some(Value::Str("Mod.TestPistol".into()))]).unwrap();
    let mut item = vm.prop(pawn, "Inventory").as_object();
    let mut weapon = None;
    while let Some(i) = item {
        if vm.objects[i].as_ref().is_some_and(|o| o.class == c) {
            weapon = Some(i);
        }
        item = vm.prop(i, "Inventory").as_object();
    }
    let weapon = weapon.unwrap_or_else(|| panic!("no weapon in the inventory\n{}", vm.log.join("\n")));
    assert_eq!(vm.prop(weapon, "ReloadCount"), Value::Byte(21));
    assert!(vm.find_function(weapon, "Fire", false).is_some());
}
