//! Main loop of the game in a window: MainLoop of XIII.exe (body,
//! "UpdateWorld", "EnforceTickRate", "MessagePump", "ThrottleEditor" of UnEngineWin.h). The
//! dedicated server has its own (server.rs, Engine's UServerCommandlet::Main).
//!
//! Each iteration: frame duration from the rdtsc counter (low word only) × GSecondsPerCycle,
//! unbounded; if the engine is allowed not to be updated (UGameEngine::AllowBeingNotTicked, true in
//! single-player), the foreground window must belong to the game thread and not be minimized: otherwise neither
//! GEngine->Tick nor sound (PauseSound(0xFFF, 0) on loss, ResumeSound(0xFFF, 0) on return); then
//! GWindowManager->Tick, CurrentTickRate (UEngine +0xB8) recomputed after one second of accumulated
//! durations, wait until 1 / GetMaxTickRate (120 in single-player) since the start of the frame, messages.
//!
//! The FPU is at 24-bit precision (Direct3D): each product, sum and quotient is rounded once
//! to the mantissa of a single-precision float.

/// (32-bit unsigned word) × GSecondsPerCycle: fild of a 64-bit integer (exact) then fmul
/// rounded only once to 24 bits (to nearest, ties to even), stored as a single-precision float.
pub fn cycles_f32(cycles: u32, seconds_per_cycle: f64) -> f32 {
    if cycles == 0 || seconds_per_cycle == 0.0 || !seconds_per_cycle.is_finite() {
        return (cycles as f64 * seconds_per_cycle) as f32;
    }
    // seconds_per_cycle = ± m × 2^e, m an integer; the exact product c × m fits in 85 bits.
    let bits = seconds_per_cycle.to_bits();
    let neg = bits >> 63 != 0;
    let exp = ((bits >> 52) & 0x7FF) as i32;
    let (m, e) = if exp == 0 { (bits & ((1 << 52) - 1), -1074) } else { ((bits & ((1 << 52) - 1)) | 1 << 52, exp - 1075) };
    let p = cycles as u128 * m as u128;
    let n = 128 - p.leading_zeros() as i32;
    let (mut q, mut sh) = if n > 24 {
        let s = n - 24;
        let q = p >> s;
        let rest = p & ((1u128 << s) - 1);
        let half_value = 1u128 << (s - 1);
        (if rest > half_value || (rest == half_value && q & 1 == 1) { q + 1 } else { q }, s)
    } else {
        (p, 0)
    };
    if q >> 24 != 0 {
        q >>= 1;
        sh += 1;
    }
    // q < 2^24 and the exponent stays within single-precision range for any play time: exact.
    let v = (q as f64 * 2f64.powi(e + sh)) as f32;
    if neg {
        -v
    } else {
        v
    }
}

/// UGameEngine::AllowBeingNotTicked: true without a pending level (GPendingLevel,
/// +0x1B8) or a network driver for the level (GLevel +0x3C); so always true in single-player.
pub fn allow_being_not_ticked(pending_level: bool, level_net_driver: bool) -> bool {
    !pending_level && !level_net_driver
}

/// What the window asks for at the start of a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// GEngine->Tick(delta) (vtable +0x6C).
    pub tick: bool,
    /// Audio->PauseSound(0xFFF, 0) (vtable +0xA8): the window has just lost the foreground.
    pub pause_sound: bool,
    /// Audio->ResumeSound(0xFFF, 0) (vtable +0xAC): it has just regained it.
    pub resume_sound: bool,
    /// GUnIsCdAvailable->TestCdCooperativeLevel (vtable +8), when the object exists (InstallType non-zero).
    pub cd_check: bool,
}

/// MainLoop state: OldTime (rdtsc of the previous frame), seconds and frames counted since the
/// last CurrentTickRate, and the foreground state of the previous frame (1 at launch,
/// kept from one MainLoop call to the next).
#[derive(Clone, Debug)]
pub struct ClientLoop {
    pub old_time: u32,
    pub new_time: u32,
    pub seconds: f32,
    pub tick_count: i32,
    pub had_focus: bool,
}

impl ClientLoop {
    /// Entering the loop: OldTime = rdtsc, counters at zero.
    pub fn new(start: u32, had_focus: bool) -> Self {
        ClientLoop { old_time: start, new_time: start, seconds: 0.0, tick_count: 0, had_focus }
    }

    /// Start of a frame at counter `now`: DeltaTime duration given to Tick and added to the counted seconds.
    pub fn delta(&mut self, now: u32, seconds_per_cycle: f64) -> f32 {
        self.new_time = now;
        let delta = cycles_f32(now.wrapping_sub(self.old_time), seconds_per_cycle);
        self.seconds += delta;
        delta
    }

    /// World update: `allow_not_ticked` (AllowBeingNotTicked, asked only outside the editor)
    /// then `focused` (foreground owned by the game thread and not minimized, asked only if needed);
    /// `audio`: GEngine->Audio non-null.
    pub fn frame(&mut self, editor: bool, allow_not_ticked: impl FnOnce() -> bool, focused: impl FnOnce() -> bool, audio: bool) -> Frame {
        if editor || !allow_not_ticked() {
            return Frame { tick: true, ..Frame::default() };
        }
        let focus = focused();
        let f = Frame {
            tick: focus,
            pause_sound: audio && self.had_focus && !focus,
            resume_sound: audio && !self.had_focus && focus,
            cd_check: true,
        };
        self.had_focus = focus;
        f
    }

    /// After Tick: OldTime = NewTime, one more frame; past 1.0 s accumulated,
    /// CurrentTickRate = frames / seconds and reset. Returns the new CurrentTickRate.
    pub fn after_tick(&mut self) -> Option<f32> {
        self.old_time = self.new_time;
        self.tick_count += 1;
        if self.seconds > 1.0 {
            let rate = self.tick_count as f32 / self.seconds;
            self.seconds = 0.0;
            self.tick_count = 0;
            return Some(rate);
        }
        None
    }

    /// EnforceTickRate: MaxTickRate = 1.0 / GetMaxTickRate(); time spent since NewTime at counter
    /// `now`; appSleep(MaxTickRate − spent) if MaxTickRate is larger (nothing if GetMaxTickRate is
    /// negative or NaN; an infinite wait if it is zero).
    pub fn sleep_seconds(&self, max_tick_rate: f32, now: u32, seconds_per_cycle: f64) -> Option<f32> {
        let period = 1.0f32 / max_tick_rate;
        let spent = cycles_f32(now.wrapping_sub(self.new_time), seconds_per_cycle);
        (period > spent).then(|| period - spent)
    }
}

