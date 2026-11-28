//! Mods build only: sounds and content from any level, on the real Plage01 (OXIII_GAME =
//! original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::content::LoadList;
use openxiii_mods::placement::{ActorValue, Edit, MapEdit, Source};
use openxiii_mods::{DefaultOverride, ModValue};

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

const SNIPER_FIRE: &str = "XIII.Beretta";
const SNIPER_SOUND: &str = "XIIIsound.Guns__SnipFire.SnipFire__hSnipFire";

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn source() -> Source {
    Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }
}

fn class_value(class: &str, property: &str, value: ModValue) -> DefaultOverride {
    DefaultOverride { class: class.into(), property: property.into(), index: None, value, mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1, map: None, apply_to_placed: false }
}

struct Mods {
    defaults: Vec<DefaultOverride>,
    edits: Vec<MapEdit>,
    loads: Vec<LoadList>,
}

/// Plage01 started with its sounds loaded, as the game does: (vm, player controller, pawn).
fn plage01(game: &Path, mods: Option<Mods>) -> (Vm, ObjId, ObjId) {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    if let Some(m) = mods {
        vm.set_mod_defaults_here(m.defaults);
        vm.set_mod_edits_here(m.edits);
        vm.set_mod_loads_here(m.loads);
    }
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
    (vm, pc, pawn)
}

fn loaded(vm: &mut Vm, path: &str) -> bool {
    let id = vm.audio_sound_ref(path).expect("sound of the game").cuuid;
    vm.audio.as_ref().is_some_and(|h| h.engine.registry.contains_key(&id))
}

/// Sounds of voice `code_mesh` that no loaded bank holds.
fn missing_voice(vm: &mut Vm, code_mesh: i32) -> usize {
    let list = vm.mod_voice_sounds(code_mesh);
    assert!(!list.is_empty(), "no sounds for voice {code_mesh}");
    list.iter().filter(|(_, id)| !vm.audio.as_ref().is_some_and(|h| h.engine.registry.contains_key(id))).count()
}

#[test]
fn own_texture_as_a_value() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut t = std::collections::HashMap::new();
    t.insert("Mod.TestCross".to_string(), std::sync::Arc::new(vec![(1, 1, vec![255, 0, 0, 255])]));
    openxiii_assets::mod_textures::set(t);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_defaults_here(vec![
        class_value("XIII.Beretta", "CrossHair", ModValue::Str("Mod.TestCross".into())),
        class_value("XIII.Magnum", "CrossHair", ModValue::Str("Mod.NoSuchTexture".into())),
    ]);
    vm.load_dir(&game.join("System")).unwrap();
    let c = vm.class("XIII.Beretta").unwrap();
    vm.create_cdo(c);
    assert_eq!(vm.prop(vm.classes[c].cdo, "CrossHair"), Value::Asset("Mod.TestCross".into()));
    let m = vm.class("XIII.Magnum").unwrap();
    vm.create_cdo(m);
    assert!(vm.mod_messages.iter().any(|x| x.contains("Mod.NoSuchTexture is not an object of the game's packages")), "{:?}", vm.mod_messages);
}

#[test]
fn sounds_and_voices_from_other_levels() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Without mods: the sniper's firing sound and Carrington's voice (CodeMesh 16) are not on
    // Plage01, and nothing extra is loaded.
    let (mut plain, _, _) = plage01(&game, None);
    assert!(!loaded(&mut plain, SNIPER_SOUND), "the sniper sound is already on Plage01: the test proves nothing");
    let plain_missing = missing_voice(&mut plain, 16);
    assert!(plain_missing > 0);
    assert!(plain.mod_banks.is_empty());

    let mods = Mods {
        defaults: vec![
            class_value(SNIPER_FIRE, "hFireSound", ModValue::Str(format!("Sound'{SNIPER_SOUND}'"))),
            class_value("XIII.Beretta", "hReloadSound", ModValue::Str("XIIIsound.Nope.NoSuchSound".into())),
        ],
        edits: vec![MapEdit {
            map: "Plage01".into(),
            edit: Edit::Add {
                class: "XIDPawn.BaseSoldier".into(),
                position: [-242.0, -808.0, 1121.0],
                yaw: None,
                properties: vec![
                    ActorValue { property: "CodeMesh".into(), index: None, value: ModValue::Int(16) },
                    ActorValue { property: "Mesh".into(), index: None, value: ModValue::Str("XIIIPersos.CarringtonM".into()) },
                ],
            },
            source: source(), checkpoint: Default::default(),
        }],
        loads: vec![LoadList { map: Some("Plage01".into()), sound_banks: vec!["NoSuchBank".into()], packages: vec![], preload: vec![], source: source() }],
    };
    let (mut vm, _, pawn) = plage01(&game, Some(mods));
    let messages = vm.mod_messages.join("\n");
    eprintln!("{messages}");

    // The 9mm's firing sound is now the sniper's, and its resource came with a bank.
    let class = vm.class("Beretta").unwrap();
    vm.create_cdo(class);
    assert_eq!(vm.prop(vm.classes[class].cdo, "hFireSound"), Value::Asset(SNIPER_SOUND.into()));
    assert!(loaded(&mut vm, SNIPER_SOUND), "{messages}");
    // In the player's hands: the shot plays the sniper sound.
    if let Some(f) = vm.find_function(pawn, "GiveWeapon", false) {
        vm.call(f, pawn, vec![Some(Value::Str("XIII.Beretta".into()))]).unwrap();
    }
    let mut item = vm.prop(pawn, "Inventory").as_object();
    let mut beretta = None;
    while let Some(i) = item {
        if vm.objects[i].as_ref().is_some_and(|o| o.class == class) {
            beretta = Some(i);
        }
        item = vm.prop(i, "Inventory").as_object();
    }
    let beretta = beretta.expect("no Beretta given");
    let Value::Asset(fire) = vm.prop(beretta, "hFireSound") else { panic!("hFireSound") };
    let id = vm.audio_sound_ref(&fire).unwrap().cuuid;
    assert_eq!(id, vm.audio_sound_ref(SNIPER_SOUND).unwrap().cuuid);
    // Loaded once: asking again loads nothing more.
    let banks = vm.mod_banks.len();
    assert!(banks >= 1, "{messages}");
    vm.mod_ensure_sound(SNIPER_SOUND, "again").unwrap();
    assert_eq!(vm.mod_banks.len(), banks);

    // The added character speaks with Carrington's voice: all its sounds are loaded now.
    assert_eq!(missing_voice(&mut vm, 16), 0, "{messages}");
    let visitor = vm.actors.iter().copied().find(|&a| vm.is_a(a, "BaseSoldier") && vm.prop(a, "CodeMesh") == Value::Int(16)).expect("no visitor");
    assert_eq!(vm.native_value(visitor, Field::Mesh).to_string().to_lowercase().contains("carringtonm"), true);

    // Unknown names: one warning each, no stop.
    assert!(messages.contains("NoSuchSound is not an object of the game's packages"), "{messages}");
    assert!(messages.contains("sound bank NoSuchBank could not be loaded"), "{messages}");
    for (bank, ms, size) in &vm.mod_banks {
        eprintln!("bank {bank}: {ms:.1} ms, {} KB", size / 1024);
    }

    // Level change: the banks loaded for the mods leave with the map's.
    vm.audio_clean_up();
    assert!(vm.mod_banks.is_empty());
    assert!(!loaded(&mut vm, SNIPER_SOUND));

    // A key bound to a sound: its bank comes back when the key is pressed.
    let pc = vm.native_prop(pawn, Field::Controller).as_object().unwrap();
    let action = openxiii_mods::behaviour::ModAction {
        key: "J".into(),
        kind: openxiii_mods::behaviour::ActionKind::Sound { sound: SNIPER_SOUND.into() },
        source: source(),
    };
    vm.set_mod_behaviour_here(vec![action], vec![]);
    assert!(vm.mod_key(pc, "J"));
    assert!(loaded(&mut vm, SNIPER_SOUND));
    assert_eq!(vm.mod_banks.len(), 1);
}

/// The game classes a mod brings keep their own sounds: beach-weapons's sniper rifle and the
/// magnums replacing the knives are heard on Plage01, whose bank lacks them; without the mod no
/// bank is loaded. Select level's test weapons use the same loading (load_class_sounds_named).
#[test]
fn sounds_of_the_classes_a_mod_brings() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let sounds = |vm: &mut Vm, class: &str| -> Vec<String> {
        let c = vm.class(class).unwrap();
        vm.create_cdo(c);
        let cdo = vm.classes[c].cdo;
        ["hFireSound", "hReloadSound"].iter().filter_map(|p| match vm.prop(cdo, p) { Value::Asset(s) => Some(s), _ => None }).collect()
    };
    let (mut plain, _, _) = plage01(&game, None);
    let sniper = sounds(&mut plain, "XIII.FusilSnipe");
    assert!(!sniper.is_empty());
    assert!(sniper.iter().any(|s| !loaded(&mut plain, s)), "the sniper's sounds are already on Plage01: the test proves nothing");
    assert!(plain.mod_banks.is_empty());

    let mods = Mods {
        defaults: vec![],
        edits: vec![
            MapEdit {
                map: "Plage01".into(),
                edit: Edit::Add { class: "XIII.FusilSnipePick".into(), position: [-450.0, -912.0, 1190.0], yaw: None, properties: vec![] },
                source: source(), checkpoint: Default::default(),
            },
            MapEdit {
                map: "Plage01".into(),
                edit: Edit::Replace { from: "XIII.TKnifePick".into(), to: "XIII.MagnumPick".into() },
                source: source(), checkpoint: Default::default(),
            },
        ],
        loads: vec![],
    };
    let (mut vm, _, _) = plage01(&game, Some(mods));
    let messages = vm.mod_messages.join("\n");
    eprintln!("{messages}");
    for class in ["XIII.FusilSnipe", "XIII.Magnum"] {
        for s in sounds(&mut vm, class) {
            assert!(loaded(&mut vm, &s), "{class}: {s} not loaded\n{messages}");
        }
    }
    // The rifle's shot comes with its bank (named after the class that names it first).
    assert!(messages.contains("sound bank Hual04a loaded for XIII.FusilSnipe"), "{messages}");
    // A pickup's respawn sound is only heard in multiplayer: no bank for it in solo.
    assert!(!messages.contains("hRespawnSound"), "{messages}");

    // Select level: test weapons given on a level without their sounds.
    let (mut sel, _, _) = plage01(&game, None);
    assert!(sel.load_class_sounds_named(&["XIII.FusilSnipe"]) >= 1);
    for s in sounds(&mut sel, "XIII.FusilSnipe") {
        assert!(loaded(&mut sel, &s), "{s}");
    }
}

/// Select level's test weapons keep their sounds after the map's sounds are unloaded and loaded
/// again (next level, checkpoint reload): their banks leave with the map's own and come back
/// while a player still carries the weapon; a weapon no longer carried brings nothing back.
#[test]
fn select_level_weapons_keep_their_sounds_after_a_reload() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let sounds = |vm: &mut Vm, class: &str| -> Vec<String> {
        let c = vm.class(class).unwrap();
        vm.create_cdo(c);
        let cdo = vm.classes[c].cdo;
        ["hFireSound", "hReloadSound"].iter().filter_map(|p| match vm.prop(cdo, p) { Value::Asset(s) => Some(s), _ => None }).collect()
    };
    let (mut vm, _, pawn) = plage01(&game, None);
    let sniper = sounds(&mut vm, "XIII.FusilSnipe");
    assert!(sniper.iter().any(|s| !loaded(&mut vm, s)), "the sniper's sounds are already on Plage01: the test proves nothing");
    // Given as Select level does (the weapon in the player's inventory, its banks loaded).
    let class = vm.class("XIII.FusilSnipe").unwrap();
    let weapon = vm.spawn(class);
    vm.set_prop(weapon, "Owner", Value::Object(Some(pawn)));
    let first = vm.prop(pawn, "Inventory");
    vm.set_prop(weapon, "Inventory", first);
    vm.set_prop(pawn, "Inventory", Value::Object(Some(weapon)));
    assert!(vm.load_class_sounds_named(&["XIII.FusilSnipe"]) >= 1);
    for s in &sniper {
        assert!(loaded(&mut vm, s), "{s} before the reload");
    }
    // The map's sounds unloaded (LoadMap's CleanUp, KillAllSounds) and loaded again.
    vm.audio_clean_up();
    for s in &sniper {
        assert!(!loaded(&mut vm, s), "{s} still there after CleanUp: the test proves nothing");
    }
    vm.audio_load_sounds();
    for s in &sniper {
        assert!(loaded(&mut vm, s), "{s} lost after the reload while the player still carries the sniper rifle");
    }
    // No longer carried: nothing comes back.
    let rest = vm.prop(weapon, "Inventory");
    vm.set_prop(pawn, "Inventory", rest);
    vm.audio_clean_up();
    vm.audio_load_sounds();
    assert!(sniper.iter().any(|s| !loaded(&mut vm, s)), "sniper sounds loaded although no player carries it");
}
