//! Network co-op in lockstep (improved build): every machine runs the whole split-screen co-op
//! simulation of the same map from the same start, and only the players' commands cross the
//! network.
//!
//! - The simulation advances by fixed steps of 1/60 s. Step `S` is simulated only once the
//!   commands of every player for `S` are known; each machine schedules its own player's commands
//!   a few steps ahead (the input delay) so that they normally arrive in time.
//! - A command ([`PlayerCmd`]) is what one player's devices gave the game during one step, taken
//!   where the keyboard, mouse, gamepad and touch controls meet: key and axis events, the raw
//!   gamepad state (read by the same code as a split-screen player's gamepad), the look and move
//!   sticks, and the aim assist values of the device in use. Every machine applies every player's
//!   command in that player's own UpdateInput ([`Vm::lockstep_update_input`]).
//! - Each player's view is computed for that player's own screen size on every machine (visibility,
//!   HUD, attached objects all feed the game), but each machine only draws its own player's view.
//! - A cheap fingerprint of the game state ([`Vm::lockstep_fingerprint`]) travels with the commands:
//!   a difference stops nothing but is reported with its step and kind.
//!
//! Nothing here runs outside a lockstep session ([`Vm::lockstep`] is `None`): the solo game, the
//! split screen and the other network modes are untouched.

use std::collections::BTreeMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::pad::PadState;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Simulation steps per second in a lockstep session.
pub const STEP_HZ: u32 = 60;
/// Duration of one step (seconds).
pub const STEP: f32 = 1.0 / STEP_HZ as f32;
/// Players of a lockstep session (the host is player 1, the guest player 2).
pub const PLAYERS: usize = 2;
/// Rate of the real audio outputs (the window's), for the sound channels' follow mode.
pub(crate) const FOLLOW_RATE: u32 = 44_100;
/// Version of the lockstep protocol (datagrams and commands): raised at every change of what a
/// datagram or a command carries, so that two different versions never meet (the meeting's
/// agreement includes it). 2: the command's view turn (v1.9); 3: its Windows mouse (v1.10);
/// 6: its chat line (`PlayerCmd::say`); 7: each datagram ends with the sender's clock advantage
/// ([`Session::clock_ahead`]).
pub const PROTOCOL: u16 = 7;

pub(crate) const MAGIC: [u8; 4] = *b"OXLS";
const KIND_STEPS: u8 = 4;
pub(crate) const KIND_BYE: u8 = 5;
/// The New game video before the first level was skipped on the sending machine.
pub(crate) const KIND_SKIP: u8 = 6;
/// The sending machine went to the background (Android: the home screen) or came back (flag byte).
pub(crate) const KIND_PAUSE: u8 = 7;
/// How long a machine waits for the other one while that one says it is in the background (its
/// game loop stopped by the system), instead of the usual timeout.
pub const PAUSED_TIMEOUT: Duration = Duration::from_secs(300);
/// Largest datagram sent (bytes): under a usual path MTU.
const DATAGRAM_MAX: usize = 1200;
/// Most commands of one player repeated in one datagram (redundancy against loss and bursts):
/// one second of steps, the oldest unacknowledged first. The sending side's rate never depends on
/// what it receives: one datagram per new local step, a keep-alive while nothing new is to be
/// sent, so a congested link is never fed more than it can carry (a send on each receipt and a
/// growing repeat made the queue feed itself: round trips of seconds).
const REPEAT_MAX: usize = STEP_HZ as usize;
/// Fingerprints in each datagram: the newest one (a lost one is compared at another step).
const FP_REPEAT: usize = 1;
/// Time between two datagrams when nothing new is to be sent (seconds): the keep-alive, which
/// also carries the acknowledgements and the unacknowledged commands again during a stall.
const KEEP_ALIVE: f64 = 0.1;
/// The keep-alive while some of this machine's commands are not acknowledged yet (seconds): a
/// lost datagram is sent again sooner when no new step goes out (both machines waiting).
const KEEP_ALIVE_UNACKED: f64 = 1.0 / 30.0;
/// Clock advantage (see [`Session::clock_ahead`]): smoothing of each new estimate, how old the
/// two machines' newest estimates may be (seconds) and the largest one believed (steps: a larger
/// one is a map loading or a held window, not a clock running ahead).
const AHEAD_SMOOTH: f64 = 0.05;
const AHEAD_FRESH: f64 = 0.3;
const AHEAD_MAX: f64 = 30.0;
/// A pause longer than this (seconds) in this machine's pumping or in the other player's commands
/// makes the round trips of the next QUIET_AFTER seconds not count for the delay.
pub const QUIET_GAP: f64 = 0.25;
pub const QUIET_AFTER: f64 = 1.0;
/// Seconds a shorter input delay must be asked for before the delay shrinks by one step.
pub const SHRINK_AFTER: f64 = 2.0;
/// Input delay bounds (steps).
pub const DELAY_MIN: u64 = 1;
pub const DELAY_MAX: u64 = 40;
/// Largest input delay the measured round trip may ask for (steps: 200 ms).
pub const ADAPTIVE_MAX: u64 = 12;
/// Round trips the input delay follows: about the last 5 seconds (one per datagram received).
const RTT_WINDOW: usize = 300;
/// The adaptive input delay's margin follows the link's jitter (the 95th percentile of the round
/// trips over the shortest one), so that a Wi-Fi link's slower trips do not hold the game. Its
/// price: on a calm Wi-Fi link the delay is about 5 steps (83 ms) instead of 3 (50 ms). `false`
/// returns to the former rule (half the 75th percentile of the round trips) and changes nothing
/// else: the clock sync and the music's follow mode stay as they are.
pub const DELAY_FROM_JITTER: bool = true;

/// Input delay (steps, before the bounds) asked for by the round trips `sorted` (seconds, in
/// increasing order, not empty): from the jitter (`DELAY_FROM_JITTER`) or by the former rule.
fn adaptive_delay_wish(sorted: &[f64], from_jitter: bool) -> u64 {
    let step = f64::from(STEP);
    if from_jitter {
        // A Wi-Fi link: a short shortest round trip, and a few slower ones every second.
        // A command is late when its own trip is slow: half the shortest round trip plus the
        // 95th percentile's extra time (taken as one trip's, the other one quick). The rarest
        // spikes (5 %) and the holes (a pause, `quiet_until`) do not lengthen everybody's
        // delay: those wait.
        let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
        let one_way = sorted[0] / 2.0 + (p95 - sorted[0]);
        // A few milliseconds of margin, in steps, plus one step for the sending rhythm (commands
        // leave once a step): 2 or 3 steps on a clean local link (the round trips include the
        // frame each machine waits before reading its datagrams).
        ((one_way + 0.004) / step).ceil() as u64 + 1
    } else {
        // The former rule: half the 75th percentile of the round trips.
        let p75 = sorted[sorted.len() * 3 / 4];
        ((p75 / 2.0 + 0.004) / step).ceil() as u64 + 1
    }
}

/// Settings and save files belonging to one game, including while its VM changes maps.
#[derive(Clone, Debug, Default)]
pub struct SessionContext {
    /// Portable math is active through map loads, simulation and drawing for this game only.
    pub math: openxiii_math::Mode,
    /// Host's local time in seconds since 1970, as if UTC, at the meeting.
    pub clock: Option<i64>,
    /// The host's text and sound languages used by every map in this session.
    pub language: Option<(String, String)>,
    /// The host's User.ini and optional game ini; machine-local sections stay local.
    pub ini: Option<(String, Option<String>)>,
    /// The host mirrors session saves to its active profile. Guests keep a separate copy.
    pub host: bool,
    pub files: crate::session_disk::SessionFiles,
}

/// This machine's local date and time now, as [`SessionContext::clock`] counts it (the host's at the meeting).
pub fn local_clock_now() -> i64 {
    chrono::Local::now().naive_local().and_utc().timestamp()
}

/// LevelInfo's date fields at a level tick: in a session (`step`, the step running) with a
/// session clock `base`, that clock at the step; else this machine's clock now.
pub fn level_date(step: Option<u64>, base: Option<i64>) -> [i32; 8] {
    match step.zip(base).filter(|_| true) {
        Some((step, base)) => session_date(base, step as f64 * f64::from(STEP)),
        None => session_date(local_clock_now(), f64::from(chrono::Local::now().timestamp_subsec_millis()) / 1000.0),
    }
}

/// appSystemTime's eight fields (Year, Month, DayOfWeek from Sunday, Day, Hour, Minute, Second,
/// Millisecond) of the session clock `seconds` after its start `base`.
pub fn session_date(base: i64, seconds: f64) -> [i32; 8] {
    use chrono::{Datelike, Timelike};
    let ms = (seconds.max(0.0) * 1000.0) as i64;
    let t = chrono::DateTime::from_timestamp_millis(base.saturating_mul(1000).saturating_add(ms)).unwrap_or_default().naive_utc();
    [t.year(), t.month() as i32, t.weekday().num_days_from_sunday() as i32, t.day() as i32, t.hour() as i32, t.minute() as i32, t.second() as i32, (t.nanosecond() / 1_000_000).min(999) as i32]
}

/// The most the host's save files sent at the meeting weigh (its profile and side files).
const DISK_CAP: usize = 512 * 1024;

/// A settings section that describes this machine, not the game: its window and display, its
/// render and sound devices (the sound volume is each player's own), the window positions. It
/// keeps this machine's values in a session; the game never computes from them (the screen size
/// travels in the commands).
pub fn machine_local_section(section: &str) -> bool {
    let s = section.to_ascii_lowercase();
    ["openxiii.video", "windrv.windowsclient", "windowpositions", "firstrun"].contains(&s.as_str())
        || s.ends_with("renderdevice")
        || s.ends_with("audiosubsystem")
}

/// Whether the viewer handles network co-op travel through a lockstep session.
static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The improved viewer enables lockstep when it starts so players can host or join from the Co-op page.
pub fn set_wanted(on: bool) {
    WANTED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// See [`set_wanted`].
pub fn wanted() -> bool {
    WANTED.load(std::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------------
// Commands

/// Bits of [`PlayerCmd::flags`].
const F_PAD: u8 = 1;
const F_LOOK: u8 = 2;
const F_MOVE: u8 = 4;
const F_AIM: u8 = 8;
const F_RESET: u8 = 16;
const F_SIZE: u8 = 32;
const F_DEV: u8 = 64;
const F_TURN: u8 = 128;

/// What one player's devices gave the game during one step (see the module).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerCmd {
    /// Key, button and axis events (EInputKey code, EInputAction, value), in order.
    pub events: Vec<(u8, u8, f32)>,
    /// Raw gamepad state, read like a split-screen player's gamepad (coop.rs, `coop_pad_input`).
    pub pad: Option<PadState>,
    /// Look stick of the touch controls or of a gamepad read by the display layer, and its tune
    /// (speed, vertical inversion).
    pub look: Option<(f32, f32)>,
    pub look_tune: (f32, bool),
    /// Move stick (analog walking).
    pub mv: Option<(f32, f32)>,
    /// Aim assist values of the device just taken (iAutoAimMode, bRotationAssist).
    pub aim: Option<(i32, bool)>,
    /// The player's input was reset (window focus lost or gained).
    pub reset: bool,
    /// The player's screen took this size (applied by every machine at this step).
    pub size: Option<(i32, i32)>,
    /// Testing (development builds): a command every machine runs at this step (a level's end,
    /// players killed...), so that what it starts is measured like the rest of the game.
    pub dev: Option<String>,
    /// The view turn (yaw, pitch, rotator units) of this player's mouse look since the previous
    /// command, integrated frame by frame on the player's own machine (`lockstep_local_look`);
    /// the step gives it to the game where the mouse's value enters the view rotation.
    pub turn: Option<(f32, f32)>,
    /// The Windows mouse of this player's window (WM_MOUSEMOVE position and
    /// bWindowsMouseAvailable) when it changed: the menus read it when the mouse is not captured.
    pub wmouse: Option<(Option<(f32, f32)>, Option<bool>)>,
    /// The look sticks (gamepad, touch) of this player are in `turn`, integrated frame by frame
    /// as the mouse: the step does not turn the view from them again.
    pub stick_turned: bool,
    /// The chat line this player sent (lockstep_chat.rs): their console's command ("Say ...").
    pub say: Option<String>,
}

impl PlayerCmd {
    /// The command that continues `self` when a step has none of its own: held states kept, no
    /// new event.
    pub fn continuation(&self) -> PlayerCmd {
        PlayerCmd { events: Vec::new(), pad: self.pad, look: self.look, look_tune: self.look_tune, mv: self.mv, aim: None, reset: false, size: None, dev: None, turn: None, wmouse: None, stick_turned: self.stick_turned, say: None }
    }

    /// `later` taken in the same step as `self` (events of both, the latest held states).
    pub fn merge(&mut self, later: PlayerCmd) {
        self.events.extend(later.events);
        self.pad = later.pad;
        self.look = later.look;
        self.look_tune = later.look_tune;
        self.mv = later.mv;
        if later.aim.is_some() {
            self.aim = later.aim;
        }
        self.reset |= later.reset;
        if later.size.is_some() {
            self.size = later.size;
        }
        if later.dev.is_some() {
            self.dev = later.dev;
        }
        self.turn = match (self.turn, later.turn) {
            (Some(a), Some(b)) => Some((a.0 + b.0, a.1 + b.1)),
            (a, b) => a.or(b),
        };
        self.wmouse = match (self.wmouse, later.wmouse) {
            (Some(a), Some(b)) => Some((b.0.or(a.0), b.1.or(a.1))),
            (a, b) => b.or(a),
        };
        self.stick_turned = later.stick_turned;
        if later.say.is_some() {
            self.say = later.say;
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let mut flags = 0u8;
        if self.pad.is_some() {
            flags |= F_PAD;
        }
        if self.look.is_some() {
            flags |= F_LOOK;
        }
        if self.mv.is_some() {
            flags |= F_MOVE;
        }
        if self.aim.is_some() {
            flags |= F_AIM;
        }
        if self.reset {
            flags |= F_RESET;
        }
        if self.size.is_some() {
            flags |= F_SIZE;
        }
        if self.dev.is_some() {
            flags |= F_DEV;
        }
        if self.turn.is_some() {
            flags |= F_TURN;
        }
        out.push(flags);
        let n = self.events.len().min(255);
        out.push(n as u8);
        for &(k, a, v) in &self.events[..n] {
            out.extend_from_slice(&[k, a]);
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
        if let Some(p) = self.pad {
            let b = p.buttons.iter().enumerate().fold(0u16, |m, (i, &d)| if d { m | 1 << i } else { m });
            out.extend_from_slice(&b.to_le_bytes());
            for a in p.axes {
                out.extend_from_slice(&a.to_le_bytes());
            }
        }
        if let Some((x, y)) = self.look {
            out.extend_from_slice(&x.to_bits().to_le_bytes());
            out.extend_from_slice(&y.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&self.look_tune.0.to_bits().to_le_bytes());
        out.push(u8::from(self.look_tune.1));
        if let Some((x, y)) = self.mv {
            out.extend_from_slice(&x.to_bits().to_le_bytes());
            out.extend_from_slice(&y.to_bits().to_le_bytes());
        }
        if let Some((m, r)) = self.aim {
            out.extend_from_slice(&m.to_le_bytes());
            out.push(u8::from(r));
        }
        if let Some((w, h)) = self.size {
            out.extend_from_slice(&(w as u32).to_le_bytes());
            out.extend_from_slice(&(h as u32).to_le_bytes());
        }
        if let Some(d) = &self.dev {
            put_bytes(out, d.as_bytes());
        }
        if let Some((y, p)) = self.turn {
            out.extend_from_slice(&y.to_bits().to_le_bytes());
            out.extend_from_slice(&p.to_bits().to_le_bytes());
        }
        // Second flags byte: 1 Windows mouse position, 2 its availability (value in bit 4), 8 the
        // sticks in the turn, 16 a chat line.
        let (pos, avail) = self.wmouse.unwrap_or((None, None));
        out.push(u8::from(pos.is_some()) | u8::from(avail.is_some()) << 1 | u8::from(avail == Some(true)) << 2 | u8::from(self.stick_turned) << 3 | u8::from(self.say.is_some()) << 4);
        if let Some((x, y)) = pos {
            out.extend_from_slice(&x.to_bits().to_le_bytes());
            out.extend_from_slice(&y.to_bits().to_le_bytes());
        }
        if let Some(t) = &self.say {
            put_bytes(out, t.as_bytes());
        }
    }

    pub fn decode(r: &mut Reader) -> Option<PlayerCmd> {
        let flags = r.u8()?;
        let n = r.u8()?;
        let mut c = PlayerCmd::default();
        for _ in 0..n {
            let (k, a) = (r.u8()?, r.u8()?);
            c.events.push((k, a, f32::from_bits(r.u32()?)));
        }
        if flags & F_PAD != 0 {
            let b = r.u16()?;
            let mut p = PadState::default();
            for (i, d) in p.buttons.iter_mut().enumerate() {
                *d = b & (1 << i) != 0;
            }
            for a in p.axes.iter_mut() {
                *a = r.u32()? as i32;
            }
            c.pad = Some(p);
        }
        if flags & F_LOOK != 0 {
            c.look = Some((f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)));
        }
        c.look_tune = (f32::from_bits(r.u32()?), r.u8()? != 0);
        if flags & F_MOVE != 0 {
            c.mv = Some((f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)));
        }
        if flags & F_AIM != 0 {
            c.aim = Some((r.u32()? as i32, r.u8()? != 0));
        }
        c.reset = flags & F_RESET != 0;
        if flags & F_SIZE != 0 {
            c.size = Some((r.u32()? as i32, r.u32()? as i32));
        }
        if flags & F_DEV != 0 {
            c.dev = Some(String::from_utf8_lossy(r.bytes()?).into_owned());
        }
        if flags & F_TURN != 0 {
            c.turn = Some((f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)));
        }
        let more = r.u8()?;
        let pos = if more & 1 != 0 { Some((f32::from_bits(r.u32()?), f32::from_bits(r.u32()?))) } else { None };
        let avail = (more & 2 != 0).then_some(more & 4 != 0);
        c.stick_turned = more & 8 != 0;
        if pos.is_some() || avail.is_some() {
            c.wmouse = Some((pos, avail));
        }
        if more & 16 != 0 {
            c.say = Some(String::from_utf8_lossy(r.bytes()?).into_owned());
        }
        Some(c)
    }
}

/// Little-endian reader of a datagram.
pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn u64(&mut self) -> Option<u64> {
        Some(u64::from(self.u32()?) | u64::from(self.u32()?) << 32)
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u16()? as usize;
        self.take(n)
    }
}

pub(crate) fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len().min(u16::MAX as usize) as u16).to_le_bytes());
    out.extend_from_slice(&b[..b.len().min(u16::MAX as usize)]);
}

// ---------------------------------------------------------------------------------------------
// The simulation's side (inside the VM)

/// Lockstep state the simulation reads (Vm::lockstep).
#[derive(Debug, Default)]
pub struct SimSide {
    /// Step being simulated (from the session's start; it goes on across maps).
    pub step: u64,
    /// The player who last pressed Escape (the in-game menu's key): the one whose "Main menu"
    /// ends the session for both, the other machine says that player left.
    pub last_menu_player: Option<usize>,
    /// Commands of the step being simulated, by player (taken by that player's UpdateInput).
    pub cmds: Vec<Option<PlayerCmd>>,
    /// Screen size of each player (width, height), in player order.
    pub sizes: Vec<(i32, i32)>,
    /// Real gamepads of this machine, kept aside while the step runs with the players' ones.
    real_pads: Option<([Option<PadState>; 4], i32)>,
    /// Aim assist values given by this machine's device switch, for its own next command.
    pub(crate) local_aim: Option<(i32, bool)>,
    /// The player's input was reset by the window (focus), for its own next command.
    pub(crate) local_reset: bool,
    /// This machine's screen took a new size, for its own next command.
    pub local_size: Option<(i32, i32)>,
    /// The viewports' gamepads of this machine, kept aside while the step runs.
    real_viewport_pads: Option<Vec<Option<usize>>>,
    /// Each player's own settings (keys, mouse, view, gamepad), in player order.
    pub profiles: Vec<Option<Profile>>,
    /// What each player's settings were last applied to: controller, PlayerInput, HUD.
    applied: Vec<Option<(ObjId, Option<ObjId>, Option<ObjId>)>>,
    /// Random generator of this machine's drawing (see `lockstep_draw_begin`), and the game's
    /// kept aside meanwhile.
    draw_seed: u32,
    game_seed: Option<u32>,
    /// This machine's player's mouse look, integrated frame by frame (`lockstep_local_look`).
    pub local_look: LocalLook,
    /// Each player's look sticks are in this step's turn (see `PlayerCmd::stick_turned`).
    pub sticks_turned: Vec<bool>,
    /// The periodic full fingerprint being made (lockstep_full.rs).
    pub full: crate::lockstep_full::FullPrint,
    /// This machine's player (rank), when it has one (a replay has none).
    pub local_rank: Option<usize>,
    /// This machine's Windows mouse (position, availability) since the last command.
    pub local_wmouse: (Option<(f32, f32)>, Option<bool>),
    /// Each player's view turn of the step being simulated, kept when their UpdateInput takes
    /// the command (the mouse's value is read after that, in PlayerInput).
    pub turns: Vec<Option<(f32, f32)>>,
    /// The VM's window and drawing fields as the last step left them (`HostFields`), put back
    /// at the next step's start.
    host_fields: Option<HostFields>,
    /// This machine's player's chat line (lockstep_chat.rs), never the simulation's.
    pub chat: crate::lockstep_chat::ChatLine,
    /// The console's fields while this machine draws its open chat line.
    pub(crate) chat_draw: Option<crate::lockstep_chat::DrawSwap>,
    /// The last gamepad state this machine put in its player's command was not at rest
    /// (`cmd_pad`).
    pub(crate) pad_sent: bool,
}

/// The gamepad state of this machine's player's command: `first`, the first connected gamepad,
/// while the gamepad is the device in use (`active`, aim_device.rs) or is being used now (a
/// button or a stick or trigger beyond 0.3), and once more after that while the last state sent
/// held something (so that the gamepad lets go of what it held). Otherwise none, as for a
/// player without a gamepad: a gamepad left plugged in while its player plays with the keyboard
/// and mouse (or the touch controls) is not read for them (its aim assist, its sticks at rest).
pub(crate) fn cmd_pad(first: Option<PadState>, active: bool, sent: &mut bool) -> Option<PadState> {
    let Some(state) = first else {
        *sent = false;
        return None;
    };
    let used = pad_state_used(&state);
    if active || used || *sent {
        *sent = used;
        return Some(state);
    }
    None
}

/// A button is pressed or an axis is beyond 0.3 of its range (as the display layer's
/// `pad_in_use`).
pub(crate) fn pad_state_used(state: &PadState) -> bool {
    state.buttons.iter().any(|&b| b) || state.axes.iter().any(|&a| a.unsigned_abs() > 9830)
}

/// The VM's fields that are not objects but belong to this machine's window and drawing
/// (sizes, the viewport and player being drawn, the render flags): this machine's frames and
/// window events change them between two steps, the scripts of a step read them
/// (GETCURRENTRES, the HUD, the views).
#[derive(Debug, Clone, PartialEq)]
pub struct HostFields {
    viewport: (i32, i32),
    client_size: (i32, i32),
    display_window: (i32, i32),
    current_viewport: Option<ObjId>,
    render_player: Option<ObjId>,
    only_post_render: bool,
    precache_render: bool,
}

/// The local player's mouse look between two commands: what the game's per-frame input does
/// with the mouse (UPlayerInput's axis, sensitivity, SmoothMouse, then
/// XIIIPlayerController.UpdateRotation's 43 or 21.5 x DeltaTime), on this machine's own copy of
/// the smoothing state, never written to the game.
#[derive(Debug, Default, Clone)]
pub struct LocalLook {
    /// View turn (yaw, pitch) since the last command, for the next one.
    pub acc: (f32, f32),
    /// The mouse was taken by the look this frame (no menu, no console, mouse look).
    pub active: bool,
    /// The view turn this frame added (yaw, pitch), for the session's measurements.
    pub frame: (f32, f32),
    /// Measurements (the session's per-second log): frames the look took the mouse, raw mouse
    /// units it took, and frames it did not because of a menu, no mouse look or strafing, the
    /// mouse buffer not read (no capture).
    pub stats: [f64; 5],
    smooth: [f32; 2],
    borrowed: [f32; 2],
    zero_time: f64,
    clock: f64,
    /// The look ran since the last command: its sticks are in the turn (`PlayerCmd::stick_turned`).
    ran: bool,
    /// The joystick path's own state, as PlayerInput keeps it (ViewTurnAcc, ViewTurnBoost,
    /// iMemaTurnSign, ViewUpAcc, iMemaLookUpSign), frame by frame on this machine.
    stick_acc: f32,
    stick_boost: f32,
    stick_sign: i32,
    stick_up_acc: f32,
    stick_up_sign: i32,
}

/// One player's own settings, as their machine has them (lockstep co-op: every machine applies
/// them to that player): their User.ini (key bindings; mouse, view and HUD settings of their
/// controller, PlayerInput and HUD) and their gamepad settings.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub bindings: crate::input::Bindings,
    /// [section] and its (key, value) pairs, for the per-player sections only ([`PLAYER_SECTIONS`]).
    pub sections: Vec<(String, Vec<(String, String)>)>,
    pub pad: crate::gamepad_config::PlayerPad,
    /// Their machine's Graphics setting (Enhanced: true), when sent: the host's is the session's.
    pub enhanced: Option<bool>,
    /// Their name in the co-op chat (the Co-op page's), when they have one.
    pub name: Option<String>,
}

/// User.ini sections whose values belong to one player (their controller, PlayerInput, HUD).
pub const PLAYER_SECTIONS: [&str; 5] = ["Engine.PlayerController", "XIII.XIIIPlayerController", "Engine.Controller", "Engine.PlayerInput", "Engine.HUD"];

impl Profile {
    /// From a player's User.ini text and their gamepad settings text (its first player's section
    /// is theirs).
    pub fn from_texts(user_ini: &str, gamepad: &str) -> Profile {
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut current: Option<usize> = None;
        for line in user_ini.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = if PLAYER_SECTIONS.iter().any(|s| s.eq_ignore_ascii_case(name)) {
                    sections.push((name.to_string(), Vec::new()));
                    Some(sections.len() - 1)
                } else {
                    None
                };
                continue;
            }
            if let (Some(i), Some((k, v))) = (current, line.split_once('=')) {
                sections[i].1.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        Profile { bindings: crate::input::Bindings::parse(user_ini), sections, pad: crate::gamepad_config::GamepadConfig::parse(gamepad).players[0], enhanced: None, name: None }
    }
}

impl SimSide {
    /// This machine is drawing its own view (between `lockstep_draw_begin` and `_end`).
    pub fn drawing(&self) -> bool {
        self.game_seed.is_some()
    }
}

/// Fingerprint kinds (index in [`Vm::lockstep_fingerprint`]'s result).
pub const FP_KINDS: [&str; 4] = ["random", "time", "players", "actors"];

impl Vm {
    /// A lockstep session is running.
    pub fn lockstep_on(&self) -> bool {
        self.lockstep.is_some()
    }

    /// Starts the simulation's side of a lockstep session: every player's screen size, in player
    /// order. The portable transcendental functions are used from now on.
    pub fn lockstep_start(&mut self, sizes: Vec<(i32, i32)>) {
        self.session.math = openxiii_math::Mode::Portable;
        self.engine.client.lockstep_sizes = Some(sizes.clone());
        let step = self.lockstep.as_ref().map_or(0, |l| l.step);
        self.lockstep = Some(Box::new(SimSide { step, sizes, ..Default::default() }));
        // The sound engine moves its channels at the game's pace on every machine (with or
        // without an audio output); a real output only plays what they passed over.
        if let Some(hx) = self.audio.as_mut() {
            hx.engine.stream_follow_rate = FOLLOW_RATE;
            hx.engine.external_audio = false;
        }
    }

    /// Release this game's negotiated settings when returning to the local menus.
    pub fn lockstep_stop(&mut self) {
        if let Some(hx) = self.audio.as_mut() {
            hx.engine.stream_follow_rate = 0;
        }
        self.lockstep = None;
        self.session = Default::default();
        self.engine.client.session_graphics = Default::default();
        self.engine.client.lockstep_sizes = None;
    }

    /// Every player's own settings (player order).
    pub fn lockstep_set_profiles(&mut self, profiles: Vec<Option<Profile>>) {
        // The host's Graphics setting is the session's on every machine (it changes what the game
        // computes); without it (an older machine, a replay without profiles), this machine's.
        let shared = profiles.first().and_then(|p| p.as_ref()?.enhanced);
        self.engine.client.session_graphics.host = shared;
        if let Some(l) = self.lockstep.as_mut() {
            l.profiles = profiles;
            l.applied.clear();
        }
    }

    /// Lockstep co-op: the rank and own bindings of the player whose controller is `pc`.
    pub(crate) fn lockstep_profile_bindings(&self, pc: ObjId) -> Option<(usize, crate::input::Bindings)> {
        let l = self.lockstep.as_ref()?;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        Some((rank, l.profiles.get(rank)?.as_ref()?.bindings.clone()))
    }

    /// Lockstep co-op: the gamepad settings of the player of rank `rank`.
    pub(crate) fn lockstep_pad_settings(&self, rank: usize) -> Option<crate::gamepad_config::PlayerPad> {
        self.lockstep.as_ref()?.profiles.get(rank)?.as_ref().map(|p| p.pad)
    }

    /// What a start loaded from a checkpoint needs beyond its URL, as blob entries (the host
    /// sends them to the guest, and both start from them): the host's rand() seeds, the music kept
    /// by UAudioSaver, and the save device's checkpoints of the map start and the last one (a later
    /// Retry loads them).
    pub fn lockstep_start_entries(&self) -> Vec<(String, Vec<u8>)> {
        let a = &self.engine.audio_saver;
        let mut b = Vec::new();
        b.extend_from_slice(&a.flag.to_le_bytes());
        put_bytes(&mut b, a.name.as_bytes());
        for p in a.params {
            b.extend_from_slice(&p.to_le_bytes());
        }
        b.extend_from_slice(&(a.vars.len() as u32).to_le_bytes());
        for (n, v, t) in &a.vars {
            put_bytes(&mut b, n.as_bytes());
            b.extend_from_slice(&v.to_le_bytes());
            b.push(*t);
        }
        let mut out = vec![("start/audio_saver".to_string(), b)];
        // The host's rand() seeds (its clock's, as each machine seeds its own game): every machine's
        // new VM starts from them, else each would draw its own sequence.
        let (seed, hx) = self.lockstep_seeds();
        let mut s = seed.to_le_bytes().to_vec();
        if let Some(h) = hx {
            s.extend_from_slice(&h.to_le_bytes());
        }
        out.push(("start/seed".into(), s));
        if let Some(d) = self.engine.save.as_ref() {
            out.push(("start/save_start".into(), d.start.clone()));
            out.push(("start/save_start_desc".into(), d.start_desc.clone().into_bytes()));
            out.push(("start/save_checkpoint".into(), d.checkpoint.clone()));
            out.push(("start/save_checkpoint_desc".into(), d.checkpoint_desc.clone().into_bytes()));
            // The session's save files: the host's profile's (session_disk.rs).
            out.push(("start/save_profile".into(), d.profile.clone().into_bytes()));
            out.extend(crate::session_disk::entries(&d.dir, &d.profile, DISK_CAP));
        }
        out
    }

    /// See [`Vm::lockstep_start_entries`]: given to this machine's VM before the start map loads
    /// (the music) and again once it is loaded (the checkpoints, when the save device exists).
    pub fn lockstep_apply_start_entries(&mut self, entries: &[(String, Vec<u8>)]) {
        let get = |n: &str| entries.iter().find(|e| e.0 == n).map(|e| e.1.as_slice());
        if let Some(b) = get("start/seed") {
            let word = |i: usize| b.get(i..i + 4).map(|w| u32::from_le_bytes(w.try_into().unwrap()));
            if let Some(seed) = word(0) {
                self.set_lockstep_seeds(seed, word(4));
            }
        }
        if let Some(b) = get("start/audio_saver") {
            let mut r = Reader::new(b);
            let mut a = crate::save::AudioSaver::default();
            let ok = (|| {
                a.flag = r.u32()? as i32;
                a.name = String::from_utf8_lossy(r.bytes()?).into_owned();
                for p in a.params.iter_mut() {
                    *p = r.u32()? as i32;
                }
                let n = r.u32()?;
                for _ in 0..n {
                    let name = String::from_utf8_lossy(r.bytes()?).into_owned();
                    let v = r.u32()? as i32;
                    a.vars.push((name, v, r.u8()?));
                }
                Some(())
            })();
            if ok.is_some() {
                self.engine.audio_saver = a;
            }
        }
        if let Some(d) = self.engine.save.as_mut() {
            let text = |n: &str| get(n).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            // The session's profile (the host's) and its files, in place of this machine's.
            if let Some(host) = get("start/save_profile").map(|b| String::from_utf8_lossy(b).into_owned()) {
                // Once per session (a later map keeps the session's files as the game left them).
                if !self.session.files.active() {
                    self.session.files.set(Some(crate::session_disk::from_entries(entries, &host, None, self.session.host)));
                }
                self.session.files.set_own_if_missing(&d.dir, &d.profile);
                if !host.is_empty() {
                    d.place_session_profile(&host);
                }
            }
            if let Some(b) = get("start/save_start") {
                d.start = b.to_vec();
                d.start_desc = text("start/save_start_desc");
            }
            if let Some(b) = get("start/save_checkpoint") {
                d.checkpoint = b.to_vec();
                d.checkpoint_desc = text("start/save_checkpoint_desc");
            }
        }
    }

    /// Each player's own settings on their objects, when those are new (a map's first step, a
    /// controller made again): their bindings in their UInput, their User.ini values on their
    /// controller, PlayerInput and HUD (the class defaults stay the host's on every machine).
    fn lockstep_apply_profiles(&mut self) {
        let Some(l) = self.lockstep.as_ref() else { return };
        if l.profiles.iter().all(Option::is_none) {
            return;
        }
        let players = self.coop.players.clone();
        for (rank, &pc) in players.iter().enumerate() {
            let Some(profile) = self.lockstep.as_ref().and_then(|l| l.profiles.get(rank).cloned().flatten()) else { continue };
            if self.objects.get(pc).is_none_or(|o| o.is_none()) {
                continue;
            }
            // Bindings: a UInput of this controller built from someone else's keys.
            let built = if self.input_sys.as_ref().is_some_and(|s| s.pc == pc) {
                self.input_sys.as_ref().map(|s| s.profile)
            } else {
                self.other_inputs.iter().find(|s| s.pc == pc).map(|s| s.profile)
            };
            if built.is_some_and(|b| b != Some(rank)) {
                self.input_init(pc, &profile.bindings);
            }
            let live = |vm: &Vm, o: Option<ObjId>| o.filter(|&x| vm.objects.get(x).is_some_and(|y| y.is_some()));
            let input = live(self, self.prop(pc, "PlayerInput").as_object());
            let hud = live(self, self.prop(pc, "myHUD").as_object());
            let now = (pc, input, hud);
            let before = self.lockstep.as_ref().and_then(|l| l.applied.get(rank).copied().flatten());
            if before == Some(now) {
                continue;
            }
            for (section, pairs) in &profile.sections {
                let target = match section.to_ascii_lowercase().as_str() {
                    "engine.playerinput" => input,
                    "engine.hud" => hud,
                    _ => Some(pc),
                };
                let Some(target) = target else { continue };
                for (key, value) in pairs {
                    self.lockstep_set_config_prop(target, key, value);
                }
            }
            if let Some(l) = self.lockstep.as_mut() {
                if l.applied.len() <= rank {
                    l.applied.resize(rank + 1, None);
                }
                l.applied[rank] = Some(now);
            }
        }
    }

    /// A config property of `obj` set from its ini text (as UObject::LoadConfig imports it).
    fn lockstep_set_config_prop(&mut self, obj: ObjId, key: &str, value: &str) {
        let Some(class) = self.objects.get(obj).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let Some(&i) = self.classes[class].prop_index.get(&key.to_ascii_lowercase()) else { return };
        let p = self.classes[class].props[i].clone();
        if p.flags & crate::config::CPF_CONFIG == 0 || p.dim != 1 || matches!(p.ty, crate::types::Type::Array(_)) {
            return;
        }
        let mut v = self.propr(obj, &p.name).clone();
        let chars: Vec<char> = value.chars().collect();
        if self.import_text(&p.ty, p.enumeration.as_deref(), &chars, 0, 0, &mut v).is_some() {
            self.set_prop(obj, &p.name, v);
        }
    }

    /// A player's screen size changed (applied by both machines at the same step).
    pub fn lockstep_set_size(&mut self, player: usize, size: (i32, i32)) {
        if let Some(l) = self.lockstep.as_mut() {
            if l.sizes.len() <= player {
                l.sizes.resize(player + 1, size);
            }
            l.sizes[player] = size;
            self.engine.client.lockstep_sizes = Some(l.sizes.clone());
        }
    }

    /// Screen size of player `player` in the lockstep session.
    pub fn lockstep_size(&self, player: usize) -> Option<(i32, i32)> {
        self.lockstep.as_ref().and_then(|l| l.sizes.get(player).copied())
    }

    /// Time of the session's steps (seconds), in place of the PC's clock inside the simulation.
    pub fn lockstep_clock(&self) -> Option<f64> {
        self.lockstep.as_ref().map(|l| l.step as f64 / f64::from(STEP_HZ))
    }

    /// This machine's player (`rank`) at one frame of `dt` seconds: the mouse moves of the frame
    /// turn the view as the game's own per-frame input would (solo, split screen), on the
    /// machine's own copy of the smoothing, into the next command's turn (`PlayerCmd::turn`); the
    /// moves are taken out of the mouse buffer, so the step does not count them again. Nothing is
    /// taken while a menu, a message box or the console has the input (the menus read the mouse),
    /// with strafing by mouse or without mouse look (the mouse moves the player there).
    pub fn lockstep_local_look(&mut self, rank: usize, dt: f32) {
        if let Some(l) = self.lockstep.as_mut() {
            l.local_look.frame = (0.0, 0.0);
        }
        let Some(pc) = self.coop.players.get(rank).copied().filter(|&p| self.alive(p)) else { return };
        let Some(input) = self.po(pc, "PlayerInput") else { return };
        if let Some(l) = self.lockstep.as_mut() {
            l.local_look.ran = true;
        }
        let in_menu = self.propr(pc, "bIsInMenu").as_bool() || self.coop_menu_open(pc) || self.coop_end_menu_open(pc);
        let mouse_look = self.propr(pc, "bAlwaysMouseLook").as_bool() || self.propr(pc, "bLook").as_int() != 0;
        let strafe = self.propr(pc, "bStrafe").as_int() != 0;
        if in_menu || !mouse_look || strafe {
            if let Some(l) = self.lockstep.as_mut() {
                l.local_look.stats[if in_menu { 2 } else { 3 }] += 1.0;
            }
            return;
        }
        // The look sticks (this machine's gamepad, else the touch or display layer's stick):
        // the joystick path of DealWithPlayerInputEvent frame by frame (lockstep_stick_turn).
        #[allow(unused_mut)]
        let mut stick = self.lockstep_stick_turn(rank, pc, input, dt);
        let Some((dx, dy)) = self.win_input.take_mouse_axes() else {
            if let Some(l) = self.lockstep.as_mut() {
                l.local_look.stats[4] += 1.0;
                l.local_look.acc.0 += stick.0;
                l.local_look.acc.1 += stick.1;
                l.local_look.frame = stick;
                if stick != (0.0, 0.0) {
                    l.local_look.active = true;
                }
            }
            return;
        };
        if let Some(l) = self.lockstep.as_mut() {
            l.local_look.stats[0] += 1.0;
            l.local_look.stats[1] += f64::from(dx.abs() + dy.abs());
        }
        // The player's own binding of the axes (Axis aMouseX Speed=6.0): value × Invert × Speed × 0.01.
        let scale = |key: &str| -> f32 {
            let cmd = self.lockstep.as_ref().and_then(|l| l.profiles.get(rank)?.as_ref()?.bindings.command(key).map(str::to_string)).unwrap_or_default();
            let lower = cmd.to_ascii_lowercase();
            let num = |tag: &str, default: f32| lower.split_whitespace().find_map(|w| w.strip_prefix(tag)?.parse::<f32>().ok()).unwrap_or(default);
            num("speed=", 1.0) * num("invert=", 1.0) * 0.01
        };
        let (kx, ky) = (scale("MouseX"), scale("MouseY"));
        let fov = crate::ai_move::fmul_d(self.propr(pc, "DesiredFOV").as_float(), 0.01111);
        let sens = fov * self.propr(input, "MouseSensitivity").as_float();
        let max_smoothing = self.propr(input, "bMaxMouseSmoothing").as_bool();
        let threshold = self.propr(input, "MouseSmoothThreshold").as_float();
        let invert = self.propr(input, "bInvertMouse").as_bool();
        let zoomed = self.propr(pc, "DesiredFOV").as_float() != self.propr(pc, "DefaultFOV").as_float();
        let dilation = self.level.map(|l| self.propr(l, "TimeDilation").as_float()).filter(|&d| d != 0.0).unwrap_or(1.0);
        let Some(l) = self.lockstep.as_mut() else { return };
        let look = &mut l.local_look;
        look.clock += f64::from(dt * dilation);
        // UInput::ReadInput (input_read): the axes of the frame × 20 / DeltaTime.
        let read = if dt > 0.0 { 20.0 / dt } else { 0.0 };
        let (mx, my) = (sens * (dx * kx * read), sens * (-dy * ky * read));
        let since = ((look.clock - look.zero_time) as f32) / dilation;
        let (mut sx, mut bx) = (look.smooth[0], look.borrowed[0]);
        let keep_x = crate::player_input::smooth_mouse(max_smoothing, threshold, mx, since, &mut sx, &mut bx);
        let (mut sy, mut by) = (look.smooth[1], look.borrowed[1]);
        let keep_y = crate::player_input::smooth_mouse(max_smoothing, threshold, my, since, &mut sy, &mut by);
        look.smooth = [keep_x, keep_y];
        look.borrowed = [bx, by];
        if mx != 0.0 || my != 0.0 {
            look.zero_time = look.clock;
        }
        // UpdateRotation: 43 (21.5 zoomed) × DeltaTime × aTurn, aTurn being the smoothed value × 0.24.
        let k = if zoomed { 21.5 } else { 43.0 } * dt * dilation * 0.24;
        let pitch = if invert { -sy } else { sy };
        look.acc.0 += k * sx + stick.0;
        look.acc.1 += k * pitch + stick.1;
        look.frame = (k * sx + stick.0, k * pitch + stick.1);
        look.active = true;
    }

    /// Drawing only (the instant look, lockstep_play.rs): does player `pc`'s look turn the view at
    /// this step? Not while a cinematic shows a player's view (co-op), nor while the controller's
    /// state keeps the rotation whatever the look (XIIIPlayerController: NoControl, which the
    /// cinematics set, Toits01's grapple demonstration included; Kicked, WaitForFirstDisplay; the
    /// view of a camera, CameraView and BossView; a video; the end of the game), nor in NoMove,
    /// whose look is slower and held within a few degrees: the drawn view would go past where
    /// the steps put it, then jump back.
    pub fn lockstep_look_follows(&self, pc: ObjId) -> bool {
        const HELD: [&str; 12] = ["NoControl", "NoMove", "Kicked", "WaitForFirstDisplay", "CameraView", "BossView", "PlayingVideo", "GameEnded", "GameEndedDeath", "GameEndedDrown", "GameEndedFalling", "GameEndedSuccess"];
        if self.coop_cine_shown_rank().is_some() || !self.alive(pc) {
            return false;
        }
        match self.current_state(pc) {
            Some((_, s)) => !HELD.iter().any(|h| s.name.eq_ignore_ascii_case(h)),
            None => true,
        }
    }

    /// This frame's view turn (yaw, pitch, rotator units) from player `rank`'s look stick on this
    /// machine: the stick as the console bindings read it ("JoyU Axis aTurn SpeedBase=1.0
    /// DeadZone=0.4", "JoyV Axis aLookup … INVERT=-1", look_stick_axes, with the player's gamepad
    /// speed and inversion), × 20 / DeltaTime (UInput::ReadInput), then DealWithPlayerInputEvent's
    /// joystick path over `dt` (60 × dt × fLookSpeed, bInverseLook, the ConfigType 1 curve, ViewTurnAcc / ViewTurnBoost
    /// and ViewUpAcc, × 2.42), the field of view (DesiredFOV × 0.01111), × 0.24, and
    /// UpdateRotation's 43 (21.5 zoomed) × dt. The step then gives it to the game as the mouse's
    /// turn and does not read the stick for the view again (`PlayerCmd::stick_turned`).
    fn lockstep_stick_turn(&mut self, rank: usize, pc: ObjId, input: ObjId, dt: f32) -> (f32, f32) {
        let dt = dt;
        let dead = |v: i32| if v.abs() < 6554 { 0.0 } else { v as f32 / 32768.0 };
        // This machine's gamepad, while its player plays with it (as `cmd_pad`).
        let active = self.aim_device.pad_active;
        let real = self.pads.states.iter().flatten().next().copied().filter(|p| active || pad_state_used(p));
        let (x, y, tune) = match real {
            // The right stick is lRx / lRy (axes 3 and 4, as coop_pad_input reads it); lZ (axis 2)
            // is the combined triggers.
            Some(p) => (dead(p.axes[3]), dead(p.axes[4]), self.lockstep_pad_settings(rank).map_or((1.0, false), |s| s.look_tune())),
            None => match self.look_stick {
                Some((x, y)) => (x, y, self.look_stick_tune),
                None => (0.0, 0.0, (1.0, false)),
            },
        };
        let (speed, invert_y) = tune;
        let y = if invert_y { -y } else { y };
        let axis = |v: f32| if v.abs() > 0.4 { v.signum() * (v.abs() - 0.4) / 0.6 * speed } else { 0.0 };
        let f = |n: &str| self.propr(pc, n).as_float();
        // The axis as DirectAxis leaves it (× the gamepad's duration), × 20 / DeltaTime
        // (UInput::ReadInput), × 60 × DeltaTime (DealWithPlayerInputEvent): DeltaTime cancels.
        let k = crate::pad::pad_look_delta() * 20.0 * 59.999_996;
        let look_speed = f("fLookSpeed");
        let mut turn = k * look_speed * axis(x);
        let mut up = k * look_speed * axis(y);
        if self.propr(pc, "bInverseLook").as_bool() {
            up = -up;
        }
        let range = self.propr(input, "fInputRange").as_float();
        if self.propr(pc, "ConfigType").as_byte() == 1 && turn != 0.0 && up != 0.0 && range != 0.0 {
            let r = ((turn * turn + up * up).sqrt() / range).min(1.0);
            let curve = r * r * r * r;
            turn *= curve;
            up *= curve;
        }
        let zoomed = f("DesiredFOV") != f("DefaultFOV");
        let fov = crate::ai_move::fmul_d(f("DesiredFOV"), 0.01111);
        let dilation = self.level.map(|l| self.propr(l, "TimeDilation").as_float()).filter(|&d| d != 0.0).unwrap_or(1.0);
        let Some(l) = self.lockstep.as_mut() else { return (0.0, 0.0) };
        let s = &mut l.local_look;
        // ViewTurnAcc / ViewTurnBoost (the walking part of the boost is left out: aBaseY 0).
        if turn <= 0.0 {
            if 0.0 <= turn {
                s.stick_acc = (s.stick_acc - dt * 6.0).max(0.0);
                s.stick_boost = 0.0;
            } else {
                if s.stick_sign < 0 {
                    s.stick_acc = (dt * 3.0 + s.stick_acc).min(1.0);
                } else {
                    s.stick_acc = 0.0;
                    s.stick_boost = 0.0;
                }
                s.stick_boost = if s.stick_acc != 1.0 || range * -0.95 <= turn { 0.0 } else { (dt + s.stick_boost).min(1.0) };
                s.stick_sign = -1;
            }
        } else {
            if s.stick_sign < 0 {
                s.stick_acc = 0.0;
                s.stick_boost = 0.0;
            } else {
                s.stick_acc = (dt * 3.0 + s.stick_acc).min(1.0);
            }
            s.stick_boost = if s.stick_acc != 1.0 || turn <= range * 0.9 { 0.0 } else { (dt + s.stick_boost).min(1.0) };
            s.stick_sign = 1;
        }
        let turn = (s.stick_boost + s.stick_acc) * turn;
        if up <= 0.0 {
            if 0.0 <= up {
                s.stick_up_acc = (s.stick_up_acc - dt * 6.0).max(0.0);
            } else if s.stick_up_sign < 0 {
                s.stick_up_acc = (dt * 3.0 + s.stick_up_acc).min(1.0);
                s.stick_up_sign = -1;
            } else {
                s.stick_up_acc = 0.0;
                s.stick_up_sign = -1;
            }
        } else if s.stick_up_sign < 0 {
            s.stick_up_acc = 0.0;
            s.stick_up_sign = 1;
        } else {
            s.stick_up_acc = (dt * 3.0 + s.stick_up_acc).min(1.0);
            s.stick_up_sign = 1;
        }
        let up = s.stick_up_acc * up;
        // × 2.42 (the look up × −2.42: aLookUp's sign, the view's pitch up positive), the field of
        // view, × 0.24, UpdateRotation's 43 × DeltaTime.
        let k = if zoomed { 21.5 } else { 43.0 } * dt * dilation * 0.24 * fov;
        (k * crate::ai_move::fmul_d(turn, 2.42), k * crate::ai_move::fmul_d(up, -2.42))
    }

    /// Player `pc` belongs to the other machine (lockstep co-op): what its menus and bindings ask
    /// of the window or of this machine's display settings (fullscreen, resolution, the Video
    /// page) is that machine's own business, not done here.
    pub fn lockstep_foreign(&self, pc: ObjId) -> bool {
        let Some(l) = self.lockstep.as_ref() else { return false };
        let Some(local) = l.local_rank else { return false };
        self.coop.players.iter().position(|&p| p == pc).is_some_and(|rank| rank != local)
    }

    /// The player controller owning a menu page (its root window's viewport's Actor).
    pub fn page_owner(&self, page: ObjId) -> Option<ObjId> {
        let root = self.propr(page, "Controller").as_object().or_else(|| self.propr(page, "myRoot").as_object()).or_else(|| self.propr(page, "Root").as_object())?;
        let viewport = self.propr(root, "ViewportOwner").as_object()?;
        self.propr(viewport, "Actor").as_object()
    }

    /// The view turn the step gives player `pc`'s mouse (lockstep co-op, `PlayerCmd::turn`), if
    /// that player's machine integrated it.
    pub(crate) fn lockstep_turn_of(&self, pc: ObjId) -> Option<(f32, f32)> {
        let l = self.lockstep.as_ref()?;
        let rank = self.coop.players.iter().position(|&p| p == pc)?;
        *l.turns.get(rank)?
    }

    /// Player `pc`'s look sticks are in their command's turn this step (lockstep co-op): the step
    /// does not turn the view from them again (coop_pad_input's right stick).
    pub(crate) fn lockstep_sticks_turned(&self, pc: ObjId) -> bool {
        let Some(l) = self.lockstep.as_ref() else { return false };
        self.coop.players.iter().position(|&p| p == pc).and_then(|r| l.sticks_turned.get(r).copied()).unwrap_or(false)
    }

    /// What this machine's devices gave since the previous step, taken out of the VM so that the
    /// simulation never reads them itself: the window's queued events (keys, typed characters,
    /// mouse axes and buttons), the buffered mouse, the first connected gamepad (`pad`), the look
    /// and move sticks, the aim assist of a device switch. `keys`: the keyboard and mouse part.
    pub fn lockstep_harvest(&mut self, keys: bool, pad: bool) -> PlayerCmd {
        let mut c = PlayerCmd::default();
        if keys {
            if let Some(s) = self.input_sys.as_mut() {
                c.events = std::mem::take(&mut s.queue);
            }
            if self.win_input.active {
                c.events.extend(crate::windrv_input::update_input_mouse(&mut self.win_input));
            }
            // The chat line takes its keys on this machine; only a sent line goes in the command.
            let (events, say) = self.lockstep_chat_filter(std::mem::take(&mut c.events));
            c.events = events;
            c.say = say;
            c.look = self.look_stick;
            c.look_tune = self.look_stick_tune;
            if let Some(l) = self.lockstep.as_mut() {
                let w = std::mem::take(&mut l.local_wmouse);
                if w.0.is_some() || w.1.is_some() {
                    c.wmouse = Some(w);
                }
                // The sticks are in the turn when the look ran for this player (and a gamepad or
                // stick is this command's).
                c.stick_turned = std::mem::take(&mut l.local_look.ran) && pad;
                if std::mem::take(&mut l.local_look.active) || l.local_look.acc != (0.0, 0.0) {
                    // Whole rotator units (the step's UpdateRotation adds whole units): the rest
                    // waits for the next command.
                    let a = l.local_look.acc;
                    let whole = (a.0.round(), a.1.round());
                    l.local_look.acc = (a.0 - whole.0, a.1 - whole.1);
                    c.turn = Some(whole);
                }
            }
            c.mv = self.move_stick;
            if let Some(l) = self.lockstep.as_mut() {
                c.aim = l.local_aim.take();
                c.reset = std::mem::take(&mut l.local_reset);
                c.size = l.local_size.take();
            }
        } else {
            c.look_tune = self.look_stick_tune;
        }
        if pad {
            let real = self.lockstep.as_ref().and_then(|l| l.real_pads.as_ref()).map_or(self.pads.states, |r| r.0);
            let first = real.iter().flatten().next().copied();
            // The gamepad goes in the command only while this machine's player plays with it
            // (`cmd_pad`): a gamepad left plugged in while the keyboard and mouse are used is not
            // that player's device.
            let active = self.aim_device.pad_active;
            if let Some(l) = self.lockstep.as_mut() {
                c.pad = cmd_pad(first, active, &mut l.pad_sent);
            } else {
                c.pad = first;
            }
        }
        c
    }

    /// Start of step `step`: every player's command (player order). The gamepads the simulation
    /// sees are the players' (player `i`'s on handle `i`), this machine's real ones are kept aside.
    pub fn lockstep_begin_step(&mut self, step: u64, cmds: Vec<PlayerCmd>) {
        let _math = self.session.math.enter();
        let Some(l) = self.lockstep.as_mut() else { return };
        l.step = step;
        self.engine.client.session_graphics.in_step = true;
        // What this machine's frames and window changed since the last step is not the game's.
        if let Some(h) = l.host_fields.clone() {
            self.viewport = h.viewport;
            self.client_size = h.client_size;
            self.display_window = h.display_window;
            // (The viewport and player being drawn are objects, None outside a view; a map change
            // remakes them: not put back.)
            self.only_post_render = h.only_post_render;
            self.precache_render = h.precache_render;
        }
        let Some(l) = self.lockstep.as_mut() else { return };
        // Each player's view turn is taken with their command by their UpdateInput.
        l.turns.clear();
        l.sticks_turned.clear();
        if l.real_pads.is_none() {
            l.real_pads = Some((self.pads.states, self.pads.count));
            l.real_viewport_pads = Some(self.engine.client.viewport_pads.clone());
        }
        // Screen sizes changed at this step.
        let sizes: Vec<(usize, (i32, i32))> = cmds.iter().enumerate().filter_map(|(i, c)| c.size.map(|s| (i, s))).collect();
        let mut states = [None; 4];
        for (i, c) in cmds.iter().enumerate().take(4) {
            states[i] = c.pad;
        }
        l.cmds = cmds.into_iter().map(Some).collect();
        for (i, size) in sizes {
            self.lockstep_set_size(i, size);
        }
        self.lockstep_apply_profiles();
        // An emitter's last frame drawn once (its animation ended): drawn by the frames after the
        // step that ended it, cleared here on every machine (particles.rs, particle_frame).
        for s in self.particles.states.values_mut() {
            s.last_draw = None;
        }
        self.pads.states = states;
        self.pads.count = PLAYERS as i32;
        // Each player's own gamepad on their own viewport.
        let slots = self.engine.client.viewports.clone();
        if self.engine.client.viewport_pads.len() < 4 {
            self.engine.client.viewport_pads.resize(4, None);
        }
        for (i, slot) in slots.iter().enumerate().take(4) {
            if let Some(v) = self.engine.client.viewport_pads.get_mut(usize::from(*slot)) {
                *v = states[i].map(|_| i);
            }
        }
    }

    /// End of a step: this machine's real gamepads back for its display layer.
    pub fn lockstep_end_step(&mut self) {
        let _math = self.session.math.enter();
        self.engine.client.session_graphics.in_step = false;
        let fields = self.lockstep_host_fields();
        let Some(l) = self.lockstep.as_mut() else { return };
        l.host_fields = Some(fields);
        if let Some((states, count)) = l.real_pads.take() {
            self.pads.states = states;
            self.pads.count = count;
        }
        if let Some(v) = l.real_viewport_pads.take() {
            self.engine.client.viewport_pads = v;
        }
        l.cmds.clear();
        // The actors' places of this step, for the frames drawn until the next one.
        self.lockstep_interp_keep();
    }

    /// This machine draws its own player's view (a frame, not a step): what drawing draws at
    /// random (particles, shadows...) comes from a generator of its own, so that the game's
    /// stays the same on every machine. `lockstep_draw_end` gives the game's back.
    pub fn lockstep_draw_begin(&mut self) {
        let seed = self.seed;
        if let Some(l) = self.lockstep.as_mut() {
            if l.game_seed.is_none() {
                l.game_seed = Some(seed);
                self.seed = l.draw_seed;
            }
        }
        self.lockstep_chat_draw_begin();
    }

    /// See [`Vm::lockstep_draw_begin`].
    pub fn lockstep_draw_end(&mut self) {
        self.lockstep_chat_draw_end();
        let seed = self.seed;
        if let Some(l) = self.lockstep.as_mut() {
            if let Some(g) = l.game_seed.take() {
                l.draw_seed = seed;
                self.seed = g;
            }
        }
    }

    /// Player rank (0…) of controller `pc` in the lockstep session.
    fn lockstep_rank(&self, pc: ObjId) -> Option<usize> {
        self.lockstep.as_ref()?;
        self.coop.players.iter().position(|&p| p == pc)
    }

    /// UpdateInput of player `pc` in a lockstep session: their command of this step, through the
    /// same paths as a split-screen player's devices. True when the session handles the
    /// player (nothing else must read a device for them). `param`: 1 from UInput::ResetInput (no
    /// device read then).
    pub(crate) fn lockstep_update_input(&mut self, pc: ObjId, param: u32) -> bool {
        let Some(rank) = self.lockstep_rank(pc) else { return self.lockstep.is_some() };
        if param != 0 {
            return true;
        }
        let Some(cmd) = self.lockstep.as_mut().and_then(|l| l.cmds.get_mut(rank)).and_then(Option::take) else { return true };
        if let Some(l) = self.lockstep.as_mut() {
            if l.turns.len() <= rank {
                l.turns.resize(rank + 1, None);
            }
            l.turns[rank] = cmd.turn;
            if l.sticks_turned.len() <= rank {
                l.sticks_turned.resize(rank + 1, false);
            }
            l.sticks_turned[rank] = cmd.stick_turned;
        }
        if cmd.reset {
            if let Some(slot) = self.viewport_slot_of(pc) {
                self.reset_slot_input(slot);
            }
        }
        // The player's Windows mouse, on their own viewport, and on the failure menu in the whole
        // window that every player's keys go to (coop_end.rs): each machine's mouse moves the
        // cursor of the menu it shows.
        if let (Some((Some((x, y)), _)), Some(menu)) = (cmd.wmouse, self.coop_end_key_viewport()) {
            if self.propr(pc, "Player").as_object() != Some(menu) {
                self.set_prop(menu, "WindowsMouseX", Value::Float(x));
                self.set_prop(menu, "WindowsMouseY", Value::Float(y));
            }
        }
        if let (Some((pos, avail)), Some(viewport)) = (cmd.wmouse, self.propr(pc, "Player").as_object().filter(|&v| self.alive(v))) {
            if let Some((x, y)) = pos {
                self.set_prop(viewport, "WindowsMouseX", Value::Float(x));
                self.set_prop(viewport, "WindowsMouseY", Value::Float(y));
            }
            if let Some(b) = avail {
                if self.propr(viewport, "bWindowsMouseAvailable").as_bool() != b {
                    self.set_prop(viewport, "bWindowsMouseAvailable", Value::Bool(b));
                }
            }
        }
        if let Some((mode, rotation)) = cmd.aim {
            self.set_prop(pc, "iAutoAimMode", Value::Int(mode));
            if self.is_a(pc, "XIIIPlayerController") {
                self.set_prop(pc, "bRotationAssist", Value::Bool(rotation));
            }
        }
        for (key, action, value) in cmd.events {
            if self.objects.get(pc).is_none_or(|o| o.is_none()) {
                break;
            }
            if key == 0x1B && action == crate::player_input::IST_PRESS {
                if let Some(l) = self.lockstep.as_mut() {
                    l.last_menu_player = Some(rank);
                }
            }
            if action == crate::player_input::IST_TYPE {
                self.master_key_type(key);
            } else {
                self.input_process(pc, key, action, value);
            }
        }
        if let Some(line) = cmd.say.as_deref() {
            if self.objects.get(pc).is_some_and(|o| o.is_some()) {
                self.lockstep_chat_run(pc, line);
            }
        }
        if let Some(state) = cmd.pad {
            self.coop_pad_input(pc, state);
        }
        if cmd.look.is_some() && !cmd.stick_turned {
            let own = std::mem::replace(&mut self.look_stick, cmd.look);
            let own_tune = std::mem::replace(&mut self.look_stick_tune, cmd.look_tune);
            self.look_stick_axes(pc);
            self.look_stick = own;
            self.look_stick_tune = own_tune;
        }
        // The move stick, read by this player's DealWithPlayerInputEvent (move_stick_of). A
        // gamepad's left stick (coop_pad_input) already wrote it.
        if cmd.pad.is_none() {
            match self.move_sticks.iter_mut().find(|m| m.0 == pc) {
                Some(m) => m.1 = cmd.mv.unwrap_or((0.0, 0.0)),
                None => self.move_sticks.push((pc, cmd.mv.unwrap_or((0.0, 0.0)))),
            }
        }
        true
    }

    /// The session is lost: back to the menu's map (the engine's own travel, next frame).
    pub fn lockstep_back_to_menu(&mut self) {
        let local = self.config.as_mut().and_then(|c| c.get_string("URL", "LocalMap", "System")).unwrap_or_else(|| "MapMenu".into());
        self.client_travel = Some((local, crate::travel::TRAVEL_ABSOLUTE, false));
    }

    /// Lockstep co-op: the live controller of this machine's player (None outside a session, in
    /// a replay, or before the player's controller exists).
    pub fn lockstep_local_controller(&self) -> Option<ObjId> {
        let rank = self.lockstep.as_ref()?.local_rank?;
        self.coop.players.get(rank).copied().filter(|&pc| self.objects.get(pc).is_some_and(Option::is_some))
    }

    /// A device switch of this machine's player gave aim assist values (aim_device.rs): they go
    /// into its next command instead of the controller.
    pub(crate) fn lockstep_local_aim(&mut self, mode: i32, rotation: bool) -> bool {
        match self.lockstep.as_mut() {
            Some(l) => {
                l.local_aim = Some((mode, rotation));
                true
            }
            None => false,
        }
    }

    /// The window reset this machine's player's input (focus): done in their next command.
    pub(crate) fn lockstep_local_reset(&mut self) -> bool {
        match self.lockstep.as_mut() {
            Some(l) => {
                l.local_reset = true;
                true
            }
            None => false,
        }
    }

    /// Objects only drawing changes in a window (AShadowProjector::UpdateShadow and its shadow
    /// texture are called by the scene's rendering, which each machine does for its own view).
    pub fn lockstep_drawing_only(&self, o: ObjId) -> bool {
        self.is_a(o, "ShadowProjector") || self.is_a(o, "ShadowBitmapMaterial")
    }

    /// Development builds: each actor's part of the fingerprint's actor column (place, physics,
    /// state), named, to find the actor two machines disagree on after a difference.
    /// The VM's window and drawing fields now (see `HostFields`).
    pub fn lockstep_host_fields(&self) -> HostFields {
        HostFields {
            viewport: self.viewport,
            client_size: self.client_size,
            display_window: self.display_window,
            current_viewport: self.current_viewport,
            render_player: self.render_player,
            only_post_render: self.only_post_render,
            precache_render: self.precache_render,
        }
    }

    /// Cheap fingerprint of the game state, by kind ([`FP_KINDS`]): random generators; level
    /// time and number of actors; the players (controllers and pawns: place, rotation, health,
    /// state); every actor (place, rotation, physics, state, in the level's order).
    pub fn lockstep_fingerprint(&self) -> [u32; 4] {
        use crate::NativeField as Field;
        fn mix(h: &mut u64, v: u64) {
            *h ^= v;
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let fold = |h: u64| (h ^ (h >> 32)) as u32;
        let live = |o: ObjId| self.objects.get(o).is_some_and(|x| x.is_some());
        let place = |h: &mut u64, a: ObjId| {
            let l = self.native_prop(a, Field::Location).as_vector();
            for c in l {
                mix(h, u64::from(c.to_bits()));
            }
            let r = self.native_prop(a, Field::Rotation).as_rotator();
            for c in r {
                mix(h, c as u32 as u64);
            }
            mix(h, u64::from(self.native_prop(a, Field::Physics).as_byte()));
            if let Some(s) = self.objects[a].as_ref().and_then(|x| x.state.as_deref()) {
                for b in s.bytes() {
                    mix(h, u64::from(b));
                }
            }
        };
        let basis = 0xcbf2_9ce4_8422_2325u64;
        let mut r = basis;
        mix(&mut r, u64::from(self.seed));
        let mut t = basis;
        mix(&mut t, self.level_time.to_bits());
        let mut p = basis;
        for &pc in &self.coop.players {
            if !live(pc) {
                mix(&mut p, 0xfe);
                continue;
            }
            place(&mut p, pc);
            if let Some(pawn) = self.prop(pc, "Pawn").as_object().filter(|&x| live(x)) {
                place(&mut p, pawn);
                mix(&mut p, self.prop(pawn, "Health").as_int() as u32 as u64);
            }
        }
        let mut a = basis;
        let mut n = 0u64;
        for &x in &self.actors {
            if !live(x) {
                continue;
            }
            n += 1;
            // Drawing-only actors: each machine draws (and so updates) only its own view's.
            if self.lockstep_drawing_only(x) {
                continue;
            }
            place(&mut a, x);
        }
        mix(&mut t, n);
        [fold(r), fold(t), fold(p), fold(a)]
    }
}

// ---------------------------------------------------------------------------------------------
// The network side

/// Role of this machine in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Guest,
}

/// What both machines must share before the first step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Agreement {
    /// Game version and data checksum (code packages, mods list).
    pub checksum: u64,
    /// Start: map URL and options, and the host's settings that reach the game. A guest that
    /// does not know it yet (it joins from the menu) gives it empty and takes the host's.
    pub start: String,
    /// Checksum of the start map's file (0 for a guest that does not know the map yet).
    pub map_sum: u64,
    /// The build (commit, changes, game features, pointer width and byte order:
    /// OXIII_BUILD_ID of the viewer): only identical builds meet. The renderer features and
    /// the operating system / processor family are not part of it, since the lockstep
    /// simulation uses portable math.
    pub build: String,
}

/// Artificial network conditions on this machine's sending side (testing).
#[derive(Clone, Debug, Default)]
pub struct NetSim {
    /// Fixed delay and random extra delay (seconds).
    pub latency: f64,
    pub jitter: f64,
    /// Datagrams held and released together, this many at a time (0 or 1: none).
    pub burst: usize,
    /// Share of datagrams lost (0…1).
    pub loss: f64,
    /// A spike: from `at` seconds after the start, this much more delay for `len` seconds.
    pub spike_at: f64,
    pub spike: f64,
    pub spike_len: f64,
    /// Wi-Fi holes: on average every `hole_every` seconds (0: none), nothing goes through for
    /// `hole_min`…`hole_max` seconds, then what was held arrives together.
    pub hole_every: f64,
    pub hole_min: f64,
    pub hole_max: f64,
}

impl NetSim {
    pub fn parse(v: &str) -> NetSim {
        let n: Vec<f64> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        let g = |i: usize| n.get(i).copied().unwrap_or(0.0);
        NetSim {
            latency: g(0) / 1000.0,
            jitter: g(1) / 1000.0,
            burst: g(2) as usize,
            loss: g(3) / 100.0,
            spike_at: g(4),
            spike: g(5) / 1000.0,
            spike_len: g(6),
            hole_every: g(7),
            hole_min: g(8) / 1000.0,
            hole_max: g(9) / 1000.0,
        }
    }
}

/// Statistics of a session.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Steps that had to wait for the other machine's commands, and how long in all (seconds).
    pub stalls: u64,
    pub stall_time: f64,
    pub longest_stall: f64,
    /// Round trip time: last smoothed value and highest (seconds).
    pub rtt: f64,
    pub rtt_max: f64,
    /// Input delay now and highest (steps).
    pub delay: u64,
    pub delay_max: u64,
    pub sent: u64,
    pub received: u64,
    /// Fingerprints compared with the other machine's.
    pub compared: u64,
    /// Bytes of the datagrams sent and received, commands carried in the datagrams sent (each
    /// unacknowledged command counts once per datagram that carries it), the largest datagram
    /// sent (bytes), and the datagrams sent because one arrived (none since protocol 4: new
    /// commands or the keep-alive only).
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub commands_sent: u64,
    pub largest: u64,
    pub sent_on_receive: u64,
}

/// Steps the players' or the actors' fingerprints must keep differing before the two games are
/// taken as apart for good (3 s): a passing difference does not end a session.
pub const APART_STEPS: u64 = 3 * STEP_HZ as u64;

/// Follows the players' and actors' differences step after step: `since` is the first step of
/// the current run of differences; true once that run lasted [`APART_STEPS`].
pub fn apart_after(since: &mut Option<u64>, step: u64, differ: bool) -> bool {
    if !differ {
        *since = None;
        return false;
    }
    let first = *since.get_or_insert(step);
    step >= first + APART_STEPS
}

/// A difference between the two machines' fingerprints.
#[derive(Clone, Debug)]
pub struct Desync {
    pub step: u64,
    pub kind: &'static str,
    pub local: [u32; 4],
    pub remote: [u32; 4],
}

/// The network side of a lockstep session (one per machine).
pub struct Session {
    sock: UdpSocket,
    peer: SocketAddr,
    pub role: Role,
    /// This machine's player (0: host, 1: guest).
    pub local: usize,
    /// Input delay wanted now (steps); `adaptive`: follows the measured round trip.
    pub delay: u64,
    adaptive: bool,
    /// Next step this machine's commands are scheduled for.
    next_local: u64,
    /// Command held while the delay shrinks (merged into the next one).
    held: Option<PlayerCmd>,
    last_local: PlayerCmd,
    local_cmds: BTreeMap<u64, Vec<u8>>,
    remote_cmds: BTreeMap<u64, Vec<u8>>,
    /// Every remote step below this one is known.
    remote_contig: u64,
    /// The other machine knows every one of our steps below this one.
    peer_ack: u64,
    local_fp: BTreeMap<u64, [u32; 4]>,
    remote_fp: BTreeMap<u64, [u32; 4]>,
    pub desync: Option<Desync>,
    /// Full fingerprints (lockstep_full.rs) by period: this machine's, the other's; the first
    /// period whose two disagree, and the part that differs (detection only).
    local_full: BTreeMap<u64, [u32; 5]>,
    remote_full: BTreeMap<u64, [u32; 5]>,
    pub full_desync: Option<(u64, &'static str)>,
    /// Periods whose full fingerprints were compared.
    pub full_compared: u64,
    /// First step of the current run of player or actor differences (see [`apart_after`]).
    apart_since: Option<u64>,
    /// The players' or actors' fingerprints kept differing for [`APART_STEPS`]: the session ends.
    pub apart: bool,
    /// Step the simulation reached (for pruning).
    simulated: u64,
    /// Delay agreed at the start: steps 0…first_delay − 1 have no command.
    first_delay: u64,
    /// Steps whose fingerprints were compared.
    compared_steps: std::collections::BTreeSet<u64>,
    start: Instant,
    last_send: f64,
    dirty: bool,
    /// Peer's last send time we saw (ms) and when we saw it.
    echo: Option<(u32, f64)>,
    rtt_samples: Vec<f64>,
    pub stats: Stats,
    netsim: Option<(NetSim, Vec<(f64, Vec<u8>)>, u64, Vec<Vec<u8>>)>,
    /// The other machine said goodbye.
    pub peer_left: bool,
    /// The other machine skipped the New game video, or already sent commands of the first
    /// level (it is past its video): this machine's video is skipped too.
    pub peer_skipped: bool,
    /// The other machine is in the background (Android: the player went to the home screen):
    /// its silence is waited for up to [`PAUSED_TIMEOUT`].
    pub peer_paused: bool,
    /// The long wait of the current step already logged (`wait`).
    wait_logged: bool,
    /// Since when (session time) the measured round trip has asked for a shorter delay.
    shrink_since: Option<f64>,
    /// The next datagram is sent because one arrived (measurement only).
    dirty_by_receive: bool,
    /// Session time of the last pump and of the last progress of the other player's commands;
    /// until `quiet_until`, round trips do not change the delay (see `rtt_sample_at`).
    last_pump: f64,
    remote_progress_at: f64,
    quiet_until: f64,
    /// Session time of the last datagram that came from the other machine.
    last_heard: f64,
    /// The other machine's newest simulated step and when it was heard of; the other machine's
    /// own clock advantage and when it came; this machine's smoothed one (`clock_ahead`).
    remote_seen: Option<(u64, f64)>,
    remote_ahead: Option<(f64, f64)>,
    ahead: Option<f64>,
    /// Testing (NetSim holes): start and end of the next hole (session time).
    hole: (f64, f64),
}

impl Session {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub(crate) fn new(sock: UdpSocket, peer: SocketAddr, role: Role, delay: u64) -> Session {
        let _ = sock.set_read_timeout(Some(Duration::from_millis(2)));
        Session {
            sock,
            peer,
            role,
            local: if role == Role::Host { 0 } else { 1 },
            delay,
            adaptive: false,
            next_local: delay,
            held: None,
            last_local: PlayerCmd::default(),
            local_cmds: BTreeMap::new(),
            remote_cmds: BTreeMap::new(),
            remote_contig: delay,
            peer_ack: delay,
            local_fp: BTreeMap::new(),
            remote_fp: BTreeMap::new(),
            desync: None,
            local_full: BTreeMap::new(),
            remote_full: BTreeMap::new(),
            full_desync: None,
            full_compared: 0,
            apart_since: None,
            apart: false,
            simulated: 0,
            first_delay: delay,
            compared_steps: std::collections::BTreeSet::new(),
            start: Instant::now(),
            last_send: f64::NEG_INFINITY,
            dirty: true,
            echo: None,
            rtt_samples: Vec::new(),
            stats: Stats { delay, delay_max: delay, ..Default::default() },
            netsim: None,
            peer_left: false,
            peer_skipped: false,
            peer_paused: false,
            wait_logged: false,
            shrink_since: None,
            dirty_by_receive: false,
            last_pump: 0.0,
            remote_progress_at: 0.0,
            quiet_until: 0.0,
            last_heard: 0.0,
            remote_seen: None,
            remote_ahead: None,
            ahead: None,
            hole: (f64::INFINITY, f64::INFINITY),
        }
    }

    /// This machine's raw clock advantage now, in steps: its step minus the other machine's
    /// step guessed from the newest one it reported (one step on from it, and 60 a second since
    /// it was heard of). The one-way trip, missing from the guess, is the same on both sides and
    /// goes away in the difference of the two machines' advantages (`clock_ahead`).
    fn raw_ahead(&self, now: f64) -> Option<f64> {
        let (r, at) = self.remote_seen?;
        if now - at > AHEAD_FRESH {
            return None;
        }
        let remote = (r + 1) as f64 + (now - at) * f64::from(STEP_HZ);
        Some(self.simulated as f64 - remote)
    }

    /// How many steps this machine's clock runs ahead of the other machine's (smoothed; negative:
    /// behind), from both machines' own estimates, so that the time the datagrams take cancels
    /// out. None while either estimate is old or out of bounds (a map loading, a held window, the
    /// first second). Pacing only: which commands a step simulates never depends on it.
    pub fn clock_ahead(&self) -> Option<f64> {
        let now = self.now();
        let (_, at) = self.remote_ahead?;
        if now - at > AHEAD_FRESH || now < self.quiet_until {
            return None;
        }
        self.raw_ahead(now)?;
        self.ahead.filter(|a| a.abs() <= AHEAD_MAX)
    }

    /// The input delay follows the measured round trip from now on.
    pub fn set_adaptive(&mut self, on: bool) {
        self.adaptive = on;
    }

    /// Artificial network conditions on our sending side (testing).
    pub fn set_netsim(&mut self, sim: NetSim) {
        self.netsim = Some((sim, Vec::new(), 0x9e37_79b9_7f4a_7c15 ^ self.local as u64, Vec::new()));
    }

    /// This machine's command taken at simulation step `step`: scheduled `delay` steps later.
    pub fn push_local(&mut self, step: u64, cmd: PlayerCmd) {
        let cmd = match self.held.take() {
            Some(mut h) => {
                h.merge(cmd);
                h
            }
            None => cmd,
        };
        let target = step + self.delay;
        if self.next_local > target && self.next_local > step + 1 {
            // Delay shrinking: this command waits for the next step's.
            self.held = Some(cmd);
            return;
        }
        while self.next_local < target {
            let c = self.last_local.continuation();
            self.schedule(c);
        }
        self.schedule(cmd);
    }

    fn schedule(&mut self, cmd: PlayerCmd) {
        let mut b = Vec::new();
        cmd.encode(&mut b);
        self.local_cmds.insert(self.next_local, b);
        self.last_local = cmd;
        self.next_local += 1;
        self.dirty = true;
    }

    /// Commands of every player for `step`, if known. The session's first steps (before the
    /// first delay) have no command of anybody.
    pub fn commands(&self, step: u64) -> Option<Vec<PlayerCmd>> {
        let mut out = vec![PlayerCmd::default(); PLAYERS];
        if step < self.first_delay {
            return Some(out);
        }
        let get = |m: &BTreeMap<u64, Vec<u8>>| -> Option<PlayerCmd> { PlayerCmd::decode(&mut Reader::new(m.get(&step)?)) };
        out[self.local] = get(&self.local_cmds)?;
        out[1 - self.local] = get(&self.remote_cmds)?;
        Some(out)
    }

    /// Waits until every player's commands for `step` are known (sending and receiving
    /// meanwhile), up to `timeout`. `on_stall` is called once when the wait gets long (to show a
    /// waiting sign). None: still missing (the other machine is gone or the link is down).
    pub fn wait(&mut self, step: u64, timeout: Duration, mut on_stall: impl FnMut(f64)) -> Option<Vec<PlayerCmd>> {
        let t0 = self.now();
        let mut stalled = false;
        loop {
            self.pump();
            if let Some(c) = self.commands(step) {
                self.wait_logged = false;
                if stalled {
                    let d = self.now() - t0;
                    self.stats.stall_time += d;
                    self.stats.longest_stall = self.stats.longest_stall.max(d);
                }
                return Some(c);
            }
            let waited = self.now() - t0;
            if !stalled && waited > 0.002 {
                stalled = true;
                self.stats.stalls += 1;
            }
            if stalled {
                on_stall(waited);
            }
            // A long wait says what is missing (the other player's command of this step, what
            // this machine sent and what the other one acknowledged), once.
            if waited > 2.0 && !self.wait_logged {
                self.wait_logged = true;
                let have = self.remote_contig.saturating_sub(1);
                eprintln!(
                    "lockstep: waiting since 2 s at step {step} for player {}'s command (received from it up to step {have}); this machine scheduled up to step {}, the other machine acknowledged up to step {}, delay {}, last datagram sent {:.0} ms ago",
                    1 - self.local,
                    self.next_local.saturating_sub(1),
                    self.peer_ack.saturating_sub(1),
                    self.delay,
                    (self.now() - self.last_send) * 1000.0
                );
            }
            if waited > self.timeout_now(timeout).as_secs_f64() || self.peer_left {
                return None;
            }
            // Blocking receive with a short timeout: no busy waiting.
            self.receive_blocking();
        }
    }

    /// The step was simulated: its fingerprint goes to the other machine and is compared.
    pub fn simulated(&mut self, step: u64, fp: [u32; 4]) {
        self.simulated = step + 1;
        self.local_fp.insert(step, fp);
        self.compare(step);
        // (Its fingerprint goes with the next datagram: a step's own command, or the keep-alive.)
        // Old commands and fingerprints no longer needed.
        let keep = step.saturating_sub(600);
        self.local_fp = self.local_fp.split_off(&keep);
        self.remote_fp = self.remote_fp.split_off(&keep);
        self.compared_steps = self.compared_steps.split_off(&keep);
        let keep_cmd = step.min(self.peer_ack).saturating_sub(4);
        self.local_cmds = self.local_cmds.split_off(&keep_cmd);
        self.remote_cmds = self.remote_cmds.split_off(&step.saturating_sub(4));
    }

    /// This machine's full fingerprint of period `period`: sent with the next datagrams and
    /// compared with the other machine's.
    pub fn simulated_full(&mut self, period: u64, fp: [u32; 5]) {
        self.local_full.insert(period, fp);
        self.compare_full(period);
        let keep = period.saturating_sub(4);
        self.local_full = self.local_full.split_off(&keep);
        self.remote_full = self.remote_full.split_off(&keep);
    }

    fn compare_full(&mut self, period: u64) {
        let (Some(l), Some(r)) = (self.local_full.get(&period), self.remote_full.get(&period)) else { return };
        self.full_compared += 1;
        if self.full_desync.is_none() {
            if let Some(part) = crate::lockstep_full::first_difference(l, r) {
                self.full_desync = Some((period, part));
            }
        }
    }

    fn compare(&mut self, step: u64) {
        let (Some(l), Some(r)) = (self.local_fp.get(&step), self.remote_fp.get(&step)) else { return };
        if !self.compared_steps.insert(step) {
            return;
        }
        self.stats.compared += 1;
        if l != r && self.desync.is_none() {
            let kind = (0..4).find(|&i| l[i] != r[i]).map_or("?", |i| FP_KINDS[i]);
            self.desync = Some(Desync { step, kind, local: *l, remote: *r });
        }
        // The players (2) or the actors (3) still differ: the games may have gone apart for good.
        let differ = l[2] != r[2] || l[3] != r[3];
        let (l, r) = (*l, *r);
        if apart_after(&mut self.apart_since, step, differ) && !self.apart {
            self.apart = true;
            eprintln!("lockstep: APART at step {step}: players or actors differ since step {:?}: here {l:08x?}, other machine {r:08x?}", self.apart_since);
        }
    }

    /// Sends what is due and reads what arrived (never blocks).
    pub fn pump(&mut self) {
        // This machine did not pump for a while (a map loading, the window held): the round
        // trips measured just after count that wait, not the link.
        let now = self.now();
        if now - self.last_pump > QUIET_GAP {
            self.quiet_until = self.quiet_until.max(now + QUIET_AFTER);
        }
        self.last_pump = now;
        let _ = self.sock.set_nonblocking(true);
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            if from == self.peer {
                self.read(&buf[..n].to_vec());
            }
        }
        let _ = self.sock.set_nonblocking(false);
        let now = self.now();
        let keep_alive = if self.unacknowledged() > 0 {
            KEEP_ALIVE_UNACKED
        } else {
            KEEP_ALIVE
        };
        if self.dirty || now - self.last_send >= keep_alive {
            self.send_steps();
        }
        self.flush_netsim();
    }

    fn receive_blocking(&mut self) {
        let _ = self.sock.set_read_timeout(Some(Duration::from_millis(2)));
        let mut buf = [0u8; 2048];
        if let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            if from == self.peer {
                self.read(&buf[..n].to_vec());
            }
        }
    }

    fn send_steps(&mut self) {
        let now = self.now();
        let mut out = MAGIC.to_vec();
        out.push(KIND_STEPS);
        out.push(self.local as u8);
        out.extend_from_slice(&((now * 1000.0) as u32).to_le_bytes());
        let (echo, age) = self.echo.map_or((0u32, u16::MAX), |(t, at)| (t, ((now - at) * 1000.0).min(60000.0) as u16));
        out.extend_from_slice(&echo.to_le_bytes());
        out.extend_from_slice(&age.to_le_bytes());
        out.extend_from_slice(&(self.remote_contig as u32).to_le_bytes());
        // Our commands not yet acknowledged, the oldest first: the other machine waits for the
        // first one it lacks, so the newest ones wait for the next datagram when they do not all
        // fit (dropping the oldest could leave a command nobody sends again: both machines then
        // waited for each other).
        let fps: Vec<(u64, [u32; 4])> = self.local_fp.iter().rev().take(FP_REPEAT).map(|(&s, &f)| (s, f)).collect();
        let room = DATAGRAM_MAX.saturating_sub(out.len() + 8 + 1 + fps.len() * 24 + 25);
        let mut steps: Vec<(u64, &Vec<u8>)> = Vec::new();
        let mut size = 0usize;
        for (&s, b) in self.local_cmds.range(self.peer_ack..) {
            if !steps.is_empty() && (steps.len() >= REPEAT_MAX || size + b.len() + 2 > room) {
                break;
            }
            size += b.len() + 2;
            steps.push((s, b));
        }
        let first = steps.first().map_or(self.next_local, |s| s.0);
        out.extend_from_slice(&(first as u32).to_le_bytes());
        out.extend_from_slice(&(steps.len() as u16).to_le_bytes());
        for (_, b) in &steps {
            put_bytes(&mut out, b);
        }
        out.push(fps.len() as u8);
        for (s, f) in fps {
            out.extend_from_slice(&(s as u32).to_le_bytes());
            for v in f {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        // The newest full fingerprint (its period, five parts), repeated until a newer one.
        match self.local_full.iter().next_back() {
            Some((&p, f)) => {
                out.push(1);
                out.extend_from_slice(&(p as u32).to_le_bytes());
                for v in f {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            None => out.push(0),
        }
        // This machine's own clock advantage (hundredths of a step; i16::MIN: not known yet).
        let ahead = self.raw_ahead(now).map_or(i16::MIN, |a| (a * 100.0).clamp(-32000.0, 32000.0) as i16);
        out.extend_from_slice(&ahead.to_le_bytes());
        self.last_send = now;
        self.dirty = false;
        self.stats.sent += 1;
        self.stats.sent_bytes += out.len() as u64;
        self.stats.commands_sent += steps.len() as u64;
        self.stats.largest = self.stats.largest.max(out.len() as u64);
        if std::mem::take(&mut self.dirty_by_receive) {
            self.stats.sent_on_receive += 1;
        }
        self.send_raw(out);
    }

    fn send_raw(&mut self, out: Vec<u8>) {
        let now = self.now();
        let Some((sim, queue, seed, bunch)) = self.netsim.as_mut() else {
            let _ = self.sock.send_to(&out, self.peer);
            return;
        };
        let mut rnd = || {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed >> 11) as f64 / (1u64 << 53) as f64
        };
        if rnd() < sim.loss {
            return;
        }
        let spike = if now >= sim.spike_at && now < sim.spike_at + sim.spike_len { sim.spike } else { 0.0 };
        let mut at = now + sim.latency + sim.jitter * rnd() + spike;
        // Wi-Fi holes: what is sent during one arrives at its end.
        if sim.hole_every > 0.0 {
            if self.hole.0.is_infinite() || now >= self.hole.1 {
                let start = now + sim.hole_every * (0.5 + rnd());
                let len = sim.hole_min + (sim.hole_max - sim.hole_min).max(0.0) * rnd();
                self.hole = (start, start + len);
            }
            if now >= self.hole.0 {
                at = at.max(self.hole.1 + sim.latency);
            }
        }
        if sim.burst > 1 {
            bunch.push(out);
            if bunch.len() < sim.burst {
                return;
            }
            for b in bunch.drain(..) {
                queue.push((at, b));
            }
        } else {
            queue.push((at, out));
        }
    }

    fn flush_netsim(&mut self) {
        let now = self.now();
        let Some((_, queue, _, _)) = self.netsim.as_mut() else { return };
        // Jitter may reorder datagrams, as a real link may.
        let mut i = 0;
        while i < queue.len() {
            if queue[i].0 <= now {
                let (_, b) = queue.remove(i);
                let _ = self.sock.send_to(&b, self.peer);
            } else {
                i += 1;
            }
        }
    }

    pub(crate) fn read(&mut self, b: &[u8]) {
        let mut r = Reader::new(b);
        if r.take(4) != Some(&MAGIC[..]) {
            return;
        }
        // Silence before this datagram (a machine back from the background has one).
        let gap = self.now() - self.last_heard;
        self.last_heard = self.now();
        match r.u8() {
            Some(KIND_STEPS) => {}
            Some(KIND_BYE) => {
                self.peer_left = true;
                return;
            }
            Some(KIND_PAUSE) => {
                self.peer_paused = r.u8() == Some(1);
                return;
            }
            Some(KIND_SKIP) => {
                self.peer_skipped = true;
                return;
            }
            _ => return,
        }
        self.stats.received += 1;
        self.stats.received_bytes += b.len() as u64;
        let now = self.now();
        let (Some(_from), Some(sent), Some(echo), Some(age), Some(ack)) = (r.u8(), r.u32(), r.u32(), r.u16(), r.u32()) else { return };
        self.echo = Some((sent, now));
        if age != u16::MAX {
            let rtt = (now * 1000.0 - f64::from(echo) - f64::from(age)) / 1000.0;
            if (0.0..10.0).contains(&rtt) {
                self.rtt_sample(rtt);
            }
        }
        self.peer_ack = self.peer_ack.max(u64::from(ack));
        let (Some(first), Some(count)) = (r.u32(), r.u16()) else { return };
        if count > 0 {
            self.peer_skipped = true;
            // Commands again after a silence: the other machine is back from the background
            // (those it sent just after its notice, before the system stopped it, say nothing).
            if gap > 1.0 {
                self.peer_paused = false;
            }
        }
        for i in 0..u64::from(count) {
            let Some(c) = r.bytes() else { return };
            let s = u64::from(first) + i;
            if s >= self.simulated.saturating_sub(4) {
                self.remote_cmds.entry(s).or_insert_with(|| c.to_vec());
            }
        }
        let before = self.remote_contig;
        while self.remote_cmds.contains_key(&self.remote_contig) {
            self.remote_contig += 1;
        }
        // The other player's commands come again after a gap (its map loading, its machine
        // held): the same.
        if self.remote_contig > before {
            let now = self.now();
            if now - self.remote_progress_at > QUIET_GAP {
                self.quiet_until = self.quiet_until.max(now + QUIET_AFTER);
            }
            self.remote_progress_at = now;
        }
        if let Some(n) = r.u8() {
            for _ in 0..n {
                let (Some(s), Some(a), Some(b2), Some(c), Some(d)) = (r.u32(), r.u32(), r.u32(), r.u32(), r.u32()) else { break };
                self.remote_fp.insert(u64::from(s), [a, b2, c, d]);
                self.compare(u64::from(s));
                if self.remote_seen.is_none_or(|(seen, _)| u64::from(s) > seen) {
                    self.remote_seen = Some((u64::from(s), now));
                }
            }
        }
        if r.u8() == Some(1) {
            if let (Some(p), Some(a), Some(b2), Some(c), Some(d), Some(e)) = (r.u32(), r.u32(), r.u32(), r.u32(), r.u32(), r.u32()) {
                let p = u64::from(p);
                if !self.remote_full.contains_key(&p) && self.local_full.keys().next().is_none_or(|&f| p >= f) {
                    self.remote_full.insert(p, [a, b2, c, d, e]);
                    self.compare_full(p);
                }
            }
        }
        // The other machine's clock advantage: with ours, how far this machine runs ahead.
        if let Some(v) = r.u16().map(|v| v as i16).filter(|&v| v != i16::MIN) {
            let theirs = f64::from(v) / 100.0;
            self.remote_ahead = Some((theirs, now));
            if let Some(mine) = self.raw_ahead(now) {
                let e = (mine - theirs) / 2.0;
                self.ahead = Some(match self.ahead {
                    Some(a) if (e - a).abs() < AHEAD_MAX => a + (e - a) * AHEAD_SMOOTH,
                    _ => e,
                });
            }
        }
    }

    fn rtt_sample(&mut self, rtt: f64) {
        let now = self.now();
        self.rtt_sample_at(rtt, now);
    }

    /// `rtt_sample` at session time `now` (seconds).
    fn rtt_sample_at(&mut self, rtt: f64, now: f64) {
        self.stats.rtt = if self.stats.rtt == 0.0 { rtt } else { self.stats.rtt * 0.9 + rtt * 0.1 };
        self.stats.rtt_max = self.stats.rtt_max.max(rtt);
        self.rtt_samples.push(rtt);
        if self.rtt_samples.len() > RTT_WINDOW {
            self.rtt_samples.remove(0);
        }
        if now < self.quiet_until {
            self.rtt_samples.pop();
            return;
        }
        if self.adaptive && self.rtt_samples.len() >= 10 {
            let mut v = self.rtt_samples.clone();
            v.sort_by(f64::total_cmp);
            let from_jitter =
                DELAY_FROM_JITTER;
            let want = adaptive_delay_wish(&v, from_jitter);
            // At most ADAPTIVE_MAX: during a real cut the game waits, rather than every command
            // coming late all the time.
            let want = want.clamp(DELAY_MIN, ADAPTIVE_MAX);
            // Grow at once, shrink slowly: one step once the shorter wish has lasted
            // SHRINK_AFTER seconds, then one more step each further SHRINK_AFTER. Each change of
            // the delay is a small jump of this player's movement (a rise repeats the last
            // command, a fall merges two), so a jittery link must not move it all the time.
            if want > self.delay {
                self.delay = want;
                self.shrink_since = None;
            } else if want + 1 < self.delay {
                match self.shrink_since {
                    Some(t) if now - t >= SHRINK_AFTER => {
                        self.delay -= 1;
                        self.shrink_since = Some(now);
                    }
                    Some(_) => {}
                    None => self.shrink_since = Some(now),
                }
            } else {
                self.shrink_since = None;
            }
            self.stats.delay = self.delay;
            self.stats.delay_max = self.stats.delay_max.max(self.delay);
        }
    }

    /// This machine's commands the other machine has not acknowledged yet.
    pub fn unacknowledged(&self) -> usize {
        self.local_cmds.range(self.peer_ack..).count()
    }

    /// Effective input delay now (steps between taking a command and simulating it).
    pub fn effective_delay(&self, step: u64) -> u64 {
        self.next_local.saturating_sub(step + 1)
    }

    /// Seconds since the other machine was last heard from (its steps or its keep-alive): a
    /// machine still showing its video or its loading picture keeps answering.
    pub fn silent_for(&self) -> f64 {
        self.now() - self.last_heard
    }

    /// The latest step the other machine says it simulated (its fingerprints), if any.
    pub fn remote_simulated(&self) -> Option<u64> {
        self.remote_fp.keys().next_back().copied()
    }

    /// Tells the other machine the New game video was skipped here: a skip is both players'.
    /// Sent a few times (a datagram can be lost); the first level's commands that follow say
    /// it again (`peer_skipped`).
    pub fn skip_video(&mut self) {
        let mut out = MAGIC.to_vec();
        out.push(KIND_SKIP);
        for _ in 0..3 {
            let _ = self.sock.send_to(&out, self.peer);
        }
    }

    /// The wait for the other machine before it counts as gone: `usual`, or [`PAUSED_TIMEOUT`]
    /// while it says it is in the background.
    pub fn timeout_now(&self, usual: Duration) -> Duration {
        if self.peer_paused {
            usual.max(PAUSED_TIMEOUT)
        } else {
            usual
        }
    }

    /// Tells the other machine this one goes to the background (`on`) or is back: its game loop
    /// stops meanwhile (Android), and the other machine waits for it longer than the usual timeout.
    pub fn tell_paused(&mut self, on: bool) {
        let mut out = MAGIC.to_vec();
        out.push(KIND_PAUSE);
        out.push(u8::from(on));
        for _ in 0..3 {
            let _ = self.sock.send_to(&out, self.peer);
        }
    }

    /// Tells the other machine we leave.
    pub fn bye(&mut self) {
        let mut out = MAGIC.to_vec();
        out.push(KIND_BYE);
        for _ in 0..3 {
            let _ = self.sock.send_to(&out, self.peer);
        }
    }
}

#[cfg(test)]
mod tests {
    /// A passing difference of the players or actors does not end the session; one that lasts
    /// APART_STEPS does. Breaking `apart_after` (never true, or true at once) fails.
    #[test]
    fn only_a_lasting_difference_ends_the_session() {
        use super::{apart_after, APART_STEPS};
        let mut since = None;
        // Differences on and off for a long time: never apart.
        for step in 0..10 * APART_STEPS {
            assert!(!apart_after(&mut since, step, step % 100 < 90), "step {step}");
        }
        // A run that lasts: apart exactly APART_STEPS after it began.
        let start = 20 * APART_STEPS;
        for step in start..start + APART_STEPS {
            assert!(!apart_after(&mut since, step, true), "step {step}");
        }
        assert!(apart_after(&mut since, start + APART_STEPS, true));
        // The same again after equal steps.
        assert!(!apart_after(&mut since, start + APART_STEPS + 1, false));
        assert_eq!(since, None);
    }

    use super::*;

    #[test]
    fn command_round_trip() {
        let mut p = PadState::default();
        p.axes = [1, -30000, 7, 0, 32767, -32768];
        p.buttons[3] = true;
        p.buttons[15] = true;
        let c = PlayerCmd { events: vec![(0x57, 1, 0.0), (0xe4, 4, -12.5)], pad: Some(p), look: Some((0.25, -1.0)), look_tune: (1.5, true), mv: Some((0.0, 0.5)), aim: Some((2, true)), reset: true, size: Some((1920, 1080)), dev: Some("endlevel Banque01".into()), turn: Some((12.5, -3.0)), wmouse: Some((Some((3.0, 4.5)), Some(true))), stick_turned: true, say: Some("Say hello".into()) };
        let mut b = Vec::new();
        c.encode(&mut b);
        assert_eq!(PlayerCmd::decode(&mut Reader::new(&b)), Some(c));
        let e = PlayerCmd::default();
        let mut b = Vec::new();
        e.encode(&mut b);
        assert_eq!(PlayerCmd::decode(&mut Reader::new(&b)), Some(e));
    }

    /// What a machine knows of the other between steps (a video, a loading picture): the last
    /// step the other one simulated comes with its datagrams, and its keep-alive alone keeps it
    /// heard from. Breaking either (no fingerprint read, `last_heard` never set) fails.
    #[test]
    fn the_other_machine_is_heard_from_between_steps() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let mut h = Session::new(a, pb, Role::Host, 2);
        let mut g = Session::new(b, pa, Role::Guest, 2);
        std::thread::sleep(Duration::from_millis(60));
        assert!(h.silent_for() >= 0.05, "nothing heard yet");
        assert_eq!(h.remote_simulated(), None);
        // The guest in its level: its first steps simulated.
        for step in 0..3 {
            g.push_local(step, PlayerCmd::default());
            g.simulated(step, [step as u32, 0, 0, 0]);
        }
        g.pump();
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert_eq!(h.remote_simulated(), Some(2));
        assert!(h.silent_for() < 0.02, "{}", h.silent_for());
        // The guest only keeps alive (no new step): still heard from.
        std::thread::sleep(Duration::from_secs_f64(KEEP_ALIVE + 0.02));
        g.pump();
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert!(h.silent_for() < 0.02, "keep-alive heard: {}", h.silent_for());
        assert_eq!(h.remote_simulated(), Some(2));
    }

    /// Clock sync: two machines stepping at the same pace, the host 6 steps ahead, each with its
    /// own datagrams' one-way trip unknown: the host reads about +6, the guest about -6 (the two
    /// estimates' difference cancels the trips). Even pace: about 0.
    #[test]
    fn the_machine_ahead_knows_it() {
        fn run(lead: u64) -> (f64, f64) {
            let a = UdpSocket::bind("127.0.0.1:0").unwrap();
            let b = UdpSocket::bind("127.0.0.1:0").unwrap();
            let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
            let mut h = Session::new(a, pb, Role::Host, 2);
            let mut g = Session::new(b, pa, Role::Guest, 2);
            // 30 ms each way, on both sides.
            h.set_netsim(NetSim { latency: 0.030, ..Default::default() });
            g.set_netsim(NetSim { latency: 0.030, ..Default::default() });
            let t0 = Instant::now();
            let mut done = 0u64;
            while t0.elapsed().as_secs_f64() < 1.5 {
                let due = (t0.elapsed().as_secs_f64() * f64::from(STEP_HZ)) as u64;
                while done <= due {
                    h.simulated(done + lead, [0; 4]);
                    g.simulated(done, [0; 4]);
                    h.dirty = true;
                    g.dirty = true;
                    done += 1;
                }
                h.pump();
                g.pump();
                std::thread::sleep(Duration::from_millis(2));
            }
            (h.clock_ahead().unwrap(), g.clock_ahead().unwrap())
        }
        let (h, g) = run(6);
        assert!((h - 6.0).abs() < 1.0 && (g + 6.0).abs() < 1.0, "host {h:.2} guest {g:.2}");
        let (h, g) = run(0);
        assert!(h.abs() < 1.0 && g.abs() < 1.0, "host {h:.2} guest {g:.2}");
    }

    /// A skip of the New game video on one machine is heard by the other one (the video is both
    /// players'), and so is the first level's first command when the skip itself was lost; a
    /// keep-alive alone skips nothing.
    #[test]
    fn a_video_skip_is_both_machines() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let mut h = Session::new(a, pb, Role::Host, 2);
        let mut g = Session::new(b, pa, Role::Guest, 2);
        std::thread::sleep(Duration::from_secs_f64(KEEP_ALIVE + 0.02));
        h.pump();
        std::thread::sleep(Duration::from_millis(20));
        g.pump();
        assert!(!g.peer_skipped, "a keep-alive is not a skip");
        h.skip_video();
        std::thread::sleep(Duration::from_millis(20));
        g.pump();
        assert!(g.peer_skipped, "the host's skip heard by the guest");
        assert!(!h.peer_skipped);
        // The guest's skip lost: its first command says it.
        g.push_local(0, PlayerCmd::default());
        g.pump();
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert!(h.peer_skipped, "the guest's first command heard by the host");
    }

    /// A machine going to the background says so: the other one then waits for it up to
    /// PAUSED_TIMEOUT instead of its usual timeout, and back to the usual one when it says it is
    /// back (or its commands come again).
    #[test]
    fn a_machine_in_the_background_is_waited_for() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let mut h = Session::new(a, pb, Role::Host, 2);
        let mut g = Session::new(b, pa, Role::Guest, 2);
        let usual = Duration::from_secs(20);
        assert_eq!(h.timeout_now(usual), usual);
        g.tell_paused(true);
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert!(h.peer_paused, "the guest's notice heard");
        assert_eq!(h.timeout_now(usual), PAUSED_TIMEOUT);
        g.tell_paused(false);
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert_eq!(h.timeout_now(usual), usual, "back");
        // Paused again: commands sent at once after the notice (the system has not stopped the
        // game yet) leave it paused; commands after a silence say it is back.
        g.tell_paused(true);
        g.push_local(0, PlayerCmd::default());
        g.pump();
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert!(h.peer_paused, "commands right after the notice");
        std::thread::sleep(Duration::from_millis(1200));
        g.push_local(1, PlayerCmd::default());
        g.pump();
        std::thread::sleep(Duration::from_millis(20));
        h.pump();
        assert!(!h.peer_paused, "its commands after a silence say it is back");
    }

    /// Full fingerprints go through the datagrams: equal ones are compared without a
    /// difference; at a later period, another inventory part is reported as such (detection
    /// only, the session goes on). Negative control: the guest's fingerprint never sent (an empty
    /// local list) compares nothing.
    #[test]
    fn full_fingerprints_are_exchanged_and_compared() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let mut h = Session::new(a, pb, Role::Host, 2);
        let mut g = Session::new(b, pa, Role::Guest, 2);
        let exchange = |h: &mut Session, g: &mut Session| {
            for k in 0..20u64 {
                h.push_local(k, PlayerCmd::default());
                g.push_local(k, PlayerCmd::default());
                for _ in 0..3 {
                    h.pump();
                    g.pump();
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        };
        let same = [1, 2, 3, 4, 5];
        h.simulated_full(0, same);
        g.simulated_full(0, same);
        exchange(&mut h, &mut g);
        assert!(h.full_compared >= 1 && g.full_compared >= 1, "{} {}", h.full_compared, g.full_compared);
        assert_eq!((h.full_desync, g.full_desync), (None, None));
        h.simulated_full(1, same);
        g.simulated_full(1, [1, 2, 9, 4, 5]);
        exchange(&mut h, &mut g);
        assert_eq!(h.full_desync, Some((1, "inventory")));
        assert_eq!(g.full_desync, Some((1, "inventory")));
    }

    /// Two sessions over the loopback with a bad link (40 ms each way, 10 ms of jitter, 5 % loss)
    /// at the steps' pace, the guest held 3 s (no pumping) in the middle: both play every step
    /// with the same commands; while the host waits, it sends keep-alives only (no more datagrams
    /// nor bytes a second than the keep-alive rate allows, whatever arrives); nothing is ever
    /// sent because a datagram arrived; the round trip is back to the link's after the hold.
    #[test]
    fn fixed_rate_sender_under_a_stall() {
        let a = UdpSocket::bind("127.0.0.1:0").unwrap();
        let b = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (pa, pb) = (a.local_addr().unwrap(), b.local_addr().unwrap());
        let sim = NetSim { latency: 0.040, jitter: 0.010, burst: 0, loss: 0.05, ..Default::default() };
        const STEPS: u64 = 420;
        const HOLD_AT: u64 = 120;
        const HOLD: f64 = 3.0;
        let side = move |sock: UdpSocket, peer: SocketAddr, role: Role, hold: bool| {
            let mut s = Session::new(sock, peer, role, 6);
            s.set_netsim(sim.clone());
            let me = s.local;
            let t0 = Instant::now();
            let mut seen: Vec<[f32; 2]> = Vec::new();
            // The longest wait for the other machine: its length, datagrams and bytes sent meanwhile.
            let mut longest = (0.0f64, 0u64, 0u64);
            let mut rtt_after = Vec::new();
            for step in 0..STEPS {
                let pace = step as f64 / f64::from(STEP_HZ);
                while t0.elapsed().as_secs_f64() < pace {
                    s.pump();
                    std::thread::sleep(Duration::from_millis(1));
                }
                if hold && step == HOLD_AT {
                    std::thread::sleep(Duration::from_secs_f64(HOLD));
                }
                s.push_local(step, PlayerCmd { mv: Some((step as f32, me as f32)), ..Default::default() });
                let (w0, n0, b0) = (Instant::now(), s.stats.sent, s.stats.sent_bytes);
                let cmds = s.wait(step, Duration::from_secs(10), |_| {}).expect("commands");
                let waited = w0.elapsed().as_secs_f64();
                if waited > longest.0 {
                    longest = (waited, s.stats.sent - n0, s.stats.sent_bytes - b0);
                }
                seen.push([cmds[0].mv.map_or(-1.0, |m| m.0), cmds[1].mv.map_or(-1.0, |m| m.0)]);
                s.simulated(step, [step as u32, 0, 0, 0]);
                if step > STEPS - 30 {
                    rtt_after.push(s.stats.rtt);
                }
            }
            // A last moment for the other machine's last steps.
            let end = Instant::now();
            while end.elapsed() < Duration::from_millis(300) {
                s.pump();
                std::thread::sleep(Duration::from_millis(2));
            }
            (seen, longest, rtt_after, s.stats.clone(), s.desync.is_some(), t0.elapsed().as_secs_f64())
        };
        let side2 = side.clone();
        let host = std::thread::spawn(move || side(a, pb, Role::Host, false));
        let guest = std::thread::spawn(move || side2(b, pa, Role::Guest, true));
        let (h, g) = (host.join().unwrap(), guest.join().unwrap());
        for (who, r) in [("host", &h), ("guest", &g)] {
            let (seen, _, _, st, desync, _) = r;
            assert_eq!(seen.len() as u64, STEPS, "{who}");
            // (A command taken at step s is played at s + 6, the delay.)
            for (step, c) in seen.iter().enumerate().skip(6) {
                let taken = (step - 6) as f32;
                assert_eq!(*c, [taken, taken], "{who} step {step}: both players' commands");
            }
            assert!(!desync, "{who}");
            assert_eq!(st.sent_on_receive, 0, "{who}: a datagram sent because one arrived");
        }
        // The host's wait for the held guest.
        let (waited, n, bytes) = h.1;
        assert!(waited > HOLD * 0.8, "the host waited {waited:.2} s");
        let (per_s, bytes_per_s) = (n as f64 / waited, bytes as f64 / waited);
        // (The keep-alive while commands are unacknowledged: a fixed rate too.)
        let cap = 1.0 / KEEP_ALIVE_UNACKED + 3.0;
        eprintln!("host waited {waited:.2} s: {per_s:.1} datagrams/s, {bytes_per_s:.0} bytes/s; round trip at the end {:.3} s, whole run {:.0} bytes/s sent", h.2.last().unwrap(), h.3.sent_bytes as f64 / h.5);
        assert!(per_s <= cap, "{per_s:.1} datagrams/s while waiting (at most {cap})");
        assert!(bytes_per_s <= cap * DATAGRAM_MAX as f64, "{bytes_per_s:.0} bytes/s while waiting");
        // The whole run: about one datagram a step, never more than one a step plus keep-alives.
        assert!(h.3.sent as f64 / h.5 <= f64::from(STEP_HZ) + 1.0 / KEEP_ALIVE_UNACKED + 5.0, "{} datagrams in {:.1} s", h.3.sent, h.5);
        // The round trip is the link's again (80 to 100 ms here) at the end.
        for (who, r) in [("host", &h), ("guest", &g)] {
            let rtt = *r.2.last().unwrap();
            assert!(rtt < 0.2, "{who}: round trip {rtt:.3} s after the hold");
        }
    }

    /// Round trips measured while a side was held (a map loading on the other machine, this one
    /// not pumping) do not raise the delay: the wait is not the link's.
    #[test]
    fn delay_ignores_round_trips_of_a_wait() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = sock.local_addr().unwrap();
        let mut s = Session::new(sock, peer, Role::Host, 3);
        s.set_adaptive(true);
        for i in 0..40 {
            s.rtt_sample_at(0.020, f64::from(i) * 0.01);
        }
        let clean = s.delay;
        // The other machine loads for 6 s: the round trips just after read seconds.
        s.quiet_until = 10.0 + QUIET_AFTER;
        for i in 0..40 {
            s.rtt_sample_at(3.0, 10.0 + f64::from(i) * 0.02);
        }
        assert_eq!(s.delay, clean, "the wait's round trips raised the delay");
        // A real slow link afterwards raises it at once.
        for i in 0..40 {
            s.rtt_sample_at(0.300, 12.0 + f64::from(i) * 0.01);
        }
        assert!(s.delay > clean);
    }

    /// LevelInfo's date in a session is the session clock at the step, whatever this machine's
    /// clock says: 2004-01-04 03:00:00 (a Sunday) plus 90 s and 500 ms of steps.
    #[test]
    fn session_date_counts_from_the_host_clock() {
        let base = chrono::NaiveDate::from_ymd_opt(2004, 1, 4).unwrap().and_hms_opt(3, 0, 0).unwrap().and_utc().timestamp();
        let step = (90.5 * f64::from(STEP_HZ)) as u64;
        assert_eq!(level_date(Some(step), Some(base)), [2004, 1, 0, 4, 3, 1, 30, 500]);
        assert_eq!(level_date(Some(0), Some(base)), [2004, 1, 0, 4, 3, 0, 0, 0]);
        // A day later on: the date moves on with the steps.
        assert_eq!(level_date(Some(86_400 * u64::from(STEP_HZ)), Some(base))[..4], [2004, 1, 1, 5]);
        // Outside a session: this machine's clock (some date, not the session's).
        assert_ne!(level_date(None, Some(base))[0], 2004);
    }

    /// The measured input delay rises at once and falls one step at a time, only once a shorter
    /// wish has lasted SHRINK_AFTER seconds (each change is a jump of the player's movement).
    #[test]
    fn delay_rises_at_once_and_falls_slowly() {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = sock.local_addr().unwrap();
        let mut s = Session::new(sock, peer, Role::Host, 3);
        s.set_adaptive(true);
        // A 300 ms round trip: the delay goes up at once.
        for i in 0..20 {
            s.rtt_sample_at(0.300, f64::from(i) * 0.01);
        }
        let high = s.delay;
        assert!(high >= 10, "{high}");
        // The link gets clean (20 ms): nothing in the first seconds, however many samples (the
        // slow round trips are still among the last ones: a link that varies).
        let mut t = 0.2;
        for _ in 0..200 {
            s.rtt_sample_at(0.020, t);
            t += 0.005;
        }
        assert!(s.delay >= high, "the delay fell before the shorter wish had lasted");
        let high = s.delay;
        // Then, once the slow ones left the window, one step per SHRINK_AFTER seconds.
        let until = t + RTT_WINDOW as f64 / 60.0 + 3.0 * SHRINK_AFTER + 0.1;
        while t < until {
            s.rtt_sample_at(0.020, t);
            t += 1.0 / 60.0;
        }
        assert!(s.delay < high && s.delay + 5 >= high, "{} from {high}", s.delay);
        // A spike raises it again at once.
        for _ in 0..120 {
            s.rtt_sample_at(0.300, t);
            t += 0.005;
        }
        assert!(s.delay >= high, "{} from {high}", s.delay);
    }

    /// The jitter margin of the adaptive delay (`DELAY_FROM_JITTER`), both ways, on a calm Wi-Fi
    /// link (shortest round trip 20 ms, one in ten at 50 ms): from the jitter the delay is 4
    /// steps, by the former rule 2; a clean wired link (all at 20 ms) asks for the same delay
    /// either way. A session follows the constant. Negative control: swapping the two rules in
    /// `adaptive_delay_wish` fails.
    #[test]
    fn delay_margin_from_jitter_can_be_turned_off() {
        let link = |i: usize| if i % 10 == 9 { 0.050 } else { 0.020 };
        let mut wifi: Vec<f64> = (0..100).map(link).collect();
        wifi.sort_by(f64::total_cmp);
        assert_eq!(adaptive_delay_wish(&wifi, true), 4);
        assert_eq!(adaptive_delay_wish(&wifi, false), 2);
        let wired = vec![0.020; 100];
        assert_eq!(adaptive_delay_wish(&wired, true), adaptive_delay_wish(&wired, false));
        // A session uses the constant's position.
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = sock.local_addr().unwrap();
        let mut s = Session::new(sock, peer, Role::Host, DELAY_MIN);
        s.set_adaptive(true);
        for i in 0..100 {
            s.rtt_sample_at(link(i), i as f64 * 0.01);
        }
        assert_eq!(s.delay, adaptive_delay_wish(&wifi, DELAY_FROM_JITTER).clamp(DELAY_MIN, ADAPTIVE_MAX));
    }

}
