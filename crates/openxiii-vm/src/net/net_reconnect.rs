//! Enhanced network play (`Vm::net_enhanced`): a client that loses its connection to the server
//! joins the same address again for a while instead of going back to the menu at once.
//!
//! The server's URL this client joined (with its own options: name, skin, team) is joined again
//! for up to `RECONNECT_SECONDS`. Network co-op is played in lockstep (lockstep.rs), not here.
//!
//! Only a silent loss (the connection timed out, the server not answering yet) is tried again:
//! a server that closes the connection (its player quits, a host back to its menu), a
//! refusal (the server full, a wrong password, an outdated version), a kick and the player's own
//! quit go back to the menu as in the original. Escape
//! (or the gamepad's back) during the attempts gives up at once. When the time is up: the
//! game's own "?failed" travel, back to the menu.
//!
//! The protocol is the game's own: a reconnection is an ordinary join.

use crate::travel::Url;
use crate::value::Value;
use crate::Vm;

/// How long a client tries to join its server again (seconds of application time).
pub(crate) const RECONNECT_SECONDS: f64 = 30.0;
/// Wait before another attempt after one failed (seconds).
const RETRY_DELAY: f32 = 2.0;
/// The attempt's message is shown again every this many seconds.
const MESSAGE_EVERY: f64 = 1.0;

/// A client's reconnection state.
#[derive(Debug, Default, Clone)]
pub(crate) struct NetReconnect {
    /// Classic multiplayer: the URL of the server this client joined.
    pub url: Option<String>,
    /// Until when (application time) the server is joined again; None: not trying.
    pub until: Option<f64>,
    /// The server refused or kicked this client: never joined again.
    pub refused: bool,
    /// The connection to the server timed out (a silent loss: the network cut, the server's
    /// machine gone). A server that closes the connection (its player quit, a host back to its
    /// menu) is not joined again: back to the menu at once, as in the original.
    pub timed_out: bool,
    /// Application time the attempt's message was last shown.
    pub shown: f64,
}

impl NetReconnect {
    /// A join of a network game started (not a reconnection): its URL, if it is not co-op.
    pub fn joined(&mut self, url: &str, coop: bool) {
        if self.until.is_some() {
            return;
        }
        self.refused = false;
        self.timed_out = false;
        self.url = (!coop).then(|| url.to_string());
    }

    /// The URL to join again at `now`, or None: give up (and forget the server).
    pub fn rejoin(&mut self, now: f64) -> Option<String> {
        // Not a silent loss (and not already trying): the server closed the connection.
        let lost = self.timed_out || self.until.is_some();
        let url = self.url.clone().filter(|_| !self.refused && lost);
        let Some(url) = url else {
            self.clear();
            return None;
        };
        let until = *self.until.get_or_insert(now + RECONNECT_SECONDS);
        if now > until {
            self.clear();
            return None;
        }
        Some(url)
    }

    pub fn clear(&mut self) {
        self.url = None;
        self.until = None;
        self.shown = 0.0;
        self.timed_out = false;
    }
}

impl Vm {
    /// UGameEngine::Browse of a network URL (travel.rs, the connection started): remembered to
    /// be joined again (enhanced network play).
    pub(crate) fn net_reconnect_note_join(&mut self, url: &Url) {
        if self.net_enhanced() {
            let coop = url.has_option(crate::coop_address::COOP_OPTION);
            self.coop.reconnect.joined(&url.string(), coop);
        }
    }

    /// A travel to a map of this machine (the menu, a solo map): no server to join again.
    pub(crate) fn net_reconnect_local_travel(&mut self) {
        self.coop.reconnect.clear();
    }

    /// The pending level loaded the server's map: joined again (or joined at last).
    pub(crate) fn net_reconnect_joined(&mut self) {
        // Joined (again): no "host left" box later.
        self.coop.host_left_notice = false;
        if self.coop.reconnect.until.take().is_some() {
            self.log.push("Log: Network: joined the server again".into());
        }
    }

    /// Enhanced network play: this client's game ended because its host left (closed the
    /// connection, ended the co-op session) or was lost; the menu will say so.
    pub(crate) fn net_host_left(&mut self) {
        if self.net_enhanced() && !self.coop.reconnect.refused {
            self.coop.host_left_notice = true;
        }
    }

    /// The menu map is back after a host left (`net_host_left`): its page is shown, then a box
    /// tells why (without it, the guest only saw a second of "connection failed" before the
    /// menu). The engine's connection status of a failed join attempt is cleared first: on the
    /// menu map XIIIRootWindow.Tick opens XIIIMenuLiveStatsWritePage("netfailure") for it, a
    /// console page whose message box is decided before its parameter arrives (InitComponent
    /// then HandleParameters): an empty page that only Escape left.
    pub(crate) fn net_host_left_notice_tick(&mut self) {
        if !self.coop.host_left_notice || self.ipdrv.net_driver.is_some() {
            return;
        }
        let map = self.url.map.to_ascii_lowercase();
        if !map.starts_with("mapmenu") {
            return;
        }
        if self.ipdrv.connection_status != 0 {
            self.ipdrv.connection_status = 0;
            self.ipdrv.connection_error.clear();
        }
        let Some(root) = self.gui_local() else { return };
        let active = self.propr(root, "ActivePage").as_object();
        if self.level_time < 1.5 || active.is_none() {
            return;
        }
        self.coop.host_left_notice = false;
        let line = self.host_left_message();
        let opened = self.call_named(root, "OpenMenu", vec![Value::Str("XIDInterf.XIIIMsgBox".into()), Value::Bool(false)]).is_ok_and(|v| v.as_bool());
        let msgbox = self.propr(root, "ActivePage").as_object().filter(|&m| opened && Some(m) != active && self.is_a(m, "XIIIMsgBox"));
        match msgbox {
            Some(m) => {
                // QBTN_Ok (GUI.uc): one OK button, which closes the box.
                let _ = self.call_named(m, "SetupQuestion", vec![Value::Str(line), Value::Byte(1), Value::Byte(1)]);
                let _ = self.call_named(m, "InitBox", vec![Value::Float(120.0), Value::Float(130.0), Value::Float(10.0), Value::Float(10.0), Value::Float(400.0), Value::Float(230.0)]);
                self.log.push("Log: Network: the menu says the host left".into());
            }
            None => self.log.push("Log: Network: the host left (no message box opened)".into()),
        }
    }

    /// The connection to the server timed out (net_game.rs): a silent loss.
    pub(crate) fn net_reconnect_timed_out(&mut self) {
        self.coop.reconnect.timed_out = true;
    }

    /// The server refused this client (FAILURE, a kick, an outdated version): not joined again.
    pub(crate) fn net_reconnect_refused(&mut self) {
        self.coop.reconnect.refused = true;
    }

    /// End of UGameEngine::Tick, a network failure reported (travel.rs): the classic multiplayer
    /// server's URL to join again, or None (the game's own "?failed").
    pub(crate) fn net_reconnect_rejoin(&mut self) -> Option<String> {
        if !self.net_enhanced() {
            return None;
        }
        let url = self.coop.reconnect.rejoin(self.app_seconds)?;
        self.log.push(format!("Log: Network: joining the server again ({url})"));
        self.net_reconnect_message();
        Some(url)
    }

    /// A lost connection (ULevel::TickNetClient, net_game.rs) that will be joined again: the
    /// attempts start at once instead of after the game's 5 to 8 s of "connection failed".
    pub(crate) fn net_reconnect_soon(&self) -> bool {
        let r = &self.coop.reconnect;
        self.net_enhanced() && r.url.is_some() && !r.refused && r.timed_out && !self.ipdrv.connection_failure
    }

    /// This client is trying to join its server again.
    pub(crate) fn net_reconnecting(&self) -> bool {
        self.net_enhanced() && self.coop.reconnect.until.is_some()
    }

    /// UGameEngine::Tick of the pending level, its connection failed (travel.rs): while trying
    /// again, another attempt in RETRY_DELAY seconds (the end of the engine's tick calls the
    /// rejoin again); true when it is so. A refusal stops the attempts.
    pub(crate) fn net_reconnect_pending_failed(&mut self) -> bool {
        if !self.net_reconnecting() || self.coop.reconnect.refused {
            return false;
        }
        self.engine_report_net_failure(RETRY_DELAY);
        true
    }

    /// Each engine tick while trying again: the message kept on screen; the time up during an
    /// attempt: the attempt abandoned and back to the menu.
    pub(crate) fn net_reconnect_tick(&mut self) {
        if !self.net_reconnecting() {
            return;
        }
        let until = self.coop.reconnect.until.unwrap_or(f64::MAX);
        if self.app_seconds > until && !self.ipdrv.connection_failure {
            self.net_reconnect_give_up("the time is up");
            return;
        }
        if self.app_seconds - self.coop.reconnect.shown >= MESSAGE_EVERY {
            self.net_reconnect_message();
        }
    }

    /// The attempt's message: the game's own "Connecting to the server" of its Join LAN page,
    /// under its "connection failed".
    fn net_reconnect_message(&mut self) {
        self.coop.reconnect.shown = self.app_seconds;
        let failed = self.localize("Errors", "ConnectionFailed", "Engine", false);
        let connecting = self.localize("XIIIMenuMultiLANJoin", "ConnectingText", "XIDInterf", false);
        self.engine_set_progress(&failed, &connecting, (MESSAGE_EVERY * 2.0) as f32);
    }

    /// Escape (or the gamepad's back) while trying again (UInteractionMaster's key event): the
    /// attempts given up, back to the menu. True when the key was taken.
    pub(crate) fn net_reconnect_key(&mut self, key: u8, action: u8) -> bool {
        if action != 1 || key != 0x1B || !self.net_reconnecting() {
            return false;
        }
        self.net_reconnect_give_up("cancelled");
        true
    }

    fn net_reconnect_give_up(&mut self, why: &str) {
        self.log.push(format!("Log: Network: no more joining the server again ({why})"));
        if why != "cancelled" {
            self.net_host_left();
        }
        self.coop.reconnect.clear();
        self.engine_cancel_pending();
        self.ipdrv.connection_failure = false;
        self.engine_set_progress("", "", 0.0);
        self.client_travel = Some(("?failed".into(), crate::travel::TRAVEL_ABSOLUTE, false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lost_server_is_joined_again_for_a_while() {
        let mut r = NetReconnect::default();
        r.joined("203.0.113.7?Name=XIII?SK=x?team=0", false);
        // The server closed the connection (its player quit): back to the menu, no attempt.
        assert_eq!(r.rejoin(50.0), None);
        r.joined("203.0.113.7?Name=XIII?SK=x?team=0", false);
        r.timed_out = true;
        assert_eq!(r.rejoin(100.0).as_deref(), Some("203.0.113.7?Name=XIII?SK=x?team=0"));
        // Still the same window of time: a failed attempt does not restart it.
        r.joined("203.0.113.7?Name=XIII?SK=x?team=0", false);
        assert!(r.rejoin(100.0 + RECONNECT_SECONDS - 1.0).is_some());
        assert_eq!(r.rejoin(100.0 + RECONNECT_SECONDS + 0.1), None, "time up");
        assert_eq!(r.rejoin(200.0), None, "the server is forgotten");
        // Refused or kicked: never joined again.
        r.joined("203.0.113.7", false);
        r.timed_out = true;
        r.refused = true;
        assert_eq!(r.rejoin(300.0), None);
        // A new join clears the refusal; a co-op URL is not one to join again.
        r.joined("203.0.113.7", false);
        r.timed_out = true;
        assert!(r.rejoin(400.0).is_some());
        r.clear();
        r.joined("203.0.113.7?OXIIICoop", true);
        assert_eq!(r.rejoin(500.0), None);
    }

    #[test]
    fn the_menu_says_the_host_left_and_no_empty_failure_page() {
        let mut vm = Vm::default();
        crate::net_smooth::set_test_net_enhanced(true);
        vm.net_host_left();
        assert!(vm.coop.host_left_notice);
        // Back on the menu map: the failed join's status is cleared (it opened an empty page).
        vm.url = Url::parse(None, "MapMenu", crate::travel::TRAVEL_ABSOLUTE);
        vm.ipdrv.connection_status = 1;
        vm.ipdrv.connection_error = "unreachable".into();
        vm.level_time = 2.0;
        vm.net_host_left_notice_tick();
        assert_eq!(vm.ipdrv.connection_status, 0);
        assert!(vm.coop.host_left_notice, "told once a menu page is shown (none here)");
        // Joined again after the loss: nothing to tell any more.
        vm.net_reconnect_joined();
        assert!(!vm.coop.host_left_notice);
        // A refused or kicked client is told by the game's own words.
        let mut vm = Vm::default();
        vm.coop.reconnect.refused = true;
        vm.net_host_left();
        assert!(!vm.coop.host_left_notice);
        // Negative control: Graphics: Original keeps the original's behaviour.
        crate::net_smooth::set_test_net_enhanced(false);
        let mut vm = Vm::default();
        vm.net_host_left();
        assert!(!vm.coop.host_left_notice);
        vm.url = Url::parse(None, "MapMenu", crate::travel::TRAVEL_ABSOLUTE);
        vm.ipdrv.connection_status = 1;
        vm.net_host_left_notice_tick();
        assert_eq!(vm.ipdrv.connection_status, 1);
    }

    #[test]
    fn a_multiplayer_client_reconnects_only_in_enhanced_play() {
        let url = Url::parse(None, "203.0.113.7?Name=XIII?SK=x?team=0", crate::travel::TRAVEL_ABSOLUTE);
        let mut vm = Vm::default();
        // Enhanced: the lost server is joined again, the attempts show the message, Escape stops.
        crate::net_smooth::set_test_net_enhanced(true);
        vm.net_reconnect_note_join(&url);
        vm.app_seconds = 10.0;
        // A server that closed the connection: not joined again (negative control below).
        assert_eq!(vm.net_reconnect_rejoin(), None);
        assert!(!vm.net_reconnect_soon());
        vm.net_reconnect_note_join(&url);
        vm.net_reconnect_timed_out();
        assert!(vm.net_reconnect_soon());
        assert!(vm.net_reconnect_rejoin().is_some_and(|u| u.starts_with("203.0.113.7")));
        assert!(vm.net_reconnecting());
        assert!(!vm.net_reconnect_key(0x1B, 3), "a release does nothing");
        assert!(vm.net_reconnect_key(0x1B, 1));
        assert!(!vm.net_reconnecting());
        assert_eq!(vm.client_travel.as_ref().map(|t| t.0.as_str()), Some("?failed"));
        // Refused: back to the menu.
        vm.client_travel = None;
        vm.net_reconnect_note_join(&url);
        vm.net_reconnect_timed_out();
        vm.net_reconnect_refused();
        assert_eq!(vm.net_reconnect_rejoin(), None);
        // Negative control: Graphics: Original (or the faithful build) keeps the original's
        // return to the menu.
        crate::net_smooth::set_test_net_enhanced(false);
        let mut vm = Vm::default();
        vm.net_reconnect_note_join(&url);
        assert_eq!(vm.net_reconnect_rejoin(), None);
        assert!(!vm.net_reconnecting() && !vm.net_reconnect_key(0x1B, 1));
    }
}
