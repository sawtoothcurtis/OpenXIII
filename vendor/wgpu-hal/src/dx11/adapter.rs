//! The adapter: opening the device and the capabilities announced to wgpu.
//!
//! - `capabilities_list`: features (none required by OpenXIII; only announce what is
//!   really implemented), limits (`wgt::Limits::downlevel_defaults()` raised to what D3D11
//!   11_0 allows: 14 constant buffers per stage, 1 of them reserved (REGISTRY_SPECIAL_CONSTANTS),
//!   128 t# registers, 16 samplers, 16384 textures, `min_uniform_buffer_offset_alignment` =
//!   ALIGNMENT_CONSTANTS, `max_uniform_buffer_binding_size` = 65536), alignments, `downlevel`
//!   (shader_model Sm5, flags of level 11_0).
//! - `Adapter::open`: D3D11CreateDevice on `self.raw` (D3D_DRIVER_TYPE_UNKNOWN, levels 11_1 then
//!   11_0, D3D11_CREATE_DEVICE_DEBUG if InstanceFlags::VALIDATION and available), cast to
//!   ID3D11Device1 / ID3D11DeviceContext1, then `assemble`.
//! - `texture_format_capabilities`: CheckFormatSupport on `conv::format_dxgi(format)`. `Adapter`
//!   keeps no device: one query device per adapter is created on the first
//!   question and kept in `DEVICES_LOOKUP`.
//! - `surface_capabilities`: Bgra8Unorm/Rgba8Unorm formats (+ Srgb as a view), sizes,
//!   Fifo/Immediate modes, Opaque alpha, usage COLOR_TARGET | COPY_DST | COPY_SRC, 2 to 16 images.

use alloc::{sync::Arc, vec, vec::Vec};

use parking_lot::Mutex;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{Direct3D, Direct3D11 as d3d11, Dxgi},
        UI::WindowsAndMessaging,
    },
};

use super::{conv, Adapter, Api, Device, Queue, Surface};
use crate::auxil::dxgi::result::HResult as _;

/// Levels requested from D3D11CreateDevice, from highest to lowest.
const LEVELS: [Direct3D::D3D_FEATURE_LEVEL; 2] =
    [Direct3D::D3D_FEATURE_LEVEL_11_1, Direct3D::D3D_FEATURE_LEVEL_11_0];

/// Creates a D3D11 device on the adapter (levels 11_1 then 11_0). With `debug`, first tries
/// with the debug layer, then without it if it is not installed.
pub(super) fn create_device(
    adapter_value: &Dxgi::IDXGIAdapter1,
    debug: bool,
) -> windows::core::Result<(d3d11::ID3D11Device, d3d11::ID3D11DeviceContext, Direct3D::D3D_FEATURE_LEVEL)> {
    let test_value = |flag_bits: d3d11::D3D11_CREATE_DEVICE_FLAG| {
        let mut device_value = None;
        let mut context_value = None;
        let mut level_value = Direct3D::D3D_FEATURE_LEVEL::default();
        unsafe {
            d3d11::D3D11CreateDevice(
                adapter_value,
                // Adapter given: the driver type must be UNKNOWN.
                Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                flag_bits,
                Some(&LEVELS),
                d3d11::D3D11_SDK_VERSION,
                Some(&mut device_value),
                Some(&mut level_value),
                Some(&mut context_value),
            )
        }?;
        match (device_value, context_value) {
            (Some(a), Some(c)) => Ok((a, c, level_value)),
            _ => Err(windows::core::Error::from(windows::Win32::Foundation::E_POINTER)),
        }
    };
    // By default, the driver does not start its worker thread (D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS):
    // its work stays on the calling thread (on a single-core machine this matches the original
    // or does better). Same D3D11 calls.
    let base = d3d11::D3D11_CREATE_DEVICE_BGRA_SUPPORT | d3d11::D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS;
    if debug {
        match test_value(base | d3d11::D3D11_CREATE_DEVICE_DEBUG) {
            Ok(r) => return Ok(r),
            // DXGI_ERROR_SDK_COMPONENT_MISSING: debug layer missing.
            Err(e) => log::warn!("Direct3D 11: debug layer unavailable ({e}), normal device"),
        }
    }
    test_value(base)
}

/// True if the device has what the backend requires: level 11_0 at least, ID3D11Device1, and
/// constant buffer offsetting and partial update (D3D 11.1).
pub(super) fn device_enough(device_value: &d3d11::ID3D11Device, level_value: Direct3D::D3D_FEATURE_LEVEL) -> bool {
    if level_value.0 < Direct3D::D3D_FEATURE_LEVEL_11_0.0 {
        return false;
    }
    if device_value.cast::<d3d11::ID3D11Device1>().is_err() {
        return false;
    }
    let mut options = d3d11::D3D11_FEATURE_DATA_D3D11_OPTIONS::default();
    let value_read = unsafe {
        device_value.CheckFeatureSupport(
            d3d11::D3D11_FEATURE_D3D11_OPTIONS,
            <*mut _>::cast(&mut options),
            size_of_val(&options) as u32,
        )
    };
    value_read.is_ok() && options.ConstantBufferOffsetting.as_bool() && options.ConstantBufferPartialUpdate.as_bool()
}

/// Capabilities of an adapter at level `level_value` (called by `Instance::enumerate_adapters`).
pub(super) fn capabilities_list(
    device_value: &d3d11::ID3D11Device,
    level_value: Direct3D::D3D_FEATURE_LEVEL,
) -> (wgt::Features, crate::Capabilities) {
    // Only what the backend really implements: depth clipping (dithering) and
    // BC1-BC7 textures (all present at level 11_0).
    let features = wgt::Features::DEPTH_CLIP_CONTROL | wgt::Features::TEXTURE_COMPRESSION_BC;

    // t# registers: 128 per stage, shared between sampled textures and read-only storage
    // buffers (raw SRV, pipelines.rs). u# registers: 8 at level 11_0 (compute; in pixel shaders,
    // shared with the render targets), shared between writable storage buffers and storage
    // textures.
    let buffers_storage = 4;
    let textures_storage = 4;
    let constants = super::REGISTRY_SPECIAL_CONSTANTS; // b0..b12; b13 reserved
    let limits = crate::auxil::adjust_raw_limits(wgt::Limits {
        // D3D11_REQ_TEXTURE1D_U_DIMENSION, TEXTURE2D_U_OR_V_DIMENSION, TEXTURECUBE_DIMENSION
        max_texture_dimension_1d: 16384,
        max_texture_dimension_2d: 16384,
        // D3D11_REQ_TEXTURE3D_U_V_OR_W_DIMENSION, TEXTURE2D_ARRAY_AXIS_DIMENSION
        max_texture_dimension_3d: 2048,
        max_texture_array_layers: 2048,
        max_bind_groups: 4,
        max_sampled_textures_per_shader_stage: d3d11::D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT - buffers_storage,
        max_samplers_per_shader_stage: d3d11::D3D11_COMMONSHADER_SAMPLER_SLOT_COUNT,
        max_storage_buffers_per_shader_stage: buffers_storage,
        max_storage_textures_per_shader_stage: textures_storage,
        max_uniform_buffers_per_shader_stage: constants,
        max_buffers_and_acceleration_structures_per_shader_stage: constants + buffers_storage + 16,
        // D3D11_REQ_CONSTANT_BUFFER_ELEMENT_COUNT (4096) * 16 bytes
        max_uniform_buffer_binding_size: 65536,
        // Guaranteed size of a resource: D3D11_REQ_RESOURCE_SIZE_IN_MEGABYTES_EXPRESSION_A_TERM.
        max_storage_buffer_binding_size: 128 << 20,
        max_buffer_size: 128 << 20,
        max_vertex_buffers: 16,
        max_vertex_attributes: 16,
        max_vertex_buffer_array_stride: 2048,
        max_inter_stage_shader_variables: 16,
        min_uniform_buffer_offset_alignment: super::ALIGNMENT_CONSTANTS as u32,
        min_storage_buffer_offset_alignment: 256,
        max_color_attachments: d3d11::D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT,
        max_color_attachment_bytes_per_sample: 32,
        // Compute at level 11_0 (cs_5_0).
        max_compute_workgroup_storage_size: d3d11::D3D11_CS_TGSM_REGISTER_COUNT * 4,
        max_compute_invocations_per_workgroup: d3d11::D3D11_CS_THREAD_GROUP_MAX_THREADS_PER_GROUP,
        max_compute_workgroup_size_x: d3d11::D3D11_CS_THREAD_GROUP_MAX_X,
        max_compute_workgroup_size_y: d3d11::D3D11_CS_THREAD_GROUP_MAX_Y,
        max_compute_workgroup_size_z: d3d11::D3D11_CS_THREAD_GROUP_MAX_Z,
        max_compute_workgroups_per_dimension: d3d11::D3D11_CS_DISPATCH_MAX_THREAD_GROUPS_PER_DIMENSION,
        // No immediate constants, mesh shaders, ray tracing or multiview.
        max_immediate_size: 0,
        ..wgt::Limits::downlevel_defaults()
    });

    let alignments = crate::Alignments {
        buffer_copy_offset: wgt::BufferSize::new(4).unwrap(),
        buffer_copy_pitch: wgt::BufferSize::new(256).unwrap(),
        // queue.rs binds constant ranges rounded to 256 bytes; out-of-range reads
        // are bounded by D3D (like DX12).
        uniform_bounds_check_alignment: wgt::BufferSize::new(16).unwrap(),
        raw_tlas_instance_size: 0,
        ray_tracing_scratch_buffer_alignment: 0,
        ray_tracing_pipeline_group_data_size: 0,
        ray_tracing_pipeline_group_data_alignment: 0,
        ray_tracing_pipeline_data_offset_alignment: 0,
    };

    use wgt::DownlevelFlags as Df;
    let flags = Df::all().difference(
        // Partial copy of a depth texture impossible (CopySubresourceRegion: whole subresource
        // only; UpdateSubresource refused on a depth texture).
        Df::DEPTH_TEXTURE_AND_BUFFER_COPIES
            // The formats' real capabilities come from CheckFormatSupport (typed storage
            // limited at level 11_0): wgpu-core must ask for them.
            | Df::WEBGPU_TEXTURE_FORMAT_SUPPORT
            // Query results are read by the CPU (GetData) during replay.
            | Df::NONBLOCKING_QUERY_RESOLVE
            // Views of other formats on the window's texture: not implemented (resources.rs, surface.rs).
            | Df::SURFACE_VIEW_FORMATS
            // Metal only.
            | Df::MSL2_1
            // Indirect draws: wgpu-core validates them with a compute pipeline using immediate
            // constants, which this backend does not have; OpenXIII does not use them.
            | Df::INDIRECT_EXECUTION,
    );
    let downlevel = wgt::DownlevelCapabilities {
        flags,
        limits: wgt::DownlevelLimits::default(),
        shader_model: wgt::ShaderModel::Sm5,
    };
    let _ = (device_value, level_value);

    (features, crate::Capabilities { limits, alignments, downlevel, cooperative_matrix_properties: Vec::new() })
}

/// Query device (CheckFormatSupport) of an adapter, identified by its LUID.
struct Lookup {
    luid: (u32, i32),
    device_value: d3d11::ID3D11Device,
}
// Only the CheckFormatSupport / CheckMultisampleQualityLevels functions are called: safe
// across threads (ID3D11Device is free-threaded).
unsafe impl Send for Lookup {}

static DEVICES_LOOKUP: Mutex<Vec<Lookup>> = Mutex::new(Vec::new());

impl Adapter {
    /// Query device of this adapter, created on the first request then kept.
    fn device_lookup(&self) -> Option<d3d11::ID3D11Device> {
        let desc = unsafe { self.raw.GetDesc1() }.ok()?;
        let luid = (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart);
        let mut cache = DEVICES_LOOKUP.lock();
        if let Some(c) = cache.iter().find(|c| c.luid == luid) {
            return Some(c.device_value.clone());
        }
        let (device_value, _context, _level) = create_device(&self.raw, false)
            .inspect_err(|e| log::error!("Direct3D 11: query device: {e}"))
            .ok()?;
        cache.push(Lookup { luid, device_value: device_value.clone() });
        Some(device_value)
    }
}

/// Bits of D3D11_FORMAT_SUPPORT (or SUPPORT2) present in `value_value`.
fn a(value_value: u32, bit: i32) -> bool {
    value_value & bit as u32 != 0
}

impl crate::Adapter for Adapter {
    type A = Api;

    unsafe fn open(
        &self,
        features: wgt::Features,
        _limits: &wgt::Limits,
        _memory_hints: &wgt::MemoryHints,
    ) -> Result<crate::OpenDevice<Api>, crate::DeviceError> {
        let debug = self.flags.contains(wgt::InstanceFlags::VALIDATION);
        let (device_value, context_value, level_value) =
            create_device(&self.raw, debug).into_device_result("D3D11CreateDevice")?;
        if level_value != self.feature_level {
            log::warn!("Direct3D 11: level {:#x} when opened, {:#x} when listed", level_value.0, self.feature_level.0);
        }
        let device1 = device_value.cast::<d3d11::ID3D11Device1>().into_device_result("ID3D11Device1")?;
        let context1 = context_value.cast::<d3d11::ID3D11DeviceContext1>().into_device_result("ID3D11DeviceContext1")?;
        Ok(assemble(device1, context1, level_value))
    }

    unsafe fn texture_format_capabilities(&self, format: wgt::TextureFormat) -> crate::TextureFormatCapabilities {
        use crate::TextureFormatCapabilities as Tfc;
        use d3d11::*;

        // Formats without a DXGI equivalent (ASTC, ETC2…): nothing.
        if crate::auxil::dxgi::conv::map_texture_format_failable(format).is_none() {
            return Tfc::empty();
        }
        let Some(device_value) = self.device_lookup() else {
            return Tfc::empty();
        };
        let raw_value = conv::format_dxgi(format);
        // Read and write views of a depth texture use the equivalent color format.
        let view_value = if format.is_depth_stencil_format() {
            let aspect = if format.has_depth_aspect() { wgt::TextureAspect::DepthOnly } else { wgt::TextureAspect::StencilOnly };
            conv::format_srv(format, aspect)
        } else {
            raw_value
        };
        let support = |f| unsafe { device_value.CheckFormatSupport(f) }.unwrap_or(0);
        let support2 = |f| {
            let mut d = D3D11_FEATURE_DATA_FORMAT_SUPPORT2 { InFormat: f, OutFormatSupport2: 0 };
            let value_read = unsafe {
                device_value.CheckFeatureSupport(D3D11_FEATURE_FORMAT_SUPPORT2, <*mut _>::cast(&mut d), size_of_val(&d) as u32)
            };
            if value_read.is_ok() { d.OutFormatSupport2 } else { 0 }
        };
        let s = support(raw_value);
        let s_view = if view_value == raw_value { s } else { support(view_value) };
        let s2_view = support2(view_value);

        let mut caps = Tfc::COPY_SRC | Tfc::COPY_DST;
        let texture = a(
            s,
            D3D11_FORMAT_SUPPORT_TEXTURE1D.0
                | D3D11_FORMAT_SUPPORT_TEXTURE2D.0
                | D3D11_FORMAT_SUPPORT_TEXTURE3D.0
                | D3D11_FORMAT_SUPPORT_TEXTURECUBE.0,
        );
        caps.set(Tfc::SAMPLED, texture && a(s_view, D3D11_FORMAT_SUPPORT_SHADER_LOAD.0));
        caps.set(Tfc::SAMPLED_LINEAR, texture && a(s_view, D3D11_FORMAT_SUPPORT_SHADER_SAMPLE.0));
        caps.set(Tfc::COLOR_ATTACHMENT, a(s, D3D11_FORMAT_SUPPORT_RENDER_TARGET.0));
        caps.set(Tfc::COLOR_ATTACHMENT_BLEND, a(s, D3D11_FORMAT_SUPPORT_BLENDABLE.0));
        caps.set(Tfc::DEPTH_STENCIL_ATTACHMENT, a(s, D3D11_FORMAT_SUPPORT_DEPTH_STENCIL.0));

        let uav = a(s_view, D3D11_FORMAT_SUPPORT_TYPED_UNORDERED_ACCESS_VIEW.0);
        caps.set(Tfc::STORAGE_READ_ONLY, uav && a(s2_view, D3D11_FORMAT_SUPPORT2_UAV_TYPED_LOAD.0));
        caps.set(Tfc::STORAGE_WRITE_ONLY, uav && a(s2_view, D3D11_FORMAT_SUPPORT2_UAV_TYPED_STORE.0));
        caps.set(Tfc::STORAGE_READ_WRITE, caps.contains(Tfc::STORAGE_READ_ONLY | Tfc::STORAGE_WRITE_ONLY));
        caps.set(Tfc::STORAGE_ATOMIC, uav && a(s2_view, D3D11_FORMAT_SUPPORT2_UAV_ATOMIC_UNSIGNED_MIN_OR_MAX.0));

        // Multisampling: the target must accept it, and reading too if the format is readable.
        let target_value = a(s, D3D11_FORMAT_SUPPORT_RENDER_TARGET.0 | D3D11_FORMAT_SUPPORT_DEPTH_STENCIL.0);
        let ms_possible = target_value
            && a(s, D3D11_FORMAT_SUPPORT_MULTISAMPLE_RENDERTARGET.0)
            && (!caps.contains(Tfc::SAMPLED) || a(s_view, D3D11_FORMAT_SUPPORT_MULTISAMPLE_LOAD.0));
        caps.set(Tfc::MULTISAMPLE_RESOLVE, a(s, D3D11_FORMAT_SUPPORT_MULTISAMPLE_RESOLVE.0));
        if ms_possible {
            for (n, bit) in [
                (2, Tfc::MULTISAMPLE_X2),
                (4, Tfc::MULTISAMPLE_X4),
                (8, Tfc::MULTISAMPLE_X8),
                (16, Tfc::MULTISAMPLE_X16),
            ] {
                let levels = unsafe { device_value.CheckMultisampleQualityLevels(raw_value, n) }.unwrap_or(0);
                caps.set(bit, levels != 0);
            }
        }
        caps
    }

    unsafe fn surface_capabilities(&self, surface: &Surface) -> Option<crate::SurfaceCapabilities> {
        let mut rect = RECT::default();
        let current_extent = if unsafe { WindowsAndMessaging::GetClientRect(surface.hwnd, &mut rect) }.is_ok() {
            Some(wgt::Extent3d {
                width: (rect.right - rect.left).max(0) as u32,
                height: (rect.bottom - rect.top).max(0) as u32,
                depth_or_array_layers: 1,
            })
        } else {
            log::warn!("Direct3D 11: GetClientRect failed");
            None
        };
        Some(crate::SurfaceCapabilities {
            // srgb formats go through the view (surface.rs); the swap chain keeps the non-srgb format.
            formats: [
                wgt::TextureFormat::Bgra8Unorm,
                wgt::TextureFormat::Rgba8Unorm,
                wgt::TextureFormat::Bgra8UnormSrgb,
                wgt::TextureFormat::Rgba8UnormSrgb,
            ]
            .map(|format| wgt::SurfaceFormatCapabilities { format, color_spaces: wgt::SurfaceColorSpaces::SRGB })
            .to_vec(),
            maximum_frame_latency: 2..=16,
            current_extent,
            usage: wgt::TextureUses::COLOR_TARGET | wgt::TextureUses::COPY_DST | wgt::TextureUses::COPY_SRC,
            // "Auto" modes are never announced by a backend: wgpu-core replaces them
            // with Fifo (AutoVsync) or Immediate (AutoNoVsync) before calling configure.
            present_modes: vec![wgt::PresentMode::Fifo, wgt::PresentMode::Immediate],
            composite_alpha_modes: vec![wgt::CompositeAlphaMode::Opaque],
        })
    }

    unsafe fn get_presentation_timestamp(&self) -> wgt::PresentationTimestamp {
        wgt::PresentationTimestamp::INVALID_TIMESTAMP
    }

    fn get_ordered_buffer_usages(&self) -> wgt::BufferUses {
        wgt::BufferUses::INCLUSIVE | wgt::BufferUses::MAP_WRITE
    }

    fn get_ordered_texture_usages(&self) -> wgt::TextureUses {
        wgt::TextureUses::INCLUSIVE | wgt::TextureUses::COLOR_TARGET | wgt::TextureUses::DEPTH_STENCIL_WRITE
    }
}

/// Builds the device and the queue from an open D3D11 device (usable by `open`).
pub(super) fn assemble(
    device: d3d11::ID3D11Device1,
    context: d3d11::ID3D11DeviceContext1,
    feature_level: Direct3D::D3D_FEATURE_LEVEL,
) -> crate::OpenDevice<Api> {
    let shared = Arc::new(super::Shared {
        device,
        context: Mutex::new(context),
        feature_level,
        zeros: vec![0; super::ZERO_BUFFER_SIZE],
        pass_constants: parking_lot::Mutex::new(None),
        targets_generation: core::sync::atomic::AtomicU64::new(0),
        objects_shaders: parking_lot::Mutex::new(Default::default()),
        buffer_back: core::sync::atomic::AtomicUsize::new(0),
        views_back: parking_lot::Mutex::new(Vec::new()),
    });
    crate::OpenDevice {
        device: Device { shared: shared.clone() },
        queue: Queue { shared, state_value: Mutex::new(super::queue::StateContext::default()) },
    }
}
