//! Keys bound to mod actions (`[[action]]`) and mod characters (`[character.<Name>]`).
//!
//! ```toml
//! [[action]]
//! key = "K"                           # F1…F12, A…Z, 0…9, NumPad0…NumPad9, Insert, Delete,
//!                                     # Home, End, PageUp, PageDown
//! spawn = "Guard"                     # a [character] of the mods, or a class "Package.Class"
//! in_front_of_player = 150            # distance (optional, 150 by default)
//! # or: give = "XIII.Bazook"          # a weapon for the player
//! # or: command = "modpos"            # a console command
//! # or: sound = "Package.Group.Sound" # a sound of the game, played at the player
//!
//! [character.Guard]
//! base = "XIDPawn.BaseSoldier"        # any character class
//! team = "neutral"                    # friend | neutral | enemy: how it treats the player
//! allowed_states = ["Attente"]        # states of its AI it may be in (optional)
//! can_die = true                      # false: cannot be hurt or killed
//! death_animation = "Mort01"          # optional
//! properties = { Health = 200 }       # optional
//!
//! [character.Guard.state.Attente]     # look of the character in one state of its AI
//! animation = "Attente01"
//! sound = "Package.Group.Sound"       # optional
//! loop = true                         # optional, true by default
//! ```
//!
//! This module only reads the files; the game's VM applies them (openxiii-vm, feature `mods`).

use std::collections::BTreeMap;
use std::path::Path;

use toml::Spanned;

use crate::defaults::{convert, parse_key, valid_class_name};
use crate::placement::{ActorValue, Source};
use crate::{line_of, Problem};

/// Default `in_front_of_player` distance.
pub const DEFAULT_DISTANCE: f32 = 150.0;

/// Key names an action can use.
pub fn valid_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    if k.len() == 1 {
        return k.chars().all(|c| c.is_ascii_alphanumeric());
    }
    if let Some(n) = k.strip_prefix("numpad") {
        return n.len() == 1 && n.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(n) = k.strip_prefix('f') {
        return !n.starts_with('0') && matches!(n.parse::<u32>(), Ok(1..=12));
    }
    ["insert", "delete", "home", "end", "pageup", "pagedown"].contains(&k.as_str())
}

/// Keys the game window keeps for itself in some build, with what uses them: a keyboard
/// shortcut or a `key` event of a mod on one of them does not run there.
pub const RESERVED_KEYS: &[(&str, &str)] = &[
    ("F3", "the co-op character screen"),
    ("F4", "the co-op character screen"),
    ("F8", "the co-op character screen"),
    ("F5", "the cheats of the test builds"),
    ("F6", "the cheats of the test builds"),
    ("F7", "the cheats of the test builds"),
    ("F11", "the cheats of the test builds"),
    ("F9", "its own use"),
];

/// Keys free for the mods with the game's own settings (DefUser.ini binds none of them).
pub const FREE_KEYS: &str = "K, L, J, H, U, O, P, Insert, NumPad2 to NumPad9";

/// What the game window uses key `key` for, if it keeps it.
pub fn reserved_key(key: &str) -> Option<&'static str> {
    RESERVED_KEYS.iter().find(|(k, _)| k.eq_ignore_ascii_case(key.trim())).map(|(_, what)| *what)
}

/// The warning for a mod's key `key` the game window keeps, if it does.
pub fn reserved_key_warning(key: &str) -> Option<String> {
    reserved_key(key).map(|what| format!("key {key} is kept by the game window for {what}: it never runs a mod's shortcut or reaction; choose a free key ({FREE_KEYS})"))
}

/// What an action does.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionKind {
    /// Create a [character] (by name) or an actor of a class in front of the player.
    Spawn { what: String, distance: f32 },
    /// Give a weapon to the player.
    Give { class: String },
    /// Run a console command, as if typed by the player.
    Command { text: String },
    /// Play a sound of the game at the player.
    Sound { sound: String },
}

/// A key bound to an action.
#[derive(Debug, Clone, PartialEq)]
pub struct ModAction {
    /// Key name as written ("K").
    pub key: String,
    pub kind: ActionKind,
    pub source: Source,
}

/// How a mod character treats the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Friend,
    Neutral,
    Enemy,
}

/// Look of a character in one state of its AI.
#[derive(Debug, Clone, PartialEq)]
pub struct StateLook {
    pub state: String,
    pub animation: Option<String>,
    pub sound: Option<String>,
    pub looping: bool,
}

/// A `[character.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterDef {
    pub name: String,
    pub base: String,
    pub team: Option<Team>,
    pub allowed_states: Vec<String>,
    pub can_die: bool,
    pub death_animation: Option<String>,
    pub properties: Vec<ActorValue>,
    pub states: Vec<StateLook>,
    pub source: Source,
    /// The simple keys of the table or entry (simple.rs). A table's are already translated into
    /// `base`, `team`, `can_die` and `properties`; a derived definition's are merged by
    /// simple::resolve.
    pub keys: crate::simple::SimpleKeys,
    /// A definition made from an entry's simple keys (`[[map.<Map>.add]]` with `character`,
    /// `[[action]]` with `spawn`, `[[map.<Map>.change]]`): the character it starts from, merged
    /// once every mod is read (simple::resolve). None for a `[character.<Name>]` table.
    pub like: Option<String>,
}

pub(crate) type CharacterTables = BTreeMap<String, Spanned<toml::Table>>;

/// The file being read.
pub(crate) struct Reader<'a> {
    pub text: &'a str,
    pub file: &'a Path,
    pub mod_name: &'a str,
}

impl Reader<'_> {
    fn source(&self, line: usize) -> Source {
        Source { mod_name: self.mod_name.to_string(), file: self.file.to_path_buf(), line }
    }

    fn error(&self, line: usize, message: String) -> Problem {
        Problem::Error { file: self.file.to_path_buf(), line: Some(line), message }
    }

    /// The `[[action]]` entries. A wrong entry gives an error (with its line) and is skipped.
    /// A `spawn` with simple keys adds a derived definition to `characters`.
    pub fn actions(&self, list: Vec<Spanned<toml::Table>>, out: &mut Vec<ModAction>, characters: &mut Vec<CharacterDef>, problems: &mut Vec<Problem>) {
        for entry in list {
            let line = line_of(self.text, entry.span().start);
            match action(entry.get_ref()) {
                Ok((key, mut kind, keys)) => {
                    if let Some(w) = reserved_key_warning(&key) {
                        problems.push(Problem::Warning { path: self.file.to_path_buf(), message: format!("line {line}: shortcut: {w}") });
                    }
                    if let (ActionKind::Spawn { what, .. }, false) = (&mut kind, keys.is_empty()) {
                        let derived = derived_name(what, self.file, line);
                        characters.push(derived_def(&derived, what, keys, self.source(line)));
                        *what = derived;
                    }
                    out.push(ModAction { key, kind, source: self.source(line) });
                }
                Err(why) => problems.push(self.error(line, format!("action: {why}; entry skipped"))),
            }
        }
    }

    /// The `[character.<Name>]` tables. A wrong table gives an error (with its line) and is
    /// skipped.
    pub fn characters(&self, tables: CharacterTables, out: &mut Vec<CharacterDef>, problems: &mut Vec<Problem>) {
        for (name, table) in tables {
            let line = line_of(self.text, table.span().start);
            match character(&name, table.get_ref()) {
                Ok(mut c) => {
                    c.source = self.source(line);
                    out.push(c);
                }
                Err(why) => problems.push(self.error(line, format!("character {name}: {why}; character skipped"))),
            }
        }
    }
}

fn known_keys(t: &toml::Table, keys: &[&str]) -> Result<(), String> {
    match t.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown key {k} (keys: {})", keys.join(", "))),
        None => Ok(()),
    }
}

fn string(t: &toml::Table, key: &str) -> Result<Option<String>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(v) => Err(format!("{key} = {v} must be a non-empty string")),
    }
}

fn boolean(t: &toml::Table, key: &str, default: bool) -> Result<bool, String> {
    match t.get(key) {
        None => Ok(default),
        Some(toml::Value::Boolean(b)) => Ok(*b),
        Some(v) => Err(format!("{key} = {v} must be true or false")),
    }
}

/// Name of a derived definition: the character it starts from, then where it was written
/// ("Jones@mod.toml:12"). Never the name of a table (a table name has no "@").
pub fn derived_name(like: &str, file: &Path, line: usize) -> String {
    format!("{like}@{}:{line}", file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())
}

/// A derived definition: `keys` over the character `like` (merged by simple::resolve).
pub fn derived_def(name: &str, like: &str, keys: crate::simple::SimpleKeys, source: Source) -> CharacterDef {
    CharacterDef {
        name: name.to_string(),
        base: String::new(),
        team: None,
        allowed_states: Vec::new(),
        can_die: true,
        death_animation: None,
        properties: Vec::new(),
        states: Vec::new(),
        source,
        keys,
        like: Some(like.to_string()),
    }
}

fn action(t: &toml::Table) -> Result<(String, ActionKind, crate::simple::SimpleKeys), String> {
    let mut allowed = vec!["key", "spawn", "in_front_of_player", "give", "command", "sound"];
    allowed.extend_from_slice(crate::simple::KEYS);
    known_keys(t, &allowed)?;
    let keys = crate::simple::read(t)?;
    if !keys.is_empty() && !t.contains_key("spawn") {
        return Err(format!("{} only go with spawn", crate::simple::KEYS.join(", ")));
    }
    let key = string(t, "key")?.ok_or("key = \"K\" is missing")?;
    if !valid_key(&key) {
        return Err(format!("unknown key name {key:?} (F1…F12, A…Z, 0…9, NumPad0…NumPad9, Insert, Delete, Home, End, PageUp, PageDown)"));
    }
    let kinds = ["spawn", "give", "command", "sound"].iter().filter(|k| t.contains_key(**k)).count();
    if kinds != 1 {
        return Err("write exactly one of spawn, give, command or sound".into());
    }
    let kind = if let Some(what) = string(t, "spawn")? {
        let distance = match t.get("in_front_of_player") {
            None => DEFAULT_DISTANCE,
            Some(toml::Value::Integer(i)) if *i > 0 => *i as f32,
            Some(toml::Value::Float(f)) if *f > 0.0 => *f as f32,
            Some(v) => return Err(format!("in_front_of_player = {v} must be a positive distance")),
        };
        ActionKind::Spawn { what, distance }
    } else if t.contains_key("in_front_of_player") {
        return Err("in_front_of_player only goes with spawn".into());
    } else if let Some(class) = string(t, "give")? {
        if !valid_class_name(&class) {
            return Err(format!("give = {class:?} must be \"Package.Class\""));
        }
        ActionKind::Give { class }
    } else if let Some(sound) = string(t, "sound")? {
        ActionKind::Sound { sound }
    } else {
        ActionKind::Command { text: string(t, "command")?.ok_or("command must be a non-empty string")? }
    };
    Ok((key, kind, keys))
}

fn character(name: &str, t: &toml::Table) -> Result<CharacterDef, String> {
    if name.is_empty() || name.contains('.') || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("the name may only use letters, digits, _ and - (no dot: it is not a class)".into());
    }
    let mut allowed = vec!["base", "team", "allowed_states", "can_die", "death_animation", "properties", "state"];
    allowed.extend_from_slice(crate::simple::KEYS);
    known_keys(t, &allowed)?;
    let keys = crate::simple::read(t)?;
    // Without a base class: the one of its behaviour, or a calm story character when it has a
    // look of its own.
    let base = match (string(t, "base")?, keys.behaviour, &keys.look) {
        (Some(b), _, _) => b,
        (None, Some(b), _) => b.class().to_string(),
        (None, None, Some(_)) => crate::simple::STANDS_CLASS.to_string(),
        (None, None, None) => return Err("base = \"Package.Class\" is missing (or give a look or a behaviour)".into()),
    };
    if !valid_class_name(&base) {
        return Err(format!("base = {base:?} must be \"Package.Class\""));
    }
    let team = keys.team.or_else(|| keys.behaviour.and_then(crate::simple::Behaviour::team));
    let allowed_states = match t.get("allowed_states") {
        None => Vec::new(),
        Some(toml::Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                toml::Value::String(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
                v => Err(format!("allowed_states: {v} must be a state name")),
            })
            .collect::<Result<_, _>>()?,
        Some(v) => return Err(format!("allowed_states = {v} must be a list of state names")),
    };
    // The simple keys' values first: raw properties written after them win.
    let mut properties = keys.with_defaults().values();
    match t.get("properties") {
        None => {}
        Some(toml::Value::Table(p)) => {
            for (key, value) in p {
                let (property, index) = parse_key(key).ok_or_else(|| format!("properties: \"{key}\" is not a property name"))?;
                let value = convert(value).map_err(|e| format!("properties.{key}: {e}"))?;
                properties.push(ActorValue { property, index, value });
            }
        }
        Some(_) => return Err("properties must be a table, for example { Health = 200 }".into()),
    }
    let mut states = Vec::new();
    match t.get("state") {
        None => {}
        Some(toml::Value::Table(s)) => {
            for (state, look) in s {
                let toml::Value::Table(look) = look else { return Err(format!("state {state} must be a table")) };
                known_keys(look, &["animation", "sound", "loop"]).map_err(|e| format!("state {state}: {e}"))?;
                let animation = string(look, "animation").map_err(|e| format!("state {state}: {e}"))?;
                let sound = string(look, "sound").map_err(|e| format!("state {state}: {e}"))?;
                if animation.is_none() && sound.is_none() {
                    return Err(format!("state {state}: give an animation, a sound, or both"));
                }
                let looping = boolean(look, "loop", true).map_err(|e| format!("state {state}: {e}"))?;
                states.push(StateLook { state: state.clone(), animation, sound, looping });
            }
        }
        Some(_) => return Err("state must hold tables such as [character.Name.state.StateName]".into()),
    }
    Ok(CharacterDef {
        name: name.to_string(),
        base,
        team,
        allowed_states,
        death_animation: string(t, "death_animation")?,
        properties,
        states,
        source: Source { mod_name: String::new(), file: Default::default(), line: 0 },
        can_die: keys.can_die.unwrap_or(boolean(t, "can_die", true)?),
        keys,
        like: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::parse_mod_file;
    use crate::ModValue;

    fn parse(text: &str) -> crate::defaults::ModFile {
        parse_mod_file(text, Path::new("m/mod.toml"), "m")
    }

    #[test]
    fn keys() {
        for k in ["F1", "f12", "A", "z", "0", "9", "NumPad0", "numpad9", "Insert", "PageDown"] {
            assert!(valid_key(k), "{k}");
        }
        for k in ["F0", "F13", "F01", "AB", "NumPad10", "Space", "", "10"] {
            assert!(!valid_key(k), "{k}");
        }
    }

    #[test]
    fn actions_and_characters() {
        let text = r#"
[[action]]
key = "K"
spawn = "Guard"
in_front_of_player = 200

[[action]]
key = "F10"
give = "XIII.Bazook"

[[action]]
key = "K"
command = "modpos"

[[action]]
key = "J"
sound = "XIIIsound.Guns__SnipFire.SnipFire__hSnipFire"

[character.Guard]
base = "XIDPawn.BaseSoldier"
team = "Neutral"
allowed_states = ["Attente"]
can_die = false
death_animation = "Mort01"
properties = { Health = 200 }

[character.Guard.state.Attente]
animation = "Attente01"

[character.Guard.state.Dying]
sound = "XIIIsound.A.B"
loop = false
"#;
        let f = parse(text);
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let kinds: Vec<(&str, &ActionKind, usize)> = f.actions.iter().map(|a| (a.key.as_str(), &a.kind, a.source.line)).collect();
        assert_eq!(kinds[0], ("K", &ActionKind::Spawn { what: "Guard".into(), distance: 200.0 }, 2));
        assert_eq!(kinds[1], ("F10", &ActionKind::Give { class: "XIII.Bazook".into() }, 7));
        assert_eq!(kinds[2], ("K", &ActionKind::Command { text: "modpos".into() }, 11));
        assert_eq!(kinds[3], ("J", &ActionKind::Sound { sound: "XIIIsound.Guns__SnipFire.SnipFire__hSnipFire".into() }, 15));
        assert_eq!(f.characters.len(), 1);
        let c = &f.characters[0];
        assert_eq!(c.name, "Guard");
        assert_eq!(c.base, "XIDPawn.BaseSoldier");
        assert_eq!(c.team, Some(Team::Neutral));
        assert_eq!(c.allowed_states, ["Attente"]);
        assert!(!c.can_die);
        assert_eq!(c.death_animation.as_deref(), Some("Mort01"));
        assert_eq!(c.properties[0].value, ModValue::Int(200));
        assert_eq!(c.source.line, 19);
        assert_eq!(c.states.len(), 2);
        let wait_value = c.states.iter().find(|s| s.state == "Attente").unwrap();
        assert_eq!((wait_value.animation.as_deref(), wait_value.sound.as_deref(), wait_value.looping), (Some("Attente01"), None, true));
        let dying = c.states.iter().find(|s| s.state == "Dying").unwrap();
        assert_eq!((dying.animation.as_deref(), dying.sound.as_deref(), dying.looping), (None, Some("XIIIsound.A.B"), false));
    }

    #[test]
    fn defaults_and_errors() {
        let text = r#"
[[action]]
key = "K"
spawn = "XIDPawn.BaseSoldier"

[[action]]
key = "Space"
spawn = "Guard"

[[action]]
key = "F9"
spawn = "Guard"
give = "XIII.Bazook"

[[action]]
key = "F7"
give = "Bazook"

[character.Bad]
team = "neutral"

[character.Wrong]
base = "XIDPawn.BaseSoldier"
team = "ally"

[character.Quiet]
base = "XIDPawn.BaseSoldier"
[character.Quiet.state.Attente]
loop = true
"#;
        let f = parse(text);
        assert_eq!(f.actions.len(), 1);
        assert_eq!(f.actions[0].kind, ActionKind::Spawn { what: "XIDPawn.BaseSoldier".into(), distance: DEFAULT_DISTANCE });
        assert!(f.characters.is_empty(), "{:?}", f.characters);
        let all: Vec<String> = f.problems.iter().map(|p| p.to_string()).collect();
        let has = |line: usize, what: &str| all.iter().any(|x| x.contains(&format!("(line {line})")) && x.contains(what));
        assert!(has(6, "unknown key name \"Space\""), "{all:?}");
        assert!(has(10, "exactly one of"), "{all:?}");
        assert!(has(15, "must be \"Package.Class\""), "{all:?}");
        assert!(has(19, "base = \"Package.Class\" is missing"), "{all:?}");
        assert!(has(22, "friend, neutral or enemy"), "{all:?}");
        assert!(all.iter().any(|x| x.contains("character Quiet") && x.contains("give an animation, a sound, or both")), "{all:?}");
    }
}
