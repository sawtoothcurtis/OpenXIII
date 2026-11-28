//! Network additions for the improved edition. Per-connection state follows that connection.
use crate::travel::Url;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Not in the original: control-channel text by which a client updates its aim assist on the
/// host after LOGIN ("OXIIIAUTOAIM V=0|1", see Vm::net_client_aim_tick). A host that does not
/// know it only logs it (ULevel::NotifyReceivedText ignores unknown commands). Improved build only.
const AIM_COMMAND: &str = "OXIIIAUTOAIM";

/// Not in the original: control-channel text by which an enhanced LAN host tells each client
/// the frame rate to play at ("OXIIIHOSTRATE R=<frames a second>", see Vm::net_host_rate_tick).
/// A client that does not know it only logs it (ULevel::NotifyReceivedText ignores unknown
/// commands) and keeps the game's own rate. Improved build only.
const RATE_COMMAND: &str = "OXIIIHOSTRATE";

/// Enhanced LAN game: lowest and highest frame rates of a host or a client. The host draws at
/// its screen's refresh rate, never below the game's LanServerMaxTickRate nor above
/// LAN_RATE_MAX; a client draws at the rate its host announces, within these bounds.
pub(crate) const LAN_RATE_MIN: u32 = 60;
pub(crate) const LAN_RATE_MAX: u32 = 240;

/// Enhanced LAN game: moves a second the host takes from all its clients together (each
/// client sends one move a frame). Every client gets an equal share, at least LAN_RATE_MIN and
/// at most the host's own rate: LAN_RATE_MAX each up to 4 clients, less beyond, whatever the
/// number of players.
pub(crate) const LAN_MOVES_BUDGET: u32 = 960;

/// Rate a host drawing `host` frames a second announces to each of its `clients` clients.
pub(crate) fn announced_rate(host: u32, clients: usize) -> u32 {
    let share = LAN_MOVES_BUDGET / (clients.max(1) as u32).min(LAN_MOVES_BUDGET);
    host.min(share.max(LAN_RATE_MIN))
}

/// The floor of PlayerController.ReplicateMove's time between two moves
/// (`NetMoveDelta = FMax(64.0 / Player.CurrentNetSpeed, 0.011)`), 90 moves a second.
pub(crate) const REPLICATE_MOVE_FLOOR: f32 = 0.011;

// Keep the public driver and connection fields available to struct literals and patterns.
// These additions live and drop with the corresponding driver or connection.
macro_rules! with_edition_fields {
    ($(#[$attr:meta])* pub struct IpDrv { $($fields:tt)* }) => {
        $(#[$attr])*
        pub struct IpDrv {
            $($fields)*
        }
    };
    ($(#[$attr:meta])* pub struct NetConn { $($fields:tt)* }) => {
        $(#[$attr])*
        pub struct NetConn {
            $($fields)*
            /// The last aim value sent to the host, or a value received before JOIN.
            pub aim_value: Option<i32>,
            /// Enhanced LAN game: on the host, the frame rate last announced to this client
            /// (RATE_COMMAND); on a client, the rate its host announced.
            pub tick_rate: Option<u32>,
        }
    };
    (NetConn { $($values:tt)* }) => {
        NetConn { $($values)* aim_value: None, tick_rate: None }
    };
}
pub(crate) use with_edition_fields;

pub(crate) fn network_presentation_enhanced(graphics: crate::video_settings::SessionGraphics) -> bool {
    graphics.enhanced()
}

impl Vm {
    /// Handle a control-channel change even if its value is malformed or the connection has gone.
    pub(crate) fn receive_aim_update(&mut self, i: usize, text: &str) -> bool {
        let Some(rest) = crate::console::parse_command(text, AIM_COMMAND) else { return false };
        let Some(v) = crate::net_game::parse_int(rest, "V=").map(|v| i32::from(v != 0)) else { return true };
        let Some(c) = self.net_conn(Some(i)) else { return true };
        match c.pc {
            Some(pc) => {
                if self.objects.get(pc).is_some_and(Option::is_some) {
                    self.set_prop(pc, "iAutoAimMode", Value::Int(v));
                    self.log.push(format!("DevNet: client aim assist: {v}"));
                }
            }
            // Before JOIN: kept for the controller spawned then (aim_client_at_join).
            None => c.aim_value = Some(v),
        }
        true
    }

    pub(crate) fn apply_network_login(&mut self, i: usize, pc: ObjId, url: &Url) {
        self.aim_client_at_join(i, pc, url);
        self.coop_character_from_login(pc, url);
    }

    pub(crate) fn login_pad_aim(&mut self) -> Option<i32> {
        self.aim_device.pad_active.then(|| self.aim_pad_value())
    }

    pub(crate) fn remember_login_aim(&mut self, mode: i32) {
        if let Some(c) = self.net_conn(None) {
            c.aim_value = Some(mode);
        }
    }

    pub(crate) fn add_network_login_options(&mut self, url: &mut Url) {
        if let Some(option) = self.coop_character_login_option() {
            url.add_option(&option);
        }
    }

    /// Enhanced LAN host: its frame rate (`original`: the game's GetMaxTickRate), raised to its
    /// screen's refresh rate, at most LAN_RATE_MAX. Without a known screen: the game's.
    pub(crate) fn lan_host_tick_rate(&self, original: f32) -> f32 {
        match self.engine.client.display_hz {
            Some(hz) => original.max(hz.min(LAN_RATE_MAX) as f32),
            None => original,
        }
    }

    /// Enhanced LAN client: the rate its host announced, within LAN_RATE_MIN and LAN_RATE_MAX;
    /// before any announcement, or with a host that sends none (the original game), `fallback`.
    pub(crate) fn lan_client_tick_rate(&self, fallback: i32) -> f32 {
        let announced = self.ipdrv.net_driver.as_ref().and_then(|d| d.server_conn.as_ref()).and_then(|c| c.tick_rate);
        announced.map_or(fallback as f32, |r| r.clamp(LAN_RATE_MIN, LAN_RATE_MAX) as f32)
    }

    /// Enhanced LAN host: tells each joined client the rate to play at (announced_rate of the
    /// host's rate and the number of joined clients), again whenever it changes (a player
    /// joining or leaving, the window moved to another screen). Nothing is sent while it stays
    /// the same.
    pub(crate) fn net_host_rate_tick(&mut self) {
        if !self.net_enhanced() {
            return;
        }
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return };
        if !d.lan || d.server_conn.is_some() {
            return;
        }
        let joined: Vec<usize> = d.conns.iter().enumerate().filter(|(_, c)| c.pc.is_some()).map(|(i, _)| i).collect();
        if joined.is_empty() {
            return;
        }
        let host = self.net_max_tick_rate().round_ties_even() as u32;
        let rate = announced_rate(host, joined.len());
        for i in joined {
            let Some(c) = self.net_conn(Some(i)) else { continue };
            if c.tick_rate == Some(rate) {
                continue;
            }
            c.tick_rate = Some(rate);
            self.net_send(Some(i), &format!("{RATE_COMMAND} R={rate}"));
        }
    }

    /// A client's handling of RATE_COMMAND (true if the text is one, even malformed).
    pub(crate) fn receive_host_rate(&mut self, text: &str) -> bool {
        let Some(rest) = crate::console::parse_command(text, RATE_COMMAND) else { return false };
        let Some(r) = crate::net_game::parse_int(rest, "R=").filter(|&r| r > 0) else { return true };
        if let Some(c) = self.net_conn(None) {
            c.tick_rate = Some(r as u32);
        }
        true
    }

    /// FMax(A, B) called by PlayerController.ReplicateMove with B = REPLICATE_MOVE_FLOOR, on an
    /// enhanced LAN client: the floor becomes 1 / LAN_RATE_MAX, so that the client sends a move
    /// each frame, as the host's own player moves each frame; its frame rate (the one its host
    /// announced) is what bounds its moves. Otherwise B unchanged.
    pub(crate) fn replicate_move_floor(&self, b: f32) -> f32 {
        if b != REPLICATE_MOVE_FLOOR || !self.net_enhanced() {
            return b;
        }
        if !self.ipdrv.net_driver.as_ref().is_some_and(|d| d.lan && d.server_conn.is_some()) {
            return b;
        }
        // stack: [.., ReplicateMove, FMax].
        let caller = self.stack.len().checked_sub(2).and_then(|i| self.stack.get(i)).map(|&f| &self.functions[f]);
        match caller {
            Some(f) if f.name.eq_ignore_ascii_case("ReplicateMove") && self.classes[f.owner].name.eq_ignore_ascii_case("PlayerController") => 1.0 / LAN_RATE_MAX as f32,
            _ => b,
        }
    }

    pub(crate) fn host_left_message(&self) -> String {
        self.added_text(crate::added_text::Text::HostLeft).to_string()
    }

    /// Not in the original: aim_client at the JOIN of connection `i`, then the value the client
    /// sent with AIM_COMMAND before its controller existed, which is more recent than the URL's.
    /// Improved build only.
    pub(crate) fn aim_client_at_join(&mut self, i: usize, pc: ObjId, url: &crate::travel::Url) {
        self.aim_client(pc, url);
        if let Some(v) = self.net_conn(Some(i)).and_then(|c| c.aim_value.take()) {
            self.set_prop(pc, "iAutoAimMode", Value::Int(v));
            self.log.push(format!("DevNet: client aim assist: {v}"));
        }
    }

    /// Not in the original: on a network client, once LOGIN has sent the aim assist, each change
    /// of the local controller's iAutoAimMode (switch between gamepad and keyboard, menu option)
    /// is sent to the host, which computes this player's shots with its copy of the controller
    /// (XIIIPlayerController.AdjustAim). Nothing is sent while the value stays the same.
    /// Improved build only.
    pub(crate) fn net_client_aim_tick(&mut self) {
        let Some(sent) = self.net_conn(None).and_then(|c| c.aim_value) else { return };
        let Some(pc) = self.local_aim_controller() else { return };
        let mode = i32::from(self.propr(pc, "iAutoAimMode").as_int() != 0);
        if mode == sent {
            return;
        }
        if let Some(c) = self.net_conn(None) {
            c.aim_value = Some(mode);
        }
        self.net_send(None, &format!("{AIM_COMMAND} V={mode}"));
    }

    /// Lockstep co-op: why a network game could not start or ended, told once the menu map is
    /// back: the Co-op page, and over it the game's message box with the reason (OK closes it).
    /// The profile page first shown on a new menu VM is left to the player before. True when
    /// shown (or nothing can be shown here).
    pub fn lockstep_show_notice(&mut self, reason: crate::added_text::Text) -> bool {
        if !self.url.map.to_ascii_lowercase().starts_with("mapmenu") {
            return false;
        }
        let Some(root) = self.gui_local() else { return false };
        let Some(active) = self.propr(root, "ActivePage").as_object() else { return false };
        if self.level_time < 1.5 || self.is_a(active, "XIIIMenuSelectProfile") {
            return false;
        }
        if !self.open_coop_page() {
            return true;
        }
        let page = self.propr(root, "ActivePage").as_object();
        let line = self.added_text(reason).to_string();
        let opened = self.call_named(root, "OpenMenu", vec![Value::Str("XIDInterf.XIIIMsgBox".into()), Value::Bool(false)]).is_ok_and(|v| v.as_bool());
        match self.propr(root, "ActivePage").as_object().filter(|&m| opened && Some(m) != page && self.is_a(m, "XIIIMsgBox")) {
            Some(m) => {
                let _ = self.call_named(m, "SetupQuestion", vec![Value::Str(line), Value::Byte(1), Value::Byte(1)]);
                let _ = self.call_named(m, "InitBox", vec![Value::Float(120.0), Value::Float(130.0), Value::Float(10.0), Value::Float(10.0), Value::Float(400.0), Value::Float(230.0)]);
                self.log.push(format!("Log: Network: the menu says why the network game stopped ({reason:?})"));
            }
            None => self.log.push("Log: Network: no message box opened for the network game's end".into()),
        }
        true
    }
}

#[cfg(test)]
mod lan_rate_tests {
    use super::{announced_rate, LAN_RATE_MAX, REPLICATE_MOVE_FLOOR};
    use crate::ipdrv::{NetDriver, NetDriverCfg, Sock, SpeedCfg, ENHANCED_CLIENT_INTERNET_MAX, ENHANCED_LAN_CLIENT_RATE};
    use crate::net::{ConnState, Connection};
    use crate::net_game::NetConn;
    use crate::value::Value;
    use crate::Vm;

    fn conn(speed: i32) -> NetConn {
        let mut c = Connection::new(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 7777), ConnState::Open, 0.0, speed);
        c.current_net_speed = speed;
        NetConn::new(c)
    }

    /// A VM with XIII.ini's rates and a network driver (LAN or not): a client's (`client`) or a
    /// host's with `conns` connections at `speed`, the first `joined` of them with a controller.
    fn vm_with(lan: bool, client: bool, conns: usize, joined: usize, speed: i32) -> Vm {
        let mut vm = Vm::new();
        let mut config = crate::config::ConfigCache::new(std::env::temp_dir().join("oxiii-lan-rate"));
        config.set_string("Engine.Player", "ConfiguredInternetSpeed", "5000", "User");
        config.set_string("Engine.Player", "ConfiguredLanSpeed", "10000", "User");
        vm.config = Some(config);
        let cfg = NetDriverCfg {
            net_server_max_tick_rate: 45,
            net_server_min_tick_rate: 30,
            lan_server_max_tick_rate: 90,
            lan_server_min_tick_rate: 60,
            net_client_max_tick_rate: 45,
            lan_client_max_tick_rate: 60,
            client: SpeedCfg { step_up: 20, internet_min: 1500, internet_max: 3000, lan_min: 10000, lan_max: 11000, lag_to_take_action: 0.15, max_allowed_lag: 0.3 },
            ..Default::default()
        };
        let mut driver = NetDriver::new(Sock(None, 0), 0, lan, cfg);
        if client {
            driver.server_conn = Some(conn(speed));
        } else {
            for i in 0..conns {
                let mut c = conn(speed);
                c.pc = (i < joined).then_some(1000 + i);
                driver.conns.push(c);
            }
        }
        vm.ipdrv.net_driver = Some(driver);
        vm
    }

    fn enhanced<T>(on: bool, f: impl FnOnce() -> T) -> T {
        crate::net_smooth::set_test_net_enhanced(on);
        let r = f();
        crate::net_smooth::set_test_net_enhanced(false);
        r
    }

    fn sent_rates(vm: &Vm) -> Vec<Option<u32>> {
        vm.ipdrv.net_driver.as_ref().unwrap().conns.iter().map(|c| c.tick_rate).collect()
    }

    fn client_rate(vm: &Vm) -> Option<u32> {
        vm.ipdrv.net_driver.as_ref().unwrap().server_conn.as_ref().unwrap().tick_rate
    }

    #[test]
    fn share_of_the_moves_budget_follows_the_number_of_clients() {
        // 960 moves a second shared out, at least 60 each, at most the host's rate, whatever
        // the number of clients (negative control: a fixed 240, or a table that stops at 4
        // players, fails from 5 clients on).
        let cases = [(240, 1, 240), (240, 2, 240), (240, 4, 240), (240, 5, 192), (240, 6, 160), (240, 8, 120), (240, 12, 80), (240, 16, 60), (240, 17, 60), (240, 1000, 60), (144, 1, 144), (144, 7, 137), (90, 3, 90), (90, 20, 60), (240, 0, 240)];
        for (host, n, want) in cases {
            assert_eq!(announced_rate(host, n), want, "host {host}, {n} clients");
        }
    }

    #[test]
    fn enhanced_lan_host_draws_at_its_screen_rate() {
        let rate = |on: bool, lan: bool, hz: Option<u32>, conns: usize| {
            let mut vm = vm_with(lan, false, conns, conns, 10000);
            vm.engine.client.display_hz = hz;
            enhanced(on, || vm.net_max_tick_rate())
        };
        // Screens at 60, 75, 120, 144, 240, 360 Hz: never below the game's 90, at most 240.
        for (hz, want) in [(60, 90.0), (75, 90.0), (120, 120.0), (144, 144.0), (240, 240.0), (360, 240.0)] {
            assert_eq!(rate(true, true, Some(hz), 1), want, "{hz} Hz");
        }
        // With 4 clients the game comes down to 60: the screen's rate still wins.
        assert_eq!(rate(true, true, Some(144), 4), 144.0);
        assert_eq!(rate(true, true, Some(60), 4), 60.0);
        // No client yet: the game's 45, or the screen's.
        assert_eq!(rate(true, true, Some(144), 0), 144.0);
        // Unchanged (negative controls): no screen (dedicated server), Graphics: Original, the
        // Internet.
        assert_eq!(rate(true, true, None, 1), 90.0);
        assert_eq!(rate(false, true, Some(144), 1), 90.0);
        assert_eq!(rate(false, true, Some(144), 4), 60.0);
        assert_eq!(rate(true, false, Some(144), 0), 45.0);
    }

    #[test]
    fn enhanced_lan_client_plays_at_the_announced_rate() {
        let rate = |on: bool, lan: bool, announced: Option<u32>, speed: i32| {
            let mut vm = vm_with(lan, true, 0, 0, speed);
            vm.ipdrv.net_driver.as_mut().unwrap().server_conn.as_mut().unwrap().tick_rate = announced;
            enhanced(on, || vm.net_max_tick_rate())
        };
        for (announced, want) in [(Some(144), 144.0), (Some(240), 240.0), (Some(120), 120.0), (Some(30), 60.0), (Some(1000), 240.0), (None, 90.0)] {
            assert_eq!(rate(true, true, announced, 10020), want, "announced {announced:?}");
        }
        // Graphics: Original and the Internet: the game's rates (60 at ConfiguredLanSpeed, 45
        // otherwise), whatever was announced.
        assert_eq!(rate(false, true, Some(144), 10000), 60.0);
        assert_eq!(rate(false, true, Some(144), 10020), 45.0);
        assert_eq!(rate(true, false, Some(144), 10020), 45.0);
    }

    #[test]
    fn host_announces_its_rate_to_joined_clients_again_when_it_changes() {
        let mut vm = vm_with(true, false, 3, 2, 10000);
        vm.engine.client.display_hz = Some(144);
        // Graphics: Original: nothing announced.
        enhanced(false, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![None, None, None]);
        // The two joined clients only.
        enhanced(true, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![Some(144), Some(144), None]);
        // The window moved to a 240 Hz screen.
        vm.engine.client.display_hz = Some(240);
        enhanced(true, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![Some(240), Some(240), None]);
        // Six joined clients: 960 / 6 = 160 each.
        let d = vm.ipdrv.net_driver.as_mut().unwrap();
        d.conns[2].pc = Some(2000);
        for i in 0..3 {
            let mut c = conn(10000);
            c.pc = Some(3000 + i);
            d.conns.push(c);
        }
        enhanced(true, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![Some(160); 6]);
        // One leaves: 960 / 5 = 192.
        vm.ipdrv.net_driver.as_mut().unwrap().conns.pop();
        enhanced(true, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![Some(192); 5]);
        // Not a LAN game: nothing.
        let mut vm = vm_with(false, false, 1, 1, 5000);
        vm.engine.client.display_hz = Some(144);
        enhanced(true, || vm.net_host_rate_tick());
        assert_eq!(sent_rates(&vm), vec![None]);
    }

    #[test]
    fn client_reads_the_announced_rate() {
        let mut vm = vm_with(true, true, 0, 0, 10000);
        assert!(!vm.receive_host_rate("OXIIIHOSTRATE2 R=144"));
        assert!(!vm.receive_host_rate("USERFLAG 1"));
        assert_eq!(client_rate(&vm), None);
        // Malformed: taken as the command, nothing kept.
        assert!(vm.receive_host_rate("oxiiihostrate"));
        assert!(vm.receive_host_rate("OXIIIHOSTRATE R=0"));
        assert_eq!(client_rate(&vm), None);
        assert!(vm.receive_host_rate("OXIIIHOSTRATE R=144"));
        assert_eq!(client_rate(&vm), Some(144));
        assert!(vm.receive_host_rate("OXIIIHOSTRATE R=120"));
        assert_eq!(client_rate(&vm), Some(120));
    }

    /// Function `name` of a new class `class` on the VM's call stack, FMax called by it.
    fn call_from(vm: &mut Vm, class: &str, name: &str) {
        use crate::types::{ClassDef, FunctionDef};
        let owner = vm.classes.len();
        vm.classes.push(ClassDef { name: class.into(), package: 0, export: 0, super_name: None, super_class: None, props: Vec::new(), prop_index: Default::default(), functions: Default::default(), states: Default::default(), cdo: 0, linked: true, flags: 0, config_name: String::new(), probe_mask: 0 });
        let function = |name: &str| FunctionDef { name: name.into(), owner, state: None, package: 0, export: 0, flags: 0, native: 0, params: Vec::new(), locals: Vec::new(), local_index: Default::default(), ret: None, code: None };
        vm.functions.push(function(name));
        vm.stack.push(vm.functions.len() - 1);
        vm.functions.push(function("FMax"));
        vm.stack.push(vm.functions.len() - 1);
    }

    #[test]
    fn replicate_move_floor_is_lowered_only_there() {
        let floor = |on: bool, lan: bool, client: bool, class: &str, name: &str, b: f32| {
            let mut vm = vm_with(lan, client, 1, 1, 10000);
            call_from(&mut vm, class, name);
            enhanced(on, || vm.replicate_move_floor(b))
        };
        let lowered = 1.0 / LAN_RATE_MAX as f32;
        assert_eq!(floor(true, true, true, "PlayerController", "ReplicateMove", REPLICATE_MOVE_FLOOR), lowered);
        assert_eq!(floor(true, true, true, "playercontroller", "replicatemove", 0.011), lowered);
        // Unchanged (negative controls): another value, another function or class, a host, the
        // Internet, Graphics: Original.
        assert_eq!(floor(true, true, true, "PlayerController", "ReplicateMove", 0.012), 0.012);
        assert_eq!(floor(true, true, true, "PlayerController", "PlayerMove", 0.011), 0.011);
        assert_eq!(floor(true, true, true, "Fragment", "ReplicateMove", 0.011), 0.011);
        assert_eq!(floor(true, true, false, "PlayerController", "ReplicateMove", 0.011), 0.011);
        assert_eq!(floor(true, false, true, "PlayerController", "ReplicateMove", 0.011), 0.011);
        assert_eq!(floor(false, true, true, "PlayerController", "ReplicateMove", 0.011), 0.011);
        // Through the FMax native: NetMoveDelta = FMax(64 / 10000, 0.011).
        let mut vm = vm_with(true, true, 0, 0, 10000);
        call_from(&mut vm, "PlayerController", "ReplicateMove");
        let fmax = |vm: &mut Vm, a: f32, b: f32| {
            let mut args = [Value::Float(a), Value::Float(b)];
            crate::natives::call(vm, "FMax", 0, &mut args, &[true, true]).unwrap().as_float()
        };
        assert_eq!(enhanced(true, || fmax(&mut vm, 64.0 / 10000.0, 0.011)), 64.0 / 10000.0);
        assert_eq!(enhanced(true, || fmax(&mut vm, 0.001, 0.011)), lowered);
        assert_eq!(enhanced(true, || fmax(&mut vm, 0.02, 0.011)), 0.02);
        assert_eq!(enhanced(false, || fmax(&mut vm, 64.0 / 10000.0, 0.011)), 0.011);
    }

    #[test]
    fn enhanced_lan_client_sends_at_its_raised_rate_once_in_a_level() {
        let speed = |on: bool, lan: bool, pending: bool| {
            let mut vm = vm_with(lan, true, 0, 0, 10000);
            if pending {
                vm.ipdrv.pending = Some(Default::default());
            }
            enhanced(on, || vm.net_update_speed_for_lag(None));
            vm.ipdrv.net_driver.as_ref().unwrap().server_conn.as_ref().unwrap().conn.current_net_speed
        };
        assert_eq!(speed(true, true, false), ENHANCED_LAN_CLIENT_RATE);
        // Unchanged (negative controls): still connecting (its NETSPEED not yet sent), Graphics:
        // Original, the Internet.
        assert_eq!(speed(true, true, true), 10000);
        assert_eq!(speed(false, true, false), 10000);
        assert!(speed(true, false, false) <= ENHANCED_CLIENT_INTERNET_MAX);
    }
}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn recognizes_aim_updates_without_consuming_other_control_messages() {
        let mut vm = Vm::new();
        assert!(!vm.receive_aim_update(0, "HELLO MINVER=600"));
        assert!(!vm.receive_aim_update(0, "OXIIIAUTOAIM2 V=1"));
        assert!(vm.receive_aim_update(0, "oxiiiautoaim"));
        assert!(vm.receive_aim_update(0, "OXIIIAUTOAIM V=1"));
    }
}
