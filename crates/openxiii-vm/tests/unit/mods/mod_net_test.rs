//! Mods build only: a mod's new class and a mod's sound designated over the network by their
//! rank in the mods' package: what the server writes, a client with the same mods reads back
//! as the same class and sound (not the parent class, nor nothing). Runs on the real game's
//! scripts (OXIII_GAME = original game folder; skipped when it is not set).

use std::path::PathBuf;

use openxiii_mods::new_classes::NewClass;
use openxiii_mods::placement::Source;
use openxiii_mods::ModId;

use crate::mod_net::ModNetwork;
use crate::netbits::{BitReader, BitWriter};
use crate::value::Value;
use crate::Vm;

#[test]
fn mod_class_and_sound_cross_the_network() {
    let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| g.join("System").exists()) else {
        eprintln!("OXIII_GAME not set: skipped");
        return;
    };
    let source = Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 };
    let ids = vec![ModId { name: "test".into(), version: "1".into(), fingerprint: "0123456789ABCDEF".into() }];
    let objects = vec![("Mod.NetPistol".to_string(), "Class"), ("Mod.NetShot".to_string(), "Sound"), ("Mod.NetCrate".to_string(), "StaticMesh")];
    let start = |vm: &mut Vm| {
        vm.ipdrv.offline = true;
        vm.set_mod_classes_here(vec![NewClass { name: "NetPistol".into(), parent: "XIII.Beretta".into(), source: source.clone() }]);
        vm.set_mod_network_here(ModNetwork::new(ids.clone(), objects.clone()));
        vm.load_dir(&game.join("System")).unwrap();
    };
    let (mut server, mut client) = (Vm::new(), Vm::new());
    start(&mut server);
    start(&mut client);
    let class = server.class("Mod.NetPistol").expect("new class");
    let beretta = server.class("XIII.Beretta").unwrap();
    // The package map: a game package, then the mods' package (as the server announces it).
    let mut map = crate::net_map::PackageMap::default();
    let mut load = |name: &str| server.library.as_mut().and_then(|l| l.get(name)).map(|lp| (lp.name.clone(), lp.path.display().to_string(), lp.package.clone(), lp.data.len()));
    map.add_linker("XIII", &mut load);
    map.compute();
    server.mod_net_add_package(&mut map);
    let index = server.net_build_index(&map);
    let mods = index.packages.last().unwrap();
    assert_eq!((mods.name.as_str(), mods.object_count), ("Mod", 3));
    // The class's rank is in the mods' package, not the Beretta's export.
    let rank = server.class_net_index(&index, class).expect("rank");
    assert!(rank >= mods.object_base, "{rank}");
    assert_ne!(Some(rank), server.class_net_index(&index, beretta));
    let channels = std::collections::HashMap::new();
    let mut w = BitWriter::new(4096);
    for v in [Value::Class(Some(class)), Value::Asset("Mod.NetShot".into()), Value::Class(Some(beretta))] {
        assert!(server.net_write_object(&mut w, &index, &channels, &v));
    }
    // The client builds its map from the same announced packages.
    let client_index = client.net_build_index(&map);
    let data = w.data().to_vec();
    let mut r = BitReader::new(&data, data.len() * 8);
    let none = |_: usize| -> Option<crate::value::ObjId> { None };
    let read_class = client.net_read_object(&mut r, &client_index, &none, "Class");
    assert_eq!(read_class, Value::Class(client.class("Mod.NetPistol")), "{:?}", client.log);
    let read_sound = client.net_read_object(&mut r, &client_index, &none, "Sound");
    assert_eq!(read_sound, Value::Asset("Mod.NetShot".into()));
    let read_beretta = client.net_read_object(&mut r, &client_index, &none, "Class");
    assert_eq!(read_beretta, Value::Class(client.class("XIII.Beretta")));
}
