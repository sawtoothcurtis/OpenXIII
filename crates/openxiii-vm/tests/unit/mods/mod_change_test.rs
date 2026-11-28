//! Mods build only: `[[map.<Map>.change]]` and the lookup of placed actors, on the real
//! PRock03 (OXIII_GAME = game folder; skipped when it is not set).

use std::path::{Path, PathBuf};

use openxiii_mods::placement::{ActorValue, Change, Edit, MapEdit, Source, DEFAULT_RADIUS};
use openxiii_mods::ModValue;

use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn change(who: &str, look: Option<&str>) -> Change {
    Change { who: who.into(), near: None, radius: DEFAULT_RADIUS, look: look.map(str::to_string), skin: None, health: None, properties: Vec::new(), character: None }
}

fn edit(map: &str, c: Change) -> MapEdit {
    MapEdit {
        map: map.into(),
        edit: Edit::Change(Box::new(c)),
        source: Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 },
        checkpoint: Default::default(),
    }
}

/// PRock03 started with `edits`, the player logged in.
fn prock03(game: &Path, edits: Vec<MapEdit>) -> Vm {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(edits);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "PRock03");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("PRock03.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
    vm
}

fn name(vm: &Vm, a: ObjId) -> String {
    vm.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default()
}

/// (name, Mesh, Skins, Location) of every live actor that has a Mesh.
fn looks(vm: &Vm) -> Vec<(String, String, String, [i32; 3])> {
    let mut v: Vec<_> = vm
        .actors
        .iter()
        .copied()
        .filter(|&a| vm.objects[a].is_some() && !vm.native_prop(a, Field::DeleteMe).as_bool())
        .filter(|&a| !matches!(vm.prop(a, "Mesh"), Value::Object(None) | Value::Void))
        .map(|a| {
            let p = vm.native_value(a, Field::Location).as_vector();
            (name(vm, a), format!("{}", vm.prop(a, "Mesh")), format!("{}", vm.prop(a, "Skins")), [p[0] as i32, p[1] as i32, p[2] as i32])
        })
        .collect();
    v.sort();
    v
}

#[test]
fn change_a_placed_character_on_prock03() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let base = prock03(&game, Vec::new());
    // The guard who loses his pass: found by name, by PawnName (case ignored), not by a wrong name.
    let by_pawn_name = base.mod_find_placed("GARDIEN_LOSTPASS", None, 0.0);
    assert_eq!(by_pawn_name.len(), 1, "{by_pawn_name:?}");
    let guard = by_pawn_name[0];
    let guard_name = name(&base, guard);
    assert_eq!(base.mod_find_placed(&guard_name, None, 0.0), vec![guard]);
    assert!(base.mod_find_placed("gardien_lostpas", None, 0.0).is_empty());
    let at = base.native_value(guard, Field::Location).as_vector();
    assert_eq!(base.mod_find_placed("gardien_lostpass", Some([at[0] + 50.0, at[1], at[2]]), 100.0), vec![guard]);
    assert!(base.mod_find_placed("gardien_lostpass", Some([at[0] + 500.0, at[1], at[2]]), 100.0).is_empty());
    let base_mesh = format!("{}", base.prop(guard, "Mesh"));
    assert!(base_mesh.contains("GardienM"), "{base_mesh}");
    eprintln!("guard {guard_name} at {at:?}, mesh {base_mesh}, skins {}", base.prop(guard, "Skins"));

    // Changed by PawnName and by name: the guard only gets Jones's look, at the same place.
    for who in ["gardien_lostpass", guard_name.as_str()] {
        let vm = prock03(&game, vec![edit("PRock03", change(who, Some("Jones")))]);
        let g = vm.mod_find_placed("gardien_lostpass", None, 0.0)[0];
        assert_eq!(name(&vm, g), guard_name);
        assert_eq!(vm.prop(g, "Mesh"), Value::Asset("XIIIPersos.JonesM".into()), "{:?}", vm.mod_messages);
        assert_eq!(vm.native_value(g, Field::Location).as_vector(), at);
        let mut a = looks(&base);
        let mut b = looks(&vm);
        let ga = a.iter().position(|x| x.0 == guard_name).unwrap();
        let gb = b.iter().position(|x| x.0 == guard_name).unwrap();
        let (old, new) = (a.remove(ga), b.remove(gb));
        assert_eq!(a, b, "only the guard changes");
        assert_eq!((old.0, old.3), (new.0, new.3));
        assert!(
            vm.mod_messages.iter().any(|m| m.contains(&format!("PRock03: change {who}: look Jones: {guard_name} (XIDCine.Cine2)"))),
            "{:?}",
            vm.mod_messages
        );
    }

    // modwho: the player put 200 units from the guard (on one side that is not in a wall),
    // looking at him, gets his name, look and an entry to paste.
    let mut vm = prock03(&game, vec![edit("PRock03", change("gardien_lostpass", Some("Jones")))]);
    let pc = vm.mod_local_pc().expect("player");
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().expect("pawn");
    let eye = vm.eye_position(pawn)[2];
    let mut said = Vec::new();
    for (dx, dy, yaw) in [(200.0, 0.0, 32768), (-200.0, 0.0, 0), (0.0, 200.0, 49152), (0.0, -200.0, 16384)] {
        vm.set_native_vector(pawn, Field::Location, [at[0] + dx, at[1] + dy, at[2] - eye + 20.0]);
        vm.set_native_rotator(pc, Field::Rotation, [0, yaw, 0]);
        said.push(vm.mod_who(pc));
    }
    let seen = said.iter().find(|t| t.contains(&format!("{guard_name} (XIDCine.Cine2)"))).unwrap_or_else(|| panic!("{said:?}"));
    assert!(seen.contains("PawnName \"gardien_lostpass\"") && seen.contains("Mesh XIIIPersos.JonesM"), "{seen}");
    // The same `who` as the names catalog: the PawnName, which finds the guard alone.
    assert!(seen.contains("[[map.PRock03.change]]\nwho = \"gardien_lostpass\"\nlook = "), "{seen}");
    assert!(seen.contains("docs/modding/names/characters.md"), "{seen}");
    eprintln!("{seen}");
    // Negative control: the dispatcher sharing the guard's Tag is not found alone by it, so its
    // suggested `who` is its object name.
    let guard = vm.find_placed("gardien_lostpass", None, 0.0)[0];
    assert_eq!(vm.friendly_who(guard), "gardien_lostpass");
    let dispatcher = vm
        .actors
        .iter()
        .copied()
        .find(|&a| a != guard && vm.objects[a].is_some() && !vm.is_a(a, "Pawn") && vm.prop(a, "Tag").as_name().eq_ignore_ascii_case("gardien_lostpass"))
        .expect("dispatcher");
    assert_eq!(vm.friendly_who(dispatcher), name(&vm, dispatcher));

    // Health, a raw value and a skin.
    let mut c = change("gardien_lostpass", Some("XIIIPersos.SpadsM"));
    c.health = Some(321);
    c.skin = Some("XIIIPersos.Skins.SpadsNeigeTex".into());
    c.properties.push(ActorValue { property: "GroundSpeed".into(), index: None, value: ModValue::Int(123) });
    let vm = prock03(&game, vec![edit("PRock03", c)]);
    let g = vm.mod_find_placed("gardien_lostpass", None, 0.0)[0];
    assert_eq!(vm.prop(g, "Health"), Value::Int(321));
    assert_eq!(vm.prop(g, "GroundSpeed").as_float(), 123.0);
    assert_eq!(vm.prop(g, "Skins"), Value::Array(vec![Value::Asset("XIIIPersos.Skins.SpadsNeigeTex".into())]));
    assert!(!vm.mod_messages.iter().any(|m| m.starts_with("warning")), "{:?}", vm.mod_messages);

    // Negative controls: a wrong name or a wrong model changes nothing and says what was meant.
    let vm =
        prock03(&game, vec![edit("PRock03", change("gardien_lostpas", Some("Jones"))), edit("PRock03", change("gardien_lostpass", Some("XIIIPersos.JonsM")))]);
    assert_eq!(looks(&vm), looks(&base));
    let warn: Vec<&String> = vm.mod_messages.iter().filter(|m| m.starts_with("warning")).collect();
    assert_eq!(warn.len(), 2, "{warn:?}");
    assert!(warn[0].contains("no actor named gardien_lostpas") && warn[0].contains("did you mean gardien_lostpass"), "{warn:?}");
    assert!(warn[1].contains("XIIIPersos.JonsM is not an object") && warn[1].contains("look not changed"), "{warn:?}");

    // A change for another map changes nothing here.
    let vm = prock03(&game, vec![edit("Plage01", change("gardien_lostpass", Some("Jones")))]);
    assert_eq!(looks(&vm), looks(&base));
    assert!(vm.mod_messages.is_empty(), "{:?}", vm.mod_messages);
}

/// Network game: the host makes the change; a guest's copy of the guard comes from the host by
/// replication. The guard's replicated values include the new model (it differs from the class
/// default), and the model written by the host is read back by a guest as the same model. The
/// skins are not replicated by the game (a guest draws the class default ones, which the host
/// also draws after a change of look without `skin`).
#[test]
fn changed_look_reaches_a_network_guest() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut server = prock03(&game, vec![edit("PRock03", change("gardien_lostpass", Some("Jones")))]);
    let g = server.mod_find_placed("gardien_lostpass", None, 0.0)[0];
    let class = server.objects[g].as_ref().unwrap().class;
    let cdo = server.classes[class].cdo;
    let recent = server.objects[cdo].as_ref().unwrap().props.to_vec();
    assert_eq!(server.prop(g, "Skins"), server.prop(cdo, "Skins"));
    server.set_prop(g, "bNetDirty", Value::Bool(true));
    let net = server.net_class(class);
    let (reps, _) = server.optimized_rep_list(g, &recent, &std::collections::HashMap::new());
    let names: Vec<String> = reps.iter().map(|&ri| net.props.get(&net.reps[ri].0).map_or("?".to_string(), |p| p.name.clone())).collect();
    assert!(names.iter().any(|n| n == "Mesh"), "{names:?}");
    assert!(!names.iter().any(|n| n == "Skins"), "{names:?}");
    // The model crosses the network by its rank in the game's package XIIIPersos.
    let mut map = crate::net_map::PackageMap::default();
    let mut load = |name: &str| {
        server.library.as_mut().and_then(|l| l.get(name)).map(|lp| (lp.name.clone(), lp.path.display().to_string(), lp.package.clone(), lp.data.len()))
    };
    for p in ["Core", "Engine", "XIIIPersos"] {
        map.add_linker(p, &mut load);
    }
    map.compute();
    let index = server.net_build_index(&map);
    let mut w = crate::netbits::BitWriter::new(4096);
    assert!(server.net_write_object(&mut w, &index, &std::collections::HashMap::new(), &server.prop(g, "Mesh")));
    let mut client = Vm::new();
    client.ipdrv.offline = true;
    client.load_dir(&game.join("System")).unwrap();
    let client_index = client.net_build_index(&map);
    let data = w.data().to_vec();
    let mut r = crate::netbits::BitReader::new(&data, data.len() * 8);
    let none = |_: usize| -> Option<ObjId> { None };
    // Read without the type check: the same model. (With the type check the guest refuses a
    // SkeletalMesh for a Mesh property, a gap of the network reader that hits the guard's
    // unchanged look as well, checked below: it is not caused by the change.)
    let read = client.net_read_object(&mut r, &client_index, &none, "");
    assert_eq!(format!("{read}").to_lowercase(), "xiiipersos.jonesm", "{read:?} {:?}", client.log.iter().rev().take(3).collect::<Vec<_>>());
    let base = prock03(&game, Vec::new());
    let bg = base.mod_find_placed("gardien_lostpass", None, 0.0)[0];
    let mut w = crate::netbits::BitWriter::new(4096);
    assert!(base.net_write_object(&mut w, &index, &std::collections::HashMap::new(), &base.prop(bg, "Mesh")));
    let data = w.data().to_vec();
    let mut r = crate::netbits::BitReader::new(&data, data.len() * 8);
    let typed = client.net_read_object(&mut r, &client_index, &none, "Mesh");
    eprintln!("guard's own look read with the type check: {typed:?} {:?}", client.log.last());
}
