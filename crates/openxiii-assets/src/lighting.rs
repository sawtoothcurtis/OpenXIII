//! Engine.dll's static lighting: lights (FLight::Init, FLight::SampleIntensity), HSV colours (FGetHSV), per-vertex lighting of StaticMeshes
//! (CalculateLighting), actor ambient colour (FDynamicActor::Refresh), BSP leaves touched by an actor (AActor::UpdateLeaves,
//! UModel::BoxLeaves) and the FColor(FPlane) conversion.

// Negated comparisons reproduce those of the original code (FCOM: NaN included).
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]

use crate::model::{LightRecord, Model};

/// Inverse of 255 as stored in Engine.dll.
pub const INV_255: f32 = 0.003_921_569;

/// FGetHSV(H, S, V): RGB colour from 8-bit hue, saturation and brightness.
pub fn fget_hsv(h: u8, s: u8, v: u8) -> [f32; 4] {
    // Each x87 operation rounded to 24 bits: (H · 6) · 1/256, integer part through
    // _ftol, Sinv = (255 − S) · (1/255 as a float), Brt = V · (1/255);
    // A = (1 − Sinv) · Brt, B = (1 − Sinv · F) · Brt, C = (1 − (1 − F) · Sinv) · Brt.
    const INV255: f32 = f32::from_bits(0x3B80_8081);
    let h6 = (h as f32 * 6.0) * 0.003_906_25;
    let i = h6 as i32;
    let f = h6 - i as f32;
    let sinv = (255 - s as i32) as f32 * INV255;
    let brt = v as f32 * INV255;
    let a = (1.0 - sinv) * brt;
    let b = (1.0 - sinv * f) * brt;
    let c = (1.0 - (1.0 - f) * sinv) * brt;
    let (r, g, bl) = match i {
        0 => (brt, c, a),
        1 => (b, brt, a),
        2 => (a, brt, c),
        3 => (a, b, brt),
        4 => (c, a, brt),
        _ => (brt, a, b),
    };
    const EPS: f32 = 0.0003;
    [r + EPS, g + EPS, bl + EPS, 1.0]
}

/// FColor(FPlane) of one component: x·255 - 0.5 rounded to nearest, clamped to 0..255.
pub fn plane_byte(x: f32) -> u8 {
    let v = x * 255.0;
    (v - 0.5).round_ties_even().clamp(0.0, 255.0) as u8
}

/// FColor::operator+=: per-component saturating addition.
pub fn add_sat(a: [u8; 3], b: [u8; 3]) -> [u8; 3] {
    [a[0].saturating_add(b[0]), a[1].saturating_add(b[1]), a[2].saturating_add(b[2])]
}

/// FGlobalMath::SinTab.
pub(crate) fn sin_u(a: i32) -> f32 {
    crate::coords::gmath_sin(a)
}

/// Light ready to illuminate (FLight, 0x40 bytes).
#[derive(Debug, Clone)]
pub struct Light {
    pub color: [f32; 4],
    pub location: [f32; 3],
    pub direction: [f32; 3],
    pub radius: f32,
    /// FLight+0x30: dynamic light (not baked into StaticMeshes).
    pub dynamic: bool,
    /// FLight+0x31: animated light (variable type or effect).
    pub animated: bool,
    pub light_type: u8,
    pub effect: u8,
    /// Metadata read by the FLightManager::Illuminate effects.
    pub period: u8,
    pub phase: u8,
    pub effect_time: f64,
    pub brightness: u8,
    pub cone: u8,
    pub zone: u8,
    pub flags: u32,
}

/// Engine.dll statics read by FLight::Init for type 5: last time seen (
/// float) and toggle, zero at process start.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StateType5 {
    pub last_time: f32,
    pub toggle: u32,
}

static STATE_TYPE5: std::sync::Mutex<StateType5> = std::sync::Mutex::new(StateType5 { last_time: 0.0, toggle: 0 });

/// Rounding of each x87 operation to a 24-bit mantissa (the game's precision control).
fn r24(x: f64) -> f64 {
    (x as f32) as f64
}

/// _ftol: truncation to a 64-bit integer, low word returned.
fn ftol_low(x: f64) -> i32 {
    (x as i64) as i32
}

/// FLight::Init with the process's type-5 static state.
pub fn init_light(r: &LightRecord, level_brightness: f32, time: f64, frand: &mut dyn FnMut() -> f32) -> Light {
    let mut state_value = STATE_TYPE5.lock().unwrap_or_else(|e| e.into_inner());
    init_light_state(r, level_brightness, time, frand, &mut state_value)
}

/// FLight::Init: intensity according to LightType at time `time` (GLevel->TimeSeconds), colour
/// HSV × LevelInfo.Brightness, radius (LightRadius + 1) × 25 (AActor::WorldLightRadius; FLight::WorldLightRadius). x87 on 24 bits: each operation is
/// rounded to a 32-bit float, _ftol returns the low word of a 64-bit integer. Type 5: the last
/// time seen and the toggle are Engine.dll statics (`state_value`); the toggle flips on every
/// different time, intensity is 1.0 when the toggle is zero, otherwise 0.0.
pub fn init_light_state(r: &LightRecord, level_brightness: f32, time: f64, frand: &mut dyn FnMut() -> f32, state_value: &mut StateType5) -> Light {
    // FLight+0x31 is not serialized: zero on load, set to 1 by animated types.
    let mut animated = false;
    let mut intensity: f32 = 0.0;
    match r.light_type {
        1 => intensity = 1.0,
        2 | 7 => {
            let period = (r.period as i32).max(1) as f64;
            let a = ftol_low(r24(r24(r24(time * 2_293_760.0) / period) + ((r.phase as i32) << 8) as f64));
            let (m, c) = if r.light_type == 2 { (0.39_f32, 0.6_f32) } else { (0.09_f32, 0.9_f32) };
            intensity = sin_u(a) * m + c;
            animated = true;
        }
        3 => {
            let d = (r.period as i32 + 1) as f64;
            let first = ftol_low(r24(r24(r24(time * 70.0) + r.phase as f64) / d));
            intensity = if first & 1 == 0 {
                1.0
            } else {
                let second = ftol_low(r24(r24(r24(time * 60.0) + r.phase as f64) / d));
                if second & 1 == 0 {
                    1.0
                } else {
                    0.0
                }
            };
            animated = true;
        }
        4 => {
            let f = frand();
            intensity = if f < 0.5 { 0.0 } else { f };
            animated = true;
        }
        5 => {
            if state_value.last_time as f64 != time {
                state_value.toggle ^= 1;
                state_value.last_time = time as f32;
            }
            intensity = if state_value.toggle == 0 { 1.0 } else { 0.0 };
            animated = true;
        }
        _ => {}
    }
    if (r.flags >> 8) as u8 & 0x80 != 0 || !matches!(r.effect, 0 | 0x14 | 0x11 | 0xD | 8) {
        animated = true;
    }
    let intensity = if intensity < 0.0 {
        0.0
    } else if intensity >= 1.0 {
        1.0
    } else {
        intensity
    };
    let hsv = fget_hsv(r.hue, r.saturation, r.brightness);
    let color = hsv.map(|c| intensity * c * level_brightness);
    Light {
        color,
        location: r.location,
        direction: r.direction,
        radius: (r.radius as i32 + 1) as f32 * 25.0,
        dynamic: (r.flags >> 11) & 1 != 0,
        animated,
        light_type: r.light_type,
        effect: r.effect,
        period: r.period,
        phase: r.phase,
        effect_time: time,
        brightness: r.brightness,
        cone: r.cone,
        zone: r.zone,
        flags: r.flags,
    }
}

/// FVector::SafeNormal: (x² + y²) + z² then 1/sqrt, all in 32-bit floats
/// (x87 on 24 bits: each operation is rounded).
pub fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    let sq = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let s = crate::coords::rsqrt(sq);
    [s * v[0], s * v[1], s * v[2]]
}

/// FLight::SampleIntensity(position, normal) according to LightEffect, 32-bit floats
/// (x87 on 24 bits) and dot products in machine-code order.
pub fn sample_intensity(l: &Light, p: [f32; 3], n: [f32; 3]) -> f32 {
    let (dir, radius_list, cone) = (l.direction, l.radius, l.cone);
    let d = [l.location[0] - p[0], l.location[1] - p[1], l.location[2] - p[2]];
    match l.effect {
        // LE_Sunlight.
        0x14 => {
            let dot = (n[1] * dir[1] + n[2] * dir[2]) + n[0] * dir[0];
            if !(dot < 0.0) {
                return 0.0;
            }
            ((-dir[1]) * n[1] + (-dir[2]) * n[2]) + (-dir[0]) * n[0]
        }
        // LE_StaticSpot, LE_Spotlight.
        8 | 12 => {
            let u = safe_normal(d);
            let ndl = (n[1] * u[1] + n[2] * u[2]) + n[0] * u[0];
            if ndl < 0.0 {
                return 0.0;
            }
            let dsq = (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0];
            let dist = dsq.sqrt();
            if !(dist < radius_list) {
                return 0.0;
            }
            let dotdir = -((d[2] * dir[2] + d[1] * dir[1]) + d[0] * dir[0]);
            // s is rounded to a float (not the extended value) before being used for k.
            let s = (1.0f64 - (cone as f64 * 0.003_906_25)) as f32;
            let k = (1.0f64 / (1.0 - s as f64)) as f32;
            if dotdir < 0.0 {
                return 0.0;
            }
            if !((s * s) * dsq < dotdir * dotdir) {
                return 0.0;
            }
            let t = (k / dist) * dotdir - k * s;
            let v = ((radius_list / (dist + dist)) * ndl * t) * t;
            if v >= 1.0 {
                1.0
            } else {
                v
            }
        }
        // LE_NonIncidence.
        13 => {
            let dsq = (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0];
            let dist = dsq.sqrt();
            let dn = (d[1] * n[1] + d[2] * n[2]) + d[0] * n[0];
            if dn <= 0.0 || !(dist < radius_list) {
                return 0.0;
            }
            ((1.02f32 - dist / radius_list) as f64).sqrt() as f32
        }
        // LE_Cylinder: horizontal distance.
        0x11 => {
            let (x2, y2) = (d[0] * d[0], d[1] * d[1]);
            let dsq = (d[2] * d[2] + x2) + y2;
            let dist = dsq.sqrt();
            if !(dist < radius_list) {
                return 0.0;
            }
            let v = 1.0 - (x2 + y2) / (radius_list * radius_list);
            if 0.0 < v {
                v
            } else {
                0.0
            }
        }
        _ => {
            let u = safe_normal(d);
            let ndl = (n[1] * u[1] + n[2] * u[2]) + n[0] * u[0];
            if ndl < 0.0 {
                return 0.0;
            }
            let dsq = (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0];
            let dist = dsq.sqrt();
            if dist < radius_list {
                (radius_list / (dist + dist)) * ndl
            } else {
                0.0
            }
        }
    }
}

/// An actor's LocalToWorld matrix: rotation rows (sine table)
/// scaled by DrawScale × DrawScale3D, then translation Location - PrePivot. Same
/// operations, in the same order, as the VM's radius_iter::local_to_world;
/// angles in wrapping 32-bit arithmetic as in the machine code.
pub fn local_to_world(rotation: [i32; 3], scale3d: [f32; 3], draw_scale: f32, location: [f32; 3], pre_pivot: [f32; 3]) -> [[f32; 4]; 4] {
    let (sr, cr) = (sin_u(rotation[2].wrapping_neg()), sin_u(0x4000i32.wrapping_sub(rotation[2])));
    let (sy, cy) = (sin_u(rotation[1]), sin_u(rotation[1].wrapping_add(0x4000)));
    let (sp, cp) = (sin_u(rotation[0].wrapping_neg()), sin_u(0x4000i32.wrapping_sub(rotation[0])));
    let mut m = [[0.0f32; 4]; 4];
    // cp × (DrawScale × DrawScale3D.X), then × cos / sin of the yaw.
    let a = cp * (draw_scale * scale3d[0]);
    m[0] = [a * cy, a * sy, -(draw_scale * scale3d[0] * sp), 0.0];
    let b = draw_scale * scale3d[1];
    m[1] = [(sp * sr * cy - sy * cr) * b, (cy * cr + sp * sr * sy) * b, b * cp * sr, 0.0];
    let c = draw_scale * scale3d[2];
    m[2] = [(sy * sr + sp * cr * cy) * c, (sp * cr * sy - cy * sr) * c, c * cp * cr, 0.0];
    let p = pre_pivot;
    // Origin: transformed Location − PrePivot, sums in machine-code order.
    m[3] = [
        location[0] - ((p[0] * m[0][0] + m[2][0] * p[2]) + p[1] * m[1][0]),
        location[1] - ((p[1] * m[1][1] + p[0] * m[0][1]) + m[2][1] * p[2]),
        location[2] - ((m[2][2] * p[2] + p[0] * m[0][2]) + p[1] * m[1][2]),
        1.0,
    ];
    m
}

/// FMatrix::TransformFVector: ((z · m[2][k] + y · m[1][k]) + x · m[0][k]) + m[3][k].
pub fn transform_point(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| ((v[2] * m[2][k] + v[1] * m[1][k]) + v[0] * m[0][k]) + m[3][k])
}

/// Inline transformation of the normal in CalculateLighting, same order
/// as TransformFVector, without translation.
pub fn transform_vector(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|k| (v[2] * m[2][k] + v[1] * m[1][k]) + v[0] * m[0][k])
}

/// UModel::BoxLeaves: BSP leaves touched by a box (min, max).
/// Render box of a placed StaticMesh (GetRenderBoundingBox then FBox::TransformBy, like the VM's
/// radius_iter::transform_box): centre (max + min) × 0.5 transformed, half-size through the
/// |axes|, not the 8 corners. Rounding matters for AActor::UpdateLeaves: a
/// box resting on a BSP plane (e.g. min Y −1088.0001 with the 8 corners) must not
/// spill into the neighbouring leaf, whose zone changes CalculateLighting's ambient light.
pub fn static_mesh_world_box(mn: [f32; 3], mx: [f32; 3], l2w: &[[f32; 4]; 4]) -> ([f32; 3], [f32; 3]) {
    let center = transform_point(l2w, [0, 1, 2].map(|k| (mx[k] + mn[k]) * 0.5));
    let half = [0, 1, 2].map(|k| (mx[k] - mn[k]) * 0.5);
    let ext = [0, 1, 2].map(|k| (l2w[2][k].abs() * half[2] + l2w[1][k].abs() * half[1]) + l2w[0][k].abs() * half[0]);
    let (lo, hi) = ([0, 1, 2].map(|k| center[k] - ext[k]), [0, 1, 2].map(|k| center[k] + ext[k]));
    // FBox +=: C + E into an empty box, then C − E.
    ([0, 1, 2].map(|k| if lo[k] < hi[k] { lo[k] } else { hi[k] }), [0, 1, 2].map(|k| if lo[k] > hi[k] { lo[k] } else { hi[k] }))
}

pub fn box_leaves(model: &Model, bmin: [f32; 3], bmax: [f32; 3]) -> Vec<i32> {
    let mut out = Vec::new();
    if model.nodes.is_empty() {
        return out;
    }
    let extent = [0, 1, 2].map(|k| (bmax[k] - bmin[k]) * 0.5);
    let center = [0, 1, 2].map(|k| extent[k] + bmin[k]);
    box_leaves_node(model, 0, center, extent, &mut out);
    out
}

fn box_leaves_node(model: &Model, mut node: usize, center: [f32; 3], extent: [f32; 3], out: &mut Vec<i32>) {
    loop {
        let n = &model.nodes[node];
        let p = n.plane;
        // (|Z| + |Y|) + |X| and (X + Z) + Y, as in bsp.rs.
        let push = ((p[2] * extent[2]).abs() + (extent[1] * p[1]).abs()) + (p[0] * extent[0]).abs();
        let dist = ((p[0] * center[0] + p[2] * center[2]) + p[1] * center[1]) - p[3];
        if dist < push {
            if n.links[0] == -1 {
                if n.leaf[0] != -1 {
                    out.push(n.leaf[0] as i32);
                }
            } else {
                box_leaves_node(model, n.links[0] as usize, center, extent, out);
            }
        }
        if dist <= -push {
            return;
        }
        if n.links[1] == -1 {
            if n.leaf[1] != -1 {
                out.push(n.leaf[1] as i32);
            }
            return;
        }
        node = n.links[1] as usize;
    }
}

/// Ambient light of a zone (ZoneInfo, or LevelInfo for zone 0 or without an actor): AmbientHue,
/// AmbientSaturation, AmbientBrightness.
#[derive(Debug, Clone, Copy, Default)]
pub struct ZoneAmbient {
    pub hue: u8,
    pub saturation: u8,
    pub brightness: u8,
}

/// Maximum components of the ambient lights of distinct zones (at most 4, in leaf
/// order), as in FDynamicActor::Refresh and CalculateLighting.
pub fn zones_ambient_max(zones: &[usize], ambient: &dyn Fn(usize) -> ZoneAmbient) -> [f32; 3] {
    zones_ambient_max_iter(zones.iter().copied(), ambient)
}

pub fn zones_ambient_max_iter(zones: impl IntoIterator<Item = usize>, ambient: &dyn Fn(usize) -> ZoneAmbient) -> [f32; 3] {
    zones_rgb_max_iter(zones, &|z| {
        let a = ambient(z);
        let c = fget_hsv(a.hue, a.saturation, a.brightness);
        [c[0], c[1], c[2]]
    })
}

/// Like `zones_ambient_max`, with each zone's RGB colour already computed.
pub fn zones_rgb_max(zones: &[usize], rgb: &dyn Fn(usize) -> [f32; 3]) -> [f32; 3] {
    zones_rgb_max_iter(zones.iter().copied(), rgb)
}

fn zones_rgb_max_iter(zones: impl IntoIterator<Item = usize>, rgb: &dyn Fn(usize) -> [f32; 3]) -> [f32; 3] {
    let mut seen = [0; 4];
    let mut count = 0;
    for z in zones {
        if !seen[..count].contains(&z) {
            seen[count] = z;
            count += 1;
            if count == seen.len() { break; }
        }
    }
    let mut m = [0.0f32; 3];
    for &z in &seen[..count] {
        let c = rgb(z);
        for k in 0..3 {
            if m[k] < c[k] {
                m[k] = c[k];
            }
        }
    }
    m
}

/// FColor(FPlane) as (R, G, B) of an ambient light plus AmbientGlow.
pub fn ambient_color(max: [f32; 3], glow: u8) -> [u8; 3] {
    let g = glow as f32 * INV_255;
    [plane_byte(g + max[0]), plane_byte(g + max[1]), plane_byte(g + max[2])]
}

/// Entry of a UStaticMeshInstance's light list (FStaticMeshLightInfo).
#[derive(Debug, Clone)]
pub struct MeshLightInfo {
    pub light: i16,
    pub visibility: Vec<u8>,
    pub applied: i32,
}

/// CalculateLighting: colour of each vertex (R, G, B) of a placed StaticMesh. `base` is the
/// starting colour (zone ambient and AmbientGlow for bIgnoreDynLight, black otherwise); the
/// instance's non-animated static lights are added where their visibility bit is set.
pub fn calculate_lighting(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    local_to_world: &[[f32; 4]; 4],
    base: [u8; 3],
    infos: &[MeshLightInfo],
    lights: &[Light],
) -> Vec<[u8; 3]> {
    let mut colors = vec![base; positions.len()];
    for info in infos {
        if info.light == -1 {
            continue;
        }
        let Some(light) = lights.get(info.light as usize) else { continue };
        if light.animated || light.dynamic {
            continue;
        }
        for (i, color) in colors.iter_mut().enumerate() {
            let Some(&byte) = info.visibility.get(i / 8) else { break };
            if byte & (1 << (i % 8)) == 0 {
                continue;
            }
            let p = transform_point(local_to_world, positions[i]);
            let n = safe_normal(transform_vector(local_to_world, normals[i]));
            let s = sample_intensity(light, p, n);
            let plane = [light.color[0] * 0.5 * s, light.color[1] * 0.5 * s, light.color[2] * 0.5 * s];
            *color = add_sat(*color, [plane_byte(plane[0]), plane_byte(plane[1]), plane_byte(plane[2])]);
        }
    }
    colors
}

/// Starting colour of a StaticMesh instance's vertices in CalculateLighting: black without
/// bIgnoreDynLight (0x2000000 of the actor) or with bNoAmbientLight (0x200000); otherwise the maximum of the ambient lights of the
/// actor's leaf zones (at most 4 distinct zones, no zone if the actor has no leaf), or of the pivot's zone with bUseOnlyPivotAmbient
/// (0x100000 of +0x34, no zone without a zone actor), plus AmbientGlow.
pub fn calculate_lighting_base(
    ignore_dyn: bool,
    no_ambient: bool,
    pivot_only: bool,
    leaf_zones: &[usize],
    pivot: Option<usize>,
    glow: u8,
    rgb: &dyn Fn(usize) -> [f32; 3],
) -> [u8; 3] {
    if !ignore_dyn || no_ambient {
        return [0; 3];
    }
    let zones: Vec<usize> = if pivot_only { pivot.into_iter().collect() } else { leaf_zones.to_vec() };
    ambient_color(zones_rgb_max(&zones, rgb), glow)
}
