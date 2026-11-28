//! Game keys: bindings and aliases from DefUser.ini / User.ini ([Engine.Input]), interpreted as
//! the engine does ("Button bFire | Fire | onrelease UnFire"…).

use crate::value::{ObjId, Value};
use crate::Vm;
use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Bindings {
    /// Unreal key name (lowercase) -> command.
    keys: HashMap<String, String>,
    /// Alias (lowercase) -> command.
    aliases: HashMap<String, String>,
    /// UInput's alias slots (Aliases[n], at most 40; "None" for an empty slot): (name, command).
    alias_order: Vec<(String, String)>,
}

/// Number of UInput alias slots (OptimizeAliases: 40 of 16 bytes from +0x2C).
pub const ALIASES_MAX: usize = 40;

impl Bindings {
    /// Tests: alias slots (slot, name, command) and bindings (key name, command) given as is.
    #[cfg(test)]
    pub(crate) fn from_slots(aliases: &[(usize, String, String)], keys: &[(String, String)]) -> Self {
        let mut b = Bindings::default();
        for (n, alias, command) in aliases {
            if b.alias_order.len() <= *n {
                b.alias_order.resize(*n + 1, ("None".to_string(), String::new()));
            }
            b.alias_order[*n] = (alias.clone(), command.clone());
            b.aliases.insert(alias.to_lowercase(), command.clone());
        }
        for (k, c) in keys {
            b.keys.insert(k.to_lowercase(), c.clone());
        }
        b
    }

    /// Reads the [Engine.Input] section of an ini file (DefUser.ini, User.ini).
    pub fn parse(ini: &str) -> Self {
        let mut b = Bindings::default();
        let mut in_input = false;
        for line in ini.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_input = line.eq_ignore_ascii_case("[Engine.Input]");
                continue;
            }
            if !in_input || line.starts_with(';') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.starts_with("Aliases[") {
                // Aliases[n]=(Command="…",Alias="…"): the Alias struct (Alias: name, Command:
                // string, CPF_Config) built by UInput::StaticInitInput.
                let field = |name: &str| {
                    let start = value.find(&format!("{name}=\""))? + name.len() + 2;
                    let end = value[start..].find('"')? + start;
                    Some(value[start..end].to_string())
                };
                let case = key.trim().strip_prefix("Aliases[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.trim().parse::<usize>().ok()).filter(|&n| n < ALIASES_MAX);
                if let (Some(command), Some(alias), Some(n)) = (field("Command"), field("Alias"), case) {
                    if b.alias_order.len() <= n {
                        b.alias_order.resize(n + 1, ("None".to_string(), String::new()));
                    }
                    if alias.eq_ignore_ascii_case("None") {
                        b.alias_order[n] = ("None".to_string(), String::new());
                    } else {
                        b.alias_order[n] = (alias.clone(), command.clone());
                        b.aliases.insert(alias.to_lowercase(), command);
                    }
                }
            } else if !value.trim().is_empty() {
                b.keys.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }
        b
    }

    /// Bindings and aliases already read (tests): keys (name, command) and aliases (name, command) in order.
    pub(crate) fn from_parts(keys: &[(String, String)], aliases: &[(String, String)]) -> Self {
        let mut b = Bindings::default();
        for (k, v) in keys {
            b.keys.insert(k.to_lowercase(), v.clone());
        }
        for (a, c) in aliases {
            b.alias_order.push((a.clone(), c.clone()));
            b.aliases.insert(a.to_lowercase(), c.clone());
        }
        b
    }

    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path).map(|d| Self::parse(&String::from_utf8_lossy(&d))).unwrap_or_default()
    }

    /// The alias slots in index order (Aliases[n], "None" for an empty slot).
    pub fn alias_list(&self) -> &[(String, String)] {
        &self.alias_order
    }

    pub fn command(&self, key: &str) -> Option<&str> {
        self.keys.get(&key.to_lowercase()).map(|s| s.as_str())
    }
}

impl Vm {
    /// Key `key` (Unreal name: "LeftMouse", "1", "E"…) pressed or released for player `pc`.
    pub fn input_key(&mut self, pc: ObjId, bindings: &Bindings, key: &str, pressed: bool) {
        // Engine inputs active (input_init): the event goes through UInput at the start of
        // the next frame.
        if self.input_sys.is_some() {
            if let Some(code) = Vm::key_code(key) {
                let action = if pressed { crate::player_input::IST_PRESS } else { crate::player_input::IST_RELEASE };
                self.input_event(code, action, 0.0);
            }
            return;
        }
        if let Some(command) = bindings.command(key) {
            let command = command.to_string();
            self.input_command(pc, bindings, &command, pressed, &mut Vec::new());
        }
    }

    /// `expanding`: alias being expanded; in its own text, an alias refers to the
    /// command of the same name ("Fire" in the Fire alias is the exec function Fire).
    fn input_command(&mut self, pc: ObjId, bindings: &Bindings, command: &str, pressed: bool, expanding: &mut Vec<String>) {
        if expanding.len() > 8 {
            return;
        }
        for part in command.split('|') {
            let part = part.trim();
            let mut words = part.split_whitespace();
            let Some(first) = words.next() else { continue };
            match first.to_lowercase().as_str() {
                "button" => {
                    if let Some(var) = words.next() {
                        self.set_prop(pc, var, Value::Byte(pressed as u8));
                    }
                }
                "toggle" => {
                    if let (Some(var), true) = (words.next(), pressed) {
                        let on = self.propr(pc, var).as_int() != 0;
                        self.set_prop(pc, var, Value::Byte((!on) as u8));
                    }
                }
                // Analog axes: movement is handled by the interface for now.
                "axis" => {}
                "onrelease" => {
                    if !pressed {
                        let rest = part[first.len()..].trim().to_string();
                        self.input_command(pc, bindings, &rest, true, expanding);
                    }
                }
                name => {
                    let alias = bindings.aliases.get(name).filter(|_| !expanding.iter().any(|e| e == name)).cloned();
                    if let Some(alias) = alias {
                        expanding.push(name.to_string());
                        self.input_command(pc, bindings, &alias, pressed, expanding);
                        expanding.pop();
                    } else if pressed {
                        self.console_command(pc, part);
                    }
                }
            }
        }
    }
}
