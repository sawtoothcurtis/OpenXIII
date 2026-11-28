//! Resources: buffers, textures, views, samplers.
//!
//! - `create_buffer_impl`: according to `desc.usage` (wgt::BufferUses):
//!   * MAP_WRITE (with or without COPY_SRC) and no other GPU usage -> CPU buffer:
//!     `raw = None`, `shadow = Some(vec![0; size])` (this is write_buffer's staging buffer);
//!   * MAP_READ -> `raw` DEFAULT (if COPY_DST) + `read_value` STAGING CPU_ACCESS_READ + `shadow`;
//!   * UNIFORM -> D3D11_BIND_CONSTANT_BUFFER alone (size rounded up to 256; D3D 11.1 allows more than
//!     64 KB with offsets); if UNIFORM is combined with VERTEX/INDEX/STORAGE: create without
//!     CONSTANT_BUFFER and report it in the log (OpenXIII has none, to be checked);
//!   * VERTEX / INDEX -> BIND_VERTEX_BUFFER | BIND_INDEX_BUFFER;
//!   * STORAGE_READ_ONLY -> BIND_SHADER_RESOURCE + MISC_BUFFER_ALLOW_RAW_VIEWS;
//!     STORAGE_READ_WRITE -> + BIND_UNORDERED_ACCESS;
//!   * INDIRECT -> MISC_DRAWINDIRECT_ARGS.
//!   Size rounded up to 4 bytes. Label (SetPrivateData WKPDID_D3DDebugObjectName) if given.
//! - `open_buffer` / `close_buffer`: pointer into `shadow` (is_coherent = true). For MAP_READ,
//!   first copy `read_value` into `shadow` (Map READ on the context, under the lock).
//! - `create_texture_impl`: Texture1D/2D/3D depending on the dimension; format `conv::format_dxgi`, as
//!   typeless if the texture has views of other formats or if it is a depth texture read as a
//!   texture (conv::format_typeless); BIND_* according to usage; MISC_TEXTURECUBE if 6·k square
//!   layers and RESOURCE usage (approximation: wgpu does not give the view dimension at creation).
//! - `create_view`: SRV if RESOURCE, RTV if COLOR_TARGET, DSV (+ read-only DSV) if DEPTH_STENCIL,
//!   UAV if STORAGE; mip and layer ranges from `desc.range`.
//! - `create_sampler_impl`: D3D11_SAMPLER_DESC through conv (filters, addressing, comparison,
//!   anisotropy, LOD, border color).

use alloc::{sync::Arc, vec};
use core::ptr::NonNull;

use parking_lot::Mutex;
use windows::Win32::Graphics::{Direct3D, Direct3D11 as d3d11, Dxgi::Common as dxgi};

use super::{conv, Buffer, Device, DeviceResult, Sampler, Texture, TextureView};
use windows::core::Interface;

use crate::auxil::dxgi::result::HResult as _;

/// Names an object for debugging tools (PIX, D3D11 debug layer).
unsafe fn label_value<T: Interface>(object_value: &T, label: crate::Label) {
    if let (Some(name_value), Ok(object_value)) = (label, object_value.cast::<d3d11::ID3D11DeviceChild>()) {
        let _ = unsafe {
            object_value.SetPrivateData(&Direct3D::WKPDID_D3DDebugObjectName, name_value.len() as u32, Some(name_value.as_ptr().cast()))
        };
    }
}

fn round(value_value: u64, alignment: u64) -> u64 {
    value_value.div_ceil(alignment) * alignment
}

// ---------------------------------------------------------------------------------------------
// Buffers
// ---------------------------------------------------------------------------------------------

pub(super) unsafe fn create_buffer_impl(device: &Device, desc: &crate::BufferDescriptor) -> DeviceResult<Buffer> {
    use wgt::BufferUses as U;
    let usage = desc.usage;
    let device_value = &device.shared.device;

    // Usages that require a real buffer on the GPU.
    let map_value = U::COPY_DST
        | U::INDEX
        | U::VERTEX
        | U::UNIFORM
        | U::STORAGE_READ_ONLY
        | U::STORAGE_READ_WRITE
        | U::INDIRECT
        | U::QUERY_RESOLVE;

    // write_buffer's staging buffer (MAP_WRITE | COPY_SRC): CPU copy only.
    if usage.contains(U::MAP_WRITE) && !usage.intersects(map_value) {
        return Ok(Buffer {
            raw: None,
            size: desc.size,
            usage,
            shadow: Some(Arc::new(Mutex::new(memory_pass(desc.size as usize)))),
            read_value: None,
        });
    }

    let size_value = round(desc.size.max(1), 4);

    // CPU readback: STAGING buffer read by Map, copied into the shadow.
    let (read_value, shadow) = if usage.contains(U::MAP_READ) {
        let d = d3d11::D3D11_BUFFER_DESC {
            ByteWidth: size_value as u32,
            Usage: d3d11::D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: d3d11::D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut read_value = None;
        unsafe { device_value.CreateBuffer(&d, None, Some(&mut read_value)) }.into_device_result("CreateBuffer (read)")?;
        let read_value = read_value.ok_or(crate::DeviceError::Unexpected)?;
        unsafe { label_value(&read_value, desc.label) };
        (Some(read_value), Some(Arc::new(Mutex::new(vec![0; desc.size as usize]))))
    } else if usage.contains(U::MAP_WRITE) {
        // MAP_WRITE combined with a GPU usage (MAPPABLE_PRIMARY_BUFFERS feature, not used
        // by OpenXIII): the CPU copy is not sent back to the GPU.
        log::warn!("Direct3D 11: MAP_WRITE buffer with GPU usage ({usage:?}), not supported");
        (None, Some(Arc::new(Mutex::new(vec![0; desc.size as usize]))))
    } else {
        (None, None)
    };

    // GPU buffer, including for MAP_READ | COPY_DST: copies go into `raw`, then
    // execution (queue.rs) copies `raw` into `read_value`.
    let raw = if usage.intersects(map_value) {
        let mut bind = 0i32;
        let mut misc = 0i32;
        let mut size_map = size_value;
        let others = U::VERTEX | U::INDEX | U::STORAGE_READ_ONLY | U::STORAGE_READ_WRITE | U::INDIRECT;
        if usage.contains(U::UNIFORM) {
            if usage.intersects(others) {
                log::warn!("Direct3D 11: UNIFORM buffer combined with {usage:?}, created without CONSTANT_BUFFER");
            } else {
                // D3D11 refuses CONSTANT_BUFFER with another binding; size in blocks of 256
                // bytes (read in ranges of 16 constants with VSSetConstantBuffers1).
                bind |= d3d11::D3D11_BIND_CONSTANT_BUFFER.0;
                size_map = round(size_value, super::ALIGNMENT_CONSTANTS);
            }
        }
        if usage.intersects(U::VERTEX | U::INDEX) {
            bind |= d3d11::D3D11_BIND_VERTEX_BUFFER.0 | d3d11::D3D11_BIND_INDEX_BUFFER.0;
        }
        if usage.intersects(U::STORAGE_READ_ONLY | U::STORAGE_READ_WRITE) {
            bind |= d3d11::D3D11_BIND_SHADER_RESOURCE.0;
            misc |= d3d11::D3D11_RESOURCE_MISC_BUFFER_ALLOW_RAW_VIEWS.0;
        }
        if usage.contains(U::STORAGE_READ_WRITE) {
            bind |= d3d11::D3D11_BIND_UNORDERED_ACCESS.0;
        }
        if usage.contains(U::INDIRECT) {
            misc |= d3d11::D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS.0;
        }
        let d = d3d11::D3D11_BUFFER_DESC {
            ByteWidth: size_map as u32,
            Usage: d3d11::D3D11_USAGE_DEFAULT,
            BindFlags: bind as u32,
            CPUAccessFlags: 0,
            MiscFlags: misc as u32,
            StructureByteStride: 0,
        };
        let mut raw = None;
        unsafe { device_value.CreateBuffer(&d, None, Some(&mut raw)) }.into_device_result("CreateBuffer")?;
        let raw = raw.ok_or(crate::DeviceError::Unexpected)?;
        unsafe { label_value(&raw, desc.label) };
        Some(raw)
    } else {
        None
    };

    Ok(Buffer { raw, size: desc.size, usage, shadow, read_value })
}

pub(super) unsafe fn open_buffer(device: &Device, buffer: &Buffer, range: crate::MemoryRange) -> DeviceResult<crate::BufferMapping> {
    let shadow = buffer.shadow.as_ref().ok_or_else(|| {
        log::error!("Direct3D 11: mapping a buffer without a CPU copy ({:?})", buffer.usage);
        crate::DeviceError::Unexpected
    })?;
    let mut data_value = shadow.lock();

    if let Some(read_value) = &buffer.read_value {
        // Map READ waits until the GPU has finished writing `read_value`.
        let ctx = device.shared.context.lock();
        let mut unlocked = d3d11::D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { ctx.Map(read_value, 0, d3d11::D3D11_MAP_READ, 0, Some(&mut unlocked)) }.into_device_result("Map (read)")?;
        let start_value = (range.start as usize).min(data_value.len());
        let end_value = (range.end as usize).min(data_value.len());
        if end_value > start_value {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (unlocked.pData as *const u8).add(start_value),
                    data_value.as_mut_ptr().add(start_value),
                    end_value - start_value,
                )
            };
        }
        unsafe { ctx.Unmap(read_value, 0) };
    }

    // The Vec never changes size: the pointer stays valid after the lock.
    let ptr = unsafe { data_value.as_mut_ptr().add(range.start as usize) };
    Ok(crate::BufferMapping { ptr: NonNull::new(ptr).ok_or(crate::DeviceError::Unexpected)?, is_coherent: true })
}

pub(super) unsafe fn close_buffer(device: &Device, buffer: &Buffer) {}

// ---------------------------------------------------------------------------------------------
// Textures
// ---------------------------------------------------------------------------------------------

pub(super) unsafe fn create_texture_impl(device: &Device, desc: &crate::TextureDescriptor) -> DeviceResult<Texture> {
    use wgt::TextureUses as U;
    let usage = desc.usage;
    let device_value = &device.shared.device;
    let depth_value = desc.format.is_depth_stencil_format();

    let format = if !desc.view_formats.is_empty() || (depth_value && usage.contains(U::RESOURCE)) {
        conv::format_typeless(desc.format)
    } else {
        conv::format_dxgi(desc.format)
    };

    let mut bind = 0i32;
    if usage.contains(U::RESOURCE) {
        bind |= d3d11::D3D11_BIND_SHADER_RESOURCE.0;
    }
    if usage.contains(U::COLOR_TARGET) {
        bind |= d3d11::D3D11_BIND_RENDER_TARGET.0;
    }
    if usage.intersects(U::DEPTH_STENCIL_READ | U::DEPTH_STENCIL_WRITE) {
        bind |= d3d11::D3D11_BIND_DEPTH_STENCIL.0;
    }
    if usage.intersects(U::STORAGE_READ_ONLY | U::STORAGE_WRITE_ONLY | U::STORAGE_READ_WRITE | U::STORAGE_ATOMIC) {
        bind |= d3d11::D3D11_BIND_UNORDERED_ACCESS.0;
    }

    let layers = desc.size.depth_or_array_layers;
    let raw: d3d11::ID3D11Resource = match desc.dimension {
        wgt::TextureDimension::D1 => {
            let d = d3d11::D3D11_TEXTURE1D_DESC {
                Width: desc.size.width,
                MipLevels: desc.mip_level_count,
                ArraySize: layers,
                Format: format,
                Usage: d3d11::D3D11_USAGE_DEFAULT,
                BindFlags: bind as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut t = None;
            unsafe { device_value.CreateTexture1D(&d, None, Some(&mut t)) }.into_device_result("CreateTexture1D")?;
            t.ok_or(crate::DeviceError::Unexpected)?.cast().map_err(|_| crate::DeviceError::Unexpected)?
        }
        wgt::TextureDimension::D2 => {
            // Cube: D3D11 requires MISC_TEXTURECUBE at creation for a cube view (6·k square
            // layers, without multisampling).
            let cube = usage.contains(U::RESOURCE)
                && layers >= 6
                && layers % 6 == 0
                && desc.size.width == desc.size.height
                && desc.sample_count == 1;
            let d = d3d11::D3D11_TEXTURE2D_DESC {
                Width: desc.size.width,
                Height: desc.size.height,
                MipLevels: desc.mip_level_count,
                ArraySize: layers,
                Format: format,
                SampleDesc: dxgi::DXGI_SAMPLE_DESC { Count: desc.sample_count, Quality: 0 },
                Usage: d3d11::D3D11_USAGE_DEFAULT,
                BindFlags: bind as u32,
                CPUAccessFlags: 0,
                MiscFlags: if cube { d3d11::D3D11_RESOURCE_MISC_TEXTURECUBE.0 as u32 } else { 0 },
            };
            let mut t = None;
            unsafe { device_value.CreateTexture2D(&d, None, Some(&mut t)) }.into_device_result("CreateTexture2D")?;
            t.ok_or(crate::DeviceError::Unexpected)?.cast().map_err(|_| crate::DeviceError::Unexpected)?
        }
        wgt::TextureDimension::D3 => {
            let d = d3d11::D3D11_TEXTURE3D_DESC {
                Width: desc.size.width,
                Height: desc.size.height,
                Depth: layers,
                MipLevels: desc.mip_level_count,
                Format: format,
                Usage: d3d11::D3D11_USAGE_DEFAULT,
                BindFlags: bind as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut t = None;
            unsafe { device_value.CreateTexture3D(&d, None, Some(&mut t)) }.into_device_result("CreateTexture3D")?;
            t.ok_or(crate::DeviceError::Unexpected)?.cast().map_err(|_| crate::DeviceError::Unexpected)?
        }
    };
    unsafe { label_value(&raw, desc.label) };

    Ok(Texture {
        raw,
        format: desc.format,
        dimension: desc.dimension,
        size: desc.size,
        mip_level_count: desc.mip_level_count,
        sample_count: desc.sample_count,
        usage,
    })
}

// ---------------------------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------------------------

fn mode_views_surface() -> u8 {
    return 0;
}

/// Texture view. Views of the swap chain's back buffer are kept by description:
/// wgpu asks for two again every frame (render view, wgpu-core's clear view) on the same
/// buffer; the same D3D11 objects are returned (same resource, same description, hence the same views).
pub(super) unsafe fn create_view(device: &Device, texture: &Texture, desc: &crate::TextureViewDescriptor) -> DeviceResult<TextureView> {
    let mode = mode_views_surface();
    let back = device.shared.buffer_back.load(core::sync::atomic::Ordering::Acquire);
    if mode == 0 || (mode != 2 && (back == 0 || texture.raw.as_raw() as usize != back)) {
        return unsafe { create_new_view(device, texture, desc) };
    }
    let key_tuple = alloc::format!(
        "{:?} {:?} {:?} {:?} {} {:?} {} {:?}",
        desc.format, desc.dimension, desc.usage & texture.usage, desc.range.aspect, desc.range.base_mip_level,
        desc.range.mip_level_count, desc.range.base_array_layer, desc.range.array_layer_count
    );
    if let Some((_, v)) = device.shared.views_back.lock().iter().find(|(c, _)| *c == key_tuple) {
        return Ok(v.clone());
    }
    let v = unsafe { create_new_view(device, texture, desc) }?;
    device.shared.views_back.lock().push((key_tuple, v.clone()));
    Ok(v)
}

unsafe fn create_new_view(device: &Device, texture: &Texture, desc: &crate::TextureViewDescriptor) -> DeviceResult<TextureView> {
    use wgt::{TextureDimension as TD, TextureUses as U, TextureViewDimension as VD};
    let device_value = &device.shared.device;
    let usage = desc.usage & texture.usage;

    let mip0 = desc.range.base_mip_level;
    let nb_mips = desc.range.mip_level_count.unwrap_or(texture.mip_level_count.saturating_sub(mip0));
    let layer0 = desc.range.base_array_layer;
    let nb_layers = desc.range.array_layer_count.unwrap_or(texture.layers().saturating_sub(layer0));
    let ms = texture.sample_count > 1;
    // A 1D/2D view set on a layer other than the first goes through the "array" form.
    let array = matches!(desc.dimension, VD::D1 | VD::D2) && (layer0 != 0 || nb_layers > 1);
    let array = array || desc.dimension == VD::D2Array;

    let mut size_value = texture.size.mip_level_size(mip0, texture.dimension);
    if texture.dimension != TD::D3 {
        size_value.depth_or_array_layers = nb_layers;
    }

    let depth_value = texture.format.is_depth_stencil_format();
    // Depth views take the typed format of the whole texture (both planes),
    // even when the resource is typeless.
    let format_target = if depth_value { conv::format_dsv(texture.format) } else { conv::format_dxgi(desc.format) };

    // --- read (SRV) ---
    let read_format = if depth_value { conv::format_srv(texture.format, desc.range.aspect) } else { conv::format_dxgi(desc.format) };
    // UNKNOWN: no SRV represents the requested aspect (conv::format_srv); no SRV.
    let srv = if usage.contains(U::RESOURCE) && read_format != dxgi::DXGI_FORMAT_UNKNOWN {
        let format = read_format;
        let mut d = d3d11::D3D11_SHADER_RESOURCE_VIEW_DESC { Format: format, ..Default::default() };
        match desc.dimension {
            VD::D1 if !array => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE1D;
                d.Anonymous.Texture1D = d3d11::D3D11_TEX1D_SRV { MostDetailedMip: mip0, MipLevels: nb_mips };
            }
            VD::D1 => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE1DARRAY;
                d.Anonymous.Texture1DArray =
                    d3d11::D3D11_TEX1D_ARRAY_SRV { MostDetailedMip: mip0, MipLevels: nb_mips, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            VD::D2 | VD::D2Array if ms && !array => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE2DMS;
            }
            VD::D2 | VD::D2Array if ms => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE2DMSARRAY;
                d.Anonymous.Texture2DMSArray = d3d11::D3D11_TEX2DMS_ARRAY_SRV { FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            VD::D2 | VD::D2Array if !array => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE2D;
                d.Anonymous.Texture2D = d3d11::D3D11_TEX2D_SRV { MostDetailedMip: mip0, MipLevels: nb_mips };
            }
            VD::D2 | VD::D2Array => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE2DARRAY;
                d.Anonymous.Texture2DArray =
                    d3d11::D3D11_TEX2D_ARRAY_SRV { MostDetailedMip: mip0, MipLevels: nb_mips, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            VD::Cube if layer0 == 0 => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURECUBE;
                d.Anonymous.TextureCube = d3d11::D3D11_TEXCUBE_SRV { MostDetailedMip: mip0, MipLevels: nb_mips };
            }
            // A cube that does not start at layer 0: only the "cube array" form allows it.
            VD::Cube | VD::CubeArray => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURECUBEARRAY;
                d.Anonymous.TextureCubeArray = d3d11::D3D11_TEXCUBE_ARRAY_SRV {
                    MostDetailedMip: mip0,
                    MipLevels: nb_mips,
                    First2DArrayFace: layer0,
                    NumCubes: nb_layers / 6,
                };
            }
            VD::D3 => {
                d.ViewDimension = Direct3D::D3D11_SRV_DIMENSION_TEXTURE3D;
                d.Anonymous.Texture3D = d3d11::D3D11_TEX3D_SRV { MostDetailedMip: mip0, MipLevels: nb_mips };
            }
        }
        let mut v = None;
        unsafe { device_value.CreateShaderResourceView(&texture.raw, Some(&d), Some(&mut v)) }
            .into_device_result("CreateShaderResourceView")?;
        v
    } else {
        None
    };

    // --- color target (RTV): a single mip ---
    let rtv = if usage.contains(U::COLOR_TARGET) {
        let mut d = d3d11::D3D11_RENDER_TARGET_VIEW_DESC { Format: format_target, ..Default::default() };
        match texture.dimension {
            TD::D1 if !array => {
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE1D;
                d.Anonymous.Texture1D = d3d11::D3D11_TEX1D_RTV { MipSlice: mip0 };
            }
            TD::D1 => {
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE1DARRAY;
                d.Anonymous.Texture1DArray = d3d11::D3D11_TEX1D_ARRAY_RTV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            TD::D2 if ms && !array => d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE2DMS,
            TD::D2 if ms => {
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE2DMSARRAY;
                d.Anonymous.Texture2DMSArray = d3d11::D3D11_TEX2DMS_ARRAY_RTV { FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            TD::D2 if !array => {
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE2D;
                d.Anonymous.Texture2D = d3d11::D3D11_TEX2D_RTV { MipSlice: mip0 };
            }
            TD::D2 => {
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE2DARRAY;
                d.Anonymous.Texture2DArray = d3d11::D3D11_TEX2D_ARRAY_RTV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            TD::D3 => {
                // All the slices of the mip (-1 = up to the end).
                d.ViewDimension = d3d11::D3D11_RTV_DIMENSION_TEXTURE3D;
                d.Anonymous.Texture3D = d3d11::D3D11_TEX3D_RTV { MipSlice: mip0, FirstWSlice: 0, WSize: u32::MAX };
            }
        }
        let mut v = None;
        unsafe { device_value.CreateRenderTargetView(&texture.raw, Some(&d), Some(&mut v)) }
            .into_device_result("CreateRenderTargetView")?;
        v
    } else {
        None
    };

    // --- depth / stencil (DSV, and read-only DSV) ---
    let (dsv, dsv_read) = if usage.intersects(U::DEPTH_STENCIL_READ | U::DEPTH_STENCIL_WRITE) && texture.dimension != TD::D3 {
        let mut d = d3d11::D3D11_DEPTH_STENCIL_VIEW_DESC { Format: format_target, ..Default::default() };
        match texture.dimension {
            TD::D1 if !array => {
                d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE1D;
                d.Anonymous.Texture1D = d3d11::D3D11_TEX1D_DSV { MipSlice: mip0 };
            }
            TD::D1 => {
                d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE1DARRAY;
                d.Anonymous.Texture1DArray = d3d11::D3D11_TEX1D_ARRAY_DSV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            _ if ms && !array => d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE2DMS,
            _ if ms => {
                d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE2DMSARRAY;
                d.Anonymous.Texture2DMSArray = d3d11::D3D11_TEX2DMS_ARRAY_DSV { FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            _ if !array => {
                d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE2D;
                d.Anonymous.Texture2D = d3d11::D3D11_TEX2D_DSV { MipSlice: mip0 };
            }
            _ => {
                d.ViewDimension = d3d11::D3D11_DSV_DIMENSION_TEXTURE2DARRAY;
                d.Anonymous.Texture2DArray = d3d11::D3D11_TEX2D_ARRAY_DSV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
        }
        let dsv = if usage.contains(U::DEPTH_STENCIL_WRITE) {
            let mut v = None;
            unsafe { device_value.CreateDepthStencilView(&texture.raw, Some(&d), Some(&mut v)) }
                .into_device_result("CreateDepthStencilView")?;
            v
        } else {
            None
        };
        let mut flag_bits = d3d11::D3D11_DSV_READ_ONLY_DEPTH.0;
        if texture.format.has_stencil_aspect() {
            flag_bits |= d3d11::D3D11_DSV_READ_ONLY_STENCIL.0;
        }
        d.Flags = flag_bits as u32;
        let mut v = None;
        unsafe { device_value.CreateDepthStencilView(&texture.raw, Some(&d), Some(&mut v)) }
            .into_device_result("CreateDepthStencilView (read only)")?;
        (dsv, v)
    } else {
        (None, None)
    };

    // --- write (UAV): a single mip, no multisampling ---
    let uav = if usage.intersects(U::STORAGE_READ_ONLY | U::STORAGE_WRITE_ONLY | U::STORAGE_READ_WRITE | U::STORAGE_ATOMIC) && !ms {
        let mut d = d3d11::D3D11_UNORDERED_ACCESS_VIEW_DESC { Format: format_target, ..Default::default() };
        match texture.dimension {
            TD::D1 if !array => {
                d.ViewDimension = d3d11::D3D11_UAV_DIMENSION_TEXTURE1D;
                d.Anonymous.Texture1D = d3d11::D3D11_TEX1D_UAV { MipSlice: mip0 };
            }
            TD::D1 => {
                d.ViewDimension = d3d11::D3D11_UAV_DIMENSION_TEXTURE1DARRAY;
                d.Anonymous.Texture1DArray = d3d11::D3D11_TEX1D_ARRAY_UAV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            TD::D2 if !array => {
                d.ViewDimension = d3d11::D3D11_UAV_DIMENSION_TEXTURE2D;
                d.Anonymous.Texture2D = d3d11::D3D11_TEX2D_UAV { MipSlice: mip0 };
            }
            TD::D2 => {
                d.ViewDimension = d3d11::D3D11_UAV_DIMENSION_TEXTURE2DARRAY;
                d.Anonymous.Texture2DArray = d3d11::D3D11_TEX2D_ARRAY_UAV { MipSlice: mip0, FirstArraySlice: layer0, ArraySize: nb_layers };
            }
            TD::D3 => {
                d.ViewDimension = d3d11::D3D11_UAV_DIMENSION_TEXTURE3D;
                d.Anonymous.Texture3D = d3d11::D3D11_TEX3D_UAV { MipSlice: mip0, FirstWSlice: 0, WSize: u32::MAX };
            }
        }
        let mut v = None;
        unsafe { device_value.CreateUnorderedAccessView(&texture.raw, Some(&d), Some(&mut v)) }
            .into_device_result("CreateUnorderedAccessView")?;
        v
    } else {
        None
    };

    Ok(TextureView {
        texture: texture.raw.clone(),
        format: desc.format,
        dimension: desc.dimension,
        mips: mip0..mip0 + nb_mips,
        layers: layer0..layer0 + nb_layers,
        nb_mips_texture: texture.mip_level_count,
        size_value,
        srv,
        rtv,
        dsv,
        dsv_read,
        uav,
    })
}

// ---------------------------------------------------------------------------------------------
// Samplers
// ---------------------------------------------------------------------------------------------

pub(super) unsafe fn create_sampler_impl(device: &Device, desc: &crate::SamplerDescriptor) -> DeviceResult<Sampler> {
    let color_edge = match desc.border_color {
        Some(wgt::SamplerBorderColor::OpaqueBlack) => [0.0, 0.0, 0.0, 1.0],
        Some(wgt::SamplerBorderColor::OpaqueWhite) => [1.0; 4],
        Some(wgt::SamplerBorderColor::TransparentBlack | wgt::SamplerBorderColor::Zero) | None => [0.0; 4],
    };
    let d = d3d11::D3D11_SAMPLER_DESC {
        Filter: conv::filter_value(desc),
        AddressU: conv::addressing(desc.address_modes[0]),
        AddressV: conv::addressing(desc.address_modes[1]),
        AddressW: conv::addressing(desc.address_modes[2]),
        MipLODBias: 0.0,
        MaxAnisotropy: (desc.anisotropy_clamp as u32).clamp(1, d3d11::D3D11_DEFAULT_MAX_ANISOTROPY),
        ComparisonFunc: desc.compare.map_or(d3d11::D3D11_COMPARISON_NEVER, conv::comparison),
        BorderColor: color_edge,
        MinLOD: desc.lod_clamp.start,
        MaxLOD: desc.lod_clamp.end,
    };
    let mut raw = None;
    unsafe { device.shared.device.CreateSamplerState(&d, Some(&mut raw)) }.into_device_result("CreateSamplerState")?;
    let raw = raw.ok_or(crate::DeviceError::Unexpected)?;
    unsafe { label_value(&raw, desc.label) };
    Ok(Sampler { raw })
}

// ---------------------------------------------------------------------------------------------
// Pool of staging buffers
// ---------------------------------------------------------------------------------------------

/// Size from which a staging buffer's memory is kept for the next one: a
/// `vec![0; n]` of this size is taken from the system as fresh pages (page faults when zeroing,
/// then given back to the system), on every write_buffer (character vertices: several MB
/// per frame). Smaller ones are already recycled by the process heap.
const RESERVE_MIN: usize = 64 << 10;
/// At most this number of kept memories, and this total of bytes.
const RESERVE_NB: usize = 64;
const RESERVE_BYTES: usize = 64 << 20;

static RESERVE: Mutex<alloc::vec::Vec<alloc::vec::Vec<u8>>> = parking_lot::const_mutex(alloc::vec::Vec::new());

fn reserve_active() -> bool {
    true
}

/// Memory of a staging buffer of `n` bytes, zeroed like `vec![0; n]`: taken from the
/// pool (the smallest one that suffices, at most 4 times too large) or new.
fn memory_pass(n: usize) -> alloc::vec::Vec<u8> {
    if n >= RESERVE_MIN && reserve_active() {
        let mut r = RESERVE.lock();
        let i = r.iter().enumerate().filter(|(_, v)| v.capacity() >= n && v.capacity() / 4 <= n).min_by_key(|(_, v)| v.capacity()).map(|(i, _)| i);
        if let Some(i) = i {
            let mut v = r.swap_remove(i);
            drop(r);
            v.clear();
            v.resize(n, 0);
            return v;
        }
    }
    vec![0; n]
}

/// Buffer dropped by wgpu-core (after its submission): a staging buffer's memory returns
/// to the pool if nothing holds it any more.
pub(super) fn return_buffer(buffer: Buffer) {
    let Buffer { raw: None, read_value: None, shadow: Some(o), .. } = buffer else { return };
    if !reserve_active() {
        return;
    }
    let Ok(m) = Arc::try_unwrap(o) else { return };
    let v = m.into_inner();
    if v.capacity() < RESERVE_MIN {
        return;
    }
    let mut r = RESERVE.lock();
    let total: usize = r.iter().map(|x| x.capacity()).sum();
    if r.len() < RESERVE_NB && total + v.capacity() <= RESERVE_BYTES {
        r.push(v);
    }
}
