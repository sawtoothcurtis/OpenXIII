//! Enhanced network play (improved build, Graphics: Enhanced): the other players of a classic
//! multiplayer game drawn smoothly between their moves, and the switch of enhanced network play
//! itself (`Vm::net_enhanced`).
//!
//! Only what this machine draws: the game's state is untouched (shots are tested at the true
//! places), and the protocol is the game's own.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::ObjId;
use crate::Vm;

/// Enhanced multiplayer: farthest a remote player is drawn from where it is (units).
const VERSUS_SMOOTH_MAX: f32 = 6.0;
/// Enhanced multiplayer: packets a second a machine may send only to acknowledge, shared by
/// its connections.
pub(crate) const VERSUS_ACK_RATE: f32 = 60.0;
/// Time the drawing error of a new move fades out in (seconds).
const FADE: f64 = 0.08;

/// Network play: a remote player's pawn drawn smoothly (Vm::net_smooth_tick).
#[derive(Debug, Clone)]
pub(crate) struct NetSmooth {
    pub pawn: ObjId,
    /// Place at its last move, and the level time of that move.
    pub last: [f32; 3],
    pub t_last: f64,
    /// Speed between its last two moves (units per second).
    pub vel: [f32; 3],
    /// Drawn place minus true place, fading out.
    pub offset: [f32; 3],
    /// Average time between moves.
    pub interval: f64,
    /// Place to draw (absolute), or None: drawn at its true place.
    pub drawn: Option<[f32; 3]>,
}

impl NetSmooth {
    fn new(pawn: ObjId, loc: [f32; 3], now: f64) -> NetSmooth {
        NetSmooth {
            pawn,
            last: loc,
            t_last: now,
            vel: [0.0; 3],
            offset: [0.0; 3],
            interval: 0.033,
            drawn: None,
        }
    }

    /// Extrapolated drawing (a versus game, where shots are tested at the true place): its
    /// place, plus its speed times the time since its last move (at most a move and a half),
    /// plus an error that fades out in 80 ms so that a new move never makes it jump. `cap`: how
    /// far from its true place it may be drawn.
    fn extrapolated(&mut self, loc: [f32; 3], now: f64, dt: f64, cap: Option<f32>) {
        if loc != self.last {
            let gap = now - self.t_last;
            let ext = gap.min(self.interval * 1.5).min(0.1) as f32;
            // Where it was being drawn (within the cap, as drawn below: otherwise a capped
            // drawing jumped forward at every move by what the cap had held back).
            let mut lead: [f32; 3] = std::array::from_fn(|k| self.vel[k] * ext + self.offset[k]);
            if let Some(cap) = cap {
                let len = lead.iter().map(|v| v * v).sum::<f32>().sqrt();
                if len > cap {
                    lead = lead.map(|v| v * cap / len);
                }
            }
            let drawn: [f32; 3] = std::array::from_fn(|k| self.last[k] + lead[k]);
            self.vel = if gap > 0.0 && gap <= 0.25 { std::array::from_fn(|k| (loc[k] - self.last[k]) / gap as f32) } else { [0.0; 3] };
            self.offset = std::array::from_fn(|k| drawn[k] - loc[k]);
            if self.offset.iter().map(|v| v * v).sum::<f32>() > 150.0 * 150.0 {
                self.offset = [0.0; 3];
                self.vel = [0.0; 3];
            }
            if gap > 0.0 && gap <= 0.25 {
                self.interval = self.interval * 0.8 + gap * 0.2;
            }
            self.last = loc;
            self.t_last = now;
        }
        let fade = (-dt / FADE).dexp() as f32;
        self.offset = self.offset.map(|v| v * fade);
        let ext = (now - self.t_last).min(self.interval * 1.5).min(0.1) as f32;
        let mut render: [f32; 3] = std::array::from_fn(|k| self.vel[k] * ext + self.offset[k]);
        if let Some(cap) = cap {
            let len = render.iter().map(|v| v * v).sum::<f32>().sqrt();
            if len > cap {
                render = render.map(|v| v * cap / len);
            }
        }
        self.drawn = Some(std::array::from_fn(|k| loc[k] + render[k]));
    }
}

#[cfg(test)]
thread_local! {
    static TEST_NET_ENHANCED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: enhanced network play on this thread (Graphics: Enhanced of the improved build).
#[cfg(test)]
pub(crate) fn set_test_net_enhanced(on: bool) {
    TEST_NET_ENHANCED.set(on);
}

/// Whose pawns a machine draws smoothly (Vm::net_smooth_tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmoothKind {
    None,
    /// Enhanced multiplayer, a listen server: its remote players' pawns.
    VersusServer,
    /// Enhanced multiplayer, a client: the other players' copies.
    VersusClient,
}

/// `driver`: this machine's network driver, `Some(true)` for a client's (it has a server
/// connection), `Some(false)` for a listen server's.
pub(crate) fn smooth_kind(enhanced: bool, driver: Option<bool>) -> SmoothKind {
    match driver {
        Some(false) if enhanced => SmoothKind::VersusServer,
        Some(true) if enhanced => SmoothKind::VersusClient,
        _ => SmoothKind::None,
    }
}

impl Vm {
    /// Enhanced network play (improved build with Graphics: Enhanced in the Video menu, the
    /// same switch as the look; Original keeps the game's own netcode): on a LAN,
    /// acknowledgements sent without waiting for the keep-alive (a bounded number a second),
    /// and remote players drawn smoothly. Only what this machine sends more often or draws: the
    /// protocol is the game's, an original client or server plays with it.
    pub(crate) fn net_enhanced(&self) -> bool {
        #[cfg(test)]
        if TEST_NET_ENHANCED.get() {
            return true;
        }
        !cfg!(test) && crate::improved::network::network_presentation_enhanced(self.engine.client.session_graphics)
    }

    /// Whose pawns this machine draws smoothly (net_smooth_tick).
    fn net_smooth_kind(&self) -> SmoothKind {
        let driver = self.ipdrv.net_driver.as_ref().map(|d| d.server_conn.is_some());
        smooth_kind(self.net_enhanced(), driver)
    }

    /// Network play: the pawn of a player on another machine only moves when one of its moves
    /// arrives (ServerMove, at that machine's frame rate: 25 to 45 a second, and in bursts on a
    /// mobile link or a VPN), so it was drawn in steps. It is drawn (not moved: the game's state
    /// is untouched) ahead of its last move, within a few units (NetSmooth::extrapolated).
    pub(crate) fn net_smooth_tick(&mut self) {
        let pawns: Vec<ObjId> = match self.net_smooth_kind() {
            // A listen server: its remote players' pawns.
            SmoothKind::VersusServer => {
                let pcs: Vec<ObjId> = self.ipdrv.net_driver.as_ref().map(|d| d.conns.iter().filter_map(|c| c.pc).collect()).unwrap_or_default();
                pcs.into_iter().filter(|&pc| self.alive(pc)).filter_map(|pc| self.prop(pc, "Pawn").as_object()).filter(|&p| self.alive(p)).collect()
            }
            // A client: the other players' copies (simulated or dumb proxies; its own pawn is an
            // autonomous proxy), moved only when an update arrives.
            SmoothKind::VersusClient => self
                .actors
                .iter()
                .copied()
                .filter(|&a| self.alive(a) && self.is_a(a, "XIIIPlayerPawn") && matches!(self.prop(a, "Role").as_int(), 1 | 2))
                .collect(),
            SmoothKind::None => Vec::new(),
        };
        if pawns.is_empty() {
            if !self.coop.net_smooth.is_empty() {
                self.coop.net_smooth.clear();
            }
            return;
        }
        let now = self.level_time;
        let dt = f64::from(self.engine_delta).max(0.0);
        let mut seen = Vec::new();
        for p in pawns {
            seen.push(p);
            let loc = self.prop(p, "Location").as_vector();
            let e = match self.coop.net_smooth.iter_mut().find(|e| e.pawn == p) {
                Some(e) => e,
                None => {
                    self.coop.net_smooth.push(NetSmooth::new(p, loc, now));
                    continue;
                }
            };
            // Shots are tested where the pawn is, not where it is drawn: within a few units.
            e.extrapolated(loc, now, dt, Some(VERSUS_SMOOTH_MAX));
        }
        self.coop.net_smooth.retain(|e| seen.contains(&e.pawn));
    }

    /// How far from its place an actor is drawn (a smoothed remote player's pawn, or an actor
    /// attached to it: its weapon).
    pub(crate) fn net_render_offset(&self, a: ObjId) -> Option<[f32; 3]> {
        if self.coop.net_smooth.is_empty() {
            return None;
        }
        let find = |x: ObjId| {
            let e = self.coop.net_smooth.iter().find(|e| e.pawn == x)?;
            let d = e.drawn?;
            let l = self.prop(x, "Location").as_vector();
            Some(std::array::from_fn(|k| d[k] - l[k]))
        };
        find(a).or_else(|| self.prop(a, "Base").as_object().and_then(find))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Another player's copy on a multiplayer client, walking at 300 units a second, updated 30
    /// times a second (each update 0 to 10 ms late), seen at 120 frames a second and drawn the
    /// way `kind` draws it: (largest frame-to-frame change of the drawn step, mean step, farthest
    /// drawn from its true place).
    fn versus_copy_walk(kind: SmoothKind) -> (f32, f32, f32) {
        let mut seed = 0x0bad_5eed_1234_5678u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let updates: Vec<(f64, f32)> = (0..300).map(|i| (5.0 + i as f64 / 30.0 + rnd() * 0.01, 300.0 * i as f32 / 30.0)).collect();
        let mut e = NetSmooth::new(1, [0.0; 3], 5.0);
        let (mut loc, mut next, mut now) = ([0.0f32; 3], 0, 5.0);
        let dt = 1.0 / 120.0;
        let (mut drawn, mut farthest) = (Vec::new(), 0.0f32);
        while now < 14.0 {
            while next < updates.len() && updates[next].0 <= now {
                loc = [updates[next].1, 0.0, 0.0];
                next += 1;
            }
            let x = if kind == SmoothKind::None {
                loc[0]
            } else {
                e.extrapolated(loc, now, dt, Some(VERSUS_SMOOTH_MAX));
                e.drawn.unwrap_or(loc)[0]
            };
            if now > 7.0 {
                drawn.push(x);
                farthest = farthest.max((x - loc[0]).abs());
            }
            now += dt;
        }
        let steps: Vec<f32> = drawn.windows(2).map(|w| w[1] - w[0]).collect();
        let worst = steps.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        (worst, steps.iter().sum::<f32>() / steps.len() as f32, farthest)
    }

    #[test]
    fn other_players_drawn_smoothly_on_a_multiplayer_client() {
        // Enhanced network play, a client: the other players' copies are smoothed, within the
        // versus bound (shots are tested at the true place).
        let kind = smooth_kind(true, Some(true));
        assert_eq!(kind, SmoothKind::VersusClient);
        let (worst, mean, farthest) = versus_copy_walk(kind);
        // The true step is 2.5 units a frame; drawn without smoothing (negative control) it is
        // 10 units one frame out of four and 0 the others; smoothed within the 6-unit bound, the
        // drawn step still changes by a few units when the bound holds it back.
        assert!((mean - 2.5).abs() < 0.05, "mean step {mean}");
        assert!(worst < 5.0, "smoothed: step changes by up to {worst}");
        assert!(farthest <= VERSUS_SMOOTH_MAX + 1e-3, "drawn {farthest} from its place");
        let (raw_worst, raw_mean, _) = versus_copy_walk(SmoothKind::None);
        assert!((raw_mean - 2.5).abs() < 0.05 && raw_worst > 8.0, "unsmoothed: {raw_worst}");
        // Unchanged: Original (not enhanced) draws nothing smoothly, the listen server draws its
        // remote players, a machine without a driver draws nothing smoothly.
        assert_eq!(smooth_kind(false, Some(true)), SmoothKind::None);
        assert_eq!(smooth_kind(true, Some(false)), SmoothKind::VersusServer);
        assert_eq!(smooth_kind(true, None), SmoothKind::None);
    }
}
