//! Network game in the VM: the TcpNetDriver of the listening level (client connections,
//! ULevel::NotifyReceivedText) and, on the client side, UNetPendingLevel (connection, HELLO, CHALLENGE,
//! LOGIN, WELCOME) then JOIN once the map is loaded.
//!
//! ULevel::Tick: TickDispatch at the start (reception, UNetDriver::TickDispatch: Time +=
//! DeltaSeconds, closed connections destroyed), TickFlush at the end (UNetConnection::Tick of
//! each connection: timeouts, resends, keep-alive, sending).
//! Not reimplemented: package downloading (SKIP, DLMGR, FAILCODE).

use crate::net::{ConnState, Connection, NetEvent, CHTYPE_ACTOR, CHTYPE_CONTROL, CHTYPE_FILE};
use crate::net_map::{PackageInfo, PackageMap};
use crate::value::{ObjId, Value};
use crate::Vm;
use std::net::{Ipv4Addr, SocketAddrV4};

/// Not in the original: LOGIN option that carries the client's aim assist (login_url_with_aim).
const AIM_OPTION: &str = "OXIIIAutoAim";
/// Network version (MINVER 600, VER 829).
pub const ENGINE_MIN_NET_VERSION: i32 = 600;
pub const ENGINE_NEGOTIATION_VERSION: i32 = 0x33D;

crate::improved::network::with_edition_fields! {
/// A client's connection on the host, or the client's connection to the host.
#[derive(Debug)]
pub struct NetConn {
    pub conn: Connection,
    /// Name of the UTcpipConnection object (TcpipConnectionN), for "NetComeGo: Close".
    pub name: String,
    /// Player object of the connection (UNetConnection is a UPlayer) and its controller.
    pub player: Option<ObjId>,
    pub pc: Option<ObjId>,
    pub request_url: String,
    pub package_map: PackageMap,
    pub user_flags: i32,
    /// Index of the computed package map (replication).
    pub index: Option<std::rc::Rc<crate::net_map::MapIndex>>,
    /// FClassNetCache of each class for this map.
    pub class_caches: std::collections::HashMap<crate::value::ClassId, std::rc::Rc<crate::net_rep::ClassNetCache>>,
    /// Actor channels (by ChIndex) and ActorChannels (actor → channel).
    pub actor_channels: std::collections::HashMap<usize, crate::net_actor::ActorChannel>,
    pub actor_map: std::collections::HashMap<ObjId, usize>,
    /// TickCount (+0x100), LastRepTime (+0xF8), SentTemporaries (+0x3EA4).
    pub tick_count: u32,
    pub last_rep_time: f64,
    pub sent_temporaries: Vec<ObjId>,
    /// Insertion rank of the next actor channel (order of the pairs of ActorChannels).
    pub actor_seq: u64,
    /// ResolveInfo (+0x3EEC): host name being resolved and its result.
    pub resolve: Option<(String, crate::ipdrv::ResolveResult)>,
    /// Packages sent by the server (UFileChannels opened by this client) and package received by the client (net_file.rs).
    pub sends: Vec<crate::net_file::SendFile>,
    pub download: Option<crate::net_file::Download>,
}
}

impl NetConn {
    pub fn new(conn: Connection) -> Self {
        crate::improved::network::with_edition_fields!(NetConn {
            conn,
            name: String::new(),
            player: None,
            pc: None,
            request_url: String::new(),
            package_map: PackageMap::default(),
            user_flags: 0,
            index: None,
            class_caches: Default::default(),
            actor_channels: Default::default(),
            actor_map: Default::default(),
            tick_count: 0,
            last_rep_time: 0.0,
            sent_temporaries: Vec::new(),
            actor_seq: 0,
            resolve: None,
            sends: Vec::new(),
            download: None,
        })
    }
}

/// UNetPendingLevel: connection in progress to a host.
#[derive(Debug, Default)]
pub struct PendingLevel {
    pub url: String,
    pub level: String,
    pub lone: bool,
    /// bSuccess (+0x8C), bSentJoin (+0x90), FilesNeeded (+0x98), ConnectionError (+0x9C).
    pub success: bool,
    pub sent_join: bool,
    pub files_needed: i32,
    pub error: String,
}

/// appTimestamp: _strdate then _strtime ("MM/DD/YY HH:MM:SS").
fn app_timestamp() -> String {
    chrono::Local::now().format("%m/%d/%y %H:%M:%S").to_string()
}

/// UGameEngine::ChallengeResponse.
pub fn challenge_response(c: i32) -> i32 {
    (c >> 16) ^ c.wrapping_mul(0xED) ^ (c << 16) ^ 0x93FE_92CEu32 as i32
}

/// Core's ParseCommand: see console.rs (the word, case-insensitive, followed by something other than a letter or a digit; blanks and tabs skipped).
fn parse_command<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    crate::console::parse_command(s, word)
}

/// Parse(Stream, "KEY=", Value): value up to the space (or quotes).
fn parse_value(s: &str, key: &str) -> Option<String> {
    // Core's Parse(Stream, Match, FString&): appStrfind (start of word), value up to the
    // closing quote or the blank / comma.
    crate::value::parse_fstring(s, key)
}

/// Core's Parse(Stream, Match, int&).
pub(crate) fn parse_int(s: &str, key: &str) -> Option<i32> {
    crate::value::parse_int(s, key)
}

/// UNetDriver::Exec SOCKETS: "Connections:", "Server D" and its channels "Channel N: D" if the driver has a connection to the server, then "Client D" and
/// the channels of each client connection.
pub(crate) fn sockets_lines(server: Option<&(String, Vec<(usize, String)>)>, clients: &[(String, Vec<(usize, String)>)]) -> Vec<String> {
    let mut lines = vec!["Connections:".to_string()];
    let mut adds = |label: &str, (d, channels_list): &(String, Vec<(usize, String)>)| {
        lines.push(format!("   {label} {d}"));
        for (ch, t) in channels_list {
            lines.push(format!("      Channel {ch}: {t}"));
        }
    };
    if let Some(s) = server {
        adds("Server", s);
    }
    for c in clients {
        adds("Client", c);
    }
    lines
}

/// UNetDriver::Exec KICKPLAYER: without a client connection the command is not taken; otherwise it always is, and the first connection whose controller
/// (name, player name) has a name contained in the text receives "YOUAREKICKED player name" before being closed.
pub(crate) fn kick_player(rest: &str, conns: &[Option<(String, String)>]) -> (bool, Option<(usize, String)>) {
    if conns.is_empty() {
        return (false, None);
    }
    for (i, c) in conns.iter().enumerate() {
        if let Some((pc_name, player_name)) = c {
            if rest.contains(pc_name.as_str()) {
                return (true, Some((i, format!("YOUAREKICKED {player_name}"))));
            }
        }
    }
    (true, None)
}

/// UNetConnection::Exec: GETPING → Ar->Logf(" %i", ftol(BestLag × 1000)), GETLOSS → Ar->Logf(" %i", ftol(InLoss)) (leading space);
/// None: the command goes to UPlayer::Exec.
pub(crate) fn net_connection_command(command: &str, best_lag: f32, in_loss: f32) -> Option<String> {
    if parse_command(command, "GETPING").is_some() {
        return Some(format!(" {}", (best_lag * 1000.0) as i32));
    }
    if parse_command(command, "GETLOSS").is_some() {
        return Some(format!(" {}", in_loss as i32));
    }
    None
}

impl Vm {
    pub(crate) fn net_time(&self) -> f64 {
        self.ipdrv.net_driver.as_ref().map_or(0.0, |d| d.time)
    }

    /// Sending of a text on the control channel (Connection->Logf) and of the ready packets.
    pub(crate) fn net_send(&mut self, which: Option<usize>, text: &str) {
        if let Some(log) = self.net_conn(which).and_then(|c| c.conn.send_text(text)) {
            self.log.push(log);
        }
    }

    fn net_flush(&mut self, which: Option<usize>) {
        if let Some(c) = self.net_conn(which) {
            c.conn.flush_net();
        }
    }

    /// Connection `which` (None: the client's connection to the host).
    pub(crate) fn net_conn(&mut self, which: Option<usize>) -> Option<&mut NetConn> {
        let d = self.ipdrv.net_driver.as_mut()?;
        match which {
            Some(i) => d.conns.get_mut(i),
            None => d.server_conn.as_mut(),
        }
    }

    /// LowLevelSend of the ready packets of each connection. UTcpipConnection::LowLevelSend
    /// while the host's name is being resolved, the packet is dropped; once resolved,
    /// "Resolved %s (%s)" and RemoteAddr; on failure, the error is logged (NAME_Log), the
    /// server connection is closed and the packet dropped.
    pub(crate) fn net_send_outgoing(&mut self) {
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        let mut packets: Vec<(SocketAddrV4, Vec<u8>)> = Vec::new();
        let mut logs = Vec::new();
        let mut close_server = false;
        for c in d.conns.iter_mut().chain(d.server_conn.as_mut()) {
            for p in std::mem::take(&mut c.conn.outgoing) {
                if let Some((host, r)) = c.resolve.take() {
                    let done = r.lock().ok().and_then(|mut o| o.take());
                    match done {
                        None => {
                            c.resolve = Some((host, r));
                            continue;
                        }
                        Some(Err(e)) => {
                            logs.push(format!("Log: {e}"));
                            close_server = true;
                            continue;
                        }
                        Some(Ok(a)) => {
                            let ip = Ipv4Addr::from(a);
                            c.conn.remote = SocketAddrV4::new(ip, c.conn.remote.port());
                            logs.push(format!("Log: Resolved {host} ({})", crate::ipdrv::ip_string(a, 0)));
                        }
                    }
                }
                packets.push((c.conn.remote, p));
            }
        }
        if close_server {
            if let Some(c) = d.server_conn.as_mut() {
                c.conn.state = ConnState::Closed;
            }
        }
        self.log.extend(logs);
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        if let Some(q) = crate::ipdrv::run_loop() {
            let from = SocketAddrV4::new(Ipv4Addr::LOCALHOST, d.port);
            if let Ok(mut q) = q.lock() {
                q.extend(packets.into_iter().map(|(to, p)| (from, to, p)));
            }
            return;
        }
        for (to, p) in packets {
            let _ = d.socket.send_to(&p, to);
        }
    }

    /// UTcpNetDriver::TickDispatch then UNetDriver::TickDispatch.
    pub(crate) fn net_tick_dispatch(&mut self, dt: f32) {
        self.net_drain_queued();
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        // Time += DeltaTime: exact sum rounded to 24 bits by the game's FPU (fadd qword, then fstp), exact at 53 bits.
        d.time = if crate::net::fpu53() { d.time + dt as f64 } else { (d.time + dt as f64) as f32 as f64 };
        // UNetDriver::TickDispatch: QueuedBytes -= rate × DeltaTime, at least
        // −2 × rate × DeltaTime.
        let now = d.time;
        for c in d.conns.iter_mut().chain(d.server_conn.as_mut()) {
            c.conn.now = now;
        }
        let rate = d.server_conn.as_ref().map_or(d.server_speed, |c| c.conn.current_net_speed) as f32 * dt;
        d.queued_bytes -= rate as i32;
        if (d.queued_bytes as f32) < -(rate + rate) {
            d.queued_bytes = (-(rate + rate)) as i32;
        }
        // Closed client connections: destroyed, from last to first, on the host only (UNetDriver::TickDispatch).
        let closed: Vec<usize> = if d.server_conn.is_some() { Vec::new() } else { (0..d.conns.len()).rev().filter(|&i| d.conns[i].conn.state == ConnState::Closed).collect() };
        for i in closed {
            self.net_connection_destroy(Some(i));
        }
        if let Some(q) = crate::ipdrv::run_loop() {
            let port = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.port);
            let mine: Vec<(SocketAddrV4, Vec<u8>)> = match q.lock() {
                Ok(mut q) => {
                    let (m, rest): (Vec<_>, Vec<_>) = q.drain(..).partition(|(_, to, _)| to.port() == port);
                    *q = rest;
                    m.into_iter().map(|(f, _, p)| (f, p)).collect()
                }
                Err(_) => Vec::new(),
            };
            for (from, packet) in mine {
                self.net_received(from, &packet);
            }
            return;
        }
        loop {
            let mut buf = [0u8; 0x240];
            let Some((n, err, from)) = self.ipdrv.net_driver.as_ref().map(|d| d.socket.recv_wsa(&mut buf)) else { break };
            if n == -1 {
                if err == 0x2733 {
                    break;
                }
                if err != 0x2746 {
                    // First error only.
                    let Some(d) = self.ipdrv.net_driver.as_mut() else { break };
                    if !d.recv_error_logged {
                        d.recv_error_logged = true;
                        self.log.push(format!("Log: UDP recvfrom error: {err} from {}:{}", from.ip(), from.port()));
                    }
                    break;
                }
                // WSAECONNRESET: "port unreachable" sent back by a client that has left.
                let Some(d) = self.ipdrv.net_driver.as_ref() else { break };
                if d.server_conn.as_ref().is_some_and(|c| c.conn.remote == from) {
                    continue;
                }
                match d.conns.iter().position(|c| c.conn.remote == from) {
                    Some(i) => {
                        self.log.push(format!("Log: Received ICMP port unreachable from client {}:{}.  Disconnecting.", from.ip(), from.port()));
                        self.net_connection_destroy(Some(i));
                    }
                    None => self.log.push(format!("Log: Received ICMP port unreachable from {}:{}.  No matching connection found.", from.ip(), from.port())),
                }
                continue;
            }
            let packet = buf[..n as usize].to_vec();
            self.net_received(from, &packet);
        }
    }

    /// ULevel::NotifyAcceptingConnection: 0 on a client ("Client refused"), 2 if the level has a NextURL ("Server MyLevel refused"), 1 otherwise
    /// ("Server MyLevel accept"). GetName of the level: MyLevel.
    pub(crate) fn level_accepting_connection(&mut self, is_client: bool) -> u8 {
        if is_client {
            self.log.push("DevNet: NotifyAcceptingConnection: Client refused".into());
            return 0;
        }
        let level = "MyLevel";
        let next_url = self.level.map(|l| self.prop(l, "NextURL").as_name()).unwrap_or_default();
        if !next_url.is_empty() {
            self.log.push(format!("DevNet: NotifyAcceptingConnection: Server {level} refused"));
            return 2;
        }
        self.log.push(format!("DevNet: NotifyAcceptingConnection: Server {level} accept"));
        1
    }

    /// ULevel::NotifyAcceptedConnection: "Open Level Date Address" (NAME 0x313).
    pub(crate) fn level_accepted_connection(&mut self, from: SocketAddrV4) {
        self.log.push(format!("NetComeGo: Open MyLevel {} {from}", app_timestamp()));
    }

    /// A datagram received by the driver: known connection, or a new one if the level accepts it.
    fn net_received(&mut self, from: SocketAddrV4, packet: &[u8]) {
        let time = self.net_time();
        let is_client = self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_some());
        if is_client {
            if self.ipdrv.net_driver.as_ref().and_then(|d| d.server_conn.as_ref()).is_some_and(|c| c.conn.remote == from) {
                self.net_receive_packet(None, packet);
            } else if self.ipdrv.pending.is_none() {
                // Unknown address: Notify->NotifyAcceptingConnection (TickDispatch);
                // ULevel refuses and logs it, UNetPendingLevel refuses without writing
                // anything.
                self.level_accepting_connection(true);
            }
            return;
        }
        let index = self.ipdrv.net_driver.as_ref().and_then(|d| d.conns.iter().position(|c| c.conn.remote == from));
        let index = match index {
            Some(i) => i,
            None => {
                // More than 5 connections from this IP created within the minute (appSeconds): packet
                // ignored.
                let app = self.app_seconds;
                let recent = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.iter().filter(|c| c.conn.remote.ip() == from.ip() && app - 60.0 < c.conn.connect_time).count());
                if recent >= 5 {
                    self.log.push(format!("Log: More than 5 logins per minute for {}.  Ignoring.", from.ip()));
                    return;
                }
                if self.level_accepting_connection(false) != 1 {
                    return;
                }
                let speed = self.net_speed_config();
                let mut conn = Connection::new(from, ConnState::Open, time, speed);
                conn.negotiated_ver = ENGINE_MIN_NET_VERSION;
                conn.connect_time = self.app_seconds;
                self.net_conn_settings(&mut conn);
                self.level_accepted_connection(from);
                let speed = conn.current_net_speed;
                let mut nc = NetConn::new(conn);
                nc.name = self.net_next_connection_name();
                let d = self.ipdrv.net_driver.as_mut().unwrap();
                d.conns.push(nc);
                // UTcpNetDriver::TickDispatch (IpDrv 0x107086xx): the host's rate starts from that
                // of the first connection.
                if d.conns.len() == 1 {
                    d.server_speed = speed;
                }
                d.conns.len() - 1
            }
        };
        self.net_receive_packet(Some(index), packet);
    }

    /// Driver settings for a new connection (KeepAliveTime, timeouts, client).
    fn net_conn_settings(&mut self, conn: &mut Connection) {
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return };
        conn.on_client = d.server_conn.is_some();
        conn.keep_alive_time = d.cfg.keep_alive_time;
        conn.connection_timeout = d.cfg.connection_timeout;
        conn.initial_connect_timeout = d.cfg.initial_connect_timeout;
        // Deliberate difference, only with OXIII_CONNECTION_DELAY=seconds (LAN launchers): a slow PC
        // that is loading the map does not answer during that time; beyond InitialConnectTimeout (30 s
        // in XIII.ini) the server disconnects it before it has joined.
        if let Some(s) = openxiii_package::env::var("OXIII_CONNECTION_DELAY").ok().and_then(|v| v.trim().parse::<f32>().ok()) {
            conn.initial_connect_timeout = conn.initial_connect_timeout.max(s);
        }
    }

    /// UNetConnection::ReceivedRawPacket / ReceivedPacket of a packet of
    /// connection `which`: what each ack or bunch asks of the engine is done in its turn.
    fn net_receive_packet(&mut self, which: Option<usize>, packet: &[u8]) {
        let now = self.net_time();
        let Some(mut cur) = self.net_conn(which).and_then(|c| {
            c.conn.now = now;
            c.conn.begin_packet(packet)
        }) else {
            return;
        };
        loop {
            let Some(ev) = self.net_conn(which).and_then(|c| c.conn.next_event(&mut cur)) else { break };
            match ev {
                NetEvent::Accept { ch, ch_type } => {
                    let ok = self.net_notify_accepting_channel(which, ch, ch_type);
                    if let Some(c) = self.net_conn(which) {
                        c.conn.accept_channel(&mut cur, ok);
                    }
                }
                NetEvent::Text(t) => match which {
                    Some(i) => self.net_server_text(i, &t),
                    None => self.net_client_text(&t),
                },
                NetEvent::Bunch(mut b) => {
                    let file_value = self.net_conn_ref(which).and_then(|c| c.conn.channels.get(b.ch_index).and_then(|x| x.as_ref())).is_some_and(|x| x.ch_type == CHTYPE_FILE);
                    if file_value {
                        let ch = b.ch_index;
                        self.net_file_bunch(which, ch, &mut b.data);
                    } else {
                        self.net_received_actor_bunch(which, b);
                    }
                }
                NetEvent::Nak { ch, packet_id } => self.net_actor_channel_nak(which, ch, packet_id),
                NetEvent::Destroyed { ch, ch_type, open_acked } => {
                    if ch_type == CHTYPE_ACTOR {
                        self.net_actor_channel_destroyed(which, ch, open_acked);
                    } else if ch_type == CHTYPE_FILE {
                        self.net_file_closes(which, ch);
                    }
                }
                NetEvent::Log(s) => self.log.push(s),
            }
        }
    }

    /// FNetworkNotify::NotifyAcceptingChannel: on the pending client, none;
    /// ULevel::NotifyAcceptingChannel: the client takes the actor channels, the host
    /// the control channel (channel 0) and the files; logs with the level's GetFullName ("Level
    /// Map.MyLevel").
    pub(crate) fn net_notify_accepting_channel(&mut self, which: Option<usize>, ch: usize, t: u32) -> bool {
        if which.is_none() {
            if self.ipdrv.pending.is_some() {
                return false;
            }
            if t != CHTYPE_ACTOR {
                self.log.push(format!("DevNet: Client refusing unwanted channel of type {t}"));
                return false;
            }
            return true;
        }
        let level = format!("Level {}.MyLevel", self.map_package.map(|p| self.packages[p].name.clone()).unwrap_or_default());
        match t {
            CHTYPE_CONTROL if ch == 0 => {
                self.log.push(format!("DevNet: NotifyAcceptingChannel Control {ch} server {level}: Accepted"));
                true
            }
            CHTYPE_FILE => {
                self.log.push(format!("DevNet: NotifyAcceptingChannel File {ch} server {level}: Accepted"));
                true
            }
            _ => {
                self.log.push(format!("DevNet: NotifyAcceptingChannel {t} {ch} server {level}: Refused"));
                false
            }
        }
    }

    /// Configured rate of a new connection (speed_configured) depending on the driver's bLAN.
    fn net_speed_config(&mut self) -> i32 {
        let lan = self.ipdrv.net_driver.as_ref().is_some_and(|d| d.lan);
        crate::net::speed_configured(lan, self.player_default_int("ConfiguredLanSpeed"), self.player_default_int("ConfiguredInternetSpeed"))
    }

    /// Integer property of UPlayer's default object: values from Engine.u then
    /// LoadConfig of the class (config=User: [Engine.Player] of User.ini), as NETSPEED and
    /// LANSPEED change them.
    pub(crate) fn player_default_int(&mut self, key: &str) -> i32 {
        let Some(c) = self.class("Player") else {
            // Without Engine.u (tests): the key of [Engine.Player] of the user file, imported as an int.
            let t: Vec<char> = self.config.as_mut().and_then(|c| c.get_string("Engine.Player", key, "User")).unwrap_or_default().chars().collect();
            return if t.first().is_some_and(|&c| c == '-' || c.is_ascii_digit()) { crate::config::atoi(&t, 0) } else { 0 };
        };
        self.create_cdo(c);
        let cdo = self.classes[c].cdo;
        if cdo == usize::MAX {
            return 0;
        }
        self.propr(cdo, key).as_int()
    }

    /// ULevel::NotifyReceivedText on the host.
    fn net_server_text(&mut self, i: usize, text: &str) {
        // ParseCommand skips leading blanks even when the word does not match: the log and what follows see the text without them.
        let text = text.trim_start_matches([' ', '\t']);
        if let Some(rest) = parse_command(text, "USERFLAG") {
            let v = crate::config::atoi(&rest.chars().collect::<Vec<_>>(), 0);
            if let Some(c) = self.net_conn(Some(i)) {
                c.user_flags = v;
            }
            return;
        }
        self.log.push(format!("DevNet: Level server received: {text}"));
        if self.receive_aim_update(i, text) {
            return;
        }
        if let Some(rest) = parse_command(text, "HELLO") {
            let min_ver = parse_int(rest, "MINVER=").unwrap_or(0xDB);
            let ver = parse_int(rest, "VER=").unwrap_or(0xDB);
            if ver >= ENGINE_MIN_NET_VERSION && min_ver <= ENGINE_NEGOTIATION_VERSION {
                let negotiated = ver.min(ENGINE_NEGOTIATION_VERSION);
                // Challenge = low word of rdtsc.
                let challenge = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as i32);
                let stats = self.level.and_then(|l| self.po(l, "Game")).is_some_and(|g| self.pb(g, "bWorldLog")) as i32;
                if let Some(c) = self.net_conn(Some(i)) {
                    c.conn.negotiated_ver = negotiated;
                    c.conn.challenge = challenge;
                }
                self.net_send(Some(i), &format!("CHALLENGE VER={negotiated} CHALLENGE={challenge} STATS={stats}"));
                self.net_flush(Some(i));
                return;
            }
            self.net_send(Some(i), &format!("UPGRADE MINVER={ENGINE_MIN_NET_VERSION} VER={ENGINE_NEGOTIATION_VERSION}"));
            self.net_flush(Some(i));
            if let Some(c) = self.net_conn(Some(i)) {
                c.conn.state = ConnState::Closed;
            }
        } else if let Some(rest) = parse_command(text, "NETSPEED") {
            let rate = crate::config::atoi(&rest.chars().collect::<Vec<_>>(), 0);
            let max = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.cfg.max_client_rate);
            let speed = if let Some(c) = self.net_conn(Some(i)) {
                if rate >= 500 {
                    c.conn.current_net_speed = rate.min(max);
                }
                c.conn.current_net_speed
            } else {
                return;
            };
            self.log.push(format!("Log: Client netspeed is {speed}"));
        } else if let Some(rest) = parse_command(text, "HAVE") {
            // HAVE GUID= GEN=: RemoteGeneration of each package with this GUID (Parse leaves the
            // value without GEN=), which bounds the indices at the JOIN's Compute.
            let Some(guid) = parse_value(rest, "GUID=").and_then(|g| crate::net_map::parse_guid(&g)) else { return };
            let generation = parse_int(rest, "GEN=");
            if let Some(c) = self.net_conn(Some(i)) {
                for p in c.package_map.list.iter_mut().filter(|p| p.guid == guid) {
                    if let Some(g) = generation.filter(|&g| g >= 0) {
                        p.remote_generation = g;
                    }
                }
            }
        } else if let Some(rest) = parse_command(text, "SKIP") {
            // SKIP GUID=: the client gives up downloading this package; the first one in the map with this GUID is removed.
            let Some(guid) = parse_value(rest, "GUID=").and_then(|g| crate::net_map::parse_guid(&g)) else { return };
            let removes = self.net_conn(Some(i)).and_then(|c| {
                let k = c.package_map.list.iter().position(|p| p.guid == guid)?;
                Some(c.package_map.list.remove(k).name)
            });
            if let Some(name) = removes {
                self.log.push(format!("Log: User skipped download of '{name}'"));
            }
        } else if let Some(rest) = parse_command(text, "LOGIN") {
            let response = parse_int(rest, "RESPONSE=");
            let (challenge, remote) = match self.net_conn(Some(i)) {
                Some(c) => (c.conn.challenge, c.conn.remote),
                None => return,
            };
            if response != Some(challenge_response(challenge)) {
                self.log.push(format!("DevNet: Client {remote} failed CHALLENGE."));
                self.net_send(Some(i), "FAILURE CHALLENGE");
                self.net_flush(Some(i));
                if let Some(c) = self.net_conn(Some(i)) {
                    c.conn.state = ConnState::Closed;
                }
                return;
            }
            // Connection+0x104: FlushNet then sends the host's KeepAlives.
            if let Some(c) = self.net_conn(Some(i)) {
                c.conn.logged_in = true;
            }
            let url = parse_value(rest, "URL=").unwrap_or_default();
            self.log.push(format!("DevNet: Login request: {url}"));
            if let Some(c) = self.net_conn(Some(i)) {
                c.request_url = url.clone();
            }
            // Mods build only: a client whose mods differ from the server's is refused (mod_net.rs).
            #[cfg(feature = "mods")]
            if let Some(error) = self.mod_login_refusal(&url) {
                self.log.push(format!("DevNet: PreLogin failure: {error} (mods)"));
                self.net_send(Some(i), &format!("FAILURE {error}"));
                self.net_flush(Some(i));
                if let Some(c) = self.net_conn(Some(i)) {
                    c.conn.state = ConnState::Closed;
                }
                return;
            }
            let options = url.find('?').map_or(String::new(), |p| url[p..].to_string());
            let (error, fail_code) = self.net_pre_login(&options, &remote.to_string());
            if error.is_empty() {
                self.welcome_player(i, "");
            } else {
                self.log.push(format!("DevNet: PreLogin failure: {error} ({fail_code})"));
                self.net_send(Some(i), &format!("FAILURE {error}"));
                if !fail_code.is_empty() {
                    self.net_send(Some(i), &format!("FAILCODE {fail_code}"));
                }
                self.net_flush(Some(i));
                if let Some(c) = self.net_conn(Some(i)) {
                    c.conn.state = ConnState::Closed;
                }
            }
        } else if parse_command(text, "JOIN").is_some() {
            if self.net_conn(Some(i)).is_none_or(|c| c.pc.is_some()) {
                return;
            }
            if let Some(c) = self.net_conn(Some(i)) {
                c.package_map.compute();
            }
            let url = self.net_conn(Some(i)).map(|c| c.request_url.clone()).unwrap_or_default();
            self.log.push(format!("DevNet: Join request: {url}"));
            match self.net_spawn_play_actor(i, &url) {
                Ok(pc) => {
                    let name = self.po(pc, "PlayerReplicationInfo").map(|p| self.prop(p, "PlayerName").as_name()).unwrap_or_default();
                    self.log.push(format!("DevNet: Join succeeded: {name}"));
                }
                Err(error) => {
                    self.log.push(format!("DevNet: Join failure: {error}"));
                    self.net_send(Some(i), &format!("FAILURE {error}"));
                    self.net_flush(Some(i));
                    if let Some(c) = self.net_conn(Some(i)) {
                        c.conn.state = ConnState::Closed;
                    }
                }
            }
        }
    }

    /// AGameInfo::eventPreLogin(Options, Address, Error, FailCode).
    fn net_pre_login(&mut self, options: &str, address: &str) -> (String, String) {
        let Some(game) = self.level.and_then(|l| self.po(l, "Game")) else { return (String::new(), String::new()) };
        let Some(f) = self.find_function(game, "PreLogin", false) else { return (String::new(), String::new()) };
        let args = vec![Some(Value::Str(options.into())), Some(Value::Str(address.into())), Some(Value::Str(String::new())), Some(Value::Str(String::new()))];
        match self.call(f, game, args) {
            Ok((_, outs)) => {
                let s = |k: usize| outs.get(k).and_then(|v| v.as_ref()).map(|v| v.as_name()).unwrap_or_default();
                (s(2), s(3))
            }
            Err(_) => (String::new(), String::new()),
        }
    }

    /// ULevel::WelcomePlayer: package map, USES, WELCOME.
    pub(crate) fn welcome_player(&mut self, i: usize, extra: &str) {
        let master = self.ipdrv.net_driver.as_ref().map(|d| d.master_map.clone()).unwrap_or_default();
        let lines: Vec<String> = master.list.iter().map(PackageInfo::uses_line).collect();
        let index = self.ipdrv.net_driver.as_ref().and_then(|d| d.master_index.clone());
        // UPackageMap::Copy of the driver's map.
        if let Some(c) = self.net_conn(Some(i)) {
            c.package_map = master;
            c.index = index;
        }
        for l in lines {
            self.net_send(Some(i), &l);
        }
        let level = self.map_package.map(|p| self.packages[p].name.clone()).unwrap_or_default();
        let lone = self.level.is_some_and(|l| self.pb(l, "bLonePlayer")) as i32;
        let text = if extra.is_empty() { format!("WELCOME LEVEL={level} LONE={lone}") } else { format!("WELCOME LEVEL={level} LONE={lone} {extra}") };
        self.net_send(Some(i), &text);
        self.net_flush(Some(i));
    }

    /// ULevel::SpawnPlayActor (Player = the connection, RemoteRole ROLE_AutonomousProxy):
    /// GameInfo.Login, SetPlayer, roles, ShowFlags, RendMap, PostLogin.
    fn net_spawn_play_actor(&mut self, i: usize, url: &str) -> Result<ObjId, String> {
        let game = self.level.and_then(|l| self.po(l, "Game")).ok_or("no game")?;
        let parsed = crate::travel::Url::parse(None, url, crate::travel::TRAVEL_ABSOLUTE);
        let f = self.find_function(game, "Login", false).ok_or("no Login")?;
        let args = vec![Some(Value::Str(parsed.portal.clone())), Some(Value::Str(parsed.options_string())), Some(Value::Str(String::new()))];
        let (pc, outs) = self.call(f, game, args).map_err(|e| e.to_string())?;
        let error = outs.get(2).and_then(|v| v.as_ref()).map(|v| v.as_name()).unwrap_or_default();
        let Some(pc) = pc.as_object() else { return Err(error) };
        // Player object of the connection (UTcpipConnection, Transient).
        // UTcpipConnection: NetConnection (native class without a script, see DynamicCast).
        let player_class = self.class("Player").ok_or("no Player")?;
        let name = self.net_conn(Some(i)).map(|c| c.name.clone()).unwrap_or_default();
        let player = self.spawn_in(player_class, "Transient".into(), Some(name));
        self.set_player(pc, player, false);
        self.set_prop(pc, "Role", Value::Byte(4));
        self.set_prop(pc, "RemoteRole", Value::Byte(3));
        // ULevel::SpawnPlayActor: ShowFlags = 0x30CC80C, RendMap = REN_DynLight.
        self.set_prop(pc, "ShowFlags", Value::Int(0x30cc80c));
        self.set_prop(pc, "RendMap", Value::Int(5));
        if let Some(c) = self.net_conn(Some(i)) {
            c.player = Some(player);
            c.pc = Some(pc);
        }
        self.apply_network_login(i, pc, &parsed);
        if let Some(f) = self.find_function(game, "PostLogin", false) {
            self.call(f, game, vec![Some(Value::Object(Some(pc)))]).map_err(|e| e.to_string())?;
        }
        Ok(pc)
    }

    /// Not in the original: the client's LOGIN URL with its aim assist setting ("OXIIIAutoAim=").
    /// XIIIPlayerController.AdjustAim has the host compute a remote player's aim
    /// (AdjustAimForDisplay when the pawn is not locally controlled), with iAutoAimMode, a
    /// globalconfig setting of the host's machine. In the original all the players of a game are on
    /// the same platform; here an Android host (aim assist on by default) would give it to the PC player
    /// using the mouse, and a PC host would take it away from the Android player. So the client sends the value of its
    /// own controller (otherwise that of its User.ini; improved build: the gamepad's value when the
    /// gamepad is in use, see aim_device.rs); the kept pending URL does not have it. Improved build:
    /// later changes are sent by net_client_aim_tick.
    fn login_url_with_aim(&mut self, url: &crate::travel::Url) -> String {
        let mode = match self.local_aim_controller() {
            Some(p) => self.propr(p, "iAutoAimMode").as_int(),
            None => self.login_pad_aim().unwrap_or_else(|| {
                self.config.as_mut().and_then(|c| c.get_string("Engine.PlayerController", "iAutoAimMode", "User")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
            }),
        };
        let mode = i32::from(mode != 0);
        self.remember_login_aim(mode);
        let mut u = url.clone();
        u.add_option(&format!("{AIM_OPTION}={mode}"));
        self.add_network_login_options(&mut u);
        u.string()
    }

    /// Not in the original: a remote player's controller takes on the host the aim assist that
    /// its client sent at LOGIN (see login_url_with_aim); a client that does not send it
    /// keeps the host's, as in the original.
    pub(crate) fn aim_client(&mut self, pc: ObjId, url: &crate::travel::Url) {
        let key_tuple = format!("{AIM_OPTION}=");
        if let Some(v) = url.get_option(&key_tuple).and_then(|v| v.trim().parse::<i32>().ok()) {
            self.set_prop(pc, "iAutoAimMode", Value::Int(i32::from(v != 0)));
            self.log.push(format!("DevNet: client aim assist: {}", i32::from(v != 0)));
        }
    }

    /// The local player's live controller (viewport 0), if any.
    pub(crate) fn local_aim_controller(&mut self) -> Option<ObjId> {
        let pc = self.find_viewport_object(0).and_then(|v| self.propr(v, "Actor").as_object());
        pc.filter(|&p| self.objects.get(p).is_some_and(Option::is_some))
    }

    /// Name of the next UTcpipConnection object (TcpipConnectionN, when it is created).
    fn net_next_connection_name(&mut self) -> String {
        let n = self.ipdrv.next_connection;
        self.ipdrv.next_connection += 1;
        format!("TcpipConnection{n}")
    }

    /// UNetConnection::Destroy: "NetComeGo: Close Name Date", Close of the control
    /// channel and FlushNet; its channels are destroyed (UActorChannel::Destroy: the client's
    /// actors), the connection leaves the driver, then UPlayer::Destroy: the
    /// controller's pawn, then the controller (except RendMap 0x14 / 0x15).
    pub(crate) fn net_connection_destroy(&mut self, which: Option<usize>) {
        let time = self.net_time();
        let Some(c) = self.net_conn(which) else { return };
        let name = c.name.clone();
        self.log.push(format!("NetComeGo: Close {name} {}", app_timestamp()));
        let Some(c) = self.net_conn(which) else { return };
        c.conn.now = time;
        if c.conn.channels[0].is_some() {
            c.conn.close_channel(0);
        }
        c.conn.flush_net();
        c.conn.state = ConnState::Closed;
        self.net_send_outgoing();
        let chans = self.net_conn(which).map(|c| c.conn.destroy_channels()).unwrap_or_default();
        for (ch, t, open_acked) in chans {
            if t == CHTYPE_ACTOR {
                self.net_actor_channel_destroyed(which, ch, open_acked);
            }
        }
        let Some(d) = self.ipdrv.net_driver.as_mut() else { return };
        let c = match which {
            Some(i) if i < d.conns.len() => d.conns.remove(i),
            None => match d.server_conn.take() {
                Some(c) => c,
                None => return,
            },
            _ => return,
        };
        if let Some(pc) = c.pc.filter(|&p| self.alive(p)) {
            self.set_prop(pc, "Player", Value::Object(None));
            if let Some(pawn) = self.native_object(pc, crate::NativeField::Pawn).filter(|&p| self.alive(p)) {
                let _ = self.destroy_actor(pawn);
            }
            if !matches!(self.propr(pc, "RendMap").as_int(), 0x14 | 0x15) && self.alive(pc) {
                let _ = self.destroy_actor(pc);
            }
        }
    }

    /// UNetDriver::Destroy, at the next LoadMap (UGameEngine::LoadMap):
    /// ServerConnection then each ClientConnection destroyed, then LowLevelDestroy: socket closed, "Exit: WinSock shut down".
    pub fn net_destroy_driver(&mut self) {
        if self.ipdrv.net_driver.is_none() {
            return;
        }
        if self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_some()) {
            self.net_connection_destroy(None);
        }
        while let Some(n) = self.ipdrv.net_driver.as_ref().map(|d| d.conns.len()).filter(|&n| n > 0) {
            self.net_connection_destroy(Some(0));
            if self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len()) == n {
                break;
            }
        }
        if let Some(d) = self.ipdrv.net_driver.take() {
            // Simulated socket: its port is released; a real socket closes with it.
            if d.socket.0.is_none() {
                crate::ipdrv::offline_release(&mut self.ipdrv, d.socket.1);
            }
            drop(d);
            self.log.push("Exit: WinSock shut down".into());
        }
        self.ipdrv.pending = None;
    }

    /// Close requested by the client (UGameEngine::Exec DISCONNECT, EXIT/QUIT): Close of the control
    /// channel of the connection to the server, then FlushNet.
    pub(crate) fn net_close_server_connection(&mut self) {
        let Some(c) = self.net_conn(None) else { return };
        if c.conn.channels[0].is_none() {
            return;
        }
        c.conn.close_channel(0);
        c.conn.flush_net();
        self.net_send_outgoing();
    }

    /// Channel->Describe() of an open channel: UControlChannel::Describe "Text" then UChannel::Describe (
    /// "State=open" or "State=closing"); UActorChannel::Describe "Actor=GetFullName (Role=… RemoteRole=…)" if the channel
    /// is not closing and has an actor, otherwise "Actor=None".
    pub(crate) fn net_describe_channel(&self, c: &NetConn, ch: usize) -> Option<String> {
        let x = c.conn.channels[ch].as_ref()?;
        let state = c.conn.channel_state(ch);
        Some(match x.ch_type {
            CHTYPE_CONTROL => format!("Text {state}"),
            CHTYPE_ACTOR => match c.actor_channels.get(&ch).and_then(|a| a.actor).filter(|_| !x.closing).filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some())) {
                Some(a) => {
                    let class = self.objects[a].as_ref().map_or(String::new(), |o| self.classes[o.class].name.clone());
                    format!("Actor={class} {} (Role={} RemoteRole={}) {state}", self.path_name(a), self.propr(a, "Role").as_byte(), self.propr(a, "RemoteRole").as_byte())
                }
                None => format!("Actor=None {state}"),
            },
            _ => state.to_string(),
        })
    }

    /// UNetDriver::Exec KICKPLAYER: the connection whose controller is named in the
    /// command receives "YOUAREKICKED Name", is closed (FlushNet) and the controller receives
    /// IAmKicked. True if the command is KICKPLAYER.
    pub(crate) fn net_driver_exec(&mut self, command: &str, out: &mut String) -> bool {
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return false };
        // UTcpNetDriver::Exec "ListeningPort": ntohs of the word read at +0xE6 of the
        // driver, which is the high half of CONFListenServerMaxAllowedLag (+0xE4), not the port.
        if parse_command(command, "ListeningPort").is_some() {
            let w = (d.cfg.listen_server.max_allowed_lag.to_bits() >> 16) as u16;
            out.push_str(&w.swap_bytes().to_string());
            return true;
        }
        // UNetDriver::Exec SOCKETS: connections (LowLevelDescribe) and their channels
        // (UControlChannel::Describe, UActorChannel::Describe).
        if parse_command(command, "SOCKETS").is_some() {
            let describe = |vm: &Vm, c: &NetConn| -> Vec<(usize, String)> {
                c.conn.open_channels.iter().filter_map(|&ch| vm.net_describe_channel(c, ch).map(|t| (ch, t))).collect()
            };
            let server = d.server_conn.as_ref().map(|c| (c.conn.low_level_describe(), describe(self, c)));
            let clients: Vec<(String, Vec<(usize, String)>)> = d.conns.iter().map(|c| (c.conn.low_level_describe(), describe(self, c))).collect();
            out.push_str(&sockets_lines(server.as_ref(), &clients).join("\n"));
            return true;
        }
        let Some(rest) = parse_command(command, "KICKPLAYER") else { return false };
        let n = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.conns.len());
        let conns: Vec<Option<(String, String)>> = (0..n)
            .map(|i| {
                let pc = self.net_conn(Some(i)).and_then(|c| c.pc).filter(|&p| self.alive(p))?;
                let pc_name = self.objects[pc].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                let player_name = self.po(pc, "PlayerReplicationInfo").map(|r| self.propr(r, "PlayerName").as_name()).unwrap_or_default();
                Some((pc_name, player_name))
            })
            .collect();
        let (taken, target_value) = kick_player(rest, &conns);
        if let Some((i, message)) = target_value {
            let pc = self.net_conn(Some(i)).and_then(|c| c.pc).unwrap();
            if let Some(c) = self.net_conn(Some(i)) {
                let _ = c.conn.send_text(&message);
                c.conn.state = ConnState::Closed;
                c.conn.flush_net();
            }
            self.net_send_outgoing();
            // APlayerController::eventIAmKicked.
            self.event_pub(pc, "IAmKicked", vec![]);
        }
        taken
    }

    /// UNetConnection::Exec of a remote controller (Player = the connection): GETPING
    /// (BestLag × 1000), GETLOSS (InLoss); None if `player` is not a connection or if the
    /// command goes to UPlayer::Exec.
    pub(crate) fn net_connection_exec(&mut self, player: ObjId, command: &str) -> Option<String> {
        let d = self.ipdrv.net_driver.as_ref()?;
        let c = d.conns.iter().find(|c| c.player == Some(player))?;
        net_connection_command(command, c.conn.best_lag, c.conn.in_loss)
    }

    /// Is controller `pc` the one of a client connection (Player = UNetConnection)?
    pub(crate) fn net_is_remote_player(&self, player: ObjId) -> bool {
        self.ipdrv.net_driver.as_ref().is_some_and(|d| d.conns.iter().any(|c| c.player == Some(player)))
    }

    /// UGameEngine::SetProgress: messages 0 and 1 (red) and duration to the controller of the
    /// first viewport; a duration of −1 opens the HUD's ShowUpgradeMenu.
    pub(crate) fn engine_set_progress(&mut self, s1: &str, s2: &str, seconds: f32) {
        let Some(pc) = self.find_viewport_object(0).and_then(|v| self.propr(v, "Actor").as_object()).filter(|&p| self.alive(p)) else { return };
        if seconds == -1.0 {
            if let Some(hud) = self.po(pc, "myHUD").filter(|&h| self.alive(h)) {
                self.event_pub(hud, "ShowUpgradeMenu", vec![]);
            }
        }
        let red = Value::Struct("object.color".into(), vec![Value::Byte(0), Value::Byte(0), Value::Byte(255), Value::Byte(255)].into());
        // APlayerController::eventSetProgressMessage.
        self.event_pub(pc, "SetProgressMessage", vec![Value::Int(0), Value::Str(s1.to_string()), red.clone()]);
        self.event_pub(pc, "SetProgressMessage", vec![Value::Int(1), Value::Str(s2.to_string()), red]);
        // APlayerController::eventSetProgressTime.
        self.event_pub(pc, "SetProgressTime", vec![Value::Float(seconds)]);
    }

    /// UGameEngine::SetClientTravel: the URL, the travel type and bItems that the engine runs at the end of the frame.
    pub(crate) fn engine_set_client_travel(&mut self, url: String, travel_type: u8, items: bool) {
        self.client_travel = Some((url, travel_type, items));
    }

    /// UGameEngine::ReportNetFailure, type 1: ConnectionStatus 1 and ConnectionError.
    pub(crate) fn engine_report_net_status(&mut self, error: &str) {
        self.ipdrv.connection_status = 1;
        self.ipdrv.connection_error = error.to_string();
    }

    /// UGameEngine::ReportNetFailure, type 0: ConnectionFailure and TimeToWaitUpTo;
    /// UGameEngine::Tick goes back to the menu ("?failed") when appSeconds reaches it.
    pub(crate) fn engine_report_net_failure(&mut self, seconds: f32) {
        if !self.ipdrv.connection_failure {
            self.ipdrv.connection_failure = true;
            // fld appSeconds (double); fadd Seconds (single) at 24-bit precision, rounded only once; fstp dword.
            self.ipdrv.time_to_wait_up_to = crate::natives::x87_24(self.app_seconds + seconds as f64) as f32 as f64;
        }
    }

    /// Connection failure seen by the client (ULevel::TickNetClient, NotifyReceivedText):
    /// SetProgress(ConnectionFailed, "", duration), ReportNetFailure(5, 0, Errors.ConnectionFailed).
    pub(crate) fn net_connection_failed(&mut self, seconds: f32) {
        // Not in the original: the menu will say the host left (enhanced network play).
        if self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_some()) {
            self.net_host_left();
        }
        // Not in the original: enhanced network play joins the server again (net_reconnect.rs).
        if self.net_reconnect_soon() {
            self.engine_report_net_failure(0.5);
            return;
        }
        let msg = self.localize("Errors", "ConnectionFailed", "Engine", false);
        self.engine_set_progress(&msg, "", seconds);
        self.engine_report_net_failure(5.0);
    }

    /// UNetDriver::TickFlush: UNetConnection::Tick of each connection, with
    /// UpdateNetSpeedAccordingToLag and HUD.bBadConnectionAlert at each end of period.
    pub(crate) fn net_tick_flush(&mut self) {
        self.net_client_aim_tick();
        self.net_host_rate_tick();
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return };
        // The ServerConnection first, then the ClientConnections in order (vtable +0x90 of each).
        let list: Vec<Option<usize>> = d.server_conn.iter().map(|_| None).chain((0..d.conns.len()).map(Some)).collect();
        let app = self.app_seconds;
        let prompt_acks = self.net_enhanced();
        let conns = list.len().max(1) as f32;
        for which in list {
            if let Some(c) = self.net_conn(which) {
                c.conn.prompt_acks = prompt_acks;
                // Enhanced multiplayer: packets that only acknowledge are bounded, by a rate
                // shared between this machine's connections and by a small share of the
                // connection's speed (they count in its budget; about 36 bytes each with the
                // packet overhead, so at most a quarter of it: 35 a second at the Internet
                // rate of 5000, measured with 12 a second: lag 92 ms instead of 32).
                let rate = (crate::net_smooth::VERSUS_ACK_RATE / conns).min(c.conn.current_net_speed as f32 / 144.0);
                c.conn.prompt_ack_interval = 1.0 / rate.max(1.0);
            }
            // UFileChannel::Tick of the connection's file channels.
            self.net_files_tick(which);
            let Some(c) = self.net_conn(which) else { continue };
            let pc = c.pc;
            let fatal = std::mem::take(&mut c.conn.fatal);
            let r = c.conn.tick(app, pc.is_some());
            for message in fatal {
                self.log.push(format!("Critical: {message}"));
                self.warn(format!("{message} (fatal error in the original game)"));
            }
            if let Some(log) = r.log {
                // Not in the original: the server connection timed out (a silent loss, not the
                // server closing it): the only loss joined again (net_reconnect.rs).
                if which.is_none() && log.contains("timed out") {
                    self.net_reconnect_timed_out();
                }
                self.log.push(log);
            }
            if r.stats {
                self.net_update_speed_for_lag(which);
            }
            if let (Some(bad), Some(pc)) = (r.bad_connection, pc) {
                if let Some(hud) = self.po(pc, "myHUD").filter(|&h| self.alive(h)) {
                    self.set_prop(hud, "bBadConnectionAlert", Value::Bool(bad));
                }
            }
        }
        self.net_send_outgoing();
    }

    /// UGameEngine::BuildServerMasterMap, after InitGame of a listening level.
    pub(crate) fn build_server_master_map(&mut self) {
        let Some(level_pkg) = self.map_package.map(|p| self.packages[p].name.clone()) else { return };
        let server_packages = self.config.as_mut().and_then(|c| c.multi_find("Engine.GameEngine", "ServerPackages", "System")).unwrap_or_default();
        let game_pkg = self.level.and_then(|l| self.po(l, "Game")).and_then(|g| self.objects[g].as_ref().map(|o| self.packages[self.classes[o.class].package].name.clone()));
        let mut map = PackageMap::default();
        let mut logs = Vec::new();
        {
            let packages = &self.packages;
            let library = &mut self.library;
            // URL of each package: Linker->Filename (the file read), which UFileChannel opens to send it (net_file.rs).
            let (map_file, map_name) = (self.map_file.clone(), level_pkg.clone());
            let mut load = |name: &str| -> Option<(String, String, openxiii_package::Package, usize)> {
                if let Some(p) = packages.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
                    let file_value = if p.name.eq_ignore_ascii_case(&map_name) { map_file.clone() } else { library.as_ref().and_then(|l| l.file_of(name).cloned()) };
                    return Some((p.name.clone(), file_value.map_or(String::new(), |f| f.display().to_string()), p.package.clone(), p.data.len()));
                }
                let lp = library.as_mut()?.get(name)?;
                Some((lp.name.clone(), lp.path.display().to_string(), lp.package.clone(), lp.data.len()))
            };
            let map = std::cell::RefCell::new(&mut map);
            let logs_cell = std::cell::RefCell::new(&mut logs);
            master_map_build(
                &server_packages,
                game_pkg.is_some(),
                &mut |what| {
                    let name_value = match what {
                        MasterAdd::Level => level_pkg.as_str(),
                        MasterAdd::Server(sp) => sp,
                        MasterAdd::Game => game_pkg.as_deref().unwrap_or(""),
                    };
                    map.borrow_mut().add_linker(name_value, &mut load)
                },
                &mut || map.borrow_mut().compute(),
                &mut |l| logs_cell.borrow_mut().push(l),
            );
        }
        self.log.extend(logs);
        // Mods build only: the mods' package after the game's ones (mod_net.rs).
        #[cfg(feature = "mods")]
        self.mod_net_add_package(&mut map);
        let index = self.net_build_index(&map);
        if let Some(d) = self.ipdrv.net_driver.as_mut() {
            d.master_map = map;
            d.master_index = Some(index);
        }
    }

    /// MapIndex of a computed package map.
    pub(crate) fn net_build_index(&mut self, map: &PackageMap) -> std::rc::Rc<crate::net_map::MapIndex> {
        #[allow(unused_mut)]
        let mut index = {
            let packages = &self.packages;
            let library = &mut self.library;
            let mut load = |name: &str| -> Option<openxiii_package::Package> {
                if let Some(p) = packages.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
                    return Some(p.package.clone());
                }
                library.as_mut()?.get(name).map(|lp| lp.package.clone())
            };
            crate::net_map::MapIndex::build(map, &mut load)
        };
        // Mods build only: the exports of the mods' package (mod_net.rs).
        #[cfg(feature = "mods")]
        self.mod_net_index(&mut index);
        std::rc::Rc::new(index)
    }

    /// UNetPendingLevel: client driver, connection to `host:port`, HELLO.
    pub fn net_connect(&mut self, url: &str) -> Result<(), String> {
        let host = url.split(['?', '#', '/']).next().unwrap_or("");
        let (name, port) = match host.split_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().unwrap_or(7777)),
            None => (host, 7777),
        };
        // UTcpipConnection: four dotted numbers → inet_addr (INADDR_NONE not
        // checked), otherwise resolution in a thread.
        let dotted = crate::ipdrv::host_is_dotted(name);
        let ip = if dotted { Ipv4Addr::from(u32::from_be(crate::ipdrv::inet_addr(name))) } else { Ipv4Addr::UNSPECIFIED };
        let remote = SocketAddrV4::new(ip, port);
        self.ipdrv.started = true;
        // UTcpNetDriver::InitConnect: InitBase as a client (any port, 32 KiB queues).
        let (socket, bound) = match crate::ipdrv::bind_udp(&mut self.ipdrv, 0, 20, true) {
            Ok(x) => x,
            Err(e) => {
                self.log.push("Init: WinSock: Socket queue 32768 / 32768".into());
                return Err(format!("WinSock: binding to port 0 failed ({})", crate::ipdrv::socket_error_text_address(e) as i32));
            }
        };
        if let Some(s) = &socket.0 {
            let _ = s.set_broadcast(true);
            let _ = s.set_nonblocking(true);
        }
        let (rcv, snd) = crate::ipdrv::socket_queue(&socket, 0x8000);
        self.log.push(format!("Init: WinSock: Socket queue {rcv} / {snd}"));
        let local_addr = self.log_local_bind_addr(false);
        // UNetDriver::InitConnect: bLAN = URL.HasOption("LAN").
        let parsed = crate::travel::Url::parse(None, url, crate::travel::TRAVEL_ABSOLUTE);
        let lan = parsed.has_option("LAN");
        let mut cfg = self.net_driver_config();
        // Not in the original: enhanced network play, higher Internet ceilings (minimums kept).
        if self.net_enhanced() {
            cfg.apply_enhanced_internet();
        }
        let mut driver = crate::ipdrv::NetDriver::new(socket, bound, lan, cfg);
        driver.local_addr = local_addr;
        self.ipdrv.net_driver = Some(driver);
        let speed = self.net_speed_config();
        let mut conn = Connection::new(remote, ConnState::Pending, 0.0, speed);
        conn.url_host = host.split(':').next().unwrap_or("").to_string();
        conn.connect_time = self.app_seconds;
        self.net_conn_settings(&mut conn);
        conn.on_client = true;
        let resolve = if dotted {
            None
        } else {
            self.log.push(format!("Log: Resolving {name}..."));
            Some((name.to_string(), crate::ipdrv::resolve_host(name)))
        };
        // UTcpNetDriver::InitConnect.
        self.log.push(format!("DevNet: Game client on port {bound}, rate {}", conn.current_net_speed));
        conn.create_channel(CHTYPE_CONTROL, true, Some(0));
        let mut nc = NetConn::new(conn);
        nc.resolve = resolve;
        nc.name = self.net_next_connection_name();
        if let Some(d) = self.ipdrv.net_driver.as_mut() {
            d.server_conn = Some(nc);
        }
        self.ipdrv.pending = Some(PendingLevel { url: url.to_string(), ..Default::default() });
        let hello = format!("HELLO REVISION=0 MINVER={ENGINE_MIN_NET_VERSION} VER={ENGINE_NEGOTIATION_VERSION}");
        self.net_send(None, &hello);
        self.net_flush(None);
        self.net_send_outgoing();
        Ok(())
    }

    /// Tick of the pending level: TickDispatch then TickFlush of the driver.
    /// Tick of the pending level: connection closed without a recorded error →
    /// "The server is unreachable"; otherwise TickDispatch then TickFlush of the driver.
    pub fn net_pending_tick(&mut self, dt: f32) {
        let closed = self.ipdrv.net_driver.as_ref().and_then(|d| d.server_conn.as_ref()).is_some_and(|c| c.conn.state == ConnState::Closed);
        if closed && self.ipdrv.pending.as_ref().is_some_and(|p| p.error.is_empty()) {
            let e = self.localize("Errors", "ServerUnreachable", "Engine", false);
            if let Some(p) = self.ipdrv.pending.as_mut() {
                p.error = e;
            }
            return;
        }
        self.net_tick_dispatch(dt);
        self.net_tick_flush();
    }

    /// Texts received by the client (while pending, otherwise ULevel::NotifyReceivedText).
    pub(crate) fn net_client_text(&mut self, text: &str) {
        if self.ipdrv.pending.is_none() {
            // ULevel::NotifyReceivedText on the client side: USERFLAG first (UserFlags of the connection = atoi), then the log.
            if let Some(rest) = parse_command(text, "USERFLAG") {
                let flags = crate::config::atoi(&rest.chars().collect::<Vec<char>>(), 0);
                if let Some(c) = self.net_conn(None) {
                    c.user_flags = flags;
                }
                return;
            }
            self.log.push(format!("DevNet: Level client received: {text}"));
            // Not in the original: the frame rate an enhanced LAN host announces.
            if self.receive_host_rate(text) {
                return;
            }
            if parse_command(text, "FAILURE").is_some() {
                // Not in the original: refused, not joined again (net_reconnect.rs).
                self.net_reconnect_refused();
                self.net_connection_failed(6.0);
            } else if let Some(name) = parse_command(text, "YOUAREKICKED") {
                self.net_reconnect_refused();
                self.net_connection_failed(6.0);
                let pcs: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.alive(a) && self.is_a(a, "PlayerController")).collect();
                for pc in pcs {
                    let kicked = self.po(pc, "PlayerReplicationInfo").is_some_and(|r| self.propr(r, "PlayerName").as_name() == name);
                    if kicked {
                        self.event_pub(pc, "IAmKicked", vec![]);
                    }
                }
            }
            return;
        }
        self.log.push(format!("DevNet: PendingLevel received: {text}"));
        // UNetPendingLevel::NotifyReceivedText.
        if let Some(rest) = parse_command(text, "UPGRADE") {
            // MINVER < 0x33E: localized "ConnectionFailed: ServerOutdated" and SetProgress 6 s;
            // otherwise SetProgress("", "", -1) (the HUD's upgrade menu), without an error.
            let min_ver = parse_int(rest, "MINVER=").unwrap_or(0);
            if min_ver < 0x33E {
                self.net_reconnect_refused();
                let outdated = self.localize("Errors", "ServerOutdated", "Engine", false);
                let failed = self.localize("Errors", "ConnectionFailed", "Engine", false);
                if let Some(p) = self.ipdrv.pending.as_mut() {
                    p.error = format!("{failed}: {outdated}");
                }
                let outdated = self.localize("Errors", "ServerOutdated", "Engine", false);
                let failed = self.localize("Errors", "ConnectionFailed", "Engine", false);
                self.engine_set_progress(&failed, &outdated, 6.0);
            } else {
                self.engine_set_progress("", "", -1.0);
            }
        } else if parse_command(text, "MISMATCH").is_some() {
            self.net_reconnect_refused();
            let failed = self.localize("Errors", "ConnectionFailed", "Engine", false);
            if let Some(p) = self.ipdrv.pending.as_mut() {
                p.error = failed;
            }
        } else if let Some(rest) = parse_command(text, "FAILURE") {
            // Empty: ConnectionFailed; "POM:Package:Section:Key": Localize, malformed or
            // not found ("<?") → ConnectionFailed with a log; otherwise the text. Then
            // SetProgress("Rejected By Server", text, 10 s).
            let error = if rest.is_empty() {
                self.localize("Errors", "ConnectionFailed", "Engine", false)
            } else if let Some(pom) = rest.strip_prefix("POM:") {
                let mut parts = pom.splitn(3, ':');
                match (parts.next(), parts.next(), parts.next()) {
                    (Some(package), Some(section), Some(key)) => {
                        let text = self.localize(section, key, package, false);
                        if text.starts_with("<?") {
                            self.log.push(format!("Log: Received {rest} from server, but unable to find the corresponding localized text"));
                            self.localize("Errors", "ConnectionFailed", "Engine", false)
                        } else {
                            text
                        }
                    }
                    _ => {
                        self.log.push(format!("Log: Received {rest} from server, but POM message is malformed"));
                        self.localize("Errors", "ConnectionFailed", "Engine", false)
                    }
                }
            } else {
                rest.to_string()
            };
            if let Some(p) = self.ipdrv.pending.as_mut() {
                p.error = error;
            }
            // Not in the original: refused (full, a password), not joined again.
            self.net_reconnect_refused();
            self.engine_set_progress("Rejected By Server", rest, 10.0);
        } else if let Some(rest) = parse_command(text, "USES") {
            let name = parse_value(rest, "PKG=").unwrap_or_default();
            let Some(guid) = parse_value(rest, "GUID=").and_then(|g| crate::net_map::parse_guid(&g)) else { return };
            let generation = parse_int(rest, "GEN=").unwrap_or(0);
            let size = parse_int(rest, "SIZE=").unwrap_or(0);
            if name.is_empty() || generation < 0 || size < 0 {
                return;
            }
            let flags = parse_int(rest, "FLAGS=").unwrap_or(0) as u32;
            let info = PackageInfo {
                name,
                url: parse_value(rest, "FNAME=").unwrap_or_default(),
                guid,
                file_size: size,
                object_base: -1,
                object_count: -1,
                name_index: -1,
                name_count: -1,
                local_generation: 0,
                remote_generation: generation,
                package_flags: flags,
                generations: Vec::new(),
                export_count: 0,
                names_total: 0,
            };
            if let Some(c) = self.net_conn(None) {
                c.package_map.list.push(info);
            }
        } else if let Some(rest) = parse_command(text, "USERFLAG") {
            let v = crate::config::atoi(&rest.chars().collect::<Vec<_>>(), 0);
            if let Some(c) = self.net_conn(None) {
                c.user_flags = v;
            }
        } else if let Some(rest) = parse_command(text, "CHALLENGE") {
            let ver = parse_int(rest, "VER=").unwrap_or(ENGINE_MIN_NET_VERSION);
            let challenge = parse_int(rest, "CHALLENGE=").unwrap_or(0);
            let speed = match self.net_conn(None) {
                Some(c) => {
                    c.conn.negotiated_ver = ver;
                    c.conn.challenge = challenge;
                    c.conn.current_net_speed
                }
                None => return,
            };
            // STATS=1 (host with bWorldLog): AddOption "Checksum=" to the pending URL,
            // MD5(PlayerReplicationInfo.PlayerName + ngWorldSecret) of the viewport's controller as
            // %02x, or "Checksum=NoChecksum" without ngWorldSecret.
            let mut url = crate::travel::Url::parse(None, &self.ipdrv.pending.as_ref().map(|p| p.url.clone()).unwrap_or_default(), crate::travel::TRAVEL_ABSOLUTE);
            if parse_int(rest, "STATS=") == Some(1) {
                let pc = self.find_viewport_object(0).and_then(|v| self.propr(v, "Actor").as_object());
                let secret = pc.map(|p| self.prop(p, "ngWorldSecret").to_string()).unwrap_or_default();
                let option = if secret.is_empty() {
                    "Checksum=NoChecksum".to_string()
                } else {
                    let name = pc.and_then(|p| self.propr(p, "PlayerReplicationInfo").as_object()).map(|r| self.prop(r, "PlayerName").to_string()).unwrap_or_default();
                    format!("Checksum={}", crate::stat_log::hex(&crate::stat_log::Md5::digest(&name.chars().chain(secret.chars()).map(|c| c as u32 as u8).collect::<Vec<u8>>())))
                };
                url.add_option(&option);
                if let Some(p) = self.ipdrv.pending.as_mut() {
                    p.url = url.string();
                }
            }
            // FURL::String of a copy of the pending URL ("game=" options included); then the "game=" options are removed from the pending
            // URL itself (the level's array at +0x44).
            let out = self.login_url_with_aim(&url);
            // Mods build only: the client's mods, for the server's check (mod_net.rs).
            #[cfg(feature = "mods")]
            let out = self.mod_login_url(out);
            url.options.retain(|o| !o.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("game=")));
            if let Some(p) = self.ipdrv.pending.as_mut() {
                p.url = url.string();
            }
            self.net_send(None, &format!("NETSPEED {speed}"));
            self.net_send(None, &format!("LOGIN RESPONSE={} URL={out}", challenge_response(challenge)));
            self.net_flush(None);
        } else if let Some(rest) = parse_command(text, "WELCOME") {
            self.log.push(format!("DevNet: Welcomed by server: {rest}"));
            let level = parse_value(rest, "LEVEL=").unwrap_or_default();
            let lone = parse_value(rest, "LONE=").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
            if let Some(challenge) = parse_int(rest, "CHALLENGE=") {
                if let Some(c) = self.net_conn(None) {
                    c.conn.challenge = challenge;
                }
            }
            // Announced packages: present here with the same GUID, otherwise they would have to be downloaded.
            let list = self.net_conn(None).map(|c| c.package_map.list.clone()).unwrap_or_default();
            let dir = self.config.as_ref().map(|c| c.dir().to_path_buf()).unwrap_or_default();
            let mut packages_list = Vec::new();
            for p in &list {
                // appFindPackageFile (Core): a file with this name and this GUID; a "Name.dll" with a size > 0 replaces it.
                let mut here = self.packages.iter().find(|q| q.name.eq_ignore_ascii_case(&p.name)).map(|q| q.package.header.guid).or_else(|| self.library.as_mut().and_then(|l| l.get(&p.name)).map(|q| q.package.header.guid));
                // appFindPackageFile: a package with this GUID already received into the cache (Cache\cache.ini, GUID.uxx).
                if here != Some(p.guid) {
                    if let Some(path_buf) = self.cache_finds(&p.guid) {
                        if let Some(lib) = self.library.as_mut() {
                            lib.set_file(&p.name, path_buf);
                        }
                        here = Some(p.guid);
                    }
                }
                // Mods build only: the mods' package is here when the mods are the same (mod_net.rs).
                #[cfg(feature = "mods")]
                if let Some(ours) = self.mod_net_package_here(&p.name, &p.guid) {
                    here = ours.then_some(p.guid);
                }
                let dll_size = std::fs::metadata(dir.join(format!("{}.dll", p.name))).map_or(-1, |m| m.len() as i32);
                packages_list.push(WelcomePackage { name: p.name.clone(), flags: p.package_flags, found: here == Some(p.guid), dll_size });
            }
            #[cfg(feature = "mods")]
            self.mod_net_welcome_check(&list);
            let allow = self.ipdrv.net_driver.as_ref().is_some_and(|d| d.cfg.allow_downloads);
            let r = welcome_packages(&packages_list, allow);
            if let Some(c) = self.net_conn(None) {
                for (info, flags) in c.package_map.list.iter_mut().zip(&r.flags) {
                    info.package_flags = *flags;
                }
            }
            let p = self.ipdrv.pending.as_mut().unwrap();
            p.level = level;
            p.lone = lone;
            p.files_needed += r.files_needed;
            if let Some(e) = r.error {
                p.error = e;
            }
            if r.closed {
                if let Some(c) = self.ipdrv.net_driver.as_mut().and_then(|d| d.server_conn.as_mut()) {
                    c.conn.state = ConnState::Closed;
                }
            }
            // Outside the error exit, ReceiveNextFile (net_file.rs) requests the
            // first package marked 0x8000, then bSuccess = 1 (the JOIN still waits for FilesNeeded = 0).
            if !r.closed {
                self.net_receive_next();
                if let Some(p) = self.ipdrv.pending.as_mut() {
                    p.success = true;
                }
            }
        }
    }

    /// After a client's map is loaded: the level takes the driver, JOIN.
    pub fn net_join(&mut self) {
        self.ipdrv.pending = None;
        // UGameEngine::VerifyNetPackageMap then Compute: the announced packages are
        // the local ones (same GUIDs, checked at WELCOME).
        let mut map = self.net_conn(None).map(|c| c.package_map.clone()).unwrap_or_default();
        for info in map.list.iter_mut() {
            let local = self.packages.iter().find(|p| p.name.eq_ignore_ascii_case(&info.name)).map(|p| p.package.clone()).or_else(|| self.library.as_mut().and_then(|l| l.get(&info.name)).map(|p| p.package.clone()));
            if let Some(p) = local {
                info.generations = p.header.generations.iter().map(|g| (g.export_count as i32, g.name_count as i32)).collect();
                info.export_count = p.exports.len() as i32;
                info.names_total = p.names.len() as i32;
            }
            // Mods build only: the mods' package has no file (mod_net.rs).
            #[cfg(feature = "mods")]
            self.mod_net_local_info(info);
        }
        map.compute();
        let index = self.net_build_index(&map);
        if let Some(c) = self.net_conn(None) {
            c.package_map = map;
            c.index = Some(index);
        }
        if let Some(level) = self.level {
            self.set_prop(level, "NetMode", Value::Byte(3));
        }
        self.net_send(None, "JOIN");
        self.net_flush(None);
        self.net_send_outgoing();
    }
}

/// The packages that UGameEngine::BuildServerMasterMap adds to the server's map, in order.
pub(crate) enum MasterAdd<'a> {
    Level,
    Server(&'a str),
    Game,
}

/// BuildServerMasterMap: the level's package, each of the ServerPackages ("Server Package: %s" before, "(server-side only)" after when
/// UPackageMap::AddLinker returns -1), the package of the GameInfo's class, then Compute.
pub(crate) fn master_map_build(server_packages: &[String], has_game: bool, add: &mut dyn FnMut(MasterAdd<'_>) -> i32, compute: &mut dyn FnMut(), log: &mut dyn FnMut(String)) {
    add(MasterAdd::Level);
    for sp in server_packages {
        log(format!("Server Package: {sp}"));
        if add(MasterAdd::Server(sp)) == -1 {
            log("   (server-side only)".into());
        }
    }
    if has_game {
        add(MasterAdd::Game);
    }
    compute();
}

/// Package announced by the server, as seen at WELCOME (UNetPendingLevel::NotifyReceivedText): flags, presence of a file with the same GUID
/// (appFindPackageFile), size of "Name.dll" (−1 if missing).
pub(crate) struct WelcomePackage {
    pub name: String,
    pub flags: u32,
    pub found: bool,
    pub dll_size: i32,
}

pub(crate) struct WelcomeResult {
    pub files_needed: i32,
    pub error: Option<String>,
    pub closed: bool,
    pub flags: Vec<u32>,
}

/// WELCOME loop: each missing package (neither a file with this GUID, nor a "Name.dll" with a size > 0) counts in FilesNeeded and takes the flag
/// 0x8000; without download permission (driver's AllowDownloads and the package's flag 1) the error "Downloading 'Name' not allowed" and the closing
/// of the server connection, and the loop stops there.
pub(crate) fn welcome_packages(packages_list: &[WelcomePackage], allow_downloads: bool) -> WelcomeResult {
    let mut r = WelcomeResult { files_needed: 0, error: None, closed: false, flags: Vec::new() };
    for p in packages_list {
        let mut flags = p.flags;
        if !r.closed && !p.found && p.dll_size <= 0 {
            r.files_needed += 1;
            flags |= 0x8000;
            if !(allow_downloads && flags & 1 != 0) {
                // The error closes the connection and leaves the loop (the following packages are not seen).
                r.error = Some(format!("Downloading '{}' not allowed", p.name));
                r.closed = true;
            }
        }
        r.flags.push(flags);
    }
    r
}

