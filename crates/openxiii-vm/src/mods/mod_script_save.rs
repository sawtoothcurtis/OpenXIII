//! Mods build only (feature `mods`): the scripts' memory, the `save` table of each mod.
//!
//! - It holds plain values only: numbers, strings, booleans and tables of them (a table is a
//!   list, or has names as keys). It is written as JSON.
//! - It lives with the game's subsystems that survive a map change (`EngineState`): it goes
//!   on from one map of the campaign to the next, and starts empty from the main menu.
//! - When the game saves at a checkpoint (SaveAtCheckpoint) it is copied with the checkpoint;
//!   loading the checkpoint (LaunchFromCheckpoint) brings back that copy, as the game brings
//!   back the player's inventory.
//! - A checkpoint written to a slot of the player's profile keeps the memory in a file of its
//!   own next to the profile (`<profile>.mods-slot<N>.json`): the profile file itself is never
//!   changed, so saves are byte for byte those of the game without mods.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mlua::{Lua, Table};

use crate::mod_script::ScriptRuntime;
use crate::Vm;

/// Deepest nesting of tables in a `save`.
const MAX_DEPTH: usize = 32;

/// The memory of each mod: folder name -> JSON text.
pub type Saves = BTreeMap<String, String>;

fn to_json(v: &mlua::Value, path: &str, depth: usize) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    if depth > MAX_DEPTH {
        return Err(format!("{path}: tables nested more than {MAX_DEPTH} times (a table inside itself?)"));
    }
    Ok(match v {
        mlua::Value::Nil => J::Null,
        mlua::Value::Boolean(b) => J::Bool(*b),
        mlua::Value::Integer(i) => J::from(*i),
        mlua::Value::Number(n) => serde_json::Number::from_f64(*n).map(J::Number).ok_or_else(|| format!("{path}: {n} cannot be kept"))?,
        mlua::Value::String(s) => J::String(s.to_str().map_err(|e| e.to_string())?.to_string()),
        mlua::Value::Table(t) => {
            let n = t.raw_len();
            let count = t.clone().pairs::<mlua::Value, mlua::Value>().count();
            if n > 0 && count == n {
                let mut out = Vec::new();
                for i in 1..=n {
                    out.push(to_json(&t.raw_get::<mlua::Value>(i).map_err(|e| e.to_string())?, &format!("{path}[{i}]"), depth + 1)?);
                }
                J::Array(out)
            } else {
                let mut out = serde_json::Map::new();
                for pair in t.clone().pairs::<mlua::Value, mlua::Value>() {
                    let (k, x) = pair.map_err(|e| e.to_string())?;
                    let key = match &k {
                        mlua::Value::String(s) => s.to_str().map_err(|e| e.to_string())?.to_string(),
                        _ => return Err(format!("{path}: keys must be names (a list is written {{a, b, c}})")),
                    };
                    let x = to_json(&x, &format!("{path}.{key}"), depth + 1)?;
                    out.insert(key, x);
                }
                J::Object(out)
            }
        }
        other => return Err(format!("{path}: a {} cannot be kept (numbers, strings, true/false and tables only)", other.type_name())),
    })
}

fn from_value(lua: &Lua, v: &serde_json::Value) -> mlua::Result<mlua::Value> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => mlua::Value::Nil,
        J::Bool(b) => mlua::Value::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => mlua::Value::Integer(i),
            None => mlua::Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => mlua::Value::String(lua.create_string(s)?),
        J::Array(items) => {
            let t = lua.create_table()?;
            for x in items {
                t.raw_push(from_value(lua, x)?)?;
            }
            mlua::Value::Table(t)
        }
        J::Object(m) => {
            let t = lua.create_table()?;
            for (k, x) in m {
                t.raw_set(k.as_str(), from_value(lua, x)?)?;
            }
            mlua::Value::Table(t)
        }
    })
}

/// A `save` table as JSON.
pub fn save_to_json(t: &Table) -> Result<String, String> {
    let j = to_json(&mlua::Value::Table(t.clone()), "save", 0)?;
    serde_json::to_string(&j).map_err(|e| e.to_string())
}

/// A `save` table read back from JSON (an empty table for "{}" or "[]").
pub(crate) fn from_json(lua: &Lua, text: &str) -> Result<Table, String> {
    let j: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match from_value(lua, &j).map_err(|e| e.to_string())? {
        mlua::Value::Table(t) => Ok(t),
        _ => Err("not a table".into()),
    }
}

/// The side file of slot `slot` of the profile at `profile`.
fn side_file(profile: &std::path::Path, slot: i32) -> PathBuf {
    let name = profile.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    profile.with_file_name(format!("{name}.mods-slot{slot}.json"))
}

impl Vm {
    /// After the scripts ran: each mod's `save` table, kept by the game. A value that cannot be
    /// kept gives one warning, and that mod's memory stays as it was.
    pub(crate) fn mod_script_keep_saves(&mut self, rt: &ScriptRuntime) {
        for (name, env) in rt.envs() {
            let save = match env.raw_get::<mlua::Value>("save") {
                Ok(mlua::Value::Table(t)) => save_to_json(&t),
                Ok(mlua::Value::Nil) => Ok("{}".to_string()),
                Ok(other) => Err(format!("save is a {}, not a table", other.type_name())),
                Err(e) => Err(e.to_string()),
            };
            match save {
                Ok(text) if text == "{}" || text == "[]" => {
                    self.engine.mod_script_save.remove(name);
                }
                Ok(text) => {
                    self.engine.mod_script_save.insert(name.clone(), text);
                }
                Err(why) => self.mod_report(format!("warning: script {name}: {why}; the save is kept as it was")),
            }
        }
    }

    /// SaveAtCheckpoint stored a checkpoint (`start_too`: also the level's starting point):
    /// the scripts' memory goes with it.
    pub(crate) fn mod_script_save_checkpoint(&mut self, start_too: bool) {
        if let Some(rt) = self.mods.scripts.take() {
            self.mod_script_keep_saves(&rt);
            self.mods.scripts = Some(rt);
        }
        let live = self.engine.mod_script_save.clone();
        if let Some(d) = self.engine.save.as_mut() {
            if start_too {
                d.mod_save_start = live.clone();
            }
            d.mod_save_checkpoint = live;
        }
    }

    /// LaunchFromCheckpoint: the memory saved with that checkpoint, for the next map's scripts.
    pub(crate) fn mod_script_save_load(&mut self, from_start: bool) {
        let kept = self.engine.save.as_ref().map(|d| if from_start { d.mod_save_start.clone() } else { d.mod_save_checkpoint.clone() });
        self.engine.mod_script_save_restore = kept;
    }

    /// OperationWriteSlot wrote checkpoint `data_from_checkpoint` (else the starting point) to
    /// slot `slot`: its memory goes to the slot's side file (removed when it is empty).
    pub(crate) fn mod_script_save_write_slot(&mut self, profile: &str, slot: i32, data_from_checkpoint: bool) {
        let Some(d) = self.engine.save.as_ref() else { return };
        let saves = if data_from_checkpoint { &d.mod_save_checkpoint } else { &d.mod_save_start };
        let file = side_file(std::path::Path::new(profile), slot);
        if saves.is_empty() {
            self.session.files.remove(&file);
            return;
        }
        if let Ok(text) = serde_json::to_string_pretty(saves) {
            if !self.session.files.write(&file, text.as_bytes()) {
                self.mod_report(format!("warning: scripts: {}: not written", file.display()));
            }
        }
    }

    /// OperationReadSlot read slot `slot`: its memory, from the side file (none: empty).
    pub(crate) fn mod_script_save_read_slot(&mut self, profile: &str, slot: i32) {
        let Some(d) = self.engine.save.as_mut() else { return };
        let file = side_file(std::path::Path::new(profile), slot);
        let saves: Saves = self.session.files.read(&file).and_then(|b| String::from_utf8(b).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        d.mod_save_checkpoint = saves.clone();
        d.mod_save_start = saves;
    }

    /// OperationEraseSlot: the slot's side file goes too.
    pub(crate) fn mod_script_save_erase_slot(&mut self, profile: &str, slot: i32) {
        self.session.files.remove(&side_file(std::path::Path::new(profile), slot));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_and_refusals() {
        let lua = Lua::new();
        let t: Table =
            lua.load("return { wave = 3, name = 'beach', done = false, ratio = 0.5, list = {1, 2, 3}, nested = { a = { b = 'c' } } }").eval().unwrap();
        let text = save_to_json(&t).unwrap();
        let back = from_json(&lua, &text).unwrap();
        assert_eq!(save_to_json(&back).unwrap(), text);
        assert_eq!(back.get::<i64>("wave").unwrap(), 3);
        assert_eq!(back.get::<Table>("list").unwrap().raw_len(), 3);
        for bad in ["return { f = print }", "return { [1] = 1, [3] = 3 }", "local t = {} t.self = t return t"] {
            let t: Table = lua.load(bad).eval().unwrap();
            assert!(save_to_json(&t).is_err(), "{bad}");
        }
        assert_eq!(side_file(std::path::Path::new("SAVE/Player.prf"), 2), std::path::Path::new("SAVE/Player.prf.mods-slot2.json"));
    }
}
