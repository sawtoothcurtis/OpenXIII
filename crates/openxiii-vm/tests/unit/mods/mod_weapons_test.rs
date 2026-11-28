//! Mods build only: weapon animation speeds set by mods (mod_weapons.rs), on the real Plage01
//! (OXIII_GAME = original game folder; skipped when it is not set): the time the player's 9mm
//! takes to fire again, to reload and to be put down, without mods and with the settings.

use std::path::{Path, PathBuf};

use openxiii_mods::{DefaultOverride, ModValue};

use crate::mod_weapons::{anim_kind, rate_value, AnimKind};
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn overrides(text: &str) -> Vec<DefaultOverride> {
    let (o, p) = openxiii_mods::defaults::parse_class_tables(text, Path::new("test/mod.toml"), "test");
    assert!(p.is_empty(), "{p:?}");
    o
}

#[test]
fn kinds_and_values() {
    assert_eq!(anim_kind("PlayFiring", None), Some(AnimKind::Fire));
    assert_eq!(anim_kind("playaltfiring", Some("NormalAltFire")), Some(AnimKind::Fire));
    assert_eq!(anim_kind("PlayReloading", None), Some(AnimKind::Reload));
    assert_eq!(anim_kind("BeginState", Some("WaitForSlaveReload")), Some(AnimKind::Reload));
    assert_eq!(anim_kind("BeginState", Some("Active")), None);
    assert_eq!(anim_kind("PlaySelect", None), Some(AnimKind::Select));
    assert_eq!(anim_kind("TweenDown", None), Some(AnimKind::Select));
    assert_eq!(anim_kind("PlayIdleAnim", None), None);
    assert_eq!(rate_value(&ModValue::Float(1.5)), Ok(1.5));
    assert_eq!(rate_value(&ModValue::Int(2)), Ok(2.0));
    assert!(rate_value(&ModValue::Float(0.0)).is_err());
    assert!(rate_value(&ModValue::Float(25.0)).is_err());
    assert!(rate_value(&ModValue::Str("fast".into())).is_err());
}

/// Plage01 started with `text`, the player logged in with the 9mm in hand and ready: (vm, pc, 9mm).
fn plage01_with_9mm(game: &Path, text: &str) -> (Vm, ObjId, ObjId) {
    plage01_with(game, text, "XIII.Beretta")
}

/// Plage01 started with the mod file `text` (class values, new classes), the player logged in
/// with a weapon of class `weapon` in hand and ready: (vm, pc, weapon).
fn plage01_with(game: &Path, text: &str, weapon_class: &str) -> (Vm, ObjId, ObjId) {
    let file = openxiii_mods::defaults::parse_mod_file(text, Path::new("test/mod.toml"), "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(file.new_classes);
    vm.set_mod_defaults_here(file.overrides);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let class = vm.class(weapon_class).unwrap_or_else(|| panic!("class {weapon_class}"));
    let _ = vm.call_named(pawn, "GiveWeapon", vec![Value::Str(weapon_class.into())]);
    let _ = vm.console_command_out(pc, &format!("getweapon {weapon_class}"));
    let mut weapon = None;
    for _ in 0..(10 * 60) {
        vm.tick(1.0 / 60.0);
        weapon = vm.native_prop(pawn, Field::Weapon).as_object().filter(|&w| vm.objects[w].as_ref().is_some_and(|o| o.class == class));
        if weapon.is_some_and(|w| state(&vm, w) == "idle") {
            break;
        }
    }
    let w = weapon.unwrap_or_else(|| panic!("no {weapon_class} in hand: {:?}, log {:?}", vm.native_prop(pawn, Field::Weapon), vm.mod_messages));
    assert_eq!(state(&vm, w), "idle", "{weapon_class} not ready");
    (vm, pc, w)
}

/// Rounds fired by the weapon in hand in `seconds`, the player holding the fire button.
fn rounds_held(vm: &mut Vm, pc: ObjId, w: ObjId, seconds: f32) -> i32 {
    let ammo = vm.prop(w, "AmmoType").as_object().expect("ammo");
    vm.set_prop(ammo, "AmmoAmount", Value::Int(100));
    let before = vm.prop(w, "ReloadCount").as_int();
    vm.set_prop(pc, "bFire", Value::Byte(1));
    vm.call_named(w, "Fire", vec![Value::Float(1.0)]).unwrap();
    for _ in 0..(seconds * 60.0) as usize {
        vm.tick(1.0 / 60.0);
    }
    vm.set_prop(pc, "bFire", Value::Byte(0));
    before - vm.prop(w, "ReloadCount").as_int()
}

/// Mesh drawn for the weapon in first person.
fn mesh(vm: &Vm, w: ObjId) -> String {
    format!("{}", vm.native_value(w, Field::Mesh))
}

#[test]
fn first_person_look_of_another_weapon() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let class = "[classes.Rifle9]\nparent = \"XIII.Beretta\"\n";
    // Negative check: a new class made from the 9mm looks like the 9mm.
    let (mut vm, _, w) = plage01_with(&game, class, "Mod.Rifle9");
    let plain_mesh = mesh(&vm, w);
    assert!(plain_mesh.contains("FpsBerrettaM"), "{plain_mesh}");
    let plain_offset = vm.prop(w, "PlayerViewOffset");
    vm.call_named(w, "Fire", vec![Value::Float(4.0)]).unwrap();
    let plain_fire = time_in(&mut vm, w, "normalfire");

    // With the M16's first-person look: its mesh and place in front of the eye, the 9mm's scripts.
    let text = format!("{class}[class.\"Mod.Rifle9\"]\nFirstPersonFrom = \"XIII.m16\"\nPlayerViewOffset = {{ Z = -7.0 }}\n");
    let (rifle, log) = timings_of(&game, &text, "Mod.Rifle9");
    let (mut vm, _, w) = plage01_with(&game, &text, "Mod.Rifle9");
    let m = mesh(&vm, w);
    assert!(m.contains("FpsM16M"), "{m}");
    assert_eq!(format!("{}", vm.prop(w, "MeshName")), format!("{}", Value::Str("XIIIArmes.FpsM16M".into())));
    // The mod's own value wins over the copied one (only Z changes, X and Y are the M16's).
    assert_eq!(vm.prop(w, "PlayerViewOffset"), Value::vector(6.0, 10.0, -7.0), "9mm: {plain_offset}");
    // Its behaviour is the 9mm's: one hand, 9mm ammo, a 13-round magazine.
    let ammo_class = match vm.prop(w, "AmmoName") {
        Value::Class(Some(c)) => vm.class_path(c),
        other => format!("{other:?}"),
    };
    assert_eq!(ammo_class, "XIII.c9mmAmmo");
    assert_eq!(vm.prop(w, "ReloadCount"), Value::Byte(13));
    // The 9mm's script animations play on the M16's mesh: firing, reload and put down end.
    vm.call_named(w, "Fire", vec![Value::Float(4.0)]).unwrap();
    let channels = vm.anim_channels(w);
    eprintln!("9mm fire {plain_fire} s; with the M16's look: {rifle:?}, channels {channels:?}");
    assert!(channels.first().is_some_and(|c| c.0.eq_ignore_ascii_case("Fire")), "{channels:?}");
    assert!(rifle.iter().all(|&t| t > 0.05 && t < 5.0), "{rifle:?}");
    let log = log.join("\n");
    assert!(log.contains("Mod.Rifle9.FirstPersonFrom = \"XIII.m16\": first-person mesh"), "{log}");
    assert!(!log.contains("first-person animation"), "{log}");

    // FirstPersonAnimations: the name the 9mm's script plays = an animation of the M16.
    let text = format!("{class}[class.\"Mod.Rifle9\"]\nFirstPersonFrom = \"XIII.m16\"\nFirstPersonAnimations = {{ Fire = \"FireGrenad\" }}\n");
    let (mut vm, _, w) = plage01_with(&game, &text, "Mod.Rifle9");
    vm.call_named(w, "Fire", vec![Value::Float(4.0)]).unwrap();
    let channels = vm.anim_channels(w);
    assert!(channels.first().is_some_and(|c| c.0.eq_ignore_ascii_case("FireGrenad")), "{channels:?}");

    // Wrong values: one warning each, the class keeps its look.
    let text = format!("{class}[class.\"Mod.Rifle9\"]\nFirstPersonFrom = \"XIII.c9mmAmmo\"\n[class.\"XIII.Beretta\"]\nFirstPersonFrom = \"XIII.BerettaPlus\"\n");
    let (vm, _, w) = plage01_with(&game, &text, "Mod.Rifle9");
    assert!(mesh(&vm, w).contains("FpsBerrettaM"), "{}", mesh(&vm, w));
    let log = vm.mod_messages.join("\n");
    assert!(log.contains("warning: Mod.Rifle9.FirstPersonFrom: XIII.c9mmAmmo is not a weapon class"), "{log}");
    assert!(log.contains("warning: XIII.Beretta.FirstPersonFrom: XIII.BerettaPlus is this class or one of its subclasses"), "{log}");
}

#[test]
fn fallback_animation_names() {
    use crate::mod_weapons::fallback_names;
    assert_eq!(fallback_names("FireLast"), ["Fire", "FireLast1", "Fire1"]);
    assert_eq!(fallback_names("WaitActVide"), ["WaitAct", "Wait", "WaitActVide1", "Wait1"]);
    assert_eq!(fallback_names("Fire2"), ["Fire"]);
    assert_eq!(fallback_names("Fire"), ["Fire1"]);
}

/// The game's own weapon values that change what the player does: WeaponMode (automatic,
/// semi-automatic, burst), FiringMode (the character's animations in third person), WHand (one
/// or two hands).
#[test]
fn weapon_mode_firing_mode_and_hands() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // WeaponMode, the fire button held for 1.5 s.
    let mut rounds = Vec::new();
    for mode in ["", "WM_Auto", "WM_Burst"] {
        let text = if mode.is_empty() { String::new() } else { format!("[class.\"XIII.Beretta\"]\nWeaponMode = \"{mode}\"\n") };
        let (mut vm, pc, w) = plage01_with_9mm(&game, &text);
        rounds.push(rounds_held(&mut vm, pc, w, 1.5));
    }
    eprintln!("rounds in 1.5 s: semi-automatic {}, automatic {}, burst {}", rounds[0], rounds[1], rounds[2]);
    assert_eq!(rounds[0], 1, "{rounds:?}");
    assert!(rounds[1] >= 4, "{rounds:?}");
    assert_eq!(rounds[2], 3, "{rounds:?}");

    // FiringMode: the player's character holds and fires the 9mm as an assault rifle. The
    // weapon's value gives the way it is held (XIIIPawn.SetFiringMode), its attachment's value the
    // firing, reload and weapon change animations (WeaponAttachment.ThirdPersonEffects).
    // The weapon's value alone is enough: its attachment gets it too, unless a mod sets the
    // attachment's own.
    let rifle = "[class.\"XIII.Beretta\"]\nFiringMode = \"FM_M16\"\n";
    let own = "[class.\"XIII.Beretta\"]\nFiringMode = \"FM_M16\"\n[class.\"XIII.BerettaAttach\"]\nFiringMode = \"FM_Bazook\"\n";
    for (text, expected) in [("", "FM_1H"), (rifle, "FM_M16"), (own, "FM_M16")] {
        let (mut vm, _, w) = plage01_with_9mm(&game, text);
        let pawn = vm.prop(w, "Owner").as_object().unwrap();
        assert_eq!(format!("{}", vm.prop(pawn, "WeaponMode")), expected);
        vm.call_named(w, "Fire", vec![Value::Float(4.0)]).unwrap();
        // The character's shot, as the engine's pawn tick starts it when the flash count changes.
        let attachment = vm.prop(w, "ThirdPersonActor").as_object().expect("attachment");
        vm.call_named(attachment, "ThirdPersonEffects", vec![]).unwrap();
        vm.tick(1.0 / 60.0);
        let channels = vm.anim_channels(pawn);
        eprintln!("{expected}: character animations {channels:?}");
        let shown: Vec<String> = channels.iter().map(|c| c.0.to_lowercase()).collect();
        if expected == "FM_1H" {
            assert!(shown.iter().any(|n| n == "pistoletretour"), "{channels:?}");
        } else if text == own {
            assert!(shown.iter().any(|n| n == "bazookaretour"), "{channels:?}");
        } else {
            assert!(shown.iter().any(|n| n == "gunretour" || n == "retourkalash"), "{channels:?}");
            let log = vm.mod_messages.join("\n");
            assert!(log.contains("XIII.Beretta.FiringMode = \"FM_M16\" also on its attachment XIII.BerettaAttach"), "{log}");
        }
    }

    // WHand: with something in the left hand, a two-handed weapon is skipped when choosing
    // the next weapon.
    for (text, two_hands) in [("", false), ("[class.\"XIII.Beretta\"]\nWHand = \"WHA_2HShot\"\n", true)] {
        let (mut vm, _, w) = plage01_with_9mm(&game, text);
        let pawn = vm.prop(w, "Owner").as_object().unwrap();
        let lhand = vm.prop(pawn, "LHand").as_object().expect("left hand");
        vm.set_prop(lhand, "bActive", Value::Bool(true));
        let next = vm.call_named(w, "NextWeapon", vec![Value::Object(None), Value::Object(None)]).unwrap();
        eprintln!("two hands {two_hands}: NextWeapon gives {next:?}");
        assert_eq!(next == Value::Object(Some(w)), !two_hands, "{next:?}");
    }
}

/// timings() for a weapon of class `weapon`.
fn timings_of(game: &Path, text: &str, weapon: &str) -> ([f32; 3], Vec<String>) {
    let (mut vm, _pc, w) = plage01_with(game, text, weapon);
    measure(&mut vm, w)
}

fn state(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().and_then(|o| o.state.clone()).unwrap_or_default().to_lowercase()
}

/// Seconds (at 60 frames per second) until `a` leaves state `s`, after it entered it.
fn time_in(vm: &mut Vm, a: ObjId, s: &str) -> f32 {
    assert_eq!(state(vm, a), s, "not in state {s}");
    for frame in 1..(10 * 60) {
        vm.tick(1.0 / 60.0);
        if state(vm, a) != s {
            return frame as f32 / 60.0;
        }
    }
    panic!("still in {s} after 10 s");
}

/// (time to fire again, time to reload, time to put the gun down) of the player's 9mm.
fn timings(game: &Path, text: &str) -> ([f32; 3], Vec<String>) {
    timings_of(game, text, "XIII.Beretta")
}

/// (time to fire again, time to reload, time to put the gun down) of weapon `w`, ready in hand.
fn measure(vm: &mut Vm, w: ObjId) -> ([f32; 3], Vec<String>) {
    let mut vm = vm;
    // Spare rounds, so that the magazine can be reloaded.
    let ammo = vm.prop(w, "AmmoType").as_object().expect("9mm ammo");
    vm.set_prop(ammo, "AmmoAmount", Value::Int(100));
    // One shot: the 9mm stays in NormalFire until its firing animation ends.
    vm.call_named(w, "Fire", vec![Value::Float(4.0)]).unwrap();
    let fire = time_in(&mut vm, w, "normalfire");
    for _ in 0..30 {
        vm.tick(1.0 / 60.0);
    }
    // Reload (the magazine is no longer full).
    vm.call_named(w, "ForceReload", vec![]).unwrap();
    let reload = time_in(&mut vm, w, "reloading");
    for _ in 0..30 {
        vm.tick(1.0 / 60.0);
    }
    // Put down: DownWeapon lasts until the put down animation ends.
    vm.call_named(w, "PutDown", vec![]).unwrap();
    for _ in 0..3 {
        if state(&vm, w) == "downweapon" {
            break;
        }
        vm.tick(1.0 / 60.0);
    }
    let down = time_in(&mut vm, w, "downweapon");
    ([fire, reload, down], vm.mod_messages.clone())
}

#[test]
fn animation_speeds_of_the_9mm() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    // Negative check: without the settings (and with them for another map only), the game's times.
    let (plain, plain_log) = timings(&game, "");
    assert!(plain_log.is_empty(), "{plain_log:?}");
    let (other_map, _) = timings(&game, "[map.Bateau01.class.\"XIII.Beretta\"]\nFireAnimRate = 3.0\n");
    assert_eq!(plain, other_map);
    eprintln!("without mods: fire again {:.3} s, reload {:.3} s, put down {:.3} s", plain[0], plain[1], plain[2]);
    assert!(plain.iter().all(|&t| t > 0.1), "{plain:?}");

    // Twice as fast to fire, three times to reload, half as fast to put down.
    let text = "[class.\"XIII.Beretta\"]\nFireAnimRate = 2.0\nReloadAnimRate = 3\nSelectAnimRate = 0.5\n";
    let (fast, log) = timings(&game, text);
    eprintln!("with the settings: {fast:?}");
    let near = |t: f32, expected: f32| (t - expected).abs() <= 2.0 / 60.0 + expected * 0.05;
    assert!(near(fast[0], plain[0] / 2.0), "fire {} instead of {}", fast[0], plain[0] / 2.0);
    assert!(near(fast[1], plain[1] / 3.0), "reload {} instead of {}", fast[1], plain[1] / 3.0);
    assert!(near(fast[2], plain[2] * 2.0), "put down {} instead of {}", fast[2], plain[2] * 2.0);
    let log = log.join("\n");
    assert!(log.contains("XIII.Beretta.FireAnimRate = 2.0 (mod test"), "{log}");

    // A map value wins on its map, over the class value.
    let text = "[class.\"XIII.Beretta\"]\nFireAnimRate = 3.0\n[map.Plage01.class.\"XIII.Beretta\"]\nFireAnimRate = 2.0\n";
    let (map_value, log) = timings(&game, text);
    assert!(near(map_value[0], plain[0] / 2.0), "map value: fire {} instead of {}", map_value[0], plain[0] / 2.0);
    assert!(log.iter().any(|l| l.starts_with("Plage01: XIII.Beretta.FireAnimRate = 2.0")), "{log:?}");

    // A selector of several maps ("solo") applies on Plage01, the map's own value wins over it.
    let (solo, _) = timings(&game, "[map.\"solo\".class.\"XIII.Beretta\"]\nFireAnimRate = 2.0\n");
    assert!(near(solo[0], plain[0] / 2.0), "solo: fire {} instead of {}", solo[0], plain[0] / 2.0);
    let (both, _) = timings(&game, "[map.Plage01.class.\"XIII.Beretta\"]\nFireAnimRate = 3.0\n[map.\"solo\".class.\"XIII.Beretta\"]\nFireAnimRate = 2.0\n");
    assert!(near(both[0], plain[0] / 3.0), "Plage01 over solo: fire {} instead of {}", both[0], plain[0] / 3.0);

    // Wrong values: one warning, the game's speed.
    let text = "[class.\"XIII.Beretta\"]\nFireAnimRate = 0\n[class.\"XIII.c9mmAmmo\"]\nReloadAnimRate = 2.0\n";
    let (bad, log) = timings(&game, text);
    assert_eq!(bad[0], plain[0]);
    let log = log.join("\n");
    assert!(log.contains("warning: XIII.Beretta.FireAnimRate: 0 is out of range"), "{log}");
    assert!(log.contains("warning: XIII.c9mmAmmo.ReloadAnimRate: XIII.c9mmAmmo is not a weapon class"), "{log}");
}

/// FirstPersonModel: the 9mm's hands with its gun shrunk to nothing, and another model attached
/// to the gun's bone, as the game attaches the 9mm's silencer.
#[test]
fn first_person_model_in_the_hand() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let class = "[classes.Held]\nparent = \"XIII.Beretta\"\n";
    // Negative check: without the setting, nothing is attached.
    let (vm, _, w) = plage01_with(&game, class, "Mod.Held");
    assert!(vm.mods.fp_models.get(&w).is_none());
    let text = format!(
        "{class}[class.\"Mod.Held\"]\nFirstPersonModel = \"MeshArmesPickup.sniper\"\nFirstPersonHideBones = [\"Berretta\", \"chargeur\", \"percuteur\"]\nFirstPersonModelScale = 0.5\nFirstPersonModelOffset = {{ X = 2.0 }}\n"
    );
    let (vm, _, w) = plage01_with(&game, &text, "Mod.Held");
    let log = vm.mod_messages.join("\n");
    let att = *vm.mods.fp_models.get(&w).unwrap_or_else(|| panic!("no model attached\n{log}"));
    assert_eq!(vm.prop(att, "StaticMesh"), Value::Asset("MeshArmesPickup.sniper".into()));
    assert_eq!(vm.prop(att, "DrawScale"), Value::Float(0.5));
    assert_eq!(vm.native_prop(att, Field::Base).as_object(), Some(w));
    assert!(format!("{}", vm.prop(att, "AttachmentBone")).eq_ignore_ascii_case("berretta"), "{}", vm.prop(att, "AttachmentBone"));
    assert!(log.contains("Mod.Held: first-person model MeshArmesPickup.sniper on bone"), "{log}");
    assert!(!log.contains("FirstPersonHideBones: no bone"), "{log}");
}

/// The front end of a model (mesh_front_of): the farthest point forward (+X) and the middle of
/// the points around it, whatever the model's other parts.
#[test]
fn front_end_of_a_model() {
    let pts = [[-10.0, 0.0, 0.0], [20.0, 1.0, 3.0], [20.0, -1.0, 5.0], [19.9, 0.0, 4.0], [0.0, 30.0, -20.0]];
    let f = crate::mod_weapons::mesh_front_of(&pts).unwrap();
    assert_eq!(f[0], 20.0);
    assert!((f[1] - 0.0).abs() < 1e-5 && (f[2] - 4.0).abs() < 1e-5, "{f:?}");
    assert!(crate::mod_weapons::mesh_front_of(&[]).is_none());
}

/// FirstPersonModel and the tracer drawn in first person (XIIIWeapon.DrawTraceBullet): the tile
/// is sized so that the end of its beam lands on the model's muzzle as the view projects it,
/// for several view sizes (4:3, wide, a split-screen half) and texture ends; a weapon without a
/// FirstPersonModel keeps the game's size (FlashOffsetX/Y). Negative control: the game's size
/// misses the muzzle.
#[test]
fn first_person_tracer_ends_at_the_muzzle() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let class = "[classes.Held]\nparent = \"XIII.Beretta\"\n";
    let text = format!("{class}[class.\"Mod.Held\"]\nFirstPersonModel = \"MeshArmesPickup.sniper\"\nFirstPersonHideBones = [\"Berretta\", \"chargeur\", \"percuteur\"]\nFirstPersonModelScale = 0.5\n");
    let (mut vm, pc, w) = plage01_with(&game, &text, "Mod.Held");
    let draw = vm.functions.iter().position(|f| f.name.eq_ignore_ascii_case("DrawTraceBullet")).expect("DrawTraceBullet");
    let canvas = vm.spawn(vm.class("Canvas").expect("Canvas"));
    let muzzle = vm.mod_first_person_muzzle(w).expect("a muzzle");
    // The muzzle is in front of the eyes, a little to the right and below.
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let mut eyes = vm.native_prop(pawn, Field::Location).as_vector();
    eyes[2] += vm.prop(pawn, "BaseEyeHeight").as_float();
    let d: Vec<f32> = (0..3).map(|k| muzzle[k] - eyes[k]).collect();
    assert!(d.iter().map(|x| x * x).sum::<f32>().sqrt() < 120.0, "muzzle {muzzle:?}, eyes {eyes:?}");
    let rot = vm.native_prop(pc, Field::Rotation).as_rotator();
    let tex = Value::Asset("XIIIArmes.TraceBullet1".into());
    let mut checked = 0;
    for (i, (w_px, h_px)) in [(640u32, 480u32), (1920, 1080), (1920, 540), (1280, 1024), (800, 600)].into_iter().enumerate() {
        for k in 0..25 {
            // A camera at the eyes, turned a little each time (the muzzle stays in view).
            let r = [rot[0] + (k as i32 - 12) * 40, rot[1] + (k as i32 % 5 - 2) * 60, 0];
            let fov = 70.0 + (k % 4) as f32 * 8.0 + i as f32;
            let node = vm.camera_scene_view(Some(pc), Some(pc), eyes, r, fov, (w_px, h_px));
            vm.mods.view_projection = Some((node.clone(), (w_px as i32, h_px as i32)));
            // Where the first-person projection (25° half angle, the same camera) puts the
            // muzzle: the camera's own frame, worked out from scratch.
            let ax = crate::ai_natives::axes(r);
            let rel: Vec<f32> = (0..3).map(|k| muzzle[k] - eyes[k]).collect();
            let dot = |a: [f32; 3]| (0..3).map(|k| a[k] * rel[k]).sum::<f32>();
            let (fwd, right, up) = (dot(ax[0]), dot(ax[1]), dot(ax[2]));
            assert!(fwd > 0.0, "muzzle in front of the camera");
            let s = vm.first_person_scale(w_px as f32, h_px as f32);
            let ndc = [right / fwd * s[0], up / fwd * s[1]];
            let want = [(ndc[0] + 1.0) * 0.5 * w_px as f32 + 0.5, (1.0 - ndc[1]) * 0.5 * h_px as f32 + 0.5];
            let start = [w_px as f32 / 2.0 + (k % 3) as f32 - 1.0, h_px as f32 / 2.0 - (k % 2) as f32];
            for (name, v) in [("ClipX", w_px as f32), ("ClipY", h_px as f32), ("CurX", start[0]), ("CurY", start[1])] {
                vm.set_prop(canvas, name, Value::Float(v));
            }
            let game_size = (w_px as f32 * 0.19, h_px as f32 * 0.29);
            vm.hud.clear();
            vm.stack.push(draw);
            vm.this_stack.push(w);
            vm.stack.push(draw);
            vm.this_stack.push(canvas);
            let (xl, yl) = vm.mod_tracer_size(canvas, &tex, game_size.0, game_size.1);
            vm.stack.truncate(vm.stack.len() - 2);
            vm.this_stack.truncate(vm.this_stack.len() - 2);
            // A game texture: its beam runs corner to corner.
            let end = [start[0] + xl, start[1] + yl];
            assert!((end[0] - want[0]).abs() < 0.05 && (end[1] - want[1]).abs() < 0.05, "{w_px}x{h_px} k {k}: ends at {end:?}, muzzle at {want:?}");
            // Placed again after the HUD (the weapon has not moved here): the same end.
            vm.hud.clear();
            vm.hud.push(crate::canvas::HudDraw::Tile { x1: start[0], y1: start[1], x2: 0.0, y2: 0.0, uv: [0.0; 4], material: "#canvas:4:0:127|XIIIArmes.TraceBullet1".into(), color: [255; 4], point: false, enlarged: false, glyph: false });
            vm.mod_tracer_fixup(canvas);
            let Some(crate::canvas::HudDraw::Tile { x2, y2, .. }) = vm.hud.first() else { panic!("tile") };
            assert!((x2 - want[0]).abs() < 0.05 && (y2 - want[1]).abs() < 0.05, "after the HUD: ({x2}, {y2}), muzzle at {want:?}");
            // The point the scene shows there (the flamethrower's flame): drawn by the scene's
            // own projection on the muzzle as the first-person projection draws it.
            vm.mods.views.insert(pc, (node.clone(), (w_px as i32, h_px as i32)));
            let seen = vm.first_person_seen(pc, muzzle);
            let q = node.project(seen);
            let at = [(q[0] + 1.0) * 0.5 * w_px as f32 + 0.5, (1.0 - q[1]) * 0.5 * h_px as f32 + 0.5];
            assert!((at[0] - want[0]).abs() < 1.0 && (at[1] - want[1]).abs() < 1.0, "seen at {at:?}, muzzle drawn at {want:?}");
            let ahead: f32 = (0..3).map(|k| (seen[k] - eyes[k]) * ax[0][k]).sum();
            assert!((ahead - fwd).abs() < 1.0, "as far ahead as the muzzle: {ahead} / {fwd}");
            let missed = [start[0] + game_size.0, start[1] + game_size.1];
            assert!((missed[0] - want[0]).abs() + (missed[1] - want[1]).abs() > 5.0, "negative control: the game's size already lands there");
            checked += 1;
        }
    }
    assert_eq!(checked, 125);
    // The first-person angles of a 4:3 view: 25° across half of it, 18.8° up.
    let tan = 25f32.to_radians().tan();
    let s = vm.first_person_scale(640.0, 480.0);
    assert!((s[0] - 1.0 / tan).abs() < 1e-3 && (s[1] - 1.0 / (tan * 0.75)).abs() < 1e-3, "{s:?}");
    // Not called by DrawTraceBullet: unchanged.
    assert_eq!(vm.mod_tracer_size(canvas, &tex, 10.0, 20.0), (10.0, 20.0));
    // A weapon without a FirstPersonModel: the game's size.
    let (mut vm, _, w) = plage01_with(&game, class, "Mod.Held");
    let canvas = vm.spawn(vm.class("Canvas").expect("Canvas"));
    vm.stack.push(draw);
    vm.this_stack.push(w);
    vm.stack.push(draw);
    vm.this_stack.push(canvas);
    assert_eq!(vm.mod_tracer_size(canvas, &tex, 10.0, 20.0), (10.0, 20.0));
}

/// A mod's attachment class made from a game one (the Blaster's from the .44's, MagnumAttach)
/// shows its own StaticMeshName, also after a gun of the game class was seen in a hand: that
/// one's InventoryAttachment.PostBeginPlay writes its mesh into its class's defaults.
#[test]
fn mod_attachment_keeps_its_own_model() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[classes.HeldAttach]\nparent = \"XIII.MagnumAttach\"\n[class.\"Mod.HeldAttach\"]\nStaticMeshName = \"MeshArmesPickup.sniper\"\n";
    let (mut vm, pc, _) = plage01_with(&game, text, "XIII.magnum");
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let parent = vm.class("XIII.MagnumAttach").unwrap();
    let parent_mesh = vm.prop(vm.classes[parent].cdo, "StaticMesh");
    eprintln!("MagnumAttach default StaticMesh: {parent_mesh}");
    let c = vm.class("Mod.HeldAttach").unwrap();
    let spawn = |vm: &mut Vm| {
        let (l, r) = (vm.prop(pawn, "Location"), vm.prop(pawn, "Rotation"));
        let a = vm.spawn_actor(c, Value::Object(Some(pawn)), None, l, r, Value::Object(None), true).unwrap().unwrap();
        vm.prop(a, "StaticMesh")
    };
    let first = spawn(&mut vm);
    let _ = vm.reload_class_defaults(c);
    let reloaded = spawn(&mut vm);
    eprintln!("Mod.HeldAttach: {first} then, its defaults reloaded (a new level), {reloaded}");
    assert_eq!(first, Value::Asset("MeshArmesPickup.sniper".into()), "parent {parent_mesh}");
    assert_eq!(reloaded, Value::Asset("MeshArmesPickup.sniper".into()), "parent {parent_mesh}");
}
