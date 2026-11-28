//! Mods build only: shot effects set by mods, on the real Plage01 (OXIII_GAME = original game
//! folder; skipped when it is not set): values of an Emitter class's particle emitters
//! (`"Emitters[n]" = { … }`, mod_defaults.rs), ImpactEffectsBySurface and TracerColor
//! (mod_weapons.rs). Each check has its negative counterpart: the game's own classes, and the
//! same classes without the setting, keep the original behaviour.

use std::path::{Path, PathBuf};

use openxiii_mods::ModValue;

use crate::mod_weapons::tracer_color;
use crate::value::{ObjId, Value};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// Plage01 started with the mod file `text` (new classes, class values, map changes).
fn plage01(game: &Path, text: &str) -> Vm {
    let file = openxiii_mods::defaults::parse_mod_file(text, Path::new("test/mod.toml"), "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(file.new_classes);
    vm.set_mod_defaults_here(file.overrides);
    vm.set_mod_edits_here(file.edits);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
    vm
}

fn cdo(vm: &mut Vm, class: &str) -> ObjId {
    let c = vm.class(class).unwrap_or_else(|| panic!("class {class}"));
    vm.create_cdo(c);
    vm.classes[c].cdo
}

/// The ParticleEmitter at `index` of an Emitter's `Emitters` list (object or sub-object path).
fn emitter(vm: &mut Vm, holder: ObjId, index: usize) -> ObjId {
    let Value::Array(items) = vm.prop(holder, "Emitters") else { panic!("no Emitters list") };
    match &items[index] {
        Value::Object(Some(o)) => *o,
        Value::Asset(path) => vm.materialize_subobject(&path.clone()).expect("sub-object"),
        v => panic!("Emitters[{index}] = {v}"),
    }
}

fn texture(vm: &Vm, e: ObjId) -> String {
    format!("{}", vm.prop(e, "Texture")).to_ascii_lowercase()
}

#[test]
fn tracer_colour_values() {
    assert_eq!(tracer_color(&ModValue::Table(vec![("R".into(), ModValue::Int(10)), ("g".into(), ModValue::Int(20)), ("B".into(), ModValue::Int(255))])), Ok([255, 20, 10, 255]));
    assert_eq!(tracer_color(&ModValue::Table(vec![("A".into(), ModValue::Int(0))])), Ok([0, 0, 0, 0]));
    assert!(tracer_color(&ModValue::Table(vec![("R".into(), ModValue::Int(256))])).is_err());
    assert!(tracer_color(&ModValue::Table(vec![("X".into(), ModValue::Int(1))])).is_err());
    assert!(tracer_color(&ModValue::Str("blue".into())).is_err());
}

const SPARKS: &str = "[classes.Sparks]\nparent = \"XIII.BulletMetalEmitter\"\n[class.\"Mod.Sparks\"]\n\"Emitters[1]\" = { Texture = \"XIIICine.effets.mist3\", MaxParticles = 16, StartVelocityRange = { X = { Max = 321.0 } } }\n\"Emitters[2]\" = { DrawStyle = \"PTDS_Darken\" }\n";

#[test]
fn emitter_values_of_a_new_class_only() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut vm = plage01(&game, SPARKS);
    let (game_cdo, mod_cdo) = (cdo(&mut vm, "XIII.BulletMetalEmitter"), cdo(&mut vm, "Mod.Sparks"));
    // The new class has its own emitters 1 and 2, with the values; emitter 0 is the parent's.
    let (g0, g1, g2) = (emitter(&mut vm, game_cdo, 0), emitter(&mut vm, game_cdo, 1), emitter(&mut vm, game_cdo, 2));
    let (m0, m1, m2) = (emitter(&mut vm, mod_cdo, 0), emitter(&mut vm, mod_cdo, 1), emitter(&mut vm, mod_cdo, 2));
    assert_eq!(g0, m0);
    assert_ne!(g1, m1);
    assert_ne!(g2, m2);
    assert!(texture(&vm, m1).contains("mist3"), "{}", texture(&vm, m1));
    assert_eq!(vm.prop(m1, "MaxParticles").as_int(), 16);
    assert_eq!(vm.prop(m2, "DrawStyle").as_byte(), 5);
    // Only the given fields change: the rest of the struct and the other values are the parent's.
    let range = format!("{}", vm.prop(m1, "StartVelocityRange"));
    let parent_range = format!("{}", vm.prop(g1, "StartVelocityRange"));
    assert_ne!(range, parent_range);
    assert!(range.contains("321"), "{range}");
    assert_eq!(vm.prop(m1, "LifetimeRange"), vm.prop(g1, "LifetimeRange"));
    assert_eq!(vm.prop(m2, "Texture"), vm.prop(g2, "Texture"));
    // Negative check: the game's class keeps its own emitters as they are.
    assert!(texture(&vm, g1).contains("hjaunestatic"), "{}", texture(&vm, g1));
    assert_eq!(vm.prop(g1, "MaxParticles").as_int(), 10);
    assert_eq!(vm.prop(g2, "DrawStyle").as_byte(), 3);
    // Spawned, each actor copies its class's emitters (the mod's values for the new class).
    let loc = Value::vector(-6606.0, -11610.0, 800.0);
    let rot = Value::Rotator([0, 0, 0]);
    for (class, tex, max) in [("Mod.Sparks", "mist3", 16), ("XIII.BulletMetalEmitter", "hjaunestatic", 10)] {
        let c = vm.class(class).unwrap();
        let a = vm.spawn_actor(c, Value::Object(None), None, loc.clone(), rot.clone(), Value::Object(None), true).unwrap().expect("spawned");
        let e = emitter(&mut vm, a, 1);
        assert!(e != m1 && e != g1, "{class}: the actor shares its class's emitter");
        assert!(texture(&vm, e).contains(tex), "{class}: {}", texture(&vm, e));
        assert_eq!(vm.prop(e, "MaxParticles").as_int(), max, "{class}");
    }
    for _ in 0..30 {
        vm.tick(1.0 / 30.0);
    }
}

#[test]
fn emitter_values_errors() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[classes.Sparks]\nparent = \"XIII.BulletMetalEmitter\"\n[class.\"Mod.Sparks\"]\n\"Emitters[7]\" = { MaxParticles = 1 }\nEmitters = { MaxParticles = 1 }\n\"Emitters[0]\" = { NoSuchThing = 1 }\n";
    let mut vm = plage01(&game, text);
    let c = cdo(&mut vm, "Mod.Sparks");
    let log = vm.mod_messages.join("\n");
    assert!(log.contains("Emitters[7]: index out of range"), "{log}");
    assert!(log.contains("is a list of objects"), "{log}");
    assert!(log.contains("NoSuchThing"), "{log}");
    // Nothing applied: the parent's emitters.
    let g = cdo(&mut vm, "XIII.BulletMetalEmitter");
    assert_eq!(emitter(&mut vm, c, 0), emitter(&mut vm, g, 0));
}

const AMMO: &str = "[classes.Cells]\nparent = \"XIII.c9mmAmmo\"\n[class.\"Mod.Cells\"]\nImpactEmitterMem = \"XIII.BulletMetalEmitter\"\nDecalProjectorClass = \"XIII.BulletScorchMetal\"\n";

/// ImpactEmitterMem and DecalProjectorClass of a new ammo of class `class` after the game chose
/// the effects for a hit on the surface of `sound` (XIIIBulletsAmmo.SetUpImpactEmitter).
fn effects_after_hit(vm: &mut Vm, class: &str, sound: &str) -> (String, String) {
    let c = vm.class(class).unwrap();
    let a = vm.spawn_actor(c, Value::Object(None), None, Value::vector(-6606.0, -11610.0, 800.0), Value::Rotator([0, 0, 0]), Value::Object(None), true).unwrap().expect("ammo");
    vm.call_named(a, "SetUpImpactEmitter", vec![Value::Asset(sound.into())]).unwrap();
    let name = |v: Value| match v {
        Value::Class(Some(k)) => vm.class_path(k),
        v => format!("{v}"),
    };
    (name(vm.prop(a, "ImpactEmitterMem")), name(vm.prop(a, "DecalProjectorClass")))
}

#[test]
fn impact_effects_by_surface() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let wood = "XIIIsound.Impacts__ImpBoisC.ImpBoisC__hPlayImpBoiC";
    // Negative check: without the setting, the game picks the wood effects for a wood surface.
    let mut vm = plage01(&game, AMMO);
    let (emitter, decal) = effects_after_hit(&mut vm, "Mod.Cells", wood);
    assert!(emitter.contains("WoodDustEmitter"), "{emitter}");
    assert!(decal.contains("BulletScorchWood"), "{decal}");
    // With ImpactEffectsBySurface = false: the class's own effects stay, on the new class only.
    let mut vm = plage01(&game, &format!("{AMMO}ImpactEffectsBySurface = false\n"));
    let (emitter, decal) = effects_after_hit(&mut vm, "Mod.Cells", wood);
    assert!(emitter.contains("BulletMetalEmitter"), "{emitter}");
    assert!(decal.contains("BulletScorchMetal"), "{decal}");
    let (emitter, _) = effects_after_hit(&mut vm, "XIII.c9mmAmmo", wood);
    assert!(emitter.contains("WoodDustEmitter"), "9mm: {emitter}");
    assert!(vm.mod_messages.iter().any(|l| l.contains("Mod.Cells.ImpactEffectsBySurface = false")), "{:?}", vm.mod_messages);
    // On a class without impact effects: a warning, the value is ignored.
    let vm = plage01(&game, "[class.\"XIII.Beretta\"]\nImpactEffectsBySurface = false\n");
    assert!(vm.mod_messages.iter().any(|l| l.contains("ImpactEffectsBySurface") && l.contains("no impact effects")), "{:?}", vm.mod_messages);
}

/// The actors of class `class` the mods added (named by the log line of their addition).
fn added(vm: &Vm, class: &str) -> Vec<ObjId> {
    let c = vm.class(class).unwrap();
    vm.actors.iter().copied().filter(|&a| vm.objects.get(a).and_then(|o| o.as_ref()).is_some_and(|o| o.class == c)).collect()
}

#[test]
fn placement_rotation_scale_collision() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[[map.Plage01.add]]\nclass = \"XIII.MedPick\"\nposition = [-500, -872, 1190]\nrotation = [10, 45, 0]\nscale = [1, 2, 3]\ncollision = true\n\n[[map.Plage01.add]]\nclass = \"XIII.MedPick\"\nposition = [-700, -872, 1190]\n";
    let vm = plage01(&game, text);
    let mut list = added(&vm, "XIII.MedPick");
    list.retain(|&a| {
        let l = vm.prop(a, "Location").as_vector();
        (l[1] + 872.0).abs() < 1.0 && (l[0] + 500.0).abs().min((l[0] + 700.0).abs()) < 1.0
    });
    list.sort_by(|&a, &b| vm.prop(b, "Location").as_vector()[0].total_cmp(&vm.prop(a, "Location").as_vector()[0]));
    assert_eq!(list.len(), 2, "{:?}", vm.mod_messages);
    let (set, plain) = (list[0], list[1]);
    assert_eq!(vm.prop(set, "Rotation").as_rotator(), [1820, 8192, 0]);
    assert_eq!(format!("{}", vm.prop(set, "DrawScale3D")), format!("{}", Value::vector(1.0, 2.0, 3.0)));
    for p in ["bCollideActors", "bBlockActors", "bBlockPlayers", "bBlockZeroExtentTraces", "bBlockNonZeroExtentTraces"] {
        assert!(vm.prop(set, p).as_bool(), "{p}");
    }
    // Negative check: the same pickup without the keys keeps its class's values.
    assert_eq!(vm.prop(plain, "Rotation").as_rotator(), [0, 0, 0]);
    assert_eq!(format!("{}", vm.prop(plain, "DrawScale3D")), format!("{}", Value::vector(1.0, 1.0, 1.0)));
    assert!(!vm.prop(plain, "bBlockPlayers").as_bool());
}

/// Forward axis (X) of a rotation.
fn forward(r: [i32; 3]) -> [f32; 3] {
    crate::ai_natives::axes(r)[0]
}

#[test]
fn first_person_model_faces_forward() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[classes.Held]\nparent = \"XIII.Beretta\"\n[class.\"Mod.Held\"]\nFirstPersonModel = \"MeshArmesPickup.sniper\"\nFirstPersonHideBones = [\"Berretta\", \"chargeur\", \"percuteur\"]\n";
    let mut vm = plage01(&game, text);
    let pc = vm.actors.iter().copied().find(|&a| vm.is_a(a, "PlayerController")).expect("player");
    let pawn = vm.native_prop(pc, crate::NativeField::Pawn).as_object().unwrap();
    let _ = vm.call_named(pawn, "GiveWeapon", vec![Value::Str("Mod.Held".into())]);
    let _ = vm.console_command_out(pc, "getweapon Mod.Held");
    let held = vm.class("Mod.Held").unwrap();
    let mut seen = Vec::new();
    for i in 0..(6 * 60) {
        // The view turns: the model must turn with it.
        if i % 60 == 0 {
            let r = vm.prop(pc, "Rotation").as_rotator();
            vm.set_prop(pc, "Rotation", Value::Rotator([r[0], r[1].wrapping_add(9000), r[2]]));
        }
        vm.tick(1.0 / 60.0);
        let Some(w) = vm.native_prop(pawn, crate::NativeField::Weapon).as_object().filter(|&w| vm.objects[w].as_ref().is_some_and(|o| o.class == held)) else { continue };
        let Some(&att) = vm.mods.fp_models.get(&w) else { continue };
        if vm.objects[w].as_ref().and_then(|o| o.state.clone()).is_some_and(|s| s.eq_ignore_ascii_case("idle")) && vm.set_attachment_location(w, att) {
            let (fw, fm) = (forward(vm.prop(w, "Rotation").as_rotator()), forward(vm.prop(att, "Rotation").as_rotator()));
            seen.push(fw[0] * fm[0] + fw[1] * fm[1] + fw[2] * fm[2]);
        }
    }
    assert!(seen.len() > 60, "weapon never idle with its model: {:?}", vm.mod_messages);
    // Its front points where the weapon points (the hand's idle sway moves it a little).
    let worst = seen.iter().copied().fold(1.0f32, f32::min);
    assert!(worst > 0.9, "model front against the weapon's: worst cosine {worst}");
}

#[test]
fn tracer_colour_of_a_weapon() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let text = "[classes.Gun]\nparent = \"XIII.Beretta\"\n[class.\"Mod.Gun\"]\nTracerColor = { R = 40, G = 120, B = 255 }\n";
    let mut vm = plage01(&game, text);
    let orange = [100, 200, 255, 255];
    let loc = Value::vector(-6606.0, -11610.0, 800.0);
    let colour = |vm: &mut Vm, class: &str, caller: &str| {
        let c = vm.class(class).unwrap();
        let w = vm.spawn_actor(c, Value::Object(None), None, loc.clone(), Value::Rotator([0, 0, 0]), Value::Object(None), true).unwrap().expect("weapon");
        let f = vm.find_function(w, caller, false).unwrap_or_else(|| panic!("{caller}"));
        let draw = vm.find_function(w, "PlaySound", false).unwrap();
        vm.stack.extend([f, draw]);
        vm.this_stack.extend([w, w]);
        let k = vm.mod_tracer_draw_color(orange);
        vm.stack.truncate(vm.stack.len() - 2);
        vm.this_stack.truncate(vm.this_stack.len() - 2);
        k
    };
    assert_eq!(colour(&mut vm, "Mod.Gun", "DrawTraceBullet"), [255, 120, 40, 255]);
    // Negative checks: another tile drawn by the same weapon, and the game's 9mm.
    assert_eq!(colour(&mut vm, "Mod.Gun", "RenderOverlays"), orange);
    assert_eq!(colour(&mut vm, "XIII.Beretta", "DrawTraceBullet"), orange);
}
