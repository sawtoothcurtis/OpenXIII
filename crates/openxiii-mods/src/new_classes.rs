//! New classes made by mods: `[classes.<Name>]` tables.
//!
//! ```toml
//! [classes.Blaster]            # a new class "Mod.Blaster"
//! parent = "XIII.Beretta"      # it behaves exactly as this game class
//!
//! [class."Mod.Blaster"]        # then its own values, as for any class
//! ItemName = "Blaster"
//! ```
//!
//! A new class has the parent's scripts (functions and states) and starts from the parent's
//! default values; its own values come from `[class."Mod.<Name>"]` tables. Wherever the game
//! asks "is this a Beretta?", the answer is yes, as for any subclass.

use std::collections::BTreeMap;
use std::path::Path;

use toml::Spanned;

use crate::placement::Source;
use crate::{line_of, Problem};

/// Package name of the mods' new classes: "Mod.<Name>".
pub const PACKAGE: &str = "Mod";

/// A `[classes.<Name>]` definition.
#[derive(Debug, Clone, PartialEq)]
pub struct NewClass {
    pub name: String,
    /// "Package.Class" of the game class it is made from.
    pub parent: String,
    pub source: Source,
}

impl NewClass {
    /// Full class name: "Mod.<Name>".
    pub fn class_name(&self) -> String {
        format!("{PACKAGE}.{}", self.name)
    }
}

pub(crate) type ClassDefTables = BTreeMap<String, Spanned<toml::Table>>;

/// Reads the `[classes.<Name>]` tables of a file.
pub(crate) fn read(text: &str, file: &Path, mod_name: &str, tables: ClassDefTables, out: &mut Vec<NewClass>, problems: &mut Vec<Problem>) {
    for (name, t) in tables {
        let line = line_of(text, t.span().start);
        let err = |m: String| Problem::Error { file: file.to_path_buf(), line: Some(line), message: format!("classes.{name}: {m}; class skipped") };
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.starts_with(|c: char| c.is_ascii_digit()) || name.is_empty() {
            problems.push(err("the name may only use letters, digits and _, and not start with a digit".into()));
            continue;
        }
        if name.eq_ignore_ascii_case(crate::placement::DECORATION_NAME) {
            problems.push(err(format!("{PACKAGE}.{name} is the class of the decorations added with model = \"…\"; choose another name")));
            continue;
        }
        let t = t.get_ref();
        if let Some(k) = t.keys().find(|k| k.as_str() != "parent") {
            problems.push(err(format!("unknown key {k} (keys: parent; the class's values go in a [class.\"{PACKAGE}.{name}\"] table)")));
            continue;
        }
        let parent = match t.get("parent") {
            Some(toml::Value::String(p)) if crate::defaults::valid_class_name(p.trim()) => p.trim().to_string(),
            Some(v) => {
                problems.push(err(format!("parent = {v} must be a game class \"Package.Class\" (for example \"XIII.Beretta\")")));
                continue;
            }
            None => {
                problems.push(err("parent = \"Package.Class\" is missing".into()));
                continue;
            }
        };
        if parent.split('.').next().is_some_and(|p| p.eq_ignore_ascii_case(PACKAGE)) {
            problems.push(err(format!("parent {parent} must be a game class, not another new class")));
            continue;
        }
        out.push(NewClass { name, parent, source: Source { mod_name: mod_name.to_string(), file: file.to_path_buf(), line } });
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    fn parse(text: &str) -> crate::defaults::ModFile {
        crate::defaults::parse_mod_file(text, Path::new("m/mod.toml"), "m")
    }

    #[test]
    fn reads_new_classes() {
        let f = parse("[classes.Blaster]\nparent = \"XIII.Beretta\"\n\n[class.\"Mod.Blaster\"]\nItemName = \"Blaster\"\n");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        assert_eq!(f.new_classes.len(), 1);
        let c = &f.new_classes[0];
        assert_eq!((c.name.as_str(), c.parent.as_str(), c.class_name()), ("Blaster", "XIII.Beretta", "Mod.Blaster".to_string()));
        assert_eq!(c.source.line, 1);
        // Its values are ordinary class values of "Mod.Blaster".
        assert_eq!(f.overrides.len(), 1);
        assert_eq!(f.overrides[0].class, "Mod.Blaster");
    }

    #[test]
    fn wrong_tables_are_skipped_with_their_line() {
        let f = parse(
            "[classes.NoParent]\n\n[classes.Bad]\nparent = \"Beretta\"\n\n[classes.\"Two Words\"]\nparent = \"XIII.Beretta\"\n\n[classes.Extra]\nparent = \"XIII.Beretta\"\nItemName = \"x\"\n\n[classes.Chain]\nparent = \"Mod.Blaster\"\n\n[classes.Good]\nparent = \"XIDPawn.BaseSoldier\"\n",
        );
        assert_eq!(f.new_classes.len(), 1, "{:?}", f.new_classes);
        assert_eq!(f.new_classes[0].name, "Good");
        let mut lines: Vec<Option<usize>> = f.problems.iter().map(|p| match p {
            crate::Problem::Error { line, .. } => *line,
            _ => None,
        }).collect();
        lines.sort();
        assert_eq!(lines, [Some(1), Some(3), Some(6), Some(9), Some(13)], "{:?}", f.problems);
    }
}
