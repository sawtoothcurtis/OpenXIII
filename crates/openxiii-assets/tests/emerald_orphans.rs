//! Actors deleted in the editor (bDeleteMe) left as exports of the map, outside
//! ULevelBase::Actors: the game does not see them (AllActors, rendering, collision). Hual01b had
//! a terrain (patchy snow), a SkyZoneInfo (black sky) and StaticMeshes; Toits01 had
//! tables and chairs (all-black furniture).
//! OXIII_GAME=<game dir> cargo test -p openxiii-assets --test emerald_orphans -- --ignored
use openxiii_package::library::Library;
use openxiii_package::{ObjectRef, Package, Value};

fn map_value(game_value: &str, name_value: &str) -> (Package, Vec<u8>) {
    let data = std::fs::read(format!("{game_value}/Maps/{name_value}.unr")).unwrap();
    (Package::parse(&data).unwrap(), data)
}

fn destroyed(p: &Package, data: &[u8], i: usize) -> bool {
    p.object_data(data, i).is_ok_and(|o| matches!(o.get(p, "bDeleteMe"), Some(Value::Bool(true))))
}

/// All maps: an export of ULevel::Actors is never bDeleteMe, and level::mesh_actors
/// keeps only those; check: there really are deleted StaticMeshes outside the list.
#[test]
#[ignore]
fn destroyed_actors_outside_geometry() {
    let Ok(game_value) = openxiii_package::env::var("OXIII_GAME") else { eprintln!("skipped: needs the game's files (OXIII_GAME)"); return; };
    let (mut maps, mut rejected) = (0, 0);
    for f in std::fs::read_dir(format!("{game_value}/Maps")).unwrap() {
        let f = f.unwrap().path();
        if f.extension().is_none_or(|e| !e.eq_ignore_ascii_case("unr")) {
            continue;
        }
        let name_value = f.file_stem().unwrap().to_string_lossy().to_string();
        let (p, data) = map_value(&game_value, &name_value);
        let Some(list_value) = openxiii_assets::level::level_actor_exports(&p, &data) else { continue };
        maps += 1;
        for &i in &list_value {
            assert!(!destroyed(&p, &data, i), "{name_value}: {} in Actors but bDeleteMe", p.object_path(ObjectRef::Export(i)));
        }
        let kept_names: std::collections::HashSet<String> = openxiii_assets::level::mesh_actors(&name_value, &p, &data).into_iter().map(|m| m.actor).collect();
        for i in 0..p.exports.len() {
            let Ok(o) = p.object_data(&data, i) else { continue };
            if !matches!(o.get(&p, "StaticMesh"), Some(Value::Object(m)) if *m != 0) || o.get(&p, "Location").is_none() {
                continue;
            }
            let path_buf = p.object_path(ObjectRef::Export(i));
            if list_value.contains(&i) {
                continue;
            }
            assert!(destroyed(&p, &data, i), "{name_value}: {path_buf} outside Actors without bDeleteMe");
            assert!(!kept_names.contains(&path_buf), "{name_value}: {path_buf} destroyed but kept in the geometry");
            rejected += 1;
        }
    }
    assert!(maps > 40, "{maps} maps");
    // Counts per map in the game data (Plage01 13, Toits01 10, Hual01b 3…).
    assert!(rejected >= 100, "{rejected} destroyed StaticMeshes left out");
}

/// Hual01b: sky of SkyZoneInfo0 (the only one in ULevel::Actors), not the deleted SkyZoneInfo1.
#[test]
#[ignore]
fn sky_hual01b() {
    let Ok(game_value) = openxiii_package::env::var("OXIII_GAME") else { eprintln!("skipped: needs the game's files (OXIII_GAME)"); return; };
    let (p, data) = map_value(&game_value, "Hual01b");
    let mut library = Library::new(std::path::Path::new(&game_value)).unwrap();
    library.insert("Hual01b", format!("{game_value}/Maps/Hual01b.unr").into(), data.clone(), p.clone());
    let world = openxiii_assets::world::build_world(&mut library, "Hual01b").unwrap();
    let skies: std::collections::HashSet<[i32; 3]> = world.zones.iter().filter_map(|z| z.sky).map(|(l, _, _)| l.map(|x| x.round() as i32)).collect();
    assert_eq!(skies.into_iter().collect::<Vec<_>>(), vec![[-422, -16696, 11867]]);
    let list_value = openxiii_assets::level::level_actor_exports(&p, &data).unwrap();
    let terrains: Vec<String> = list_value.iter().filter(|&&i| p.export_class(i) == "TerrainInfo").map(|&i| p.object_path(ObjectRef::Export(i))).collect();
    assert_eq!(terrains, vec!["TerrainInfo2".to_string()]);
}

/// USA01 (Sabotage): two live SkyZoneInfo; LinkToSkybox keeps the last one of AllActors,
/// hence of ULevel::Actors (SkyZoneInfo1, slot 436), not the last export (SkyZoneInfo0, slot 23,
/// outside the world: black sky).
#[test]
#[ignore]
fn sky_usa01() {
    let Ok(game_value) = openxiii_package::env::var("OXIII_GAME") else { eprintln!("skipped: needs the game's files (OXIII_GAME)"); return; };
    let (p, data) = map_value(&game_value, "USA01");
    let mut library = Library::new(std::path::Path::new(&game_value)).unwrap();
    library.insert("USA01", format!("{game_value}/Maps/USA01.unr").into(), data.clone(), p.clone());
    let world = openxiii_assets::world::build_world(&mut library, "USA01").unwrap();
    let skies: std::collections::HashSet<[i32; 3]> = world.zones.iter().filter_map(|z| z.sky).map(|(l, _, _)| l.map(|x| x.round() as i32)).collect();
    assert_eq!(skies.into_iter().collect::<Vec<_>>(), vec![[5756, -2431, 97]]);
}
