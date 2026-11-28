//! Class default values set by mods: `[class."Package.Class"]` tables.
//!
//! ```toml
//! [class."XIII.Beretta"]
//! ReloadCount = 17
//! ShotTime = 0.35
//! FireOffset = { X = 9.0, Y = 5.0, Z = -4.0 }   # struct: only the given fields change
//! "SomeArray[2]" = 4                              # one element of a static array
//! ```
//!
//! This module only reads the files. The game's VM converts each value to the declared type of
//! the property when the class's default values are loaded (openxiii-vm, feature `mods`).

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Spanned;

use crate::{line_of, Mod, Problem};

/// A value from a mod file, before it is converted to the property's type.
#[derive(Debug, Clone, PartialEq)]
pub enum ModValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// A string: text, name, enum value name, or object/class reference.
    Str(String),
    /// A TOML array: dynamic array property.
    Array(Vec<ModValue>),
    /// An inline table: struct property, only the given fields change.
    Table(Vec<(String, ModValue)>),
}

impl fmt::Display for ModValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModValue::Int(v) => write!(f, "{v}"),
            ModValue::Float(v) => write!(f, "{v:?}"),
            ModValue::Bool(v) => write!(f, "{v}"),
            ModValue::Str(v) => write!(f, "{v:?}"),
            ModValue::Array(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{v}")?;
                }
                write!(f, "]")
            }
            ModValue::Table(fields) => {
                write!(f, "{{ ")?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k} = {v}")?;
                }
                write!(f, " }}")
            }
        }
    }
}

/// One property default changed by a mod.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultOverride {
    /// Class as written by the mod: "Package.Class".
    pub class: String,
    /// Property name as written by the mod (without the index).
    pub property: String,
    /// Element of a static array (`"Prop[2]"`), else None.
    pub index: Option<usize>,
    pub value: ModValue,
    /// Folder name of the mod that sets it.
    pub mod_name: String,
    pub file: PathBuf,
    pub line: usize,
    /// `[map.<Map>.class."Package.Class"]`: only on that map (as written), applied to the actors
    /// of the class and its subclasses present when the level starts. None: a class default.
    pub map: Option<String>,
    /// `apply_to_placed = true` in the class table: besides the class default, also set on the
    /// actors the map placed, which can carry their own value saved in the map.
    pub apply_to_placed: bool,
}

impl DefaultOverride {
    /// "Package.Class" in lower case: the lookup key.
    pub fn class_key(&self) -> String {
        self.class.to_lowercase()
    }

    /// Property as written, with its index: "ReloadCount", "Prop[2]".
    pub fn property_label(&self) -> String {
        match self.index {
            Some(i) => format!("{}[{i}]", self.property),
            None => self.property.clone(),
        }
    }

    /// Where the value comes from, for log lines: "mod example, <Mods folder>/example/mod.toml line 5".
    pub fn source(&self) -> String {
        format!("mod {}, {} line {}", self.mod_name, self.file.display(), self.line)
    }

    /// "Map: Package.Class" for a map-scoped value, else "Package.Class".
    pub fn scope_label(&self) -> String {
        match &self.map {
            Some(m) => format!("{m}: {}", self.class),
            None => self.class.clone(),
        }
    }
}

/// Reserved key of a class table: also change the actors placed by the maps.
pub const APPLY_TO_PLACED: &str = "apply_to_placed";

type ClassTables = BTreeMap<String, BTreeMap<String, Spanned<toml::Value>>>;

/// The blocks a mod file can have (`[mod]` is read by the loader).
const TOP_BLOCKS: &[&str] = &["mod", "class", "map", "shortcut", "action", "character", "load_sound_banks", "load_packages", "preload", "sounds", "textures", "models", "arms", "classes"];

#[derive(Debug, Deserialize)]
struct ClassFile {
    #[serde(default)]
    class: ClassTables,
    #[serde(default)]
    map: BTreeMap<String, MapScope>,
    /// `[[shortcut]]` (documented) and `[[action]]` (its earlier name, still read): keyboard
    /// shortcuts.
    #[serde(default)]
    action: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    shortcut: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    character: crate::behaviour::CharacterTables,
    #[serde(default)]
    load_sound_banks: Option<Spanned<toml::Value>>,
    #[serde(default)]
    load_packages: Option<Spanned<toml::Value>>,
    #[serde(default)]
    preload: Option<Spanned<toml::Value>>,
    #[serde(default)]
    sounds: crate::own_sounds::SoundTables,
    #[serde(default)]
    textures: crate::own_textures::TextureTables,
    #[serde(default)]
    models: crate::own_models::ModelTables,
    #[serde(default)]
    arms: crate::own_arms::ArmsTables,
    #[serde(default)]
    classes: crate::new_classes::ClassDefTables,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapScope {
    #[serde(default)]
    class: ClassTables,
    #[serde(default)]
    add: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    remove: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    replace: Option<Spanned<toml::Table>>,
    #[serde(default)]
    start: Option<Spanned<toml::Table>>,
    #[serde(default)]
    say: Vec<Spanned<toml::Table>>,
    /// `[[map.<Map>.change]]` (placement.rs).
    #[serde(default)]
    change: Vec<Spanned<toml::Table>>,
    /// `[map.<Map>.replace_sounds]` (placement.rs).
    #[serde(default)]
    replace_sounds: Option<Spanned<toml::Table>>,
    /// `[[map.<Map>.when]]` (when.rs).
    #[serde(default)]
    when: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    load_sound_banks: Option<Spanned<toml::Value>>,
    #[serde(default)]
    load_packages: Option<Spanned<toml::Value>>,
    #[serde(default)]
    preload: Option<Spanned<toml::Value>>,
    /// `[map.<Map>.checkpoint.<N>.start]`.
    #[serde(default)]
    checkpoint: BTreeMap<String, CheckpointScope>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointScope {
    #[serde(default)]
    start: Option<Spanned<toml::Table>>,
}

/// Everything one mod file sets.
#[derive(Debug, Default)]
pub struct ModFile {
    pub overrides: Vec<DefaultOverride>,
    pub edits: Vec<crate::placement::MapEdit>,
    pub actions: Vec<crate::behaviour::ModAction>,
    pub characters: Vec<crate::behaviour::CharacterDef>,
    pub loads: Vec<crate::content::LoadList>,
    pub sounds: Vec<crate::own_sounds::OwnSound>,
    pub textures: Vec<crate::own_textures::OwnTexture>,
    pub models: Vec<crate::own_models::OwnModel>,
    pub arms: Vec<crate::own_arms::OwnArms>,
    pub new_classes: Vec<crate::new_classes::NewClass>,
    pub problems: Vec<Problem>,
}

/// Reads the `[class."Package.Class"]` tables of one file. A value that cannot be used gives an
/// error (with its line) and is skipped; the other values are kept.
pub fn parse_class_tables(text: &str, file: &Path, mod_name: &str) -> (Vec<DefaultOverride>, Vec<Problem>) {
    let f = parse_mod_file(text, file, mod_name);
    (f.overrides, f.problems)
}

/// Reads the class tables and the map changes (placement.rs) of one file. A value or entry
/// that cannot be used gives an error (with its line) and is skipped; the others are kept.
pub fn parse_mod_file(text: &str, file: &Path, mod_name: &str) -> ModFile {
    parse_mod_file_in(text, file, mod_name, file.parent().unwrap_or(Path::new("")))
}

/// [`parse_mod_file`] of a file of mod folder `mod_dir` (the files a mod names, such as its
/// sounds, are found from the mod folder, also for a file of its `maps` sub-folder).
pub fn parse_mod_file_in(text: &str, file: &Path, mod_name: &str, mod_dir: &Path) -> ModFile {
    let mut out = Vec::new();
    let mut problems = Vec::new();
    let mut edits = Vec::new();
    let mut loads = Vec::new();
    let parsed: ClassFile = match toml::from_str(text) {
        Ok(p) => p,
        Err(e) => {
            let line = e.span().map(|s| line_of(text, s.start));
            problems.push(Problem::Error { file: file.to_path_buf(), line, message: format!("{}; file skipped", e.message().trim()) });
            return ModFile { overrides: out, edits, problems, ..Default::default() };
        }
    };
    loads.extend(crate::content::list(text, file, mod_name, None, parsed.load_sound_banks.as_ref(), parsed.load_packages.as_ref(), parsed.preload.as_ref(), &mut problems));
    let ctx = Ctx { text, file, mod_name };
    let mut characters = Vec::new();
    for (class, props) in parsed.class {
        ctx.table(&class, props, None, &mut out, &mut problems);
    }
    for (map, scope) in parsed.map {
        if !crate::maps::valid_selector(&map) {
            problems.push(Problem::Error {
                file: file.to_path_buf(),
                line: first_line(text, &scope.class),
                message: format!("map \"{map}\" is not a map name (the file name without .unr, for example Plage01), a beginning of map names (\"Hual*\"), \"solo\", \"multi\" or \"*\"; tables skipped"),
            });
            continue;
        }
        let MapScope { class, add, remove, replace, start, say, change, replace_sounds, when, load_sound_banks, load_packages, preload, checkpoint } = scope;
        let checkpoint_start = checkpoint.into_iter().filter_map(|(n, c)| c.start.map(|s| (n, s))).collect();
        loads.extend(crate::content::list(text, file, mod_name, Some(&map), load_sound_banks.as_ref(), load_packages.as_ref(), preload.as_ref(), &mut problems));
        for (class, props) in class {
            ctx.table(&class, props, Some(&map), &mut out, &mut problems);
        }
        let reader = crate::placement::Reader { text, file, mod_name };
        reader.map_actors(
            &map,
            crate::placement::MapActors { add, remove, replace, start, say, change, replace_sounds, when, checkpoint_start },
            &mut edits,
            &mut characters,
            &mut problems,
        );
    }
    let mut actions = Vec::new();
    let reader = crate::behaviour::Reader { text, file, mod_name };
    // Keyboard shortcuts, `[[shortcut]]` and `[[action]]` alike, in the order of the file.
    let mut shortcuts = parsed.action;
    shortcuts.extend(parsed.shortcut);
    shortcuts.sort_by_key(|s| s.span().start);
    reader.actions(shortcuts, &mut actions, &mut characters, &mut problems);
    // A block name the game does not know: one warning with the closest names (a typo would
    // otherwise be ignored without a word).
    if let Ok(top) = text.parse::<toml::Table>() {
        for name in top.keys().filter(|k| !TOP_BLOCKS.contains(&k.as_str())) {
            let close = crate::looks::closest(name, TOP_BLOCKS.iter().copied().filter(|b| *b != "action"), 2);
            let line = text.lines().position(|l| l.trim_start().trim_start_matches('[').starts_with(name.as_str())).map(|i| i + 1);
            problems.push(Problem::Error { file: file.to_path_buf(), line, message: format!("unknown block {name}{}; ignored", crate::looks::did_you_mean(&close)) });
        }
    }
    reader.characters(parsed.character, &mut characters, &mut problems);
    let mut sounds = Vec::new();
    crate::own_sounds::read(text, file, mod_name, mod_dir, parsed.sounds, &mut sounds, &mut problems);
    let mut textures = Vec::new();
    crate::own_textures::read(text, file, mod_name, mod_dir, parsed.textures, &mut textures, &mut problems);
    let mut models = Vec::new();
    crate::own_models::read(text, file, mod_name, mod_dir, parsed.models, &mut models, &mut problems);
    let mut arms = Vec::new();
    crate::own_arms::read(text, file, mod_name, parsed.arms, &mut arms, &mut problems);
    let mut new_classes = Vec::new();
    crate::new_classes::read(text, file, mod_name, parsed.classes, &mut new_classes, &mut problems);
    ModFile { overrides: out, edits, actions, characters, loads, sounds, textures, models, arms, new_classes, problems }
}

fn first_line(text: &str, tables: &ClassTables) -> Option<usize> {
    tables.values().flat_map(|t| t.values()).map(|v| line_of(text, v.span().start)).min()
}

/// The file being read.
struct Ctx<'a> {
    text: &'a str,
    file: &'a Path,
    mod_name: &'a str,
}

impl Ctx<'_> {
    /// One `[class."Package.Class"]` table, global or of map `map`.
    fn table(
        &self,
        class: &str,
        mut props: BTreeMap<String, Spanned<toml::Value>>,
        map: Option<&str>,
        out: &mut Vec<DefaultOverride>,
        problems: &mut Vec<Problem>,
    ) {
        let line_at = |v: &Spanned<toml::Value>| line_of(self.text, v.span().start);
        let err = |line: Option<usize>, message: String| Problem::Error { file: self.file.to_path_buf(), line, message };
        let scope = map.map_or(String::new(), |m| format!("map {m}: "));
        if !valid_class_name(class) {
            let line = props.values().map(line_at).min();
            problems.push(err(line, format!("{scope}class \"{class}\" must be written \"Package.Class\" (for example \"XIII.Beretta\"); table skipped")));
            return;
        }
        let mut apply_to_placed = false;
        if let Some(v) = props.remove(APPLY_TO_PLACED) {
            match (v.get_ref(), map) {
                (toml::Value::Boolean(b), None) => apply_to_placed = *b,
                (toml::Value::Boolean(_), Some(_)) => problems.push(Problem::Warning {
                    path: self.file.to_path_buf(),
                    message: format!("line {}: {scope}{class}: {APPLY_TO_PLACED} is not needed in a map table (map values always apply to the actors of the map); ignored", line_at(&v)),
                }),
                _ => problems.push(err(Some(line_at(&v)), format!("{scope}{class}.{APPLY_TO_PLACED} must be true or false; ignored"))),
            }
        }
        for (key, value) in props {
            let line = line_at(&value);
            let Some((property, index)) = parse_key(&key) else {
                problems.push(err(Some(line), format!("{scope}{class}: \"{key}\" is not a property name (use Name or \"Name[2]\"); value skipped")));
                continue;
            };
            match convert(value.get_ref()) {
                Ok(v) => out.push(DefaultOverride {
                    class: class.to_string(),
                    property,
                    index,
                    value: v,
                    mod_name: self.mod_name.to_string(),
                    file: self.file.to_path_buf(),
                    line,
                    map: map.map(str::to_string),
                    apply_to_placed,
                }),
                Err(m) => problems.push(err(Some(line), format!("{scope}{class}.{key}: {m}; value skipped"))),
            }
        }
    }
}

pub(crate) fn valid_class_name(s: &str) -> bool {
    let mut parts = s.split('.');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// "Name" or "Name[2]".
pub(crate) fn parse_key(key: &str) -> Option<(String, Option<usize>)> {
    let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    match key.split_once('[') {
        None => ident(key).then(|| (key.to_string(), None)),
        Some((name, rest)) => {
            let index = rest.strip_suffix(']')?;
            if !ident(name) || index.is_empty() || !index.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some((name.to_string(), Some(index.parse().ok()?)))
        }
    }
}

pub(crate) fn convert(v: &toml::Value) -> Result<ModValue, String> {
    Ok(match v {
        toml::Value::Integer(i) => ModValue::Int(*i),
        toml::Value::Float(f) => ModValue::Float(*f),
        toml::Value::Boolean(b) => ModValue::Bool(*b),
        toml::Value::String(s) => ModValue::Str(s.clone()),
        toml::Value::Array(items) => ModValue::Array(items.iter().map(convert).collect::<Result<_, _>>()?),
        toml::Value::Table(t) => ModValue::Table(t.iter().map(|(k, v)| Ok((k.clone(), convert(v)?))).collect::<Result<_, String>>()?),
        toml::Value::Datetime(_) => return Err("dates are not supported".into()),
    })
}

/// All the overrides of `mods`, in mod order (folder order, then file order inside a mod). When
/// two values set the same property, the later one wins and a warning says so.
pub fn collect_overrides(mods: &[Mod]) -> (Vec<DefaultOverride>, Vec<Problem>) {
    collect_overrides_into(mods, &mut Vec::new())
}

/// [`collect_overrides`], also noting in `conflicts` each value one mod sets over another's.
pub(crate) fn collect_overrides_into(mods: &[Mod], conflicts: &mut Vec<crate::Conflict>) -> (Vec<DefaultOverride>, Vec<Problem>) {
    // First the values of the decorations' class (when a mod adds one): a mod may change them.
    let mut out: Vec<DefaultOverride> = crate::decoration_overrides(mods);
    // Those values are the engine's, not a mod's choice: changing them is no conflict.
    let decorations = out.len();
    let mut seen: HashMap<(Option<String>, String, String, Option<usize>), usize> = out.iter().enumerate().map(|(i, o)| ((None, o.class_key(), o.property.to_lowercase(), None), i)).collect();
    let mut problems = Vec::new();
    for m in mods {
        for o in &m.overrides {
            let key = (o.map.as_ref().map(|m| m.to_lowercase()), o.class_key(), o.property.to_lowercase(), o.index);
            match seen.get(&key) {
                Some(&i) => {
                    let old = &out[i];
                    if i >= decorations {
                        crate::Conflict::note(conflicts, &old.mod_name, &o.mod_name, format!("value {}.{}", o.scope_label(), o.property_label()));
                    }
                    problems.push(Problem::Warning {
                        path: o.file.clone(),
                        message: format!(
                            "{}.{} = {} (line {}) replaces {} set by mod {} ({} line {})",
                            o.scope_label(),
                            o.property_label(),
                            o.value,
                            o.line,
                            old.value,
                            old.mod_name,
                            old.file.display(),
                            old.line
                        ),
                    });
                    out[i] = o.clone();
                }
                None => {
                    seen.insert(key, out.len());
                    out.push(o.clone());
                }
            }
        }
    }
    (out, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Vec<DefaultOverride>, Vec<Problem>) {
        parse_class_tables(text, Path::new("m/mod.toml"), "m")
    }

    #[test]
    fn all_value_kinds() {
        let text = r#"
[mod]
name = "x"
version = "1"

[class."XIII.Beretta"]
ReloadCount = 17
ShotTime = 0.35
bUseSilencer = false
WeaponMode = "WM_Auto"
FireOffset = { X = 9.0, Y = 5.0, Z = -4 }
"Sounds[2]" = "Sound'XIIIsound.A.B'"
List = [1, 2, 3]
"#;
        let (o, p) = parse(text);
        assert!(p.is_empty(), "{p:?}");
        let get = |name: &str| o.iter().find(|x| x.property == name).unwrap();
        assert_eq!(get("ReloadCount").value, ModValue::Int(17));
        assert_eq!(get("ReloadCount").line, 7);
        assert_eq!(get("ShotTime").value, ModValue::Float(0.35));
        assert_eq!(get("bUseSilencer").value, ModValue::Bool(false));
        assert_eq!(get("WeaponMode").value, ModValue::Str("WM_Auto".into()));
        assert_eq!(
            get("FireOffset").value,
            ModValue::Table(vec![("X".into(), ModValue::Float(9.0)), ("Y".into(), ModValue::Float(5.0)), ("Z".into(), ModValue::Int(-4))])
        );
        let s = get("Sounds");
        assert_eq!(s.index, Some(2));
        assert_eq!(s.property_label(), "Sounds[2]");
        assert_eq!(get("List").value, ModValue::Array(vec![ModValue::Int(1), ModValue::Int(2), ModValue::Int(3)]));
        assert!(o.iter().all(|x| x.class == "XIII.Beretta" && x.mod_name == "m"));
    }

    #[test]
    fn bad_keys_and_values_are_skipped_with_line() {
        let text = "[class.\"XIII.Beretta\"]\nReloadCount = 17\n\"Bad[x]\" = 1\nWhen = 1979-05-27\n\n[class.\"NoPackage\"]\nA = 1\n";
        let (o, p) = parse(text);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].property, "ReloadCount");
        assert_eq!(p.len(), 3, "{p:?}");
        let lines: Vec<_> = p.iter().map(|x| match x {
            Problem::Error { line, .. } => *line,
            _ => None,
        }).collect();
        assert!(lines.contains(&Some(3)), "{lines:?}");
        assert!(lines.contains(&Some(4)), "{lines:?}");
        assert!(lines.contains(&Some(7)), "{lines:?}");
        let all: String = p.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("\n");
        assert!(all.contains("Package.Class"), "{all}");
        assert!(all.contains("dates are not supported"), "{all}");
    }

    #[test]
    fn syntax_error_reports_line() {
        let (o, p) = parse("[class.\"XIII.Beretta\"]\nReloadCount = 17\nShotTime = = 2\n");
        assert!(o.is_empty());
        assert!(matches!(&p[0], Problem::Error { line: Some(3), .. }), "{p:?}");
    }

    #[test]
    fn keys() {
        assert_eq!(parse_key("ReloadCount"), Some(("ReloadCount".into(), None)));
        assert_eq!(parse_key("A[12]"), Some(("A".into(), Some(12))));
        assert_eq!(parse_key("A[]"), None);
        assert_eq!(parse_key("A[1"), None);
        assert_eq!(parse_key("A.B"), None);
        assert!(valid_class_name("XIII.c9mmAmmo"));
        assert!(!valid_class_name("c9mmAmmo"));
        assert!(!valid_class_name("A.B.C"));
    }

    fn mod_with(name: &str, text: &str) -> Mod {
        let (overrides, p) = parse_class_tables(text, Path::new(&format!("{name}/mod.toml")), name);
        assert!(p.is_empty());
        Mod {
            folder_name: name.into(),
            folder: PathBuf::from(name),
            info: crate::ModInfo { name: name.into(), version: "1".into(), author: None, description: None, openxiii: None },
            overrides,
            edits: Vec::new(),
            actions: Vec::new(),
            characters: Vec::new(),
            loads: Vec::new(),
            sounds: Vec::new(),
            textures: Vec::new(),
            models: Vec::new(),
            arms: Vec::new(),
            new_classes: Vec::new(),
            scripts: Vec::new(),
        }
    }

    #[test]
    fn later_mod_wins_with_warning() {
        let a = mod_with("a", "[class.\"XIII.Beretta\"]\nReloadCount = 17\nShotTime = 0.3\n");
        let b = mod_with("b", "[class.\"xiii.beretta\"]\nreloadcount = 30\n");
        let (o, p) = collect_overrides(&[a, b]);
        assert_eq!(o.len(), 2);
        let rc = o.iter().find(|x| x.property.eq_ignore_ascii_case("ReloadCount")).unwrap();
        assert_eq!(rc.value, ModValue::Int(30));
        assert_eq!(rc.mod_name, "b");
        assert_eq!(p.len(), 1);
        let w = p[0].to_string();
        assert!(w.starts_with("warning:"), "{w}");
        assert!(w.contains("replaces 17 set by mod a"), "{w}");
    }

    #[test]
    fn map_scope_and_apply_to_placed() {
        let text = r#"
[class."XIDPawn.BaseSoldier"]
Health = 200
apply_to_placed = true

[class."XIII.Beretta"]
ReloadCount = 17

[map.Plage01.class."XIDPawn.BaseSoldier"]
Health = 300
"#;
        let (o, p) = parse(text);
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(o.len(), 3);
        let global = o.iter().find(|x| x.class == "XIDPawn.BaseSoldier" && x.map.is_none()).unwrap();
        assert!(global.apply_to_placed);
        assert_eq!(global.value, ModValue::Int(200));
        assert!(o.iter().all(|x| x.property != APPLY_TO_PLACED));
        let beretta = o.iter().find(|x| x.class == "XIII.Beretta").unwrap();
        assert!(!beretta.apply_to_placed);
        let local = o.iter().find(|x| x.map.is_some()).unwrap();
        assert_eq!(local.map.as_deref(), Some("Plage01"));
        assert_eq!(local.value, ModValue::Int(300));
        assert_eq!(local.line, 10);
        assert_eq!(local.scope_label(), "Plage01: XIDPawn.BaseSoldier");
    }

    #[test]
    fn scope_errors() {
        let text = "[class.\"XIDPawn.BaseSoldier\"]\nHealth = 1\napply_to_placed = \"yes\"\n\n[map.Plage01.class.\"XIDPawn.BaseSoldier\"]\nHealth = 2\napply_to_placed = true\n\n[map.\"Bad Map\".class.\"XIDPawn.BaseSoldier\"]\nHealth = 3\n";
        let (o, p) = parse(text);
        assert_eq!(o.len(), 2, "{o:?}");
        assert!(!o[0].apply_to_placed);
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(all.iter().any(|x| x.contains("(line 3)") && x.contains("must be true or false")), "{all:?}");
        assert!(all.iter().any(|x| x.starts_with("warning:") && x.contains("line 7") && x.contains("not needed in a map table")), "{all:?}");
        assert!(all.iter().any(|x| x.contains("(line 10)") && x.contains("is not a map name")), "{all:?}");
        // A typo inside a map scope is an error of the whole file, with its line.
        let (o, p) = parse("[class.\"XIII.Beretta\"]\nReloadCount = 17\n\n[map.Plage01.clas.\"XIII.Beretta\"]\nReloadCount = 1\n");
        assert!(o.is_empty());
        assert!(matches!(&p[0], Problem::Error { line: Some(4), .. }), "{p:?}");
    }

    #[test]
    fn map_and_global_values_do_not_conflict() {
        let a = mod_with("a", "[class.\"XIDPawn.BaseSoldier\"]\nHealth = 200\n[map.Plage01.class.\"XIDPawn.BaseSoldier\"]\nHealth = 300\n");
        let b = mod_with("b", "[map.plage01.class.\"XIDPawn.BaseSoldier\"]\nHealth = 400\n");
        let (o, p) = collect_overrides(&[a, b]);
        assert_eq!(o.len(), 2, "{o:?}");
        assert!(o.iter().any(|x| x.map.is_none() && x.value == ModValue::Int(200)));
        // Same map in another spelling: the later mod wins, with a warning.
        assert!(o.iter().any(|x| x.map.is_some() && x.value == ModValue::Int(400) && x.mod_name == "b"));
        assert_eq!(p.len(), 1);
        assert!(p[0].to_string().contains("plage01: XIDPawn.BaseSoldier.Health = 400"), "{}", p[0]);
    }

    #[test]
    fn different_indexes_do_not_conflict() {
        let a = mod_with("a", "[class.\"XIII.X\"]\n\"P[0]\" = 1\n\"P[1]\" = 2\n");
        let (o, p) = collect_overrides(&[a]);
        assert_eq!(o.len(), 2);
        assert!(p.is_empty());
    }
}
