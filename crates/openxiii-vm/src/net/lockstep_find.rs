//! Network co-op in lockstep: finding the co-op games open on the local network, for the Join
//! page's list (improved build).
//!
//! A guest sends a small "find" datagram, a few times a second, to the game's default port on
//! every local broadcast address (one per network adapter, a VPN's included) and to
//! 255.255.255.255. A host whose meeting still waits for its guest (lockstep_meet.rs) answers
//! with a "here" datagram: its player's name, its map, its port, its build and the checksum of
//! its game files, so that the page can show a game this build would be refused by as another
//! version instead of a game to join. The classic multiplayer LAN list (ipdrv.rs, UdpBeacon) is
//! not used nor changed: co-op hosts are not listen servers.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use crate::lockstep::{put_bytes, Reader, MAGIC, PROTOCOL};

pub(crate) const KIND_FIND: u8 = 8;
pub(crate) const KIND_HERE: u8 = 9;

/// Time between two finds sent (seconds).
const FIND_EVERY: f64 = 0.4;
/// A host not heard from for this long leaves the list.
const FORGET: Duration = Duration::from_secs(3);
/// The broadcast addresses are asked for again this often (an adapter coming up, a VPN started).
const ADAPTERS_EVERY: f64 = 5.0;
/// Most hosts kept.
pub const FOUND_MAX: usize = 8;
/// Longest name or map kept from an answer (characters).
const TEXT_MAX: usize = 40;
/// Most datagrams read in one poll (the page polls once a frame): a network that keeps sending
/// never holds the frame up.
pub const READS_MAX: usize = 32;
/// Most broadcast addresses a find goes to (machines with many virtual adapters).
const TARGETS_MAX: usize = 16;

/// This build and the checksum of its game files and mods, given by the game window (they are
/// the viewer's: its OXIII_BUILD_ID and the meeting's data checksum).
type OwnGame = Box<dyn Fn() -> (String, u64) + Send>;
static OWN_GAME: std::sync::Mutex<Option<OwnGame>> = std::sync::Mutex::new(None);

/// Sets how to know this build and its game files (see [`own_game`]).
pub fn set_own_game(f: OwnGame) {
    if let Ok(mut g) = OWN_GAME.lock() {
        *g = Some(f);
    }
}

/// This build and the checksum of its game files (None before the game window gave them).
pub fn own_game() -> Option<(String, u64)> {
    OWN_GAME.lock().ok().and_then(|g| g.as_ref().map(|f| f()))
}

/// What a waiting host says about its game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    /// A number the host drew for this game (the same host heard on two networks is one game).
    pub id: u64,
    /// Its player's co-op name.
    pub name: String,
    /// The map it starts on.
    pub map: String,
    /// The port its meeting listens on.
    pub port: u16,
    /// Its build (OXIII_BUILD_ID of the viewer).
    pub build: String,
    /// The checksum of its game files and mods (the meeting's agreement).
    pub checksum: u64,
}

/// The find datagram a guest sends.
pub fn find_datagram() -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.push(KIND_FIND);
    out.extend_from_slice(&PROTOCOL.to_le_bytes());
    out
}

/// True for a find datagram of this protocol.
pub fn is_find(b: &[u8]) -> bool {
    let mut r = Reader::new(b);
    r.take(4) == Some(&MAGIC[..]) && r.u8() == Some(KIND_FIND) && r.u16() == Some(PROTOCOL)
}

fn cut(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(TEXT_MAX).collect()
}

/// A host's answer.
pub fn here_datagram(l: &Listing) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.push(KIND_HERE);
    out.extend_from_slice(&PROTOCOL.to_le_bytes());
    out.extend_from_slice(&l.port.to_le_bytes());
    out.extend_from_slice(&(l.id as u32).to_le_bytes());
    out.extend_from_slice(&((l.id >> 32) as u32).to_le_bytes());
    out.extend_from_slice(&(l.checksum as u32).to_le_bytes());
    out.extend_from_slice(&((l.checksum >> 32) as u32).to_le_bytes());
    put_bytes(&mut out, cut(&l.name).as_bytes());
    put_bytes(&mut out, cut(&l.map).as_bytes());
    put_bytes(&mut out, l.build.as_bytes());
    out
}

/// See [`here_datagram`]: the protocol of the host and what it said. None for anything else.
pub fn read_here(b: &[u8]) -> Option<(u16, Listing)> {
    let mut r = Reader::new(b);
    if r.take(4) != Some(&MAGIC[..]) || r.u8()? != KIND_HERE {
        return None;
    }
    let proto = r.u16()?;
    let port = r.u16()?;
    let id = r.u64()?;
    let checksum = r.u64()?;
    let text = |r: &mut Reader| r.bytes().map(|b| cut(&String::from_utf8_lossy(b)));
    let name = text(&mut r)?;
    let map = text(&mut r)?;
    let build = String::from_utf8_lossy(r.bytes()?).into_owned();
    Some((proto, Listing { id, name, map, port, build, checksum }))
}

/// A co-op game found on the network.
#[derive(Clone, Debug)]
pub struct Found {
    /// Where to join it (the host's address as seen from here, its meeting's port).
    pub addr: SocketAddrV4,
    pub listing: Listing,
    /// This build could join it (same protocol, build and game files).
    pub same_game: bool,
    seen: Instant,
}

/// The guest's side: sends finds and keeps the hosts that answered.
pub struct Finder {
    sock: UdpSocket,
    port: u16,
    /// This build and the checksum of its game files (None: every game shown as joinable).
    ours: Option<(String, u64)>,
    started: Instant,
    last_find: f64,
    last_adapters: f64,
    targets: Vec<SocketAddr>,
    /// Fixed targets (tests): the adapters are not asked.
    fixed: bool,
    found: Vec<Found>,
    /// Datagrams read by the last poll.
    pub(crate) read_last: usize,
}

impl Finder {
    /// Finds the hosts waiting on `port` (the game's default one) on every local network.
    pub fn new(port: u16, ours: Option<(String, u64)>) -> std::io::Result<Finder> {
        Self::make(port, ours, Vec::new(), false)
    }

    /// Finds the hosts waiting on `port` at the given addresses only (tests).
    pub fn at(port: u16, ours: Option<(String, u64)>, targets: Vec<Ipv4Addr>) -> std::io::Result<Finder> {
        let targets = targets.into_iter().map(|a| SocketAddr::V4(SocketAddrV4::new(a, port))).collect();
        Self::make(port, ours, targets, true)
    }

    fn make(port: u16, ours: Option<(String, u64)>, targets: Vec<SocketAddr>, fixed: bool) -> std::io::Result<Finder> {
        let sock = UdpSocket::bind(("0.0.0.0", 0))?;
        sock.set_nonblocking(true)?;
        let _ = sock.set_broadcast(true);
        Ok(Finder {
            sock,
            port,
            ours,
            started: Instant::now(),
            last_find: f64::NEG_INFINITY,
            last_adapters: f64::NEG_INFINITY,
            targets,
            fixed,
            found: Vec::new(),
            read_last: 0,
        })
    }

    fn broadcast_targets(&self) -> Vec<SocketAddr> {
        let mut v: Vec<Ipv4Addr> = crate::ipdrv::locals_broadcasts();
        v.truncate(TARGETS_MAX);
        v.push(Ipv4Addr::BROADCAST);
        // A host on this same computer (two games side by side).
        v.push(Ipv4Addr::LOCALHOST);
        v.into_iter().map(|a| SocketAddr::V4(SocketAddrV4::new(a, self.port))).collect()
    }

    /// Sends what is due and reads the answers: the hosts heard from lately, in the order they
    /// first answered.
    pub fn poll(&mut self) -> &[Found] {
        let now = self.started.elapsed().as_secs_f64();
        if !self.fixed && now - self.last_adapters >= ADAPTERS_EVERY {
            self.last_adapters = now;
            self.targets = self.broadcast_targets();
        }
        if now - self.last_find >= FIND_EVERY {
            self.last_find = now;
            let find = find_datagram();
            for t in &self.targets {
                let _ = self.sock.send_to(&find, t);
            }
        }
        let mut buf = [0u8; 1024];
        let reads = READS_MAX;
        self.read_last = 0;
        for _ in 0..reads {
            self.read_last += 1;
            let (n, from) = match self.sock.recv_from(&mut buf) {
                Ok(x) => x,
                // Nothing more waiting.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                // Windows: a find that reached no game comes back as a "port unreachable" error.
                Err(_) => continue,
            };
            let SocketAddr::V4(from) = from else { continue };
            let Some((proto, listing)) = read_here(&buf[..n]) else { continue };
            let addr = SocketAddrV4::new(*from.ip(), listing.port);
            let same_game = proto == PROTOCOL && self.ours.as_ref().is_none_or(|(b, c)| *b == listing.build && *c == listing.checksum);
            let seen = Instant::now();
            // The same host heard on two networks (its LAN and a VPN, or this computer): its
            // first address kept.
            let same_host = |f: &Found| f.addr == addr || f.listing.id == listing.id;
            if let Some(f) = self.found.iter_mut().find(|f| same_host(f)) {
                f.listing = listing;
                f.same_game = same_game;
                f.seen = seen;
            } else if self.found.len() < FOUND_MAX {
                self.found.push(Found { addr, listing, same_game, seen });
            }
        }
        self.found.retain(|f| f.seen.elapsed() < FORGET);
        &self.found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockstep::Agreement;
    use crate::lockstep_meet::Meeting;

    fn listing() -> Listing {
        Listing {
            id: 0x1234_5678_9abc_def0,
            name: "Jones".into(),
            map: "Plage01".into(),
            port: 7777,
            build: "c4206d279d1c|improved,mods|release|64-little|release".into(),
            checksum: 0xfeed_beef_0bad_cafe,
        }
    }

    #[test]
    fn here_round_trip() {
        let l = listing();
        assert_eq!(read_here(&here_datagram(&l)), Some((PROTOCOL, l.clone())));
        // Long or control characters are cut on both sides.
        let mut long = l.clone();
        long.name = format!("A\u{7}{}", "x".repeat(100));
        let (_, back) = read_here(&here_datagram(&long)).unwrap();
        assert_eq!(back.name.chars().count(), TEXT_MAX);
        assert!(!back.name.contains('\u{7}'));
    }

    #[test]
    fn other_datagrams_are_not_answers() {
        let here = here_datagram(&listing());
        assert!(read_here(&find_datagram()).is_none());
        assert!(read_here(&here[..here.len() - 3]).is_none(), "cut short");
        assert!(read_here(b"OXLX\x09").is_none());
        assert!(read_here(&[]).is_none());
        assert!(is_find(&find_datagram()));
        assert!(!is_find(&here));
        let mut old = find_datagram();
        old[5] ^= 0xff;
        assert!(!is_find(&old), "another protocol");
    }

    fn wait_found(f: &mut Finder, want: usize) -> Vec<Found> {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if f.poll().len() >= want {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        f.poll().to_vec()
    }

    /// A host's meeting waiting for its guest answers the Join page's find with its name, map
    /// and port; this build sees it as its own game, another build as another version, and a
    /// host that was not listed does not answer.
    #[test]
    fn a_waiting_host_answers_finds() {
        let port = 48_100 + (std::process::id() % 40) as u16;
        let a = Agreement { checksum: 7, start: "Plage01|Start=NewGame".into(), map_sum: 9, build: "b1".into() };
        let mut host = Meeting::host(port, a, Box::new(|w: &str| w.to_string()), (1280, 720), Some(2), Vec::new(), Duration::from_secs(10)).unwrap();
        host.list_as("Jones", "Plage01");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s = stop.clone();
        let h = std::thread::spawn(move || {
            while !s.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = host.poll();
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let mut same = Finder::at(port, Some(("b1".into(), 7)), vec![Ipv4Addr::LOCALHOST]).unwrap();
        let mut other = Finder::at(port, Some(("b2".into(), 7)), vec![Ipv4Addr::LOCALHOST]).unwrap();
        let found = wait_found(&mut same, 1);
        let found_other = wait_found(&mut other, 1);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = h.join();
        assert_eq!(found.len(), 1, "the host answered");
        let f = &found[0];
        assert_eq!((f.listing.name.as_str(), f.listing.map.as_str(), f.addr), ("Jones", "Plage01", SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)));
        assert!(f.same_game);
        assert_eq!(found_other.len(), 1);
        assert!(!found_other[0].same_game, "another build is another version");

        // Not listed: silent.
        let port = port + 40;
        let a = Agreement { checksum: 7, start: "Plage01".into(), map_sum: 9, build: "b1".into() };
        let mut quiet = Meeting::host(port, a, Box::new(|w: &str| w.to_string()), (1280, 720), Some(2), Vec::new(), Duration::from_secs(10)).unwrap();
        let mut f = Finder::at(port, None, vec![Ipv4Addr::LOCALHOST]).unwrap();
        let end = Instant::now() + Duration::from_millis(800);
        while Instant::now() < end {
            let _ = quiet.poll();
            assert!(f.poll().is_empty(), "a host not listed does not answer");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// A network that keeps answering (or a flood on the search's port) never holds a frame up:
    /// one poll reads at most READS_MAX datagrams, keeps at most FOUND_MAX games and returns at
    /// once; the rest waits for the next frames.
    #[test]
    fn a_flood_of_answers_is_read_a_little_each_frame() {
        let mut f = Finder::at(9, None, vec![]).unwrap();
        let to = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, f.sock.local_addr().unwrap().port()));
        let flood = UdpSocket::bind("127.0.0.1:0").unwrap();
        for i in 0..1500u64 {
            let mut l = listing();
            l.id = i;
            let _ = flood.send_to(&here_datagram(&l), to);
        }
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        let n = f.poll().len();
        let took = t.elapsed();
        assert!(n <= FOUND_MAX, "{n} games kept");
        assert!(f.read_last <= READS_MAX, "{} datagrams read in one poll", f.read_last);
        assert!(took < Duration::from_millis(50), "one poll took {took:?}");
        // The others are still waiting: read by the next polls, never all at once.
        let _ = f.poll();
        assert_eq!(f.read_last, READS_MAX, "more were waiting");
    }
}
