//! Improved build only: a gamepad direction held in a menu repeats, as a held keyboard arrow does
//! (Windows sends WM_KEYDOWN again; the menus take each one as a new press). PC XIII reads no
//! gamepad in its menus at all: OpenXIII turns the gamepad into arrow keys (the viewer's
//! pad_keys.rs for the first player, coop.rs for the others), and without this a held stick or
//! d-pad moved by one step only.
//!
//! One place for every player's gamepad: [`stick_directions`] turns a stick into up / down / left
//! / right with a dead zone, hysteresis and its dominant axis only (a vertical hold with some
//! sideways noise stays vertical), and [`DirectionRepeat`] gives the repeats of the held
//! directions: the first after [`REPEAT_DELAY`] seconds, then one every [`REPEAT_PERIOD`].

/// Seconds a direction is held before it repeats.
pub const REPEAT_DELAY: f64 = 0.38;
/// Seconds between two repeats (about 9 per second).
pub const REPEAT_PERIOD: f64 = 0.11;
/// Stick deflection (0…1) that presses a direction, and the one under which it is released.
pub const STICK_PRESS: f32 = 0.5;
pub const STICK_RELEASE: f32 = 0.35;

/// Directions in the order up, down, left, right.
pub type Directions = [bool; 4];

/// Seconds of a steady clock (real time: a menu repeats the same whether the game is paused or not).
pub fn clock() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// The stick (`x` right, `y` down, −1…1) as directions, given those it held in the previous
/// frame: a direction is pressed past [`STICK_PRESS`] on its axis when that axis is the larger,
/// and stays held until under [`STICK_RELEASE`] or until the other axis clearly takes over.
pub fn stick_directions(x: f32, y: f32, before: Directions) -> Directions {
    let (ax, ay) = (x.abs(), y.abs());
    let vertical_held = before[0] || before[1];
    let horizontal_held = before[2] || before[3];
    // The axis held keeps the stick unless the other one is clearly larger (diagonal noise).
    let vertical = if vertical_held { ay >= STICK_RELEASE && ay * 1.5 >= ax } else { ay >= STICK_PRESS && ay >= ax };
    let horizontal = if horizontal_held { ax >= STICK_RELEASE && ax * 1.5 >= ay } else { ax >= STICK_PRESS && ax > ay };
    let (vertical, horizontal) = if vertical && horizontal {
        // Both could hold: the one already held, else the larger.
        if vertical_held || (!horizontal_held && ay >= ax) {
            (true, false)
        } else {
            (false, true)
        }
    } else {
        (vertical, horizontal)
    };
    [vertical && y < 0.0, vertical && y > 0.0, horizontal && x < 0.0, horizontal && x > 0.0]
}

/// The repeats of held directions, one gamepad.
#[derive(Clone, Debug, Default)]
pub struct DirectionRepeat {
    /// For each direction held: when the next repeat is due (seconds of the caller's clock).
    due: [Option<f64>; 4],
}

impl DirectionRepeat {
    /// The directions held at `now` (seconds, any steady clock): which of them repeat in this
    /// frame. A direction just pressed does not repeat (its press is the caller's own); a
    /// released one starts again from its press.
    pub fn update(&mut self, held: Directions, now: f64) -> Directions {
        let mut out = [false; 4];
        for i in 0..4 {
            match (held[i], self.due[i]) {
                (false, _) => self.due[i] = None,
                (true, None) => self.due[i] = Some(now + REPEAT_DELAY),
                (true, Some(t)) if now >= t => {
                    out[i] = true;
                    // A long frame gives one repeat, not a burst.
                    self.due[i] = Some((t + REPEAT_PERIOD).max(now + REPEAT_PERIOD * 0.5));
                }
                (true, Some(_)) => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Repeats of a direction held from 0 to `until` seconds, frames every `dt`.
    fn repeats(until: f64, dt: f64) -> Vec<f64> {
        let mut r = DirectionRepeat::default();
        let mut out = Vec::new();
        let mut t = 0.0;
        while t <= until + 1e-9 {
            if r.update([false, true, false, false], t)[1] {
                out.push(t);
            }
            t += dt;
        }
        out
    }

    #[test]
    fn a_held_direction_waits_then_repeats_steadily() {
        let r = repeats(1.0, 1.0 / 120.0);
        assert!(!r.is_empty() && (r[0] - REPEAT_DELAY).abs() < 0.01, "first repeat after the delay: {r:?}");
        for w in r.windows(2) {
            assert!((w[1] - w[0] - REPEAT_PERIOD).abs() < 0.01, "steady rate: {r:?}");
        }
        // 0.38 s then every 0.11 s up to 1 s: 0.38, 0.49, 0.60, 0.71, 0.82, 0.93.
        assert_eq!(r.len(), 6);
        // The same at 30 frames per second (frames of 33 ms): about the same count, no burst.
        let slow = repeats(1.0, 1.0 / 30.0);
        assert!((5..=6).contains(&slow.len()), "{slow:?}");
    }

    #[test]
    fn a_release_stops_at_once_and_a_new_press_waits_again() {
        let mut r = DirectionRepeat::default();
        assert_eq!(r.update([true, false, false, false], 0.0), [false; 4], "the press itself is not a repeat");
        assert!(r.update([true, false, false, false], 0.5)[0]);
        assert_eq!(r.update([false; 4], 0.52), [false; 4]);
        assert_eq!(r.update([true, false, false, false], 0.6), [false; 4]);
        assert_eq!(r.update([true, false, false, false], 0.9), [false; 4], "pressed again: the delay again");
        assert!(r.update([true, false, false, false], 0.99)[0]);
        // A frame of half a second gives one repeat.
        assert_eq!(r.update([true, false, false, false], 1.5), [true, false, false, false]);
        assert_eq!(r.update([true, false, false, false], 1.52), [false; 4]);
    }

    #[test]
    fn the_stick_has_a_dead_zone_hysteresis_and_one_axis() {
        let none = [false; 4];
        assert_eq!(stick_directions(0.0, 0.3, none), none, "dead zone");
        let down = stick_directions(0.1, 0.6, none);
        assert_eq!(down, [false, true, false, false]);
        assert_eq!(stick_directions(0.0, 0.4, down), down, "held down to the release threshold");
        assert_eq!(stick_directions(0.0, 0.3, down), none, "released under it");
        // Held down with sideways noise: never left or right.
        for x in [-0.55, 0.45, 0.6] {
            assert_eq!(stick_directions(x, 0.7, down), down, "x {x}");
        }
        // The other axis clearly larger takes over.
        assert_eq!(stick_directions(0.9, 0.4, down), [false, false, false, true]);
        // From rest, a diagonal goes to its larger axis.
        assert_eq!(stick_directions(-0.6, -0.8, none), [true, false, false, false]);
        assert_eq!(stick_directions(-0.8, -0.6, none), [false, false, true, false]);
    }
}
