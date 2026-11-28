//! BSP lightmaps computed at load time (FLightManager, Engine.dll, UnRenderBSP.cpp).
//!
//! Surfaces without an editor lightmap (PolyFlags & 0x40, or texture index out of the list)
//! get theirs from FLightManager::Cache: zone ambient light, then for
//! each light of the surface, its smoothed shadow mask (Smooth), the illumination
//! of each texel (Illuminate) and the coloured addition saturated at 0x7F (Merge).
//! The rendered texture (GetTextureData) doubles each byte (TEXF_RGBA7 format).
//!
//! Computations follow the original code's operation order in single precision (D3D sets
//! the FPU to 24 bits); roundings are those of FISTP (round to nearest even).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::lighting::{fget_hsv, plane_byte, Light};
use crate::model::{LightMapIndex, Model, ShadowMask};
use std::sync::OnceLock;

/// Tables built by FLightManager::FLightManager.
struct Tables {
    /// Attenuation (2S³ - 3S² + 1) / S for S = √((i + 1) / 4096).
    falloff: Vec<f32>,
    /// Shadow smoothing filter: for each 7-bit window, 4 kernel rows, 4 texels
    /// per integer (one byte each).
    filter: Vec<[u32; 4]>,
    /// Fast square root (2 × 1024 entries).
    sqrt: Vec<u32>,
    /// Fast x^-0.5: mantissa and exponent.
    rsqrt_mantissa: Vec<f32>,
    rsqrt_exponent: Vec<f32>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        // 24-bit FPU: attenuation s = √((i + 1) / 4096) in single precision, then
        // ((s²·s + s²·s) − s²·3 + 1) / s in single precision.
        let falloff = (0..0x1000)
            .map(|i| {
                let s = ((i + 1) as f32 * 0.000_244_140_625).sqrt();
                let s2 = s * s;
                let s3 = s2 * s;
                ((((s3 + s3) - s2 * 3.0) + 1.0) / s).to_bits()
            })
            .map(f32::from_bits)
            .collect();
        // 8 × 8 kernel (three useful rows), total weight 320.
        let mut kernel = [[0u32; 8]; 8];
        kernel[0][1..4].copy_from_slice(&[0x18, 0x28, 0x18]);
        kernel[1][1..4].copy_from_slice(&[0x28, 0x40, 0x28]);
        kernel[2][1..4].copy_from_slice(&[0x18, 0x28, 0x18]);
        let total: u32 = kernel.iter().flatten().sum();
        let filter = (0..0x80u32)
            .map(|v| {
                let mut e = [0u32; 4];
                for (row, slot) in e.iter_mut().enumerate() {
                    for lane in 0..4 {
                        let mut sum = 0;
                        for (j, weight) in kernel[row].iter().enumerate() {
                            if v & (1 << (j + lane)) != 0 {
                                sum += weight;
                            }
                        }
                        *slot += (sum * 0xFF / total) << (lane * 8);
                    }
                }
                e
            })
            .collect();
        let mut sqrt = vec![0u32; 0x800];
        // √2 comes from appSqrt(2), rounded to single precision.
        for i in 0..0x400u32 {
            let f = f32::from_bits((0x3F80_0000 & 0xFF80_0000) + (i << 13));
            let s = f32::from_bits(f.sqrt().to_bits().wrapping_sub(0x2000_0000));
            sqrt[i as usize] = (s * std::f32::consts::SQRT_2).to_bits();
            sqrt[0x400 + i as usize] = (s + s).to_bits();
        }
        // (exponent, mantissa, -0.5): pow on each exponent (sign included) and
        // each 10-bit mantissa; NaN replaced by 0.
        let pow = |x: f32| {
            let v = (x.abs() as f64).dpowf(-0.5) as f32;
            if v.is_nan() {
                0.0
            } else {
                v
            }
        };
        let rsqrt_exponent = (0..0x200u32).map(|i| pow(f32::from_bits(i << 23))).collect();
        // msvcr70's pow (_CIpow) with a 24-bit FPU: 2^t with t = y·log2(x) rounded to single
        // (FSUB of the fractional part), then the result in single precision. Mantissas: x in [1, 2[.
        let rsqrt_mantissa = (0..0x400u32)
            .map(|i| {
                let x = f32::from_bits((0x3F80_0000 & 0xFF80_0000) + (i << 13));
                (((-0.5 * (x as f64).dlog2()) as f32 as f64).dexp2()) as f32
            })
            .collect();
        Tables { falloff, filter, sqrt, rsqrt_mantissa, rsqrt_exponent }
    })
}

/// Table-based square root.
fn fast_sqrt(x: f32) -> f32 {
    let b = x.to_bits();
    let t = tables().sqrt[((b >> 11) & 0x1FFC) as usize / 4];
    f32::from_bits(t.wrapping_add((b & 0x7F00_0000) >> 1))
}

/// Table-based x^-0.5.
fn fast_rsqrt(x: f32) -> f32 {
    let b = x.to_bits();
    let t = tables();
    t.rsqrt_mantissa[((b >> 13) & 0x3FF) as usize] * t.rsqrt_exponent[(b >> 23) as usize]
}

/// FISTP in round-to-nearest mode.
fn fistp(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// (_ftol): truncation.
fn ftol(x: f32) -> i32 {
    x as i32
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// FLightManager::Smooth: filtered shadow mask, one byte per texel (`row_bytes` × 8 per row).
fn smooth(lm: &LightMapIndex, mask: &ShadowMask) -> Vec<u8> {
    let t = tables();
    let bpr = mask.row_bytes.max(0) as usize;
    let rows = mask.rows.max(0) as usize;
    let mut out = vec![0u32; rows * bpr * 2];
    let (mut prev, mut cur, mut next) = (0usize, 0usize, bpr * 2);
    let mut src = 0usize;
    let byte = |i: usize| mask.bits.get(i).copied().unwrap_or(0) as u32;
    for y in 0..rows {
        let mut w = byte(src) << 10;
        let mut s = src + 1;
        if w & 0x400 != 0 {
            w |= 0x300;
        }
        for x in 0..bpr {
            let n = if x + 1 < bpr {
                let v = byte(s) << 10;
                s += 1;
                v
            } else if (w >> 8) & 0x200 != 0 {
                0xC00
            } else {
                0
            };
            w = (w >> 8) + n;
            let a = &t.filter[(w & 0x7F) as usize];
            let b = &t.filter[((w >> 4) & 0x7F) as usize];
            let mut add = |i: usize, v: u32| {
                if let Some(o) = out.get_mut(i) {
                    *o = o.wrapping_add(v);
                }
            };
            add(prev, a[0]);
            add(cur, a[1]);
            add(next, a[2]);
            add(prev + 1, b[0]);
            add(cur + 1, b[1]);
            add(next + 1, b[2]);
            prev += 2;
            cur += 2;
            next += 2;
        }
        src += bpr;
        if y == 0 {
            prev -= bpr * 2;
        }
        if y as i32 == lm.size[1] - 2 {
            next = next.wrapping_sub(bpr * 2);
        }
    }
    out.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Rectangle of texels touched by a light: x0, y0, x1, y1.
type Rect = (i32, i32, i32, i32);

/// FLightManager::Illuminate: illumination (0..255) of each texel of the rectangle, for a
/// static light. `shadow` comes from Smooth, with `stride` bytes per row.
/// Animated branches of Illuminate, keeping the x87 roundings.
fn illuminate_effect(light: &Light, rel: [f32; 3], shadow: f32, scale: f32, inv: f32, height: f32, t: &Tables) -> u8 {
    let [x, y, z] = rel;
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let d2 = match light.effect {
        4 | 10 | 18 => (zz + yy) + xx,
        5 | 6 | 9 => (zz + xx) + yy,
        _ => (xx + yy) + zz,
    };
    let idx = fistp(d2 * scale) as u32;
    if shadow == 0.0 || idx >= 0x1000 { return 0; }
    // A qword constant stays in double until the operation, then PC=24 rounds.
    let mul = |a: f32, b: f64| (f64::from(a) * b) as f32;
    let add = |a: f32, b: f64| (f64::from(a) + b) as f32;
    let trunc = |a: f32| {
        let a = f64::from(a);
        if !a.is_finite() || a.abs() >= 9_223_372_036_854_775_808.0 { 0 } else { (a as i64) as i32 }
    };
    let sin = crate::lighting::sin_u;
    let cosine = |angle: f32| {
        let index = trunc((angle * 65_536.0) * f32::from_bits(0xBE22_F983));
        sin(0x4000i32.wrapping_sub(index))
    };
    // The Atan2 return value stays in double; only FMUL then rounds to 24 bits.
    let atan = |a: f32, b: f32| f64::from(a).datan2(f64::from(b));
    let falloff = t.falloff[idx as usize];
    let result = match light.effect {
        4 => {
            // Searchlight: horizontal angle, period and phase.
            let phase_time = if light.period == 0 { 0.0 } else { (light.effect_time * 35.0) as f32 / light.period as f32 };
            let phase = add(mul(light.phase as f32, 0.09817477315664291) + phase_time, 6.2831854820251465);
            let a = add((atan(x, y) * 4.0) as f32, f64::from(phase));
            let angle = (f64::from(a) % 25.132741928100586) as f32;
            if f64::from(angle) < 3.1415927410125732 || f64::from(angle) > 9.42477822303772 { return 0; }
            let mut m = mul(add(cosine(angle), 1.0), 0.5);
            let radial = mul(yy + xx, 0.00006);
            if radial < 1.0 { m *= radial; }
            ((shadow * falloff) * m) * height
        }
        5 | 6 => {
            // SlowWave/FastWave: integer distance, then the wave.
            let distance = trunc(fast_sqrt(d2));
            let (distance, speed) = if light.effect == 6 { (distance >> 2, 2048.0) } else { (distance, 1024.0) };
            let angle = trunc(mul(distance as f32 - (light.effect_time * 35.0) as f32, speed));
            let modulation = add(mul(sin(angle), 0.3), 0.7);
            ((modulation * falloff) * shadow) * height
        }
        9 => {
            // Three waves with the phases and multiplication order.
            let distance = trunc(fast_sqrt((xx + yy) + zz) * 8.0);
            let time = (light.effect_time * 4000.0) as f32;
            let a = trunc((distance as f32 + time) * -16.0);
            let m1 = add(mul(sin(0x4000i32.wrapping_sub(a)), 0.1), 0.9);
            let a = trunc((distance.wrapping_mul(2) as f32 - time) * 16.0);
            let m2 = add(mul(sin(a), 0.1), 0.9);
            let a = trunc(((distance >> 1) as f32 - time) * 16.0);
            let m3 = add(mul(sin(a), 0.1), 0.9);
            ((((m1 * m2) * m3) * falloff) * shadow) * height
        }
        10 => {
            // Two angles and union of the lobes, attenuated near the axis.
            let time = (light.effect_time * 5.0) as f32;
            let first = mul(add(cosine(time + (atan(x, y) * 11.0) as f32), 1.0), 0.5);
            let second_angle = atan((yy + xx).sqrt(), z);
            let second = mul(add(cosine((second_angle * 11.0) as f32 + time), 1.0), 0.5);
            let mut m = (first + second) - second * first;
            let radial = mul(yy + xx, 0.00005);
            if radial < 1.0 { m *= radial; }
            (((1.0 - m) * falloff) * shadow) * height
        }
        14 => {
            // Shell: only the band 0.8 < distance/radius < 1.
            let distance = fast_sqrt(d2) * inv;
            if f64::from(distance) >= 1.0 || f64::from(distance) <= 0.8 { return 0; }
            let width = ((f64::from(distance) - 0.9) as f32).abs();
            (1.0 - mul(width, 10.0)) * shadow
        }
        16 => {
            // NonIncidence: vertical angle and time.
            let angle = (atan(fast_sqrt(xx + yy), z) * 11.0) as f32 + (light.effect_time * 5.0) as f32;
            let m = mul(add(cosine(angle), 1.0), 0.5);
            ((m * falloff) * shadow) * height
        }
        18 => {
            // Interference: angular wave faded towards 1 near the axis.
            let angle = (atan(x, y) * 6.0) as f32 + (light.effect_time * 3.5) as f32;
            let mut m = mul(add(cosine(angle), 1.0), 0.5);
            let radial = mul(yy + xx, 0.0001);
            if radial < 1.0 { m = m * radial + (1.0 - radial); }
            ((shadow * falloff) * m) * height
        }
        _ => unreachable!("animated effect handled separately"),
    };
    fistp(result - 0.5) as u8
}

fn illuminate(shadow: &[u8], stride: usize, plane: [f32; 4], two_sided: bool, lm: &LightMapIndex, light: &Light, rect: Rect) -> Vec<u8> {
    let t = tables();
    let usize_ = lm.size[0].max(0) as usize;
    let mut out = vec![0u8; usize_ * lm.size[1].max(0) as usize];
    let normal = [plane[0], plane[1], plane[2]];
    let sh = |x: i32, y: i32| shadow.get(y as usize * stride + x as usize).copied().unwrap_or(0) as f32;
    if light.effect == 0x14 {
        // LE_Sunlight: N · -direction, the whole lightmap.
        // ((N.x · −D.x) + (N.z · −D.z)) + (N.y · −D.y).
        let mut incidence = normal[0] * -light.direction[0] + normal[2] * -light.direction[2] + normal[1] * -light.direction[1];
        if !two_sided {
            if incidence <= 0.0 {
                incidence = 0.0;
            }
        } else if incidence < 0.0 {
            incidence = -incidence;
        }
        for y in 0..lm.size[1] {
            for x in 0..lm.size[0] {
                out[y as usize * usize_ + x as usize] = fistp(sh(x, y) * incidence - 0.5) as u8;
            }
        }
        return out;
    }
    let radius = if light.radius <= 1.0 { 1.0 } else { light.radius };
    let inv = 1.0 / radius;
    let scale = inv * inv * 4093.0;
    let o = lm.origin;
    let mut height =
        ((light.location[0] - o[0]) * normal[0] + (light.location[2] - o[2]) * normal[2] + (light.location[1] - o[1]) * normal[1]) * (1.0 / light.radius);
    if height < 0.0 {
        height = -height;
    }
    let (x0, y0, x1, y1) = rect;
    let (fx, fy) = (x0 as f32, y0 as f32);
    let (u, v) = (lm.u_step, lm.v_step);
    let mut rel = [0.0f32; 3];
    // (Origin − Light + U · x0) + V · y0.
    for k in 0..3 {
        rel[k] = ((o[k] - light.location[k]) + u[k] * fx) + v[k] * fy;
    }
    let n = (x1 - x0) as f32;
    // Spotlight-specific data (LE_StaticSpot, LE_Spotlight).
    let cone = 1.0 - light.cone as f32 * 0.003_906_25;
    let spot_scale = 1.0 / (1.0 - cone);
    let spot_bias = spot_scale * cone;
    let cone2 = cone * cone;
    for y in y0..y1 {
        for x in x0..x1 {
            let s = sh(x, y);
            let d2 = (rel[0] * rel[0] + rel[1] * rel[1]) + rel[2] * rel[2];
            let idx = fistp(d2 * scale) as u32;
            let value: u8 = match light.effect {
                4 | 5 | 6 | 9 | 10 | 14 | 16 | 18 => illuminate_effect(light, rel, s, scale, inv, height, t),
                0x11 => {
                    // LE_Cylinder: distance to the vertical axis, neither table nor zero-shadow test.
                    let i = fistp((1.0 - (rel[0] * rel[0] + rel[1] * rel[1]) * inv * inv) * s - 0.5);
                    if i <= 0 {
                        0
                    } else {
                        i as u8
                    }
                }
                _ if s == 0.0 || idx >= 0x1000 => 0,
                0x0D => fistp((1.02 - fast_sqrt(d2) * inv) * s - 0.5) as u8,
                0x08 | 0x0C => {
                    let d2b = (rel[2] * rel[2] + rel[0] * rel[0]) + rel[1] * rel[1];
                    let along = rel[2] * light.direction[2] + light.direction[1] * rel[1] + light.direction[0] * rel[0];
                    if along <= 0.0 || along * along <= d2b * cone2 {
                        0
                    } else {
                        let k = fast_rsqrt(d2b) * along * spot_scale - spot_bias;
                        fistp(k * k * t.falloff[idx as usize] * s * height - 0.5) as u8
                    }
                }
                // LE_None and the other fixed effects: attenuation × incidence, rounded by
                // adding 2^23.
                _ => {
                    let f = s * t.falloff[idx as usize] * height + 8_388_608.0;
                    f.to_bits() as u8
                }
            };
            out[y as usize * usize_ + x as usize] = value;
            for k in 0..3 {
                rel[k] += u[k];
            }
        }
        for k in 0..3 {
            rel[k] = (rel[k] - u[k] * n) + v[k];
        }
    }
    out
}

/// FLightManager::Merge: adds the coloured light to the rectangle of `dst` (7-bit BGRA).
fn merge(dst: &mut [u32], usize_: usize, illum: &[u8], light: &Light, rect: Rect) {
    let (x0, y0, x1, y1) = rect;
    if x1 - x0 <= 0 {
        return;
    }
    let step = |c: f32| fistp(c * 65536.0 - 0.5);
    let (sr, sg, sb) = (step(light.color[0]), step(light.color[1]), step(light.color[2]));
    let mut table = [0u32; 256];
    let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
    for e in table.iter_mut() {
        let c = |acc: i32| (acc >> 16).min(0x7F) as u32 & 0xFF;
        *e = c(b) | c(g) << 8 | c(r) << 16 | 0xFF << 24;
        r = r.wrapping_add(sr);
        g = g.wrapping_add(sg);
        b = b.wrapping_add(sb);
    }
    for y in y0..y1 {
        for x in x0..x1 {
            let i = y as usize * usize_ + x as usize;
            let sum = dst[i].wrapping_add(table[illum[i] as usize]);
            let high = sum & 0x8080_8080;
            dst[i] = if high != 0 { high.wrapping_sub(high >> 7) | (sum & 0x7F7F_7F7F) } else { sum };
        }
    }
}

/// Rectangle of texels a light can reach (FLightManager::Cache): the whole
/// lightmap for LE_Sunlight, otherwise the intersection circle of its sphere (radius × √1.05) with the
/// plane, projected by WorldToLightMap.
fn light_rect(plane: [f32; 4], lm: &LightMapIndex, light: &Light) -> Rect {
    if light.effect == 0x14 {
        return (0, 0, lm.size[0], lm.size[1]);
    }
    let p = [plane[0], plane[1], plane[2]];
    let d = (light.location[2] * p[2] + light.location[1] * p[1] + p[0] * light.location[0]) - plane[3];
    let mut r2 = light.radius * light.radius * 1.05 - d * d;
    if r2 < 0.0 {
        r2 = 0.0;
    }
    let radius = r2.sqrt();
    let c = [light.location[0] - d * p[0], light.location[1] - d * p[1], light.location[2] - d * p[2]];
    let m = &lm.world_to_lightmap;
    let tu = ((m[8] * c[2] + m[4] * c[1]) + c[0] * m[0]) + m[12];
    let tv = ((m[9] * c[2] + m[5] * c[1]) + m[1] * c[0]) + m[13];
    let su = radius / fast_sqrt(dot(lm.u_step, lm.u_step));
    let sv = radius / fast_sqrt(dot(lm.v_step, lm.v_step));
    let clamp = |v: i32, max: i32| v.clamp(0, max.max(0));
    (clamp(ftol(tu - su), lm.size[0]), clamp(ftol(tv - sv), lm.size[1]), clamp(ftol(tu + su), lm.size[0]), clamp(ftol(tv + sv), lm.size[1]))
}

/// A surface's lightmap as FLightManager::Cache keeps it: buffer of the static
/// lights (local_1c[3]) and lights added to it every frame.
#[derive(Debug, Clone)]
pub struct SurfaceLightmap {
    pub surf: usize,
    /// Computed texels (USize × VSize) and texture size.
    pub size: [usize; 2],
    pub texture: [u32; 2],
    /// Merged static lights, 7-bit BGRA.
    pub static_texels: Vec<u32>,
    /// Dynamic or animated lights touching the surface (list:
    /// sun, or plane within the radius), merged without a shadow mask by the second pass.
    pub dynamic: Vec<usize>,
}

/// First call of FLightManager::Cache for a surface: zone ambient light (hue,
/// saturation, brightness), then the static lights from its shadow masks.
/// Animated or dynamic lights of each BSP surface, distributed as during the
/// traversal (BSP rendering): all start at the root; at each node, a
/// light more than its radius in front of (behind) the plane only goes down the front (back);
/// otherwise it goes down both sides and attaches to the surface of each coplanar node whose
/// sphere touches its own. The sun (effect 0x14) attaches everywhere.
pub fn surface_dynamic_lights(model: &Model, lights: &[Light]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); model.surfs.len()];
    if model.nodes.is_empty() {
        return out;
    }
    let root: Vec<usize> = lights.iter().enumerate().filter(|(_, l)| l.dynamic || l.animated).map(|(i, _)| i).collect();
    let mut stack: Vec<(usize, Vec<usize>)> = vec![(0, root)];
    let attach = |out: &mut Vec<Vec<usize>>, surf: i16, light: usize| {
        if let Some(list) = out.get_mut(surf as usize) {
            if !list.contains(&light) {
                list.push(light);
            }
        }
    };
    while let Some((n, list)) = stack.pop() {
        let Some(node) = model.nodes.get(n) else { continue };
        let (mut front, mut back) = (Vec::new(), Vec::new());
        for &i in &list {
            let l = &lights[i];
            let coplanar = |model: &Model| {
                let mut v = Vec::new();
                let mut c = n as i32;
                while c != -1 && v.len() < model.nodes.len() {
                    v.push(c as usize);
                    c = model.nodes[c as usize].links[2] as i32;
                }
                v
            };
            if l.effect == 0x14 {
                for c in coplanar(model) {
                    attach(&mut out, model.nodes[c].surf, i);
                }
                front.push(i);
                back.push(i);
                continue;
            }
            let p = node.plane;
            let d = (l.location[0] * p[0] + l.location[1] * p[1] + l.location[2] * p[2]) - p[3];
            if d < -l.radius {
                back.push(i);
            } else if d <= l.radius {
                for c in coplanar(model) {
                    let s = model.nodes[c].sphere;
                    let dz = l.location[2] - s[2];
                    let dy = l.location[1] - s[1];
                    let dx = l.location[0] - s[0];
                    let r = s[3] + l.radius;
                    if dz * dz + dy * dy + dx * dx < r * r {
                        attach(&mut out, model.nodes[c].surf, i);
                    }
                }
                front.push(i);
                back.push(i);
            } else {
                front.push(i);
            }
        }
        if node.links[1] >= 0 && !front.is_empty() {
            stack.push((node.links[1] as usize, front));
        }
        if node.links[0] >= 0 && !back.is_empty() {
            stack.push((node.links[0] as usize, back));
        }
    }
    out
}

/// `dynamic`: animated or dynamic lights of the surface (`surface_dynamic_lights`), added
/// every frame without shadow; the surface's other lights are baked once with their
/// shadow mask.
pub fn cache(model: &Model, lights: &[Light], surf: usize, ambient: (u8, u8, u8), dynamic: &[usize]) -> Option<SurfaceLightmap> {
    let s = model.surfs.get(surf)?;
    if s.light_map == -1 || s.poly_flags & 0x40_0000 != 0 {
        return None;
    }
    let lm = model.light_maps.get(s.light_map as usize)?;
    let (w, h) = (lm.size[0].max(0) as usize, lm.size[1].max(0) as usize);
    let channel = (s.poly_flags >> 9) & 0xF;
    // Ambient: FColor(FGetHSV(H, S, V) × 0.5), zero alpha; none if PolyFlags & 0x200000.
    let base = if s.poly_flags & 0x20_0000 == 0 {
        let c = fget_hsv(ambient.0, ambient.1, ambient.2);
        let (r, g, b) = (plane_byte(c[0] * 0.5), plane_byte(c[1] * 0.5), plane_byte(c[2] * 0.5));
        b as u32 | (g as u32) << 8 | (r as u32) << 16
    } else {
        0
    };
    let mut texels = vec![base; w * h];
    for i in 0..lm.num_lights.max(0) {
        let Some(mask) = model.light_bits.get((lm.first_light + i) as usize) else { continue };
        if mask.light == -1 {
            continue;
        }
        let Some(light) = lights.get(mask.light as usize) else { continue };
        if (1u32 << channel) & (light.flags & 0xFFFF) == 0 {
            continue;
        }
        // The lights of the surface's dynamic list only count in the second pass.
        if dynamic.contains(&(mask.light as usize)) {
            continue;
        }
        let rect = light_rect(s.plane, lm, light);
        let shadow = smooth(lm, mask);
        let stride = (mask.row_bytes.max(0) as usize) * 8;
        let illum = illuminate(&shadow, stride, s.plane, s.poly_flags & 0x100 != 0, lm, light, rect);
        if crate::diag::lightmap_surface() == Some(surf) {
            let mx = illum.iter().copied().max().unwrap_or(0);
            let sh = shadow.iter().filter(|&&v| v > 0).count();
            eprintln!("surf {surf} light {} type {} effect {} color {:?} radius {} pos {:?}: max {mx} shadow>0 {sh}/{} rect {rect:?}", mask.light, light.light_type, light.effect, light.color, light.radius, light.location, shadow.len());
        }
        merge(&mut texels, w, &illum, light, rect);
    }
    if crate::diag::lightmap_surface() == Some(surf) {
        eprintln!("surf {surf}: {}x{} ambience {base:06x} dynamic {dynamic:?} plane {:?} flags {:x}", w, h, s.plane, s.poly_flags);
        for &i in dynamic {
            if let Some(l) = lights.get(i) {
                eprintln!("   dynamic {i}: type {} effect {} colour {:?} radius {} pos {:?} dyn {} anim {} flags {:x}", l.light_type, l.effect, l.color, l.radius, l.location, l.dynamic, l.animated, l.flags);
            }
        }
    }
    let dynamic = dynamic.iter().copied().filter(|&i| lights.get(i).is_some_and(|l| (1u32 << channel) & (l.flags & 0xFFFF) != 0)).collect();
    let texture = [lm.size[2].max(lm.size[0]).max(1) as u32, lm.size[3].max(lm.size[1]).max(1) as u32];
    Some(SurfaceLightmap { surf, size: [w, h], texture, static_texels: texels, dynamic })
}

impl SurfaceLightmap {
    /// End of FLightManager::Cache and GetTextureData: copy of the static buffer, addition of
    /// the list's lights (full shadow mask), then each integer shifted by one bit, into
    /// a texture of `texture` texels (RGBA).
    pub fn texture_data(&self, model: &Model, lights: &[Light]) -> Vec<u8> {
        let [w, h] = self.size;
        let mut texels = self.static_texels.clone();
        if let Some(s) = model.surfs.get(self.surf) {
            if let Some(lm) = model.light_maps.get(s.light_map as usize) {
                let full = vec![0xFFu8; w * h];
                for &i in &self.dynamic {
                    let Some(light) = lights.get(i) else { continue };
                    let rect = light_rect(s.plane, lm, light);
                    let illum = illuminate(&full, w, s.plane, s.poly_flags & 0x100 != 0, lm, light, rect);
                    merge(&mut texels, w, &illum, light, rect);
                }
            }
        }
        let (tw, th) = (self.texture[0] as usize, self.texture[1] as usize);
        let mut rgba = vec![0u8; tw * th * 4];
        for y in 0..h.min(th) {
            for x in 0..w.min(tw) {
                let p = texels[y * w + x] << 1;
                let o = (y * tw + x) * 4;
                rgba[o..o + 4].copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
            }
        }
        rgba
    }
}

