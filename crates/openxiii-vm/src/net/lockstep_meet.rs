//! Network co-op in lockstep: the meeting of the two machines before the first step (lockstep.rs).
//!
//! The guest says hello (version, data checksum, screen size, its wishes); the host answers with
//! its start (map and options, settings of the game) or refuses. Then each machine sends the other
//! its blob, in chunks with a checksum: its player's own settings (keys, mouse, gamepad, view), and
//! for the host the save the start loads. Nothing blocks: [`Meeting::poll`] is called again and
//! again (by the window once a frame, so that it keeps drawing while it waits), until the session
//! is ready.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::lockstep::{put_bytes, Agreement, Reader, Role, Session, DELAY_MAX, DELAY_MIN, KIND_BYE, MAGIC, PROTOCOL};
use crate::lockstep_find::{here_datagram, is_find, Listing, KIND_FIND};

pub(crate) const KIND_HELLO: u8 = 1;
pub(crate) const KIND_WELCOME: u8 = 2;
pub(crate) const KIND_REFUSE: u8 = 3;
pub(crate) const KIND_STEPS: u8 = 4;
pub(crate) const KIND_BLOB: u8 = 6;
pub(crate) const KIND_BLOB_ACK: u8 = 7;

/// Bytes of a blob in one datagram.
const CHUNK: usize = 1000;
/// The largest blob a machine accepts (its settings, the start state and the host's save
/// files are far below): a datagram announcing more is not believed.
const BLOB_MAX: usize = 8 * 1024 * 1024;
/// Chunks sent ahead of the other machine's acknowledgement.
const WINDOW: usize = 48;
/// Time between two sendings of the same datagrams (seconds).
const RESEND: f64 = 0.03;

/// FNV-1a of a blob (its checksum).
pub fn checksum(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Named files in one blob.
pub fn pack(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_le_bytes().to_vec();
    for (name, data) in entries {
        put_bytes(&mut out, name.as_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

/// See [`pack`]. None for a damaged blob.
pub fn unpack(b: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    if b.is_empty() {
        return Some(Vec::new());
    }
    let mut r = Reader::new(b);
    let n = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let name = String::from_utf8_lossy(r.bytes()?).into_owned();
        let len = r.u32()? as usize;
        out.push((name, r.take(len)?.to_vec()));
    }
    Some(out)
}

/// What the meeting gave.
pub struct Met {
    pub session: Box<Session>,
    /// The other player's screen size.
    pub peer_size: (i32, i32),
    /// Guest: the host's start (map, options, settings) and its map checksum. Host: what it sent.
    pub start: Agreement,
    /// The other machine's blob (see [`pack`]).
    pub blob: Vec<u8>,
}

/// Where a meeting is.
pub enum MeetPoll {
    Waiting,
    Failed(String),
    Ready(Met),
}

#[derive(Default)]
struct Incoming {
    total: Option<usize>,
    count: usize,
    sum: u64,
    chunks: Vec<Option<Vec<u8>>>,
    contig: usize,
}

impl Incoming {
    /// A chunk of the other machine's blob: `total` bytes in `count` chunks of CHUNK bytes (the
    /// last one shorter), checksum `sum`. The first datagram fixes the blob; one that announces
    /// another blob, a size beyond BLOB_MAX, a count that does not match the size, an index past
    /// the count or a chunk of the wrong length is ignored. True: the chunk was taken (or was
    /// already there).
    fn accept(&mut self, total: usize, count: usize, index: usize, sum: u64, data: Option<&[u8]>) -> bool {
        if (total > BLOB_MAX || count != chunks_of(total)) {
            return false;
        }
        match self.total {
            None => *self = Incoming { total: Some(total), count, sum, chunks: vec![None; count], contig: 0 },
            Some(t) if t != total || self.count != count || self.sum != sum => return false,
            Some(_) => {}
        }
        if count == 0 {
            return true;
        }
        let Some(data) = data else { return false };
        let expected = if index + 1 == count { total - (count - 1) * CHUNK } else { CHUNK };
        if index >= self.count || data.len() != expected {
            return false;
        }
        if self.chunks[index].is_none() {
            self.chunks[index] = Some(data.to_vec());
        }
        while self.contig < self.count && self.chunks[self.contig].is_some() {
            self.contig += 1;
        }
        true
    }
}

/// The meeting, from either side (see the module).
pub struct Meeting {
    sock: UdpSocket,
    role: Role,
    peer: Option<SocketAddr>,
    ours: Agreement,
    compose: Box<dyn Fn(&str) -> String + Send>,
    size: (i32, i32),
    delay: u64,
    started: Instant,
    timeout: Duration,
    last_send: f64,
    hello: Vec<u8>,
    welcome: Vec<u8>,
    peer_size: (i32, i32),
    start: Option<Agreement>,
    mine: Vec<u8>,
    mine_acked: usize,
    inc: Incoming,
    /// Steps of the other machine that came before this one was ready (fed to the session).
    early: Vec<Vec<u8>>,
    peer_stepping: bool,
    /// Guest: why the host refused (it is asked again until the time is up: a host still in an
    /// earlier session may be the right one a moment later).
    refused: Option<String>,
    /// Host: the last refusal reason logged.
    refusal_logged: Option<String>,
    /// Host (improved build): what it answers to the Join page's finds while it waits
    /// (lockstep_find.rs); None: it does not answer.
    listing: Option<Listing>,
}

fn bind_ip() -> &'static str {
    bind_address()
}

fn chunks_of(len: usize) -> usize {
    len.div_ceil(CHUNK)
}

fn bind_address() -> &'static str {
    "0.0.0.0"
}

impl Meeting {
    /// Host: listens on `port` for a guest whose agreement matches `ours`. `compose` makes the
    /// start sent from the guest's wishes ("Key=value;…", empty for none).
    pub fn host(port: u16, ours: Agreement, compose: Box<dyn Fn(&str) -> String + Send>, size: (i32, i32), delay: Option<u64>, blob: Vec<u8>, timeout: Duration) -> Result<Meeting, String> {
        let sock = UdpSocket::bind((bind_ip(), port)).map_err(|e| format!("port {port}: {e}"))?;
        Ok(Self::new(sock, Role::Host, None, ours, compose, size, delay.unwrap_or(2).clamp(DELAY_MIN, DELAY_MAX), blob, timeout))
    }

    /// Guest: asks `host` to join with our agreement (its start: our wishes, or empty).
    pub fn join(host: SocketAddr, ours: Agreement, size: (i32, i32), blob: Vec<u8>, timeout: Duration) -> Result<Meeting, String> {
        let sock = UdpSocket::bind((bind_ip(), 0)).map_err(|e| e.to_string())?;
        let mut m = Self::new(sock, Role::Guest, Some(host), ours, Box::new(|s: &str| s.to_string()), size, 2, blob, timeout);
        let mut hello = MAGIC.to_vec();
        hello.push(KIND_HELLO);
        hello.extend_from_slice(&PROTOCOL.to_le_bytes());
        hello.extend_from_slice(&(m.ours.checksum as u32).to_le_bytes());
        hello.extend_from_slice(&((m.ours.checksum >> 32) as u32).to_le_bytes());
        hello.extend_from_slice(&(size.0 as u32).to_le_bytes());
        hello.extend_from_slice(&(size.1 as u32).to_le_bytes());
        put_bytes(&mut hello, m.ours.start.as_bytes());
        put_bytes(&mut hello, m.ours.build.as_bytes());
        m.hello = hello;
        Ok(m)
    }

    #[allow(clippy::too_many_arguments)]
    fn new(sock: UdpSocket, role: Role, peer: Option<SocketAddr>, ours: Agreement, compose: Box<dyn Fn(&str) -> String + Send>, size: (i32, i32), delay: u64, blob: Vec<u8>, timeout: Duration) -> Meeting {
        let _ = sock.set_nonblocking(true);
        Meeting {
            sock,
            role,
            peer,
            ours,
            compose,
            size,
            delay,
            started: Instant::now(),
            timeout,
            last_send: f64::NEG_INFINITY,
            hello: Vec::new(),
            welcome: Vec::new(),
            peer_size: (0, 0),
            start: None,
            mine: blob,
            mine_acked: 0,
            inc: Incoming { total: None, count: 0, sum: 0, chunks: Vec::new(), contig: 0 },
            early: Vec::new(),
            peer_stepping: false,
            refused: None,
            refusal_logged: None,
            listing: None,
        }
    }

    /// Host: answers the Join page's finds while it waits for its guest, with its player's
    /// `name` and its `map` (lockstep_find.rs).
    pub fn list_as(&mut self, name: &str, map: &str) {
        let port = self.sock.local_addr().map_or(0, |a| a.port());
        let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let id = checksum(&[clock.to_le_bytes(), u64::from(std::process::id()).to_le_bytes()].concat());
        self.listing = Some(Listing { id, name: name.to_string(), map: map.to_string(), port, build: self.ours.build.clone(), checksum: self.ours.checksum });
    }

    /// Bytes received of the other machine's blob, and its size (for a progress line).
    pub fn progress(&self) -> (usize, usize) {
        let got = self.inc.chunks.iter().flatten().map(Vec::len).sum();
        (got, self.inc.total.unwrap_or(0))
    }

    /// The other machine answered (the meeting is past the hello).
    pub fn met(&self) -> bool {
        self.start.is_some()
    }

    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn send(&self, b: &[u8]) {
        if let Some(p) = self.peer {
            let _ = self.sock.send_to(b, p);
        }
    }

    /// Sends what is due, reads what came; Ready once both blobs crossed.
    pub fn poll(&mut self) -> MeetPoll {
        if self.started.elapsed() > self.timeout {
            return MeetPoll::Failed(if self.start.is_none() {
                match (&self.refused, self.role) {
                    (Some(why), _) => format!("refused by the host: {why}"),
                    (None, Role::Host) => "no guest came".into(),
                    (None, Role::Guest) => "the host does not answer".into(),
                }
            } else {
                "the other machine stopped answering".into()
            });
        }
        let mut buf = [0u8; 2048];
        loop {
            let (n, from) = match self.sock.recv_from(&mut buf) {
                Ok(x) => x,
                Err(_) => break,
            };
            if let Some(why) = self.read(&buf[..n].to_vec(), from) {
                return MeetPoll::Failed(why);
            }
        }
        let now = self.now();
        if now - self.last_send >= RESEND {
            self.last_send = now;
            if self.start.is_none() {
                if self.role == Role::Guest {
                    let h = self.hello.clone();
                    self.send(&h);
                }
            } else {
                self.send_blob();
                self.send_ack();
            }
        }
        let theirs_done = self.inc.total.is_some() && self.inc.contig >= self.inc.count;
        let mine_done = self.mine_acked >= chunks_of(self.mine.len()).max(1) || self.peer_stepping;
        if self.start.is_none() || !theirs_done || !mine_done {
            return MeetPoll::Waiting;
        }
        let blob: Vec<u8> = self.inc.chunks.iter().flatten().flat_map(|c| c.iter().copied()).collect();
        let blob = blob[..self.inc.total.unwrap_or(0).min(blob.len())].to_vec();
        if checksum(&blob) != self.inc.sum {
            return MeetPoll::Failed("the other machine's settings or save arrived damaged".into());
        }
        // A last acknowledgement for the other machine, in case it still waits for ours.
        for _ in 0..3 {
            self.send_ack();
        }
        let Some(peer) = self.peer else { return MeetPoll::Failed("no other machine".into()) };
        let sock = match self.sock.try_clone() {
            Ok(s) => s,
            Err(e) => return MeetPoll::Failed(e.to_string()),
        };
        let _ = sock.set_nonblocking(false);
        let mut session = Session::new(sock, peer, self.role, self.delay);
        for b in std::mem::take(&mut self.early) {
            session.read(&b);
        }
        MeetPoll::Ready(Met { session: Box::new(session), peer_size: self.peer_size, start: self.start.clone().unwrap_or_default(), blob })
    }

    fn read(&mut self, b: &[u8], from: SocketAddr) -> Option<String> {
        let mut r = Reader::new(b);
        if r.take(4) != Some(&MAGIC[..]) {
            return None;
        }
        let kind = r.u8()?;
        if self.peer.is_some_and(|p| p != from) && !(self.role == Role::Host && self.start.is_none()) {
            return None;
        }
        match kind {
            KIND_HELLO if self.role == Role::Host => {
                if self.start.is_some() {
                    // The guest did not get our welcome: again.
                    if self.peer == Some(from) {
                        let w = self.welcome.clone();
                        self.send(&w);
                    }
                    return None;
                }
                let (Some(proto), Some(sum), Some(w), Some(h)) = (r.u16(), r.u64(), r.u32(), r.u32()) else { return None };
                let start = r.bytes().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                let build = r.bytes().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                // The guest's start: empty (it takes the host's), its wishes ("Key=value;…"), or
                // the same start as the host's (both given it).
                let wishes = start.contains('=') && !start.contains('?') && !start.contains('|');
                let same_build = build == self.ours.build;
                if proto != PROTOCOL || !same_build || sum != self.ours.checksum || !(start.is_empty() || wishes || start == self.ours.start) {
                    let mut out = MAGIC.to_vec();
                    out.push(KIND_REFUSE);
                    let why = if proto != PROTOCOL || !same_build {
                        // Both builds' short ids (commit and features), for the logs.
                        let short = |b: &str| b.split('|').take(2).collect::<Vec<_>>().join(" ");
                        format!("other game version (host {}, guest {})", short(&self.ours.build), short(&build))
                    } else if sum != self.ours.checksum {
                        "other game files (code, sounds and voices) or mods".to_string()
                    } else {
                        "other map or start".to_string()
                    };
                    // Logged once per reason (a refused guest says hello again until its time is up).
                    if self.refusal_logged.as_deref() != Some(why.as_str()) {
                        eprintln!("lockstep: refused a guest: {why}");
                        self.refusal_logged = Some(why.clone());
                    }
                    put_bytes(&mut out, why.as_bytes());
                    let _ = self.sock.send_to(&out, from);
                    return None;
                }
                self.peer = Some(from);
                self.peer_size = (w as i32, h as i32);
                let sent = (self.compose)(if wishes { start.as_str() } else { "" });
                let mut out = MAGIC.to_vec();
                out.push(KIND_WELCOME);
                out.extend_from_slice(&(self.size.0 as u32).to_le_bytes());
                out.extend_from_slice(&(self.size.1 as u32).to_le_bytes());
                out.extend_from_slice(&(self.delay as u32).to_le_bytes());
                put_bytes(&mut out, sent.as_bytes());
                out.extend_from_slice(&(self.ours.map_sum as u32).to_le_bytes());
                out.extend_from_slice(&((self.ours.map_sum >> 32) as u32).to_le_bytes());
                self.welcome = out.clone();
                self.send(&out);
                self.start = Some(Agreement { checksum: self.ours.checksum, start: sent, map_sum: self.ours.map_sum, build: self.ours.build.clone() });
                self.last_send = f64::NEG_INFINITY;
            }
            KIND_FIND if self.role == Role::Host && self.start.is_none() && is_find(b) => {
                if let Some(l) = &self.listing {
                    let _ = self.sock.send_to(&here_datagram(l), from);
                }
            }
            KIND_WELCOME if self.role == Role::Guest && self.start.is_none() => {
                let (Some(w), Some(h), Some(d)) = (r.u32(), r.u32(), r.u32()) else { return None };
                let start = r.bytes().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                let map_sum = r.u64().unwrap_or(0);
                self.peer_size = (w as i32, h as i32);
                self.delay = u64::from(d);
                self.start = Some(Agreement { checksum: self.ours.checksum, start, map_sum, build: self.ours.build.clone() });
                self.last_send = f64::NEG_INFINITY;
            }
            KIND_REFUSE if self.role == Role::Guest && self.start.is_none() => {
                self.refused = Some(r.bytes().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default());
            }
            KIND_BLOB if self.start.is_some() => {
                let (Some(total), Some(count), Some(index), Some(sum)) = (r.u32(), r.u32(), r.u32(), r.u64()) else { return None };
                let (total, count, index) = (total as usize, count as usize, index as usize);
                // A damaged or foreign datagram is dropped (Incoming::accept).
                if self.inc.accept(total, count, index, sum, r.bytes()) {
                    self.send_ack();
                }
            }
            KIND_BLOB_ACK if self.start.is_some() => {
                let Some(contig) = r.u32() else { return None };
                self.mine_acked = self.mine_acked.max(contig as usize);
            }
            // The other player gave up the meeting (`cancel`): the guest is told; the host waits
            // for another guest, as before this one came.
            KIND_BYE if self.peer == Some(from) && self.start.is_some() => {
                if self.role == Role::Guest {
                    return Some("the other player left".into());
                }
                eprintln!("lockstep: the guest left the meeting, waiting for another one");
                self.peer = None;
                self.start = None;
                self.welcome.clear();
                self.peer_size = (0, 0);
                self.mine_acked = 0;
                self.inc = Incoming::default();
                self.early.clear();
                self.peer_stepping = false;
                self.last_send = f64::NEG_INFINITY;
            }
            KIND_STEPS => {
                // The other machine is done with the meeting.
                self.peer_stepping = true;
                self.early.push(b.to_vec());
            }
            _ => {}
        }
        None
    }

    fn send_blob(&self) {
        let count = chunks_of(self.mine.len());
        let sum = checksum(&self.mine);
        let head = |index: usize| {
            let mut out = MAGIC.to_vec();
            out.push(KIND_BLOB);
            out.extend_from_slice(&(self.mine.len() as u32).to_le_bytes());
            out.extend_from_slice(&(count as u32).to_le_bytes());
            out.extend_from_slice(&(index as u32).to_le_bytes());
            out.extend_from_slice(&(sum as u32).to_le_bytes());
            out.extend_from_slice(&((sum >> 32) as u32).to_le_bytes());
            out
        };
        if count == 0 {
            // An empty blob: its header alone.
            self.send(&head(0));
            return;
        }
        for i in self.mine_acked..(self.mine_acked + WINDOW).min(count) {
            let mut out = head(i);
            put_bytes(&mut out, &self.mine[i * CHUNK..((i + 1) * CHUNK).min(self.mine.len())]);
            self.send(&out);
        }
    }

    fn send_ack(&self) {
        if self.inc.total.is_none() {
            return;
        }
        let mut out = MAGIC.to_vec();
        out.push(KIND_BLOB_ACK);
        // An empty blob counts as one chunk received.
        let contig = if self.inc.count == 0 { 1 } else { self.inc.contig };
        out.extend_from_slice(&(contig as u32).to_le_bytes());
        self.send(&out);
    }

    /// This player gives the meeting up (Escape on the waiting sign): the other machine, once it
    /// answered, is told (a guest goes back to its menus, a host waits for another guest), and
    /// the socket is closed. A guest always tells its host (its welcome may have been lost on
    /// the way: the host may have met it already).
    pub fn cancel(self) {
        if self.start.is_some() || self.role == Role::Guest {
            let mut out = MAGIC.to_vec();
            out.push(KIND_BYE);
            for _ in 0..3 {
                self.send(&out);
            }
        }
    }

    /// Blocking meeting (capture runs and tests): polls until ready or failed.
    pub fn wait(mut self) -> Result<Met, String> {
        loop {
            match self.poll() {
                MeetPoll::Ready(m) => return Ok(m),
                MeetPoll::Failed(e) => return Err(e),
                MeetPoll::Waiting => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Damaged or hostile blob datagrams never break the meeting: a huge announced size, a count
    /// that does not match the size, an index past the count kept from the first datagram, a
    /// later datagram announcing another blob, chunks of the wrong length or missing data are
    /// all ignored; the right chunks still make the blob.
    #[test]
    fn blob_chunks_are_checked() {
        use super::{Incoming, BLOB_MAX, CHUNK};
        let blob: Vec<u8> = (0..2500u32).map(|i| i as u8).collect();
        let chunk = |i: usize| &blob[i * CHUNK..((i + 1) * CHUNK).min(blob.len())];
        let mut inc = Incoming::default();
        assert!(!inc.accept(BLOB_MAX + 1, (BLOB_MAX + 1).div_ceil(CHUNK), 0, 7, Some(&blob[..CHUNK])), "too big");
        assert!(!inc.accept(2500, 1_000_000, 0, 7, Some(chunk(0))), "count not the size's");
        assert!(inc.total.is_none(), "nothing fixed by a refused datagram");
        assert!(inc.accept(2500, 3, 1, 7, Some(chunk(1))));
        assert!(!inc.accept(2500, 3, 3, 7, Some(chunk(2))), "index past the count");
        assert!(!inc.accept(250_000, 250, 200, 7, Some(chunk(0))), "another blob announced later");
        assert!(!inc.accept(2500, 3, 1, 8, Some(chunk(1))), "another checksum");
        assert!(!inc.accept(2500, 3, 0, 7, Some(&blob[..10])), "short chunk");
        assert!(!inc.accept(2500, 3, 2, 7, Some(chunk(1))), "last chunk of the wrong length");
        assert!(!inc.accept(2500, 3, 0, 7, None), "no data");
        assert!(inc.accept(2500, 3, 0, 7, Some(chunk(0))));
        assert!(inc.accept(2500, 3, 2, 7, Some(chunk(2))));
        assert_eq!(inc.contig, 3);
        let got: Vec<u8> = inc.chunks.iter().flatten().flatten().copied().collect();
        assert_eq!(got, blob);
        // An empty blob: its header alone.
        let mut empty = Incoming::default();
        assert!(empty.accept(0, 0, 0, 1, None));
        assert_eq!(empty.total, Some(0));
    }

    use super::*;

    #[test]
    fn pack_round_trip() {
        let e = vec![("profile/User.ini".to_string(), b"[Engine.Input]\r\nW=MoveForward\r\n".to_vec()), ("save/x.sav".to_string(), vec![0u8; 5000])];
        assert_eq!(unpack(&pack(&e)), Some(e));
        assert_eq!(unpack(&[]), Some(Vec::new()));
    }

    #[test]
    fn meeting_over_loopback_with_blobs() {
        let port = 47_900 + (std::process::id() % 50) as u16;
        let a = Agreement { checksum: 7, start: "Plage01".into(), map_sum: 9, build: "b".into() };
        let big: Vec<u8> = (0..123_457u32).map(|i| (i * 31 % 251) as u8).collect();
        let host = Meeting::host(port, a.clone(), Box::new(|w: &str| format!("Plage01|{w}")), (1280, 720), Some(2), big.clone(), Duration::from_secs(20)).unwrap();
        let guest = Meeting::join(format!("127.0.0.1:{port}").parse().unwrap(), Agreement { checksum: 7, start: "Chars=1.0".into(), map_sum: 0, build: "b".into() }, (800, 600), b"mine".to_vec(), Duration::from_secs(20)).unwrap();
        let h = std::thread::spawn(move || host.wait());
        let g = guest.wait().unwrap();
        let h = h.join().unwrap().unwrap();
        assert_eq!(g.blob, big);
        assert_eq!(h.blob, b"mine".to_vec());
        assert_eq!(g.start.start, "Plage01|Chars=1.0");
        assert_eq!((g.peer_size, h.peer_size), ((1280, 720), (800, 600)));
    }

    /// Two different builds never meet: the host refuses the guest whose build (commit, changes,
    /// features, platform) is not its own, and says which ones.
    #[test]
    fn other_build_is_refused() {
        let port = 47_960 + (std::process::id() % 30) as u16;
        let a = Agreement { checksum: 7, start: "Plage01".into(), map_sum: 9, build: "1111aaaa|dx11,improved|release|x86_64-pc-windows-msvc|release".into() };
        let host = Meeting::host(port, a, Box::new(|w: &str| format!("Plage01|{w}")), (1280, 720), Some(2), Vec::new(), Duration::from_secs(4)).unwrap();
        let other = Agreement { checksum: 7, start: String::new(), map_sum: 0, build: "2222bbbb+dirty|improved|release|x86_64-pc-windows-msvc|release".into() };
        let guest = Meeting::join(format!("127.0.0.1:{port}").parse().unwrap(), other, (800, 600), Vec::new(), Duration::from_secs(3)).unwrap();
        let h = std::thread::spawn(move || host.wait());
        let g = guest.wait();
        let _ = h.join();
        let why = g.err().expect("refused");
        assert!(why.contains("other game version"), "{why}");
        assert!(why.contains("1111aaaa") && why.contains("2222bbbb+dirty"), "{why}");
    }

    /// Polls `m` until `done` says so (at most 3 s); its last answer.
    fn poll_until(m: &mut Meeting, done: impl Fn(&MeetPoll, &Meeting) -> bool) -> MeetPoll {
        let t = Instant::now();
        loop {
            let p = m.poll();
            if done(&p, m) || t.elapsed() > Duration::from_secs(3) {
                return p;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A meeting given up (Escape on the waiting sign) closes its socket at once and tells the
    /// other machine: a guest that leaves a host that met it lets the host wait for another
    /// guest (who then meets it); a host that leaves sends its met guest back to its menus at
    /// once ("the other player left"), long before its time is up; a guest whose host never
    /// answers leaves at once; the host's port is free again.
    #[test]
    fn a_meeting_given_up_tells_the_other_machine() {
        // A free port of this machine (another test or game may hold a fixed one).
        let port = UdpSocket::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let a = Agreement { checksum: 7, start: "Plage01".into(), map_sum: 9, build: "b".into() };
        let wish = || Agreement { checksum: 7, start: String::new(), map_sum: 0, build: "b".into() };
        let long = Duration::from_secs(60);
        let mut host = Meeting::host(port, a.clone(), Box::new(|w: &str| format!("Plage01|{w}")), (1280, 720), Some(2), vec![1; 5000], long).unwrap();
        // A guest says hello, the host welcomes it, then the guest gives up.
        let mut guest = Meeting::join(addr, wish(), (800, 600), b"first".to_vec(), long).unwrap();
        assert!(matches!(guest.poll(), MeetPoll::Waiting));
        assert!(matches!(poll_until(&mut host, |_, m| m.met()), MeetPoll::Waiting));
        assert!(host.met(), "the host met the first guest");
        guest.cancel();
        let p = poll_until(&mut host, |_, m| !m.met());
        assert!(matches!(p, MeetPoll::Waiting), "the host keeps waiting, it does not fail");
        assert!(!host.met(), "the host waits for another guest");
        // Another guest meets it.
        let second = Meeting::join(addr, wish(), (640, 480), b"second".to_vec(), Duration::from_secs(10)).unwrap();
        let g = std::thread::spawn(move || second.wait());
        let h = match poll_until(&mut host, |p, _| !matches!(p, MeetPoll::Waiting)) {
            MeetPoll::Ready(m) => m,
            MeetPoll::Failed(e) => panic!("{e}"),
            MeetPoll::Waiting => panic!("the second guest never met the host"),
        };
        assert_eq!((h.blob.as_slice(), h.peer_size), (&b"second"[..], (640, 480)));
        let _ = g.join().unwrap().unwrap();
        drop(h);
        host.cancel();
        // A host that leaves its met guest: the guest is told at once.
        let mut host = Meeting::host(port, a.clone(), Box::new(|w: &str| format!("Plage01|{w}")), (1280, 720), Some(2), vec![1; 5000], long).unwrap();
        let mut guest = Meeting::join(addr, wish(), (800, 600), vec![2; 5000], long).unwrap();
        let t = Instant::now();
        while !guest.met() && t.elapsed() < Duration::from_secs(3) {
            assert!(matches!(guest.poll(), MeetPoll::Waiting));
            // The host is left where it is once the guest met it (still before the session).
            if !guest.met() {
                assert!(matches!(host.poll(), MeetPoll::Waiting));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(guest.met(), "the guest met the host");
        let t = Instant::now();
        host.cancel();
        match poll_until(&mut guest, |p, _| !matches!(p, MeetPoll::Waiting)) {
            MeetPoll::Failed(why) => assert!(why.contains("other player left"), "{why}"),
            _ => panic!("the guest was not told that the host left"),
        }
        assert!(t.elapsed() < Duration::from_secs(2));
        drop(guest);
        // The host's port is free again at once (its socket closed with the meeting).
        let host = Meeting::host(port, a.clone(), Box::new(|w: &str| w.to_string()), (1280, 720), Some(2), Vec::new(), long).expect("the port is free again");
        // A host still waiting, no guest ever came: given up, its port is free again too.
        host.cancel();
        let host = Meeting::host(port, a, Box::new(|w: &str| w.to_string()), (1280, 720), Some(2), Vec::new(), long).expect("the waiting host's port is free again");
        host.cancel();
        // A guest whose host never answers (nothing on that port): it leaves at once.
        let nobody: SocketAddr = format!("127.0.0.1:{}", port + 1).parse().unwrap();
        let mut guest = Meeting::join(nobody, wish(), (800, 600), Vec::new(), long).unwrap();
        assert!(matches!(guest.poll(), MeetPoll::Waiting));
        let t = Instant::now();
        guest.cancel();
        assert!(t.elapsed() < Duration::from_millis(500));
    }
}
