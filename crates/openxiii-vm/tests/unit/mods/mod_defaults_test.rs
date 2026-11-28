//! Mods build only: class default values set by mods, on the real game scripts
//! (OXIII_GAME = original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::DefaultOverride;

use crate::value::Value;
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// Overrides from a mod.toml text, as the viewer reads them.
fn overrides(text: &str) -> Vec<DefaultOverride> {
    let (o, p) = openxiii_mods::defaults::parse_class_tables(text, Path::new("test/mod.toml"), "test");
    assert!(p.is_empty(), "{p:?}");
    o
}

fn load(game: &Path, list: Vec<DefaultOverride>) -> Vm {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_defaults_here(list);
    vm.load_dir(&game.join("System")).unwrap();
    vm
}

fn cdo(vm: &mut Vm, class: &str) -> usize {
    let c = vm.class(class).unwrap_or_else(|| panic!("class {class}"));
    vm.create_cdo(c);
    vm.classes[c].cdo
}

fn val(vm: &mut Vm, class: &str, prop: &str) -> String {
    let o = cdo(vm, class);
    format!("{}", vm.prop(o, prop))
}

const EXAMPLE: &str = r#"
[class."XIII.Beretta"]
ReloadCount = 17
ShotTime = 0.35
TraceAccuracy = 0.8

[class."XIII.c9mmAmmo"]
SRDamages = 35
MaxAmmo = 300
"#;

#[test]
fn example_mod_values_and_reload() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Negative check: without mods, the original values.
    let mut plain = load(&game, Vec::new());
    let (pb, pa) = (cdo(&mut plain, "Beretta"), cdo(&mut plain, "c9mmAmmo"));
    assert_eq!(plain.prop(pb, "ReloadCount"), Value::Byte(13));
    assert_eq!(plain.prop(pa, "SRDamages"), Value::Int(25));
    assert_eq!(plain.prop(pa, "MaxAmmo"), Value::Int(200));
    assert!(plain.mod_messages.is_empty());

    let mut vm = load(&game, overrides(EXAMPLE));
    let b = cdo(&mut vm, "Beretta");
    assert_eq!(vm.prop(b, "ReloadCount"), Value::Byte(17));
    assert_eq!(vm.prop(b, "ShotTime"), Value::Float(0.35));
    assert_eq!(vm.prop(b, "TraceAccuracy"), Value::Float(0.8));
    let a = cdo(&mut vm, "c9mmAmmo");
    assert_eq!(vm.prop(a, "SRDamages"), Value::Int(35));
    assert_eq!(vm.prop(a, "MaxAmmo"), Value::Int(300));
    // Untouched values stay the original ones.
    assert_eq!(vm.prop(b, "PickupAmmoCount"), Value::Int(13));
    assert_eq!(vm.prop(a, "LRDamages"), Value::Int(10));
    assert!(vm.mod_messages.iter().any(|m| m.starts_with("XIII.Beretta.ReloadCount = 17")), "{:?}", vm.mod_messages);

    // Class defaults reloaded (level change): the mod values come back.
    let c = vm.class("Beretta").unwrap();
    vm.set_prop(b, "ReloadCount", Value::Byte(99));
    vm.reload_class_defaults(c);
    let b = vm.classes[c].cdo;
    assert_eq!(vm.prop(b, "ReloadCount"), Value::Byte(17));
    assert_eq!(vm.prop(b, "ShotTime"), Value::Float(0.35));
    let c = vm.class("c9mmAmmo").unwrap();
    vm.reload_class_defaults(c);
    assert_eq!(vm.prop(vm.classes[c].cdo, "SRDamages"), Value::Int(35));
}

/// Subclasses of `class` (by name), without mods: (name, Health).
fn subclass_health(vm: &mut Vm, class: &str) -> Vec<(String, Value)> {
    let root = vm.class(class).unwrap();
    let subs: Vec<usize> = (0..vm.classes.len()).filter(|&c| c != root && vm.is_child_of(c, root)).collect();
    subs.into_iter()
        .map(|c| {
            vm.create_cdo(c);
            let o = vm.classes[c].cdo;
            (vm.classes[c].name.clone(), vm.prop(o, "Health"))
        })
        .collect()
}

#[test]
fn character_class_defaults_and_subclasses() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut plain = load(&game, Vec::new());
    let base = val(&mut plain, "BaseSoldier", "Health");
    let plain_subs = subclass_health(&mut plain, "BaseSoldier");
    assert_eq!(val(&mut plain, "XIDPawn.Johansson", "Health"), "2000");
    let mut vm = load(&game, overrides("[class.\"XIDPawn.BaseSoldier\"]\nHealth = 222\nTempsVisee = 0.25\n"));
    assert_eq!(val(&mut vm, "BaseSoldier", "Health"), "222");
    let soldier = cdo(&mut vm, "BaseSoldier");
    assert_eq!(vm.prop(soldier, "TempsVisee"), Value::Float(0.25));
    // Subclasses that do not set Health themselves inherit the new value; the others keep theirs.
    let subs = subclass_health(&mut vm, "BaseSoldier");
    let (mut inherited, mut kept) = (0, 0);
    for ((name, before), (_, after)) in plain_subs.iter().zip(&subs) {
        if format!("{before}") == base {
            assert_eq!(*after, Value::Int(222), "{name}");
            inherited += 1;
        } else {
            assert_eq!(after, before, "{name}");
            kept += 1;
        }
    }
    assert!(inherited > 0 && kept > 0, "inherited {inherited}, kept {kept}");
    assert_eq!(val(&mut vm, "XIDPawn.Johansson", "Health"), "2000");
}

/// Plage01 started (actors placed and BeginPlay run), with `list`.
fn plage01(game: &Path, list: Vec<DefaultOverride>) -> Vm {
    let mut vm = load(game, list);
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    vm
}

/// Soldiers placed in the map: (name, class name, Health).
fn soldiers(vm: &Vm) -> Vec<(String, String, Value)> {
    let mut out: Vec<_> = vm
        .actors
        .iter()
        .filter(|&&a| vm.is_a(a, "BaseSoldier"))
        .filter_map(|&a| vm.objects[a].as_ref().map(|o| (o.name.clone(), vm.classes[o.class].name.clone(), vm.prop(a, "Health"))))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn placed_characters_and_map_values() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let plain = soldiers(&plage01(&game, Vec::new()));
    assert!(plain.len() >= 3, "{plain:?}");
    let class_default = {
        let mut vm = load(&game, Vec::new());
        val(&mut vm, "BaseSoldier", "Health")
    };
    // At least one soldier carries its own health saved in the map.
    assert!(plain.iter().any(|(_, _, h)| format!("{h}") != class_default), "{plain:?}");

    // Class value only: soldiers with their own value in the map keep it.
    let class_only = soldiers(&plage01(&game, overrides("[class.\"XIDPawn.BaseSoldier\"]\nHealth = 222\n")));
    for ((name, _, before), (_, _, after)) in plain.iter().zip(&class_only) {
        if format!("{before}") == class_default {
            assert_eq!(*after, Value::Int(222), "{name}");
        } else {
            assert_eq!(after, before, "{name}");
        }
    }

    // apply_to_placed reaches all of them; the map value wins for its class (Cine2 here).
    let text = "[class.\"XIDPawn.BaseSoldier\"]\nHealth = 222\napply_to_placed = true\n\n[map.Plage01.class.\"XIDCine.Cine2\"]\nHealth = 333\n\n[map.Bateau01.class.\"XIDPawn.BaseSoldier\"]\nHealth = 1\n";
    let mut vm = plage01(&game, overrides(text));
    let all = soldiers(&vm);
    assert!(all.iter().any(|(_, c, _)| c == "Cine2") && all.iter().any(|(_, c, _)| c != "Cine2"), "{all:?}");
    for (name, class, health) in &all {
        let expected = if class == "Cine2" { 333 } else { 222 };
        assert_eq!(*health, Value::Int(expected), "{name} ({class})");
    }
    let messages = vm.mod_messages.join("\n");
    assert!(messages.contains("Plage01: XIDCine.Cine2.Health = 333 on"), "{messages}");
    assert!(!messages.contains("Health = 1 "), "{messages}");
    vm.report_mod_classes();
}

#[test]
fn value_kinds_and_warnings() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = r#"
[class."XIII.Beretta"]
FireOffset = { X = 20.0 }
WeaponMode = "WM_Auto"
bUseSilencer = false
ItemName = "Big 9mm"
NoSuchProperty = 1
ReloadCount = 1.5
"FireOffset[1]" = { X = 1.0 }

[class."XIII.NoSuchClass"]
A = 1
"#;
    let mut plain = load(&game, Vec::new());
    let orig_fire = val(&mut plain, "Beretta", "FireOffset");
    let mut vm = load(&game, overrides(text));
    let fire = val(&mut vm, "Beretta", "FireOffset");
    assert_ne!(fire, orig_fire);
    // Only X changes: Y and Z stay 5 and -4.
    assert_eq!(fire, orig_fire.replacen("9", "20", 1), "{orig_fire} -> {fire}");
    let b = cdo(&mut vm, "Beretta");
    let mode = vm.prop(b, "WeaponMode");
    let c = vm.class("Beretta").unwrap();
    let slot = vm.classes[c].prop_index["weaponmode"];
    let e = vm.classes[c].props[slot].enumeration.clone().expect("WeaponMode enum");
    let names = vm.enum_names(&e);
    if let (Value::Byte(m), Some(names)) = (&mode, names) {
        assert_eq!(names[*m as usize], "WM_Auto");
    } else {
        panic!("WeaponMode = {mode:?}");
    }
    assert_eq!(vm.prop(b, "bUseSilencer"), Value::Bool(false));
    assert_eq!(format!("{}", vm.prop(b, "ItemName")), format!("{}", Value::Str("Big 9mm".into())));
    // Wrong type and unknown property: warning, original value kept.
    assert_eq!(vm.prop(b, "ReloadCount"), Value::Byte(13));
    let all = vm.mod_messages.join("\n");
    assert!(all.contains("has no property NoSuchProperty"), "{all}");
    assert!(all.contains("ReloadCount: expects a byte"), "{all}");
    assert!(all.contains("FireOffset is not a static array"), "{all}");
    assert!(all.contains("test/mod.toml line"), "{all}");
    vm.report_mod_classes();
    let all = vm.mod_messages.join("\n");
    assert!(all.contains("unknown class XIII.NoSuchClass"), "{all}");
}
