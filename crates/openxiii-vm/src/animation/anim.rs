//! Animation of skeletal meshes, reimplemented from Engine.dll: channels of USkeletalMeshInstance
//! (PlayAnim, LoopAnim, TweenAnim, blends), frame-by-frame advance with the sequence
//! notifications (script functions like PlayFootStep) and AnimEnd (UpdateAnimation), latent
//! wait FinishAnim, and automatic movement animation of pawns (APawn::UpdateMovementAnimation,
//! bPhysicsAnimUpdate) called by APawn::TickSpecial.
//!
//! The fields keep the engine's layout (MeshAnimChannel, 0x60 bytes) and the computations their
//! original order: animation times are fractions of a sequence (0…1), a non-looping
//! sequence stops at AnimLast = 1 − 1/frames, an entry transition (tween) starts from a negative
//! frame.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::NativeField as Field;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_package::ObjectRef;
use std::rc::Rc;

/// Sequence of a MeshAnimation object.
#[derive(Debug, Clone)]
pub struct SeqInfo {
    pub name: String,
    pub groups: Vec<String>,
    pub num_frames: i32,
    pub rate: f32,
    /// (time as a fraction of the sequence, script function called).
    pub notifies: Vec<(f32, String)>,
}

/// Loaded MeshAnimation object.
#[derive(Debug)]
pub struct AnimSet {
    pub path: String,
    pub seqs: Vec<SeqInfo>,
    /// RefBones: names of the animated bones (the index is that of each movement's tracks).
    pub bones: Vec<String>,
    /// Moves (MotionChunk), parallel to the sequences.
    pub moves: Vec<openxiii_assets::animation::Move>,
}

impl AnimSet {
    /// UMeshAnimation::GetAnimSeq: first sequence with this name.
    pub fn seq(&self, name: &str) -> Option<usize> {
        self.seqs.iter().position(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// UMeshAnimation::GetMovement: movement with the same rank as the first
    /// sequence with this name.
    pub fn movement(&self, name: &str) -> Option<&openxiii_assets::animation::Move> {
        self.moves.get(self.seq(name)?)
    }
}

/// Skeletal mesh: default animation (USkeletalMesh + 0x168) and reference bones.
#[derive(Debug)]
pub struct MeshInfo {
    pub default_anim: Option<String>,
    pub bones: Vec<String>,
    /// RefSkeleton (0x3C bytes per bone): flags, orientation, position, parent.
    pub ref_bones: Vec<openxiii_assets::skeletal::Bone>,
    /// Scale (+0x8C), Origin (+0x98), RotOrigin (+0xA4) of the mesh.
    pub scale: [f32; 3],
    pub origin: [f32; 3],
    pub rot_origin: [i32; 3],
    /// TagAliases → TagNames (+0x184, +0x190), which MatchRefBone also applies.
    pub aliases: Vec<(String, String)>,
}

/// Fields of a channel read by each pass of UpdateAnimation.
struct ReadChannel {
    anim_index: i32,
    frame: f32,
    rate: f32,
    notify: bool,
    notify_ignore: bool,
}

/// Animation channel (MeshAnimChannel, 0x60 bytes at USkeletalMeshInstance + 0x1C4).
#[derive(Debug, Clone, Default)]
pub struct Channel {
    /// +0x04: index of the MeshAnimation object in the instance's list.
    pub anim_index: i32,
    /// +0x08 AnimSequence (empty = None).
    pub sequence: String,
    /// +0x0C AnimRate, as a fraction of the sequence per second; negative: proportional to speed.
    pub rate: f32,
    /// +0x10 AnimFrame (0…1, negative during the entry transition).
    pub frame: f32,
    /// +0x14 AnimLast.
    pub last: f32,
    /// +0x18 TweenRate.
    pub tween_rate: f32,
    /// +0x1C OldAnimRate.
    pub old_rate: f32,
    /// +0x20: Rate / NumFrames of the sequence, which SetAnimRate multiplies.
    pub orig_rate: f32,
    /// +0x24 AnimMinRate.
    pub min_rate: f32,
    /// +0x28 bAnimLoop.
    pub looping: bool,
    /// +0x2C bAnimNotify: the sequence has notifications.
    pub notify: bool,
    /// +0x30: timed blend in progress (AnimBlendToAlpha).
    pub timed_blend: bool,
    /// +0x3C: notifications and AnimEnd cut (EnableChannelNotify(channel, 0)).
    pub notify_ignore: bool,
    /// +0x40 BlendAlpha.
    pub blend_alpha: f32,
    /// +0x44, +0x48: in and out times of AnimBlendParams (clamped to 1).
    pub blend_in: f32,
    pub blend_out: f32,
    /// +0x4C: time left in the timed blend.
    pub time_to_go: f32,
    /// +0x50: alpha targeted by the timed blend.
    pub blend_target: f32,
    /// +0x54: starting bone of the blend (index in the reference skeleton).
    pub start_bone: i32,
    /// +0x38: sequence played backwards (GetFrame reads 1 − AnimFrame).
    pub reversed: bool,
    /// +0x58: frame of the last pose computed during the entry transition (GetFrame).
    pub tween_base: f32,
    /// +0x5C: sequence of this transition.
    pub tween_seq: String,
}

/// Mesh instance of an actor (USkeletalMeshInstance).
#[derive(Debug, Clone)]
pub struct MeshInstance {
    pub mesh: String,
    pub info: Rc<MeshInfo>,
    /// +0x7C: linked MeshAnimation objects (LinkSkelAnim); the mesh's default animation goes in
    /// at the first lookup if the list is empty.
    pub anims: Vec<Rc<AnimSet>>,
    /// +0x1C4: channels.
    pub channels: Vec<Channel>,
    /// +0x1F4: channel of the last notification (GetNotifyChannel).
    pub notify_channel: i32,
    /// +0x1F8: channel of the last AnimEnd.
    pub end_channel: i32,
    /// GetFrame state (cached pose, bone mappings): see `skel.rs`.
    pub pose: crate::skel::PoseState,
}

impl MeshInstance {
    pub(crate) fn channel(&self, ch: i32) -> Option<&Channel> {
        if ch < 0 {
            return None;
        }
        self.channels.get(ch as usize)
    }

    fn channel_mut(&mut self, ch: i32) -> Option<&mut Channel> {
        if ch < 0 {
            return None;
        }
        self.channels.get_mut(ch as usize)
    }

    /// ValidateAnimChannel: channels 0…256, created at zero when needed.
    pub(crate) fn validate(&mut self, ch: i32) -> bool {
        if !(0..0x101).contains(&ch) {
            return false;
        }
        if self.channels.len() <= ch as usize {
            self.channels.resize(ch as usize + 1, Channel::default());
        }
        true
    }

    /// IsAnimating (vtable +0xDC): non-null sequence, TweenRate if AnimFrame < 0
    /// otherwise AnimRate, non-zero.
    pub(crate) fn is_animated(&self, ch: i32) -> bool {
        self.channel(ch).is_some_and(|c| !c.sequence.is_empty() && (if c.frame < 0.0 { c.tween_rate } else { c.rate }) != 0.0)
    }

    /// IsAnimTweening: AnimFrame < 0 and IsAnimating.
    pub(crate) fn in_transition(&self, ch: i32) -> bool {
        self.channel(ch).is_some_and(|c| c.frame < 0.0) && self.is_animated(ch)
    }

    /// IsAnimLooping: bAnimLoop.
    pub(crate) fn in_loop(&self, ch: i32) -> bool {
        self.channel(ch).is_some_and(|c| c.looping)
    }

    /// IsAnimPastLastFrame: AnimLast <= AnimFrame.
    pub(crate) fn pass_end(&self, ch: i32) -> bool {
        self.channel(ch).is_some_and(|c| c.last <= c.frame)
    }

    /// AnimStopLooping: bAnimLoop and AnimMinRate at 0; false outside the channels.
    pub(crate) fn stops_loop(&mut self, ch: i32) -> bool {
        let Some(c) = self.channel_mut(ch) else { return false };
        c.looping = false;
        c.min_rate = 0.0;
        true
    }

    /// StopAnimating: AnimRate and TweenRate of all channels at 0.
    pub(crate) fn stops_all(&mut self) {
        for c in &mut self.channels {
            c.rate = 0.0;
            c.tween_rate = 0.0;
        }
    }

    /// GetActiveAnimSequence / Rate / Frame: fields of the
    /// channel, None and 0 outside the channels.
    pub(crate) fn active_state(&self, ch: i32) -> (String, f32, f32) {
        self.channel(ch).map(|c| (c.sequence.clone(), c.rate, c.frame)).unwrap_or((String::new(), 0.0, 0.0))
    }

    /// SetAnimFrame.
    pub(crate) fn set_image(&mut self, ch: i32, frame: f32) {
        if let Some(c) = self.channel_mut(ch) {
            c.frame = frame;
        }
    }

    /// BlendToAlpha: valid channel (a negative time outside the channels would write outside the
    /// array in the engine; nothing here).
    pub(crate) fn fade_to(&mut self, ch: i32, target: f32, time: f32) {
        if let Some(c) = self.channel_mut(ch) {
            c.blend_target = target;
            c.time_to_go = time;
            c.timed_blend = true;
        }
    }

    /// SetBlendParams: not on the base channel (warning); starting bone through
    /// MatchRefBone (0 without a bone), in and out times clamped to 1. False if the channel is
    /// not valid.
    pub(crate) fn params_blend(&mut self, ch: i32, alpha: f32, t_in: f32, t_out: f32, bone: &str) -> Option<bool> {
        if !self.validate(ch) {
            return None;
        }
        if ch == 0 {
            return Some(false);
        }
        let bone_index = crate::skel::match_ref_bone(&self.info, bone).max(0);
        let c = &mut self.channels[ch as usize];
        c.blend_alpha = alpha;
        c.blend_in = if t_in < 1.0 { t_in } else { 1.0 };
        c.blend_out = if t_out < 1.0 { t_out } else { 1.0 };
        c.start_bone = bone_index;
        Some(true)
    }

    /// GetBlendAlpha: 0 outside the channels.
    pub(crate) fn alpha(&self, ch: i32) -> f32 {
        self.channel(ch).map_or(0.0, |c| c.blend_alpha)
    }

    /// SetBlendAlpha: clamped to 0…1.
    pub(crate) fn set_alpha(&mut self, ch: i32, alpha: f32) {
        if let Some(c) = self.channel_mut(ch) {
            c.blend_alpha = if alpha < 0.0 {
                0.0
            } else if alpha < 1.0 {
                alpha
            } else {
                1.0
            };
        }
    }

    /// EnableChannelNotify: ValidateAnimChannel, then +0x3C = (bEnable == 0).
    pub(crate) fn notif_channel(&mut self, ch: i32, on: bool) -> bool {
        if !self.validate(ch) {
            return false;
        }
        self.channels[ch as usize].notify_ignore = !on;
        true
    }

    /// First linked MeshAnimation that contains the sequence (GetAnimNamed;
    /// FindAnimObjectForSequence, UMeshAnimation::GetAnimSeq).
    fn named(&self, name: &str) -> Option<(usize, Rc<AnimSet>, usize)> {
        self.anims.iter().enumerate().find_map(|(i, a)| a.seq(name).map(|s| (i, a.clone(), s)))
    }

    /// USkeletalMeshInstance::CurrentSkelAnim: the channel's animation (index +4 in the
    /// list +0x7C, channel and index within their bounds); otherwise `None`: the mesh's default animation
    /// (USkeletalMesh + 0x168, which `anim_named` puts in the list). No caller in
    /// the single-player campaign (GetAnimSeq, called through the editor accessors' vtable).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn current_skel_anim(&self, ch: i32) -> Option<usize> {
        let c = self.channel(ch)?;
        (c.anim_index > -1 && (c.anim_index as usize) < self.anims.len()).then_some(c.anim_index as usize)
    }
}

/// USkeletalMeshInstance::GetAnimSeq on the animation of CurrentSkelAnim(0): the
/// sequence with index `i`, null without an animation or out of bounds.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_seq(set: Option<&AnimSet>, i: usize) -> Option<usize> {
    set.filter(|s| i < s.seqs.len()).map(|_| i)
}

/// AnimGetGroup: first group of the sequence, empty (NAME_None) without a group.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_get_group(seq: &SeqInfo) -> &str {
    seq.groups.first().map_or("", |g| g.as_str())
}

/// AnimGetName: name of the sequence, empty (NAME_None) for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_get_name(seq: Option<&SeqInfo>) -> &str {
    seq.map_or("", |s| s.name.as_str())
}

/// AnimGetFrameCount: NumFrames as a float, 0 for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_frame_count(seq: Option<&SeqInfo>) -> f32 {
    seq.map_or(0.0, |s| s.num_frames as f32)
}

/// AnimGetRate: Rate of the sequence, 0 for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_rate(seq: Option<&SeqInfo>) -> f32 {
    seq.map_or(0.0, |s| s.rate)
}

/// AnimGetNotifyCount: number of notifications, 0 for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_notify_count(seq: Option<&SeqInfo>) -> usize {
    seq.map_or(0, |s| s.notifies.len())
}

/// AnimGetNotifyTime: time of notification `k`, 0 for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_notify_time(seq: Option<&SeqInfo>, k: usize) -> f32 {
    seq.map_or(0.0, |s| s.notifies[k].0)
}

/// AnimGetNotifyFunction: function of notification `k`, NAME_None (empty) for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_notify_function(seq: Option<&SeqInfo>, k: usize) -> &str {
    seq.map_or("", |s| s.notifies[k].1.as_str())
}

/// AnimGetNotifyText: function of notification `k` (text of the FName at +4 of
/// the 8-byte element), null for a null sequence.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn anim_get_notify_text(seq: Option<&SeqInfo>, k: usize) -> Option<&str> {
    seq.map(|s| s.notifies[k].1.as_str())
}

/// Unreal 2's PHYS_*.
const PHYS_WALKING: u8 = 1;
const PHYS_FALLING: u8 = 2;
const PHYS_LADDER: u8 = 11;

/// FVector::Size: appSqrt in double of (X² + Y²) + Z².
fn vsize(v: [f32; 3]) -> f32 {
    ((((v[0] * v[0] + v[1] * v[1]) + v[2] * v[2]) as f64).sqrt()) as f32
}

/// Start of UpdateAnimation for a channel: timed blend (AnimBlendToAlpha)
/// advanced by `dt`. False if the channel is skipped (blend in progress without an animation).
pub(crate) fn blend_channel(c: &mut Channel, dt: f32) -> bool {
    if c.timed_blend {
        if c.anim_index <= -1 {
            return false;
        }
        if dt > 0.0 {
            let mut t = dt / c.time_to_go;
            if 1.0 <= t {
                t = 1.0;
            }
            c.blend_alpha = (c.blend_target - c.blend_alpha) * t + c.blend_alpha;
            let mut left = c.time_to_go - dt;
            if left < 0.0 {
                left = 0.0;
            }
            c.time_to_go = left;
            if left == 0.0 {
                c.timed_blend = false;
            }
        }
    }
    true
}

/// Continuation of an advance step of a channel (USkeletalMeshInstance::UpdateAnimation).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StepMode {
    /// The channel stops for this frame.
    Stop,
    /// New step (if there is time left).
    Continued,
    /// Notification crossed: the script function, called if the actor has it (FindFunction),
    /// the channel recorded at +0x1F4; then a new step.
    Notification(String),
    /// End of sequence or of transition: the channel recorded at +0x1F8; AnimEnd if true (vt 0xC0);
    /// then a new step.
    EndAnim(bool),
}

/// One advance step of channel `c` whose frame was `old` at the start of the step: entry transition,
/// advance at the rate (negative: proportional to `speed`, at least 0.3), nearest
/// notification crossed (`notifies`: those of the sequence, when UpdateAnimation looks
/// for them), end of sequence. `remaining`: time left in the frame for this channel.
pub(crate) fn step_channel(c: &mut Channel, old: f32, remaining: &mut f32, speed: f32, notifies: Option<&[(f32, String)]>) -> StepMode {
    if c.frame < 0.0 {
        // Entry transition towards the first frame.
        let f = *remaining * c.tween_rate + c.frame;
        c.frame = f;
        if f < 0.0 {
            return StepMode::Stop;
        }
        c.frame = 0.0;
        *remaining = (*remaining * f) / (f - old);
        if c.rate == 0.0 {
            return StepMode::EndAnim(!c.notify_ignore);
        }
        return StepMode::Continued;
    }
    let step = if c.rate < 0.0 {
        let v = -(speed * c.rate);
        (if 0.3 < v { v } else { 0.3 }) * *remaining
    } else {
        *remaining * c.rate
    };
    let frame = step + c.frame;
    c.frame = frame;
    // Nearest notification crossed (old < time <= frame).
    if let Some(notifies) = notifies {
        let mut best: Option<usize> = None;
        let mut best_elapsed = 100000.0f32;
        for (i, (t, _)) in notifies.iter().enumerate() {
            let t = *t;
            // Fcomp then test ah,1; a NaN frame (NaN rate from
            // UpdateMovementAnimation, acceleration without a useful component) raises C0 like
            // "frame < time": no notification.
            if !(old < t) || frame < t || frame.is_nan() {
                continue;
            }
            let e = t - old;
            if best.is_some() && !(e < best_elapsed) {
                continue;
            }
            best = Some(i);
            best_elapsed = e;
        }
        if let Some(b) = best {
            let (t, function) = notifies[b].clone();
            *remaining = ((frame - t) * *remaining) / (frame - old);
            c.frame = t;
            return StepMode::Notification(function);
        }
    }
    // End of sequence.
    if c.frame < c.last {
        return StepMode::Stop;
    }
    if c.looping {
        if c.frame >= 1.0 || c.frame.is_nan() {
            *remaining = ((c.frame - 1.0) * *remaining) / (c.frame - old);
            c.frame = 0.0;
        } else {
            *remaining = 0.0;
        }
        if old < c.last {
            return StepMode::EndAnim(!c.notify_ignore);
        }
        StepMode::Continued
    } else {
        *remaining = ((c.frame - c.last) * *remaining) / (c.frame - old);
        c.frame = c.last;
        let fire = !c.notify_ignore && c.rate > 0.0;
        c.rate = 0.0;
        StepMode::EndAnim(fire)
    }
}

/// FVector::SafeNormal: (X² + Y²) + Z², zero below 1e-8 (comparison in double).
fn safe_normal(v: [f32; 3]) -> [f32; 3] {
    crate::physics::safe_normal(v)
}

/// End of USkeletalMeshInstance::PlayAnim once the sequence is found (frames `nf`,
/// rate `seq_rate`, notifications): loop (same loop in progress: only the speed changes),
/// TweenAnim (zero rate) or simple playback. `animating`: AActor::IsAnimating of the channel before;
/// `speed`: |Velocity| of the actor. False if the call fails.
#[allow(clippy::too_many_arguments)]
pub(crate) fn play_channel(c: &mut Channel, sequence: &str, nf: f32, seq_rate: f32, notifies: bool, rate: f32, tween: f32, looping: bool, animating: bool, speed: f32) -> bool {
    if looping {
        if nf <= 0.0 {
            return false;
        }
        // Same loop already in progress: only the speed changes.
        if c.sequence.eq_ignore_ascii_case(sequence) && c.looping && animating {
            c.min_rate = 0.0;
            c.orig_rate = seq_rate / nf;
            c.rate = seq_rate / nf * rate;
            c.old_rate = c.rate;
            return true;
        }
        start_sequence(c, sequence, nf, seq_rate, rate, tween, true, notifies, 0.0001, speed);
        return true;
    }
    if rate <= 0.0 {
        // TweenAnim: transition towards the first frame, without playback.
        if rate != 0.0 || nf <= 0.0 {
            return false;
        }
        let f = 1.0 / nf;
        c.sequence = sequence.to_string();
        c.last = 0.0;
        c.notify = false;
        c.min_rate = 0.0;
        c.looping = false;
        c.rate = 0.0;
        c.old_rate = 0.0;
        c.orig_rate = seq_rate * f;
        if tween <= 0.0 {
            c.tween_rate = 0.0;
            c.frame = 0.0;
        } else {
            c.tween_rate = 1.0 / (nf * tween);
            c.frame = f * -1.0;
        }
        return true;
    }
    let tween = if c.sequence.is_empty() { 0.0 } else { tween };
    if nf <= 0.0 {
        return false;
    }
    start_sequence(c, sequence, nf, seq_rate, rate, tween, false, notifies, 0.001, speed);
    true
}

/// A channel taking over a sequence, the part shared by PlayAnim and LoopAnim
/// (USkeletalMeshInstance::PlayAnim). `small` is the starting frame without a transition.
#[allow(clippy::too_many_arguments)]
fn start_sequence(c: &mut Channel, name: &str, nf: f32, seq_rate: f32, rate: f32, tween: f32, looping: bool, notifies: bool, small: f32, speed: f32) {
    let f = 1.0 / nf;
    c.sequence = name.to_string();
    c.orig_rate = f * seq_rate;
    c.rate = f * seq_rate * rate;
    c.last = 1.0 - f;
    c.min_rate = 0.0;
    c.looping = looping;
    c.notify = notifies;
    if c.last == 0.0 {
        // Single-frame sequence: transition only (0.1 s by default).
        c.notify = false;
        c.old_rate = 0.0;
        c.tween_rate = if tween <= 0.0 { 10.0 } else { 1.0 / tween };
        c.rate = 0.0;
        c.frame = f * -1.0;
        return;
    }
    if tween <= 0.0 {
        if tween != -1.0 {
            c.tween_rate = 0.0;
            c.frame = small;
            c.old_rate = c.rate;
            return;
        }
        c.frame = f * -1.0;
        if c.old_rate > 0.0 {
            c.tween_rate = c.old_rate;
        } else if c.old_rate < 0.0 {
            // Old loop proportional to speed (the larger one).
            let (a, b) = (c.rate * 0.5, speed * c.old_rate * -1.0);
            c.tween_rate = if b <= a { a } else { b };
        } else {
            c.tween_rate = 1.0 / (nf * 0.025);
        }
        c.old_rate = c.rate;
        return;
    }
    c.old_rate = c.rate;
    c.tween_rate = 1.0 / (nf * tween);
    c.frame = f * -1.0;
}

/// The three bytes that AActor::ReplicateAnim writes into SimAnim: AnimRate, AnimFrame, TweenRate.
pub(crate) fn sim_anim_bytes(rate: f32, frame: f32, tween: f32, last: f32) -> [u8; 3] {
    // _ftol truncates; the byte is the low part of the result.
    let ftol = |x: f32| x as i64 as u8;
    let r = if last == 0.0 { 0x7c } else { ftol((if rate < -4.0 { -4.0 } else if rate < 4.0 { rate } else { 4.0 } + 4.0) * 31.0) };
    let f = ftol((if frame < -1.0 { -1.0 } else if frame < 1.0 { frame } else { 1.0 } + 1.0) * 127.0);
    let t = ftol(if tween < 0.0 { 0.0 } else if tween < 63.0 { tween } else { 63.0 } * 4.0);
    [r, f, t]
}

impl Vm {
    /// MeshAnimation object (cached).
    pub(crate) fn anim_set(&mut self, path: &str) -> Option<Rc<AnimSet>> {
        let key = path.to_lowercase();
        if let Some(s) = self.anim_sets.get(&key) {
            return s.clone();
        }
        // Mods build only: the animation of a mod's animated model.
        #[cfg(feature = "mods")]
        if let Some(a) = openxiii_assets::mod_skeletal::animation(path) {
            let seqs = a.sequences.iter().map(|s| SeqInfo { name: s.name.clone(), groups: Vec::new(), num_frames: s.num_frames, rate: s.rate, notifies: s.notifies.clone() }).collect();
            let set = Some(Rc::new(AnimSet { path: path.to_string(), seqs, bones: a.bones.clone(), moves: a.moves.clone() }));
            self.anim_sets.insert(key, set.clone());
            return set;
        }
        let set = self.library.as_mut().and_then(|lib| {
            let (loaded, index) = lib.resolve(path)?;
            if loaded.package.export_class(index) != "MeshAnimation" {
                return None;
            }
            let anim = openxiii_assets::animation::MeshAnimation::read(&loaded.package, &loaded.data, index).ok()?;
            let name = |n: i32| {
                let s = loaded.package.name(n);
                if s.eq_ignore_ascii_case("None") {
                    String::new()
                } else {
                    s.to_string()
                }
            };
            let seqs = anim
                .sequences
                .iter()
                .map(|s| SeqInfo {
                    name: name(s.name),
                    groups: s.groups.iter().map(|&g| name(g)).collect(),
                    num_frames: s.num_frames,
                    rate: s.rate,
                    notifies: s.notifies.iter().map(|&(t, f)| (t, name(f))).collect(),
                })
                .collect();
            let bones = anim.bones.iter().map(|b| name(b.0)).collect();
            Some(Rc::new(AnimSet { path: path.to_string(), seqs, bones, moves: anim.moves }))
        });
        self.anim_sets.insert(key, set.clone());
        set
    }

    /// Skeletal mesh (cached); None for other meshes.
    pub(crate) fn mesh_info(&mut self, mesh: &str) -> Option<Rc<MeshInfo>> {
        if let Some(cached) = crate::with_lower(mesh, |key| self.mesh_infos.get(key).cloned()) {
            return cached;
        }
        // Mods build only: a mod's animated model.
        #[cfg(feature = "mods")]
        if let Some(m) = openxiii_assets::mod_skeletal::get(mesh) {
            let info = Some(Rc::new(MeshInfo {
                default_anim: Some(m.animation_name.clone()),
                bones: m.bone_names.clone(),
                scale: m.mesh.scale,
                origin: m.mesh.origin,
                rot_origin: m.mesh.rot_origin,
                ref_bones: m.mesh.bones.clone(),
                aliases: m.aliases.clone(),
            }));
            self.mesh_infos.insert(mesh.to_lowercase(), info.clone());
            return info;
        }
        let info = self.library.as_mut().and_then(|lib| {
            let (loaded, index) = lib.resolve(mesh)?;
            if loaded.package.export_class(index) != "SkeletalMesh" {
                return None;
            }
            let m = openxiii_assets::skeletal::SkeletalMesh::read(&loaded.package, &loaded.data, index).ok()?;
            let default_anim = match ObjectRef::from(m.animation) {
                ObjectRef::None => None,
                ObjectRef::Export(_) => Some(format!("{}.{}", loaded.name, loaded.package.object_path(m.animation.into()))),
                _ => Some(loaded.package.object_path(m.animation.into())),
            };
            let bones = m.bones.iter().map(|b| loaded.package.name(b.name).to_string()).collect();
            let aliases = openxiii_assets::skeletal_collision::read_attach_tags(&loaded.package, &loaded.data, index)
                .map(|t| t.aliases.iter().zip(&t.names).map(|(&a, &n)| (loaded.package.name(a).to_string(), loaded.package.name(n).to_string())).collect())
                .unwrap_or_default();
            Some(Rc::new(MeshInfo { default_anim, bones, scale: m.scale, origin: m.origin, rot_origin: m.rot_origin, ref_bones: m.bones, aliases }))
        });
        self.mesh_infos.insert(mesh.to_lowercase(), info.clone());
        info
    }

    /// UMesh::MeshGetInstance: the actor's instance for its current mesh,
    /// recreated (empty channels) when the mesh changes.
    pub(crate) fn mesh_instance(&mut self, actor: ObjId) -> Option<&mut MeshInstance> {
        // Instance already made for this mesh: returned without copying the path.
        if let (Value::Asset(mesh), Some(i)) = (self.native_prop(actor, Field::Mesh), self.mesh_insts.get(&actor)) {
            if i.mesh.eq_ignore_ascii_case(mesh) {
                return self.mesh_insts.get_mut(&actor);
            }
        }
        let Value::Asset(mesh) = self.native_value(actor, Field::Mesh) else {
            self.mesh_insts.remove(&actor);
            return None;
        };
        self.mesh_instance_from(actor, &mesh)
    }

    /// MeshGetInstance with an explicit Mesh this (HalteAuFeu can target another actor).
    pub(crate) fn mesh_instance_from(&mut self, actor: ObjId, mesh: &str) -> Option<&mut MeshInstance> {
        if self.mesh_insts.get(&actor).is_none_or(|i| !i.mesh.eq_ignore_ascii_case(mesh)) {
            let Some(info) = self.mesh_info(mesh) else {
                self.mesh_insts.remove(&actor);
                return None;
            };
            self.mesh_insts.insert(
                actor,
                MeshInstance { mesh: mesh.to_string(), info, anims: Vec::new(), channels: Vec::new(), notify_channel: 0, end_channel: 0, pose: Default::default() },
            );
        }
        self.mesh_insts.get_mut(&actor)
    }

    /// Lookup of a sequence in the instance's animations (GetAnimNamed); an empty list
    /// first receives the mesh's default animation (SetSkelAnim, like CurrentSkelAnim
    /// which falls back on USkeletalMesh+0x168).
    pub(crate) fn anim_named(&mut self, actor: ObjId, name: &str) -> Option<(usize, Rc<AnimSet>, usize)> {
        let default = {
            let inst = self.mesh_instance(actor)?;
            if inst.anims.is_empty() {
                inst.info.default_anim.clone()
            } else {
                None
            }
        };
        if let Some(path) = default {
            if let Some(set) = self.anim_set(&path) {
                if let Some(inst) = self.mesh_insts.get_mut(&actor) {
                    inst.anims.push(set);
                }
            }
        }
        self.mesh_insts.get(&actor)?.named(name)
    }

    /// AActor::IsAnimating: drawn mesh (DT_Mesh) and moving channel
    /// (USkeletalMeshInstance::IsAnimating, vtable +0xDC: non-null sequence, TweenRate
    /// if AnimFrame < 0 otherwise AnimRate, non-zero).
    pub fn is_animating(&mut self, actor: ObjId, ch: i32) -> bool {
        if self.native_prop(actor, Field::DrawType).as_byte() != 2 {
            return false;
        }
        self.mesh_instance(actor).is_some_and(|inst| inst.is_animated(ch))
    }

    /// AActor::PlayAnim then USkeletalMeshInstance::PlayAnim.
    pub fn play_anim(&mut self, actor: ObjId, ch: i32, sequence: &str, rate: f32, tween: f32, looping: bool) -> bool {
        if matches!(self.native_value(actor, Field::Mesh), Value::Object(None) | Value::Void) {
            let name = self.objects[actor].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("Log: {name} playanim with no mesh!"));
            return false;
        }
        let Some(inst) = self.mesh_instance(actor) else {
            return false;
        };
        if ch < 0 || !inst.validate(ch) {
            // The original format adds "[instance name]" (GetName): the object name's number is not modelled.
            self.log.push(format!("Log: Invalid active channel: [{ch}] for MeshInstance [SkeletalMeshInstance]"));
            return false;
        }
        let Some((ai, set, si)) = self.anim_named(actor, sequence) else {
            // "Log: PlayAnim: Sequence '%s' not found for mesh '%s'" with Mesh->GetName() (without the
            // package: "GardienM" in the original XIII.log).
            let mesh = self.mesh_insts.get(&actor).map(|i| i.mesh.clone()).unwrap_or_default();
            let mesh = mesh.rsplit('.').next().unwrap_or(&mesh).to_string();
            self.log.push(format!("Log: PlayAnim: Sequence '{sequence}' not found for mesh '{mesh}'"));
            return false;
        };
        let seq = &set.seqs[si];
        let (nf, seq_rate, notifies) = (seq.num_frames as f32, seq.rate, !seq.notifies.is_empty());
        let animating = self.is_animating(actor, ch);
        let speed = vsize(self.native_prop(actor, Field::Velocity).as_vector());
        let Some(c) = self.mesh_insts.get_mut(&actor).and_then(|i| i.channel_mut(ch)) else {
            return false;
        };
        c.anim_index = ai as i32;
        play_channel(c, sequence, nf, seq_rate, notifies, rate, tween, looping, animating, speed)
    }

    fn channel_value<T>(&mut self, actor: ObjId, ch: i32, get: impl Fn(&Channel) -> T) -> Option<T> {
        self.mesh_instance(actor).and_then(|i| i.channel(ch)).map(get)
    }

    pub(crate) fn channel_set(&mut self, actor: ObjId, ch: i32, set: impl FnOnce(&mut Channel)) {
        if let Some(c) = self.mesh_instance(actor).and_then(|i| i.channel_mut(ch)) {
            set(c);
        }
    }

    /// SetAnimRate: speed relative to the sequence's.
    fn set_anim_rate(&mut self, actor: ObjId, ch: i32, rate: f32) {
        self.channel_set(actor, ch, |c| cadence_channel(c, rate));
    }

    /// SetAnimSequence: changes the sequence without starting from zero.
    fn set_anim_sequence(&mut self, actor: ObjId, ch: i32, sequence: &str) {
        if self.mesh_instance(actor).and_then(|i| i.channel(ch)).is_none() {
            return;
        }
        let orig = self.anim_named(actor, sequence).map(|(_, set, si)| set.seqs[si].rate / set.seqs[si].num_frames as f32);
        self.channel_set(actor, ch, |c| sequence_channel(c, sequence, orig));
    }

    /// GetAnimSequence: sequence of the channel, None (empty) outside the channels.
    fn anim_sequence(&mut self, actor: ObjId, ch: i32) -> String {
        self.channel_value(actor, ch, |c| c.sequence.clone()).unwrap_or_default()
    }

    /// SetBlendAlpha: clamped to 0…1.
    pub(crate) fn set_blend_alpha(&mut self, actor: ObjId, ch: i32, alpha: f32) {
        if let Some(inst) = self.mesh_instance(actor) {
            inst.set_alpha(ch, alpha);
        }
    }

    /// UpdateBlendAlpha: moves the alpha towards `target` by at most `delta`.
    fn update_blend_alpha(&mut self, actor: ObjId, ch: i32, target: f32, delta: f32) {
        self.channel_set(actor, ch, |c| alpha_to(c, target, delta));
    }

    /// CopyAnimation: copy of a channel's state into another.
    fn copy_animation(&mut self, actor: ObjId, from: i32, to: i32) {
        let Some(inst) = self.mesh_instance(actor) else {
            return;
        };
        let Some(src) = inst.channel(from).cloned() else {
            return;
        };
        if to < 0 || !inst.validate(to) {
            return;
        }
        copy_channel(src, &mut inst.channels[to as usize]);
    }

    /// AActor::NotifyAnimEnd and APawn::NotifyAnimEnd: bAnimFinished,
    /// then AnimEnd on the pawn's controller if it probes it (IsProbing: ProbeMask, so an
    /// AnimEnd function defined in its class or state), otherwise on the actor. IAController does not
    /// define one: the AnimEnd goes to the pawn (BaseSoldier.AnimEnd).
    pub(crate) fn notify_anim_end(&mut self, actor: ObjId, ch: i32) {
        self.set_prop(actor, "bAnimFinished", Value::Bool(true));
        let pawn = self.class("Pawn").is_some_and(|p| self.objects[actor].as_ref().is_some_and(|o| self.is_child_of(o.class, p)));
        if pawn {
            crate::improved::presentation::finish_pawn_gesture(self, actor, ch);
            if let Some(c) = self.native_prop(actor, Field::Controller).as_object() {
                if self.objects[c].is_some() && self.is_probing(c, "AnimEnd") {
                    self.event_pub(c, "AnimEnd", vec![Value::Int(ch)]);
                    return;
                }
            }
        }
        self.event_pub(actor, "AnimEnd", vec![Value::Int(ch)]);
    }

    /// AActor::UpdateAnimation (MeshGetInstance, vt 0x90, then vt 0xA4 of the instance)
    /// → USkeletalMeshInstance::UpdateAnimation, called
    /// by AActor::Tick: timed blends, transitions, channel advance (up to four
    /// passes per channel), crossed notifications and ends of sequence (AnimEnd).
    pub(crate) fn update_animation(&mut self, actor: ObjId, dt: f32) {
        let Some(inst) = self.mesh_instance(actor) else {
            return;
        };
        // +0xFC: the root will be recomputed by the next GetRoot*.
        inst.pose.root_dirty = true;
        let mut ch = 0usize;
        loop {
            if self.objects[actor].is_none() {
                return;
            }
            let Some(count) = self.mesh_insts.get(&actor).map(|i| i.channels.len()) else {
                return;
            };
            if ch >= count {
                return;
            }
            let chi = ch as i32;
            ch += 1;
            'canal: {
                let mut remaining = dt;
                let mut passes = 0;
                {
                    let Some(c) = self.mesh_insts.get_mut(&actor).and_then(|i| i.channel_mut(chi)) else {
                        return;
                    };
                    if !blend_channel(c, dt) {
                        break 'canal;
                    }
                }
                loop {
                    // Fields of the channel read without copying the sequence (copied only for its notifications).
                    let Some(c) = self.mesh_insts.get(&actor).and_then(|i| i.channel(chi)).map(|c| ReadChannel {
                        anim_index: c.anim_index,
                        frame: c.frame,
                        rate: c.rate,
                        notify: c.notify,
                        notify_ignore: c.notify_ignore,
                    }) else {
                        break;
                    };
                    if c.anim_index <= -1 || !self.is_animating(actor, chi) || !(remaining > 0.0) {
                        break;
                    }
                    passes += 1;
                    if passes > 4 {
                        break;
                    }
                    let old = c.frame;
                    let speed = if c.frame >= 0.0 && c.rate < 0.0 { vsize(self.native_prop(actor, Field::Velocity).as_vector()) } else { 0.0 };
                    // Notifications of the sequence, read only if the branch looks for them:
                    // AnimGetNotifyCount, AnimGetNotifyTime,
                    // AnimGetNotifyFunction.
                    let has_mesh = !matches!(self.native_prop(actor, Field::Mesh), Value::Object(None) | Value::Void);
                    let notifies = if c.frame >= 0.0 && c.notify && has_mesh && !c.notify_ignore {
                        let sequence = self.mesh_insts.get(&actor).and_then(|i| i.channel(chi)).map(|c| c.sequence.clone()).unwrap_or_default();
                        self.anim_named(actor, &sequence).map(|(_, set, si)| set.seqs[si].notifies.clone())
                    } else {
                        None
                    };
                    let Some(cm) = self.mesh_insts.get_mut(&actor).and_then(|i| i.channel_mut(chi)) else {
                        return;
                    };
                    match step_channel(cm, old, &mut remaining, speed, notifies.as_deref()) {
                        StepMode::Stop => break,
                        StepMode::Continued => {}
                        StepMode::Notification(function) => {
                            if self.find_function(actor, &function, false).is_some() {
                                if let Some(m) = self.mesh_insts.get_mut(&actor) {
                                    m.notify_channel = chi;
                                }
                                self.event_pub(actor, &function, vec![]);
                            }
                        }
                        StepMode::EndAnim(signal) => {
                            if let Some(m) = self.mesh_insts.get_mut(&actor) {
                                m.end_channel = chi;
                            }
                            if signal {
                                self.notify_anim_end(actor, chi);
                            }
                        }
                    }
                }
            }
            // AActor::ReplicateAnim, channel 0 only, at the end of each pass.
            if chi == 0 {
                self.replicate_anim(actor);
            }
        }
    }

    /// AActor::ReplicateAnim: SimAnim takes channel 0 compressed (sequence,
    /// bAnimLoop, AnimRate = (Rate + 4) × 31 or 0x7C if AnimLast = 0, AnimFrame = (Frame + 1) ×
    /// 127, TweenRate × 4), for all roles.
    fn replicate_anim(&mut self, actor: ObjId) {
        let Some(c) = self.mesh_insts.get(&actor).and_then(|i| i.channel(0)) else {
            return;
        };
        let [rate, frame, tween] = sim_anim_bytes(c.rate, c.frame, c.tween_rate, c.last);
        let looping = c.looping;
        // Same name already in place (the case every frame): the bytes alone, without copying SimAnim.
        if let (Ok(slot), Some(i)) = (self.prop_slot(actor, "SimAnim"), self.mesh_insts.get(&actor)) {
            let seq = i.channels.first().map_or("", |c| c.sequence.as_str());
            if let Some(Value::Struct(_, f)) = self.objects[actor].as_ref().map(|o| &o.props[slot]) {
                if f.len() >= 5 && matches!(&f[0], Value::Name(n) if **n == *seq) {
                    if let Some(Value::Struct(_, f)) = self.objects[actor].as_mut().map(|o| &mut o.props[slot]) {
                        f[1] = Value::Bool(looping);
                        f[2] = Value::Byte(rate);
                        f[3] = Value::Byte(frame);
                        f[4] = Value::Byte(tween);
                    }
                    return;
                }
            }
        }
        let Some(seq) = self.mesh_insts.get(&actor).and_then(|i| i.channel(0)).map(|c| c.sequence.clone()) else {
            return;
        };
        let Value::Struct(n, mut f) = self.propr(actor, "SimAnim").clone() else {
            return;
        };
        if f.len() >= 5 {
            f[0] = Value::Name(crate::value::NameStruct::from(seq));
            f[1] = Value::Bool(looping);
            f[2] = Value::Byte(rate);
            f[3] = Value::Byte(frame);
            f[4] = Value::Byte(tween);
            self.set_prop(actor, "SimAnim", Value::Struct(n, f));
        }
    }

    /// AActor::PlayReplicatedAnim, called by PostNetReceive when SimAnim changes:
    /// PlayAnim of channel 0 (speed (AnimRate − 124) / 31, transition 4 / TweenRate), then
    /// SetAnimFrame at (AnimFrame − 127) / 127.
    pub(crate) fn play_replicated_anim(&mut self, actor: ObjId) {
        let Value::Struct(_, f) = self.propr(actor, "SimAnim").clone() else {
            return;
        };
        if f.len() < 5 || self.mesh_instance(actor).is_none() {
            return;
        }
        let b = |v: &Value| v.as_int() as u8 as f32;
        let seq = f[0].as_name();
        let seq = if seq.is_empty() { "None".to_string() } else { seq };
        self.play_anim(actor, 0, &seq, (b(&f[2]) - 124.0) * 0.032258064, 4.0 / b(&f[4]), f[1].as_bool());
        let frame = (b(&f[3]) - 127.0) * 0.007874016;
        if let Some(c) = self.mesh_insts.get_mut(&actor).and_then(|i| i.channel_mut(0)) {
            c.frame = frame;
        }
    }

    /// AActor::StartAnimPoll / AController::StartAnimPoll, started by
    /// FinishAnim: a loop stops looping (IsAnimLooping then AnimStopLooping
    /// bAnimLoop +0x28 and AnimMinRate +0x24 at 0); true if we must wait for the end of the
    /// sequence (IsAnimating and not IsAnimPastLastFrame: AnimLast +0x14 <= AnimFrame).
    pub(crate) fn start_anim_poll(&mut self, this: ObjId, ch: i32) -> bool {
        let Some(target) = self.anim_poll_target(this) else {
            return false;
        };
        if self.mesh_instance(target).is_none() {
            return false;
        }
        if let Some(inst) = self.mesh_instance(target) {
            if inst.in_loop(ch) {
                inst.stops_loop(ch);
            }
        }
        self.is_animating(target, ch) && !self.mesh_instance(target).is_some_and(|i| i.pass_end(ch))
    }

    /// CheckAnimFinished: true when FinishAnim can return.
    pub(crate) fn check_anim_finished(&mut self, this: ObjId, ch: i32) -> bool {
        let Some(target) = self.anim_poll_target(this) else {
            return true;
        };
        if self.mesh_instance(target).is_none() {
            return true;
        }
        !(self.is_animating(target, ch) && !self.mesh_instance(target).is_some_and(|i| i.pass_end(ch)))
    }

    /// Actor whose animation FinishAnim waits for: the pawn for a controller.
    fn anim_poll_target(&self, this: ObjId) -> Option<ObjId> {
        let controller = self.class("Controller").is_some_and(|c| self.objects[this].as_ref().is_some_and(|o| self.is_child_of(o.class, c)));
        let target = if controller { self.native_prop(this, Field::Pawn).as_object()? } else { this };
        (self.objects[target].is_some() && !matches!(self.native_value(target, Field::Mesh), Value::Object(None) | Value::Void)).then_some(target)
    }

    /// Actor's animation natives (Engine.dll, AActor::exec*); None if `name` is not one of them.
    pub(crate) fn anim_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        let given = |i: usize| supplied.get(i) == Some(&true);
        let f = |i: usize, d: f32| if given(i) { args[i].as_float() } else { d };
        let n = |i: usize, d: i32| if given(i) { args[i].as_int() } else { d };
        let s = |i: usize| args.get(i).map(|v| v.as_name()).unwrap_or_default();
        Some(match name {
            // AActor::execPlayAnim, execLoopAnim, execTweenAnim
            // vt 300 (AActor::PlayAnim) with Rate 1 / Tween 0 by default; TweenAnim
            // passes Rate 0.
            #[cfg(not(feature = "mods"))]
            "PlayAnim" => Value::Bool(self.play_anim(this, n(3, 0), &s(0), f(1, 1.0), f(2, 0.0), false)),
            #[cfg(not(feature = "mods"))]
            "LoopAnim" => Value::Bool(self.play_anim(this, n(3, 0), &s(0), f(1, 1.0), f(2, 0.0), true)),
            // Mods build: a weapon's animation speeds and first-person look set by mods
            // (mod_weapons.rs).
            #[cfg(feature = "mods")]
            "PlayAnim" | "LoopAnim" => {
                let (sequence, rate) = self.mod_weapon_anim(this, s(0), f(1, 1.0));
                Value::Bool(self.play_anim(this, n(3, 0), &sequence, rate, f(2, 0.0), name == "LoopAnim"))
            }
            "TweenAnim" => Value::Bool(self.play_anim(this, n(2, 0), &s(0), 0.0, f(1, 0.0), false)),
            // AActor::execIsAnimating: AActor::IsAnimating.
            "IsAnimating" => Value::Bool(self.is_animating(this, n(0, 0))),
            "FinishAnim" => {
                // AActor::execFinishAnim: LatentFloat = channel, then StartAnimPoll (vt 200).
                let ch = n(0, 0);
                self.set_prop(this, "LatentFloat", Value::Float(ch as f32));
                if self.start_anim_poll(this, ch) {
                    self.pending_latent = Some(crate::Latent::FinishAnim);
                }
                Value::Void
            }
            // AActor::execHasAnim: without a Mesh, nothing is written (false); otherwise
            // MeshGetInstance then GetAnimNamed (vtable +0xB4) non-null.
            "HasAnim" => {
                if matches!(self.native_value(this, Field::Mesh), Value::Object(None) | Value::Void) {
                    Value::Bool(false)
                } else {
                    Value::Bool(self.anim_named(this, &s(0)).is_some())
                }
            }
            // AActor::execStopAnimating → USkeletalMeshInstance::StopAnimating
            // AnimRate and TweenRate of all channels at 0.
            "StopAnimating" => {
                if let Some(inst) = self.mesh_instance(this) {
                    inst.stops_all();
                }
                Value::Void
            }
            // AActor::execIsTweening → IsAnimTweening: AnimFrame < 0 and
            // IsAnimating of the instance (vt 0xDC, without the DrawType test).
            "IsTweening" => {
                let ch = n(0, 0);
                Value::Bool(self.mesh_instance(this).is_some_and(|i| i.in_transition(ch)))
            }
            // AActor::execEnableChannelNotify → EnableChannelNotify:
            // ValidateAnimChannel, then +0x3C = (bEnable == 0).
            "EnableChannelNotify" => {
                let (ch, on) = (n(0, 0), n(1, 1));
                if let Some(inst) = self.mesh_instance(this) {
                    inst.notif_channel(ch, on != 0);
                }
                Value::Void
            }
            // AActor::execGetNotifyChannel: +0x1F4 of the instance, 0 without a SkeletalMesh.
            "GetNotifyChannel" => Value::Int(self.mesh_instance(this).map(|i| i.notify_channel).unwrap_or(0)),
            // AActor::execSetBoneScale, execSetBoneScalePerAxis,
            // execSetBoneLocation, execSetBoneRotation,
            // execSetBoneDirection: default values of the Steps (scale 1, alpha 0 or 1).
            "SetBoneScale" => Value::Bool(self.set_bone_scale(this, n(0, 0), [f(1, 1.0); 3], &s(2))),
            "SetBoneScalePerAxis" => Value::Bool(self.set_bone_scale(this, n(0, 0), [f(1, 1.0), f(2, 1.0), f(3, 1.0)], &s(4))),
            "SetBoneLocation" => {
                let t = if given(1) { args[1].as_vector() } else { [0.0; 3] };
                Value::Bool(self.set_bone_location(this, &s(0), t, f(2, 0.0)))
            }
            "SetBoneRotation" => {
                let r = if given(1) { args[1].as_rotator() } else { [0; 3] };
                Value::Bool(self.set_bone_rotation(this, &s(0), r, n(2, 0), f(3, 0.0)))
            }
            "SetBoneDirection" => {
                // The rotator has no default value in execSetBoneDirection.
                let r = args.get(1).map(|v| v.as_rotator()).unwrap_or([0; 3]);
                let t = if given(2) { args[2].as_vector() } else { [0.0; 3] };
                Value::Bool(self.set_bone_direction(this, &s(0), r, t, f(3, 1.0)))
            }
            // AActor::execLockRootMotion: Lock = 1 by default, skeletal mesh.
            "LockRootMotion" => {
                self.lock_root_motion(this, n(0, 1));
                Value::Void
            }
            // AActor::execLinkSkelAnim → USkeletalMeshInstance::SetSkelAnim:
            // AddUniqueItem into the instance's animation list. SetSkelAnim always returns
            // 1: the message "Trying to assign invalid animation object…" is never written.
            "LinkSkelAnim" => {
                if let Value::Asset(path) = args.first().cloned().unwrap_or(Value::Void) {
                    let set = self.anim_set(&path);
                    if let (Some(set), Some(inst)) = (set, self.mesh_instance(this)) {
                        if !inst.anims.iter().any(|a| Rc::ptr_eq(a, &set)) {
                            inst.anims.push(set);
                        }
                    }
                }
                Value::Void
            }
            "AnimBlendParams" => {
                // AActor::execAnimBlendParams → SetBlendParams: not on
                // the base channel.
                let (ch, alpha, t_in, t_out, bone) = (n(0, 0), f(1, 0.0), f(2, 0.0), f(3, 0.0), s(4));
                if self.mesh_instance(this).and_then(|inst| inst.params_blend(ch, alpha, t_in, t_out, &bone)) == Some(false) {
                    self.log.push("Log: Warning: attempt to set blending parameters for base stage in meshinstance [SkeletalMeshInstance]".to_string());
                }
                Value::Void
            }
            "AnimBlendToAlpha" => {
                // AActor::execAnimBlendToAlpha → BlendToAlpha.
                let (ch, target, time) = (n(0, 0), f(1, 0.0), f(2, 0.0));
                if let Some(inst) = self.mesh_instance(this) {
                    inst.fade_to(ch, target, time);
                }
                Value::Void
            }
            // AActor::execGetAnimParams: None, 0, 0, then with a Mesh
            // GetActiveAnimSequence / Frame / Rate: fields of the
            // channel if it exists (0 <= channel < Blends.Num()).
            "GetAnimParams" => {
                let ch = n(0, 0);
                let (seq, rate, frame) = self.mesh_instance(this).map(|i| i.active_state(ch)).unwrap_or((String::new(), 0.0, 0.0));
                if args.len() > 3 {
                    args[1] = Value::Name(crate::value::NameStruct::from(seq));
                    args[2] = Value::Float(frame);
                    args[3] = Value::Float(rate);
                }
                Value::Void
            }
            // AActor::execAnimIsInGroup: active sequence of the channel, GetAnimNamed, then
            // AnimIsInGroup: the group is in the sequence's list of groups.
            "AnimIsInGroup" => {
                let (ch, group) = (n(0, 0), s(1));
                let seq = self.anim_sequence(this, ch);
                let found = !seq.is_empty()
                    && self.anim_named(this, &seq).is_some_and(|(_, set, si)| set.seqs[si].groups.iter().any(|g| g.eq_ignore_ascii_case(&group)));
                Value::Bool(found)
            }
            _ => return None,
        })
    }

    /// APawn::ZeroMovementAlpha: channels [first, last) brought towards alpha 0 (alpha read
    /// by GetBlendAlpha, 0 outside the channels); when they are all there, they stop on
    /// their first frame.
    pub(crate) fn zero_movement_alpha(&mut self, pawn: ObjId, first: i32, last: i32, delta: f32) {
        let mut all_zero = true;
        for ch in first..last {
            let alpha = self.mesh_insts.get(&pawn).and_then(|i| i.channel(ch)).map(|c| c.blend_alpha).unwrap_or(0.0);
            if 0.0 < alpha {
                all_zero = false;
                self.update_blend_alpha(pawn, ch, 0.0, delta);
            }
        }
        if all_zero {
            for ch in first..last {
                self.set_anim_rate(pawn, ch, 0.0);
                self.channel_set(pawn, ch, |c| c.frame = 0.0);
            }
        }
    }

    /// APawn::UpdateMovementAnimation: walking animations (channels 4 to 7 and their
    /// copies 8 to 11), turning on the spot (2, 3) and weapon (12, 13 from "X Spine"), blended
    /// according to the direction of the acceleration relative to the view; notifications on channel 4.
    fn update_movement_animation(&mut self, pawn: ObjId, dt: f32) {
        if self.level.is_some_and(|l| self.propr(l, "NetMode").as_byte() == 1) {
            return;
        }
        if self.mesh_instance(pawn).is_none() {
            return;
        }
        if !self.propr(pawn, "bInitializeAnimation").as_bool() {
            self.set_prop(pawn, "bInitializeAnimation", Value::Bool(true));
            let right = self.propr(pawn, "TurnRightAnim").as_name();
            let left = self.propr(pawn, "TurnLeftAnim").as_name();
            self.play_anim(pawn, 2, &right, 1.0, 0.1, true);
            self.play_anim(pawn, 3, &left, 1.0, 0.1, true);
            for ch in 4..8 {
                let a = self.movement_anim(pawn, ch - 4);
                self.play_anim(pawn, ch, &a, 1.0, 0.1, true);
            }
            let weapon = self.propr(pawn, "WeaponAnim").as_name();
            self.play_anim(pawn, 0xd, &weapon, 1.0, 0.1, true);
            if let Some(inst) = self.mesh_instance(pawn) {
                let spine = crate::skel::match_ref_bone(&inst.info, "X Spine").max(0);
                for (ch, alpha) in [(0xd, 0.0), (0xc, 1.0)] {
                    if inst.validate(ch) {
                        let c = &mut inst.channels[ch as usize];
                        c.blend_alpha = alpha;
                        c.blend_in = 0.0;
                        c.blend_out = 0.0;
                        c.start_bone = spine;
                    }
                }
                for ch in 2..0xe {
                    if inst.validate(ch) {
                        inst.channels[ch as usize].notify_ignore = true;
                    }
                }
            }
            for ch in 2..0xe {
                self.set_blend_alpha(pawn, ch, 0.0);
            }
            if let Some(inst) = self.mesh_instance(pawn) {
                if inst.validate(4) {
                    inst.channels[4].notify_ignore = false;
                }
            }
        }
        movement_pawn(&mut PawnVm { vm: self, pawn }, dt);
    }

    fn movement_anim(&self, pawn: ObjId, i: i32) -> String {
        match self.prop(pawn, "MovementAnims") {
            Value::Array(items) => items.get(i as usize).map(|v| v.as_name()).unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn movement_anim_rates(&self, pawn: ObjId) -> [f32; 4] {
        match self.prop(pawn, "MovementAnimRate") {
            Value::Array(items) => [0, 1, 2, 3].map(|i| items.get(i).map(|v| v.as_float()).unwrap_or(0.0)),
            _ => [0.0; 4],
        }
    }

    /// APawn::UpdateSpineControl / UpdateHeadControl: the torso ("X
    /// Spine") or the head ("X Head") turned towards the controller's aim (or its Focus, if it is
    /// in front of the NPC), with the yaw oscillation of SpineYawControl: see `check_os`.
    fn update_bone_control(&mut self, pawn: ObjId, dt: f32, head: bool) {
        let (flag, max_p, yaw_p, speed_p, bone) =
            if head { ("bHeadYaw", "MaxHeadYaw", "HeadYaw", "HeadYawRotationSpeed", "X Head") } else { ("bSpineYaw", "MaxSpineYaw", "SpineYaw", "SpineYawRotationSpeed", "X Spine") };
        let controller = self.native_prop(pawn, Field::Controller).as_object().filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()));
        // APlayerController::LocalPlayerController (vtable +0x150), false for an AController.
        let local = controller.is_some_and(|c| self.local_player_controller(c).is_some());
        let focus = controller
            .and_then(|c| self.propr(c, "Focus").as_object())
            .filter(|&f| self.objects.get(f).is_some_and(|o| o.is_some()))
            .map(|f| self.native_prop(f, Field::Location).as_vector());
        let input_value = InputOs {
            pri: self.propr(pawn, "PlayerReplicationInfo").as_object().is_some(),
            controller: controller.is_some(),
            controller_rotation: controller.map(|c| self.native_prop(c, Field::Rotation).as_rotator()),
            local,
            focus,
            pawn_rotation: self.native_prop(pawn, Field::Rotation).as_rotator(),
            pawn_location: self.native_prop(pawn, Field::Location).as_vector(),
            controller_pitch: self.propr(pawn, "ControllerPitch").as_byte(),
            flag: self.propr(pawn, flag).as_bool(),
            max_yaw: self.propr(pawn, max_p).as_int(),
            yaw: self.propr(pawn, yaw_p).as_int(),
            speed: self.propr(pawn, speed_p).as_float(),
            angle: self.propr(pawn, "Angle").as_float(),
            dt,
        };
        let output = check_os(&input_value);
        self.set_prop(pawn, "Angle", Value::Float(output.angle));
        if let Some(v) = output.new_yaw {
            self.set_prop(pawn, yaw_p, Value::Int(v));
        }
        let rotation = crate::improved::presentation::bone_rotation(self, pawn, bone, head, dt, output.rotation);
        self.set_bone_direction(pawn, bone, rotation, [0.0; 3], 1.0);
    }

    /// APawn::TickSpecial, logic part: dead pawns seen by the controllers
    /// (SeeDeadPawn), movement animation, UpdateEyeHeight and BreathTimer.
    pub(crate) fn pawn_tick_special(&mut self, pawn: ObjId, dt: f32) {
        if self.native_prop(pawn, crate::NativeField::IsDead).as_bool() {
            let counter = self.propr(pawn, "DeadPawnSightCounter").as_float() - dt;
            self.set_prop(pawn, "DeadPawnSightCounter", Value::Float(counter));
            if counter < 0.0 {
                // Each controller of Level.ControllerList without bDisableEventSeeDeadPawn (players
                // included): SeePawn(C, dead pawn, bMaySkipChecks = 1), with its draws.
                self.set_prop(pawn, "DeadPawnSightCounter", Value::Float(1.0));
                let mut c = self.level.and_then(|l| self.po(l, "ControllerList"));
                let mut guard = 0;
                while let Some(ctrl) = c {
                    guard += 1;
                    if guard > 10_000 || self.objects[ctrl].is_none() {
                        break;
                    }
                    if !self.pb(ctrl, "bDisableEventSeeDeadPawn") && self.see_pawn(ctrl, Some(pawn), true) {
                        // AController::eventSeeDeadPawn, from APawn::TickSpecial.
                        self.event_pub(ctrl, "SeeDeadPawn", vec![Value::Object(Some(pawn))]);
                    }
                    if self.objects[ctrl].is_none() {
                        break;
                    }
                    c = self.native_object(ctrl, Field::NextController);
                }
            }
        }
        if self.objects[pawn].is_none() {
            return;
        }
        let has_mesh = !matches!(self.native_value(pawn, Field::Mesh), Value::Object(None) | Value::Void);
        if !self.native_prop(pawn, crate::NativeField::Interpolating).as_bool() && self.native_prop(pawn, crate::NativeField::PhysicsAnimUpdate).as_bool() && has_mesh {
            self.update_movement_animation(pawn, dt);
        }
        if self.objects[pawn].is_none() {
            return;
        }
        // UpdateSpineControl / UpdateHeadControl: alive, bPhysicsAnimUpdate and a mesh.
        let alive = !self.native_prop(pawn, crate::NativeField::IsDead).as_bool();
        if alive && has_mesh && self.native_prop(pawn, crate::NativeField::PhysicsAnimUpdate).as_bool() {
            if self.native_prop(pawn, crate::NativeField::SpineControl).as_bool() {
                self.update_bone_control(pawn, dt, false);
            }
            if self.native_prop(pawn, crate::NativeField::HeadControl).as_bool() {
                self.update_bone_control(pawn, dt, true);
            }
        }
        // The weapon follows the pawn (ULevel::FarMoveActor, vt 0xA4, test 0, bNoCheck 0,
        // bAttachedMove 1) except for a pawn seen in first person by its local player:
        // IsLocallyControlled (Controller->LocalPlayerController: a PlayerController
        // whose Player is a UViewport), IsHumanControlled (PlayerController
        // controller), ViewTarget (+0x3A4) the pawn and no bBehindView (+0x334 bit 0x20).
        // A soldier's weapon, set at Active's BeginState on a motionless controller, thus takes
        // the soldier's position: ProcessTraceHit derives the damage distance from it (3 damage
        // instead of 0).
        if let Some(w) = self.native_prop(pawn, crate::NativeField::Weapon).as_object().filter(|&w| self.objects.get(w).is_some_and(|o| o.is_some())) {
            let c = self.native_prop(pawn, Field::Controller).as_object().filter(|&c| self.objects.get(c).is_some_and(|o| o.is_some()));
            let player = c.filter(|&c| self.is_a(c, "PlayerController"));
            let local = player.is_some_and(|c| self.local_player_controller(c).is_some());
            let first_person = player.is_some_and(|c| self.native_prop(c, crate::NativeField::ViewTarget).as_object() == Some(pawn) && !self.native_prop(c, crate::NativeField::BehindView).as_bool());
            if !(local && first_person) {
                let l = self.native_prop(pawn, Field::Location).as_vector();
                self.far_move_actor(w, l, false, false, true);
            }
        }
        if self.native_prop(pawn, crate::NativeField::UpdateEyeheight).as_bool() {
            self.event_pub(pawn, "UpdateEyeHeight", vec![Value::Float(dt)]);
        }
        let breath = self.native_prop(pawn, crate::NativeField::BreathTime).as_float();
        if self.native_prop(pawn, Field::Role).as_byte() == 4 && 0.0 < breath {
            let left = breath - dt;
            self.set_native_prop(pawn, crate::NativeField::BreathTime, Value::Float(left));
            if left < 0.001 {
                self.set_native_prop(pawn, crate::NativeField::BreathTime, Value::Float(0.0));
                self.event_pub(pawn, "BreathTimer", vec![]);
            }
        }
        // The pawn's shadow (Pawn.Shadow, +0x318) not drawn for more than half a
        // second and not bShadowIsStatic is detached (Detach, vtable +0x144, force = 1), like
        // that of a decoration (ADecoration::TickSpecial).
        if let Some(sp) = self.native_object(pawn, crate::NativeField::Shadow).filter(|&s| self.alive(s)) {
            let now = self.level.map_or(0.0, |l| self.native_prop(l, Field::TimeSeconds).as_float());
            if let Some(force) = crate::tick::shadow_to_detach(now, self.native_prop(pawn, Field::LastRenderTime).as_float(), self.pb(sp, "bShadowIsStatic")) {
                self.projector_detach(sp, force);
            }
        }
    }

    /// Animation channels of an actor for display: (sequence, frame 0…frames, alpha, starting
    /// bone); channel 0 always has alpha 1.
    pub fn anim_channels(&self, actor: ObjId) -> Vec<(String, f32, f32, i32)> {
        let Some(inst) = self.mesh_insts.get(&actor) else {
            return Vec::new();
        };
        inst.channels
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.sequence.is_empty())
            .map(|(i, c)| {
                let frames = inst.named(&c.sequence).map(|(_, set, si)| set.seqs[si].num_frames as f32).unwrap_or(1.0);
                (c.sequence.clone(), c.frame.max(0.0) * frames, if i == 0 { 1.0 } else { c.blend_alpha }, c.start_bone)
            })
            .collect()
    }
}

/// SetAnimRate on a channel.
pub(crate) fn cadence_channel(c: &mut Channel, rate: f32) {
    c.rate = rate * c.orig_rate;
}

/// SetAnimSequence on a channel; `orig`: Rate / NumFrames of the sequence found.
pub(crate) fn sequence_channel(c: &mut Channel, sequence: &str, orig: Option<f32>) {
    c.sequence = sequence.to_string();
    c.orig_rate = orig.unwrap_or(0.0);
}

/// UpdateBlendAlpha: |alpha − target| ≤ delta (jnp: C0 or C3) → the target.
pub(crate) fn alpha_to(c: &mut Channel, target: f32, delta: f32) {
    let gap = (c.blend_alpha - target).abs();
    c.blend_alpha = if gap <= delta {
        target
    } else if target <= c.blend_alpha {
        c.blend_alpha - delta
    } else {
        delta + c.blend_alpha
    };
}

/// CopyAnimation: the fields copied from one channel to another.
pub(crate) fn copy_channel(src: Channel, d: &mut Channel) {
    d.sequence = src.sequence;
    d.anim_index = src.anim_index;
    d.rate = src.rate;
    d.orig_rate = src.orig_rate;
    d.frame = src.frame;
    d.blend_alpha = src.blend_alpha;
    d.last = src.last;
    d.notify = src.notify;
    d.min_rate = src.min_rate;
    d.looping = src.looping;
    d.tween_rate = src.tween_rate;
    d.old_rate = src.old_rate;
    d.timed_blend = src.timed_blend;
    d.blend_target = src.blend_target;
    d.time_to_go = src.time_to_go;
}

/// What APawn::UpdateMovementAnimation reads and writes of the pawn and its skeletal instance
/// (the Vm, or a test).
pub(crate) trait PawnAnim {
    fn b(&self, name_value: &str) -> bool;
    fn set_b(&mut self, name_value: &str, v: bool);
    fn byte(&self, name_value: &str) -> u8;
    fn set_byte(&mut self, name_value: &str, v: u8);
    fn fl(&self, name_value: &str) -> f32;
    fn set_fl(&mut self, name_value: &str, v: f32);
    fn vect(&self, name_value: &str) -> [f32; 3];
    fn rotation(&self) -> [i32; 3];
    fn set_rotation(&mut self, r: [i32; 3]);
    fn name_value(&self, name_value: &str) -> String;
    fn movement_anim(&self, i: i32) -> String;
    fn movement_rates(&self) -> [f32; 4];
    /// ClimbDir of OnLadder, if there is one.
    fn ladder_dir(&self) -> Option<[f32; 3]>;
    fn time_seconds(&self) -> f32;
    /// Script event (ChangeAnimation, PlayDying…).
    fn event(&mut self, name_value: &str);
    /// Skeletal mesh (IsA USkeletalMesh).
    fn skeletal_value(&self) -> bool;
    fn set_bone_direction(&mut self, os: &str, rotation: [i32; 3], translation: [f32; 3], alpha: f32);
    fn blend_alpha(&self, ch: i32) -> f32;
    fn update_blend_alpha(&mut self, ch: i32, target_value: f32, delta: f32);
    fn set_anim_rate(&mut self, ch: i32, rate: f32);
    fn anim_sequence(&self, ch: i32) -> String;
    fn set_anim_sequence(&mut self, ch: i32, seq: &str);
    fn copy_animation(&mut self, from: i32, to: i32);
    fn anim_frame(&self, ch: i32) -> f32;
    fn set_anim_frame(&mut self, ch: i32, frame: f32);
}

/// APawn::ZeroMovementAlpha: channels [first, last) brought towards alpha 0; when they are
/// all there, they stop on frame 0.
pub(crate) fn zero_movement_alpha_pawn<H: PawnAnim>(h: &mut H, first: i32, last: i32, delta: f32) {
    let mut all_zero = true;
    for ch in first..last {
        if 0.0 < h.blend_alpha(ch) {
            all_zero = false;
            h.update_blend_alpha(ch, 0.0, delta);
        }
    }
    if all_zero {
        for ch in first..last {
            h.set_anim_rate(ch, 0.0);
            h.set_anim_frame(ch, 0.0);
        }
    }
}

/// APawn::UpdateMovementAnimation after initialization: death (bTearOff),
/// state changes (crouched, walking, falling), then walking channels according to the direction of
/// the acceleration, or turning on the spot.
pub(crate) fn movement_pawn<H: PawnAnim>(h: &mut H, dt: f32) {
    let step = dt / h.fl("BlendChangeTime");
    if h.b("bTearOff") {
        if !h.b("bPlayedDeath") {
            h.event("PlayDying");
        }
        zero_movement_alpha_pawn(h, 1, 0x10, 1.0);
        h.set_b("bSpineControl", false);
        h.set_b("bHeadControl", false);
        // Skeletal mesh, the head's direction is reset to zero
        // (SetBoneDirection("X Head", zero rotator and translation, alpha 0)).
        if h.skeletal_value() {
            h.set_bone_direction("X Head", [0; 3], [0.0; 3], 0.0);
        }
        h.set_b("bPhysicsAnimUpdate", false);
        h.set_b("bIsDead", true);
        return;
    }
    let physics = h.byte("Physics");
    let falling = physics == PHYS_FALLING;
    let crouched = h.b("bIsCrouched");
    let walking = h.b("bIsWalking");
    if crouched != h.b("bWasCrouched") || walking != h.b("bWasWalking") {
        h.event("ChangeAnimation");
        let (crouched, walking) = (h.b("bIsCrouched"), h.b("bIsWalking"));
        h.set_b("bWasCrouched", crouched);
        h.set_b("bWasWalking", walking);
    }
    if h.b("bWasFalling") == falling {
        if h.byte("Physics") != h.byte("OldPhysics") {
            h.event("ChangeAnimation");
        }
    } else {
        match h.byte("Physics") {
            PHYS_WALKING => h.event("PlayLandingAnimation"),
            PHYS_FALLING => {
                if h.b("bWasOnGround") && h.vect("Velocity")[2] > 0.0 {
                    h.event("PlayJump");
                } else {
                    h.event("PlayFalling");
                }
            }
            _ => h.event("ChangeAnimation"),
        }
        h.set_b("bWasFalling", falling);
    }
    let physics = h.byte("Physics");
    h.set_byte("OldPhysics", physics);
    h.set_b("bWasOnGround", physics == PHYS_WALKING);
    let time = h.time_seconds();
    if physics == PHYS_FALLING || h.fl("MovementBlendStartTime") > time {
        zero_movement_alpha_pawn(h, 2, 0xc, step);
        return;
    }
    let accel = h.vect("Acceleration");
    if accel != [0.0; 3] {
        zero_movement_alpha_pawn(h, 2, 4, step);
        // Directions: view (flat when walking, climbing the ladder when on a ladder).
        let mut dir = crate::ai_natives::axes(h.rotation())[0];
        if physics == PHYS_WALKING {
            dir[2] = 0.0;
        } else if physics == PHYS_LADDER {
            if let Some(d) = h.ladder_dir() {
                dir = d;
            }
        }
        let dir = safe_normal(dir);
        let a = safe_normal(accel);
        let up = [0.0, 0.0, 1.0];
        let side = safe_normal([dir[1] * up[2] - dir[2] * up[1], dir[2] * up[0] - up[2] * dir[0], dir[0] * up[1] - dir[1] * up[0]]);
        let mut forward = dir[0] * a[0] + a[2] * dir[2] + a[1] * dir[1];
        if physics == PHYS_LADDER {
            forward = forward.abs();
            if accel[2] <= 0.0 {
                forward = -forward;
            }
        }
        let lateral = side[0] * a[0] + side[2] * a[2] + side[1] * a[1];
        let f2 = forward * forward;
        let s2 = lateral * lateral;
        let frac = f2 / (s2 + f2);
        let rates = h.movement_rates();
        let fb = if forward >= 0.0 { rates[0] } else { rates[2] };
        let lr = if lateral >= 0.0 { rates[1] } else { rates[3] };
        let rate = frac * fb + (1.0 - frac) * lr;
        for ch in 4..8 {
            let wanted = h.movement_anim(ch - 4);
            if !h.anim_sequence(ch).eq_ignore_ascii_case(&wanted) {
                h.copy_animation(ch, ch + 4);
                h.set_anim_sequence(ch, &wanted);
            }
        }
        let weapon = h.name_value("WeaponAnim");
        if !h.anim_sequence(0xd).eq_ignore_ascii_case(&weapon) {
            h.set_anim_sequence(0xd, &weapon);
        }
        let moving = if forward != 0.0 || lateral != 0.0 { 1.0 } else { 0.0 };
        h.update_blend_alpha(0xd, moving, step);
        if forward >= 0.0 {
            h.update_blend_alpha(4, f2, step);
            h.update_blend_alpha(6, 0.0, step);
        } else {
            h.update_blend_alpha(4, 0.0, step);
            h.update_blend_alpha(6, f2, step);
        }
        if lateral >= 0.0 {
            h.update_blend_alpha(5, s2, step);
            h.update_blend_alpha(7, 0.0, step);
        } else {
            h.update_blend_alpha(5, 0.0, step);
            h.update_blend_alpha(7, s2, step);
        }
        for ch in 8..0xc {
            h.update_blend_alpha(ch, 0.0, step);
        }
        for ch in 4..0xe {
            h.set_anim_rate(ch, rate);
        }
        // GetAnimFrame: 0 outside the channels.
        let frame = h.anim_frame(4);
        for ch in 5..0xe {
            h.set_anim_frame(ch, frame);
        }
        return;
    }
    // At rest: turning on the spot.
    if h.vect("OldAcceleration") != [0.0; 3] {
        h.event("ChangeAnimation");
    }
    zero_movement_alpha_pawn(h, 4, 0xc, step);
    h.update_blend_alpha(0xd, 0.0, step);
    let mut rotation = h.rotation();
    rotation[1] &= 0xffff;
    h.set_rotation(rotation);
    let yaw = rotation[1] as f32;
    let mut old = h.fl("OldRotYaw");
    if yaw - old > 32768.0 {
        old += 65536.0;
        h.set_fl("OldRotYaw", old);
    } else if old - yaw > 32768.0 {
        old -= 65536.0;
        h.set_fl("OldRotYaw", old);
    }
    if physics == PHYS_WALKING && h.byte("Role") > 2 {
        let turn = (yaw - old) / dt;
        let right = turn > 1000.0;
        let left = turn < -1000.0;
        if right {
            let a = h.name_value("TurnRightAnim");
            h.set_anim_sequence(2, &a);
            h.update_blend_alpha(2, 1.0, step);
            h.set_anim_rate(2, turn.abs() * 6.103_515_6e-5);
        } else {
            zero_movement_alpha_pawn(h, 2, 3, step);
        }
        if left {
            let a = h.name_value("TurnLeftAnim");
            h.set_anim_sequence(3, &a);
            h.update_blend_alpha(3, 1.0, step);
            h.set_anim_rate(3, turn.abs() * 6.103_515_6e-5);
        } else {
            zero_movement_alpha_pawn(h, 3, 4, step);
        }
    }
    let yaw = h.rotation()[1] as f32;
    if yaw != h.fl("OldRotYaw") {
        h.set_fl("OldRotYaw", yaw);
    }
}

/// The Vm's pawn as seen by movement_pawn.
struct PawnVm<'a> {
    vm: &'a mut Vm,
    pawn: ObjId,
}

impl PawnAnim for PawnVm<'_> {
    fn b(&self, name_value: &str) -> bool {
        self.vm.propr(self.pawn, name_value).as_bool()
    }
    fn set_b(&mut self, name_value: &str, v: bool) {
        self.vm.set_prop(self.pawn, name_value, Value::Bool(v));
    }
    fn byte(&self, name_value: &str) -> u8 {
        self.vm.propr(self.pawn, name_value).as_byte()
    }
    fn set_byte(&mut self, name_value: &str, v: u8) {
        self.vm.set_prop(self.pawn, name_value, Value::Byte(v));
    }
    fn fl(&self, name_value: &str) -> f32 {
        self.vm.propr(self.pawn, name_value).as_float()
    }
    fn set_fl(&mut self, name_value: &str, v: f32) {
        self.vm.set_prop(self.pawn, name_value, Value::Float(v));
    }
    fn vect(&self, name_value: &str) -> [f32; 3] {
        self.vm.propr(self.pawn, name_value).as_vector()
    }
    fn rotation(&self) -> [i32; 3] {
        self.vm.native_prop(self.pawn, Field::Rotation).as_rotator()
    }
    fn set_rotation(&mut self, r: [i32; 3]) {
        self.vm.set_native_prop(self.pawn, Field::Rotation, Value::rotator(r[0], r[1], r[2]));
    }
    fn name_value(&self, name_value: &str) -> String {
        self.vm.propr(self.pawn, name_value).as_name()
    }
    fn movement_anim(&self, i: i32) -> String {
        self.vm.movement_anim(self.pawn, i)
    }
    fn movement_rates(&self) -> [f32; 4] {
        self.vm.movement_anim_rates(self.pawn)
    }
    fn ladder_dir(&self) -> Option<[f32; 3]> {
        self.vm.propr(self.pawn, "OnLadder").as_object().map(|l| self.vm.propr(l, "ClimbDir").as_vector())
    }
    fn time_seconds(&self) -> f32 {
        self.vm.level.map(|l| self.vm.native_prop(l, Field::TimeSeconds).as_float()).unwrap_or(self.vm.time)
    }
    fn event(&mut self, name_value: &str) {
        let pawn = self.pawn;
        let args = match name_value {
            "PlayDying" => vec![self.vm.prop(pawn, "HitDamageType"), self.vm.prop(pawn, "TakeHitLocation")],
            "PlayLandingAnimation" => vec![Value::Float(self.vm.native_prop(pawn, Field::Velocity).as_vector()[2])],
            _ => vec![],
        };
        self.vm.event_pub(pawn, name_value, args);
    }
    fn skeletal_value(&self) -> bool {
        self.vm.mesh_insts.contains_key(&self.pawn)
    }
    fn set_bone_direction(&mut self, os: &str, rotation: [i32; 3], translation: [f32; 3], alpha: f32) {
        self.vm.set_bone_direction(self.pawn, os, rotation, translation, alpha);
    }
    fn blend_alpha(&self, ch: i32) -> f32 {
        self.vm.mesh_insts.get(&self.pawn).and_then(|i| i.channel(ch)).map(|c| c.blend_alpha).unwrap_or(0.0)
    }
    fn update_blend_alpha(&mut self, ch: i32, target_value: f32, delta: f32) {
        self.vm.update_blend_alpha(self.pawn, ch, target_value, delta);
    }
    fn set_anim_rate(&mut self, ch: i32, rate: f32) {
        self.vm.set_anim_rate(self.pawn, ch, rate);
    }
    fn anim_sequence(&self, ch: i32) -> String {
        self.vm.mesh_insts.get(&self.pawn).and_then(|i| i.channel(ch)).map(|c| c.sequence.clone()).unwrap_or_default()
    }
    fn set_anim_sequence(&mut self, ch: i32, seq: &str) {
        self.vm.set_anim_sequence(self.pawn, ch, seq);
    }
    fn copy_animation(&mut self, from: i32, to: i32) {
        self.vm.copy_animation(self.pawn, from, to);
    }
    fn anim_frame(&self, ch: i32) -> f32 {
        self.vm.mesh_insts.get(&self.pawn).and_then(|i| i.channel(ch)).map(|c| c.frame).unwrap_or(0.0)
    }
    fn set_anim_frame(&mut self, ch: i32, frame: f32) {
        self.vm.channel_set(self.pawn, ch, |c| c.frame = frame);
    }
}

/// Inputs of UpdateSpineControl / UpdateHeadControl (fields read by the original code).
pub(crate) struct InputOs {
    /// PlayerReplicationInfo (+0x324) non-null.
    pub pri: bool,
    pub controller: bool,
    pub controller_rotation: Option<[i32; 3]>,
    /// APlayerController::LocalPlayerController of the controller non-null (vtable +0x150).
    pub local: bool,
    /// Location of the controller's Focus, if it exists.
    pub focus: Option<[f32; 3]>,
    pub pawn_rotation: [i32; 3],
    pub pawn_location: [f32; 3],
    pub controller_pitch: u8,
    pub flag: bool,
    pub max_yaw: i32,
    pub yaw: i32,
    pub speed: f32,
    pub angle: f32,
    pub dt: f32,
}

pub(crate) struct OutputOs {
    pub angle: f32,
    /// New value of SpineYaw / HeadYaw, if the original code writes it.
    pub new_yaw: Option<i32>,
    /// Rotation given to SetBoneDirection.
    pub rotation: [i32; 3],
}

/// Core of APawn::UpdateSpineControl and UpdateHeadControl.
pub(crate) fn check_os(e: &InputOs) -> OutputOs {
    let mut r: [i32; 3];
    if !e.pri {
        r = match (e.controller, e.controller_rotation) {
            (true, Some(c)) => c,
            _ => e.pawn_rotation,
        };
        let x = crate::ai_natives::axes(r)[0];
        if let Some(a) = e.focus.filter(|_| e.controller) {
            let b = e.pawn_location;
            let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            if (x[1] * d[1] + x[0] * d[0]) as f64 > 0.9 {
                r = crate::physics::vector_rotation(d);
            }
        }
    } else {
        r = match e.controller_rotation {
            Some(c) if e.local => c,
            _ => [(e.controller_pitch as i32) << 8, e.pawn_rotation[1], e.pawn_rotation[2]],
        };
    }
    // Frame of the rotation, axes permuted (X = Z, Y = -X, Z = -Y), then OrthoRotation.
    let [x, y, z] = crate::ai_natives::axes(r);
    let neg = |v: [f32; 3]| [-v[0], -v[1], -v[2]];
    let [pitch, mut yaw, roll] = crate::world::ortho_rotation([z, neg(x), neg(y)]);
    let pitch = if pitch > 9000 { pitch } else { 9000 };
    let two_pi = std::f32::consts::TAU;
    let pi = std::f32::consts::PI;
    let max = e.max_yaw as f64;
    let (speed, dt) = (e.speed, e.dt);
    let mut angle = e.angle;
    // fsin leaves its result at full precision on the stack: the following fimul only rounds
    // once to 24 bits (x87_d24); __ftol truncates; a negative sine gets 65535 first.
    let product = |a: f32| crate::ai_natives::x87_d24(max, (a as f64).dsin(), false);
    let mut new_yaw: Option<i32> = None;
    if e.flag {
        angle += dt * speed;
        if angle > two_pi {
            angle -= two_pi;
        }
        let v = product(angle);
        new_yaw = Some(if angle > pi { (v + 65535.0) as i32 } else { v as i32 });
    } else if e.yaw != 0 {
        if angle < pi {
            angle += dt * speed;
            if angle >= pi {
                angle = 0.0;
            }
            new_yaw = Some(product(angle) as i32);
        } else {
            angle += dt;
            if angle < two_pi {
                new_yaw = Some((product(angle) + 65535.0) as i32);
            } else {
                angle = 0.0;
                new_yaw = Some(0);
            }
        }
    }
    if let Some(v) = new_yaw {
        yaw += v;
    }
    OutputOs { angle, new_yaw, rotation: [pitch, yaw, roll] }
}

