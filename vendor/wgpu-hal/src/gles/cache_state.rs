//! A call that would set the value already in place is not sent: program, enabled tests, index buffer, uniform
//! ranges, textures and samplers per unit, front face, face culling, depth, color mask and blending. The shadow is
//! only safe as long as nothing else touches these states. It is therefore forgotten: - on each new lock of the GL
//! context (`lock_guard`, called by the `lock`s of wgl.rs/egl.rs): between two submissions, the device (creating or
//! destroying textures, programs, buffers) and presentation bind their own objects, and a destroyed name may be
//! reused; - on each command that binds other objects along the way (copies, barriers, clears by shader, indexed
//! bindings per draw buffer): see `Queue::process`.

use alloc::{format, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use glow::HasContext;

static LOCKS: AtomicU64 = AtomicU64::new(0);

/// A lock of the context has just been taken: the GL state may have changed outside the queue.
pub(super) fn lock_guard() {
    LOCKS.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn active_flag() -> bool {
    true
}

pub(super) fn active_count() -> bool {
    false
}

/// Tests enabled by glEnable/glDisable that the shadow tracks.
const CAPS: [u32; 8] = [
    glow::DEPTH_TEST,
    glow::STENCIL_TEST,
    glow::SCISSOR_TEST,
    glow::BLEND,
    glow::CULL_FACE,
    glow::POLYGON_OFFSET_FILL,
    glow::SAMPLE_ALPHA_TO_COVERAGE,
    glow::DEPTH_CLAMP,
];

/// Categories of the count: program, uniform range, texture, active unit, sampler, tests,
/// other states (faces, depth, color, blending), index buffer.
#[derive(Clone, Copy)]
pub(super) enum Kind {
    ProgramState,
    Span,
    Texture,
    Unit,
    Sampler,
    Test,
    State,
    Index,
}
const KINDS: [&str; 8] = ["program", "plage", "texture", "unit", "sampler", "test", "state", "index"];

#[derive(Default)]
struct Count {
    submission: u64,
    draws: u64,
    emitted: [u64; 8],
    avoided: [u64; 8],
}

impl Count {
    fn write_value(&mut self) {
        if self.draws > 0 {
            let mut l = format!("gl-count submission {} draws {}", self.submission, self.draws);
            for (k, name_str) in KINDS.iter().enumerate() {
                l += &format!(" {name_str} {}/{}", self.emitted[k], self.avoided[k]);
            }
            eprintln!("{l}");
        }
        *self = Self { submission: self.submission + 1, ..Default::default() };
    }
}

#[derive(Default)]
pub(super) struct Shadow {
    lock_guard: u64,
    program_value: Option<Option<glow::Program>>,
    caps: [Option<bool>; CAPS.len()],
    index: Option<Option<glow::Buffer>>,
    /// (target, binding point) → (buffer, start, size).
    ranges_list: Vec<((u32, u32), (glow::Buffer, i32, i32))>,
    /// Per unit: last (target, texture) bound.
    textures: [Option<(u32, glow::Texture)>; super::MAX_TEXTURE_SLOTS],
    samplers: [Option<Option<glow::Sampler>>; super::MAX_TEXTURE_SLOTS],
    face_before: Option<u32>,
    face_removed: Option<u32>,
    depth_test: Option<u32>,
    written_depth: Option<bool>,
    colors: Option<[bool; 4]>,
    equation: Option<(u32, u32)>,
    factors: Option<[u32; 4]>,
    tally: Count,
}

impl core::fmt::Debug for Shadow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Shadow").finish_non_exhaustive()
    }
}

impl Drop for Shadow {
    fn drop(&mut self) {
        if active_count() {
            self.tally.write_value();
        }
    }
}

impl Shadow {
    /// All the tracked state becomes unknown again.
    pub(super) fn forget(&mut self) {
        self.program_value = None;
        self.caps = Default::default();
        self.index = None;
        self.ranges_list.clear();
        self.textures = Default::default();
        self.samplers = Default::default();
        self.face_before = None;
        self.face_removed = None;
        self.depth_test = None;
        self.written_depth = None;
        self.colors = None;
        self.equation = None;
        self.factors = None;
    }

    /// Start of a command buffer (`reset_state`): if the context has been locked again since,
    /// this is a new submission and the state may have changed elsewhere.
    pub(super) fn start_value(&mut self) {
        let v = LOCKS.load(Ordering::Relaxed);
        if v != self.lock_guard {
            self.lock_guard = v;
            self.forget();
            if active_count() {
                self.tally.write_value();
            }
        }
    }

    pub(super) fn drawing(&mut self) {
        self.tally.draws += 1;
    }

    /// `true`: the call must go out (and it is counted as issued), otherwise it is avoided.
    fn emit(&mut self, kind_index: Kind, already: bool) -> bool {
        let avoided_value = already && active_flag();
        if active_count() {
            if avoided_value {
                self.tally.avoided[kind_index as usize] += 1;
            } else {
                self.tally.emitted[kind_index as usize] += 1;
            }
        }
        !avoided_value
    }

    /// Counts a call issued outside the shadow (active unit, texture parameters).
    pub(super) fn emitted(&mut self, kind_index: Kind) {
        self.emit(kind_index, false);
    }

    pub(super) unsafe fn program_value(&mut self, gl: &glow::Context, p: Option<glow::Program>) {
        if self.emit(Kind::ProgramState, self.program_value == Some(p)) {
            unsafe { gl.use_program(p) };
        }
        self.program_value = Some(p);
    }

    pub(super) unsafe fn test(&mut self, gl: &glow::Context, cap: u32, active_flag: bool) {
        let k = CAPS.iter().position(|&c| c == cap);
        let already = k.is_some_and(|k| self.caps[k] == Some(active_flag));
        if self.emit(Kind::Test, already) {
            if active_flag {
                unsafe { gl.enable(cap) };
            } else {
                unsafe { gl.disable(cap) };
            }
        }
        if let Some(k) = k {
            self.caps[k] = Some(active_flag);
        }
    }

    /// A test changed some other way (glEnablei/glDisablei): its state is no longer known.
    pub(super) fn test_unknown(&mut self, cap: u32) {
        if let Some(k) = CAPS.iter().position(|&c| c == cap) {
            self.caps[k] = None;
        }
    }

    pub(super) unsafe fn index(&mut self, gl: &glow::Context, b: Option<glow::Buffer>) {
        if self.emit(Kind::Index, self.index == Some(b)) {
            unsafe { gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, b) };
        }
        self.index = Some(b);
    }

    pub(super) unsafe fn range_value(&mut self, gl: &glow::Context, target: u32, slot: u32, buffer: glow::Buffer, offset: i32, size: i32) {
        let key_tuple = (target, slot);
        let value_raw = (buffer, offset, size);
        let k = self.ranges_list.iter().position(|(c, _)| *c == key_tuple);
        let already = k.is_some_and(|k| self.ranges_list[k].1 == value_raw);
        if self.emit(Kind::Span, already) {
            unsafe { gl.bind_buffer_range(target, slot, Some(buffer), offset, size) };
        }
        match k {
            Some(k) => self.ranges_list[k].1 = value_raw,
            None => self.ranges_list.push((key_tuple, value_raw)),
        }
    }

    pub(super) unsafe fn sampler(&mut self, gl: &glow::Context, unit: u32, s: Option<glow::Sampler>) {
        let place = self.samplers.get_mut(unit as usize);
        let already = place.as_ref().is_some_and(|p| **p == Some(s));
        if let Some(p) = place {
            *p = Some(s);
        }
        if self.emit(Kind::Sampler, already) {
            unsafe { gl.bind_sampler(unit, s) };
        }
    }

    /// `true`: the texture is already bound to this unit and nothing else needs setting (levels,
    /// depth/stencil mode); the binding is avoided.
    pub(super) fn texture_bound(&mut self, unit: u32, target: u32, t: glow::Texture, nothing_to_set: bool) -> bool {
        let already = nothing_to_set && self.textures.get(unit as usize).is_some_and(|p| *p == Some((target, t)));
        !self.emit(Kind::Texture, already)
    }

    pub(super) fn note_texture(&mut self, unit: u32, target: u32, t: glow::Texture) {
        if let Some(p) = self.textures.get_mut(unit as usize) {
            *p = Some((target, t));
        }
    }

    pub(super) unsafe fn face_before(&mut self, gl: &glow::Context, f: u32) {
        if self.emit(Kind::State, self.face_before == Some(f)) {
            unsafe { gl.front_face(f) };
        }
        self.face_before = Some(f);
    }

    pub(super) unsafe fn face_removed(&mut self, gl: &glow::Context, f: u32) {
        if self.emit(Kind::State, self.face_removed == Some(f)) {
            unsafe { gl.cull_face(f) };
        }
        self.face_removed = Some(f);
    }

    pub(super) unsafe fn depth_test(&mut self, gl: &glow::Context, f: u32) {
        if self.emit(Kind::State, self.depth_test == Some(f)) {
            unsafe { gl.depth_func(f) };
        }
        self.depth_test = Some(f);
    }

    pub(super) unsafe fn written_depth(&mut self, gl: &glow::Context, m: bool) {
        if self.emit(Kind::State, self.written_depth == Some(m)) {
            unsafe { gl.depth_mask(m) };
        }
        self.written_depth = Some(m);
    }

    pub(super) unsafe fn colors(&mut self, gl: &glow::Context, m: [bool; 4]) {
        if self.emit(Kind::State, self.colors == Some(m)) {
            unsafe { gl.color_mask(m[0], m[1], m[2], m[3]) };
        }
        self.colors = Some(m);
    }

    /// Masks and blending changed per draw buffer (glColorMaski, glBlend*i, glEnablei):
    /// their global values are no longer known.
    pub(super) fn colors_by_buffer(&mut self) {
        self.colors = None;
        self.equation = None;
        self.factors = None;
        self.test_unknown(glow::BLEND);
    }

    /// glBlendEquation(c) if `c == a`, otherwise glBlendEquationSeparate(c, a), as before.
    pub(super) unsafe fn equation(&mut self, gl: &glow::Context, c: u32, a: u32, separated: bool) {
        if self.emit(Kind::State, self.equation == Some((c, a))) {
            if separated {
                unsafe { gl.blend_equation_separate(c, a) };
            } else {
                unsafe { gl.blend_equation(c) };
            }
        }
        self.equation = Some((c, a));
    }

    /// glBlendFunc(sc, dc) if not separate (then `sa == sc`, `da == dc`), otherwise glBlendFuncSeparate.
    pub(super) unsafe fn factors(&mut self, gl: &glow::Context, f: [u32; 4], separated: bool) {
        if self.emit(Kind::State, self.factors == Some(f)) {
            if separated {
                unsafe { gl.blend_func_separate(f[0], f[1], f[2], f[3]) };
            } else {
                unsafe { gl.blend_func(f[0], f[1]) };
            }
        }
        self.factors = Some(f);
    }
}
