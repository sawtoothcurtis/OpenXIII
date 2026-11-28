use openxiii_package::{PACKAGE_TAG, library::Library};
use std::{fs, path::Path};

// A real UE2 package with a name table and top-level package imports.
fn write_package(path: &Path, imports: &[&str]) {
    let mut names = Vec::new();
    for name in std::iter::once("Package").chain(imports.iter().copied()) {
        names.push((name.len() + 1) as u8);
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        names.extend(0x30000u32.to_le_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend(PACKAGE_TAG.to_le_bytes());
    bytes.extend(100u16.to_le_bytes());
    bytes.extend(58u16.to_le_bytes());
    let import_offset = 56 + names.len() as u32;
    for value in [0, imports.len() as u32 + 1, 56, 0, import_offset, imports.len() as u32, import_offset] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend([0; 20]); // GUID and generation count.
    bytes.extend(names);
    for index in 1..=imports.len() {
        bytes.extend([0, 0, 0, 0, 0, 0, index as u8]);
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn map_imports_are_shared_until_travel_and_respect_per_library_file_overrides() {
    let game = std::env::temp_dir().join(format!(
        "openxiii-map-preload-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let data = game.join("Maps");
    fs::create_dir_all(&data).unwrap();
    write_package(&data.join("Level.unr"), &["First", "FIRST", "Missing", "Hidden", "Remote", "UnusedAfterTravel"]);
    write_package(&data.join("First.usx"), &["Nested"]);
    write_package(&data.join("Nested.utx"), &["Level"]);
    write_package(&data.join("Hidden.u"), &[]);
    write_package(&data.join("Remote.u"), &[]);
    write_package(&data.join("Cache.uxx"), &["Downloaded"]);
    write_package(&data.join("UnusedAfterTravel.u"), &[]);
    write_package(&data.join("Next.unr"), &[]);

    let mut renderer = Library::new(&game).unwrap();
    let mut simulation = renderer.fresh_copy();
    simulation.forget("Hidden");
    simulation.set_file("Remote", data.join("Cache.uxx"));
    renderer.preload_map(["Level"], Some(&mut simulation));
    assert!(renderer.is_loaded("Nested"));
    assert!(!renderer.is_loaded("Next"));
    assert!(!renderer.is_loaded("Missing"));
    assert!(simulation.get("Hidden").is_none());
    assert_eq!(simulation.get("Remote").unwrap().package.name(1), "Downloaded");

    let original = std::ptr::from_ref(renderer.get("Nested").unwrap());
    let shared = simulation.get("nested").unwrap();
    assert!(std::ptr::eq(original, shared));
    simulation.set_file("First", data.join("Cache.uxx"));
    assert_eq!(simulation.get("FIRST").unwrap().package.name(1), "Downloaded");

    // An unused package from the first level is released when the next one starts.
    fs::remove_file(data.join("UnusedAfterTravel.u")).unwrap();
    renderer.preload_map(["Next"], Some(&mut simulation));
    assert!(simulation.get("UnusedAfterTravel").is_none());
    assert!(std::ptr::eq(original, simulation.get("Nested").unwrap()));
    fs::remove_dir_all(game).unwrap();
}
