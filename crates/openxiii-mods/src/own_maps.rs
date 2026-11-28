//! The mods' own maps: the `.unr` files of a mod's `maps` folder, with the packages they need
//! (`.utx`, `.usx`, `.uax`, `.ukx`, `.u` files of the same folder). The game finds them by
//! name, after its own Maps folder (openxiii-package, `library::map_path`).

use std::path::{Path, PathBuf};

/// Name of the folder of a mod that holds its maps.
pub const FOLDER: &str = "maps";

/// Package extensions a mod's maps folder may hold besides maps.
const PACKAGES: &[&str] = &["u", "usx", "utx", "uax", "ukx"];

/// A map or a package of a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModPackage {
    /// Name without extension ("BeachNight").
    pub name: String,
    pub path: PathBuf,
    /// A map (`.unr`), else a package it needs.
    pub map: bool,
    /// Folder name of the mod.
    pub mod_name: String,
}

/// The maps and packages of `<mod folder>/maps`, in name order.
pub fn find(mod_folder: &Path, mod_name: &str) -> Vec<ModPackage> {
    let Ok(entries) = std::fs::read_dir(mod_folder.join(FOLDER)) else { return Vec::new() };
    let mut out: Vec<ModPackage> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|path| {
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            let map = ext == "unr";
            if !map && !PACKAGES.contains(&ext.as_str()) {
                return None;
            }
            Some(ModPackage { name: path.file_stem()?.to_string_lossy().into_owned(), path, map, mod_name: mod_name.to_string() })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_and_packages_of_a_mod() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find(tmp.path(), "m").is_empty());
        let dir = tmp.path().join(FOLDER);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["BeachNight.unr", "NightSky.utx", "notes.txt", "Props.usx"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let found = find(tmp.path(), "m");
        let names: Vec<(&str, bool)> = found.iter().map(|p| (p.name.as_str(), p.map)).collect();
        assert_eq!(names, [("BeachNight", true), ("NightSky", false), ("Props", false)]);
    }
}
