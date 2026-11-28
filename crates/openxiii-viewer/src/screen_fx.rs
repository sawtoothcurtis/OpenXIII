//! 2D screen effects (zone flash, water, poison, blur, sharpen, injury, underwater): the
//! scene is drawn into a 512 × 512 texture (FD3DRenderInterface), then the
//! VM's `FxPass` passes (Apply2DEffect) redraw it into the frame before the
//! Canvas. The Direct3D 8 states set by are reproduced by the shader below.

use openxiii_vm::screen_fx::{FxPass, FxTarget, FxTex, FxVertex};
use std::collections::HashMap;

/// Size of the scene texture (512 × 512 viewport).
pub const SIDE: u32 = 512;

/// Size of the small flash texture (UD3DRenderDevice +0x1498: 128 × 128,
/// A8R8G8B8, D3DUSAGE_RENDERTARGET).
pub const SMALL: u32 = 128;

const SHADER: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) uv1: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct Mode { mode: u32, pad0: u32, pad1: u32, pad2: u32 };
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> m: Mode;

@vertex fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(v.pos, 0.5, 1.0);
    o.uv0 = v.uv0;
    o.uv1 = v.uv1;
    // D3DCOLOR (B, G, R, A in memory).
    o.color = v.color.zyxw;
    return o;
}

@fragment fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let t0 = textureSample(scene, samp, i.uv0);
    let t1 = textureSample(scene, samp, i.uv1);
    if (m.mode == 1u) {
        // Stage 0 SELECTARG1 (texture, vertex alpha), stage 1 SUBTRACT (current − texture).
        return vec4<f32>(clamp(t0.rgb - t1.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), i.color.a);
    }
    if (m.mode == 2u) {
        // Stage 1 BLENDDIFFUSEALPHA: current × vertex alpha + texture × (1 − alpha).
        let a = i.color.a;
        return vec4<f32>(t0.rgb * a + t1.rgb * (1.0 - a), a);
    }
    if (m.mode == 3u) {
        // MODULATE2X (colour), MODULATE (alpha).
        return vec4<f32>(clamp(t0.rgb * i.color.rgb * 2.0, vec3<f32>(0.0), vec3<f32>(1.0)), t0.a * i.color.a);
    }
    if (m.mode == 4u) {
        // DOTPRODUCT3 (texture, vertex): signed components, sum × 4 saturated, copied to the
        // four channels.
        let d = clamp(4.0 * dot(t0.rgb - vec3<f32>(0.5), i.color.rgb - vec3<f32>(0.5)), 0.0, 1.0);
        return vec4<f32>(d);
    }
    if (m.mode == 5u) {
        // SELECTARG1 (vertex) in colour and in alpha.
        return i.color;
    }
    return t0 * i.color;
}
"#;

/// Pipeline variant: shader mode, blend (`blend`), triangle list, alpha written
/// (A8R8G8B8 textures; not the X8R8G8B8 screen buffer).
type Key = (u32, u8, bool, bool);

pub struct ScreenFxRenderer {
    pub view: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    /// Small flash texture (+0x1498, surface +0x14A4).
    small: wgpu::TextureView,
    /// Stage 0 without a texture (SetTexture NULL): never read by the stages that use it.
    empty: wgpu::TextureView,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    sampler: wgpu::Sampler,
    pipelines: HashMap<Key, wgpu::RenderPipeline>,
    format: wgpu::TextureFormat,
    /// Side of the scene texture and of its depth.
    pub side: u32,
    /// Bind groups made once: [shader mode][texture: scene, small, none] (the mode uniform of each is
    /// constant), so that no GPU object is created while playing, as the original reuses its
    /// device states.
    groups: Vec<[wgpu::BindGroup; 3]>,
    /// Vertex and index rings written with `queue.write_buffer` (D3DLOCK_DISCARD-like reuse):
    /// (buffer, capacity in bytes, next write offset).
    vertex_ring: (wgpu::Buffer, u64, u64),
    index_ring: (wgpu::Buffer, u64, u64),
}

/// Shader modes of the effect shader (0..=5).
const MODES: u32 = 6;
/// Ring capacities: far above what one frame's passes use (a full-screen quad is 112 bytes, the
/// water mesh a few thousand vertices).
const VERTEX_RING: u64 = 1 << 20;
const INDEX_RING: u64 = 1 << 18;

/// Next offset in a ring for `len` bytes (multiple of 4); `None` if it is larger than the ring.
fn ring_take(ring: &mut (wgpu::Buffer, u64, u64), len: u64) -> Option<u64> {
    if len > ring.1 {
        return None;
    }
    if ring.2 + len > ring.1 {
        ring.2 = 0;
    }
    let at = ring.2;
    ring.2 += len.next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
    Some(at)
}

/// Direct3D 8 blending (same factors for alpha): 0 none, 1 REVSUBTRACT ONE / ONE,
/// 2 SRCALPHA / INVSRCALPHA, 3 ONE / SRCALPHA, 4 REVSUBTRACT ONE / SRCALPHA, 5 SRCCOLOR / ZERO,
/// 6 SRCCOLOR / ONE, 7 ONE / ONE.
fn blend(kind: u8) -> Option<wgpu::BlendState> {
    use wgpu::{BlendFactor as F, BlendOperation as O};
    let (src, dst, op) = match kind {
        1 => (F::One, F::One, O::ReverseSubtract),
        2 => return Some(wgpu::BlendState::ALPHA_BLENDING),
        3 => (F::One, F::SrcAlpha, O::Add),
        4 => (F::One, F::SrcAlpha, O::ReverseSubtract),
        5 => (F::Src, F::Zero, O::Add),
        6 => (F::Src, F::One, O::Add),
        7 => (F::One, F::One, O::Add),
        _ => return None,
    };
    let c = wgpu::BlendComponent { src_factor: src, dst_factor: dst, operation: op };
    Some(wgpu::BlendState { color: c, alpha: c })
}

/// Shader mode and blending according to its `flags`.
fn rect_state(flags: u32) -> (u32, u8) {
    let blend = if flags & 1 != 0 {
        3
    } else if flags & 0x40 != 0 {
        4
    } else if flags & 2 != 0 {
        5
    } else if flags & 0x10 != 0 {
        6
    } else if flags & 0x120 != 0 {
        // 0x100: ONE / ONE with a pixel shader that the flash does not use.
        7
    } else {
        0
    };
    let mode = if flags & 4 != 0 {
        4
    } else if flags & 8 != 0 {
        5
    } else if flags & 0x80 != 0 {
        3
    } else {
        0
    };
    (mode, blend)
}

impl ScreenFxRenderer {
    /// `side`: side of the scene texture (SIDE as the original; the improved build asks for more
    /// at high resolutions).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, side: u32) -> Self {
        let scene_side = side;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effects scene"),
            size: wgpu::Extent3d { width: scene_side, height: scene_side, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let target = |label: &str, side: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let small = target("small flash texture", SMALL);
        let empty = target("without a texture", 1);
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("effects depth"),
                size: wgpu::Extent3d { width: scene_side, height: scene_side, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effects"),
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
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("effects"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("effects"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        // ADDRESSU / ADDRESSV of stage 0 in CLAMP; linear filtering.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("effects"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        use wgpu::util::DeviceExt;
        let groups: Vec<[wgpu::BindGroup; 3]> = (0..MODES)
            .map(|m| {
                let mode = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("effect"),
                    contents: bytemuck::cast_slice(&[m, 0, 0, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                [&view, &small, &empty].map(|tex| {
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("effect"),
                        layout: &layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(tex) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                            wgpu::BindGroupEntry { binding: 2, resource: mode.as_entire_binding() },
                        ],
                    })
                })
            })
            .collect();
        let ring = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            (device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: usage | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), size, 0)
        };
        let vertex_ring = ring("effect vertices", VERTEX_RING, wgpu::BufferUsages::VERTEX);
        let index_ring = ring("effect indices", INDEX_RING, wgpu::BufferUsages::INDEX);
        let mut fx = ScreenFxRenderer {
            view,
            depth,
            small,
            empty,
            layout,
            pipeline_layout,
            shader,
            sampler,
            pipelines: HashMap::new(),
            format,
            side: scene_side,
            groups,
            vertex_ring,
            index_ring,
        };
        // The 32 possible pipelines (blend × mesh × alpha) created up front: the first
        // screen effect (explosion flash, blood) no longer compiles anything mid-game (mainly Android
        // drivers).
        for b in 0..8u8 {
            for mesh in [false, true] {
                for alpha in [false, true] {
                    fx.pipeline(device, (0, b, mesh, alpha));
                }
            }
        }
        fx
    }

    fn pipeline(&mut self, device: &wgpu::Device, key: Key) -> &wgpu::RenderPipeline {
        // The shader mode goes through the uniform: it does not change the pipeline.
        // Blends outside 1-7: no blending (blend), same pipeline as 0.
        let key = (0, if (1..=7).contains(&key.1) { key.1 } else { 0 }, key.2, key.3);
        let mesh = key.2;
        let write_mask = if key.3 { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::COLOR };
        let (layout, shader, format) = (&self.pipeline_layout, &self.shader, self.format);
        self.pipelines.entry(key).or_insert_with(|| {
            let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Unorm8x4];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("effect"),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 28, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: if mesh { wgpu::PrimitiveTopology::TriangleList } else { wgpu::PrimitiveTopology::TriangleStrip },
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    // X8R8G8B8 screen buffer: no alpha written.
                    targets: &[Some(wgpu::ColorTargetState { format, blend: blend(key.1), write_mask })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }

    /// Draws the passes: the starting surface is the scene texture, `target` is the
    /// screen buffer (width × height in pixels).
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, passes: &[FxPass], width: u32, height: u32) {
        use wgpu::util::DeviceExt;
        let mut current = FxTarget::Scene;
        let bytes = |verts: &[FxVertex], (w, h): (u32, u32)| -> Vec<u8> {
            // Direct3D 8 pixel centres: pixel i covers [i − 0.5, i + 0.5] after the
            // viewport scaling (NDC −1 → 0), whereas wgpu centres it at i + 0.5. Direct3D's pixel i
            // therefore shows what falls at i; here the vertices move half a pixel to the
            // right and down (NDC Y up) so that pixel i shows it too.
            let (dx, dy) = (1.0 / w.max(1) as f32, 1.0 / h.max(1) as f32);
            let mut out = Vec::with_capacity(verts.len() * 28);
            for v in verts {
                for f in [v.pos[0] + dx, v.pos[1] - dy, v.uv0[0], v.uv0[1], v.uv1[0], v.uv1[1]] {
                    out.extend_from_slice(&f.to_le_bytes());
                }
                out.extend_from_slice(&v.color.to_le_bytes());
            }
            out
        };
        // X8R8G8B8 screen buffer: alpha at 1; its content before the effect (previous frame) is
        // not kept.
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("effect: buffer"),
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
        for p in passes {
            let (view, size) = match current {
                FxTarget::Scene => (&self.view, (self.side, self.side)),
                FxTarget::Small => (&self.small, (SMALL, SMALL)),
                FxTarget::Back => (target, (width, height)),
            };
            let write_alpha = current != FxTarget::Back;
            let (verts, indices, key, tex) = match p {
                FxPass::Target(t) => {
                    current = *t;
                    continue;
                }
                FxPass::Clear { color } => {
                    let c = |s: u32| ((color >> s) & 0xff) as f64 / 255.0;
                    let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("effect: clear"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: c(16), g: c(8), b: c(0), a: if write_alpha { c(24) } else { 1.0 } }), store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    continue;
                }
                FxPass::Quad { verts, flags } => {
                    let mode = if flags & 8 != 0 {
                        1
                    } else if flags & 0x10 != 0 {
                        2
                    } else {
                        0
                    };
                    let blend = if flags & 2 != 0 {
                        1
                    } else if flags & 4 != 0 {
                        2
                    } else {
                        0
                    };
                    (verts.to_vec(), None, (mode, blend, false, write_alpha), 0usize)
                }
                FxPass::Mesh { verts, indices, modulate2x } => (verts.clone(), Some(indices.clone()), (if *modulate2x { 3 } else { 0 }, 0, true, write_alpha), 0usize),
                FxPass::Rect { verts, tex, flags } => {
                    let (mode, blend) = rect_state(*flags);
                    let tex = match tex {
                        FxTex::None => 2usize,
                        FxTex::Scene => 0,
                        FxTex::Small => 1,
                    };
                    (verts.to_vec(), None, (mode, blend, false, write_alpha), tex)
                }
            };
            let view = view.clone();
            let pipeline = self.pipeline(device, key).clone();
            // Vertices and indices go into the rings made at creation (a buffer is only created
            // if one pass alone is larger than its ring, which does not happen with the game's effects).
            let vertex_bytes = bytes(&verts, size);
            let vertex_own;
            let (vertex, vertex_at): (&wgpu::Buffer, u64) = match ring_take(&mut self.vertex_ring, vertex_bytes.len() as u64) {
                Some(at) => {
                    queue.write_buffer(&self.vertex_ring.0, at, &vertex_bytes);
                    (&self.vertex_ring.0, at)
                }
                None => {
                    vertex_own = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("effect"), contents: &vertex_bytes, usage: wgpu::BufferUsages::VERTEX });
                    (&vertex_own, 0)
                }
            };
            let index_own;
            let index: Option<(&wgpu::Buffer, u64, u32)> = match indices.as_ref() {
                None => None,
                Some(i) => {
                    let count = i.len() as u32;
                    let mut i = i.clone();
                    if i.len() % 2 == 1 {
                        i.push(0);
                    }
                    let data: &[u8] = bytemuck::cast_slice(&i);
                    match ring_take(&mut self.index_ring, data.len() as u64) {
                        Some(at) => {
                            queue.write_buffer(&self.index_ring.0, at, data);
                            Some((&self.index_ring.0, at, count))
                        }
                        None => {
                            index_own = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("effect"), contents: data, usage: wgpu::BufferUsages::INDEX });
                            Some((&index_own, 0, count))
                        }
                    }
                }
            };
            let group = &self.groups[key.0.min(MODES - 1) as usize][tex];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effect"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_vertex_buffer(0, vertex.slice(vertex_at..vertex_at + vertex_bytes.len() as u64));
            match &index {
                Some((buffer, at, count)) => {
                    pass.set_index_buffer(buffer.slice(*at..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..*count, 0, 0..1);
                }
                None => pass.draw(0..verts.len() as u32, 0..1),
            }
        }
    }
}
