//! Mods build only: a mod's animated model (skinned glTF, test data in
//! tests/fixtures/mods/animated-arm) played by the engine's animation code: the frames of its
//! bones and the skinned points match the glTF's own answer. The second test runs on the real
//! Plage01 (OXIII_GAME = original game folder; skipped when it is not set).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use openxiii_mods::own_animated;

use crate::value::Value;
use crate::{NativeField as Field, Vm};

const MESH: &str = "Mod.TestArm";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods")
}

/// The test mod's model, read as the game reads it, and its glTF answer.
fn read() -> (openxiii_mods::own_models::OwnModel, own_animated::AnimatedModel) {
    let found = openxiii_mods::discover(&fixture());
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    let m = found.mods.iter().flat_map(|m| m.models.iter()).find(|m| m.mesh_name() == MESH).expect("test model").clone();
    let model = own_animated::load_animated_gltf(&m.file, m.scale).unwrap();
    (m, model)
}

/// The mods' animated models table (set once for the test process).
fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let (m, _) = read();
        let mut textures = std::collections::HashMap::new();
        let mut log = Vec::new();
        let (mesh, animation) = crate::mod_models::build_animated_model(&m, &mut textures, &mut log).unwrap();
        let mut meshes = std::collections::HashMap::new();
        let mut animations = std::collections::HashMap::new();
        animations.insert(mesh.animation_name.clone(), Arc::new(animation));
        meshes.insert(MESH.to_string(), Arc::new(mesh));
        openxiii_assets::mod_skeletal::set(meshes, animations);
    });
}

#[test]
fn animated_model_and_its_game_animations() {
    install();
    let (m, _) = read();
    let mut log = Vec::new();
    let (mesh, animation) = crate::mod_models::build_animated_model(&m, &mut std::collections::HashMap::new(), &mut log).unwrap();
    // The glTF animations also keep their own names.
    assert!(log.iter().any(|l| l.contains("animations: Fire, wave, still")), "{log:?}");
    assert_eq!(mesh.bone_names, ["Root", "Hips", "Arm"]);
    // "Fire" is made from "wave" with its notify; "still" keeps its name.
    let mut vm = Vm::new();
    let info = vm.mesh_info(MESH).expect("mesh info");
    assert_eq!((info.default_anim.as_deref(), info.bones.len()), (Some("Mod.TestArm.Animation"), 3));
    let set = vm.anim_set("Mod.TestArm.Animation").expect("animation");
    let fire = &set.seqs[set.seq("Fire").expect("Fire")];
    assert_eq!((fire.num_frames, fire.rate, fire.notifies.clone()), (30, 30.0, vec![(0.5, "TestNotify".to_string())]));
    assert_eq!(set.seqs[set.seq("still").expect("still")].num_frames, 15);
    assert_eq!(animation.sequences.len(), 3);
    // A value naming the model (a class's Mesh, a weapon's MeshName) is accepted.
    assert_eq!(vm.mod_asset_value("Engine.Mesh", MESH), Some(Ok(Value::Asset(MESH.into()))));
    // Bone boxes for the shots: the hips and the arm.
    assert_eq!(vm.bone_boxes(MESH).map(|b| b.iter().map(|x| x.bone).collect::<Vec<_>>()), Some(vec![1, 2]));
}

/// The engine plays "Fire" on an actor drawn with the model: half way, its bone frames skin the
/// points where the glTF puts them (the arm turned 45°), and the actor has the model's render box.
#[test]
fn engine_plays_the_animation_as_the_gltf() {
    let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| g.join("System").exists()) else {
        eprintln!("OXIII_GAME not set: skipped");
        return;
    };
    install();
    let (_, model) = read();
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    vm.set_prop(pawn, "Mesh", Value::Asset(MESH.into()));
    assert!(vm.play_anim(pawn, 0, "Fire", 1.0, 0.0, false), "PlayAnim");
    vm.update_animation(pawn, 0.5);
    let bases = vm.get_frame(pawn).expect("frames");
    let skeletal = openxiii_assets::mod_skeletal::get(MESH).unwrap();
    let points = skeletal.mesh.skin(&bases, &skeletal.mesh.ref_bases());
    let wave = model.clips.iter().find(|c| c.name == "wave").unwrap();
    let expected = own_animated::skin(&model, &wave.samples[15]);
    for (i, (p, e)) in points.iter().zip(&expected).enumerate() {
        // Within 0.15 unit: the engine samples at its own frame time, a few thousandths of a key off.
        assert!((0..3).all(|k| (p[k] - e[k]).abs() < 0.15), "point {i}: {p:?} instead of {e:?}");
    }
    // The arm's point has moved (45° about the arm's joint).
    assert!((points[2][1] - 17.68).abs() < 0.15 && (points[2][2] - 92.68).abs() < 0.15, "{:?}", points[2]);
    // Frame 15 of 30 (the engine is at 15.03).
    let channels = vm.anim_channels(pawn);
    assert!(channels.first().is_some_and(|c| c.0 == "Fire" && (c.1 - 15.0).abs() < 0.1), "{channels:?}");
    // Its render box is the model's box (the BoundingBox the game reads from a mesh), not empty.
    let (lo, hi) = vm.render_bounding_box(pawn, false);
    assert!((0..3).all(|k| hi[k] >= lo[k]) && hi[2] - lo[2] > 40.0, "{lo:?} {hi:?}");
}
