//! Mods build only (feature `mods`): the keys of a network guest for the host's scripts.
//!
//! The scripts run on the host only. The host tells each guest which keys its scripts react to
//! (`on("key", ...)`), with the same ClientMessage lines as its goals and sounds; a guest that
//! presses one of them sends it back with a function the game already sends from a guest to the
//! host, `XIIIPlayerController.ServerUpdateSkin(string)`, the text marked so that the host takes
//! it as a key and never as a skin. The host then fires `on("key")` with the guest as the player.

use crate::mod_script_game::{guest_line, GUEST_MESSAGE};
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

/// The guest's function that reaches the host (a string parameter, sent by the game itself).
const TO_HOST: &str = "ServerUpdateSkin";
/// Start of a key sent by a guest.
const KEY_PREFIX: &str = "OXIIIMod.Key|";
/// Kind of the host's line that lists its scripts' keys.
pub(crate) const KEYS_KIND: &str = "Keys";

impl Vm {
    /// Host: tells every remote player the keys of the scripts, when one is new or the keys
    /// changed (once per scripts' frame).
    pub(crate) fn mod_keys_tell_guests(&mut self) {
        if self.mod_net_client() {
            return;
        }
        let mut keys = self.mod_script_keys();
        keys.sort_by_key(|k| k.to_ascii_lowercase());
        keys.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let remote: Vec<ObjId> = self.mod_script_player_controllers().into_iter().filter(|&pc| self.local_player_controller(pc).is_none()).collect();
        let changed = keys != self.mods.keys_told.1;
        let line = guest_line(KEYS_KIND, 0, &keys.join(","));
        for &pc in &remote {
            if changed || !self.mods.keys_told.0.contains(&pc) {
                let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(line.clone()), Value::Name("None".into())]);
            }
        }
        self.mods.keys_told = (remote, keys);
    }

    /// Guest: a key of the host's scripts pressed here goes to the host. True when it was one.
    pub(crate) fn mod_guest_key(&mut self, pc: ObjId, key: &str) -> bool {
        if !self.mod_net_client() || !self.mods.guest_keys.iter().any(|k| k.eq_ignore_ascii_case(key)) {
            return false;
        }
        match self.call_named(pc, TO_HOST, vec![Value::Str(format!("{KEY_PREFIX}{key}"))]) {
            Ok(_) => {
                self.mod_script_log(format!("guest: key {key} sent to the host"));
                true
            }
            Err(e) => {
                self.mod_report(format!("warning: guest: key {key}: {e}"));
                false
            }
        }
    }

    /// Host: a guest's key arrives (the guest's ServerUpdateSkin with a key): the scripts take
    /// it, the skin is left as it is. False for any other call.
    pub(crate) fn mod_guest_key_call(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> bool {
        if !self.functions[f].name.eq_ignore_ascii_case(TO_HOST) || self.mod_net_client() {
            return false;
        }
        let Some(Some(Value::Str(text))) = args.first() else { return false };
        let Some(key) = text.strip_prefix(KEY_PREFIX) else { return false };
        // A key the game window keeps never reaches the scripts, from here or from a guest.
        if openxiii_mods::behaviour::reserved_key(key).is_none() && !self.mod_script_key(this, key) {
            self.mod_report(format!("warning: {}: a guest pressed {key}, which no script reacts to", self.mod_map_name()));
        }
        true
    }

    /// Guest: the host's list of keys (a ClientMessage line).
    pub(crate) fn mod_guest_keys_from_host(&mut self, text: &str) {
        self.mods.guest_keys = text.split(',').map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).collect();
    }
}

/// The line the host sends (tests).
#[cfg(test)]
pub(crate) fn keys_line(keys: &[&str]) -> String {
    format!("{GUEST_MESSAGE}{KEYS_KIND}.0|{}", keys.join(","))
}
