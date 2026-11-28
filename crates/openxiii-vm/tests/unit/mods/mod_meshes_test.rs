//! Mods build only: a mod's own 3D model has the box of its own vertices. The game draws an
//! actor only when its render box is not empty, so without it the model was read but never drawn.
//! The second test runs on the real Plage01 (OXIII_GAME = original game folder; skipped when it is
//! not set).

use std::path::PathBuf;

use openxiii_mods::placement::{ActorValue, Edit, MapEdit, Source};
use openxiii_mods::ModValue;

use crate::value::Value;
use crate::{NativeField as Field, Vm};

const MODEL: &str = "Mod.TestCrate";
/// A 50-unit square pyramid (base on z = 0, apex at z = 50) colliding with its triangles…
const PYRAMID: &str = "Mod.TestPyramid";
/// …the same pyramid colliding with its box…
const PYRAMID_BOX: &str = "Mod.TestPyramidBox";
/// …and colliding with nothing.
const PYRAMID_GHOST: &str = "Mod.TestPyramidGhost";

/// The mods' model table (set once for the whole test process): a 50-unit cube standing on its
/// base, and the pyramid with each kind of collision.
fn install() {
    use openxiii_assets::mod_meshes::{build, Collision, Part};
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mut positions = Vec::new();
        for i in 0..8 {
            positions.push([if i & 1 == 0 { -25.0 } else { 25.0 }, if i & 2 == 0 { -25.0 } else { 25.0 }, if i & 4 == 0 { 0.0 } else { 50.0 }]);
        }
        let faces: [[u32; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let indices: Vec<u32> = faces.iter().flat_map(|f| [f[0], f[1], f[2], f[0], f[2], f[3]]).collect();
        let (mesh, materials) = build(&[Part { positions: &positions, normals: &[], uvs: &[], indices: &indices, material: String::new() }]).unwrap();
        let mut table = std::collections::HashMap::new();
        table.insert(MODEL.to_string(), std::sync::Arc::new((mesh, materials, Collision::Mesh)));
        let pyramid = [[-25.0, -25.0, 0.0], [25.0, -25.0, 0.0], [-25.0, 25.0, 0.0], [25.0, 25.0, 0.0], [0.0, 0.0, 50.0]];
        let indices = [0, 1, 3, 0, 3, 2, 0, 4, 1, 1, 4, 3, 3, 4, 2, 2, 4, 0];
        for (name, collision) in [(PYRAMID, Collision::Mesh), (PYRAMID_BOX, Collision::Box), (PYRAMID_GHOST, Collision::None)] {
            let (mesh, materials) = build(&[Part { positions: &pyramid, normals: &[], uvs: &[], indices: &indices, material: String::new() }]).unwrap();
            table.insert(name.to_string(), std::sync::Arc::new((mesh, materials, collision)));
        }
        openxiii_assets::mod_meshes::set(table);
    });
}

#[test]
fn own_model_has_its_box() {
    install();
    let mut vm = Vm::new();
    assert_eq!(vm.primitive_box(MODEL), Some(([-25.0, -25.0, 0.0], [25.0, 25.0, 50.0])));
    // Case does not matter, as for the game's objects.
    assert_eq!(vm.primitive_box("mod.testcrate"), Some(([-25.0, -25.0, 0.0], [25.0, 25.0, 50.0])));
    assert_eq!(vm.primitive_box("Mod.NoSuchModel"), None);
}

#[test]
fn own_model_on_an_added_actor_has_a_render_box() {
    let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| g.join("System").exists()) else {
        eprintln!("OXIII_GAME not set: skipped");
        return;
    };
    install();
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(vec![MapEdit {
        map: "Plage01".into(),
        edit: Edit::Add {
            class: "XIII.MedPick".into(),
            position: [-6643.0, -11524.0, 690.0],
            yaw: None,
            properties: vec![
                ActorValue { property: "StaticMesh".into(), index: None, value: ModValue::Str(MODEL.into()) },
                // Traces through the mesh, not the pickup's cylinder.
                ActorValue { property: "bUseCylinderCollision".into(), index: None, value: ModValue::Bool(false) },
            ],
        },
        source: Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }, checkpoint: Default::default(),
    }]);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
    let messages = vm.mod_messages.join("\n");
    let model = Value::Asset(MODEL.into());
    let added = vm.actors.clone().into_iter().find(|&a| vm.native_prop(a, Field::StaticMesh) == &model).unwrap_or_else(|| panic!("no actor with the model\n{messages}"));
    let (lo, hi) = vm.render_bounding_box(added, false);
    let scale = vm.native_prop(added, Field::DrawScale).as_float();
    let at = vm.native_prop(added, Field::Location).as_vector();
    // The cube's 50 units (times DrawScale) around the actor, not an empty box.
    for k in 0..3 {
        assert!((hi[k] - lo[k] - 50.0 * scale).abs() < 0.5, "axis {k}: {lo:?} {hi:?} scale {scale}");
        assert!(lo[k] <= at[k] + 0.5 && at[k] <= hi[k] + 0.5, "axis {k}: {lo:?} {hi:?} at {at:?}");
    }
    // A trace straight down onto the actor stops on the cube's top face (its own triangles), as
    // the game's traces and touches do on a game mesh.
    let mut hit = crate::physics::CheckResult::new(1.0);
    let flags = crate::physics::TRACE_PAWNS | crate::physics::TRACE_MOVERS | crate::physics::TRACE_OTHERS;
    let (start, end) = ([at[0] + 7.0, at[1] - 5.0, hi[2] + 100.0], [at[0] + 7.0, at[1] - 5.0, at[2] - 10.0]);
    let free = vm.single_line_check(&mut hit, None, end, start, flags, [0.0; 3]);
    assert!(!free && hit.actor == Some(added), "{hit:?}");
    // The engine moves the contact back along the trace by up to one unit.
    assert!(hit.location[2] >= hi[2] - 0.01 && hit.location[2] <= hi[2] + 1.01, "hit at {:?}, top of the cube at {}", hit.location, hi[2]);
}

/// The collision tree made for a mod's model: traces against the 50-unit cube hit where the
/// exact answer says, from every direction, and miss when they pass beside it.
#[test]
fn own_model_collision_tree_matches_the_exact_answer() {
    use crate::kdop::{build_tree, frame, line_check, point_check, KdopSet};
    let mut positions = Vec::new();
    for i in 0..8 {
        positions.push([if i & 1 == 0 { -25.0f32 } else { 25.0 }, if i & 2 == 0 { -25.0 } else { 25.0 }, if i & 4 == 0 { 0.0 } else { 50.0 }]);
    }
    let faces: [[u16; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    let tris: Vec<[u16; 4]> = faces.iter().flat_map(|f| [[f[0], f[1], f[2], 0], [f[0], f[2], f[3], 0]]).collect();
    let (verts, tris, nodes) = build_tree(&positions, &tris).expect("tree");
    let n = tris.len();
    let set = KdopSet::new(verts, tris, nodes, vec![0; n]);
    let root = [-26.0, -26.0, -1.0, 26.0, 26.0, 51.0];
    let f = frame([0; 3], [1.0; 3], [0.0; 3], [0.0; 3], 1.0);
    // Exact entry time of a segment into the box (slab method).
    let entry = |s: [f32; 3], e: [f32; 3]| -> Option<f32> {
        let (lo, hi) = ([-25.0f32, -25.0, 0.0], [25.0f32, 25.0, 50.0]);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            let d = e[k] - s[k];
            if d.abs() < 1e-6 {
                if s[k] < lo[k] || s[k] > hi[k] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((lo[k] - s[k]) / d, (hi[k] - s[k]) / d);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        (t0 <= t1).then_some(t0)
    };
    let mut seed = 7u32;
    let mut rnd = |a: f32, b: f32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        a + (b - a) * ((seed >> 8) as f32 / (1u32 << 24) as f32)
    };
    let (mut hits, mut misses) = (0, 0);
    for _ in 0..300 {
        // From a point outside the box towards a point inside or beside it.
        let s = [rnd(-120.0, 120.0), rnd(-120.0, 120.0), rnd(-70.0, 120.0)];
        if s[0].abs() < 30.0 && s[1].abs() < 30.0 && s[2] > -5.0 && s[2] < 55.0 {
            continue;
        }
        let e = [rnd(-40.0, 40.0), rnd(-40.0, 40.0), rnd(-15.0, 65.0)];
        let expected = entry(s, e);
        let got = line_check(&set, &root, &f, e, s, [0.0; 3]).map(|h| h.time);
        match expected {
            // Grazing an edge: either answer is fine.
            Some(t) if (0.001..0.999).contains(&t) => {
                let g = got.unwrap_or_else(|| panic!("{s:?} -> {e:?}: no hit, expected {t}"));
                assert!((g - t).abs() < 1e-3, "{s:?} -> {e:?}: {g} instead of {t}");
                hits += 1;
            }
            None => {
                assert!(got.is_none(), "{s:?} -> {e:?}: hit {got:?} beside the cube");
                misses += 1;
            }
            _ => {}
        }
    }
    assert!(hits > 100 && misses > 50, "{hits} hits, {misses} misses");
    // A box overlapping the top face is in contact; one above it is not.
    assert!(point_check(&set, &root, &f, [0.0, 0.0, 55.0], [10.0, 10.0, 10.0]).is_some());
    assert!(point_check(&set, &root, &f, [0.0, 0.0, 75.0], [10.0, 10.0, 10.0]).is_none());
}

/// Same check on a model that is not convex: a cube crossed by a long bar (triangles of each
/// box cross the other's planes, so the tree cuts them). The exact answer is the first box hit.
#[test]
fn own_model_collision_tree_cuts_crossing_triangles() {
    use crate::kdop::{build_tree, frame, line_check, KdopSet};
    let boxes: [([f32; 3], [f32; 3]); 2] = [([-25.0, -25.0, 0.0], [25.0, 25.0, 50.0]), ([-60.0, -8.0, 17.0], [60.0, 8.0, 33.0])];
    let mut positions = Vec::new();
    let mut tris: Vec<[u16; 4]> = Vec::new();
    for (lo, hi) in boxes {
        let b = positions.len() as u16;
        for i in 0..8 {
            positions.push([if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }]);
        }
        let faces: [[u16; 4]; 6] = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        tris.extend(faces.iter().flat_map(|f| [[b + f[0], b + f[1], b + f[2], 0], [b + f[0], b + f[2], b + f[3], 0]]));
    }
    let (verts, tris, nodes) = build_tree(&positions, &tris).expect("tree");
    assert!(tris.len() > 24, "no triangle was cut: {}", tris.len());
    let n = tris.len();
    let set = KdopSet::new(verts, tris, nodes, vec![0; n]);
    let root = [-61.0, -26.0, -1.0, 61.0, 26.0, 51.0];
    let f = frame([0; 3], [1.0; 3], [0.0; 3], [0.0; 3], 1.0);
    let entry = |s: [f32; 3], e: [f32; 3], lo: [f32; 3], hi: [f32; 3]| -> Option<f32> {
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            let d = e[k] - s[k];
            if d.abs() < 1e-6 {
                if s[k] < lo[k] || s[k] > hi[k] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((lo[k] - s[k]) / d, (hi[k] - s[k]) / d);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
        (t0 <= t1).then_some(t0)
    };
    let mut seed = 11u32;
    let mut rnd = |a: f32, b: f32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        a + (b - a) * ((seed >> 8) as f32 / (1u32 << 24) as f32)
    };
    let mut hits = 0;
    for _ in 0..400 {
        let s = [rnd(-150.0, 150.0), rnd(-120.0, 120.0), rnd(-70.0, 120.0)];
        let inside = boxes.iter().any(|(lo, hi)| (0..3).all(|k| s[k] > lo[k] - 2.0 && s[k] < hi[k] + 2.0));
        if inside {
            continue;
        }
        let e = [rnd(-70.0, 70.0), rnd(-30.0, 30.0), rnd(-10.0, 60.0)];
        let expected = boxes.iter().filter_map(|(lo, hi)| entry(s, e, *lo, *hi)).fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.min(t))));
        let got = line_check(&set, &root, &f, e, s, [0.0; 3]).map(|h| h.time);
        match expected {
            Some(t) if (0.001..0.999).contains(&t) => {
                let g = got.unwrap_or_else(|| panic!("{s:?} -> {e:?}: no hit, expected {t}"));
                assert!((g - t).abs() < 1e-3, "{s:?} -> {e:?}: {g} instead of {t}");
                hits += 1;
            }
            None => assert!(got.is_none(), "{s:?} -> {e:?}: hit {got:?} beside the model"),
            _ => {}
        }
    }
    assert!(hits > 150, "{hits} hits");
}

/// Decorations added by a mod with `model = "…"` (the mod's toml read as in the game): solid for
/// the player's moves and for shots and sight, as the map's own decorations are, with the
/// collision their model asks for. On the real Plage01, in the air above the jetty (nothing else
/// there), the pyramid four times bigger (200 units).
#[test]
fn decorations_block_moves_and_shots_as_their_model_asks() {
    let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| g.join("System").exists()) else {
        eprintln!("OXIII_GAME not set: skipped");
        return;
    };
    install();
    let base = [-6643.0f32, -11524.0, 990.0];
    let spots = [(PYRAMID, -700.0f32), (PYRAMID_BOX, 0.0), (PYRAMID_GHOST, 700.0)];
    let tmp = std::env::temp_dir().join(format!("oxiii-solid-decorations-{}", std::process::id()));
    let mut text = String::from("[mod]\nname = \"Solid decorations\"\nversion = \"1\"\n");
    for (model, dx) in spots {
        text += &format!("\n[[map.Plage01.add]]\nmodel = \"{model}\"\nposition = [{}, {}, {}]\nproperties = {{ DrawScale = 4.0 }}\n", base[0] + dx, base[1], base[2]);
    }
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("solid")).unwrap();
    std::fs::write(tmp.join("solid").join("mod.toml"), text).unwrap();
    let found = openxiii_mods::discover(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(found.problems.is_empty(), "{:?}", found.problems);

    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(openxiii_mods::collect_new_classes(&found.mods).0);
    vm.set_mod_defaults_here(openxiii_mods::collect_overrides(&found.mods).0);
    vm.set_mod_edits_here(openxiii_mods::collect_edits(&found.mods).0);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let messages = vm.mod_messages.join("\n");
    let decoration = vm.class(openxiii_mods::placement::DECORATION_CLASS).unwrap_or_else(|| panic!("no decoration class\n{messages}"));

    let flags_shot = crate::physics::TRACE_PAWNS | crate::physics::TRACE_MOVERS | crate::physics::TRACE_LEVEL | crate::physics::TRACE_OTHERS | crate::physics::TRACE_ONLY_PROJ_ACTOR | crate::physics::TRACE_LEVEL_GEOMETRY;
    for (model, _) in spots {
        let mesh = Value::Asset(model.into());
        let a = vm.actors.clone().into_iter().find(|&a| vm.native_prop(a, Field::StaticMesh) == &mesh).unwrap_or_else(|| panic!("{model}: not added\n{messages}"));
        assert_eq!(vm.objects[a].as_ref().map(|o| o.class), Some(decoration), "{model}");
        assert!(vm.is_a(a, "StaticMeshActor"), "{model}");
        let at = vm.native_prop(a, Field::Location).as_vector();
        // Straight down at (+80, +80) from the centre, inside the base (100 by 100 after scaling):
        // the pyramid's side is 40 units high there, its box 200.
        let mut heights = Vec::new();
        for flags in [flags_shot, crate::physics::TRACE_WORLD] {
            let mut hit = crate::physics::CheckResult::new(1.0);
            let (start, end) = ([at[0] + 80.0, at[1] + 80.0, at[2] + 300.0], [at[0] + 80.0, at[1] + 80.0, at[2] - 20.0]);
            vm.single_line_check(&mut hit, None, end, start, flags, [0.0; 3]);
            heights.push((hit.actor == Some(a)).then(|| hit.location[2] - at[2]));
        }
        // The player walks into it at mid-height (100 units up: the pyramid is 100 wide there).
        let h = vm.native_prop(pawn, Field::CollisionHeight).as_float();
        let r = vm.native_prop(pawn, Field::CollisionRadius).as_float();
        assert!(vm.far_move_actor(pawn, [at[0] - 250.0, at[1], at[2] + 100.0], false, true, false), "{model}: teleport");
        let mut hit = crate::physics::CheckResult::new(1.0);
        let rotation = vm.rot(pawn);
        vm.move_actor(pawn, [500.0, 0.0, 0.0], rotation, &mut hit, false, false, false, false);
        let stopped = (hit.actor == Some(a)).then(|| vm.native_prop(pawn, Field::Location).as_vector()[0] - at[0]);
        eprintln!("{model}: heights {heights:?}, stopped {stopped:?}, hit {:?} (pawn radius {r}, height {h})", hit.actor.map(|o| vm.obj_label(o)));
        match model {
            PYRAMID => {
                for z in &heights {
                    let z = z.unwrap_or_else(|| panic!("{model}: trace went through: {heights:?}"));
                    assert!((z - 40.0).abs() < 2.0, "{model}: {heights:?}");
                }
                let x = stopped.unwrap_or_else(|| panic!("{model}: the player went through"));
                // The cylinder's bottom (100 - h units up) touches first: the pyramid is wider there.
                let half = 100.0 * (1.0 - (100.0 - h) / 200.0);
                assert!((x + half + r).abs() < 4.0, "{model}: stopped at {x}, expected {}", -half - r);
            }
            PYRAMID_BOX => {
                for z in &heights {
                    let z = z.unwrap_or_else(|| panic!("{model}: trace went through: {heights:?}"));
                    assert!((z - 200.0).abs() < 2.0, "{model}: {heights:?}");
                }
                let x = stopped.unwrap_or_else(|| panic!("{model}: the player went through"));
                assert!((x + 100.0 + r).abs() < 4.0, "{model}: stopped at {x}");
            }
            _ => {
                assert_eq!(heights, [None, None], "{model}");
                assert_eq!(stopped, None, "{model}: blocked the player");
            }
        }
    }
}
