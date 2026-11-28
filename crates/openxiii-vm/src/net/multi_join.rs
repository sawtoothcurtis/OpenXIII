//! Improved build only (the same switch as the Co-op box): an "IP address" box and a Join entry
//! on the multiplayer Join LAN page (XIIIMenuMultiLANJoin), to join a game whose host is not on
//! this network (over the Internet) by typing its address, optionally with ":port".
//!
//! The join is the page's own: the page's click on a listed game (InternalOnClick on its first
//! server button: the "Connecting" box, the player's name, skin and team, the connection status
//! and its errors), with the typed address as the joined server's address (GameServers[31],
//! a slot the LAN list only fills with 32 games). The page's "?LAN" is kept for an address of
//! a local network and left out for any other, so that a game over the Internet uses the
//! game's own Internet rates and their automatic adjustment.
//!
//! The last address typed is kept in the Co-op page's file (coop_page.rs), as its own value.

use crate::added_text::Text;
use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// Class of the multiplayer Join LAN page.
pub const LAN_JOIN_PAGE: &str = "XIIIMenuMultiLANJoin";
/// Key of the last address in the kept values (coop_page.rs).
const ADDRESS_KEY: &str = "MultiAddress";
/// Slot of the page's GameServers that holds the typed address (the last of its 32).
const TYPED_SLOT: i32 = 31;
/// Where the box and the entry go, in the page's 640 x 480 units (top multiplied by fScaleTo):
/// under the list's Refresh / Back buttons (y 370), above the help bar (seen at 1280 x 720 and
/// 640 x 480: a row at y 432 ran under it).
pub(crate) const ADDRESS_RECT: [f32; 4] = [150.0, 404.0, 280.0, 28.0];
pub(crate) const JOIN_RECT: [f32; 4] = [440.0, 404.0, 80.0, 28.0];
/// Label box of the address box (its title), in pixels before the page's ratio (100 was too
/// narrow for "Adresse IP": the title ran out of it on the left).
const ADDRESS_TITLE_W: f32 = 130.0;
/// Characters the box takes (as the Co-op page's) and its longest address.
const ADDRESS_CHARS: &str = "0123456789.:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-_";
const ADDRESS_MAX: i32 = 40;

/// The page's new controls.
#[derive(Debug, Default)]
pub(crate) struct MultiJoin {
    page: Option<ObjId>,
    address: Option<ObjId>,
    join: Option<ObjId>,
    /// The typed address being joined (its "start" command is changed when it comes).
    typed: Option<String>,
}

/// Functions of the page hooked here.
pub(crate) fn multi_join_hooked(class: &str, name: &str) -> bool {
    class.eq_ignore_ascii_case(LAN_JOIN_PAGE) && matches!(name, "Created" | "Paint" | "InternalOnKeyEvent" | "InternalOnClick")
}

/// Last address typed on the page, empty without one.
pub fn last_multi_address() -> String {
    crate::coop_page::page_value(ADDRESS_KEY).unwrap_or_default()
}

/// The address is one of a local network (private, loopback or link-local IPv4, "localhost"):
/// joined with the page's own "?LAN".
pub fn local_network_address(address: &str) -> bool {
    let host = address.trim().rsplit_once(':').map_or(address.trim(), |(h, p)| if p.parse::<u16>().is_ok() { h } else { address.trim() });
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_private() || a.is_loopback() || a.is_link_local())
}

/// The URL of the page's "start" command for a typed address: "?LAN" left out when the address
/// is not on a local network (any other option kept, in its order).
pub fn typed_join_url(url: &str, address: &str) -> String {
    if local_network_address(address) {
        return url.to_string();
    }
    let mut parts = url.split('?');
    let mut out = parts.next().unwrap_or("").to_string();
    for p in parts.filter(|p| !p.eq_ignore_ascii_case("LAN")) {
        out.push('?');
        out.push_str(p);
    }
    out
}

/// What has the focus on the page, for the new controls' keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    /// Its Refresh and Back buttons (Controls 0 and 1).
    Refresh,
    Back,
    /// The first listed game (Controls 4), and whether one is listed.
    FirstServer,
    Address,
    Join,
    Other,
}

/// Where Up / Down / Left / Right (0x26, 0x28, 0x25, 0x27) take the focus with the new controls
/// under the Refresh and Back buttons, and the list going round from its first game to them;
/// None: the page's own keys.
pub(crate) fn next_focus(focus: Focus, key: i32, servers: bool) -> Option<Focus> {
    let below = if servers { Focus::FirstServer } else { Focus::Refresh };
    match (focus, key) {
        (Focus::Refresh | Focus::Back, 0x28) => Some(Focus::Address),
        (Focus::FirstServer, 0x26) => Some(Focus::Address),
        (Focus::Address, 0x26) => Some(Focus::Refresh),
        (Focus::Join, 0x26) => Some(Focus::Back),
        (Focus::Address | Focus::Join, 0x28) => Some(below),
        (Focus::Address, 0x27) | (Focus::Join, 0x25 | 0x27) => Some(if focus == Focus::Join { Focus::Address } else { Focus::Join }),
        // Left on the box deletes a character (the box's own key).
        _ => None,
    }
}

impl Vm {
    /// The page was created: the box and the entry are added last
    /// to its Controls (its own keys keep their indices).
    pub(crate) fn multi_join_created(&mut self, page: ObjId) {
        self.improved.menu_boxes.multi_join = MultiJoin { page: Some(page), ..Default::default() };
        let scale = self.page_scale_of(page);
        let add = |vm: &mut Vm, class: &str, [x, y, w, h]: [f32; 4]| -> Option<ObjId> {
            let class = vm.class(class)?;
            let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(y * scale), Value::Float(w), Value::Float(h * scale)];
            let c = vm.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object())?;
            if let Value::Array(mut controls) = vm.prop(page, "Controls") {
                controls.push(Value::Object(Some(c)));
                vm.set_prop(page, "Controls", Value::Array(controls));
            }
            Some(c)
        };
        if let Some(edit) = add(self, "XIIIEditCtrl", ADDRESS_RECT) {
            let title = self.added_text(Text::CoopAddressBox).to_string();
            self.set_prop(edit, "TitleText", Value::Str(title));
            self.set_prop(edit, "bCalculateSize", Value::Bool(false));
            self.set_prop(edit, "AllowedCharSet", Value::Str(ADDRESS_CHARS.to_string()));
            self.set_prop(edit, "MaxWidth", Value::Int(ADDRESS_MAX));
            let _ = self.call_named(edit, "SetText", vec![Value::Str(last_multi_address())]);
            self.improved.menu_boxes.multi_join.address = Some(edit);
        }
        if let Some(join) = add(self, "XIIIButton", JOIN_RECT) {
            let label = self.added_text(Text::CoopJoinNow).to_string();
            self.set_prop(join, "Text", Value::Str(label));
            self.set_prop(join, "bUseBorder", Value::Bool(true));
            self.improved.menu_boxes.multi_join.join = Some(join);
        }
    }

    /// Hooks of the page's InternalOnKeyEvent (the new controls' keys) and InternalOnClick (the
    /// Join entry, clicked or Enter).
    #[allow(clippy::type_complexity)]
    pub(crate) fn multi_join_call(&mut self, f: usize, this: ObjId, name: &str, args: Vec<Option<Value>>) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        if name == "Created" {
            let r = self.call_inner(f, this, args);
            if r.is_ok() {
                self.multi_join_created(this);
            }
            return Ok(r);
        }
        let mj = &self.improved.menu_boxes.multi_join;
        if mj.page != Some(this) {
            return Err(args);
        }
        let (address, join) = (mj.address, mj.join);
        match name {
            // The title's box in pixels, at the page's ratio (not known yet when it is created:
            // the box was a pixel wide, its title spilling out of it).
            "Paint" => {
                if let Some(edit) = address {
                    let rx = self.propr(this, "fRatioX").as_float().max(0.01);
                    self.set_prop(edit, "FirstBoxWidth", Value::Float(ADDRESS_TITLE_W * rx));
                }
                Err(args)
            }
            "InternalOnClick" => {
                let sender = args.first().cloned().flatten().and_then(|v| v.as_object());
                if sender.is_none() || sender != join {
                    return Err(args);
                }
                self.multi_join_typed(f, this);
                Ok(Ok((Value::Bool(true), vec![None; args.len()])))
            }
            "InternalOnKeyEvent" => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                if state != 1 || !matches!(key, 0x25..=0x28) {
                    return Err(args);
                }
                let controls: Vec<Option<ObjId>> = match self.prop(this, "Controls") {
                    Value::Array(v) => v.iter().map(Value::as_object).collect(),
                    _ => Vec::new(),
                };
                let at = |i: usize| controls.get(i).copied().flatten();
                let focused = self.propr(this, "FocusedControl").as_object();
                let focus = match focused {
                    Some(c) if Some(c) == address => Focus::Address,
                    Some(c) if Some(c) == join => Focus::Join,
                    Some(c) if Some(c) == at(0) => Focus::Refresh,
                    Some(c) if Some(c) == at(1) => Focus::Back,
                    Some(c) if Some(c) == at(4) => Focus::FirstServer,
                    _ => Focus::Other,
                };
                let servers = at(4).is_some_and(|c| !self.propr(c, "bNeverFocus").as_bool());
                let target = next_focus(focus, key, servers).and_then(|t| match t {
                    Focus::Address => address,
                    Focus::Join => join,
                    Focus::Refresh => at(0),
                    Focus::Back => at(1),
                    Focus::FirstServer => at(4),
                    Focus::Other => None,
                });
                let Some(target) = target else { return Err(args) };
                let _ = self.call_named(target, "FocusFirst", vec![Value::Object(Some(this)), Value::Bool(false)]);
                let outs = self.functions[f]
                    .params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Byte(0))) } else { None })
                    .collect();
                Ok(Ok((Value::Bool(true), outs)))
            }
            _ => Err(args),
        }
    }

    /// Join: the page's own join of its first listed game, with the typed address as the
    /// server's (kept for the next time). Nothing typed: the box takes the focus.
    fn multi_join_typed(&mut self, f: usize, page: ObjId) {
        let Some(edit) = self.improved.menu_boxes.multi_join.address else { return };
        let address = self.propr(edit, "TextStr").as_name().trim().to_string();
        if address.is_empty() {
            let _ = self.call_named(edit, "FocusFirst", vec![Value::Object(Some(page)), Value::Bool(false)]);
            return;
        }
        crate::coop_page::keep_page_value(ADDRESS_KEY, &address);
        let first = match self.prop(page, "ServerButton") {
            Value::Array(v) => v.first().and_then(Value::as_object),
            _ => None,
        };
        let Some(first) = first else {
            self.warn("Multiplayer: no server button to join with".to_string());
            return;
        };
        let _ = self.call_inner(f, page, vec![Some(Value::Object(Some(first)))]);
        // The page now waits 2 s before its "start" command: the server it joins is the typed one.
        let mut servers = self.prop(page, "GameServers");
        if let Value::Array(v) = &mut servers {
            if let Some(Value::Struct(_, fields)) = v.get_mut(TYPED_SLOT as usize) {
                if let Some(ip) = fields.first_mut() {
                    *ip = Value::Str(address.clone());
                }
            }
        }
        self.set_prop(page, "GameServers", servers);
        self.set_prop(page, "IndexOfServerToJoin", Value::Int(TYPED_SLOT));
        self.log.push(format!("Log: Multiplayer: joining the typed address {address}"));
        self.improved.menu_boxes.multi_join.typed = Some(address);
    }

    /// The "start" console command (console.rs): the page's join of a typed address gets its URL
    /// for that address (typed_join_url). Any other command is left as it is.
    pub(crate) fn multi_join_start_url(&mut self, url: &str) -> String {
        let Some(address) = self.improved.menu_boxes.multi_join.typed.take() else { return url.to_string() };
        let host = url.split('?').next().unwrap_or("").trim();
        if !host.eq_ignore_ascii_case(&address) {
            return url.to_string();
        }
        typed_join_url(url, &address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_and_their_urls() {
        for a in ["192.168.1.20", "10.0.0.2:7778", "172.20.3.4", "127.0.0.1", "localhost:7777", "169.254.1.1"] {
            assert!(local_network_address(a), "{a}");
        }
        for a in ["203.0.113.7", "203.0.113.7:7778", "172.32.0.1", "game.example.org", "8.8.8.8"] {
            assert!(!local_network_address(a), "{a}");
        }
        let url = "203.0.113.7:7778?LAN?Name=XIII?SK=x?team=0";
        assert_eq!(typed_join_url(url, "203.0.113.7:7778"), "203.0.113.7:7778?Name=XIII?SK=x?team=0");
        let url = "192.168.1.20?LAN?Name=XIII?SK=x?team=0";
        assert_eq!(typed_join_url(url, "192.168.1.20"), url, "a local network keeps the page's LAN");
    }

    #[test]
    fn start_url_changed_only_for_the_typed_join() {
        let mut vm = Vm::default();
        let url = "203.0.113.7?LAN?Name=XIII?SK=x?team=0";
        assert_eq!(vm.multi_join_start_url(url), url, "a join of a listed game: the page's own");
        vm.improved.menu_boxes.multi_join.typed = Some("203.0.113.7".into());
        assert_eq!(vm.multi_join_start_url(url), "203.0.113.7?Name=XIII?SK=x?team=0");
        assert_eq!(vm.multi_join_start_url(url), url, "once");
        vm.improved.menu_boxes.multi_join.typed = Some("198.51.100.1".into());
        assert_eq!(vm.multi_join_start_url(url), url, "another server: left as it is");
    }

    #[test]
    fn keys_reach_the_box_and_the_entry() {
        assert_eq!(next_focus(Focus::Refresh, 0x28, true), Some(Focus::Address));
        assert_eq!(next_focus(Focus::Back, 0x28, false), Some(Focus::Address));
        assert_eq!(next_focus(Focus::Address, 0x27, true), Some(Focus::Join));
        assert_eq!(next_focus(Focus::Join, 0x25, true), Some(Focus::Address));
        assert_eq!(next_focus(Focus::Address, 0x28, true), Some(Focus::FirstServer));
        assert_eq!(next_focus(Focus::Join, 0x28, false), Some(Focus::Refresh));
        assert_eq!(next_focus(Focus::FirstServer, 0x26, true), Some(Focus::Address));
        assert_eq!(next_focus(Focus::Address, 0x26, true), Some(Focus::Refresh));
        // The page's own keys elsewhere, and Left on the box (it deletes a character).
        assert_eq!(next_focus(Focus::Other, 0x28, true), None);
        assert_eq!(next_focus(Focus::Refresh, 0x26, true), None);
        assert_eq!(next_focus(Focus::Address, 0x25, true), None);
        assert!(multi_join_hooked("XIIIMenuMultiLANJoin", "InternalOnClick") && multi_join_hooked("XIIIMenuMultiLANJoin", "Created") && multi_join_hooked("XIIIMenuMultiLANJoin", "Paint"));
        // Under the Refresh / Back row (370 + 30), above the help bar (seen from about 418).
        for r in [ADDRESS_RECT, JOIN_RECT] {
            assert!(r[1] >= 400.0 && r[1] + r[3] <= 434.0 && r[0] + r[2] <= 640.0, "{r:?}");
        }
        assert!(ADDRESS_RECT[0] + ADDRESS_RECT[2] < JOIN_RECT[0]);
        assert!(!multi_join_hooked("XIIIMenuMultiLANHost", "InternalOnClick"));
    }
}
