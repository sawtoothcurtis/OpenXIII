//! Names needed by enabled mods before the first rendered frame of a map.

use crate::{value::Value, Vm};
use openxiii_mods::{behaviour::ActionKind, placement::ActorValue, Edit, ModValue};
use std::collections::BTreeSet;

fn asset_values(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Asset(path) if !path.is_empty() => {
            out.insert(path.clone());
        }
        Value::Array(values) => values.iter().for_each(|v| asset_values(v, out)),
        Value::Struct(_, fields) => fields.iter().for_each(|v| asset_values(v, out)),
        _ => {}
    }
}

fn declared_value(value: &ModValue, names: &mut BTreeSet<String>) {
    match value {
        ModValue::Str(s) => {
            names.insert(s.clone());
        }
        ModValue::Array(items) => items.iter().for_each(|v| declared_value(v, names)),
        ModValue::Table(fields) => fields.iter().for_each(|(_, v)| declared_value(v, names)),
        _ => {}
    }
}

fn action_value(value: &openxiii_mods::when::Value, names: &mut BTreeSet<String>) {
    use openxiii_mods::when::Value as Argument;
    match value {
        Argument::Text(s) => {
            names.insert(s.clone());
        }
        Argument::List(items) => items.iter().for_each(|v| action_value(v, names)),
        Argument::Table(fields) => fields.iter().for_each(|(_, v)| action_value(v, names)),
        _ => {}
    }
}

fn property_names(properties: &[ActorValue], names: &mut BTreeSet<String>) {
    for property in properties {
        declared_value(&property.value, names);
    }
}

impl Vm {
    pub(crate) fn add_mod_asset_roots(&self, out: &mut BTreeSet<String>) {
        let mut declared = BTreeSet::new();
        if let Some(defaults) = &self.mods.defaults {
            let map = self.mod_map_name();
            for entry in defaults.classes_all().chain(defaults.maps_of(&map)) {
                declared.insert(entry.class.clone());
                declared_value(&entry.value, &mut declared);
            }
        }
        if let Some(classes) = &self.mods.new_classes {
            declared.extend(classes.iter().map(|c| c.parent.clone()));
        }
        if let Some(behaviour) = &self.mods.behaviour {
            for actor in &behaviour.characters {
                declared.insert(actor.base.clone());
                property_names(&actor.properties, &mut declared);
                declared.extend(actor.states.iter().filter_map(|s| s.sound.clone()));
            }
            for action in &behaviour.actions {
                let name = match &action.kind {
                    ActionKind::Give { class } => class,
                    ActionKind::Spawn { what, .. } => what,
                    ActionKind::Sound { sound } => sound,
                    ActionKind::Command { .. } => continue,
                };
                declared.insert(name.clone());
            }
        }
        for entry in self.mod_edits_here() {
            match entry.edit {
                Edit::Add { class, properties, .. } => {
                    declared.insert(class);
                    property_names(&properties, &mut declared);
                }
                Edit::Replace { to, .. } | Edit::ReplaceSound { to, .. } => {
                    declared.insert(to);
                }
                Edit::StartWeapon { class } | Edit::StartAmmo { class, .. } => {
                    declared.insert(class);
                }
                Edit::Say(say) => declared.extend(say.sound),
                Edit::Change(change) => {
                    if let Some(model) = change.look.and_then(|s| openxiii_mods::looks::resolve(&s).ok()) {
                        declared.insert(model);
                    }
                    declared.extend(change.skin);
                    property_names(&change.properties, &mut declared);
                }
                Edit::When(rule) => {
                    for arg in rule.actions.iter().flat_map(|a| &a.args) {
                        if let openxiii_mods::when::Arg::Value(v) = arg {
                            action_value(v, &mut declared);
                        }
                    }
                }
                Edit::RemoveName { .. } | Edit::RemoveNear { .. } => {}
            }
        }

        for (names, script) in [(&declared, false), (&self.mods.script_events.script_references, true)] {
            for name in names {
                let Some(name) = object_name(name) else { continue };
                if let Some(class) = self.class(name) {
                    if let Some(object) = self.objects.get(self.classes[class].cdo).and_then(Option::as_ref) {
                        for value in object.props.iter() {
                            asset_values(value, out);
                        }
                    }
                }
                let Some((package, _)) = name.split_once('.') else { continue };
                if package.eq_ignore_ascii_case("Mod") {
                    if openxiii_assets::mod_meshes::get(name).is_some()
                        || openxiii_assets::mod_skeletal::get(name).is_some()
                        || openxiii_assets::mod_textures::get(name).is_some()
                        || self.mod_sound_ref(name).is_some()
                    {
                        out.insert(name.to_owned());
                    }
                } else if self.library.as_ref().is_some_and(|lib| lib.file_of(package).is_some()) {
                    // As with script_object_constants, a script literal can only visit an
                    // already-loaded package. Explicit TOML declarations may request a package.
                    out.insert(if script { format!("@{name}") } else { name.to_owned() });
                }
            }
        }
    }
}

fn is_object_name(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')))
}

fn object_name(text: &str) -> Option<&str> {
    // Unreal property values may spell a path as Material'Package.Group.Name'.
    let name = match text.split_once('\'') {
        Some((kind, path)) if is_object_name(kind) => path.strip_suffix('\'')?,
        _ => text,
    };
    is_object_name(name).then_some(name)
}

fn long_string(source: &[u8]) -> Option<(&[u8], usize)> {
    let tail = source.strip_prefix(b"[")?;
    let level = tail.iter().take_while(|&&c| c == b'=').count();
    if tail.get(level) != Some(&b'[') {
        return None;
    }
    let start = level + 2;
    let mut position = start;
    while let Some(at) = source[position..].iter().position(|&c| c == b']') {
        position += at;
        let end = source.get(position + 1..position + level + 2)?;
        if end[..level].iter().all(|&c| c == b'=') && end[level] == b']' {
            let text = &source[start..position];
            // Lua skips the first line break in a long string, including CR and LF/CR.
            let text = match text {
                [b'\r', b'\n', tail @ ..] | [b'\n', b'\r', tail @ ..] => tail,
                [b'\n' | b'\r', tail @ ..] => tail,
                _ => text,
            };
            return Some((text, position + level + 2));
        }
        position += 1;
    }
    None
}

/// Inspect literal tokens of a compiled text chunk, skipping line comments and Lua long comments.
/// A string assembled at runtime can be declared with the mod's explicit preload list.
pub(crate) fn script_names(source: &[u8], out: &mut BTreeSet<String>) {
    let mut i = 0;
    while i < source.len() {
        let rest = &source[i..];
        if rest.starts_with(b"--") {
            let comment = &rest[2..];
            i += 2 + long_string(comment)
                .map_or_else(|| comment.iter().position(|c| matches!(c, b'\n' | b'\r')).unwrap_or(comment.len()), |(_, used)| used);
            continue;
        }
        if let Some((s, used)) = long_string(rest) {
            if let Ok(s) = std::str::from_utf8(s) {
                if let Some(name) = object_name(s) {
                    out.insert(name.to_owned());
                }
            }
            i += used;
            continue;
        }
        let quote = source[i];
        i += 1;
        if !matches!(quote, b'\'' | b'"') {
            continue;
        }
        let start = i;
        while i < source.len() && source[i] != quote {
            if source[i] == b'\\' {
                i += 1;
            }
            i += 1;
        }
        let Some(raw) = source.get(start..i) else { break };
        if let Some(name) = literal_name(raw) {
            out.insert(name);
        }
        i += 1;
    }
}

/// Decode escapes which can produce bytes in an object name. Other escapes produce punctuation or whitespace.
fn literal_name(raw: &[u8]) -> Option<String> {
    if !raw.contains(&b'\\') {
        let text = std::str::from_utf8(raw).ok()?;
        return object_name(text).map(str::to_owned);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c != b'\\' {
            out.push(c);
            continue;
        }
        match *raw.get(i)? {
            b'\'' => {
                // Typed Unreal references can quote the inner path inside a single-quoted Lua literal.
                out.push(b'\'');
                i += 1;
            }
            b'x' => {
                let digits = std::str::from_utf8(raw.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(digits, 16).ok()?);
                i += 3;
            }
            b'u' if raw.get(i + 1) == Some(&b'{') => {
                i += 2;
                let n = raw[i..].iter().position(|&c| c == b'}')?;
                let digits = std::str::from_utf8(&raw[i..i + n]).ok()?;
                out.push(u8::from_str_radix(digits, 16).ok()?);
                i += n + 1;
            }
            b'z' => {
                i += 1;
                i += raw[i..].iter().take_while(|c| c.is_ascii_whitespace()).count();
            }
            b'0'..=b'9' => {
                let n = raw[i..].iter().take(3).take_while(|c| c.is_ascii_digit()).count();
                out.push(std::str::from_utf8(&raw[i..i + n]).ok()?.parse().ok()?);
                i += n;
            }
            _ => return None,
        }
    }
    let text = String::from_utf8(out).ok()?;
    object_name(&text).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_only_asset_names_in_lua_strings() {
        let code = br#"-- "Mod.Ignore"
            spawn("Dog") --[==[ 'Mod.IgnoreToo' ]==]
            effect([=[Mod.Smoke]=], "Mod.\x46ire", "Texture'Mod.Icon'")
            text("a quoted \"Mod.No\" in a sentence")
        "#;
        let mut names = BTreeSet::new();
        script_names(code, &mut names);
        assert_eq!(Vec::from_iter(names), ["Dog", "Mod.Fire", "Mod.Icon", "Mod.Smoke"]);
    }

    #[test]
    fn reads_quoted_unreal_paths_and_lua_line_breaks() {
        let mut names = BTreeSet::new();
        script_names(b"f('Texture\\'Mod.Icon\\'', [=[\rMod.A]=], [=[\n\rMod.B]=], [=[\r\nMod.C]=])", &mut names);
        assert_eq!(Vec::from_iter(names), ["Mod.A", "Mod.B", "Mod.C", "Mod.Icon"]);
    }
}
