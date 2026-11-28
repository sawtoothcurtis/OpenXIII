//! The mods' Lua scripts: the `.lua` files of a mod's `scripts` folder, run by the game in the
//! mods build (openxiii-vm, mod_script.rs). This crate only finds them; it does not run Lua.

use std::path::{Path, PathBuf};

/// Name of the folder of a mod that holds its scripts.
pub const FOLDER: &str = "scripts";

/// One script file of a mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFile {
    /// Folder name of the mod.
    pub mod_name: String,
    /// Full path of the file.
    pub path: PathBuf,
    /// Name shown in the log and in error messages: "<mod folder>/scripts/<file>".
    pub label: String,
}

/// The `.lua` files of `<mod folder>/scripts`, in the order of their names (plain byte order,
/// as the mods themselves). Sub-folders are not read.
pub fn find(mod_folder: &Path, mod_name: &str) -> Vec<ScriptFile> {
    let dir = mod_folder.join(FOLDER);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut files: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
        .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.into_iter().map(|(name, path)| ScriptFile { mod_name: mod_name.to_string(), label: format!("{mod_name}/{FOLDER}/{name}"), path }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_files_in_name_order() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find(tmp.path(), "m").is_empty(), "no scripts folder: no scripts");
        let dir = tmp.path().join(FOLDER);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["b.lua", "a.LUA", "Z.lua", "notes.txt", "sub/c.lua"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let found = find(tmp.path(), "m");
        let labels: Vec<&str> = found.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["m/scripts/Z.lua", "m/scripts/a.LUA", "m/scripts/b.lua"]);
        assert!(found.iter().all(|s| s.mod_name == "m" && s.path.is_file()));
    }
}
