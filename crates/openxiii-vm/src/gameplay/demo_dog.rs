//! The game has neither pawn nor controller for it: this is not the original's code. An Effects actor
//! in DT_Mesh plays the sequences in a logical order, with their sounds, and moves by sweeping a dog
//! box against the scenery (TRACE_World) and staying on the ground. Nothing is called without the
//! variable: the game stays unchanged.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::physics::{CheckResult, V, TRACE_WORLD};
use crate::value::{ObjId, Value};
use crate::Vm;

/// Half-size of the dog's box (length, width, height).
const BODY: V = [30.0, 30.0, 25.0];
/// Height of the mesh origin above the ground (ORIGIN Z of Dog.uc).
const HEIGHT: f32 = 37.767;

#[derive(Clone, Copy)]
enum Move {
    /// In place.
    Still,
    /// Towards the retreat point.
    Away(f32),
    /// Towards the player, up to the given distance.
    Toward(f32, f32),
    /// Backwards (back to the movement), at the given speed.
    Back(f32),
    /// Gradual turn to face the player.
    Face,
}

struct Step {
    seq: &'static str,
    looping: bool,
    /// Duration (seconds); for a movement, maximum duration.
    time: f32,
    moving: Move,
    sound: Option<&'static str>,
    text: &'static str,
}

const fn st(seq: &'static str, looping: bool, time: f32, moving: Move, sound: Option<&'static str>, text: &'static str) -> Step {
    Step { seq, looping, time, moving, sound, text }
}

/// Frames / 30 (frame rate of the DogA sequences).
const SCRIPT: &[Step] = &[
    st("coucherelax", true, 4.0, Move::Still, None, "Dog: lying down, relaxed"),
    st("couche", false, 101.0 / 30.0, Move::Still, None, "Dog: lying down"),
    st("assie", false, 12.0 / 30.0, Move::Still, None, "Dog: sits down"),
    st("assie200", false, 201.0 / 30.0, Move::Still, None, "Dog: sitting"),
    st("WalkSearch", true, 6.0, Move::Away(90.0), None, "Dog: walks sniffing"),
    st("TourneG", false, 8.0 / 30.0, Move::Face, None, "Dog: turns"),
    st("Demitr", false, 16.0 / 30.0, Move::Face, None, "Dog: turns around"),
    st("PietineG", false, 16.0 / 30.0, Move::Face, Some("Dog__hDogGrowl1"), "Dog: growls"),
    st("PietineG", false, 16.0 / 30.0, Move::Face, None, "Dog: growls"),
    st("Walk", true, 2.5, Move::Toward(140.0, 450.0), None, "Dog: walks"),
    st("Run", true, 6.0, Move::Toward(520.0, 260.0), Some("Dog__hDogBark1"), "Dog: runs and barks"),
    st("Jump", false, 21.0 / 30.0, Move::Toward(300.0, 90.0), Some("Dog__hDogLunge1"), "Dog: leaps"),
    st("Attak", false, 21.0 / 30.0, Move::Still, Some("Dog__hDogBit1"), "Dog: bites"),
    st("AttakBackNeu", false, 39.0 / 30.0, Move::Back(120.0), Some("Dog__hDogGrowl1"), "Dog: backs off"),
    st("PietineG", false, 16.0 / 30.0, Move::Face, Some("Dog__hDogYelp1"), "Dog: hit, whimpers"),
    st("Death", false, 71.0 / 30.0 + 2.0, Move::Still, Some("Dog__hDogDieA1"), "Dog: dies (death 1)"),
    st("assie", false, 12.0 / 30.0, Move::Still, None, "Dog: comes back"),
    st("PietineG", false, 16.0 / 30.0, Move::Face, Some("Dog__hDogYelp1"), "Dog: hit, whimpers"),
    st("death2", false, 51.0 / 30.0 + 2.0, Move::Still, Some("Dog__hDogDieB1"), "Dog: dies (death 2)"),
];

pub struct DemoDog {
    dog: ObjId,
    pawn: ObjId,
    home: V,
    away: V,
    step: usize,
    t: f32,
}

impl Vm {
    /// Sweep of a box from `from` to `to` against the scenery: free fraction of the path.
    pub(crate) fn demo_sweep(&self, from: V, to: V, extent: V) -> f32 {
        let mut h = CheckResult::new(1.0);
        self.single_line_check(&mut h, None, to, from, TRACE_WORLD, extent);
        h.time
    }

    /// Nearly flat ground under `p` (less than `depth` lower): its height.
    pub(crate) fn demo_floor(&self, p: V, depth: f32) -> Option<f32> {
        let mut h = CheckResult::new(1.0);
        self.single_line_check(&mut h, None, [p[0], p[1], p[2] - depth], p, TRACE_WORLD, [0.0; 3]);
        (h.time < 1.0 && h.normal[2] >= 0.7).then_some(p[2] - h.time * depth)
    }

    /// Free point on the ground at `dist` from `from` in direction `angle` (as far as possible),
    /// the dog's box passing from `from` to it.
    fn demo_free(&self, from: V, angle: f32, dist: f32) -> Option<(V, f32)> {
        let to = [from[0] + dist * angle.dcos(), from[1] + dist * angle.dsin(), from[2]];
        let t = self.demo_sweep(from, to, BODY);
        let d = (dist * t - BODY[0]).max(0.0);
        let p = [from[0] + d * angle.dcos(), from[1] + d * angle.dsin(), from[2]];
        self.demo_floor([p[0], p[1], p[2] + 30.0], 200.0).map(|z| ([p[0], p[1], z + HEIGHT], d))
    }

    fn demo_play(&mut self, a: ObjId, seq: &str, looping: bool) {
        let name = if looping { "LoopAnim" } else { "PlayAnim" };
        if let Some(f) = self.find_function(a, name, false) {
            let _ = self.call(f, a, vec![Some(Value::Name(crate::value::NameStruct::from_time(seq))), Some(Value::Float(1.0)), Some(Value::Float(0.15))]);
        }
    }

    fn demo_sound(&mut self, a: ObjId, sound: &str) {
        if let Some(f) = self.find_function(a, "PlaySound", false) {
            let _ = self.call(f, a, vec![Some(Value::Asset(format!("XIIISound.PNJ__Dog.{sound}")))]);
        }
    }

    pub(crate) fn demo_message(&mut self, pawn: ObjId, text: &str) {
        let Some(pc) = self.propr(pawn, "Controller").as_object() else { return };
        if let Some(f) = self.find_function(pc, "ClientMessage", false) {
            let _ = self.call(f, pc, vec![Some(Value::Str(text.into()))]);
        }
    }

    fn demo_yaw(&mut self, a: ObjId, yaw: f32) {
        let y = (yaw * 32768.0 / std::f32::consts::PI) as i32 & 0xFFFF;
        self.set_prop(a, "Rotation", Value::rotator(0, y, 0));
    }

    fn demo_place(&mut self, a: ObjId, p: V) {
        self.set_prop(a, "Location", Value::vector(p[0], p[1], p[2]));
    }
}

impl DemoDog {
    /// Dog placed at a free spot in front of `pawn` (failing that, on the sides or behind), with a
    /// free retreat point on the player → dog axis.
    pub fn new(vm: &mut Vm, pawn: ObjId) -> Option<DemoDog> {
        let class = vm.class("Effects")?;
        let base = vm.native_prop(pawn, crate::NativeField::Location).as_vector();
        let facing = vm.propr(pawn, "Rotation").as_rotator()[1] as f32 * std::f32::consts::PI / 32768.0;
        let mut best: Option<(V, V, f32)> = None;
        for k in 0..12 {
            let off = (k + 1) / 2 * if k % 2 == 1 { 1 } else { -1 };
            let angle = facing + off as f32 * std::f32::consts::TAU / 12.0;
            let Some((home, d)) = vm.demo_free(base, angle, 350.0) else { continue };
            if d < 200.0 {
                continue;
            }
            let Some((away, run)) = vm.demo_free(home, angle, 700.0) else { continue };
            if best.is_none_or(|b| run > b.2) {
                best = Some((home, away, run));
            }
            if run >= 450.0 {
                break;
            }
        }
        let (home, away, _) = best?;
        let dog = vm.spawn_actor(class, Value::Object(None), None, Value::vector(home[0], home[1], home[2]), Value::rotator(0, 0, 0), Value::Object(None), true).ok()??;
        vm.set_prop(dog, "DrawType", Value::Byte(2));
        vm.set_prop(dog, "Mesh", Value::Asset("XIIIPersos.DogM".into()));
        vm.set_prop(dog, "bUnlit", Value::Bool(false));
        vm.set_prop(dog, "bNetTemporary", Value::Bool(false));
        let mut demo = DemoDog { dog, pawn, home, away, step: SCRIPT.len() - 1, t: f32::MAX };
        demo.next(vm);
        Some(demo)
    }

    fn next(&mut self, vm: &mut Vm) {
        self.step = (self.step + 1) % SCRIPT.len();
        self.t = 0.0;
        if self.step == 0 {
            // Start of a round: the dog goes back to its spot, facing the player.
            vm.demo_place(self.dog, self.home);
            let p = vm.native_prop(self.pawn, crate::NativeField::Location).as_vector();
            vm.demo_yaw(self.dog, (p[1] - self.home[1]).datan2(p[0] - self.home[0]));
        }
        let s = &SCRIPT[self.step];
        vm.demo_play(self.dog, s.seq, s.looping);
        if let Some(sound) = s.sound {
            vm.demo_sound(self.dog, sound);
        }
        vm.demo_message(self.pawn, s.text);
        println!("{}", s.text);
    }

    /// One frame: movement of the step (swept against the scenery, kept on the ground), then next
    /// step at the end of its duration or when the dog has arrived or is blocked.
    pub fn tick(&mut self, vm: &mut Vm, dt: f32) {
        if vm.objects.get(self.dog).is_none_or(|o| o.is_none()) {
            return;
        }
        self.t += dt;
        let s = &SCRIPT[self.step];
        let here = vm.native_prop(self.dog, crate::NativeField::Location).as_vector();
        let player = vm.propr(self.pawn, "Location").as_vector();
        let to_player = (player[1] - here[1]).datan2(player[0] - here[0]);
        let dist = (player[0] - here[0]).dhypot(player[1] - here[1]);
        let mut done = self.t >= s.time;
        let (speed, dir, look) = match s.moving {
            Move::Still => (0.0, 0.0, None),
            Move::Away(v) => {
                let d = (self.away[1] - here[1]).datan2(self.away[0] - here[0]);
                if (self.away[0] - here[0]).dhypot(self.away[1] - here[1]) < 20.0 {
                    done = true;
                }
                (v, d, Some(d))
            }
            Move::Toward(v, stop) => {
                if dist <= stop {
                    done = true;
                }
                (v, to_player, Some(to_player))
            }
            Move::Back(v) => (v, to_player + std::f32::consts::PI, Some(to_player)),
            Move::Face => {
                let cur = vm.propr(self.dog, "Rotation").as_rotator()[1] as f32 * std::f32::consts::PI / 32768.0;
                let diff = (to_player - cur).dsin().datan2((to_player - cur).dcos());
                let turn = (std::f32::consts::PI * dt / s.time.max(0.2)).min(diff.abs()) * diff.signum();
                (0.0, 0.0, Some(cur + turn))
            }
        };
        if let Some(y) = look {
            vm.demo_yaw(self.dog, y);
        }
        if speed > 0.0 && !done {
            let step = speed * dt;
            let to = [here[0] + step * dir.dcos(), here[1] + step * dir.dsin(), here[2]];
            let free = vm.demo_sweep(here, to, BODY);
            let moved = [here[0] + (to[0] - here[0]) * free * 0.9, here[1] + (to[1] - here[1]) * free * 0.9, here[2]];
            match vm.demo_floor([moved[0], moved[1], moved[2] + 30.0], 120.0) {
                Some(z) => vm.demo_place(self.dog, [moved[0], moved[1], z + HEIGHT]),
                // No ground (edge, hole): it stays where it is.
                None => done = true,
            }
            if free < 1.0 {
                done = true;
            }
        }
        if done {
            self.next(vm);
        }
    }
}
