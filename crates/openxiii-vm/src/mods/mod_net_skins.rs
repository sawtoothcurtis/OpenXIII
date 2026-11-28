//! Mods build only (feature `mods`): the skins mods give to actors, shown on network guests.
//!
//! The game replicates an actor's `Mesh`, `DrawScale`, `DrawScale3D`, `AmbientGlow`, `bUnlit`
//! and `PrePivot` (Actor.uc's replication block), never its `Skins`. A mod's `skin` (or raw
//! `Skins` values) written by the host on a character or an object it changes or adds would
//! therefore show on the host only. Every player has the same mods (mod_net.rs refuses a guest
//! with other mods), so only the host's choice of actor has to cross the network:
//!
//! - Host: each actor a mod gave skins to is noted with them ([`Vm::mod_net_note_skins`]).
//!   When the host opens that actor's channel for a guest, it sends that guest one reliable
//!   `ClientMessage` (an RPC every PlayerController has) whose text is
//!   `OXIIIMod.Skins.<channel>.<class>|<skin>;<skin>…`, after the replication of the tick.
//! - Guest: the message is taken before the game's own ClientMessage code runs; the skins are
//!   written on the actor of that channel (same class), or kept until the channel's actor
//!   arrives.
//!
//! Nothing is noted, sent or read when no mod gives a skin, and nothing of this is compiled
//! without the `mods` feature: the game's network protocol is unchanged then. One process with
//! several views (split screen) draws the same actors in every view and needs none of this.

use crate::value::{ObjId, Value};
use crate::Vm;

/// Start of the text of a skins message (a ClientMessage's text, which crosses the network as
/// it is; its type is a name, which must be known on both sides, so the data travels in the text).
pub const SKINS_MESSAGE: &str = "OXIIIMod.Skins.";
const TEXT_SEP: char = '|';
const LIST_SEP: char = ';';

/// Skins of an actor, as noted by the host and sent to the guests.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetSkins {
    pub class: String,
    pub skins: Vec<String>,
}

impl Vm {
    /// Readable name of a skin value ("XIIIPersos.Skins.SpadsNeigeTex", "Mod.MyTexture"), or
    /// "None".
    fn mod_skin_name(&self, v: &Value) -> String {
        match v {
            Value::Asset(s) if !s.is_empty() => s.clone(),
            Value::Object(Some(o)) => self.path_name(*o),
            _ => "None".into(),
        }
    }

    /// Host: actor `a` got its `Skins` from a mod; its guests are sent them when its channel
    /// opens. A network client never notes (it does not make the mods' changes).
    pub(crate) fn mod_net_note_skins(&mut self, a: ObjId) {
        if self.mod_net_client() {
            return;
        }
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| self.class_path(o.class)) else { return };
        let skins = match self.prop(a, "Skins") {
            Value::Array(items) => items.iter().map(|v| self.mod_skin_name(v)).collect(),
            v => vec![self.mod_skin_name(&v)],
        };
        self.mods.net_skins.insert(a, NetSkins { class, skins });
    }

    /// Text of the skins message of actor `a` on channel `ch`, if a mod gave it skins.
    pub(crate) fn mod_net_skins_message(&self, a: ObjId, ch: usize) -> Option<String> {
        let s = self.mods.net_skins.get(&a)?;
        let list: Vec<&str> = s.skins.iter().map(String::as_str).collect();
        Some(format!("{SKINS_MESSAGE}{ch}.{}{TEXT_SEP}{}", s.class, list.join(&LIST_SEP.to_string())))
    }

    /// Network channel `ch` of connection `which` now carries actor `actor` (UActorChannel
    /// SetChannelActor, both sides): the host queues the actor's skins message for that guest;
    /// a guest writes skins that arrived before the actor.
    pub(crate) fn mod_net_channel_actor(&mut self, which: crate::net_actor::Which, ch: usize, actor: ObjId) {
        match which {
            Some(i) => {
                if self.mods.net_skins.contains_key(&actor) {
                    self.mods.net_skin_outbox.push((i, ch, actor));
                }
            }
            None => {
                if let Some(s) = self.mods.net_skin_pending.remove(&ch) {
                    self.mod_net_apply_skins(actor, &s);
                }
            }
        }
    }

    /// Host, after the replication of a tick: the skins messages queued for the guests, each by
    /// its connection's PlayerController (ClientMessage, reliable).
    pub(crate) fn mod_net_send_skins(&mut self) {
        for (i, ch, actor) in std::mem::take(&mut self.mods.net_skin_outbox) {
            let pc = self.net_conn_ref(Some(i)).and_then(|c| c.pc).filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()));
            let (Some(pc), Some(text)) = (pc, self.mod_net_skins_message(actor, ch)) else { continue };
            let _ = self.call_named(pc, "ClientMessage", vec![Value::Str(text), Value::Name("None".into())]);
        }
    }

    /// Guest: a ClientMessage carrying a skins message is applied here instead of being shown.
    /// True when the call was taken.
    pub(crate) fn mod_net_client_message(&mut self, f: crate::types::FuncId, args: &[Option<Value>]) -> bool {
        if !self.functions[f].name.eq_ignore_ascii_case("ClientMessage") {
            return false;
        }
        let Some(Some(Value::Str(text))) = args.first() else { return false };
        let Some((ch, skins)) = parse_skins_message(text) else { return false };
        let actor = self.net_conn_ref(None).and_then(|c| c.actor_channels.get(&ch)).and_then(|a| a.actor);
        match actor {
            Some(a) => self.mod_net_apply_skins(a, &skins),
            None => {
                self.mods.net_skin_pending.insert(ch, skins);
            }
        }
        true
    }

    /// Writes `s.skins` on actor `a` when it is of class `s.class` (a channel may have been
    /// given to another actor since).
    pub(crate) fn mod_net_apply_skins(&mut self, a: ObjId, s: &NetSkins) {
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| self.class_path(o.class)) else { return };
        if !class.eq_ignore_ascii_case(&s.class) {
            return;
        }
        let mut values = Vec::new();
        for name in &s.skins {
            let v = if name.eq_ignore_ascii_case("None") {
                Value::Object(None)
            } else {
                match self.mod_asset_value("Material", name) {
                    Some(Ok(v)) => v,
                    _ => {
                        self.mod_report(format!("warning: network: skin {name} of {class} not found on this machine"));
                        Value::Object(None)
                    }
                }
            };
            values.push(v);
        }
        if self.prop_slot(a, "Skins").is_ok() {
            self.set_prop(a, "Skins", Value::Array(values));
            let name = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.name.clone()).unwrap_or_default();
            self.mod_report(format!("network: skins of {name} ({class}) from the host: {}", s.skins.join(", ")));
        }
    }
}

/// (channel, skins) of a skins message, None for any other text.
pub fn parse_skins_message(text: &str) -> Option<(usize, NetSkins)> {
    let rest = text.strip_prefix(SKINS_MESSAGE)?;
    let (head, list) = rest.split_once(TEXT_SEP)?;
    let (ch, class) = head.split_once('.')?;
    let ch = ch.parse().ok()?;
    let skins = if list.is_empty() { Vec::new() } else { list.split(LIST_SEP).map(str::to_string).collect() };
    Some((ch, NetSkins { class: class.to_string(), skins }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let s = NetSkins { class: "XIDCine.Cine2".into(), skins: vec!["XIIIPersos.Skins.SpadsNeigeTex".into(), "None".into()] };
        let text = format!("{SKINS_MESSAGE}7.XIDCine.Cine2|XIIIPersos.Skins.SpadsNeigeTex;None");
        assert_eq!(parse_skins_message(&text), Some((7, s)));
        // Negative controls: the game's own messages and the co-op story messages are not taken.
        assert_eq!(parse_skins_message("Welcome to the server"), None);
        assert_eq!(parse_skins_message("OXIIICoop.XIIIMessage.3|text"), None);
        assert_eq!(parse_skins_message("OXIIIMod.Skins.x.Cine2|a"), None);
    }
}
