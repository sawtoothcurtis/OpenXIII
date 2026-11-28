//! IpDrv.dll: the script's network links (AInternetLink, AUdpLink) and the listening TcpNetDriver, on
//! real non-blocking UDP sockets (std::net), like WinSock in the game.
//!
//! AInternetLink: on creation, InitSockets (WSAStartup once,
//! "Init: WinSock: version…") then LinkMode 0, ReceiveMode 1 (RMODE_Event), Port 0,
//! Socket -1, RemoteSocket -1, on top of the script's default values.
//! AUdpLink::Tick: after AActor::Tick, one recvfrom of at most 254 bytes per frame in
//! RMODE_Event (ReceivedText / ReceivedLine / ReceivedBinary depending on LinkMode), otherwise DataPending
//! through select. ULevel::Listen: TcpNetDriver::InitListen (InitBase: UDP
//! socket, SO_BROADCAST, SO_REUSEADDR, 128 KiB queues, the URL's port, 20 tries), then the ServerActors
//! of [Engine.GameEngine].
//! Not reimplemented: AInternetLink::Resolve (resolver thread), GetLocalIP, Validate; TcpLink; the
//! TcpNetDriver's game traffic (connections, channels).

use crate::value::{ObjId, Value};
use crate::Vm;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};

/// WSAEWOULDBLOCK.
const WOULD_BLOCK: i32 = 0x2733;
/// WSAEMSGSIZE: datagram longer than the recvfrom buffer.
const MSG_SIZE: i32 = 0x2738;

crate::improved::network::with_edition_fields! {
#[derive(Debug, Default)]
pub struct IpDrv {
    /// GSocketsStarted: WSAStartup succeeded.
    pub started: bool,
    /// WSAStartup already attempted.
    pub wsa_done: bool,
    /// Socket of each AUdpLink (AInternetLink.Socket holds a descriptor number).
    pub udp: HashMap<ObjId, Sock>,
    /// PrivateResolveInfo (+0x200) of the AInternetLinks: resolved name and result of the resolver thread.
    pub resolves: HashMap<ObjId, (String, ResolveResult)>,
    /// Without network (headless runs, unless OXIII_NETWORK=1): sockets are
    /// simulated, bound to ports reserved within this process, without sending or receiving anything.
    pub offline: bool,
    pub offline_ports: std::collections::HashSet<u16>,
    pub next_handle: i32,
    /// WSAGetLastError of the last call.
    pub last_error: i32,
    /// UTcpNetDriver of the listening level or of the client (UDP socket, bound port, connections).
    pub net_driver: Option<NetDriver>,
    /// UNetPendingLevel of the client waiting for WELCOME.
    pub pending: Option<crate::net_game::PendingLevel>,
    /// Number of a connection's next Player object (TcpipConnectionN).
    pub next_connection: usize,
    /// UGameEngine::ConnectionStatus (+0xD8) and ConnectionError (+0xDC): ReportNetFailure of
    /// type 1.
    pub connection_status: i32,
    pub connection_error: String,
    /// The playable game follows travels to a network address inside the engine (pending level
    /// held by engine_travel_tick); command-line tools receive them as travels.
    pub engine_pending: bool,
    /// Dedicated server (UCC server, UServerCommandlet::Main): no UClient
    /// (GIsClient = 0), so neither a viewport nor a local player; LoadMap always listens and NetMode
    /// is NM_DedicatedServer.
    pub dedicated: bool,
    /// Deliberate OpenXIII difference (map downloads): AllowDownloads is true
    /// despite the shipped XIII.ini (False). Set by oxiii server, oxiii simulate and the game; false in tests.
    pub downloads: bool,
    /// UGameEngine::ConnectionFailure and TimeToWaitUpTo (ReportNetFailure): return to the menu at
    /// this appSeconds.
    pub connection_failure: bool,
    pub time_to_wait_up_to: f64,
    /// Replication caches (net_rep.rs).
    pub rep: crate::net_rep::RepCache,
    /// ULevel::SpawnActor(…, bRemoteOwned) in progress: roles swapped (UActorChannel::ReceivedBunch).
    pub spawn_remote_owned: bool,
}
}

#[derive(Debug)]
pub struct NetDriver {
    pub socket: Sock,
    pub port: u16,
    /// ClientConnections (host) and ServerConnection (client).
    pub conns: Vec<crate::net_game::NetConn>,
    pub server_conn: Option<crate::net_game::NetConn>,
    /// UNetDriver::Time (+0x44) and bLAN (+0x108).
    pub time: f64,
    pub lan: bool,
    /// MasterMap (+0x40) of the host.
    pub master_map: crate::net_map::PackageMap,
    /// Index of the master map, shared by the clients' connections.
    pub master_index: Option<std::rc::Rc<crate::net_map::MapIndex>>,
    /// QueuedBytes (+0x104): bytes sent and not yet covered by the rate.
    pub queued_bytes: i32,
    /// Host rate (+0xA8): that of the first connection.
    pub server_speed: i32,
    /// Ini settings (UNetDriver::StaticConstructor).
    pub cfg: NetDriverCfg,
    /// The first recvfrom error is logged, the following ones are not.
    pub recv_error_logged: bool,
    /// LocalAddr (+0x110): getlocalbindaddr of InitBase.
    pub local_addr: [u8; 4],
    /// bProfileStats (+0x84): ParseParam(appCmdLine(), "profilestats"), set by the constructor; read by no
    /// function of Engine or IpDrv.
    pub profile_stats: bool,
}

/// Enhanced network play (`Vm::net_enhanced`), a game over the Internet: highest rate of a
/// client's own sending in the game's automatic adjustment (UpdateNetSpeedAccordingToLag; 3000 in
/// XIII.ini; its minimum and steps are the game's). Measured on simulated links (two headless
/// games, a client walking): on a good link the client sent at its 3000 cap and the server got
/// 18 moves a second, longest pause 608 ms; at 6000, 3791 bytes/s, 23 moves a second, 75 ms. On
/// bad and narrow links the adjustment came down to 1500 as with the game's values. The servers'
/// ceilings and the rate a client asks (NETSPEED) are the game's: a two-player game sent 1 to
/// 2 KB/s, far below them, so raising them showed nothing.
pub const ENHANCED_CLIENT_INTERNET_MAX: i32 = 6000;
/// Enhanced network play, a LAN client in a level: rate (bytes per second) of its own sending,
/// instead of CONFClientLanMin/MaxNetSpeed (10000 to 11000 in XIII.ini). It sends one move each
/// frame, up to 240 a second (about 53 bytes each, 13 KB/s with the acknowledgements), which
/// the game's LAN rate would cut. The rate it asks of the host (NETSPEED, sent before the
/// level) is the game's.
pub const ENHANCED_LAN_CLIENT_RATE: i32 = 30000;

impl NetDriverCfg {
    /// Enhanced network play: a client's Internet maximum raised (never lowered), its minimum,
    /// steps and lag thresholds kept, so that a slow link comes down to where the original stays.
    pub fn apply_enhanced_internet(&mut self) {
        self.client.internet_max = self.client.internet_max.max(ENHANCED_CLIENT_INTERNET_MAX);
    }
}

impl NetDriver {
    /// UNetDriver::UNetDriver (MasterMap, Role and RemoteRole properties, which the VM
    /// designates by name) then UTcpNetDriver.
    pub fn new(socket: Sock, port: u16, lan: bool, cfg: NetDriverCfg) -> Self {
        NetDriver { socket, port, conns: Vec::new(), server_conn: None, time: 0.0, lan, master_map: Default::default(), master_index: None, queued_bytes: 0, server_speed: 0, cfg, recv_error_logged: false, local_addr: [0; 4], profile_stats: Self::profile_stats_of(&crate::config::app_cmd_line()) }
    }

    /// bProfileStats of UNetDriver::UNetDriver: ParseParam(line, "profilestats") on appCmdLine (without the program name).
    pub fn profile_stats_of(line_value: &str) -> bool {
        crate::app_init::parse_param(line_value, "profilestats")
    }
}

/// Config properties declared by UNetDriver::StaticConstructor, read from
/// [IpDrv.TcpNetDriver] then, if the key is missing, from [Engine.NetDriver] (default object of the
/// parent class); a key missing everywhere keeps the constructor's value (NET_DRIVER_DEFAULTS).
/// Offsets in UNetDriver.
#[derive(Debug, Default, Clone)]
pub struct NetDriverCfg {
    /// +0x4C, +0x50, +0x54, +0x58, +0x5C, +0x60.
    pub connection_timeout: f32,
    pub initial_connect_timeout: f32,
    pub keep_alive_time: f32,
    pub relevant_timeout: f32,
    pub spawn_priority_seconds: f32,
    pub server_travel_pause: f32,
    /// +0x64 MaxClientRate, +0x68…+0x7C rates.
    pub max_client_rate: i32,
    pub net_server_max_tick_rate: i32,
    pub net_server_min_tick_rate: i32,
    pub lan_server_max_tick_rate: i32,
    pub lan_server_min_tick_rate: i32,
    pub net_client_max_tick_rate: i32,
    pub lan_client_max_tick_rate: i32,
    /// +0x80 AllowDownloads, +0x98 MaxDownloadSize.
    pub allow_downloads: bool,
    pub max_download_size: i32,
    /// OpenXIII key (absent from the original engine): [IpDrv.TcpNetDriver] OpenXIIIDownloadRate, send rate of
    /// downloaded packets in bytes per second and per client. 0 (default): like the original, at the
    /// connection's rate (about 15 KB/s), or unlimited with -lanplay (UFileChannel::Tick, net_file.rs).
    pub download_rate: i32,
    /// CONFClient* (+0xB0…+0xC8), CONFListenServer* (+0xCC…+0xE4), CONFDedicatedServer*
    /// (+0xE8…+0x100): NetSpeedStepUp, InternetMin/Max, LanMin/Max, LagToTakeAction,
    /// MaxAllowedLag.
    pub client: SpeedCfg,
    pub listen_server: SpeedCfg,
    pub dedicated_server: SpeedCfg,
}

/// Kind of a UNetDriver config property.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum KindProp {
    Float,
    Int,
    Bool,
}

/// Config properties (CPF_Config, category "Client") declared by UNetDriver::StaticConstructor,
/// in order: name, kind, offset. DownloadManagers (TArray<FString>
/// +0x9C) is added to them, which OpenXIII does not read.
pub(crate) const NET_DRIVER_PROPS: [(&str, KindProp, u32); 36] = {
    use KindProp::{Bool as B, Float as F, Int as I};
    [
        ("ConnectionTimeout", F, 0x4C),
        ("InitialConnectTimeout", F, 0x50),
        ("KeepAliveTime", F, 0x54),
        ("RelevantTimeout", F, 0x58),
        ("SpawnPrioritySeconds", F, 0x5C),
        ("ServerTravelPause", F, 0x60),
        ("MaxClientRate", I, 0x64),
        ("NetServerMaxTickRate", I, 0x68),
        ("NetServerMinTickRate", I, 0x6C),
        ("LanServerMaxTickRate", I, 0x70),
        ("LanServerMinTickRate", I, 0x74),
        ("NetClientMaxTickRate", I, 0x78),
        ("LanClientMaxTickRate", I, 0x7C),
        ("AllowDownloads", B, 0x80),
        ("MaxDownloadSize", I, 0x98),
        ("CONFClientNetSpeedStepUp", I, 0xB0),
        ("CONFClientInternetMinNetSpeed", I, 0xB4),
        ("CONFClientInternetMaxNetSpeed", I, 0xB8),
        ("CONFClientLanMinNetSpeed", I, 0xBC),
        ("CONFClientLanMaxNetSpeed", I, 0xC0),
        ("CONFClientLagToTakeAction", F, 0xC4),
        ("CONFClientMaxAllowedLag", F, 0xC8),
        ("CONFListenServerNetSpeedStepUp", I, 0xCC),
        ("CONFListenServerInternetMinNetSpeed", I, 0xD0),
        ("CONFListenServerInternetMaxNetSpeed", I, 0xD4),
        ("CONFListenServerLanMinNetSpeed", I, 0xD8),
        ("CONFListenServerLanMaxNetSpeed", I, 0xDC),
        ("CONFListenServerLagToTakeAction", F, 0xE0),
        ("CONFListenServerMaxAllowedLag", F, 0xE4),
        ("CONFDedicatedServerNetSpeedStepUp", I, 0xE8),
        ("CONFDedicatedServerInternetMinNetSpeed", I, 0xEC),
        ("CONFDedicatedServerInternetMaxNetSpeed", I, 0xF0),
        ("CONFDedicatedServerLanMinNetSpeed", I, 0xF4),
        ("CONFDedicatedServerLanMaxNetSpeed", I, 0xF8),
        ("CONFDedicatedServerLagToTakeAction", F, 0xFC),
        ("CONFDedicatedServerMaxAllowedLag", F, 0x100),
    ]
};

/// Default values set at the end of UNetDriver::StaticConstructor:
/// offset, 32-bit word (floats as bits). The delays (+0x4C…+0x60), AllowDownloads
/// and MaxDownloadSize have none (0).
pub(crate) const NET_DRIVER_DEFAULTS: [(u32, u32); 28] = [
    (0x64, 25000),
    (0x68, 45),
    (0x6C, 30),
    (0x70, 90),
    (0x74, 60),
    (0x78, 45),
    (0x7C, 60),
    (0xB0, 20),
    (0xB4, 1500),
    (0xB8, 3000),
    (0xBC, 10000),
    (0xC0, 11000),
    (0xC4, 0x3E19_999A),
    (0xC8, 0x3E99_999A),
    (0xCC, 50),
    (0xD0, 5000),
    (0xD4, 7000),
    (0xD8, 15000),
    (0xDC, 40000),
    (0xE0, 0x3E19_999A),
    (0xE4, 0x3EB3_3333),
    (0xE8, 50),
    (0xEC, 5000),
    (0xF0, 15000),
    (0xF4, 15000),
    (0xF8, 40000),
    (0xFC, 0x3E19_999A),
    (0x100, 0x3EB3_3333),
];

/// 32-bit words of UTcpNetDriver's default object from +0x4C to +0x103: the constructor's
/// values, then for each property whose text `lookup` returns, Core's ImportText
/// (UIntProperty: nothing without a leading sign or digit; UFloatProperty:
/// appAtof; UBoolProperty: the token "1", "True" or localized True sets the bit,
/// "0", "False" or localized False clears it, anything else leaves it).
pub(crate) fn net_driver_values(lookup: &mut dyn FnMut(&str) -> Option<Vec<char>>, true_value: &str, false_value: &str) -> [u32; 46] {
    let mut m = [0u32; 46];
    let i = |off: u32| ((off - 0x4C) / 4) as usize;
    for (off, v) in NET_DRIVER_DEFAULTS {
        m[i(off)] = v;
    }
    for (name_value, kind_value, off) in NET_DRIVER_PROPS {
        let Some(t) = lookup(name_value) else { continue };
        let slot = &mut m[i(off)];
        match kind_value {
            KindProp::Int => {
                if t.first().is_some_and(|&c| c == '-' || c.is_ascii_digit()) {
                    *slot = crate::config::atoi(&t, 0) as u32;
                }
            }
            KindProp::Float => *slot = crate::config::atof(&t, 0).to_bits(),
            KindProp::Bool => {
                if let Some((tok, _)) = crate::config::read_token(&t, 0, 0x400, false, &mut Vec::new()) {
                    if tok.eq_ignore_ascii_case("1") || tok.eq_ignore_ascii_case("True") || tok.eq_ignore_ascii_case(true_value) {
                        *slot |= 1;
                    } else if tok.eq_ignore_ascii_case("0") || tok.eq_ignore_ascii_case("False") || tok.eq_ignore_ascii_case(false_value) {
                        *slot &= !1;
                    }
                }
            }
        }
    }
    m
}

impl NetDriverCfg {
    /// Fields read at the offsets of NET_DRIVER_PROPS.
    pub(crate) fn from_words(m: &[u32; 46]) -> Self {
        let w = |off: u32| m[((off - 0x4C) / 4) as usize];
        let f = |off: u32| f32::from_bits(w(off));
        let n = |off: u32| w(off) as i32;
        let speed = |b: u32| SpeedCfg { step_up: n(b), internet_min: n(b + 4), internet_max: n(b + 8), lan_min: n(b + 12), lan_max: n(b + 16), lag_to_take_action: f(b + 20), max_allowed_lag: f(b + 24) };
        NetDriverCfg {
            connection_timeout: f(0x4C),
            initial_connect_timeout: f(0x50),
            keep_alive_time: f(0x54),
            relevant_timeout: f(0x58),
            spawn_priority_seconds: f(0x5C),
            server_travel_pause: f(0x60),
            max_client_rate: n(0x64),
            net_server_max_tick_rate: n(0x68),
            net_server_min_tick_rate: n(0x6C),
            lan_server_max_tick_rate: n(0x70),
            lan_server_min_tick_rate: n(0x74),
            net_client_max_tick_rate: n(0x78),
            lan_client_max_tick_rate: n(0x7C),
            allow_downloads: w(0x80) & 1 != 0,
            max_download_size: n(0x98),
            download_rate: 0,
            client: speed(0xB0),
            listen_server: speed(0xCC),
            dedicated_server: speed(0xE8),
        }
    }
}

/// One family of CONF… keys of UpdateNetSpeedAccordingToLag.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpeedCfg {
    pub step_up: i32,
    pub internet_min: i32,
    pub internet_max: i32,
    pub lan_min: i32,
    pub lan_max: i32,
    pub lag_to_take_action: f32,
    pub max_allowed_lag: f32,
}

/// SocketError: text of a WinSock code. InitBase's messages pass it to
/// a "%i": what gets printed is the address of this text in IpDrv.dll.
pub(crate) fn socket_error_text_address(code: i32) -> u32 {
    match code {
        0 => 0x107535EC,
        0x2714 => 0x107535F8,
        0x2719 => 0x10753604,
        0x271D => 0x107535E0,
        0x271E => 0x107535C8,
        0x2726 => 0x107535D4,
        0x2728 => 0x107535BC,
        0x2733 => 0x107535AC,
        0x2734 => 0x1075359C,
        0x2735 => 0x10753590,
        0x2736 => 0x10753584,
        0x2737 => 0x10753574,
        0x2738 => 0x10753568,
        0x2739 => 0x10753558,
        0x273A => 0x10753548,
        0x273B => 0x10753534,
        0x273C => 0x10753520,
        0x273D => 0x10753510,
        0x273E => 0x10753500,
        0x273F => 0x107534F0,
        0x2740 => 0x107534E0,
        0x2741 => 0x107534CC,
        0x2742 => 0x107534C0,
        0x2743 => 0x107534B0,
        0x2744 => 0x107534A0,
        0x2745 => 0x10753490,
        0x2746 => 0x10753480,
        0x2747 => 0x10753474,
        0x2748 => 0x10753468,
        0x2749 => 0x1075345C,
        0x274A => 0x1075344C,
        0x274B => 0x1075343C,
        0x274C => 0x1075342C,
        0x274D => 0x1075341C,
        0x274E => 0x10753410,
        0x274F => 0x10753400,
        0x2750 => 0x107533F0,
        0x2751 => 0x107533E0,
        0x2752 => 0x107533D0,
        0x2753 => 0x107533C4,
        0x2754 => 0x107533B8,
        0x2755 => 0x107533AC,
        0x2756 => 0x107533A0,
        0x2757 => 0x10753394,
        0x276B => 0x10753384,
        0x276C => 0x10753370,
        0x276D => 0x1075335C,
        0x2775 => 0x10753350,
        0x2AF9 => 0x1075333C,
        0x2AFA => 0x1075332C,
        0x2AFB => 0x1075331C,
        0x2AFC => 0x10753310,
        _ => 0x10753304,
    }
}

/// Real UDP socket, or simulated without network (sends lost, nothing to read), with its bound port.
#[derive(Debug)]
pub struct Sock(pub Option<UdpSocket>, pub u16);

impl Sock {
    pub fn send_to(&self, data: &[u8], to: SocketAddrV4) -> std::io::Result<usize> {
        match &self.0 {
            Some(s) => {
                let r = s.send_to(data, to);
                // Deliberate difference (LAN between two PCs): Windows only sends a 255.255.255.255
                // broadcast on the adapter with the lowest metric (a VPN adapter can
                // come before Wi-Fi), and the game is then never seen from the other PC. The same
                // broadcast is also sent to the broadcast address of each connected adapter.
                if *to.ip() == Ipv4Addr::BROADCAST && r.is_ok() {
                    for b in locals_broadcasts() {
                        let _ = s.send_to(data, SocketAddrV4::new(b, to.port()));
                    }
                }
                r
            }
            None => {
                if let Some(q) = run_loop() {
                    if let Ok(mut q) = q.lock() {
                        q.push((SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.1), to, data.to_vec()));
                    }
                }
                Ok(data.len())
            }
        }
    }
    fn loop_read(&self, buf: &mut [u8], take: bool) -> std::io::Result<(usize, std::net::SocketAddr)> {
        let Some(q) = run_loop() else { return Err(std::io::ErrorKind::WouldBlock.into()) };
        let Ok(mut q) = q.lock() else { return Err(std::io::ErrorKind::WouldBlock.into()) };
        let Some(i) = q.iter().position(|(_, to, _)| to.port() == self.1) else { return Err(std::io::ErrorKind::WouldBlock.into()) };
        let (from, _, data) = if take { q.remove(i) } else { q[i].clone() };
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok((n, std::net::SocketAddr::V4(from)))
    }
    /// WinSock's recvfrom: (bytes or −1, WSAGetLastError, source address). The address is
    /// filled in on error too (WSAECONNRESET from a "port unreachable"), which std does not
    /// return.
    pub fn recv_wsa(&self, buf: &mut [u8]) -> (i32, i32, SocketAddrV4) {
        let unknown = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        match &self.0 {
            #[cfg(windows)]
            Some(s) => {
                use std::os::windows::io::AsRawSocket;
                let mut from = wsa::SockAddrIn { family: 0, port: 0, addr: [0; 4], zero: [0; 8] };
                let mut len = std::mem::size_of::<wsa::SockAddrIn>() as i32;
                // SAFETY: open socket, buffer and address valid during the call.
                let n = unsafe { wsa::recvfrom(s.as_raw_socket() as usize, buf.as_mut_ptr(), buf.len() as i32, 0, &mut from, &mut len) };
                let err = if n == -1 { unsafe { wsa::WSAGetLastError() } } else { 0 };
                let a = from.addr;
                (n, err, SocketAddrV4::new(Ipv4Addr::new(a[0], a[1], a[2], a[3]), u16::from_be(from.port)))
            }
            #[cfg(not(windows))]
            Some(s) => match s.recv_from(buf) {
                Ok((n, std::net::SocketAddr::V4(f))) => (n as i32, 0, f),
                Ok(_) => (-1, WOULD_BLOCK, unknown),
                Err(e) => (-1, os_error(&e), unknown),
            },
            None => match self.loop_read(buf, true) {
                Ok((n, std::net::SocketAddr::V4(f))) => (n as i32, 0, f),
                Err(e) => (-1, e.raw_os_error().unwrap_or(WOULD_BLOCK), unknown),
                _ => (-1, WOULD_BLOCK, unknown),
            },
        }
    }

    pub fn recv_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, std::net::SocketAddr)> {
        match &self.0 {
            Some(s) => s.recv_from(buf),
            None => self.loop_read(buf, true),
        }
    }
    pub fn peek_from(&self, buf: &mut [u8]) -> std::io::Result<(usize, std::net::SocketAddr)> {
        match &self.0 {
            Some(s) => s.peek_from(buf),
            None => self.loop_read(buf, false),
        }
    }
}

#[cfg(windows)]
mod wsa {
    #[repr(C)]
    pub struct SockAddrIn {
        pub family: u16,
        pub port: u16,
        pub addr: [u8; 4],
        pub zero: [u8; 8],
    }
    #[link(name = "ws2_32")]
    extern "system" {
        pub fn recvfrom(s: usize, buf: *mut u8, len: i32, flags: i32, from: *mut SockAddrIn, fromlen: *mut i32) -> i32;
        pub fn WSAGetLastError() -> i32;
        pub fn inet_addr(cp: *const u8) -> u32;
        pub fn setsockopt(s: usize, level: i32, name: i32, val: *const i32, len: i32) -> i32;
        pub fn getsockopt(s: usize, level: i32, name: i32, val: *mut i32, len: *mut i32) -> i32;
        pub fn gethostbyname(name: *const u8) -> *const HostEnt;
        pub fn gethostname(name: *mut u8, len: i32) -> i32;
        pub fn socket(af: i32, kind: i32, protocol: i32) -> usize;
        pub fn bind(s: usize, name: *const SockAddrIn, len: i32) -> i32;
        pub fn closesocket(s: usize) -> i32;
    }
    #[repr(C)]
    pub struct HostEnt {
        pub h_name: *const u8,
        pub h_aliases: *const *const u8,
        pub h_addrtype: i16,
        pub h_length: i16,
        pub h_addr_list: *const *const u8,
    }
}

/// IPv4 addresses of the connected network adapters (neither loopback nor 169.254 without a DHCP server),
/// with their mask: GetIpAddrTable (iphlpapi). OpenXIII-specific: LAN broadcast on each
/// adapter and addresses shown by the dedicated server.
pub fn locals_addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    #[cfg(windows)]
    {
        #[link(name = "iphlpapi")]
        extern "system" {
            fn GetIpAddrTable(table: *mut u8, size: *mut u32, order: i32) -> u32;
        }
        let mut size = 0u32;
        // SAFETY: first call for the size (ERROR_INSUFFICIENT_BUFFER), then a buffer of that size.
        unsafe { GetIpAddrTable(std::ptr::null_mut(), &mut size, 1) };
        if size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize + 64];
        let mut size2 = buf.len() as u32;
        if unsafe { GetIpAddrTable(buf.as_mut_ptr(), &mut size2, 1) } != 0 {
            return Vec::new();
        }
        let u = |o: usize| u32::from_ne_bytes(buf[o..o + 4].try_into().unwrap());
        let n = u(0) as usize;
        let mut out = Vec::new();
        for i in 0..n {
            // MIB_IPADDRROW: dwAddr, dwIndex, dwMask, dwBCastAddr, dwReasmSize, unused1, wType (24 bytes).
            let o = 4 + i * 24;
            if o + 24 > buf.len() {
                break;
            }
            let addr = Ipv4Addr::from(u(o).to_ne_bytes());
            let mask = Ipv4Addr::from(u(o + 8).to_ne_bytes());
            let kind = u16::from_ne_bytes([buf[o + 22], buf[o + 23]]);
            // MIB_IPADDR_DISCONNECTED (0x8), MIB_IPADDR_DELETED (0x40).
            if kind & 0x48 != 0 || addr.is_loopback() || addr.is_link_local() || addr.is_unspecified() {
                continue;
            }
            out.push((addr, mask));
        }
        out
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // getifaddrs (Android, Linux): same list as GetIpAddrTable, adapters that are up only.
        #[repr(C)]
        struct IfAddrs {
            next: *mut IfAddrs,
            name: *const u8,
            flags: u32,
            addr: *const SockAddr,
            netmask: *const SockAddr,
            ifu: *const u8,
            data: *const u8,
        }
        #[repr(C)]
        struct SockAddr {
            family: u16,
            port: u16,
            addr: [u8; 4],
        }
        extern "C" {
            fn getifaddrs(out: *mut *mut IfAddrs) -> i32;
            fn freeifaddrs(ifa: *mut IfAddrs);
        }
        const IFF_UP: u32 = 0x1;
        const AF_INET: u16 = 2;
        let mut out = Vec::new();
        let mut head_value: *mut IfAddrs = std::ptr::null_mut();
        // SAFETY: list returned by getifaddrs, read before freeifaddrs; sockaddr_in read only if its family is AF_INET.
        unsafe {
            if getifaddrs(&mut head_value) != 0 {
                return Vec::new();
            }
            let mut i = head_value;
            while !i.is_null() {
                let e = &*i;
                if e.flags & IFF_UP != 0 && !e.addr.is_null() && !e.netmask.is_null() && (*e.addr).family == AF_INET {
                    let addr = Ipv4Addr::from((*e.addr).addr);
                    let mask = Ipv4Addr::from((*e.netmask).addr);
                    if !(addr.is_loopback() || addr.is_link_local() || addr.is_unspecified()) {
                        out.push((addr, mask));
                    }
                }
                i = e.next;
            }
            freeifaddrs(head_value);
        }
        out
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    {
        Vec::new()
    }
}

/// Broadcast address of each connected adapter (address | !mask), without duplicates.
pub fn locals_broadcasts() -> Vec<Ipv4Addr> {
    broadcasts(&locals_addresses())
}

/// Broadcast address of each (address, mask), without duplicates, nor 255.255.255.255, nor the address itself (/32).
fn broadcasts(addresses: &[(Ipv4Addr, Ipv4Addr)]) -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for &(a, m) in addresses {
        let b = Ipv4Addr::from(u32::from(a) | !u32::from(m));
        if b != Ipv4Addr::BROADCAST && b != a && !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod broadcasts_tests {
    use super::*;

    #[test]
    fn broadcast_by_map() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        let maps = [
            (ip("192.0.2.10"), ip("255.255.255.0")),
            (ip("10.20.30.40"), ip("255.0.0.0")),
            (ip("203.0.113.17"), ip("255.255.240.0")),
            (ip("192.0.2.11"), ip("255.255.255.0")),
            (ip("198.51.100.5"), ip("255.255.255.255")),
            (ip("198.51.100.6"), ip("0.0.0.0")),
        ];
        assert_eq!(broadcasts(&maps), vec![ip("192.0.2.255"), ip("10.255.255.255"), ip("203.0.127.255")]);
        // The machine's real list never contains loopback or 169.254.
        for (a, _) in locals_addresses() {
            assert!(!a.is_loopback() && !a.is_link_local());
        }
    }
}

/// Host name resolution in progress (FResolveInfo): filled in by the resolver
/// thread, address in network order or error text.
pub type ResolveResult = std::sync::Arc<std::sync::Mutex<Option<Result<[u8; 4], String>>>>;

/// FResolveInfo: "Resolving %s..." then a thread: gethostbyname
/// up to 3 times, one second of waiting between two tries except for WSAHOST_NOT_FOUND and
/// WSANO_DATA; failure or a non-AF_INET address → "Can't find host %s (%s)".
pub(crate) fn resolve_host(host: &str) -> ResolveResult {
    let out: ResolveResult = Default::default();
    let result = out.clone();
    let name: Vec<u8> = host.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).chain(std::iter::once(0)).collect();
    let host = host.to_string();
    std::thread::spawn(move || {
        let found = resolve_thread(&name);
        let r = found.map_err(|e| format!("Can't find host {host} ({})", socket_error_text(e)));
        if let Ok(mut o) = result.lock() {
            *o = Some(r);
        }
    });
    out
}

#[cfg(windows)]
fn resolve_thread(name: &[u8]) -> Result<[u8; 4], i32> {
    let mut err = 0;
    for _ in 0..3 {
        let h = unsafe { wsa::gethostbyname(name.as_ptr()) };
        if !h.is_null() {
            let h = unsafe { &*h };
            if h.h_addrtype != 2 {
                return Err(err);
            }
            let a = unsafe { std::slice::from_raw_parts(*h.h_addr_list, 4) };
            return Ok([a[0], a[1], a[2], a[3]]);
        }
        err = unsafe { wsa::WSAGetLastError() };
        if err == 0x2AF9 || err == 0x2AFC {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Err(err)
}

#[cfg(not(windows))]
fn resolve_thread(name: &[u8]) -> Result<[u8; 4], i32> {
    use std::net::ToSocketAddrs;
    let host = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
    (host.as_str(), 0).to_socket_addrs().ok().and_then(|mut a| a.find_map(|a| match a {
        std::net::SocketAddr::V4(v) => Some(v.ip().octets()),
        _ => None,
    })).ok_or(0x2AF9)
}

/// UTcpipConnection::UTcpipConnection: a host opened by the client is only read by
/// inet_addr if it has the form of four numbers separated by dots (each part
/// starts with a digit, nothing after the fourth), otherwise it is resolved.
pub(crate) fn host_is_dotted(host: &str) -> bool {
    let b = host.as_bytes();
    let mut p = Some(0usize);
    let mut i = 0;
    while i < 4 {
        match p {
            Some(k) if b.get(k).is_some_and(|c| c.is_ascii_digit()) => {
                p = b[k..].iter().position(|&c| c == b'.').map(|j| k + j + 1);
            }
            _ => break,
        }
        i += 1;
    }
    i == 4 && p.is_none()
}

/// What getlocalhostaddr returns: HostAddr, bCanBindAll (the result), the messages
/// written to the caller's Ar (Logf, so NAME_Log) and GLog's "Init: WinSock: I am …" line.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct HostLocal {
    pub addr: [u8; 4],
    pub bind_all: bool,
    pub ar: Vec<String>,
    pub init: Option<String>,
}

/// Winsock as seen by getlocalhostaddr: gethostname (256 bytes), gethostbyname (h_addrtype and
/// first h_addr_list), WSAGetLastError (SocketError(-1)).
pub(crate) trait WinsockHost {
    fn gethostname(&mut self) -> Option<Vec<u8>>;
    fn gethostbyname(&mut self, name_value: &[u8]) -> Option<(i16, [u8; 4])>;
    fn last_error(&mut self) -> i32;
}

/// getlocalhostaddr. gethostname failing: "WinSock: gethostname failed (…)"
/// and the name stays empty. MULTIHOME= (Core's Parse, 255 characters): three dots are needed
/// (appStrchr), otherwise "Invalid multihome IP address …"; the four parts go through appAtoi
/// (low byte kept, IpSetBytes) and bCanBindAll stays false, without "I am". Otherwise
/// gethostbyname of the name: failure or a family other than AF_INET (2) write their message (text of
/// WSAGetLastError) and return false with the null address; found: the address, bCanBindAll =
/// no -PRIMARYNET, and the first time only "I am Name (a.b.c.d)".
pub(crate) fn get_local_host_addr(cmd: &str, ws: &mut dyn WinsockHost, already: &mut bool) -> HostLocal {
    let mut r = HostLocal::default();
    let name_value = match ws.gethostname() {
        Some(n) => n,
        None => {
            let e = ws.last_error();
            r.ar.push(format!("WinSock: gethostname failed ({})", socket_error_text(e)));
            Vec::new()
        }
    };
    if let Some(home) = crate::value::parse_fstring(cmd, "MULTIHOME=") {
        let home: Vec<char> = home.chars().take(255).collect();
        let point = |since_index: usize| home[since_index..].iter().position(|&c| c == '.').map(|i| since_index + i + 1);
        match point(0).and_then(|a| point(a).map(|b| (a, b))).and_then(|(a, b)| point(b).map(|c| (a, b, c))) {
            Some((a, b, c)) => {
                let n = |k: usize| crate::config::atoi(&home[k..], 0) as u8;
                r.addr = [n(0), n(a), n(b), n(c)];
            }
            None => r.ar.push(format!("Invalid multihome IP address {}", home.iter().collect::<String>())),
        }
        return r;
    }
    let Some((family_value, adr)) = ws.gethostbyname(&name_value) else {
        let e = ws.last_error();
        r.ar.push(format!("gethostbyname failed ({})", socket_error_text(e)));
        return r;
    };
    if family_value != 2 {
        let e = ws.last_error();
        r.ar.push(format!("gethostbyname: non-Internet address ({})", socket_error_text(e)));
        return r;
    }
    r.addr = adr;
    r.bind_all = !crate::app_init::parse_param(cmd, "PRIMARYNET");
    if !*already {
        *already = true;
        let name_value: String = name_value.iter().map(|&c| c as char).collect();
        r.init = Some(format!("Init: WinSock: I am {name_value} ({})", ip_string(adr, 0)));
    }
    r
}

/// The machine's Winsock for getlocalhostaddr.
struct WinsockMachine;

impl WinsockHost for WinsockMachine {
    fn gethostname(&mut self) -> Option<Vec<u8>> {
        #[cfg(windows)]
        {
            let mut buf = [0u8; 256];
            if unsafe { wsa::gethostname(buf.as_mut_ptr(), 256) } != 0 {
                return None;
            }
            Some(buf.iter().copied().take_while(|&c| c != 0).collect())
        }
        #[cfg(not(windows))]
        {
            Some(openxiii_package::env::var("HOSTNAME").unwrap_or_default().into_bytes())
        }
    }

    fn gethostbyname(&mut self, name_value: &[u8]) -> Option<(i16, [u8; 4])> {
        #[cfg(windows)]
        {
            let z: Vec<u8> = name_value.iter().copied().chain(std::iter::once(0)).collect();
            let h = unsafe { wsa::gethostbyname(z.as_ptr()) };
            if h.is_null() {
                return None;
            }
            unsafe {
                let p = *(*h).h_addr_list;
                Some(((*h).h_addrtype, if p.is_null() { [0; 4] } else { [*p, *p.add(1), *p.add(2), *p.add(3)] }))
            }
        }
        #[cfg(not(windows))]
        {
            // Android has neither HOSTNAME nor a name resolved to the Wi-Fi ("localhost" gives 127.0.0.1):
            // without a name, or on loopback, the address of the first connected adapter.
            use std::net::ToSocketAddrs;
            let name_value = String::from_utf8_lossy(name_value);
            let resolved = if name_value.is_empty() {
                None
            } else {
                (name_value.as_ref(), 0).to_socket_addrs().ok().and_then(|mut l| l.find_map(|a| match a {
                    std::net::SocketAddr::V4(v) => Some(*v.ip()),
                    _ => None,
                }))
            };
            match resolved {
                Some(ip) if !ip.is_loopback() => Some((2, ip.octets())),
                _ => locals_addresses().first().map(|(a, _)| (2, a.octets())).or(resolved.map(|ip| (2, ip.octets()))),
            }
        }
    }

    fn last_error(&mut self) -> i32 {
        #[cfg(windows)]
        {
            unsafe { wsa::WSAGetLastError() }
        }
        #[cfg(not(windows))]
        {
            0x2AF9
        }
    }
}

thread_local!(static I_AM_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

/// getlocalbindaddr: getlocalhostaddr on the game's command line
/// (OXIII_COMMAND_LINE), then 0.0.0.0 (INADDR_ANY) if bCanBindAll.
pub(crate) fn local_bind_addr_host() -> HostLocal {
    let line = openxiii_package::env::var("OXIII_COMMAND_LINE").unwrap_or_default();
    let mut already = I_AM_DONE.with(|f| f.get());
    let r = local_bind_addr(&line, &mut WinsockMachine, &mut already);
    I_AM_DONE.with(|f| f.set(already));
    r
}

/// getlocalbindaddr: getlocalhostaddr, then, if it returns bCanBindAll, IpSetInt(0)
/// the returned address is 0.0.0.0 (INADDR_ANY).
pub(crate) fn local_bind_addr(line: &str, ws: &mut dyn WinsockHost, already: &mut bool) -> HostLocal {
    let mut r = get_local_host_addr(line, ws, already);
    if r.bind_all {
        r.addr = [0; 4];
    }
    r
}

/// Fields that AInternetLink::AInternetLink writes after AInfo::AInfo and InitSockets, in order:
/// LinkMode (+0x1F0), ReceiveMode (+0x208), DataPending (+0x204), Port (+0x1F8), Socket (+0x1F4), RemoteSocket
/// (+0x1FC). PrivateResolveInfo (+0x200) is not touched.
pub(crate) fn internet_link_init() -> [(&'static str, Value); 6] {
    [
        ("LinkMode", Value::Byte(0)),
        ("ReceiveMode", Value::Byte(1)),
        ("DataPending", Value::Int(0)),
        ("Port", Value::Int(0)),
        ("Socket", Value::Int(-1)),
        ("RemoteSocket", Value::Int(-1)),
    ]
}

/// Start of AInternetLink::execResolve depending on inet_addr(Domain) (`inet`, network order).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResolveStart {
    /// eventResolveFailed right away: INADDR_NONE for "255.255.255.255" itself (FString::operator!= false).
    Failure,
    /// eventResolved(IpAddr(ntohl(inet), 0)) right away.
    Resolved(u32),
    /// FResolveInfo ("Resolving %s...", resolver thread) in PrivateResolveInfo.
    Search,
}

pub(crate) fn resolve_start(domain: &str, inet: u32) -> ResolveStart {
    if inet != u32::MAX {
        ResolveStart::Resolved(u32::from_be(inet))
    } else if domain != "255.255.255.255" {
        ResolveStart::Search
    } else {
        ResolveStart::Failure
    }
}

/// End of a resolution as seen by AInternetLink::Tick, once the thread is done (FResolveInfo: Error at +0x108,
/// Addr at +0): a non-empty error text gives "AInternetLink Resolve failed" and eventResolveFailed, otherwise
/// "Resolved %s (%s)" and eventResolved(ntohl(IpGetInt(Addr)), 0) (IpGetInt: the address's integer as is). Returns the log line and the address of the Resolved event
/// (host order); PrivateResolveInfo is then freed.
pub(crate) fn resolve_end(host: &str, error: &str, addr: [u8; 4]) -> (String, Option<u32>) {
    if !error.is_empty() {
        (format!("Log: AInternetLink Resolve failed: {error}"), None)
    } else {
        (format!("Log: Resolved {host} ({})", ip_string(addr, 0)), Some(u32::from_be_bytes(addr)))
    }
}

/// WSAStartup data read by InitSockets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WsaData {
    pub version: u16,
    pub high_version: u16,
    pub max_sockets: u16,
    pub max_udp: u16,
}

/// WSADATA returned by WinSock 2 for a 1.1 request: wVersion 1.1, wHighVersion 2.2,
/// iMaxSockets 32767, iMaxUdpDg 65467.
pub(crate) const WSADATA_MACHINE: WsaData = WsaData { version: 0x0101, high_version: 0x0202, max_sockets: 32767, max_udp: 65467 };

/// InitSockets: WSAStartup(1.1) attempted only once (a flag set before
/// the call: a failure is not retried); on success, GSocketsStarted and "Init: WinSock: version
/// a.b (c.d), MaxSocks=…, MaxUdp=…" (high byte of wVersion first); on failure, Error = "WSAStartup
/// failed (text of SocketError)". Returns GSocketsStarted. (The registration of the IPDRV_* names is
/// only Core's name table.)
pub(crate) fn init_sockets(done: &mut bool, started: &mut bool, wsa_startup: impl FnOnce() -> Result<WsaData, i32>, log: &mut Vec<String>, error: &mut Option<String>) -> bool {
    if !*done {
        *done = true;
        match wsa_startup() {
            Ok(d) => {
                *started = true;
                let (v, h) = (d.version, d.high_version);
                log.push(format!("Init: WinSock: version {}.{} ({}.{}), MaxSocks={}, MaxUdp={}", v >> 8, v & 0xFF, h >> 8, h & 0xFF, d.max_sockets, d.max_udp));
            }
            Err(code) => *error = Some(format!("WSAStartup failed ({})", socket_error_text(code))),
        }
    }
    *started
}

/// IpString: "a.b.c.d" (bytes in network order), then ":%i" if the
/// port is not zero.
pub(crate) fn ip_string(addr: [u8; 4], port: i32) -> String {
    let mut s = format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]);
    if port != 0 {
        s.push_str(&format!(":{port}"));
    }
    s
}

/// Text of SocketError, at the address of socket_error_text_address.
pub(crate) fn socket_error_text(code: i32) -> &'static str {
    match code {
        0 => "WSANO_ERROR",
        0x2714 => "WSAEINTR",
        0x2719 => "WSAEBADF",
        0x271D => "WSAEACCES",
        0x271E => "WSAEFAULT",
        0x2726 => "WSAEINVAL",
        0x2728 => "WSAEMFILE",
        0x2733 => "WSAEWOULDBLOCK",
        0x2734 => "WSAEINPROGRESS",
        0x2735 => "WSAEALREADY",
        0x2736 => "WSAENOTSOCK",
        0x2737 => "WSAEDESTADDRREQ",
        0x2738 => "WSAEMSGSIZE",
        0x2739 => "WSAEPROTOTYPE",
        0x273A => "WSAENOPROTOOPT",
        0x273B => "WSAEPROTONOSUPPORT",
        0x273C => "WSAESOCKTNOSUPPORT",
        0x273D => "WSAEOPNOTSUPP",
        0x273E => "WSAEPFNOSUPPORT",
        0x273F => "WSAEAFNOSUPPORT",
        0x2740 => "WSAEADDRINUSE",
        0x2741 => "WSAEADDRNOTAVAIL",
        0x2742 => "WSAENETDOWN",
        0x2743 => "WSAENETUNREACH",
        0x2744 => "WSAENETRESET",
        0x2745 => "WSAECONNABORTED",
        0x2746 => "WSAECONNRESET",
        0x2747 => "WSAENOBUFS",
        0x2748 => "WSAEISCONN",
        0x2749 => "WSAENOTCONN",
        0x274A => "WSAESHUTDOWN",
        0x274B => "WSAETOOMANYREFS",
        0x274C => "WSAETIMEDOUT",
        0x274D => "WSAECONNREFUSED",
        0x274E => "WSAELOOP",
        0x274F => "WSAENAMETOOLONG",
        0x2750 => "WSAEHOSTDOWN",
        0x2751 => "WSAEHOSTUNREACH",
        0x2752 => "WSAENOTEMPTY",
        0x2753 => "WSAEPROCLIM",
        0x2754 => "WSAEUSERS",
        0x2755 => "WSAEDQUOT",
        0x2756 => "WSAESTALE",
        0x2757 => "WSAEREMOTE",
        0x276B => "WSASYSNOTREADY",
        0x276C => "WSAVERNOTSUPPORTED",
        0x276D => "WSANOTINITIALISED",
        0x2775 => "WSAEDISCON",
        0x2AF9 => "WSAHOST_NOT_FOUND",
        0x2AFA => "WSATRY_AGAIN",
        0x2AFB => "WSANO_RECOVERY",
        0x2AFC => "WSANO_DATA",
        _ => "WSA_Unknown",
    }
}

/// UTcpNetDriver::InitBase: SO_RCVBUF then SO_SNDBUF (SOL_SOCKET) set to `size`
/// (0x8000 as a client, 0x20000 when listening), each read back by getsockopt; the length returned by
/// the first read is used for the setsockopt of SO_SNDBUF. Without a socket (loop), the requested
/// values.
pub(crate) fn socket_queue(sock: &Sock, size: i32) -> (i32, i32) {
    #[cfg(windows)]
    if let Some(s) = &sock.0 {
        let h = std::os::windows::io::AsRawSocket::as_raw_socket(s) as usize;
        let (mut rcv, mut snd, mut len) = (size, size, 4i32);
        unsafe {
            wsa::setsockopt(h, 0xffff, 0x1002, &rcv, 4);
            wsa::getsockopt(h, 0xffff, 0x1002, &mut rcv, &mut len);
            wsa::setsockopt(h, 0xffff, 0x1001, &snd, len);
            wsa::getsockopt(h, 0xffff, 0x1001, &mut snd, &mut len);
        }
        return (rcv, snd);
    }
    let _ = sock;
    (size, size)
}

/// Parse(appCmdLine(), "PORT=", Port) of InitBase: the game's command line
/// is OXIII_COMMAND_LINE (empty by default); "PORT=" searched case-insensitively, then appAtoi.
fn cmd_line_port() -> Option<u16> {
    let line = openxiii_package::env::var("OXIII_COMMAND_LINE").ok()?;
    let at = line.to_ascii_uppercase().find("PORT=")?;
    let rest: Vec<char> = line[at + 5..].chars().collect();
    Some(crate::config::atoi(&rest, 0) as u16)
}

/// Winsock's inet_addr (wsock32 ordinal 10) on appToAnsi(S): short forms ("127.1"),
/// decimal, octal and hexadecimal, stopping at a space; INADDR_NONE (0xFFFFFFFF) on failure.
/// Value in network order, as Winsock returns it.
pub(crate) fn inet_addr(s: &str) -> u32 {
    let ansi: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take_while(|&b| b != 0).chain(std::iter::once(0)).collect();
    #[cfg(windows)]
    {
        unsafe { wsa::inet_addr(ansi.as_ptr()) }
    }
    #[cfg(not(windows))]
    {
        let text = String::from_utf8_lossy(&ansi[..ansi.len() - 1]).into_owned();
        text.split(' ').next().unwrap_or("").parse::<Ipv4Addr>().map_or(u32::MAX, |ip| u32::from_ne_bytes(ip.octets()))
    }
}

/// Ports reserved by the simulated sockets of all the VMs of the loop (a single system).
static LOOP_PORTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u16>>> = std::sync::OnceLock::new();

fn offline_claim(ip: &mut IpDrv, port: u16) -> bool {
    if run_loop().is_some() {
        return LOOP_PORTS.get_or_init(Default::default).lock().is_ok_and(|mut s| s.insert(port));
    }
    ip.offline_ports.insert(port)
}

fn offline_taken(ip: &IpDrv, port: u16) -> bool {
    if run_loop().is_some() {
        return LOOP_PORTS.get_or_init(Default::default).lock().is_ok_and(|s| s.contains(&port));
    }
    ip.offline_ports.contains(&port)
}

pub(crate) fn offline_release(ip: &mut IpDrv, port: u16) {
    if run_loop().is_some() {
        if let Ok(mut s) = LOOP_PORTS.get_or_init(Default::default).lock() {
            s.remove(&port);
        }
        return;
    }
    ip.offline_ports.remove(&port);
}

fn os_error(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(if e.kind() == std::io::ErrorKind::WouldBlock { WOULD_BLOCK } else { 0x2726 })
}

/// The script's IpAddr { Addr, Port }.
pub(crate) fn ip_addr_value(addr: u32, port: i32) -> Value {
    Value::Struct("internetlink.ipaddr".into(), vec![Value::Int(addr as i32), Value::Int(port)].into())
}

fn ip_addr_of(v: &Value) -> (u32, i32) {
    match v {
        Value::Struct(_, f) => (f.first().map_or(0, |v| v.as_int()) as u32, f.get(1).map_or(0, |v| v.as_int())),
        _ => (0, 0),
    }
}

/// appToAnsi / appFromAnsi: one byte per character (Latin-1).
fn to_ansi(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

fn from_ansi(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}

/// UDP socket (AF_INET, SOCK_DGRAM) bound to 0.0.0.0:port like: `tries` tries,
/// port + 1 after each failure (port 0: a single try, the system chooses).
type Datagram = (SocketAddrV4, SocketAddrV4, Vec<u8>);

pub fn run_loop() -> Option<&'static std::sync::Mutex<Vec<Datagram>>> {
    None
}

/// UDP socket bound to `addr` with SO_REUSEADDR set before the bind, like UTcpNetDriver::InitBase
/// (setsockopt through): on Windows, a port already taken by another
/// socket without SO_EXCLUSIVEADDRUSE binds anyway.
#[cfg(windows)]
fn bind_reuse(addr: SocketAddrV4) -> std::io::Result<UdpSocket> {
    use std::os::windows::io::FromRawSocket;
    // WSAStartup through the standard library (Winsock's counter), before the first raw socket.
    let _ = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    unsafe {
        let s = wsa::socket(2, 2, 17);
        if s == usize::MAX {
            return Err(std::io::Error::from_raw_os_error(wsa::WSAGetLastError()));
        }
        let one = 1i32;
        wsa::setsockopt(s, 0xffff, 4, &one, 4);
        let its = wsa::SockAddrIn { family: 2, port: addr.port().to_be(), addr: addr.ip().octets(), zero: [0; 8] };
        if wsa::bind(s, &its, 16) != 0 {
            let err = wsa::WSAGetLastError();
            wsa::closesocket(s);
            return Err(std::io::Error::from_raw_os_error(err));
        }
        Ok(UdpSocket::from_raw_socket(s as std::os::windows::io::RawSocket))
    }
}

/// bindnextport: `tries` tries starting from `port` (port 0: a single one);
/// `reuse`: SO_REUSEADDR before the bind (the network driver's socket, not a UdpLink's).
pub(crate) fn bind_udp(ip: &mut IpDrv, port: u16, tries: i32, reuse: bool) -> Result<(Sock, u16), i32> {
    bind_udp_test(ip, port, tries, reuse).map(|(s, bound, _)| (s, bound))
}

/// bind_udp, and the port of the try that succeeded (sin_port of bindnextport's sockaddr: 0 for a port chosen by the
/// system, which getsockname returns without touching the sockaddr).
pub(crate) fn bind_udp_test(ip: &mut IpDrv, port: u16, tries: i32, reuse: bool) -> Result<(Sock, u16, u16), i32> {
    let mut p = port;
    let mut last = 0;
    for _ in 0..tries.max(1) {
        if ip.offline {
            // Free port in this process (port 0: an ephemeral port).
            let q = if p == 0 { (49152..65535).find(|&q| !offline_taken(ip, q)).unwrap_or(49152) } else { p };
            if offline_claim(ip, q) {
                return Ok((Sock(None, q), q, p));
            }
            last = 0x2740;
            p = p.wrapping_add(1);
            continue;
        }
        // OXIII_NETWORK_ADDRESS=127.0.0.1 (testing on a single machine): sockets bound to this
        // address instead of all of them (no Windows firewall prompt). Tests (cfg(test))
        // only need loopback: same rule by default; the game, LAN and server keep 0.0.0.0.
        let default_value = if cfg!(test) { Ipv4Addr::LOCALHOST } else { Ipv4Addr::UNSPECIFIED };
        let bind_ip = openxiii_package::env::var("OXIII_NETWORK_ADDRESS").ok().and_then(|v| v.parse().ok()).unwrap_or(default_value);
        #[cfg(windows)]
        let bound = if reuse { bind_reuse(SocketAddrV4::new(bind_ip, p)) } else { UdpSocket::bind(SocketAddrV4::new(bind_ip, p)) };
        #[cfg(not(windows))]
        let bound = { let _ = reuse; UdpSocket::bind(SocketAddrV4::new(bind_ip, p)) };
        match bound {
            Ok(s) => {
                let bound = s.local_addr().map(|a| a.port()).unwrap_or(p);
                return Ok((Sock(Some(s), bound), bound, p));
            }
            Err(e) => {
                last = os_error(&e);
                if p == 0 {
                    return Err(last);
                }
                p = p.wrapping_add(1);
            }
        }
    }
    Err(last)
}

impl Vm {
    /// Default object of UTcpNetDriver (net_driver_values on [IpDrv.TcpNetDriver] then
    /// [Engine.NetDriver] of System, Core's localized True / False words).
    pub(crate) fn net_driver_config(&mut self) -> NetDriverCfg {
        let (true_value, false_value) = self.bool_words();
        let config = &mut self.config;
        let mut lookup = |key: &str| -> Option<Vec<char>> {
            let c = config.as_mut()?;
            c.get_string("IpDrv.TcpNetDriver", key, "System").or_else(|| c.get_string("Engine.NetDriver", key, "System")).map(|v| v.chars().collect())
        };
        let mut cfg = NetDriverCfg::from_words(&net_driver_values(&mut lookup, &true_value, &false_value));
        cfg.download_rate = lookup("OpenXIIIDownloadRate").map_or(0, |v| crate::config::atoi(&v, 0).max(0));
        // OpenXIII testing: OXIII_DOWNLOADRATE replaces the key (without touching the game's ini).
        if let Some(v) = openxiii_package::env::var("OXIII_DOWNLOADRATE").ok().and_then(|v| v.parse::<i32>().ok()) {
            cfg.download_rate = v.max(0);
        }
        if self.ipdrv.downloads {
            cfg.allow_downloads = true;
        }
        cfg
    }

    /// InitSockets on the machine's WinSock (WSADATA_MACHINE); the Error of a failure
    /// is read by nobody here (InitListen and AInternetLink only look at the result).
    fn ip_init_sockets(&mut self) -> bool {
        let (mut done, mut started) = (self.ipdrv.wsa_done, self.ipdrv.started);
        let mut error = None;
        let r = init_sockets(&mut done, &mut started, || Ok(WSADATA_MACHINE), &mut self.log, &mut error);
        self.ipdrv.wsa_done = done;
        self.ipdrv.started = started;
        r
    }

    /// AInternetLink::AInternetLink: native fields set after the default values are copied.
    pub(crate) fn internet_link_construct(&mut self, a: ObjId) {
        self.ip_init_sockets();
        for (name_value, v) in internet_link_init() {
            self.set_prop(a, name_value, v);
        }
    }

    fn link_socket(&self, a: ObjId) -> i32 {
        self.propr(a, "Socket").as_int()
    }

    /// Natives of AInternetLink and AUdpLink; None if `name` is not one of them for `this`.
    pub(crate) fn ipdrv_native(&mut self, name: &str, this: ObjId, args: &mut [Value]) -> Option<Value> {
        if !self.is_a(this, "InternetLink") {
            return None;
        }
        let udp = self.is_a(this, "UdpLink");
        Some(match name {
            // AInternetLink::execIsDataPending.
            "IsDataPending" => Value::Bool(self.propr(this, "DataPending").as_int() != 0),
            // AInternetLink::execGetLastError: WSAGetLastError.
            "GetLastError" => Value::Int(self.ipdrv.last_error),
            // AInternetLink::execIpAddrToString: "%i.%i.%i.%i:%i", bytes of Addr from most to least significant, then
            // Port (always written, even when zero).
            "IpAddrToString" => {
                let (addr, port) = args.first().map(ip_addr_of).unwrap_or((0, 0));
                let b = addr.to_be_bytes();
                Value::Str(format!("{}.{}.{}.{}:{port}", b[0], b[1], b[2], b[3]))
            }
            // AInternetLink::execStringToIpAddr: inet_addr; -1 (INADDR_NONE) returns
            // False without touching Addr, otherwise Addr = htonl(result) (ordinal 8), Port 0.
            "StringToIpAddr" => {
                let s = args.first().map(|v| v.as_name()).unwrap_or_default();
                let addr = inet_addr(&s);
                if addr == u32::MAX {
                    Value::Bool(false)
                } else {
                    if let Some(a) = args.get_mut(1) {
                        *a = ip_addr_value(u32::from_be(addr), 0);
                    }
                    Value::Bool(true)
                }
            }
            // AInternetLink::execResolve.
            "Resolve" => {
                let domain = args.first().map(|v| v.to_string()).unwrap_or_default();
                match resolve_start(&domain, inet_addr(&domain)) {
                    ResolveStart::Failure => self.event_pub(this, "ResolveFailed", vec![]),
                    ResolveStart::Resolved(addr) => self.event_pub(this, "Resolved", vec![ip_addr_value(addr, 0)]),
                    ResolveStart::Search => {
                        // FResolveInfo: GMalloc of 0x208 bytes, then the thread.
                        self.log.push(format!("Log: Resolving {domain}..."));
                        let r = resolve_host(&domain);
                        self.ipdrv.resolves.insert(this, (domain, r));
                    }
                }
                Value::Void
            }
            "BindPort" if udp => Value::Int(self.udp_bind_port(this, args)),
            "SendText" if udp => Value::Bool(self.udp_send_text(this, args)),
            "SendBinary" if udp => Value::Bool(self.udp_send_binary(this, args)),
            "ReadText" if udp => Value::Int(self.udp_read_text(this, args)),
            "ReadBinary" if udp => Value::Int(self.udp_read_binary(this, args)),
            _ => return None,
        })
    }

    /// getlocalbindaddr before each bind of UdpLink::BindPort (Ar: the script's
    /// stack, `frame`) and of UTcpNetDriver::InitBase (Ar: GLog, "Log:" lines).
    pub(crate) fn log_local_bind_addr(&mut self, frame: bool) -> [u8; 4] {
        let r = local_bind_addr_host();
        for m in &r.ar {
            if frame {
                self.native_warn(m);
            } else {
                self.log.push(format!("Log: {m}"));
            }
        }
        if let Some(l) = r.init {
            self.log.push(l);
        }
        r.addr
    }

    /// AUdpLink::execBindPort.
    fn udp_bind_port(&mut self, this: ObjId, args: &[Value]) -> i32 {
        let port = args.first().map_or(0, |v| v.as_int());
        let next = args.get(1).is_some_and(|v| v.as_bool());
        if !self.ipdrv.started {
            self.native_warn("BindPort: winsock failed");
            return 0;
        }
        if self.link_socket(this) != -1 {
            self.native_warn("BindPort: already bound");
            return 0;
        }
        self.log_local_bind_addr(true);
        match bind_udp_test(&mut self.ipdrv, port as u16, if next { 20 } else { 1 }, false) {
            Ok((s, bound, test_value)) => {
                // SO_BROADCAST then ioctlsocket(FIONBIO).
                if s.0.as_ref().is_some_and(|s| s.set_broadcast(true).is_err()) {
                    self.native_warn("BindPort: setsockopt failed");
                    self.set_prop(this, "Socket", Value::Int(0));
                    return 0;
                }
                if s.0.as_ref().is_some_and(|s| s.set_nonblocking(true).is_err()) {
                    self.native_warn("BindPort: ioctlsocket failed");
                    self.set_prop(this, "Socket", Value::Int(0));
                    return 0;
                }
                self.ipdrv.next_handle += 4;
                let handle = 0x100 + self.ipdrv.next_handle;
                self.ipdrv.udp.insert(this, s);
                self.set_prop(this, "Socket", Value::Int(handle));
                // Port = ntohs(sin_port) of the sockaddr: the port tried, 0 for a port chosen by the system;
                // BindPort returns the bound port (getsockname of bindnextport).
                self.set_prop(this, "Port", Value::Int(test_value as i32));
                bound as i32
            }
            Err(e) => {
                self.ipdrv.last_error = e;
                self.native_warn("BindPort: bind failed");
                // closesocket, then Socket = 0 (and not INVALID_SOCKET) like the original code.
                self.set_prop(this, "Socket", Value::Int(0));
                0
            }
        }
    }

    fn udp_target(args: &[Value]) -> SocketAddrV4 {
        let (addr, port) = args.first().map(ip_addr_of).unwrap_or((0, 0));
        SocketAddrV4::new(Ipv4Addr::from(addr), port as u16)
    }

    /// AUdpLink::execSendText: true if the socket is not open.
    fn udp_send_text(&mut self, this: ObjId, args: &[Value]) -> bool {
        let text = args.get(1).map(|v| v.as_name()).unwrap_or_default();
        let target = Self::udp_target(args);
        let Some(s) = self.ipdrv.udp.get(&this) else { return true };
        if let Err(e) = s.send_to(&to_ansi(&text), target) {
            self.ipdrv.last_error = os_error(&e);
            self.native_warn("SentText: sendto failed");
            return false;
        }
        true
    }

    /// AUdpLink::execSendBinary: returns 1 on failure and 0 otherwise, like the original.
    fn udp_send_binary(&mut self, this: ObjId, args: &[Value]) -> bool {
        let count = args.get(1).map_or(0, |v| v.as_int()).clamp(0, 255) as usize;
        let bytes: Vec<u8> = match args.get(2) {
            Some(Value::Array(b)) => b.iter().take(count).map(|v| v.as_int() as u8).collect(),
            _ => Vec::new(),
        };
        let target = Self::udp_target(args);
        if let Some(s) = self.ipdrv.udp.get(&this) {
            if let Err(e) = s.send_to(&bytes, target) {
                self.ipdrv.last_error = os_error(&e);
                self.native_warn("SendBinary: sendto failed");
                return true;
            }
        }
        false
    }

    /// AUdpLink::execReadText: a datagram of at most 4096 bytes.
    fn udp_read_text(&mut self, this: ObjId, args: &mut [Value]) -> i32 {
        if let Some(a) = args.get_mut(1) {
            *a = Value::Str(String::new());
        }
        let Some(s) = self.ipdrv.udp.get(&this) else { return 0 };
        let mut buf = vec![0u8; 0x1000];
        // Winsock's recvfrom (a datagram longer than the buffer: −1, WSAEMSGSIZE).
        let (n, err, from) = s.recv_wsa(&mut buf);
        if n < 0 {
            self.ipdrv.last_error = err;
            // GLog->Logf(NAME_Log, …).
            if err != WOULD_BLOCK {
                self.log.push("Log: ReadText: Error reading text.".into());
            }
            return 0;
        }
        if let Some(a) = args.get_mut(0) {
            *a = ip_addr_value(u32::from(*from.ip()), from.port() as i32);
        }
        if let Some(a) = args.get_mut(1) {
            *a = Value::Str(from_ansi(&buf[..n as usize]));
        }
        n
    }

    /// AUdpLink::execReadBinary: at most 255 bytes.
    fn udp_read_binary(&mut self, this: ObjId, args: &mut [Value]) -> i32 {
        let count = args.get(1).map_or(0, |v| v.as_int()).min(0xff).max(0) as usize;
        let Some(s) = self.ipdrv.udp.get(&this) else { return 0 };
        let mut buf = vec![0u8; count];
        let (n, err, from) = s.recv_wsa(&mut buf);
        if n < 0 {
            self.ipdrv.last_error = err;
            // recvfrom writes directly into the script's array: a datagram that is too long leaves its first bytes there.
            if err == MSG_SIZE {
                if let Some(Value::Array(b)) = args.get_mut(2) {
                    for (i, &c) in buf.iter().enumerate() {
                        if let Some(x) = b.get_mut(i) {
                            *x = Value::Byte(c);
                        }
                    }
                }
            }
            // GLog->Logf(NAME_Log, …); the text is ReadText's.
            if err != WOULD_BLOCK {
                self.log.push("Log: ReadBinary: Error reading text.".into());
            }
            return 0;
        }
        if let Some(a) = args.get_mut(0) {
            *a = ip_addr_value(u32::from(*from.ip()), from.port() as i32);
        }
        if let Some(Value::Array(b)) = args.get_mut(2) {
            for (i, &c) in buf[..n as usize].iter().enumerate() {
                if let Some(x) = b.get_mut(i) {
                    *x = Value::Byte(c);
                }
            }
        }
        n
    }

    /// AInternetLink::Tick, after AActor::Tick: end of a resolution (thread finished).
    pub(crate) fn internet_link_tick(&mut self, a: ObjId) {
        let Some((host, r)) = self.ipdrv.resolves.get(&a) else { return };
        let Some(end_value) = r.lock().ok().and_then(|o| o.clone()) else { return };
        let host = host.clone();
        self.ipdrv.resolves.remove(&a);
        let (line_value, evt) = match &end_value {
            Ok(addr) => resolve_end(&host, "", *addr),
            Err(e) => resolve_end(&host, e, [0; 4]),
        };
        self.log.push(line_value);
        match evt {
            None => self.event_pub(a, "ResolveFailed", vec![]),
            Some(addr) => self.event_pub(a, "Resolved", vec![ip_addr_value(addr, 0)]),
        }
    }

    /// AUdpLink::Tick, after AActor::Tick.
    pub(crate) fn udp_link_tick(&mut self, a: ObjId) {
        if !self.ipdrv.udp.contains_key(&a) {
            return;
        }
        match self.propr(a, "ReceiveMode").as_byte() {
            1 => {
                let mut buf = [0u8; 0xFE + 1];
                // recvfrom of 254 bytes: a longer datagram returns −1 (WSAEMSGSIZE) and nothing is received.
                let Some((n, _, from)) = self.ipdrv.udp.get(&a).map(|s| s.recv_wsa(&mut buf[..0xFE])) else { return };
                if n < 0 {
                    return;
                }
                let n = n as usize;
                let addr = ip_addr_value(u32::from(*from.ip()), from.port() as i32);
                match self.propr(a, "LinkMode").as_byte() {
                    // AUdpLink::eventReceivedText / Line / Binary depending on LinkMode.
                    0 => self.event_pub(a, "ReceivedText", vec![addr, Value::Str(from_ansi(&buf[..n]))]),
                    1 => self.event_pub(a, "ReceivedLine", vec![addr, Value::Str(from_ansi(&buf[..n]))]),
                    2 => {
                        let bytes: Vec<Value> = (0..255).map(|i| Value::Byte(if i < n { buf[i] } else { 0 })).collect();
                        self.event_pub(a, "ReceivedBinary", vec![addr, Value::Int(n as i32), Value::Array(bytes)]);
                    }
                    _ => {}
                }
            }
            0 => {
                let mut probe = [0u8; 1];
                let pending = self.ipdrv.udp.get(&a).is_some_and(|s| s.peek_from(&mut probe).is_ok());
                self.set_prop(a, "DataPending", Value::Int(pending as i32));
            }
            _ => {}
        }
    }

    /// AUdpLink::PostScriptDestroyed: socket closed.
    pub(crate) fn udp_link_destroyed(&mut self, a: ObjId) {
        self.udp_link_close(a, "Log: AUdpLink::PostScriptDestroyed -- socket closed");
    }

    /// AUdpLink::Destroy: UObject::Destroy of the actors of the map being left, at
    /// UGameEngine::LoadMap's CollectGarbage; a socket still open is closed.
    pub(crate) fn udp_link_gc_destroy(&mut self, a: ObjId) {
        self.udp_link_close(a, "Log: AUdpLink::Destroy -- socket closed");
    }

    fn udp_link_close(&mut self, a: ObjId, message: &str) {
        if self.link_socket(a) != -1 && self.is_a(a, "UdpLink") {
            let sock = self.ipdrv.udp.remove(&a);
            if self.ipdrv.offline {
                // Bound port held by the simulated socket (Port is 0 after a BindPort(0)).
                if let Some(s) = sock {
                    offline_release(&mut self.ipdrv, s.1);
                }
            }
            self.set_prop(a, "Socket", Value::Int(-1));
            self.log.push(message.into());
        }
    }

    /// ULevel::Listen, called by UGameEngine::LoadMap after the GameInfo is
    /// created when the URL has "Listen": TcpNetDriver, ServerActors, NetMode.
    pub fn level_listen(&mut self) -> bool {
        if self.ipdrv.net_driver.is_some() {
            return false;
        }
        // UTcpNetDriver::InitListen: InitBase listening. -PORT= on the
        // command line replaces URL.Port and leaves only one bind try (20 otherwise).
        self.ip_init_sockets();
        let cmd_port = cmd_line_port();
        let port = cmd_port.unwrap_or_else(|| self.url_port());
        let socket = match bind_udp(&mut self.ipdrv, port, if cmd_port.is_some() { 1 } else { 20 }, true) {
            Ok((s, bound)) => {
                if let Some(s) = &s.0 {
                    let _ = s.set_broadcast(true);
                    let _ = s.set_nonblocking(true);
                }
                let (rcv, snd) = socket_queue(&s, 0x20000);
                self.log.push(format!("Init: WinSock: Socket queue {rcv} / {snd}"));
                let addr = self.log_local_bind_addr(false);
                Some((s, bound, addr))
            }
            Err(e) => {
                // The original's socket is set up before the bind: queue log lines first.
                self.log.push("Init: WinSock: Socket queue 131072 / 131072".into());
                self.log.push(format!("Failed to listen: WinSock: binding to port {port} failed ({})", socket_error_text_address(e) as i32));
                None
            }
        };
        let Some((socket, bound, local_addr)) = socket else { return false };
        // UTcpNetDriver::InitListen: URL.Host = IpString(LocalAddr) ("a.b.c.d"), URL.Port = bound port.
        let [a, b, c, d] = local_addr;
        self.url.host = format!("{a}.{b}.{c}.{d}");
        self.url.port = Some(bound as i32);
        self.log.push(format!("DevNet: TcpNetDriver on port {bound}"));
        let lan = self.url.has_option("LAN");
        let mut cfg = self.net_driver_config();
        // Not in the original: enhanced network play, higher Internet ceilings (minimums kept).
        if self.net_enhanced() {
            cfg.apply_enhanced_internet();
        }
        let mut driver = NetDriver::new(socket, bound, lan, cfg);
        driver.local_addr = local_addr;
        self.ipdrv.net_driver = Some(driver);
        // ServerActors of [Engine.GameEngine]: "Class Key=Value…".
        let lines = self.config.as_mut().and_then(|c| c.multi_find("Engine.GameEngine", "ServerActors", "System")).unwrap_or_default();
        for line in lines {
            // ParseToken(&Str, Buf, 0xF0, 1): the class name, then the "Key=Value" tokens.
            let mut rest: &str = &line;
            let Some((class_name, r)) = crate::console::parse_token(rest) else { continue };
            rest = r;
            let class_name: String = class_name.chars().take(0xEF).collect();
            self.log.push(format!("Log: Spawning: {class_name}"));
            // StaticLoadClass(AActor, …): a class that is not an actor is not returned, and
            // ULevel::SpawnActor without a class only warns.
            let class = self.class(&class_name).filter(|&c| self.class("Actor").is_some_and(|actor| self.is_child_of(c, actor)));
            let Some(class) = class else {
                self.log.push("Warning: SpawnActor failed because no class was specified".into());
                continue;
            };
            let a = self.spawn(class);
            self.init_execution(a);
            if let Some(level) = self.level {
                self.set_prop(a, "Level", Value::Object(Some(level)));
            }
            self.actors.push(a);
            self.actors_changed();
            while let Some((token, r)) = crate::console::parse_token(rest) {
                rest = r;
                let token: String = token.chars().take(0xEF).collect();
                if let Some((k, v)) = token.split_once('=') {
                    self.listen_import_config(a, k, v);
                }
            }
        }
        // NetMode = Client ? NM_ListenServer : NM_DedicatedServer; NextSwitchCountdown =
        // NetDriver->ServerTravelPause.
        if let Some(level) = self.level {
            self.set_prop(level, "NetMode", Value::Byte(if self.ipdrv.dedicated { 1 } else { 2 }));
            let pause = self.net_drv_cfg().server_travel_pause;
            self.set_prop(level, "NextSwitchCountdown", Value::Float(pause));
        }
        true
    }

    /// ULevel::Listen, one "Key=Value" token of a ServerActors line: the actor's CPF_Config properties (those of its class then of its
    /// parents) whose name matches case-insensitively receive ImportText of the value; the others, public or not, const or not, are left alone.
    fn listen_import_config(&mut self, a: ObjId, key: &str, value: &str) {
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        for i in 0..self.classes[class].props.len() {
            let def = self.classes[class].props[i].clone();
            if def.flags & crate::config::CPF_CONFIG == 0 || !def.name.eq_ignore_ascii_case(key) {
                continue;
            }
            let chars: Vec<char> = value.chars().collect();
            let mut v = self.objects[a].as_ref().map(|o| o.props[i].clone()).unwrap_or(Value::Void);
            if def.dim > 1 {
                if let Value::Array(items) = &mut v {
                    if let Some(first) = items.first_mut() {
                        self.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, first);
                    }
                }
            } else {
                self.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, &mut v);
            }
            if let Some(o) = self.objects[a].as_mut() {
                o.props[i] = v;
            }
        }
    }

    /// FURL::Port of a local URL: FURL::DefaultPort ([URL] Port through appAtoi).
    fn url_port(&mut self) -> u16 {
        self.url_default_port() as u16
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::inet_addr;

    /// inet_addr forms accepted by StringToIpAddr (value after htonl).
    #[test]
    fn inet_addr_shapes() {
        let h = |s: &str| u32::from_be(inet_addr(s));
        assert_eq!(h("127.0.0.1"), 0x7F00_0001);
        assert_eq!(h("127.1"), 0x7F00_0001);
        assert_eq!(h("10.1.2"), 0x0A01_0002);
        assert_eq!(h("16909060"), 0x0102_0304);
        assert_eq!(h("0x7f.0.0.1"), 0x7F00_0001);
        assert_eq!(h("010.0.0.1"), 0x0800_0001);
        assert_eq!(h("127.0.0.1 next"), 0x7F00_0001);
        assert_eq!(inet_addr("255.255.255.255"), u32::MAX);
        assert_eq!(inet_addr("not an address"), u32::MAX);
    }

    /// Hosts read by inet_addr or resolved (UTcpipConnection).
    #[test]
    fn host_tip() {
        use super::host_is_dotted;
        assert!(host_is_dotted("192.0.2.5"));
        assert!(host_is_dotted("999.1.1.1"));
        assert!(!host_is_dotted("127.1"));
        assert!(!host_is_dotted("1.2.3.4.5"));
        assert!(!host_is_dotted("machine"));
        assert!(!host_is_dotted("1.2.x.4"));
    }
}

#[cfg(test)]
mod enhanced_rates_tests {
    use super::*;

    /// A listen server's driver settings (simulated socket) for `url`.
    fn listen_cfg(enhanced: bool, url: &str) -> (NetDriverCfg, bool) {
        crate::net_smooth::set_test_net_enhanced(enhanced);
        let mut vm = crate::Vm::default();
        vm.ipdrv.offline = true;
        vm.url = crate::travel::Url::parse(None, url, crate::travel::TRAVEL_ABSOLUTE);
        assert!(vm.level_listen(), "listening");
        crate::net_smooth::set_test_net_enhanced(false);
        let d = vm.ipdrv.net_driver.take().unwrap();
        (d.cfg, d.lan)
    }

    #[test]
    fn enhanced_play_raises_only_the_clients_internet_ceiling() {
        let (original, lan) = listen_cfg(false, "DM_Warehouse?Listen");
        assert!(!lan);
        let (enhanced, _) = listen_cfg(true, "DM_Warehouse?Listen");
        // Graphics: Original (and the faithful build): the game's values, untouched.
        let defaults = NetDriverCfg::from_words(&net_driver_values(&mut |_| None, "True", "False"));
        for (o, d) in [(original.client, defaults.client), (original.listen_server, defaults.listen_server), (original.dedicated_server, defaults.dedicated_server)] {
            assert_eq!((o.internet_min, o.internet_max, o.lan_min, o.lan_max, o.step_up), (d.internet_min, d.internet_max, d.lan_min, d.lan_max, d.step_up));
        }
        // Enhanced: the client's Internet maximum raised, never lowered; everything else is the
        // game's (a slow link comes down to the original's rate).
        assert_eq!(enhanced.client.internet_max, original.client.internet_max.max(ENHANCED_CLIENT_INTERNET_MAX));
        for (e, o) in [(enhanced.client, original.client), (enhanced.listen_server, original.listen_server), (enhanced.dedicated_server, original.dedicated_server)] {
            assert_eq!((e.internet_min, e.lan_min, e.lan_max, e.step_up), (o.internet_min, o.lan_min, o.lan_max, o.step_up));
            assert_eq!((e.lag_to_take_action, e.max_allowed_lag), (o.lag_to_take_action, o.max_allowed_lag));
        }
        assert_eq!(enhanced.listen_server.internet_max, original.listen_server.internet_max);
        assert_eq!(enhanced.dedicated_server.internet_max, original.dedicated_server.internet_max);
        assert_eq!(enhanced.max_client_rate, original.max_client_rate, "what a client may ask stays the game's");
        // With XIII.ini's 3000 the enhanced ceiling is higher (negative control: without
        // apply_enhanced_internet it is not).
        let mut ini = defaults.clone();
        ini.client.internet_max = 3000;
        ini.apply_enhanced_internet();
        assert!(ini.client.internet_max > 3000);
    }
}

#[cfg(test)]
mod lag_measured_tests {
    #[test]
    fn startup_lags_are_not_measurements() {
        use crate::net_actor::lag_measured;
        // Seen on a co-op guest's first periods: 9999 s, then 833 s and 238 s.
        assert!(!lag_measured(9999.0) && !lag_measured(833.47) && !lag_measured(238.55));
        assert!(lag_measured(0.03) && lag_measured(0.35) && lag_measured(2.0));
    }
}
