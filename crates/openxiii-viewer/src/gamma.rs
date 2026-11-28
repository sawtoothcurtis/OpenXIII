//! Full-screen gamma ramp: UD3DRenderDevice::UpdateGamma sets through
//! SetGammaRamp a table of 256 16-bit entries, the same for red, green and blue,
//! derived from the client's Brightness, Contrast and Gamma (+0x50, +0x54, +0x58), if the device
//! can do it in full screen (+0x118 bit 0x20000); RestoreGamma restores
//! the identity (i << 8). Direct3D 8 only applies it in full screen. Here the finished frame is
//! copied then redrawn through the table (high 8 bits of each entry, like a screen
//! with 8 bits per component).

const SHADER: &str = r#"
@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var ramp: texture_2d<f32>;

@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn through(c: f32) -> f32 {
    let i = i32(round(clamp(c, 0.0, 1.0) * 255.0));
    return textureLoad(ramp, vec2<i32>(i, 0), 0).r;
}

@fragment fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(scene, vec2<i32>(p.xy), 0);
    return vec4<f32>(through(c.r), through(c.g), through(c.b), 1.0);
}
"#;

/// The ramp is applied in full screen, as Direct3D 8 does (the original's windowed game ignores
/// Brightness, Contrast and Gamma); the improved build with Graphics: Enhanced applies it in a
/// window too, so the Video page's settings work in every display mode. Graphics: Original stays
/// the original's.
pub fn applies(fullscreen: bool, graphics: openxiii_vm::video_settings::SessionGraphics) -> bool {
    fullscreen || crate::renderer::gamma_in_window(graphics)
}

/// FISTP round to nearest (even on ties), the FPU rounding mode.
fn fistp(x: f32) -> i32 {
    x.round_ties_even() as i32
}

/// appPow (msvcr70's _CIpow) with the FPU in 24 bits: y × log2(x) in 80 bits (FYL2X),
/// integer part FRNDINT, fraction rounded to 24 bits (FSUB), 2^fraction through F2XM1 + 1 rounded to 24 bits
/// (FADD), then FSCALE. More faithful than `powf`: the original differs by one ulp in 2 % of cases.
/// Valid for x > 0 and finite y; 0 stays 0.
fn app_pow(x: f64, y: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let t = y * x.log2();
    let n = t.round_ties_even();
    let f = (t - n) as f32;
    let s = (f as f64).exp2() as f32;
    (s as f64) * n.exp2()
}

/// UpdateGamma's table for (Brightness, Contrast, Gamma): for i from 0 to 255,
/// ((B − 0.5) × 32768 + (C + 0.5) × pow(i × (1/255), 1/G) × 65535) − C × 32768 + 16384, rounded
/// and clamped to [0, 65535]. pow is appPow (double), the rest in float (24-bit FPU).
pub fn ramp(color: [f32; 3]) -> [u16; 256] {
    let [brightness, contrast, gamma] = color;
    let inv255 = f32::from_bits(0x3b80_8081);
    let mut out = [0u16; 256];
    for (i, v) in out.iter_mut().enumerate() {
        let p = app_pow((i as f32 * inv255) as f64, (1.0 / gamma) as f64) as f32;
        let f = (((brightness - 0.5) * 32768.0 + (contrast + 0.5) * p * 65535.0) - contrast * 32768.0) + 16384.0;
        let n = fistp(f);
        *v = if n < 0 {
            0
        } else if n > 0xfffe {
            0xffff
        } else {
            n as u16
        };
    }
    out
}

/// Identity table of RestoreGamma: i << 8.
pub fn is_identity(r: &[u16; 256]) -> bool {
    r.iter().enumerate().all(|(i, &v)| (v >> 8) as usize == i)
}

pub struct GammaRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    ramp_texture: wgpu::Texture,
    ramp_view: wgpu::TextureView,
    copy: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    format: wgpu::TextureFormat,
    current: Option<[u16; 256]>,
}

impl GammaRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gamma"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gamma"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gamma"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gamma"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
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
        let ramp_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gamma ramp"),
            size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ramp_view = ramp_texture.create_view(&wgpu::TextureViewDescriptor::default());
        GammaRenderer { pipeline, layout, ramp_texture, ramp_view, copy: None, format, current: None }
    }

    /// Copies `frame` (its surface must allow COPY_SRC) and redraws it through the ramp.
    pub fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &wgpu::Texture, ramp: &[u16; 256]) {
        if self.current.as_ref() != Some(ramp) {
            let data: Vec<u8> = ramp.iter().flat_map(|&v| ((v >> 8) as f32 / 255.0).to_le_bytes()).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.ramp_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            );
            self.current = Some(*ramp);
        }
        let (w, h) = (frame.width(), frame.height());
        if !self.copy.as_ref().is_some_and(|c| c.2 == w && c.3 == h) {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("frame before the ramp"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let v = t.create_view(&wgpu::TextureViewDescriptor::default());
            self.copy = Some((t, v, w, h));
        }
        let (copy, copy_view, _, _) = self.copy.as_ref().unwrap();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gamma"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(copy_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.ramp_view) },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gamma") });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: frame, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: copy, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let target = frame.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gamma"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_xiii_ini_give_identity() {
        let r = ramp([0.5, 0.5, 1.0]);
        assert!(is_identity(&r));
        assert_eq!(r[255], 65535);
    }
}

