//! Posing of skeletal meshes, reimplemented from USkeletalMeshInstance::GetFrame (Engine.dll): sampling of each channel's tracks, starting bone masks,
//! entry transition from the cached pose, channel blending by BlendAlpha (AnimBlendParams
//! fade-in included), reference pose for non-animated bones, reference position
//! forced by the skeleton's flags, locked root (LockRootMotion).
//!
//! The engine keeps per instance the local pose of each bone (quaternion at +0x1FC, position at
//! +0x208): it is the starting point for the next frame's transitions and blends. The pose
//! is only recomputed once per tick (GTicks at +0x68), as in the game.
//!
//! The computations keep the machine code's order and constants: compressed quaternions
//! × 3.0518509e-5, key difference threshold 1e-4, normalization above 1e-5 (otherwise
//! (0, 0, 0.1, 0)), spherical interpolation without internal flipping.
// The loops keep the engine's bone indices.
#![allow(clippy::needless_range_loop)]

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::anim::{AnimSet, Channel, MeshInfo, MeshInstance};
use crate::value::ObjId;
use crate::Vm;
use openxiii_assets::animation::Move;
use openxiii_assets::coords::{self, Coords, UNIT};
use std::rc::Rc;

pub type Quat = [f32; 4];

/// Local pose of a bone: rotation and position in the parent's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BonePose {
    pub rotation: Quat,
    pub position: [f32; 3],
}

/// Mapping mesh bone → animation bone (+0x88, 0x14 bytes per MeshAnimation object).
#[derive(Debug, Clone, Default)]
struct Link {
    /// Animation for which the table was built (+0x0C).
    anim: Option<String>,
    /// Index of the bone in the animation's RefBones, -1 without a match.
    map: Vec<i32>,
}

crate::improved::presentation::pose_state! {
/// GetFrame state specific to an instance.
#[derive(Debug, Clone, Default)]
pub struct PoseState {
    /// +0x1FC, +0x208: cached local pose, one entry per reference bone.
    pub cache: Vec<BonePose>,
    /// +0x230: the cached pose is valid (at least one full computation).
    pub valid: bool,
    /// +0x68: tick of the last computation (here the level time, which advances every tick).
    pub(crate) last_tick: Option<u32>,
    links: Vec<Link>,
    /// +0x288: reference pose forced for all bones.
    pub force_ref_pose: bool,
    /// +0x98: locked root (LockRootMotion), position kept at +0x104.
    pub root_lock: i32,
    pub root_locked: [f32; 3],
    /// +0xFC: root to recompute (UpdateAnimation, LockRootMotion) by GetFrame(GF_RootOnly).
    pub root_dirty: bool,
    /// +0x9C RootLocation, +0xA8 RootRotation: root in the world (last GF_RootOnly).
    pub root_location: [f32; 3],
    pub root_rotation: [i32; 3],
    /// +0xB4 LastRootLocation, +0xC0 LastRootRotation.
    pub last_root_location: [f32; 3],
    pub last_root_rotation: [i32; 3],
    /// +0xCC: mesh → world frame kept by LockRootMotion (without the mesh's Origin).
    pub root_ref: Coords,
    /// +0x164 / +0x194: local frame of the root, then this frame composed with +0xCC.
    pub root_local: Coords,
    pub root_world: Coords,
    /// +0x104 / +0x134: the same, kept by LockRootMotion.
    pub locked_local: Coords,
    pub locked_world: Coords,
    /// SpaceBases of the last frame (+0x290): frame of each bone in the mesh's frame.
    pub bases: Vec<Coords>,
    /// +0x1D0: per-bone scales (SetBoneScale), 0x44 bytes.
    pub scalers: Vec<Scaler>,
    /// +0x1DC: local rotations and translations (SetBoneRotation, SetBoneLocation), 0x2C bytes.
    pub directors: Vec<Director>,
    /// +0x1E8: directions in the mesh's frame (SetBoneDirection), 0x2C bytes.
    pub mesh_dirs: Vec<Director>,
    /// +0x2A8: scale frame of the mesh (DrawScale × Scale × DrawScale3D, origin Origin).
    pub scale_coords: Coords,
}
}

#[derive(Debug, Clone, Default)]
pub struct Scaler {
    pub bone: i32,
    pub name: String,
    pub scale: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub struct Director {
    pub bone: i32,
    pub name: String,
    /// +0x08: rotator (pitch, yaw, roll).
    pub rotation: [i32; 3],
    /// +0x14.
    pub translation: [f32; 3],
    /// +0x20: space requested from SetBoneRotation (not read by GetFrame).
    pub space: i32,
    /// +0x24, +0x28.
    pub rot_alpha: f32,
    pub trans_alpha: f32,
}

const ZERO: f32 = 0.0;
const ONE: f32 = 1.0;
/// 1/32767 in single precision.
const QUAT_SCALE: f32 = 3.051_851e-5;
const KEY_EPSILON: f32 = 1e-4;
const NORMAL_EPSILON: f32 = 1e-5;

/// Compressed quaternion (4 × i16).
fn unpack(q: [i16; 4]) -> Quat {
    q.map(|c| c as i32 as f32 * QUAT_SCALE)
}

/// Returns `a` negated if −a is closer to `b`.
fn align(a: &mut Quat, b: &Quat) {
    let plus = (a[0] + b[0]) * (a[0] + b[0]) + (b[1] + a[1]) * (b[1] + a[1]) + (b[2] + a[2]) * (b[2] + a[2]) + (a[3] + b[3]) * (a[3] + b[3]);
    let minus = (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2]) + (a[3] - b[3]) * (a[3] - b[3]);
    if plus < minus {
        *a = a.map(|c| -c);
    }
}

/// Spherical interpolation from `a` to `b`, without flipping; `a` if cos ≥ 1.
/// cos = ((a0·b0 + a2·b2) + a1·b1) + b3·a3 (machine-code order). θ = appAcos(cos) (MSVCR70 _CIacos, which keeps the caller's 24-bit precision:
/// atan2(√((1 + c)(1 − c)), c), each operation rounded to 24 bits, fpatan at full precision)
/// stays on the FPU stack: sin(θ) (appSin, FSIN) receives it as a double, the two other
/// sines receive θ stored as a float; each sine is multiplied by 1 / sin(θ) before rounding.
fn slerp(a: &Quat, b: &Quat, t: f32) -> Quat {
    let cos = ((a[0] * b[0] + a[2] * b[2]) + a[1] * b[1]) + b[3] * a[3];
    if cos < ONE {
        let root = ((ONE + cos) * (ONE - cos)).sqrt();
        let theta64 = (root as f64).datan2(cos as f64);
        let theta = theta64 as f32;
        let inv = (1.0 / theta64.dsin()) as f32;
        let wa = ((((ONE - t) * theta) as f64).dsin() * inv as f64) as f32;
        let wb = (((theta * t) as f64).dsin() * inv as f64) as f32;
        return [wb * b[0] + wa * a[0], wb * b[1] + wa * a[1], wb * b[2] + wa * a[2], wa * a[3] + wb * b[3]];
    }
    *a
}

/// Normalization (appRSqrt); quaternion (0, 0, 0.1, 0) below the threshold.
fn normalize(q: &mut Quat) {
    let len2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if len2 >= NORMAL_EPSILON {
        let r = 1.0 / len2.sqrt();
        *q = q.map(|c| r * c);
    } else {
        *q = [0.0, 0.0, 0.1, 0.0];
    }
}

/// Pose of a track at time `t` (seconds of the track), without root
/// tracking (last argument zero in GetFrame).
fn sample(mv: &Move, track: usize, t: f32) -> Option<BonePose> {
    let tr = mv.tracks.get(track)?;
    let (qs, n, ps, pn) = (tr.quat_start as usize, tr.quat_count as i32, tr.pos_start as usize, tr.pos_count as usize);
    let times = mv.times.get(qs..qs + n as usize)?;
    let quats = mv.raw_quats.get(qs..qs + n as usize)?;
    if n == 0 {
        // Track without a key: the engine would read before the array; there is none in the game's data.
        return None;
    }
    let mut k: i32 = -1;
    let mut found = false;
    for i in 1..n {
        if t < times[i as usize] {
            k = i - 1;
            found = true;
            break;
        }
    }
    if !found && t < mv.track_time {
        k = n - 1;
    }
    if k < 0 {
        // t = TrackTime (frame 1, or 0 backwards): the engine would read key −1. Unreachable
        // in game (AnimLast < 1); we take the last key, with wrap-around.
        k = n - 1;
    }
    let before_last = k < n - 1;
    let mut next = if before_last { k + 1 } else { 0 };
    let wrap = !before_last;
    if n == 1 {
        k = 0;
        next = 0;
    }
    let (k, next) = (k as usize, next as usize);
    let mut span = if wrap {
        mv.track_time - times[k]
    } else {
        let d = times[next] - times[k];
        if d < ZERO {
            -d
        } else {
            d
        }
    };
    let alpha = if span <= KEY_EPSILON {
        span = 1.0;
        span
    } else {
        (t - times[k]) / span
    };
    let qa = unpack(quats[k]);
    let mut qb = unpack(quats[next]);
    if wrap {
        align(&mut qb, &qa);
    }
    let mut rotation = slerp(&qa, &qb, alpha);
    normalize(&mut rotation);
    let positions = mv.positions.get(ps..)?;
    let position = if pn < 2 {
        *positions.first()?
    } else {
        let (a, b) = (*positions.get(k)?, *positions.get(next)?);
        [(b[0] - a[0]) * alpha + a[0], (b[1] - a[1]) * alpha + a[1], (b[2] - a[2]) * alpha + a[2]]
    };
    Some(BonePose { rotation, position })
}

/// For each bone of the mesh, the first bone of the animation with the same name.
fn match_bones(info: &MeshInfo, set: &AnimSet) -> Vec<i32> {
    info.bones.iter().map(|b| set.bones.iter().position(|a| a.eq_ignore_ascii_case(b)).map_or(-1, |i| i as i32)).collect()
}

/// Entry of the list of poses to apply (0x28 bytes).
struct Entry {
    pose: BonePose,
    bone: usize,
    channel: usize,
}

fn ref_pose(info: &MeshInfo, bone: usize) -> BonePose {
    let b = &info.ref_bones[bone];
    BonePose { rotation: b.orientation, position: b.position }
}

/// Mask of the bones governed by a channel: the starting bone and its descendants.
fn bone_mask(info: &MeshInfo, start_bone: i32) -> Vec<bool> {
    let mut mask = vec![false; info.ref_bones.len()];
    for bone in 0..mask.len() {
        mask[bone] = if start_bone == bone as i32 {
            true
        } else {
            let parent = info.ref_bones[bone].parent;
            bone != 0 && parent >= 0 && mask.get(parent as usize).copied().unwrap_or(false)
        };
    }
    mask
}

/// Poses of the channels (first part of GetFrame): samples or transitions.
fn channel_entries(info: &MeshInfo, anims: &[Rc<AnimSet>], channels: &mut [Channel], pose: &mut PoseState, set: &mut [u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let nbones = info.ref_bones.len();
    for ch in 0..channels.len() {
        let c = &mut channels[ch];
        let alpha = if ch == 0 { ONE } else { c.blend_alpha };
        if c.anim_index < 0 || alpha <= ZERO || alpha.is_nan() {
            continue;
        }
        let ai = c.anim_index as usize;
        let (Some(link), Some(anim)) = (pose.links.get(ai), anims.get(ai)) else {
            continue;
        };
        if link.map.is_empty() {
            continue;
        }
        let tweening = c.frame < ZERO && pose.valid;
        let Some(mv) = anim.movement(&c.sequence) else {
            continue;
        };
        let seq = anim.seq(&c.sequence).map(|i| &anim.seqs[i]);
        let mut t = ZERO;
        if seq.is_some() {
            let f = if c.reversed { ONE - c.frame } else { c.frame };
            if ZERO <= f {
                t = if f >= ONE { ONE } else { f };
            }
            t *= mv.track_time;
        }
        if tweening {
            let Some(seq) = seq else {
                if pose.valid {
                    set.iter_mut().for_each(|s| *s = 2);
                }
                continue;
            };
            let mut tw = ONE - c.frame / c.tween_base;
            let base = -1.0 / seq.num_frames as f32;
            if !c.tween_seq.eq_ignore_ascii_case(&c.sequence) || tw < ZERO || ONE < tw {
                c.tween_base = base;
                tw = 0.0;
                c.tween_seq = c.sequence.clone();
            } else {
                c.tween_base = c.frame;
            }
            let mask = bone_mask(info, c.start_bone);
            for bone in 0..nbones {
                let track = link.map[bone];
                if !mask[bone] || track < 0 {
                    continue;
                }
                let Some(tr) = mv.tracks.get(track as usize) else { continue };
                let (qs, qn, ps, pn) = (tr.quat_start as usize, tr.quat_count as usize, tr.pos_start as usize, tr.pos_count as usize);
                let (qi, pi) = if c.reversed { ((qs + qn).wrapping_sub(1), (ps + pn).wrapping_sub(1)) } else { (qs, ps) };
                let (Some(&q), Some(&p)) = (mv.raw_quats.get(qi), mv.positions.get(pi)) else { continue };
                let mut target = unpack(q);
                let cur = pose.cache[bone];
                align(&mut target, &cur.rotation);
                let mut rotation = slerp(&cur.rotation, &target, tw);
                normalize(&mut rotation);
                let position = [
                    tw * (p[0] - cur.position[0]) + cur.position[0],
                    cur.position[1] + tw * (p[1] - cur.position[1]),
                    cur.position[2] + tw * (p[2] - cur.position[2]),
                ];
                entries.push(Entry { pose: BonePose { rotation, position }, bone, channel: ch });
            }
        } else if !mv.tracks.is_empty() {
            let mask = bone_mask(info, c.start_bone);
            for bone in 0..nbones {
                let track = link.map[bone];
                if !mask[bone] || track < 0 {
                    continue;
                }
                if let Some(p) = sample(mv, track as usize, t) {
                    entries.push(Entry { pose: p, bone, channel: ch });
                }
            }
        }
    }
    entries
}

/// GetFrame, skeleton part: final local pose of each bone (parent's frame).
pub(crate) fn compute_pose(info: &MeshInfo, anims: &[Rc<AnimSet>], channels: &mut Vec<Channel>, pose: &mut PoseState, tick: u32) -> Vec<Coords> {
    compute_pose_render(info, anims, channels, pose, tick, true)
}

/// `compute_pose`, without copying the kept frames when the result is not read (`render` false)
/// and the pose for this tick is already done: same effects on the instance, empty Vec returned.
pub(crate) fn compute_pose_render(info: &MeshInfo, anims: &[Rc<AnimSet>], channels: &mut Vec<Channel>, pose: &mut PoseState, tick: u32, render: bool) -> Vec<Coords> {
    let nbones = info.ref_bones.len();
    if channels.is_empty() {
        channels.push(Channel::default());
    }
    let same_tick = pose.last_tick == Some(tick);
    pose.last_tick = Some(tick);
    let mut fresh = false;
    if pose.cache.is_empty() {
        pose.cache = vec![BonePose { rotation: [0.0; 4], position: [0.0; 3] }; nbones];
        channels.iter_mut().for_each(|c| c.tween_base = 0.0);
    }
    if pose.links.len() <= anims.len() {
        pose.links.resize(anims.len() + 1, Link::default());
    }
    for (i, a) in anims.iter().enumerate() {
        if pose.links[i].anim.as_deref() != Some(a.path.as_str()) {
            pose.links[i] = Link { anim: Some(a.path.clone()), map: match_bones(info, a) };
        }
    }
    if pose.bases.is_empty() {
        fresh = true;
    }
    if same_tick && !fresh {
        return if render { pose.bases.clone() } else { Vec::new() };
    }
    let mut set = vec![0u8; nbones];
    let entries = channel_entries(info, anims, channels, pose, &mut set);
    for e in &entries {
        let b = e.bone;
        if e.channel == 0 {
            pose.cache[b] = e.pose;
            set[b] = 1;
            continue;
        }
        let c = &channels[e.channel];
        let mut a = c.blend_alpha;
        // AnimBlendParams fade-in (InTime): alpha proportional to the frame.
        let mut check_set = false;
        if c.blend_in > ZERO {
            if ZERO <= c.frame {
                if c.frame < c.blend_in {
                    a *= c.frame / c.blend_in;
                }
                check_set = a < ONE;
            } else {
                a = 0.0;
                check_set = true;
            }
        } else if a < ONE {
            check_set = true;
        }
        let init = if check_set && set[b] == 0 { true } else { pose.force_ref_pose };
        if init {
            pose.cache[b] = ref_pose(info, b);
            set[b] = 1;
        }
        if a == ZERO {
            continue;
        }
        if a < ONE {
            let cur = pose.cache[b];
            let mut target = e.pose.rotation;
            align(&mut target, &cur.rotation);
            let mut rotation = slerp(&cur.rotation, &target, a);
            normalize(&mut rotation);
            let p = e.pose.position;
            let position = [
                a * (p[0] - cur.position[0]) + cur.position[0],
                a * (p[1] - cur.position[1]) + cur.position[1],
                a * (p[2] - cur.position[2]) + cur.position[2],
            ];
            pose.cache[b] = BonePose { rotation, position };
        } else {
            pose.cache[b] = e.pose;
            set[b] = 1;
        }
    }
    // Local frames (SpaceBases before composition): quaternion → axes, position from the cache or from
    // the reference depending on the bone's flag 1, locked root.
    let mut bases = Vec::with_capacity(nbones);
    for b in 0..nbones {
        if set[b] == 0 || pose.force_ref_pose {
            pose.cache[b] = ref_pose(info, b);
        }
        let r = &info.ref_bones[b];
        let position = if r.flags & 1 == 0 { pose.cache[b].position } else { r.position };
        let mut base = coords::from_quat(pose.cache[b].rotation, position);
        if b == 0 && pose.root_lock != 0 {
            base.origin = pose.root_locked;
        }
        bases.push(base);
    }
    pose.valid = true;
    // Per-bone scales (SetBoneScale): axes multiplied.
    for sc in &pose.scalers {
        if sc.bone >= 0 && (sc.bone as usize) < nbones {
            let b = &mut bases[sc.bone as usize];
            b.x = b.x.map(|c| sc.scale[0] * c);
            b.y = b.y.map(|c| sc.scale[1] * c);
            b.z = b.z.map(|c| sc.scale[2] * c);
        }
    }
    // Local rotations and translations (SetBoneRotation, SetBoneLocation).
    for d in &pose.directors {
        if d.bone < 0 || d.bone as usize >= nbones {
            continue;
        }
        let b = d.bone as usize;
        if d.rot_alpha > ZERO {
            let rot = scaled_rotator(d.rotation, d.rot_alpha);
            let turn = UNIT.mul(&coords::div_rotator(&UNIT, rot));
            let origin = bases[b].origin;
            bases[b] = turn.mul(&bases[b].transpose());
            bases[b].origin = origin;
        }
        if d.trans_alpha > ZERO {
            let a = d.trans_alpha;
            let t = d.translation;
            let o = &mut bases[b].origin;
            *o = [a * t[0] + o[0], a * t[1] + o[1], a * t[2] + o[2]];
        }
    }
    // Composition with the parents; directions in the mesh's frame (SetBoneDirection)
    // applied to the bone once composed.
    for i in 1..nbones {
        let parent = info.ref_bones[i].parent as usize;
        bases[i] = bases[i].compose_bone(&bases[parent]);
        for d in &pose.mesh_dirs {
            if d.bone != i as i32 {
                continue;
            }
            if d.rot_alpha > ZERO {
                let rot = scaled_rotator(d.rotation, d.rot_alpha);
                let turn = coords::div_rotator(&UNIT, rot);
                let mut m = pose.scale_coords.transpose().mul(&turn);
                // Axes normalized if |axis|² ≥ 1e-8: X as (X² + Y²) + Z², Y and Z as (Z² + Y²) + X²
                for (k, axis) in [&mut m.x, &mut m.y, &mut m.z].into_iter().enumerate() {
                    let len2 = if k == 0 {
                        (axis[0] * axis[0] + axis[1] * axis[1]) + axis[2] * axis[2]
                    } else {
                        (axis[2] * axis[2] + axis[1] * axis[1]) + axis[0] * axis[0]
                    };
                    if len2 as f64 >= 1e-8 {
                        let r = 1.0 / len2.sqrt();
                        *axis = axis.map(|c| c * r);
                    }
                }
                m.origin = bases[i].origin;
                bases[i] = m;
            }
            if d.trans_alpha > ZERO {
                let w = coords::transform_vector(d.translation, &pose.scale_coords.pivot_inverse());
                let a = d.trans_alpha;
                let o = &mut bases[i].origin;
                *o = [w[0] * a + o[0], w[1] * a + o[1], w[2] * a + o[2]];
            }
        }
    }
    crate::improved::presentation::finish_pose(pose, info, &mut bases);
    pose.bases = bases.clone();
    bases
}

/// Rotator of a controller multiplied by its alpha, components truncated (_ftol).
fn scaled_rotator(r: [i32; 3], alpha: f32) -> [i32; 3] {
    r.map(|c| (c as f32 * alpha) as i32)
}

/// World → mesh frame of the actor (+0x2A8, start of GetFrame): origin −(Location +
/// PrePivot), Rotation, the mesh's RotOrigin, scale (DrawScale × Scale × DrawScale3D),
/// then the mesh's Origin.
fn actor_coords(vm: &Vm, actor: ObjId, info: &MeshInfo) -> Coords {
    let mut c = actor_coords_no_origin(vm, actor, info);
    c.origin = [info.origin[0] + c.origin[0], info.origin[1] + c.origin[1], info.origin[2] + c.origin[2]];
    c
}

/// The same frame without the mesh's Origin, which LockRootMotion keeps at +0xCC.
fn actor_coords_no_origin(vm: &Vm, actor: ObjId, info: &MeshInfo) -> Coords {
    let location = vm.propr(actor, "Location").as_vector();
    let pre_pivot = vm.propr(actor, "PrePivot").as_vector();
    let rotation = vm.propr(actor, "Rotation").as_rotator();
    let draw_scale = vm.propr(actor, "DrawScale").as_float();
    let ds3 = vm.propr(actor, "DrawScale3D").as_vector();
    frame_actor(location, pre_pivot, rotation, draw_scale, ds3, info)
}

/// actor_coords_no_origin on the actor's fields.
pub(crate) fn frame_actor(location: [f32; 3], pre_pivot: [f32; 3], rotation: [i32; 3], draw_scale: f32, ds3: [f32; 3], info: &MeshInfo) -> Coords {
    let mut c = UNIT;
    // 0.0 − V like FCoords::operator- (fld origin; fsub V), not −V: the origin is +0, not −0.
    c.origin = [0.0 - (location[0] + pre_pivot[0]), 0.0 - (pre_pivot[1] + location[1]), 0.0 - (pre_pivot[2] + location[2])];
    c = coords::mul_rotator(&c, rotation);
    c = coords::mul_rotator(&c, info.rot_origin);
    // FCoords / FScale: product by the unit frame with zero shear,
    // which turns −0 into +0, then the scale.
    c = c.mul(&UNIT);
    let scale = [draw_scale * info.scale[0] * ds3[0], draw_scale * info.scale[1] * ds3[1], draw_scale * info.scale[2] * ds3[2]];
    for axis in [&mut c.x, &mut c.y, &mut c.z] {
        *axis = [axis[0] * scale[0], axis[1] * scale[1], scale[2] * axis[2]];
    }
    c.origin = [c.origin[0] / scale[0], c.origin[1] / scale[1], c.origin[2] / scale[2]];
    c
}

/// MatchRefBone: reference bone with this name (None: −1).
pub(crate) fn match_ref_bone(info: &MeshInfo, name: &str) -> i32 {
    if name.is_empty() {
        return -1;
    }
    // First attachment alias with this name: its target bone.
    let name = info.aliases.iter().find(|(a, _)| a.eq_ignore_ascii_case(name)).map_or(name, |(_, t)| t.as_str());
    info.bones.iter().position(|b| b.eq_ignore_ascii_case(&name)).map_or(-1, |i| i as i32)
}

/// Entry of a controller list (SetBoneLocation, SetBoneRotation, SetBoneDirection):
/// that of the named bone, otherwise a new one (at most 256); `fresh` resets to zero the alpha that
/// creation initializes.
/// USkeletalMeshInstance::SetBoneDirection on the list +0x1E8: at most 0x100
/// entries, bone found by MatchRefBone, entry with the same name reused (its bone is not recomputed).
pub(crate) fn direction_os(list: &mut Vec<Director>, info: &MeshInfo, name: &str, rotation: [i32; 3], translation: [f32; 3], alpha: f32) -> bool {
    let Some(d) = director_slot(list, info, name, |d| d.trans_alpha = 0.0) else { return false };
    d.space = 0;
    d.rot_alpha = alpha;
    d.trans_alpha = alpha;
    d.rotation = rotation;
    d.translation = translation;
    true
}

/// USkeletalMeshInstance::GetBoneRotation by index, the pose being up to date: rotator
/// (OrthoRotation) of Transpose(SpaceBases[bone]) × frame +0x2A8;
/// index outside SpaceBases: zero rotator.
pub(crate) fn bone_rotation_world(pose: &PoseState, bone: usize) -> [i32; 3] {
    let Some(b) = pose.bases.get(bone) else { return [0; 3] };
    let r = b.transpose().mul(&pose.scale_coords);
    crate::world::ortho_rotation([r.x, r.y, r.z])
}

/// USkeletalMeshInstance::SetBoneScale on the list +0x1D0: slot 0…256;
/// scale 1 or bone None: slot released.
pub(crate) fn bone_scale(list: &mut Vec<Scaler>, info: &MeshInfo, slot: i32, scale: [f32; 3], name: &str) -> bool {
    if !(0..0x101).contains(&slot) {
        return false;
    }
    let slot = slot as usize;
    while list.len() <= slot {
        list.push(Scaler { bone: -1, ..Default::default() });
    }
    if scale != [1.0; 3] && !name.is_empty() {
        let bone = match_ref_bone(info, name);
        if bone >= 0 {
            list[slot] = Scaler { bone, name: name.to_string(), scale };
        }
        return true;
    }
    let sc = &mut list[slot];
    sc.bone = -1;
    sc.name.clear();
    true
}

/// USkeletalMeshInstance::SetBoneLocation on the list +0x1DC.
pub(crate) fn bone_location(list: &mut Vec<Director>, info: &MeshInfo, name: &str, translation: [f32; 3], alpha: f32) -> bool {
    let Some(d) = director_slot(list, info, name, |d| d.rot_alpha = 0.0) else { return false };
    d.trans_alpha = alpha;
    d.translation = translation;
    true
}

/// USkeletalMeshInstance::SetBoneRotation on the list +0x1DC.
pub(crate) fn bone_rotation(list: &mut Vec<Director>, info: &MeshInfo, name: &str, rotation: [i32; 3], space: i32, alpha: f32) -> bool {
    let Some(d) = director_slot(list, info, name, |d| d.trans_alpha = 0.0) else { return false };
    d.space = space;
    d.rot_alpha = alpha;
    d.rotation = rotation;
    true
}

fn director_slot<'a>(list: &'a mut Vec<Director>, info: &MeshInfo, name: &str, fresh: impl Fn(&mut Director)) -> Option<&'a mut Director> {
    if list.len() > 0xff {
        return None;
    }
    let bone = match_ref_bone(info, name);
    if bone < 0 {
        return None;
    }
    let i = match list.iter().position(|d| d.name.eq_ignore_ascii_case(name)) {
        Some(i) => i,
        None => {
            let mut d = Director { bone, name: name.to_string(), ..Default::default() };
            fresh(&mut d);
            list.push(d);
            list.len() - 1
        }
    };
    list.get_mut(i)
}

impl Vm {
    /// USkeletalMeshInstance::GetFrame: SpaceBases of the actor (one frame per bone of the RefSkeleton,
    /// in the mesh's frame), None without a skeletal mesh.
    #[track_caller]
    pub fn get_frame(&mut self, actor: ObjId) -> Option<Vec<Coords>> {
        self.get_frame_render(actor, true)
    }

    /// GetFrame of FDynamicActor::Render (USkeletalMeshInstance::Render), whose
    /// result is not read: same effects as `get_frame`, without copying the pose already done for the tick.
    pub(crate) fn get_frame_without_render(&mut self, actor: ObjId) {
        let _ = self.get_frame_render(actor, false);
    }

    #[track_caller]
    fn get_frame_render(&mut self, actor: ObjId, render: bool) -> Option<Vec<Coords>> {
        let tick = self.time.to_bits();
        let info = self.mesh_instance(actor)?.info.clone();
        if info.ref_bones.is_empty() {
            return None;
        }
        let w = actor_coords(self, actor, &info);
        let inst = self.mesh_insts.get_mut(&actor)?;
        inst.pose.scale_coords = w;
        if !crate::physics::coll::prior(crate::physics::coll::POSE) {
            let MeshInstance { anims, channels, pose, .. } = inst;
            let anims: &[Rc<AnimSet>] = anims;
            return Some(compute_pose_render(&info, anims, channels, pose, tick, render));
        }
        let anims = inst.anims.clone();
        Some(compute_pose_render(&info, &anims, &mut inst.channels, &mut inst.pose, tick, render))
    }

    /// The pose `get_frame` would give now, computed on copies of the instance's channels and
    /// cached pose: nothing of the game is written (no channel or root advance, no cache). For a
    /// drawing-only frame (lockstep co-op, Vm::preview_scene) of an actor the step did not draw.
    pub fn peek_frame(&self, actor: ObjId) -> Option<Vec<Coords>> {
        let inst = self.mesh_insts.get(&actor)?;
        let info = inst.info.clone();
        if info.ref_bones.is_empty() {
            return None;
        }
        let mut channels = inst.channels.clone();
        let mut pose = inst.pose.clone();
        pose.scale_coords = actor_coords(self, actor, &info);
        Some(compute_pose_render(&info, &inst.anims, &mut channels, &mut pose, self.time.to_bits(), true))
    }

    /// World → mesh frame computed by the actor's last GetFrame (+0x2A8).
    pub fn mesh_world_coords(&self, actor: ObjId) -> Option<Coords> {
        self.mesh_insts.get(&actor).map(|i| i.pose.scale_coords)
    }

    /// SetBoneScale.
    pub(crate) fn set_bone_scale(&mut self, actor: ObjId, slot: i32, scale: [f32; 3], name: &str) -> bool {
        let Some(inst) = self.mesh_instance(actor) else { return false };
        let info = inst.info.clone();
        bone_scale(&mut inst.pose.scalers, &info, slot, scale, name)
    }

    /// SetBoneLocation.
    pub(crate) fn set_bone_location(&mut self, actor: ObjId, name: &str, translation: [f32; 3], alpha: f32) -> bool {
        let Some(inst) = self.mesh_instance(actor) else { return false };
        let info = inst.info.clone();
        bone_location(&mut inst.pose.directors, &info, name, translation, alpha)
    }

    /// SetBoneRotation.
    pub(crate) fn set_bone_rotation(&mut self, actor: ObjId, name: &str, rotation: [i32; 3], space: i32, alpha: f32) -> bool {
        let Some(inst) = self.mesh_instance(actor) else { return false };
        let info = inst.info.clone();
        bone_rotation(&mut inst.pose.directors, &info, name, rotation, space, alpha)
    }

    /// SetBoneDirection: rotation and translation in the world's frame.
    pub(crate) fn set_bone_direction(&mut self, actor: ObjId, name: &str, rotation: [i32; 3], translation: [f32; 3], alpha: f32) -> bool {
        let Some(inst) = self.mesh_instance(actor) else { return false };
        let info = inst.info.clone();
        direction_os(&mut inst.pose.mesh_dirs, &info, name, rotation, translation, alpha)
    }

    /// USkeletalMeshInstance::LockRootMotion.
    pub(crate) fn lock_root_motion(&mut self, actor: ObjId, lock: i32) {
        let Some(info) = self.mesh_instance(actor).map(|i| i.info.clone()) else { return };
        let (w, without_original, tick) = (actor_coords(self, actor, &info), actor_coords_no_origin(self, actor, &info), self.time.to_bits());
        let Some(inst) = self.mesh_insts.get_mut(&actor) else { return };
        root_lock_motion(&mut inst.pose, &mut inst.channels, &info, w, without_original, tick, lock);
    }

    /// The Get*Root* functions only recompute the root (GF_RootOnly) if +0xFC is set.
    fn root_call<R>(&mut self, actor: ObjId, f: impl FnOnce(&mut PoseState, &mut Vec<Channel>, &MeshInfo, Coords, u32) -> R) -> Option<R> {
        let info = self.mesh_instance(actor).map(|i| i.info.clone())?;
        let tick = self.time.to_bits();
        let w = if self.mesh_insts.get(&actor).is_some_and(|i| i.pose.root_dirty) { actor_coords(self, actor, &info) } else { UNIT };
        let inst = self.mesh_insts.get_mut(&actor)?;
        Some(f(&mut inst.pose, &mut inst.channels, &info, w, tick))
    }

    /// USkeletalMeshInstance::GetRootLocation.
    pub(crate) fn root_location(&mut self, actor: ObjId) -> [f32; 3] {
        self.root_call(actor, root_location).unwrap_or([0.0; 3])
    }

    /// USkeletalMeshInstance::GetRootRotation.
    pub(crate) fn root_rotation(&mut self, actor: ObjId) -> [i32; 3] {
        self.root_call(actor, root_rotation).unwrap_or([0; 3])
    }

    /// USkeletalMeshInstance::GetRootLocationDelta.
    pub(crate) fn root_location_delta(&mut self, actor: ObjId) -> [f32; 3] {
        self.root_call(actor, root_location_delta).unwrap_or([0.0; 3])
    }

    /// USkeletalMeshInstance::GetRootRotationDelta.
    pub(crate) fn root_rotation_delta(&mut self, actor: ObjId) {
        self.root_call(actor, root_rotation_delta);
    }
}

/// GetFrame(…, GF_RootOnly) (TaskFlag 3): the pose is not recomputed (and nothing
/// is done if it was at this tick); local frame of the root (quaternion and position
/// of the cached pose) composed with +0xCC, giving RootLocation and RootRotation (OrthoRotation).
/// `w`: the actor's frame with the mesh's Origin.
pub(crate) fn root_get_frame(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) {
    if info.ref_bones.is_empty() {
        return;
    }
    if channels.is_empty() {
        channels.push(Channel::default());
    }
    pose.scale_coords = w;
    let same_tick = pose.last_tick == Some(tick);
    pose.last_tick = Some(tick);
    if pose.cache.is_empty() {
        pose.cache = vec![BonePose { rotation: [0.0; 4], position: [0.0; 3] }; info.ref_bones.len()];
        channels.iter_mut().for_each(|c| c.tween_base = 0.0);
    }
    // SpaceBases (+0x290) is allocated, zeroed, by any GetFrame, which then does
    // not take the tick into account; afterwards, at the same tick, it stops there (
    // GIsEditor zero).
    let fresh = pose.bases.is_empty();
    if fresh {
        pose.bases = vec![Coords::default(); info.ref_bones.len()];
    }
    if same_tick && !fresh {
        return;
    }
    let root = pose.cache[0];
    let local = coords::from_quat(root.rotation, root.position);
    let world = local.mul(&pose.root_ref);
    pose.root_local = local;
    pose.root_world = world;
    pose.root_location = world.origin;
    pose.root_rotation = crate::world::ortho_rotation([world.x, world.y, world.z]);
    pose.root_dirty = false;
}

/// GF_RootOnly if the root is to be recomputed (+0xFC), like the start of the Get*Root* functions.
fn root_refresh(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) {
    if pose.root_dirty {
        root_get_frame(pose, channels, info, w, tick);
    }
}

/// LockRootMotion: lock (+0x98), root to recompute, mesh → world frame
/// kept (+0xCC, `without_original`), GetRootLocation, then the root kept as the last one (+0xB4,
/// +0xC0) and locked (+0x104, +0x134); GetFrame then gives the root the origin of +0x104.
pub(crate) fn root_lock_motion(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, without_original: Coords, tick: u32, lock: i32) {
    pose.root_lock = lock;
    pose.root_dirty = true;
    pose.root_ref = without_original;
    root_location(pose, channels, info, w, tick);
    pose.last_root_location = pose.root_location;
    pose.locked_local = pose.root_local;
    pose.locked_world = pose.root_world;
    pose.last_root_rotation = pose.root_rotation;
    pose.root_locked = pose.locked_local.origin;
}

/// USkeletalMeshInstance::GetRootLocation: RootLocation.
pub(crate) fn root_location(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) -> [f32; 3] {
    root_refresh(pose, channels, info, w, tick);
    pose.root_location
}

/// USkeletalMeshInstance::GetRootRotation: RootRotation.
pub(crate) fn root_rotation(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) -> [i32; 3] {
    root_refresh(pose, channels, info, w, tick);
    pose.root_rotation
}

/// USkeletalMeshInstance::GetRootLocationDelta: RootLocation − LastRootLocation,
/// then LastRootLocation = RootLocation. (The engine also relocks the root if +0x100 is
/// set, which only sampling in GF_RootOnly would do, and GetFrame does not call it there.)
pub(crate) fn root_location_delta(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) -> [f32; 3] {
    root_refresh(pose, channels, info, w, tick);
    let (r, l) = (pose.root_location, pose.last_root_location);
    pose.last_root_location = r;
    [r[0] - l[0], r[1] - l[1], r[2] - l[2]]
}

/// USkeletalMeshInstance::GetRootRotationDelta: (0, 0, 0) once the root is
/// recomputed.
pub(crate) fn root_rotation_delta(pose: &mut PoseState, channels: &mut Vec<Channel>, info: &MeshInfo, w: Coords, tick: u32) {
    root_refresh(pose, channels, info, w, tick);
}

#[cfg(test)]
crate::improved::register_modules! {
mod body_turn_tests {
    use crate::improved::presentation::turn_skeleton;
    use super::*;

    /// A bone's frame for SetBoneDirection's world direction `r` (compute_pose_render).
    fn direction_frame(scale: &Coords, r: [i32; 3]) -> Coords {
        let m = scale.transpose().mul(&coords::div_rotator(&UNIT, r));
        let n = |a: [f32; 3]| {
            let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            a.map(|c| c / l)
        };
        Coords { origin: [0.0; 3], x: n(m.x), y: n(m.y), z: n(m.z) }
    }

    #[test]
    fn a_direction_given_without_the_turn_ends_where_asked() {
        // A character's mesh frame: turned by RotOrigin, scaled, the actor facing some yaw.
        let info = MeshInfo { default_anim: None, bones: Vec::new(), ref_bones: Vec::new(), scale: [1.1, 1.1, 1.1], origin: [0.0, 0.0, -40.0], rot_origin: [0, -16384, 0], aliases: Vec::new() };
        let mut checked = 0;
        for (actor_yaw, turn) in [(0, 5000), (12000, -9000), (-30000, 14000), (40000, 300)] {
            let scale = frame_actor([100.0, -50.0, 20.0], [0.0; 3], [0, actor_yaw, 0], 1.0, [1.0; 3], &info);
            for r in [[2000, 1000, 0], [-7000, 30000, 1200], [9000, -12000, -300], [0, 0, 0]] {
                let asked = direction_frame(&scale, r);
                let mut bases = vec![direction_frame(&scale, [r[0], r[1].wrapping_sub(turn), r[2]])];
                turn_skeleton(&mut bases, &info, &scale, turn);
                for (a, b) in [(asked.x, bases[0].x), (asked.y, bases[0].y), (asked.z, bases[0].z)] {
                    for k in 0..3 {
                        assert!((a[k] - b[k]).abs() < 2e-3, "{actor_yaw} {turn} {r:?}: {asked:?} {:?}", bases[0]);
                    }
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 16);
        // Negative control: without the turn, the frame given is not the one asked.
        let scale = frame_actor([0.0; 3], [0.0; 3], [0, 0, 0], 1.0, [1.0; 3], &info);
        let given = direction_frame(&scale, [0, -5000, 0]);
        assert!((given.x[0] - direction_frame(&scale, [0; 3]).x[0]).abs() + (given.x[1] - direction_frame(&scale, [0; 3]).x[1]).abs() + (given.y[0] - direction_frame(&scale, [0; 3]).y[0]).abs() > 0.1);
    }
}
}
