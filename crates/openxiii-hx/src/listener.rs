//! Listeners and computation of a source's 2D settings:
//! pan, Doppler pitch, distance attenuation and "front/back".

/// Attenuation curve in dB (128 values from −100 to 0).
#[allow(unused_imports)]
use openxiii_math::SimMath as _;
const CURVE: [f32; 128] = [
    -100.0, -42.16, -36.32, -32.84, -30.35, -28.41, -26.82, -25.47, -24.31, -23.27, -22.35, -21.52, -20.75, -20.05, -19.4, -18.8, -18.23, -17.7, -17.2, -16.72,
    -16.27, -15.84, -15.43, -15.04, -14.67, -14.31, -13.96, -13.63, -13.31, -13.0, -12.71, -12.42, -12.14, -11.87, -11.6, -11.35, -11.1, -10.86, -10.63, -10.4,
    -10.17, -9.96, -9.74, -9.54, -9.34, -9.14, -8.94, -8.75, -8.57, -8.39, -8.21, -8.04, -7.86, -7.7, -7.53, -7.37, -7.21, -7.06, -6.9, -6.75, -6.6, -6.46,
    -6.32, -6.18, -6.04, -5.9, -5.77, -5.63, -5.5, -5.37, -5.25, -5.12, -5.0, -4.88, -4.76, -4.64, -4.52, -4.41, -4.29, -4.18, -4.07, -3.96, -3.85, -3.75,
    -3.64, -3.54, -3.43, -3.33, -3.23, -3.13, -3.03, -2.94, -2.84, -2.74, -2.65, -2.56, -2.47, -2.37, -2.28, -2.19, -2.11, -2.02, -1.93, -1.85, -1.76, -1.68,
    -1.59, -1.51, -1.43, -1.35, -1.27, -1.19, -1.11, -1.03, -0.95, -0.87, -0.8, -0.72, -0.65, -0.57, -0.5, -0.43, -0.35, -0.28, -0.21, -0.14, -0.07, 0.0,
];

/// Speed of sound.
const SOUND_SPEED: f32 = 331.6;

/// Listener: position, velocity, look direction and up.
#[derive(Debug, Clone, Copy)]
pub struct Listener {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub front: [f32; 3],
    pub up: [f32; 3],
}

impl Default for Listener {
    /// HX listener constructor: look (0, 0, 1), up (0, 1, 0).
    fn default() -> Self {
        Listener { pos: [0.0; 3], vel: [0.0; 3], front: [0.0, 0.0, 1.0], up: [0.0, 1.0, 0.0] }
    }
}

impl Listener {
    /// Position (vtable+4), velocity (+8), look and up (+0xC): fields copied as is, then each
    /// subscriber of the listener notified through the method of the same rank (the Rust engine recomputes on demand: nothing to notify).
    pub fn set_position(&mut self, p: [f32; 3]) {
        self.pos = p;
    }
    pub fn set_velocity(&mut self, v: [f32; 3]) {
        self.vel = v;
    }
    pub fn set_orientation(&mut self, front: [f32; 3], up: [f32; 3]) {
        (self.front, self.up) = (front, up);
    }
}

// Sound is computed on the game thread, whose FPU runs at 24 bits (D3DDrv): each
// operation rounds to single precision, in machine-code order.

/// Attenuation: `rolloff` = [disabled (byte), max distance, min distance,
/// volume beyond the max].
fn attenuation(rolloff: [f32; 4], dist: f32) -> f32 {
    if rolloff[0].to_bits() as u8 != 0 {
        return 0.0;
    }
    let (max, min, far) = (rolloff[1], rolloff[2], rolloff[3]);
    // fcomp: min < dist (C0, NaN included) to continue.
    if !(min < dist || dist.is_nan()) {
        return 0.0;
    }
    // max ≤ dist (C0 or C3 alone): far volume; NaN carries on with the computation.
    if max <= dist {
        return far;
    }
    // First step of the curve above the far volume.
    let mut i0 = 0x7f;
    for i in 0..0x7f {
        if far < CURVE[i + 1] {
            i0 = i;
            break;
        }
    }
    // ((max − dist) / (max − min)) × (0x7F − i0) (FIMUL), _ftol, then the curve.
    let t = ((max - dist) / (max - min)) * (0x7f - i0) as f32;
    let i = (t as i32 + i0 as i32).clamp(0, 127) as usize;
    CURVE[i]
}

/// Source − listener offset and distance as (Z² + Y²) + X².
fn offset(l: &Listener, pos: [f32; 3]) -> ([f32; 3], f32) {
    let d = [pos[0] - l.pos[0], pos[1] - l.pos[1], pos[2] - l.pos[2]];
    (d, ((d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]).sqrt())
}

/// Settings for one listener: [pan, pitch, volume, front].
fn single(l: &Listener, pos: [f32; 3], vel: [f32; 3], rolloff: [f32; 4], pan_coeff: f32) -> [f32; 4] {
    // Attenuation distance: (Y² + X²) + Z².
    let d = [pos[0] - l.pos[0], pos[1] - l.pos[1], pos[2] - l.pos[2]];
    let dist = ((d[1] * d[1] + d[0] * d[0]) + d[2] * d[2]).sqrt();
    let volume = attenuation(rolloff, dist);
    let (f, u) = (l.front, l.up);
    let (d, dist) = offset(l, pos);
    // Pan: side from look × up, ((dX·a + dZ·c) + dY·b) / dist.
    let pan = if dist == 0.0 {
        0.5
    } else {
        let a = u[2] * f[1] - u[1] * f[2];
        let b = f[2] * u[0] - u[2] * f[0];
        let c = u[1] * f[0] - f[1] * u[0];
        let side = (d[0] * a + d[2] * c) + d[1] * b;
        ((side / dist) * pan_coeff + 1.0) * 0.5
    };
    // Front / back.
    let front = if dist == 0.0 { 0.5 } else { (((d[2] * f[2] + d[1] * f[1]) + d[0] * f[0]) / dist + 1.0) * 0.5 };
    // Doppler.
    let pitch = if dist == 0.0 {
        1.0
    } else {
        let c = dist * SOUND_SPEED;
        let lv = l.vel;
        let ll = (d[2] * lv[2] + d[1] * lv[1]) + d[0] * lv[0];
        let ls = (d[2] * vel[2] + d[1] * vel[1]) + d[0] * vel[0];
        (c - ll) / (c - ls)
    };
    [pan, pitch, volume, front]
}

/// e^v as in the machine code (FLDL2E, FMUL, FRNDINT, F2XM1, FLD1 + FADDP, FSCALE): only the
/// multiplication, subtraction and addition are rounded to 24 bits.
pub(crate) fn exp_x87(v: f32) -> f32 {
    exp_x87_stack(v) as f32
}

/// e^v as it stays on the x87 stack after FSCALE: 24-bit mantissa, but extended
/// exponent (no single-precision denormal as long as it is not stored).
fn exp_x87_stack(v: f32) -> f64 {
    let t = (v as f64 * std::f64::consts::LOG2_E) as f32;
    let i = t.round_ties_even();
    let frac = t - i;
    let m = (1.0 + (2f64.dpowf(frac as f64) - 1.0)) as f32;
    m as f64 * 2f64.dpowf(i as f64)
}

/// Round-to-nearest-even of `s + e` (exact sum, `e` small relative to `s`) to a 24-bit
/// mantissa, unbounded exponent: an x87 result in 24-bit precision before storing.
pub(crate) fn rounded24(s: f64, e: f64) -> f64 {
    if s == 0.0 || !s.is_finite() {
        return s;
    }
    let b = s.to_bits();
    let rest = b & ((1 << 29) - 1);
    let mut high = b - rest;
    let half_value = 1u64 << 28;
    let rises = rest > half_value
        || (rest == half_value && (e > 0.0) == (s > 0.0) && e != 0.0)
        || (rest == half_value && e == 0.0 && high & (1 << 29) != 0);
    if rises {
        high += 1 << 29;
    }
    f64::from_bits(high)
}

/// Sum on the x87 stack (24 bits, extended exponent).
pub(crate) fn sum24(a: f64, b: f64) -> f64 {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    rounded24(s, e)
}

/// Product on the x87 stack (24 bits, extended exponent) of two 24-bit values: exact in f64.
pub(crate) fn product24(a: f64, b: f64) -> f64 {
    rounded24(a * b, 0.0)
}

/// Time of a micro: RDTSC (high × 2^32 + low, signed words raised by
/// 2^32) × GSecondsPerCycle, each operation rounded to 24 bits (the game's FPU).
pub fn micro_time(tsc: u64, gspc: f64) -> f64 {
    let (high, low_value) = ((tsc >> 32) as u32, tsc as u32);
    // Exact FILD, then + 2^32 (FADD, 24 bits) for a negative word.
    let word_value = |m: u32| if (m as i32) < 0 { rounded24(m as f64, 0.0) } else { m as f64 };
    // × 2^32 (FMUL, 24 bits) then + low (FADDP, 24 bits).
    let t = sum24(rounded24(word_value(high), 0.0) * 4294967296.0, word_value(low_value));
    let p = t * gspc;
    rounded24(p, t.mul_add(gspc, -p))
}

/// Velocity of a micro (UpdateMicro): dt = time − previous (24 bits),
/// previous += dt (24 bits, stored as double), dt clamped to [1e-4, 1] (NaN → 1), then
/// (position − old) × (1/dt).
pub fn micro_speed(now: f64, last: &mut f64, pos: [f32; 3], old: [f32; 3]) -> [f32; 3] {
    let dt = sum24(now, -*last);
    *last = sum24(dt, *last);
    let mini = 1e-4f32 as f64;
    let dt = if dt < mini {
        mini
    } else if dt < 1.0 {
        dt
    } else {
        1.0
    };
    let q = 1.0 / dt;
    let k = rounded24(q, (-q).mul_add(dt, 1.0));
    let d = [pos[0] - old[0], pos[1] - old[1], pos[2] - old[2]];
    [product24(d[0] as f64, k) as f32, product24(d[1] as f64, k) as f32, product24(d[2] as f64, k) as f32]
}

/// Frame of the micro (UpdateMicro) from the Deprojects of (0, 0, 0, 1),
/// (0, −1, 0, 1) and (1, 0, 0, 1): U = P1 − P0, W = P2 − P0, look = W × U, U and look normalised
/// (nothing below 1e-8 or for NaN, otherwise × appRSqrt), up = −U.
pub fn frame_micro(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let mut u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let w = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let mut c = [w[1] * u[2] - u[1] * w[2], w[2] * u[0] - w[0] * u[2], w[0] * u[1] - w[1] * u[0]];
    let normalized = |v: &mut [f32; 3]| {
        // Sum kept on the stack (24 bits, extended exponent) and compared in double.
        let x = [v[0] as f64, v[1] as f64, v[2] as f64];
        let sq = sum24(sum24(product24(x[0], x[0]), product24(x[1], x[1])), product24(x[2], x[2]));
        if !(sq >= 1e-8) {
            return;
        }
        // appRSqrt of the square stored in single: FSQRT then 1/x on 24 bits.
        let k = 1.0 / (sq as f32).sqrt();
        *v = [k * v[0], k * v[1], k * v[2]];
    };
    normalized(&mut u);
    normalized(&mut c);
    (c, [-u[0], -u[1], -u[2]])
}

/// A source's settings for all listeners: average weighted by e^(dB).
pub fn params(listeners: &[Listener], pos: [f32; 3], vel: [f32; 3], rolloff: [f32; 4], pan_coeff: f32) -> [f32; 4] {
    match listeners.len() {
        0 => [0.5, 1.0, 0.0, 0.5],
        1 => single(&listeners[0], pos, vel, rolloff, pan_coeff),
        _ => {
            // The weight e^volume stays on the stack: sums and products
            // rounded to 24 bits with an extended exponent, then stored in single precision (denormals
            // below −87 dB). 1/sum also stays on the stack: finite even when the
            // stored sum is denormal.
            let mut loudest = -100.0f32;
            let (mut wsum, mut pan, mut front, mut pitch) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for l in listeners {
                let p = single(l, pos, vel, rolloff, pan_coeff);
                if loudest < p[2] {
                    loudest = p[2];
                }
                let w = exp_x87_stack(p[2]);
                wsum = sum24(wsum as f64, w) as f32;
                pan = sum24(product24(w, p[0] as f64), pan as f64) as f32;
                front = sum24(product24(w, p[3] as f64), front as f64) as f32;
                pitch = sum24(product24(w, p[1] as f64), pitch as f64) as f32;
            }
            if wsum != 0.0 {
                // Quotient rounded to 24 bits: the exact remainder 1 − q·sum breaks ties.
                let q = 1.0 / wsum as f64;
                let k = rounded24(q, (-q).mul_add(wsum as f64, 1.0));
                [product24(pan as f64, k) as f32, product24(k, pitch as f64) as f32, loudest, product24(front as f64, k) as f32]
            } else {
                [0.5, 1.0, loudest, 0.5]
            }
        }
    }
}

