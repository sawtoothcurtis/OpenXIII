//! Image quality settings (not in the original game, which has none of them): internal
//! resolution of the 3D world (render scale), multisample anti-aliasing and anisotropic
//! texture filtering.
//!
//! The values live in [OpenXIII.Video] of the System ini (XIII.ini, next to
//! FullscreenViewportX/Y): RenderScale (percent, 25 to 400), MSAA (1, 2, 4 or 8) and
//! AnisotropicFiltering (1, 2, 4, 8 or 16). Missing or invalid values take the defaults: 100,
//! 1, and 1 in the faithful build (as the original) or 16 (4 on Android) in the improved build.
//!
//! With RenderScale 100 and MSAA 1 the world is drawn straight into the frame, as before. Otherwise
//! it is drawn into an off-screen target of the scaled size (multisampled with MSAA, then
//! resolved), and copied into the frame with linear filtering (several taps when downsampling);
//! the HUD, menus and text are drawn afterwards at the frame's own resolution.
//!
//! Game state is never touched: these settings only change how the frame is drawn.

/// Section and file of the settings (the System ini, like WinDrv.WindowsClient's viewport sizes).
pub const INI_SECTION: &str = "OpenXIII.Video";
pub const INI_FILE: &str = "System";
pub const KEY_RENDER_SCALE: &str = "RenderScale";
pub const KEY_MSAA: &str = "MSAA";
pub const KEY_ANISOTROPY: &str = "AnisotropicFiltering";

/// Render scale bounds, in percent of the frame's size.
pub const MIN_RENDER_SCALE: u32 = 25;
pub const MAX_RENDER_SCALE: u32 = 400;
/// Sample counts offered (1: no multisampling).
pub const MSAA_CHOICES: [u32; 4] = [1, 2, 4, 8];
/// Anisotropy levels offered (1: trilinear only).
pub const ANISOTROPY_CHOICES: [u16; 5] = [1, 2, 4, 8, 16];

/// Default anisotropy (openxiii_vm::video_settings::ANISOTROPY_DEFAULT): 1 in the faithful build
/// (as the original), 16 on PC and 4 on Android in the improved build (mobile GPUs pay more).
pub const fn default_anisotropy() -> u16 {
    openxiii_vm::video_settings::ANISOTROPY_DEFAULT as u16
}

/// The three settings, as requested (before the device's limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderQuality {
    /// Internal size of the 3D world, in percent of the frame (25 to 400).
    pub render_scale: u32,
    /// Samples per pixel of the 3D world (1, 2, 4 or 8).
    pub msaa: u32,
    /// Anisotropic filtering of the world's textures (1, 2, 4, 8 or 16).
    pub anisotropy: u16,
}

impl Default for RenderQuality {
    fn default() -> Self {
        RenderQuality { render_scale: 100, msaa: 1, anisotropy: default_anisotropy() }
    }
}

impl RenderQuality {
    /// Values brought back into their ranges: scale clamped, MSAA and anisotropy rounded down to
    /// the nearest offered value (at least 1).
    pub fn sanitized(self) -> Self {
        RenderQuality {
            render_scale: self.render_scale.clamp(MIN_RENDER_SCALE, MAX_RENDER_SCALE),
            msaa: MSAA_CHOICES.iter().copied().filter(|&n| n <= self.msaa.max(1)).max().unwrap_or(1),
            anisotropy: ANISOTROPY_CHOICES.iter().copied().filter(|&n| n <= self.anisotropy.max(1)).max().unwrap_or(1),
        }
    }

    /// The original game's drawing (Graphics: Original): world at the frame's size, no MSAA,
    /// no anisotropic filtering.
    pub fn original() -> Self {
        RenderQuality { render_scale: 100, msaa: 1, anisotropy: 1 }
    }

    /// Values of [OpenXIII.Video], read by the VM's parser (`VideoSettings::from_texts`, shared
    /// with the Video menu): a missing, unreadable or out-of-range key gives its default.
    pub fn from_config(config: &mut openxiii_vm::config::ConfigCache) -> Self {
        let mut read = |key: &str| config.get_string(INI_SECTION, key, INI_FILE);
        let (scale, msaa, aniso) = (read(KEY_RENDER_SCALE), read(KEY_MSAA), read(KEY_ANISOTROPY));
        let s = openxiii_vm::video_settings::VideoSettings::from_texts(None, scale.as_deref(), msaa.as_deref(), aniso.as_deref(), None);
        RenderQuality { render_scale: s.render_scale, msaa: s.msaa, anisotropy: s.anisotropy.min(u32::from(u16::MAX)) as u16 }.sanitized()
    }

    /// Writes the three keys into [OpenXIII.Video] (saved with the ini's next flush).
    pub fn save(&self, config: &mut openxiii_vm::config::ConfigCache) {
        let q = self.sanitized();
        config.set_string(INI_SECTION, KEY_RENDER_SCALE, &q.render_scale.to_string(), INI_FILE);
        config.set_string(INI_SECTION, KEY_MSAA, &q.msaa.to_string(), INI_FILE);
        config.set_string(INI_SECTION, KEY_ANISOTROPY, &q.anisotropy.to_string(), INI_FILE);
    }
}

/// What the device allows, read once when the renderer is created.
#[derive(Debug, Clone)]
pub struct QualityCaps {
    /// Sample counts usable for the frame's colour format and the depth format (1 always).
    pub msaa_counts: Vec<u32>,
    /// Anisotropic filtering available (DownlevelFlags::ANISOTROPIC_FILTERING).
    pub anisotropy: bool,
    /// Largest side of a 2D texture.
    pub max_texture_dimension: u32,
}

impl Default for QualityCaps {
    fn default() -> Self {
        QualityCaps { msaa_counts: vec![1], anisotropy: false, max_texture_dimension: 2048 }
    }
}

impl QualityCaps {
    pub fn new(adapter: &wgpu::Adapter, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let mut msaa_counts = supported_msaa_counts(adapter, format);
        // Without TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES on the device, wgpu only accepts the
        // counts WebGPU guarantees (1 and 4).
        if !device.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
            msaa_counts.retain(|&n| n == 1 || n == 4);
        }
        QualityCaps {
            msaa_counts,
            anisotropy: adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING),
            max_texture_dimension: device.limits().max_texture_dimension_2d,
        }
    }

    /// Settings actually applied: highest supported sample count not above the request, scale
    /// limited by the texture size, anisotropy 1 when unavailable.
    pub fn effective(&self, q: RenderQuality, width: u32, height: u32) -> RenderQuality {
        let q = q.sanitized();
        RenderQuality {
            render_scale: q.render_scale.min(max_scale_for(width, height, self.max_texture_dimension)),
            msaa: effective_msaa(q.msaa, &self.msaa_counts),
            anisotropy: if self.anisotropy { q.anisotropy } else { 1 },
        }
    }
}

/// Sample counts (among MSAA_CHOICES) that the adapter supports for rendering and resolving
/// `format`, and for the depth target (Depth32Float). Always contains 1.
pub fn supported_msaa_counts(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> Vec<u32> {
    let color = adapter.get_texture_format_features(format);
    let depth = adapter.get_texture_format_features(wgpu::TextureFormat::Depth32Float);
    let resolve = color.flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    MSAA_CHOICES
        .iter()
        .copied()
        .filter(|&n| n == 1 || (resolve && color.flags.sample_count_supported(n) && depth.flags.sample_count_supported(n)))
        .collect()
}

/// Highest count of `supported` not above `requested` (1 if none).
pub fn effective_msaa(requested: u32, supported: &[u32]) -> u32 {
    supported.iter().copied().filter(|&n| n <= requested.max(1)).max().unwrap_or(1)
}

/// Largest render scale (percent, at most MAX_RENDER_SCALE) whose target still fits the device's
/// largest texture for a `screen_w` × `screen_h` frame.
pub fn max_render_scale_percent(screen_w: u32, screen_h: u32, limits: &wgpu::Limits) -> u32 {
    max_scale_for(screen_w, screen_h, limits.max_texture_dimension_2d)
}

fn max_scale_for(w: u32, h: u32, max_dim: u32) -> u32 {
    let side = u64::from(w.max(h).max(1));
    ((u64::from(max_dim) * 100 / side) as u32).clamp(MIN_RENDER_SCALE, MAX_RENDER_SCALE)
}

/// Size of the world's target for a `w` × `h` frame at `scale` percent (at least 1 × 1, at most
/// `max_dim` per side).
pub fn scaled_size(w: u32, h: u32, scale: u32, max_dim: u32) -> (u32, u32) {
    let s = |v: u32| ((u64::from(v) * u64::from(scale)).div_ceil(100) as u32).clamp(1, max_dim.max(1));
    (s(w), s(h))
}

/// Bilinear taps per axis of the copy into the frame: one bilinear tap averages 2 × 2 texels,
/// so a destination pixel covering `ratio` source texels needs ceil(ratio / 2) taps per axis.
pub fn composite_taps(ratio: f32) -> u32 {
    if ratio <= 1.0 { 1 } else { ((ratio / 2.0).ceil() as u32).clamp(1, 4) }
}

const COMPOSITE_SHADER: &str = r#"
struct Params {
    // Destination rectangle in frame pixels (x, y, width, height).
    dst: vec4<f32>,
    // Source rectangle in texels of the world target (x, y, width, height).
    src: vec4<f32>,
    // 1 / texture size (xy), bilinear taps per axis (z).
    texel: vec4<f32>,
};
@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_smp: sampler;
@group(0) @binding(2) var<uniform> p: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Triangle covering the viewport.
    let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(xy.x * 2.0 - 1.0, 1.0 - xy.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let ratio = p.src.zw / p.dst.zw;
    // Centre of the destination pixel, in source texels.
    let centre = p.src.xy + (pos.xy - p.dst.xy) * ratio;
    let n = u32(p.texel.z);
    if (n <= 1u) {
        return textureSampleLevel(src_tex, src_smp, centre * p.texel.xy, 0.0);
    }
    // Box filter over the pixel's footprint: n × n bilinear taps spread evenly.
    var sum = vec4<f32>(0.0);
    for (var y = 0u; y < n; y = y + 1u) {
        for (var x = 0u; x < n; x = x + 1u) {
            let o = (vec2<f32>(f32(x), f32(y)) + 0.5) / f32(n) - 0.5;
            sum = sum + textureSampleLevel(src_tex, src_smp, (centre + o * ratio) * p.texel.xy, 0.0);
        }
    }
    return sum / f32(n * n);
}
"#;

/// Off-screen target of the 3D world: colour (multisampled with MSAA), resolved colour,
/// depth, and the group the copy into the frame reads.
pub struct WorldTarget {
    pub size: (u32, u32),
    pub samples: u32,
    /// Drawn into by the world's passes.
    pub color: wgpu::TextureView,
    /// Single-sample copy of `color` (MSAA only).
    pub resolve: Option<wgpu::TextureView>,
    pub depth: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// Pipeline that copies the world's target into the frame.
pub struct Compositor {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl Compositor {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("world copy"), source: wgpu::ShaderSource::Wgsl(COMPOSITE_SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world copy"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("world copy"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world copy"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("world copy"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("world copy"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Compositor { pipeline, layout, sampler, uniform }
    }

    /// Target of `size` with `samples` samples per pixel, in `format`.
    pub fn target(&self, device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32), samples: u32) -> WorldTarget {
        let extent = wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 };
        let texture = |label: &str, samples: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let sampled_usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        // Multisampled attachments are only rendered to and resolved (render buffers on OpenGL ES).
        let (color, resolve) = if samples > 1 {
            (texture("world (multisampled)", samples, format, wgpu::TextureUsages::RENDER_ATTACHMENT), Some(texture("world", 1, format, sampled_usage)))
        } else {
            (texture("world", 1, format, sampled_usage), None)
        };
        let depth = texture("world depth", samples, wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world copy"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(resolve.as_ref().unwrap_or(&color)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.uniform.as_entire_binding() },
            ],
        });
        WorldTarget { size, samples, color, resolve, depth, group }
    }

    /// Resolves the multisampled colour (if any), then copies the target's rectangle `src`
    /// (texels) into `dst` (pixels) of `frame` (`frame_size` pixels).
    #[allow(clippy::too_many_arguments)]
    pub fn copy(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &WorldTarget,
        frame: &wgpu::TextureView,
        frame_size: (u32, u32),
        src: [f32; 4],
        dst: [f32; 4],
    ) {
        if let Some(resolve) = &target.resolve {
            let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color,
                    depth_slice: None,
                    resolve_target: Some(resolve),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        let ratio = (src[2] / dst[2].max(1.0)).max(src[3] / dst[3].max(1.0));
        let params: [f32; 12] = [
            dst[0], dst[1], dst[2], dst[3],
            src[0], src[1], src[2], src[3],
            1.0 / target.size.0 as f32, 1.0 / target.size.1 as f32, composite_taps(ratio) as f32, 0.0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world copy"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let x = dst[0].clamp(0.0, frame_size.0 as f32 - 1.0);
        let y = dst[1].clamp(0.0, frame_size.1 as f32 - 1.0);
        let w = dst[2].min(frame_size.0 as f32 - x).max(1.0);
        let h = dst[3].min(frame_size.1 as f32 - y).max(1.0);
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_scissor_rect(x as u32, y as u32, w as u32, h as u32);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &target.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_values() {
        let q = RenderQuality { render_scale: 1000, msaa: 6, anisotropy: 12 }.sanitized();
        assert_eq!(q, RenderQuality { render_scale: 400, msaa: 4, anisotropy: 8 });
        let q = RenderQuality { render_scale: 0, msaa: 0, anisotropy: 0 }.sanitized();
        assert_eq!(q, RenderQuality { render_scale: 25, msaa: 1, anisotropy: 1 });
        assert_eq!(RenderQuality::default().sanitized(), RenderQuality::default());
    }

    #[test]
    fn msaa_falls_back_to_highest_supported() {
        assert_eq!(effective_msaa(8, &[1, 2, 4]), 4);
        assert_eq!(effective_msaa(2, &[1, 4]), 1);
        assert_eq!(effective_msaa(4, &[1, 4, 8]), 4);
        assert_eq!(effective_msaa(1, &[1]), 1);
    }

    #[test]
    fn max_scale_follows_texture_limit() {
        let mut limits = wgpu::Limits::downlevel_webgl2_defaults();
        limits.max_texture_dimension_2d = 8192;
        assert_eq!(max_render_scale_percent(1920, 1080, &limits), 400);
        assert_eq!(max_render_scale_percent(3840, 2160, &limits), 213);
        limits.max_texture_dimension_2d = 2048;
        assert_eq!(max_render_scale_percent(1920, 1080, &limits), 106);
        assert_eq!(max_render_scale_percent(16384, 16384, &limits), MIN_RENDER_SCALE);
    }

    #[test]
    fn scaled_sizes() {
        assert_eq!(scaled_size(1920, 1080, 100, 8192), (1920, 1080));
        assert_eq!(scaled_size(1920, 1080, 50, 8192), (960, 540));
        assert_eq!(scaled_size(1920, 1080, 200, 8192), (3840, 2160));
        assert_eq!(scaled_size(1920, 1080, 400, 4096), (4096, 4096));
        assert_eq!(scaled_size(3, 3, 25, 8192), (1, 1));
    }

    #[test]
    fn taps_cover_the_footprint() {
        assert_eq!(composite_taps(0.5), 1);
        assert_eq!(composite_taps(1.0), 1);
        assert_eq!(composite_taps(2.0), 1);
        assert_eq!(composite_taps(3.0), 2);
        assert_eq!(composite_taps(4.0), 2);
        assert_eq!(composite_taps(8.0), 4);
    }

    #[test]
    fn composite_shader_is_valid() {
        wgpu::naga::front::wgsl::parse_str(COMPOSITE_SHADER).expect("valid WGSL");
    }

    #[test]
    fn config_round_trip() {
        let dir = std::env::temp_dir().join(format!("oxiii-quality-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("XIII.ini"), "[OpenXIII.Video]\r\nRenderScale=150\r\nMSAA=4\r\nAnisotropicFiltering=8\r\n").unwrap();
        let mut c = openxiii_vm::config::ConfigCache::with_command_line(dir.clone(), None, "XIII.exe", false);
        assert_eq!(RenderQuality::from_config(&mut c), RenderQuality { render_scale: 150, msaa: 4, anisotropy: 8 });
        RenderQuality { render_scale: 50, msaa: 2, anisotropy: 2 }.save(&mut c);
        assert_eq!(RenderQuality::from_config(&mut c), RenderQuality { render_scale: 50, msaa: 2, anisotropy: 2 });
        std::fs::write(dir.join("XIII.ini"), "[OpenXIII.Video]\r\nRenderScale=abc\r\n").unwrap();
        let mut c = openxiii_vm::config::ConfigCache::with_command_line(dir.clone(), None, "XIII.exe", false);
        assert_eq!(RenderQuality::from_config(&mut c), RenderQuality::default());
        // Out of range or not offered: the default of each key, as in the Video menu.
        std::fs::write(dir.join("XIII.ini"), "[OpenXIII.Video]
RenderScale=1000
MSAA=6
AnisotropicFiltering=12
").unwrap();
        let mut c = openxiii_vm::config::ConfigCache::with_command_line(dir.clone(), None, "XIII.exe", false);
        assert_eq!(RenderQuality::from_config(&mut c), RenderQuality::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
