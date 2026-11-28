//! Network co-op (improved build): how the menus ask for a co-op game over the network, and the
//! addresses this computer gives to the other player.
//!
//! The game itself is played in lockstep (lockstep.rs, and the window's lockstep_play.rs): the
//! Co-op page's Host LAN game marks the next campaign map the menus load with [`COOP_OPTION`]
//! ("Hual01a?Listen?OXIIICoop"), its Join LAN game travels to the host's address with it
//! ("192.0.2.20?OXIIICoop"); the window takes those travels and meets the other machine.

use crate::Vm;

/// URL option of a co-op game over the network ("Hual01a?Listen?OXIIICoop" on the host,
/// "192.0.2.20?OXIIICoop" to join).
pub const COOP_OPTION: &str = "OXIIICoop";

/// URL option of the host's New game (lockstep co-op): the first level is asked for without the
/// main menu's video, which both machines play once they met ("Plage00?OXIIIIntro").
pub const INTRO_OPTION: &str = "OXIIIIntro";

/// Default port of the host (the game's own network port).
pub const DEFAULT_PORT: u16 = 7777;

/// This computer's addresses that another computer of the local network can join, the main one
/// first: the address the system uses to reach other networks (its default route, found without
/// sending anything), then the other addresses of the computer's name (other network adapters:
/// a second network, a gaming VPN). Loopback and self-assigned (169.254.x.x) addresses are left
/// out, and so are the other addresses ending in .1 (a virtual switch's or router's own address,
/// such as the one Windows gives its virtual machines' network).
pub fn lan_addresses() -> Vec<std::net::Ipv4Addr> {
    use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs, UdpSocket};
    let usable = |a: &Ipv4Addr| !a.is_loopback() && !a.is_link_local() && !a.is_unspecified() && !a.is_broadcast() && !a.is_multicast();
    let mut out: Vec<Ipv4Addr> = Vec::new();
    // Connecting a UDP socket only chooses the route: no packet leaves the computer.
    let main = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok().and_then(|s| s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok().and_then(|_| s.local_addr().ok()));
    if let Some(IpAddr::V4(a)) = main.map(|a| a.ip()) {
        if usable(&a) {
            out.push(a);
        }
    }
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    if !name.trim().is_empty() {
        if let Ok(addrs) = (name.trim(), 0).to_socket_addrs() {
            for a in addrs {
                if let IpAddr::V4(a) = a.ip() {
                    if usable(&a) && a.octets()[3] != 1 && !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
        }
    }
    out
}

/// An address as a player types it to join: the port only when it is not the game's own.
pub fn address_text(a: std::net::Ipv4Addr, port: Option<u16>) -> String {
    match port.filter(|&p| p != DEFAULT_PORT) {
        Some(p) => format!("{a}:{p}"),
        None => a.to_string(),
    }
}

impl Vm {
    /// Entry point for a menu: joins a co-op host at `address` ("192.0.2.20",
    /// "192.0.2.20:7778"). The travel is the engine's own (UGameEngine::SetClientTravel, taken on
    /// the next frame); the window meets the host there. Returns `false` in a build without
    /// co-op (not the improved build) or for an empty address. The Co-op page's Host needs no
    /// call: with `CoopMode::LanHost` in the local-play state, the next campaign map the menu
    /// loads is hosted (`coop_lan_host_url`).
    pub fn coop_join(&mut self, address: &str) -> bool {
        use crate::local_play::{CoopMode, CoopSession};
        if !crate::improved::local_play::CAMPAIGN_AVAILABLE {
            return false;
        }
        let address = address.trim();
        if address.is_empty() {
            return false;
        }
        let url = format!("{address}?{COOP_OPTION}");
        self.change_local_play(|p| {
            let start = p.coop.filter(|s| s.mode == CoopMode::LanJoin).and_then(|s| s.start);
            p.coop = Some(CoopSession { mode: CoopMode::LanJoin, start });
        });
        self.log.push(format!("Log: Co-op: session {url}"));
        self.client_travel = Some((url, crate::travel::TRAVEL_ABSOLUTE, false));
        true
    }

    /// The local-play state asks for this machine to host the co-op game.
    fn coop_lan_host_wanted(&self) -> bool {
        self.local_play().coop.is_some_and(|s| s.mode == crate::local_play::CoopMode::LanHost)
    }

    /// A travel of this machine while the menus host a co-op game (Co-op page, Host LAN game):
    /// the next campaign map it loads (New game, Continue, Load, Select level) gets "Listen" and
    /// [`COOP_OPTION`], for the window to start the session with it. The menu's own map, a
    /// failed travel and a server's address are left as they are.
    pub(crate) fn coop_lan_host_url(&mut self, url: &mut crate::travel::Url) {
        let map = url.map.rsplit_once('.').map_or(url.map.as_str(), |(m, _)| m).to_ascii_lowercase();
        let local = self.config.as_mut().and_then(|c| c.get_string("URL", "LocalMap", "System")).unwrap_or_default();
        let local = local.rsplit_once('.').map_or(local.as_str(), |(m, _)| m).to_ascii_lowercase();
        let menu = !map.is_empty() && (map == local || map.starts_with("mapmenu"));
        if !self.coop_lan_host_wanted() || !url.host.is_empty() || url.has_option(COOP_OPTION) || url.has_option("failed") || map.is_empty() || menu {
            return;
        }
        url.add_option("Listen");
        url.add_option(COOP_OPTION);
        self.log.push(format!("Log: Co-op: the session goes on with {}", url.string()));
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_play::{CoopMode, CoopSession};
    use crate::travel::{Url, TRAVEL_ABSOLUTE};

    #[test]
    fn the_menus_ask_for_a_network_coop_game_by_its_urls() {
        let mut vm = Vm::default();
        // Joining: the host's address with the option, the session's mode set.
        assert!(!vm.coop_join("  "), "nothing to join");
        assert!(vm.client_travel.is_none());
        assert!(vm.coop_join("192.0.2.20:7778"));
        assert_eq!(vm.client_travel.as_ref().map(|t| t.0.as_str()), Some("192.0.2.20:7778?OXIIICoop"));
        assert_eq!(vm.local_play().coop.map(|s| s.mode), Some(CoopMode::LanJoin));
        // Hosting: a campaign map gets the options, the menu's own map does not.
        vm.change_local_play(|p| p.coop = Some(CoopSession { mode: CoopMode::LanHost, start: None }));
        let mut url = Url::parse(None, "Hual01a", TRAVEL_ABSOLUTE);
        vm.coop_lan_host_url(&mut url);
        assert!(url.has_option("Listen") && url.has_option(COOP_OPTION), "{}", url.string());
        let mut menu = Url::parse(None, "MapMenu", TRAVEL_ABSOLUTE);
        vm.coop_lan_host_url(&mut menu);
        assert!(!menu.has_option(COOP_OPTION));
        // Negative control: solo (no co-op in the local-play state) keeps its URLs.
        vm.change_local_play(|p| p.coop = None);
        let mut solo = Url::parse(None, "Hual01a", TRAVEL_ABSOLUTE);
        vm.coop_lan_host_url(&mut solo);
        assert!(!solo.has_option("Listen") && !solo.has_option(COOP_OPTION));
        assert_eq!(address_text(std::net::Ipv4Addr::new(192, 0, 2, 20), Some(DEFAULT_PORT)), "192.0.2.20");
        assert_eq!(address_text(std::net::Ipv4Addr::new(192, 0, 2, 20), Some(7778)), "192.0.2.20:7778");
    }

}
