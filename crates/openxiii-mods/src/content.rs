//! Sound banks and packages a mod asks to load: `load_sound_banks` and `load_packages`, at the
//! top of a mod file (every map) or in a `[map.<Map>]` table (that map only); `preload` lists
//! objects (`Package.Name`) the game should get ready before the level starts.
//!
//! ```toml
//! load_sound_banks = ["SPADS01"]        # Sounds/PC/<name>.hxc, on every map
//!
//! [map.Plage01]
//! load_sound_banks = ["USA01"]          # only on Plage01
//! load_packages = ["XIIIPersos"]        # a content package (sounds, textures, meshes…)
//! preload = ["XIIIPersos.SpadsM"]       # an object made ready before the level starts
//! ```
//!
//! Most of the time these lists are not needed: a sound, texture or mesh named in a mod value
//! is loaded with its package and sound bank when the level starts. They are for content no
//! mod value names, such as the voice lines a level's script plays.

use std::path::Path;

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// What to load, everywhere (`map` None) or on one map.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadList {
    pub map: Option<String>,
    /// Sound bank names (Sounds/PC/<name>.hxc, without the extension).
    pub sound_banks: Vec<String>,
    /// Package names (without the extension).
    pub packages: Vec<String>,
    /// Objects to make ready before the level starts (`Package.Name` or `Package.Group.Name`).
    pub preload: Vec<String>,
    pub source: Source,
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// An object's name: names separated by dots, at least a package and a name.
pub fn valid_object_name(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| valid_name(p))
}

/// Reads one `load_sound_banks` or `load_packages` value. A wrong name gives an error with the
/// line and is skipped.
pub(crate) fn names(text: &str, file: &Path, key: &str, value: &Spanned<toml::Value>, problems: &mut Vec<Problem>) -> Vec<String> {
    let line = line_of(text, value.span().start);
    let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: m };
    let toml::Value::Array(items) = value.get_ref() else {
        problems.push(err(format!("{key} must be a list of names, for example [\"SPADS01\"]; ignored")));
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            toml::Value::String(s) if key == "preload" && valid_object_name(s.trim()) => out.push(s.trim().to_string()),
            toml::Value::String(s) if key != "preload" && valid_name(s.trim()) => out.push(s.trim().to_string()),
            v if key == "preload" => problems.push(err(format!("preload: {v} is not an object's name (Package.Name); skipped"))),
            v => problems.push(err(format!("{key}: {v} is not a name (no folder, no extension); skipped"))),
        }
    }
    out
}

/// A LoadList from the three optional keys, or None when they are all empty.
pub(crate) fn list(
    text: &str,
    file: &Path,
    mod_name: &str,
    map: Option<&str>,
    banks: Option<&Spanned<toml::Value>>,
    packages: Option<&Spanned<toml::Value>>,
    preload: Option<&Spanned<toml::Value>>,
    problems: &mut Vec<Problem>,
) -> Option<LoadList> {
    let line = banks.or(packages).or(preload).map(|v| line_of(text, v.span().start))?;
    let sound_banks = banks.map(|v| names(text, file, "load_sound_banks", v, problems)).unwrap_or_default();
    let packages = packages.map(|v| names(text, file, "load_packages", v, problems)).unwrap_or_default();
    let preload = preload.map(|v| names(text, file, "preload", v, problems)).unwrap_or_default();
    if sound_banks.is_empty() && packages.is_empty() && preload.is_empty() {
        return None;
    }
    Some(LoadList {
        map: map.map(str::to_string),
        sound_banks,
        packages,
        preload,
        source: Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line },
    })
}

#[cfg(test)]
mod tests {
    use crate::defaults::parse_mod_file;
    use std::path::Path;

    #[test]
    fn global_and_map_lists() {
        let text = "load_sound_banks = [\"SPADS01\"]\n\n[map.Plage01]\nload_sound_banks = [\"USA01\", \"bad/name\"]\nload_packages = [\"XIIIPersos\"]\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert_eq!(f.loads.len(), 2, "{:?}", f.loads);
        let global = f.loads.iter().find(|l| l.map.is_none()).unwrap();
        assert_eq!(global.sound_banks, ["SPADS01"]);
        assert_eq!(global.source.line, 1);
        let local = f.loads.iter().find(|l| l.map.is_some()).unwrap();
        assert_eq!(local.map.as_deref(), Some("Plage01"));
        assert_eq!(local.sound_banks, ["USA01"]);
        assert_eq!(local.packages, ["XIIIPersos"]);
        assert_eq!(f.problems.len(), 1);
        assert!(f.problems[0].to_string().contains("(line 4)"), "{}", f.problems[0]);
        let f = parse_mod_file("load_packages = \"XIIIPersos\"\n", Path::new("m/mod.toml"), "m");
        assert!(f.loads.is_empty());
        assert!(f.problems[0].to_string().contains("must be a list"), "{}", f.problems[0]);
    }

    /// `preload` takes objects' names (a package and a name, with dots), at the top or on a map;
    /// a bare name is refused with its line (negative control: the same list as a package list is
    /// refused there).
    #[test]
    fn preload_lists_objects() {
        let text = "preload = [\"XIIIPersos.SpadsM\", \"Bare\"]\n\n[map.Plage01]\npreload = [\"XIIISound.Ambient.SancFire\"]\n";
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        assert_eq!(f.loads.len(), 2, "{:?} {:?}", f.loads, f.problems);
        let global = f.loads.iter().find(|l| l.map.is_none()).unwrap();
        assert_eq!(global.preload, ["XIIIPersos.SpadsM"]);
        assert!(global.packages.is_empty() && global.sound_banks.is_empty());
        let local = f.loads.iter().find(|l| l.map.is_some()).unwrap();
        assert_eq!(local.preload, ["XIIISound.Ambient.SancFire"]);
        assert_eq!(f.problems.len(), 1, "{:?}", f.problems);
        assert!(f.problems[0].to_string().contains("(line 1)") && f.problems[0].to_string().contains("Package.Name"), "{}", f.problems[0]);
        let f = parse_mod_file("load_packages = [\"XIIIPersos.SpadsM\"]\n", Path::new("m/mod.toml"), "m");
        assert!(f.loads.is_empty() && f.problems.len() == 1, "a package list takes no dots");
        assert!(super::valid_object_name("A.B.C") && !super::valid_object_name("A") && !super::valid_object_name("A..B"));
    }

    /// A when rule on a shot: who is optional ("all": every character), and it gives the
    /// shooter (actor) and the weapon's class. Negative control: amount is not given.
    #[test]
    fn fire_rule() {
        let read = |s: &str| crate::when::read(&s.parse::<toml::Table>().unwrap());
        let all = read("event = \"fire\"\nwho = \"all\"\nthen = [{ message = \"class\" }]\n").unwrap();
        assert_eq!(all.event, crate::when::Event::Fire { who: Some("all".into()) });
        let players = read("event = \"fire\"\nthen = [{ kill = \"actor\" }]\n").unwrap();
        assert_eq!(players.event, crate::when::Event::Fire { who: None });
        assert!(read("event = \"fire\"\nthen = [{ kill = \"amount\" }]\n").is_err_and(|e| e.contains("the event fire gives actor, class")));
    }
}
