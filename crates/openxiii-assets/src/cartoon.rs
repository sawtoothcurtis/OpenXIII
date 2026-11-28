//! XIII's "cartoon" rendering, as Engine.dll prepares it and D3DDrv.dll draws it.
//!
//! StaticMeshes whose UPrimitive has bytes +0x28 (outline) and +0x29 (cartoon) set are
//! drawn by FD3DRenderInterface::DrawCartoonStatic: texture × flat colour
//! (constant c4) × a 32 × 32 greyscale ramp, sampled without
//! filtering at coordinates N·L1 + I1 - 1 and N·L2 + I2 - 1 computed by the vertex shader
//! (or with the position as the normal), then a black outline: a second pass
//! of the back faces, vertices pushed along the normal (shader).
//! The colour, the two main lights and their intensity come, based
//! on the dynamic or animated lights touching the actor; the outline
//! thickness depends on the distance to the camera.

// Negated comparisons reproduce those of the original code (FCOM: NaN included).
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::needless_range_loop)]

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::lighting::Light;

/// FCartoonDesc.
#[derive(Debug, Clone, Default)]
pub struct CartoonDesc {
    /// +0x00: outline (UPrimitive+0x28).
    pub outline: bool,
    /// +0x01: cartoon ramp (UPrimitive+0x29).
    pub cartoon: bool,
    /// +0x02: normals taken from the position (UPrimitive+0x2A).
    pub position_normals: bool,
    /// +0x04: outline thickness factor (UPrimitive+0x2C).
    pub outline_factor: f32,
    /// +0x08: outline thickness, in mesh space.
    pub outline_width: f32,
    /// +0x0C.
    pub outline_ratio: f32,
    /// +0x10: colour (shader constant c4).
    pub color: [f32; 3],
    /// +0x1C, +0x20: intensity of the two main lights.
    pub i1: f32,
    pub i2: f32,
    /// +0x24, +0x30: their direction (mesh space).
    pub dir1: [f32; 3],
    pub dir2: [f32; 3],
    /// +0x3C: ZoneInfo.LightCurveShadow | LightCurveBright << 8.
    pub light_curve: u32,
    /// +0x40: ZoneInfo.AmbientIntensity / 255.
    pub ambient: f32,
    /// +0x44: average light direction.
    pub dir: [f32; 3],
}

fn max3(c: [f32; 4]) -> f32 {
    // Max of R and G (R if equal), then of B.
    let mut m = if c[0] < c[1] { c[1] } else { c[0] };
    if m < c[2] {
        m = c[2];
    }
    m
}

/// Colour and main lights of an actor at `pos`, from `lights` (in
/// 's priority order). `local_to_world` brings the directions back into
/// mesh space (multiplication by the transpose).
///
/// From the asm, all operations rounded to 24 bits (the game's FPU): distance
/// (Z² + Y²) + X², 1 / dist then products, strength
/// ((1 − (r·r)·r) × max) × 1.2 in single precision, cone (n × 1/32) × π/2 rounded
/// once, cos in double, Dir += d × strength in single precision, dir1:
/// (T[1][k]·y + T[2][k]·z) + T[0][k]·x, dir2: (T[0][k]·x + T[1][k]·y) + T[2][k]·z,
/// average direction: 1 / √((X² + Y²) + Z²) in single precision.
pub fn compute_lights(desc: &mut CartoonDesc, lights: &[&Light], m: &[[f32; 4]; 4], pos: [f32; 3]) {
    let fmul_d = |a: f32, b: f64| (a as f64 * b) as f32;
    let norm = |v: &mut [f32; 3]| {
        let sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        if (sq as f64) < 1e-8 {
            return;
        }
        let s = 1.0 / sq.sqrt();
        for c in v.iter_mut() {
            *c *= s;
        }
    };
    desc.dir = [0.0; 3];
    desc.i1 = 0.0;
    desc.i2 = 0.0;
    let (mut main, mut second) = ([0.0f32; 3], [0.0f32; 3]);
    for l in lights {
        let (strength, dir);
        if l.effect == 0x14 {
            dir = [-l.direction[0], -l.direction[1], -l.direction[2]];
            strength = max3(l.color);
        } else {
            let d = [l.location[0] - pos[0], l.location[1] - pos[1], l.location[2] - pos[2]];
            let dist = (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt();
            if !(dist < l.radius) {
                continue;
            }
            let inv = 1.0 / dist;
            let nd = [d[0] * inv, d[1] * inv, d[2] * inv];
            let r = dist / l.radius;
            if l.effect == 0xC || l.effect == 8 {
                let att = 1.0 - r * r * r;
                let cosang = -(nd[2] * l.direction[2] + nd[1] * l.direction[1] + nd[0] * l.direction[0]);
                if !(cosang > 0.0) {
                    continue;
                }
                let mut cone = fmul_d(l.cone as f32 * 0.03125, 1.570_796_370_506_286_6);
                if !(cone <= std::f32::consts::PI) {
                    cone = std::f32::consts::PI;
                }
                let c = ((cone * 0.5) as f64).dcos();
                let spot = ((cosang as f64 - c) as f32 / (1.0 - c) as f32) * att;
                strength = spot * max3(l.color) * 1.2;
            } else {
                strength = (1.0 - r * r * r) * max3(l.color) * 1.2;
            }
            dir = nd;
        }
        if l.flags & 0x1000 == 0 {
            for k in 0..3 {
                desc.dir[k] += dir[k] * strength;
            }
        }
        if strength > desc.i1 {
            desc.i2 = desc.i1;
            desc.i1 = strength;
            desc.dir2 = desc.dir1;
            desc.dir1 = dir;
            second = main;
            main = [l.color[0], l.color[1], l.color[2]];
        } else if strength > desc.i2 {
            desc.i2 = strength;
            desc.dir2 = dir;
            second = [l.color[0], l.color[1], l.color[2]];
        }
    }
    desc.color = [desc.ambient; 3];
    if desc.i1 == 0.0 {
        desc.dir = [0.0, 0.0, 1.0];
        return;
    }
    for k in 0..3 {
        desc.color[k] += main[k] * desc.i1 * 3.0;
    }
    let d = desc.dir1;
    desc.dir1 = [0, 1, 2].map(|k| (m[k][1] * d[1] + m[k][2] * d[2]) + m[k][0] * d[0]);
    norm(&mut desc.dir1);
    if desc.i2 != 0.0 {
        for k in 0..3 {
            desc.color[k] += second[k] * desc.i2 * 3.0;
        }
        let d = desc.dir2;
        desc.dir2 = [0, 1, 2].map(|k| (m[k][0] * d[0] + m[k][1] * d[1]) + m[k][2] * d[2]);
        norm(&mut desc.dir2);
    }
    let len = (desc.dir[0] * desc.dir[0] + desc.dir[1] * desc.dir[1] + desc.dir[2] * desc.dir[2]).sqrt();
    if len > 0.0 {
        let inv = 1.0 / len;
        for c in desc.dir.iter_mut() {
            *c *= inv;
        }
    } else {
        desc.dir = [0.0, 0.0, 1.0];
    }
}

/// Distance → thickness table (6 points).
const WIDTH_X: [f32; 6] = [0.0, 50.0, 100.0, 200.0, 400.0, 800.0];
const WIDTH_Y: [f32; 6] = [0.8, 0.7, 0.6, 0.5, 0.4, 0.3];

/// Outline thickness for a camera distance `cam_dist` and the tangent
/// of the half field of view; `scale` = |DrawScale| × max |DrawScale3D| of the actor; `boost` is
/// (1, or 3 during URenderTargetMaterial::Update).
pub fn outline_width(desc: &mut CartoonDesc, cam_dist: f32, tan_half_fov: f32, scale: Option<f32>, boost: f32) {
    let f = cam_dist * tan_half_fov;
    let n = WIDTH_X.len();
    let mut i = 1;
    while i < n - 2 {
        if f < WIDTH_X[i] {
            break;
        }
        i += 1;
    }
    let y = (f - WIDTH_X[i]) * ((WIDTH_Y[i + 1] - WIDTH_Y[i]) / (WIDTH_X[i + 1] - WIDTH_X[i])) + WIDTH_Y[i];
    desc.outline_ratio = y;
    if !(y > 0.0) {
        desc.outline_width = 0.0;
        desc.outline = false;
        return;
    }
    // = 0.01.
    let mut r = y * desc.outline_factor * 0.01;
    if tan_half_fov < 0.8 {
        r *= boost;
    }
    desc.outline_ratio = r;
    desc.outline_width = f * r;
    if let Some(s) = scale {
        let inv = 1.0 / s;
        desc.outline_ratio *= inv;
        desc.outline_width *= inv;
    }
}

/// 32 × 32 ramp (A8R8G8B8) of a lighting curve: texel (0, 0) = shadow,
/// first row and first column = bright, the rest = 2 × bright - shadow (clamped to 255).
pub fn ramp(curve: u32) -> Vec<u8> {
    let shadow = curve & 0xFF;
    let bright = (curve >> 8) & 0xFF;
    let mut out = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u32 {
        for x in 0..32u32 {
            let v = if x == 0 && y == 0 {
                shadow
            } else if x == 0 || y == 0 {
                bright
            } else {
                let v = (bright * 2).wrapping_sub(shadow);
                if v > 0xFF {
                    0xFF
                } else {
                    v
                }
            };
            out.extend_from_slice(&[v as u8, v as u8, v as u8, 255]);
        }
    }
    out
}

/// Actor for light selection (FDynamicActor).
pub struct LightTarget<'a> {
    /// Actor+0x1A1 (bSLightGroup): light flag bit to test.
    pub light_group: u8,
    /// Zones of the actor's leaves.
    pub leaf_zones: &'a [i32],
    /// Lights affecting each leaf (Model::leaf_lights), in `leaf_zones` order.
    pub leaf_lights: &'a [Vec<usize>],
    /// Render bounding sphere (FDynamicActor+0x64..+0x70).
    pub sphere: [f32; 4],
}

/// Active lights (dynamic or animated) touching the actor, sorted by
/// decreasing priority ((1 - d²/r²) × LightBrightness × k; 0x7FFFFFFF for the sun).
pub fn select_lights<'a>(target: &LightTarget, lights: &'a [Light]) -> Vec<&'a Light> {
    select_scene_candidates(target, lights, true)
}

/// Same comparisons and sort; the lists have already been distributed by
/// Scene. Do not add all the dynamic lights a second time.
pub fn select_distributed_lights<'a>(target: &LightTarget, lights: &'a [Light]) -> Vec<&'a Light> {
    select_scene_candidates(target, lights, false)
}

fn select_scene_candidates<'a>(target: &LightTarget, lights: &'a [Light], legacy_dynamic: bool) -> Vec<&'a Light> {
    let mut list: Vec<(i32, &Light)> = Vec::new();
    for (k, zone) in target.leaf_zones.iter().enumerate() {
        // Leaf list: lights that affect it, then dynamic or animated lights
        // distributed by the BSP.
        let permeating = target.leaf_lights.get(k).map(|v| v.as_slice()).unwrap_or(&[]);
        let candidates = permeating.iter().filter_map(|&i| lights.get(i)).chain(lights.iter().filter(|l| legacy_dynamic && (l.dynamic || l.animated)));
        for l in candidates {
            if (l.flags & 0xFFFF) & (1u32 << (target.light_group & 0x1F)) == 0 {
                continue;
            }
            if l.flags & 0x2000 != 0 && l.zone as i32 != *zone {
                continue;
            }
            let d = [l.location[0] - target.sphere[0], l.location[1] - target.sphere[1], l.location[2] - target.sphere[2]];
            // (Z² + Y²) + X² in both places (range, priority).
            let d2 = d[2] * d[2] + d[1] * d[1] + d[0] * d[0];
            if l.effect != 0x14 && !(d2 <= l.radius * l.radius + target.sphere[3] * target.sphere[3]) {
                continue;
            }
            if list.iter().any(|(_, x)| std::ptr::eq(*x, l)) {
                continue;
            }
            let priority = if l.effect == 0x14 {
                0x7FFF_FFFF
            } else {
                ((1.0 - d2 / (l.radius * l.radius)) * l.brightness as f32 * PRIORITY_SCALE).round_ties_even() as i32
            };
            // Inserted before the first one whose priority is not higher.
            let at = list.iter().position(|(p, _)| *p <= priority).unwrap_or(list.len());
            list.insert(at, (priority, l));
        }
    }
    list.into_iter().map(|(_, l)| l).collect()
}

/// FActorSceneNode::Render: source indices only, without zone
/// filtering or adding dynamic lights. Distance (X² + Y²) + Z², unlike
/// the Scene selector. Attached actors use their base's list.
pub fn select_actor_scene_lights<'a>(target: &LightTarget, lights: &'a [Light]) -> Vec<&'a Light> {
    let mut list: Vec<(i32, &Light)> = Vec::new();
    for indices in target.leaf_lights {
        for light in indices.iter().filter_map(|&i| lights.get(i)) {
            if (light.flags & 0xFFFF) & (1u32 << (target.light_group & 0x1F)) == 0 {
                continue;
            }
            let d = [light.location[0] - target.sphere[0], light.location[1] - target.sphere[1], light.location[2] - target.sphere[2]];
            let d2 = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
            if light.effect != 0x14 && !(d2 <= light.radius * light.radius + target.sphere[3] * target.sphere[3]) {
                continue;
            }
            if list.iter().any(|(_, l)| std::ptr::eq(*l, light)) {
                continue;
            }
            let priority = if light.effect == 0x14 {
                0x7FFF_FFFF
            } else {
                ((1.0 - d2 / (light.radius * light.radius)) * light.brightness as f32 * PRIORITY_SCALE).round_ties_even() as i32
            };
            let at = list.iter().position(|(p, _)| *p <= priority).unwrap_or(list.len());
            list.insert(at, (priority, light));
        }
    }
    list.into_iter().map(|(_, l)| l).collect()
}

/// (see `select_lights`).
pub const PRIORITY_SCALE: f32 = 1024.0;

/// Animated actor drawn by DrawCartoonSM (DrawCartoonStatic for a StaticMesh): mesh
/// (for the UPrimitive+0x28 bytes), Actor.bSLightGroup, position (zone, camera distance) and
/// render bounding sphere.
#[derive(Debug, Clone)]
pub struct CharacterCartoon {
    pub mesh: String,
    pub light_group: u8,
    pub location: [f32; 3],
    pub sphere: [f32; 4],
    /// Actor.Leaves (Actor+0xA4, AActor::UpdateLeaves): leaves whose lights takes.
    pub leaves: Vec<i32>,
    /// Actor.bUnlit or, in the game, Actor.bIgnoreDynLight: gives no light.
    pub unlit: bool,
}

