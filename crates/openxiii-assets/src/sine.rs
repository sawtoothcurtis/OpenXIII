//! USinusModifier: waving of a StaticMesh's vertices. When a section's material is a
//! SinusModifier (IsA), Engine.dll goes through SetSinusVertexStreams and D3DDrv
//! copies the vertex stream while moving each local position by a sine from
//! GMath's table; normals and coordinates are copied as is.

// Negated comparisons and bounds written as in the machine code (FCOM: NaN included).
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_clamp)]

use crate::props::Props;

/// SinusModifier properties (Amplitude +0x54, WaveLenghtX/Y/Z +0x60/+0x6C/+0x78,
/// FreqOsci +0x84, AmplitudeFadeX +0x90, RndAmplitude +0x94, RndAmpSpeed +0xA0, RndTimeVar
/// +0xA4, RndTimeSpeed +0xA8).
#[derive(Debug, Clone)]
pub struct SineParams {
    pub amplitude: [f32; 3],
    pub wave: [[f32; 3]; 3],
    pub freq: [f32; 3],
    pub fade: u8,
    pub rnd_amplitude: [f32; 3],
    pub rnd_amp_speed: f32,
    pub rnd_time_var: f32,
    pub rnd_time_speed: f32,
}

impl SineParams {
    pub fn read(p: &Props) -> Self {
        let v = |n: &str| p.vec3(n).unwrap_or([0.0; 3]);
        SineParams {
            amplitude: v("Amplitude"),
            wave: [v("WaveLenghtX"), v("WaveLenghtY"), v("WaveLenghtZ")],
            freq: v("FreqOsci"),
            fade: p.byte("AmplitudeFadeX"),
            rnd_amplitude: v("RndAmplitude"),
            rnd_amp_speed: p.float("RndAmpSpeed"),
            rnd_time_var: p.float("RndTimeVar"),
            rnd_time_speed: p.float("RndTimeSpeed"),
        }
    }
}

/// Modifier state kept at run time: LastFrameCount (+0x50, serialized), CurrentTime
/// (+0xAC), CurTimeSpeed (+0xB0), CurAmplitude (+0xB4), the last three transient (zero on
/// load).
#[derive(Debug, Clone, Default)]
pub struct SineState {
    pub last_frame: u8,
    pub current_time: f32,
    pub cur_time_speed: f32,
    pub cur_amplitude: f32,
}

/// GMath.SinTab of angle `a` in radians: ftol(a × 65536 × (1 / 2π)), then (>> 2) & 0x3FFF.
/// D3DDrv's _ftol2 truncates on 64 bits (FISTP qword) and returns the low 32 bits.
fn sin_rad(a: f32) -> f32 {
    crate::coords::gmath_sin(((a * 65536.0) * 0.159_154_94) as i64 as i32)
}

/// Start: advances the modifier, with its appFrand draws (the game thread's
/// rand() sequence, held by the VM: `Vm::sine_render`). `frame` is the byte GEngine+0xD4 (frame
/// counter), `dt` GEngine+0xD0. Returns the amplitude of this call.
pub fn advance(p: &SineParams, s: &mut SineState, frame: u8, dt: f32, frand: &mut dyn FnMut() -> f32) -> [f32; 3] {
    // Once per frame: CurrentTime advances by dt, or by a random speed.
    if s.last_frame != frame {
        s.last_frame = frame;
        let step = if p.rnd_time_speed == 0.0 {
            dt
        } else {
            let mut speed = (((frand() - 0.5) * dt) * p.rnd_time_speed) * 0.2 + s.cur_time_speed;
            if !(speed <= 1.0) {
                speed = 1.0;
            } else if speed < 0.0 {
                speed = 0.0;
            }
            s.cur_time_speed = speed;
            (p.rnd_time_var * s.cur_time_speed + 1.0) * dt
        };
        s.current_time = step + s.current_time;
        if !(s.current_time <= 6283.1855) {
            s.current_time = 0.0;
        }
    }
    // On every call: amplitude, with its random part.
    let mut amp = p.amplitude;
    if p.rnd_amp_speed != 0.0 {
        let mut a = (((frand() - 0.5) * dt) * p.rnd_amp_speed) * 0.2 + s.cur_amplitude;
        if !(a <= 1.0) {
            a = 1.0;
        } else if a < -1.0 {
            a = -1.0;
        }
        s.cur_amplitude = a;
        for k in 0..3 {
            amp[k] = a * p.rnd_amplitude[k] + amp[k];
        }
    }
    amp
}

/// `advance`, then the waved local positions; `fade` is (UStaticMesh+0x17C,
/// +0x180) from `fade_bounds`.
pub fn deform(p: &SineParams, s: &mut SineState, frame: u8, dt: f32, frand: &mut dyn FnMut() -> f32, fade: [f32; 2], local: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let amp = advance(p, s, frame, dt, frand);
    wave(p, s.current_time, amp, fade, local)
}

/// End: local positions waved at time `t` (CurrentTime after `advance`)
/// with the call's amplitude `amp`.
pub fn wave(p: &SineParams, t: f32, amp: [f32; 3], fade: [f32; 2], local: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let bits = (amp[0] != 0.0) as u32 | ((amp[1] != 0.0) as u32) << 1 | ((amp[2] != 0.0) as u32) << 2;
    let (wx, wy, wz, f) = (p.wave[0], p.wave[1], p.wave[2], p.freq);
    // Sums in the FADD order of each case of the jump table;
    // X alone (cases 0 and 1) starts with the time term, X with Y or Z with the Z and Y terms.
    let x_single = |v: [f32; 3]| ((t * f[0] + v[2] * wx[2]) + v[1] * wx[1]) + v[0] * wx[0];
    let x_multi = |v: [f32; 3]| ((v[2] * wx[2] + v[1] * wx[1]) + t * f[0]) + v[0] * wx[0];
    let y = |v: [f32; 3]| ((t * f[1] + v[2] * wy[2]) + v[1] * wy[1]) + v[0] * wy[0];
    let z = |v: [f32; 3]| ((t * f[2] + v[1] * wz[1]) + v[2] * wz[2]) + v[0] * wz[0];
    local
        .iter()
        .map(|&v| {
            // AmplitudeFadeX: sine × (1 − |X − p5| × p6).
            let k = if p.fade != 0 {
                let mut d = v[0] - fade[0];
                if d < 0.0 {
                    d = -d;
                }
                1.0 - d * fade[1]
            } else {
                1.0
            };
            let wave = |angle: f32, c: usize| if p.fade != 0 { (sin_rad(angle) * k) * amp[c] + v[c] } else { sin_rad(angle) * amp[c] + v[c] };
            let mut out = v;
            if bits & 1 != 0 || bits == 0 {
                out[0] = wave(if bits & 6 == 0 { x_single(v) } else { x_multi(v) }, 0);
            }
            if bits & 2 != 0 {
                out[1] = wave(y(v), 1);
            }
            if bits & 4 != 0 {
                out[2] = wave(z(v), 2);
            }
            out
        })
        .collect()
}

/// X fade bounds (when the StaticMesh is loaded): for each material of the
/// mesh that is a SinusModifier with a non-zero AmplitudeFadeX, minimum and maximum X of the vertices
/// of that material's triangles (starting at ±1e6); SinFade_Negative (1): (max, 1 / (max − min)),
/// SinFade_Positive (2): (min, 1 / (max − min)), SinFade_Both (3): ((max + min) × 0.5,
/// 2 / (max − min)). The last matching material wins.
pub fn fade_bounds(materials: &[(u8, Vec<[f32; 3]>)], current: [f32; 2]) -> [f32; 2] {
    let mut out = current;
    for (fade, points) in materials {
        if *fade == 0 {
            continue;
        }
        let (mut min, mut max) = (1e6f32, -1e6f32);
        for v in points {
            if v[0] < min {
                min = v[0];
            }
            if !(v[0] <= max) {
                max = v[0];
            }
        }
        out = match fade {
            1 => [max, 1.0 / (max - min)],
            2 => [min, 1.0 / (max - min)],
            3 => [(max + min) * 0.5, 2.0 / (max - min)],
            _ => out,
        };
    }
    out
}
