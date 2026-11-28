//! Force feedback (rumble): UForceFeedbackManager / UForceFeedbackClient / UForceFeedbackEffect
//! of Engine.dll, and the "platform" part missing on PC.
//!
//! In the original, the natives of ForceFeedbackController (execEnableForceFeedback,
//! execIsForceFeedbackEnable, execStartEffect) pass the request to
//! UForceFeedbackManager::Instance (vtable +0x78, +0x7C, +0x74) with the player's index Player+0xA8.
//! On PC, Instance is never created (no ForceFeedbackManager in XIII.ini, no DirectInput effect
//! code in the DLLs): IsForceFeedbackEnable returns false and XIIIPlayerController.RumbleFX does
//! nothing. `Vm::force_feedback` set to `None` reproduces exactly this case; it is the default state.
//!
//! With a manager (`Vm::enable_vibrations`, requested by the display layer when a gamepad is
//! connected), the shared part is reimplemented as is: four clients (Manager+0x2C…0x38), each
//! with four effect slots (Client+0x4…0x10) and the switch Client+0x14 (1 at
//! construction). Creating an effect (Client vtable +0x10) and its Tick (Effect vtable
//! +0) are pure virtual functions (_purecall) on PC: they only existed in the
//! console versions. They are taken here from the Xbox version of the game (where
//! the effect is UXboxForceFeedbackEffect and the client UXboxForceFeedbackClient), the console
//! version closest to the PC one:
//! - creation: only effect 0 exists (the one the scripts use), with the parameters
//!   StartEffect(effect, motor1, motor2, t1, amplitude1, t2, amplitude2, t3);
//! - effect Tick: a piecewise envelope (rising from 0 to amplitude1 at t1, then
//!   to amplitude2 at t2, then down to 0 at t3, finished afterwards) times each motor's strength;
//! - client Tick: after the shared Tick, the strengths of the four slots are ADDED
//!   (not the strongest one) and capped at 1.0, per motor; zero when the client's switch is off.
//! The Xbox code runs on the x87 unit in double precision: the computations are done in f64 and
//! rounded to f32 where the original stores them.

/// UForceFeedbackEffect (0xC bytes) and its creation parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Effect {
    /// Effect+0x4: elapsed time (GetElapsedTime / SetElapsedTime).
    pub elapsed_value: f32,
    /// Effect+0x8: state (SetState): 0 new, 1 running, 2 finished.
    pub state_value: i32,
    /// Motor 1 (left, large motor) and motor 2 (right, small motor).
    pub engines: [f32; 2],
    /// t1, amplitude1, t2, amplitude2, t3.
    pub wrapper: [f32; 5],
    /// Current strength of each motor, computed by the last `tick` (Xbox effect +0x28 / +0x2C).
    pub force: [f32; 2],
}

impl Effect {
    /// Creation (Client vtable +0x10, platform): effect 0 only, otherwise none.
    fn create(effect: i32, m1: f32, m2: f32, p: [f32; 5]) -> Option<Effect> {
        (effect == 0).then_some(Effect { elapsed_value: 0.0, state_value: 0, engines: [m1, m2], wrapper: p, force: [0.0; 2] })
    }

    /// Envelope level at time `t` (x87 in double precision); None at t3 and after,
    /// and when a comparison fails (NaN).
    pub fn amplitude(&self, t: f32) -> Option<f64> {
        let [t1, a1, t2, a2, t3] = self.wrapper.map(f64::from);
        let t = f64::from(t);
        if t < t1 {
            Some(a1 / t1 * t)
        } else if t < t2 {
            Some(a2 - (a2 - a1) * (t2 - t) / (t2 - t1))
        } else if t < t3 {
            Some((1.0 - (t - t2) / (t3 - t2)) * a2)
        } else {
            None
        }
    }

    /// Tick of the effect (Effect vtable +0, platform): the strengths are reset,
    /// then motor strength = level × motor; state 2 (strengths left at zero) from t3 on.
    fn tick(&mut self, _dt: f32) {
        self.force = [0.0; 2];
        match self.amplitude(self.elapsed_value) {
            Some(a) => self.force = [(a * f64::from(self.engines[0])) as f32, (a * f64::from(self.engines[1])) as f32],
            None => self.state_value = 2,
        }
    }
}

/// UForceFeedbackClient (0x18 bytes, constructor).
#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    /// Client+0x4…0x10.
    pub effects: [Option<Effect>; 4],
    /// Client+0x14 (integer: the value passed is kept as is).
    pub active_value: i32,
    /// Strength of each motor after the last Tick (Xbox client +0x18 / +0x1C).
    pub motors: [f32; 2],
}

impl Default for Client {
    fn default() -> Self {
        Client { effects: [None; 4], active_value: 1, motors: [0.0; 2] }
    }
}

impl Client {
    /// UForceFeedbackClient::EnableForceFeedback.
    pub fn enable(&mut self, b: i32) {
        self.active_value = b;
    }

    /// UForceFeedbackClient::StartEffect: creates the effect (vtable +0x10), stores it in the
    /// first free slot, or frees it if all four are taken. The switch is not read.
    pub fn start_effect(&mut self, effect_value: i32, m1: f32, m2: f32, p: [f32; 5]) {
        let Some(e) = Effect::create(effect_value, m1, m2, p) else { return };
        if let Some(place) = self.effects.iter_mut().find(|s| s.is_none()) {
            *place = Some(e);
        }
    }

    /// UForceFeedbackClient::Tick: for each slot, a finished effect (state 2) is
    /// freed; otherwise a new effect moves to state 1, receives its Tick, then its time advances by `dt`.
    pub fn tick(&mut self, dt: f32) {
        for place in &mut self.effects {
            let Some(e) = place else { continue };
            if e.state_value == 2 {
                *place = None;
                continue;
            }
            if e.state_value == 0 {
                e.state_value = 1;
            }
            e.tick(dt);
            e.elapsed_value += dt;
        }
        self.motors = self.mix();
    }

    /// End of the Xbox client Tick: the strengths of the slots added up per motor and
    /// capped at 1.0 (zero when the switch is off). Motor 1 is summed on the x87 stack (one
    /// rounding at the end), motor 2 in memory (rounded to f32 after each addition); the first
    /// slot's strengths are taken as they are (−0.0 stays −0.0), the other slots are added.
    fn mix(&self) -> [f32; 2] {
        if self.active_value == 0 {
            return [0.0; 2];
        }
        let (mut m1, mut m2) = (0.0f64, 0.0f32);
        for (k, e) in self.effects.iter().enumerate() {
            let Some(e) = e else { continue };
            if k == 0 {
                m1 = f64::from(e.force[0]);
                m2 = e.force[1];
            } else {
                m1 += f64::from(e.force[0]);
                m2 = (f64::from(m2) + f64::from(e.force[1])) as f32;
            }
        }
        let m1 = m1 as f32;
        [if m1 < 1.0 { m1 } else { 1.0 }, if m2 < 1.0 { m2 } else { 1.0 }]
    }

    /// UForceFeedbackClient::StopAllEffect.
    pub fn stop_all(&mut self) {
        self.effects = [None; 4];
    }

    /// Strength of each motor to send to the gamepad: the Xbox client's motors (`mix`), never below
    /// zero for the gamepad's gain.
    pub fn force(&self) -> [f32; 2] {
        self.motors.map(|m| m.max(0.0))
    }
}

/// UForceFeedbackManager (Manager+0x2C…0x38: four clients).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Manager {
    pub clients: [Client; 4],
}

impl Manager {
    /// UForceFeedbackManager::StartEffect: client 0…3, effect 0…9, otherwise nothing.
    pub fn start_effect(&mut self, client: i32, effect_value: i32, m1: f32, m2: f32, p: [f32; 5]) {
        if !(0..4).contains(&client) || !(0..10).contains(&effect_value) {
            return;
        }
        self.clients[client as usize].start_effect(effect_value, m1, m2, p);
    }

    /// UForceFeedbackManager::EnableForceFeedback.
    pub fn enable(&mut self, client: i32, b: i32) {
        if let Some(c) = usize::try_from(client).ok().and_then(|i| self.clients.get_mut(i)) {
            c.enable(b);
        }
    }

    /// UForceFeedbackManager::IsForceFeedbackEnable: Client+0x14, false outside 0…3.
    pub fn is_enable(&self, client: i32) -> i32 {
        usize::try_from(client).ok().and_then(|i| self.clients.get(i)).map_or(0, |c| c.active_value)
    }

    /// UForceFeedbackManager::Tick: Tick of each client.
    pub fn tick(&mut self, dt: f32) {
        for c in &mut self.clients {
            c.tick(dt);
        }
    }
}

/// Client index of a player: Player+0xA8, which only WinDrv's viewport copies write;
/// it stays at 0 for the single-player player.
pub(crate) const CLIENT_PLAYER: i32 = 0;

impl crate::Vm {
    /// Creates the rumble manager (absent from the original PC game): the scripts' RumbleFX then works.
    pub fn enable_vibrations(&mut self) {
        if self.force_feedback.is_none() {
            self.force_feedback = Some(Manager::default());
        }
    }

    /// Advances the rumble by `dt` seconds (UForceFeedbackManager::Tick) and returns the strength of both
    /// motors for each client; None without a manager.
    pub fn vibrations_tick(&mut self, dt: f32) -> Option<[[f32; 2]; 4]> {
        let m = self.force_feedback.as_mut()?;
        m.tick(dt);
        Some(std::array::from_fn(|i| m.clients[i].force()))
    }
}

