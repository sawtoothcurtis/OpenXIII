//! Network co-op in lockstep, the display layer's side (the VM's is openxiii_vm::lockstep): each
//! step, this machine's player's devices are taken out of the VM as a command, the commands of
//! every player for the step are awaited, the step runs, then its fingerprint is sent.

use std::time::{Duration, Instant};

use crate::renderer::{self, Camera, Renderer};
use crate::{degrees, draw_viewport_frame, game};

use openxiii_vm::lockstep::{Agreement, Desync, PlayerCmd, Profile, Session};
use openxiii_vm::lockstep_meet::{pack, unpack, MeetPoll, Meeting, Met};
use openxiii_vm::Vm;

thread_local! {
    /// Lockstep co-op: screen effect passes of the view drawn next (made by the step).
    pub(super) static LOCKSTEP_FX: std::cell::RefCell<Option<Vec<openxiii_vm::screen_fx::FxPass>>> = const { std::cell::RefCell::new(None) };
    /// Lockstep co-op: the shadow work of the other players' views, drawn with this frame's.
    pub(super) static LOCKSTEP_SHADOW_JOBS: std::cell::RefCell<Vec<renderer::ShadowRender>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Lockstep co-op: the step's own view output while the frame draws a scene seen from the
    /// turned camera (Vm::preview_scene). The shadows are game state: they are updated from what
    /// the step drew, the same on every machine.
    pub(super) static LOCKSTEP_STEP_SCENE: std::cell::RefCell<Option<openxiii_vm::client::ViewOutput>> = const { std::cell::RefCell::new(None) };
}

/// Where the players' commands come from.
pub enum Source {
    /// The other machine, over the network.
    Net(Box<Session>),
    /// Both players on this machine (player 1: keyboard and mouse, player 2: the first gamepad).
    Local,
}

pub struct Runner {
    pub(crate) session: Option<openxiii_vm::lockstep::SessionContext>,
    pub source: Source,
    /// Every player's screen size, in player order.
    pub sizes: Vec<(i32, i32)>,
    /// Step being simulated, from the session's start.
    pub step: u64,
    /// A step began and its end is still to be done.
    open: bool,
    /// The first difference with the other machine was reported.
    pub reported: bool,
    /// A full fingerprint difference was reported (once).
    pub full_reported: bool,
    /// How long to wait for the other machine before giving up.
    pub timeout: Duration,
    /// The step is waiting for the other machine (seconds), for the waiting sign.
    pub waiting: f64,
    /// Step whose own commands were already taken and sent (window, `poll`).
    prepared: Option<u64>,
    wait_from: std::time::Instant,
    /// Step whose state fingerprint was computed (window).
    pub hashed: Option<u64>,
    /// The other machine stopped sending (the session ended early).
    pub lost: bool,
    /// Each player's own settings (player order), applied by the VM to that player.
    pub profiles: Vec<Option<Profile>>,
    /// Instant look: this machine's mouse moves sent but not simulated yet.
    pub look: LookAhead,
}

/// Instant look (drawing only): the view turns of this machine's player that the session has
/// not simulated yet (each command carries the turn its frames integrated, `PlayerCmd::turn`,
/// and waits for the input delay) are added to the simulated rotation, so that the drawn view is
/// where the steps will put it. When a step did not apply its turn (a cinematic, a menu, a
/// script holding the view), nothing is added until a step applies one again.
#[derive(Default)]
pub struct LookAhead {
    /// (step that will simulate it, yaw, pitch).
    pending: std::collections::VecDeque<(u64, f32, f32)>,
    last: Option<[i32; 3]>,
    /// The last step with a turn did not apply it.
    held: bool,
}

impl LookAhead {
    fn sent(&mut self, step: u64, cmd: &PlayerCmd) {
        if let Some((y, p)) = cmd.turn {
            if y != 0.0 || p != 0.0 {
                self.pending.push_back((step, y, p));
            }
        }
    }

    /// Step `step` was simulated, the local controller's rotation now `rotation`: its turn
    /// leaves the queue; a turn of some size the rotation did not follow holds the look.
    fn simulated(&mut self, step: u64, rotation: Option<[i32; 3]>) {
        let (mut y, mut p) = (0.0f32, 0.0f32);
        while self.pending.front().is_some_and(|e| e.0 <= step) {
            if let Some((s, dy, dp)) = self.pending.pop_front() {
                if s == step {
                    y += dy;
                    p += dp;
                }
            }
        }
        if let (Some(before), Some(now)) = (self.last, rotation) {
            let d = |a: i32, b: i32| ((b - a) as i16) as f32;
            let (ry, rp) = (d(before[1], now[1]), d(before[0], now[0]));
            let size = y.abs().max(p.abs());
            if size >= 30.0 {
                let followed = ry.abs().max(rp.abs());
                self.held = followed < size * 0.2;
            }
        }
        self.last = rotation;
    }

    /// (yaw, pitch) to add to the drawn view: the turns still waiting and `fresh`, this frame's
    /// turn not sent yet.
    pub fn offset(&self, fresh: (f32, f32)) -> (i32, i32) {
        if self.held {
            return (0, 0);
        }
        let (y, p) = self.pending.iter().fold(fresh, |a, e| (a.0 + e.1, a.1 + e.2));
        (y.round() as i32, p.round() as i32)
    }
}

impl Runner {
    pub fn new(source: Source, sizes: Vec<(i32, i32)>) -> Runner {
        Runner { session: None, source, sizes, step: 0, open: false, reported: false, full_reported: false, timeout: Duration::from_secs(20), waiting: 0.0, prepared: None, wait_from: std::time::Instant::now(), hashed: None, lost: false, profiles: Vec::new(), look: LookAhead::default() }
    }

    /// This machine's player (0: player 1).
    pub fn local_player(&self) -> Option<usize> {
        match &self.source {
            Source::Net(s) => Some(s.local),
            _ => None,
        }
    }

    /// Before the step's tick: this machine's commands taken and sent, every player's awaited,
    /// the step begun in the VM. False: the step cannot run (the other machine is gone).
    pub fn begin(&mut self, vm: &mut Vm) -> bool {
        if !vm.lockstep_on() {
            if let Some(session) = &self.session { vm.session = session.clone(); }
            vm.lockstep_start(self.sizes.clone());
            vm.lockstep_set_profiles(self.profiles.clone());
        }
        if let Some(l) = vm.lockstep.as_mut() {
            l.local_rank = self.local_player();
        }
        let step = self.step;
        let cmds = match &mut self.source {
            Source::Local => {
                let p1 = vm.lockstep_harvest(true, false);
                let p2 = vm.lockstep_harvest(false, true);
                vec![p1, p2]
            }
            Source::Net(s) => {
                #[allow(unused_mut)]
                let mut own = vm.lockstep_harvest(true, true);
                s.push_local(step, own);
                let mut waited = 0.0;
                let got = s.wait(step, self.timeout, |w| waited = w);
                self.waiting = waited;
                if waited > 0.0 {
                }
                match got {
                    Some(c) => c,
                    None => {
                        self.lost = true;
                        return false;
                    }
                }
            }
        };
        vm.lockstep_begin_step(step, cmds);
        self.open = true;
        true
    }

    /// The window's way of [`Runner::begin`], never blocking: this machine's commands for the
    /// step are taken and sent once, then each call looks whether every player's have come.
    /// Some(true): the step began; Some(false): still waiting (draw and come back); None: the
    /// other machine is gone.
    pub fn poll(&mut self, vm: &mut Vm) -> Option<bool> {
        let Source::Net(_) = &self.source else { return Some(self.begin(vm)) };
        if !vm.lockstep_on() {
            if let Some(session) = &self.session { vm.session = session.clone(); }
            vm.lockstep_start(self.sizes.clone());
            vm.lockstep_set_profiles(self.profiles.clone());
        }
        if let Some(l) = vm.lockstep.as_mut() {
            l.local_rank = self.local_player();
        }
        let step = self.step;
        let Source::Net(s) = &mut self.source else { return None };
        if self.prepared != Some(step) {
            #[allow(unused_mut)]
            let mut own = vm.lockstep_harvest(true, true);
            let sent = own.clone();
            s.push_local(step, own);
            // The step that will simulate it (the input delay later).
            self.look.sent(step + s.effective_delay(step), &sent);
            self.prepared = Some(step);
            self.wait_from = std::time::Instant::now();
        }
        s.pump();
        let Some(cmds) = s.commands(step) else {
            self.waiting = self.wait_from.elapsed().as_secs_f64();
            // Gone: it said goodbye, or nothing came from it for the whole timeout (a machine
            // still on its video or its loading picture keeps sending its keep-alive).
            // While the other machine says it is in the background (Android's home screen), it
            // is waited for longer.
            let timeout = s.timeout_now(self.timeout).as_secs_f64();
            if s.peer_left || self.waiting > timeout && s.silent_for() > timeout {
                return None;
            }
            return Some(false);
        };
        // A stall: the step waited longer than a step for the other machine (shorter waits are the
        // window's frame rhythm, not a hold-up anyone sees).
        if self.waiting > 0.0 {
            let waited = self.wait_from.elapsed().as_secs_f64();
            if waited > 1.0 / f64::from(openxiii_vm::lockstep::STEP_HZ) {
                s.stats.stalls += 1;
                s.stats.stall_time += waited;
                s.stats.longest_stall = s.stats.longest_stall.max(waited);
            }
        }
        self.waiting = 0.0;
        vm.lockstep_begin_step(step, cmds);
        self.open = true;
        Some(true)
    }

    /// The other machine simulated step `step` or a later one (it is in the level loaded for
    /// that step), or left; the session's datagrams are sent and read meanwhile. Always true
    /// for a session that is not over the network.
    pub fn other_reached(&mut self, step: u64) -> bool {
        let Source::Net(s) = &mut self.source else { return true };
        s.pump();
        s.peer_left || s.remote_simulated().is_some_and(|r| r >= step)
    }

    /// How many steps this machine's clock runs ahead of the other machine's (network sessions,
    /// when known; see `Session::clock_ahead`).
    pub fn clock_ahead(&self) -> Option<f64> {
        match &self.source {
            Source::Net(s) => s.clock_ahead(),
            _ => None,
        }
    }

    /// The session's datagrams sent and read, no step taken (a video before the first level):
    /// the other machine keeps hearing from this one.
    pub fn keep_alive(&mut self) {
        if let Source::Net(s) = &mut self.source {
            s.pump();
        }
    }

    /// The other machine skipped the New game video (or is already past it), or left: this
    /// machine's video stops too.
    pub fn other_skipped(&self) -> bool {
        match &self.source {
            Source::Net(s) => s.peer_skipped || s.peer_left,
            _ => false,
        }
    }

    /// This machine goes to the background (Android: the home screen) or is back: the other
    /// machine is told, so that it waits for this one instead of ending the session.
    pub fn tell_paused(&mut self, on: bool) {
        if let Source::Net(s) = &mut self.source {
            s.tell_paused(on);
        }
    }

    /// This machine's player skipped the New game video: the other machine is told.
    pub fn tell_skip(&mut self) {
        if let Source::Net(s) = &mut self.source {
            s.skip_video();
        }
    }

    /// After the step (tick and every player's view): the VM's real devices back, the
    /// fingerprint sent and compared. A new difference with the other machine is returned once.
    pub fn end(&mut self, vm: &mut Vm) -> Option<Desync> {
        if !std::mem::take(&mut self.open) {
            return None;
        }
        vm.lockstep_end_step();
        let fp = vm.lockstep_fingerprint();
        let step = self.step;
        // This step's slice of the periodic full fingerprint (lockstep_full.rs).
        let full = vm.lockstep_full_slice(step);
        if let Some(local) = self.local_player() {
            let rotation = vm.coop.players.get(local).copied().map(|pc| vm.propr(pc, "Rotation").as_rotator());
            self.look.simulated(step, rotation);
        }
        self.step += 1;
        if let Source::Net(s) = &mut self.source {
            s.simulated(step, fp);
            if let Some((period, f)) = full {
                s.simulated_full(period, f);
            }
            // A difference the full fingerprint saw (detection only: the game goes on).
            if let Some((period, part)) = s.full_desync.filter(|_| !self.full_reported) {
                self.full_reported = true;
                let line = format!("FULL DIFFERENCE period={period} (steps {} to {}) part={part}", period * openxiii_vm::lockstep_full::PERIOD, (period + 1) * openxiii_vm::lockstep_full::PERIOD - 1);
                eprintln!("lockstep: {line}");
            }
            s.pump();
            if let Some(d) = s.desync.clone().filter(|_| !self.reported) {
                self.reported = true;
                return Some(d);
            }
        }
        None
    }

    /// End of the session: the other machine is told, the last fingerprints exchanged.
    pub fn finish(&mut self) {
        if let Source::Net(s) = &mut self.source {
            // A moment for the last datagrams (fingerprints) to cross.
            let until = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < until && !s.peer_left {
                s.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            s.bye();
        }
    }
}

/// This machine's player's own settings as blob entries: its User.ini and its gamepad settings
/// (its first local player's).
pub fn profile_entries(vm: &mut Vm, pads: &openxiii_vm::gamepad_config::GamepadConfig) -> Vec<(String, Vec<u8>)> {
    let user = vm.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
    let system = vm.config.as_mut().map(|c| c.file_text("System")).unwrap_or_default();
    let mut own = openxiii_vm::gamepad_config::GamepadConfig::default();
    own.players[0] = pads.players[0];
    let graphics = local_graphics_profile_value();
    let name = vm.coop_chat_name();
    vec![
        ("profile/User.ini".into(), user.into_bytes()),
        ("profile/Gamepad.ini".into(), own.to_text().into_bytes()),
        ("profile/Graphics".into(), graphics.as_bytes().to_vec()),
        ("profile/System.ini".into(), system.into_bytes()),
        // Its name in the co-op chat (the Co-op page's, else the multiplayer one).
        ("profile/CoopName".into(), name.into_bytes()),
    ]
}

/// A player's settings from blob entries (see [`profile_entries`]); None without a User.ini.
pub fn profile_of(entries: &[(String, Vec<u8>)]) -> Option<Profile> {
    let get = |n: &str| entries.iter().find(|e| e.0 == n).map(|e| String::from_utf8_lossy(&e.1).into_owned());
    let mut p = Profile::from_texts(&get("profile/User.ini")?, &get("profile/Gamepad.ini").unwrap_or_default());
    p.enhanced = get("profile/Graphics").map(|g| g.trim() == "1");
    p.name = get("profile/CoopName").map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    Some(p)
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h ^= u64::from(x);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Checksum of a map's file (both machines must have the same).
pub fn map_checksum(game: &std::path::Path, map: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let name = map.split(['?', '#']).next().unwrap_or(map).to_ascii_lowercase();
    let name = name.strip_suffix(".unr").unwrap_or(&name).to_string();
    if let Ok(dir) = std::fs::read_dir(game.join("Maps")) {
        for e in dir.flatten() {
            let f = e.file_name().to_string_lossy().to_ascii_lowercase();
            if f == format!("{name}.unr") {
                if let Ok(b) = std::fs::read(e.path()) {
                    fnv(&mut h, &b);
                }
            }
        }
    }
    h
}

/// Game version and data checksum both machines must share: this build's version, the sizes of
/// the game's code packages and the mods in use.
/// This build's identity (build.rs): commit and changes, game features, tools, pointer width
/// and byte order, profile. Not the renderer features nor the operating system / processor
/// family: the lockstep simulation uses portable math.
pub const BUILD_ID: &str = env!("OXIII_BUILD_ID");

pub fn data_checksum(game: &std::path::Path, mods: &[String]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: &[u8]| fnv(&mut h, b);
    eat(env!("CARGO_PKG_VERSION").as_bytes());
    eat(&openxiii_vm::lockstep::PROTOCOL.to_le_bytes());
    if let Ok(dir) = std::fs::read_dir(game.join("System")) {
        let mut v: Vec<(String, u64)> = dir
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.ends_with(".u").then(|| (n, e.metadata().map_or(0, |m| m.len())))
            })
            .collect();
        v.sort();
        for (n, l) in v {
            eat(n.as_bytes());
            eat(&l.to_le_bytes());
        }
    }
    // The sound banks and voice packages (Sounds\PC\*.hxc/*.hsc, Sounds\*.uax): a voice of
    // another length or language would time the scenes differently.
    for sub in ["Sounds", "Sounds/PC"] {
        if let Ok(dir) = std::fs::read_dir(game.join(sub)) {
            let mut v: Vec<(String, u64)> = dir
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                    (n.ends_with(".uax") || n.ends_with(".hxc") || n.ends_with(".hsc")).then(|| (n, e.metadata().map_or(0, |m| m.len())))
                })
                .collect();
            v.sort();
            for (n, l) in v {
                eat(n.as_bytes());
                eat(&l.to_le_bytes());
            }
        }
    }
    for m in mods {
        eat(m.as_bytes());
    }
    h
}

/// It is always written to stderr and to the session's log.
pub(crate) fn differ_shown() -> bool {
    {
        false
    }
}

/// A line about the session's network (end of a capture run, testing).
pub fn report(r: &Runner) -> String {
    match &r.source {
        Source::Net(s) => {
            let st = &s.stats;
            format!(
                "lockstep: {} steps, stalls {} ({:.2} s in all, longest {:.3} s), rtt {:.1} ms (max {:.1}), delay {} (max {}), sent {} received {}, fingerprints compared {}, desync {}, full fingerprints compared {} (difference {})",
                r.step,
                st.stalls,
                st.stall_time,
                st.longest_stall,
                st.rtt * 1000.0,
                st.rtt_max * 1000.0,
                st.delay,
                st.delay_max,
                st.sent,
                st.received,
                st.compared,
                s.desync.as_ref().map_or("none".to_string(), |d| format!("step {} ({})", d.step, d.kind)),
                s.full_compared,
                s.full_desync.map_or("none".to_string(), |(p, part)| format!("period {p}, {part}"))
            )
        }
        _ => format!("lockstep: {} steps (no network)", r.step),
    }
}

/// Port of a lockstep session when the address gives none (the game's own).
pub const PORT: u16 = 7777;
/// How long a host waits for its guest, and a guest for its host (seconds).
const MEET_WAIT: u64 = 600;
/// Real time a late machine may owe the session, in steps (beyond it, the session slows down).
const CATCH_UP: u32 = 8;
/// Clock sync: steps of advance over the other machine left alone (the estimate's own noise),
/// how much slower the session time passes per step of advance beyond it, and at most.
const CLOCK_DEAD: f64 = 0.5;
const CLOCK_SLOW_PER_STEP: f64 = 0.04;
const CLOCK_SLOW_MAX: f64 = 0.1;

fn clock_sync_on() -> bool {
    {
        true
    }
}
/// Steps the window may run in one frame to catch up: a slow machine that ran more drew longer
/// and longer frames, each owing more steps to the next (a spiral of 5 to 8 steps a frame).
const STEPS_PER_FRAME: u32 = 3;
/// Frames in a row a late machine may leave undrawn, its time going to the steps.
const UNDRAWN_IN_A_ROW: u32 = 1;

fn steps_per_frame() -> u32 {
    STEPS_PER_FRAME
}

fn undrawn_in_a_row() -> u32 {
    UNDRAWN_IN_A_ROW
}

/// The mods this machine plays with (both machines must have the same).
fn mods_in_use() -> Vec<String> {
    #[cfg(feature = "mods")]
    {
        // Each mod's folder name and a checksum of its files (relative names and sizes).
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(String, u64)>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    let rel = p.strip_prefix(base).map_or(String::new(), |r| r.to_string_lossy().to_ascii_lowercase().replace('\\', "/"));
                    out.push((rel, e.metadata().map_or(0, |m| m.len())));
                }
            }
        }
        let mut v: Vec<String> = openxiii_mods::mods_folder()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| (e.file_name().to_string_lossy().to_ascii_lowercase(), e.path()))
                    .filter(|(n, _)| !n.starts_with('_'))
                    .map(|(n, p)| {
                        let mut files = Vec::new();
                        walk(&p, &p, &mut files);
                        files.sort();
                        let mut h = 0xcbf2_9ce4_8422_2325u64;
                        for (f, l) in files {
                            fnv(&mut h, f.as_bytes());
                            fnv(&mut h, &l.to_le_bytes());
                        }
                        format!("{n}:{h:016x}")
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
    #[cfg(not(feature = "mods"))]
    Vec::new()
}

/// A lockstep meeting under way in the window (it keeps drawing while it waits).
pub struct MeetState {
    pub meeting: Meeting,
    /// The travel that started it (the host's start map; the guest's join of the host).
    pub req: openxiii_vm::travel::TravelRequest,
    pub guest: bool,
    pub size: (i32, i32),
    pub extra: u32,
    pub start_mode: Option<openxiii_vm::local_play::CoopStart>,
    pub mine: Vec<(String, Vec<u8>)>,
}

/// The game's texts in language `ext` ("int", "frt"…) are installed (Core.<ext> in System).
pub fn language_installed(game: &std::path::Path, ext: &str) -> bool {
    std::fs::read_dir(game.join("System")).is_ok_and(|d| d.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(&format!("Core.{ext}"))))
}

/// "Chars=1.0,3.2" of a start's settings: both players' (roster index, variant).
fn characters_of(settings: &str) -> Option<[(u8, u8); 2]> {
    let v = settings.split(';').find_map(|kv| kv.strip_prefix("Chars="))?;
    let mut it = v.split(',').map(|c| {
        let (r, k) = c.split_once('.')?;
        Some((r.parse::<u8>().ok()?, k.parse::<u8>().ok()?))
    });
    Some([it.next()??, it.next()??])
}

/// The rank of the view this machine presents: its own player's, or during a cinematic the
/// player it holds (Vm::coop_cine_shown_rank), as split screen shows that one alone. Drawing
/// only: every machine still makes every view in player order.
pub(super) fn shown_rank(g: &game::Game, r: &Runner) -> usize {
    let local = r.local_player().unwrap_or(0);
    g.vm.coop_cine_shown_rank().filter(|&k| k < g.vm.engine.client.viewports.len()).unwrap_or(local)
}

/// Lockstep co-op: another player's view (a cinematic, shown_rank) has their screen's size;
/// shown in this window, it is scaled to fit, centred, keeping its shape (scene and canvas),
/// instead of being drawn at that size from the window's corner (cut off on a smaller screen,
/// a black strip on a larger one). Drawing only.
pub(super) fn fit_view_to_window(f: &mut openxiii_vm::client::ViewportFrame, client: (u32, u32)) {
    let (w, h) = (f.rect[2].max(1) as f32, f.rect[3].max(1) as f32);
    let (cw, ch) = (client.0.max(1) as f32, client.1.max(1) as f32);
    if (w, h) == (cw, ch) {
        return;
    }
    let s = (cw / w).min(ch / h);
    let (fw, fh) = ((w * s).round(), (h * s).round());
    let (ox, oy) = (((cw - fw) / 2.0).round(), ((ch - fh) / 2.0).round());
    // The canvas's primitives stay relative to the view's origin (draw_viewport_frame adds it).
    for d in &mut f.hud {
        openxiii_vm::gui::place_scaled(d, s, [0.0, 0.0]);
    }
    if let Some(r) = f.scene.as_mut() {
        let at = |v: i32, origin: i32| (v - origin) as f32 * s;
        *r = [(ox + at(r[0], f.rect[0])) as i32, (oy + at(r[1], f.rect[1])) as i32, (r[2] as f32 * s) as i32, (r[3] as f32 * s) as i32];
    }
    f.rect = [ox as i32, oy as i32, fw as i32, fh as i32];
}

/// Advance every split-screen viewport during fixed-step capture in player order.
pub(super) fn draw_capture_views(
    g: &mut game::Game,
    size: (u32, u32),
) {
    for (_i, slot) in g.active_viewports().into_iter().enumerate() {
        let drawn = g.draw_viewport(slot, size);
        if let Some(f) = drawn {
            let _ = g.vm.screen_fx_frame(f.player, f.view.map(|v| v.1));
        }
    }
}

/// Lockstep co-op: the shadow update split screen does at the present of another player's
/// view (`update_shadows` for viewport `slot`), with that view's own output and view put in the
/// VM meanwhile; its shadow work, to draw with this machine's frame.
fn other_view_shadows(
    r: &Renderer,
    g: &mut game::Game,
    c: &game::Characters,
    output: &mut openxiii_vm::client::ViewOutput,
    view: Option<(Option<openxiii_vm::value::ObjId>, [f32; 3], [i32; 3])>,
    slot: u8,
    frame: u32,
) -> Vec<renderer::ShadowRender> {
    let Some((_, eye, _)) = view else { return Vec::new() };
    let vm_output = g.vm.swap_view_output(std::mem::take(output));
    let kept_view = g.swap_view(view);
    let jobs = g.update_shadows(c, eye, &|k| r.cartoon_dir(k), frame, Some(slot));
    *output = g.vm.swap_view_output(vm_output);
    g.swap_view(kept_view);
    jobs
}

/// Frames of the level drawn off screen behind the loading picture of a session's travel, once a
/// step made this machine's view (`lockstep_hold_frame`), before the picture goes.
const HOLD_WARM_FRAMES: u32 = 4;

/// Steps both machines simulate behind the loading picture of a session's travel before it goes
/// (`lockstep_hold_frame`): a second of the level.
const HOLD_SETTLE_STEPS: u64 = 60;

impl crate::App {
    /// Lockstep co-op: while the window is minimized or hidden, Windows sends it no redraw, yet
    /// the other machine needs this one's steps: frames without drawing run at the steps' pace
    /// (sleeping between them, no busy loop), the game going on as with the window shown.
    pub(super) fn run_background_lockstep(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::application::ApplicationHandler;
        use winit::event::WindowEvent;
        use winit::event_loop::ControlFlow;
        let hidden = self.occluded || self.window.as_ref().and_then(|w| w.is_minimized()).unwrap_or(false);
        if self.lockstep.is_none() || self.game.is_none() || !hidden {
            if self.background_last.take().is_some() {
                event_loop.set_control_flow(ControlFlow::Wait);
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            return;
        }
        const PERIOD: std::time::Duration = std::time::Duration::from_micros(1_000_000 / openxiii_vm::lockstep::STEP_HZ as u64);
        let now = Instant::now();
        if self.background_last.is_none_or(|t| now - t >= PERIOD) {
            self.background_last = Some(now);
            if let Some(id) = self.window.as_ref().map(|w| w.id()) {
                self.background_frame = true;
                self.window_event(event_loop, id, WindowEvent::RedrawRequested);
                self.background_frame = false;
            }
        }
        let next = self.background_last.unwrap_or(now) + PERIOD;
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }

    /// Draw this machine's most recent lockstep view into the current frame.
    pub(super) fn draw_lockstep_view(
        &mut self,
        lockstep_view: Option<(openxiii_vm::client::ViewportFrame, Vec<openxiii_vm::screen_fx::FxPass>)>,
        client: (u32, u32),
        target: &wgpu::TextureView,
    ) {
        let sign = if lockstep_view.is_some() { self.lockstep_sign(client.0 as f32, client.1 as f32) } else { Vec::new() };
        let steps_only = false;
        // The presented view's player: this machine's, or a cinematic's (shown_rank, the same
        // rank the last step kept the view of).
        let local_rank = self.lockstep.as_ref().and_then(|l| l.local_player()).unwrap_or(0);
        let shown = match (&self.game, &self.lockstep) {
            (Some(g), Some(l)) => shown_rank(g, l),
            _ => local_rank,
        };
        let fresh = self.game.as_ref().and_then(|g| g.vm.lockstep.as_ref()).map_or((0.0, 0.0), |l| l.local_look.acc);
        // Another player's view (a cinematic): this machine's look does not turn it; nor while
        // the game holds this player's view whatever the look (a cinematic's NoControl, a
        // camera's view, the end of the game: Vm::lockstep_look_follows), where the drawn view
        // would turn with the mouse and jump back once the steps show it did not.
        let follows = match (&self.game, lockstep_view.as_ref().and_then(|(f, _)| f.player)) {
            (Some(g), Some(pc)) => g.vm.lockstep_look_follows(pc),
            _ => true,
        };
        let look = self.lockstep.as_ref().filter(|_| !steps_only && shown == local_rank && follows).map_or((0, 0), |l| l.look.offset(fresh));
        let step_place = false;
        let alpha = ((self.lockstep_time * f64::from(openxiii_vm::lockstep::STEP_HZ)) as f32).clamp(0.0, 1.0);
        let last_step = self.lockstep.as_ref().map_or(0, |l| l.step).wrapping_sub(1);
        let camera_from = match (self.lockstep_view_prev, lockstep_view.as_ref().and_then(|(f, _)| f.view)) {
            (Some((step, actor, from, _)), Some((now_actor, now, _)))
                if !step_place && step.wrapping_add(1) == self.lockstep_view_step && self.lockstep_view_step == last_step && actor == now_actor && {
                    let d = [0, 1, 2].map(|i| now[i] - from[i]);
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= openxiii_vm::lockstep_interp::SNAP_DISTANCE
                } =>
            {
                Some(from)
            }
            _ => None,
        };
        let camera_place = match (camera_from, lockstep_view.as_ref().and_then(|(f, _)| f.view)) {
            (Some(from), Some((_, now, _))) => Some([0, 1, 2].map(|i| from[i] + (now[i] - from[i]) * alpha)),
            _ => None,
        };
        let moved = camera_place.zip(lockstep_view.as_ref().and_then(|(f, _)| f.view)).is_some_and(|(p, v)| p != v.1);
        let arms_turned = false;
        let first_person = lockstep_view.as_ref().and_then(|(f, _)| f.view).filter(|_| (look != (0, 0) || moved) && !arms_turned).map(|(_, location, rotation)| Camera {
            position: location,
            yaw: degrees(rotation[1]),
            pitch: degrees(rotation[0]),
            roll: degrees(rotation[2]),
        });
        // The camera the first-person models are drawn with: the step's (the look's turn left out).
        let step_camera = lockstep_view.as_ref().and_then(|(f, _)| f.view).map(|(_, l, r)| (l, r));
        // The HUD's icons over a world point: (anchors, primitives moved).
        let mut anchored = (0usize, 0usize);
        let lockstep_view = lockstep_view.map(|(mut f, fx)| {
            if let Some(v) = f.view.as_mut() {
                v.2[1] = v.2[1].wrapping_add(look.0);
                // XIIIPlayerController.UpdateRotation's limits of the pitch (15500 up, 50000 down).
                let pitch = v.2[0].wrapping_add(look.1) & 65535;
                v.2[0] = if pitch > 15500 && pitch < (50000 + 15500) / 2 {
                    15500
                } else if pitch > (50000 + 15500) / 2 && pitch < 50000 {
                    50000
                } else {
                    pitch
                };
                if let Some(p) = camera_place {
                    v.1 = p;
                }
            }
            // The HUD's icons over a world point (a grapple's hook point, a hint frame, a
            // speaker's arrow) were placed with the step's camera: moved onto the drawn one.
            if let (Some(step), Some((_, l, r))) = (step_camera, f.view) {
                let anchors = std::mem::take(&mut f.anchors);
                anchored = (anchors.len(), openxiii_vm::lockstep_anchor::shift_hud(&mut f.hud, &anchors, step, (l, r)));
                f.anchors = anchors;
            }
            // Another player's view (a cinematic): made for that player's screen size, fitted
            // to this window.
            if shown != local_rank {
                fit_view_to_window(&mut f, client);
            }
            (f, fx)
        });
        let _ = anchored;
        if let (Some((f, fx)), Some(r), Some(g), Some(library)) = (lockstep_view, &mut self.renderer, &mut self.game, &mut self.library) {
            LOCKSTEP_FX.with(|l| *l.borrow_mut() = Some(fx));
            let slot = g.active_viewports().get(shown).copied().unwrap_or(0);
            // This machine's view output in the VM while its frame is drawn (the other
            // players' views were drawn after it in the step), then the VM as it was.
            let kept = self.lockstep_output.take().map(|o| (g.vm.swap_view_output(o), g.swap_view(f.view)));
            // Split screen updates the shadows at each view's present, in player order: the
            // views before this machine's one now (their work drawn with this frame), those after
            // it once this frame is drawn (drawn with the next one): the same work, in the same
            // order, on every machine.
            let local_rank = shown;
            if let Some(c) = self.characters.as_ref() {
                let frame = self.shadow_frame;
                let mut before = std::mem::take(&mut self.lockstep_shadow_after);
                for (rank, s, o, v) in self.lockstep_others.iter_mut().filter(|x| x.0 < local_rank) {
                    let _ = rank;
                    before.extend(other_view_shadows(r, g, c, o, *v, *s, frame));
                }
                LOCKSTEP_SHADOW_JOBS.with(|l| *l.borrow_mut() = before);
            }
            r.overlay_camera = first_person;
            let no_preview = false;
            // The kept scene lists are another step's than the shown view's: made for its camera.
            let other_output = self.lockstep_view_step != self.lockstep_output_step;
            // The frame's camera must be the last step's (the world it draws); counted otherwise.
            let no_interp = false;
            // With the camera between the two steps' cameras, this player's pawn and what it
            // draws in the world too; its first-person models in front of the step's camera, at
            // their place between the two steps' in its space (the weapon's sway and bob every
            // frame instead of once a step).
            let whole = camera_from.is_some();
            let mut interp = match (no_interp, f.player) {
                (false, Some(pc)) if whole => g.vm.lockstep_interp_begin(alpha, &[pc], true),
                (false, Some(pc)) => {
                    let mine: Vec<_> = [Some(pc), g.vm.native_prop(pc, openxiii_vm::NativeField::Pawn).as_object()].into_iter().flatten().collect();
                    g.vm.lockstep_interp_begin(alpha, &mine, false)
                }
                _ => None,
            };
            if let (false, true, Some(pc), Some(now), Some((_, _, from_place, from_rot))) = (no_interp, whole, f.player, step_camera, self.lockstep_view_prev) {
                if let Some(pawn) = g.vm.native_prop(pc, openxiii_vm::NativeField::Pawn).as_object() {
                    let offsets = g.vm.lockstep_first_person_offsets(pawn, (from_place, from_rot), now);
                    g.vm.lockstep_interp_first_person(&mut interp, &offsets, now, alpha);
                }
            }
            let preview = match (f.player, f.view) {
                (Some(pc), Some((actor, location, rotation))) if (look != (0, 0) || moved || other_output || interp.is_some()) && kept.is_some() && f.scene.is_none() && !g.vm.only_post_render && !no_preview => {
                    let step = g.vm.view_output();
                    g.vm.preview_scene(slot, pc, actor, location, rotation, f.fov);
                    LOCKSTEP_STEP_SCENE.with(|l| *l.borrow_mut() = Some(step));
                    true
                }
                _ => false,
            };
            g.vm.preview_drawing = preview;
            let drawn = draw_viewport_frame(r, g, library, self.characters.as_ref(), &f, slot, self.shadow_frame, target, &sign);
            g.vm.lockstep_interp_end(interp);
            g.vm.preview_drawing = false;
            if preview {
                if let Some(step) = LOCKSTEP_STEP_SCENE.with(|l| l.borrow_mut().take()) {
                    let _ = g.vm.swap_view_output(step);
                }
            }
            r.overlay_camera = None;
            if let Some(camera) = drawn {
                self.camera = camera;
            }
            if let Some((vm_output, view)) = kept {
                self.lockstep_output = Some(g.vm.swap_view_output(vm_output));
                g.swap_view(view);
            }
            if let Some(c) = self.characters.as_ref() {
                let frame = self.shadow_frame;
                let mut after = Vec::new();
                for (_, s, o, v) in self.lockstep_others.iter_mut().filter(|x| x.0 > local_rank) {
                    after.extend(other_view_shadows(r, g, c, o, *v, *s, frame));
                }
                self.lockstep_shadow_after = after;
            }
        }
    }

    /// The window's size, as a player's screen size.
    fn lockstep_screen(&self) -> (i32, i32) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        (size.width.max(1) as i32, size.height.max(1) as i32)
    }

    /// A network game that could not start or that ended without this player choosing it: the
    /// reason in the window's title, then on the menus (the Co-op page and a message box,
    /// `lockstep_notice_frame`); off the menus, back to them.
    pub(crate) fn lockstep_fail(&mut self, why: &str) {
        self.lockstep_title(why);
        self.set_lockstep_notice(why);
        self.lockstep_menus_back();
    }

    /// Escape, the gamepad's back or Android's back while meeting (the host waiting for its
    /// guest, or the guest joining a host that does not answer): the meeting is given up (the
    /// other machine told, the socket closed) and the menus come back on the pages left for it,
    /// without a message. True: the key was taken.
    pub(crate) fn lockstep_meet_cancel(&mut self) -> bool {
        let Some(state) = self.lockstep_meeting.take() else { return false };
        let MeetState { meeting, guest, .. } = *state;
        meeting.cancel();
        eprintln!("lockstep: the meeting was given up by this player ({})", if guest { "joining" } else { "hosting" });
        self.lockstep_title("");
        self.lockstep_menus_back();
        true
    }

    /// A network game that did not start: this machine's menus again.
    fn lockstep_menus_back(&mut self) {
        if let Some(g) = self.game.as_mut() {
            g.vm.lockstep_menu_back_wanted();
            if !g.vm.url.map.to_ascii_lowercase().starts_with("mapmenu") {
                // The menus come back on the pages left for the network game, as on any return
                // to the menus (a later CloseAll may have kept an empty list over them).
                if let Some(m) = self.lockstep_menus.as_ref() {
                    g.vm.give_saved_menu_stack(m);
                }
                g.vm.lockstep_back_to_menu();
            }
        }
    }

    fn lockstep_title(&mut self, note: &str) {
        if self.lockstep_note != note {
            self.lockstep_note = note.to_string();
            if let Some(w) = &self.window {
                let base = "OpenXIII".to_string();
                w.set_title(if note.is_empty() { base } else { format!("{base} - {note}") }.as_str());
            }
            if !note.is_empty() {
                eprintln!("lockstep: {note}");
            }
        }
    }

    /// --lockstep, a travel: the Co-op page's Host LAN game (a map with "Listen" and the co-op
    /// option) starts waiting for the guest; its Join LAN game (the host's address with the
    /// option) starts meeting the host. The meeting goes on frame after frame
    /// (`lockstep_meet_poll`, the window keeps drawing with a waiting sign), then both load the
    /// start map as a split-screen co-op game played in lockstep. Back to the menu, the session
    /// ends. False: no travel now.
    pub(crate) fn lockstep_travel(&mut self, req: &mut openxiii_vm::travel::TravelRequest) -> bool {
        use openxiii_vm::coop_address::COOP_OPTION;
        let map = crate::map_stem(&req.url).to_ascii_lowercase();
        let menu = map.starts_with("mapmenu") || map.is_empty() && req.url.host.is_empty();
        if self.lockstep.is_some() {
            if menu {
                // Back to the menus by a player's in-game menu: played on both machines; the
                // machine of the other player tells its player who left.
                let local = self.lockstep.as_ref().and_then(|r| r.local_player());
                let by = self.game.as_ref().and_then(|g| g.vm.lockstep.as_ref()).and_then(|l| l.last_menu_player);
                self.lockstep_end("");
                self.note_departing_player(by, local);
            }
            return true;
        }
        // Another travel while meeting: the meeting is given up (and a met session still on its
        // New game video).
        self.lockstep_intro_drop();
        if self.lockstep_meeting.take().is_some() {
            self.lockstep_title("");
        }
        if !req.url.has_option(COOP_OPTION) {
            return true;
        }
        // The host's own character screen (a stage before the start, its URL carries the co-op
        // options too) stays this machine's: the session starts with the start it then loads.
        // Without "Listen" and its port: no server-client game opens the port the session uses.
        if character_selection_travel(&mut req.url) { return true; }
        // The menus' latest choices (this player's character, the way the game starts).
        if let Some(p) = self.game.as_mut().and_then(|g| g.vm.take_local_play_change()) {
            self.local_play = p;
        }
        let size = self.lockstep_screen();
        // The menu pages this machine's menus kept when they were left (given back after the
        // session).
        self.lockstep_menus = self.game.as_ref().and_then(|g| g.vm.saved_menu_stack());
        let game_dir = self.options.game.clone();
        let mods = mods_in_use();
        let checksum = data_checksum(&game_dir, &mods);
        let extra = self.game.as_ref().map_or(0, |g| g.vm.coop_extra_enemies());
        let start_mode = self.local_play.coop.and_then(|c| c.start);
        let pads = self.gamepad_config;
        let mut mine = self.game.as_mut().map(|g| profile_entries(&mut g.vm, &pads)).unwrap_or_default();
        // Host: what the start loads besides its URL (travel items, music, checkpoints, the Select
        // level equipment), for the guest and for its own new VM.
        if req.url.host.is_empty() {
            if let Some(g) = self.game.as_ref() {
                mine.extend(g.vm.lockstep_start_entries());
            }
            let info: Vec<(String, Vec<u8>)> = req.info.iter().map(|(n, t)| (n.clone(), t.clone().into_bytes())).collect();
            mine.push(("start/info".into(), pack(&info)));
            if let Some((m, classes)) = &self.weapons_for {
                mine.push(("start/weapons".into(), std::iter::once(m.clone()).chain(classes.iter().map(|c| c.to_string())).collect::<Vec<_>>().join("\n").into_bytes()));
            }
        }
        let wait = Duration::from_secs(MEET_WAIT);
        let meeting = if !req.url.host.is_empty() {
            // Guest: the host's address; its own character goes with the request.
            let port = req.url.port.filter(|&p| p > 0).map_or(PORT, |p| p as u16);
            let addr = std::net::ToSocketAddrs::to_socket_addrs(&(req.url.host.as_str(), port)).ok().and_then(|mut a| a.find(|x| x.is_ipv4()));
            let Some(addr) = addr else {
                self.lockstep_fail("the host's address is not valid");
                return false;
            };
            let wish = self.local_play.characters.map_or(String::new(), |c| format!("Chars={}.{}", c[0].0, c[0].1));
            Meeting::join(addr, Agreement { checksum, start: wish, map_sum: 0, build: BUILD_ID.into() }, size, pack(&mine), wait)
        } else {
            // Host: the map without the server-client options, with its difficulty (GameInfo's
            // globalconfig Difficulty, which InitGame reads from the URL first).
            let port = req.url.get_option("Port=").and_then(|p| p.parse().ok()).unwrap_or(PORT);
            req.url.options.retain(|o| {
                let key = o.split('=').next().unwrap_or("");
                !key.eq_ignore_ascii_case("Listen") && !key.eq_ignore_ascii_case(COOP_OPTION) && !key.eq_ignore_ascii_case("Port")
            });
            if !req.url.has_option("Difficulty") {
                let difficulty = self.game.as_mut().and_then(|g| g.vm.config.as_mut()).and_then(|c| c.get_string("Engine.GameInfo", "Difficulty", "System"));
                if let Some(d) = difficulty.filter(|d| d.trim().parse::<u8>().is_ok()) {
                    req.url.add_option(&format!("Difficulty={}", d.trim()));
                }
            }
            let (text_lang, sound_lang) = self.game.as_mut().and_then(|g| g.vm.config.as_mut()).map(|c| (c.language.clone(), c.get_string("HXAudio.HXAudioSubsystem", "SoundLanguage", "User").unwrap_or_default())).unwrap_or_default();
            let start_word = match start_mode {
                Some(openxiii_vm::local_play::CoopStart::NewGame) => "NewGame",
                Some(openxiii_vm::local_play::CoopStart::Continue) => "Continue",
                Some(openxiii_vm::local_play::CoopStart::Load) => "Load",
                Some(openxiii_vm::local_play::CoopStart::SelectLevel) => "SelectLevel",
                None => "None",
            };
            let start = format!("{}|ExtraEnemies={extra};Lang={text_lang},{sound_lang};Start={start_word};Clock={}", req.url.string(), openxiii_vm::lockstep::local_clock_now());
            let ours = Agreement { checksum, start: start.clone(), map_sum: map_checksum(&game_dir, &map), build: BUILD_ID.into() };
            // Player 1's character is this machine's, player 2's the guest's (both or none).
            let own_char = self.local_play.characters.map(|c| c[0]);
            let compose = move |wishes: &str| {
                let theirs = wishes.split(';').find_map(|kv| kv.strip_prefix("Chars=")).and_then(|v| {
                    let (r, k) = v.split_once('.')?;
                    Some((r.parse::<u8>().ok()?, k.parse::<u8>().ok()?))
                });
                // Always both (the default one for a player who chose none): a character screen
                // must never come back on one machine only.
                let (a, b) = (own_char.unwrap_or((0, 0)), theirs.unwrap_or((0, 0)));
                format!("{start};Chars={}.{},{}.{}", a.0, a.1, b.0, b.1)
            };
            let mut m = Meeting::host(port, ours, Box::new(compose), size, Some(2), pack(&mine), wait);
            if let (Ok(m), Some(g)) = (m.as_mut(), self.game.as_mut()) {
                list_coop_host(m, &mut g.vm, &crate::map_stem(&req.url));
            }
            m
        };
        match meeting {
            Ok(m) => {
                let guest = !req.url.host.is_empty();
                self.lockstep_meeting = Some(Box::new(MeetState { meeting: m, req: req.clone(), guest, size, extra, start_mode, mine }));
                self.lockstep_title(if guest { "joining the host..." } else { "waiting for the other player..." });
            }
            Err(e) => self.lockstep_fail(&e),
        }
        false
    }

    /// Once a frame: the meeting under way goes on; once both machines met, the session starts and
    /// the start map is loaded.
    pub(crate) fn lockstep_meet_poll(&mut self) {
        use openxiii_vm::local_play::{CoopMode, CoopSession};
        let Some(state) = self.lockstep_meeting.as_mut() else { return };
        let met = match state.meeting.poll() {
            MeetPoll::Waiting => return,
            MeetPoll::Failed(e) => {
                self.lockstep_meeting = None;
                self.lockstep_fail(&e);
                return;
            }
            MeetPoll::Ready(m) => m,
        };
        let Some(state) = self.lockstep_meeting.take() else { return };
        let MeetState { mut req, guest, size, extra, start_mode, mine, .. } = *state;
        let Met { mut session, peer_size, start, blob } = met;
        let theirs = unpack(&blob).unwrap_or_default();
        let game_dir = self.options.game.clone();
        let (sizes, extra, settings) = if guest {
            let (url, settings) = start.start.split_once('|').unwrap_or((start.start.as_str(), ""));
            let url = openxiii_vm::travel::Url::parse(None, url, openxiii_vm::travel::TRAVEL_ABSOLUTE);
            if map_checksum(&game_dir, &crate::map_stem(&url)) != start.map_sum {
                session.bye();
                self.lockstep_fail("the host's map is not the same as this one");
                return;
            }
            let extra = settings.split(';').find_map(|kv| kv.strip_prefix("ExtraEnemies=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            req.url = url;
            (vec![peer_size, size], extra, settings.to_string())
        } else {
            (vec![size, peer_size], extra, start.start.split_once('|').map_or("", |x| x.1).to_string())
        };
        let characters = characters_of(&settings);
        // The way the game started, as the host's (the menus' co-op state the game reads).
        let start_mode = match settings.split(';').find_map(|kv| kv.strip_prefix("Start=")) {
            Some("NewGame") => Some(openxiii_vm::local_play::CoopStart::NewGame),
            Some("Continue") => Some(openxiii_vm::local_play::CoopStart::Continue),
            Some("Load") => Some(openxiii_vm::local_play::CoopStart::Load),
            Some("SelectLevel") => Some(openxiii_vm::local_play::CoopStart::SelectLevel),
            Some(_) => None,
            None => start_mode,
        };
        // The host's language for both machines (texts and voices are part of the game).
        let language = settings.split(';').find_map(|kv| kv.strip_prefix("Lang=")).and_then(|v| v.split_once(',')).map(|(t, s)| (t.to_string(), s.to_string()));
        if let Some((text, _)) = language.as_ref().filter(|_| guest) {
            if !language_installed(&game_dir, text) {
                session.bye();
                self.lockstep_fail(&format!("the host plays in another language ({text}), not installed here"));
                return;
            }
        }
        // The host's clock at the meeting, LevelInfo's date on both machines.
        let session_clock = settings.split(';').find_map(|kv| kv.strip_prefix("Clock=")).and_then(|v| v.parse().ok());
        // The class defaults are the host's User.ini on both machines; each player's own values
        // are put on their objects by the VM (Profile).
        let (host_entries, guest_entries) = if guest { (&theirs, &mine) } else { (&mine, &theirs) };
        // The start as the host has it: travel items, the Select level equipment, then (in
        // `travel`) the music and checkpoints of its new VM.
        let get = |n: &str| host_entries.iter().find(|e| e.0 == n).map(|e| e.1.clone());
        if let Some(info) = get("start/info").and_then(|b| unpack(&b)) {
            req.info = info.into_iter().map(|(n, t)| (n, String::from_utf8_lossy(&t).into_owned())).collect();
        }
        if guest {
            self.weapons_for = get("start/weapons").map(|b| {
                let text = String::from_utf8_lossy(&b).into_owned();
                let mut lines = text.split('\n').map(str::to_string);
                // The class names live as long as the game (the host's list is small).
                (lines.next().unwrap_or_default(), lines.map(|c| &*Box::leak(c.into_boxed_str())).collect())
            });
        }
        // The Select level equipment is session state, the same on both machines (the guest's
        // from the host's start, the host's own): the first arrival's equipment, the loadout given
        // again on a Retry of its level, the save slots read from the session's start.
        let (weapons, loadout, reads) = crate::choice_level::session_equipment(self.weapons_for.take());
        self.weapons_for = weapons;
        self.loadout = loadout;
        self.slot_reads_seen = reads;
        self.lockstep_start_state = Some(host_entries.iter().filter(|e| e.0.starts_with("start/")).cloned().collect());
        // The session's settings files: the host's (this machine's own sections kept), for every
        // VM of the session (ConfigCache::apply_session_ini).
        let host_file = |n: &str| host_entries.iter().find(|e| e.0 == n).map(|e| String::from_utf8_lossy(&e.1).into_owned());
        let ini = host_file("profile/User.ini").map(|u| (u, host_file("profile/System.ini")));
        // The session's saves are written into this machine's own profile only on the host.
        session.set_adaptive(true);
        let lp = openxiii_vm::local_play::LocalPlay { coop: Some(CoopSession { mode: CoopMode::SplitScreen, start: start_mode }), local_players: 2, characters };
        self.local_play = lp;
        if let Some(g) = &mut self.game {
            g.vm.set_local_play(lp);
            g.vm.set_coop_extra_enemies(extra);
        }
        let _math = openxiii_math::Mode::Portable.enter();
        let mut runner = Runner::new(Source::Net(session), sizes);
        runner.session = Some(openxiii_vm::lockstep::SessionContext {
            math: openxiii_math::Mode::Portable,
            clock: session_clock,
            language,
            ini,
            host: !guest,
            ..Default::default()
        });
        runner.profiles = vec![profile_of(host_entries), profile_of(guest_entries)];
        // A new game: the main menu's New game video first, on both machines (each player can
        // skip it), then the first level. The host's menus asked for the level without the video
        // (coop_page.rs) so that the player who joins sees it too.
        let intro = openxiii_vm::coop_address::INTRO_OPTION;
        if req.url.has_option(intro) {
            req.url.options.retain(|o| !o.eq_ignore_ascii_case(intro));
            if self.game.as_mut().is_some_and(|g| g.vm.play_new_game_video()) {
                self.lockstep_intro_pad = self.lockstep_pad_buttons();
                self.lockstep_intro = Some(Box::new((runner, req)));
                self.lockstep_title("");
                return;
            }
        }
        self.lockstep_go(runner, req);
    }

    /// The session met (and its New game video over): it starts with its first level.
    fn lockstep_go(&mut self, runner: Runner, req: openxiii_vm::travel::TravelRequest) {
        self.lockstep = Some(runner);
        self.lockstep_time = 0.0;
        self.lockstep_undrawn = 0;
        self.lockstep_view = None;
        self.lockstep_output = None;
        self.lockstep_others.clear();
        self.lockstep_fresh = true;
        self.lockstep_title("");
        // As every travel of the main loop: the New game video's last frame taken away, then the
        // loading picture drawn (PaintProgress) before the load blocks, so that the load shows
        // it and the picture kept for the other machine is that one (a skipped video left its
        // frame on screen for the whole load, then a level still being set up).
        self.video_show = None;
        if let Some(r) = self.renderer.as_mut() {
            r.set_video(None);
        }
        // The video's sound stopped now: the travel gives the session a new VM, and the old
        // one's Stop would be lost with it (the sound went on into the level and the menus).
        self.apply_video_audio();
        self.paint_progress();
        self.travel_held(req);
    }

    /// Once a frame while the New game video of a met session plays: the other machine keeps
    /// hearing from this one; Enter, Escape, Space or a new gamepad press skips the video (as
    /// the main menu's), and once it is over the first level loads. The machine that gets there
    /// first waits on its loading picture for the other one (`lockstep_hold_frame`).
    pub(crate) fn lockstep_intro_poll(&mut self) {
        let Some(intro) = self.lockstep_intro.as_mut() else { return };
        intro.0.keep_alive();
        // A skip on the other machine is this one's too (one player skips for both).
        let remote = intro.0.other_skipped();
        let down = self.lockstep_pad_buttons();
        let pressed = down & !std::mem::replace(&mut self.lockstep_intro_pad, down) != 0;
        if pressed {
            self.lockstep_intro_skip();
        }
        let Some(g) = self.game.as_mut() else { return };
        if remote && g.vm.video_status() != 0 {
            eprintln!("lockstep: the other player skipped the video");
            g.vm.video_skip();
        }
        if g.vm.video_status() != 0 {
            return;
        }
        if let Some(intro) = self.lockstep_intro.take() {
            let (runner, req) = *intro;
            self.lockstep_go(runner, req);
        }
    }

    /// A key of the window while the New game video of a met session plays: true when it skips
    /// the video (the key goes no further).
    pub(crate) fn lockstep_intro_key(&mut self, code: winit::keyboard::KeyCode, pressed: bool) -> bool {
        use winit::keyboard::KeyCode;
        if self.lockstep_intro.is_none() || !matches!(code, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape | KeyCode::Space) {
            return false;
        }
        if pressed {
            self.lockstep_intro_skip();
        }
        true
    }

    /// Android: the app goes to the background (`on`) or comes back during a session (or its New
    /// game video): the other machine is told (Runner::tell_paused).
    #[allow(dead_code)]
    pub(crate) fn lockstep_paused(&mut self, on: bool) {
        if let Some(r) = self.lockstep.as_mut() {
            r.tell_paused(on);
        }
        if let Some(intro) = self.lockstep_intro.as_mut() {
            intro.0.tell_paused(on);
        }
        eprintln!("lockstep: this machine {} the background", if on { "goes to" } else { "is back from" });
    }

    /// This machine's player skips the New game video of a met session: stopped here, and the
    /// other machine is told so that it stops too.
    fn lockstep_intro_skip(&mut self) {
        let Some(g) = self.game.as_mut() else { return };
        if g.vm.video_status() == 0 {
            return;
        }
        g.vm.video_skip();
        if let Some(intro) = self.lockstep_intro.as_mut() {
            intro.0.tell_skip();
        }
    }

    /// The meeting's session given up before its first level (another travel, the window
    /// closed): the other machine is told.
    fn lockstep_intro_drop(&mut self) {
        if let Some(intro) = self.lockstep_intro.take() {
            let (mut runner, _) = *intro;
            if let Source::Net(s) = &mut runner.source {
                s.bye();
            }
            if let Some(g) = self.game.as_mut() {
                g.vm.video_skip();
            }
            self.apply_video_audio();
        }
    }

    /// After a travel of a session, the loading picture is kept (warm_after_travel) until the
    /// other machine is in the level too: both players see it at the same time, instead of a
    /// level still being set up behind the waiting sign. This machine's steps go on behind the
    /// picture meanwhile (the session is the same whatever is shown): only the drawing waits.
    fn lockstep_hold_frame(&mut self) {
        let Some(first) = self.lockstep_hold.as_ref().map(|h| h.1) else { return };
        // Both machines past the level's first second of steps: each one's load and warm-up are
        // over and its steps come at their pace (a slow machine still making its first frames
        // left the other one on a revealed level held still for seconds, as if not loaded).
        let settle = HOLD_SETTLE_STEPS;
        let own = self.lockstep.as_ref().is_none_or(|r| r.step >= first + settle);
        let arrived = own && self.lockstep.as_mut().map_or(true, |r| r.other_reached(first + settle));
        // The level's warm-up after the travel (warm_after_travel) ran before the session's first
        // step: nothing of the level was drawn yet (no view). Behind the picture, this machine's
        // view is drawn off screen a few times once its steps made one, so that the level's
        // textures, pipelines and targets exist before the picture goes (on a slow machine the
        // level was otherwise seen being built, objects appearing one after the other).
        let warmed = self.lockstep_hold_warm >= HOLD_WARM_FRAMES;
        if arrived && warmed {
            eprintln!("lockstep: both machines a second into the level, the loading picture goes (from step {first}, now {}, {} frames warmed)", self.lockstep.as_ref().map_or(0, |r| r.step), self.lockstep_hold_warm);
            self.lockstep_reveal();
            return;
        }
        // Drawn off screen at every frame of the hold once a view exists (at least
        // HOLD_WARM_FRAMES of them): what the level's first second brings out (the weapon raised
        // in hand, the characters coming in) is made behind the picture too.
        let draw = self.lockstep_view.is_some();
        if let Some(r) = self.renderer.as_mut() {
            r.warming = draw;
        }
        if draw {
            // Drawn off screen this frame (Renderer::warming: the surface is the picture's).
            self.lockstep_hold_warm += 1;
            self.lockstep_skip_draw = false;
            if self.lockstep_hold_warm == HOLD_WARM_FRAMES {
                eprintln!("lockstep: the level drawn {HOLD_WARM_FRAMES} times behind the loading picture");
            }
            let _ = warmed;
            return;
        }
        // The picture is shown by its own thread: this frame draws nothing, and waits a little
        // instead of spinning.
        self.lockstep_skip_draw = true;
        std::thread::sleep(std::time::Duration::from_millis(4));
    }

    /// The loading picture kept after a travel taken away (the other machine arrived, or the
    /// session ended).
    pub(crate) fn lockstep_reveal(&mut self) {
        if let Some((hold, _)) = self.lockstep_hold.take() {
            if let Some(r) = self.renderer.as_mut() {
                r.warming = false;
                r.hold_stop(hold);
            }
            self.last = std::time::Instant::now();
        }
    }

    /// The session is over (back to the menu, the other machine gone): this machine plays as
    /// before again.
    pub(crate) fn lockstep_end(&mut self, why: &str) {
        self.lockstep_reveal();
        if let Some(mut r) = self.lockstep.take() {
            r.finish();
            eprintln!("{}", report(&r));
        }
        if let Some(g) = &mut self.game {
            g.vm.lockstep_stop();
        }
        let _math = openxiii_math::Mode::default().enter();
        self.lockstep_restore = true;
        self.lockstep_view = None;
        self.lockstep_output = None;
        self.lockstep_others.clear();
        self.lockstep_title(why);
    }

    /// One frame of a lockstep session: the session's fixed steps the real time asks for (at most
    /// a few to catch up), each with every player's view made for that player's own screen.
    pub(crate) fn lockstep_steps(&mut self, dt: f32) {
        const STEP: f64 = 1.0 / openxiii_vm::lockstep::STEP_HZ as f64;
        self.lockstep_skip_draw = false;
        // A load's warm-up step (no time): not a step of the session.
        if dt <= 0.0 {
            return;
        }
        // This frame's mouse look of this machine's player, for its next command.
        if let (Some(g), Some(r)) = (&mut self.game, &mut self.lockstep) {
            if let Some(i) = r.local_player() {
                g.vm.lockstep_local_look(i, dt);
            }
        }
        // Clock sync (pacing only): a machine whose clock runs ahead of the other one's lets its
        // session time pass a little slower (at most CLOCK_SLOW_MAX), so that both keep the same
        // margin before each other's commands, instead of one of them stopping short at every
        // late command and catching up just after (a hitch each time).
        let ahead = self.lockstep.as_ref().and_then(|r| r.clock_ahead()).filter(|_| clock_sync_on());
        let slow = ahead.map_or(0.0, |a| ((a - CLOCK_DEAD) * CLOCK_SLOW_PER_STEP).clamp(0.0, CLOCK_SLOW_MAX));
        self.lockstep_time = (self.lockstep_time + f64::from(dt) * (1.0 - slow)).min(STEP * f64::from(CATCH_UP) * 2.0);
        let size = self.lockstep_screen();
        let client = (size.0 as u32, size.1 as u32);
        let mut lost = false;
        let mut n = 0;
        // Catching up after a wait: the steps this frame's time asks for, plus one at most, so
        // that the game catches up over several frames instead of jumping forward (pacing only:
        // a little more latency for a while, no burst of steps in one frame).
        let cap = ((f64::from(dt) / STEP).ceil() as u32 + 1).clamp(2, steps_per_frame());
        let mut waited = false;
        while self.lockstep_time >= STEP && n < cap {
            let (Some(g), Some(r)) = (&mut self.game, &mut self.lockstep) else { return };
            // This machine's screen size changed: it goes with this player's next command.
            if let Some(i) = r.local_player() {
                if g.vm.lockstep_size(i).is_some_and(|s| s != size) {
                    if let Some(l) = g.vm.lockstep.as_mut() {
                        l.local_size = Some(size);
                    }
                }
            }
            match r.poll(&mut g.vm) {
                Some(true) => {
                }
                Some(false) => {
                    waited = true;
                    // Waiting because this machine runs ahead: the time of the wait is not owed
                    // (catching it up afterwards would put this machine ahead again, and the
                    // next late command would stop it again).
                    if ahead.is_some_and(|a| a > CLOCK_DEAD) {
                        self.lockstep_time = self.lockstep_time.min(STEP);
                    }
                    break;
                }
                None => {
                    lost = true;
                    break;
                }
            }
            g.update(STEP as f32);
            let travel = g.vm.engine_travel_tick_start(STEP as f32);
            crate::game_log::write_value(&mut g.vm.log);
            if travel.is_none() && !g.vm.engine.video.draw_skipped {
                // (the drawn view gets the instant look in draw_frame_views)
                // This machine's view, or during a cinematic the view of the player it holds
                // (split screen shows that one alone): every machine shows the cinematic.
                let local = shown_rank(g, r);
                // This step's own view is presented when no other step runs in this frame after it.
                // A frame left undrawn (still late after its last step, below) presents none.
                let undrawn = n + 1 >= cap && self.lockstep_time - STEP >= STEP && self.lockstep_undrawn < undrawn_in_a_row();
                let presented = (self.lockstep_time - STEP < STEP || n + 1 >= cap) && !undrawn;
                let state_only = true;
                let mut others = Vec::new();
                for (i, slot) in g.active_viewports().into_iter().enumerate() {
                    // A view no frame draws (another player's, or this machine's on a step another
                    // one follows in this frame): only what the game reads back.
                    g.vm.view_state_only = state_only && (i != local || !presented);
                    let drawn = g.draw_viewport(slot, client);
                    g.vm.view_state_only = false;
                    if let Some(f) = drawn {
                        let fx = g.vm.screen_fx_frame(f.player, f.view.map(|v| v.1));
                        // The other players' views: what they drew, for the work split screen
                        // does at each view's present (the shadows of the actors that view drew).
                        if i != local {
                            others.push((i, slot, g.vm.view_output(), f.view));
                        }
                        let every = true;
                        // The previous step's camera, for a frame's camera between the two.
                        if i == local && (presented || every) {
                            self.lockstep_view_prev = self.lockstep_view.as_ref().and_then(|(v, _)| v.view).map(|(a, l, rot)| (self.lockstep_view_step, a, l, rot));
                        }
                        if i == local && !presented && every {
                            self.lockstep_view = Some((f.clone(), fx.clone()));
                            self.lockstep_view_step = r.step;
                        }
                        if i == local && presented {
                            self.lockstep_view_step = r.step;
                            self.lockstep_output_step = r.step;
                            // What this view left in the VM, for its frame (the views after it
                            // overwrite it): put back only while the frame is presented.
                            self.lockstep_output = Some(g.vm.view_output());
                            self.lockstep_view = Some((f, fx));
                        }
                    }
                }
                self.lockstep_others = others;
            }
            let desync = r.end(&mut g.vm);
            // The Select level equipment: given at a step on every machine.
            self.lockstep_equipment_now = true;
            self.give_start_equipment();
            self.lockstep_equipment_now = false;
            let (Some(g), Some(r)) = (&mut self.game, &mut self.lockstep) else { return };
            let _ = (&g, &r);
            if let Some(d) = desync {
                eprintln!("lockstep: DESYNC at step {} ({}): here {:08x?}, other machine {:08x?}", d.step, d.kind, d.local, d.remote);
                if differ_shown() {
                    self.lockstep_title(&format!("the two games differ (step {}, {})", d.step, d.kind));
                }
            }
            self.lockstep_time -= STEP;
            n += 1;
            if let Some(req) = travel {
                self.pending_travel = Some(req);
                self.lockstep_view = None;
                self.lockstep_output = None;
        self.lockstep_others.clear();
                break;
            }
        }
        // A machine still late after its frame's steps (the steps alone use its time): this frame
        // is not drawn, its time goes to the next frame's steps; never more than a few frames in
        // a row, so that the picture keeps moving. Drawing only: the steps and their commands
        // are the same whatever is drawn.
        let late = !waited && !lost && n >= cap && self.lockstep_time >= STEP && self.pending_travel.is_none();
        self.lockstep_skip_draw = late && self.lockstep_undrawn < undrawn_in_a_row();
        self.lockstep_undrawn = if self.lockstep_skip_draw { self.lockstep_undrawn + 1 } else { 0 };
        // The loading picture still kept for the other machine.
        if self.pending_travel.is_none() {
            self.lockstep_hold_frame();
        }
        // The two games went apart for good (lockstep.rs, `apart`): both players are told and go
        // back to the menus; a passing difference only goes to the log.
        let apart = self.lockstep.as_ref().is_some_and(|r| matches!(&r.source, Source::Net(s) if s.apart));
        if apart {
            let why = "the two games went apart";
            self.lockstep_end(why);
            self.lockstep_fail(why);
            return;
        }
        if lost {
            // The other machine said goodbye (it went back to its menus or closed the game), or
            // nothing came from it any more. After a difference of the games, the other machine
            // most likely left because its game went apart.
            let (left, differed) = self.lockstep.as_ref().map_or((false, false), |r| match &r.source {
                Source::Net(s) => (s.peer_left, s.desync.is_some()),
                _ => (false, false),
            });
            let why = if left && differed {
                "the two games went apart"
            } else if left {
                "the other player left"
            } else {
                "the other machine stopped answering"
            };
            self.lockstep_end(why);
            self.lockstep_fail(why);
            return;
        }
        let waiting = self.lockstep.as_ref().map_or(0.0, |r| r.waiting);
        if waiting > 0.25 {
            self.lockstep_title("waiting for the other player...");
        } else if self.lockstep_note.starts_with("waiting") {
            self.lockstep_title("");
        }
    }
}

register_viewer_layer!("lockstep_menu");
