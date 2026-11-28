//! XIII materials and their translation to Direct3D 8 states, reimplemented from D3DDrv.dll.
//!
//! The engine passes a UMaterial to FD3DRenderInterface::SetMaterial, which
//! translates it into one or more passes: frame buffer blend states, alpha test,
//! depth, and up to four texture stages (D3DTOP/D3DTA operations and arguments).
//! This module reproduces that translation exactly: SetShaderMaterial,
//! SetSimpleMaterial, HandleCombinedMaterial, ApplyFinalBlend,
//! ApplyTexModifier and the CheckMaterial<T> templates
//! The texture modifier matrices
//! come from Engine.dll (UTexPanner::GetMatrix, UTexScaler,
//! UTexRotator, UTexOscillator, UTexMatrix,
//! UTexEnvMap).
//!
//! Objects are read with their class defaults (defaultproperties of the .u files),
//! then their serialized properties, like the engine's object loader.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::props::{PVal, PropCache, Props};
use openxiii_package::library::Library;
use std::collections::HashMap;

use crate::props::FastMap;

/// Direct3D 8 constants used by D3DDrv.
pub mod d3d {
    pub const TOP_DISABLE: u32 = 1;
    pub const TOP_SELECTARG1: u32 = 2;
    pub const TOP_SELECTARG2: u32 = 3;
    pub const TOP_MODULATE: u32 = 4;
    pub const TOP_MODULATE2X: u32 = 5;
    pub const TOP_ADD: u32 = 7;
    pub const TOP_SUBTRACT: u32 = 10;
    pub const TOP_BLENDDIFFUSEALPHA: u32 = 12;
    pub const TOP_BLENDTEXTUREALPHA: u32 = 13;
    pub const TOP_BLENDCURRENTALPHA: u32 = 16;
    pub const TOP_MODULATEALPHA_ADDCOLOR: u32 = 18;
    pub const TOP_MULTIPLYADD: u32 = 25;
    pub const TA_DIFFUSE: u32 = 0;
    pub const TA_CURRENT: u32 = 1;
    pub const TA_TEXTURE: u32 = 2;
    pub const TA_TFACTOR: u32 = 3;
    pub const TA_COMPLEMENT: u32 = 0x10;
    pub const TA_ALPHAREPLICATE: u32 = 0x20;
    pub const TADDRESS_WRAP: u32 = 1;
    pub const TADDRESS_CLAMP: u32 = 3;
    pub const BLEND_ZERO: u32 = 1;
    pub const BLEND_ONE: u32 = 2;
    pub const BLEND_SRCCOLOR: u32 = 3;
    pub const BLEND_INVSRCCOLOR: u32 = 4;
    pub const BLEND_SRCALPHA: u32 = 5;
    pub const BLEND_INVSRCALPHA: u32 = 6;
    pub const BLEND_DESTCOLOR: u32 = 9;
    pub const TCI_CAMERASPACEPOSITION: u32 = 0x20000;
    pub const TCI_CAMERASPACEREFLECTIONVECTOR: u32 = 0x30000;
    pub const TTFF_PROJECTED: u32 = 0x100;
}
use d3d::*;

/// D3DCAPS8::MaxSimultaneousTextures of a modern graphics card (UD3DRenderDevice+0x1A4):
/// D3DDrv derives the number of stages per pass from it.
pub const MAX_SIMULTANEOUS_TEXTURES: usize = 8;
/// UD3DRenderDevice+0x628: default W addressing mode (3 = CLAMP, 1 on NVidia cards).
const DEVICE_ADDRESS_W: u32 = TADDRESS_CLAMP;
/// Stages of a pass (FD3DMaterialState: 5 × 0x80 bytes).
pub const STAGES: usize = 5;

pub const IDENTITY: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Texture of a stage: image of a BitmapMaterial (object path) or the current lightmap.
#[derive(Debug, Clone, PartialEq)]
pub enum TexRef {
    Bitmap(String),
    Lightmap,
}

/// One texture stage (0x80 bytes in FD3DMaterialState).
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub texture: Option<TexRef>,
    pub address: [u32; 3],
    pub color_op: u32,
    pub alpha_op: u32,
    pub color_arg0: u32,
    pub color_arg1: u32,
    pub color_arg2: u32,
    pub alpha_arg0: u32,
    pub alpha_arg1: u32,
    pub alpha_arg2: u32,
    pub result_arg: u32,
    pub tex_coord_index: u32,
    pub transform_flags: u32,
    pub use_matrix: bool,
    pub matrix: [f32; 16],
    /// D3DTSS_MAGFILTER/MINFILTER = D3DTEXF_POINT (cartoon rendering ramp); otherwise linear.
    pub point_filter: bool,
}

impl Default for Stage {
    fn default() -> Self {
        Stage {
            texture: None,
            address: [TADDRESS_CLAMP; 3],
            color_op: TOP_DISABLE,
            alpha_op: TOP_DISABLE,
            color_arg0: TA_CURRENT,
            color_arg1: TA_CURRENT,
            color_arg2: TA_CURRENT,
            alpha_arg0: TA_CURRENT,
            alpha_arg1: TA_CURRENT,
            alpha_arg2: TA_CURRENT,
            result_arg: TA_CURRENT,
            tex_coord_index: 0,
            transform_flags: 0,
            use_matrix: false,
            matrix: IDENTITY,
            point_filter: false,
        }
    }
}

/// A pass (FD3DMaterialState, 0x2A0 bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    pub alpha_blending: bool,
    pub alpha_test: bool,
    pub alpha_ref: u8,
    pub z_test: bool,
    pub z_write: bool,
    pub two_sided: bool,
    /// Pass-specific fog colour (+0x06); IgnoreFog (+0x07).
    pub override_fog_color: bool,
    pub ignore_fog: bool,
    /// +0x08: 2 = solid, 0 = wireframe, 1 = lighting only.
    pub fill_mode: i32,
    /// D3DRS_TEXTUREFACTOR (D3DCOLOR 0xAARRGGBB).
    pub tfactor: u32,
    pub src_blend: u32,
    pub dest_blend: u32,
    /// D3DCOLOR.
    pub fog_color: u32,
    pub stages_used: usize,
    pub stages: [Stage; STAGES],
}

impl Default for Pass {
    fn default() -> Self {
        Pass {
            alpha_blending: false,
            alpha_test: false,
            alpha_ref: 0,
            z_test: true,
            z_write: true,
            two_sided: false,
            override_fog_color: false,
            ignore_fog: false,
            fill_mode: 2,
            tfactor: 0,
            src_blend: BLEND_ONE,
            dest_blend: BLEND_ZERO,
            fog_color: 0,
            stages_used: 0,
            stages: Default::default(),
        }
    }
}

/// Lighting state of the FD3DRenderInterface at the time of SetMaterial (EnableLighting).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lighting {
    /// D3DRS_LIGHTING (+0x154).
    pub lighting: bool,
    /// D3DRS_COLORVERTEX (+0x155).
    pub color_vertex: bool,
    /// Lightmap provided (+0x158).
    pub lightmap: bool,
    /// +0x156.
    pub lighting_only: bool,
}

/// Result of SetMaterial: the passes to draw (FD3DRenderInterface+0xC14).
#[derive(Debug, Clone, Default)]
pub struct MaterialPasses {
    pub passes: Vec<Pass>,
    /// The material requires sorting (UMaterial::RequiresSorting): drawn after opaque geometry.
    pub requires_sorting: bool,
    /// D3DDrv error messages (FString* ErrorString).
    pub error: Option<String>,
}

/// Binding of a section to an immutable source name. New geometry creates new
/// bindings; a change in the replacements renews those used afterwards.
#[derive(Default)]
pub struct BatchMaterial {
    binding: Option<(u64, bool)>,
    replacement: Option<String>,
    is_static: bool,
    translated: Option<(Lighting, std::rc::Rc<MaterialPasses>)>,
}

impl BatchMaterial {
    /// Forgets the binding: the next set_batch_material translates the material again (the
    /// mods build's texture changes on scenery actors).
    pub fn reset(&mut self) {
        self.binding = None;
    }
}

/// Engine frame and time (GEngine+0xD4: frame number, +0xD0: frame duration).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameTime {
    pub count: u32,
    pub delta: f32,
}

/// FModifierInfo (0x54 bytes).
#[derive(Debug, Clone)]
struct ModifierInfo {
    modified_texcoords: bool,
    final_blend: bool,
    color_modifier: bool,
    tex_coord_source: u8,
    tex_coord_count: u8,
    tex_coord_projected: bool,
    frame_buffer_blending: u8,
    z_write: bool,
    z_test: bool,
    alpha_test: bool,
    two_sided: bool,
    ignore_fog: bool,
    alpha_ref: u8,
    alpha_blend: bool,
    matrix: [f32; 16],
    color: u32,
}

impl Default for ModifierInfo {
    fn default() -> Self {
        ModifierInfo {
            modified_texcoords: false,
            final_blend: false,
            color_modifier: false,
            tex_coord_source: 0,
            tex_coord_count: 0,
            tex_coord_projected: false,
            frame_buffer_blending: 0,
            z_write: true,
            z_test: true,
            alpha_test: false,
            two_sided: false,
            ignore_fog: false,
            alpha_ref: 0,
            alpha_blend: false,
            matrix: IDENTITY,
            color: 0,
        }
    }
}

/// Transient state of a texture modifier ("transient" fields of the C++ class).
#[derive(Debug, Clone, Default)]
struct ModState {
    last_frame_count: u8,
    cur_u: f32,
    cur_v: f32,
    current_rotation: [i32; 3],
    tex_coord_source: u8,
    m: Option<[f32; 16]>,
}

/// Loaded material: shared properties (Rc: the `get(…).cloned()` calls of the render paths no
/// longer copy the property table every frame) and the state of its modifiers.
#[derive(Debug, Clone)]
struct Obj {
    props: std::rc::Rc<Props>,
    state: ModState,
}

impl std::ops::Deref for Obj {
    type Target = Props;
    fn deref(&self) -> &Props {
        &self.props
    }
}

/// GMath.SinTab[(a >> 2) & 0x3FFF] with a signed integer a.
fn sin_u(a: i32) -> f32 {
    crate::coords::gmath_sin(a)
}

/// Truncation towards zero of the compiler's float → integer conversions.
fn ftol(x: f32) -> i32 {
    x as i32
}

/// Product of 4×4 row-major matrices (out = a × b).
/// D3DXMATRIX::operator* as D3DDrv compiled it in x87 (CheckMaterial<T>): each
/// element sums its four products in its own order.
fn mat_mul_op(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    const ORDER: [[usize; 4]; 16] = [
        [0, 2, 1, 3], [0, 2, 1, 3], [0, 2, 1, 3], [1, 3, 0, 2],
        [1, 2, 3, 0], [0, 1, 2, 3], [1, 2, 3, 0], [0, 3, 2, 1],
        [1, 2, 3, 0], [0, 1, 3, 2], [1, 3, 2, 0], [0, 3, 2, 1],
        [1, 2, 3, 0], [0, 1, 3, 2], [1, 3, 2, 0], [0, 3, 2, 1],
    ];
    std::array::from_fn(|i| {
        let (r, c) = (i / 4, i % 4);
        let t: [f32; 4] = std::array::from_fn(|k| a[r * 4 + k] * b[k * 4 + c]);
        let o = ORDER[i];
        ((t[o[0]] + t[o[1]]) + t[o[2]]) + t[o[3]]
    })
}

fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            o[r * 4 + c] = a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c] + a[r * 4 + 3] * b[12 + c];
        }
    }
    o
}

/// State of an animated UTexture held at run time: AnimCurrent (+0x74, None: null), Accumulator
/// (+0x80, reset to 0 by UTexture::PostLoad), PrimeCurrent (+0x79), LastFrameCount
/// (+0x7A).
#[derive(Debug, Clone)]
pub struct TexAnim {
    pub current: Option<String>,
    pub accumulator: f32,
    pub prime_current: u8,
    pub last_frame: u8,
}

/// UTexture::Tick of texture `path`, `anim_next` giving the AnimNext of a texture
/// in the chain: UTexture::Prime (ConstantTimeTick up to PrimeCount), then one more
/// frame per call if MaxFrameRate is zero, otherwise Accumulator += dt and one frame per period
/// 1 / MaxFrameRate (clamped to [0.1; 100]). Outside the editor, the last frame of an open chain
/// stays displayed.
pub fn texture_tick(path: &str, a: &mut TexAnim, prime_count: u8, max_frame_rate: f32, dt: f32, mut anim_next: impl FnMut(&str) -> Option<String>) {
    // UTexture::ConstantTimeTick: AnimCurrent (or the texture) → its AnimNext, or
    // the texture at the end of the chain.
    while a.prime_current < prime_count {
        a.prime_current += 1;
        let cur = a.current.clone().unwrap_or_else(|| path.to_string());
        a.current = Some(anim_next(&cur).unwrap_or_else(|| path.to_string()));
    }
    if a.current.is_none() {
        a.current = Some(path.to_string());
    }
    if max_frame_rate == 0.0 {
        let cur = a.current.clone().unwrap();
        if let Some(next) = anim_next(&cur) {
            a.current = Some(next);
        }
        return;
    }
    let rate = if max_frame_rate < 0.1 {
        0.1
    } else if max_frame_rate < 100.0 {
        max_frame_rate
    } else {
        100.0
    };
    let period = 1.0 / rate;
    a.accumulator += dt;
    while period <= a.accumulator {
        let cur = a.current.clone().unwrap();
        if let Some(next) = anim_next(&cur) {
            a.current = Some(next);
        }
        a.accumulator -= period;
    }
}

/// Translation of the projector at the same rank of the last list. 's static
/// combiner has an incoming and outgoing state, even if the material key is reused.
struct ProjectorKeySlot {
    texture: String,
    flags: u16,
    matrix: [u32; 16],
    gradient: [u32; 16],
    case_in: u8,
    case_out: u8,
    key: Option<std::rc::Rc<str>>,
}

/// Fixed translation of this projector batch in its last view. The draw's
/// material includes the two matrices and 's combiner case.
#[derive(Default)]
pub struct ProjectorMaterial {
    key: Option<std::rc::Rc<str>>,
    generation: u64,
    fixed: bool,
    translated: Option<([u32; 16], std::rc::Rc<MaterialPasses>)>,
}

/// Loaded materials and the transient state of their modifiers.
#[derive(Default)]
pub struct Materials {
    props: PropCache,
    objects: FastMap<String, Option<Obj>>,
    /// 's static UCombiner: last case set (FrameBufferBlending 1, 2, 5 or 6),
    /// kept as is for the other modes.
    proj_case: u8,
    projector_keys: Vec<Option<ProjectorKeySlot>>,
    /// Animated textures (AnimNext chain), by lowercase path.
    anims: FastMap<String, TexAnim>,
    /// SinusModifier states, by lowercase path.
    sine: FastMap<String, crate::sine::SineState>,
    /// AActor::execReplaceATextureByAnOther: (source, replacement) in call order.
    replacements: Vec<(String, String)>,
    replacement_generation: u64,
    /// set_material of materials with nothing that depends on time or view (see
    /// `is_static`): lowercase path -> passes per lighting state (4 bits). Cleared when the replacements
    /// change.
    pass_cache: FastMap<String, [Option<std::rc::Rc<MaterialPasses>>; 16]>,
    /// is_static by lowercase path.
    static_cache: FastMap<String, bool>,
    /// Variability by lowercase path. Cleared when the replacements change.
    variability_cache: FastMap<String, u8>,
    /// set_material of the other materials within the same frame and view: (frame number and duration,
    /// view) and lowercase path -> passes per lighting state. Their states (LastFrameCount of
    /// modifiers and animated textures, SinusModifier) only change once per frame: a second
    /// translation within the frame returns the same passes without changing anything.
    image_key: Option<(u32, u32, [u32; 16])>,
    image_cache: FastMap<String, [Option<std::rc::Rc<MaterialPasses>>; 16]>,
    /// Projectors ("#proj:…") whose texture does not depend on time: the key
    /// contains everything else (matrices included); with the same view (world coordinates
    /// passed through the view), the passes are the same from one frame to the next. Kept for two frames
    /// (current and previous): a moving projector changes key every frame.
    proj_image: u32,
    /// Recipes of fixed projectors, bounded like the passes to the last two frames.
    projector_plans: FastMap<String, Option<ProjectorPlan>>,
    previous_projector_plans: FastMap<String, Option<ProjectorPlan>>,
    /// Templates of projector plans: same key without the two
    /// matrices; None if a plan of that shape cannot be reused.
    templates: FastMap<String, Option<TemplateProjector>>,
    /// Recipes of terrain layers ("#terrain:", variability 1) by lowercase path and
    /// lighting: passes of one translation and stages with TexCoordSource 8 recomputed for each
    /// view (same computation as ApplyTexModifier). Terrain has nothing that depends on time: kept
    /// until the replacements change.
    terrain_plans: FastMap<(String, u8), Option<ProjectorPlan>>,
    /// While recording a terrain recipe: modifiers unrelated to the view
    /// (TexCoordSource other than 8 and 10) do not prevent the recipe.
    recipe_terrain: bool,
    proj_current: FastMap<(String, [u32; 16]), [Option<std::rc::Rc<MaterialPasses>>; 16]>,
    proj_previous: FastMap<(String, [u32; 16]), [Option<std::rc::Rc<MaterialPasses>>; 16]>,
    /// Buffer for `get`'s lowercase key.
    key: String,
}

#[cfg(test)]
#[test]
fn projector_key_replays_case_and_exact_matrix_bits() {
    let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut m = Materials::new();
    let mut matrix = [0.0; 16];
    let gradient = matrix;
    let first = m.projector_key_at(0, &mut library, "#shadow:2", 1, &matrix, &gradient).unwrap();
    assert_eq!(m.proj_case, 5);
    m.proj_case = 0;
    let cached = m.projector_key_at(0, &mut library, "#shadow:2", 1, &matrix, &gradient).unwrap();
    assert!(std::rc::Rc::ptr_eq(&first, &cached));
    assert_eq!(m.proj_case, 5);
    matrix[0] = -0.0;
    let expected = m.projector_key(&mut library, "#shadow:2", 1, &matrix, &gradient).unwrap();
    let updated = m.projector_key_at(0, &mut library, "#shadow:2", 1, &matrix, &gradient).unwrap();
    assert_eq!(updated.as_ref(), expected);
    assert_ne!(updated, first);
    m.set_replacements(&[("a".into(), "b".into())]);
    assert!(m.projector_keys.is_empty());
}

#[cfg(test)]
#[test]
fn projector_material_tracks_key_view_frame_and_replacements() {
    use std::rc::Rc;
    let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut m = Materials::new();
    let mut slot = ProjectorMaterial::default();
    let shadow = m.projector_key_at(0, &mut library, "#shadow:1", 1, &IDENTITY, &IDENTITY).unwrap();
    let dynamic = m.projector_key_at(1, &mut library, "#123", 1, &IDENTITY, &IDENTITY).unwrap();
    let draw = |m: &mut Materials, slot: &mut ProjectorMaterial, lib: &mut Library, key, count, view| {
        m.set_projector_material(slot, lib, key, FrameTime { count, delta: 0.03 }, view)
    };
    let first = draw(&mut m, &mut slot, &mut library, &shadow, 1, IDENTITY);
    let kept = draw(&mut m, &mut slot, &mut library, &shadow, 2, IDENTITY);
    assert!(Rc::ptr_eq(&first, &kept));
    let mut other_view = IDENTITY;
    other_view[1] = -0.0;
    let moved = draw(&mut m, &mut slot, &mut library, &shadow, 2, other_view);
    assert!(!Rc::ptr_eq(&kept, &moved));
    m.set_replacements(&[("a".into(), "b".into())]);
    let replaced = draw(&mut m, &mut slot, &mut library, &shadow, 2, other_view);
    assert!(!Rc::ptr_eq(&moved, &replaced));

    m.set_replacements(&[]);
    let a = draw(&mut m, &mut slot, &mut library, &dynamic, 2, other_view);
    let b = draw(&mut m, &mut slot, &mut library, &dynamic, 3, other_view);
    assert!(!Rc::ptr_eq(&replaced, &a));
    assert!(!Rc::ptr_eq(&a, &b));
    assert!(slot.translated.is_none());
}

#[cfg(test)]
#[test]
fn absent_projector_expires_while_replacements_are_active() {
    let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut m = Materials::new();
    m.set_replacements(&[("source".into(), "replacement".into())]);
    let q = m.projector_key_at(0, &mut library, "#shadow:0", 1, &IDENTITY, &IDENTITY).unwrap();
    let p = m.projector_key_at(1, &mut library, "#shadow:1", 1, &IDENTITY, &IDENTITY).unwrap();
    let mut q_slot = ProjectorMaterial::default();
    let mut draw = |slot: &mut ProjectorMaterial, key: &std::rc::Rc<str>, count| {
        m.set_projector_material(slot, &mut library, key, FrameTime { count, delta: 0.03 }, IDENTITY)
    };
    draw(&mut q_slot, &q, 1);
    let before = draw(&mut ProjectorMaterial::default(), &p, 1);
    for frame in 2..=4 { draw(&mut q_slot, &q, frame); }
    let after = draw(&mut ProjectorMaterial::default(), &p, 4);
    assert!(!std::rc::Rc::ptr_eq(&before, &after));
}

#[cfg(test)]
#[test]
fn batch_material_tracks_lighting_replacements_and_dynamic_frames() {
    let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut materials = Materials::new();
    let mut slot = BatchMaterial::default();
    let source = Some("Test.Fixed");
    let lit = Lighting { lighting_only: true, ..Default::default() };
    let draw = |materials: &mut Materials, slot: &mut BatchMaterial, library: &mut Library, replace, lighting, count| {
        let (r, name) = materials.set_batch_material(slot, library, source, replace, lighting, FrameTime { count, ..Default::default() }, IDENTITY);
        (r, name.map(str::to_owned))
    };
    let (a, _) = draw(&mut materials, &mut slot, &mut library, true, lit, 1);
    let (b, _) = draw(&mut materials, &mut slot, &mut library, true, lit, 2);
    assert!(std::rc::Rc::ptr_eq(&a, &b));
    let (c, _) = draw(&mut materials, &mut slot, &mut library, true, Lighting::default(), 3);
    assert!(!std::rc::Rc::ptr_eq(&a, &c));
    materials.set_replacements(&[("test.fixed".into(), "Next".into()), ("NEXT".into(), "#123".into())]);
    let (d, name) = draw(&mut materials, &mut slot, &mut library, true, lit, 4);
    let (e, _) = draw(&mut materials, &mut slot, &mut library, true, lit, 5);
    assert_eq!(name.as_deref(), Some("#123"));
    assert!(!std::rc::Rc::ptr_eq(&d, &e));
    let (_, name) = draw(&mut materials, &mut slot, &mut library, false, lit, 5);
    assert_eq!(name.as_deref(), source);
}

/// `path` as ASCII lowercase into `out` (to_ascii_lowercase without allocation).
/// Optional object path passed to `Materials::get`: a `&str` without copying it into an
/// `Option<String>` (one allocation per call), or an `Option<String>` already read.
trait PathSource {
    fn path_buf(&self) -> Option<&str>;
}
impl PathSource for str {
    fn path_buf(&self) -> Option<&str> {
        Some(self)
    }
}
impl PathSource for Option<String> {
    fn path_buf(&self) -> Option<&str> {
        self.as_deref()
    }
}

fn lower_into(out: &mut String, path: &str) {
    out.clear();
    out.push_str(path);
    out.make_ascii_lowercase();
}

/// AActor::execReplaceATextureByAnOther on a loaded object: Diffuse, Opacity,
/// Specular, SelfIllumination and Detail of a UShader (+0x4C, +0x50, +0x54, +0x5C, +0x64; not the
/// masks), Material of a UModifier (+0x4C), each replacement in order.
fn replace_fields(props: &mut Props, list: &[(String, String)]) {
    let fields: &[&str] = if props.is_a("Shader") {
        &["diffuse", "opacity", "specular", "selfillumination", "detail"]
    } else if props.is_a("Modifier") {
        &["material"]
    } else {
        return;
    };
    for (src, dst) in list {
        for f in fields {
            if let Some(PVal::Obj(Some(cur))) = props.values.get(*f) {
                if cur.eq_ignore_ascii_case(src) {
                    props.values.insert(f.to_string(), PVal::Obj((!dst.is_empty()).then(|| dst.clone())));
                }
            }
        }
    }
}

struct ViewMatrix {
    pass: usize,
    stage: usize,
    world: [f32; 16],
}

/// Projector plan for a key shape ("#proj:fb:case:flags:|texture"): passes of one
/// recording and, for each stage recomputed per view, which of the key's two matrices
/// (0: texture matrix, 1: gradient) is its world matrix. A moving projector
/// (character shadow) changes key every frame: its plan follows from this without retranslating
/// the whole synthetic graph.
struct TemplateProjector {
    passes: std::rc::Rc<MaterialPasses>,
    stages_list: Vec<(usize, usize, u8)>,
}

/// Shape of a "#proj:fb:case:flags:M:G|texture" key (without the matrices) and its two
/// matrices, None if either is the identity or if they are equal (indistinguishable roles).
fn split_key_projector(key: &str) -> Option<(String, [f32; 16], [f32; 16])> {
    let (head_value, texture) = key.split_once('|')?;
    let mut fields_list = head_value.splitn(6, ':');
    let (proj, fb, case_value, flag_bits, m, g) = (fields_list.next()?, fields_list.next()?, fields_list.next()?, fields_list.next()?, fields_list.next()?, fields_list.next()?);
    if proj != "#proj" {
        return None;
    }
    let read_from = |t: &str| -> Option<[f32; 16]> {
        let mut out = [0f32; 16];
        let mut n = 0;
        for (i, h) in t.split(',').enumerate() {
            *out.get_mut(i)? = f32::from_bits(u32::from_str_radix(h, 16).ok()?);
            n += 1;
        }
        (n == 16).then_some(out)
    };
    let (m, g) = (read_from(m)?, read_from(g)?);
    let bits = |x: &[f32; 16]| x.map(f32::to_bits);
    if bits(&m) == bits(&IDENTITY) || bits(&g) == bits(&IDENTITY) || bits(&m) == bits(&g) {
        return None;
    }
    Some((format!("#proj:{fb}:{case_value}:{flag_bits}:|{texture}"), m, g))
}

struct ProjectorPlan {
    passes: std::rc::Rc<MaterialPasses>,
    matrices: Vec<ViewMatrix>,
}

impl ProjectorPlan {
    fn for_view(&self, view: &[f32; 16]) -> std::rc::Rc<MaterialPasses> {
        if self.matrices.is_empty() { return self.passes.clone(); }
        let inv = invert(view);
        let mut out = (*self.passes).clone();
        for matrix in &self.matrices {
            out.passes[matrix.pass].stages[matrix.stage].matrix = projector_view_matrix(&inv, &matrix.world);
        }
        std::rc::Rc::new(out)
    }
}

// Same arithmetic path as ApplyTexModifier for TexCoordSource = 8.
fn projector_view_matrix(inverse_view: &[f32; 16], world: &[f32; 16]) -> [f32; 16] {
    if *world == IDENTITY { *inverse_view } else { mat_mul(inverse_view, world) }
}

/// Context of a translation (FD3DRenderInterface members read by the reimplemented functions).
struct Ctx {
    lighting: Lighting,
    frame: FrameTime,
    projector_matrices: Option<Vec<ViewMatrix>>,
    /// Terrain recipe: view-independent TexCoordSource values kept (see `Materials::recipe_terrain`).
    sources_fixed: bool,
    /// Current view matrix (FD3DRenderInterface+0x90), for generated coordinates.
    view: [f32; 16],
    passes: Vec<Pass>,
    error: Option<String>,
}

impl Ctx {
    fn pass(&mut self, p: usize) -> &mut Pass {
        while self.passes.len() <= p {
            self.passes.push(Pass::default());
        }
        &mut self.passes[p]
    }
    fn stage(&mut self, p: usize, s: usize) -> &mut Stage {
        // D3DDrv's stage overflows write into the next pass (0x80 × 5 = 0x280):
        // we stay within bounds and report it.
        let s = if s >= STAGES {
            self.err(format!("stage {s} outside the pass"));
            STAGES - 1
        } else {
            s
        };
        &mut self.pass(p).stages[s]
    }
    fn err(&mut self, e: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(e.into());
        }
    }
    /// New pass with default values.
    fn reset_pass(&mut self, p: usize) {
        *self.pass(p) = Pass::default();
        if let Some(matrices) = self.projector_matrices.as_mut() { matrices.retain(|m| m.pass != p); }
    }
}

impl Materials {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every material reachable from `path`, added to `out` once (`seen`: lowercase paths already
    /// visited): the material itself, the materials its object properties name (Diffuse,
    /// Opacity, Material1 / Material2, FallbackMaterial, Shader and Combiner inputs…) and every
    /// frame of its texture animation (AnimNext chain). Only their properties are read, as
    /// drawing them would: no animation state advances.
    pub fn reachable(&mut self, library: &mut Library, path: &str, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
        let mut stack = vec![path.to_string()];
        while let Some(p) = stack.pop() {
            if p.starts_with('#') || !seen.insert(p.to_ascii_lowercase()) {
                continue;
            }
            #[cfg(feature = "mods")]
            if crate::mod_assets::reachable(&p, out, &mut stack) {
                continue;
            }
            if !self.load(library, &p) {
                continue;
            }
            let Some(Some(o)) = self.objects.get(&p.to_ascii_lowercase()) else { continue };
            if !o.is_a("Material") {
                // Fonts, StaticMeshes and skeletal meshes: the textures and materials their
                // serialized data names (font pages, section materials, default skins).
                let (font, static_mesh, mesh) = (o.is_a("Font"), o.is_a("StaticMesh"), o.is_a("Mesh"));
                if font || static_mesh || mesh {
                    if let Some((lp, i)) = library.resolve(&p) {
                        if font {
                            if let Ok(f) = crate::font::Font::read(&lp.name, &lp.package, &lp.data, i, "") {
                                stack.extend(f.pages.into_iter().map(|pg| pg.texture).filter(|t| !t.is_empty()));
                            }
                        } else {
                            let refs: Vec<i32> = if static_mesh {
                                crate::static_mesh::StaticMesh::read(&lp.package, &lp.data, i).map(|m| m.materials).unwrap_or_default()
                            } else {
                                crate::skeletal::SkeletalMesh::read(&lp.package, &lp.data, i).map(|m| m.textures).unwrap_or_default()
                            };
                            stack.extend(refs.into_iter().filter(|&r| r != 0).map(|r| crate::props::qualified(&lp.name, &lp.package, r)));
                        }
                    }
                }
                continue;
            }
            for v in o.values.values() {
                if let crate::props::PVal::Obj(Some(x)) = v {
                    stack.push(x.clone());
                }
            }
            out.push(p);
        }
    }

    fn load(&mut self, library: &mut Library, path: &str) -> bool {
        let key = path.to_ascii_lowercase();
        if let Some(o) = self.objects.get(&key) {
            return o.is_some();
        }
        let obj = match path.strip_prefix('#') {
            Some(_) => self.synthetic(library, path),
            None => self.props.object(library, path).cloned(),
        };
        let obj = obj.map(|mut props| {
            replace_fields(&mut props, &self.replacements);
            let state = ModState {
                tex_coord_source: props.byte("TexCoordSource"),
                last_frame_count: props.byte("LastFrameCount"),
                cur_u: props.float("CurPosU"),
                cur_v: props.float("CurPosV"),
                current_rotation: props.rot("CurrentRotation"),
                m: None,
            };
            Obj { props: std::rc::Rc::new(props), state }
        });
        let found = obj.is_some();
        self.objects.insert(key, obj);
        found
    }

    /// Materials created by the engine for UCanvas (keys of `openxiii_vm::canvas::HudDraw`):
    /// "#white" (GEngine->WhiteTextureMaterial), "#n" (the script's render texture n),
    /// "#canvas:fb:test:ref|material" (UCanvas::DrawTile's static UFinalBlend: ZWrite,
    /// ZTest and TwoSided cleared) and "#canvasline:fb:test" (FCanvasUtil::DrawLine's on
    /// WhiteTextureMaterial, class defaults).
    /// Material of an FProjectorRenderInfo: "#proj:…" key of a UFinalBlend
    /// (ZWrite cleared, FrameBufferBlending of ProjTexture if it is a FinalBlend, otherwise 1) over a
    /// UCombiner of two UTexMatrix in world coordinates: the texture through `matrix` (projected
    /// if `flags` & 1), ProjGradient (bFade, `flags` & 4) or ProjClip through `gradient`. The
    /// matrices are row-major (row vector). None if ProjTexture is missing.
    pub fn projector_key(&mut self, library: &mut Library, texture: &str, flags: u16, matrix: &[f32; 16], gradient: &[f32; 16]) -> Option<String> {
        if texture.is_empty() {
            return None;
        }
        let tex = Some(texture.to_string());
        // Shadow (ShadowProjector): FB_Darken FinalBlend of the UShadowBitmapMaterial.
        let (fb, inner) = if let Some(slot) = texture.strip_prefix("#shadow:") {
            (5, format!("#shadowtex:{slot}"))
        } else if self.is_a(library, &tex, "FinalBlend") {
            let o = self.get(library, &tex)?;
            (o.byte("FrameBufferBlending"), o.obj("Material").unwrap_or_default())
        } else {
            (1, texture.to_string())
        };
        if matches!(fb, 1 | 2 | 5 | 6) {
            self.proj_case = fb;
        }
        // Same text as format!("{:08x}") of each float joined by commas, written in one go
        // (one format! per float, 32 per projector and per frame, was expensive).
        let mut key = format!("#proj:{fb}:{}:{flags}:", self.proj_case);
        key.reserve(2 * 16 * 9 + 2 + inner.len());
        hex_matrix(&mut key, matrix);
        key.push(':');
        hex_matrix(&mut key, gradient);
        key.push('|');
        key.push_str(&inner);
        Some(key)
    }

    /// Texture at the end of `path_buf`'s FinalBlends, with both addressing modes clamped (UClampMode and
    /// VClampMode at TC_Clamp): read at the edge outside its frame (projector clipping).
    pub fn texture_blocked_edges(&mut self, library: &mut Library, path_buf: &str) -> bool {
        let mut m = Some(path_buf.to_string());
        for _ in 0..4 {
            let Some(o) = self.get(library, &m) else { return false };
            if o.is_a("FinalBlend") {
                m = o.obj("Material");
                continue;
            }
            return o.is_a("Texture") && o.byte("UClampMode") == 1 && o.byte("VClampMode") == 1;
        }
        false
    }

    /// Keeps one key per projector of the list, bounded by the maximum number of draws in a view.
    /// Floats are compared by bits as in the material's hexadecimal name.
    pub fn projector_key_at(&mut self, index: usize, library: &mut Library, texture: &str, flags: u16, matrix: &[f32; 16], gradient: &[f32; 16]) -> Option<std::rc::Rc<str>> {
        let m = matrix.map(f32::to_bits);
        let g = gradient.map(f32::to_bits);
        if let Some(slot) = self.projector_keys.get(index).and_then(Option::as_ref) {
            if slot.case_in == self.proj_case && slot.flags == flags && slot.matrix == m && slot.gradient == g && slot.texture == texture {
                self.proj_case = slot.case_out;
                return slot.key.clone();
            }
        }
        let case_in = self.proj_case;
        let key = self.projector_key(library, texture, flags, matrix, gradient).map(std::rc::Rc::<str>::from);
        if self.projector_keys.len() <= index {
            self.projector_keys.resize_with(index + 1, || None);
        }
        self.projector_keys[index] = Some(ProjectorKeySlot {
            texture: texture.to_owned(), flags, matrix: m, gradient: g, case_in, case_out: self.proj_case, key: key.clone(),
        });
        key
    }

    /// Lighting is fixed for projector draws. Animated textures
    /// still go through SetMaterial in draw order.
    pub fn set_projector_material(&mut self, slot: &mut ProjectorMaterial, library: &mut Library, key: &std::rc::Rc<str>, frame: FrameTime, view: [f32; 16]) -> std::rc::Rc<MaterialPasses> {
        let lighting = Lighting { lighting: false, color_vertex: false, lightmap: false, lighting_only: false };
        // A replacement may give an animated texture to a TexMatrix classified as fixed by
        // its original name. All projectors must then advance the cache per
        // frame: another projector absent for two frames loses its old pass.
        if !self.replacements.is_empty() {
            return self.set_material(library, Some(key.as_ref()), lighting, frame, view);
        }
        if slot.generation != self.replacement_generation || !slot.key.as_ref().is_some_and(|k| std::rc::Rc::ptr_eq(k, key)) {
            slot.fixed = self.projector_is_fixed(library, key);
            slot.key = Some(key.clone());
            slot.generation = self.replacement_generation;
            slot.translated = None;
        }
        let bits = view.map(f32::to_bits);
        if let Some((_, passes)) = slot.translated.as_ref().filter(|(v, _)| *v == bits && !crate::diag::check_materials()) {
            return passes.clone();
        }
        let passes = self.set_material(library, Some(key.as_ref()), lighting, frame, view);
        if slot.fixed {
            slot.translated = Some((bits, passes.clone()));
        }
        passes
    }

    /// Projector with a fixed material (translation with no advancing state: cache or plan per view);
    /// classification kept in `slot` like set_projector_material.
    pub fn fixed_projector(&mut self, slot: &mut ProjectorMaterial, library: &mut Library, key: &std::rc::Rc<str>) -> bool {
        if slot.generation != self.replacement_generation || !slot.key.as_ref().is_some_and(|k| std::rc::Rc::ptr_eq(k, key)) {
            slot.fixed = self.projector_is_fixed(library, key);
            slot.key = Some(key.clone());
            slot.generation = self.replacement_generation;
            slot.translated = None;
        }
        slot.fixed
    }

    fn projector_is_fixed(&mut self, library: &mut Library, key: &str) -> bool {
        key.starts_with("#proj:") && key.rsplit_once('|').is_some_and(|(_, inner)| inner.starts_with("#shadowtex:") || self.is_static(library, inner, 0))
    }

    fn synthetic(&mut self, library: &mut Library, path: &str) -> Option<Props> {
        use crate::props::PVal;
        // Terrain layers (ATerrainInfo::Render, UTerrainRenderMaterial::SetTerrain):
        // "#terrain:matrix|texture|alpha map" = UFinalBlend (FB_AlphaBlend, ZWrite, ZTest,
        // no alpha test) of a UShader (Diffuse = Opacity = UCombiner, ModulateStaticLighting2X);
        // the UCombiner (CO_Multiply) multiplies the texture by a UTexMatrix in world coordinates
        // and by the alpha map (UTexModifier, stream 0), which is also its mask.
        if let Some(rest) = path.strip_prefix("#terrain") {
            let (kind, rest) = rest.split_once(':')?;
            let mut parts = rest.split('|');
            let (m, tex, am) = (parts.next()?, parts.next()?, parts.next()?);
            let suffix = format!(":{rest}");
            return match kind {
                "" => {
                    let mut p = self.props.class(library, "Engine.FinalBlend")?;
                    p.values.insert("material".into(), PVal::Obj(Some(format!("#terrainsh{suffix}"))));
                    p.values.insert("framebufferblending".into(), PVal::Byte(2));
                    p.values.insert("zwrite".into(), PVal::Bool(true));
                    p.values.insert("ztest".into(), PVal::Bool(true));
                    p.values.insert("alphatest".into(), PVal::Bool(false));
                    p.values.insert("alpharef".into(), PVal::Byte(0));
                    Some(p)
                }
                "sh" => {
                    let mut p = self.props.class(library, "Engine.Shader")?;
                    let c = Some(format!("#terraincb{suffix}"));
                    p.values.insert("diffuse".into(), PVal::Obj(c.clone()));
                    p.values.insert("opacity".into(), PVal::Obj(c));
                    p.values.insert("modulatestaticlighting2x".into(), PVal::Bool(true));
                    Some(p)
                }
                "cb" => {
                    let mut p = self.props.class(library, "Engine.Combiner")?;
                    let a = Some(format!("#terrainam{suffix}"));
                    p.values.insert("combineoperation".into(), PVal::Byte(TERRAIN_LAYER.combine_operation));
                    p.values.insert("material1".into(), PVal::Obj(Some(format!("#terraintm{suffix}"))));
                    p.values.insert("material2".into(), PVal::Obj(a.clone()));
                    p.values.insert("mask".into(), PVal::Obj(a));
                    p.values.insert("modulate2x".into(), PVal::Bool(TERRAIN_LAYER.modulate2x));
                    Some(p)
                }
                "tm" => {
                    let m = m.rsplit(':').next()?;
                    let v: Vec<u32> = m.split(',').filter_map(|x| u32::from_str_radix(x, 16).ok()).collect();
                    if v.len() != 16 {
                        return None;
                    }
                    let mut p = self.props.class(library, "Engine.TexMatrix")?;
                    p.values.insert("material".into(), PVal::Obj(Some(tex.to_string())));
                    p.values.insert("texcoordsource".into(), PVal::Byte(TERRAIN_LAYER.matrix_source));
                    p.values.insert("matrix".into(), PVal::Raw(v.iter().flat_map(|b| b.to_le_bytes()).collect()));
                    Some(p)
                }
                "am" => {
                    let mut p = self.props.class(library, "Engine.TexModifier")?;
                    p.values.insert("material".into(), PVal::Obj(Some(am.to_string())));
                    p.values.insert("texcoordsource".into(), PVal::Byte(TERRAIN_LAYER.alpha_source));
                    Some(p)
                }
                _ => None,
            };
        }
        // Projectors: "#proj", "#projc" (UCombiner), "#projt" and "#projg"
        // (UTexMatrix: TexCoordSource 8, TexCoordCount 1).
        if let Some(rest) = path.strip_prefix("#proj") {
            let (kind, rest) = rest.split_once(':')?;
            let (head, inner) = rest.split_once('|')?;
            let f: Vec<&str> = head.split(':').collect();
            let (fb, case, flags): (u8, u8, u16) = (f.first()?.parse().ok()?, f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?);
            let matrix = |s: &str| -> Option<Vec<u8>> {
                let v: Vec<u32> = s.split(',').filter_map(|x| u32::from_str_radix(x, 16).ok()).collect();
                (v.len() == 16).then(|| v.iter().flat_map(|b| b.to_le_bytes()).collect())
            };
            let suffix = format!(":{head}|{inner}");
            let tex_matrix = |s: &mut Self, library: &mut Library, material: String, raw: Vec<u8>, projected: bool| -> Option<Props> {
                let mut p = s.props.class(library, "Engine.TexMatrix")?;
                p.values.insert("material".into(), PVal::Obj(Some(material)));
                p.values.insert("texcoordsource".into(), PVal::Byte(8));
                p.values.insert("texcoordcount".into(), PVal::Byte(1));
                p.values.insert("texcoordprojected".into(), PVal::Bool(projected));
                p.values.insert("matrix".into(), PVal::Raw(raw));
                Some(p)
            };
            return match kind {
                "" => {
                    let mut p = self.props.class(library, "Engine.FinalBlend")?;
                    p.values.insert("material".into(), PVal::Obj(Some(format!("#projc{suffix}"))));
                    p.values.insert("framebufferblending".into(), PVal::Byte(fb));
                    p.values.insert("zwrite".into(), PVal::Bool(false));
                    Some(p)
                }
                "c" => {
                    let mut p = self.props.class(library, "Engine.Combiner")?;
                    let (t, g) = (Some(format!("#projt{suffix}")), Some(format!("#projg{suffix}")));
                    let (m1, m2, mask, op, aop, invert) = match case {
                        1 => (t, g.clone(), g, 5, 0, true),
                        2 => (t, g, None, 0, 1, false),
                        5 | 6 => (g, t, None, 7, 1, false),
                        _ => (None, None, None, 0, 0, false),
                    };
                    p.values.insert("material1".into(), PVal::Obj(m1));
                    p.values.insert("material2".into(), PVal::Obj(m2));
                    p.values.insert("mask".into(), PVal::Obj(mask));
                    p.values.insert("combineoperation".into(), PVal::Byte(op));
                    p.values.insert("alphaoperation".into(), PVal::Byte(aop));
                    p.values.insert("invertmask".into(), PVal::Bool(invert));
                    Some(p)
                }
                "t" => tex_matrix(self, library, inner.to_string(), matrix(f.get(3)?)?, flags & 1 != 0),
                "g" => {
                    let texture = if flags & 4 != 0 { "Engine.ProjGradient" } else { "Engine.ProjClip" };
                    tex_matrix(self, library, texture.to_string(), matrix(f.get(4)?)?, false)
                }
                _ => None,
            };
        }
        // Trail (ATrail::Render): "#trail:FrameBufferBlending|texture", UFinalBlend
        // built with its class defaults.
        if let Some(rest) = path.strip_prefix("#trail:") {
            let (fb, inner) = rest.split_once('|')?;
            let mut p = self.props.class(library, "Engine.FinalBlend")?;
            p.values.insert("material".into(), PVal::Obj(Some(inner.to_string())));
            p.values.insert("framebufferblending".into(), PVal::Byte(fb.parse().ok()?));
            return Some(p);
        }
        // Sprite (FDynamicActor::Render): the static UFinalBlend,
        // "#sprite:FrameBufferBlending:ZWrite:ZTest:AlphaTest:AlphaRef:IgnoreFog|texture",
        // TwoSided always set.
        if let Some(rest) = path.strip_prefix("#sprite:") {
            let (head, inner) = rest.split_once('|')?;
            let f: Vec<u8> = head.split(':').filter_map(|x| x.parse().ok()).collect();
            let mut p = self.props.class(library, "Engine.FinalBlend")?;
            p.values.insert("material".into(), PVal::Obj(Some(inner.to_string())));
            p.values.insert("framebufferblending".into(), PVal::Byte(*f.first()?));
            p.values.insert("zwrite".into(), PVal::Bool(*f.get(1)? != 0));
            p.values.insert("ztest".into(), PVal::Bool(*f.get(2)? != 0));
            p.values.insert("alphatest".into(), PVal::Bool(*f.get(3)? != 0));
            p.values.insert("alpharef".into(), PVal::Byte(*f.get(4)?));
            p.values.insert("ignorefog".into(), PVal::Bool(*f.get(5)? != 0));
            p.values.insert("twosided".into(), PVal::Bool(true));
            return Some(p);
        }
        if path == "#white" {
            let mut p = self.props.class(library, "Engine.BitmapMaterial")?;
            p.values.insert("usize".into(), PVal::Int(1));
            p.values.insert("vsize".into(), PVal::Int(1));
            return Some(p);
        }
        if let Some(rest) = path.strip_prefix("#canvas:") {
            let (head, inner) = rest.split_once('|')?;
            let f: Vec<u8> = head.split(':').filter_map(|x| x.parse().ok()).collect();
            let mut p = self.props.class(library, "Engine.FinalBlend")?;
            p.values.insert("material".into(), PVal::Obj((!inner.is_empty()).then(|| inner.to_string())));
            p.values.insert("framebufferblending".into(), PVal::Byte(*f.first()?));
            p.values.insert("zwrite".into(), PVal::Bool(false));
            p.values.insert("ztest".into(), PVal::Bool(false));
            p.values.insert("twosided".into(), PVal::Bool(false));
            p.values.insert("alphatest".into(), PVal::Bool(*f.get(1)? != 0));
            p.values.insert("alpharef".into(), PVal::Byte(*f.get(2)?));
            return Some(p);
        }
        if let Some(rest) = path.strip_prefix("#canvasline:") {
            let f: Vec<u8> = rest.split(':').filter_map(|x| x.parse().ok()).collect();
            let mut p = self.props.class(library, "Engine.FinalBlend")?;
            p.values.insert("material".into(), PVal::Obj(Some("#white".into())));
            p.values.insert("framebufferblending".into(), PVal::Byte(*f.first()?));
            let test = *f.get(1)? != 0;
            p.values.insert("alphatest".into(), PVal::Bool(test));
            if test {
                p.values.insert("alpharef".into(), PVal::Byte(127));
            }
            return Some(p);
        }
        // D3DDrv shadow target (256 × 256).
        if path.starts_with("#shadowtex:") {
            return self.props.class(library, "Engine.RenderTargetMaterial");
        }
        // The script's render texture (RenderTargetMaterial, 256 × 256).
        path[1..].parse::<usize>().ok()?;
        self.props.class(library, "Engine.RenderTargetMaterial")
    }

    fn get<P: PathSource + ?Sized>(&mut self, library: &mut Library, path: &P) -> Option<&Obj> {
        let path = path.path_buf()?;
        // Lowercase key in a kept buffer: no allocation when the object is already
        // loaded (the case every frame).
        lower_into(&mut self.key, path);
        if !self.objects.contains_key(self.key.as_str()) {
            if !self.load(library, path) {
                return None;
            }
            lower_into(&mut self.key, path);
        }
        self.objects.get(self.key.as_str()).and_then(Option::as_ref)
    }

    fn get_mut(&mut self, path: &str) -> Option<&mut Obj> {
        lower_into(&mut self.key, path);
        self.objects.get_mut(self.key.as_str()).and_then(Option::as_mut)
    }

    /// Class (most derived) of a material, for diagnostics.
    pub fn class_of(&mut self, library: &mut Library, path: &str) -> Option<String> {
        self.get(library, path).map(|o| o.props.chain[0].clone())
    }

    /// UObject::IsA of material `path` (its class or one of its parents).
    pub fn material_is_a(&mut self, library: &mut Library, path: &str, class: &str) -> bool {
        self.is_a(library, path, class)
    }

    /// Whether object `path` is of class `class` (or a subclass).
    pub fn is_a<P: PathSource + ?Sized>(&mut self, library: &mut Library, path: &P, class: &str) -> bool {
        self.get(library, path).is_some_and(|o| o.is_a(class))
    }

    /// Texture (UTexture) properties used by the renderer.
    pub fn texture_info(&mut self, library: &mut Library, path: &str) -> Option<TextureInfo> {
        let o = self.get(library, path)?;
        o.is_a("BitmapMaterial").then(|| TextureInfo {
            masked: o.bool("bMasked"),
            alpha_texture: o.bool("bAlphaTexture"),
            usize: o.int("USize"),
            vsize: o.int("VSize"),
            cube: o.is_a("Cubemap"),
        })
    }

    /// ReplaceATextureByAnOther replacements so far (the VM's list, which only
    /// grows during a map): applied to objects already loaded, and to later ones on
    /// load (the game has them all in memory from the start of the map).
    pub fn set_replacements(&mut self, list: &[(String, String)]) {
        if list.len() == self.replacements.len() && list == self.replacements.as_slice() {
            return;
        }
        self.replacement_generation += 1;
        self.projector_keys.clear();
        self.pass_cache.clear();
        self.image_cache.clear();
        self.proj_current.clear();
        self.projector_plans.clear();
        self.previous_projector_plans.clear();
        self.templates.clear();
        self.terrain_plans.clear();
        self.proj_previous.clear();
        self.static_cache.clear();
        self.variability_cache.clear();
        if !list.starts_with(&self.replacements) {
            self.objects.clear();
            self.replacements = list.to_vec();
            return;
        }
        let new = &list[self.replacements.len()..];
        if crate::diag::texture_replacements() {
            for (src, dst) in new {
                eprintln!("ReplaceATextureByAnOther {src} → {dst}");
            }
        }
        for o in self.objects.values_mut().flatten() {
            replace_fields(std::rc::Rc::make_mut(&mut o.props), new);
        }
        self.replacements = list.to_vec();
    }

    /// Material `path` after the replacements in Materials[i].Material of UStaticMeshes and the
    /// texture (+0x1D4) of UParticleEmitters.
    /// Changes with every new replacement list (callers' caches).
    pub fn generation_replacements(&self) -> u64 {
        self.replacement_generation
    }

    pub fn replaced(&self, path: &str) -> String {
        self.replacement(path).unwrap_or(path).to_owned()
    }

    /// Final destination of the replacement chain. None lets the batch keep
    /// the name it already has when no rule applies to it.
    fn replacement(&self, path: &str) -> Option<&str> {
        let mut replaced: Option<&str> = None;
        for (src, dst) in &self.replacements {
            if replaced.unwrap_or(path).eq_ignore_ascii_case(src) {
                replaced = Some(dst.as_str());
            }
        }
        replaced
    }

    /// Properties of SinusModifier `path` (None if it is not one).
    pub fn sine_params(&mut self, library: &mut Library, path: &str) -> Option<crate::sine::SineParams> {
        let o = self.get(library, path)?;
        o.is_a("SinusModifier").then(|| crate::sine::SineParams::read(o))
    }

    /// Material (+0x4C) of SinusModifier `path`, which UMeshEmitter::RenderParticles
    /// uses instead after SetSinusVertexStreams; None if it is not a SinusModifier.
    pub fn sine_material(&mut self, library: &mut Library, path: &str) -> Option<String> {
        let o = self.get(library, path)?;
        o.is_a("SinusModifier").then(|| o.obj("Material").unwrap_or_default())
    }

    /// For SinusModifier `path` (SetSinusVertexStreams): waved local
    /// positions; None if it is not a SinusModifier.
    pub fn sine_deform(
        &mut self,
        library: &mut Library,
        path: &str,
        frame: FrameTime,
        frand: &mut dyn FnMut() -> f32,
        fade: [f32; 2],
        local: &[[f32; 3]],
    ) -> Option<Vec<[f32; 3]>> {
        let o = self.get(library, path)?;
        if !o.is_a("SinusModifier") {
            return None;
        }
        let (params, last_frame) = (crate::sine::SineParams::read(o), o.byte("LastFrameCount"));
        let state = self.sine.entry(path.to_ascii_lowercase()).or_insert_with(|| crate::sine::SineState { last_frame, ..Default::default() });
        Some(crate::sine::deform(&params, state, frame.count as u8, frame.delta, frand, fade, local))
    }

    /// AnimNext of a texture (path), if any.
    fn anim_next(&mut self, library: &mut Library, path: &str) -> Option<String> {
        self.get(library, path).and_then(|o| o.obj("AnimNext"))
    }

    /// Animation state of texture `path` (created from its loaded properties).
    fn anim_state(&mut self, library: &mut Library, path: &str) -> Option<TexAnim> {
        lower_into(&mut self.key, path);
        if let Some(a) = self.anims.get(self.key.as_str()) {
            return Some(a.clone());
        }
        let o = self.get(library, path)?;
        let a = TexAnim { current: o.obj("AnimCurrent"), accumulator: 0.0, prime_current: o.byte("PrimeCurrent"), last_frame: o.byte("LastFrameCount") };
        self.anims.insert(path.to_ascii_lowercase(), a.clone());
        Some(a)
    }

    /// UTexture::Get: with AnimNext, UTexture::Update (once per
    /// frame, LastFrameCount ≠ GEngine+0xD4) calls UTexture::Tick(GEngine+0xD0); returns
    /// AnimCurrent if not null, otherwise the texture itself.
    pub fn texture_get(&mut self, library: &mut Library, path: &str, frame: FrameTime) -> String {
        let Some(mut a) = self.anim_state(library, path) else { return path.to_string() };
        // Without AnimNext, the state (stored by anim_state) does not change: nothing to write back.
        let animated = self.get(library, path).is_some_and(|o| matches!(o.get("AnimNext"), Some(PVal::Obj(Some(_)))));
        if animated {
            if let (last, _, Some(dt)) = texture_update(a.last_frame, 0, frame.count as u8, frame.delta) {
                a.last_frame = last;
                self.texture_tick(library, path, &mut a, dt);
            }
        }
        let current = a.current.clone().unwrap_or_else(|| path.to_string());
        if animated {
            lower_into(&mut self.key, path);
            match self.anims.get_mut(self.key.as_str()) {
                Some(e) => *e = a,
                None => {
                    self.anims.insert(path.to_ascii_lowercase(), a);
                }
            }
        }
        current
    }

    /// USpriteEmitter::RenderParticles of a NoSynchroAnim emitter: the texture's current frame and
    /// Accumulator are the emitter's (`cursor`, `accumulator`), LastFrameCount is
    /// set back to the previous frame (so the texture advances on every draw), then UTexture::Get; returns
    /// the frame drawn, the raw AnimCurrent and the texture's Accumulator after the call.
    pub fn texture_get_swapped(&mut self, library: &mut Library, path: &str, cursor: Option<String>, accumulator: f32, frame: FrameTime) -> (String, Option<String>, f32) {
        // An emitter may force another AnimCurrent, even on a bitmap without AnimNext.
        self.projector_plans.clear();
        self.previous_projector_plans.clear();
        self.templates.clear();
        self.terrain_plans.clear();
        let key = path.to_ascii_lowercase();
        let Some(mut a) = self.anim_state(library, path) else { return (path.to_string(), cursor, accumulator) };
        a.current = cursor;
        a.accumulator = accumulator;
        a.last_frame = (frame.count as u8).wrapping_sub(1);
        self.anims.insert(key.clone(), a);
        let drawn = self.texture_get(library, path, frame);
        let after = self.anims.get(&key).cloned().unwrap_or(TexAnim { current: None, accumulator: 0.0, prime_current: 0, last_frame: 0 });
        (drawn, after.current, after.accumulator)
    }

    /// AnimNext (+0x70) of a texture: next one in the animation chain.
    pub fn texture_anim_next(&mut self, library: &mut Library, path: &str) -> Option<String> {
        self.anim_next(library, path)
    }

    /// UTexture::Tick of a loaded texture: see `texture_tick`.
    fn texture_tick(&mut self, library: &mut Library, path: &str, a: &mut TexAnim, dt: f32) {
        let (prime_count, max_frame_rate) = match self.get(library, path) {
            Some(o) => (o.byte("PrimeCount"), o.float("MaxFrameRate")),
            None => return,
        };
        texture_tick(path, a, prime_count, max_frame_rate, dt, |cur| self.anim_next(library, cur));
    }

    /// FD3DRenderInterface::SetParticleMaterial for a sprite UParticleMaterial
    /// (USpriteEmitter::RenderParticles: DrawStyle, texture, no constant colour,
    /// double-sided, the actor's IgnoreFog): one pass, texture × vertex colour.
    pub fn particle_pass(&mut self, library: &mut Library, texture: &str, draw_style: u8, ignore_fog: bool, frame: FrameTime) -> Pass {
        self.particle_pass_frame(library, texture, draw_style, ignore_fog, frame, None)
    }

    /// Like `particle_pass`, with the animation frame already chosen by the emitter's render (NoSynchroAnim,
    /// `texture_get_swapped`): the texture is not advanced a second time.
    pub fn particle_pass_frame(&mut self, library: &mut Library, texture: &str, draw_style: u8, ignore_fog: bool, frame: FrameTime, resolved: Option<&str>) -> Pass {
        let mut p = Pass { ignore_fog, ..Pass::default() };
        p.z_write = false;
        let mut blend_diffuse_alpha = false;
        match draw_style {
            0 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = BLEND_ZERO;
                p.alpha_blending = false;
            }
            1 => {
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_INVSRCALPHA;
                p.alpha_blending = true;
            }
            2 => {
                p.src_blend = BLEND_DESTCOLOR;
                p.dest_blend = BLEND_SRCCOLOR;
                blend_diffuse_alpha = true;
                p.alpha_blending = true;
                p.override_fog_color = true;
                p.fog_color = 0x007F_7F7F;
            }
            3 | 4 | 6 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = match draw_style {
                    3 => BLEND_ONE,
                    4 => BLEND_INVSRCALPHA,
                    _ => BLEND_INVSRCCOLOR,
                };
                p.alpha_blending = true;
                p.override_fog_color = true;
                p.fog_color = 0;
            }
            5 => {
                p.src_blend = BLEND_ZERO;
                p.dest_blend = BLEND_INVSRCCOLOR;
                p.alpha_blending = true;
                p.override_fog_color = true;
                p.fog_color = 0;
            }
            7 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = BLEND_ZERO;
                p.alpha_blending = false;
                p.alpha_test = true;
                p.alpha_ref = 0x7F;
                p.z_write = true;
            }
            _ => {}
        }
        p.z_test = true;
        p.two_sided = true;
        let current = match resolved {
            Some(r) => r.to_string(),
            None => self.texture_get(library, texture, frame),
        };
        let st = &mut p.stages[0];
        if let Some(o) = self.get(library, texture) {
            st.texture = Some(TexRef::Bitmap(current));
            match o.byte("UClampMode") {
                0 => st.address[0] = TADDRESS_WRAP,
                1 => st.address[0] = TADDRESS_CLAMP,
                _ => {}
            }
            match o.byte("VClampMode") {
                0 => st.address[1] = TADDRESS_WRAP,
                1 => st.address[1] = TADDRESS_CLAMP,
                _ => {}
            }
        }
        st.address[2] = DEVICE_ADDRESS_W;
        st.color_arg1 = TA_TEXTURE;
        st.alpha_arg1 = TA_TEXTURE;
        st.color_arg2 = TA_DIFFUSE;
        st.alpha_arg2 = TA_DIFFUSE;
        if blend_diffuse_alpha {
            st.color_op = TOP_BLENDDIFFUSEALPHA;
            st.alpha_op = TOP_BLENDDIFFUSEALPHA;
        } else {
            st.color_op = TOP_MODULATE;
            st.alpha_op = TOP_MODULATE;
        }
        p.stages_used = 1;
        p
    }

    /// SetParticleMaterial: options of the loaded material, beyond the
    /// subset used by sprites. Subdivision uses two texture samples
    /// then the diffuse colour; UseTFactor only applies to the simple case.
    fn set_particle(&mut self, library: &mut Library, ctx: &mut Ctx, path: &str) -> bool {
        let Some(o) = self.get(library, path).cloned() else { return false };
        let Some(texture) = o.obj("BitmapMaterial") else { return false };
        let blend = o.int("ParticleBlending");
        let draw_style = if (0..=7).contains(&blend) { blend as u8 } else { u8::MAX };
        let mut p = self.particle_pass_frame(library, &texture, draw_style, o.bool("IgnoreFog"), ctx.frame, None);
        p.two_sided = (o.int("RenderTwoSided") as u8) != 0;
        if o.int("BlendBetweenSubdivisions") != 0 {
            p.stages[0].color_op = TOP_SELECTARG1;
            p.stages[0].alpha_op = TOP_SELECTARG1;
            p.stages[0].color_arg2 = TA_CURRENT;
            p.stages[0].alpha_arg2 = TA_CURRENT;
            let texture = p.stages[0].texture.clone();
            let address = p.stages[0].address;
            p.stages[1] = Stage {
                texture, address, color_op: TOP_BLENDDIFFUSEALPHA, alpha_op: TOP_BLENDDIFFUSEALPHA,
                color_arg1: TA_TEXTURE, alpha_arg1: TA_TEXTURE, tex_coord_index: 1,
                ..Stage::default()
            };
            diffuse_stage(&mut p.stages[2], false);
            p.stages_used = 3;
        } else if o.int("UseTFactor") != 0 {
            p.stages[0].color_arg2 = TA_TFACTOR;
            p.stages[0].alpha_arg2 = TA_TFACTOR;
        }
        ctx.passes.push(p);
        true
    }

    /// MaterialUSize and MaterialVSize of a material (FCanvasUtil, UCanvas).
    pub fn size(&mut self, library: &mut Library, path: &str) -> (i32, i32) {
        let p = Some(path.to_string());
        (self.material_size(library, &p, false), self.material_size(library, &p, true))
    }

    /// UMaterial::MaterialUSize / VSize (UBitmapMaterial: USize; Modifier, Shader, Combiner).
    fn material_size<P: PathSource + ?Sized>(&mut self, library: &mut Library, path: &P, v: bool) -> i32 {
        let Some(o) = self.get(library, path).cloned() else { return 0 };
        if o.is_a("Cubemap") {
            // UCubemap::MaterialUSize: first face.
            return self.material_size(library, &o.obj("Faces"), v);
        }
        if o.is_a("BitmapMaterial") {
            return o.int(if v { "VSize" } else { "USize" });
        }
        if o.is_a("Modifier") {
            return self.material_size(library, &o.obj("Material"), v);
        }
        if o.is_a("Shader") {
            if o.obj("Diffuse").is_some() {
                return self.material_size(library, &o.obj("Diffuse"), v);
            }
            if o.obj("SelfIllumination").is_some() {
                return self.material_size(library, &o.obj("SelfIllumination"), v);
            }
            return 0;
        }
        if o.is_a("TexCoordMaterial") {
            // UTexCoordMaterial::MaterialUSize.
            return self.material_size(library, &o.obj("Texture"), v);
        }
        if o.is_a("Combiner") {
            let a = self.material_size(library, &o.obj("Material2"), v);
            let b = self.material_size(library, &o.obj("Material1"), v);
            return b.max(a);
        }
        0
    }

    /// UTexModifier::GetMatrix (vtable +0x8C) of each subclass.
    fn get_matrix(&mut self, library: &mut Library, path: &str, frame: FrameTime) -> Option<[f32; 16]> {
        let o = self.get(library, path)?.clone();
        let fc = frame.count as u8;
        if o.is_a("TexPanner") {
            let mut st = o.state.clone();
            if fc != st.last_frame_count {
                st.last_frame_count = fc;
                // (GMath.UnitCoords / PanDirection).XAxis: X and Y components.
                let r = o.rot("PanDirection");
                let (sp_n, cp) = (sin_u(-r[0]), sin_u(0x4000 - r[0]));
                let _ = sp_n;
                let (sy_n, cy) = (sin_u(-r[1]), sin_u(0x4000 - r[1]));
                let dir_x = cp * cy;
                let dir_y = -(cp * sy_n);
                let rate = frame.delta * o.float("PanRate");
                let mut u = dir_x * rate + st.cur_u;
                st.cur_u = u;
                let mut v = rate * dir_y + st.cur_v;
                st.cur_v = v;
                if u >= 1.0 {
                    u = st.cur_u;
                    loop {
                        u -= 1.0;
                        if u < 1.0 {
                            break;
                        }
                    }
                    st.cur_u = u;
                }
                if v >= 1.0 {
                    v = st.cur_v;
                    loop {
                        v -= 1.0;
                        if v < 1.0 {
                            break;
                        }
                    }
                    st.cur_v = v;
                }
                st.cur_u -= ftol(st.cur_u) as f32;
                st.cur_v -= ftol(st.cur_v) as f32;
            }
            let mut m = IDENTITY;
            m[8] = st.cur_u;
            m[9] = st.cur_v;
            st.m = Some(m);
            self.get_mut(path)?.state = st;
            return Some(m);
        }
        if o.is_a("TexScaler") {
            let mut m = IDENTITY;
            m[0] = 1.0 / o.float("UScale");
            m[5] = 1.0 / o.float("VScale");
            let us = self.material_size(library, path, false);
            m[8] = o.float("UOffset") / us as f32;
            let vs = self.material_size(library, path, true);
            m[9] = o.float("VOffset") / vs as f32;
            return Some(m);
        }
        if o.is_a("TexRotator") {
            let mut st = o.state.clone();
            if fc != st.last_frame_count {
                st.last_frame_count = fc;
                let rot = o.rot("Rotation");
                if o.bool("ConstantRotation") {
                    let d = [ftol(rot[0] as f32 * frame.delta), ftol(rot[1] as f32 * frame.delta), ftol(rot[2] as f32 * frame.delta)];
                    for (r, d) in st.current_rotation.iter_mut().zip(d) {
                        *r = r.wrapping_add(d) & 0xFFFF;
                    }
                } else {
                    st.current_rotation = rot;
                }
            }
            let mut m = rotation_matrix(st.current_rotation);
            let (uc, vc) = (o.float("URotCenter"), o.float("VRotCenter"));
            if uc != 0.0 || vc != 0.0 {
                m[8] = uc - (m[0] * uc + m[4] * vc);
                m[9] = vc - (m[1] * uc + m[5] * vc);
            }
            st.m = Some(m);
            self.get_mut(path)?.state = st;
            return Some(m);
        }
        if o.is_a("TexOscillator") {
            let mut st = o.state.clone();
            let mut m = IDENTITY;
            if fc != st.last_frame_count {
                st.last_frame_count = fc;
                st.cur_u += frame.delta * o.float("UOscillationRate");
                st.cur_v += frame.delta * o.float("VOscillationRate");
                st.cur_u -= ftol(st.cur_u) as f32;
                st.cur_v -= ftol(st.cur_v) as f32;
            }
            const TWO_PI: f32 = 6.283_185_5;
            // appSin (double) stays in st(0): fmul dword by the amplitude, a single rounding to 24 bits.
            let su = (((st.cur_u * TWO_PI + o.float("UOscillationPhase")) as f64).dsin() * o.float("UOscillationAmplitude") as f64) as f32;
            let sv = (((st.cur_v * TWO_PI + o.float("VOscillationPhase")) as f64).dsin() * o.float("VOscillationAmplitude") as f64) as f32;
            match o.byte("UOscillationType") {
                0 => m[8] = su,
                1 => {
                    m[0] = su + 1.0;
                    m[8] = (1.0 - m[0]) * o.float("UCenter");
                }
                _ => {}
            }
            match o.byte("VOscillationType") {
                0 => m[9] = sv,
                1 => {
                    m[5] = sv + 1.0;
                    m[9] = (1.0 - m[5]) * o.float("VCenter");
                }
                _ => {}
            }
            st.m = Some(m);
            self.get_mut(path)?.state = st;
            return Some(m);
        }
        if o.is_a("TexEnvMap") {
            // Changes TexCoordSource and gives no matrix.
            let tcs = match o.byte("EnvMapType") {
                0 => Some(10),
                1 => Some(11),
                _ => None,
            };
            if let Some(t) = tcs {
                self.get_mut(path)?.state.tex_coord_source = t;
            }
            return None;
        }
        if o.is_a("TexMatrix") {
            return Some(o.matrix("Matrix"));
        }
        // UTexModifier::GetMatrix: no matrix.
        None
    }

    /// CheckMaterial<T>: the material if it has the requested class, otherwise by walking the
    /// Modifier chain; then fills the FModifierInfo.
    fn check(&mut self, library: &mut Library, mat: &Option<String>, class: &str, mi: Option<&mut ModifierInfo>, frame: FrameTime) -> Option<String> {
        if self.is_a(library, mat, class) {
            return mat.clone();
        }
        if !self.is_a(library, mat, "Modifier") {
            return None;
        }
        let mut found = None;
        let mut m = mat.clone();
        while self.is_a(library, &m, "Modifier") {
            let inner = self.get(library, &m)?.obj("Material");
            found = if self.is_a(library, &inner, class) { inner.clone() } else { None };
            m = inner;
        }
        // (template for UBitmapMaterial): the material found is returned even without
        // an FModifierInfo to fill.
        let Some(found) = found else { return None };
        let Some(mi) = mi else { return Some(found) };
        let mut first = true;
        let mut m = mat.clone();
        while self.is_a(library, &m, "Modifier") {
            let path = m.clone().unwrap();
            let o = self.get(library, &m)?.clone();
            if o.is_a("TexModifier") {
                if let Some(mm) = self.get_matrix(library, &path, frame) {
                    mi.modified_texcoords = true;
                    mi.matrix = mat_mul_op(&mi.matrix, &mm);
                }
                let o = self.get(library, &m)?.clone();
                if first && o.state.tex_coord_source != 13 {
                    mi.modified_texcoords = true;
                    mi.tex_coord_source = o.state.tex_coord_source;
                    mi.tex_coord_count = o.byte("TexCoordCount");
                    mi.tex_coord_projected = o.bool("TexCoordProjected");
                    first = false;
                }
            }
            if o.is_a("FinalBlend") {
                mi.final_blend = true;
                mi.frame_buffer_blending = o.byte("FrameBufferBlending");
                mi.z_write = o.bool("ZWrite");
                mi.z_test = o.bool("ZTest");
                mi.alpha_test = o.bool("AlphaTest");
                mi.two_sided = o.bool("TwoSided");
                mi.alpha_ref = o.byte("AlphaRef");
                mi.ignore_fog = o.bool("IgnoreFog");
            }
            if o.is_a("ColorModifier") {
                mi.color_modifier = true;
                mi.alpha_blend |= o.bool("AlphaBlend");
                mi.two_sided |= o.bool("RenderTwoSided");
                mi.color = o.color("Color");
            }
            m = o.obj("Material");
        }
        Some(found)
    }

    /// Cartoon rendering material: the texture (a UTexture, or the Diffuse — failing
    /// that the SelfIllumination — of a UShader, otherwise the default FallbackMaterial) and the
    /// blend taken from its flags or from the shader's OutputBlending. Other classes
    /// (Combiner, modifiers…) take the default texture.
    pub fn cartoon_material(&mut self, library: &mut Library, material: Option<&str>) -> CartoonMaterial {
        let mat = material.map(str::to_string);
        let fallback = |this: &mut Self, library: &mut Library| this.props.class(library, "Engine.Material").and_then(|d| d.obj("FallbackMaterial"));
        let o = self.get(library, &mat).cloned();
        let mut out = CartoonMaterial::default();
        match o {
            Some(t) if t.is_a("Texture") => {
                out.texture = mat.clone();
                if t.bool("bMasked") {
                    out.alpha_test = true;
                } else if t.bool("bAlphaTexture") {
                    out.blend = Some((BLEND_SRCALPHA, BLEND_INVSRCALPHA));
                }
            }
            Some(sh) if sh.is_a("Shader") => {
                let base = sh.obj("Diffuse").or_else(|| sh.obj("SelfIllumination"));
                out.texture = if self.is_a(library, &base, "Texture") { base } else { fallback(self, library) };
                match sh.byte("OutputBlending") {
                    1 => out.alpha_test = true,
                    2 => out.blend = Some((BLEND_DESTCOLOR, BLEND_SRCCOLOR)),
                    3 => out.blend = Some((BLEND_ONE, BLEND_ONE)),
                    5 => out.blend = Some((BLEND_SRCALPHA, BLEND_INVSRCALPHA)),
                    6 => out.blend = Some((BLEND_ZERO, BLEND_INVSRCCOLOR)),
                    7 => out.blend = Some((BLEND_ONE, BLEND_INVSRCCOLOR)),
                    0 => {}
                    // OB_Invisible, OB_AddWhiteFog: D3DDrv leaves the previous states.
                    _ => out.keeps_previous = true,
                }
                out.outline = sh.byte("OutputBlending") != 1;
                return out;
            }
            _ => out.texture = fallback(self, library),
        }
        // The outline is only drawn for a texture that is neither masked nor alpha, or an unmasked
        // shader (DrawCartoonStatic).
        out.outline = match self.get(library, &mat) {
            Some(t) if t.is_a("Texture") => !t.bool("bMasked") && !t.bool("bAlphaTexture"),
            _ => false,
        };
        out
    }

    /// UMaterial::RequiresSorting (UTexture, UShader, UFinalBlend,
    /// UModifier, UCombiner: 0).
    pub fn requires_sorting(&mut self, library: &mut Library, path: &Option<String>) -> bool {
        let Some(o) = self.get(library, path).cloned() else { return false };
        if o.is_a("Texture") {
            return o.bool("bAlphaTexture");
        }
        if o.is_a("Shader") {
            return !matches!(o.byte("OutputBlending"), 0 | 1 | 4);
        }
        if o.is_a("FinalBlend") {
            return matches!(o.byte("FrameBufferBlending"), 1..=6);
        }
        if o.is_a("Modifier") {
            return self.requires_sorting(library, &o.obj("Material"));
        }
        false
    }

    /// Texture of a BitmapMaterial in a stage, with its addressing modes.
    fn set_texture(&mut self, library: &mut Library, ctx: &mut Ctx, p: usize, s: usize, bm: &Option<String>) {
        let Some(o) = self.get(library, bm).cloned() else {
            ctx.stage(p, s).texture = None;
            return;
        };
        // UTexture::Get: current frame of an animated texture.
        let current = self.texture_get(library, bm.as_deref().unwrap(), ctx.frame);
        let st = ctx.stage(p, s);
        st.texture = Some(TexRef::Bitmap(current));
        if !o.is_a("Cubemap") {
            match o.byte("UClampMode") {
                0 => st.address[0] = TADDRESS_WRAP,
                1 => st.address[0] = TADDRESS_CLAMP,
                _ => {}
            }
            match o.byte("VClampMode") {
                0 => st.address[1] = TADDRESS_WRAP,
                1 => st.address[1] = TADDRESS_CLAMP,
                _ => {}
            }
        } else {
            st.address[0] = DEVICE_ADDRESS_W;
            st.address[1] = DEVICE_ADDRESS_W;
        }
        st.address[2] = DEVICE_ADDRESS_W;
    }

    /// FD3DRenderInterface::ApplyTexModifier.
    fn apply_tex_modifier(ctx: &mut Ctx, p: usize, s: usize, mi: &ModifierInfo) {
        if !mi.modified_texcoords {
            return;
        }
        let view = ctx.view;
        if let Some(matrices) = &mut ctx.projector_matrices {
            if mi.tex_coord_source == 8 && s < STAGES {
                matrices.push(ViewMatrix { pass: p, stage: s, world: mi.matrix });
            } else if ctx.sources_fixed && mi.tex_coord_source != 10 {
                // Without the view (0-7, 9, 11, 13): same stage for every view.
            } else {
                // A path that uses another kind of modifier keeps SetMaterial.
                ctx.projector_matrices = None;
            }
        }
        let st = ctx.stage(p, s);
        match mi.tex_coord_count {
            0 => st.transform_flags = 2,
            1 => st.transform_flags = 3,
            2 => st.transform_flags = 4,
            _ => {}
        }
        if mi.tex_coord_projected {
            st.transform_flags |= TTFF_PROJECTED;
        }
        let identity = mi.matrix == IDENTITY;
        match mi.tex_coord_source {
            0..=7 => {
                st.tex_coord_index = mi.tex_coord_source as u32;
                st.use_matrix = !identity;
                if !identity {
                    st.matrix = mi.matrix;
                }
            }
            8 => {
                let inv = invert(&view);
                st.matrix = projector_view_matrix(&inv, &mi.matrix);
                st.use_matrix = true;
                st.tex_coord_index = TCI_CAMERASPACEPOSITION;
            }
            9 => {
                st.use_matrix = !identity;
                if !identity {
                    st.matrix = mi.matrix;
                }
                st.tex_coord_index = TCI_CAMERASPACEPOSITION;
            }
            10 => {
                let m = if identity { view } else { mat_mul(&view, &mi.matrix) };
                st.matrix = transpose(&m);
                st.use_matrix = true;
                st.tex_coord_index = TCI_CAMERASPACEREFLECTIONVECTOR;
            }
            11 => {
                st.use_matrix = !identity;
                if !identity {
                    st.matrix = mi.matrix;
                }
                st.tex_coord_index = TCI_CAMERASPACEREFLECTIONVECTOR;
            }
            13 => {
                st.tex_coord_index = 0;
                st.use_matrix = false;
            }
            other => ctx.err(format!("Unknown TexCoordSource {other}")),
        }
    }

    /// FD3DRenderInterface::ApplyFinalBlend: writes into pass 0.
    fn apply_final_blend(ctx: &mut Ctx, mi: &ModifierInfo) {
        let p = ctx.pass(0);
        let fog_black = match mi.frame_buffer_blending {
            0 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = BLEND_ZERO;
                p.alpha_blending = false;
                None
            }
            1 => {
                p.src_blend = BLEND_DESTCOLOR;
                p.dest_blend = BLEND_SRCCOLOR;
                Some(0x007F_7F7F)
            }
            2 => {
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_INVSRCALPHA;
                p.alpha_blending = true;
                None
            }
            3 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = BLEND_INVSRCALPHA;
                Some(0)
            }
            4 => {
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_ONE;
                Some(0)
            }
            5 => {
                p.src_blend = BLEND_ZERO;
                p.dest_blend = BLEND_INVSRCCOLOR;
                Some(0)
            }
            6 => {
                p.src_blend = BLEND_ONE;
                p.dest_blend = BLEND_INVSRCCOLOR;
                Some(0)
            }
            7 => {
                p.src_blend = BLEND_ZERO;
                p.dest_blend = BLEND_ONE;
                Some(0)
            }
            _ => None,
        };
        if let Some(c) = fog_black {
            p.override_fog_color = true;
            p.alpha_blending = true;
            p.fog_color = c;
        }
        p.ignore_fog = mi.ignore_fog;
        p.z_write = mi.z_write;
        p.z_test = mi.z_test;
        p.two_sided = mi.two_sided;
        p.alpha_test = mi.alpha_test;
        p.alpha_ref = mi.alpha_ref;
    }

    /// FD3DRenderInterface::HandleCombinedMaterial.
    #[allow(clippy::too_many_arguments)]
    // HandleCombinedMaterial's signature: some flags are only used by recursive calls.
    #[allow(clippy::only_used_in_recursion)]
    fn handle_combined(
        &mut self,
        library: &mut Library,
        ctx: &mut Ctx,
        mat: &Option<String>,
        pass: &mut usize,
        stage: &mut usize,
        mut mi: ModifierInfo,
        use_texture_alpha: bool,
        invert_output_alpha: bool,
    ) -> bool {
        if mat.is_none() {
            return true;
        }
        let f = ctx.frame;
        if let Some(bm) = self.check(library, mat, "BitmapMaterial", Some(&mut mi), f) {
            self.set_texture(library, ctx, *pass, *stage, &Some(bm));
            let st = ctx.stage(*pass, *stage);
            st.alpha_arg1 = TA_TEXTURE | if invert_output_alpha { TA_COMPLEMENT } else { 0 };
            st.alpha_op = TOP_SELECTARG1;
            st.color_arg1 = TA_TEXTURE;
            st.color_op = TOP_SELECTARG1;
            Self::apply_tex_modifier(ctx, *pass, *stage, &mi);
            *stage += 1;
            return true;
        }
        let Some(comb_path) = self.check(library, mat, "Combiner", None, f) else {
            return true;
        };
        let comb = self.get(library, &Some(comb_path)).unwrap().clone();
        let (m1, m2, mask) = (comb.obj("Material1"), comb.obj("Material2"), comb.obj("Mask"));
        let invert_mask = comb.bool("InvertMask");
        let m1bm = self.check(library, &m1, "BitmapMaterial", None, f);
        let m2bm = self.check(library, &m2, "BitmapMaterial", None, f);
        if m1.is_none() {
            ctx.err("Combiner must specify Material1");
            return false;
        }
        // first: handled by the recursive call; this_stage: combined in this stage.
        let (first, this_stage, swapped);
        if m1bm.is_some() && m2bm.is_none() {
            first = m2.clone();
            this_stage = m1.clone();
            swapped = true;
        } else {
            if m1bm.is_none() && m2bm.is_none() {
                ctx.err("Either Material1 or Material2 must be a simple bitmap material.");
                return false;
            }
            if invert_mask && m1bm.is_some() && mask == m2 {
                first = m2.clone();
                this_stage = m1.clone();
                swapped = true;
            } else {
                first = m1.clone();
                this_stage = m2.clone();
                swapped = false;
            }
        }
        let rec_invert = mask == first && invert_mask;
        if !self.handle_combined(library, ctx, &first, pass, stage, mi.clone(), use_texture_alpha, rec_invert) {
            return false;
        }
        let mut texture_alpha = false;
        if mask.is_some() {
            if mask == this_stage {
                if !invert_mask {
                    texture_alpha = true;
                } else {
                    let mut mi2 = mi.clone();
                    let mbm = self.check(library, &mask, "BitmapMaterial", Some(&mut mi2), f);
                    self.set_texture(library, ctx, *pass, *stage, &mbm);
                    Self::apply_tex_modifier(ctx, *pass, *stage, &mi2);
                    let st = ctx.stage(*pass, *stage);
                    st.alpha_arg1 = TA_TEXTURE | TA_COMPLEMENT;
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_arg1 = TA_CURRENT;
                    st.color_op = TOP_SELECTARG1;
                    *stage += 1;
                }
            } else if mask != first {
                if self.check(library, &mask, "BitmapMaterial", None, f).is_none() {
                    ctx.err("Combiner Mask must be a simple bitmap material, or be Material1 or Material2.");
                    return false;
                }
                let mut mi2 = mi.clone();
                let mbm = self.check(library, &mask, "BitmapMaterial", Some(&mut mi2), f);
                self.set_texture(library, ctx, *pass, *stage, &mbm);
                Self::apply_tex_modifier(ctx, *pass, *stage, &mi2);
                let st = ctx.stage(*pass, *stage);
                st.alpha_arg1 = TA_TEXTURE | if invert_mask { TA_COMPLEMENT } else { 0 };
                st.alpha_op = TOP_SELECTARG1;
                st.color_arg1 = TA_CURRENT;
                st.color_op = TOP_SELECTARG1;
                *stage += 1;
            }
        }
        let mut mi3 = mi.clone();
        let bm = self.check(library, &this_stage, "BitmapMaterial", Some(&mut mi3), f);
        // UShadowBitmapMaterial (projector shadows): stage 0 modulated by
        // TFACTOR, the shadow's current intensity (texture +0x78) replicated in RGB
        // The silhouette itself is drawn white.
        if let Some(i) = bm.as_deref().and_then(shadow_target_intensity) {
            let st = ctx.stage(*pass, 0);
            st.color_op = TOP_MODULATE;
            st.color_arg1 = TA_TEXTURE;
            st.color_arg2 = TA_TFACTOR;
            ctx.pass(*pass).tfactor = i << 16 | i << 8 | i;
        }
        self.set_texture(library, ctx, *pass, *stage, &bm);
        Self::apply_tex_modifier(ctx, *pass, *stage, &mi3);
        let (p, s) = (*pass, *stage);
        let modulate2x = comb.bool("Modulate2X");
        match comb.byte("CombineOperation") {
            0 => {
                let st = ctx.stage(p, s);
                st.color_arg1 = TA_TEXTURE;
                st.color_arg2 = TA_CURRENT;
                st.color_op = if swapped { TOP_SELECTARG1 } else { TOP_SELECTARG2 };
            }
            1 => {
                let st = ctx.stage(p, s);
                st.color_arg1 = TA_TEXTURE;
                st.color_arg2 = TA_CURRENT;
                st.color_op = if swapped { TOP_SELECTARG2 } else { TOP_SELECTARG1 };
            }
            2 => {
                let st = ctx.stage(p, s);
                st.color_arg1 = TA_TEXTURE;
                st.color_arg2 = TA_CURRENT;
                st.color_op = if modulate2x { TOP_MODULATE2X } else { TOP_MODULATE };
            }
            3 => {
                let st = ctx.stage(p, s);
                st.color_arg1 = TA_TEXTURE;
                st.color_arg2 = TA_CURRENT;
                st.color_op = TOP_ADD;
            }
            op @ 4..=6 => {
                let st = ctx.stage(p, s);
                st.color_arg1 = if swapped { TA_CURRENT } else { TA_TEXTURE };
                st.color_arg2 = if swapped { TA_TEXTURE } else { TA_CURRENT };
                st.color_op = match op {
                    4 => TOP_SUBTRACT,
                    5 => {
                        if texture_alpha {
                            TOP_BLENDTEXTUREALPHA
                        } else {
                            TOP_BLENDCURRENTALPHA
                        }
                    }
                    _ => TOP_MODULATEALPHA_ADDCOLOR,
                };
            }
            7 => {
                // CO_ProjDarken: ALPHAREPLICATE on argument 1 of the previous stage.
                if s > 0 {
                    ctx.stage(p, s - 1).color_arg1 |= TA_ALPHAREPLICATE;
                }
                let st = ctx.stage(p, s);
                st.color_arg1 = TA_TEXTURE;
                st.color_arg2 = TA_CURRENT;
                st.color_op = TOP_MODULATE;
            }
            _ => {}
        }
        let mask_ok = mask.is_none() || mask == m1 || mask == m2;
        let invert_stage = |ctx: &mut Ctx, stage: &mut usize| {
            *stage += 1;
            let st = ctx.stage(p, *stage);
            st.color_arg1 = TA_CURRENT;
            st.color_op = TOP_SELECTARG1;
            st.alpha_arg1 = TA_CURRENT | TA_COMPLEMENT;
            st.alpha_op = TOP_SELECTARG1;
        };
        match comb.byte("AlphaOperation") {
            0 => {
                let st = ctx.stage(p, s);
                st.alpha_arg1 = if texture_alpha { TA_TEXTURE } else { TA_CURRENT } | if invert_output_alpha { TA_COMPLEMENT } else { 0 };
                st.alpha_op = TOP_SELECTARG1;
            }
            op @ 1..=4 => {
                if !mask_ok {
                    ctx.err(match op {
                        1 => "Combiner Mask must be Material1, or Material2 or None when using Alpha Operation AO_Multiply.",
                        2 => "Combiner Mask must be Material1, or Material2 or None when using Alpha Operation AO_Add.",
                        3 => "Combiner Mask must be Material1, or Material2 or None when using Alpha Operation AO_Use_Alpha_From_Material1.",
                        _ => "Combiner Mask must be Material1, or Material2 or None when using Alpha Operation AO_Use_Alpha_From_Material2.",
                    });
                    return false;
                }
                let st = ctx.stage(p, s);
                st.alpha_arg1 = TA_TEXTURE;
                st.alpha_arg2 = TA_CURRENT;
                st.alpha_op = match op {
                    1 => TOP_MODULATE,
                    2 => TOP_ADD,
                    3 => {
                        if swapped {
                            TOP_SELECTARG1
                        } else {
                            TOP_SELECTARG2
                        }
                    }
                    _ => {
                        if swapped {
                            TOP_SELECTARG2
                        } else {
                            TOP_SELECTARG1
                        }
                    }
                };
                if invert_output_alpha {
                    invert_stage(ctx, stage);
                }
            }
            _ => {}
        }
        *stage += 1;
        true
    }

    /// FD3DRenderInterface::SetSimpleMaterial.
    fn set_simple(&mut self, library: &mut Library, ctx: &mut Ctx, mat: &Option<String>, mut mi: ModifierInfo) -> bool {
        if !mi.final_blend {
            if let Some(t) = self.get(library, mat).filter(|o| o.is_a("Texture")).cloned() {
                let masked = t.bool("bMasked");
                if masked || t.bool("bAlphaTexture") {
                    mi.final_blend = true;
                    if masked {
                        mi.frame_buffer_blending = if mi.color_modifier && mi.alpha_blend { 2 } else { 0 };
                        mi.z_write = true;
                        mi.alpha_ref = 0x7F;
                    } else {
                        mi.frame_buffer_blending = 2;
                        mi.z_write = false;
                        mi.alpha_ref = 0;
                    }
                    mi.z_test = true;
                    mi.alpha_test = true;
                    if !mi.color_modifier {
                        mi.two_sided = false;
                    }
                }
            }
        }
        let (mut pass, mut stage) = (0usize, 0usize);
        ctx.reset_pass(0);
        if !self.handle_combined(library, ctx, mat, &mut pass, &mut stage, mi.clone(), false, false) {
            return false;
        }
        if ctx.lighting.lightmap {
            lightmap_stage(ctx.stage(pass, stage));
            stage += 1;
        }
        if ctx.lighting.color_vertex || ctx.lighting.lighting {
            let st = ctx.stage(pass, 0);
            st.color_arg2 = TA_DIFFUSE;
            st.alpha_arg2 = TA_DIFFUSE;
            st.color_op = TOP_MODULATE;
            st.alpha_op = TOP_MODULATE;
        }
        if mi.final_blend {
            Self::apply_final_blend(ctx, &mi);
        }
        if mi.color_modifier {
            let p = ctx.pass(pass);
            if mi.alpha_blend && p.src_blend == BLEND_ONE && p.dest_blend == BLEND_ZERO {
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_INVSRCALPHA;
            }
            color_modifier_stage(ctx.stage(pass, stage));
            let p = ctx.pass(pass);
            p.tfactor = mi.color;
            p.alpha_blending |= mi.alpha_blend;
            p.two_sided |= mi.two_sided;
            stage += 1;
        }
        ctx.pass(pass).stages_used = stage;
        ctx.passes.truncate(pass + 1);
        true
    }

    /// FD3DRenderInterface::SetShaderMaterial.
    fn set_shader(&mut self, library: &mut Library, ctx: &mut Ctx, shader_path: &str, mi: ModifierInfo) -> bool {
        let f = ctx.frame;
        let sh = self.get(library, shader_path).unwrap().clone();
        let max = MAX_SIMULTANEOUS_TEXTURES;
        let flags = (sh.bool("TwoSided") as u32)
            | (sh.bool("Wireframe") as u32) << 1
            | (sh.bool("ModulateStaticLighting2X") as u32) << 2
            | (sh.bool("InvertOpacity") as u32) << 3
            | (sh.bool("InvertSelfIllumMask") as u32) << 4;
        let inv_op = if flags & 8 != 0 { TA_COMPLEMENT } else { 0 };
        let inv_si = if flags & 0x10 != 0 { TA_COMPLEMENT } else { 0 };
        let (opacity, specular, spec_mask) = (sh.obj("Opacity"), sh.obj("Specular"), sh.obj("SpecularityMask"));
        let (self_illum, self_illum_mask) = (sh.obj("SelfIllumination"), sh.obj("SelfIlluminationMask"));
        let (mut pass, mut stage) = (0usize, 0usize);
        ctx.reset_pass(0);
        let mut has_lightmap = ctx.lighting.lightmap;
        let mut self_illum_only = false;
        let mut lit = ctx.lighting.color_vertex || ctx.lighting.lighting;
        let mut diffuse = sh.obj("Diffuse");
        if self_illum.is_some() && self_illum_mask.is_none() {
            self_illum_only = true;
            lit = false;
            has_lightmap = false;
            diffuse = self_illum.clone();
        }
        // Stage with opacity (jumps: final setup).
        if opacity.is_some() {
            if !self.handle_combined(library, ctx, &diffuse, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                return false;
            }
            let mut extra = 0usize;
            if lit {
                if stage == 1 {
                    let st = ctx.stage(pass, 0);
                    st.color_arg2 = TA_DIFFUSE;
                    st.alpha_arg2 = TA_DIFFUSE;
                    st.color_op = TOP_MODULATE;
                    st.alpha_op = TOP_MODULATE;
                } else {
                    diffuse_stage(ctx.stage(pass, stage), flags & 4 != 0);
                    stage += 1;
                    extra = 1;
                }
            }
            if has_lightmap {
                if stage >= extra + max {
                    ctx.err("No stages left for lightmap");
                    return false;
                }
                lightmap_stage(ctx.stage(pass, stage));
                stage += 1;
            }
            if opacity != diffuse {
                let mut mi_o = mi.clone();
                let Some(obm) = self.check(library, &opacity, "BitmapMaterial", Some(&mut mi_o), f) else {
                    ctx.err("Cannot handle complex opacity when diffuse is not a simple bitmap material");
                    return false;
                };
                if stage >= extra + max {
                    ctx.err("No stages left for opacity bitmap");
                    return false;
                }
                self.set_texture(library, ctx, pass, stage, &Some(obm));
                let st = ctx.stage(pass, stage);
                st.alpha_op = TOP_SELECTARG1;
                st.color_op = TOP_SELECTARG1;
                st.alpha_arg2 = TA_CURRENT;
                st.color_arg1 = TA_CURRENT;
                st.alpha_arg1 = TA_TEXTURE | inv_op;
                Self::apply_tex_modifier(ctx, pass, stage, &mi_o);
                stage += 1;
            }
            if !self_illum_only && self_illum.is_some() {
                ctx.pass(pass).stages_used = stage;
                pass += 1;
                stage = 0;
                ctx.reset_pass(pass);
                let mut mi_op = mi.clone();
                let Some(op_bm) = self.check(library, &opacity, "BitmapMaterial", Some(&mut mi_op), f) else {
                    ctx.err("Opacity must be a simple bitmap when using self-illumination");
                    return false;
                };
                let mut mi_m = mi.clone();
                let mask_bm = self.check(library, &self_illum_mask, "BitmapMaterial", Some(&mut mi_m), f);
                if !self.handle_combined(library, ctx, &self_illum, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                    return false;
                }
                let Some(mask_bm) = mask_bm else {
                    ctx.err("SelfIllimunationMask must be a simple bitmap material");
                    return false;
                };
                if stage + 1 >= max {
                    ctx.err(format!("No room for SelfIllimunationMask in multipass SelfIllumination (StagesUsed was {stage})"));
                    return false;
                }
                self.set_texture(library, ctx, pass, stage, &Some(mask_bm));
                let st = ctx.stage(pass, stage);
                st.alpha_op = TOP_SELECTARG1;
                st.color_op = TOP_SELECTARG1;
                st.alpha_arg2 = TA_CURRENT;
                st.color_arg1 = TA_CURRENT;
                st.alpha_arg1 = TA_TEXTURE | inv_si;
                Self::apply_tex_modifier(ctx, pass, stage, &mi_m);
                stage += 1;
                self.set_texture(library, ctx, pass, stage, &Some(op_bm));
                let st = ctx.stage(pass, stage);
                st.alpha_arg2 = TA_CURRENT;
                st.color_arg1 = TA_CURRENT;
                st.alpha_arg1 = TA_TEXTURE | inv_op;
                st.alpha_op = TOP_MODULATE;
                st.color_op = TOP_SELECTARG1;
                Self::apply_tex_modifier(ctx, pass, stage, &mi_op);
                stage += 1;
                let p = ctx.pass(pass);
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_INVSRCALPHA;
                p.alpha_blending = true;
            }
            if specular.is_some() {
                let mut mi_op = mi.clone();
                let Some(op_bm) = self.check(library, &opacity, "BitmapMaterial", Some(&mut mi_op), f) else {
                    ctx.err("Opacity must be a simple bitmap when using specular");
                    return false;
                };
                let mut mi_m = mi.clone();
                let mask_bm = self.check(library, &spec_mask, "BitmapMaterial", Some(&mut mi_m), f);
                let mask_const = self.check(library, &spec_mask, "ConstantMaterial", Some(&mut mi_m), f);
                ctx.pass(pass).stages_used = stage;
                pass += 1;
                stage = 0;
                ctx.reset_pass(pass);
                if !self.handle_combined(library, ctx, &specular, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                    return false;
                }
                if spec_mask.is_none() {
                    if stage >= max {
                        ctx.err(format!("No room for opacity in multipass specular (StagesUsed was {stage})"));
                        return false;
                    }
                    self.set_texture(library, ctx, pass, stage, &Some(op_bm));
                    let st = ctx.stage(pass, stage);
                    st.alpha_arg2 = TA_CURRENT;
                    st.color_arg1 = TA_CURRENT;
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_op = TOP_SELECTARG1;
                    st.alpha_arg1 = TA_TEXTURE | inv_op;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_op);
                } else {
                    if mask_const.is_some() {
                        ctx.err("not supported: constant SpecularityMask with opacity");
                        return false;
                    }
                    let Some(mask_bm) = mask_bm else {
                        ctx.err("SpecularityMask must be a simple bitmap material");
                        return false;
                    };
                    if stage + 1 >= max {
                        ctx.err(format!("No room for SpecularityMask in multipass specular (StagesUsed was {stage})"));
                        return false;
                    }
                    self.set_texture(library, ctx, pass, stage, &Some(mask_bm));
                    let st = ctx.stage(pass, stage);
                    st.alpha_arg1 = TA_TEXTURE;
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_op = TOP_SELECTARG1;
                    st.alpha_arg2 = TA_CURRENT;
                    st.color_arg1 = TA_CURRENT;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_m);
                    stage += 1;
                    self.set_texture(library, ctx, pass, stage, &Some(op_bm));
                    let st = ctx.stage(pass, stage);
                    st.alpha_arg2 = TA_CURRENT;
                    st.color_arg1 = TA_CURRENT;
                    st.alpha_arg1 = TA_TEXTURE | inv_op;
                    st.alpha_op = TOP_MODULATE;
                    st.color_op = TOP_SELECTARG1;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_op);
                }
                let p = ctx.pass(pass);
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_ONE;
                stage += 1;
                additive_pass(ctx.pass(pass));
            }
        } else {
            if !self.handle_combined(library, ctx, &diffuse, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                return false;
            }
            let mut separate_pass = false;
            if stage == 1 {
                if lit {
                    let st = ctx.stage(pass, 0);
                    st.color_arg2 = TA_DIFFUSE;
                    st.alpha_arg2 = TA_DIFFUSE;
                    st.color_op = TOP_MODULATE;
                    st.alpha_op = TOP_MODULATE;
                }
                if has_lightmap {
                    lightmap_stage(ctx.stage(pass, 1));
                    stage += 1;
                }
            } else if stage < max {
                if !has_lightmap {
                    if lit {
                        diffuse_stage(ctx.stage(pass, stage), flags & 4 != 0);
                        stage += 1;
                    }
                } else if lit {
                    separate_pass = true;
                } else {
                    lightmap_stage(ctx.stage(pass, stage));
                    stage += 1;
                }
            } else if lit || has_lightmap {
                separate_pass = true;
            }
            if separate_pass {
                ctx.pass(pass).stages_used = stage;
                pass += 1;
                stage = 0;
                ctx.reset_pass(pass);
                let st = ctx.stage(pass, stage);
                st.color_arg1 = TA_TEXTURE;
                st.alpha_op = TOP_SELECTARG1;
                st.alpha_arg1 = TA_DIFFUSE;
                st.color_arg2 = TA_DIFFUSE;
                st.color_op = if !has_lightmap {
                    TOP_SELECTARG2
                } else if lit {
                    TOP_MODULATE
                } else {
                    TOP_SELECTARG1
                };
                st.texture = has_lightmap.then_some(TexRef::Lightmap);
                st.address[0] = TADDRESS_WRAP;
                st.address[1] = TADDRESS_WRAP;
                st.tex_coord_index = 1;
                st.use_matrix = false;
                stage += 1;
                let p = ctx.pass(pass);
                if !lit || has_lightmap || flags & 4 != 0 {
                    p.src_blend = BLEND_DESTCOLOR;
                    p.dest_blend = BLEND_SRCCOLOR;
                    p.fog_color = 0x007F_7F7F;
                    p.override_fog_color = true;
                } else {
                    p.src_blend = BLEND_DESTCOLOR;
                    p.dest_blend = BLEND_ZERO;
                }
                p.alpha_blending = true;
            }
            if !self_illum_only && self_illum.is_some() {
                let mut mi_s = mi.clone();
                let si_bm = self.check(library, &self_illum, "BitmapMaterial", Some(&mut mi_s), f);
                let mut mi_m = mi.clone();
                let mask_bm = self.check(library, &self_illum_mask, "BitmapMaterial", Some(&mut mi_m), f);
                if pass != 0 || stage + 1 >= max || si_bm.is_none() || mask_bm.is_none() {
                    ctx.pass(pass).stages_used = stage;
                    pass += 1;
                    stage = 0;
                    ctx.reset_pass(pass);
                    if !self.handle_combined(library, ctx, &self_illum, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                        return false;
                    }
                    let Some(mask_bm) = mask_bm else {
                        ctx.err("SelfIllimunationMask must be a simple bitmap material");
                        return false;
                    };
                    if stage >= max {
                        ctx.err(format!("No room for SelfIllimunationMask in multipass SelfIllumination (StagesUsed was {stage})"));
                        return false;
                    }
                    self.set_texture(library, ctx, pass, stage, &Some(mask_bm));
                    let st = ctx.stage(pass, stage);
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_op = TOP_SELECTARG1;
                    st.alpha_arg2 = TA_CURRENT;
                    st.color_arg1 = TA_CURRENT;
                    st.alpha_arg1 = TA_TEXTURE | inv_si;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_m);
                    stage += 1;
                    let p = ctx.pass(pass);
                    p.src_blend = BLEND_SRCALPHA;
                    p.dest_blend = BLEND_INVSRCALPHA;
                    p.alpha_blending = true;
                } else {
                    // D3DDrv writes into pass 0 here (which is the current pass).
                    self.set_texture(library, ctx, 0, stage, &mask_bm);
                    let st = ctx.stage(0, stage);
                    st.alpha_arg2 = TA_CURRENT;
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_arg1 = TA_CURRENT;
                    st.color_op = TOP_SELECTARG1;
                    st.alpha_arg1 = TA_TEXTURE | inv_si;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_m);
                    stage += 1;
                    self.set_texture(library, ctx, pass, stage, &si_bm);
                    let st = ctx.stage(pass, stage);
                    st.alpha_arg1 = TA_CURRENT;
                    st.alpha_op = TOP_SELECTARG1;
                    st.color_arg1 = TA_TEXTURE;
                    st.color_arg2 = TA_CURRENT;
                    st.color_op = TOP_BLENDCURRENTALPHA;
                    Self::apply_tex_modifier(ctx, pass, stage, &mi_s);
                    stage += 1;
                }
            }
            if specular.is_some() {
                let mut mi_s = mi.clone();
                let spec_bm = self.check(library, &specular, "BitmapMaterial", Some(&mut mi_s), f);
                let mut mi_m = mi.clone();
                let mask_bm = self.check(library, &spec_mask, "BitmapMaterial", Some(&mut mi_m), f);
                let mask_const = self.check(library, &spec_mask, "ConstantMaterial", Some(&mut mi_m), f);
                let mut multipass = pass != 0;
                if !multipass {
                    // Mask absent or equal to the Diffuse, or constant mask: a single stage;
                    // otherwise two stages or one more pass.
                    let single = stage < max && spec_bm.is_some() && (spec_mask.is_none() || spec_mask == sh.obj("Diffuse") || mask_const.is_some());
                    if single {
                        if mask_const.is_some() {
                            // UConstantMaterial::GetColor: alpha replicated in TEXTUREFACTOR.
                            let c = self.get(library, &mask_const).map(|o| o.color("Color")).unwrap_or(0);
                            let a = c >> 24;
                            ctx.pass(pass).tfactor = a << 24 | a << 16 | a << 8 | a;
                        }
                        self.set_texture(library, ctx, pass, stage, &spec_bm);
                        let st = ctx.stage(pass, stage);
                        st.alpha_arg1 = TA_CURRENT;
                        st.alpha_op = TOP_SELECTARG1;
                        if spec_mask.is_none() {
                            st.color_op = TOP_ADD;
                            st.color_arg1 = TA_CURRENT;
                            st.color_arg2 = TA_TEXTURE;
                        } else if mask_const.is_some() {
                            st.color_arg1 = TA_TFACTOR;
                            st.color_arg0 = TA_CURRENT;
                            st.color_op = TOP_MULTIPLYADD;
                            st.color_arg2 = TA_TEXTURE;
                        } else {
                            st.color_op = TOP_MODULATEALPHA_ADDCOLOR;
                            st.color_arg1 = TA_CURRENT;
                            st.color_arg2 = TA_TEXTURE;
                        }
                        Self::apply_tex_modifier(ctx, pass, stage, &mi_s);
                        stage += 1;
                    } else if stage + 1 < max && spec_bm.is_some() && mask_bm.is_some() {
                        self.two_stage_specular(library, ctx, pass, &mut stage, &mask_bm, &spec_bm, &mi_m, &mi_s);
                    } else {
                        multipass = true;
                    }
                }
                if multipass {
                    ctx.pass(pass).stages_used = stage;
                    pass += 1;
                    stage = 0;
                    ctx.reset_pass(pass);
                    if !self.handle_combined(library, ctx, &specular, &mut pass, &mut stage, mi.clone(), false, inv_op != 0) {
                        return false;
                    }
                    if spec_mask.is_some() {
                        if mask_const.is_some() {
                            ctx.err("not supported: constant SpecularityMask with multipass");
                            return false;
                        }
                        let Some(mask_bm) = mask_bm else {
                            ctx.err("SpecularityMask must be a simple bitmap material");
                            return false;
                        };
                        if stage >= max {
                            ctx.err(format!("No room for SpecularityMask in multipass specular (StagesUsed was {stage})"));
                            return false;
                        }
                        self.set_texture(library, ctx, pass, stage, &Some(mask_bm));
                        let st = ctx.stage(pass, stage);
                        st.alpha_arg2 = TA_CURRENT;
                        st.color_arg1 = TA_CURRENT;
                        st.alpha_arg1 = TA_TEXTURE;
                        st.alpha_op = TOP_SELECTARG1;
                        st.color_op = TOP_SELECTARG1;
                        Self::apply_tex_modifier(ctx, pass, stage, &mi_m);
                        let p = ctx.pass(pass);
                        p.src_blend = BLEND_SRCALPHA;
                        p.dest_blend = BLEND_ONE;
                        stage += 1;
                    } else {
                        let p = ctx.pass(pass);
                        p.src_blend = BLEND_ONE;
                        p.dest_blend = BLEND_ONE;
                    }
                    additive_pass(ctx.pass(pass));
                }
            }
        }
        // Output to the frame buffer (pass 0).
        if mi.final_blend {
            Self::apply_final_blend(ctx, &mi);
        } else {
            let opacity_set = opacity.is_some();
            let p = ctx.pass(0);
            p.z_test = true;
            let mut out = sh.byte("OutputBlending");
            if out == 0 && opacity_set {
                out = 5;
            }
            match out {
                0 => {
                    p.src_blend = BLEND_ONE;
                    p.dest_blend = BLEND_ZERO;
                    p.alpha_blending = false;
                    p.alpha_test = false;
                    p.z_write = true;
                }
                1 => {
                    p.src_blend = BLEND_ONE;
                    p.dest_blend = BLEND_ZERO;
                    p.alpha_blending = false;
                    p.alpha_test = true;
                    p.alpha_ref = 0x7F;
                    p.z_write = true;
                }
                2 | 3 => {
                    if out == 2 {
                        p.src_blend = BLEND_DESTCOLOR;
                        p.dest_blend = BLEND_SRCCOLOR;
                        p.fog_color = 0x007F_7F7F;
                    } else {
                        p.src_blend = BLEND_ONE;
                        p.dest_blend = BLEND_ONE;
                        p.fog_color = 0;
                    }
                    p.alpha_blending = true;
                    p.z_write = false;
                    p.override_fog_color = true;
                }
                4 => {
                    p.src_blend = BLEND_ZERO;
                    p.dest_blend = BLEND_ONE;
                    p.alpha_blending = true;
                    p.z_write = false;
                }
                5 => {
                    p.src_blend = BLEND_SRCALPHA;
                    p.dest_blend = BLEND_INVSRCALPHA;
                    p.alpha_test = true;
                    p.alpha_blending = true;
                    p.z_write = false;
                }
                6 | 7 => {
                    p.src_blend = if out == 6 { BLEND_ZERO } else { BLEND_ONE };
                    p.dest_blend = BLEND_INVSRCCOLOR;
                    p.alpha_blending = true;
                    p.override_fog_color = true;
                    p.fog_color = 0;
                    p.z_write = false;
                }
                8 => {
                    p.src_blend = BLEND_ONE;
                    p.dest_blend = BLEND_ONE;
                    p.alpha_blending = true;
                    p.z_write = false;
                }
                _ => {}
            }
            p.two_sided = flags & 1 != 0;
        }
        ctx.pass(0).fill_mode = if flags & 2 != 0 { 0 } else { 2 };
        if mi.color_modifier {
            let p = ctx.pass(0);
            if mi.alpha_blend && p.src_blend == BLEND_ONE && p.dest_blend == BLEND_ZERO {
                p.src_blend = BLEND_SRCALPHA;
                p.dest_blend = BLEND_INVSRCALPHA;
            }
            color_modifier_stage(ctx.stage(0, stage));
            let p = ctx.pass(0);
            p.tfactor = mi.color;
            p.alpha_blending |= mi.alpha_blend;
            stage += 1;
            p.two_sided |= mi.two_sided;
        }
        ctx.pass(pass).stages_used = stage;
        ctx.passes.truncate(pass + 1);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn two_stage_specular(
        &mut self,
        library: &mut Library,
        ctx: &mut Ctx,
        pass: usize,
        stage: &mut usize,
        mask_bm: &Option<String>,
        spec_bm: &Option<String>,
        mi_m: &ModifierInfo,
        mi_s: &ModifierInfo,
    ) {
        self.set_texture(library, ctx, 0, *stage, mask_bm);
        let st = ctx.stage(0, *stage);
        st.alpha_arg2 = TA_CURRENT;
        st.color_arg1 = TA_CURRENT;
        st.alpha_arg1 = TA_TEXTURE;
        st.alpha_op = TOP_SELECTARG1;
        st.color_op = TOP_SELECTARG1;
        Self::apply_tex_modifier(ctx, pass, *stage, mi_m);
        *stage += 1;
        self.set_texture(library, ctx, pass, *stage, spec_bm);
        let st = ctx.stage(pass, *stage);
        st.alpha_arg1 = TA_CURRENT;
        st.color_arg1 = TA_CURRENT;
        st.alpha_op = TOP_SELECTARG1;
        st.color_arg2 = TA_TEXTURE;
        st.color_op = TOP_MODULATEALPHA_ADDCOLOR;
        Self::apply_tex_modifier(ctx, pass, *stage, mi_s);
        *stage += 1;
    }

    /// FD3DRenderInterface::SetMaterial: passes to draw for `material`.
    /// `None` = default material (UMaterial.FallbackMaterial by default, Engine.DefaultTexture).
    /// The result of set_material for this material depends on neither frame nor view: no
    /// texture modifier (TexModifier: panning, rotation, oscillation, environment,
    /// matrix), no animated texture (AnimNext), sequence, or script or render texture,
    /// anywhere in the graph of objects it references; synthetic materials ("#") excluded.
    fn is_static(&mut self, library: &mut Library, path: &str, depth: u32) -> bool {
        if path.starts_with('#') || depth > 16 {
            return false;
        }
        lower_into(&mut self.key, path);
        if let Some(&v) = self.static_cache.get(self.key.as_str()) {
            return v;
        }
        let key = self.key.clone();
        // Provisional during the walk (cycle: dynamic to be safe).
        self.static_cache.insert(key.clone(), false);
        let v = match self.get(library, path).cloned() {
            None => true,
            Some(o) => {
                let dynamic = ["TexModifier", "MaterialSequence", "MaterialSwitch", "ScriptedTexture", "RenderTargetMaterial", "SinusModifier", "Projector"]
                    .iter()
                    .any(|c| o.is_a(c))
                    || (o.is_a("Texture") && o.obj("AnimNext").is_some());
                !dynamic
                    && o.values.values().all(|v| match v {
                        crate::props::PVal::Obj(Some(r)) => self.is_static(library, r, depth + 1),
                        _ => true,
                    })
            }
        };
        self.static_cache.insert(key, v);
        v
    }

    /// What the translation of `path` depends on (lighting aside): 0 nothing (same passes every
    /// frame), 1 the view only, 2 time or a state that advances per frame. Same variable
    /// classes as `is_static`, except the UTexModifiers whose GetMatrix reads neither time nor an
    /// advancing state (UTexModifier, UTexMatrix, UTexScaler, UTexEnvMap: exact class), which
    /// only depend on the view (ApplyTexModifier, coordinates 8 and 10). Among the synthetic
    /// materials, only the terrain and UCanvas ones are classified; the others (projectors,
    /// render targets, sprites, trails) stay variable. For a path without "#" whose
    /// graph has no cycle, 0 if and only if `is_static`.
    fn variability(&mut self, library: &mut Library, path: &str, depth: u32) -> u8 {
        if depth > 16 {
            return 2;
        }
        if path.starts_with('#') && !["#terrain", "#canvas:", "#canvasline:"].iter().any(|p| path.starts_with(p)) && path != "#white" {
            return 2;
        }
        lower_into(&mut self.key, path);
        if let Some(&v) = self.variability_cache.get(self.key.as_str()) {
            return v;
        }
        let key = self.key.clone();
        // Provisional during the walk (cycle: variable to be safe, like `is_static`).
        self.variability_cache.insert(key.clone(), 2);
        let v = match self.get(library, path).cloned() {
            None => 0,
            Some(o) => {
                let time_value = ["MaterialSequence", "MaterialSwitch", "ScriptedTexture", "RenderTargetMaterial", "SinusModifier", "Projector"]
                    .iter()
                    .any(|c| o.is_a(c))
                    || (o.is_a("Texture") && o.obj("AnimNext").is_some());
                let mut v = if time_value {
                    2
                } else if o.is_a("TexModifier") {
                    let class_value = o.props.chain.first().map(String::as_str).unwrap_or("");
                    if ["TexModifier", "TexMatrix", "TexScaler", "TexEnvMap"].iter().any(|c| class_value.eq_ignore_ascii_case(c)) { 1 } else { 2 }
                } else {
                    0
                };
                for r in o.values.values() {
                    if v == 2 {
                        break;
                    }
                    if let crate::props::PVal::Obj(Some(r)) = r {
                        v = v.max(self.variability(library, r, depth + 1));
                    }
                }
                v
            }
        };
        self.variability_cache.insert(key, v);
        v
    }

    /// SetMaterial, with the result kept for static materials (`is_static`): same
    /// passes every frame without redoing the translation.
    /// Shared passes (Rc): a reuse copies neither the passes nor their texture names.
    pub fn set_material(&mut self, library: &mut Library, material: Option<&str>, lighting: Lighting, frame: FrameTime, view: [f32; 16]) -> std::rc::Rc<MaterialPasses> {
        let cacheable = material.is_some_and(|m| self.is_static(library, m, 0));
        self.set_classified_material(library, material, lighting, frame, view, cacheable)
    }

    /// Translation already bound to this section, if it is still the one for the requested
    /// material and lighting. A material sensitive to frame or view must always
    /// go through its normal set_batch_material call.
    pub fn static_batch_passes<'a>(&self, bound: &'a BatchMaterial, replace: bool, lighting: Lighting) -> Option<&'a std::rc::Rc<MaterialPasses>> {
        if bound.binding != Some((self.replacement_generation, replace)) { return None; }
        bound.translated.as_ref().filter(|(l, _)| *l == lighting).map(|(_, passes)| passes)
    }

    /// Same translation as set_material for the section that owns `bound`. The section's
    /// rank is enough to find its replaced name and its static translation. Variable
    /// materials are still evaluated in the normal order, per frame and per view.
    #[allow(clippy::too_many_arguments)]
    /// Binding of the section (as at the start of set_batch_material): true if its material is
    /// fixed (translation kept, no advancing state).
    pub fn bind_batch(&mut self, bound: &mut BatchMaterial, library: &mut Library, original: Option<&str>, replace: bool) -> bool {
        let binding = (self.replacement_generation, replace);
        if bound.binding != Some(binding) {
            bound.binding = Some(binding);
            bound.translated = None;
            bound.replacement = original.filter(|_| replace).and_then(|m| self.replacement(m)).map(str::to_owned);
            let material = bound.replacement.as_deref().or(original).filter(|m| !replace || !m.is_empty());
            bound.is_static = material.is_some_and(|m| self.is_static(library, m, 0));
        }
        bound.is_static
    }

    pub fn set_batch_material<'a>(
        &mut self, bound: &'a mut BatchMaterial, library: &mut Library, original: Option<&'a str>,
        replace: bool, lighting: Lighting, frame: FrameTime, view: [f32; 16],
    ) -> (std::rc::Rc<MaterialPasses>, Option<&'a str>) {
        let binding = (self.replacement_generation, replace);
        let changed = bound.binding != Some(binding);
        if changed {
            bound.binding = Some(binding);
            bound.translated = None;
            bound.replacement = original.filter(|_| replace).and_then(|m| self.replacement(m)).map(str::to_owned);
        }
        let material = bound.replacement.as_deref().or(original).filter(|m| !replace || !m.is_empty());
        if changed {
            bound.is_static = material.is_some_and(|m| self.is_static(library, m, 0));
        }
        if let Some((_, passes)) = bound.translated.as_ref().filter(|(l, _)| *l == lighting) {
            return (passes.clone(), material);
        }
        let passes = self.set_classified_material(library, material, lighting, frame, view, bound.is_static);
        if bound.is_static {
            bound.translated = Some((lighting, passes.clone()));
        }
        (passes, material)
    }

    fn set_classified_material(&mut self, library: &mut Library, material: Option<&str>, lighting: Lighting, frame: FrameTime, view: [f32; 16], cacheable: bool) -> std::rc::Rc<MaterialPasses> {
        let lkey = lighting.lighting as u8 | (lighting.color_vertex as u8) << 1 | (lighting.lightmap as u8) << 2 | (lighting.lighting_only as u8) << 3;
        // Materials with nothing that depends on time: variability 0 (UCanvas on a fixed texture)
        // kept like a static material; 1 (terrain, fixed modifiers in camera
        // coordinates) kept per view like a fixed projector. Projectors keep their
        // classification (`projector_is_fixed`) and the others are translated once per frame.
        let synth = match material {
            Some(m) if !cacheable && !m.starts_with("#proj") => self.variability(library, m, 0),
            _ => 2,
        };
        if cacheable || synth == 0 {
            lower_into(&mut self.key, material.unwrap_or_default());
            if let Some(r) = self.pass_cache.get(self.key.as_str()).and_then(|e| e[lkey as usize].as_ref()) {
                let r = r.clone();
                return r;
            }
            let key = self.key.clone();
            let r = std::rc::Rc::new(self.set_material_uncached(library, material, lighting, frame, view));
            self.pass_cache.entry(key).or_insert_with(|| std::array::from_fn(|_| None))[lkey as usize] = Some(r.clone());
            return r;
        }
        let Some(m) = material else { return std::rc::Rc::new(self.set_material_uncached(library, material, lighting, frame, view)) };
        if synth == 1 || self.projector_is_fixed(library, m) {
            if self.proj_image != frame.count {
                self.proj_image = frame.count;
                self.proj_previous = std::mem::take(&mut self.proj_current);
                self.previous_projector_plans = std::mem::take(&mut self.projector_plans);
            }
            lower_into(&mut self.key, m);
            let key_tuple = (self.key.clone(), view.map(f32::to_bits));
            if !self.proj_current.contains_key(&key_tuple) {
                if let Some(e) = self.proj_previous.remove(&key_tuple) {
                    self.proj_current.insert(key_tuple.clone(), e);
                }
            }
            if let Some(r) = self.proj_current.get(&key_tuple).and_then(|e| e[lkey as usize].as_ref()) {
                let r = r.clone();
                return r;
            }
            let r = if m.starts_with("#proj:") && lkey == 0 {
                self.fixed_projector_view(library, m, &key_tuple.0, lighting, frame, view)
            } else if m.starts_with("#terrain:") {
                self.terrain_view(library, m, &key_tuple.0, lkey, lighting, frame, view)
            } else {
                std::rc::Rc::new(self.set_material_uncached(library, material, lighting, frame, view))
            };
            self.proj_current.entry(key_tuple).or_insert_with(|| std::array::from_fn(|_| None))[lkey as usize] = Some(r.clone());
            return r;
        }
        let key_tuple = (frame.count, frame.delta.to_bits(), view.map(f32::to_bits));
        if self.image_key != Some(key_tuple) {
            self.image_key = Some(key_tuple);
            self.image_cache.clear();
        }
        lower_into(&mut self.key, m);
        if let Some(r) = self.image_cache.get(self.key.as_str()).and_then(|e| e[lkey as usize].as_ref()) {
            let r = r.clone();
            return r;
        }
        let key = self.key.clone();
        let r = std::rc::Rc::new(self.set_material_uncached(library, material, lighting, frame, view));
        self.image_cache.entry(key).or_insert_with(|| std::array::from_fn(|_| None))[lkey as usize] = Some(r.clone());
        r
    }

    /// States modified by a translation (check of `image_cache`), as sorted text.
    #[cfg(test)]
    fn state_variable(&self) -> String {
        let mut a: Vec<String> = self.anims.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
        a.extend(self.sine.iter().map(|(k, v)| format!("{k}={v:?}")));
        a.extend(self.objects.iter().filter_map(|(k, o)| o.as_ref().map(|o| format!("{k}={:?}", o.state))));
        a.push(format!("proj={}", self.proj_case));
        a.sort();
        a.join("
")
    }

    /// Terrain layer through its recipe (see `terrain_plans`), recorded on its first
    /// translation; without a recipe (a stage depends on the view in another way), full translation.
    #[allow(clippy::too_many_arguments)]
    fn terrain_view(&mut self, library: &mut Library, path: &str, key: &str, lkey: u8, lighting: Lighting, frame: FrameTime, view: [f32; 16]) -> std::rc::Rc<MaterialPasses> {
        if let Some(plan) = self.terrain_plans.get(&(key.to_owned(), lkey)) {
            if let Some(plan) = plan {
                let r = plan.for_view(&view);
                return r;
            }
            return std::rc::Rc::new(self.set_material_uncached(library, Some(path), lighting, frame, view));
        }
        let mut matrices = Some(Vec::new());
        self.recipe_terrain = true;
        let passes = std::rc::Rc::new(self.set_material_recording(library, Some(path), lighting, frame, view, &mut matrices));
        self.recipe_terrain = false;
        let plan = matrices.filter(|_| passes.error.is_none()).map(|mut matrices| {
            matrices.retain(|m| m.pass < passes.passes.len());
            ProjectorPlan { passes: passes.clone(), matrices }
        });
        self.terrain_plans.insert((key.to_owned(), lkey), plan);
        passes
    }

    fn fixed_projector_view(&mut self, library: &mut Library, path: &str, key: &str, lighting: Lighting, frame: FrameTime, view: [f32; 16]) -> std::rc::Rc<MaterialPasses> {
        if !self.projector_plans.contains_key(key) {
            if let Some(plan) = self.previous_projector_plans.remove(key) {
                self.projector_plans.insert(key.to_owned(), plan);
            }
        }
        if let Some(plan) = self.projector_plans.get(key) {
            if let Some(plan) = plan {
                return plan.for_view(&view);
            }
            return std::rc::Rc::new(self.set_material_uncached(library, Some(path), lighting, frame, view));
        }
        // The only replaceable references of the synthetic graph are those of the
        // FinalBlend and the two TexMatrix (replace_fields). Replacing a synthetic link
        // keeps the normal translation. The leaves actually chosen after
        // ReplaceATexture must be fixed bitmaps (or the immutable shadow target).
        let can_record = !self.replacements.iter().any(|(source, _)| source.starts_with('#'))
            && path.rsplit_once('|').is_some_and(|(_, inner)| [inner, "Engine.ProjGradient", "Engine.ProjClip"].into_iter().all(|texture| {
                let leaf = self.replaced(texture);
                leaf.starts_with("#shadowtex:")
                    || (self.is_a(library, leaf.as_str(), "BitmapMaterial") && self.is_static(library, &leaf, 0))
            }));
        // Template of the same shape already recorded: plan of this key through its two matrices.
        let shape = can_record.then(|| split_key_projector(key)).flatten();
        if let Some((f, m, g)) = &shape {
            if let Some(Some(gab)) = self.templates.get(f) {
                let matrices = gab.stages_list.iter().map(|&(pass, stage, role)| ViewMatrix { pass, stage, world: if role == 0 { *m } else { *g } }).collect();
                let plan = ProjectorPlan { passes: gab.passes.clone(), matrices };
                let r = plan.for_view(&view);
                self.projector_plans.insert(key.to_owned(), Some(plan));
                return r;
            }
        }
        let mut matrices = can_record.then(Vec::new);
        let passes = std::rc::Rc::new(self.set_material_recording(library, Some(path), lighting, frame, view, &mut matrices));
        let plan = matrices.filter(|_| passes.error.is_none()).map(|mut matrices| {
            matrices.retain(|m| m.pass < passes.passes.len());
            ProjectorPlan { passes: passes.clone(), matrices }
        });
        // First plan of this shape: a template if each recomputed stage has as its world matrix
        // exactly (bit for bit) one of the key's two matrices.
        if let Some((f, m, g)) = shape {
            if !self.templates.contains_key(&f) {
                let bits = |x: &[f32; 16]| x.map(f32::to_bits);
                let gab = plan.as_ref().and_then(|p| {
                    let stages_list: Option<Vec<(usize, usize, u8)>> = p.matrices.iter().map(|vm| {
                        let w = bits(&vm.world);
                        if w == bits(&m) { Some((vm.pass, vm.stage, 0)) } else if w == bits(&g) { Some((vm.pass, vm.stage, 1)) } else { None }
                    }).collect();
                    stages_list.map(|stages_list| TemplateProjector { passes: p.passes.clone(), stages_list })
                });
                self.templates.insert(f, gab);
            }
        }
        self.projector_plans.insert(key.to_owned(), plan);
        passes
    }

    fn set_material_uncached(&mut self, library: &mut Library, material: Option<&str>, lighting: Lighting, frame: FrameTime, view: [f32; 16]) -> MaterialPasses {
        self.set_material_recording(library, material, lighting, frame, view, &mut None)
    }

    fn set_material_recording(&mut self, library: &mut Library, material: Option<&str>, lighting: Lighting, frame: FrameTime, view: [f32; 16], matrices: &mut Option<Vec<ViewMatrix>>) -> MaterialPasses {
        let start = false.then(std::time::Instant::now);
        let passes = self.translate_recording(library, material, lighting, frame, view, matrices);
        if let Some(start) = start {
            let ns = start.elapsed().as_nanos() as u64;
            tally::by_name(material.unwrap_or(""), ns);
        }
        passes
    }

    fn translate_recording(&mut self, library: &mut Library, material: Option<&str>, lighting: Lighting, frame: FrameTime, view: [f32; 16], matrices: &mut Option<Vec<ViewMatrix>>) -> MaterialPasses {
        let mut mat = material.map(str::to_string);
        if mat.is_none() {
            if let Some(defaults) = self.props.class(library, "Engine.Material") {
                mat = defaults.obj("FallbackMaterial");
            }
        }
        let mut ctx = Ctx {
            lighting,
            frame,
            view,
            projector_matrices: matrices.take(),
            sources_fixed: self.recipe_terrain,
            passes: Vec::new(),
            error: None,
        };
        let ok = if lighting.lighting_only {
            set_lighting_only(&mut ctx);
            true
        } else {
            let mut mi = ModifierInfo::default();
            if let Some(s) = self.check(library, &mat, "Shader", Some(&mut mi), frame) {
                self.set_shader(library, &mut ctx, &s, mi)
            } else if let Some(c) = self.check(library, &mat, "Combiner", Some(&mut mi), frame) {
                self.set_simple(library, &mut ctx, &Some(c), mi)
            } else if let Some(b) = self.check(library, &mat, "BitmapMaterial", Some(&mut mi), frame) {
                self.set_simple(library, &mut ctx, &Some(b), mi)
            } else if let Some(p) = self.check(library, &mat, "ParticleMaterial", None, frame) {
                self.set_particle(library, &mut ctx, &p)
            } else {
                // Material without a translation: D3DDrv keeps the previous passes.
                ctx.err("material not translated by SetMaterial");
                false
            }
        };
        if !ok {
            // D3DDrv ignores the failure and draws with half-written states: we draw nothing.
            return MaterialPasses { passes: Vec::new(), requires_sorting: false, error: ctx.error };
        }
        *matrices = ctx.projector_matrices;
        let requires_sorting = self.requires_sorting(library, &mat);
        MaterialPasses { passes: ctx.passes, requires_sorting, error: ctx.error }
    }
}

/// Simplified material of the cartoon rendering.
#[derive(Debug, Clone, Default)]
pub struct CartoonMaterial {
    pub texture: Option<String>,
    /// Blend (SRCBLEND, DESTBLEND); none: opaque.
    pub blend: Option<(u32, u32)>,
    /// Alpha test at 127.
    pub alpha_test: bool,
    /// OutputBlending 4 or 8: previous states kept (drawn here as opaque).
    pub keeps_previous: bool,
    /// The section gets the black outline.
    pub outline: bool,
}

/// Texture properties used by the renderer.
#[derive(Debug, Clone, Copy)]
pub struct TextureInfo {
    pub masked: bool,
    pub alpha_texture: bool,
    pub usize: i32,
    pub vsize: i32,
    pub cube: bool,
}

/// Lightmap stage (modulated onto the current colour, coordinates 1).
fn lightmap_stage(st: &mut Stage) {
    st.color_arg1 = TA_TEXTURE;
    st.alpha_op = TOP_SELECTARG1;
    st.texture = Some(TexRef::Lightmap);
    st.alpha_arg1 = TA_CURRENT;
    st.color_arg2 = TA_CURRENT;
    st.color_op = TOP_MODULATE;
    st.address[0] = TADDRESS_CLAMP;
    st.address[1] = TADDRESS_CLAMP;
    st.tex_coord_index = 1;
    st.use_matrix = false;
}

/// Stage modulating by the diffuse colour (per-vertex lighting).
fn diffuse_stage(st: &mut Stage, modulate2x: bool) {
    st.color_arg1 = TA_DIFFUSE;
    st.color_arg2 = TA_CURRENT;
    st.alpha_arg2 = TA_CURRENT;
    st.alpha_op = TOP_SELECTARG2;
    st.color_op = if modulate2x { TOP_MODULATE2X } else { TOP_MODULATE };
}

/// ColorModifier stage: current colour × TEXTUREFACTOR.
fn color_modifier_stage(st: &mut Stage) {
    st.alpha_arg2 = TA_TFACTOR;
    st.color_arg2 = TA_TFACTOR;
    st.alpha_arg1 = TA_CURRENT;
    st.color_arg1 = TA_CURRENT;
    st.alpha_op = TOP_MODULATE;
    st.color_op = TOP_MODULATE;
    st.texture = None;
}

/// End of additive passes: black fog.
fn additive_pass(p: &mut Pass) {
    p.alpha_blending = true;
    p.override_fog_color = true;
    p.fog_color = 0;
}

/// FD3DRenderInterface::SetLightingOnlyMaterial.
fn set_lighting_only(ctx: &mut Ctx) {
    ctx.reset_pass(0);
    let lightmap = ctx.lighting.lightmap;
    let st = ctx.stage(0, 0);
    st.color_arg1 = TA_DIFFUSE;
    st.color_op = TOP_SELECTARG1;
    st.alpha_arg1 = TA_DIFFUSE;
    st.alpha_op = TOP_SELECTARG1;
    let mut used = 1;
    if lightmap {
        lightmap_stage(ctx.stage(0, 1));
        used = 2;
    }
    ctx.pass(0).stages_used = used;
    ctx.passes.truncate(1);
}

/// Unreal's FRotationMatrix through the sine table, row-major.
pub fn rotation_matrix(r: [i32; 3]) -> [f32; 16] {
    let (sr, cr) = (sin_u(-r[2]), sin_u(0x4000 - r[2]));
    let (sy, cy) = (sin_u(r[1]), sin_u(r[1] + 0x4000));
    let (sp, cp) = (sin_u(-r[0]), sin_u(0x4000 - r[0]));
    let mut m = [0.0; 16];
    m[0] = cp * cy;
    m[1] = cp * sy;
    m[2] = -sp;
    m[4] = sp * sr * cy - sy * cr;
    m[5] = cy * cr + sp * sr * sy;
    m[6] = cp * sr;
    m[8] = sy * sr + sp * cr * cy;
    m[9] = sp * cr * sy - cy * sr;
    m[10] = cp * cr;
    m[15] = 1.0;
    m
}

fn transpose(m: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|i| m[(i % 4) * 4 + i / 4])
}

/// Inverse of a 4×4 matrix (D3DXMatrixInverse).
pub fn invert(m: &[f32; 16]) -> [f32; 16] {
    let a = m.map(|v| v as f64);
    let mut inv = [0.0f64; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15] + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15] - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15] + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14] - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15] - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15] + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15] - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14] + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15] + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15] - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15] + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14] - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11] - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11] + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11] - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10] + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    if det == 0.0 {
        return *m;
    }
    inv.map(|v| (v / det) as f32)
}

/// UTexture::Update: once per frame (LastFrameCount ≠ frame byte), LastFrameCount takes the byte,
/// bit 4 of the flags (+0x6C) becomes 1 if bit 2 is set, then Tick receives the delta. Returns (LastFrameCount,
/// flags, Tick delta).
/// Intensity carried by a shadow target path "#shadowtex:<slot>:<intensity>" (the
/// ShadowBitmapMaterial's +0x78, rewritten by AShadowProjector::UpdateShadow every frame).
pub fn shadow_target_intensity(path: &str) -> Option<u32> {
    path.strip_prefix("#shadowtex:")?.split_once(':')?.1.parse().ok()
}

pub fn texture_update(last_frame: u8, flags: u32, frame_count: u8, delta: f32) -> (u8, u32, Option<f32>) {
    if last_frame == frame_count {
        return (last_frame, flags, None);
    }
    (frame_count, if flags & 4 != 0 { flags | 0x10 } else { flags }, Some(delta))
}

/// Fields that UTerrainRenderMaterial::SetTerrain sets on the three shared materials of a terrain layer:
/// UTexMatrix in world coordinates (TexCoordSource 8), UTexModifier of the alpha map (source 0),
/// UCombiner CO_Multiply (2). Modulate2X is the "RendMap == 6" flag passed by ATerrainInfo::Render; the
/// map is built at load time, out of reach of that console mode (RMODE 6): false.
pub struct TerrainLayerFields {
    pub matrix_source: u8,
    pub alpha_source: u8,
    pub combine_operation: u8,
    pub modulate2x: bool,
}

pub const TERRAIN_LAYER: TerrainLayerFields = TerrainLayerFields { matrix_source: 8, alpha_source: 0, combine_operation: 2, modulate2x: false };

/// Matrix of a "#terrain:<rank>:<16 hex words>|texture|alpha map" key (the 16 words SetTerrain copies to +0x58).
pub fn terrain_layer_matrix(key: &str) -> Option<[u32; 16]> {
    let rest = key.strip_prefix("#terrain:")?;
    let m = rest.split('|').next()?.rsplit(':').next()?;
    let v: Vec<u32> = m.split(',').filter_map(|x| u32::from_str_radix(x, 16).ok()).collect();
    v.try_into().ok()
}

/// The 16 floats of `m` as 8 lowercase hex digits (IEEE bits), separated by
/// commas: the matrix part of the `#proj` key of `Materials::projector_key`.
fn hex_matrix(out: &mut String, m: &[f32; 16]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for (i, f) in m.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let b = f.to_bits();
        for k in (0..8).rev() {
            out.push(DIGITS[((b >> (k * 4)) & 0xF) as usize] as char);
        }
    }
}

#[cfg(test)]
mod hex_matrix_tests {
    /// Same text as the old formatting (format! "{:08x}" of each float, joined by commas).
    #[test]
    fn hex_matrix_as_format() {
        let mut seed_value = 0x1234_5678u32;
        for _ in 0..200 {
            let mut m = [0f32; 16];
            for v in &mut m {
                seed_value = seed_value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *v = f32::from_bits(seed_value);
            }
            m[0] = 0.0;
            m[1] = -0.0;
            m[2] = 1.0;
            let want = m.iter().map(|f| format!("{:08x}", f.to_bits())).collect::<Vec<_>>().join(",");
            let mut s = String::new();
            super::hex_matrix(&mut s, &m);
            assert_eq!(s, want);
        }
    }
}

/// Profiling counters: SetMaterial calls (including cache hits and
/// full translations, time in ns), Materials::check, Materials::get and Props::get.
pub mod tally {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    pub static SET_MAT: AtomicU64 = AtomicU64::new(0);
    pub static HITS: AtomicU64 = AtomicU64::new(0);
    pub static UNCACHED: AtomicU64 = AtomicU64::new(0);
    pub static NS_SET: AtomicU64 = AtomicU64::new(0);
    pub static NS_UNCACHED: AtomicU64 = AtomicU64::new(0);
    pub static CHECK: AtomicU64 = AtomicU64::new(0);
    pub static MAT_GET: AtomicU64 = AtomicU64::new(0);
    pub static PROPS_GET: AtomicU64 = AtomicU64::new(0);
    /// set, hits, translations, ns set, ns translations, check, get, Props::get.
    pub fn read_from() -> [u64; 8] {
        [&SET_MAT, &HITS, &UNCACHED, &NS_SET, &NS_UNCACHED, &CHECK, &MAT_GET, &PROPS_GET].map(|c| c.load(Relaxed))
    }
    /// Full translations per material (count, ns), cleared when taken by name.
    static BY_NAME: std::sync::Mutex<Vec<(String, u64, u64)>> = std::sync::Mutex::new(Vec::new());
    pub fn by_name(name_value: &str, ns: u64) {
        let mut v = BY_NAME.lock().unwrap();
        match v.iter_mut().find(|e| e.0 == name_value) {
            Some(e) => {
                e.1 += 1;
                e.2 += ns;
            }
            None => v.push((name_value.to_owned(), 1, ns)),
        }
    }
    pub fn take_by_name() -> Vec<(String, u64, u64)> {
        std::mem::take(&mut *BY_NAME.lock().unwrap())
    }
}

/// Kept passes (fixed, per view, per frame) against a full translation on every call,
/// on materials sharing textures and modifiers: repeated or changed views (including
/// -0.0), lighting states, call orders, frame durations and replacements that make
/// a "view only" graph animated.
#[cfg(test)]
mod cache_by_view {
    use super::*;

    fn object_value(m: &mut Materials, name_value: &str, class_value: &str, value_list: Vec<(&str, PVal)>) {
        let parents: &[&str] = match class_value {
            "Texture" => &["Texture", "BitmapMaterial", "RenderedMaterial", "Material", "Object"],
            "Shader" => &["Shader", "RenderedMaterial", "Material", "Object"],
            "Combiner" => &["Combiner", "Material", "Object"],
            "FinalBlend" => &["FinalBlend", "Modifier", "Material", "Object"],
            _ => &[class_value, "TexModifier", "Modifier", "Material", "Object"],
        };
        let parents: Vec<&str> = if class_value == "TexModifier" { parents[1..].to_vec() } else { parents.to_vec() };
        let props = Props { chain: parents.iter().map(|c| c.to_string()).collect(), values: value_list.into_iter().map(|(k, v)| (k.to_ascii_lowercase(), v)).collect() };
        let state = ModState { tex_coord_source: props.byte("TexCoordSource"), last_frame_count: props.byte("LastFrameCount"), ..Default::default() };
        m.objects.insert(name_value.to_ascii_lowercase(), Some(Obj { props: std::rc::Rc::new(props), state }));
    }

    fn o(s: &str) -> PVal {
        PVal::Obj(Some(s.to_string()))
    }

    fn materials_list() -> Materials {
        let mut m = Materials::new();
        let tex = |w| vec![("USize", PVal::Int(w)), ("VSize", PVal::Int(w)), ("UClampMode", PVal::Byte(1))];
        object_value(&mut m, "T.A", "Texture", tex(64));
        object_value(&mut m, "T.B", "Texture", tex(32));
        let mut a1 = tex(16);
        a1.push(("AnimNext", o("T.Anim2")));
        object_value(&mut m, "T.Anim1", "Texture", a1);
        let mut a2 = tex(16);
        a2.extend([("AnimNext", o("T.Anim1")), ("MaxFrameRate", PVal::Float(12.0))]);
        object_value(&mut m, "T.Anim2", "Texture", a2);
        let matrix_value: Vec<u8> = [0.5f32, 0.25, 0.0, 0.0, -0.125, 0.75, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.3, -0.7, 0.0, 1.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        object_value(&mut m, "M.Mat", "TexMatrix", vec![("Material", o("T.A")), ("TexCoordSource", PVal::Byte(8)), ("Matrix", PVal::Raw(matrix_value))]);
        object_value(&mut m, "M.Alpha", "TexModifier", vec![("Material", o("T.B")), ("TexCoordSource", PVal::Byte(0))]);
        object_value(&mut m, "M.Comb", "Combiner", vec![("Material1", o("M.Mat")), ("Material2", o("M.Alpha")), ("Mask", o("M.Alpha")), ("CombineOperation", PVal::Byte(2))]);
        object_value(&mut m, "M.Sh", "Shader", vec![("Diffuse", o("M.Comb")), ("Opacity", o("M.Comb")), ("ModulateStaticLighting2X", PVal::Bool(true))]);
        object_value(&mut m, "M.Terrain", "FinalBlend", vec![("Material", o("M.Sh")), ("FrameBufferBlending", PVal::Byte(2)), ("ZWrite", PVal::Bool(true)), ("ZTest", PVal::Bool(true))]);
        object_value(&mut m, "M.Env", "TexEnvMap", vec![("Material", o("T.A")), ("EnvMapType", PVal::Byte(0)), ("TexCoordSource", PVal::Byte(0))]);
        object_value(&mut m, "M.Scaler", "TexScaler", vec![("Material", o("T.B")), ("UScale", PVal::Float(2.0)), ("VScale", PVal::Float(3.0)), ("UOffset", PVal::Float(5.0))]);
        object_value(&mut m, "M.Osc", "TexOscillator", vec![
            ("Material", o("T.A")), ("UOscillationRate", PVal::Float(0.3)), ("UOscillationAmplitude", PVal::Float(0.1)),
            ("VOscillationRate", PVal::Float(0.7)), ("VOscillationAmplitude", PVal::Float(0.2)), ("VOscillationType", PVal::Byte(1)), ("VCenter", PVal::Float(0.5)),
        ]);
        object_value(&mut m, "M.OscEnv", "TexOscillator", vec![("Material", o("M.Env")), ("UOscillationRate", PVal::Float(0.45)), ("UOscillationAmplitude", PVal::Float(0.05))]);
        object_value(&mut m, "M.Pan", "TexPanner", vec![("Material", o("M.Scaler")), ("PanRate", PVal::Float(0.25)), ("PanDirection", PVal::Rot([0, 0x2000, 0]))]);
        object_value(&mut m, "M.AnimFB", "FinalBlend", vec![("Material", o("T.Anim1")), ("FrameBufferBlending", PVal::Byte(3))]);
        object_value(&mut m, "M.Fixed", "FinalBlend", vec![("Material", o("T.B")), ("FrameBufferBlending", PVal::Byte(1))]);
        m
    }

    #[test]
    fn fixed_projector_view_reprojects_the_real_stages_and_falls_back_on_replacements() {
        let path = "#proj:2:2:5:X:Y|T.A";
        let combiner = "#projc:2:2:5:X:Y|T.A";
        let texture = "#projt:2:2:5:X:Y|T.A";
        let gradient = "#projg:2:2:5:X:Y|T.A";
        let key = path.to_ascii_lowercase();
        let lighting = Lighting { lighting: false, color_vertex: false, lightmap: false, lighting_only: false };
        let fixture = || {
            let mut m = materials_list();
            for name in ["Engine.ProjGradient", "Engine.ProjClip"] {
                object_value(&mut m, name, "Texture", vec![("USize", PVal::Int(64))]);
            }
            object_value(&mut m, path, "FinalBlend", vec![("Material", o(combiner)), ("FrameBufferBlending", PVal::Byte(2))]);
            object_value(
                &mut m,
                combiner,
                "Combiner",
                vec![
                    ("Material1", o(texture)),
                    ("Material2", o(gradient)),
                    ("CombineOperation", PVal::Byte(0)),
                    ("AlphaOperation", PVal::Byte(1)),
                ],
            );
            let mut matrix = IDENTITY;
            matrix[0] = 3.25;
            matrix[13] = -0.0;
            matrix[14] = -19.5;
            for (part, leaf, mx) in [(texture, "T.A", matrix), (gradient, "Engine.ProjGradient", IDENTITY)] {
                object_value(
                    &mut m,
                    part,
                    "TexMatrix",
                    vec![
                        ("Material", o(leaf)),
                        ("TexCoordSource", PVal::Byte(8)),
                        ("TexCoordCount", PVal::Byte(1)),
                        ("Matrix", PVal::Raw(mx.iter().flat_map(|f| f.to_le_bytes()).collect())),
                    ],
                );
            }
            m
        };
        let mut lib = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        for (source, replacement, expected_plan) in [("unrelated", "T.B", true), ("T.A", "T.Anim1", false), (combiner, "M.Pan", false)] {
            let (mut planned, mut reference) = (fixture(), fixture());
            let list = vec![(source.to_string(), replacement.to_string())];
            planned.set_replacements(&list);
            reference.set_replacements(&list);
            for i in 1..=5 {
                let mut view = IDENTITY;
                view[0] = if i == 5 { 0.0 } else { i as f32 * 0.25 };
                view[3] = -0.0;
                view[12] = i as f32 * 200.5;
                let frame = FrameTime { count: i, delta: 0.03125 };
                let a = planned.fixed_projector_view(&mut lib, path, &key, lighting, frame, view);
                let b = reference.set_material_uncached(&mut lib, Some(path), lighting, frame, view);
                assert_eq!(format!("{a:?}"), format!("{b:?}"), "{source} frame {i}");
                for (p, q) in a.passes.iter().zip(&b.passes) {
                    for (its, sb) in p.stages.iter().zip(&q.stages) {
                        assert_eq!(its.matrix.map(f32::to_bits), sb.matrix.map(f32::to_bits));
                    }
                }
                assert_eq!(planned.state_variable(), reference.state_variable());
                assert_eq!(planned.projector_plans[&key].is_some(), expected_plan);
            }
            if expected_plan {
                assert!(!planned.projector_plans[&key].as_ref().unwrap().matrices.is_empty());
                planned.set_replacements(&[]);
                assert!(planned.projector_plans.is_empty() && planned.previous_projector_plans.is_empty());
            }
        }
    }

    const NAMES: [&str; 9] = ["M.Terrain", "M.Osc", "M.Env", "M.OscEnv", "M.Pan", "M.AnimFB", "M.Fixed", "M.Scaler", "M.Mat"];

    #[test]
    fn ranking_graphs() {
        let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut m = materials_list();
        let v: Vec<u8> = NAMES.iter().map(|n| m.variability(&mut library, n, 0)).collect();
        assert_eq!(v, [1, 2, 1, 2, 2, 2, 0, 1, 1]);
        for n in NAMES {
            assert_eq!(m.variability(&mut library, n, 0) == 0, m.is_static(&mut library, n, 0), "{n}");
        }
        m.set_replacements(&[("t.a".into(), "T.Anim1".into())]);
        assert_eq!(m.variability(&mut library, "M.Terrain", 0), 2);
    }

    #[test]
    fn passes_kept_identical_to_complete_translation() {
        let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let (mut fresh_list_new, mut reference) = (materials_list(), materials_list());
        let mut v1 = IDENTITY;
        v1[0] = 0.8;
        v1[2] = -0.6;
        v1[8] = 0.6;
        v1[10] = 0.8;
        v1[12] = 120.0;
        v1[14] = -35.5;
        let mut v2 = v1;
        v2[1] = -0.0;
        let views_list = [IDENTITY, v1, v1, v2, v1, IDENTITY, IDENTITY, v2];
        let lightings = [
            Lighting::default(),
            Lighting { color_vertex: true, ..Default::default() },
            Lighting { lighting: true, lightmap: true, ..Default::default() },
        ];
        let mut calls_list = 0;
        for image in 1..=64u32 {
            if image == 40 {
                // Replacement: the terrain graph becomes animated (view only -> time).
                let r = [("t.a".to_string(), "T.Anim1".to_string())];
                fresh_list_new.set_replacements(&r);
                reference.set_replacements(&r);
            }
            let frame = FrameTime { count: image, delta: [0.033, 0.016, 0.05][image as usize % 3] };
            let view_value = views_list[(image as usize / 3) % views_list.len()];
            for k in 0..NAMES.len() * 2 {
                let name_value = NAMES[(k * 5 + image as usize) % NAMES.len()];
                let lighting = lightings[(k + image as usize / 7) % lightings.len()];
                let a = fresh_list_new.set_material(&mut library, Some(name_value), lighting, frame, view_value);
                let b = reference.set_material_uncached(&mut library, Some(name_value), lighting, frame, view_value);
                assert_eq!(format!("{:?}", *a), format!("{b:?}"), "frame {image} {name_value} {lighting:?}");
                calls_list += 1;
            }
            assert_eq!(fresh_list_new.state_variable(), reference.state_variable(), "frame {image}");
        }
        assert_eq!(calls_list, 64 * 18);
    }

    /// The previous UTexture::Get (state read then always written back under an allocated key).
    fn texture_get_old(m: &mut Materials, library: &mut Library, path: &str, frame: FrameTime) -> String {
        let Some(mut a) = m.anim_state(library, path) else { return path.to_string() };
        if m.anim_next(library, path).is_some() {
            if let (last, _, Some(dt)) = texture_update(a.last_frame, 0, frame.count as u8, frame.delta) {
                a.last_frame = last;
                m.texture_tick(library, path, &mut a, dt);
            }
        }
        let current = a.current.clone().unwrap_or_else(|| path.to_string());
        m.anims.insert(path.to_ascii_lowercase(), a);
        current
    }

    #[test]
    fn texture_get_identical_to_old() {
        let mut library = Library::new(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let (mut fresh_list_new, mut prior) = (materials_list(), materials_list());
        let paths = ["T.Anim1", "t.anim2", "T.A", "T.Absent", "T.ANIM2", "t.b"];
        let mut count = 0u32;
        for i in 0..900usize {
            // Repeated and skipped frames, and the 8-bit counter wrapping to zero.
            count += [1, 0, 1, 2, 1, 0, 255][i % 7];
            let frame = FrameTime { count, delta: [0.033, 0.0, 0.2, 0.016, 0.09][i % 5] };
            let p = paths[(i * 7 + i / 5) % paths.len()];
            assert_eq!(fresh_list_new.texture_get(&mut library, p, frame), texture_get_old(&mut prior, &mut library, p, frame), "call {i} {p}");
            let state_value = |m: &Materials| {
                let mut v: Vec<String> = m.anims.iter().map(|(k, a)| format!("{k}={a:?}")).collect();
                v.sort();
                v
            };
            assert_eq!(state_value(&fresh_list_new), state_value(&prior), "call {i} {p}");
        }
    }
}
