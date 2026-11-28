//! wgpu renderer: the map's geometry in one buffer, drawn batch by batch with the
//! Direct3D passes D3DDrv derives from each material (see `d3d.rs` and `openxiii_assets::material`).

use crate::d3d::{self, DrawBits, FrameUniform, PassUniform, PipelineKey, PASS_STRIDE};
use crate::game::CharacterFrame;
#[path = "impacts.rs"]
mod impacts;

// On by default;
fn impacts_incremental() -> bool { true }
fn matrices_impacts() -> bool { impacts_incremental() }
use bytemuck::{Pod, Zeroable};
use openxiii_assets::material::{FrameTime, Lighting, Materials, TexRef, STAGES};
use openxiii_assets::world::{DrawState, Vertex, World};
use openxiii_package::library::Library;
use std::collections::HashMap;

/// Tables consulted on every draw (pipelines, texture groups, textures, samplers):
/// fast hashing, same content, never iterated in an order that matters.
type TableFast<K, V> = HashMap<K, V, rustc_hash::FxBuildHasher>;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

/// HUD: quads in screen pixels, texture × colour, alpha blending.
const HUD_SHADER: &str = r#"
struct HudU { screen: vec4<f32> };
@group(0) @binding(0) var<uniform> hu: HudU;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct HOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) c: vec4<f32>) -> HOut {
    var o: HOut;
    o.pos = vec4<f32>(p.x / hu.screen.x * 2.0 - 1.0, 1.0 - p.y / hu.screen.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    o.color = c;
    return o;
}

@fragment
fn fs_main(i: HOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv) * i.color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HudVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

/// A camera view to render into a rectangle (texels of a 256 × 256 texture) of a render texture.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetView {
    pub rect: [f32; 4],
    pub camera: Camera,
    /// Horizontal field of view (degrees), as in Unreal.
    pub fov: f32,
    /// FilterColor (R, G, B, A) and |HighLight| of URenderTargetMaterial::Update.
    pub filter: [u8; 4],
    pub highlight: f32,
    /// HighLight > 0 (PlayerController+0x5B1).
    pub high_light: bool,
    /// FilterTexture (+0x5BC), material path; empty: WhiteTexture.
    pub filter_texture: String,
    /// DistanceFogColor of the camera's zone if it has bForceClear.
    pub force_clear: Option<[u8; 4]>,
    /// Number of the Update call: a new number renders the view again.
    pub serial: u64,
}

/// Clipping planes of the engine's projection (5 and 65536).
const NEAR_PLANE: f32 = 5.0;
const FAR_PLANE: f32 = 65_536.0;

/// Stage drawn by a scene pass.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    /// Child sky scene (FSkySceneNode), seen from its camera; `vignette`: its
    /// bIgnoreVignetteAlpha actors set aside, after the filter.
    Sky { vignette: bool },
    /// Opaque then translucent parts of the map.
    Scene,
    /// bIgnoreVignetteAlpha actors set aside when UseVignetteHighlight, after the filter.
    Vignette,
    /// Deferred actors, depth cleared.
    Delayed,
}

/// bDelayDisplay actor drawn after the scene with the camera's projection.
pub const DELAYED: u8 = 1;
/// bDelayDisplay and bSpecialDelayFov actor: 50° projection.
pub const DELAYED_FOV: u8 = 2;
/// `DELAYED_FOV` model that reaches the eye (character::REACHES_EYE): same projection, with the
/// near plane at `NEAR_PLANE_EYE` when the view is widened past 4:3 (`view_widen`).
pub const DELAYED_FOV_EYE: u8 = 3;
/// Near plane of `DELAYED_FOV_EYE` in a widened view. Only the depth row of the projection
/// changes: x, y and w (what lands where on the screen) stay bit for bit those of `NEAR_PLANE`,
/// so the 4:3 picture is untouched and the wider sides show the part that was cut away.
const NEAR_PLANE_EYE: f32 = 0.5;

/// Size (texels) of XIII's render textures: RenderTargetMaterial USize = VSize = 256 (class
/// values), AllocRect's grid (32 × 32 cells of 8) and the comic panels
/// (XIIIBaseHud.DrawCartoonWindowBis: 256 × 192 + 32) assume it; rendered at this size, like
/// the original (pixelated panel views).
const TARGET_SIDE: f32 = 256.0;
/// Render textures made ahead at load (the HUD's comic panels and the levels' cameras).
const SPARE_TARGETS: usize = 4;
/// Flakes (drawn cells × active points) the rain and snow buffers made at load hold.
const WEATHER_QUADS: usize = 16384;
/// Frames of a level (since its first pipeline) during which new passes
/// get a specialised pipeline; afterwards the combiner is read from the uniforms.
const IMAGES_SPECIALIZED: u32 = 3;
fn submit_value<I: IntoIterator<Item = wgpu::CommandBuffer>>(queue: &wgpu::Queue, cbs: I) {
    // Pending encoder of this queue (comic panel views, ordered writes): sent first,
    // in the same submission.
    let wait_value = ORDER.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_some_and(|x| x.queue == *queue && x.encoder_value.is_some()) {
            let mut x = o.take().unwrap();
            let done_list = x.finish_value();
            if let Some(a) = x.ring {
                return_ring(&x.queue, a);
            }
            done_list
        } else {
            Vec::new()
        }
    });
    // Grouped writes of the queue: a single upload and their copies, at the head of the submission,
    // where wgpu applies write_buffer calls (before all the commands sent).
    let lot = empty_batch(queue);
    let (lot_before, lot_after) = if batch_test() { (None, lot) } else { (lot, None) };
    let cbs = lot_before.into_iter().chain(wait_value).chain(cbs).chain(lot_after);
    queue.submit(cbs);
}
/// Each submission has a fixed cost (wgpu-core, driver) that weighs on slow CPUs: the render
/// texture views are recorded into a pending encoder, sent with the main frame by the next
/// `submit_value`. While it waits, the buffer writes (`write`) go through a ring then a copy
/// recorded in their place in this encoder: each view reads exactly the bytes it read with its
/// own submission (frame and pass uniforms, vertices rewritten between two views). Texture
/// writes (dynamic lightmaps): the pending encoder is first sent alone.
struct Order {
    queue: wgpu::Queue,
    device: wgpu::Device,
    /// None while a view records its passes into it (direct writes meanwhile).
    encoder_value: Option<wgpu::CommandEncoder>,
    /// Encoders already finished (wgpu-core only resolves buffers at `finish`: each view is
    /// finished as soon as it is recorded), sent in order.
    done_list: Vec<wgpu::CommandBuffer>,
    /// Buffers of the copies of the open encoder, kept alive until its `finish` (a buffer
    /// replaced then dropped before would have disappeared).
    kept_names: Vec<wgpu::Buffer>,
    /// Buffers read by the pending passes or already copied into the encoder: only their
    /// writes need to take their place in the order. The others (main view pass
    /// buffer, Canvas, shadows...) stay direct: applied before the submission, they
    /// change nothing in what the pending views read.
    read_count: Vec<wgpu::Buffer>,
    ring: Option<wgpu::Buffer>,
    pos: u64,
}

impl Order {
    fn finish_value(&mut self) -> Vec<wgpu::CommandBuffer> {
        if let Some(e) = self.encoder_value.take() {
            self.done_list.push(e.finish());
        }
        self.kept_names.clear();
        std::mem::take(&mut self.done_list)
    }
}

thread_local! {
    static ORDER: std::cell::RefCell<Option<Order>> = const { std::cell::RefCell::new(None) };
    /// Free rings from one frame to the next, per queue.
    static RINGS: std::cell::RefCell<Vec<(wgpu::Queue, wgpu::Buffer)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn submission_unique() -> bool {
    true
}

/// At load: the ordered writes ring of `queue` (1 MiB, its first size), when none is free.
fn prewarm_ring(device: &wgpu::Device, queue: &wgpu::Queue) {
    let held = ORDER.with(|o| o.borrow().as_ref().is_some_and(|x| x.queue == *queue && x.ring.is_some()));
    let free = RINGS.with(|a| a.borrow().iter().any(|(q, _)| q == queue));
    if held || free {
        return;
    }
    let ring = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ordered writes ring"),
        size: 1 << 20,
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    return_ring(queue, ring);
}

fn return_ring(queue: &wgpu::Queue, ring: wgpu::Buffer) {
    RINGS.with(|a| {
        let mut a = a.borrow_mut();
        a.retain(|(q, _)| q != queue);
        a.push((queue.clone(), ring));
    });
}

/// Pending encoder of `queue` (opened if needed), taken while passes are recorded into it;
/// to be returned through `return_encoder`.
fn take_encoder(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::CommandEncoder {
    // A pending encoder of another queue (renderer dropped without sending it) is abandoned:
    // its buffers may have disappeared with it.
    ORDER.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_some_and(|x| x.queue != *queue) {
            *o = None;
        }
    });
    ORDER.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_none_or(|x| x.queue != *queue) {
            let ring = RINGS.with(|a| {
                let mut a = a.borrow_mut();
                a.iter().position(|(q, _)| q == queue).map(|i| a.remove(i).1)
            });
            *o = Some(Order {
                queue: queue.clone(),
                device: device.clone(),
                encoder_value: Some(device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame (pending)") })),
                done_list: Vec::new(),
                kept_names: Vec::new(),
                read_count: Vec::new(),
                ring,
                pos: 0,
            });
        }
        o.as_mut().unwrap().encoder_value.take().expect("pending encoder already taken")
    })
}

fn return_encoder(queue: &wgpu::Queue, encoder_value: wgpu::CommandEncoder) {
    ORDER.with(|o| {
        let mut o = o.borrow_mut();
        let x = o.as_mut().filter(|x| x.queue == *queue).expect("encoder given back without a wait");
        // Finished right away: its resources are resolved before being dropped.
        x.done_list.push(encoder_value.finish());
        x.kept_names.clear();
        x.encoder_value = Some(x.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame (pending)") }));
    });
}

/// Writes of `queue` to perform in order in the pending encoder.
fn active_order(queue: &wgpu::Queue) -> bool {
    ORDER.with(|o| o.borrow().as_ref().is_some_and(|x| x.queue == *queue && x.encoder_value.is_some()))
}

/// Write of `buffer` to put in order (read by the pending passes).
fn order_concerns(queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> bool {
    ORDER.with(|o| o.borrow().as_ref().is_some_and(|x| x.queue == *queue && x.encoder_value.is_some() && x.read_count.contains(buffer)))
}

/// `buffer` read by passes recorded in the pending encoder of `queue`.
fn note_read_back(queue: &wgpu::Queue, buffer: &wgpu::Buffer) {
    ORDER.with(|o| {
        if let Some(x) = o.borrow_mut().as_mut().filter(|x| x.queue == *queue) {
            if !x.read_count.contains(buffer) {
                x.read_count.push(buffer.clone());
            }
        }
    });
}

/// The pending encoder is sent alone (before a write that cannot be ordered).
fn empty_order(queue: &wgpu::Queue) {
    if active_order(queue) {
        submit_value(queue, []);
    }
}

/// `data` written at `offset` of `buffer` in its place in the pending encoder; false when nothing is pending.
fn write_ordered(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) -> bool {
    ORDER.with(|o| {
        let mut o = o.borrow_mut();
        let Some(x) = o.as_mut().filter(|x| x.queue == *queue && x.encoder_value.is_some()) else { return false };
        if !x.read_count.contains(buffer) {
            return false;
        }
        let n = data.len() as u64;
        // The ring made at load (`prewarm_ring`), when none is held yet.
        if x.ring.is_none() && x.pos == 0 {
            x.ring = RINGS.with(|a| {
                let mut a = a.borrow_mut();
                a.iter().position(|(q, r)| q == queue && r.size() >= n).map(|i| a.remove(i).1)
            });
        }
        if x.ring.as_ref().is_none_or(|a| x.pos + n > a.size()) {
            // Ring too small: a new one (the old one stays alive while the encoder copies it).
            let size_value = (x.pos + n).max(1 << 20).next_power_of_two();
            x.ring = Some(x.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ordered writes ring"),
                size: size_value,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            x.pos = 0;
        }
        let a = x.ring.clone().unwrap();
        if !write_group(queue, &a, x.pos, data) {
            queue.write_buffer(&a, x.pos, data);
        }
        x.encoder_value.as_mut().unwrap().copy_buffer_to_buffer(&a, x.pos, buffer, offset, Some(n));
        x.kept_names.push(a);
        x.kept_names.push(buffer.clone());
        x.pos += n.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        true
    })
}

/// Grouped writes of a queue. Each `queue.write_buffer` made wgpu-core create a new staging buffer (in Direct3D 11 a `vec![0; n]`, with its
/// tracking and deferred destruction): ~26 per frame, never reused. The writes between two submissions are placed one after another in
/// `data_value`; `submit_value` sends them in a single `write_buffer` to `send_value` (buffer kept from one frame to the next, recreated only if
/// too small), then one copy per write, in call order, in an encoder placed at the head of the submission: where wgpu applies its pending writes.
/// Same bytes at the same places, seen by the same commands.
struct Lot {
    queue: wgpu::Queue,
    device: wgpu::Device,
    data_value: Vec<u8>,
    /// (destination, offset in the destination, offset in `data_value`, size).
    copies: Vec<(wgpu::Buffer, u64, u64, u64)>,
    send_value: Option<wgpu::Buffer>,
}

thread_local! {
    static BATCHES: std::cell::RefCell<Vec<Lot>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn writes_grouped() -> bool {
    false
}

fn batch_test() -> bool {
    false
}

/// Registration of a renderer's queue for grouped writes, removed when it is dropped
/// (a dropped queue keeps neither its device nor its buffers).
pub(crate) struct KeptLot(Option<wgpu::Queue>);

impl KeptLot {
    pub(crate) fn register(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        if !writes_grouped() {
            return KeptLot(None);
        }
        BATCHES.with(|l| {
            let mut l = l.borrow_mut();
            l.retain(|x| x.queue != *queue);
            l.push(Lot { queue: queue.clone(), device: device.clone(), data_value: Vec::new(), copies: Vec::new(), send_value: None });
        });
        KeptLot(Some(queue.clone()))
    }
}

impl Drop for KeptLot {
    fn drop(&mut self) {
        if let Some(q) = self.0.take() {
            // try_with: nothing to do if the thread is already ending.
            let _ = BATCHES.try_with(|l| l.borrow_mut().retain(|x| x.queue != q));
        }
    }
}

const THRESHOLD_GROUP_DEFAULT: usize = 16 << 10;

fn threshold_group() -> usize {
    THRESHOLD_GROUP_DEFAULT
}

/// Batch of `queue` that must take this write: any small write, and any large one that
/// overlaps a write already pending in the same buffer (direct, wgpu would apply it
/// before it: reversed order). A large write without overlap stays direct: applied
/// before the batch's copies, it touches none of their bytes.
fn batch_for(l: &mut [Lot], queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, size: u64) -> Option<usize> {
    let i = l.iter().position(|x| x.queue == *queue)?;
    if size as usize <= threshold_group() || l[i].copies.iter().any(|c| c.0 == *buffer && c.1 < offset + size && offset < c.1 + c.3) {
        Some(i)
    } else {
        None
    }
}

/// `size` zero bytes reserved in `queue`'s shared upload for `buffer` at `offset`:
/// (batch, start in `data_value`); None: direct write.
fn reserve_group(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, size: u64) -> Option<(usize, usize)> {
    BATCHES.with(|l| {
        let mut l = l.borrow_mut();
        let i = batch_for(&mut l, queue, buffer, offset, size)?;
        let x = &mut l[i];
        let start_value = x.data_value.len();
        // Aligned like buffer copies (write_buffer already requires sizes that are multiples of 4).
        let end_value = start_value + (size as usize).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT as usize);
        x.data_value.resize(end_value, 0);
        x.copies.push((buffer.clone(), offset, start_value as u64, size));
        Some((i, start_value))
    })
}

/// `data` written at `offset` of `buffer` through `queue`'s shared upload; false: direct write.
fn write_group(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    BATCHES.with(|l| {
        let mut l = l.borrow_mut();
        let Some(i) = batch_for(&mut l, queue, buffer, offset, data.len() as u64) else { return false };
        let x = &mut l[i];
        let start_value = x.data_value.len();
        x.data_value.extend_from_slice(data);
        x.data_value.resize(start_value + data.len().next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT as usize), 0);
        x.copies.push((buffer.clone(), offset, start_value as u64, data.len() as u64));
        true
    })
}

/// Pending writes of `queue`: one upload and their copies, in a finished encoder.
fn empty_batch(queue: &wgpu::Queue) -> Option<wgpu::CommandBuffer> {
    BATCHES.with(|l| {
        let mut l = l.borrow_mut();
        let x = l.iter_mut().find(|x| x.queue == *queue)?;
        if x.copies.is_empty() {
            return None;
        }
        let n = x.data_value.len() as u64;
        // Beyond 16 MiB (loading), upload for this submission only.
        const KEPT_MAX: u64 = 16 << 20;
        let send_value = match &x.send_value {
            Some(e) if e.size() >= n => e.clone(),
            _ => {
                let size_value = if n > KEPT_MAX { n } else { n.next_power_of_two().max(1 << 20) };
                let e = x.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("upload of the grouped writes"),
                    size: size_value,
                    usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if size_value <= KEPT_MAX {
                    x.send_value = Some(e.clone());
                }
                e
            }
        };
        queue.write_buffer(&send_value, 0, &x.data_value);
        let mut encoder_value = x.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grouped writes") });
        for (dst, dst_off, src_off, size_value) in x.copies.drain(..) {
            encoder_value.copy_buffer_to_buffer(&send_value, src_off, &dst, dst_off, Some(size_value));
        }
        x.data_value.clear();
        Some(encoder_value.finish())
    })
}

/// Device opened by `open_adapter` able to create pipelines from several threads.
static CREATION_PARALLEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Target {
    /// Views already rendered (only those of a new Update call are rendered).
    rendered: Vec<TargetView>,
    view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

/// Immutable buffers of a filter rectangle. Passes already encoded may
/// still read them when the next rectangle changes.
struct FilterTileBuffers {
    rect: [u32; 4],
    color: [u32; 4],
    size: [u32; 2],
    vertices: wgpu::Buffer,
    uniform: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    /// Frame that last drew it (its contents cannot be rewritten before that frame is sent).
    frame: u32,
}

/// The mirror goes with its GPU buffer: a pass's rank is specific to the view.
struct PassBuffers {
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    capacity: u64,
    gpu: Vec<PassUniform>,
    shared: Vec<Option<Rc<PassUniform>>>,
}

/// The script's render textures and the pipelines of their filter rectangles.
struct Hud {
    pipeline: wgpu::RenderPipeline,
    modulate_pipeline: wgpu::RenderPipeline,
    /// Alpha only written, at 1: render textures in X8R8G8B8 (see `opaque_alpha`).
    alpha_pipeline: wgpu::RenderPipeline,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    /// The three pipelines above for the multisampled world target (sample count, [hud, alpha,
    /// modulate]): filter tiles drawn over the world with MSAA.
    msaa: Option<(u32, [wgpu::RenderPipeline; 3])>,
    targets: HashMap<String, Target>,
}

/// HUD pipelines (alpha blended, alpha only, FB_Modulate) for `samples` samples per pixel.
fn hud_pipelines(device: &wgpu::Device, shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, color_format: wgpu::TextureFormat, samples: u32) -> [wgpu::RenderPipeline; 3] {
    let make_masked = |label: &str, blend: wgpu::BlendState, write_mask: wgpu::ColorWrites| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(blend), write_mask })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let make = |label: &str, blend: wgpu::BlendState| make_masked(label, blend, wgpu::ColorWrites::ALL);
    let pipeline = make("hud", wgpu::BlendState::ALPHA_BLENDING);
    let alpha_pipeline = make_masked("alpha opaque", wgpu::BlendState::REPLACE, wgpu::ColorWrites::ALPHA);
    // FinalBlend FB_Modulate (ApplyFinalBlend: DESTCOLOR, SRCCOLOR), for HighLight.
    let modulate = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add };
    let modulate_pipeline = make("modulated hud", wgpu::BlendState { color: modulate, alpha: modulate });
    [pipeline, alpha_pipeline, modulate_pipeline]
}

/// Scene vertex: position, normal, diffuse colour, two sets of coordinates.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SceneVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [u8; 4],
    uv0: [f32; 2],
    uv1: [f32; 2],
}

impl From<&Vertex> for SceneVertex {
    fn from(v: &Vertex) -> Self {
        Self { position: v.position, normal: v.normal, color: [255; 4], uv0: v.uv, uv1: [0.; 2] }
    }
}

/// The indices stay fixed. Merging neighbouring writes limits the calls to the GL driver.
const VERTICES_PER_PAGE: usize = 128;

struct VertexUpdates {
    staging: Vec<SceneVertex>,
    /// Mirror of the bytes actually uploaded, shared by every view of this buffer.
    uploaded: Vec<SceneVertex>,
    /// Pages to redo in this call: those of a new, changed or vanished mesh.
    dirty: Vec<bool>,
    /// Meshes and colours of the previous call, whose bytes the mirror keeps exactly.
    /// The Rcs kept alive prevent a new allocation from taking the same address.
    last: Vec<(usize, usize, Option<std::rc::Rc<dyn std::any::Any>>)>,
    last_colors: Vec<(usize, Vec<[u8; 4]>)>,
    /// First call: every page (the loading geometry may already contain
    /// visible actors).
    fresh: bool,
}

/// Same allocation, hence same vertices (the content of a shared Rc does not change).
fn same_vertices(a: &Option<std::rc::Rc<dyn std::any::Any>>, b: &Option<std::rc::Rc<dyn std::any::Any>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => std::ptr::addr_eq(std::rc::Rc::as_ptr(a), std::rc::Rc::as_ptr(b)),
        _ => false,
    }
}

impl VertexUpdates {
    fn new(initial: Vec<SceneVertex>) -> Self {
        let pages = initial.len().div_ceil(VERTICES_PER_PAGE);
        Self {
            staging: vec![SceneVertex::from(&Vertex::default()); initial.len()],
            uploaded: initial,
            dirty: vec![false; pages],
            last: Vec::new(),
            last_colors: Vec::new(),
            fresh: true,
        }
    }

    fn mark(&mut self, first: usize, end: usize) {
        let end = end.min(self.staging.len());
        if first < end {
            self.dirty[first / VERTICES_PER_PAGE..end.div_ceil(VERTICES_PER_PAGE)].fill(true);
        }
    }

    #[cfg(test)]
    fn write<'a>(
        &mut self,
        meshes: impl Iterator<Item = (usize, &'a [Vertex])>,
        colors: &[(usize, Vec<[u8; 4]>)],
        upload: impl FnMut(usize, &[SceneVertex]),
    ) {
        self.write_keyed(meshes.map(|(first, mesh)| (first, mesh, None)), colors, upload);
    }

    /// Pages where no mesh changed since the previous call (same Rc at the same
    /// rank, same colours) are neither reconverted nor compared: the mirror already holds them.
    /// `None`: vertices rebuilt this frame, always compared.
    fn write_keyed<'a>(
        &mut self,
        meshes: impl Iterator<Item = (usize, &'a [Vertex], Option<std::rc::Rc<dyn std::any::Any>>)>,
        colors: &[(usize, Vec<[u8; 4]>)],
        mut upload: impl FnMut(usize, &[SceneVertex]),
    ) {
        let mut meshes: Vec<_> = meshes.collect();
        self.dirty.fill(self.fresh);
        self.fresh = false;
        // Both lists are sorted by first vertex. After Game::characters_merge, an inserted or
        // changed rank has its vertices at the end of the array: sorted here (same final content,
        // dense array) instead of redoing everything.
        if !meshes.windows(2).all(|w| w[0].0 <= w[1].0) {
            if crate::game::characters_revived() {
                meshes.sort_by_key(|m| m.0);
            } else {
                self.dirty.fill(true);
            }
        }
        let last = std::mem::take(&mut self.last);
        let (mut i, mut j) = (0, 0);
        while i < meshes.len() || j < last.len() {
            match (meshes.get(i), last.get(j)) {
                (Some((first, mesh, id)), Some((old_first, old_len, old_id))) if first == old_first => {
                    if mesh.len() != *old_len || !same_vertices(id, old_id) {
                        self.mark(*first, first + mesh.len().max(*old_len));
                    }
                    i += 1;
                    j += 1;
                }
                (Some((first, mesh, _)), old) if old.is_none_or(|o| *first < o.0) => {
                    self.mark(*first, first + mesh.len());
                    i += 1;
                }
                (_, Some((old_first, old_len, _))) => {
                    self.mark(*old_first, old_first + old_len);
                    j += 1;
                }
                (_, None) => unreachable!(),
            }
        }
        if colors != self.last_colors.as_slice() {
            let ranges: Vec<(usize, usize)> = colors.iter().chain(&self.last_colors).map(|(first, c)| (*first, first + c.len())).collect();
            for (first, end) in ranges {
                self.mark(first, end);
            }
            self.last_colors = colors.to_vec();
        }
        self.last = meshes.iter().map(|(first, mesh, id)| (*first, mesh.len(), id.clone())).collect();

        // Vertices and colours of the pages to redo only.
        let dirty = &self.dirty;
        let page_of = |k: usize| k / VERTICES_PER_PAGE;
        for (first, mesh, _) in &meshes {
            // A mesh past the end of the mirror (list not enlarged yet this frame: SPADS02b and
            // Kello01a's checkpoint) is cut at the mirror's end instead of panicking; the
            // next set_dynamic enlarges the list.
            let (first, end) = (*first, (first + mesh.len()).min(self.staging.len()));
            let mut k = first;
            while k < end {
                let page_end = ((page_of(k) + 1) * VERTICES_PER_PAGE).min(end);
                if dirty.get(page_of(k)).copied().unwrap_or(false) {
                    for (dst, src) in self.staging[k..page_end].iter_mut().zip(&mesh[k - first..]) {
                        *dst = SceneVertex::from(src);
                    }
                }
                k = page_end;
            }
        }
        for (first, colors) in colors {
            let first = (*first).min(self.staging.len());
            let end = (first + colors.len()).min(self.staging.len());
            for (k, (dst, color)) in self.staging[first..end].iter_mut().zip(colors).enumerate() {
                if dirty[page_of(first + k)] {
                    dst.color = *color;
                }
            }
        }

        let mut page = 0;
        while page < self.dirty.len() {
            if !self.dirty[page] {
                page += 1;
                continue;
            }
            let first = page * VERTICES_PER_PAGE;
            page += 1;
            while page < self.dirty.len() && self.dirty[page] {
                page += 1;
            }
            let end = (page * VERTICES_PER_PAGE).min(self.staging.len());
            let chunk = &mut self.staging[first..end];
            let sent = &mut self.uploaded[first..end];
            // Keep a single upload per active range and skip the identical pages at both
            // ends. The colour is part of the bytes, including in a hidden view.
            let pages = chunk.chunks(VERTICES_PER_PAGE).zip(sent.chunks(VERTICES_PER_PAGE));
            let differs = |(a, b): (&[SceneVertex], &[SceneVertex])| {
                bytemuck::cast_slice::<_, u8>(a) != bytemuck::cast_slice::<_, u8>(b)
            };
            if let Some(left) = pages.clone().position(differs) {
                let start = left * VERTICES_PER_PAGE;
                let last = pages.skip(left + 1).rposition(differs).map_or(left, |n| left + 1 + n);
                let stop = ((last + 1) * VERTICES_PER_PAGE).min(chunk.len());
                upload(first + start, &chunk[start..stop]);
                sent[start..stop].copy_from_slice(&chunk[start..stop]);
            }
            chunk.fill(SceneVertex::from(&Vertex::default()));
        }
    }
}

#[cfg(test)]
#[test]
fn sparse_vertex_writes_match_dense_frames() {
    let size = 4 * VERTICES_PER_PAGE + 1;
    let posed = Vertex { position: [1., 2., 3.], ..Vertex::default() };
    let collapsed = SceneVertex::from(&Vertex::default());
    let mut gpu = vec![SceneVertex::from(&posed); size];
    let mut updates = VertexUpdates::new(gpu.clone());
    let start = VERTICES_PER_PAGE - 1;
    let mesh = [posed; 3];
    let colors = vec![(size - 2, vec![[1, 2, 3, 4]; 3])];

    // An actor crosses a page boundary, disappears in the next view, then comes back.
    // The colour overflows onto the last partial page and disappears too.
    for (visible, colored) in [(true, true), (false, true), (false, false), (false, false), (true, false)] {
        let mut expected = vec![collapsed; size];
        if visible {
            expected[start..start + mesh.len()].fill(SceneVertex::from(&posed));
        }
        if colored {
            for vertex in &mut expected[size - 2..] {
                vertex.color = [1, 2, 3, 4];
            }
        }
        let meshes = visible.then_some((start, mesh.as_slice())).into_iter();
        let mut written = 0;
        updates.write(meshes, if colored { &colors } else { &[] }, |first, span| {
            gpu[first..first + span.len()].copy_from_slice(span);
            written += span.len();
        });
        assert_eq!(bytemuck::cast_slice::<_, u8>(&gpu), bytemuck::cast_slice::<_, u8>(&expected));
        if !colored && visible {
            assert_eq!(written, 2 * VERTICES_PER_PAGE);
        }
    }
    let meshes = [(start, mesh.as_slice())].into_iter();
    updates.write(meshes, &[], |_, _| panic!("an identical view must not send anything"));
    let mut changed = mesh;
    changed[2].uv[0] = -0.0;
    let mut ranges = Vec::new();
    updates.write([(start, changed.as_slice())].into_iter(), &[], |first, span| ranges.push((first, span.len())));
    assert_eq!(ranges, [(VERTICES_PER_PAGE, VERTICES_PER_PAGE)]);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Position in Unreal coordinates.
    pub position: [f32; 3],
    /// Yaw, pitch and roll in degrees (Unreal convention: yaw around Z).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl Camera {
    /// Frame read by USpriteEmitter::FillVertexBuffer (position, right, up, forward).
    pub fn view_frame(&self) -> openxiii_assets::particles::ViewFrame {
        let [x, y, z] = self.axes();
        openxiii_assets::particles::ViewFrame { location: self.position, right: y, up: z, forward: x }
    }

    /// X (look), Y (right) and Z (up) axes of the Unreal rotation (FRotationMatrix).
    pub(crate) fn axes(&self) -> [[f32; 3]; 3] {
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let (sr, cr) = self.roll.to_radians().sin_cos();
        [[cp * cy, cp * sy, sp], [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp], [-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp]]
    }

    /// View of a sky zone (FSkySceneNode, FLevelSceneNode): the camera
    /// placed at the SkyZoneInfo and turned by its Rotation (matrix T(-Location)·R⁻¹·T(origin));
    /// a direction D of the view becomes Rot(D) in the sky zone (e.g. Sanc03: yaw -10832).
    pub(crate) fn sky_view(&self, location: [f32; 3], rotation: [i32; 3]) -> Camera {
        let unit = |r: i32| r as f32 * 360.0 / 65536.0;
        let sky = Camera { position: location, pitch: unit(rotation[0]), yaw: unit(rotation[1]), roll: unit(rotation[2]) }.axes();
        let rot = |v: [f32; 3]| [0, 1, 2].map(|k| v[0] * sky[0][k] + v[1] * sky[1][k] + v[2] * sky[2][k]);
        let [x, y, z] = self.axes().map(rot);
        // Inverse of `axes`: x = (cp·cy, cp·sy, sp), y.z = -sr·cp, z.z = cr·cp.
        let deg = |r: f32| r.to_degrees();
        Camera { position: location, yaw: deg(x[1].atan2(x[0])), pitch: deg(x[2].clamp(-1.0, 1.0).asin()), roll: deg((-y[2]).atan2(z[2])) }
    }
}

type Mat = [[f32; 4]; 4];

#[path = "target_clip.rs"]
mod target_clip;

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Right-handed view matrix (columns), camera at `eye` looking towards `dir`.
fn look_to_rh(eye: [f32; 3], dir: [f32; 3], up: [f32; 3]) -> Mat {
    let f = normalize(dir);
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [[s[0], u[0], -f[0], 0.0], [s[1], u[1], -f[1], 0.0], [s[2], u[2], -f[2], 0.0], [-dot(s, eye), -dot(u, eye), dot(f, eye), 1.0]]
}

/// Right-handed perspective projection, depth 0..1 (wgpu convention).
fn perspective_rh(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat {
    let f = 1.0 / (fov_y / 2.0).tan();
    [[f / aspect, 0.0, 0.0, 0.0], [0.0, f, 0.0, 0.0], [0.0, 0.0, far / (near - far), -1.0], [0.0, 0.0, near * far / (near - far), 0.0]]
}

/// A projection `m` moved by `lens` in normalised device units (an off-centre projection: the
/// picture slides, nothing turns).
fn lens_shifted(m: Mat, lens: [f32; 2]) -> Mat {
    if lens == [0.0; 2] {
        return m;
    }
    // x' = x + lens.x · w, y' = y + lens.y · w.
    let mut t = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    t[3][0] = lens[0];
    t[3][1] = lens[1];
    mul(&t, &m)
}

/// Product a × b of column-major matrices.
fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0; 4]; 4];
    for (c, column) in m.iter_mut().enumerate() {
        for (r, value) in column.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    m
}

/// FCameraSceneNode's WorldToCamera, row-major (row vector × matrix): translation by
/// -ViewOrigin, inverse camera rotation, then conversion to Direct3D's axes
/// (Unreal X to Z, Y to X, Z to Y).
fn d3d_view(camera: &Camera) -> [f32; 16] {
    let [x, y, z] = camera.axes();
    let o = camera.position;
    let t = [-dot(o, x), -dot(o, y), -dot(o, z)];
    // Columns: camera X = Unreal Y, camera Y = Z, camera Z = X.
    [y[0], z[0], x[0], 0.0, y[1], z[1], x[1], 0.0, y[2], z[2], x[2], 0.0, t[1], t[2], t[0], 1.0]
}

/// Drawn batch: indices, material and engine state.
struct SceneBatch {
    first_index: u32,
    index_count: u32,
    material: Option<String>,
    state: DrawState,
    bound: openxiii_assets::material::BatchMaterial,
    prepared: Option<Box<PreparedSection>>,
}

/// Section resources, independent of its ranges and of its rank in each view.
struct PreparedSection {
    material: Rc<openxiii_assets::material::MaterialPasses>,
    cull: u8,
    bias: u8,
    inputs: Option<SectionUniformInputs>,
    lights: Vec<[[f32; 4]; 4]>,
    passes: Vec<PreparedPass>,
}

struct PreparedPass {
    pipeline: usize,
    bind: usize,
    uniform: Rc<PassUniform>,
}

// PartialEq treats zeros of opposite signs as equal. The matrices go as is
// into the uniforms: their bits must match too.
fn same_translated_pass(a: &openxiii_assets::material::Pass, b: &openxiii_assets::material::Pass) -> bool {
    a == b && a.stages.iter().zip(&b.stages).all(|(x, y)| bytemuck::bytes_of(&x.matrix) == bytemuck::bytes_of(&y.matrix))
}

#[cfg(test)]
#[test]
fn translated_section_preserves_signed_matrix_zero() {
    let a = openxiii_assets::material::Pass::default();
    let mut b = a.clone();
    assert!(same_translated_pass(&a, &b));
    b.stages[0].matrix[1] = -0.0;
    assert_eq!(a, b);
    assert!(!same_translated_pass(&a, &b));
}

/// All the parameters of PassUniform::new and set_dyn_refresh that may vary for a
/// section. The rest comes from the material translation and the light array above.
#[derive(PartialEq, Eq)]
struct SectionUniformInputs {
    flags: [bool; 8],
    ambient: [u8; 3],
    matrix: Option<[u32; 16]>,
    zone_fog: [u32; 4],
}

impl SectionUniformInputs {
    fn new(bits: DrawBits, refresh: Option<&openxiii_vm::visibility::DynRefresh>) -> Self {
        Self {
            flags: [bits.lighting, bits.color_vertex, bits.has_color, bits.local_normals, bits.diffuse_c4,
                    bits.outline, bits.position_normals, bits.screen],
            ambient: bits.ambient,
            matrix: refresh.map(|r| r.matrix.map(f32::to_bits)),
            zone_fog: bits.zone_fog,
        }
    }
}

struct Geometry {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    batches: Vec<SceneBatch>,
}

struct Dynamic {
    updates: VertexUpdates,
    /// The inactive view keeps its buffer and the mirror of what was actually written to it.
    inactive_vertices: Option<(wgpu::Buffer, VertexUpdates)>,
    target_active: bool,
    geometry: Geometry,
    /// For each batch: rank of the actor it belongs to.
    owners: Vec<usize>,
    /// bDelayDisplay actors (weapon, hands): `DELAYED` or `DELAYED_FOV`, otherwise 0.
    overlay: Vec<u8>,
    /// bIgnoreVignetteAlpha actors.
    ignore_vignette: Vec<bool>,
    /// Shadow silhouette vertices (AShadowProjector::UpdateShadow): pose of each actor
    /// drawn by a scene of the frame, main view or comic panel, even if it is not in the
    /// main view (set_shadow_vertices).
    shadow_vertices: wgpu::Buffer,
    /// Shared vertices (same Rc) already uploaded in `shadow_vertices`, by first vertex.
    shadow_sent: HashMap<usize, std::rc::Rc<dyn std::any::Any>>,
}

/// Key of a texture group: texture and U, V addressing of each stage.
/// Per stage: texture, U and V addressing, filter (SAMPLER_WORLD, SAMPLER_POINT or SAMPLER_SCREEN).
type BindKey = [(Option<String>, u32, u32, u8); STAGES];

/// Linear filtering of a world texture, with the anisotropy setting.
const SAMPLER_WORLD: u8 = 0;
/// D3DTEXF_POINT (stage point filter, Canvas tiles drawn in POINT).
const SAMPLER_POINT: u8 = 1;
/// Linear filtering of a texture drawn flat on the screen (Canvas, filter tiles): no anisotropy.
const SAMPLER_SCREEN: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    World,
    /// StaticMeshes of map actors whose mesh the script changed.
    Replaced,
    Dynamic,
    Particles,
    Canvas,
    Projectors,
    Weather,
}

/// Batch of Canvas primitives: material (key of `HudDraw`), lines or triangles, indices.
#[derive(Clone)]
struct CanvasBatch {
    material: std::sync::Arc<str>,
    lines: bool,
    first: u32,
    count: u32,
    /// Canvas slice: 0 before the first DrawPortal, n after the n-th.
    segment: usize,
    /// Stage 0 in D3DTEXF_POINT (text: FRenderInterface +0xAC).
    point: bool,
    /// Tiles drawn larger than their own size (HudDraw::Tile `enlarged`): with enhanced
    /// graphics, enlarged text is read with the bicubic filter instead of POINT.
    enlarged: bool,
    /// Font letters (HudDraw::Tile `glyph`), with or without POINT.
    glyph: bool,
}

#[path="refresh_ambient.rs"]
mod refresh_ambient;

/// Lighting of a StaticMesh drawn outside the static geometry (without a
/// StaticMeshInstance): vertex box, Actor.bSLightGroup, AmbientGlow and flags.
#[derive(Clone, Debug)]
pub struct PropLight {
    /// Actor name (lowercase, kept from one frame to the next), first vertex in the animated buffer.
    pub name: std::rc::Rc<str>,
    pub first_vertex: usize,
    /// The actor's StaticMeshInstance; world positions and normals of its vertices.
    pub instance: Option<std::rc::Rc<openxiii_assets::static_mesh::MeshInstance>>,
    /// The actor has already been drawn by the engine (Vm::first_render): positions of that render.
    pub first_rendered: bool,
    pub geometry: std::rc::Rc<crate::prop_geometry::WorldGeometry>,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub light_group: u8,
    pub glow: u8,
    pub unlit: bool,
    pub ignore_dyn: bool,
    pub no_ambient: bool,
    pub pivot_ambient: bool,
    pub location: [f32; 3],
    /// Region.ZoneNumber of the actor: the zone whose fog sets before its draw
    pub zone: Option<u8>,
    /// Actor.Leaves (Actor+0xA4): leaves of FDynamicActor::Refresh.
    pub leaves: Vec<i32>,
}

/// Shadow silhouette to draw (UShadowBitmapMaterial::Update) into target `slot`: animated actor
/// `owner` seen from `location` + `light_dir` × 1000, half-width `usize` / 2, in white, with
/// the weapons it carries (`attached`, ranks of its WeaponAttachments on a bone).
#[derive(Clone, Debug)]
pub struct ShadowRender {
    pub slot: usize,
    pub owner: usize,
    pub attached: Vec<usize>,
    pub location: [f32; 3],
    pub light_dir: [f32; 3],
    pub usize: i32,
}

/// Projector batch: material key, first index, index count.
/// Projector batch: key, first index, index count, SetZBias (8 on the BSP, 0 on terrain),
/// BSP node (None: terrain).
type ProjectorBatch = (std::rc::Rc<str>, u32, u32, u8, Option<u32>);

#[derive(Default)]
struct ProjectorPacket {
    material: openxiii_assets::material::ProjectorMaterial,
    section: Option<Box<PreparedSection>>,
}

/// UCanvas::execDrawPortal: camera and rectangle (x, y, width, height) in pixels.
#[derive(Clone)]
struct Portal {
    rect: [f32; 4],
    camera: Camera,
    fov: f32,
    /// Colour filter (R, G, B, A) of its FCameraSceneNode::Render.
    filter: Option<[u8; 4]>,
    hud_index: usize,
}

/// Particle batch: material, how to draw it, IgnoreFog, first index, index count.
#[derive(Clone)]
struct ParticleBatch {
    bounds: Option<target_clip::Bounds>,
    material: String,
    kind: ParticleBatchKind,
    ignore_fog: bool,
    first: u32,
    count: u32,
    /// The actor's Vm::draw_list: deferred (`DELAYED`, `DELAYED_FOV`) and bIgnoreVignetteAlpha.
    delayed: u8,
    ignore_vignette: bool,
    /// Actor (Emitter, Trail…): rank in the translucent list.
    actor: openxiii_vm::value::ObjId,
    /// Animation frame already chosen by the emitter's render (NoSynchroAnim).
    frame: Option<String>,
}

#[derive(Clone, Copy)]
enum ParticleBatchKind {
    /// UParticleMaterial (sprites, or meshes without UseMeshBlendMode): DrawStyle, double-sided.
    Particle { draw_style: u8, two_sided: bool },
    /// The mesh's material, lit (UseMeshBlendMode): global colour (TFACTOR), ambient.
    Mesh { tfactor: u32, ambient: [u8; 3] },
    /// Trail (ATrail::Render): UFinalBlend "#trail", triangle strip or outline lines.
    Trail { lines: bool },
}

/// Sections (material, first index, count) of an animated actor drawn by DrawCartoonSM, and
/// whether it is in first-person view.
type CartoonSections = (Vec<(String, u32, u32)>, u8);

/// One pass of a batch, ready to draw.
#[derive(Clone, Copy)]
struct DrawCmd {
    pipeline: usize,
    pass_offset: u32,
    bind: usize,
    which: Which,
    first_index: u32,
    index_count: u32,
    sorted: bool,
    /// 0: scene; `DELAYED` or `DELAYED_FOV`: actor drawn after the scene.
    overlay: u8,
    /// bIgnoreVignetteAlpha actor set aside (UseVignetteHighlight): drawn after the filter.
    vignette: bool,
    /// Canvas primitive, drawn after the scene in emission order.
    canvas: bool,
    /// Canvas slice (between two DrawPortals).
    segment: usize,
    /// Rank 's translucent list (Vm::translucent_order); sorted
    /// passes are drawn in this (stable) order. −2: actor outside the list, −1: BSP node
    /// outside the list, i64::MAX: the rest (particles, projectors, weather).
    order: i64,
    /// Sorted pass without blending (opaque section of a translucent-list actor): at the same
    /// rank, drawn before the blended passes (SSH102b: StaticMeshActor19's lamp before its halo).
    opaque_first: bool,
    /// Some(false): root/ActorScene; Some(true): child scene; None: shared geometry.
    light_sky: Option<bool>,
    /// Shared geometry of the player's scene (particles, weather, projectors without a BSP node):
    /// not drawn by the sky stage (child scene).
    outside_sky: bool,
}

/// Culled index buffer (`set_bsp_visible`) without its collapsed triangles, in the same order: the
/// triangle t of the culled buffer, if kept, is at rank "non-collapsed triangles before t" there.
fn compact_indices(indices: &[u32]) -> Vec<u32> {
    let mut v = Vec::with_capacity(indices.len());
    for t in indices.chunks_exact(3) {
        if t[0] != t[1] || t[1] != t[2] {
            v.extend_from_slice(t);
        }
    }
    v
}

/// Range of a map draw in the compact buffer: `kept` (cumulative non-collapsed) at the edges
/// of the culled buffer's range. None: range not made of whole triangles (drawn from the culled buffer).
fn compact_range(kept: &[u32], first: u32, count: u32) -> Option<(u32, u32)> {
    if first % 3 != 0 || count % 3 != 0 {
        return None;
    }
    let (a, b) = (*kept.get((first / 3) as usize)?, *kept.get(((first + count) / 3) as usize)?);
    Some((a * 3, (b - a) * 3))
}

#[cfg(test)]
#[test]
fn compact_draws_send_the_same_non_flat_triangles_in_the_same_order() {
    // Pseudo-random culled buffers (collapsed triangles, triangles already reduced to a point),
    // draws picked at random: the compact range gives exactly the non-collapsed triangles of
    // the culled range, in the same order.
    let mut seed_value = 0x2545_f491_u32;
    let mut random = move |n: u32| {
        seed_value ^= seed_value << 13;
        seed_value ^= seed_value >> 17;
        seed_value ^= seed_value << 5;
        seed_value % n
    };
    let non_flat = |tris: &[u32]| tris.chunks_exact(3).filter(|t| t[0] != t[1] || t[1] != t[2]).flat_map(|t| t.to_vec()).collect::<Vec<u32>>();
    let mut tests = 0;
    let mut control_rejected = 0;
    for _ in 0..200 {
        let n = 1 + random(400);
        let mut indices = Vec::new();
        for _ in 0..n {
            match random(4) {
                0 => { let v = random(1000); indices.extend([v, v, v]); }
                _ => indices.extend([random(1000), random(1000), random(1000)]),
            }
        }
        let mut kept = vec![0u32];
        for t in indices.chunks_exact(3) {
            let k = *kept.last().unwrap() + (t[0] != t[1] || t[1] != t[2]) as u32;
            kept.push(k);
        }
        let compact = compact_indices(&indices);
        for _ in 0..20 {
            let a = random(n + 1);
            let b = a + random(n + 1 - a);
            let (f, c) = compact_range(&kept, a * 3, (b - a) * 3).unwrap();
            let want = non_flat(&indices[(a * 3) as usize..(b * 3) as usize]);
            assert_eq!(&compact[f as usize..(f + c) as usize], want.as_slice());
            tests += 1;
            // Control: wrong mapping (range shifted by one triangle) rejected by the comparison.
            if c >= 3 && ((f + 3 + c) as usize) <= compact.len() && compact[(f + 3) as usize..(f + 3 + c) as usize] != *want.as_slice() {
                control_rejected += 1;
            }
        }
    }
    assert!(tests >= 100 && control_rejected > 100, "{tests} trials, control rejected {control_rejected}");
    assert_eq!(compact_range(&[0, 1], 1, 3), None);
}

/// Resolved texture of a particle batch material and its last prepared passes.
struct KeptParticles {
    generation: u64,
    texture: String,
    passes: Vec<ParticlePrepared>,
}

struct ParticlePrepared {
    pass: openxiii_assets::material::Pass,
    cull: u8,
    z_bias: u8,
    pipeline: usize,
    bind: usize,
    uniform: Rc<PassUniform>,
}

fn kept_particles() -> bool {
    true
}

fn particles_test() -> bool {
    false
}

/// The time gain is not established: keep the usual path by default.
fn particles_frustum() -> bool {
    false
}

fn merge_particles_requested() -> bool {
    true
}

fn merge_particles_image(_image: u32) -> bool {
    true
}

/// The box comes from the vertices actually uploaded, not from the actor's box. A missing or
/// non-finite box, a deferred batch (other FOV), or a batch visible from the sky stays drawn.
fn particle_outside_view(bounds: Option<&target_clip::Bounds>, delayed: u8, outside_sky: bool, clip: &target_clip::TargetClip) -> bool {
    if delayed != 0 { return false; }
    bounds.is_some_and(|b| {
        let (scene, sky) = clip.visible_stages(b);
        !scene && (outside_sky || !sky)
    })
}

#[cfg(test)]
#[test]
fn particles_frustum_keeps_crossing_invalid_delayed_and_sky_batches() {
    let frustum = |yaw| {
        let camera = Camera { position: [0.; 3], pitch: 0., yaw, roll: 0. };
        target_clip::Frustum::new(Renderer::frame_uniform(&camera, 90f32.to_radians(), 1., NEAR_PLANE, FAR_PLANE, None).view_proj)
    };
    let clip = target_clip::TargetClip { scene: frustum(0.), sky: Some(frustum(180.)) };
    let behind = target_clip::Bounds::from_world([[-11., -1., -1.], [-10., 1., 1.]]);
    let crossing = target_clip::Bounds::from_world([[10., -20., -20.], [10., 20., 20.]]);
    let tangent = target_clip::Bounds::from_world([[10., 10., -1.], [10., 16., 1.]]);
    assert!(particle_outside_view(behind.as_ref(), 0, true, &clip));
    assert!(!particle_outside_view(behind.as_ref(), 0, false, &clip), "the sky scene sees this batch");
    assert!(!particle_outside_view(behind.as_ref(), DELAYED_FOV, true, &clip), "distinct deferred projection");
    assert!(!particle_outside_view(crossing.as_ref(), 0, true, &clip), "quad overflowing each side of the view");
    assert!(!particle_outside_view(tangent.as_ref(), 0, true, &clip), "edge and rounding margin kept");
    assert!(!particle_outside_view(None, 0, true, &clip));
    let invalid = target_clip::Bounds::from_world([[f32::NAN, 0., 0.]]);
    assert!(!particle_outside_view(invalid.as_ref(), 0, true, &clip));
    let other_view = target_clip::TargetClip { scene: frustum(180.), sky: None };
    assert!(!particle_outside_view(behind.as_ref(), 0, true, &other_view), "another view reuses the same geometry");
}

fn target_projector_clip() -> bool {
    true
}

fn target_particle_clip() -> bool {
    true
}

/// All the passes of this polygon/batch have the same geometry and belong
/// to the same scenes. The materials have already been evaluated in their original order.
fn clip_target_batch(commands: &mut Vec<DrawCmd>, begin: usize, clip: &target_clip::TargetClip, bounds: Option<&target_clip::Bounds>) {
    if let Some(bounds) = bounds { clip_target_stages(commands, begin, clip.sky.is_some(), clip.visible_stages(bounds)); }
}

fn clip_target_stages(commands: &mut Vec<DrawCmd>, begin: usize, has_sky: bool, stages: (bool, bool)) {
    let Some(first) = commands.get(begin) else { return };
    if first.overlay != 0 { return; }
    let (scene, sky) = stages;
    let scene = scene && first.light_sky != Some(true);
    let sky = sky && !first.outside_sky && first.light_sky != Some(false);
    if !scene && !sky { commands.truncate(begin); }
    else if has_sky && scene != sky {
        for c in &mut commands[begin..] { c.light_sky = Some(sky); }
    }
}

#[cfg(test)]
#[test]
fn target_polygons_use_each_real_camera_without_changing_prior_or_delayed_draws() {
    let frustum = |yaw| {
        let c = Camera { position: [0.; 3], pitch: 0., yaw, roll: 0. };
        target_clip::Frustum::new(Renderer::frame_uniform(&c, 90f32.to_radians(), 1., NEAR_PLANE, FAR_PLANE, None).view_proj)
    };
    let clip = target_clip::TargetClip { scene: frustum(0.), sky: Some(frustum(180.)) };
    let command = DrawCmd { opaque_first: false, pipeline: 0, pass_offset: 0, bind: 0, which: Which::Projectors, first_index: 0, index_count: 6,
        sorted: true, overlay: 0, vignette: false, canvas: false, segment: 0, order: -1, light_sky: None, outside_sky: false };
    let front = target_clip::Bounds::from_world([[10., -2., 0.], [11., 2., 0.]]);
    let back = target_clip::Bounds::from_world([[-10., -2., 0.], [-11., 2., 0.]]);
    let outside = target_clip::Bounds::from_world([[0., 25., 0.], [0., 26., 0.]]);
    let mut cmds = vec![command; 2];
    clip_target_batch(&mut cmds, 1, &clip, front.as_ref());
    assert_eq!(cmds[0].light_sky, None);
    assert_eq!(cmds[1].light_sky, Some(false));
    cmds.push(command);
    clip_target_batch(&mut cmds, 2, &clip, back.as_ref());
    assert_eq!(cmds[2].light_sky, Some(true));
    cmds.push(command);
    clip_target_batch(&mut cmds, 3, &clip, outside.as_ref());
    assert_eq!(cmds.len(), 3);
    cmds.push(DrawCmd { overlay: DELAYED_FOV, ..command });
    clip_target_batch(&mut cmds, 3, &clip, outside.as_ref());
    assert_eq!(cmds.len(), 4);
    assert!(target_clip::Bounds::from_world([[f32::NAN, 1., 0.]]).is_none());
}

/// Section order of a view. DrawKeys are indices into the current passes:
/// their content (lights, face change, matrix) may vary with each draw.
#[derive(Default)]
struct JobList {
    sections: Vec<(Which, usize, Option<crate::lights_scene::DrawKey>)>,
    view: Option<Rc<crate::lights_scene::IndexedPasses>>,
    hidden: Option<Rc<std::collections::HashSet<String>>>,
    enabled: bool,
}

impl JobList {
    fn matches(&self, view: &Rc<crate::lights_scene::IndexedPasses>, hidden: &Rc<std::collections::HashSet<String>>, enabled: bool) -> bool {
        self.enabled == enabled && self.hidden.as_ref().is_some_and(|h| Rc::ptr_eq(h, hidden) || h == hidden)
            && self.view.as_ref().is_some_and(|v| !enabled || Rc::ptr_eq(v, view) || v.same_jobs(view))
    }
}

/// Commands have a single index table, whoever owns the bytes.
/// Temporary passes stay contiguous; reusing a section keeps its uniform
/// by reference, including when several views draw it at different GPU locations.
#[derive(Default)]
struct FramePasses {
    entries: Vec<FramePass>,
    transient: Vec<PassUniform>,
}

enum FramePass {
    Prepared(Rc<PassUniform>),
    Transient(usize),
}

impl FramePasses {
    fn clear(&mut self) {
        self.entries.clear();
        self.transient.clear();
    }

    fn len(&self) -> usize { self.entries.len() }

    fn push(&mut self, value: PassUniform) {
        self.entries.push(FramePass::Transient(self.transient.len()));
        self.transient.push(value);
    }

    fn push_prepared(&mut self, value: &Rc<PassUniform>) {
        self.entries.push(FramePass::Prepared(value.clone()));
    }

    fn shared(&self, index: usize) -> Option<&Rc<PassUniform>> {
        match &self.entries[index] {
            FramePass::Prepared(u) => Some(u),
            FramePass::Transient(_) => None,
        }
    }
}

impl std::ops::Index<usize> for FramePasses {
    type Output = PassUniform;

    fn index(&self, i: usize) -> &PassUniform {
        match &self.entries[i] {
            FramePass::Prepared(u) => u,
            FramePass::Transient(k) => &self.transient[*k],
        }
    }
}

/// Work buffers kept between views. Entries of more than a kilobyte
/// stay in the uniform array, only the small draw orders are moved.
#[derive(Default)]
struct CommandAssembly {
    sorted_slots: Vec<usize>,
    fused_commands: Vec<DrawCmd>,
    uniform_remap: Vec<u32>,
    /// Identical uniforms (byte for byte) of two neighbouring draws in a single slot, without rebinding the range
    /// between them (on by default;
    uniforms_shared: bool,
    /// Indices of the Particles batches only, copied in opaque then translucent order.
    particle_indices: Vec<u32>,
}

impl CommandAssembly {
    /// Rendering already walks the opaque then the translucent passes, and the Canvas separately.
    /// Materialising this stable order therefore changes the sub-order of no pass/view/segment.
    /// Particles indices are copied in this order without sorting by material: two
    /// neighbours with the same state become contiguous and `assembled` can merge them normally.
    /// Commands keep their source uniforms until `assembled` renumbers them.
    fn pack_particles(&mut self, commands: &mut Vec<DrawCmd>, original_indices: &[u32]) -> bool {
        if commands.iter().filter(|c| c.which == Which::Particles).take(2).count() < 2 { return false; }
        let mut count = 0usize;
        for c in commands.iter().filter(|c| c.which == Which::Particles) {
            let first = c.first_index as usize;
            let Some(end) = first.checked_add(c.index_count as usize) else { return false };
            let Some(total) = count.checked_add(c.index_count as usize) else { return false };
            if end > original_indices.len() || total > u32::MAX as usize { return false; }
            count = total;
        }
        self.sorted_slots.clear();
        self.sorted_slots.extend((0..commands.len()).filter(|&k| commands[k].sorted && !commands[k].canvas));
        self.sorted_slots.sort_unstable_by_key(|&k| (commands[k].order, !commands[k].opaque_first, k));
        self.fused_commands.clear();
        self.fused_commands.extend(commands.iter().filter(|c| !c.sorted && !c.canvas).copied());
        self.fused_commands.extend(self.sorted_slots.iter().map(|&k| commands[k]));
        self.fused_commands.extend(commands.iter().filter(|c| c.canvas).copied());
        self.particle_indices.clear();
        self.particle_indices.reserve(count);
        for c in self.fused_commands.iter_mut().filter(|c| c.which == Which::Particles) {
            let first = c.first_index as usize;
            c.first_index = self.particle_indices.len() as u32;
            self.particle_indices.extend_from_slice(&original_indices[first..first + c.index_count as usize]);
        }
        std::mem::swap(commands, &mut self.fused_commands);
        true
    }

    /// Groups neighbouring commands that only differ by their index range, when the
    /// second follows the first in the index buffer: same pipeline, same textures, same
    /// uniforms (byte for byte), same geometry and same stage. The GPU then receives a single
    /// draw for the same triangles in the same order: identical image, unchanged order
    /// (only adjacency in `commands` is used).
    /// `empty_value`: cumulative non-collapsed triangles of the culled map and sky buffers
    /// (`world_masked_kept`), when the scenery is drawn from them; two scenery draws with
    /// the same state separated only by collapsed triangles (no pixels) in both buffers
    /// become a single draw that also covers the gap.
    fn assembled(&mut self, commands: &mut Vec<DrawCmd>, uniforms: &FramePasses, sources: &mut Vec<usize>, empty_value: Option<(&[u32], &[u32])>) {
        sources.clear();
        let stride = PASS_STRIDE as u32;
        // Translucent ranks only: at equal rank, the original index guarantees a stable
        // order. Opaque entries stay in place, without shifting the whole array.
        let sorted = &mut self.sorted_slots;
        sorted.clear();
        sorted.extend((0..commands.len()).filter(|&k| commands[k].sorted && !commands[k].canvas));
        sorted.sort_unstable_by_key(|&k| (commands[k].order, !commands[k].opaque_first, k));
        let mut ordered = sorted.iter();
        let diff_empty = |p: &DrawCmd, c: &DrawCmd| {
            let end_value = p.first_index + p.index_count;
            p.which == Which::World && c.first_index > end_value && end_value % 3 == 0 && c.first_index % 3 == 0
                && empty_value.is_some_and(|(a, b)| {
                    let (t0, t1) = ((end_value / 3) as usize, (c.first_index / 3) as usize);
                    let empty = |k: &[u32]| k.get(t1).zip(k.get(t0)).is_some_and(|(y, x)| y == x);
                    empty(a) && empty(b)
                })
        };
        let out = &mut self.fused_commands;
        out.clear();
        out.reserve(commands.len());
        for original in commands.iter() {
            let c = if original.sorted && !original.canvas { commands[*ordered.next().unwrap()] } else { *original };
            if let Some(p) = out.last_mut() {
                let same = p.pipeline == c.pipeline
                    && p.bind == c.bind
                    && p.which == c.which
                    && p.sorted == c.sorted
                    && p.overlay == c.overlay
                    && p.vignette == c.vignette
                    && p.canvas == c.canvas
                    && p.segment == c.segment
                    && p.light_sky == c.light_sky
                    && p.outside_sky == c.outside_sky
                    && (p.first_index + p.index_count == c.first_index || diff_empty(p, &c))
                    && (p.pass_offset == c.pass_offset
                        || bytemuck::bytes_of(&uniforms[(p.pass_offset / stride) as usize]) == bytemuck::bytes_of(&uniforms[(c.pass_offset / stride) as usize]));
                if same {
                    p.index_count = c.first_index + c.index_count - p.first_index;
                    continue;
                }
            }
            out.push(c);
        }
        // Renumber the uniforms actually used, with a flat table indexed
        // by the source entry. The bytes stay in the array where they were computed.
        self.uniform_remap.clear();
        self.uniform_remap.resize(uniforms.len(), u32::MAX);
        // With sharing, the next draw takes the previous one's slot when its uniforms have
        // the same bytes: same offset, hence no new range binding (passes).
        let shared_value = true;
        let mut prec_src: Option<usize> = None;
        for c in out.iter_mut() {
            let mut src = (c.pass_offset / stride) as usize;
            if let Some(p) = prec_src.filter(|&p| shared_value && p != src) {
                if bytemuck::bytes_of(&uniforms[p]) == bytemuck::bytes_of(&uniforms[src]) {
                    src = p;
                }
            }
            prec_src = Some(src);
            let offset = &mut self.uniform_remap[src];
            if *offset == u32::MAX {
                *offset = sources.len() as u32 * stride;
                sources.push(src);
            }
            c.pass_offset = *offset;
        }
        std::mem::swap(commands, out);
    }
}

#[cfg(test)]
#[test]
fn stable_draw_order_and_shared_uniforms() {
    let stride = PASS_STRIDE as u32;
    let command = |sorted, order, first_index, src: u32| DrawCmd { opaque_first: false,
        pipeline: 0, pass_offset: src * stride, bind: 0, which: Which::World,
        first_index, index_count: 3, sorted, overlay: 0, vignette: false,
        canvas: false, segment: 0, order, light_sky: None, outside_sky: false,
    };
    let mut commands = vec![
        command(true, 9, 30, 0), command(false, 0, 60, 1), command(true, -1, 0, 2),
        command(true, 9, 33, 0), command(true, 9, 36, 1), command(true, 9, 45, 2),
    ];
    let mut uniforms = FramePasses::default();
    uniforms.push_prepared(&Rc::new(PassUniform::zeroed()));
    let mut negative = PassUniform::zeroed();
    negative.c4[0] = -0.0; // Same numeric value, different bytes: no merge.
    uniforms.push(negative);
    uniforms.push_prepared(&Rc::new(PassUniform::zeroed()));
    let mut assembly = CommandAssembly::default();
    let mut sources = Vec::new();
    assembly.assembled(&mut commands, &uniforms, &mut sources, None);
    assert_eq!(sources, [2, 1, 0]);
    assert_eq!(commands.iter().map(|c| (c.first_index, c.index_count, c.pass_offset / stride)).collect::<Vec<_>>(),
               [(0, 3, 0), (60, 3, 1), (30, 6, 2), (36, 3, 1), (45, 3, 0)]);
    commands.clear();
    uniforms.clear();
    assembly.assembled(&mut commands, &uniforms, &mut sources, None);
    assert!(commands.is_empty() && sources.is_empty());
}

#[cfg(test)]
#[test]
fn neighbouring_identical_uniforms_share_one_slot() {
    let stride = PASS_STRIDE as u32;
    // Textures differing from one draw to the next: no merge, only the uniforms count.
    let command = |bind, first_index, src: u32| DrawCmd { opaque_first: false,
        pipeline: 0, pass_offset: src * stride, bind, which: Which::World,
        first_index, index_count: 3, sorted: false, overlay: 0, vignette: false,
        canvas: false, segment: 0, order: 0, light_sky: None, outside_sky: false,
    };
    let make = || vec![command(0, 0, 0), command(1, 6, 2), command(0, 12, 1), command(1, 18, 0)];
    let mut uniforms = FramePasses::default();
    uniforms.push_prepared(&Rc::new(PassUniform::zeroed()));
    let mut negative = PassUniform::zeroed();
    negative.c4[0] = -0.0;
    uniforms.push(negative);
    uniforms.push_prepared(&Rc::new(PassUniform::zeroed()));
    let offsets = |commands: &[DrawCmd]| commands.iter().map(|c| c.pass_offset / stride).collect::<Vec<_>>();
    let mut sources = Vec::new();
    let mut commands = make();
    let mut without = CommandAssembly::default();
    without.assembled(&mut commands, &uniforms, &mut sources, None);
    let mut commands = make();
    let mut with_value = CommandAssembly { uniforms_shared: true, ..Default::default() };
    with_value.assembled(&mut commands, &uniforms, &mut sources, None);
    // The 2nd takes the 1st's slot (same bytes); -0.0 stays separate; the 4th finds the 1st again.
    assert_eq!((sources, offsets(&commands)), (vec![0, 1], vec![0, 0, 1, 0]));
}

#[cfg(test)]
#[test]
fn packed_particle_indices_preserve_every_pass_and_canvas_order() {
    let stride = PASS_STRIDE as u32;
    let command = |first_index, sorted, order| DrawCmd { opaque_first: false,
        pipeline: 0, pass_offset: 0, bind: 0, which: Which::Particles,
        first_index, index_count: 3, sorted, overlay: 0, vignette: false,
        canvas: false, segment: 0, order, light_sky: None, outside_sky: true,
    };
    let original_indices: Vec<u32> = (0..300).collect();
    let mut uniforms = FramePasses::default();
    uniforms.push(PassUniform::zeroed());
    let mut other = PassUniform::zeroed();
    other.c4[0] = -0.0; // Different bytes always prevent a merge.
    uniforms.push(other);
    let mut original = vec![command(0, true, 1), command(3, false, 0), command(6, true, 0)];
    let mut packed = original.clone();
    let mut a = CommandAssembly::default();
    let mut sources = Vec::new();
    assert!(a.pack_particles(&mut packed, &original_indices));
    a.assembled(&mut packed, &uniforms, &mut sources, None);
    assert_eq!(packed.len(), 2, "the two translucent ones separated by an opaque one merge");
    assert_eq!(&a.particle_indices[packed[1].first_index as usize..][..6], &[6, 7, 8, 0, 1, 2]);

    // Mixed orders, other geometries, scenes, uniforms, deferred projections and Canvas.
    for k in 3..80u32 {
        let mut c = command(k * 3, k % 3 != 0, i64::from((k * 17) % 13));
        c.pipeline = (k % 2) as usize;
        c.bind = (k % 3) as usize;
        c.pass_offset = (k % 2) * stride;
        c.overlay = if k % 5 == 0 { 1 + (k % 2) as u8 } else { 0 };
        c.vignette = k % 7 == 0;
        c.light_sky = match k % 3 { 0 => None, 1 => Some(false), _ => Some(true) };
        c.outside_sky = k % 2 == 0;
        if k % 11 == 0 { c.which = Which::World; }
        if k % 13 == 0 { c.which = Which::Canvas; c.canvas = true; c.segment = (k % 3) as usize; }
        original.push(c);
    }
    let mut baseline = original.clone();
    let mut base_assembly = CommandAssembly::default();
    let mut base_sources = Vec::new();
    base_assembly.assembled(&mut baseline, &uniforms, &mut base_sources, None);
    let mut candidate = original;
    assert!(a.pack_particles(&mut candidate, &original_indices));
    a.assembled(&mut candidate, &uniforms, &mut sources, None);
    let expanded = |commands: &[DrawCmd], particle_indices: &[u32], src: &[usize], stage: usize, sorted: bool| {
        let wanted = |c: &&DrawCmd| {
            if stage >= 5 { return c.canvas && c.segment == stage - 5; }
            !c.canvas && c.sorted == sorted && match stage {
                0 | 1 => c.light_sky != Some(false) && !c.outside_sky && c.overlay == 0 && c.vignette == (stage == 1),
                2 => c.light_sky != Some(true) && c.overlay == 0 && !c.vignette,
                3 => c.light_sky != Some(true) && c.overlay == 0 && c.vignette,
                _ => c.light_sky != Some(true) && c.overlay != 0,
            }
        };
        commands.iter().filter(wanted).flat_map(|c| {
            let indices = if c.which == Which::Particles { particle_indices } else { &original_indices };
            let uniform = uniforms[src[c.pass_offset as usize / stride as usize]].c4[0].to_bits();
            indices[c.first_index as usize..(c.first_index + c.index_count) as usize].iter()
                .map(move |&i| (c.which as u8, c.pipeline, c.bind, c.overlay, uniform, i))
        }).collect::<Vec<_>>()
    };
    for stage in 0..8 {
        for sorted in [false, true] {
            assert_eq!(expanded(&baseline, &original_indices, &base_sources, stage, sorted),
                       expanded(&candidate, &a.particle_indices, &sources, stage, sorted), "pass {stage}, sort {sorted}");
        }
    }
}

#[cfg(test)]
#[test]
fn packed_particle_indices_reject_bad_ranges_without_changing_commands() {
    let c = DrawCmd { opaque_first: false, pipeline: 0, pass_offset: 0, bind: 0, which: Which::Particles,
        first_index: 9, index_count: 3, sorted: false, overlay: 0, vignette: false,
        canvas: false, segment: 0, order: 0, light_sky: None, outside_sky: true };
    let mut commands = vec![c, DrawCmd { first_index: u32::MAX, ..c }];
    assert!(!CommandAssembly::default().pack_particles(&mut commands, &[0, 1, 2]));
    assert_eq!(commands.iter().map(|c| c.first_index).collect::<Vec<_>>(), [9, u32::MAX]);
}

/// Scenery kept by the engine's visibility: BSP nodes and static
/// geometry actors (lowercase names) of the player's view and its sky scenes.
#[derive(Default)]
pub struct SceneryVisible {
    pub root: Vec<bool>,
    pub sky: Vec<bool>,
    pub root_owners: std::collections::HashSet<String>,
    pub sky_owners: std::collections::HashSet<String>,
    /// Terrains rendered by the player's view and its sky scenes (lowercase names).
    pub root_terrains: std::collections::HashSet<String>,
    pub sky_terrains: std::collections::HashSet<String>,
    /// Actors drawn (FDynamicActor) by the player's view and its sky scenes.
    pub root_actors: std::collections::HashSet<openxiii_vm::value::ObjId>,
    pub sky_actors: std::collections::HashSet<openxiii_vm::value::ObjId>,
}

/// Ranks in the visibility's translucent list, per BSP node, per
/// animated actor (rank in `Characters`) and per static geometry actor (name).
#[derive(Default)]
pub struct TranslucentKeys {
    pub nodes: HashMap<u32, i64>,
    pub characters: HashMap<usize, i64>,
    pub baked: HashMap<String, i64>,
    /// Any actor of the list (emitters, trails…).
    pub actors: HashMap<openxiii_vm::value::ObjId, i64>,
}

/// A pending encoder (comic panels without a main frame after them) is sent with the renderer.
impl Drop for Renderer {
    fn drop(&mut self) {
        empty_order(&self.queue);
        // Its free ring goes with it: a queue created afterwards may take the same number
        // in wgpu-core (equal for PartialEq) without being on the same device.
        RINGS.with(|a| a.borrow_mut().retain(|(q, _)| q != &self.queue));
        ORDER.with(|o| {
            let mut o = o.borrow_mut();
            if o.as_ref().is_some_and(|x| x.queue == self.queue) {
                *o = None;
            }
        });
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The frame's grouped writes (`write_group`): queue registered while the renderer lives.
    _batch: KeptLot,
    surface: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    /// Window instance, kept to recreate the surface (Android: the native surface
    /// disappears when the game goes to the background).
    instance: Option<wgpu::Instance>,
    /// Settings of the dropped surface (drop_surface), reused by recreate_surface.
    surface_lost: Option<wgpu::SurfaceConfiguration>,
    color_format: wgpu::TextureFormat,
    width: u32,
    height: u32,

    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    /// Uniforms of the first-person pass (reduced near plane).
    overlay_frame_buffer: wgpu::Buffer,
    overlay_frame_group: wgpu::BindGroup,
    /// Uniforms of the first-person models that reach the eye (`DELAYED_FOV_EYE`).
    overlay_eye_frame_buffer: wgpu::Buffer,
    overlay_eye_frame_group: wgpu::BindGroup,
    /// bDelayDisplay models with the camera's own projection (`DELAYED`), from `overlay_camera`.
    delayed_frame_buffer: wgpu::Buffer,
    delayed_frame_group: wgpu::BindGroup,
    /// Uniforms of the sky zone view (FSkySceneNode).
    sky_frame_buffer: wgpu::Buffer,
    sky_frame_group: wgpu::BindGroup,
    /// Zones: BSP nodes for PointRegion, fog and sky per zone.
    region_nodes: Vec<openxiii_assets::world::RegionNode>,
    root_outside: bool,
    zones: Vec<openxiii_assets::world::ZoneRender>,
    pass_layout: wgpu::BindGroupLayout,
    pass_buffers: PassBuffers,
    target_pass_buffers: Option<PassBuffers>,
    /// Render textures (side, colour view, depth view) made at load, handed to the script's
    /// first render texture views instead of making them while playing.
    spare_targets: Vec<(u32, wgpu::TextureView, wgpu::TextureView)>,
    /// Entries of `uniforms` already written to `pass_buffer` since the last flush: a new
    /// upload only copies the following ones (the table used to be sent whole after the main
    /// list, the HUD then the shadows).
    passes_written: usize,
    /// Changed entries of upload_passes (reused buffer).
    passes_changed: Vec<bool>,
    /// Comic panel view recorded in the pending encoder: buffers read recorded (`note_read_back`).
    note_read_flag: std::cell::Cell<bool>,
    /// Static scenery textures uploaded to the GPU from the first frame (`warm_up_scenery`).
    scenery_warmup: bool,
    /// Map name used to select the shipped GPU hints.
    map_value: String,
    /// Particle batches per material: resolved texture (replacement generation) and
    /// resources of the last passes (the prepared particles).
    kept_particles: TableFast<String, KeptParticles>,
    /// Matrices actually uploaded to the current view, including portals and split screen.
    particles_projection: std::cell::Cell<Option<(Mat, Option<Mat>)>>,
    stage_layout: wgpu::BindGroupLayout,
    scene_layout: wgpu::PipelineLayout,
    scene_shader: wgpu::ShaderModule,
    impact_shader: wgpu::ShaderModule,
    pipeline_ids: TableFast<PipelineKey, usize>,
    pipelines: Vec<(PipelineKey, wgpu::RenderPipeline)>,
    /// Variants of `pipelines` (same index) at `samples` samples per pixel, for the multisampled
    /// world target (empty without MSAA).
    pipelines_msaa: Vec<Option<wgpu::RenderPipeline>>,
    /// Samples per pixel of the world target (1: no multisampling).
    samples: u32,
    /// Samples per pixel of the target the passes being recorded draw into.
    pass_samples: std::cell::Cell<u32>,
    /// Anisotropy of the world's texture samplers.
    anisotropy: u16,
    /// Image quality settings (render_quality.rs): as requested, what the device allows, the
    /// world's off-screen target and its copy into the frame.
    pub(crate) session_graphics: openxiii_vm::video_settings::SessionGraphics,
    quality: crate::render_quality::RenderQuality,
    quality_caps: crate::render_quality::QualityCaps,
    world_target: Option<crate::render_quality::WorldTarget>,
    compositor: Option<crate::render_quality::Compositor>,
    /// Frame (frame.count) of the first pipeline requested in this level: specialised pipelines
    /// are only created during the first frames (loading, precache).
    first_image: Option<u32>,
    pub fresh_list: ([u32; 3], [f64; 3]),
    world: Geometry,
    /// StaticMeshes drawn actor by actor (cartoon, outline) and the map's lights.
    actors: Vec<openxiii_assets::world::ActorMesh>,
    /// Non-static owning actor of each static geometry batch.
    world_owners: Vec<Option<String>>,
    /// Static geometry batches of each owner, and the last preparation in which the name
    /// was drawn by the view (`world_owner_stamp`).
    world_owner_batches: TableFast<String, Vec<u32>>,
    world_owner_seen: Vec<u32>,
    world_owner_stamp: u32,
    /// StaticMeshes of map actors whose mesh the script changed (build_mesh_replacements):
    /// geometry, and owner of each batch.
    replaced: Option<(Geometry, Vec<Option<String>>)>,
    /// Replaced-meshes buffers while no mesh is replaced (made at load, kept between swaps).
    replaced_spare: Option<Geometry>,
    replaced_reserved: bool,
    /// Load prewarm: pipeline keys asked, made together by flush_pipeline_batch.
    pipeline_batch: Option<Vec<PipelineKey>>,
    /// SinusModifier sections of the map and of the replaced meshes, with their vertices.
    world_sine: Vec<(openxiii_assets::world::SineSection, Vec<SceneVertex>)>,
    replaced_sine: Vec<(openxiii_assets::world::SineSection, Vec<SceneVertex>)>,
    /// rand() of the SinusModifiers' appFrand draws.
    sine_seed: u32,
    /// Non-static static geometry actors hidden or destroyed this frame.
    hidden_owners: std::rc::Rc<std::collections::HashSet<String>>,
    lights: Vec<openxiii_assets::lighting::Light>,
    /// The map's lights at the head of `lights`; the level's dynamic lights follow.
    model_lights: usize,
    dynamic: Option<Dynamic>,
    /// For each animated actor: what USkeletalMeshInstance::Render needs for cartoon
    /// rendering (DrawCartoonSM), and the UPrimitive+0x28 bytes of each mesh.
    cartoon: Vec<Option<openxiii_assets::cartoon::CharacterCartoon>>,
    scene_light_views: crate::lights_scene::Views,
    scene_light_passes: std::rc::Rc<crate::lights_scene::IndexedPasses>,
    active_light_key: Option<crate::lights_scene::DrawKey>,
    /// Particles of the frame: geometry and batches (texture, DrawStyle, IgnoreFog, first index,
    /// index count).
    particles: Option<(Geometry, Vec<ParticleBatch>)>,
    /// Immutable mirror of the frame; each view starts again from the same original ranges.
    particle_indices: Vec<u32>,
    particle_packed_buffer: Option<wgpu::Buffer>,
    particle_packed_active: bool,
    /// Weather (DrawRndCubeSpr): point sprites as quads, texture, mask.
    weather: Option<(Geometry, String, bool, u32)>,
    /// Rain and snow buffers while nothing falls (made at load, kept between showers).
    weather_spare: Option<Geometry>,
    /// Projectors on the BSP: polygons and material key ("#proj…").
    projectors: Option<(Geometry, Vec<ProjectorBatch>)>,
    /// Polygons present in the GPU buffers and triangulation mode, in draw order.
    projector_polys: Vec<(std::rc::Rc<[[f32; 3]]>, bool)>,
    projector_bounds: Vec<Option<target_clip::Bounds>>,
    /// split_projectors: matrices of each batch (texture, gradient, projected, clamped edges).
    projector_rect_infos: Vec<([f32; 16], [f32; 16], bool, bool, bool)>,
    /// Screen rectangle of each projector batch of the main view (key: first index).
    /// (first index, end, rectangle or None = everything) sorted by first index: a draw may
    /// combine several neighbouring batches (impact matrices), its rectangle is their union.
    projector_rects: Vec<(u32, u32, Option<[u32; 4]>)>,
    /// Size of these rectangles' target.
    projector_rects_size: [u32; 2],
    projector_shadows: Vec<u32>,
    /// split_projectors: current view = main window frame (Renderer::draw), not
    /// a portal, a render texture or a split-screen view.
    view_window: bool,
    /// Target of the main frame (view address): the rectangles only apply to it.
    target_window: usize,
    /// Clamped edges (UClampMode and VClampMode at TC_Clamp) per texture.
    textures_blocked: HashMap<String, bool>,
    projector_packets: Vec<ProjectorPacket>,
    impact_arena: impacts::Arena,
    impact_indices_gpu: Vec<u32>,
    impact_matrices_gpu: Option<wgpu::Buffer>,
    /// Current SetZBias (passes pushed by push_pass).
    z_bias: u8,
    /// Passes pushed by push_pass: bIgnoreVignetteAlpha actor set aside.
    push_vignette: bool,
    /// Rank of the added passes (DrawCmd::order).
    push_order: i64,
    /// Sort rank of the terrain's layers in this view (sorted, alpha-blended layers): its projectors are drawn right after it.
    terrain_order: Option<i64>,
    /// For the next commands: `light_sky` forced (projectors of a BSP node) and
    /// shared geometry outside the sky stage.
    push_sky: Option<Option<bool>>,
    push_outside_sky: bool,
    translucent_keys: TranslucentKeys,
    /// World::bsp_node_ranges: index ranges of each batch per BSP node.
    world_node_ranges: Vec<Vec<(u32, u32, u32)>>,
    /// World::terrain_names and World::terrain_ranges.
    world_terrains: (Vec<String>, Vec<(usize, u32, u32)>),
    /// Static geometry actors (names) that have bIgnoreVignetteAlpha.
    vignette_owners: std::rc::Rc<std::collections::HashSet<String>>,
    /// Lighting of the dynamic StaticMeshes, per animated actor.
    prop_lights: Vec<Option<PropLight>>,
    /// Vertex colours of animated StaticMeshes with a StaticMeshInstance: (first vertex,
    /// colours); those of bIgnoreDynLight computed only once, per actor.
    prop_colors: Vec<(usize, Vec<[u8; 4]>)>,
    prop_colors_once: HashMap<String, Vec<[u8; 4]>>,
    /// Trails of the frame (ATrail::Render).
    trails: Vec<openxiii_vm::trail_render::TrailBatch>,
    /// Clear colour of the frame (camera zone: bForceClear, DistanceFogColor).
    clear_color: [u8; 3],
    /// The camera's zone has bForceClear (UGameEngine::Draw then clears to `clear_color`).
    force_clear: bool,
    /// Map indices (copy of its buffer) and, when the engine's visibility sorts the scenery,
    /// buffer of the indices kept this frame with (first index, count) of each batch.
    world_indices: Vec<u32>,
    target_world_bounds: Vec<Option<target_clip::Section>>,
    target_range_cache: target_clip::RangeCache,
    world_culled: Option<(wgpu::Buffer, Vec<(u32, u32)>)>,
    /// Map index buffers where the triangles of the BSP nodes not kept are collapsed
    /// (`set_bsp_visible`): the player's view one, its sky scenes one. None: everything.
    world_masked: Option<(wgpu::Buffer, wgpu::Buffer)>,
    /// For `world_masked` (map, sky): number of non-collapsed triangles before each triangle
    /// of the buffer (one more at the end). A draw whose triangles are all collapsed
    /// produces no pixel: it is skipped, with its state changes.
    world_masked_kept: (Vec<u32>, Vec<u32>),
    /// Bytes of `world_masked` as uploaded (player view, sky); empty: new buffers,
    /// unknown content. `set_bsp_visible` only writes the ranges that differ.
    world_masked_mirror: (Vec<u32>, Vec<u32>),
    /// Same pixels, only the triangles that produce some are uploaded (like the original's dynamic
    /// BSP index buffer).
    world_compact: Option<(wgpu::Buffer, wgpu::Buffer)>,
    // The static geometry's ranges and owners belong to this Renderer.
    last_bsp_visibility: Option<SceneryVisible>,
    /// D3DDrv's eight 256 × 256 shadow targets: colour and depth.
    shadow_targets: Vec<(wgpu::TextureView, wgpu::TextureView)>,
    /// A shadow target could not be made (out of video memory): no more are tried.
    shadow_targets_full: bool,
    /// Side of the shadow targets: 256 as in D3DDrv, larger in the improved build at high
    /// resolutions (`shadow_target_size`).
    shadow_size: u32,
    /// Side asked by the Video menu's Shadows line (improved build), 0 for Auto.
    shadow_quality: u32,
    /// Texels per unit of the render textures (`target_scale`).
    target_scale: u32,
    /// Side of the screen effects' scene texture (`fx_scene_side`).
    fx_side: u32,
    /// Mods build only: generation of the scripts' texture changes on scenery actors already
    /// taken into the world batches' materials.
    #[cfg(feature = "mods")]
    retexture_generation: u64,
    #[cfg(feature = "mods")]
    mod_surfaces: std::sync::Arc<openxiii_vm::mod_retexture::SurfaceEffects>,
    /// Mods build only: materials drawn darker for charred scenery, by original passes and
    /// darkness (the original is kept, so that its address names it).
    #[cfg(feature = "mods")]
    charred_materials: HashMap<(usize, u32), (std::rc::Rc<openxiii_assets::material::MaterialPasses>, std::rc::Rc<openxiii_assets::material::MaterialPasses>)>,
    /// Mods build only: positions of the map buffer's vertices (charred areas of the level).
    #[cfg(feature = "mods")]
    world_positions: Vec<[f32; 3]>,
    /// Mods build only: the level's surfaces in charred areas, by world batch: index ranges and
    /// darkness level (0 to 8), found again when the areas change (their generation).
    #[cfg(feature = "mods")]
    charred_ranges: (u64, HashMap<usize, Vec<(u32, u32, u8)>>),
    /// Mods build only: the darkening pass drawn over charred surfaces, by batch and level.
    #[cfg(feature = "mods")]
    charred_prepared: HashMap<(usize, u8), Option<Box<PreparedSection>>>,
    /// filter_tile only writes alpha (opaque_alpha).
    alpha_only: bool,
    /// Last ordinary filter and full-alpha rectangle of the render texture.
    filter_tiles: [Vec<FilterTileBuffers>; 6],
    /// Texture groups of the filter tiles, by texture.
    filter_texture_groups: HashMap<String, wgpu::BindGroup>,
    shadow_frame_buffer: wgpu::Buffer,
    shadow_frame_group: wgpu::BindGroup,
    /// Frame uniforms of the shadows beyond the first of the frame (one per shadow drawn):
    /// all shadows thus go in a single upload instead of one per shadow.
    shadow_frames_extra: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    frame_layout: wgpu::BindGroupLayout,
    shadow_jobs: Vec<ShadowRender>,
    /// Cartoon light direction of each animated actor (FCartoonDesc+0x44, instance+0x23C).
    cartoon_dirs: TableFast<usize, [f32; 3]>,
    /// Canvas primitives of the frame (UCanvas / FCanvasUtil).
    canvas: Option<(Geometry, Vec<CanvasBatch>)>,
    /// Canvas buffers kept during a frame without primitives.
    canvas_free: Option<Geometry>,
    /// Particle buffers kept during a frame without particles.
    particles_free: Option<Geometry>,
    /// DrawPortals of the frame, in Canvas order.
    portals: Vec<Portal>,
    /// UMeshEmitter meshes and their materials, read once.
    particle_meshes: HashMap<String, Option<(openxiii_assets::static_mesh::StaticMesh, Vec<String>)>>,
    primitive_bytes: HashMap<String, Option<[u8; 8]>>,
    /// Outline thickness multiplier (3 in a render texture).
    outline_boost: f32,
    /// Group of a white texture for colour rectangles (render texture filter).
    white_group: Option<wgpu::BindGroup>,
    materials: Materials,
    /// Textures uploaded to the GPU, by object path (None: unreadable).
    textures: TableFast<String, Option<wgpu::TextureView>>,
    /// Improved build: the menu's scale (screen pixels per menu unit) the onomatopoeias are
    /// wanted at, the scale they are prepared at, and the prepared textures by lower-case name
    /// (width, height, cache key); onomatopoeias.rs.
    onomatopoeia_scale: f32,
    onomatopoeia_prepared_scale: f32,
    onomatopoeia_prepared: TableFast<String, Vec<(u32, u32, String)>>,
    /// Mods and improved builds: picture uploaded for each mod texture name, and the count of
    /// textures made while the game runs last seen (refresh_runtime_textures).
    #[cfg(feature = "mods")]
    mod_uploaded: HashMap<String, Arc<openxiii_assets::mod_textures::Mips>>,
    #[cfg(feature = "mods")]
    mod_generation: u64,
    lightmaps: RuntimeLightmaps,
    white: wgpu::TextureView,
    samplers: TableFast<(u32, u32, u8), wgpu::Sampler>,
    bind_keys: TableFast<BindKey, usize>,
    /// stage_bind: fingerprint of a pass's stages (borrowed names, no copy) -> groups already
    /// made for this fingerprint (rank in bind_groups), full key in `bind_key_list`.
    bind_fingerprints: TableFast<u64, Vec<usize>>,
    bind_key_list: Vec<BindKey>,
    bind_groups: Vec<wgpu::BindGroup>,
    /// Passes of the last frame (recomputed every frame: animated modifiers).
    commands: Vec<DrawCmd>,
    uniforms: FramePasses,
    /// Compact GPU slot → index in `uniforms` (creation order). Merging
    /// and sorting only move these indices, including for the shared BSP passes.
    uniform_sources: Vec<usize>,
    assembly: CommandAssembly,
    // Separate lists for the main view and the last secondary view.
    job_lists: [JobList; 2],
    depth: wgpu::TextureView,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    hud: Hud,
    /// Horizontal field of view (degrees) imposed by the game (FovAngle), in play mode.
    pub fov: Option<f32>,
    /// Off-centre projection of the view (improved build: the character screen's live view moved
    /// in its panel): the picture moved by this in normalised device units (x right, y up).
    pub lens: [f32; 2],
    /// The scene is drawn in a panel of the view (draw_view_in with a scene rectangle): its angle
    /// is the panel's own, never widened by Hor+.
    pub in_panel: bool,
    /// A render texture (comic panel) or a portal is being drawn with its own camera angle.
    in_camera_view: bool,
    /// Local split screen: widest aspect ratio used for the height of the first-person projection
    /// (the whole window's, see `window_aspect`).
    pub overlay_aspect_cap: Option<f32>,
    /// Lockstep co-op: the camera the first-person models (bDelayDisplay) are drawn with, when it
    /// is not the scene's. The scripts place them in front of the simulated camera, while the
    /// scene's camera also turns with the mouse moves not simulated yet (instant look): drawn with
    /// the simulated one they stay where the game put them on the screen.
    pub overlay_camera: Option<Camera>,
    /// The first-person projection of the view being drawn shows more on the sides than a 4:3
    /// window (Hor+, or a split-screen view wider than `overlay_aspect_cap`): `DELAYED_FOV_EYE`
    /// models get the closer near plane (set by `update_uniforms_with`).
    eye_view: std::cell::Cell<bool>,
    /// Split screen: size of the viewport being drawn (the weather's projection is that of its own view).
    pub view_size: Option<[f32; 2]>,
    /// UViewport::SetOnlyPostRender: the scene is not drawn (FCameraSceneNode::Render).
    pub only_post_render: bool,
    /// Improved build: the presented frames are kept (`hold_tex`) so that a blocking load can
    /// keep showing the last one (`hold_start`).
    pub hold_capture: bool,
    /// Frames drawn while the surface is held are drawn off screen (render_to_surface).
    pub warming: bool,
    /// The next frame is drawn off screen and not presented (faithful build, the warm-up frame
    /// after a travel, like the original's precache frame).
    pub hide_next_frame: bool,
    /// Data objects named by the level's VM objects (Game::precache_roots), prepared with the
    /// scenery prewarm: their materials' textures and texture groups.
    precache_roots: Vec<String>,
    /// Textures found by the references prewarm, for their cartoon ramp groups once the first
    /// frame has made the level's ramps (`warm_up_cartoon`).
    precache_textures: Vec<String>,
    cartoon_warmed: bool,
    /// blit_fx_view's quad and uniform group, kept for the same rectangle and window size.
    fx_view_blit: std::cell::RefCell<Option<([u32; 6], wgpu::Buffer, wgpu::BindGroup)>>,
    /// The kept frames (at most HOLD_FRAMES, oldest first): shared with the thread of
    /// `hold_start`, which shows them in turn.
    hold_frames: std::sync::Arc<std::sync::Mutex<Vec<wgpu::Texture>>>,
    /// Improved build, the loading recap: frames kept while `hold_capture_alt` (the loading
    /// animation, made off screen) that `hold_start`'s thread shows instead once `hold_switch`
    /// is set (a press during the level's preparation), and the instant of the first one shown.
    pub hold_capture_alt: bool,
    hold_alt_frames: std::sync::Arc<std::sync::Mutex<Vec<wgpu::Texture>>>,
    pub hold_switch: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub hold_switched_at: std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>,
    /// Vignette of the viewport's PlayerController (after the translucent passes).
    pub vignette: openxiii_vm::visibility::Vignette,
    /// Screen flash of the PlayerController (FlashFog, alpha 1 − FlashScale.X), R, G, B, A.
    pub flash: Option<[u8; 4]>,
    /// DistanceFogColor of the view's volume (bDistanceFog), which replaces that of a
    /// bForceClear zone for clearing (Vm::view_volume_clear), R, G, B.
    pub clear_volume: Option<[u8; 3]>,
    /// Colour filter of the PlayerController (FilterColor other than neutral grey), R, G, B, A.
    pub color_filter: Option<[u8; 4]>,
    /// Apply2DEffect passes (FD3DRenderInterface, vtable +0x94) of the frame; non-empty: the
    /// scene is drawn into the 512 × 512 effects texture, then redrawn by them.
    pub fx_passes: Vec<openxiii_vm::screen_fx::FxPass>,
    /// Screen flash active while drawing the scene (bit 0 of +0xD70): cartoon ramp
    /// 0xFF40.
    pub cartoon_flash: bool,
    fx: Option<crate::screen_fx::ScreenFxRenderer>,
    /// The client's Brightness, Contrast, Gamma when the window is full screen
    /// (UD3DRenderDevice::UpdateGamma changes nothing in windowed mode); `None` when windowed.
    pub gamma: Option<[f32; 3]>,
    gamma_renderer: Option<crate::gamma::GammaRenderer>,
    pub perf_surface: [f64; 4],
    pub perf_send: [f64; 8],
    pub count_send: std::cell::Cell<[u64; 4]>,
    scene_triangles: std::cell::Cell<u64>,
    /// Split screen: Apply2DEffect image of a viewport (its size), copied into its rectangle.
    fx_view_target: Option<(wgpu::Texture, wgpu::TextureView, wgpu::BindGroup, (u32, u32))>,
    /// Engine time (GEngine->CurrentFrameCount, FrameLenght) for the modifiers.
    pub frame: FrameTime,
    /// Fog of the camera's zone (SetDistanceFog): start, end, colour.
    pub fog: Option<(f32, f32, [u8; 3])>,
    /// Fog of the view's volume (Vm::view_volume_fog), which replaces the zone's.
    pub volume_fog: Option<(f32, f32, [u8; 3])>,
    /// Sky zone view to draw before the map.
    sky: Option<Camera>,
    sky_fog: Option<(f32, f32, [u8; 3])>,
    /// bFogPerZone of the camera zone: per-zone fog of the drawn elements.
    fog_per_zone: bool,
    /// Materials without a translation already reported.
    reported: std::collections::HashSet<String>,
    /// Current video (UPCVideoPlayerDevice::Tick): Video_Init's texture and what the
    /// screen shows instead of the game.
    video_texture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    video: Option<VideoShow>,
}

/// Last Present of the video player: the image (texture coordinates of its bottom-right corner) or
/// the black screen at the end.
#[derive(Debug, Clone, Copy)]
enum VideoShow {
    Frame([f32; 2]),
    Black,
}

/// Where a `video_w` × `video_h` video goes on a `width` × `height` screen, in pixels
/// (left, top, right, bottom): as large as possible with its own proportions, centred, touching
/// two opposite edges of the screen. A screen of the video's shape (4:3 for the game's videos)
/// gets the whole screen, like the original's stretched quad.
fn video_rect(width: u32, height: u32, video_w: u32, video_h: u32) -> [f32; 4] {
    let (w, h) = (width as u64, height as u64);
    if video_w == 0 || video_h == 0 || w == 0 || h == 0 {
        return [0.0, 0.0, width as f32, height as f32];
    }
    let (vw, vh) = (video_w as u64, video_h as u64);
    if w * vh > h * vw {
        // Wider than the video: full height, bars on the left and right.
        let inner = ((h * vw + vh / 2) / vh).min(w);
        let x0 = (w - inner) / 2;
        [x0 as f32, 0.0, (x0 + inner) as f32, height as f32]
    } else {
        // Narrower (or the same shape): full width, bars above and below when needed.
        let inner = ((w * vh + vw / 2) / vw).min(h);
        let y0 = (h - inner) / 2;
        [0.0, y0 as f32, width as f32, (y0 + inner) as f32]
    }
}

#[cfg(test)]
mod video_rect_tests {
    use super::video_rect;

    #[test]
    fn four_three_screens_keep_the_whole_screen() {
        for (w, h) in [(640, 480), (800, 600), (1024, 768), (1280, 960), (1600, 1200), (2048, 1536)] {
            assert_eq!(video_rect(w, h, 640, 480), [0.0, 0.0, w as f32, h as f32], "{w}x{h}");
        }
    }

    #[test]
    fn other_shapes_keep_the_video_proportions() {
        // (screen, expected rectangle): bars only on the two sides that need them.
        let cases = [
            ((1920, 1080), [240.0, 0.0, 1680.0, 1080.0]),
            ((1280, 720), [160.0, 0.0, 1120.0, 720.0]),
            ((2560, 1080), [560.0, 0.0, 2000.0, 1080.0]),
            ((3440, 1440), [760.0, 0.0, 2680.0, 1440.0]),
            ((5120, 1440), [1600.0, 0.0, 3520.0, 1440.0]),
            ((2400, 1080), [480.0, 0.0, 1920.0, 1080.0]),
            ((1280, 1024), [0.0, 32.0, 1280.0, 992.0]),
            ((1080, 2400), [0.0, 795.0, 1080.0, 1605.0]),
        ];
        for ((w, h), want) in cases {
            let r = video_rect(w, h, 640, 480);
            assert_eq!(r, want, "{w}x{h}");
            // Touches two opposite edges, never a frame all around.
            assert!((r[0] == 0.0 && r[2] == w as f32) || (r[1] == 0.0 && r[3] == h as f32), "{w}x{h}");
        }
    }

    #[test]
    fn many_sizes_fit_without_distortion() {
        let mut n = 0;
        for w in (200..4000).step_by(97) {
            for h in (200..3000).step_by(89) {
                let [x0, y0, x1, y1] = video_rect(w, h, 640, 480);
                assert!(x0 >= 0.0 && y0 >= 0.0 && x1 <= w as f32 && y1 <= h as f32);
                assert!(x0 == 0.0 || y0 == 0.0);
                // Centred, and 4:3 within one pixel of rounding.
                assert!(((x0 - (w as f32 - x1)).abs() <= 1.0) && ((y0 - (h as f32 - y1)).abs() <= 1.0));
                let (rw, rh) = (x1 - x0, y1 - y0);
                assert!((rw * 3.0 - rh * 4.0).abs() <= 4.0, "{w}x{h}: {rw}x{rh}");
                n += 1;
            }
        }
        assert!(n > 1000);
    }
}

/// 8-bit RGBA texture with its mip levels (values taken as is, like Direct3D 8).
/// Initial contents of the 1024 × 512 RGBA video texture (zeros, as wgpu's own clear would give).
static VIDEO_BLACK: [u8; 4 * 1024 * 512] = [0; 4 * 1024 * 512];

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, mips: &[(u32, u32, Vec<u8>)]) -> wgpu::TextureView {
    upload_texture(device, queue, mips).create_view(&wgpu::TextureViewDescriptor::default())
}

fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, mips: &[(u32, u32, Vec<u8>)]) -> wgpu::Texture {
    let _ch = openxiii_package::loadings::start_value(openxiii_package::loadings::TEXTURE_SENT);
    let size = wgpu::Extent3d { width: mips[0].0, height: mips[0].1, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size,
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (w, h, rgba)) in mips.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(*h) },
            wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
        );
    }
    texture
}

/// FLightManager::Cache lightmaps: texture, state kept by the engine, and what is needed to
/// redo FLight::Init and Cache's second pass every frame.
struct RuntimeLightmaps {
    maps: Vec<(wgpu::Texture, openxiii_assets::lightmap::SurfaceLightmap)>,
    names: Vec<String>,
    model: Option<std::sync::Arc<openxiii_assets::model::Model>>,
    records: Vec<openxiii_assets::model::LightRecord>,
    brightness: f32,
    /// appFrand generator (LT_Flicker lights): fixed sequence, for stable screenshots.
    seed: u32,
    /// GLevel->TimeSeconds.
    time: f64,
}

impl RuntimeLightmaps {
    /// FLight::Init of every light at the current time, then the lightmaps touched by a
    /// dynamic or animated light are completed and uploaded to the GPU again.
    fn update(&mut self, queue: &wgpu::Queue) {
        let Some(model) = &self.model else { return };
        if self.maps.iter().all(|(_, m)| m.dynamic.is_empty()) {
            return;
        }
        let seed = &mut self.seed;
        let mut frand = || {
            *seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
            ((*seed >> 16) & 0x7FFF) as f32 / 32767.0
        };
        let lights: Vec<_> = self.records.iter().map(|r| openxiii_assets::lighting::init_light(r, self.brightness, self.time, &mut frand)).collect();
        // Unordered texture writes: the pending views (comic panels) are sent first,
        // with the previous frame's lightmaps as before.
        empty_order(queue);
        for (texture, map) in &self.maps {
            if map.dynamic.is_empty() {
                continue;
            }
            let rgba = map.texture_data(model, &lights);
            let [w, h] = map.texture;
            count_write(1, rgba.len() as u64);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
    }
}

/// Images of a BitmapMaterial as D3DDrv uploads them (Texture only).
fn bitmap_mips(library: &mut Library, path: &str) -> Option<Vec<(u32, u32, Vec<u8>)>> {
    // Mods and improved builds: a mod's texture for this name, or one made while the game runs
    // (the menus' pictures) (openxiii_assets::mod_textures).
    #[cfg(feature = "mods")]
    if let Some(m) = openxiii_assets::mod_textures::get(path) {
        return Some((*m).clone());
    }
    let (loaded, index) = library.resolve(path)?;
    let texture = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).ok()?;
    let mips = texture.d3d_mips();
    (!mips.is_empty()).then_some(mips)
}

/// Side of D3DDrv's shadow targets.
const SHADOW_TARGET_SIZE: u32 = 256;

/// D3DDrv's shadow targets (colour and depth) of `size` × `size`: `count` of them (eight, more in split screen).
fn shadow_targets(device: &wgpu::Device, color_format: wgpu::TextureFormat, size: u32, count: usize) -> Vec<(wgpu::TextureView, wgpu::TextureView)> {
    (0..count).map(|_| shadow_target(device, color_format, size)).collect()
}

#[cfg(test)]
#[test]
fn split_screen_shadow_targets_exist_before_their_first_silhouette() {
    let Ok(game) = openxiii_package::env::var("OXIII_GAME") else { eprintln!("skipped: needs the game's files (OXIII_GAME)"); return; };
    let mut library = Library::new(Path::new(&game)).unwrap();
    let mut r = pollster::block_on(Renderer::new_headless(16, 16, &World::default(), &mut library)).expect("headless device required");
    assert_eq!(r.shadow_targets.len(), openxiii_vm::projector::SHADOW_SLOTS);
    // A projection on the ninth and twelfth targets prepared before any silhouette is drawn
    // (the bind group is made then and kept): real targets, not the white fallback.
    for i in [8, 11] {
        assert!(r.texture(&mut library, &format!("#shadowtex:{i}:7")).is_some(), "target {i}");
    }
    assert_eq!(r.shadow_targets.len(), 12);
    let key: BindKey = std::array::from_fn(|stage| (Some(format!("#shadowtex:{}", 12 + stage % 2)), 3, 3, SAMPLER_WORLD));
    r.bind_group(&mut library, key);
    assert_eq!(r.shadow_targets.len(), 14);
    // No fixed bound: the fortieth target is made when a shadow takes it.
    assert!(r.texture(&mut library, "#shadowtex:39").is_some());
    assert_eq!(r.shadow_targets.len(), 40);
    // Out of video memory: no more targets, the ones made are kept and the VM stops there.
    r.shadow_targets_full = true;
    assert!(r.texture(&mut library, "#shadowtex:40").is_none());
    assert_eq!(r.shadow_targets.len(), 40);
}

/// One of D3DDrv's shadow targets (colour and depth) of `size` × `size`.
fn shadow_target(device: &wgpu::Device, color_format: wgpu::TextureFormat, size: u32) -> (wgpu::TextureView, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: color_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    (t.create_view(&wgpu::TextureViewDescriptor::default()), depth_view(device, size, size))
}

fn depth_view(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn uniform_layout(device: &wgpu::Device, label: &str, dynamic: bool) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        }],
    })
}

/// Like `geometry`, reusing the buffers of `old` if they are large enough (written by
/// write_buffer; the end of a larger buffer is read by no index).
fn geometry_kept(device: &wgpu::Device, queue: &wgpu::Queue, old: Option<Geometry>, vertices: &[SceneVertex], indices: &[u32], label: &str) -> Geometry {
    let (v, i): (&[u8], &[u8]) = (bytemuck::cast_slice(vertices), bytemuck::cast_slice(indices));
    let buffer_value = |b: Option<wgpu::Buffer>, content: &[u8], usage: wgpu::BufferUsages| match b {
        Some(b) if b.size() >= content.len() as u64 => {
            if !content.is_empty() {
                write_value(queue, &b, 0, content);
            }
            b
        }
        _ => {
            let size = (content.len() as u64 * 3 / 2).max(64).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
            let b = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            if !content.is_empty() {
                write_value(queue, &b, 0, content);
            }
            b
        }
    };
    let (ov, oi) = old.map(|g| (g.vertex_buffer, g.index_buffer)).unzip();
    Geometry {
        vertex_buffer: buffer_value(ov, v, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST),
        index_buffer: buffer_value(oi, i, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST),
        batches: Vec::new(),
    }
}

/// Room of the buffers made at load for per-frame geometry (bytes). Well above the largest frame
/// seen over every map start and checkpoint (Canvas 81 KB / 11 KB, particles 125 KB / 18 KB).
const CANVAS_VERTEX_BYTES: u64 = 256 << 10;
const CANVAS_INDEX_BYTES: u64 = 64 << 10;
const PARTICLE_VERTEX_BYTES: u64 = 512 << 10;
const PARTICLE_INDEX_BYTES: u64 = 128 << 10;

/// Empty vertex and index buffers of the given sizes, filled later by `geometry_kept`.
fn dynamic_geometry(device: &wgpu::Device, label: &str, vertex_bytes: u64, index_bytes: u64) -> Geometry {
    let buffer = |size: u64, usage: wgpu::BufferUsages| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    Geometry { vertex_buffer: buffer(vertex_bytes, wgpu::BufferUsages::VERTEX), index_buffer: buffer(index_bytes, wgpu::BufferUsages::INDEX), batches: Vec::new() }
}

fn geometry(device: &wgpu::Device, vertices: &[SceneVertex], indices: &[u32], batches: Vec<SceneBatch>, label: &str) -> Geometry {
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: if vertices.is_empty() { &[0u8; 44] } else { bytemuck::cast_slice(vertices) },
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: if indices.is_empty() { &[0u8; 4] } else { bytemuck::cast_slice(indices) },
        usage: wgpu::BufferUsages::INDEX,
    });
    Geometry { vertex_buffer, index_buffer, batches }
}

/// SinusModifier sections of `world` and their vertices (slices of `vertices`).
fn sine_sections(world: &World, vertices: &[SceneVertex]) -> Vec<(openxiii_assets::world::SineSection, Vec<SceneVertex>)> {
    world.sine.iter().map(|s| (s.clone(), vertices[s.base as usize..s.base as usize + s.local.len()].to_vec())).collect()
}

fn scene_vertices(world: &World) -> Vec<SceneVertex> {
    world
        .vertices
        .iter()
        .enumerate()
        .map(|(i, v)| SceneVertex {
            position: v.position,
            normal: v.normal,
            color: world.colors.get(i).copied().unwrap_or([255; 4]),
            uv0: v.uv,
            uv1: world.uv2.get(i).copied().unwrap_or([0.0; 2]),
        })
        .collect()
}

impl Renderer {
    #[cfg(feature = "mods")]
    pub(crate) fn use_mod_surface_effects(&mut self, effects: &std::sync::Arc<openxiii_vm::mod_retexture::SurfaceEffects>) {
        if !std::sync::Arc::ptr_eq(&self.mod_surfaces, effects) {
            // Independent maps may have reached the same generation with different materials.
            if self.retexture_generation != 0 || effects.generation() != 0 {
                self.retexture_generation = u64::MAX;
            }
            if self.charred_ranges.0 != 0 || effects.char_areas_generation() != 0 {
                self.charred_ranges.0 = u64::MAX;
            }
            self.mod_surfaces = effects.clone();
        }
    }

    /// Mods build only: the triangles of the level's own surfaces (batches without an owner
    /// actor) whose centre is in a charred area, as index ranges by batch with their darkness
    /// level (1 to 8). Found again only when the areas change.
    #[cfg(feature = "mods")]
    fn refresh_charred_ranges(&mut self) {
        let generation = self.mod_surfaces.char_areas_generation();
        if generation == self.charred_ranges.0 {
            return;
        }
        let areas = self.mod_surfaces.char_areas();
        let mut out: HashMap<usize, Vec<(u32, u32, u8)>> = HashMap::new();
        if !areas.is_empty() {
            for (i, b) in self.world.batches.iter().enumerate() {
                if self.world_owners.get(i).is_some_and(Option::is_some) {
                    continue;
                }
                let first = b.first_index as usize;
                let end = first + b.index_count as usize;
                let Some(indices) = self.world_indices.get(first..end) else { continue };
                let mut ranges: Vec<(u32, u32, u8)> = Vec::new();
                for (t, tri) in indices.chunks_exact(3).enumerate() {
                    let mut c = [0.0f32; 3];
                    for &v in tri {
                        let p = self.world_positions.get(v as usize).copied().unwrap_or_default();
                        for k in 0..3 {
                            c[k] += p[k] / 3.0;
                        }
                    }
                    // Full in the inner half of an area, then lighter towards its edge: no
                    // straight frontier between burnt and unburnt surfaces, only steps.
                    let k = areas
                        .iter()
                        .map(|(at, r, k)| {
                            let d = ((c[0] - at[0]).powi(2) + (c[1] - at[1]).powi(2) + (c[2] - at[2]).powi(2)).sqrt();
                            k * ((r - d) / (0.5 * r)).clamp(0.0, 1.0)
                        })
                        .fold(0.0f32, f32::max);
                    let level = (k * 8.0).round() as u8;
                    if level == 0 {
                        continue;
                    }
                    let at = (first + t * 3) as u32;
                    match ranges.last_mut() {
                        Some(r) if r.2 == level && r.0 + r.1 == at => r.1 += 3,
                        _ => ranges.push((at, 3, level)),
                    }
                }
                if !ranges.is_empty() {
                    out.insert(i, ranges);
                }
            }
        }
        self.charred_ranges = (generation, out);
        self.charred_prepared.clear();
    }

    /// Mods build only: the pass that darkens what is already drawn (the colour below times a
    /// grey; no depth written, fog leaves it unchanged), `level` from 1 (a little) to 8.
    #[cfg(feature = "mods")]
    fn charred_overlay_material(level: u8) -> std::rc::Rc<openxiii_assets::material::MaterialPasses> {
        thread_local! {
            static MADE: std::cell::RefCell<HashMap<u8, std::rc::Rc<openxiii_assets::material::MaterialPasses>>> = std::cell::RefCell::new(HashMap::new());
        }
        MADE.with(|m| m.borrow_mut().entry(level).or_insert_with(|| Self::make_charred_overlay(level)).clone())
    }

    #[cfg(feature = "mods")]
    fn make_charred_overlay(level: u8) -> std::rc::Rc<openxiii_assets::material::MaterialPasses> {
        use openxiii_assets::material::d3d as m;
        let grey = (255.0 * (1.0 - 0.88 * level as f32 / 8.0)).round() as u32;
        let mut p = openxiii_assets::material::Pass {
            alpha_blending: true,
            z_write: false,
            src_blend: m::BLEND_ZERO,
            dest_blend: m::BLEND_SRCCOLOR,
            override_fog_color: true,
            fog_color: 0x00FF_FFFF,
            tfactor: 0xFF00_0000 | grey << 16 | grey << 8 | grey,
            stages_used: 1,
            ..Default::default()
        };
        p.stages[0].color_op = m::TOP_SELECTARG1;
        p.stages[0].color_arg1 = m::TA_TFACTOR;
        p.stages[0].alpha_op = m::TOP_SELECTARG1;
        p.stages[0].alpha_arg1 = m::TA_TFACTOR;
        std::rc::Rc::new(openxiii_assets::material::MaterialPasses { passes: vec![p], requires_sorting: false, error: None })
    }

    /// Mods build only: `passes` drawn charred (k from 0 to 1): each pass's colour multiplied
    /// by a dark grey in one more texture stage (its alpha unchanged), kept for the next frames.
    #[cfg(feature = "mods")]
    fn charred_material(&mut self, passes: &std::rc::Rc<openxiii_assets::material::MaterialPasses>, k: f32) -> std::rc::Rc<openxiii_assets::material::MaterialPasses> {
        use openxiii_assets::material::d3d as m;
        let level = (k.clamp(0.0, 1.0) * 32.0).round() as u32;
        let key = (std::rc::Rc::as_ptr(passes) as usize, level);
        if let Some((original, dark)) = self.charred_materials.get(&key) {
            if std::rc::Rc::ptr_eq(original, passes) {
                return dark.clone();
            }
        }
        let grey = (255.0 * (1.0 - 0.9 * level as f32 / 32.0)).round() as u32;
        let tint = 0xFF00_0000 | grey << 16 | grey << 8 | grey;
        let mut dark = (**passes).clone();
        for p in dark.passes.iter_mut() {
            let n = p.stages_used;
            let uses_tfactor = p.stages[..n].iter().any(|st| {
                [st.color_arg0, st.color_arg1, st.color_arg2, st.alpha_arg0, st.alpha_arg1, st.alpha_arg2].iter().any(|a| a & 0xF == m::TA_TFACTOR)
            });
            if n == 0 || n >= openxiii_assets::material::STAGES || uses_tfactor {
                continue;
            }
            let st = &mut p.stages[n];
            *st = Default::default();
            st.color_op = m::TOP_MODULATE;
            st.color_arg1 = m::TA_CURRENT;
            st.color_arg2 = m::TA_TFACTOR;
            st.alpha_op = m::TOP_SELECTARG1;
            st.alpha_arg1 = m::TA_CURRENT;
            p.stages_used = n + 1;
            p.tfactor = tint;
        }
        let dark = std::rc::Rc::new(dark);
        if self.charred_materials.len() > 4096 {
            self.charred_materials.clear();
        }
        self.charred_materials.insert(key, (passes.clone(), dark.clone()));
        dark
    }

    /// UEngine::Flush / UClient::Flush: invalidates the resources cached by
    /// the renderer. The scene's source data and the live lightmaps stay
    /// owned by the Renderer; only their cached views are rebuilt.
    pub fn consume_engine_flush(&mut self, vm: &mut openxiii_vm::Vm) -> bool {
        if !std::mem::take(&mut vm.pending_renderer_flush) {
            return false;
        }
        self.commands.clear();
        self.uniforms.clear();
        self.uniform_sources.clear();
        self.passes_written = 0;
        self.bind_keys.clear();
        self.bind_fingerprints.clear();
        self.bind_key_list.clear();
        self.bind_groups.clear();
        self.kept_particles.clear();
        self.samplers.clear();
        // The flush changes the numbers the sections kept for the GPU resources.
        for b in self.world.batches.iter_mut()
            .chain(self.replaced.iter_mut().flat_map(|(g, _)| &mut g.batches))
            .chain(self.dynamic.iter_mut().flat_map(|d| &mut d.geometry.batches)) {
            b.prepared = None;
        }
        self.projector_packets.clear();
        self.pipeline_ids.clear();
        self.pipelines.clear();
        self.pipelines_msaa.clear();
        self.white_group = None;
        self.filter_tiles = Default::default();
        self.filter_texture_groups.clear();
        self.target_pass_buffers = None;
        self.textures.clear();
        for ((texture, _), name) in self.lightmaps.maps.iter().zip(&self.lightmaps.names) {
            self.textures.insert(name.clone(), Some(texture.create_view(&wgpu::TextureViewDescriptor::default())));
        }
        // UEngine::Flush sets PrecacheOnFlip (RenDev+0x64): the next frame is a precache frame
        // that uploads the level again (UGameEngine::Draw). Same here: the load
        // prewarm runs again on the next frame drawn, with the level's references.
        self.scenery_warmup = false;
        self.cartoon_warmed = false;
        self.precache_roots.clear();
        true
    }

    pub async fn new_windowed(window: Arc<Window>, world: &World, library: &mut Library) -> Result<Self, String> {
        // WGPU_BACKEND (dx12, vulkan, gl) and WGPU_ADAPTER_NAME choose the API and the adapter.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())));
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let (adapter, device, queue) = open_adapter(&instance, Some(&surface), true).await?;
        #[cfg(target_os = "android")]
        println!("render: {:?} ({})", adapter.get_info().backend, adapter.get_info().name);
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // Direct3D 8 has no sRGB space: colours go to the screen as is.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        // COPY_SRC for the gamma ramp (gamma.rs) when the surface allows it.
        // Improved build: COPY_DST for the held frame shown again during a load (hold_start).
        let held = held_frame_usage();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | (caps.usages & (wgpu::TextureUsages::COPY_SRC | held));
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            // V-sync of the Video menu, or OXIII_VSYNC when set; set_vsync follows later changes.
            present_mode: present_mode(openxiii_vm::video_settings::vsync_mode()),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        diag(&format!("window {}×{}, format {:?}, presentation {:?}", config.width, config.height, config.format, config.present_mode));
        diag(&vsync_line(openxiii_vm::video_settings::vsync_mode(), config.present_mode));
        let caps = crate::render_quality::QualityCaps::new(&adapter, &device, format);
        let mut r = Self::build(device, queue, format, config.width, config.height, world, library);
        r.set_quality_caps(caps);
        r.generic_precompile();
        // Direct3D 11 passed opening, the surface and pipeline compilation: success.
        test_d3d11(&adapter, false, true);
        r.surface = Some((surface, config));
        r.instance = Some(instance);
        Ok(r)
    }

    /// Android: the native surface is destroyed when going to the background (suspended).
    #[allow(dead_code)]
    pub fn drop_surface(&mut self) {
        if let Some((surface, config)) = self.surface.take() {
            drop(surface);
            self.surface_lost = Some(config);
        }
    }

    /// Android: new surface on the new native window on return (resumed), same
    /// format; the size is that of the window.
    #[allow(dead_code)]
    pub fn recreate_surface(&mut self, window: Arc<Window>) -> Result<(), String> {
        let (Some(instance), Some(mut config)) = (&self.instance, self.surface_lost.take()) else { return Ok(()) };
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let size = window.inner_size();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        surface.configure(&self.device, &config);
        self.surface = Some((surface, config));
        self.resize(size.width, size.height);
        Ok(())
    }

    /// New map on the same surface and device (travel): the window keeps the
    /// last frame presented (loading screen) during loading, instead of being
    /// cleared to white between the destruction of the old surface and the new one.
    pub fn rebuild(mut self, world: &World, library: &mut Library) -> Self {
        let surface = self.surface.take();
        let (w, h) = surface.as_ref().map_or((self.width, self.height), |(_, c)| (c.width, c.height));
        let mut r = Self::build(self.device.clone(), self.queue.clone(), self.color_format, w, h, world, library);
        // Pipelines kept from one map to the next (same device, same format): their OpenGL
        // programs are not recompiled on the next load.
        r.stage_layout = self.stage_layout.clone();
        r.scene_layout = self.scene_layout.clone();
        r.scene_shader = self.scene_shader.clone();
        r.pipelines = std::mem::take(&mut self.pipelines);
        r.pipeline_ids = std::mem::take(&mut self.pipeline_ids);
        r.pipelines_msaa = std::mem::take(&mut self.pipelines_msaa);
        r.samples = self.samples;
        r.anisotropy = self.anisotropy;
        r.quality = self.quality;
        r.quality_caps = self.quality_caps.clone();
        r.hud.msaa = self.hud.msaa.take();
        r.compositor = self.compositor.take();
        r.surface = surface;
        r.instance = self.instance.take();
        r.surface_lost = self.surface_lost.take();
        r
    }

    pub async fn new_headless(width: u32, height: u32, world: &World, library: &mut Library) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let (adapter, device, queue) = open_adapter(&instance, None, false).await?;
        let caps = crate::render_quality::QualityCaps::new(&adapter, &device, wgpu::TextureFormat::Rgba8Unorm);
        let mut r = Self::build(device, queue, wgpu::TextureFormat::Rgba8Unorm, width, height, world, library);
        r.set_quality_caps(caps);
        r.generic_precompile();
        Ok(r)
    }

    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        world: &World,
        library: &mut Library,
    ) -> Self {
        let _ = library;
        let batches = world
            .batches
            .iter()
            .enumerate()
            .map(|(i, b)| SceneBatch {
                first_index: b.first_index,
                index_count: b.index_count,
                material: (!b.material.is_empty()).then(|| b.material.clone()),
                state: world.states.get(i).cloned().unwrap_or_default(),
                bound: Default::default(),
                prepared: None,
            })
            .collect();
        // FLightManager::Cache lightmaps: first computation at time 0, animated lights off.
        let mut runtime_lightmaps: HashMap<String, Option<wgpu::TextureView>> = HashMap::new();
        let mut lightmaps = RuntimeLightmaps {
            maps: Vec::new(),
            names: Vec::new(),
            model: world.model.clone(),
            records: world.light_records.clone(),
            brightness: world.level_brightness,
            seed: 1,
            time: 0.0,
        };
        if let Some(model) = &world.model {
            for (name, map) in &world.runtime_lightmaps {
                let [w, h] = map.texture;
                let texture = upload_texture(&device, &queue, &[(w, h, map.texture_data(model, &world.lights))]);
                runtime_lightmaps.insert(name.clone(), Some(texture.create_view(&wgpu::TextureViewDescriptor::default())));
                lightmaps.maps.push((texture, map.clone()));
                lightmaps.names.push(name.clone());
            }
        }
        #[allow(unused_mut)]
        let mut world_vertices = scene_vertices(world);
        let world_sine = sine_sections(world, &world_vertices);
        let world_geometry = geometry(&device, &world_vertices, &world.indices, batches, "map");
        let frame_layout = uniform_layout(&device, "image", false);
        let pass_layout = uniform_layout(&device, "pass", true);
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buffer.as_entire_binding() }],
        });
        let overlay_frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("first-person view frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overlay_frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("first-person view frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: overlay_frame_buffer.as_entire_binding() }],
        });
        let overlay_eye_frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("first-person view frame (eye)"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let overlay_eye_frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("first-person view frame (eye)"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: overlay_eye_frame_buffer.as_entire_binding() }],
        });
        let delayed_frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("first-person view frame (camera projection)"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let delayed_frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("first-person view frame (camera projection)"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: delayed_frame_buffer.as_entire_binding() }],
        });
        let sky_frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_targets = shadow_targets(&device, color_format, SHADOW_TARGET_SIZE, openxiii_vm::projector::SHADOW_SLOTS);
        let shadow_frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_frame_buffer.as_entire_binding() }],
        });
        let sky_frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: sky_frame_buffer.as_entire_binding() }],
        });
        let pass_capacity = 1024;
        let pass_buffers = Self::pass_buffer(&device, &pass_layout, pass_capacity);
        let mut entries = Vec::new();
        for i in 0..STAGES as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        for i in 0..STAGES as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: STAGES as u32 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let stage_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("stages"), entries: &entries });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&frame_layout), Some(&pass_layout), Some(&stage_layout)],
            immediate_size: 0,
        });
        let scene_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("d3d8"), source: wgpu::ShaderSource::Wgsl(d3d::main_shader()) });
        let white = upload(&device, &queue, &[(1, 1, vec![255; 4])]);
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let depth = depth_view(&device, width, height);
        let hud = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("hud"), source: wgpu::ShaderSource::Wgsl(HUD_SHADER.into()) });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("hud"),
                bind_group_layouts: &[Some(&frame_layout), Some(&texture_layout)],
                immediate_size: 0,
            });
            let [pipeline, alpha_pipeline, modulate_pipeline] = hud_pipelines(&device, &shader, &layout, color_format, 1);
            Hud { pipeline, modulate_pipeline, alpha_pipeline, shader, layout, msaa: None, targets: Default::default() }
        };
        let impact_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("impacts"), source: wgpu::ShaderSource::Wgsl(d3d::IMPACT_SHADER.into()),
        });
        // Canvas and particle buffers made at load with room for any frame, so that they never
        // grow (a new buffer) while playing: the original fills fixed dynamic buffers.
        let canvas_reserve = dynamic_geometry(&device, "canvas", CANVAS_VERTEX_BYTES, CANVAS_INDEX_BYTES);
        let particles_reserve = dynamic_geometry(&device, "particles", PARTICLE_VERTEX_BYTES, PARTICLE_INDEX_BYTES);
        let packed_reserve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles: indices in pass order"),
            size: PARTICLE_INDEX_BYTES,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Renderer {
            _batch: KeptLot::register(&device, &queue),
            device,
            queue,
            surface: None,
            instance: None,
            surface_lost: None,
            color_format,
            width,
            height,

            frame_buffer,
            frame_group,
            overlay_frame_buffer,
            overlay_frame_group,
            overlay_eye_frame_buffer,
            overlay_eye_frame_group,
            delayed_frame_buffer,
            delayed_frame_group,
            sky_frame_buffer,
            sky_frame_group,
            region_nodes: world.region_nodes.clone(),
            root_outside: world.root_outside,
            zones: world.zones.clone(),
            pass_layout,
            pass_buffers,
            target_pass_buffers: None,
            spare_targets: Vec::new(),
            passes_written: 0,
            passes_changed: Vec::new(),
            note_read_flag: std::cell::Cell::new(false),
            scenery_warmup: false,
            map_value: world.map_value.to_ascii_lowercase(),
            kept_particles: TableFast::default(),
            particles_projection: std::cell::Cell::new(None),
            stage_layout,
            scene_layout,
            scene_shader,
            impact_shader,
            pipeline_ids: Default::default(),
            pipelines: Vec::new(),
            pipelines_msaa: Vec::new(),
            samples: 1,
            pass_samples: std::cell::Cell::new(1),
            anisotropy: 1,
            session_graphics: Default::default(),
            quality: Default::default(),
            quality_caps: Default::default(),
            world_target: None,
            compositor: None,
            fresh_list: Default::default(),
            first_image: None,
            world: world_geometry,
            actors: world.actor_meshes.clone(),
            world_owners: world.batch_owners.clone(),
            world_owner_batches: {
                let mut t = TableFast::<String, Vec<u32>>::default();
                for (i, o) in world.batch_owners.iter().enumerate() {
                    if let Some(o) = o {
                        t.entry(o.clone()).or_default().push(i as u32);
                    }
                }
                t
            },
            world_owner_seen: Vec::new(),
            world_owner_stamp: 0,
            replaced: None,
            replaced_spare: None,
            replaced_reserved: false,
            pipeline_batch: None,
            world_sine,
            replaced_sine: Vec::new(),
            sine_seed: 1,
            hidden_owners: Default::default(),
            lights: world.lights.clone(),
            model_lights: world.lights.len(),
            dynamic: None,
            cartoon: Vec::new(),
            scene_light_views: Default::default(),
            scene_light_passes: Default::default(),
            active_light_key: None,
            particles: None,
            particle_indices: Vec::new(),
            particle_packed_buffer: Some(packed_reserve),
            particle_packed_active: false,
            weather: None,
            weather_spare: None,
            projectors: None,
            projector_polys: Vec::new(),
            projector_bounds: Vec::new(),
            projector_rect_infos: Vec::new(),
            projector_rects: Vec::new(),
            projector_rects_size: [0, 0],
            projector_shadows: Vec::new(),
            view_window: false,
            target_window: 0,
            textures_blocked: HashMap::new(),
            projector_packets: Vec::new(),
            impact_arena: Default::default(),
            impact_indices_gpu: Vec::new(),
            impact_matrices_gpu: None,
            z_bias: 0,
            push_vignette: false,
            push_order: i64::MAX,
            terrain_order: None,
            push_sky: None,
            push_outside_sky: false,
            translucent_keys: TranslucentKeys::default(),
            world_node_ranges: world.bsp_node_ranges.clone(),
            world_terrains: (world.terrain_names.clone(), world.terrain_ranges.clone()),
            vignette_owners: Default::default(),
            prop_lights: Vec::new(),
            prop_colors: Vec::new(),
            prop_colors_once: HashMap::new(),
            trails: Vec::new(),
            clear_color: [0; 3],
            force_clear: false,
            world_indices: world.indices.clone(),
            target_world_bounds: target_clip::world_bounds(world),
            target_range_cache: Default::default(),
            world_culled: None,
            world_masked: None,
            world_masked_kept: (Vec::new(), Vec::new()),
            world_masked_mirror: (Vec::new(), Vec::new()),
            world_compact: None,
            last_bsp_visibility: None,
            shadow_targets,
            shadow_targets_full: false,
            shadow_size: SHADOW_TARGET_SIZE,
            shadow_quality: 0,
            target_scale: 1,
            fx_side: crate::screen_fx::SIDE,
            #[cfg(feature = "mods")]
            retexture_generation: 0,
            #[cfg(feature = "mods")]
            mod_surfaces: Default::default(),
            #[cfg(feature = "mods")]
            charred_materials: HashMap::new(),
            #[cfg(feature = "mods")]
            world_positions: world.vertices.iter().map(|v| v.position).collect(),
            #[cfg(feature = "mods")]
            charred_ranges: (0, HashMap::new()),
            #[cfg(feature = "mods")]
            charred_prepared: HashMap::new(),
            alpha_only: false,
            filter_tiles: Default::default(),
            filter_texture_groups: HashMap::new(),
            shadow_frame_buffer,
            shadow_frame_group,
            shadow_frames_extra: Vec::new(),
            frame_layout: frame_layout.clone(),
            shadow_jobs: Vec::new(),
            cartoon_dirs: Default::default(),
            canvas: None,
            canvas_free: Some(canvas_reserve),
            particles_free: Some(particles_reserve),
            portals: Vec::new(),
            particle_meshes: HashMap::new(),
            primitive_bytes: HashMap::new(),
            outline_boost: 1.0,
            white_group: None,
            video_texture: None,
            video: None,
            materials: Materials::new(),
            // FLightManager::Cache lightmaps, computed when the map is loaded.
            textures: runtime_lightmaps.into_iter().collect(),
            onomatopoeia_scale: 0.0,
            onomatopoeia_prepared_scale: 0.0,
            onomatopoeia_prepared: Default::default(),
            #[cfg(feature = "mods")]
            mod_uploaded: HashMap::new(),
            #[cfg(feature = "mods")]
            mod_generation: u64::MAX,
            lightmaps,
            white,
            samplers: Default::default(),
            bind_keys: Default::default(),
            bind_fingerprints: Default::default(),
            bind_key_list: Vec::new(),
            bind_groups: Vec::new(),
            commands: Vec::new(),
            uniforms: Default::default(),
            uniform_sources: Vec::new(),
            assembly: Default::default(),
            job_lists: Default::default(),
            depth,
            texture_layout,
            sampler,
            hud,
            fov: None,
            lens: [0.0; 2],
            in_panel: false,
            in_camera_view: false,
            overlay_aspect_cap: None,
            overlay_camera: None,
            eye_view: std::cell::Cell::new(false),
            view_size: None,
            only_post_render: false,
            hold_capture: false,
            warming: false,
            hide_next_frame: false,
            precache_roots: Vec::new(),
            precache_textures: Vec::new(),
            cartoon_warmed: false,
            fx_view_blit: std::cell::RefCell::new(None),
            hold_frames: Default::default(),
            hold_capture_alt: false,
            hold_alt_frames: Default::default(),
            hold_switch: Default::default(),
            hold_switched_at: Default::default(),
            vignette: Default::default(),
            flash: None,
            clear_volume: None,
            color_filter: None,
            fx_passes: Vec::new(),
            cartoon_flash: false,
            fx: None,
            gamma: None,
            gamma_renderer: None,
            perf_surface: [0.0; 4],
            perf_send: [0.0; 8],
            count_send: Default::default(),
            scene_triangles: Default::default(),
            fx_view_target: None,
            frame: FrameTime::default(),
            fog: None,
            volume_fog: None,
            sky: None,
            sky_fog: None,
            fog_per_zone: false,
            reported: Default::default(),
        }
    }

    fn pass_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, capacity: u64) -> PassBuffers {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("passes"),
            size: capacity * PASS_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("passes"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<PassUniform>() as u64),
                }),
            }],
        });
        PassBuffers { buffer, group, capacity, gpu: Vec::new(), shared: Vec::new() }
    }

    /// Installs the animated geometry: fixed indices and batches, vertices provided every frame.
    /// `owners`: rank of the actor of each batch (for the first-person view).
    pub fn set_dynamic(
        &mut self,
        frame: &CharacterFrame<'_>,
        indices: &[u32],
        batches: &[openxiii_assets::world::Batch],
        owners: &[usize],
        library: &mut Library,
    ) {
        for jobs in &mut self.job_lists { jobs.view = None; }
        let _ = library;
        if frame.vertex_count == 0 || indices.is_empty() {
            self.dynamic = None;
            return;
        }
        let mut gpu = vec![SceneVertex::from(&Vertex::default()); frame.vertex_count];
        for (first, mesh) in frame.visible_meshes() {
            for (dst, src) in gpu[first..].iter_mut().zip(mesh) {
                *dst = SceneVertex::from(src);
            }
        }
        let batches = batches
            .iter()
            .map(|b| SceneBatch {
                first_index: b.first_index,
                index_count: b.index_count,
                material: (!b.material.is_empty()).then(|| b.material.clone()),
                state: DrawState::default(),
                bound: Default::default(),
                prepared: None,
            })
            .collect();
        let actors = owners.iter().max().map(|m| m + 1).unwrap_or(0);
        // Buffers of the previous list reused when large enough (an actor appearing or
        // disappearing used to recreate three buffers); same bytes
        // written, the end of a larger buffer is read by no index.
        let old = self.dynamic.take();
        let (vertex_bytes, index_bytes): (&[u8], &[u8]) = (bytemuck::cast_slice(&gpu), bytemuck::cast_slice(indices));
        // Buffer reused if it keeps a quarter of margin (an almost full list would then refuse
        // the addition of an enemy: extend_dynamic, then set_dynamic of the whole list);
        let margin = |n: usize| if crate::game::characters_revived() { n as u64 * 5 / 4 } else { n as u64 };
        let reuse = |b: Option<wgpu::Buffer>, contents: &[u8], label: &str, usage: wgpu::BufferUsages| match b {
            Some(b) if b.size() >= margin(contents.len()) => {
                write_value(&self.queue, &b, 0, contents);
                b
            }
            _ => {
                // One and a half times the requested size: the list grows in small steps. With
                // the fix, twice: the loading list then grows by a good third
                // without enlarging during play.
                // Three times since the load prewarm: the actors a level spawns later (SSH102b's
                // reinforcements) fit without a new buffer while playing.
                let size = if crate::game::characters_revived() { contents.len() as u64 * 3 } else { contents.len() as u64 * 3 / 2 }.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
                let b = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
                write_value(&self.queue, &b, 0, contents);
                b
            }
        };
        if crate::game::dbg_reason() {
            let cap = old.as_ref().map_or(0, |d| d.geometry.vertex_buffer.size());
            diag(&format!(
                "list reason t={:.3} set_dynamic: {} vertices, {} indices, {} batches, bytes sent {} (vertices {} x2 + indices {}), buffers {}",
                crate::game::dbg_time(), frame.vertex_count, indices.len(), owners.len(), vertex_bytes.len() * 2 + index_bytes.len(), vertex_bytes.len(), index_bytes.len(),
                if cap >= margin(vertex_bytes.len()) { "reused" } else { "created" }
            ));
        }
        // COPY_SRC: grow_dynamic copies the content on the GPU into a larger buffer.
        let vertex_usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let (old_shadow, old_vertices, old_indices, target_vertices) = match old {
            Some(d) => (Some(d.shadow_vertices), Some(d.geometry.vertex_buffer), Some(d.geometry.index_buffer), d.inactive_vertices),
            None => (None, None, None, None),
        };
        let shadow_vertices = reuse(old_shadow, vertex_bytes, "shadow silhouettes", vertex_usage);
        let vertex_buffer = reuse(old_vertices, vertex_bytes, "animated", vertex_usage);
        let index_buffer = reuse(old_indices, index_bytes, "animated", wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC);
        let mut updates = VertexUpdates::new(gpu);
        // Mirror and work buffer reserved like the buffers (adding an actor used to
        // reallocate them on each appearance).
        if crate::game::characters_revived() {
            let n = updates.uploaded.len();
            updates.uploaded.reserve(n);
            updates.staging.reserve(n);
            updates.dirty.reserve(n.div_ceil(VERTICES_PER_PAGE));
        }
        self.dynamic = Some(Dynamic {
            updates,
            inactive_vertices: target_vertices,
            target_active: false,
            shadow_vertices,
            shadow_sent: HashMap::new(),
            geometry: Geometry { vertex_buffer, index_buffer, batches },
            owners: owners.to_vec(),
            overlay: vec![0; actors],
            ignore_vignette: vec![false; actors],
        });
        if crate::game::characters_revived() {
            if let Some(d) = &mut self.dynamic {
                prepare_target(&self.device, &self.queue, d, frame.vertex_count);
            }
        }
    }

    /// Character list rebuilt by `Game::characters_merge` with inserted ranks or a
    /// changed model from rank `since_index`: the ranks before `since_index` keep their batches;
    /// the vertices beyond `before_value` (new models) are written like `extend_dynamic`, the indices
    /// rewritten from the first changed batch, and the following batches redone like `set_dynamic`.
    /// The vertices of the kept ranks do not move. False: `set_dynamic` needed.
    pub fn relayout_dynamic(&mut self, frame: &CharacterFrame<'_>, indices: &[u32], batches: &[openxiii_assets::world::Batch], owners: &[usize], before_value: usize, since_index: usize) -> bool {
        let t0 = std::time::Instant::now();
        let Some(d) = &mut self.dynamic else {
            refusal_extend("no list in place".to_owned());
            return false;
        };
        if d.target_active || frame.vertex_count < before_value || d.updates.uploaded.len() != before_value || owners.len() != batches.len() {
            if crate::game::dbg_reason() {
                refusal_extend(format!("relayout: target_active {}; vertices {} / before {}; sent {}", d.target_active, frame.vertex_count, before_value, d.updates.uploaded.len()));
            }
            return false;
        }
        let size = std::mem::size_of::<SceneVertex>();
        let vertex_bytes = (frame.vertex_count * size) as u64;
        let index_bytes = std::mem::size_of_val(indices) as u64;
        grow_dynamic(&self.device, &self.queue, d, vertex_bytes, index_bytes);
        prepare_target(&self.device, &self.queue, d, frame.vertex_count);
        if vertex_bytes > d.geometry.vertex_buffer.size() || vertex_bytes > d.shadow_vertices.size() || index_bytes > d.geometry.index_buffer.size() {
            if crate::game::dbg_reason() {
                refusal_extend(format!("relayout: buffers too small: vertices {} B for {} B, indices {} B for {} B", vertex_bytes, d.geometry.vertex_buffer.size(), index_bytes, d.geometry.index_buffer.size()));
            }
            return false;
        }
        // With the first indices per rank (Characters::ibases), a batch that starts before the end
        // of the indices in place is a batch in place (same first index, same count, same
        // material): it is reused as is, prepared resources included; the others are
        // new, their indices at the end of the array. Otherwise: set_dynamic.
        let end_value = d.geometry.batches.iter().map(|b| (b.first_index + b.index_count) as usize).max().unwrap_or(0);
        let mut in_place: std::collections::HashMap<u32, usize> = d.geometry.batches.iter().enumerate().map(|(k, b)| (b.first_index, k)).collect();
        let mut resumed: Vec<Option<usize>> = Vec::with_capacity(batches.len());
        for b in batches {
            if (b.first_index as usize) >= end_value {
                resumed.push(None);
                continue;
            }
            let prior = in_place.remove(&b.first_index).filter(|&k| {
                let old = &d.geometry.batches[k];
                old.index_count == b.index_count && old.material.as_deref() == (!b.material.is_empty()).then_some(b.material.as_str())
            });
            if prior.is_none() {
                if crate::game::dbg_reason() {
                    refusal_extend(format!("relayout: batch in place at index {} differs (rank {since_index})", b.first_index));
                }
                return false;
            }
            resumed.push(prior);
        }
        let kept_value = resumed.iter().filter(|r| r.is_some()).count();
        let t1 = std::time::Instant::now();
        for jobs in &mut self.job_lists { jobs.view = None; }
        let mut gpu = vec![SceneVertex::from(&Vertex::default()); frame.vertex_count - before_value];
        for (first, mesh) in frame.visible_meshes() {
            if first < before_value {
                continue;
            }
            for (dst, src) in gpu[first - before_value..].iter_mut().zip(mesh) {
                *dst = SceneVertex::from(src);
            }
        }
        let offset = (before_value * size) as u64;
        if !gpu.is_empty() {
            write_value(&self.queue, &d.geometry.vertex_buffer, offset, bytemuck::cast_slice(&gpu));
            write_value(&self.queue, &d.shadow_vertices, offset, bytemuck::cast_slice(&gpu));
        }
        let t2 = std::time::Instant::now();
        let from = end_value.min(indices.len());
        if from < indices.len() {
            write_value(&self.queue, &d.geometry.index_buffer, (from * std::mem::size_of::<u32>()) as u64, bytemuck::cast_slice(&indices[from..]));
        }
        let t3 = std::time::Instant::now();
        let u = &mut d.updates;
        u.uploaded.extend_from_slice(&gpu);
        u.staging.resize(frame.vertex_count, SceneVertex::from(&Vertex::default()));
        u.dirty.resize(frame.vertex_count.div_ceil(VERTICES_PER_PAGE), false);
        // No comparison of every page: the kept ranks did not move (same first
        // vertex), the new ones are absent from `last` and compared alone (see extend_dynamic).
        u.fresh = false;
        let mut previous_list: Vec<Option<SceneBatch>> = std::mem::take(&mut d.geometry.batches).into_iter().map(Some).collect();
        d.geometry.batches = batches
            .iter()
            .zip(&resumed)
            .map(|(b, r)| match r.and_then(|k| previous_list[k].take()) {
                Some(old) => old,
                None => SceneBatch {
                    first_index: b.first_index,
                    index_count: b.index_count,
                    material: (!b.material.is_empty()).then(|| b.material.clone()),
                    state: DrawState::default(),
                    bound: Default::default(),
                    prepared: None,
                },
            })
            .collect();
        d.owners = owners.to_vec();
        let actors = owners.iter().max().map(|m| m + 1).unwrap_or(0);
        d.overlay.resize(actors.max(d.overlay.len()), 0);
        d.ignore_vignette.resize(actors.max(d.ignore_vignette.len()), false);
        let t4 = std::time::Instant::now();
        if crate::game::dbg_reason() {
            let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
            diag(&format!("list reason t={:.3} relayout_dynamic time: checks and buffers {:.3}, new vertices {:.3}, indices {:.3}, mirror and batches {:.3} ms", crate::game::dbg_time(), ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, t4)));
            let added = (frame.vertex_count - before_value) * size;
            let idx = (indices.len() - from.min(indices.len())) * std::mem::size_of::<u32>();
            diag(&format!("list reason t={:.3} relayout_dynamic: {} vertices ({} added), {} batches kept out of {}, bytes sent {} (vertices {} x2 + indices {})", crate::game::dbg_time(), frame.vertex_count, frame.vertex_count - before_value, kept_value, batches.len(), added * 2 + idx, added, idx));
        }
        true
    }

    /// Character list extended at the end (`Game::characters_add`): the batches, vertices and
    /// indices of the first `before_value` vertices are those of the list in place; only those of the
    /// added actors are written (vertices in the poses of `frame`, like `set_dynamic`), into the buffers
    /// in place when they are large enough. The vertices of the kept actors stay those of the
    /// last upload: the frame's `update_dynamic` compares them then redoes them as after
    /// `set_dynamic` (first call: every page). False: `set_dynamic` needed (no list,
    /// buffers too small, render texture view active, kept batches different).
    pub fn extend_dynamic(&mut self, frame: &CharacterFrame<'_>, indices: &[u32], batches: &[openxiii_assets::world::Batch], owners: &[usize], before_value: usize) -> bool {
        let t0 = std::time::Instant::now();
        let Some(d) = &mut self.dynamic else {
            refusal_extend("no list in place".to_owned());
            return false;
        };
        let old_batches = d.geometry.batches.len();
        if d.target_active || frame.vertex_count < before_value || d.updates.uploaded.len() != before_value || batches.len() < old_batches || owners.len() != batches.len() {
            if crate::game::dbg_reason() {
                refusal_extend(format!(
                    "target_active {}; vertices {} < before {}: {}; sent {} != before {}: {}; batches {} < old {}: {}; owners {} != batches {}: {}",
                    d.target_active, frame.vertex_count, before_value, frame.vertex_count < before_value, d.updates.uploaded.len(), before_value, d.updates.uploaded.len() != before_value,
                    batches.len(), old_batches, batches.len() < old_batches, owners.len(), batches.len(), owners.len() != batches.len()
                ));
            }
            return false;
        }
        let same = d.geometry.batches.iter().zip(batches).zip(&d.owners).zip(owners).all(|(((old, new), o), n)| {
            old.first_index == new.first_index && old.index_count == new.index_count && o == n && old.material.as_deref() == (!new.material.is_empty()).then_some(new.material.as_str())
        });
        let old_indices = d.geometry.batches.iter().map(|b| (b.first_index + b.index_count) as usize).max().unwrap_or(0);
        if !same || indices.len() < old_indices || batches[old_batches..].iter().any(|b| (b.first_index as usize) < old_indices) {
            if crate::game::dbg_reason() {
                let first_one = d.geometry.batches.iter().zip(batches).zip(&d.owners).zip(owners).position(|(((old, new), o), n)| {
                    !(old.first_index == new.first_index && old.index_count == new.index_count && o == n && old.material.as_deref() == (!new.material.is_empty()).then_some(new.material.as_str()))
                });
                let detail = first_one.map_or(String::new(), |k| {
                    let (o, n) = (&d.geometry.batches[k], &batches[k]);
                    format!(" (batch {k}: index {}+{} -> {}+{}, owner {} -> {}, material {:?} -> {:?})", o.first_index, o.index_count, n.first_index, n.index_count, d.owners[k], owners[k], o.material.as_deref(), n.material)
                });
                refusal_extend(format!("kept batches differ {}{detail}; indices {} < old {}: {}; new batch before the end of the old indices: {}", !same, indices.len(), old_indices, indices.len() < old_indices, batches[old_batches..].iter().any(|b| (b.first_index as usize) < old_indices)));
            }
            return false;
        }
        let size = std::mem::size_of::<SceneVertex>();
        let vertex_bytes = (frame.vertex_count * size) as u64;
        let index_bytes = std::mem::size_of_val(indices) as u64;
        if crate::game::characters_revived() {
            grow_dynamic(&self.device, &self.queue, d, vertex_bytes, index_bytes);
            prepare_target(&self.device, &self.queue, d, frame.vertex_count);
        }
        let t1 = std::time::Instant::now();
        if vertex_bytes > d.geometry.vertex_buffer.size() || vertex_bytes > d.shadow_vertices.size() || index_bytes > d.geometry.index_buffer.size() {
            if crate::game::dbg_reason() {
                refusal_extend(format!("buffers too small: vertices {} B for {} B (shadows {} B), indices {} B for {} B", vertex_bytes, d.geometry.vertex_buffer.size(), d.shadow_vertices.size(), index_bytes, d.geometry.index_buffer.size()));
            }
            return false;
        }
        for jobs in &mut self.job_lists { jobs.view = None; }
        let t2 = std::time::Instant::now();
        let mut gpu = vec![SceneVertex::from(&Vertex::default()); frame.vertex_count - before_value];
        for (first, mesh) in frame.visible_meshes() {
            let end = first + mesh.len();
            if end <= before_value {
                continue;
            }
            let skip = before_value.saturating_sub(first);
            for (dst, src) in gpu[first + skip - before_value..].iter_mut().zip(&mesh[skip..]) {
                *dst = SceneVertex::from(src);
            }
        }
        let offset = (before_value * size) as u64;
        // Without a new actor (only resurrected or hidden ones): no vertex to write.
        if !gpu.is_empty() {
            write_value(&self.queue, &d.geometry.vertex_buffer, offset, bytemuck::cast_slice(&gpu));
            write_value(&self.queue, &d.shadow_vertices, offset, bytemuck::cast_slice(&gpu));
        }
        let index_offset = old_indices * std::mem::size_of::<u32>();
        if indices.len() > old_indices {
            write_value(&self.queue, &d.geometry.index_buffer, index_offset as u64, bytemuck::cast_slice(&indices[old_indices..]));
        }
        // Mirror: what the buffer now contains, every page compared on the next call.
        let u = &mut d.updates;
        let t3 = std::time::Instant::now();
        u.uploaded.extend_from_slice(&gpu);
        u.staging.resize(frame.vertex_count, SceneVertex::from(&Vertex::default()));
        u.dirty.resize(frame.vertex_count.div_ceil(VERTICES_PER_PAGE), false);
        // Comparing every page on the next call used to make a single upload from the first to the
        // last changed vertex (1.4 MB for one added actor); with the fix, only the
        // pages of new, changed or vanished meshes (absent from `last` or different) are compared,
        // the others keep their first vertex and the mirror their bytes.
        u.fresh = !crate::game::characters_revived();
        d.geometry.batches.extend(batches[old_batches..].iter().map(|b| SceneBatch {
            first_index: b.first_index,
            index_count: b.index_count,
            material: (!b.material.is_empty()).then(|| b.material.clone()),
            state: DrawState::default(),
            bound: Default::default(),
            prepared: None,
        }));
        d.owners = owners.to_vec();
        let actors = owners.iter().max().map(|m| m + 1).unwrap_or(0);
        d.overlay.resize(actors.max(d.overlay.len()), 0);
        d.ignore_vignette.resize(actors.max(d.ignore_vignette.len()), false);
        if crate::game::dbg_reason() {
            let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
            diag(&format!("list reason t={:.3} extend_dynamic time: checks and buffers {:.3}, work lists {:.3}, vertices and writes {:.3}, mirror and batches {:.3} ms", crate::game::dbg_time(), ms(t0, t1), ms(t1, t2), ms(t2, t3), ms(t3, std::time::Instant::now())));
        }
        if crate::game::dbg_reason() {
            let added = (frame.vertex_count - before_value) * size;
            let idx = indices.len().saturating_sub(old_indices) * std::mem::size_of::<u32>();
            diag(&format!("list reason t={:.3} extend_dynamic: {} vertices ({} added), bytes sent {} (vertices {} x2 + indices {})", crate::game::dbg_time(), frame.vertex_count, frame.vertex_count - before_value, added * 2 + idx, added, idx));
        }
        true
    }

    /// Particles of the frame (USpriteEmitter::FillVertexBuffer of each emitter drawn):
    /// four vertices per particle, triangles i, i+1, i+2 and i+2, i+1, i+3.
    /// StaticMeshes of map actors with a changed mesh (their original batches are hidden by
    /// `set_hidden_owners`); None: none.
    pub fn set_replaced(&mut self, world: Option<&World>) {
        for jobs in &mut self.job_lists { jobs.view = None; }
        // Buffers of the previous call rewritten if they are enough (called almost every frame in
        // combat: two new buffers each time).
        let prior = self.replaced.take().map(|(g, _)| g).or_else(|| self.replaced_spare.take());
        // Nothing replaced any more: the buffers kept for the next swap.
        let world = match world.filter(|w| !w.batches.is_empty()) {
            Some(w) => Some(w),
            None => {
                self.replaced_spare = prior;
                self.replaced_sine.clear();
                return;
            }
        };
        let prior = prior;
        self.replaced = world.filter(|w| !w.batches.is_empty()).map(|w| {
            let batches = w
                .batches
                .iter()
                .enumerate()
                .map(|(i, b)| SceneBatch {
                    first_index: b.first_index,
                    index_count: b.index_count,
                    material: (!b.material.is_empty()).then(|| b.material.clone()),
                    state: w.states.get(i).cloned().unwrap_or_default(),
                    bound: Default::default(),
                    prepared: None,
                })
                .collect();
            let vertices = scene_vertices(w);
            self.replaced_sine = sine_sections(w, &vertices);
            let mut g = geometry_kept(&self.device, &self.queue, prior, &vertices, &w.indices, "replaced meshes");
            g.batches = batches;
            (g, w.batch_owners.clone())
        });
        if self.replaced.is_none() {
            self.replaced_sine.clear();
        }
    }

    /// At load: replaced-meshes buffers able to hold every static geometry actor of the map
    /// (`Game::replaced_capacity`), so a script's mesh swap rewrites them while playing.
    pub fn reserve_replaced(&mut self, world: Option<&World>) {
        if self.replaced.is_some() || self.replaced_spare.is_some() {
            return;
        }
        // Nothing yet (the map's actors not spawned): asked again next frame, which costs
        // nothing until there are static geometry actors.
        let Some(w) = world else { return };
        // Made once per renderer (the map's whole static geometry is built for it).
        self.replaced_reserved = true;
        let vertices = scene_vertices(w);
        self.replaced_spare = Some(geometry_kept(&self.device, &self.queue, None, &vertices, &w.indices, "replaced meshes"));
    }

    /// Whether the replaced-meshes buffers are still to be made at load.
    pub fn wants_replaced_reserve(&self) -> bool {
        !self.replaced_reserved && self.replaced.is_none() && self.replaced_spare.is_none() && !self.scenery_warmup
    }

    /// AActor::execReplaceATextureByAnOther replacements (the VM's list) for display.
    pub fn set_texture_replacements(&mut self, list: &[(String, String)]) {
        self.materials.set_replacements(list);
    }

    /// End for each SinusModifier section drawn this frame: `drawn`
    /// gives, in the VM's draw order, (actor, modifier, CurrentTime, amplitude) of
    /// each SetSinusVertexStreams (Vm::sine_drawn); None (no game, or forced view): every
    /// section of the non-hidden actors, advanced here with their own draw sequence. The
    /// waved positions are put back into world space and rewritten into the buffer.
    pub fn update_sine(&mut self, library: &mut Library, drawn: Option<&[(String, String, f32, [f32; 3])]>) {
        if self.world_sine.is_empty() && self.replaced_sine.is_empty() {
            return;
        }
        type Job = (bool, usize, Option<(f32, [f32; 3])>);
        let order: Vec<Job> = match drawn {
            Some(records) => {
                // Sections of the same actor and modifier taken in section order.
                let mut used: std::collections::HashSet<(bool, usize)> = std::collections::HashSet::new();
                let mut jobs = Vec::new();
                for (owner, key, t, amp) in records {
                    let fits = |s: &openxiii_assets::world::SineSection| s.owner.as_deref() == Some(owner.as_str()) && s.modifier.eq_ignore_ascii_case(key);
                    let found = (0..self.world_sine.len())
                        .map(|i| (false, i))
                        .chain((0..self.replaced_sine.len()).map(|i| (true, i)))
                        .find(|&(r, i)| !used.contains(&(r, i)) && fits(if r { &self.replaced_sine[i].0 } else { &self.world_sine[i].0 }));
                    if let Some((r, i)) = found {
                        used.insert((r, i));
                        jobs.push((r, i, Some((*t, *amp))));
                    }
                }
                jobs
            }
            None => (0..self.world_sine.len())
                .filter(|&i| !self.world_sine[i].0.owner.as_ref().is_some_and(|o| self.hidden_owners.contains(o)))
                .map(|i| (false, i, None))
                .chain((0..self.replaced_sine.len()).map(|i| (true, i, None)))
                .collect(),
        };
        let frame = self.frame;
        let seed = &mut self.sine_seed;
        // appFrand: rand() × (1 / 32767).
        let mut frand = || {
            *seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011);
            ((*seed >> 16) & 0x7FFF) as f32 * 3.051_851e-5
        };
        for (replaced, i, call) in order {
            let (list, buffer) = if replaced {
                match &self.replaced {
                    Some((g, _)) => (&mut self.replaced_sine, &g.vertex_buffer),
                    None => continue,
                }
            } else {
                (&mut self.world_sine, &self.world.vertex_buffer)
            };
            let (section, vertices) = &mut list[i];
            let local = match call {
                Some((t, amp)) => {
                    self.materials.sine_params(library, &section.modifier).map(|p| openxiii_assets::sine::wave(&p, t, amp, section.fade, &section.local))
                }
                None => self.materials.sine_deform(library, &section.modifier, frame, &mut frand, section.fade, &section.local),
            };
            let Some(local) = local else { continue };
            // Same LocalToWorld as the static geometry (world::add_static_meshes).
            let l2w = section.actor.local_to_world();
            // Vertices unchanged since the last upload (waving stopped, zero amplitude): nothing to
            // send to the GPU again.
            let mut change = false;
            for (v, p) in vertices.iter_mut().zip(&local) {
                let q = openxiii_assets::lighting::transform_point(&l2w, *p);
                change |= v.position.map(f32::to_bits) != q.map(f32::to_bits);
                v.position = q;
            }
            let unchanged = !change;
            if unchanged {
                continue;
            }
            write_value(&self.queue, buffer, section.base as u64 * std::mem::size_of::<SceneVertex>() as u64, bytemuck::cast_slice(vertices));
        }
    }

    /// bIgnoreVignetteAlpha of the animated actors (rank of `set_dynamic`).
    pub fn set_ignore_vignette(&mut self, flags: &[bool]) {
        if let Some(d) = &mut self.dynamic {
            for (k, f) in flags.iter().enumerate() {
                if let Some(slot) = d.ignore_vignette.get_mut(k) {
                    *slot = *f;
                }
            }
        }
    }

    /// Static geometry actors that have bIgnoreVignetteAlpha.
    pub fn set_vignette_owners(&mut self, owners: std::rc::Rc<std::collections::HashSet<String>>) {
        self.vignette_owners = owners;
    }

    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Improved build: the same wait while another thread presents the loading picture
    /// (`hold_start`): the device is polled without blocking every 2 ms until the work submitted
    /// so far is done (at most `limit`), so the presenting thread is never held back for the
    /// whole wait (a blocking poll kept it from presenting: the picture's longest gap was the
    /// wait itself, 40 to 80 ms after a level's warm-up).
    pub fn wait_idle_sharing(&self, limit: std::time::Duration) {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        self.queue.on_submitted_work_done(move || flag.store(true, std::sync::atomic::Ordering::Release));
        let started = std::time::Instant::now();
        loop {
            let _ = self.device.poll(wgpu::PollType::Poll);
            if done.load(std::sync::atomic::Ordering::Acquire) || started.elapsed() >= limit {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The visibility's translucent list for the order of sorted passes.
    pub fn set_translucent_keys(&mut self, keys: TranslucentKeys) {
        self.translucent_keys = keys;
    }

    /// Non-static static geometry actors not to draw (hidden or destroyed).
    pub fn set_hidden_owners(&mut self, hidden: std::rc::Rc<std::collections::HashSet<String>>) {
        self.hidden_owners = hidden;
    }

    /// Computes the passes (and the actors' cartoon directions) without drawing: single-frame
    /// screenshot, where there is no previous frame.
    pub fn prepare_cartoon_dirs(&mut self, library: &mut Library, camera: &Camera) {
        self.update_zone(camera, true);
        // No projection uploaded by this preparation without drawing.
        let projection = self.particles_projection.take();
        self.prepare(library, camera);
        self.particles_projection.set(projection);
    }

    /// Cartoon light direction computed for animated actor `owner` in the previous frame.
    pub fn cartoon_dir(&self, owner: usize) -> Option<[f32; 3]> {
        self.cartoon_dirs.get(&owner).copied()
    }

    /// Character list rebuilt or extended: each cartoon direction of the previous frame
    /// follows its actor to its new rank (from its old rank), a rank without a former actor has none.
    /// Kept by rank, they would otherwise go for one frame to the actor that took another's rank
    /// (shadow turned towards another character's light after each rebuild).
    pub fn cartoon_dirs_follow(&mut self, ranks: usize, prior: impl Fn(usize) -> Option<usize>) {
        let before_value = std::mem::take(&mut self.cartoon_dirs);
        self.cartoon_dirs = (0..ranks).filter_map(|k| prior(k).and_then(|o| before_value.get(&o)).map(|d| (k, *d))).collect();
    }

    /// Shadow silhouettes to draw before the frame.
    pub fn set_shadow_jobs(&mut self, jobs: Vec<ShadowRender>) {
        self.shadow_jobs = jobs;
    }

    /// Shadow targets brought to `shadow_target_size` for the world's current size; the
    /// texture groups that held the old views are made again (as after a quality change).
    /// Returns true when they were made again: the shadows they held must be drawn again.
    fn fit_shadow_targets(&mut self) -> bool {
        let (_, world_h) = self.world_size();
        #[allow(unused_mut)]
        // Graphics: Original: D3DDrv's 256 x 256, as the faithful build.
        let mut size = configured_shadow_target_size(world_h, self.quality_caps.max_texture_dimension, self.shadow_quality);
        if size == self.shadow_size {
            return false;
        }
        diag(&format!("shadows: targets of {size} x {size} for a world {world_h} pixels high"));
        // Made one by one as in grow_shadow_targets: a size the video memory cannot hold for
        // every target keeps those it could make (at least D3DDrv's eight, made as before).
        let count = std::mem::take(&mut self.shadow_targets).len();
        self.shadow_targets_full = false;
        self.shadow_targets = shadow_targets(&self.device, self.color_format, size, count.min(openxiii_vm::projector::SHADOW_SLOTS));
        self.shadow_size = size;
        self.grow_shadow_targets(count);
        self.flush_bindings();
        true
    }

    /// Shadow targets and render textures brought to the sizes of the world as drawn now
    /// (improved build; the faithful build keeps D3DDrv's sizes). Called at the start of a frame,
    /// before anything of the frame is prepared. True when the shadow targets were made again.
    pub fn fit_targets(&mut self) -> bool {
        let (_, world_h) = self.world_size();
        // Graphics: Original: the original's sizes (pixelated comic panels, 512 screen effects).
        let target_sizes = RenderTargetSizes::local(self);
        let scale = target_sizes.panel_scale(world_h, self.quality_caps.max_texture_dimension);
        if scale != self.target_scale {
            diag(&format!("render textures: {0} x {0} for a world {world_h} pixels high", TARGET_SIDE as u32 * scale));
            self.target_scale = scale;
            // Made again at the new size by the next render_target, with every view of the frame.
            self.hud.targets.clear();
            self.flush_bindings();
        }
        // Mods build only: a script's heat shimmer also gets the screen's resolution.
        let fx_side = target_sizes.effects_side(world_h, self.quality_caps.max_texture_dimension);
        if fx_side != self.fx_side {
            diag(&format!("screen effects: scene texture of {fx_side} x {fx_side} for a world {world_h} pixels high"));
            self.fx_side = fx_side;
            self.fx = None;
        }
        self.fit_shadow_targets()
    }

    /// UShadowBitmapMaterial::Update: the owner's mesh
    /// seen from the light (orthographic, half-width USize / 2, z × 1.9e-6 + 0.5), in white
    /// (TFACTOR 0xFF, R5G6B5) on a black background: the intensity is applied when the
    /// projection is drawn, viewport shrunk by one texel (no depth
    /// test, no blending, no lighting).
    /// Split screen and Graphics: Enhanced: shadow targets up to `n`, the ones beyond D3DDrv's
    /// eight made on their first use, of the same size, black. A target the video memory
    /// cannot hold is not kept: the targets stop at those already made and the VM reuses the
    /// oldest one, as does (openxiii_vm::projector::note_shadow_target_limit).
    fn grow_shadow_targets(&mut self, n: usize) {
        while self.shadow_targets.len() < n && !self.shadow_targets_full {
            let Some(target) = self.try_shadow_target(self.shadow_size) else { break };
            self.shadow_targets.push(target);
        }
    }

    /// One more shadow target of `size`, or none when the video memory cannot hold it: the
    /// targets then stop at those made (`shadow_targets_full`, and the VM is told).
    fn try_shadow_target(&mut self, size: u32) -> Option<(wgpu::TextureView, wgpu::TextureView)> {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let invalid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let target = shadow_target(&self.device, self.color_format, size);
        let failed = pollster::block_on(invalid.pop()).is_some() | pollster::block_on(oom.pop()).is_some();
        if failed {
            self.shadow_targets_full = true;
            openxiii_vm::projector::note_shadow_target_limit(self.shadow_targets.len());
            return None;
        }
        Some(target)
    }

    fn render_shadows(&mut self, library: &mut Library, encoder: &mut wgpu::CommandEncoder) {
        let jobs = std::mem::take(&mut self.shadow_jobs);
        if jobs.is_empty() || self.dynamic.is_none() {
            return;
        }
        // Split screen and Graphics: Enhanced: the targets beyond D3DDrv's eight are made when a
        // shadow first takes one (openxiii_vm::projector::shadow_slot_count); a single view in
        // Graphics: Original keeps the eight.
        let wanted = jobs.iter().map(|j| j.slot + 1).max().unwrap_or(0);
        self.grow_shadow_targets(wanted);
        use openxiii_assets::material::d3d::{TA_TFACTOR, TOP_SELECTARG1};
        let mut prepared = Vec::new();
        for job in &jobs {
            let mut pass = openxiii_assets::material::Pass { z_test: false, z_write: false, two_sided: true, stages_used: 1, ..Default::default() };
            // D3DFMT_R5G6B5 target (0x17): no alpha, read back as 1 everywhere.
            pass.tfactor = 0xffff_ffff;
            pass.stages[0].color_op = TOP_SELECTARG1;
            pass.stages[0].color_arg1 = TA_TFACTOR;
            pass.stages[0].alpha_op = TOP_SELECTARG1;
            pass.stages[0].alpha_arg1 = TA_TFACTOR;
            let key = PipelineKey::new(&pass, 2);
            let pipeline = self.pipeline(key);
            let offset = self.append_uniform(PassUniform::new(&pass, DrawBits::default()));
            prepared.push((job.clone(), pipeline, offset));
        }
        self.upload_passes();
        let bind = self.bind_group(library, std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD)));
        // One frame uniform per shadow, then a single upload: same passes in the same order.
        while self.shadow_frames_extra.len() + 1 < prepared.len() {
            self.add_shadow_frame();
        }
        for (n, (job, pipeline, offset)) in prepared.into_iter().enumerate() {
            let dir = job.light_dir;
            let eye = [job.location[0] + dir[0] * 1000.0, job.location[1] + dir[1] * 1000.0, job.location[2] + dir[2] * 1000.0];
            let f = [-dir[0], -dir[1], -dir[2]];
            let yaw = f[1].atan2(f[0]).to_degrees();
            let pitch = f[2].atan2((f[0] * f[0] + f[1] * f[1]).sqrt()).to_degrees();
            let camera = Camera { position: eye, yaw, pitch, roll: 0.0 };
            let [x, _, z] = camera.axes();
            let view = look_to_rh([eye[0], -eye[1], eye[2]], [x[0], -x[1], x[2]], [z[0], -z[1], z[2]]);
            let half = (job.usize / 2).max(1) as f32;
            let k = 1.907_348_6e-6f32;
            let proj: Mat = [[1.0 / half, 0.0, 0.0, 0.0], [0.0, 1.0 / half, 0.0, 0.0], [0.0, 0.0, -k, 0.0], [0.0, 0.0, 0.5, 1.0]];
            let u = FrameUniform { view_proj: mul(&proj, &view), view: d3d_view(&camera), fog: [0.0; 4], fog_color: [0.0; 4], ambient: [0.0; 4] };
            let (frame_buffer, frame_group) = match n {
                0 => (&self.shadow_frame_buffer, &self.shadow_frame_group),
                _ => (&self.shadow_frames_extra[n - 1].0, &self.shadow_frames_extra[n - 1].1),
            };
            write_value(&self.queue, frame_buffer, 0, bytemuck::bytes_of(&u));
            let Some((target, depth)) = self.shadow_targets.get(job.slot) else { continue };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: depth,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // One texel of 256 on each side, the same share of a larger target (same
                // silhouette size and black border, only finer).
                let side = self.shadow_size as f32;
                let border = side / SHADOW_TARGET_SIZE as f32;
                pass.set_viewport(border, border, side - 2.0 * border, side - 2.0 * border, 0.0, 1.0);
                pass.set_bind_group(0, frame_group, &[]);
                pass.set_bind_group(1, &self.pass_buffers.group, &[offset]);
                pass.set_bind_group(2, &self.bind_groups[bind], &[]);
                pass.set_pipeline(&self.pipelines[pipeline].1);
                let d = self.dynamic.as_ref().unwrap();
                pass.set_vertex_buffer(0, d.shadow_vertices.slice(..));
                pass.set_index_buffer(d.geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for (i, b) in d.geometry.batches.iter().enumerate() {
                    if d.owners.get(i).is_some_and(|o| *o == job.owner || job.attached.contains(o)) {
                        pass.draw_indexed(b.first_index..b.first_index + b.index_count, 0, 0..1);
                    }
                }
            }
        }
    }

    /// Lighting of the StaticMeshes drawn outside the static geometry (per animated actor).
    pub fn set_prop_lights(&mut self, lights: Vec<Option<PropLight>>) {
        self.prop_colors.clear();
        for p in lights.iter().flatten() {
            if let Some(c) = self.prop_instance_colors(p) {
                self.prop_colors.push((p.first_vertex, c));
            }
        }
        self.prop_lights = lights;
    }

    /// Colour stream of the StaticMeshInstance: with bIgnoreDynLight,
    /// CalculateLighting while the stream's revision (+0x48) is at most 1, so
    /// once, zone ambient included except with bNoAmbientLight; otherwise CalculateLighting without
    /// ambient if a light is no longer in the recorded state, or the recorded colours.
    fn prop_instance_colors(&mut self, p: &PropLight) -> Option<Vec<[u8; 4]>> {
        if p.unlit {
            return None;
        }
        let inst = p.instance.as_ref()?;
        let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let rgba = |c: Vec<[u8; 3]>| c.into_iter().map(|c| [c[0], c[1], c[2], 255]).collect::<Vec<_>>();
        if p.ignore_dyn {
            if let Some(c) = self.prop_colors_once.get(&*p.name) {
                return Some(c.clone());
            }
            // Not drawn by the engine yet: CalculateLighting has not happened.
            if !p.first_rendered {
                return None;
            }
            // CalculateLighting: ambient of the zones of the actor's leaves, or of the pivot with bUseOnlyPivotAmbient; neither the
            // "at most one leaf → pivot" rule of FDynamicActor::Refresh, nor a zone without a leaf.
            let (_, leaf_zones, _, pivot) = self.prop_ambient(p);
            let rgb = |z: usize| self.zones.get(z).map(|zr| zr.ambient_hsv).unwrap_or([0.0; 3]);
            let leaf_zones: Vec<usize> = leaf_zones.iter().map(|&z| z.max(0) as usize).collect();
            let base = openxiii_assets::lighting::calculate_lighting_base(true, p.no_ambient, p.pivot_ambient, &leaf_zones, Some(pivot.max(0) as usize), p.glow, &rgb);
            let c = rgba(openxiii_assets::lighting::calculate_lighting(&p.geometry.positions(), &p.geometry.normals(), &identity, base, &inst.lights, &self.lights));
            self.prop_colors_once.insert(p.name.to_string(), c.clone());
            return Some(c);
        }
        let lit = |idx: i16| self.lights.get(idx as usize).is_none_or(|l| l.dynamic || l.animated);
        let recalc = inst.lights.iter().any(|i| i.light != -1 && lit(i.light) as i32 == i.applied);
        if recalc || inst.colors.len() != p.geometry.len() {
            Some(rgba(openxiii_assets::lighting::calculate_lighting(&p.geometry.positions(), &p.geometry.normals(), &identity, [0; 3], &inst.lights, &self.lights)))
        } else {
            Some(inst.colors.iter().map(|c| [c[2], c[1], c[0], 255]).collect())
        }
    }

    /// Ambient of an animated StaticMesh (FDynamicActor::Refresh): maximum of those of the zones of
    /// its leaves (at most 4), or of the pivot's zone with bUseOnlyPivotAmbient or at most one
    /// leaf, plus AmbientGlow; with leaf zones, lights per leaf and the pivot's zone.
    fn prop_ambient(&self, p: &PropLight) -> ([u8; 3], Vec<i32>, Vec<Vec<usize>>, i32) {
        refresh_ambient::prop_ambient(
            p,self.lightmaps.model.as_deref(),self.active_light_pass(),self.active_light_draw(),
            &self.region_nodes,self.root_outside,
            &|z|self.zones.get(z).map(|zr|zr.ambient_hsv).unwrap_or([0.0;3]),
        )
    }

    /// Without a StaticMeshInstance: bUnlit (white, no lighting); bIgnoreDynLight:
    /// EnableLighting(0, 1), without a colour stream hence white; otherwise
    /// EnableLighting(1, 1), D3DRS_AMBIENT from FDynamicActor::Refresh (maximum of the
    /// leaf zones' ambient plus AmbientGlow; black with bNoAmbientLight) and SetLight
    /// of the lights.
    fn prop_light_state(&self, p: &PropLight) -> (DrawBits, Vec<[[f32; 4]; 4]>) {
        if let Some(result)=refresh_ambient::prop_light_guard(p) { return result; }
        let has_color=p.instance.is_some();
        let pass = self.active_light_pass();
        let draw = self.active_light_draw();
        let selected = crate::lights_scene::pass_lights(pass, draw, self.lightmaps.model.as_deref(), &self.lights);
        let captured = draw.and_then(|d| d.refresh.as_ref()).map(|r| r.ambient);
        let diagnostic = false;
        // Real Scene draws already have their ambient and light selection.
        // The StaticMesh's own leaves are still used by the source path and the trace.
        let (ambient, leaf_zones, selected) = match (selected, captured, diagnostic) {
            (Some(lights), Some(ambient), false) => (ambient, Vec::new(), lights),
            (selected, _, _) => {
                let (ambient, zones, leaf_lights, pivot) = self.prop_ambient(p);
                let zones = if zones.is_empty() { vec![pivot] } else { zones };
                let lights = selected.unwrap_or_else(|| {
                    let c = [0, 1, 2].map(|k| (p.bmin[k] + p.bmax[k]) * 0.5);
                    let e = [0, 1, 2].map(|k| (p.bmax[k] - p.bmin[k]) * 0.5);
                    let sphere = [c[0], c[1], c[2], (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt()];
                    let target = openxiii_assets::cartoon::LightTarget { light_group: p.light_group, leaf_zones: &zones, leaf_lights: &leaf_lights, sphere };
                    openxiii_assets::cartoon::select_lights(&target, &self.lights)
                });
                (ambient, zones, lights)
            }
        };
        let ambient = if p.no_ambient { [0; 3] } else { ambient };
        let bits = DrawBits { lighting: true, color_vertex: true, has_color, ambient, ..Default::default() };
        if diagnostic {
            eprintln!(
                "prop @ {:?} group {} leaves {} zones {:?} ambient {ambient:?}: {} lights {:?}",
                p.location,
                p.light_group,
                p.leaves.len(),
                leaf_zones,
                selected.len(),
                selected.iter().take(8).map(|l| (l.location.map(|x| x as i32), l.light_type, l.effect, l.cone, l.color, l.radius, l.direction)).collect::<Vec<_>>()
            );
        }
        let lights = selected.into_iter().take(8).map(d3d::d3d_light).collect();
        (bits, lights)
    }

    /// The map's BSP model.
    pub fn model(&self) -> Option<std::sync::Arc<openxiii_assets::model::Model>> {
        self.lightmaps.model.clone()
    }

    /// split_projectors: union of the rectangles of the batches covered by the draw [first,
    /// first + count); None (draw everything) if a batch has no rectangle or goes beyond.
    fn rect_projectors(&self, first_one: u32, count_value: u32) -> Option<[u32; 4]> {
        let end_value = first_one + count_value;
        let mut i = self.projector_rects.partition_point(|x| x.0 < first_one);
        let mut u: Option<[u32; 4]> = None; // x0, y0, x1, y1
        let mut covered = first_one;
        while let Some(&(a, b, r)) = self.projector_rects.get(i) {
            if a >= end_value {
                break;
            }
            if a != covered || b > end_value {
                return None;
            }
            let [x, y, w, h] = r?;
            if w > 0 && h > 0 {
                u = Some(match u {
                    None => [x, y, x + w, y + h],
                    Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h)],
                });
            }
            covered = b;
            i += 1;
        }
        if covered != end_value {
            return None;
        }
        Some(u.map_or([0, 0, 0, 0], |[x0, y0, x1, y1]| [x0, y0, x1 - x0, y1 - y0]))
    }

    /// split_projectors: matrices of the batch and clamped edges of its two textures.
    pub(crate) fn projector_rect_info(&mut self, d: &openxiii_vm::projector::ProjectorDraw, library: &mut Library) -> ([f32; 16], [f32; 16], bool, bool, bool) {
        // Texture frame: only for shadows (UShadowBitmapMaterial, edge cleared by the
        // engine). Impacts and stains have almost neutral edges (1 to 2 levels of difference):
        // their whole polygon stays drawn, as in the original.
        let texture = d.texture.starts_with("#shadow:");
        let clip = self.texture_blocked(library, if d.info.flags & 4 != 0 { "Engine.ProjGradient" } else { "Engine.ProjClip" });
        (d.info.matrix, d.info.gradient, d.info.flags & 1 != 0, texture, clip)
    }

    /// Texture (at the end of the FinalBlends) with both addressing modes clamped (TC_Clamp); shadows
    /// ("#shadow:", UShadowBitmapMaterial): clamped.
    fn texture_blocked(&mut self, library: &mut Library, path_buf: &str) -> bool {
        if path_buf.starts_with("#shadow:") {
            return true;
        }
        if let Some(&b) = self.textures_blocked.get(path_buf) {
            return b;
        }
        let b = self.materials.texture_blocked_edges(library, path_buf);
        self.textures_blocked.insert(path_buf.to_string(), b);
        b
    }

    /// Projectors of the drawn BSP nodes (Vm::bsp_projector_draws): material key from
    /// for each.
    pub fn set_bsp_projectors(&mut self, draws: &[openxiii_vm::projector::ProjectorDraw], library: &mut Library) {
        let t0 = false.then(std::time::Instant::now);
        if impacts_incremental() { self.set_bsp_projectors_incremental(draws, library); }
        else { self.set_bsp_projectors_in(draws, library); }
        if let Some(t0) = t0 {
            use std::sync::atomic::Ordering::Relaxed;
            PROJ_REDONE[0].fetch_add(draws.len() as u64, Relaxed);
            PROJ_REDONE[3].fetch_add(t0.elapsed().as_nanos() as u64, Relaxed);
        }
    }

    fn set_bsp_projectors_in(&mut self, draws: &[openxiii_vm::projector::ProjectorDraw], library: &mut Library) {
        let (mut geometry_sent, mut batches_list) = match self.projectors.take() {
            Some((g, batches_list)) => (Some(g), batches_list),
            None => (None, Vec::new()),
        };
        batches_list.clear();
        let mut first = 0;
        let mut changed = geometry_sent.is_none();
        for (i, d) in draws.iter().enumerate() {
            // The translation may change the current blend mode, even for an empty polygon.
            let Some(key) = self.materials.projector_key_at(i, library, &d.texture, d.info.flags, &d.info.matrix, &d.info.gradient) else { continue };
            if d.poly.len() < 3 {
                continue;
            }
            let slot = batches_list.len();
            if let Some((previous, triangles)) = self.projector_polys.get_mut(slot) {
                if !std::rc::Rc::ptr_eq(previous, &d.poly) {
                    // Some producers return a new Rc at the same positions. Compare
                    // the bytes (including the sign of zero) before unfolding the SceneVertex.
                    changed |= bytemuck::cast_slice::<_, u8>(previous.as_ref()) != bytemuck::cast_slice::<_, u8>(d.poly.as_ref());
                    *previous = d.poly.clone();
                }
                changed |= *triangles != d.triangles;
                *triangles = d.triangles;
            } else {
                self.projector_polys.push((d.poly.clone(), d.triangles));
                changed = true;
            }
            let count = if d.triangles { d.poly.len() / 3 * 3 } else { (d.poly.len() - 2) * 3 } as u32;
            batches_list.push((key, first, count, if d.triangles { 0 } else { 8 }, (!d.triangles).then_some(d.node as u32)));
            first += count;
            let info = self.projector_rect_info(d, library);
            self.projector_rect_infos.truncate(batches_list.len() - 1);
            self.projector_rect_infos.push(info);
        }
        changed |= self.projector_polys.len() != batches_list.len();
        self.projector_polys.truncate(batches_list.len());
        self.projector_packets.truncate(batches_list.len());
        if batches_list.is_empty() {
            return;
        }
        if changed {
            self.projector_bounds.clear();
            self.target_range_cache.projector_stages.clear();
            let mut vertices = Vec::new();
            let mut indices = Vec::with_capacity(first as usize);
            for (poly, triangles) in &self.projector_polys {
                let base = vertices.len() as u32;
                vertices.extend(poly.iter().map(|p| SceneVertex {
                    position: *p, normal: [0.0; 3], color: [255; 4], uv0: [0.0; 2], uv1: [0.0; 2],
                }));
                if *triangles {
                    indices.extend(base..base + (poly.len() / 3 * 3) as u32);
                } else {
                    for k in 1..poly.len() as u32 - 1 {
                        indices.extend([base, base + k, base + k + 1]);
                    }
                }
            }
            // Previous buffers reused when they are large enough.
            geometry_sent = Some(geometry_kept(&self.device, &self.queue, geometry_sent.take(), &vertices, &indices, "projectors"));
        }
        self.projectors = Some((geometry_sent.unwrap(), batches_list));
    }

    /// Weather of the frame (DrawRndCubeSpr): each point of each cell becomes a square facing
    /// the camera with a side of height × Size / distance pixels (D3DRS_POINTSCALE A = B = 0, C = 1),
    /// at most SizeMax, texture coordinates (0, 0) at the top left.
    pub fn set_weather(&mut self, draw: Option<&openxiii_vm::weather::WeatherDraw>, camera: &[[f32; 4]; 4]) {
        let Some(d) = draw.filter(|d| !d.cells.is_empty() && !d.flakes.is_empty() && !d.texture.is_empty()) else {
            // Its buffers kept for the next shower.
            if let Some(w) = self.weather.take() {
                self.weather_spare = Some(w.0);
            }
            return;
        };
        let [vw, vh] = self.view_size.unwrap_or([self.width as f32, self.height as f32]);
        let aspect = vw / vh;
        let proj11 = 1.0 / (self.view_fov_y(aspect) * 0.5).tan();
        // 512 viewport when the scene goes into the effects texture.
        let h = if self.fx_passes.is_empty() { vh } else { self.fx_side as f32 };
        let right = [camera[0][0], camera[0][1], camera[0][2]];
        let down = [camera[1][0], camera[1][1], camera[1][2]];
        let fwd = [camera[2][0], camera[2][1], camera[2][2]];
        let eye = [camera[3][0], camera[3][1], camera[3][2]];
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for cell in &d.cells {
            for (p, c) in &d.flakes {
                let w = [p[0] + cell[0], p[1] + cell[1], p[2] + cell[2]];
                let v = [w[0] - eye[0], w[1] - eye[1], w[2] - eye[2]];
                let z = v[0] * fwd[0] + v[1] * fwd[1] + v[2] * fwd[2];
                if z <= 1.0 {
                    continue;
                }
                let from_time = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                let px = (h * d.size / from_time).clamp(0.0, d.size_max);
                let half = px * z / (proj11 * h);
                let color = [((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8, (c >> 24) as u8];
                let base = vertices.len() as u32;
                for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let position = [
                        w[0] + (right[0] * sx + down[0] * sy) * half,
                        w[1] + (right[1] * sx + down[1] * sy) * half,
                        w[2] + (right[2] * sx + down[2] * sy) * half,
                    ];
                    vertices.push(SceneVertex { position, normal: [0.0; 3], color, uv0: [(sx + 1.0) * 0.5, (sy + 1.0) * 0.5], uv1: [0.0; 2] });
                }
                indices.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
            }
        }
        // Buffers kept from frame to frame and sized once for every flake of every cell (the
        // original fills fixed dynamic buffers): no GPU object is created while it rains or snows.
        let count = indices.len() as u32;
        let quads = (d.cells.len() * d.flakes.len()).max(1);
        let prior = self.weather.take().map(|w| w.0).or_else(|| self.weather_spare.take()).filter(|g| g.vertex_buffer.size() >= (quads * 4 * std::mem::size_of::<SceneVertex>()) as u64 && g.index_buffer.size() >= (quads * 6 * 4) as u64);
        let geometry_value = match prior {
            Some(g) => {
                if count > 0 {
                    write_value(&self.queue, &g.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
                    write_value(&self.queue, &g.index_buffer, 0, bytemuck::cast_slice(&indices));
                }
                g
            }
            None => {
                let g = self.weather_geometry(quads);
                if count > 0 {
                    write_value(&self.queue, &g.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
                    write_value(&self.queue, &g.index_buffer, 0, bytemuck::cast_slice(&indices));
                }
                g
            }
        };
        self.weather = Some((geometry_value, d.texture.clone(), d.mask, count));
    }

    /// Map scenery culled by the engine's visibility (zones, portals, far
    /// plane at the end of GetViewFrustum's fog): only the BSP nodes kept by the
    /// player's view (`Game::bsp_visible`) are drawn; None: all the scenery.
    pub fn set_bsp_visible(&mut self, visible: Option<SceneryVisible>) {
        let Some(d) = visible.filter(|_| true) else {
            self.world_masked = None;
            self.world_compact = None;
            self.last_bsp_visibility = None;
            return;
        };
        // Same positions as the map buffer (batches, translucent per node, StaticMeshes
        // drawn actor by actor): a triangle not kept becomes three times the same vertex.
        let flatten = |indices: &mut [u32], first: u32, count: u32| {
            if let Some(tris) = indices.get_mut(first as usize..(first + count) as usize) {
                for t in tris.chunks_exact_mut(3) {
                    t[1] = t[0];
                    t[2] = t[0];
                }
            }
        };
        let masked = |visible: &[bool], owners: &std::collections::HashSet<String>, terrains: &std::collections::HashSet<String>| {
            let mut indices = self.world_indices.clone();
            for ranges in &self.world_node_ranges {
                for &(node, first, count) in ranges {
                    if node != u32::MAX && !visible.get(node as usize).copied().unwrap_or(false) {
                        flatten(&mut indices, first, count);
                    }
                }
            }
            // Static geometry actors this scene did not draw (those of the sky zone
            // are only seen by the sky scene).
            for (i, b) in self.world.batches.iter().enumerate() {
                if self.world_owners.get(i).cloned().flatten().is_some_and(|o| !owners.contains(&o)) {
                    flatten(&mut indices, b.first_index, b.index_count);
                }
            }
            for a in &self.actors {
                if a.owner.as_ref().is_some_and(|o| !owners.contains(o)) {
                    for &(_, first, count) in &a.sections {
                        flatten(&mut indices, first, count);
                    }
                }
            }
            // Terrains this scene does not render (zone not seen, or no bTerrainZone).
            let (names_list, ranges_list) = &self.world_terrains;
            for &(t, first, count) in ranges_list {
                if names_list.get(t).is_none_or(|n| !terrains.contains(n)) {
                    flatten(&mut indices, first, count);
                }
            }
            indices
        };
        // Cumulative non-collapsed triangles (an original triangle already reduced to a point counts
        // as collapsed: it does not produce any pixel either).
        let kept = |indices: &[u32]| {
            let mut v = Vec::with_capacity(indices.len() / 3 + 1);
            let mut n = 0u32;
            v.push(0);
            for t in indices.chunks_exact(3) {
                n += (t[0] != t[1] || t[1] != t[2]) as u32;
                v.push(n);
            }
            v
        };
        let size = (self.world_indices.len().max(1) * 4) as u64;
        let previous = self.last_bsp_visibility.as_ref()
            .filter(|_| self.world_masked.as_ref().is_some_and(|(a, b)| a.size() == size && b.size() == size));
        let (buf_a, buf_b) = match self.world_masked.take() {
            Some((x, y)) if x.size() == size => (x, y),
            _ => {
                let make = |label| {
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                };
                self.world_masked_mirror = (Vec::new(), Vec::new());
                (make("cut map"), make("cut sky"))
            }
        };
        let compact = false;
        let mut fresh = false;
        let (compact_a, compact_b) = match self.world_compact.take() {
            Some((x, y)) if compact && x.size() == size => (Some(x), Some(y)),
            _ if compact => {
                let make = |label| {
                    self.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(label),
                        size,
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                };
                fresh = true;
                (Some(make("compact map")), Some(make("compact sky")))
            }
            _ => (None, None),
        };
        // New copies: both buffers are rewritten.
        let previous = previous.filter(|_| !fresh);
        if !self.world_indices.is_empty() && !previous.is_some_and(|p| p.root == d.root && p.root_owners == d.root_owners && p.root_terrains == d.root_terrains) {
            let a = masked(&d.root, &d.root_owners, &d.root_terrains);
            write_ranges(&self.queue, &buf_a, &mut self.world_masked_mirror.0, &a);
            if let Some(c) = &compact_a {
                write_value(&self.queue, c, 0, bytemuck::cast_slice(&compact_indices(&a)));
            }
            self.world_masked_kept.0 = kept(&a);
        }
        if !self.world_indices.is_empty() && !previous.is_some_and(|p| p.sky == d.sky && p.sky_owners == d.sky_owners && p.sky_terrains == d.sky_terrains) {
            let b = masked(&d.sky, &d.sky_owners, &d.sky_terrains);
            write_ranges(&self.queue, &buf_b, &mut self.world_masked_mirror.1, &b);
            if let Some(c) = &compact_b {
                write_value(&self.queue, c, 0, bytemuck::cast_slice(&compact_indices(&b)));
            }
            self.world_masked_kept.1 = kept(&b);
        }
        self.world_compact = compact_a.zip(compact_b);
        self.world_masked = Some((buf_a, buf_b));
        self.last_bsp_visibility = Some(d);
    }

    /// Map scenery sorted by the engine's visibility: for each map batch (order of
    /// `World::batches`), the ranges (first index, count) of the map's index buffer to
    /// draw this frame. None: all the scenery (no sorting).
    #[allow(dead_code)]
    pub fn set_world_ranges(&mut self, ranges: Option<&[Vec<(u32, u32)>]>) {
        let Some(ranges) = ranges else {
            self.world_culled = None;
            return;
        };
        // Full copy first: the StaticMeshes drawn actor by actor (push_cartoon,
        // Which::World) keep their original indices; the kept ranges follow.
        let mut indices: Vec<u32> = self.world_indices.clone();
        let mut batches_list = Vec::with_capacity(self.world.batches.len());
        for b in 0..self.world.batches.len() {
            let first = indices.len() as u32;
            for &(start, count) in ranges.get(b).map(|v| v.as_slice()).unwrap_or(&[]) {
                if let Some(slice) = self.world_indices.get(start as usize..(start + count) as usize) {
                    indices.extend_from_slice(slice);
                }
            }
            batches_list.push((first, indices.len() as u32 - first));
        }
        let size = (indices.len().max(1) * 4) as u64;
        let buffer = match self.world_culled.take() {
            Some((buf, _)) if buf.size() >= size => buf,
            _ => self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sorted map"),
                size,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        };
        if !indices.is_empty() {
            write_value(&self.queue, &buffer, 0, bytemuck::cast_slice(&indices));
        }
        self.world_culled = Some((buffer, batches_list));
    }

    /// Dynamic lights of the level this frame (ULevel+0x1118, FLight::Refresh then Init),
    /// after the map's lights (the leaf lists keep their indices).
    pub fn set_dyn_lights(&mut self, records: &[openxiii_assets::model::LightRecord]) {
        dynamic_lights::refresh_value(
            &mut self.lights, self.model_lights, records,
            self.lightmaps.brightness, self.lightmaps.time, self.frame.count,
        );
    }

    /// Trails of the frame, added to the particles by `set_particles`.
    pub fn set_trails(&mut self, trails: Vec<openxiii_vm::trail_render::TrailBatch>) {
        self.trails = trails;
    }

    pub fn set_particles(&mut self, batches: &[openxiii_vm::particles::SpriteBatch], meshes: &[openxiii_vm::particles::MeshBatch], library: &mut Library) {
        let mut vertices = Vec::new();
        let keep_indices = merge_particles_requested();
        let mut indices = if keep_indices { std::mem::take(&mut self.particle_indices) } else { Vec::new() };
        indices.clear();
        self.particle_packed_active = false;
        let mut batches_list = Vec::new();
        let bound_particles = particles_frustum() || (!self.scene_light_views.targets.is_empty() && target_particle_clip());
        let bounds = |vertices: &[SceneVertex], indices: &[u32], base: u32, delayed: u8| {
            if bound_particles && delayed == 0 && indices.iter().all(|&i| i >= base && (i - base) < vertices.len() as u32) {
                target_clip::Bounds::from_world(vertices.iter().map(|v| v.position))
            } else { None }
        };
        // ATrail::Render: vertices in world coordinates, D3DCOLOR (bytes B, G, R, A).
        for t in std::mem::take(&mut self.trails) {
            let base = vertices.len() as u32;
            let first = indices.len() as u32;
            for v in &t.vertices {
                vertices.push(SceneVertex { position: v.position, normal: [0.0; 3], color: v.color.to_le_bytes(), uv0: v.uv, uv1: [0.0; 2] });
            }
            indices.extend(t.indices.iter().map(|i| base + i));
            let texture = t.texture.unwrap_or_else(|| "#white".to_string());
            batches_list.push(ParticleBatch {
                bounds: if t.lines { None } else { bounds(&vertices[base as usize..], &indices[first as usize..], base, t.delayed) },
                material: t.material.unwrap_or_else(|| format!("#trail:{}|{texture}", t.frame_buffer_blending)),
                kind: ParticleBatchKind::Trail { lines: t.lines },
                ignore_fog: false,
                first,
                count: indices.len() as u32 - first,
                delayed: t.delayed,
                ignore_vignette: t.ignore_vignette,
                actor: t.actor,
                frame: None,
            });
        }
        for b in batches {
            let Some(texture) = &b.texture else { continue };
            let base = vertices.len() as u32;
            let first = indices.len() as u32;
            for v in &b.vertices {
                let c = v.color;
                vertices.push(SceneVertex { position: v.position, normal: [0.0; 3], color: [c[2], c[1], c[0], c[3]], uv0: v.uv, uv1: [0.0; 2] });
            }
            if b.indices.is_empty() {
                for q in 0..(b.vertices.len() / 4) as u32 {
                    let i = base + q * 4;
                    indices.extend([i, i + 1, i + 2, i + 2, i + 1, i + 3]);
                }
            } else {
                indices.extend(b.indices.iter().map(|i| base + i));
            }
            batches_list.push(ParticleBatch {
                bounds: bounds(&vertices[base as usize..], &indices[first as usize..], base, b.delayed),
                material: texture.clone(),
                kind: ParticleBatchKind::Particle { draw_style: b.draw_style, two_sided: true },
                ignore_fog: b.ignore_fog,
                first,
                count: indices.len() as u32 - first,
                delayed: b.delayed,
                ignore_vignette: b.ignore_vignette,
                actor: b.actor,
                frame: b.frame_texture.clone(),
            });
        }
        // UMeshEmitter::RenderParticles: each particle's mesh, section by section.
        for m in meshes {
            if !self.particle_meshes.contains_key(&m.static_mesh) {
                let loaded = library.resolve(&m.static_mesh).and_then(|(loaded, index)| {
                    let mesh = openxiii_assets::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
                    let materials: Vec<String> = mesh
                        .materials
                        .iter()
                        .map(|&r| if r == 0 { String::new() } else { loaded.package.object_path(r.into()) })
                        .map(|p| if p.contains('.') || p.is_empty() { p } else { format!("{}.{p}", loaded.name) })
                        .collect();
                    Some((mesh, materials))
                });
                self.particle_meshes.insert(m.static_mesh.clone(), loaded);
            }
            let Some(Some((mesh, materials))) = self.particle_meshes.get(&m.static_mesh) else { continue };
            let zone = openxiii_assets::world::point_zone(&self.region_nodes, self.root_outside, m.location);
            let ambient = self.zones.get(zone).map_or([0; 3], |z| z.ambient);
            for (s, section) in mesh.sections.iter().enumerate() {
                let material = materials.get(s).cloned().unwrap_or_default();
                for inst in &m.instances {
                    let base = vertices.len() as u32;
                    let c = inst.color;
                    for (i, p) in mesh.positions.iter().enumerate() {
                        let position = openxiii_assets::lighting::transform_point(&inst.local_to_world, *p);
                        let normal = openxiii_assets::lighting::transform_vector(&inst.local_to_world, mesh.normals[i]);
                        let uv0 = mesh.uvs.first().and_then(|u| u.get(i)).copied().unwrap_or([0.0; 2]);
                        // Without UseMeshBlendMode, the global colour (TFACTOR) modulates the texture:
                        // here it goes through the vertex colour.
                        let color = if m.use_mesh_blend_mode { [255; 4] } else { [c[2], c[1], c[0], c[3]] };
                        vertices.push(SceneVertex { position, normal, color, uv0, uv1: [0.0; 2] });
                    }
                    let first = indices.len() as u32;
                    for t in mesh.section_triangles(section) {
                        indices.extend(t.map(|i| base + i as u32));
                    }
                    let kind = if m.use_mesh_blend_mode {
                        ParticleBatchKind::Mesh { tfactor: u32::from_le_bytes(c), ambient }
                    } else {
                        ParticleBatchKind::Particle { draw_style: m.draw_style, two_sided: m.render_two_sided }
                    };
                    batches_list.push(ParticleBatch {
                        bounds: bounds(&vertices[base as usize..], &indices[first as usize..], base, m.delayed),
                        material: material.clone(),
                        kind,
                        ignore_fog: m.ignore_fog,
                        first,
                        count: indices.len() as u32 - first,
                        delayed: m.delayed,
                        ignore_vignette: m.ignore_vignette,
                        actor: m.actor,
                        frame: None,
                    });
                }
            }
        }
        // Buffers of the previous frame reused when they are large enough, like the Canvas ones.
        let prior = self.particles.take().map(|(g, _)| g).or_else(|| self.particles_free.take());
        if batches_list.is_empty() {
            self.particles_free = prior;
        } else {
            self.particles = Some((geometry_kept(&self.device, &self.queue, prior, &vertices, &indices, "particles"), batches_list));
        }
        if keep_indices { self.particle_indices = indices; }
    }

    /// Leaf lists actually processed by in the VM.
    pub fn set_scene_light_leaves(&mut self, views: crate::lights_scene::Views) {
        self.scene_light_passes = views.main.clone();
        self.scene_light_views = views;
        self.active_light_key = None;
    }

    fn light_passes_for_name(&self, name: Option<&str>) -> &[Option<crate::lights_scene::DrawKey>] {
        self.scene_light_views.name_passes(&self.scene_light_passes, name)
    }

    fn light_passes_for_character(&self, owner: usize) -> &[Option<crate::lights_scene::DrawKey>] {
        self.scene_light_views.character_passes(&self.scene_light_passes, owner)
    }

    fn use_light_pass(&mut self, index: Option<crate::lights_scene::DrawKey>) {
        self.active_light_key = index;
    }

    fn active_light_pass(&self) -> Option<&crate::lights_scene::Pass> {
        self.active_light_key.map(|(i, _)| &self.scene_light_passes[i])
    }

    fn active_light_draw(&self) -> Option<&crate::lights_scene::Draw> {
        self.active_light_key.map(|(i, j)| &self.scene_light_passes[i].draws[j])
    }

    /// Cartoon data of the previous frame, to rewrite in place (`Game::character_cartoon_in`).
    pub fn cartoon_mut(&mut self) -> &mut Vec<Option<openxiii_assets::cartoon::CharacterCartoon>> {
        &mut self.cartoon
    }

    /// Updates the animated vertices (same count as at `set_dynamic`).
    /// Shadow silhouette vertices of the frame (see `Dynamic::shadow_vertices`).
    /// `set_shadow_vertices` without re-uploading when the same shared vertices (`id`) are already at `first`.
    pub fn set_shadow_vertices_id(&mut self, first: usize, vertices: &[openxiii_assets::world::Vertex], id: Option<std::rc::Rc<dyn std::any::Any>>) {
        let Some(d) = &mut self.dynamic else { return };
        match id {
            Some(id) => match d.shadow_sent.get(&first) {
                Some(old) if std::ptr::addr_eq(std::rc::Rc::as_ptr(old), std::rc::Rc::as_ptr(&id)) => return,
                // The kept Rc prevents another allocation from taking this address.
                _ => { d.shadow_sent.insert(first, id); }
            },
            None => { d.shadow_sent.remove(&first); }
        }
        self.set_shadow_vertices(first, vertices);
    }

    pub fn set_shadow_vertices(&mut self, first: usize, vertices: &[openxiii_assets::world::Vertex]) {
        if let Some(d) = &self.dynamic {
            let gpu: Vec<SceneVertex> =
                vertices.iter().map(|v| SceneVertex { position: v.position, normal: v.normal, color: [255; 4], uv0: v.uv, uv1: [0.0; 2] }).collect();
            let offset = (first * std::mem::size_of::<SceneVertex>()) as u64;
            let bytes = bytemuck::cast_slice(&gpu);
            if offset + bytes.len() as u64 <= d.shadow_vertices.size() {
                write_value(&self.queue, &d.shadow_vertices, offset, bytes);
            }
        }
    }

    /// The two views share indices, batches and poses, but show different actors.
    /// On returning from the texture, the main buffer still contains its last vertices.
    pub fn use_target_vertices(&mut self, target: bool) {
        let Some(d) = &mut self.dynamic else { return };
        if target == d.target_active {
            return;
        }
        let count = d.updates.uploaded.len();
        if target && d.inactive_vertices.as_ref().is_none_or(|(_, u)| u.uploaded.len() < count) {
            // Same margin as set_dynamic for actor appearances. The initial content
            // guaranteed to be zero by wgpu serves as the mirror: no separate initialisation transfer.
            let capacity = count + count / 2;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("animated vertices of the render textures"),
                size: (capacity * std::mem::size_of::<SceneVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            d.inactive_vertices = Some((buffer, VertexUpdates::new(vec![SceneVertex::zeroed(); capacity])));
        }
        let (buffer, updates) = d.inactive_vertices.as_mut().unwrap();
        std::mem::swap(&mut d.geometry.vertex_buffer, buffer);
        std::mem::swap(&mut d.updates, updates);
        d.target_active = target;
    }

    pub fn update_dynamic(&mut self, frame: &CharacterFrame<'_>, overlay: &[u8]) {
        if let Some(d) = &mut self.dynamic {
            for (k, o) in overlay.iter().enumerate() {
                if let Some(slot) = d.overlay.get_mut(k) {
                    *slot = *o;
                }
            }
            d.updates.write_keyed(frame.visible_meshes_keyed(), &self.prop_colors, |first, vertices| {
                let offset = (first * std::mem::size_of::<SceneVertex>()) as u64;
                write_value(&self.queue, &d.geometry.vertex_buffer, offset, bytemuck::cast_slice(vertices));
            });
        }
    }

    /// Width / height of the whole window.
    pub fn window_aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }

    /// V-sync mode (video_settings::vsync_mode, read before each frame): the surface is
    /// configured again only when the presentation changes.
    pub fn set_vsync(&mut self, vsync: openxiii_vm::video_settings::VsyncMode) {
        let mode = present_mode(vsync);
        if let Some(config) = &mut self.surface_lost {
            config.present_mode = mode;
        }
        if let Some((surface, config)) = &mut self.surface {
            if config.present_mode != mode {
                config.present_mode = mode;
                surface.configure(&self.device, config);
                diag(&format!("presentation {:?}", config.present_mode));
                diag(&vsync_line(vsync, mode));
            }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        if let Some((surface, config)) = &mut self.surface {
            config.width = width;
            config.height = height;
            surface.configure(&self.device, config);
        }
        self.depth = depth_view(&self.device, width, height);
        self.world_target = None;
    }

    /// Image quality settings (render_quality.rs), applied at once: anisotropy rebuilds the
    /// texture groups, MSAA the multisampled pipelines, the render scale the world's target.
    /// Values the device does not allow are lowered (`render_quality_applied`).
    pub fn set_render_quality(&mut self, quality: crate::render_quality::RenderQuality) {
        let quality = quality.sanitized();
        if quality != self.quality {
            let q = self.quality_caps.effective(quality, self.width, self.height);
            diag(&format!(
                "image quality: render scale {} %, MSAA x{}, anisotropy x{} (requested {} %, x{}, x{}; device MSAA {:?})",
                q.render_scale, q.msaa, q.anisotropy, quality.render_scale, quality.msaa, quality.anisotropy, self.quality_caps.msaa_counts
            ));
        }
        self.quality = quality;
        let q = self.render_quality_applied();
        if q.anisotropy != self.anisotropy {
            self.anisotropy = q.anisotropy;
            self.flush_bindings();
        }
        if q.msaa != self.samples {
            self.set_samples(q.msaa);
        }
    }

    /// Shadows line of the Video menu: side of the shadow targets, 0 for Auto (applied at the
    /// next frame; the faithful build keeps 256).
    pub fn set_shadow_quality(&mut self, side: u32) {
        self.shadow_quality = side;
    }

    /// Settings as requested (last `set_render_quality`).
    #[allow(dead_code)]
    pub fn render_quality(&self) -> crate::render_quality::RenderQuality {
        self.quality
    }

    /// Settings as applied, after the device's limits.
    pub fn render_quality_applied(&self) -> crate::render_quality::RenderQuality {
        self.quality_caps.effective(self.quality, self.width, self.height)
    }

    /// MSAA sample counts the device supports for the frame (1 included).
    pub fn supported_msaa_counts(&self) -> &[u32] {
        &self.quality_caps.msaa_counts
    }

    /// Largest render scale (percent) for the current frame size.
    pub fn max_render_scale_percent(&self) -> u32 {
        let limits = wgpu::Limits { max_texture_dimension_2d: self.quality_caps.max_texture_dimension, ..wgpu::Limits::downlevel_webgl2_defaults() };
        crate::render_quality::max_render_scale_percent(self.width, self.height, &limits)
    }

    /// Anisotropic filtering available on this device.
    pub fn anisotropy_supported(&self) -> bool {
        self.quality_caps.anisotropy
    }

    fn set_quality_caps(&mut self, caps: crate::render_quality::QualityCaps) {
        diag(&format!("image quality: device MSAA {:?}, anisotropic filtering {}, textures up to {}", caps.msaa_counts, caps.anisotropy, caps.max_texture_dimension));
        self.quality_caps = caps;
        let q = self.quality;
        self.set_render_quality(q);
    }

    /// The samplers' anisotropy changed: texture groups and samplers created again on their
    /// next use (as after UEngine::Flush, textures and pipelines kept).
    fn flush_bindings(&mut self) {
        self.commands.clear();
        self.bind_keys.clear();
        self.bind_fingerprints.clear();
        self.bind_key_list.clear();
        self.bind_groups.clear();
        self.kept_particles.clear();
        self.samplers.clear();
        for b in self.world.batches.iter_mut()
            .chain(self.replaced.iter_mut().flat_map(|(g, _)| &mut g.batches))
            .chain(self.dynamic.iter_mut().flat_map(|d| &mut d.geometry.batches)) {
            b.prepared = None;
        }
        self.projector_packets.clear();
        self.white_group = None;
    }

    /// New sample count of the world target: multisampled variants of every pipeline (scene and
    /// filter tiles), created now rather than in the middle of a frame.
    fn set_samples(&mut self, samples: u32) {
        self.samples = samples;
        self.pipelines_msaa.clear();
        self.world_target = None;
        self.hud.msaa = None;
        if samples <= 1 {
            return;
        }
        self.hud.msaa = Some((samples, hud_pipelines(&self.device, &self.hud.shader, &self.hud.layout, self.color_format, samples)));
        let keys: Vec<PipelineKey> = self.pipelines.iter().map(|p| p.0).collect();
        let (device, layout, shader, impact_shader, format) = (&self.device, &self.scene_layout, &self.scene_shader, &self.impact_shader, self.color_format);
        let create = |k: &PipelineKey| Self::create_pipeline(device, layout, if k.impact { impact_shader } else { shader }, shader, format, k, samples);
        let variants: Vec<wgpu::RenderPipeline> = if CREATION_PARALLEL.load(std::sync::atomic::Ordering::Relaxed) && keys.len() > 1 {
            let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
            let chunk = keys.len().div_ceil(threads);
            std::thread::scope(|s| {
                let handles: Vec<_> = keys.chunks(chunk).map(|part| s.spawn(move || part.iter().map(create).collect::<Vec<_>>())).collect();
                handles.into_iter().flat_map(|h| h.join().expect("pipeline creation")).collect()
            })
        } else {
            keys.iter().map(create).collect()
        };
        self.pipelines_msaa = variants.into_iter().map(Some).collect();
    }

    /// Pipeline `id` for the target being drawn into (its multisampled variant over the
    /// multisampled world target).
    fn scene_pipeline(&self, id: usize) -> &wgpu::RenderPipeline {
        if self.pass_samples.get() > 1 {
            if let Some(Some(p)) = self.pipelines_msaa.get(id) {
                return p;
            }
        }
        &self.pipelines[id].1
    }

    /// Size of the target the world is drawn into: the frame, or the scaled off-screen target.
    fn world_size(&self) -> (u32, u32) {
        let q = self.render_quality_applied();
        if q.render_scale == 100 && self.samples <= 1 {
            return (self.width, self.height);
        }
        crate::render_quality::scaled_size(self.width, self.height, q.render_scale, self.quality_caps.max_texture_dimension)
    }

    /// Off-screen target of the world (colour and depth views, size), None when the world goes
    /// straight into the frame (render scale 100 %, no MSAA).
    fn world_target_views(&mut self) -> Option<(wgpu::TextureView, wgpu::TextureView, (u32, u32))> {
        let q = self.render_quality_applied();
        if q.render_scale == 100 && self.samples <= 1 {
            self.world_target = None;
            return None;
        }
        let size = self.world_size();
        if !self.world_target.as_ref().is_some_and(|t| t.size == size && t.samples == self.samples) {
            let compositor = self.compositor.get_or_insert_with(|| crate::render_quality::Compositor::new(&self.device, self.color_format));
            self.world_target = Some(compositor.target(&self.device, self.color_format, size, self.samples));
        }
        let t = self.world_target.as_ref()?;
        Some((t.color.clone(), t.depth.clone(), size))
    }

    /// Resolves the world's target and copies its rectangle `src` (texels) into `dst` (pixels)
    /// of the frame `target`.
    fn copy_world(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, src: [f32; 4], dst: [f32; 4]) {
        if let (Some(t), Some(c)) = (&self.world_target, &self.compositor) {
            c.copy(&self.queue, encoder, t, target, (self.width, self.height), src, dst);
        }
    }

    fn fov_y(&self, aspect: f32) -> f32 {
        // As in Unreal: fixed horizontal field of view (the player's FovAngle), otherwise 70° vertical.
        match self.fov {
            Some(h) => 2.0 * ((h.clamp(1.0, 170.0).to_radians() / 2.0).tan() / aspect).atan(),
            None => 70f32.to_radians(),
        }
    }

    /// Hor+ factor of the window (openxiii_vm::display_format, improved build): `None` in 4:3,
    /// narrower, in the faithful build, or in local split screen (`overlay_aspect_cap` set: the
    /// views' angle comes from coop::split_view_fov, not widened a second time).
    /// Also `None` while a render texture or a portal is drawn: their projection uses the camera's
    /// own ViewFOV, so the outline thickness must use it as well.
    fn view_widen(&self) -> Option<f32> {
        if self.overlay_aspect_cap.is_some() || self.in_panel || self.in_camera_view {
            return None;
        }
        openxiii_vm::display_format::hor_plus(self.width, self.height, self.session_graphics)
    }

    /// `fov_y` of a player's view on the screen (main view, split-screen view): on a window wider
    /// than 4:3, the vertical angle of a 4:3 window of the same height (the sides show more).
    fn view_fov_y(&self, aspect: f32) -> f32 {
        match (self.fov, self.view_widen()) {
            (Some(h), Some(k)) => 2.0 * ((h.clamp(1.0, 170.0).to_radians() / 2.0).tan() * k / aspect).atan(),
            _ => self.fov_y(aspect),
        }
    }

    fn frame_uniform(camera: &Camera, fov_y: f32, aspect: f32, near: f32, far: f32, fog: Option<(f32, f32, [u8; 3])>) -> FrameUniform {
        let [x, _, z] = camera.axes();
        // Conversion to right-handed: Y flipped (as in the shader).
        let eye = [camera.position[0], -camera.position[1], camera.position[2]];
        let view = look_to_rh(eye, [x[0], -x[1], x[2]], [z[0], -z[1], z[2]]);
        let (fog, fog_color) = match fog {
            Some((start, end, c)) => ([1.0, start, end, 0.0], [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 0.0]),
            None => ([0.0; 4], [0.0; 4]),
        };
        let proj = perspective_rh(fov_y, aspect, near, far);
        FrameUniform { view_proj: mul(&proj, &view), view: d3d_view(camera), fog, fog_color, ambient: [0.0; 4] }
    }

    /// `widen`: Hor+ factor of a view on the screen (`view_widen`), also applied to the
    /// bSpecialDelayFov projection; `None` for portals and render textures.
    fn update_uniforms_with(&self, camera: &Camera, fov_y: f32, aspect: f32, widen: Option<f32>, first_person: Option<&Camera>) -> (Mat, Option<Mat>) {
        // FCameraSceneNode::UpdateMatrices: planes at 5 and 65536.
        let mut uniforms = Self::frame_uniform(camera, fov_y, aspect, NEAR_PLANE, FAR_PLANE, self.fog);
        uniforms.view_proj = lens_shifted(uniforms.view_proj, self.lens);
        write_value(&self.queue, &self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
        // The first-person models' camera (`overlay_camera`): the scene's unless given.
        let camera = first_person.unwrap_or(camera);
        if first_person.is_some() {
            let mut delayed = Self::frame_uniform(camera, fov_y, aspect, NEAR_PLANE, FAR_PLANE, self.fog);
            delayed.view_proj = lens_shifted(delayed.view_proj, self.lens);
            write_value(&self.queue, &self.delayed_frame_buffer, 0, bytemuck::bytes_of(&delayed));
        } else {
            write_value(&self.queue, &self.delayed_frame_buffer, 0, bytemuck::bytes_of(&uniforms));
        }
        // bSpecialDelayFov actors: with a horizontal half-angle
        // of 0x3EDF65BE (25°) instead of FovAngle, same planes.
        // Local split screen: first-person models keep the height they have in the whole window
        // (the old co-op mod's 86° across a 32:9 half of a 16:9 window); never widened by Hor+
        // there (`view_widen`).
        let special_aspect = self.overlay_aspect_cap.map_or(aspect, |cap| aspect.min(cap));
        let special = match widen {
            Some(k) => 2.0 * (f32::from_bits(0x3edf_65be).tan() * k / special_aspect).atan(),
            None => 2.0 * (f32::from_bits(0x3edf_65be).tan() / special_aspect).atan(),
        };
        let mut overlay = Self::frame_uniform(camera, special, aspect, NEAR_PLANE, FAR_PLANE, self.fog);
        overlay.view_proj = lens_shifted(overlay.view_proj, self.lens);
        write_value(&self.queue, &self.overlay_frame_buffer, 0, bytemuck::bytes_of(&overlay));
        // Split screen (never widened by Hor+): a view wider than the cap shows the sides past
        // the 4:3 picture as well, where the bazooka's tube is cut by the usual near plane.
        let eye = widen.is_some() || (!self.in_panel && !self.in_camera_view && aspect > special_aspect);
        self.eye_view.set(eye);
        if eye {
            let mut eye = Self::frame_uniform(camera, special, aspect, NEAR_PLANE_EYE, FAR_PLANE, self.fog);
            eye.view_proj = lens_shifted(eye.view_proj, self.lens);
            write_value(&self.queue, &self.overlay_eye_frame_buffer, 0, bytemuck::bytes_of(&eye));
        } else {
            write_value(&self.queue, &self.overlay_eye_frame_buffer, 0, bytemuck::bytes_of(&overlay));
        }
        let sky = self.sky.as_ref().map(|sky| {
            let mut u = Self::frame_uniform(sky, fov_y, aspect, NEAR_PLANE, FAR_PLANE, self.sky_fog);
            u.view_proj = lens_shifted(u.view_proj, self.lens);
            write_value(&self.queue, &self.sky_frame_buffer, 0, bytemuck::bytes_of(&u));
            u.view_proj
        });
        self.particles_projection.set(Some((uniforms.view_proj, sky)));
        (uniforms.view_proj, sky)
    }

    /// DrawBits::zone_fog of a zone (fog of its ZoneInfo, none if it lacks bDistanceFog).
    fn zone_fog_bits(&self, zone: u8) -> [u32; 4] {
        // Only when the camera zone's ZoneInfo has bFogPerZone (otherwise).
        if !self.fog_per_zone { return [0; 4]; }
        crate::d3d::zone_fog_bits(self.zones.get(zone as usize).and_then(|z| z.fog))
    }

    /// Camera's zone (UModel::PointRegion): fog (FLevelSceneNode::Render,
    /// SetDistanceFog) and view of its sky zone (FSkySceneNode: same rotation, placed at the
    /// SkyZoneInfo). The sky is only seen through the PF_FakeBackdrop surfaces, which are not drawn.
    /// `volume`: the fog of the view's volume takes precedence (except with UseVignetteFilter).
    fn update_zone(&mut self, camera: &Camera, volume: bool) {
        let zone = openxiii_assets::world::point_zone(&self.region_nodes, self.root_outside, camera.position);
        let z = self.zones.get(zone).cloned().unwrap_or_default();
        self.fog = self.volume_fog.filter(|_| volume).or(z.fog);
        self.fog_per_zone = z.fog_per_zone;
        // UGameEngine::Draw: Clear(1, colour, …) before FCameraSceneNode::Render.
        self.clear_color = z.clear.map(|c| self.clear_volume.unwrap_or(c)).unwrap_or([0; 3]);
        self.force_clear = z.clear.is_some();
        self.sky = z.sky.map(|(location, rotation, _)| camera.sky_view(location, rotation));
        self.sky_fog = z.sky.and_then(|(_, _, sz)| self.zones.get(sz as usize)).and_then(|zr| zr.fog);
    }

    fn sampler_for(&mut self, u: u32, v: u32, mode: u8) -> &wgpu::Sampler {
        let device = &self.device;
        // Anisotropic filtering (render_quality.rs) on the world's linear samplers only.
        let anisotropy = if mode == SAMPLER_WORLD { self.anisotropy.max(1) } else { 1 };
        self.samplers.entry((u, v, mode)).or_insert_with(|| {
            let filter = if mode == SAMPLER_POINT { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear };
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("stage"),
                address_mode_u: d3d::address(u),
                address_mode_v: d3d::address(v),
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            })
        })
    }

    /// Improved build: the menu's scale (screen pixels per menu unit) the onomatopoeias are
    /// to be prepared at (from the display, every frame).
    pub fn set_onomatopoeia_scale(&mut self, scale: f32) {
        self.onomatopoeia_scale = scale;
    }

    /// Window size (pixels).
    pub fn screen_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Testing: onomatopoeia textures in the cache (one series expected).
    pub fn onomatopoeia_textures(&self) -> usize {
        self.textures.keys().filter(|k| k.starts_with("#ono:")).count()
    }

    /// A texture no longer used: out of the cache, and the groups that bound it bound again
    /// without it (their key no longer names it), so its video memory is freed.
    fn drop_texture(&mut self, library: &mut Library, name: &str) {
        if self.textures.remove(name).is_none() {
            return;
        }
        for i in 0..self.bind_key_list.len().min(self.bind_groups.len()) {
            if self.bind_key_list[i].iter().any(|(t, ..)| t.as_deref() == Some(name)) {
                let old = self.bind_key_list[i].clone();
                self.bind_keys.remove(&old);
                let mut key = old;
                for slot in key.iter_mut() {
                    if slot.0.as_deref() == Some(name) {
                        slot.0 = None;
                    }
                }
                self.bind_groups[i] = self.make_bind_group(library, &key);
                self.bind_key_list[i] = key;
            }
        }
    }

    fn texture(&mut self, library: &mut Library, path: &str) -> Option<wgpu::TextureView> {
        // GEngine->WhiteTextureMaterial, the script's render textures.
        if path == "#white" {
            return Some(self.white.clone());
        }
        if path.strip_prefix('#').is_some_and(|n| n.parse::<usize>().is_ok()) {
            return self.hud.targets.get(path).map(|t| t.view.clone());
        }
        if let Some(n) = path.strip_prefix("#shadowtex:") {
            // A projection's material can be prepared (bind group made and kept) before the
            // frame's silhouettes are drawn: a target beyond the eight is made here too, so it
            // never falls back to the white texture (a uniform dark square under the shadow).
            let i = n.split(':').next().and_then(|n| n.parse::<usize>().ok())?;
            self.grow_shadow_targets(i + 1);
            return self.shadow_targets.get(i).map(|t| t.0.clone());
        }
        self.refresh_runtime_textures(library);
        if !self.textures.contains_key(path) {
            let t0 = std::time::Instant::now();
            let view = match path.strip_prefix("#ramp:") {
                // Cartoon rendering ramp, one per lighting curve.
                Some(curve) => curve.parse::<u32>().ok().map(|c| upload(&self.device, &self.queue, &[(32, 32, openxiii_assets::cartoon::ramp(c))])),
                // An onomatopoeia's prepared size no longer kept: the game's own picture.
                None => match path.strip_prefix("#ono:").and_then(|r| r.split(':').next()) {
                    Some(name) => bitmap_mips(library, &format!("XIIIMenuStart.{name}")).map(|m| upload(&self.device, &self.queue, &m)),
                    None => bitmap_mips(library, path).map(|m| upload(&self.device, &self.queue, &m)),
                },
            };
            if view.is_none() {
                eprintln!("unreadable texture: {path}");
            }
            #[cfg(feature = "mods")]
            if let Some(m) = openxiii_assets::mod_textures::get(path).filter(|_| view.is_some()) {
                self.mod_uploaded.insert(path.to_string(), m);
            }
            self.textures.insert(path.to_string(), view);
            self.fresh_list.0[1] += 1;
            self.fresh_list.1[1] += t0.elapsed().as_secs_f64();
        }
        self.textures[path].clone()
    }

    /// Mods and improved builds: textures the game made or replaced while it runs (the improved menus'
    /// pictures, openxiii_assets::mod_textures::set_runtime) since the last look. A name asked
    /// for before its picture existed (the prewarm list at map load) or whose picture was
    /// replaced is read again, and the groups that hold it are remade in place (same rank: the
    /// passes keep their group numbers). Without this, such a texture stayed missing (drawn
    /// white) or old for the whole run.
    fn refresh_runtime_textures(&mut self, library: &mut Library) {
        #[cfg(feature = "mods")]
        {
            let generation = openxiii_assets::mod_textures::runtime_generation();
            if generation == self.mod_generation {
                return;
            }
            self.mod_generation = generation;
            let stale: Vec<String> = self
                .textures
                .iter()
                .filter(|(name, view)| match openxiii_assets::mod_textures::get(name) {
                    // A picture that replaced a game texture and was taken away: the game's own
                    // texture is read again.
                    None => self.mod_uploaded.contains_key(*name),
                    Some(now) => view.is_none() || !self.mod_uploaded.get(*name).is_some_and(|was| Arc::ptr_eq(was, &now)),
                })
                .map(|(name, _)| name.clone())
                .collect();
            if stale.is_empty() {
                return;
            }
            for name in &stale {
                self.textures.remove(name);
                self.mod_uploaded.remove(name);
            }
            for i in 0..self.bind_key_list.len().min(self.bind_groups.len()) {
                if self.bind_key_list[i].iter().any(|(t, ..)| t.as_ref().is_some_and(|t| stale.contains(t))) {
                    let key = self.bind_key_list[i].clone();
                    self.bind_groups[i] = self.make_bind_group(library, &key);
                }
            }
        }
        #[cfg(not(feature = "mods"))]
        let _ = library;
    }

    fn make_bind_group(&mut self, library: &mut Library, key: &BindKey) -> wgpu::BindGroup {
        let views: Vec<wgpu::TextureView> =
            key.iter().map(|(t, _, _, _)| t.as_ref().and_then(|p| self.texture(library, p)).unwrap_or_else(|| self.white.clone())).collect();
        let samplers: Vec<wgpu::Sampler> = key.iter().map(|(_, u, v, mode)| self.sampler_for(*u, *v, *mode).clone()).collect();
        let mut entries = Vec::new();
        for (i, v) in views.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: i as u32, resource: wgpu::BindingResource::TextureView(v) });
        }
        for (i, s) in samplers.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: (STAGES + i) as u32, resource: wgpu::BindingResource::Sampler(s) });
        }
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("stages"), layout: &self.stage_layout, entries: &entries })
    }

    fn bind_group(&mut self, library: &mut Library, key: BindKey) -> usize {
        self.refresh_runtime_textures(library);
        if let Some(&i) = self.bind_keys.get(&key) {
            return i;
        }
        let t0 = std::time::Instant::now();
        let group = self.make_bind_group(library, &key);
        openxiii_package::events::log("bind_group_key", || key_bind_text(&key), String::new);
        self.bind_groups.push(group);
        while self.bind_key_list.len() + 1 < self.bind_groups.len() {
            self.bind_key_list.push(Default::default());
        }
        self.bind_key_list.push(key.clone());
        self.bind_keys.insert(key, self.bind_groups.len() - 1);
        self.fresh_list.0[2] += 1;
        self.fresh_list.1[2] += t0.elapsed().as_secs_f64();
        self.bind_groups.len() - 1
    }

    /// Programs of the combiner read from the uniforms, one per stage count, compiled once (pipelines
    /// that only differ by render states reuse them). Created in parallel: on slow drivers this takes
    /// from tens to hundreds of ms per program (the more stages, the longer), over a second of loading
    /// if done one after another.
    fn generic_precompile(&mut self) {
        let t0 = std::time::Instant::now();
        // Impact programs (vs_impacts, P2) too: the first impact of a game compiles
        // nothing (pipelines that only differ by states reuse these programs).
        let impacts = matrices_impacts();
        let keys_list: Vec<PipelineKey> = [false, true]
            .into_iter()
            .filter(|&impact| !impact || impacts)
            .flat_map(|impact| (0..=STAGES as u8).map(move |stage_count| PipelineKey { impact, stage_count, blend: None, z_test: true, z_write: true, cull: 2, lines: false, z_bias: 0, combine: None, alpha_test: false }))
            .filter(|k| !self.pipeline_ids.contains_key(k))
            .collect();
        if !CREATION_PARALLEL.load(std::sync::atomic::Ordering::Relaxed) {
            for key in keys_list {
                self.pipeline(key);
            }
            return;
        }
        let (device, layout, shader, impact_shader, format) = (&self.device, &self.scene_layout, &self.scene_shader, &self.impact_shader, self.color_format);
        let created: Vec<wgpu::RenderPipeline> = std::thread::scope(|s| {
            let threads: Vec<_> = keys_list.iter().map(|k| s.spawn(move || Self::create_pipeline(device, layout, if k.impact { impact_shader } else { shader }, shader, format, k, 1))).collect();
            threads.into_iter().map(|f| f.join().expect("pipeline creation")).collect()
        });
        for (key, pipeline) in keys_list.into_iter().zip(created) {
            self.record_pipeline(key, pipeline, t0);
        }
    }

    fn pipeline(&mut self, key: PipelineKey) -> usize {
        if let Some(&id) = self.pipeline_ids.get(&key) {
            return id;
        }
        // Load prewarm: the key is noted, the pipelines are made together on worker threads
        // (`flush_pipeline_batch`); the prewarm does not use the returned number.
        if let Some(batch) = self.pipeline_batch.as_mut() {
            if !batch.contains(&key) {
                batch.push(key);
            }
            return usize::MAX;
        }
        // Pass seen for the first time during play (flash, gunshot impact): compiling its
        // specialised program would block the frame (tens of ms, much more on slow machines). It
        // takes the combiner read from the uniforms (same computation, program already compiled at
        // load time by stage_count).
        if key.combine.is_some() && self.frame.count.wrapping_sub(*self.first_image.get_or_insert(self.frame.count)) >= IMAGES_SPECIALIZED {
            let id = self.pipeline(PipelineKey { combine: None, ..key });
            self.pipeline_ids.insert(key, id);
            return id;
        }
        // Alpha-tested opaque pass: at one sample, the same pipeline as without the test (only the
        // multisampled variant differs, by its alpha to coverage).
        if key.alpha_test {
            let base = self.pipeline(PipelineKey { alpha_test: false, ..key });
            let pipeline = self.pipelines[base].1.clone();
            return self.record_pipeline(key, pipeline, std::time::Instant::now());
        }
        let t0 = std::time::Instant::now();
        let shader = if key.impact { &self.impact_shader } else { &self.scene_shader };
        let pipeline = Self::create_pipeline(&self.device, &self.scene_layout, shader, &self.scene_shader, self.color_format, &key, 1);
        openxiii_package::events::log("pipeline_key", || key_pipeline_text(&key), || format!("combine={:?} alpha_test={} camera_view={}", key.combine.map(|c| c[..key.stage_count as usize].to_vec()), key.alpha_test, self.in_camera_view));
        self.record_pipeline(key, pipeline, t0)
    }

    /// The pipelines the load prewarm asked for, made on up to four worker threads (the device is
    /// shared; each pipeline is the same as one made alone), then numbered in the order asked.
    fn flush_pipeline_batch(&mut self) {
        let Some(keys) = self.pipeline_batch.take() else { return };
        let t0 = std::time::Instant::now();
        let mut keys: Vec<PipelineKey> = keys.into_iter().filter(|k| !self.pipeline_ids.contains_key(k)).collect();
        // An alpha-tested key shares the pipeline of its key without the test (made first).
        for k in keys.clone() {
            let base = PipelineKey { alpha_test: false, ..k };
            if k.alpha_test && !self.pipeline_ids.contains_key(&base) && !keys.contains(&base) {
                keys.push(base);
            }
        }
        let samples = self.samples;
        let (device, layout, scene, impact, format) = (&self.device, &self.scene_layout, &self.scene_shader, &self.impact_shader, self.color_format);
        let make = |k: &PipelineKey| {
            let shader = if k.impact { impact } else { scene };
            let one = (!k.alpha_test).then(|| Self::create_pipeline(device, layout, shader, scene, format, k, 1));
            let msaa = (samples > 1).then(|| Self::create_pipeline(device, layout, shader, scene, format, k, samples));
            (one, msaa)
        };
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4);
        let chunk = keys.len().div_ceil(threads).max(1);
        let made: Vec<(Option<wgpu::RenderPipeline>, Option<wgpu::RenderPipeline>)> = std::thread::scope(|s| {
            let jobs: Vec<_> = keys.chunks(chunk).map(|part| s.spawn(move || part.iter().map(make).collect::<Vec<_>>())).collect();
            jobs.into_iter().flat_map(|j| j.join().expect("pipeline worker")).collect()
        });
        let made_count = keys.len();
        let mut alpha = Vec::new();
        for (k, (one, msaa)) in keys.into_iter().zip(made) {
            match one {
                Some(p) => {
                    openxiii_package::events::log("pipeline_key", || key_pipeline_text(&k), || format!("combine={:?} alpha_test={} prewarm", k.combine.map(|c| c[..k.stage_count as usize].to_vec()), k.alpha_test));
                    self.record_made(k, p, msaa, t0);
                }
                None => alpha.push((k, msaa)),
            }
        }
        for (k, msaa) in alpha {
            let base = self.pipeline_ids[&PipelineKey { alpha_test: false, ..k }];
            let p = self.pipelines[base].1.clone();
            self.record_made(k, p, msaa, t0);
        }
        if made_count > 0 {
            diag(&format!("prewarm: {made_count} pipelines in {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0));
            crate::pipeline_cache::save();
        }
    }

    fn record_pipeline(&mut self, key: PipelineKey, pipeline: wgpu::RenderPipeline, t0: std::time::Instant) -> usize {
        let msaa = (self.samples > 1).then(|| {
            let shader = if key.impact { &self.impact_shader } else { &self.scene_shader };
            Self::create_pipeline(&self.device, &self.scene_layout, shader, &self.scene_shader, self.color_format, &key, self.samples)
        });
        self.record_made(key, pipeline, msaa, t0)
    }

    fn record_made(&mut self, key: PipelineKey, pipeline: wgpu::RenderPipeline, msaa: Option<wgpu::RenderPipeline>, t0: std::time::Instant) -> usize {
        let id = self.pipelines.len();
        self.pipelines.push((key, pipeline));
        // Multisampled world target: its variant created with it.
        if let Some(variant) = msaa {
            self.pipelines_msaa.resize(id, None);
            self.pipelines_msaa.push(Some(variant));
        }
        self.pipeline_ids.insert(key, id);
        self.fresh_list.0[0] += 1;
        self.fresh_list.1[0] += t0.elapsed().as_secs_f64();
        id
    }

    /// Pipeline of `key` for `samples` samples per pixel (1: the frame, render textures and
    /// shadows; more: the multisampled world target, with alpha to coverage for alpha-tested
    /// opaque passes).
    #[allow(clippy::too_many_arguments)]
    fn create_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, fragment_shader: &wgpu::ShaderModule, color_format: wgpu::TextureFormat, key: &PipelineKey, samples: u32) -> wgpu::RenderPipeline {
        let a2c = samples > 1 && key.alpha_test;
        let mut constants = vec![("STAGE_COUNT", f64::from(key.stage_count))];
        if a2c {
            constants.push(("A2C", 1.0));
        }
        if let Some(c) = key.combine {
            constants.push(("SPECIAL", 1.0));
            for (names_list, vals) in d3d::COMBINE_NAMES.iter().zip(c) {
                for (n, v) in names_list.iter().zip(vals) {
                    constants.push((*n, f64::from(v)));
                }
            }
        }
        let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Float32x2, 4 => Float32x2];
        let impact_attributes = wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let mut buffers = vec![Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SceneVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        if key.impact {
            buffers.push(Some(wgpu::VertexBufferLayout {
                array_stride: 64, step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &impact_attributes,
            }));
        }
        let cache = crate::pipeline_cache::get();
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pass"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(if key.impact { "vs_impacts" } else { "vs_main" }),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: if key.lines { wgpu::PrimitiveTopology::LineList } else { wgpu::PrimitiveTopology::TriangleList },
                cull_mode: d3d::cull_face(key.cull),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(key.z_write),
                // D3DRS_ZFUNC: D3DCMP_LESSEQUAL, or ALWAYS without a depth test.
                depth_compare: Some(if key.z_test { wgpu::CompareFunction::LessEqual } else { wgpu::CompareFunction::Always }),
                stencil: Default::default(),
                // D3DRS_ZBIAS has no scale defined by Direct3D 8 (hardware-specific): one
                // step is 256 depth units of the Depth32Float target here.
                bias: wgpu::DepthBiasState { constant: -(key.z_bias as i32) * 256, slope_scale: 0.0, clamp: 0.0 },
            }),
            multisample: wgpu::MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: a2c },
            fragment: Some(wgpu::FragmentState {
                module: fragment_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: d3d::blend_state(key), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: cache.as_ref(),
        })
    }

    /// Adds a pass to draw: pipeline, texture group, uniforms.
    #[allow(clippy::too_many_arguments)]
    fn push_pass(
        &mut self,
        library: &mut Library,
        pass: &openxiii_assets::material::Pass,
        uniform: PassUniform,
        cull: u8,
        lightmap: &Option<String>,
        geometry: (Which, u32, u32),
        sorted: bool,
        overlay: u8,
    ) {
        let (pipeline, bind) = self.pass_resources(library, pass, &uniform, cull, lightmap);
        self.emit_pass(pipeline, bind, geometry, sorted, overlay);
        self.uniforms.push(uniform);
    }

    fn pass_resources(&mut self, library: &mut Library, pass: &openxiii_assets::material::Pass, uniform: &PassUniform, cull: u8, lightmap: &Option<String>) -> (usize, usize) {
        let pipeline = self.pass_pipeline(pass, uniform, cull, None);
        let bind = self.stage_bind(library, pass, lightmap, None);
        (pipeline, bind)
    }

    fn pass_pipeline(&mut self, pass: &openxiii_assets::material::Pass, uniform: &PassUniform, cull: u8, previous: Option<usize>) -> usize {
        let mut key = PipelineKey::new(pass, cull);
        key.stage_count = uniform.stage_count();
        key.impact = uniform.impact();
        key.z_bias = self.z_bias;
        {
            key.combine = uniform.combine();
        }
        if let Some(id) = previous.filter(|&id| self.pipelines[id].0 == key) { return id; }
        self.pipeline(key)
    }

    fn emit_pass(&mut self, pipeline: usize, bind: usize, geometry: (Which, u32, u32), sorted: bool, overlay: u8) {
        let (which, first_index, index_count) = geometry;
        self.commands.push(DrawCmd {
            pipeline,
            pass_offset: (self.uniforms.len() as u64 * PASS_STRIDE) as u32,
            bind,
            which,
            first_index,
            index_count,
            sorted,
            overlay,
            vignette: self.push_vignette && overlay == 0,
            canvas: false,
            segment: 0,
            order: self.push_order,
            // Opaque pass of a translucent-list actor: drawn before the actor's blended passes.
            opaque_first: sorted && self.pipelines[pipeline].0.blend.is_none(),
            light_sky: self.push_sky.unwrap_or_else(|| self.active_light_pass().map(|p| p.sky_zone.is_some())),
            outside_sky: self.push_outside_sky,
        });
    }

    /// Binds the material's resources once per section. Changing view, rank,
    /// visible leaf or BSP range changes none of the values kept here.
    #[allow(clippy::too_many_arguments)]
    fn prepare_section<'a>(
        &mut self, slot: &'a mut Option<Box<PreparedSection>>, library: &mut Library,
        material: &Rc<openxiii_assets::material::MaterialPasses>, lightmap: &Option<String>,
        cull: u8, bits: DrawBits, dynamic_refresh: bool, lights: &[[[f32; 4]; 4]],
    ) -> &'a PreparedSection {
        let refresh = if dynamic_refresh { self.active_light_draw().and_then(|d| d.refresh.clone()) } else { None };
        let inputs = SectionUniformInputs::new(bits, refresh.as_deref());
        let lights = &lights[..lights.len().min(8)];
        let make_uniform = |pass: &openxiii_assets::material::Pass| {
            let mut u = PassUniform::new(pass, bits);
            u.set_dyn_refresh(refresh.as_deref());
            u.set_lights(lights);
            u
        };
        let packet = slot.get_or_insert_with(|| Box::new(PreparedSection {
            material: material.clone(), cull, bias: self.z_bias, inputs: None, lights: Vec::new(), passes: Vec::new(),
        }));
        let same_material = Rc::ptr_eq(&packet.material, material);
        let same_lighting = packet.inputs.as_ref() == Some(&inputs)
            && bytemuck::cast_slice::<_, u8>(&packet.lights) == bytemuck::cast_slice::<_, u8>(lights);
        let same_cull = packet.cull == cull && packet.bias == self.z_bias;
        if same_material && same_lighting && same_cull { return packet; }

        // An animated material may have redone its translation for this view. Even when the Rc
        // changes, its stages, pipeline and uniforms may stay identical.
        for (index, pass) in material.passes.iter().enumerate() {
            if let Some(p) = packet.passes.get_mut(index) {
                let same_pass = same_material || packet.material.passes.get(index).is_some_and(|old| same_translated_pass(old, pass));
                if !same_lighting || !same_pass {
                    let u = make_uniform(pass);
                    if bytemuck::bytes_of(&u) != bytemuck::bytes_of(p.uniform.as_ref()) {
                        p.uniform = Rc::new(u);
                    }
                }
                if !same_pass || !same_cull {
                    p.pipeline = self.pass_pipeline(pass, &p.uniform, cull, Some(p.pipeline));
                }
                if !same_pass {
                    p.bind = self.stage_bind(library, pass, lightmap, Some(p.bind));
                }
            } else {
                let uniform = Rc::new(make_uniform(pass));
                let (pipeline, bind) = self.pass_resources(library, pass, &uniform, cull, lightmap);
                packet.passes.push(PreparedPass { pipeline, bind, uniform });
            }
        }
        packet.passes.truncate(material.passes.len());
        if !same_material { packet.material = material.clone(); }
        if !same_lighting {
            packet.inputs = Some(inputs);
            packet.lights.clear();
            packet.lights.extend_from_slice(lights);
        }
        packet.cull = cull;
        packet.bias = self.z_bias;
        packet
    }

    /// Same criterion as scene_pass_with, checked in both visibility buffers.
    fn world_range_hidden(&self, first: u32, count: u32) -> bool {
        if self.world_culled.is_some() || self.world_masked.is_none() || first % 3 != 0 || count % 3 != 0 {
            return false;
        }
        let (start, end) = ((first / 3) as usize, ((first + count) / 3) as usize);
        let hidden = |kept: &[u32]| kept.get(end).zip(kept.get(start)).is_some_and(|(b, a)| b == a);
        hidden(&self.world_masked_kept.0) && hidden(&self.world_masked_kept.1)
    }

    /// Group of the textures and samplers of a pass's stages.
    fn stage_bind(&mut self, library: &mut Library, pass: &openxiii_assets::material::Pass, lightmap: &Option<String>, previous: Option<usize>) -> usize {
        self.stage_bind_with(library, pass, lightmap, previous, false)
    }

    /// `screen`: a pass drawn flat on the screen (Canvas), without anisotropic filtering.
    fn stage_bind_with(&mut self, library: &mut Library, pass: &openxiii_assets::material::Pass, lightmap: &Option<String>, previous: Option<usize>, screen: bool) -> usize {
        self.refresh_runtime_textures(library);
        let mode = |st: &openxiii_assets::material::Stage| if st.point_filter { SAMPLER_POINT } else if screen { SAMPLER_SCREEN } else { SAMPLER_WORLD };
        // Stages borrowed from the pass: an existing group is found without copying the names.
        let stages_list: [(Option<&str>, u32, u32, u8); STAGES] = std::array::from_fn(|s| {
            let st = &pass.stages[s];
            let used = s < pass.stages_used && st.color_op != 1;
            let tex = if used {
                match &st.texture {
                    Some(TexRef::Bitmap(p)) => Some(group_texture(p)),
                    Some(TexRef::Lightmap) => lightmap.as_deref(),
                    None => None,
                }
            } else {
                None
            };
            (tex, st.address[0], st.address[1], mode(st))
        });
        let same_check = |key_tuple: &BindKey| key_tuple.iter().zip(&stages_list).all(|((t, u, v, p), (t2, u2, v2, p2))| t.as_deref() == *t2 && u == u2 && v == v2 && p == p2);
        if let Some(i) = previous.filter(|&i| self.bind_key_list.get(i).is_some_and(|k| same_check(k))) { return i; }
        let footprint = {
            use std::hash::{Hash, Hasher};
            let mut h = rustc_hash::FxHasher::default();
            stages_list.hash(&mut h);
            h.finish()
        };
        if let Some(&i) = self.bind_fingerprints.get(&footprint).and_then(|l| l.iter().find(|&&i| self.bind_key_list.get(i).is_some_and(|c| same_check(c)))) {
            return i;
        }
        let mut bind: BindKey = Default::default();
        for (s, slot) in bind.iter_mut().enumerate() {
            let st = &pass.stages[s];
            let used = s < pass.stages_used && st.color_op != 1;
            let tex = if used {
                match &st.texture {
                    Some(TexRef::Bitmap(p)) => Some(group_texture(p).to_string()),
                    Some(TexRef::Lightmap) => lightmap.clone(),
                    None => None,
                }
            } else {
                None
            };
            *slot = (tex, st.address[0], st.address[1], mode(st));
        }
        let i = self.bind_group(library, bind);
        let list_value = self.bind_fingerprints.entry(footprint).or_default();
        if !list_value.contains(&i) {
            list_value.push(i);
        }
        i
    }

    /// Passes of a mesh drawn by DrawCartoonStatic or DrawCartoonSM
    /// cartoon ramp per section (or materials of the normal path if only the outline
    /// is requested), then outline extruded on the back faces.
    #[allow(clippy::too_many_arguments)]
    fn push_cartoon(
        &mut self,
        library: &mut Library,
        view: [f32; 16],
        desc: &openxiii_assets::cartoon::CartoonDesc,
        sections: &[(String, u32, u32)],
        (which, overlay): (Which, u8),
        cull: u8,
        l2w: [f32; 16],
        local_positions: bool,
        state: &DrawState,
    ) {
        let desc = desc.clone();
        // Fog of the actor's zone, for its cartoon and outline passes.
        let zone_fog = state.zone.map_or([0; 4], |z| self.zone_fog_bits(z));
        // Actor of the translucent list (push_order is its rank): all its passes, outline
        // included, are drawn at that rank (table).
        let whole = self.push_order >= 0 && self.push_order != i64::MAX;

        let w2l = openxiii_assets::material::invert(&l2w);
        let refresh=self.active_light_draw().and_then(|d|d.refresh.clone());
        let constants = |u: &mut PassUniform, desc: &openxiii_assets::cartoon::CartoonDesc| {
            u.c4 = [desc.color[0], desc.color[1], desc.color[2], 1.0];
            u.c5 = if desc.i1 != 0.0 { [desc.dir1[0], desc.dir1[1], desc.dir1[2], desc.i1 - 1.0] } else { [0.0; 4] };
            u.c6 = if desc.i2 != 0.0 { [desc.dir2[0], desc.dir2[1], desc.dir2[2], desc.i2 - 1.0] } else { [0.0; 4] };
            u.l2w = l2w;
            u.w2l = w2l;
            if local_positions { u.set_dyn_refresh(refresh.as_deref()); }
        };
        let mut outline_sections = Vec::new();
        // Mods build only: a mod model's outline shell sections (mod_meshes::with_outline_shell)
        // carry the outline of the section they copy, and are not drawn otherwise.
        #[cfg(feature = "mods")]
        let has_shell = sections.iter().any(|(m, _, _)| openxiii_assets::mod_meshes::outline_shell_of(m).is_some());
        #[cfg(not(feature = "mods"))]
        let has_shell = false;
        // A model that reaches the eye, in a view wider than 4:3 (DELAYED_FOV_EYE, `eye_view`):
        // its other parts (the hands) keep the usual near plane, cut in the shader; the whole
        // model shares one depth range, so the hands and the weapon still hide each other as
        // before. In the bazooka's firing recoil the forearm passes next to the eye and would
        // fill the side of the view.
        let eye = overlay == DELAYED_FOV_EYE && self.eye_view.get();
        let near_cut = |material: &str| eye && !openxiii_assets::character::reaches_eye_material(material);
        for (material, first, count) in sections {
            #[cfg(feature = "mods")]
            if let Some(outlined) = openxiii_assets::mod_meshes::outline_shell_of(material) {
                let mat = (!outlined.is_empty()).then_some(outlined);
                if self.materials.cartoon_material(library, mat).outline {
                    outline_sections.push((*first, *count, near_cut(outlined)));
                }
                continue;
            }
            let cut = near_cut(material);
            let mat = (!material.is_empty()).then_some(material.as_str());
            let cm = self.materials.cartoon_material(library, mat);
            if cm.outline && !has_shell {
                outline_sections.push((*first, *count, cut));
            }
            if desc.cartoon {
                // LightCurve 0xFF40 while the FlashEffectDesc is enabled, otherwise
                // that of the actor's zone.
                let curve = if self.cartoon_flash { 0xFF40 } else { desc.light_curve };
                let pass = Self::cartoon_pass(&cm, curve);
                let bits = DrawBits { diffuse_c4: true, local_normals: true, position_normals: desc.position_normals, zone_fog, ..Default::default() };
                let mut u = PassUniform::new(&pass, bits);
                constants(&mut u, &desc);
                if cut { u.set_near_cut(NEAR_PLANE); }
                self.push_pass(library, &pass, u, cull, &None, (which, *first, *count), cm.blend.is_some() || whole, overlay);
            } else {
                // Outline only: SetMaterial and the normal path's lighting.
                let st = state;
                let lighting = Lighting { lighting: st.lighting, color_vertex: st.color_vertex, lightmap: false, lighting_only: false };
                let result = self.materials.set_material(library, mat, lighting, self.frame, view);
                let bits = DrawBits {
                    lighting: st.lighting,
                    color_vertex: st.color_vertex,
                    has_color: st.has_color,
                    ambient: st.ambient,
                    local_normals: true,
                    zone_fog,
                    ..Default::default()
                };
                for pass in &result.passes {
                    let mut u = PassUniform::new(pass, bits);
                    constants(&mut u, &desc);
                    if cut { u.set_near_cut(NEAR_PLANE); }
                    self.push_pass(library, pass, u, cull, &None, (which, *first, *count), result.requires_sorting || whole, overlay);
                }
            }
        }
        if desc.outline && desc.outline_width > 0.0 {
            // Back faces: CULL_CCW if the current mode is CW or NONE, otherwise CW.
            let reverse = if cull == 0 { 1 } else { 0 };
            let pass = Self::outline_pass();
            let bits = DrawBits { outline: true, local_normals: true, zone_fog, ..Default::default() };
            for (first, count, cut) in outline_sections {
                let mut u = PassUniform::new(&pass, bits);
                constants(&mut u, &desc);
                u.c4 = [0.0, 0.0, 0.0, desc.outline_width];
                if cut { u.set_near_cut(NEAR_PLANE); }
                self.push_pass(library, &pass, u, reverse, &None, (which, first, count), whole, overlay);
            }
        }
    }

    /// UPrimitive+0x28 bytes of an animated actor's mesh (outline, cartoon, normals from
    /// position, thickness factor), read once per mesh.
    fn cartoon_flags(&mut self, library: &mut Library, owner: usize) -> Option<[u8; 8]> {
        let mesh = &self.cartoon.get(owner)?.as_ref()?.mesh;
        if !self.primitive_bytes.contains_key(mesh) {
            // Mods build only: a mod's model (or the one drawn instead of a game mesh) carries its
            // own bytes, outline and cartoon shading on unless the mod turns them off.
            #[cfg(feature = "mods")]
            if let Some(b) = openxiii_assets::mod_meshes::get(mesh).map(|m| m.0.unknown).or_else(|| openxiii_assets::mod_skeletal::get(mesh).map(|m| m.cartoon)) {
                self.primitive_bytes.insert(mesh.clone(), Some(b));
            }
        }
        if !self.primitive_bytes.contains_key(mesh) {
            let bytes =
                library.resolve(mesh).and_then(|(loaded, index)| openxiii_assets::static_mesh::primitive_cartoon(&loaded.package, &loaded.data, index).ok());
            self.primitive_bytes.insert(mesh.clone(), bytes);
        }
        self.primitive_bytes[mesh]
    }

    /// Cartoon rendering pass of a section: texture × c4,
    /// then the lighting curve's ramp at coordinates N·c5, N·c6, without filtering.
    fn cartoon_pass(cm: &openxiii_assets::material::CartoonMaterial, light_curve: u32) -> openxiii_assets::material::Pass {
        use openxiii_assets::material::{d3d as t, Pass, Stage, TexRef};
        let mut p = Pass { z_test: true, z_write: true, ..Pass::default() };
        if let Some((src, dst)) = cm.blend {
            p.alpha_blending = true;
            p.src_blend = src;
            p.dest_blend = dst;
        }
        if cm.alpha_test {
            p.alpha_test = true;
            p.alpha_ref = 0x7F;
        }
        p.stages[0] = Stage {
            texture: cm.texture.clone().map(TexRef::Bitmap),
            address: [t::TADDRESS_WRAP, t::TADDRESS_WRAP, t::TADDRESS_CLAMP],
            color_op: t::TOP_MODULATE,
            color_arg1: t::TA_TEXTURE,
            color_arg2: t::TA_DIFFUSE,
            // D3DDrv does not set stage 0's alpha: we keep that of the simple materials.
            alpha_op: t::TOP_SELECTARG1,
            alpha_arg1: t::TA_TEXTURE,
            ..Stage::default()
        };
        p.stages[1] = Stage {
            texture: Some(TexRef::Bitmap(format!("#ramp:{light_curve}"))),
            address: [t::TADDRESS_CLAMP, t::TADDRESS_CLAMP, t::TADDRESS_CLAMP],
            color_op: t::TOP_MODULATE,
            color_arg1: t::TA_TEXTURE,
            color_arg2: t::TA_CURRENT,
            alpha_op: t::TOP_DISABLE,
            tex_coord_index: d3d::TCI_CARTOON,
            point_filter: true,
            ..Stage::default()
        };
        p.stages_used = 2;
        p
    }

    /// Outline pass: TEXTUREFACTOR colour (0, black), without blending or alpha test.
    fn outline_pass() -> openxiii_assets::material::Pass {
        use openxiii_assets::material::{d3d as t, Pass, Stage};
        let mut p = Pass { z_test: true, z_write: true, tfactor: 0, ..Pass::default() };
        p.stages[0] =
            Stage { color_op: t::TOP_SELECTARG1, color_arg1: t::TA_TFACTOR, alpha_op: t::TOP_SELECTARG1, alpha_arg1: t::TA_TFACTOR, ..Stage::default() };
        p.stages_used = 1;
        p
    }

    fn rebuild_job_list(&mut self, list: &mut JobList) {
        let hidden = |owner: &Option<String>| owner.as_ref().is_some_and(|o| self.hidden_owners.contains(o));
        let batches_list = &mut list.sections;
        batches_list.clear();
        // Most of the map's actor batches belong to actors the view does not
        // draw (no light pass): mark the batches of the drawn names, one
        // lookup per name, rather than looking up the name of each batch.
        let marking = self.scene_light_views.enabled;
        if marking {
            self.world_owner_stamp = self.world_owner_stamp.wrapping_add(1);
            if self.world_owner_stamp == 0 {
                self.world_owner_stamp = 1;
                self.world_owner_seen.fill(0);
            }
            self.world_owner_seen.resize(self.world.batches.len(), 0);
            for name in self.scene_light_passes.iter().flat_map(|p| p.draws.iter()).flat_map(|d| d.names.iter()) {
                if let Some(batches) = self.world_owner_batches.get(&**name) {
                    for &b in batches {
                        if let Some(seen) = self.world_owner_seen.get_mut(b as usize) {
                            *seen = self.world_owner_stamp;
                        }
                    }
                }
            }
        }
        for i in 0..self.world.batches.len() {
            let owner = self.world_owners.get(i).unwrap_or(&None);
            if marking && owner.is_some() && self.world_owner_seen[i] != self.world_owner_stamp {
                continue;
            }
            if !hidden(owner) {
                batches_list.extend(self.light_passes_for_name(owner.as_deref()).iter().copied().map(|p| (Which::World, i, p)));
            }
        }
        if let Some((g, owners)) = &self.replaced {
            for i in 0..g.batches.len() {
                batches_list.extend(self.light_passes_for_name(owners.get(i).and_then(|o| o.as_deref())).iter().copied().map(|p| (Which::Replaced, i, p)));
            }
        }
        if let Some(d) = &self.dynamic {
            for (i, owner) in d.owners.iter().enumerate() {
                batches_list.extend(self.light_passes_for_character(*owner).iter().copied().map(|p| (Which::Dynamic, i, p)));
            }
        }
        list.view = Some(self.scene_light_passes.clone());
        list.hidden = Some(self.hidden_owners.clone());
        list.enabled = self.scene_light_views.enabled;
    }

    /// Translates the materials of every batch (SetMaterial every frame, like the engine) and
    /// writes the passes' uniforms.
    fn prepare(&mut self, library: &mut Library, camera: &Camera) {
        self.prepare_view(library, camera, None);
    }

    fn prepare_view(&mut self, library: &mut Library, camera: &Camera, target: Option<&target_clip::TargetClip>) {
        let mut t_prep = std::time::Instant::now();
        let main_value = target.is_none();
        let view = d3d_view(camera);
        self.uniforms.clear();
        self.uniform_sources.clear();
        self.passes_written = 0;
        self.commands.clear();
        // With UseVignetteHighlight, the bIgnoreVignetteAlpha actors leave the
        // opaque and translucent lists for the one drawn after the filter.
        let vignette_on = self.vignette.highlight.is_some();
        let slot = usize::from(!Rc::ptr_eq(&self.scene_light_passes, &self.scene_light_views.main));
        let mut list = std::mem::take(&mut self.job_lists[slot]);
        let measure_target = target.is_some() && (false);
        let t_list = std::time::Instant::now();
        let was_reused = list.matches(&self.scene_light_passes, &self.hidden_owners, self.scene_light_views.enabled);
        if !was_reused {
            self.rebuild_job_list(&mut list);
        }
        if measure_target {
            timer_target(3, t_list);
            if !was_reused {
                TIMER_TARGETS.lock().unwrap()[6] += 1.0;
            }
        }
        let t_batches = std::time::Instant::now();
        // Batches of the actors drawn by DrawCartoonSM: (sections, first-person view) per actor.
        let mut cartoon_sections: HashMap<(usize, Option<crate::lights_scene::DrawKey>), CartoonSections> = HashMap::new();
        // Sections are immutable during preparation. Holding their three arrays
        // here allows borrowing the material and the lightmap while preparing the draw.
        let mut world_batches = std::mem::take(&mut self.world.batches);
        // Mods build only: a script changed a texture on an actor of the scenery: the world
        // batches translate their material again.
        #[cfg(feature = "mods")]
        {
            let generation = self.mod_surfaces.generation();
            if generation != self.retexture_generation {
                self.retexture_generation = generation;
                for b in world_batches.iter_mut() {
                    b.bound.reset();
                }
            }
            self.world.batches = std::mem::take(&mut world_batches);
            self.refresh_charred_ranges();
            world_batches = std::mem::take(&mut self.world.batches);
        }
        let mut replaced_batches = self.replaced.as_mut().map(|(g, _)| std::mem::take(&mut g.batches));
        let mut dynamic_batches = self.dynamic.as_mut().map(|d| std::mem::take(&mut d.geometry.batches));
        // An actor's sections are adjacent. Its selection does not change between two
        // sections of the same draw; a new view starts again with an empty selection.
        let mut last_prop_state = None;
        // Same actor as the previous batch (its batches are adjacent): same cartoon bytes.
        let mut last_cartoon: Option<(usize, bool)> = None;
        let measure_batches = false;
        let (mut t_sec, mut prec_which) = (std::time::Instant::now(), None::<Which>);
        let mut note_lot = |w: Option<Which>, t: &mut std::time::Instant| {
            if let Some(w) = w {
                let n = std::time::Instant::now();
                let k = match w { Which::World => 0, Which::Replaced => 2, _ => 4 };
                let mut c = TIMER_BATCHES.lock().unwrap();
                c[k] += (n - *t).as_secs_f64() * 1000.0;
                c[k + 1] += 1.0;
                *t = n;
            }
        };
        for &(which, i, light_pass) in &list.sections {
            if measure_batches {
                note_lot(prec_which, &mut t_sec);
                t_sec = std::time::Instant::now();
                prec_which = Some(which);
            }
            let overlay = if which == Which::Dynamic {
                let d = self.dynamic.as_ref().unwrap();
                d.overlay.get(d.owners[i]).copied().unwrap_or(0)
            } else { 0 };
            if which == Which::World {
                let batch = &world_batches[i];
                if let Some(packet) = &batch.prepared {
                    // The first pass has already resolved the textures. Keep all the
                    // translucent slots and the calls to animated materials.
                    if !packet.material.requires_sorting && self.world_range_hidden(batch.first_index, batch.index_count)
                        {
                        let state = &batch.state;
                        let lighting = Lighting { lighting: state.lighting, color_vertex: state.color_vertex, lightmap: state.lightmap.is_some(), lighting_only: false };
                        let replace = self.world_owners.get(i).is_some_and(Option::is_some);
                        if self.materials.static_batch_passes(&batch.bound, replace, lighting).is_some_and(|m| Rc::ptr_eq(m, &packet.material)) {
                            continue;
                        }
                    }
                }
            }
            self.use_light_pass(light_pass);
            // Comic panel: an opaque scenery batch with an already resolved fixed material, outside the field of the view's camera
            // (target_range_cache.visible would remove it after its preparation), is not prepared.
            if which == Which::World && self.world_culled.is_none() {
                if let (Some(clip), Some(Some(bounds)), Some(packet)) = (target, self.target_world_bounds.get(i), &world_batches[i].prepared) {
                    let wanted = !packet.material.requires_sorting;
                    if wanted {
                        let batch = &world_batches[i];
                        let state = &batch.state;
                        let lighting = Lighting { lighting: state.lighting, color_vertex: state.color_vertex, lightmap: state.lightmap.is_some(), lighting_only: false };
                        let replace = self.world_owners.get(i).is_some_and(Option::is_some);
                        if self.materials.static_batch_passes(&batch.bound, replace, lighting).is_some_and(|m| Rc::ptr_eq(m, &packet.material)) {
                            let light_sky = self.push_sky.unwrap_or_else(|| self.active_light_pass().map(|p| p.sky_zone.is_some()));
                            if self.target_range_cache.visible(i, bounds, clip, light_sky).is_none() {
                                continue;
                            }
                        }
                    }
                }
            }
            if which == Which::Dynamic {
                let owner = self.dynamic.as_ref().unwrap().owners[i];
                let cartoon = match last_cartoon {
                    Some((o, c)) if o == owner => c,
                    _ => {
                        let c = self.cartoon_flags(library, owner).is_some_and(|b| b[0] != 0 || b[1] != 0);
                        last_cartoon = Some((owner, c));
                        c
                    }
                };
                if cartoon {
                    let b = &dynamic_batches.as_ref().unwrap()[i];
                    let e = cartoon_sections.entry((owner, light_pass)).or_insert_with(|| (Vec::new(), overlay));
                    e.0.push((b.material.clone().unwrap_or_default(), b.first_index, b.index_count));
                    continue;
                }
            }
            let batch = match which {
                Which::World => &mut world_batches[i],
                Which::Replaced => &mut replaced_batches.as_mut().unwrap()[i],
                Which::Dynamic => &mut dynamic_batches.as_mut().unwrap()[i],
                Which::Particles | Which::Canvas | Which::Projectors | Which::Weather => continue,
            };
            self.push_vignette = vignette_on
                && match which {
                    Which::World => self.world_owners.get(i).and_then(Option::as_ref).is_some_and(|o| self.vignette_owners.contains(o)),
                    Which::Replaced => self.replaced.as_ref().and_then(|r| r.1.get(i)).and_then(Option::as_ref).is_some_and(|o| self.vignette_owners.contains(o)),
                    _ => {
                        let d = self.dynamic.as_ref().unwrap();
                        d.ignore_vignette.get(d.owners[i]).copied().unwrap_or(false)
                    }
                };
            let state = &batch.state;
            let (mut first_index, mut index_count) = (batch.first_index, batch.index_count);
            if which == Which::World {
                if let Some((_, batches_list)) = &self.world_culled {
                    (first_index, index_count) = batches_list.get(i).copied().unwrap_or((0, 0));
                    if index_count == 0 {
                        continue;
                    }
                }
            }
            let prop = if which == Which::Dynamic {
                let owner = self.dynamic.as_ref().unwrap().owners[i];
                self.prop_lights.get(owner).and_then(Option::as_ref).map(|p| (owner, p))
            } else { None };
            let prop_zone = prop.and_then(|(_, p)| p.zone);
            // ReplaceATextureByAnOther: Materials[i].Material of UStaticMeshes (batches of map
            // actors, replaced meshes, animated StaticMeshes), not the BSP or the Skins.
            let static_mesh = match which {
                Which::World => self.world_owners.get(i).is_some_and(|o| o.is_some()),
                Which::Replaced => true,
                _ => prop.is_some(),
            };
            let dynamic_refresh = prop.is_some();
            let prop_state = if let Some((owner, p)) = prop {
                let key = (owner, light_pass);
                if last_prop_state.as_ref().is_none_or(|(cached, _)| *cached != key)
                    {
                    last_prop_state = Some((key, self.prop_light_state(p)));
                }
                last_prop_state.as_ref().map(|(_, state)| state)
            } else { None };
            // EnableLighting(UseDynamic, UseStatic) for animated StaticMeshes.
            let (lit, color_vertex) = prop_state.as_ref().map_or((state.lighting, state.color_vertex), |(b, _)| (b.lighting, b.color_vertex));
            let lighting = Lighting { lighting: lit, color_vertex, lightmap: state.lightmap.is_some(), lighting_only: false };
            // Batch timers (measure-send): the time is only taken if they are recorded.
            let t_mat = measure_batches.then(std::time::Instant::now);
            // Mods build only: the texture a script gave this actor of the scenery.
            #[cfg(feature = "mods")]
            let retextured: Option<String> = if which == Which::World && !self.mod_surfaces.is_empty() {
                self.world_owners.get(i).and_then(Option::as_deref).and_then(|o| self.mod_surfaces.lookup(o, batch.material.as_deref().unwrap_or("")))
            } else {
                None
            };
            #[cfg(not(feature = "mods"))]
            let retextured: Option<String> = None;
            let original = retextured.as_deref().or(batch.material.as_deref());
            let (result, material) = self.materials.set_batch_material(
                &mut batch.bound, library, original, static_mesh, lighting, self.frame, view,
            );
            // Mods build only: a script charred this actor of the scenery: its own material,
            // darker.
            #[cfg(feature = "mods")]
            let result = match (!self.mod_surfaces.no_charred() || which == Which::Dynamic)
                .then(|| match which {
                    Which::World => self.world_owners.get(i).and_then(Option::as_deref).and_then(|owner| self.mod_surfaces.charred(owner)),
                    // Scenery the game moves or swaps (doors, shutters, windows: movers and the
                    // scenes' props), drawn from their own batches.
                    Which::Replaced => self.replaced.as_ref().and_then(|r| r.1.get(i)).and_then(Option::as_deref).and_then(|owner| self.mod_surfaces.charred(owner)),
                    // Doors, shutters and objects the game moves are drawn as animated static
                    // meshes, by the rank of their actor (as the characters).
                    Which::Dynamic => {
                        let k = crate::game::mod_charred(self.dynamic.as_ref().map_or(usize::MAX, |d| d.owners[i]));
                        (k > 0.0).then_some(k)
                    }
                    _ => None,
                })
                .flatten()
            {
                Some(k) => self.charred_material(&result, k),
                None => result,
            };
            if let Some(t_mat) = t_mat.filter(|_| which == Which::World) {
                TIMER_BATCHES.lock().unwrap()[6] += t_mat.elapsed().as_secs_f64() * 1000.0;
            }
            if let Some(e) = &result.error {
                let name = material.unwrap_or_default();
                if self.reported.insert(name.to_owned()) {
                    eprintln!("material {name}: {e}");
                }
            }
            let bits = prop_state.map_or_else(|| DrawBits {
                lighting: state.lighting,
                color_vertex: state.color_vertex,
                has_color: state.has_color,
                ambient: state.ambient,
                ..Default::default()
            }, |(b, _)| *b);
            // SetDistanceFog of the element's zone before drawing it, not only the camera zone's of FLevelSceneNode::Render.
            let mut bits = bits;
            let own_zone = true;
            if let Some(zone) = prop_zone.filter(|_| own_zone).or(state.zone) {
                bits.zone_fog = self.zone_fog_bits(zone);
            }
            // Also lights the map's placed StaticMeshes that have no StaticMeshInstance
            // colours: EnableLighting(1, 1) and SetLight of the lights selected by
            // for the actor's Scene draw (in Bateau01, StaticMeshActor62 is drawn
            // with LIGHTING 1 and five lights at its DrawIndexedPrimitive).
            let world_lights: Vec<[[f32; 4]; 4]> = if which == Which::World
                && static_mesh
                && state.lighting
                && !state.has_color
               
            {
                crate::lights_scene::pass_lights(self.active_light_pass(), self.active_light_draw(), self.lightmaps.model.as_deref(), &self.lights)
                    .unwrap_or_default()
                    .into_iter()
                    .take(8)
                    .map(d3d::d3d_light)
                    .collect()
            } else {
                Vec::new()
            };
            let d3d_lights = prop_state.map_or(world_lights.as_slice(), |(_, lights)| lights.as_slice());
            // SetCullMode: CM_CW (0), or CM_None (2) for double-sided faces.
            let cull = openxiii_vm::dyn_init_cull::cull_mode_dynamic(
                self.active_light_draw().and_then(|d| d.cull_mode), which == Which::Dynamic, state.two_sided, 0,
            );
            // Translucent order: a sorted map batch is drawn BSP node
            // by BSP node, each at its rank in the list; actor batches at their
            // actor's rank.
            let keys = &self.translucent_keys;
            let owner_key = |o: &Option<String>| o.as_ref().and_then(|n| keys.baked.get(n).copied()).unwrap_or(-2);
            let lot_key = match which {
                Which::World => owner_key(self.world_owners.get(i).unwrap_or(&None)),
                Which::Replaced => owner_key(self.replaced.as_ref().and_then(|r| r.1.get(i)).unwrap_or(&None)),
                Which::Dynamic => keys.characters.get(&self.dynamic.as_ref().unwrap().owners[i]).copied().unwrap_or(-2),
                _ => i64::MAX,
            };
            // Actor 's translucent list (table: a StaticMesh with a material
            // that requires sorting, …): the whole actor is drawn at its rank in that list, its
            // opaque sections included (SSH102b: the nearer passage covers the halo drawn before it).
            let translucent_actor = lot_key >= 0 && lot_key != i64::MAX;
            let ranges = self.world_node_ranges.get(i).filter(|r| which == Which::World && result.requires_sorting && self.world_culled.is_none() && !r.is_empty() && !keys.nodes.is_empty());
            // A pass of the batch has the same textures, uniforms and pipeline for all its
            // nodes. Only the draw's order and index range vary from one node to another.
            let node_order = |n| if n == u32::MAX { lot_key } else { keys.nodes.get(&n).copied().unwrap_or(-1) };
            let first = ranges.map(|r| r[0]);
            if let Some((n, f, c)) = first {
                (first_index, index_count) = (f, c);
                self.push_order = node_order(n);
            } else {
                self.push_order = lot_key;
            }
            // Terrain layers (ATerrainInfo::Render) sorted with the translucent passes: its
            // projectors (after the sector's layers) take the same rank.
            if which == Which::World && result.requires_sorting && material.is_some_and(|m| m.starts_with("#terrain:")) {
                self.terrain_order = Some(self.push_order);
            }
            let begin = self.commands.len();
            let t_sec2 = measure_batches.then(std::time::Instant::now);
            let section = self.prepare_section(&mut batch.prepared, library, &result, &state.lightmap, cull, bits, dynamic_refresh, d3d_lights);
            if let Some(t_sec2) = t_sec2.filter(|_| which == Which::World) {
                TIMER_BATCHES.lock().unwrap()[7] += t_sec2.elapsed().as_secs_f64() * 1000.0;
            }
            // Translation (animated materials included) and resource resolution
            // happened in their normal order. Only the ordinary opaque draws of the
            // fixed buffer can be skipped, with each scene's own projection.
            let mut target_stage = None;
            let skip_candidate = which == Which::World && !result.requires_sorting && self.world_culled.is_none();
            if skip_candidate {
                if let Some((clip, bounds)) = target.zip(self.target_world_bounds.get(i).and_then(Option::as_ref)) {
                    let light_sky = self.push_sky.unwrap_or_else(|| self.active_light_pass().map(|p| p.sky_zone.is_some()));
                    let Some((range, stage)) = self.target_range_cache.visible(i, bounds, clip, light_sky) else { continue };
                    first_index += range.start;
                    index_count = range.end - range.start;
                    target_stage = stage;
                }
            }
            for pass in &section.passes {
                self.emit_pass(pass.pipeline, pass.bind, (which, first_index, index_count), result.requires_sorting || translucent_actor, overlay);
                if let Some(sky) = target_stage {
                    self.commands.last_mut().unwrap().light_sky = Some(sky);
                }
                self.uniforms.push_prepared(&pass.uniform);
            }
            // Mods build only: the level's surfaces in a charred area are darkened right after
            // they are drawn (opaque ones only: a translucent one is drawn later, in order).
            #[cfg(feature = "mods")]
            if which == Which::World && !result.requires_sorting && !translucent_actor {
                if let Some(ranges) = self.charred_ranges.1.get(&i).cloned() {
                    for (first, count, level) in ranges {
                        let material = Self::charred_overlay_material(level);
                        let mut slot = self.charred_prepared.remove(&(i, level)).unwrap_or(None);
                        let passes: Vec<(usize, usize, Rc<PassUniform>)> = {
                            let dark_bits = DrawBits { zone_fog: bits.zone_fog, ..Default::default() };
                            let prepared = self.prepare_section(&mut slot, library, &material, &None, cull, dark_bits, false, &[]);
                            prepared.passes.iter().map(|p| (p.pipeline, p.bind, p.uniform.clone())).collect()
                        };
                        for (pipeline, bind, uniform) in passes {
                            self.emit_pass(pipeline, bind, (which, first, count), false, overlay);
                            self.uniforms.push_prepared(&uniform);
                        }
                        self.charred_prepared.insert((i, level), slot);
                    }
                }
            }
            if first.is_some() {
                let end = self.commands.len();
                for &(n, first_index, index_count) in &self.world_node_ranges[i][1..] {
                    let order = if n == u32::MAX { lot_key } else { self.translucent_keys.nodes.get(&n).copied().unwrap_or(-1) };
                    for k in begin..end {
                        self.commands.push(DrawCmd { first_index, index_count, order, ..self.commands[k] });
                    }
                }
            }
        }
        if measure_batches {
            note_lot(prec_which, &mut t_sec);
        }
        self.world.batches = world_batches;
        if let (Some((g, _)), Some(batches)) = (&mut self.replaced, replaced_batches) {
            g.batches = batches;
        }
        if let (Some(d), Some(batches)) = (&mut self.dynamic, dynamic_batches) {
            d.geometry.batches = batches;
        }
        self.job_lists[slot] = list;
        if measure_target {
            timer_target(4, t_batches);
        }
        timer_prep(main_value, &mut t_prep, 0);
        let t_cartoon = std::time::Instant::now();
        // StaticMeshes drawn by / DrawCartoonStatic.
        let aspect = self.width as f32 / self.height as f32;
        // Viewport->Actor->FovAngle (horizontal): the game's, or the free camera's.
        let fov_x = match self.fov {
            Some(f) if self.view_widen().is_some() => openxiii_vm::display_format::widen_fov(f, (self.width, self.height), self.session_graphics),
            Some(f) => f,
            None => 2.0 * ((self.fov_y(aspect) / 2.0).tan() * aspect).atan().to_degrees(),
        };
        // Outline thickness from the player's own FovAngle, never the Hor+ angle:
        // Hor+ keeps the objects' size tied to the height of the window, so a widened angle would
        // thicken the outline relative to them on a wide screen (`outline_tan_half`).
        let tan_half_fov = outline_tan_half(self.fov, fov_x);
        let actors = std::mem::take(&mut self.actors);
        let jobs: Vec<_> = actors.iter().flat_map(|actor| self.light_passes_for_name(actor.owner.as_deref()).iter().copied().map(move |p| (actor, p))).collect();
        for (actor, light_pass) in jobs {
            self.use_light_pass(light_pass);
            if actor.owner.as_ref().is_some_and(|o| self.hidden_owners.contains(o)) {
                continue;
            }
            self.push_vignette = vignette_on && actor.owner.as_ref().is_some_and(|o| self.vignette_owners.contains(o));
            self.push_order = actor.owner.as_ref().and_then(|n| self.translucent_keys.baked.get(n).copied()).unwrap_or(-2);
            let mut desc = actor.desc.clone();
            if desc.cartoon {
                let lights = crate::lights_scene::pass_lights(
                    self.active_light_pass(), self.active_light_draw(), self.lightmaps.model.as_deref(), &self.lights,
                ).unwrap_or_else(|| {
                    let scene_lights = self.active_light_pass().and_then(|pass| {
                        self.lightmaps.model.as_ref().map(|model| {
                            actor.leaves.iter().filter(|&&l| model.leaves.get(l as usize).is_some()).map(|&l| pass.leaf_lights(model, l as usize)).collect::<Vec<_>>()
                        })
                    });
                    let target = openxiii_assets::cartoon::LightTarget {
                        light_group: actor.light_group,
                        leaf_zones: &actor.leaf_zones,
                        leaf_lights: scene_lights.as_deref().unwrap_or(&actor.leaf_lights),
                        sphere: actor.sphere,
                    };
                    openxiii_assets::cartoon::select_lights(&target, &self.lights)
                });
                openxiii_assets::cartoon::compute_lights(&mut desc, &lights, &actor.local_to_world, actor.location);
            }
            if desc.outline {
                let d = [0, 1, 2].map(|k| actor.location[k] - camera.position[k]);
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                openxiii_assets::cartoon::outline_width(&mut desc, dist, tan_half_fov, Some(actor.scale), self.outline_boost);
            }
            let l2w: [f32; 16] = std::array::from_fn(|i| actor.local_to_world[i / 4][i % 4]);
            // SetCullMode(Determinant < 0): faces flipped by a negative scale.
            let cull = if actor.mirrored { 1 } else { 0 };
            let sections: Vec<(String, u32, u32)> = actor.sections.iter().map(|(m, f, c)| (self.materials.replaced(m), *f, *c)).collect();
            self.push_cartoon(library, view, &desc, &sections, (Which::World, 0), cull, l2w, false, &actor.state);
        }
        self.actors = actors;
        self.use_light_pass(None);
        self.push_vignette = false;
        self.push_order = i64::MAX;
        if measure_target {
            timer_target(5, t_cartoon);
        }
        // Projectors of the BSP nodes: SetZBias(8), EnableLighting(0, 0, 0),
        // material; of terrain sectors (ATerrainInfo::Render): no bias,
        // EnableLighting(0, 0, 0, 0).
        // Player view (buffers culled by its visibility): a projector of a BSP node is
        // drawn by the scene that sees that node (player, sky, or both); without a node, by the
        // player's scene only. Other views (portals, render textures): as before.
        let view_player = self.scene_light_views.enabled && self.world_masked.is_some();
        timer_prep(main_value, &mut t_prep, 1);
        if let Some(projectors) = self.projectors.take() {
            let proj_clip = target.filter(|_| target_projector_clip());
            if impacts_incremental() {
                if let Some(clip) = proj_clip {
                    let generation = self.target_range_cache.generation;
                    self.target_range_cache.projector_stages.clear();
                    for id in &self.impact_arena.visible {
                        let f = self.impact_arena.fragments.get_mut(id).unwrap();
                        if f.stages.is_none_or(|(g, _)| g != generation) {
                            f.stages = Some((generation, f.bounds.as_ref().map(|b| clip.visible_stages(b))));
                        }
                        self.target_range_cache.projector_stages.push(f.stages.unwrap().1);
                    }
                }
            } else {
            if proj_clip.is_some() && self.projector_bounds.len() != self.projector_polys.len() {
                self.projector_bounds = self.projector_polys.iter().map(|(poly, _)| target_clip::Bounds::from_world(poly.iter().copied())).collect();
            }
            if let Some(clip) = proj_clip {
                if self.target_range_cache.projector_stages.len() != self.projector_bounds.len() {
                    self.target_range_cache.projector_stages.extend(self.projector_bounds.iter().map(|b| b.as_ref().map(|b| clip.visible_stages(b))));
                }
            }
            }
            let mut packets = std::mem::take(&mut self.projector_packets);
            packets.resize_with(projectors.1.len(), ProjectorPacket::default);
            let mut previous: Option<usize> = None;
            for (index, (key, first, count, bias, node)) in projectors.1.iter().enumerate() {
                (self.push_sky, self.push_outside_sky) = match (self.last_bsp_visibility.as_ref().filter(|_| view_player), node) {
                    (Some(visible), Some(n)) => {
                        let seen_flag = |v: &Vec<bool>| v.get(*n as usize).copied().unwrap_or(false);
                        (Some(match (seen_flag(&visible.root), seen_flag(&visible.sky)) {
                            (true, false) => Some(false),
                            (false, true) => Some(true),
                            _ => None,
                        }), false)
                    }
                    (_, None) => (None, view_player),
                    (None, Some(_)) => (None, false),
                };
                self.z_bias = *bias;
                // Draws a node's projectors right after its surface: with the
                // opaque BSP (before the translucent passes, the sea for example), or at its rank in
                // the translucent list for a sorted node. Terrain (bias 0, no node): ATerrainInfo::Render
                // draws a sector's projectors right after its layers, so at the
                // terrain's rank when its layers are sorted, otherwise with the opaque passes.
                let rank = match node {
                    Some(n) => self.translucent_keys.nodes.get(n).copied(),
                    None if *bias == 0 => self.terrain_order,
                    None => None,
                };
                self.push_order = rank.unwrap_or(i64::MAX);
                // Comic panel: a projector outside both scenes of the view (which clip_target_stages would
                // remove) is not prepared when its material is fixed (translation without an advancing
                // state).
                if let Some(Some(stages)) = proj_clip.map(|_| self.target_range_cache.projector_stages[index]) {
                    let light_sky = self.push_sky.unwrap_or_else(|| self.active_light_pass().map(|p| p.sky_zone.is_some()));
                    let scene = stages.0 && light_sky != Some(true);
                    let sky = stages.1 && !self.push_outside_sky && light_sky != Some(false);
                    let outside = !scene && !sky;
                    if outside && self.materials.fixed_projector(&mut packets[index].material, library, key) {
                        continue;
                    }
                }
                let t_mat = false.then(std::time::Instant::now);
                let result = self.materials.set_projector_material(&mut packets[index].material, library, key, self.frame, view);
                if let Some(t_mat) = t_mat {
                    TIMER_PREP.lock().unwrap()[9 + if main_value { 0 } else { 12 }] += t_mat.elapsed().as_secs_f64() * 1000.0;
                }
                if let Some(e) = &result.error {
                    if !self.reported.contains(key.as_ref()) {
                        self.reported.insert(key.to_string());
                        eprintln!("projector material {key}: {e}");
                    }
                }
                let matrix_wanted = matrices_impacts() && self.impact_arena.can_matrix(index) && result.passes.len() == 1;
                if matrix_wanted {
                    let pass = &result.passes[0];
                    let mut uniform = PassUniform::new(pass, DrawBits::default());
                    if let Some(mut matrix) = uniform.take_impact_matrix() {
                        self.impact_arena.matrix(index, matrix);
                        let begin = self.commands.len();
                        self.push_pass(library, pass, uniform, 0, &None, (Which::Projectors, *first, *count), result.requires_sorting && rank.is_some(), 0);
                        if let Some(clip) = proj_clip {
                            if let Some(stages) = self.target_range_cache.projector_stages[index] {
                                clip_target_stages(&mut self.commands, begin, clip.sky.is_some(), stages);
                            }
                        }
                        previous = None;
                        continue;
                    }
                }
                // Two successive polygons with the same translation and bias
                // share the resources. Rank, view and range stay per draw.
                let shared = previous.filter(|&i| packets[i].section.as_ref().is_some_and(|s| s.bias == *bias && Rc::ptr_eq(&s.material, &result)));
                let section = if let Some(i) = shared {
                    packets[i].section.as_deref().unwrap()
                } else {
                    previous = Some(index);
                    self.prepare_section(&mut packets[index].section, library, &result, &None, 0, DrawBits::default(), false, &[])
                };
                let begin = self.commands.len();
                for pass in &section.passes {
                    self.emit_pass(pass.pipeline, pass.bind, (Which::Projectors, *first, *count), result.requires_sorting && rank.is_some(), 0);
                    self.uniforms.push_prepared(&pass.uniform);
                }
                if let Some(clip) = proj_clip {
                    if let Some(stages) = self.target_range_cache.projector_stages[index] {
                        let before_value = self.commands.len();
                        clip_target_stages(&mut self.commands, begin, clip.sky.is_some(), stages);
                    }
                }
            }
            self.upload_impact_matrices();
            self.projector_packets = packets;
            // split_projectors: screen rectangle of each batch, main view only.
            self.projector_rects.clear();
            self.projector_shadows.clear();
            if main_value && self.view_window && crate::split_projectors::active_value() && self.projector_rect_infos.len() >= projectors.1.len() {
                let aspect = self.width as f32 / self.height.max(1) as f32;
                let vp = Self::frame_uniform(camera, self.view_fov_y(aspect), aspect, NEAR_PLANE, FAR_PLANE, None).view_proj;
                let size_value = { let (w, h) = self.world_size(); [w, h] };
                self.projector_rects_size = size_value;
                for (index, (_, first, count, _, _)) in projectors.1.iter().enumerate() {
                    let (m, g, projected, bt, bg) = &self.projector_rect_infos[index];
                    let (poly, triangles) = &self.projector_polys[index];
                    // Only the shadows on a BSP polygon (computed for every batch, impacts included, the
                    // rectangle cost more CPU time than it saved on the GPU).
                    // Terrain (thousands of triangles) and impacts: drawn in full.
                    // Shadows: rectangle of the 8 corners of their frame (terrain included).
                    let (useful, frame_rect, by_t, by_g) = (*bt, *bt && *bg, *bt, *bg);
                    let r = if !useful { None } else if frame_rect {
                        // Shadow frame (long along the light axis) ∩ the receiver's vertices.
                        crate::split_projectors::inter(crate::split_projectors::rectangle_frame(m, g, *projected, &vp, size_value), crate::split_projectors::rectangle_vertices(poly, &vp, size_value))
                    } else if *triangles { None } else { crate::split_projectors::rectangle(poly, *triangles, m, g, *projected, by_t, by_g, &vp, size_value) };
                    self.projector_rects.push((*first, *first + *count, r));
                }
                self.projector_rects.sort_unstable_by_key(|x| x.0);
            }
            self.projectors = Some(projectors);
            self.z_bias = 0;
            self.terrain_order = None;
        }
        self.push_sky = None;
        self.push_outside_sky = view_player;
        // Particles: UParticleMaterial (SetParticleMaterial), vertex colour, without
        // lighting (EnableLighting(0, 1)).
        timer_prep(main_value, &mut t_prep, 2);
        let particle_clip = particles_frustum().then(|| self.particles_projection.get()).flatten().map(|(scene, sky)| {
            target_clip::TargetClip { scene: target_clip::Frustum::new(scene), sky: sky.map(target_clip::Frustum::new) }
        });
        if let Some(particles) = self.particles.take() {
            for lot in &particles.1 {
                let begin = self.commands.len();
                // Sprite or emitter that only a sky scene draws (FSkySceneNode, child scene): drawn by the sky stage, seen from its camera (e.g. the sun's
                // halo on Plage01, Light80 of the sky zone). The others stay those of
                // the player's scene, outside the sky stage.
                let sky_single = view_player && self.last_bsp_visibility.as_ref()
                    .is_some_and(|v| v.sky_actors.contains(&lot.actor) && !v.root_actors.contains(&lot.actor));
                (self.push_sky, self.push_outside_sky) = if sky_single { (Some(Some(true)), false) } else { (None, view_player) };
                self.push_vignette = vignette_on && lot.ignore_vignette;
                self.push_order = self.translucent_keys.actors.get(&lot.actor).copied().unwrap_or(i64::MAX);
                match lot.kind {
                    ParticleBatchKind::Particle { draw_style, two_sided } => {
                        // Mesh: the section's texture if it is a BitmapMaterial. Texture
                        // (+0x1D4) of emitters and materials of StaticMeshes after
                        // ReplaceATextureByAnOther.
                        let generation = self.materials.generation_replacements();
                        let kept_names = kept_particles();
                        let texture = match self.kept_particles.get(lot.material.as_str()).filter(|g| kept_names && g.generation == generation) {
                            Some(g) => g.texture.clone(),
                            None => {
                                let material = self.materials.replaced(&lot.material);
                                // SinusModifier section: its Material (UMeshEmitter::RenderParticles).
                                let material = self.materials.sine_material(library, &material).unwrap_or(material);
                                let texture = if self.materials.texture_info(library, &material).is_some() { material } else { String::new() };
                                if kept_names {
                                    self.kept_particles.insert(lot.material.clone(), KeptParticles { generation, texture: texture.clone(), passes: Vec::new() });
                                }
                                texture
                            }
                        };
                        let t_mat = std::time::Instant::now();
                        let mut pass = self.materials.particle_pass_frame(library, &texture, draw_style, lot.ignore_fog, self.frame, lot.frame.as_deref());
                        // Evaluate the material in the same order (texture animations), then
                        // avoid resources/uniforms/draw if no triangle can produce
                        // a pixel in the scenes that drew this batch. The simulation and the
                        // vertices of the other views stay intact.
                        if particle_clip.as_ref().is_some_and(|clip| particle_outside_view(lot.bounds.as_ref(), lot.delayed, self.push_outside_sky, clip)) {
                            continue;
                        }
                        pass.two_sided = two_sided;
                        let bits = DrawBits { has_color: true, ..Default::default() };
                        let sorted = pass.alpha_blending;
                        let cull = if two_sided { 2 } else { 0 };
                        if kept_names {
                            // Same pass as one of this material's last draws: same
                            // pipeline, textures and uniform bytes (kept shared).
                            let z_bias = self.z_bias;
                            let already = self.kept_particles.get(lot.material.as_str())
                                .and_then(|g| g.passes.iter().find(|p| p.cull == cull && p.z_bias == z_bias && (p.pass == pass || particles_test())))
                                .map(|p| (p.pipeline, p.bind, p.uniform.clone()));
                            let (pipeline, bind, uniform) = match already {
                                Some(d) => d,
                                None => {
                                    let uniform = PassUniform::new(&pass, bits);
                                    let (pipeline, bind) = self.pass_resources(library, &pass, &uniform, cull, &None);
                                    let uniform = Rc::new(uniform);
                                    if let Some(g) = self.kept_particles.get_mut(lot.material.as_str()) {
                                        if g.passes.len() >= 4 {
                                            g.passes.remove(0);
                                        }
                                        g.passes.push(ParticlePrepared { pass: pass.clone(), cull, z_bias, pipeline, bind, uniform: uniform.clone() });
                                    }
                                    (pipeline, bind, uniform)
                                }
                            };
                            self.emit_pass(pipeline, bind, (Which::Particles, lot.first, lot.count), sorted, lot.delayed);
                            self.uniforms.push_prepared(&uniform);
                        } else {
                            self.push_pass(
                                library,
                                &pass,
                                PassUniform::new(&pass, bits),
                                cull,
                                &None,
                                (Which::Particles, lot.first, lot.count),
                                sorted,
                                lot.delayed,
                            );
                        }
                    }
                    ParticleBatchKind::Trail { lines } => {
                        // SetMaterial of the UFinalBlend, EnableLighting(0, 1), SetCullMode(CM_None).
                        let lighting = Lighting { lighting: false, color_vertex: true, lightmap: false, lighting_only: false };
                        let result = self.materials.set_material(library, Some(&lot.material), lighting, self.frame, view);
                        let bits = DrawBits { color_vertex: true, has_color: true, ..Default::default() };
                        for pass in result.passes.iter() {
                            self.push_pass(
                                library,
                                &pass,
                                PassUniform::new(&pass, bits),
                                2,
                                &None,
                                (Which::Particles, lot.first, lot.count),
                                result.requires_sorting,
                                lot.delayed,
                            );
                            if lines {
                                let mut key = self.pipelines[self.commands.last().unwrap().pipeline].0;
                                key.lines = true;
                                let pipeline = self.pipeline(key);
                                self.commands.last_mut().unwrap().pipeline = pipeline;
                            }
                        }
                    }
                    ParticleBatchKind::Mesh { tfactor, ambient } => {
                        let material = self.materials.replaced(&lot.material);
                        let material = self.materials.sine_material(library, &material).unwrap_or(material);
                        let mat = (!material.is_empty()).then_some(material.as_str());
                        let lighting = Lighting { lighting: true, color_vertex: false, lightmap: false, lighting_only: false };
                        let result = self.materials.set_material(library, mat, lighting, self.frame, view);
                        // The meshes' positions are already in world space;
                        // their passes enable neither a local transform nor a GPU outline.
                        if particle_clip.as_ref().is_some_and(|clip| particle_outside_view(lot.bounds.as_ref(), lot.delayed, self.push_outside_sky, clip)) {
                            continue;
                        }
                        let bits = DrawBits { lighting: true, ambient, ..Default::default() };
                        for pass in result.passes.iter() {
                            let mut pass = pass.clone();
                            let b = tfactor.to_le_bytes();
                            pass.tfactor = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                            let cull = if pass.two_sided { 2 } else { 0 };
                            self.push_pass(
                                library,
                                &pass,
                                PassUniform::new(&pass, bits),
                                cull,
                                &None,
                                (Which::Particles, lot.first, lot.count),
                                result.requires_sorting,
                                lot.delayed,
                            );
                        }
                    }
                }
                if let Some(clip) = target { clip_target_batch(&mut self.commands, begin, clip, lot.bounds.as_ref()); }
            }
            self.particles = Some(particles);
        }
        (self.push_sky, self.push_outside_sky) = (None, view_player);
        self.push_vignette = false;
        // Weather (FD3DRenderInterface, DrawRndCubeSpr): point texture in
        // MODULATE (texture × colour), U and V in CLAMP, without fog or depth
        // write; blending ONE / INVSRCCOLOR, or alpha test at 0x7F (IsMask).
        timer_prep(main_value, &mut t_prep, 3);
        if let Some((_, texture, mask, count)) = &self.weather {
            let (count, texture, mask) = (*count, texture.clone(), *mask);
            let mut pass = self.materials.particle_pass(library, &texture, 6, true, self.frame);
            pass.override_fog_color = false;
            pass.stages[0].address[0] = openxiii_assets::material::d3d::TADDRESS_CLAMP;
            pass.stages[0].address[1] = openxiii_assets::material::d3d::TADDRESS_CLAMP;
            if mask {
                pass.alpha_blending = false;
                pass.src_blend = openxiii_assets::material::d3d::BLEND_ONE;
                pass.dest_blend = openxiii_assets::material::d3d::BLEND_ZERO;
                pass.alpha_test = true;
                pass.alpha_ref = 0x7f;
            }
            pass.z_write = false;
            let bits = DrawBits { has_color: true, ..Default::default() };
            self.push_order = i64::MAX;
            self.push_pass(library, &pass, PassUniform::new(&pass, bits), 2, &None, (Which::Weather, 0, count), !mask, 0);
        }
        self.push_outside_sky = false;
        // Characters, hands and weapons: USkeletalMeshInstance::Render then
        // DrawCartoonSM. Their vertices are already in world space.
        timer_prep(main_value, &mut t_prep, 4);
        let mut owners: Vec<(usize, Option<crate::lights_scene::DrawKey>)> = cartoon_sections.keys().copied().collect();
        owners.sort_unstable();
        // Mods build only: where the game's own first-person weapon (the hands mesh) stands. A mod
        // model held in first person takes its outline thickness from that distance, as the
        // weapon it is held in (the thickness is proportional to the distance of the actor's
        // location). From its own location, further from the eye than the weapon's, its outline
        // was about twice as thick as the 9mm's and the magnum's. Arms a mod made from a game mesh
        // (`[arms.<Name>]`) count as the game's own.
        #[cfg(feature = "mods")]
        let first_person_weapon: Option<[f32; 3]> = owners.iter().find_map(|k| {
            let (_, overlay) = cartoon_sections.get(k)?;
            let c = self.cartoon.get(k.0)?.as_ref()?;
            (*overlay != 0 && openxiii_assets::mod_meshes::get(&c.mesh).is_none() && openxiii_assets::mod_skeletal::get(&c.mesh).map_or(true, |m| m.game_look)).then_some(c.location)
        });
        for (owner, light_pass) in owners {
            self.use_light_pass(light_pass);
            let (sections, overlay) = cartoon_sections.remove(&(owner, light_pass)).unwrap();
            self.push_vignette = vignette_on && self.dynamic.as_ref().and_then(|d| d.ignore_vignette.get(owner).copied()).unwrap_or(false);
            self.push_order = self.translucent_keys.characters.get(&owner).copied().unwrap_or(-2);
            // Bytes read first (they only exist if the actor has its cartoon data), then
            // data borrowed for the computation rather than copied (name and leaves).
            let Some(bytes) = self.cartoon_flags(library, owner) else { continue };
            let cartoons = std::mem::take(&mut self.cartoon);
            let Some(c) = cartoons.get(owner).and_then(Option::as_ref) else {
                self.cartoon = cartoons;
                continue;
            };
            let zone = openxiii_assets::world::point_zone(&self.region_nodes, self.root_outside, c.location);
            let z = self.zones.get(zone).cloned().unwrap_or_default();
            // bUnlit StaticMesh (Actor+0x30 & 0x20000): clears the cartoon
            // flag as for the scenery (world.rs); the outline stays. Characters
            // (USkeletalMeshInstance) do not go through this.
            let prop_unlit = self.prop_lights.get(owner).and_then(Option::as_ref).is_some_and(|p| p.unlit);
            let mut desc = openxiii_assets::cartoon::CartoonDesc {
                outline: bytes[0] != 0,
                cartoon: bytes[1] != 0 && !prop_unlit,
                position_normals: bytes[2] != 0,
                outline_factor: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                light_curve: z.light_curve,
                ambient: z.ambient_intensity,
                ..Default::default()
            };
            // Actors drawn with their mesh-space positions and normals (StaticMesh actors: the
            // shader applies their LocalToWorld) get their light directions in mesh space, as
            // does with the actor's LocalToWorld (DrawCartoonStatic, constant c5);
            // characters, already in world space, keep the identity.
            let local_positions = self.prop_lights.get(owner).is_some_and(|p| p.is_some());
            let matrix = if local_positions { self.active_light_draw().and_then(|d| d.refresh.as_ref()).map_or(openxiii_assets::material::IDENTITY, |r| r.matrix) } else { openxiii_assets::material::IDENTITY };
            let local_to_world: [[f32; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|k| matrix[r * 4 + k]));
            if desc.cartoon {
                let lights = crate::lights_scene::draw_character_lights(
                    self.active_light_pass(), self.active_light_draw(), c,
                    &cartoons, self.lightmaps.model.as_deref(), &self.lights,
                );
                openxiii_assets::cartoon::compute_lights(&mut desc, &lights, &local_to_world, c.location);
                self.cartoon_dirs.insert(owner, desc.dir);
            }
            // Mods build only: an actor charred by a script is drawn darker, down to black.
            #[cfg(feature = "mods")]
            {
                let k = crate::game::mod_charred(owner);
                if k > 0.0 {
                    // The colour multiplies the texture once clamped to 1 (stage 0): brought
                    // back to 1 first, so that the darkening is the same in any light.
                    let brightest = desc.color.iter().fold(1.0f32, |m, &x| m.max(x));
                    let f = (1.0 - 0.95 * k) / brightest;
                    desc.color = desc.color.map(|x| x * f);
                    desc.ambient *= 1.0 - 0.95 * k;
                }
            }
            if desc.outline {
                #[allow(unused_mut)]
                let mut at = c.location;
                #[cfg(feature = "mods")]
                if overlay != 0 && openxiii_assets::mod_meshes::get(&c.mesh).is_some() {
                    at = first_person_weapon.unwrap_or(at);
                }
                let d = [0, 1, 2].map(|k| at[k] - camera.position[k]);
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                // Thickness divided by the actor's scale then restored by its transform:
                // in world space, the thickness is the table's.
                openxiii_assets::cartoon::outline_width(&mut desc, dist, tan_half_fov, None, self.outline_boost);
                // Mods build only: a mod model drawn with its own transform (a pickup, the model
                // held in first person with FirstPersonModelScale) keeps the table's thickness on
                // screen whatever its scale, as the game's own guns held in first person.
                #[cfg(feature = "mods")]
                if local_positions && openxiii_assets::mod_meshes::get(&c.mesh).is_some() {
                    let s = (local_to_world[0][0].powi(2) + local_to_world[0][1].powi(2) + local_to_world[0][2].powi(2)).sqrt();
                    if s > 1.0e-6 {
                        desc.outline_width /= s;
                        desc.outline_ratio /= s;
                    }
                }
            }
            self.cartoon = cartoons;
            let cull = 0;
            let own_zone = true;
            let actor_zone = self.prop_lights.get(owner).and_then(Option::as_ref).and_then(|p| p.zone).or(u8::try_from(zone).ok());
            let state = DrawState { zone: actor_zone.filter(|_| own_zone && overlay == 0), ..Default::default() };
            self.push_cartoon(library, view, &desc, &sections, (Which::Dynamic, overlay), cull, matrix, local_positions, &state);
        }
        self.push_vignette = false;
        self.push_order = i64::MAX;
        self.use_light_pass(None);
        let empty_value = (self.world_culled.is_none() && self.world_masked.is_some())
            .then(|| (self.world_masked_kept.0.as_slice(), self.world_masked_kept.1.as_slice()));
        timer_prep(main_value, &mut t_prep, 5);
        let merge_particles = merge_particles_image(self.frame.count);
        self.particle_packed_active = merge_particles && self.assembly.pack_particles(&mut self.commands, &self.particle_indices);
        if self.particle_packed_active {
            let data: &[u8] = bytemuck::cast_slice(&self.assembly.particle_indices);
            if self.particle_packed_buffer.as_ref().is_none_or(|b| b.size() < data.len() as u64) {
                self.particle_packed_buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("particles: indices in pass order"),
                    size: (data.len() as u64 * 3 / 2).max(PARTICLE_INDEX_BYTES).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if !data.is_empty() { write_value(&self.queue, self.particle_packed_buffer.as_ref().unwrap(), 0, data); }
        }
        self.assembly.assembled(&mut self.commands, &self.uniforms, &mut self.uniform_sources, empty_value);
        timer_prep(main_value, &mut t_prep, 6);
        self.upload_passes();
        timer_prep(main_value, &mut t_prep, 7);
    }

    /// Uniform added after the scene merge (Canvas and shadow silhouettes).
    fn append_uniform(&mut self, uniform: PassUniform) -> u32 {
        let offset = self.uniform_sources.len() as u32 * PASS_STRIDE as u32;
        self.uniform_sources.push(self.uniforms.len());
        self.uniforms.push(uniform);
        offset
    }

    /// Pass uniforms in the GPU buffer.
    fn upload_passes(&mut self) {
        if self.uniform_sources.len() as u64 > self.pass_buffers.capacity {
            let capacity = (self.uniform_sources.len() as u64).next_power_of_two();
            self.pass_buffers = Self::pass_buffer(&self.device, &self.pass_layout, capacity);
            self.passes_written = 0;
        }
        // Written directly into wgpu's staging area (no intermediate buffer of
        // several MB per frame), only the
        // entries not yet uploaded: the previous ones are unchanged in the buffer. Among
        // them, those already equal to what the buffer contains (same pass as the previous
        // frame at the same rank) are not re-uploaded: uploads by runs of changed entries.
        if empty_window() {
            self.passes_written = self.uniform_sources.len();
            return;
        }
        let start_value = self.passes_written.min(self.uniform_sources.len());
        let end_value = self.uniform_sources.len();
        let n = std::mem::size_of::<PassUniform>();
        let old_len = self.pass_buffers.gpu.len();
        if old_len < end_value {
            self.pass_buffers.gpu.resize(end_value, PassUniform::zeroed());
            self.pass_buffers.shared.resize(end_value, None);
        }
        let change = |gpu: &[PassUniform], shared: &[Option<Rc<PassUniform>>], k: usize| {
            let src = self.uniform_sources[k];
            if k >= old_len { return true; }
            if shared[k].as_ref().zip(self.uniforms.shared(src)).is_some_and(|(a, b)| Rc::ptr_eq(a, b)) {
                return false;
            }
            bytemuck::bytes_of(&gpu[k]) != bytemuck::bytes_of(&self.uniforms[src])
        };
        // One comparison per entry (1,328 bytes): the runs below used to read each entry two or three times. Entries are only written
        // afterwards, in their run, and a run never looks at an entry already written: same runs as before.
        let one_times = true;
        let mut changed_list = std::mem::take(&mut self.passes_changed);
        changed_list.clear();
        if one_times {
            changed_list.extend((start_value..end_value).map(|k| change(&self.pass_buffers.gpu, &self.pass_buffers.shared, k)));
        }
        let changed_one = |gpu: &[PassUniform], shared: &[Option<Rc<PassUniform>>], k: usize| {
            if one_times { changed_list[k - start_value] } else { change(gpu, shared, k) }
        };
        let mut k = start_value;
        while k < end_value {
            if !changed_one(&self.pass_buffers.gpu, &self.pass_buffers.shared, k) {
                k += 1;
                continue;
            }
            // An unchanged entry between two changed ones is uploaded with them (one less upload).
            let first_one = k;
            let mut last_one = k;
            k += 1;
            while k < end_value && (changed_one(&self.pass_buffers.gpu, &self.pass_buffers.shared, k) || (k + 1 < end_value && changed_one(&self.pass_buffers.gpu, &self.pass_buffers.shared, k + 1))) {
                if changed_one(&self.pass_buffers.gpu, &self.pass_buffers.shared, k) {
                    last_one = k;
                }
                k += 1;
            }
            let size = (last_one - first_one) as u64 * PASS_STRIDE + n as u64;
            self.perf_send[7] += size as f64;
            count_write(0, size);
            if order_concerns(&self.queue, &self.pass_buffers.buffer) {
                // Pending encoder (comic panels of the frame): copy in its place (zero margins,
                // never read).
                let mut bytes_value = vec![0u8; size as usize];
                for (j, &src) in self.uniform_sources[first_one..=last_one].iter().enumerate() {
                    let u = &self.uniforms[src];
                    self.pass_buffers.gpu[first_one + j] = *u;
                    self.pass_buffers.shared[first_one + j] = self.uniforms.shared(src).cloned();
                    let at = j * PASS_STRIDE as usize;
                    bytes_value[at..at + n].copy_from_slice(bytemuck::bytes_of(u));
                }
                write_ordered(&self.queue, &self.pass_buffers.buffer, first_one as u64 * PASS_STRIDE, &bytes_value);
            } else if let Some((lot, start_value)) = reserve_group(&self.queue, &self.pass_buffers.buffer, first_one as u64 * PASS_STRIDE, size) {
                // Grouped writes: entries placed in the shared upload (zero margins).
                BATCHES.with(|l| {
                    let mut l = l.borrow_mut();
                    let bytes_value = &mut l[lot].data_value[start_value..start_value + size as usize];
                    for (j, &src) in self.uniform_sources[first_one..=last_one].iter().enumerate() {
                        let u = &self.uniforms[src];
                        self.pass_buffers.gpu[first_one + j] = *u;
                        self.pass_buffers.shared[first_one + j] = self.uniforms.shared(src).cloned();
                        let at = j * PASS_STRIDE as usize;
                        bytes_value[at..at + n].copy_from_slice(bytemuck::bytes_of(u));
                    }
                });
            } else if let Some(mut view) = self.queue.write_buffer_with(&self.pass_buffers.buffer, first_one as u64 * PASS_STRIDE, std::num::NonZeroU64::new(size).unwrap()) {
                for (j, &src) in self.uniform_sources[first_one..=last_one].iter().enumerate() {
                    let u = &self.uniforms[src];
                    self.pass_buffers.gpu[first_one + j] = *u;
                    self.pass_buffers.shared[first_one + j] = self.uniforms.shared(src).cloned();
                    let at = j * PASS_STRIDE as usize;
                    view.slice(at..at + n).copy_from_slice(bytemuck::bytes_of(u));
                }
            }
            k = last_one + 1;
        }
        self.passes_changed = changed_list;
        self.passes_written = end_value;
    }

    /// Textures and texture groups of all the scenery batches with fixed materials, created on the
    /// first frame as the engine loads the level's textures with it: a part of the scenery seen for the
    /// first time (comic panel, end of a cinematic) no longer uploads its textures in the middle of the
    /// game (visible hitches on slow machines). Neither pipelines nor the state of animated materials:
    /// only device resources, same contents. What the level's objects reference (Game::precache_roots),
    /// to prepare with the scenery's prewarm, before the first frame is drawn. Ignored once the prewarm
    /// is done.
    pub fn set_precache_roots(&mut self, roots: Vec<String>) {
        if !self.scenery_warmup {
            self.precache_roots = roots;
        }
    }

    /// Whether the scenery prewarm still waits for its first frame.
    pub fn wants_precache_roots(&self) -> bool {
        !self.scenery_warmup && self.precache_roots.is_empty()
    }

    /// Textures and texture groups of every material reachable from the level's references
    /// (sub-materials and animation frames included), for world and screen samplers: what the
    /// original's level holds after LoadMap and its precache frame prepares.
    fn warm_up_references(&mut self, library: &mut Library, view: &[f32; 16]) {
        // Blended passes (particles, decals, HUD effects, translucent surfaces): the D3D blend
        // pairs the game's materials use (FinalBlend and the engine's own modes), for one or two
        // stages, with and without depth writing, for each face culling.
        // Also: alpha-tested or not, impact decals (second vertex stream, two stages, depth bias
        // 0 or 8) and Canvas lines.
        // The specialised combiner of a one-stage blended draw (texture × current, colour and alpha,
        // with a texture matrix: emitters and panned effects), first drawn in a render texture view.
        // Without a texture matrix too (sprites and surfaces drawn with FB_Brighten or additive).
        let modulated: Vec<_> = [(true, 2), (false, 0)]
            .into_iter()
            .map(|(use_matrix, flags)| {
                let mut p = openxiii_assets::material::Pass::default();
                p.stages_used = 1;
                let s = &mut p.stages[0];
                (s.color_op, s.alpha_op, s.color_arg0, s.color_arg1, s.color_arg2) = (4, 4, 1, 2, 0);
                (s.alpha_arg0, s.alpha_arg1, s.alpha_arg2) = (1, 2, 0);
                (s.use_matrix, s.transform_flags) = (use_matrix, flags);
                PassUniform::new(&p, DrawBits::default()).combine()
            })
            .collect();
        // Also DESTCOLOR, SRCCOLOR (impact decals' modulate 2x), and one-stage draws without depth
        // test (HUD-like overlays drawn in the world).
        for blend in ["-", "5,6", "2,4", "2,2", "1,4", "5,2", "2,6", "9,2", "1,2", "9,3"] {
            for stages in 1..=2 {
                for z_write in 0..=1 {
                    if stages == 1 && z_write == 0 {
                        for cull in 0..=2 {
                            if let Some(key) = key_pipeline_read(&format!("pipeline:0:1:{blend}:0:0:{cull}:0:0")) {
                                if !self.pipeline_ids.contains_key(&key) {
                                    self.pipeline(key);
                                }
                            }
                        }
                    }
                    for cull in 0..=2 {
                        for (impact, lines, z_bias) in [(0, 0, 0), (1, 0, 0), (1, 0, 8), (0, 1, 0)] {
                            if impact == 1 && stages != 2 {
                                continue;
                            }
                            for alpha_test in [false, true] {
                                if let Some(mut key) = key_pipeline_read(&format!("pipeline:{impact}:{stages}:{blend}:1:{z_write}:{cull}:{lines}:{z_bias}")) {
                                    key.alpha_test = alpha_test;
                                    if stages == 1 && impact == 0 && lines == 0 && blend != "-" {
                                        for combine in &modulated {
                                            let mut specialised = key.clone();
                                            specialised.combine = *combine;
                                            if !self.pipeline_ids.contains_key(&specialised) {
                                                self.pipeline(specialised);
                                            }
                                        }
                                    }
                                    if !self.pipeline_ids.contains_key(&key) {
                                        self.pipeline(key);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Shadow projections: the gradient and each of the eight shadow targets the original
        // makes at load, their targets included.
        for slot in 0..8 {
            let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD));
            key[0] = (Some("Engine.ProjGradient".to_string()), 3, 3, SAMPLER_WORLD);
            key[1] = (Some(format!("#shadowtex:{slot}")), 3, 3, SAMPLER_WORLD);
            if !self.bind_keys.contains_key(&key) {
                self.bind_group(library, key);
            }
        }
        // The group without any texture (untextured passes) and the engine's white texture, for each
        // sampler kind and addressing.
        for mode in [SAMPLER_WORLD, SAMPLER_POINT, SAMPLER_SCREEN] {
            let key: BindKey = std::array::from_fn(|_| (None, 3, 3, mode));
            if !self.bind_keys.contains_key(&key) {
                self.bind_group(library, key);
            }
            for (u, v) in [(1u32, 1u32), (3, 3)] {
                let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, mode));
                key[0] = (Some("#white".to_string()), u, v, mode);
                if !self.bind_keys.contains_key(&key) {
                    self.bind_group(library, key);
                }
            }
        }
        let (targets, mut roots): (Vec<String>, Vec<String>) =
            std::mem::take(&mut self.precache_roots).into_iter().partition(|r| r.strip_prefix('#').is_some_and(|n| n.parse::<usize>().is_ok()));
        self.warm_up_targets(library, &targets);
        // LoadMap's package reading: every package the read ones import (XIIIArmes.u imports
        // XIIIMeshArmes, ...), read and indexed, so the rules below see their objects.
        library.read_imported_packages();
        // The materials those script packages import (UObject::VerifyImport loads them with the
        // package): grapple control, HUD pictures named only in an import table.
        const MATERIALS: [&str; 11] = ["Texture", "Shader", "FinalBlend", "Combiner", "TexPanner", "TexRotator", "TexScaler", "TexOscillator", "TexEnvMap", "MaterialSwitch", "MaterialSequence"];
        roots.extend(library.imported_objects(&MATERIALS).into_iter().map(|p| format!("@{p}")));
        let mut materials_list = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in &roots {
            // "@path": named by script code; only objects of packages already read (a script's
            // constant never makes a package load here).
            let r = match r.strip_prefix('@') {
                Some(path) if path.split('.').next().is_some_and(|p| !library.is_loaded(p)) => continue,
                Some(path) => path,
                None => r.as_str(),
            };
            self.materials.reachable(library, r, &mut materials_list, &mut seen);
        }
        let lighting = Lighting { lighting: false, color_vertex: false, lightmap: false, lighting_only: false };
        for path in &materials_list {
            // Every texture uploaded, each frame of an animation included (its passes only name
            // the current frame).
            if self.materials.is_a(library, path.as_str(), "Texture") {
                if !self.textures.contains_key(path.as_str()) {
                    self.texture(library, path);
                }
                self.precache_textures.push(path.clone());
                // As a projected decal (bullet impacts, blood): the texture over the projector
                // gradient.
                // Also over the clipping gradient (Engine.ProjClip: blood spots and splashes).
                for gradient in ["Engine.ProjGradient", "Engine.ProjClip"] {
                    let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD));
                    key[0] = (Some(path.clone()), 3, 3, SAMPLER_WORLD);
                    key[1] = (Some(gradient.to_string()), 3, 3, SAMPLER_WORLD);
                    if !self.bind_keys.contains_key(&key) {
                        self.bind_group(library, key);
                    }
                }
                // The same texture in two wrapped stages (a shader's diffuse and self-illumination,
                // e.g. the screens a script switches on in USA02).
                {
                    let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD));
                    key[0] = (Some(path.clone()), 1, 1, SAMPLER_WORLD);
                    key[1] = (Some(path.clone()), 1, 1, SAMPLER_WORLD);
                    if !self.bind_keys.contains_key(&key) {
                        self.bind_group(library, key);
                    }
                }
                // Its one-stage groups as emitters, decals and the HUD bind it: wrapped or
                // clamped, world, point or screen sampler, the other stages empty and clamped.
                for (u, v) in [(1u32, 1u32), (3, 3)] {
                    for mode in [SAMPLER_WORLD, SAMPLER_POINT, SAMPLER_SCREEN] {
                        let rest = if mode == SAMPLER_POINT { SAMPLER_SCREEN } else { mode };
                        for others in [rest, SAMPLER_WORLD] {
                            let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, others));
                            key[0] = (Some(path.clone()), u, v, mode);
                            if !self.bind_keys.contains_key(&key) {
                                self.bind_group(library, key);
                            }
                        }
                    }
                }
            }
            let mut bound = openxiii_assets::material::BatchMaterial::default();
            let (result, _) = self.materials.set_batch_material(&mut bound, library, Some(path), false, lighting, self.frame, *view);
            for pass in &result.passes {
                // Its pipelines for each face culling, from its render states.
                let uniform = PassUniform::new(pass, DrawBits::default());
                for cull in 0..3u8 {
                    self.pass_pipeline(pass, &uniform, cull, None);
                }
                // The same textures drawn with wrapping or clamping (emitters and decals set
                // their own) and with point filtering (font pages): every sampler variant.
                for address in [None, Some([1u32, 1]), Some([3, 3])] {
                    for point in [false, true] {
                        let mut p = pass.clone();
                        let used = p.stages_used;
                        for (s, st) in p.stages.iter_mut().enumerate() {
                            if s < used && st.texture.is_some() {
                                if let Some([u, v]) = address {
                                    st.address[0] = u;
                                    st.address[1] = v;
                                }
                                st.point_filter = point;
                            } else if address.is_some() {
                                // Unused stages as the draws leave them: clamped.
                                st.address = [3, 3, 3];
                            }
                        }
                        self.stage_bind_with(library, &p, &None, None, false);
                        self.stage_bind_with(library, &p, &None, None, true);
                    }
                }
            }
        }
    }

    /// After the first frame (the level's cartoon ramps made by its lit actors): the cartoon
    /// group of every texture found by the references prewarm with each of those ramps, as a
    /// mesh drawn later (a weapon, a pickup) would bind it.
    fn warm_up_cartoon(&mut self, library: &mut Library) {
        if self.cartoon_warmed || !self.scenery_warmup {
            return;
        }
        self.cartoon_warmed = true;
        // Every ramp a cartoon draw can use: each zone's light curve and the flash's (0xFF40).
        let mut curves: Vec<u32> = self.zones.iter().map(|z| z.light_curve).collect();
        curves.push(0xFF40);
        curves.sort_unstable();
        curves.dedup();
        for c in &curves {
            let path = format!("#ramp:{c}");
            if !self.textures.contains_key(&path) {
                self.texture(library, &path);
            }
        }
        let mut ramps: Vec<String> = self.textures.keys().filter(|k| k.starts_with("#ramp:")).cloned().collect();
        ramps.sort();
        // Every texture prepared so far (the references' and the scenery's and actors' own).
        let mut textures = std::mem::take(&mut self.precache_textures);
        textures.extend(self.textures.keys().filter(|k| !k.starts_with('#')).cloned());
        textures.sort();
        textures.dedup();
        for t in &textures {
            for r in &ramps {
                let mut key: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD));
                key[0] = (Some(t.clone()), 1, 1, SAMPLER_WORLD);
                key[1] = (Some(r.clone()), 3, 3, SAMPLER_POINT);
                if !self.bind_keys.contains_key(&key) {
                    self.bind_group(library, key);
                }
            }
        }
    }

    /// The pipelines a pass's first draw would create (combiner from its stages), for each face
    /// culling: render texture views draw the whole scenery, surfaces the main view never showed.
    fn warm_up_pass_pipelines(&mut self, pass: &openxiii_assets::material::Pass) {
        let uniform = PassUniform::new(pass, DrawBits::default());
        for cull in 0..3u8 {
            self.pass_pipeline(pass, &uniform, cull, None);
        }
    }

    fn warm_up_scenery(&mut self, library: &mut Library, camera: &Camera) {
        // Pipelines asked by the prewarm are made together at its end, on worker threads.
        self.pipeline_batch = Some(Vec::new());
        self.warm_up_scenery_steps(library, camera);
        self.flush_pipeline_batch();
    }

    fn warm_up_scenery_steps(&mut self, library: &mut Library, camera: &Camera) {
        self.scenery_warmup = true;
        let view = d3d_view(camera);
        // Screen effects (Apply2DEffect): texture and pipelines created at load time, not on the first
        // explosion flash.
        if self.fx.is_none() {
            self.fx = Some(crate::screen_fx::ScreenFxRenderer::new(&self.device, self.color_format, self.fx_side));
        }
        let mut batches = std::mem::take(&mut self.world.batches);
        for (i, batch) in batches.iter_mut().enumerate() {
            let replace = self.world_owners.get(i).is_some_and(Option::is_some);
            let state = &batch.state;
            let lighting = Lighting { lighting: state.lighting, color_vertex: state.color_vertex, lightmap: state.lightmap.is_some(), lighting_only: false };
            // Only fixed materials (their translation advances no state).
            if !self.materials.bind_batch(&mut batch.bound, library, batch.material.as_deref(), replace) {
                continue;
            }
            let (result, _) = self.materials.set_batch_material(&mut batch.bound, library, batch.material.as_deref(), replace, lighting, self.frame, view);
            for pass in &result.passes {
                self.stage_bind(library, pass, &state.lightmap, None);
                self.warm_up_pass_pipelines(pass);
            }
        }
        self.world.batches = batches;
        // Every actor of the list too (meshes with their skins, animated StaticMeshes, pickups,
        // weapons), whether in view or not, as the original's precache frame draws the whole level
        // from zone 0 without a frustum: their texture groups are made now, not when first seen.
        if let Some(mut d) = self.dynamic.take() {
            for (i, batch) in d.geometry.batches.iter_mut().enumerate() {
                let static_mesh = d.owners.get(i).is_some_and(|&o| self.prop_lights.get(o).and_then(Option::as_ref).is_some());
                let state = &batch.state;
                let lighting = Lighting { lighting: state.lighting, color_vertex: state.color_vertex, lightmap: state.lightmap.is_some(), lighting_only: false };
                if !self.materials.bind_batch(&mut batch.bound, library, batch.material.as_deref(), static_mesh) {
                    continue;
                }
                let (result, _) = self.materials.set_batch_material(&mut batch.bound, library, batch.material.as_deref(), static_mesh, lighting, self.frame, view);
                for pass in &result.passes {
                    self.stage_bind(library, pass, &state.lightmap, None);
                    self.warm_up_pass_pipelines(pass);
                }
            }
            self.dynamic = Some(d);
        }
        self.warm_up_references(library, &view);
        // Cartoon groups now: the ramps come from the level's zones, known at load.
        self.warm_up_cartoon(library);
        {
            // Embedded data is available on Android as well as desktop.
            let mut texts: Vec<String> = Vec::new();
            if let Some(t) = list_included(&self.map_value) {
                texts.push(t.to_string());
            }
            if let Some(folder) = folder_warmup_shipped() {
                if let Ok(t) = std::fs::read_to_string(folder.join(format!("{}.txt", self.map_value))) {
                    texts.push(t);
                }
            }
            for text_value in &texts {
                for path in text_value.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                    if path.starts_with("package:") {
                        continue;
                    }
                    if path.starts_with("bind:") {
                        // Texture group (textures and samplers of the stages) first made while
                        // playing on an earlier run: made now, with the scenery's.
                        if let Some(key) = key_bind_read(path) {
                            if !self.bind_keys.contains_key(&key) {
                                self.bind_group(library, key);
                            }
                        }
                        continue;
                    }
                    if path.starts_with("pipeline:") {
                        // Same pipeline as its first draw would have created (combiner read from
                        // the uniforms), created ahead of time.
                        if let Some(key) = key_pipeline_read(path) {
                            if !self.pipeline_ids.contains_key(&key) {
                                self.pipeline(key);
                            }
                        }
                        continue;
                    }
                    if !self.textures.contains_key(path) {
                        self.texture(library, path);
                    }
                }
            }
        }
    }

    pub(crate) fn draw(&mut self, target: &wgpu::TextureView, camera: &Camera, library: &mut Library) {
        if !self.scenery_warmup && !self.world.batches.is_empty() && self.video.is_none() {
            self.warm_up_scenery(library, camera);
        }
        if let Some(show) = self.video {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("video") });
            self.video_pass(&mut encoder, target, show);
            submit_value(&self.queue, [encoder.finish()]);
            return;
        }
        let aspect = self.width as f32 / self.height as f32;
        #[allow(unused_mut)]
        let mut mark_value = |perf: &mut [f64; 8], k: usize| {
            let _ = (perf, k);
        };
        self.update_zone(camera, true);
        self.lightmaps.time += self.frame.delta as f64;
        self.lightmaps.update(&self.queue);
        self.update_uniforms_with(camera, self.view_fov_y(aspect), aspect, self.view_widen(), self.overlay_camera.as_ref());
        mark_value(&mut self.perf_send, 0);
        self.view_window = true;
        self.target_window = target as *const wgpu::TextureView as usize;
        self.prepare(library, camera);
        self.view_window = false;
        // The first frame's preparation has made the level's cartoon ramps.
        self.warm_up_cartoon(library);
        mark_value(&mut self.perf_send, 1);
        self.prepare_canvas(library, camera);
        mark_value(&mut self.perf_send, 2);
        // Shadows and frame in a single upload (shadows first, as with their two former uploads).
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("image") });
        self.render_shadows(library, &mut encoder);
        mark_value(&mut self.perf_send, 3);
        // Sky first, then the map after clearing the depth (Clear(0, 0, 1, 1.0, 1, 0)).
        if self.only_post_render {
            // FCameraSceneNode::Render: viewport+0x120 skips FLevelSceneNode::Render,
            // the shake and the veil; only PostRender draws (on a frame cleared here).
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("without a scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        } else if !self.fx_passes.is_empty() {
            // Scene into the 512 × 512 texture, then Apply2DEffect
            // redraws it into the frame, before the Canvas (FCameraSceneNode::Render).
            let mut fx = self.fx.take().unwrap_or_else(|| crate::screen_fx::ScreenFxRenderer::new(&self.device, self.color_format, self.fx_side));
            let side = fx.side as f32;
            self.level_passes(&mut encoder, &fx.view, &fx.depth, true, None, [side; 2], library);
            self.camera_tiles(&mut encoder, &fx.view, None, [side; 2], library);
            fx.draw(&self.device, &self.queue, &mut encoder, target, &self.fx_passes, self.width, self.height);
            self.fx = Some(fx);
            // End of Apply2DEffect: Clear(0, 0, D3DCLEAR_ZBUFFER | D3DCLEAR_STENCIL).
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effects: depth"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        } else if let Some((color, depth, (tw, th))) = self.world_target_views() {
            // Render scale or MSAA: the world in its own target, copied into the frame before the
            // Canvas (drawn at the frame's resolution).
            let size = [tw as f32, th as f32];
            self.target_window = &color as *const wgpu::TextureView as usize;
            self.pass_samples.set(self.samples);
            self.level_passes(&mut encoder, &color, &depth, true, None, size, library);
            self.camera_tiles(&mut encoder, &color, None, size, library);
            self.pass_samples.set(1);
            self.target_window = target as *const wgpu::TextureView as usize;
            self.copy_world(&mut encoder, target, [0.0, 0.0, size[0], size[1]], [0.0, 0.0, self.width as f32, self.height as f32]);
        } else {
            let depth = self.depth.clone();
            let size = [self.width as f32, self.height as f32];
            self.level_passes(&mut encoder, target, &depth, true, None, size, library);
            self.camera_tiles(&mut encoder, target, None, size, library);
        }
        self.canvas_pass(&mut encoder, target, 0, None);
        let finished_at = encoder.finish();
        mark_value(&mut self.perf_send, 4);
        submit_value(&self.queue, [finished_at]);
        mark_value(&mut self.perf_send, 5);
        // UCanvas::execDrawPortal: FCameraSceneNode of the portal's camera in its
        // rectangle (SetViewport), projection (FOV, W, H), then the rest of the Canvas.
        let saved_fov = self.fov;
        // The portal has its own visibility, not tracked here: all the scenery.
        let saved_cull = self.world_masked.take();
        let saved_light_passes = self.scene_light_passes.clone();
        for (k, portal) in self.portals.clone().into_iter().enumerate() {
            self.scene_light_passes = self.scene_light_views.portals.get(&portal.hud_index).cloned().unwrap_or_default();
            let (w, h) = (self.width as f32, self.height as f32);
            let x = portal.rect[0].clamp(0.0, w - 1.0);
            let y = portal.rect[1].clamp(0.0, h - 1.0);
            let rect = [x, y, (portal.rect[0] + portal.rect[2]).min(w) - x, (portal.rect[1] + portal.rect[3]).min(h) - y];
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("portal") });
            if rect[2] >= 1.0 && rect[3] >= 1.0 {
                let aspect = portal.rect[2].max(1.0) / portal.rect[3].max(1.0);
                self.fov = Some(portal.fov);
                self.in_camera_view = true;
                self.update_zone(&portal.camera, true);
                self.update_uniforms_with(&portal.camera, self.fov_y(aspect), aspect, None, None);
                self.prepare(library, &portal.camera);
                self.prepare_canvas(library, &portal.camera);
                let depth = self.depth.clone();
                self.level_passes(&mut encoder, target, &depth, false, Some(rect), [w, h], library);
                let main_filter = std::mem::replace(&mut self.color_filter, portal.filter);
                self.camera_tiles(&mut encoder, target, Some(rect), [w, h], library);
                self.color_filter = main_filter;
            }
            self.canvas_pass(&mut encoder, target, k + 1, None);
            submit_value(&self.queue, [encoder.finish()]);
        }
        self.scene_light_passes = saved_light_passes;
        self.world_masked = saved_cull;
        self.in_camera_view = false;
        self.fov = saved_fov;
        mark_value(&mut self.perf_send, 6);
    }

    fn tally(&self, pipeline: bool, group_value: bool, geometry_value: bool) {
        return;
    }

    /// After the traversal, into `target` (size `size` in pixels): the sky (child
    /// scene, with its own end), the map (opaque then translucent, depth
    /// cleared after the sky), if UseVignetteFilter (PlayerController+0x5B0), the
    /// bIgnoreVignetteAlpha actors set aside when UseVignetteHighlight (+0x5B1),
    /// if UseVignetteHighlight, then the deferred actors after Clear(0, 0xFF000000, 1, 1.0).
    /// `rect`: viewport frame in pixels of `target` (None: the whole target); `clear`:
    /// colour cleared first.
    #[allow(clippy::too_many_arguments)]
    fn level_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        clear: bool,
        rect: Option<[f32; 4]>,
        size: [f32; 2],
        library: &mut Library,
    ) {
        let sky = self.sky.is_some();
        if sky {
            self.scene_pass_with(encoder, target, depth, clear, rect, Stage::Sky { vignette: false });
            self.vignette_end(encoder, target, depth, rect, size, Stage::Sky { vignette: true }, library);
        }
        self.scene_pass_with(encoder, target, depth, clear && !sky, rect, Stage::Scene);
        self.vignette_end(encoder, target, depth, rect, size, Stage::Vignette, library);
        self.scene_pass_with(encoder, target, depth, false, rect, Stage::Delayed);
    }

    /// FCameraSceneNode::Render after FLevelSceneNode::Render, before Apply2DEffect
    /// and the HUD: DrawTile of the whole viewport (FCanvasUtil) in colour `flash`, FinalBlend
    /// (FB_AlphaBlend, WhiteTexture); then that of the colour filter, FinalBlend
    /// (FB_Modulate).
    fn camera_tiles(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, rect: Option<[f32; 4]>, size: [f32; 2], library: &mut Library) {
        let frame = rect.unwrap_or([0.0, 0.0, size[0], size[1]]);
        let rgba = |c: [u8; 4]| c.map(|k| k as f32 / 255.0);
        if let Some(c) = self.flash {
            self.filter_tile(encoder, target, frame, rgba(c), size, false, None, library);
        }
        if let Some(c) = self.color_filter {
            self.filter_tile(encoder, target, frame, rgba(c), size, true, None, library);
        }
    }

    /// End before the deferred actors: (UseVignetteFilter), the
    /// bIgnoreVignetteAlpha actors (`stage`), (UseVignetteHighlight), over the whole frame.
    #[allow(clippy::too_many_arguments)]
    fn vignette_end(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        rect: Option<[f32; 4]>,
        size: [f32; 2],
        stage: Stage,
        library: &mut Library,
    ) {
        let frame = rect.unwrap_or([0.0, 0.0, size[0], size[1]]);
        let vignette = self.vignette.clone();
        if let Some((color, texture)) = &vignette.filter {
            self.vignette_filter(encoder, target, frame, *color, texture, size, library);
        }
        self.scene_pass_with(encoder, target, depth, false, rect, stage);
        if let Some(h) = vignette.highlight {
            self.vignette_highlight(encoder, target, frame, h, size, library);
        }
    }

    /// FilterTexture (VignetteMaterial, a UTexture clamped in U and V) or
    /// WhiteTexture, in colour VignetteColor, over the whole frame, through an FB_AlphaBlend FinalBlend
    /// without depth write.
    #[allow(clippy::too_many_arguments)]
    fn vignette_filter(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: [f32; 4],
        color: [u8; 4],
        texture: &str,
        size: [f32; 2],
        library: &mut Library,
    ) {
        let texture = (!texture.is_empty()).then_some(texture);
        self.filter_tile(encoder, target, frame, color.map(|k| k as f32 / 255.0), size, false, texture, library);
    }

    /// FB_Modulate FinalBlend (2 × source × destination) of WhiteTexture over the whole
    /// frame, f = (VignetteHighLight + 1) × 0.5; if f > 1, a first white rectangle then
    /// f − 0.5.
    fn vignette_highlight(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: [f32; 4],
        highlight: f32,
        size: [f32; 2],
        library: &mut Library,
    ) {
        let mut f = (highlight + 1.0) * 0.5;
        if f > 1.0 {
            self.filter_tile(encoder, target, frame, [1.0; 4], size, true, None, library);
            f -= 0.5;
        }
        self.filter_tile(encoder, target, frame, [f; 4], size, true, None, library);
    }

    /// Pass of a scene stage with the uniforms already written; `viewport` in pixels.
    /// Deferred: the player's weapon and hands (bDelayDisplay), 50° projection for those with
    /// bSpecialDelayFov.
    fn scene_pass_with(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        clear: bool,
        viewport: Option<[f32; 4]>,
        stage: Stage,
    ) {
        let wanted = |c: &DrawCmd| {
            !c.canvas
                && match stage {
                    // Child sky scene: the content of the sky zone, not the
                    // particles, weather and projectors of the player's scene.
                    Stage::Sky { vignette } => {
                        crate::lights_scene::belongs_to_stage(c.light_sky, true)
                            && !c.outside_sky
                            && c.overlay == 0
                            && c.vignette == vignette
                    }
                    Stage::Scene => crate::lights_scene::belongs_to_stage(c.light_sky, false) && c.overlay == 0 && !c.vignette,
                    Stage::Vignette => crate::lights_scene::belongs_to_stage(c.light_sky, false) && c.overlay == 0 && c.vignette,
                    Stage::Delayed => crate::lights_scene::belongs_to_stage(c.light_sky, false) && c.overlay != 0,
                }
        };
        let set_aside = matches!(stage, Stage::Vignette | Stage::Delayed | Stage::Sky { vignette: true });
        if set_aside && !self.commands.iter().any(wanted) {
            return;
        }
        let delayed = stage == Stage::Delayed;
        let note = self.note_read_flag.get();
        if note {
            for b in [&self.frame_buffer, &self.overlay_frame_buffer, &self.overlay_eye_frame_buffer, &self.sky_frame_buffer, &self.pass_buffers.buffer] {
                note_read_back(&self.queue, b);
            }
        }
        // Actors set aside keep the scene's depth; elsewhere it is cleared
        // (sky, map after the sky, deferred actors).
        let keep_depth = matches!(stage, Stage::Vignette | Stage::Sky { vignette: true });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(match stage {
                Stage::Sky { .. } => "sky",
                Stage::Scene if self.outline_boost == 3.0 => "comic panel",
                Stage::Scene => "map",
                Stage::Vignette => "outside vignette",
                Stage::Delayed => "first-person view",
            }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        let c = self.clear_color.map(|v| v as f64 / 255.0);
                        wgpu::LoadOp::Clear(wgpu::Color { r: c[0], g: c[1], b: c[2], a: 1.0 })
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations { load: if keep_depth { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(1.0) }, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some([x, y, w, h]) = viewport {
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
        }
        pass.set_bind_group(
            0,
            match stage {
                Stage::Sky { .. } => &self.sky_frame_group,
                Stage::Delayed => &self.overlay_frame_group,
                _ => &self.frame_group,
            },
            &[],
        );
        // Opaque first, then the materials to sort (RequiresSorting), like the engine.
        let mut prec: Option<&DrawCmd> = None;
        let steps = false;
        let mut step: Option<(Which, bool, bool)> = None;
        // split_projectors: rectangles of the main view (not the comic panels or the sky).
        let rects = viewport.is_none() && self.target_window == target as *const wgpu::TextureView as usize && !self.projector_rects.is_empty() && matches!(stage, Stage::Scene | Stage::Vignette) && self.outline_boost != 3.0;
        let mut scissor = false;
        for sorted in [false, true] {
            let mut current: Option<Which> = None;
            let mut bound_compact = false;
            let mut frame = if delayed { None } else { Some(0) };
            for c in self.commands.iter().filter(|c| wanted(c) && c.sorted == sorted) {
                if delayed && frame != Some(c.overlay) {
                    let group = match c.overlay {
                        DELAYED_FOV => &self.overlay_frame_group,
                        DELAYED_FOV_EYE => &self.overlay_eye_frame_group,
                        _ => &self.delayed_frame_group,
                    };
                    pass.set_bind_group(0, group, &[]);
                    frame = Some(c.overlay);
                }
                let compacted = match (&self.world_compact, &self.world_masked, &self.world_culled) {
                    (Some(_), Some(_), None) if c.which == Which::World => {
                        compact_range(if matches!(stage, Stage::Sky { .. }) { &self.world_masked_kept.1 } else { &self.world_masked_kept.0 }, c.first_index, c.index_count)
                    }
                    _ => None,
                };
                let geometry_value = current != Some(c.which);
                if current == Some(Which::World) && c.which == Which::World && bound_compact != compacted.is_some() {
                    // Same geometry, another index buffer: range not made of whole triangles.
                    current = None;
                }
                if current != Some(c.which) {
                    let g = match c.which {
                        Which::World => &self.world,
                        Which::Replaced => &self.replaced.as_ref().unwrap().0,
                        Which::Dynamic => &self.dynamic.as_ref().unwrap().geometry,
                        Which::Particles => &self.particles.as_ref().unwrap().0,
                        Which::Projectors => &self.projectors.as_ref().unwrap().0,
                        Which::Weather => &self.weather.as_ref().unwrap().0,
                        Which::Canvas => &self.canvas.as_ref().unwrap().0,
                    };
                    pass.set_vertex_buffer(0, g.vertex_buffer.slice(..));
                    if note {
                        note_read_back(&self.queue, &g.vertex_buffer);
                    }
                    let index = match (c.which, &self.world_culled, &self.world_masked, &self.world_compact) {
                        (Which::Particles, _, _, _) if self.particle_packed_active => self.particle_packed_buffer.as_ref().unwrap(),
                        (Which::World, Some((buf, _)), _, _) => buf,
                        // Sky: nodes of its scenes; map: those of the player's view.
                        (Which::World, None, Some(_), Some((map_value, sky_value))) if compacted.is_some() => if matches!(stage, Stage::Sky { .. }) { sky_value } else { map_value },
                        (Which::World, None, Some((map_value, sky_value)), _) => if matches!(stage, Stage::Sky { .. }) { sky_value } else { map_value },
                        _ => &g.index_buffer,
                    };
                    pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                    if note {
                        note_read_back(&self.queue, index);
                    }
                    current = Some(c.which);
                    bound_compact = compacted.is_some();
                }
                if c.which == Which::World && self.world_culled.is_none() && self.world_masked.is_some() && c.first_index % 3 == 0 && c.index_count % 3 == 0 {
                    let kept = if matches!(stage, Stage::Sky { .. }) { &self.world_masked_kept.1 } else { &self.world_masked_kept.0 };
                    let (t0, t1) = ((c.first_index / 3) as usize, ((c.first_index + c.index_count) / 3) as usize);
                    if kept.get(t1).zip(kept.get(t0)).is_some_and(|(b, a)| b == a) {
                        continue;
                    }
                }
                self.tally(prec.is_none_or(|p: &DrawCmd| p.pipeline != c.pipeline), prec.is_none_or(|p: &DrawCmd| p.bind != c.bind), geometry_value);
                // Pipeline and groups sent to wgpu again only if they change within the pass
                // (all of them after a pipeline change).
                let fresh_list_new = prec.is_none_or(|p: &DrawCmd| p.pipeline != c.pipeline);
                if self.pipelines[c.pipeline].0.impact && prec.is_none_or(|p| !self.pipelines[p.pipeline].0.impact) {
                    let matrix = self.impact_matrices_gpu.as_ref().unwrap();
                    pass.set_vertex_buffer(1, matrix.slice(..));
                    if note { note_read_back(&self.queue, matrix); }
                }
                if fresh_list_new {
                    pass.set_pipeline(self.scene_pipeline(c.pipeline));
                }
                // Groups kept by wgpu from one pipeline to another (compatible layouts):
                // re-sent only if they change.
                let all_list = fresh_list_new && groups_after_pipeline();
                if all_list || prec.is_none_or(|p: &DrawCmd| p.pass_offset != c.pass_offset) {
                    pass.set_bind_group(1, &self.pass_buffers.group, &[c.pass_offset]);
                }
                if all_list || prec.is_none_or(|p: &DrawCmd| p.bind != c.bind) {
                    pass.set_bind_group(2, &self.bind_groups[c.bind], &[]);
                }
                prec = Some(c);
                let (first_one, count_value) = compacted.unwrap_or((c.first_index, c.index_count));
                // split_projectors: the projector's draw limited to its screen rectangle.
                if rects {
                    let r = if c.which == Which::Projectors { self.rect_projectors(c.first_index, c.index_count) } else { None };
                    match r {
                        Some([_, _, 0, _]) | Some([_, _, _, 0]) => continue,
                        Some([x, y, w, h]) => {
                            pass.set_scissor_rect(x, y, w, h);
                            scissor = true;
                        }
                        None if scissor => {
                            pass.set_scissor_rect(0, 0, self.projector_rects_size[0], self.projector_rects_size[1]);
                            scissor = false;
                        }
                        None => {}
                    }
                }
                let shadow = steps && c.which == Which::Projectors && self.projector_shadows.binary_search(&c.first_index).is_ok();
                if shadow {
                    let px = match (rects, self.rect_projectors(c.first_index, c.index_count)) {
                        (true, Some([_, _, w, h])) => w as u64 * h as u64,
                        _ => self.width as u64 * self.height as u64,
                    };
                    SHADOWS_RADIO[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    SHADOWS_RADIO[1].fetch_add(px, std::sync::atomic::Ordering::Relaxed);
                    SHADOWS_RADIO[2].fetch_add(c.index_count as u64 / 3, std::sync::atomic::Ordering::Relaxed);
                }
                if steps && step != Some((c.which, sorted, shadow)) {
                    if step.is_some() {
                        pass.pop_debug_group();
                    }
                    let kind_index = match (c.which, shadow) {
                        (Which::Projectors, true) => " shadows",
                        (Which::Projectors, false) => " others",
                        _ => "",
                    };
                    pass.push_debug_group(&format!("{:?}{kind_index}{}", c.which, if sorted { " sorted" } else { "" }));
                    step = Some((c.which, sorted, shadow));
                }
                pass.draw_indexed(first_one..first_one + count_value, 0, 0..1);
            }
        }
        if step.is_some() {
            pass.pop_debug_group();
        }
    }

    /// Canvas primitives, in emission order, over the frame.
    /// `clip`: rectangle of a split-screen viewport (window pixels). The engine draws each
    /// viewport's Canvas after D3D's SetViewport at UViewport::Lock, so its primitives cannot
    /// spill into the other viewports (the zoomed crosshair's cover tiles get a negative
    /// height in a half-height viewport).
    fn canvas_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, segment: usize, clip: Option<[f32; 4]>) {
        let Some((geometry, _)) = &self.canvas else {
            return;
        };
        if !self.commands.iter().any(|c| c.canvas && c.segment == segment) {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some([x, y, w, h]) = clip {
            pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
        }
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_vertex_buffer(0, geometry.vertex_buffer.slice(..));
        pass.set_index_buffer(geometry.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        let mut prec: Option<&DrawCmd> = None;
        for c in self.commands.iter().filter(|c| c.canvas && c.segment == segment) {
            self.tally(prec.is_none_or(|p| p.pipeline != c.pipeline), prec.is_none_or(|p| p.bind != c.bind), prec.is_none());
            let fresh_list_new = prec.is_none_or(|p| p.pipeline != c.pipeline);
            if fresh_list_new {
                pass.set_pipeline(&self.pipelines[c.pipeline].1);
            }
            let all_list = fresh_list_new && groups_after_pipeline();
            if all_list || prec.is_none_or(|p| p.pass_offset != c.pass_offset) {
                pass.set_bind_group(1, &self.pass_buffers.group, &[c.pass_offset]);
            }
            if all_list || prec.is_none_or(|p| p.bind != c.bind) {
                pass.set_bind_group(2, &self.bind_groups[c.bind], &[]);
            }
            prec = Some(c);
            pass.draw_indexed(c.first_index..c.first_index + c.index_count, 0, 0..1);
        }
    }

    /// One more frame uniform for a projected shadow's silhouette view.
    fn add_shadow_frame(&mut self) {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow frame"),
            layout: &self.frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        self.shadow_frames_extra.push((buffer, group));
    }

    /// Rain or snow buffers for `quads` flakes.
    fn weather_geometry(&self, quads: usize) -> Geometry {
        let buffer = |size: usize, usage: wgpu::BufferUsages| {
            self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("weather"), size: size.next_multiple_of(4) as u64, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
        };
        Geometry {
            vertex_buffer: buffer(quads * 4 * std::mem::size_of::<SceneVertex>(), wgpu::BufferUsages::VERTEX),
            index_buffer: buffer(quads * 6 * 4, wgpu::BufferUsages::INDEX),
            batches: Vec::new(),
        }
    }

    /// A render texture of `side` texels and its depth.
    fn make_target(&self, side: u32) -> (wgpu::TextureView, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render texture"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        (texture.create_view(&wgpu::TextureViewDescriptor::default()), depth_view(&self.device, side, side))
    }

    /// At load: the render textures, their pass buffer and filter rectangles the script's first
    /// render texture views use (comic panels, cameras), so none is made while playing.
    fn warm_up_targets(&mut self, library: &mut Library, keys: &[String]) {
        if empty_window() {
            return;
        }
        let side = TARGET_SIDE as u32 * self.target_scale;
        self.spare_targets.retain(|t| t.0 == side);
        // The render textures the scripts already made ("#n"): their texture and their group as
        // the HUD's DrawTile binds them (screen sampler, clamped).
        // The texture first, then its group, which binds it (as render_target makes the groups
        // again when it makes a render texture).
        for key in keys {
            if !self.hud.targets.contains_key(key) {
                let (view, depth) = self.make_target(side);
                self.hud.targets.insert(key.clone(), Target { rendered: Vec::new(), view, depth });
            }
            let mut bind: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_SCREEN));
            bind[0] = (Some(key.clone()), 3, 3, SAMPLER_SCREEN);
            if !self.bind_keys.contains_key(&bind) {
                self.bind_group(library, bind);
            }
            // As a projector's texture: over the projector gradient, like every projected material.
            let mut projected: BindKey = std::array::from_fn(|_| (None, 3, 3, SAMPLER_WORLD));
            projected[0] = (Some("Engine.ProjGradient".to_string()), 3, 3, SAMPLER_WORLD);
            projected[1] = (Some(key.clone()), 3, 3, SAMPLER_WORLD);
            if !self.bind_keys.contains_key(&projected) {
                self.bind_group(library, projected);
            }
        }
        while self.spare_targets.len() < SPARE_TARGETS {
            let (view, depth) = self.make_target(side);
            self.spare_targets.push((side, view, depth));
        }
        // As large as the main view's (a render texture view draws the whole scenery).
        if self.target_pass_buffers.as_ref().is_none_or(|b| b.capacity < self.pass_buffers.capacity) {
            self.target_pass_buffers = Some(Self::pass_buffer(&self.device, &self.pass_layout, self.pass_buffers.capacity));
        }
        // A whole-window viewport's Apply2DEffect image and the quad copying it (first flash or
        // blur of the level).
        let (w, h) = (self.width, self.height);
        self.ensure_fx_view_target(w, h);
        self.fx_view_blit_quad([0.0, 0.0, w as f32, h as f32]);
        // The ordered writes ring of the render texture views (written while the main view's
        // passes are pending).
        if submission_unique() {
            prewarm_ring(&self.device, &self.queue);
        }
        // The filter rectangles' white group (scene and sky filters, force-clear).
        if self.white_group.is_none() {
            self.white_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("white"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.white) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            }));
        }
        // A frame uniform for each of the eight shadow targets (one is the frame's own).
        while self.shadow_frames_extra.len() + 1 < 8 {
            self.add_shadow_frame();
        }
        // Rain and snow: one set of buffers for a shower of up to WEATHER_QUADS flakes drawn.
        if self.weather.is_none() && self.weather_spare.is_none() {
            self.weather_spare = Some(self.weather_geometry(WEATHER_QUADS));
        }
        for slot in 0..self.filter_tiles.len() {
            while self.filter_tiles[slot].len() < 2 {
                let tile = FilterTileBuffers::new(&self.device, &self.hud.pipeline, [0.0; 4], [0.0; 4], [1.0; 2]);
                self.filter_tiles[slot].push(tile);
            }
        }
    }

    /// Renders camera views into render texture `key` ("#n"), like
    /// RenderTargetMaterial.Update: each view in its rectangle.
    pub fn render_target(&mut self, key: &str, views: &[TargetView], library: &mut Library) {
        if empty_window() {
            return;
        }
        let side = TARGET_SIDE as u32 * self.target_scale;
        if !self.hud.targets.contains_key(key) {
            let (view, depth) = match self.spare_targets.iter().position(|t| t.0 == side) {
                Some(i) => {
                    let (_, view, depth) = self.spare_targets.swap_remove(i);
                    (view, depth)
                }
                None => self.make_target(side),
            };
            self.hud.targets.insert(key.to_string(), Target { rendered: Vec::new(), view, depth });
            // A group made before this texture existed (the HUD's first DrawTile of its comic panel
            // material, before the first Update) holds the white texture: made again with the render
            // texture, as the original's panels show the camera views (SSH102b's objectives screen).
            for i in 0..self.bind_key_list.len().min(self.bind_groups.len()) {
                if self.bind_key_list[i].iter().any(|(t, ..)| t.as_deref() == Some(key)) {
                    let k = self.bind_key_list[i].clone();
                    self.bind_groups[i] = self.make_bind_group(library, &k);
                }
            }
        }
        // URenderTargetMaterial::Update renders the view at the time of the call: only the
        // views of a new call (changed number) are rendered, the others keep their image.
        let fresh: Vec<TargetView> = views.iter().filter(|v| !self.hud.targets[key].rendered.contains(v)).cloned().collect();
        if fresh.is_empty() {
            return;
        }
        // A comic panel camera produces a list different from the main view.
        // Keeping a second buffer avoids replacing every entry of the main
        // view before rewriting them on the next draw. A single reserve of
        // this kind is enough, whatever the number of cameras or textures.
        let main_passes = fresh.iter().any(|view| view.filter[3] < 255).then(|| {
            let secondary = self.target_pass_buffers.take().unwrap_or_else(|| Self::pass_buffer(&self.device, &self.pass_layout, 64));
            std::mem::replace(&mut self.pass_buffers, secondary)
        });
        let (saved_fog, saved_sky, saved_sky_fog, saved_per_zone) = (self.fog, self.sky, self.sky_fog, self.fog_per_zone);
        // Render texture view: its visibility is not tracked, all the scenery.
        let saved_cull = self.world_masked.take();
        let saved_light_passes = self.scene_light_passes.clone();
        for (i, v) in fresh.iter().enumerate() {
            self.scene_light_passes = self.scene_light_views.targets.get(&v.serial).cloned().unwrap_or_default();
            let s = self.target_scale as f32;
            let rect = [v.rect[0] * s, v.rect[1] * s, (v.rect[2] * s).max(1.0), (v.rect[3] * s).max(1.0)];
            let rect = [rect[0].min(side as f32 - 1.0), rect[1].min(side as f32 - 1.0), rect[2].min(side as f32 - rect[0]), rect[3].min(side as f32 - rect[1])];
            // (ViewFOV × 0.5 × 3.1415265 / 180, width, height, 5, 65536): m[0] =
            // 1 / tan, m[5] = (width / tan) / height, the aspect being that of the rectangle.
            let aspect = v.rect[2].max(1.0) / v.rect[3].max(1.0);
            let half = v.fov * 0.5 * 3.141_526_5 / 180.0;
            let fov_y = 2.0 * (half.tan() / aspect).atan();
            // The player's FovAngle = ViewFOV during the render, outlines three times thicker;
            // the scene is only rendered if FilterColor.A < 255.
            let saved_fov = self.fov;
            self.fov = Some(v.fov);
            self.outline_boost = 3.0;
            self.in_camera_view = true;
            // Single submission: the frame's pending encoder, returned for the writes
            // (view uniforms, passes) so they land in order.
            let unique = submission_unique();
            let mut encoder = Some(if unique {
                take_encoder(&self.device, &self.queue)
            } else {
                self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render texture") })
            });
            let (view, depth) = (self.hud.targets[key].view.clone(), self.hud.targets[key].depth.clone());
            let size = [side as f32; 2];
            // CamLocation's zone with bForceClear: DrawTile WhiteTexture of DistanceFogColor
            // over the whole frame (FCanvasUtil, without blending).
            if let Some(c) = v.force_clear {
                let color = [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0];
                self.filter_tile(encoder.as_mut().unwrap(), &view, rect, color, size, false, None, library);
            }
            // Viewport vignette for the view: UseVignetteFilter = 1, UseVignetteHighlight
            // = HighLight > 0, VignetteColor = FilterColor, VignetteHighLight = |HighLight|,
            // VignetteMaterial = FilterTexture.
            let vignette =
                openxiii_vm::visibility::Vignette { filter: Some((v.filter, v.filter_texture.clone())), highlight: v.high_light.then_some(v.highlight) };
            let saved_vignette = std::mem::replace(&mut self.vignette, vignette);
            if v.filter[3] < 255 {
                // FLevelSceneNode::Render: fog and sky of the camera's
                // zone, then (filter and highlight included).
                self.update_zone(&v.camera, false);
                if unique {
                    return_encoder(&self.queue, encoder.take().unwrap());
                }
                let (scene, sky) = self.update_uniforms_with(&v.camera, fov_y, aspect, None, None);
                let clip = target_clip::TargetClip { scene: target_clip::Frustum::new(scene), sky: sky.map(target_clip::Frustum::new) };
                let enabled = true;
                if enabled { self.target_range_cache.begin(scene, sky, self.target_world_bounds.len()); }
                let t_prep = std::time::Instant::now();
                self.prepare_view(library, &v.camera, enabled.then_some(&clip));
                timer_target(0, t_prep);
                if unique {
                    encoder = Some(take_encoder(&self.device, &self.queue));
                }
                let t_passes = std::time::Instant::now();
                self.note_read_flag.set(unique);
                self.level_passes(encoder.as_mut().unwrap(), &view, &depth, false, Some(rect), size, library);
                self.note_read_flag.set(false);
                timer_target(1, t_passes);
            } else {
                // Without the scene: if bHighLight.
                self.vignette_filter(encoder.as_mut().unwrap(), &view, rect, v.filter, &v.filter_texture, size, library);
                if v.high_light {
                    self.vignette_highlight(encoder.as_mut().unwrap(), &view, rect, v.highlight, size, library);
                }
            }
            self.vignette = saved_vignette;
            self.outline_boost = 1.0;
            self.in_camera_view = false;
            self.fov = saved_fov;
            if i + 1 == fresh.len() {
                self.opaque_alpha(encoder.as_mut().unwrap(), &view, size, library);
            }
            let t_send = std::time::Instant::now();
            if unique {
                return_encoder(&self.queue, encoder.take().unwrap());
            } else {
                submit_value(&self.queue, [encoder.take().unwrap().finish()]);
            }
            timer_target(2, t_send);
        }
        if let Some(main) = main_passes {
            self.target_pass_buffers = Some(std::mem::replace(&mut self.pass_buffers, main));
            self.passes_written = 0;
        }
        self.scene_light_passes = saved_light_passes;
        (self.fog, self.sky, self.sky_fog, self.fog_per_zone) = (saved_fog, saved_sky, saved_sky_fog, saved_per_zone);
        self.world_masked = saved_cull;
        if let Some(t) = self.hud.targets.get_mut(key) {
            t.rendered.clear();
            t.rendered.extend_from_slice(views);
        }
    }

    /// Render texture `key` with alpha 1: D3DDrv creates the render targets requested in
    /// A8R8G8B8 (0x15) as X8R8G8B8 (0x16), without alpha, read back as 1. The shadows'
    /// FB_Darken (INVSRCCOLOR) therefore removes nothing there; here it would clear the comic panel's
    /// alpha (e.g. a bridge under a guard's shadow on Plage01: black panel).
    fn opaque_alpha(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, size: [f32; 2], library: &mut Library) {
        self.alpha_only = true;
        self.filter_tile(encoder, view, [0.0, 0.0, size[0], size[1]], [1.0; 4], size, false, None, library);
        self.alpha_only = false;
    }

    /// Rectangle of colour `color` (R, G, B, A) alpha-blended (or FB_Modulate with
    /// `modulate`) within frame `rect` of `target` (`size` pixels).
    /// `texture`: FilterTexture stretched over the frame, instead of WhiteTexture.
    #[allow(clippy::too_many_arguments)]
    fn filter_tile(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        rect: [f32; 4],
        color: [f32; 4],
        size: [f32; 2],
        modulate: bool,
        texture: Option<&str>,
        library: &mut Library,
    ) {
        let bd = self.outline_boost == 3.;
        // WhiteTexture × zero alpha, SrcAlpha/InvSrcAlpha and One/InvSrcAlpha:
        // the filter keeps exactly the four bytes of the UNORM pixel.
        if bd && !self.alpha_only && !modulate && texture.is_none() && color[3] == 0.
            && matches!(self.color_format, wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm) {
            return;
        }
        // The filter texture's group, made once per texture (it was made again on every call).
        let filter_group = match texture {
            Some(path) if self.filter_texture_groups.contains_key(path) => self.filter_texture_groups.get(path).cloned(),
            Some(path) => {
                let group = self.texture(library, path).map(|view| {
                    use openxiii_assets::material::d3d as t;
                    let sampler = self.sampler_for(t::TADDRESS_CLAMP, t::TADDRESS_CLAMP, SAMPLER_SCREEN).clone();
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("FilterTexture"),
                        layout: &self.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                        ],
                    })
                });
                if let Some(g) = &group {
                    self.filter_texture_groups.insert(path.to_string(), g.clone());
                }
                group
            }
            None => None,
        };
        if self.white_group.is_none() {
            self.white_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("white"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.white) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            }));
        }
        // Scene and sky filter, highlight, force-clear: their
        // quads alternate but reuse the same bytes in each view.
        let index = if self.alpha_only { 1 }
                    else if !bd { 0 }
                    else if modulate && color == [1.; 4] { 5 }
                    else if modulate { 4 }
                    else if color[3] == 1. { 3 }
                    else { 2 };
        let slot = &mut self.filter_tiles[index];
        let bounds = rect.map(f32::to_bits);
        let rgba = color.map(f32::to_bits);
        let dimensions = size.map(f32::to_bits);
        let frame_now = self.frame.count;
        // A few tiles per kind (scene and sky filters alternate in one frame): the one with these
        // values, else one not drawn this frame rewritten with them (a fade), else a new one.
        let found = slot.iter().position(|t| t.rect == bounds && t.color == rgba && t.size == dimensions);
        let k = match found {
            Some(k) => k,
            None => match slot.iter().position(|t| t.frame != frame_now) {
                Some(k) => {
                    slot[k].rewrite(&self.queue, rect, color, size);
                    k
                }
                None => {
                    slot.push(FilterTileBuffers::new(&self.device, &self.hud.pipeline, rect, color, size));
                    slot.len() - 1
                }
            },
        };
        let tile = &mut slot[k];
        tile.frame = frame_now;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("filter"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // Over the multisampled world target: the same pipelines at its sample count.
        let msaa = self.hud.msaa.as_ref().filter(|(n, _)| *n == self.pass_samples.get() && *n > 1).map(|(_, p)| p);
        pass.set_pipeline(match (msaa, self.alpha_only, modulate) {
            (Some(p), true, _) => &p[1],
            (Some(p), false, true) => &p[2],
            (Some(p), false, false) => &p[0],
            (None, true, _) => &self.hud.alpha_pipeline,
            (None, false, true) => &self.hud.modulate_pipeline,
            (None, false, false) => &self.hud.pipeline,
        });
        pass.set_bind_group(0, &tile.uniform, &[]);
        pass.set_bind_group(1, filter_group.as_ref().or(self.white_group.as_ref()).unwrap(), &[]);
        pass.set_vertex_buffer(0, tile.vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
}

impl FilterTileBuffers {
    fn quad(rect: [f32; 4], color: [f32; 4]) -> [HudVertex; 6] {
        let [x, y, w, h] = rect;
        let corner = |px: f32, py: f32, u: f32, v: f32| HudVertex { position: [px, py], uv: [u, v], color };
        [
            corner(x, y, 0.0, 0.0),
            corner(x + w, y, 1.0, 0.0),
            corner(x, y + h, 0.0, 1.0),
            corner(x + w, y, 1.0, 0.0),
            corner(x + w, y + h, 1.0, 1.0),
            corner(x, y + h, 0.0, 1.0),
        ]
    }

    fn new(device: &wgpu::Device, pipeline: &wgpu::RenderPipeline, rect: [f32; 4], color: [f32; 4], size: [f32; 2]) -> Self {
        use wgpu::util::DeviceExt;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("filter"),
            contents: bytemuck::cast_slice(&Self::quad(rect, color)),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("filter"),
            contents: bytemuck::cast_slice(&[size[0], size[1], 0.0f32, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("filter"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        Self { rect: rect.map(f32::to_bits), color: color.map(f32::to_bits), size: size.map(f32::to_bits), vertices, uniform, uniform_buffer, frame: u32::MAX }
    }

    /// The same quad and uniform with new values (no buffer or group made while playing).
    fn rewrite(&mut self, queue: &wgpu::Queue, rect: [f32; 4], color: [f32; 4], size: [f32; 2]) {
        write_value(queue, &self.vertices, 0, bytemuck::cast_slice(&Self::quad(rect, color)));
        write_value(queue, &self.uniform_buffer, 0, bytemuck::cast_slice(&[size[0], size[1], 0.0f32, 0.0]));
        self.rect = rect.map(f32::to_bits);
        self.color = color.map(f32::to_bits);
        self.size = size.map(f32::to_bits);
    }
}

impl Renderer {
    /// Canvas primitives of the frame (FCanvasUtil): vertices passed in screen coordinates
    /// (Direct3D 8 pixel x shifted by half a pixel by the Canvas projection = pixel x edge
    /// here), consecutive batches with the same material and type.
    pub fn set_hud(&mut self, prims: &[openxiii_vm::canvas::HudDraw]) {
        use openxiii_vm::canvas::HudDraw;
        let (w, h) = (self.width as f32, self.height as f32);
        let clip = |p: [f32; 2]| [p[0] * 2.0 / w - 1.0, 1.0 - p[1] * 2.0 / h, 0.0];
        let mut vertices: Vec<SceneVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut batches_list: Vec<CanvasBatch> = Vec::new();
        let mut segment = 0;
        self.portals.clear();
        for (hud_index, d) in prims.iter().enumerate() {
            let (material, color, lines, point, enlarged, glyph) = match d {
                HudDraw::Tile { material, color, point, enlarged, glyph, .. } => (material, color, false, *point, *enlarged, *glyph),
                HudDraw::Triangle { material, color, .. } => (material, color, false, false, false, false),
                HudDraw::Line { material, color, .. } => (material, color, true, false, false, false),
                HudDraw::Portal { rect, location, rotation, fov, filter, .. } => {
                    let deg = |r: i32| {
                        let r = r.rem_euclid(65536);
                        (if r > 32768 { r - 65536 } else { r }) as f32 * 360.0 / 65536.0
                    };
                    let camera = Camera { position: *location, yaw: deg(rotation[1]), pitch: deg(rotation[0]), roll: deg(rotation[2]) };
                    self.portals.push(Portal { rect: *rect, camera, fov: *fov, filter: filter.map(|c| [c[2], c[1], c[0], c[3]]), hud_index });
                    segment += 1;
                    continue;
                }
            };
            let color = [color[2], color[1], color[0], color[3]];
            let vertex = |p: [f32; 2], uv: [f32; 2]| SceneVertex { position: clip(p), normal: [0.0; 3], color, uv0: uv, uv1: [0.0; 2] };
            let base = vertices.len() as u32;
            let first = indices.len() as u32;
            match d {
                HudDraw::Tile { x1, y1, x2, y2, uv, .. } => {
                    vertices.push(vertex([*x1, *y1], [uv[0], uv[1]]));
                    vertices.push(vertex([*x2, *y1], [uv[2], uv[1]]));
                    vertices.push(vertex([*x2, *y2], [uv[2], uv[3]]));
                    vertices.push(vertex([*x1, *y2], [uv[0], uv[3]]));
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
                HudDraw::Triangle { p, uv, .. } => {
                    vertices.push(vertex(p[0], [uv[0], uv[1]]));
                    vertices.push(vertex(p[1], [uv[2], uv[1]]));
                    vertices.push(vertex(p[2], [uv[2], uv[3]]));
                    indices.extend([base, base + 1, base + 2]);
                }
                HudDraw::Line { from, to, .. } => {
                    vertices.push(vertex(*from, [0.0; 2]));
                    vertices.push(vertex(*to, [0.0; 2]));
                    indices.extend([base, base + 1]);
                }
                HudDraw::Portal { .. } => {}
            }
            let count = indices.len() as u32 - first;
            match batches_list.last_mut() {
                Some(l) if l.material == *material && l.lines == lines && l.segment == segment && l.point == point && l.enlarged == enlarged && l.glyph == glyph => l.count += count,
                _ => batches_list.push(CanvasBatch { material: material.clone(), lines, first, count, segment, point, enlarged, glyph }),
            }
        }
        // Buffers of the previous frame reused when large enough (a new buffer every
        // frame cost its creation, upload and destruction).
        let prior = self.canvas.take().map(|(g, _)| g).or_else(|| self.canvas_free.take());
        if batches_list.is_empty() {
            self.canvas_free = prior;
        } else {
            self.canvas = Some((geometry_kept(&self.device, &self.queue, prior, &vertices, &indices, "canvas"), batches_list));
        }
    }

    /// Passes of the Canvas primitives (FCanvasUtil::Flush: SetMaterial without lighting, vertex
    /// colour, double-sided), after those of the scene.
    fn prepare_canvas(&mut self, library: &mut Library, camera: &Camera) {
        // Improved build: the menus' onomatopoeias ready at the menu's scale before they draw.
        prepare_canvas_textures();
        let Some((_, batches_list)) = &self.canvas else { return };
        let view = d3d_view(camera);
        for lot in batches_list.clone() {
            let mat = (!lot.material.is_empty()).then_some(&*lot.material);
            // FCanvasUtil::Flush: EnableLighting(0, 1) (D3DRS_COLORVERTEX).
            let lighting = Lighting { lighting: false, color_vertex: true, lightmap: false, lighting_only: false };
            let result = self.materials.set_material(library, mat, lighting, self.frame, view);
            if let Some(e) = &result.error {
                if self.reported.insert(lot.material.to_string()) {
                    eprintln!("Canvas material {}: {e}", lot.material);
                }
            }
            let bits = DrawBits { color_vertex: true, has_color: true, screen: true, ..Default::default() };
            for pass in result.passes.iter() {
                let mut pass = pass.clone();
                // No fog on the Canvas.
                pass.ignore_fog = true;
                // Improved build: the menus' onomatopoeias read under a name holding the size
                // they are shown at (onomatopoeias.rs).
                resolve_canvas_textures(&mut pass);
                // Improved build, enhanced graphics: enlarged text (a font letter, point-sampled or
                // not, drawn larger than its size) is read with the bicubic filter (gui::reads_bicubic).
                let text_bicubic = sample_enlarged_text(lot.point, lot.glyph, lot.enlarged, self.session_graphics);
                // FD3DRenderInterface +0xAC: stage 0's MAG/MINFILTER in POINT.
                if lot.point && !text_bicubic {
                    pass.stages[0].point_filter = true;
                }
                let mut key = PipelineKey::new(&pass, 2);
                key.lines = lot.lines;
                let pipeline = self.pipeline(key);
                let bind = self.stage_bind_with(library, &pass, &None, None, true);
                #[allow(unused_mut)]
                let mut uniform = PassUniform::new(&pass, bits);
                // Improved build: a picture made from one large master (menus' characters,
                // onomatopoeias) read with the bicubic filter at any screen size, and enlarged text.
                set_canvas_filter(&mut uniform, &pass, lot.point, text_bicubic);
                let pass_offset = self.append_uniform(uniform);
                self.commands.push(DrawCmd {
                    pipeline,
                    pass_offset,
                    bind,
                    which: Which::Canvas,
                    first_index: lot.first,
                    index_count: lot.count,
                    sorted: false,
                    overlay: 0,
                    vignette: false,
                    canvas: true,
                    segment: lot.segment,
                    order: i64::MAX,
                    opaque_first: false,
                    light_sky: None,
                    outside_sky: false,
                });
            }
        }
        self.upload_passes();
    }

    /// Video player screen instead of the game (`None`: the game). Video_LockTex /
    /// BinkCopyToBuffer: the RGB565 image goes into the top-left corner of a
    /// 1024 × 512 texture (Video_Init, D3DFMT_R5G6B5).
    pub fn set_video(&mut self, present: Option<&openxiii_vm::video::VideoPresent>) {
        use openxiii_vm::video::VideoPresent;
        self.video = match present {
            None => None,
            Some(VideoPresent::Black) => Some(VideoShow::Black),
            Some(VideoPresent::Frame { width, height, rgb565 }) => {
                let (w, h) = ((*width).min(1024), (*height).min(512));
                if self.video_texture.is_none() {
                    let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("video"),
                        size: wgpu::Extent3d { width: 1024, height: 512, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    // Whole texture written once (black): each frame then fills only its top-left
                    // corner, and nothing is left for wgpu to clear lazily. That lazy clear copies
                    // from a GPU buffer, which the Direct3D 11 backend does not support (panic at
                    // the first video, New game's cine00).
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &VIDEO_BLACK,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * 1024), rows_per_image: Some(512) },
                        wgpu::Extent3d { width: 1024, height: 512, depth_or_array_layers: 1 },
                    );
                    let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("video"),
                        address_mode_u: wgpu::AddressMode::ClampToEdge,
                        address_mode_v: wgpu::AddressMode::ClampToEdge,
                        mag_filter: wgpu::FilterMode::Linear,
                        min_filter: wgpu::FilterMode::Linear,
                        ..Default::default()
                    });
                    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("video"),
                        layout: &self.hud.pipeline.get_bind_group_layout(1),
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                        ],
                    });
                    self.video_texture = Some((texture, group));
                }
                // R5G6B5 read by the GPU: each component brought back to 8 bits.
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h as usize {
                    let row = &rgb565[y * *width as usize * 2..];
                    for x in 0..w as usize {
                        let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]);
                        let (r, g, b) = ((v >> 11) & 31, (v >> 5) & 63, v & 31);
                        rgba.extend([((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8, 255]);
                    }
                }
                if let Some((texture, _)) = &self.video_texture {
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &rgba,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                }
                Some(VideoShow::Frame([w as f32 / 1024.0, h as f32 / 512.0]))
            }
        };
    }

    /// Video_CopyTexToScreen: a quad from (−1, −1) to (1, 1), hence the
    /// image stretched over the whole screen; at the end, the screen cleared to black. The game's
    /// videos are 640 × 480: on a 4:3 screen that quad is kept as is; on any other shape the
    /// video keeps its proportions, as large as the screen allows, with black bars on two sides
    /// (`video_rect`).
    fn video_pass(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, show: VideoShow) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let (VideoShow::Frame([u, v]), Some((_, group))) = (show, &self.video_texture) else { return };
        let (w, h) = (self.width as f32, self.height as f32);
        let [x0, y0, x1, y1] = video_rect(self.width, self.height, (u * 1024.0).round() as u32, (v * 512.0).round() as u32);
        let corner = |px: f32, py: f32, tu: f32, tv: f32| HudVertex { position: [px, py], uv: [tu, tv], color: [1.0; 4] };
        let quad = [corner(x0, y0, 0.0, 0.0), corner(x1, y0, u, 0.0), corner(x0, y1, 0.0, v), corner(x1, y0, u, 0.0), corner(x1, y1, u, v), corner(x0, y1, 0.0, v)];
        let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("video"),
            contents: bytemuck::cast_slice(&quad),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("video"),
            contents: bytemuck::cast_slice(&[w, h, 0.0f32, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uniform_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video"),
            layout: &self.hud.pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        pass.set_pipeline(&self.hud.pipeline);
        pass.set_bind_group(0, &uniform_group, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..6, 0..1);
    }

    pub fn render_to_surface(&mut self, camera: &Camera, library: &mut Library) -> Result<(), String> {
        // Warm-up behind the loading picture (the surface is with `hold_start`'s thread): the
        // frame is drawn into a texture of the window's size and not shown, so that textures,
        // pipelines and render targets of the new level exist before its first visible frame.
        // Faithful build, first frame after a travel: the original does not present its precache
        // frame (no Present between Tick 0 and the first normal frame after LoadMap): drawn off
        // screen, the loading screen stays shown.
        if std::mem::take(&mut self.hide_next_frame) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("warm-up frame"),
                size: wgpu::Extent3d { width: self.width.max(1), height: self.height.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.draw(&view, camera, library);
            // The original's uploads finish inside its precache frame (Lock/Unlock copies): the
            // graphics card ends this frame's work before the first frame shown.
            self.queue.submit([]);
            self.wait_idle_sharing(std::time::Duration::from_secs(2));
            return Ok(());
        }
        if self.surface.is_none() && self.warming {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("warm-up frame"),
                size: wgpu::Extent3d { width: self.width.max(1), height: self.height.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.draw(&view, camera, library);
            self.capture_hold(&texture);
            return Ok(());
        }
        let Some((surface, _)) = &self.surface else {
            return Err("no window".into());
        };
        let t0 = std::time::Instant::now();
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            other => {
                // Outdated or lost surface: it is reconfigured and this frame is skipped.
                if let Some((surface, config)) = &self.surface {
                    surface.configure(&self.device, config);
                }
                return Err(format!("surface unavailable: {other:?}"));
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let t1 = std::time::Instant::now();
        if empty_window() {
            // Profiling: surface acquired, cleared and presented, nothing else.
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("empty window") });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("empty window"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            submit_value(&self.queue, [encoder.finish()]);
            let t2 = std::time::Instant::now();
            self.queue.present(frame);
            let p = &mut self.perf_surface;
            p[0] += (t1 - t0).as_secs_f64();
            p[1] += (t2 - t1).as_secs_f64();
            p[2] += t2.elapsed().as_secs_f64();
            p[3] += 1.0;
            return Ok(());
        }
        self.draw(&view, camera, library);
        let t2 = std::time::Instant::now();
        self.apply_gamma(&frame.texture);
        self.capture_hold(&frame.texture);
        self.queue.present(frame);
        let p = &mut self.perf_surface;
        p[0] += (t1 - t0).as_secs_f64();
        p[1] += (t2 - t1).as_secs_f64();
        p[2] += t2.elapsed().as_secs_f64();
        p[3] += 1.0;
        Ok(())
    }

    /// Split screen: window frame where each viewport is drawn by `draw_view`, then
    /// presented by `present`. `None`: no frame at this time.
    /// While `warming` with the surface given to `hold_start`'s thread, the frame is a texture of
    /// the window's size that is never shown (no SurfaceTexture): the views drawn into it make the
    /// level's textures, pipelines and targets behind the loading picture.
    pub fn begin_frame(&mut self) -> Result<Option<(Option<wgpu::SurfaceTexture>, wgpu::TextureView)>, String> {
        if self.surface.is_none() && self.warming {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("warm-up frame"),
                size: wgpu::Extent3d { width: self.width.max(1), height: self.height.max(1), depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.color_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.clear_target(&view);
            return Ok(Some((None, view)));
        }
        let Some((surface, _)) = &self.surface else {
            return Err("no window".into());
        };
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return Ok(None),
            other => {
                if let Some((surface, config)) = &self.surface {
                    surface.configure(&self.device, config);
                }
                return Err(format!("surface unavailable: {other:?}"));
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Frame cleared once (UWindowsClient::Tick before the viewports), animated lightmaps.
        self.clear_target(&view);
        Ok(Some((Some(frame), view)))
    }

    /// Presents the frame from `begin_frame` (a warm-up frame is only submitted).
    pub fn present(&mut self, frame: Option<wgpu::SurfaceTexture>) {
        let Some(frame) = frame else {
            self.queue.submit([]);
            return;
        };
        self.apply_gamma(&frame.texture);
        self.capture_hold(&frame.texture);
        self.queue.present(frame);
    }

    /// The presented frame kept for `hold_start` (improved build, while `hold_capture`).
    fn capture_hold(&mut self, frame: &wgpu::Texture) {
        if !self.hold_capture || !frame.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return;
        }
        let size = frame.size();
        let held = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("held frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: frame.format(),
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("held frame") });
        encoder.copy_texture_to_texture(frame.as_image_copy(), held.as_image_copy(), size);
        submit_value(&self.queue, [encoder.finish()]);
        let list = if self.hold_capture_alt { &self.hold_alt_frames } else { &self.hold_frames };
        if let Ok(mut frames) = list.lock() {
            frames.push(held);
            let extra = frames.len().saturating_sub(HOLD_FRAMES);
            frames.drain(..extra);
        }
    }

    /// The kept frames forgotten (a new picture or animation starts being kept).
    pub fn hold_clear(&mut self) {
        if let Ok(mut frames) = self.hold_frames.lock() {
            frames.clear();
        }
        if let Ok(mut frames) = self.hold_alt_frames.lock() {
            frames.clear();
        }
        self.hold_switch.store(false, std::sync::atomic::Ordering::Release);
        if let Ok(mut at) = self.hold_switched_at.lock() {
            *at = None;
        }
    }

    /// Improved build: during a blocking load, the kept frames are shown in turn (each for
    /// `step_ms`), presented about every 16 ms from another thread (the surface goes with it
    /// until `hold_stop`): the loading animation goes on, and a screen with a variable refresh
    /// rate (G-SYNC, FreeSync) never waits on one frame for the whole load (on many laptop
    /// panels that left a trace of the frame before it). Frames kept meanwhile (off screen,
    /// `warming`) join the turn. None when nothing is kept or the surface cannot be copied to.
    pub fn hold_start(&mut self, step_ms: u64) -> Option<PresentHold> {
        if self.hold_frames.lock().map_or(true, |f| f.is_empty()) {
            return None;
        }
        if !self.surface.as_ref().is_some_and(|(_, c)| c.usage.contains(wgpu::TextureUsages::COPY_DST)) {
            return None;
        }
        let (surface, config) = self.surface.take()?;
        let (device, queue, frames) = (self.device.clone(), self.queue.clone(), self.hold_frames.clone());
        let (alt, switch, switched_at) = (self.hold_alt_frames.clone(), self.hold_switch.clone(), self.hold_switched_at.clone());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = std::thread::Builder::new().name("present hold".into()).spawn(move || {
            let mut longest = std::time::Duration::ZERO;
            let started = std::time::Instant::now();
            // When the longest gap ended (development builds print it).
            let mut longest_end = started;
            let mut last = started;
            let mut presents = 0u32;
            // When the other frames' turn began (`hold_switch`).
            let mut alt_started: Option<std::time::Instant> = None;
            while !stop_thread.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(5));
                if stop_thread.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                let alt_len = alt.lock().map_or(0, |l| l.len());
                let use_alt = switch.load(std::sync::atomic::Ordering::Acquire) && alt_len > 0;
                // A frame about every 16 ms, and at once when the turn switches.
                let switch_now = use_alt && alt_started.is_none();
                if !switch_now && last.elapsed() < std::time::Duration::from_millis(16) {
                    continue;
                }
                if switch_now {
                    alt_started = Some(std::time::Instant::now());
                }
                let held = {
                    let Ok(list) = (if use_alt { alt.lock() } else { frames.lock() }) else { break };
                    let since = if use_alt { alt_started.unwrap_or(started) } else { started };
                    let Some(i) = held_index(list.len(), since.elapsed().as_millis() as u64, step_ms) else { continue };
                    list[i].clone()
                };
                let frame = match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    _ => continue,
                };
                if frame.texture.size() != held.size() || frame.texture.format() != held.format() {
                    continue;
                }
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("held frame again") });
                encoder.copy_texture_to_texture(held.as_image_copy(), frame.texture.as_image_copy(), held.size());
                queue.submit([encoder.finish()]);
                queue.present(frame);
                if use_alt {
                    if let Ok(mut at) = switched_at.lock() {
                        at.get_or_insert_with(std::time::Instant::now);
                    }
                }
                presents += 1;
                if last.elapsed() > longest {
                    longest = last.elapsed();
                    longest_end = std::time::Instant::now();
                }
                last = std::time::Instant::now();
            }
            (surface, config, presents, longest, longest_end)
        });
        match thread {
            Ok(thread) => Some(PresentHold { stop, thread }),
            Err(_) => None,
        }
    }

    /// The end of `hold_start`: the surface back, for the next frames.
    pub fn hold_stop(&mut self, hold: PresentHold) {
        hold.stop.store(true, std::sync::atomic::Ordering::Release);
        match hold.thread.join() {
            Ok((surface, config, presents, longest, longest_end)) => {
                self.surface = Some((surface, config));
                diag(&format!("load: the loading picture shown again {presents} times, longest gap {} ms", longest.as_millis()));
                let _ = longest_end;
            }
            Err(_) => diag("load: the thread showing the loading picture stopped; no surface"),
        }
    }

    /// Full-screen gamma ramp (UD3DRenderDevice::UpdateGamma) on the finished
    /// frame, unless it is the identity (XIII.ini values) or the surface cannot
    /// be copied.
    fn apply_gamma(&mut self, frame: &wgpu::Texture) {
        let Some(color) = self.gamma else { return };
        let ramp = crate::gamma::ramp(color);
        if crate::gamma::is_identity(&ramp) || !frame.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return;
        }
        let format = frame.format();
        let g = self.gamma_renderer.get_or_insert_with(|| crate::gamma::GammaRenderer::new(&self.device, format));
        g.apply(&self.device, &self.queue, frame, &ramp);
    }

    /// UGameEngine::Draw of a split-screen viewport: the scene of `camera` in
    /// rectangle `rect` (x, y, width, height in window pixels, like D3DDrv's SetViewport
    /// at Lock), projection at the rectangle's aspect, then the viewport's Canvas (its
    /// primitives are already in window pixels).
    pub fn draw_view(&mut self, target: &wgpu::TextureView, camera: &Camera, rect: [f32; 4], library: &mut Library) {
        self.draw_view_in(target, camera, rect, None, library);
    }

    /// `draw_view` with the scene drawn in `scene` only (window pixels; improved build: the
    /// co-op character screen's live-view panel), its aspect that of `scene`; the Canvas keeps
    /// the viewport's whole rectangle.
    pub fn draw_view_in(&mut self, target: &wgpu::TextureView, camera: &Camera, rect: [f32; 4], scene: Option<[f32; 4]>, library: &mut Library) {
        // Like Renderer::draw: scenery textures and the map's prewarm list first
        // (split screen, campaigns; the lists are learned there too).
        if !self.scenery_warmup && !self.world.batches.is_empty() && self.video.is_none() {
            self.warm_up_scenery(library, camera);
        }
        self.in_panel = scene.is_some();
        let (w, h) = (self.width as f32, self.height as f32);
        let x = rect[0].clamp(0.0, (w - 1.0).max(0.0));
        let y = rect[1].clamp(0.0, (h - 1.0).max(0.0));
        let rect = [x, y, rect[2].min(w - x).max(1.0), rect[3].min(h - y).max(1.0)];
        let full = rect;
        let rect = match scene {
            Some(r) => {
                let (sx, sy) = (r[0].clamp(0.0, (w - 1.0).max(0.0)), r[1].clamp(0.0, (h - 1.0).max(0.0)));
                [sx, sy, r[2].min(w - sx).max(1.0), r[3].min(h - sy).max(1.0)]
            }
            None => rect,
        };
        let aspect = rect[2] / rect[3];
        self.update_zone(camera, true);
        self.update_uniforms_with(camera, self.view_fov_y(aspect), aspect, self.view_widen(), self.overlay_camera.as_ref());
        self.prepare(library, camera);
        self.prepare_canvas(library, camera);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport") });
        self.render_shadows(library, &mut encoder);
        if self.only_post_render {
            // Viewport+0x120: no scene, PostRender only (on the rectangle cleared at the start).
        } else if !self.fx_passes.is_empty() {
            // / Apply2DEffect (FD3DRenderInterface, shared by the viewports): the
            // viewport's scene into the 512 × 512 texture, the effect drawn over the whole viewport (here an
            // image of its size copied into its rectangle).
            let (rw, rh) = (rect[2] as u32, rect[3] as u32);
            let mut fx = self.fx.take().unwrap_or_else(|| crate::screen_fx::ScreenFxRenderer::new(&self.device, self.color_format, self.fx_side));
            let side = fx.side as f32;
            self.level_passes(&mut encoder, &fx.view, &fx.depth, true, None, [side; 2], library);
            self.camera_tiles(&mut encoder, &fx.view, None, [side; 2], library);
            self.ensure_fx_view_target(rw, rh);
            if let Some((_, view, _, _)) = &self.fx_view_target {
                fx.draw(&self.device, &self.queue, &mut encoder, view, &self.fx_passes, rw, rh);
            }
            self.fx = Some(fx);
            self.blit_fx_view(&mut encoder, target, rect);
        } else if let Some((color, depth, (tw, th))) = self.world_target_views() {
            // Render scale or MSAA: the viewport's rectangle of the world target, cleared to the
            // frame's black (or DistanceFogColor), drawn, then copied into the frame.
            let (sx, sy) = (tw as f32 / w, th as f32 / h);
            let wrect = [rect[0] * sx, rect[1] * sy, (rect[2] * sx).max(1.0), (rect[3] * sy).max(1.0)];
            let size = [tw as f32, th as f32];
            self.pass_samples.set(self.samples);
            let c = if self.force_clear { self.clear_color.map(|v| v as f32 / 255.0) } else { [0.0; 3] };
            self.filter_tile(&mut encoder, &color, wrect, [c[0], c[1], c[2], 1.0], size, false, None, library);
            self.level_passes(&mut encoder, &color, &depth, false, Some(wrect), size, library);
            self.camera_tiles(&mut encoder, &color, Some(wrect), size, library);
            self.pass_samples.set(1);
            self.copy_world(&mut encoder, target, wrect, rect);
        } else {
            // UGameEngine::Draw: camera's zone with bForceClear, the viewport cleared to
            // DistanceFogColor (otherwise UWindowsClient::Tick's black).
            if self.force_clear {
                let c = self.clear_color.map(|v| v as f32 / 255.0);
                self.filter_tile(&mut encoder, target, rect, [c[0], c[1], c[2], 1.0], [w, h], false, None, library);
            }
            let depth = self.depth.clone();
            self.level_passes(&mut encoder, target, &depth, false, Some(rect), [w, h], library);
            self.camera_tiles(&mut encoder, target, Some(rect), [w, h], library);
        }
        let rect = full;
        self.canvas_pass(&mut encoder, target, 0, Some(rect));
        submit_value(&self.queue, [encoder.finish()]);
        // UCanvas::execDrawPortal of the viewport: FCameraSceneNode of the portal in its rectangle
        // (already in window pixels), limited to the viewport, then the rest of the Canvas.
        let saved_fov = self.fov;
        // The portal has its own visibility, not tracked here: all the scenery.
        let saved_cull = self.world_masked.take();
        let saved_light_passes = self.scene_light_passes.clone();
        for (k, portal) in self.portals.clone().into_iter().enumerate() {
            self.scene_light_passes = self.scene_light_views.portals.get(&portal.hud_index).cloned().unwrap_or_default();
            let x0 = portal.rect[0].max(rect[0]);
            let y0 = portal.rect[1].max(rect[1]);
            let x1 = (portal.rect[0] + portal.rect[2]).min(rect[0] + rect[2]);
            let y1 = (portal.rect[1] + portal.rect[3]).min(rect[1] + rect[3]);
            let r = [x0, y0, x1 - x0, y1 - y0];
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("portal") });
            if r[2] >= 1.0 && r[3] >= 1.0 {
                let aspect = portal.rect[2].max(1.0) / portal.rect[3].max(1.0);
                self.fov = Some(portal.fov);
                self.in_camera_view = true;
                self.update_zone(&portal.camera, true);
                self.update_uniforms_with(&portal.camera, self.fov_y(aspect), aspect, None, None);
                self.prepare(library, &portal.camera);
                self.prepare_canvas(library, &portal.camera);
                let depth = self.depth.clone();
                self.level_passes(&mut encoder, target, &depth, false, Some(r), [w, h], library);
                let main_filter = std::mem::replace(&mut self.color_filter, portal.filter);
                self.camera_tiles(&mut encoder, target, Some(r), [w, h], library);
                self.color_filter = main_filter;
            }
            self.canvas_pass(&mut encoder, target, k + 1, Some(rect));
            submit_value(&self.queue, [encoder.finish()]);
        }
        self.scene_light_passes = saved_light_passes;
        self.world_masked = saved_cull;
        self.in_camera_view = false;
        self.fov = saved_fov;
    }

    /// Apply2DEffect image of a split-screen viewport, at its size (recreated if it changes).
    fn ensure_fx_view_target(&mut self, w: u32, h: u32) {
        if self.fx_view_target.as_ref().is_some_and(|t| t.3 == (w, h)) {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewport effect"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport effect"),
            layout: &self.hud.pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.fx_view_target = Some((texture, view, group, (w, h)));
    }

    /// Copies the viewport's Apply2DEffect image into its window rectangle, then clears the
    /// depth (end of Apply2DEffect: Clear(0, 0, D3DCLEAR_ZBUFFER | D3DCLEAR_STENCIL)).
    fn blit_fx_view(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, rect: [f32; 4]) {
        let Some((_, _, group, _)) = &self.fx_view_target else { return };
        self.fx_view_blit_quad(rect);
        let kept = self.fx_view_blit.borrow();
        let (_, buffer, uniform_group) = kept.as_ref().unwrap();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport effect"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud.pipeline);
            pass.set_bind_group(0, uniform_group, &[]);
            pass.set_bind_group(1, group, &[]);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..6, 0..1);
        }
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effects: depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    /// The quad and uniform copying the viewport's Apply2DEffect image into `rect`, kept while the
    /// rectangle and the window size stay the same (every frame of a game).
    fn fx_view_blit_quad(&self, rect: [f32; 4]) {
        use wgpu::util::DeviceExt;
        let (w, h) = (self.width as f32, self.height as f32);
        let [x, y, rw, rh] = rect;
        let key = [x, y, rw, rh, w, h].map(f32::to_bits);
        let mut kept = self.fx_view_blit.borrow_mut();
        if kept.as_ref().is_none_or(|k| k.0 != key) {
            let corner = |px: f32, py: f32, tu: f32, tv: f32| HudVertex { position: [px, py], uv: [tu, tv], color: [1.0; 4] };
            let quad = [corner(x, y, 0.0, 0.0), corner(x + rw, y, 1.0, 0.0), corner(x, y + rh, 0.0, 1.0), corner(x + rw, y, 1.0, 0.0), corner(x + rw, y + rh, 1.0, 1.0), corner(x, y + rh, 0.0, 1.0)];
            let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewport effect"), contents: bytemuck::cast_slice(&quad), usage: wgpu::BufferUsages::VERTEX });
            let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("viewport effect"),
                contents: bytemuck::cast_slice(&[w, h, 0.0f32, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let uniform_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("viewport effect"),
                layout: &self.hud.pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
            });
            *kept = Some((key, buffer, uniform_group));
        }
    }

    /// Renders an off-screen frame and saves it as PNG.
    pub async fn screenshot(&mut self, camera: &Camera, path: &Path, library: &mut Library) -> Result<(), String> {
        let target = self.offscreen_target();
        self.draw(&target.create_view(&wgpu::TextureViewDescriptor::default()), camera, library);
        self.save_png(&target, path).await
    }

    /// A frame drawn off-screen without being read back (continuous rendering for screenshots).
    pub fn render_offscreen(&mut self, camera: &Camera, library: &mut Library) {
        let target = self.offscreen_target();
        self.draw(&target.create_view(&wgpu::TextureViewDescriptor::default()), camera, library);
    }

    /// Off-screen texture the size of the frame (screenshots).
    pub fn offscreen_target(&self) -> wgpu::Texture {
        let size = wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 };
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Off-screen split screen: the texture cleared once, before the `draw_view` calls.
    pub fn clear_target(&mut self, view: &wgpu::TextureView) {
        self.lightmaps.time += self.frame.delta as f64;
        self.lightmaps.update(&self.queue);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("split screen") });
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        submit_value(&self.queue, [encoder.finish()]);
    }

    pub async fn save_png(&mut self, target: &wgpu::Texture, path: &Path) -> Result<(), String> {
        let size = wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 };
        let row = (4 * self.width).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * self.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(self.height) },
            },
            size,
        );
        submit_value(&self.queue, [encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| e.to_string())?;
        let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
        let mut rgba = Vec::with_capacity((4 * self.width * self.height) as usize);
        for y in 0..self.height {
            let start = (y * row) as usize;
            rgba.extend_from_slice(&data[start..start + (4 * self.width) as usize]);
        }
        drop(data);
        // The screen ignores the buffer's alpha (opaque surface): so does the screenshot. An open
        // window often gives a BGRA surface: red and blue put back in PNG order.
        let bgra = matches!(self.color_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        for px in rgba.as_chunks_mut::<4>().0 {
            if bgra {
                px.swap(0, 2);
            }
            px[3] = 255;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
        let mut encoder = png::Encoder::new(file, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[path = "dynamic_lights.rs"]
mod dynamic_lights;

/// Graphics adapter and device.
///
/// With the `dx11` feature (Windows), Direct3D 11 comes first: it takes the "Dx12" slot of
/// wgpu (vendor/wgpu-hal/src/dx11). If no adapter allows it (feature level 11_0 and D3D 11.1
/// required), if opening fails, or if an earlier windowed attempt's process stopped before the end
/// of initialisation (see `test_d3d11`, d3d11_marker.rs), the usual choice resumes without Direct3D 11 (OpenGL,
/// Vulkan). WGPU_BACKEND forces the API as before; OXIII_RENDERER=gl excludes Direct3D 11,
/// OXIII_RENDERER=dx11 retries it after a recorded failure. WGPU_ADAPTER_NAME picks the adapter by name.
async fn open_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
    window_value: bool,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), String> {
    let all_set = instance.enumerate_adapters(wgpu::Backends::all()).await;
    // Adapters seen, for slowness diagnostics (software rendering, wrong adapter).
    for a in &all_set {
        diag(&format!("graphics adapter seen: {}", describe_adapter(&a.get_info())));
    }
    let shows = |a: &wgpu::Adapter| surface.is_none_or(|s| a.is_surface_supported(s));
    let open_flag = |a: wgpu::Adapter| async move {
        diag(&format!("graphics adapter chosen: {}", describe_adapter(&a.get_info())));
        diag(&format!("build: {BUILD_CHEATS}, id {}", crate::lockstep_play::BUILD_ID));
        // Android: the device's limits (some OpenGL ES drivers are below those of
        // WebGPU, e.g. 16 KB uniform buffers).
        // Direct3D 11 too: its limits (128 MB buffers guaranteed) are below the defaults.
        let mut descriptor = if cfg!(target_os = "android") || a.get_info().backend == wgpu::Backend::Dx12 && cfg!(feature = "dx11") {
            wgpu::DeviceDescriptor { required_limits: a.limits(), ..Default::default() }
        } else {
            wgpu::DeviceDescriptor::default()
        };
        // Sample counts beyond 1 and 4 (MSAA x2, x8) as the adapter supports them.
        descriptor.required_features |= a.features() & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        // The driver's compiled pipelines kept between runs, where a file is kept for them.
        descriptor.required_features |= crate::pipeline_cache::wanted_features(&a);
        let r = a.request_device(&descriptor).await;
        if let Ok((d, _)) = &r {
            crate::pipeline_cache::open(d, &a.get_info());
        }
        // Only Direct3D 11/12 and Vulkan create pipelines from several threads (OpenGL: main thread context).
        CREATION_PARALLEL.store(a.get_info().backend != wgpu::Backend::Gl, std::sync::atomic::Ordering::Relaxed);
        r.map(|(d, q)| (a, d, q))
    };
    // Adapters of an API, the dedicated one first.
    let maps_list = |keep_value: &dyn Fn(wgpu::Backend) -> bool| {
        let mut v: Vec<&wgpu::Adapter> = all_set.iter().filter(|a| keep_value(a.get_info().backend) && shows(a)).collect();
        v.sort_by_key(|a| a.get_info().device_type != wgpu::DeviceType::DiscreteGpu);
        v.first().map(|a| (*a).clone())
    };

    if let Some(wanted_value) = openxiii_package::env::var("WGPU_ADAPTER_NAME").ok().map(|n| n.to_lowercase()) {
        if let Some(a) = all_set.iter().find(|a| a.get_info().name.to_lowercase().contains(wanted_value.as_str()) && shows(a)) {
            return open_flag(a.clone()).await.map_err(|e| e.to_string());
        }
    }

    let d3d11 = cfg!(all(windows, feature = "dx11")) && openxiii_package::env::var_os("WGPU_BACKEND").is_none();
    let render = openxiii_package::env::var("OXIII_RENDERER").unwrap_or_default();
    let mut without_d3d11 = render == "gl";
    if d3d11 && !without_d3d11 {
        if window_value && render != "dx11" && test_d3d11_skip() {
            diag("Direct3D 11 skipped: the last attempt stopped during initialization (OXIII_RENDERER=dx11 to try again)");
        } else if let Some(a) = maps_list(&|b| b == wgpu::Backend::Dx12) {
            if window_value {
                test_d3d11(&a, true, false);
            }
            match open_flag(a.clone()).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    diag(&format!("Direct3D 11 refused when opening ({e}): fallback"));
                    test_d3d11(&a, false, false);
                }
            }
        } else {
            diag("Direct3D 11: no compatible adapter (feature level 11_0 and D3D 11.1 required): fallback");
        }
        without_d3d11 = true;
    }

    // Fallback: wgpu's usual choice, without the Dx12 slot (which is Direct3D 11 in this build).
    if d3d11 && without_d3d11 {
        let a = maps_list(&|b| b != wgpu::Backend::Dx12).ok_or("no graphics adapter outside Direct3D 11")?;
        return open_flag(a).await.map_err(|e| e.to_string());
    }
    let a = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| e.to_string())?;
    open_flag(a).await.map_err(|e| e.to_string())
}

/// Direct3D 11 is skipped in a window: an earlier attempt's process ended before its window's
/// start was finished (d3d11_marker.rs: attempt files carrying their process id, those of running
/// games ignored).
fn test_d3d11_skip() -> bool {
    cfg!(all(windows, feature = "dx11")) && crate::d3d11_marker::folder().is_some_and(|d| crate::d3d11_marker::should_skip_in(&d, &crate::d3d11_marker::process_alive))
}

/// Start (`in_progress`) or end of this process' Direct3D 11 attempt on adapter `a`; at the end,
/// `finished`: the window's start is finished (the ended processes' attempt files go too),
/// otherwise Direct3D 11 was refused.
fn test_d3d11(a: &wgpu::Adapter, in_progress: bool, finished: bool) {
    if !cfg!(all(windows, feature = "dx11")) || a.get_info().backend != wgpu::Backend::Dx12 {
        return;
    }
    let Some(d) = crate::d3d11_marker::folder() else { return };
    if in_progress {
        crate::d3d11_marker::begin_in(&d, &a.get_info().name);
    } else {
        crate::d3d11_marker::end_in(&d, finished, &crate::d3d11_marker::process_alive);
    }
}

pub static TIMER_TARGETS: std::sync::Mutex<[f64; 7]> = std::sync::Mutex::new([0.0; 7]);

/// Preparation of the main view (measure-send): batches, actors, projectors, particles,
/// weather, characters, assembly, uniform upload (accumulated ms), then draws.
/// Pipeline key of the combiner read from the uniforms, as text (prewarm list).
fn key_pipeline_text(k: &PipelineKey) -> String {
    let blend = k.blend.map_or("-".to_string(), |(a, b)| format!("{a},{b}"));
    format!("pipeline:{}:{}:{blend}:{}:{}:{}:{}:{}", k.impact as u8, k.stage_count, k.z_test as u8, k.z_write as u8, k.cull, k.lines as u8, k.z_bias)
}

fn key_pipeline_read(t: &str) -> Option<PipelineKey> {
    let c: Vec<&str> = t.strip_prefix("pipeline:")?.split(':').collect();
    if c.len() != 8 {
        return None;
    }
    let b = |s: &str| s.parse::<u8>().ok().map(|v| v != 0);
    let blend = if c[2] == "-" { None } else { let (a, d) = c[2].split_once(',')?; Some((a.parse().ok()?, d.parse().ok()?)) };
    let mut key = PipelineKey::new(&openxiii_assets::material::Pass::default(), 0);
    key.impact = b(c[0])?;
    key.stage_count = c[1].parse().ok()?;
    key.blend = blend;
    key.z_test = b(c[3])?;
    key.z_write = b(c[4])?;
    key.cull = c[5].parse().ok()?;
    key.lines = b(c[6])?;
    key.z_bias = c[7].parse().ok()?;
    key.combine = None;
    Some(key)
}

pub static SHADOWS_RADIO: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// Shipped prewarm lists ("[map]" sections): optional game data prewarm/lists.txt
/// (openxiii_vm::game_data).
fn lists_included() -> &'static str {
    openxiii_vm::game_data::file("prewarm/lists.txt").unwrap_or("")
}

/// Texture of a stage as its texture group sees it: a shadow target "#shadowtex:<slot>:<intensity>"
/// is the same texture whatever the intensity (read by the pass uniforms, not by the group), so
/// its group does not change as a shadow fades.
fn group_texture(p: &str) -> &str {
    match p.strip_prefix("#shadowtex:").and_then(|rest| rest.find(':')) {
        Some(i) => &p[..("#shadowtex:".len() + i)],
        None => p,
    }
}

/// "bind:" line of a prewarm list: each stage as texture (or "~"), U and V addressing and
/// sampler kind, stages separated by "|".
fn key_bind_text(key: &BindKey) -> String {
    let stages: Vec<String> = key.iter().map(|(t, u, v, m)| format!("{},{u},{v},{m}", t.as_deref().unwrap_or("~"))).collect();
    format!("bind:{}", stages.join("|"))
}

fn key_bind_read(line: &str) -> Option<BindKey> {
    let rest = line.strip_prefix("bind:")?;
    let stages: Vec<(Option<String>, u32, u32, u8)> = rest
        .split('|')
        .map(|s| {
            let mut f = s.rsplitn(4, ',');
            let (m, v, u, t) = (f.next()?, f.next()?, f.next()?, f.next()?);
            Some(((t != "~").then(|| t.to_string()), u.parse().ok()?, v.parse().ok()?, m.parse().ok()?))
        })
        .collect::<Option<_>>()?;
    stages.try_into().ok()
}

fn list_included(map_value: &str) -> Option<&'static str> {
    // Line endings \n or \r\n (git checkout on Windows): lines are split by lines().
    let header = format!("[{}]", map_value.to_ascii_lowercase());
    let mut start_value = None;
    let mut pos = 0;
    let lists = lists_included();
    for l in lists.split_inclusive('\n') {
        pos += l.len();
        if start_value.is_none() {
            if l.trim_end() == header {
                start_value = Some(pos);
            }
        } else if l.starts_with('[') {
            return Some(&lists[start_value?..pos - l.len()]);
        }
    }
    Some(&lists[start_value?..])
}

/// `dir/prewarm`.
fn prewarm_folder(dir: &std::path::Path) -> std::path::PathBuf {
    let folder = dir.join("prewarm");
    folder
}

/// Shipped prewarm lists ("prewarm" folder next to the executable).
fn folder_warmup_shipped() -> Option<std::path::PathBuf> {
    Some(prewarm_folder(std::env::current_exe().ok()?.parent()?))
}

/// Batch loop of the main view: (ms, sections) of the scenery, of the replaced meshes, of the
/// animated actors.
/// Waved sections (measure-send): [re-uploaded, unchanged and skipped].
pub static WRITTEN_SINE: std::sync::Mutex<[u64; 2]> = std::sync::Mutex::new([0; 2]);

pub static TIMER_BATCHES: std::sync::Mutex<[f64; 8]> = std::sync::Mutex::new([0.0; 8]);

/// The next 12: same items for the comic panel views.
pub static TIMER_PREP: std::sync::Mutex<[f64; 24]> = std::sync::Mutex::new([0.0; 24]);

fn timer_prep(main_value: bool, t: &mut std::time::Instant, k: usize) {
    let _ = (main_value, &t, k);
}

fn timer_target(k: usize, start_value: std::time::Instant) {
    let _ = (k, start_value);
}

/// A graphics adapter in one line: name, API, type, driver, PCI identifiers.
fn describe_adapter(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?}, driver {} {}, {:04x}:{:04x})", info.name, info.backend, info.device_type, info.driver, info.driver_info, info.vendor, info.device)
}

pub fn empty_window() -> bool {
    false
}

pub static WRITES: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];
/// measure-send: passes re-uploaded, including those already present elsewhere in the buffer (moved).
/// measure-send: BSP projectors (count, geometry rebuilt, vertices rebuilt, ns of set_bsp_projectors).
/// measure-send: scene draws, including changed uniforms, and changes per block
/// (material, view, object, lighting) from one draw to the next.
pub static BLOCKS: [std::sync::atomic::AtomicU64; 6] = [const { std::sync::atomic::AtomicU64::new(0) }; 6];

/// measure-send: first reason preventing two neighbouring draws from merging.
pub const NAMES_REASONS: [&str; 10] = ["pipeline", "textures", "geometry", "list (sort, first-person view, vignette, canvas, segment)", "scene (sky)", "non-contiguous indices", "uniforms: material", "uniforms: view", "uniforms: object", "uniforms: lighting"];
pub static REASONS_MERGE: [std::sync::atomic::AtomicU64; 10] = [const { std::sync::atomic::AtomicU64::new(0) }; 10];
/// measure-send: byte-by-byte comparisons of pass buffer entries (upload_passes).
pub static COMPARISONS_PASSES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static PROJ_REDONE: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];
pub static MOVED: [std::sync::atomic::AtomicU64; 2] = [const { std::sync::atomic::AtomicU64::new(0) }; 2];

fn count_write(k: usize, bytes_value: u64) {
    return;
}

/// Profiling: commands after merging, neighbours mergeable if their indices followed, same
/// state except uniforms, same pipeline and group.
pub static MERGES: std::sync::Mutex<[u64; 4]> = std::sync::Mutex::new([0; 4]);

/// Queue::write_buffer, counted, skipped in empty-window mode.
#[track_caller]
fn write_value(queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64, data: &[u8]) {
    count_write(0, data.len() as u64);
    if data.len() > 100_000 && crate::game::dbg_reason() {
        diag(&format!("list reason t={:.3} write of {} B (renderer.rs:{}, buffer {:?})", crate::game::dbg_time(), data.len(), std::panic::Location::caller().line(), buffer.size()));
    }
    if !empty_window() && !write_ordered(queue, buffer, offset, data) && !write_group(queue, buffer, offset, data) {
        queue.write_buffer(buffer, offset, data);
    }
}

fn groups_after_pipeline() -> bool {
    false
}

pub fn diag(line: &str) {
    eprintln!("{line}");
}

/// Tangent of the horizontal half angle the cartoon outline thickness is computed from
/// (and its 0.8 boost threshold): the game's FovAngle when there is one, whatever
/// `fov_x` (the angle the view is drawn with, widened by Hor+ on a screen wider than 4:3) is;
/// the free camera's `fov_x` otherwise. Without Hor+, `fov_x` is FovAngle itself, bit for bit.
fn outline_tan_half(fov: Option<f32>, fov_x: f32) -> f32 {
    (fov.unwrap_or(fov_x) * 0.008_726_646).tan()
}

#[cfg(test)]
mod outline_fov {
    use super::outline_tan_half;

    /// Horizontal angle Hor+ draws a `w` × `h` window with (display_format::widen_fov's formula).
    fn widened(fov: f32, w: u32, h: u32) -> f32 {
        match openxiii_vm::display_format::hor_plus_factor(w, h) {
            Some(k) => 2.0 * ((fov.to_radians() * 0.5).tan() * k).atan().to_degrees(),
            None => fov,
        }
    }

    /// The outline uses the same angle at 1920x1080, 1920x1200, 2560x1080 and 3840x1080 as at
    /// 1600x1200 (4:3) for 100 FovAngles; the widened angle (the view's) would give another one.
    #[test]
    fn outline_angle_not_widened() {
        for i in 0..100 {
            let fov = 20.0 + i as f32 * 1.2;
            let base = outline_tan_half(Some(fov), widened(fov, 1600, 1200));
            assert_eq!(base.to_bits(), (fov * 0.008_726_646).tan().to_bits());
            for (w, h) in [(1920, 1080), (1920, 1200), (2560, 1080), (3840, 1080), (7680, 4320)] {
                let fov_x = widened(fov, w, h);
                assert_eq!(outline_tan_half(Some(fov), fov_x).to_bits(), base.to_bits(), "{fov} {w}x{h}");
                // Negative control: the widened angle is not the outline's.
                assert!((fov_x * 0.008_726_646).tan() > base * 1.1, "{fov} {w}x{h}");
            }
            // Narrower than 4:3 (portrait) and the free camera: unchanged.
            assert_eq!(outline_tan_half(Some(fov), widened(fov, 1080, 2400)).to_bits(), base.to_bits());
            assert_eq!(outline_tan_half(None, fov).to_bits(), base.to_bits());
        }
    }
}

#[cfg(test)]
mod view_sky {
    use super::Camera;

    /// sky_view: axes of the sky view = Rot(SkyZoneInfo) applied to the camera axes
    /// (300 varied cameras and rotations), and yaw only = sum of the yaws.
    #[test]
    fn view_sky_turned() {
        let mut seed_value = 12345u32;
        let mut random_value = |n: f32| {
            seed_value = seed_value.wrapping_mul(1_103_515_245).wrapping_add(12345);
            ((seed_value >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * n
        };
        for k in 0..300 {
            let cam = Camera { position: [random_value(1000.0), random_value(1000.0), random_value(1000.0)], yaw: random_value(180.0), pitch: random_value(89.0), roll: if k % 3 == 0 { random_value(30.0) } else { 0.0 } };
            let rot = [if k % 2 == 0 { random_value(2000.0) as i32 } else { 0 }, random_value(32768.0) as i32, if k % 5 == 0 { random_value(2000.0) as i32 } else { 0 }];
            let v = cam.sky_view([1.0, 2.0, 3.0], rot);
            assert_eq!(v.position, [1.0, 2.0, 3.0]);
            let u = |r: i32| r as f32 * 360.0 / 65536.0;
            let s = Camera { position: [0.0; 3], pitch: u(rot[0]), yaw: u(rot[1]), roll: u(rot[2]) }.axes();
            let want = cam.axes().map(|a| [0, 1, 2].map(|j| a[0] * s[0][j] + a[1] * s[1][j] + a[2] * s[2][j]));
            for (a, b) in v.axes().iter().zip(&want) {
                for j in 0..3 {
                    assert!((a[j] - b[j]).abs() < 1e-4, "case {k}: {:?} against {:?}", v.axes(), want);
                }
            }
            if rot[0] == 0 && rot[2] == 0 && cam.roll == 0.0 {
                let d = (v.yaw - cam.yaw - u(rot[1])).rem_euclid(360.0);
                assert!(d < 1e-3 || d > 360.0 - 1e-3, "case {k}: yaw {} against {}", v.yaw, cam.yaw + u(rot[1]));
                assert!((v.pitch - cam.pitch).abs() < 1e-3);
            }
        }
        // Without rotation: the sky camera keeps the view's orientation (former case).
        let cam = Camera { position: [0.0; 3], yaw: 30.0, pitch: 10.0, roll: 0.0 };
        let v = cam.sky_view([5.0, 5.0, 5.0], [0, 0, 0]);
        assert!((v.yaw - 30.0).abs() < 1e-3 && (v.pitch - 10.0).abs() < 1e-3 && v.roll.abs() < 1e-3);
    }
}

thread_local! {
    static REASON_EXTEND: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn refusal_extend(s: String) {
    REASON_EXTEND.with(|r| *r.borrow_mut() = s);
}

pub fn reason_extend() -> String {
    REASON_EXTEND.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Character list buffers too small for `vertices_list` / `index` bytes: replaced by
/// buffers one and a half times larger, whose start is the (GPU-side) copy of the
/// old ones, pending writes included (submitted before the copy). The bytes stay those
/// of the list in place; only the new end is written afterwards by the caller. Without COPY_SRC
/// (buffer from another path): nothing, the caller rebuilds the list.
fn grow_dynamic(device: &wgpu::Device, queue: &wgpu::Queue, d: &mut Dynamic, vertices_list: u64, index: u64) {
    let too_small = |b: &wgpu::Buffer, n: u64| n > b.size() && b.usage().contains(wgpu::BufferUsages::COPY_SRC);
    if !too_small(&d.geometry.vertex_buffer, vertices_list) && !too_small(&d.shadow_vertices, vertices_list) && !too_small(&d.geometry.index_buffer, index) {
        return;
    }
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("character list enlarged") });
    let mut grow = |b: &mut wgpu::Buffer, n: u64, label: &str| {
        if !too_small(b, n) {
            return;
        }
        let size_value = (n * 2).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
        let fresh_value = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size_value, usage: b.usage(), mapped_at_creation: false });
        enc.copy_buffer_to_buffer(b, 0, &fresh_value, 0, b.size());
        if crate::game::dbg_reason() {
            diag(&format!("list reason t={:.3} buffer {label} enlarged on the GPU: {} -> {} B (copy of {} B)", crate::game::dbg_time(), b.size(), size_value, b.size()));
        }
        *b = fresh_value;
    };
    grow(&mut d.geometry.vertex_buffer, vertices_list, "animated");
    grow(&mut d.shadow_vertices, vertices_list, "shadow silhouettes");
    grow(&mut d.geometry.index_buffer, index, "animated (indices)");
    submit_value(queue, [enc.finish()]);
}

/// Vertex buffer of the render textures (comic panels, `use_target_vertices`) ready for `vertices_list`
/// vertices from the moment the list is loaded: filled with the collapsed vertex (`SceneVertex::from(&Vertex::default())`)
/// and its mirror too. Otherwise, on the first comic panel, `use_target_vertices` created it empty (zeros) and
/// all its compared pages differed from the collapsed vertex (colour 255): an upload of the whole
/// buffer (several MB). Same bytes at the end: collapsed vertex everywhere, except the
/// meshes of the view uploaded by `update_dynamic`. Too small: enlarged on the GPU (copy
/// of the content, new end filled with the collapsed vertex). Nothing while the view is active.
fn prepare_target(device: &wgpu::Device, queue: &wgpu::Queue, d: &mut Dynamic, vertices_list: usize) {
    if d.target_active || d.inactive_vertices.as_ref().is_some_and(|(_, u)| u.uploaded.len() >= vertices_list) {
        return;
    }
    let size = std::mem::size_of::<SceneVertex>();
    let overwrites = SceneVertex::from(&Vertex::default());
    // Same margin as the list buffers (set_dynamic, three times).
    let capacity = vertices_list * 3;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("animated vertices of the render textures"),
        size: (capacity * size) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let (start_value, updates) = match d.inactive_vertices.take() {
        // Old buffer copyable: its content copied on the GPU, its mirror kept.
        Some((old, mut u)) if old.usage().contains(wgpu::BufferUsages::COPY_SRC) => {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render textures: enlarged vertices") });
            let n = u.uploaded.len();
            enc.copy_buffer_to_buffer(&old, 0, &buffer, 0, (n * size) as u64);
            submit_value(queue, [enc.finish()]);
            u.uploaded.resize(capacity, overwrites);
            u.staging.resize(capacity, overwrites);
            u.dirty.resize(capacity.div_ceil(VERTICES_PER_PAGE), false);
            (n, u)
        }
        _ => {
            let mut u = VertexUpdates::new(vec![overwrites; capacity]);
            // The mirror is exact (collapsed vertex everywhere): no comparison of every page.
            u.fresh = false;
            (0, u)
        }
    };
    let tail_part = vec![overwrites; capacity - start_value];
    write_value(queue, &buffer, (start_value * size) as u64, bytemuck::cast_slice(&tail_part));
    if crate::game::dbg_reason() {
        diag(&format!("list reason t={:.3} render texture buffer prepared: {capacity} vertices ({} copied on the GPU, {} written)", crate::game::dbg_time(), start_value, capacity - start_value));
    }
    d.inactive_vertices = Some((buffer, updates));
}

/// Masked scenery indices (`set_bsp_visible`): only the ranges of 1024 indices that differ from
/// `mirror` (uploaded content of the buffer) are written, two ranges separated by a single equal range
/// in one upload. Same bytes on the GPU as a full rewrite (several hundred KB on each broken window or
/// shutter).
fn write_ranges(queue: &wgpu::Queue, buffer: &wgpu::Buffer, mirror: &mut Vec<u32>, fresh_value: &[u32]) {
    let whole = mirror.len() != fresh_value.len();
    if whole {
        write_value(queue, buffer, 0, bytemuck::cast_slice(fresh_value));
        mirror.clear();
        mirror.extend_from_slice(fresh_value);
        return;
    }
    const RANGE_SPAN: usize = 1024;
    let n = fresh_value.len().div_ceil(RANGE_SPAN);
    let deferred = |k: usize| {
        let r = k * RANGE_SPAN..((k + 1) * RANGE_SPAN).min(fresh_value.len());
        mirror[r.clone()] != fresh_value[r]
    };
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for k in 0..n {
        if deferred(k) {
            match runs.last_mut() {
                Some(r) if k <= r.1 + 1 => r.1 = k + 1,
                _ => runs.push((k, k + 1)),
            }
        }
    }
    for (a, b) in runs {
        let r = a * RANGE_SPAN..(b * RANGE_SPAN).min(fresh_value.len());
        write_value(queue, buffer, (r.start * 4) as u64, bytemuck::cast_slice(&fresh_value[r.clone()]));
        mirror[r.clone()].copy_from_slice(&fresh_value[r]);
    }
}

/// A texture the game makes while it runs and that was asked for before it existed (the prewarm
/// list at map load) is read once it is made, and a replaced one is uploaded again, in the
/// groups already made too (the improved menus' pictures were drawn white in the game window).
#[cfg(all(test, feature = "mods"))]
#[test]
fn runtime_textures_made_or_replaced_late() {
    use openxiii_assets::mod_textures;
    let empty = std::env::temp_dir().join(format!("oxiii-empty-library-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let mut library = Library::new(&empty).unwrap();
    let mut r = pollster::block_on(Renderer::new_headless(16, 16, &World::default(), &mut library)).expect("a graphics adapter is needed for this test");
    let name = "Mod.TestRuntimeLate";
    let pixel = |c: [u8; 4]| vec![(1u32, 1u32, c.to_vec())];
    // Asked for before it exists: missing.
    assert!(r.texture(&mut library, name).is_none());
    mod_textures::set_runtime(name, pixel([255, 0, 0, 255]));
    assert!(r.texture(&mut library, name).is_some(), "made after it was first asked for: must be read");
    let key: BindKey = std::array::from_fn(|s| (if s == 0 { Some(name.to_string()) } else { None }, 0, 0, SAMPLER_WORLD));
    let group = r.bind_group(&mut library, key.clone());
    let first = r.mod_uploaded.get(name).cloned().unwrap();
    mod_textures::set_runtime(name, pixel([0, 0, 255, 255]));
    let key2 = key.clone();
    assert_eq!(r.bind_group(&mut library, key), group, "same group number");
    let now = mod_textures::get(name).unwrap();
    let uploaded = r.mod_uploaded.get(name).cloned().expect("replaced picture uploaded with the group");
    assert!(!Arc::ptr_eq(&uploaded, &first) && Arc::ptr_eq(&uploaded, &now));
    // Taken away: the picture is no longer kept (the game's own texture of that name, here
    // none, is read again), in the group already made too.
    mod_textures::remove_runtime(name);
    assert_eq!(r.bind_group(&mut library, key2), group, "same group number");
    assert!(r.mod_uploaded.get(name).is_none(), "a removed picture must not stay uploaded");
    assert!(matches!(r.textures.get(name), Some(None)), "read again from the game: missing here");
    let _ = std::fs::remove_dir_all(&empty);
}

/// A load in progress showing the last frame again (Renderer::hold_start).
pub struct PresentHold {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration, u32, std::time::Duration, std::time::Instant)>,
}

/// Frames kept for `Renderer::hold_start` (a turn of the loading animation).
const HOLD_FRAMES: usize = 12;

/// The kept frame shown `elapsed_ms` after its turn began, each one `step_ms` long, among `len`
/// (None when there is none).
pub fn held_index(len: usize, elapsed_ms: u64, step_ms: u64) -> Option<usize> {
    (len > 0).then(|| (elapsed_ms / step_ms.max(1)) as usize % len)
}

/// Whether the build has the cheats, written to the diagnostics at start.
#[cfg(feature = "cheats")]
pub(crate) const BUILD_CHEATS: &str = "OXIIIFLAG cheats=on";
#[cfg(not(feature = "cheats"))]
pub(crate) const BUILD_CHEATS: &str = "OXIIIFLAG cheats=off";

/// Presentation of the window's surface for a V-sync mode: Off, no wait (AutoNoVsync); Classic,
/// each image waits for the refresh (AutoVsync).
fn present_mode(vsync: openxiii_vm::video_settings::VsyncMode) -> wgpu::PresentMode {
    use openxiii_vm::video_settings::VsyncMode;
    match vsync {
        VsyncMode::Off => wgpu::PresentMode::AutoNoVsync,
        VsyncMode::Classic => wgpu::PresentMode::AutoVsync,
    }
}

/// Journal line: the V-sync mode asked and the presentation applied.
fn vsync_line(vsync: openxiii_vm::video_settings::VsyncMode, applied: wgpu::PresentMode) -> String {
    format!("V-sync {vsync:?}: presentation {applied:?}")
}

#[cfg(test)]
mod vsync_tests {
    use super::{present_mode, vsync_line};
    use openxiii_vm::video_settings::VsyncMode;
    use wgpu::PresentMode as P;

    #[test]
    fn present_mode_of_each_vsync_mode() {
        assert_eq!(present_mode(VsyncMode::Off), P::AutoNoVsync);
        assert_eq!(present_mode(VsyncMode::Classic), P::AutoVsync);
        assert_eq!(vsync_line(VsyncMode::Classic, P::AutoVsync), "V-sync Classic: presentation AutoVsync");
    }
}

register_viewer_layer!("rendering");
