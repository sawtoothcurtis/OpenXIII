//! Bind group layouts, bind groups, pipelines and state objects.
//!
//! - `create_layout`: each entry of each group receives a register per kind (b/t/s/u
//!   counters that advance group after group, same numbers in all stages):
//!   uniform -> Constants (b0..b12, b13 = special constants), texture and read-only storage ->
//!   ReadSlot (t#), sampler -> SamplerSlot (s#), writable storage and storage
//!   texture -> WriteSlot (u#). The same map is given to naga (`map_naga`).
//! - `create_group`: one `Bound` per entry of the group's layout, in the order of its entries.
//!   Storage buffer: raw SRV / UAV (ByteAddressBuffer) created here over the range.
//! - `create_pipeline_render`: VS (outputs adjusted to the FS inputs) and PS, vertex
//!   layout, state objects (blend, depth, rasterizer). No state object cache here:
//!   D3D11 already returns the same object for the same description, and pipelines are only created
//!   at load time.
//! - `create_pipeline_compute`: CS + bindings.

use alloc::{sync::Arc, vec::Vec};

use arrayvec::ArrayVec;
use windows::Win32::Graphics::{Direct3D, Direct3D11 as d3d11, Dxgi::Common as dxgi};

use super::{
    conv, shader, AccelerationStructure, BindGroup, BindGroupLayout, Buffer, ComputePipeline, ComputePipelineInternal,
    Device, DeviceResult, LayoutBindings, Slot, Bound, PipelineCache, PipelineLayout, RenderPipeline,
    RenderPipelineInternal, Sampler, ShaderModule, Kind, TextureView, REGISTRY_SPECIAL_CONSTANTS,
};
use crate::auxil::dxgi::result::HResult as _;

/// Number of registers available per kind (level 11_0): b0..b12 (b13 reserved), t0..t127,
/// s0..s15, u0..u7 (level 11_0 only has 8 UAVs, shared between pixel shaders and outputs).
fn limit(kind_index: Kind) -> u32 {
    match kind_index {
        Kind::Constants => REGISTRY_SPECIAL_CONSTANTS,
        Kind::ReadSlot => d3d11::D3D11_COMMONSHADER_INPUT_RESOURCE_SLOT_COUNT,
        Kind::SamplerSlot => d3d11::D3D11_COMMONSHADER_SAMPLER_SLOT_COUNT,
        Kind::WriteSlot => d3d11::D3D11_PS_CS_UAV_REGISTER_COUNT,
    }
}

/// Register kind of a layout entry (None: not supported by Direct3D 11).
fn kind_of_binding(ty: &wgt::BindingType) -> Option<Kind> {
    Some(match *ty {
        wgt::BindingType::Buffer { ty: wgt::BufferBindingType::Uniform, .. } => Kind::Constants,
        wgt::BindingType::Buffer { ty: wgt::BufferBindingType::Storage { read_only: true }, .. } => Kind::ReadSlot,
        wgt::BindingType::Buffer { ty: wgt::BufferBindingType::Storage { read_only: false }, .. } => Kind::WriteSlot,
        wgt::BindingType::Texture { .. } => Kind::ReadSlot,
        // naga writes any storage texture as RWTexture*, even when read-only.
        wgt::BindingType::StorageTexture { .. } => Kind::WriteSlot,
        wgt::BindingType::Sampler(_) => Kind::SamplerSlot,
        wgt::BindingType::AccelerationStructure { .. } | wgt::BindingType::ExternalTexture => return None,
    })
}

pub(super) unsafe fn create_layout(device: &Device, desc: &crate::PipelineLayoutDescriptor<BindGroupLayout>) -> DeviceResult<PipelineLayout> {
    if desc.immediate_size != 0 {
        log::error!("Direct3D 11: immediate constants not supported ({} bytes)", desc.immediate_size);
        return Err(crate::DeviceError::Unexpected);
    }

    let mut groups = Vec::with_capacity(desc.bind_group_layouts.len());
    let mut map_naga = naga::back::hlsl::BindingMap::default();
    // Next free register per kind: b, t, s, u.
    let (mut nb_b, mut nb_t, mut nb_s, mut nb_u) = (0u32, 0u32, 0u32, 0u32);

    for (g, bgl) in desc.bind_group_layouts.iter().enumerate() {
        let Some(bgl) = bgl else {
            groups.push(Vec::new());
            continue;
        };

        // Rank of the dynamic offsets: wgpu gives them by increasing binding number.
        let mut dynamic: Vec<u32> = bgl
            .inputs
            .iter()
            .filter(|e| matches!(e.ty, wgt::BindingType::Buffer { has_dynamic_offset: true, .. }))
            .map(|e| e.binding)
            .collect();
        dynamic.sort_unstable();

        let mut slots = Vec::with_capacity(bgl.inputs.len());
        for input_value in &bgl.inputs {
            let Some(kind_index) = kind_of_binding(&input_value.ty) else {
                log::error!("Direct3D 11: binding {:?} not supported (group {g}, binding {})", input_value.ty, input_value.binding);
                return Err(crate::DeviceError::Unexpected);
            };
            if input_value.count.is_some() {
                log::error!("Direct3D 11: binding arrays not supported (group {g}, binding {})", input_value.binding);
                return Err(crate::DeviceError::Unexpected);
            }
            let counter = match kind_index {
                Kind::Constants => &mut nb_b,
                Kind::ReadSlot => &mut nb_t,
                Kind::SamplerSlot => &mut nb_s,
                Kind::WriteSlot => &mut nb_u,
            };
            let registry = *counter;
            if registry >= limit(kind_index) {
                log::error!("Direct3D 11: no free {kind_index:?} registers left (group {g}, binding {})", input_value.binding);
                return Err(crate::DeviceError::Unexpected);
            }
            *counter += 1;

            let dynamic_rank = match input_value.ty {
                wgt::BindingType::Buffer { has_dynamic_offset: true, .. } => {
                    dynamic.iter().position(|&b| b == input_value.binding).map(|r| r as u32)
                }
                _ => None,
            };
            slots.push(Slot { binding: input_value.binding, kind_index, registry, stages_list: input_value.visibility, dynamic_rank });
            map_naga.insert(
                naga::ResourceBinding { group: g as u32, binding: input_value.binding },
                naga::back::hlsl::BindTarget { space: 0, register: registry, binding_array_size: None, ..Default::default() },
            );
        }
        groups.push(slots);
    }

    Ok(PipelineLayout {
        layout_value: Arc::new(LayoutBindings {
            groups,
            map_naga,
            nb_constants: nb_b,
            read_count: nb_t,
            nb_samplers: nb_s,
            nb_write: nb_u,
        }),
    })
}

/// D3D11 buffer of a binding (absent for a purely CPU buffer: not allowed here).
fn raw_buffer(buffer: &Buffer) -> DeviceResult<&d3d11::ID3D11Buffer> {
    buffer.raw.as_ref().ok_or_else(|| {
        log::error!("Direct3D 11: CPU buffer bound to a shader");
        crate::DeviceError::Unexpected
    })
}

pub(super) unsafe fn create_group(
    device: &Device,
    desc: &crate::BindGroupDescriptor<BindGroupLayout, Buffer, Sampler, TextureView, AccelerationStructure>,
) -> DeviceResult<BindGroup> {
    let device_value = &device.shared.device;
    let mut bound_list = Vec::with_capacity(desc.layout.inputs.len());

    for input_available in &desc.layout.inputs {
        let Some(input_value) = desc.entries.iter().find(|e| e.binding == input_available.binding) else {
            log::error!("Direct3D 11: binding {} missing from the group", input_available.binding);
            return Err(crate::DeviceError::Unexpected);
        };
        let hint = input_value.resource_index as usize;

        let bound_entry = match input_available.ty {
            wgt::BindingType::Buffer { ty, .. } => {
                let binding_value = &desc.buffers[hint];
                let buffer_value = raw_buffer(binding_value.buffer)?;
                let shift_amount = binding_value.offset;
                let size_value = match binding_value.size {
                    Some(t) => t.get(),
                    None => binding_value.buffer.size - shift_amount,
                };
                match ty {
                    wgt::BufferBindingType::Uniform => Bound::Constants { buffer_value: buffer_value.clone(), shift_amount, size_value },
                    // Raw views: 4-byte elements (wgpu already aligns storage offset and size to 4).
                    wgt::BufferBindingType::Storage { read_only: true } => {
                        let view_value = d3d11::D3D11_SHADER_RESOURCE_VIEW_DESC {
                            Format: dxgi::DXGI_FORMAT_R32_TYPELESS,
                            ViewDimension: Direct3D::D3D11_SRV_DIMENSION_BUFFEREX,
                            Anonymous: d3d11::D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                                BufferEx: d3d11::D3D11_BUFFEREX_SRV {
                                    FirstElement: (shift_amount / 4) as u32,
                                    NumElements: (size_value / 4) as u32,
                                    Flags: d3d11::D3D11_BUFFEREX_SRV_FLAG_RAW.0 as u32,
                                },
                            },
                        };
                        let mut srv = None;
                        unsafe { device_value.CreateShaderResourceView(buffer_value, Some(&view_value), Some(&mut srv)) }
                            .into_device_result("CreateShaderResourceView (storage buffer)")?;
                        Bound::ReadSlot(srv.ok_or(crate::DeviceError::Unexpected)?, windows::core::Interface::as_raw(buffer_value) as usize)
                    }
                    wgt::BufferBindingType::Storage { read_only: false } => {
                        let view_value = d3d11::D3D11_UNORDERED_ACCESS_VIEW_DESC {
                            Format: dxgi::DXGI_FORMAT_R32_TYPELESS,
                            ViewDimension: d3d11::D3D11_UAV_DIMENSION_BUFFER,
                            Anonymous: d3d11::D3D11_UNORDERED_ACCESS_VIEW_DESC_0 {
                                Buffer: d3d11::D3D11_BUFFER_UAV {
                                    FirstElement: (shift_amount / 4) as u32,
                                    NumElements: (size_value / 4) as u32,
                                    Flags: d3d11::D3D11_BUFFER_UAV_FLAG_RAW.0 as u32,
                                },
                            },
                        };
                        let mut uav = None;
                        unsafe { device_value.CreateUnorderedAccessView(buffer_value, Some(&view_value), Some(&mut uav)) }
                            .into_device_result("CreateUnorderedAccessView (storage buffer)")?;
                        Bound::WriteSlot(uav.ok_or(crate::DeviceError::Unexpected)?)
                    }
                }
            }
            wgt::BindingType::Texture { .. } => {
                let view_value = desc.textures[hint].view;
                Bound::ReadSlot(
                    view_value.srv.clone().ok_or_else(|| {
                        log::error!("Direct3D 11: texture view without SRV bound for reading");
                        crate::DeviceError::Unexpected
                    })?,
                    windows::core::Interface::as_raw(&view_value.texture) as usize,
                )
            }
            wgt::BindingType::StorageTexture { .. } => {
                let view_value = desc.textures[hint].view;
                Bound::WriteSlot(view_value.uav.clone().ok_or_else(|| {
                    log::error!("Direct3D 11: texture view without UAV bound for writing");
                    crate::DeviceError::Unexpected
                })?)
            }
            wgt::BindingType::Sampler(_) => Bound::SamplerSlot(desc.samplers[hint].raw.clone()),
            wgt::BindingType::AccelerationStructure { .. } | wgt::BindingType::ExternalTexture => {
                log::error!("Direct3D 11: binding {:?} not supported", input_available.ty);
                return Err(crate::DeviceError::Unexpected);
            }
        };
        bound_list.push(bound_entry);
    }

    Ok(BindGroup { bound_list: bound_list.into() })
}

/// Blend of a missing target: no blending, no writes (valid enumeration values,
/// D3D11 checks them even when blending is off).
fn empty_blend() -> d3d11::D3D11_RENDER_TARGET_BLEND_DESC1 {
    d3d11::D3D11_RENDER_TARGET_BLEND_DESC1 {
        BlendEnable: false.into(),
        LogicOpEnable: false.into(),
        SrcBlend: d3d11::D3D11_BLEND_ONE,
        DestBlend: d3d11::D3D11_BLEND_ZERO,
        BlendOp: d3d11::D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: d3d11::D3D11_BLEND_ONE,
        DestBlendAlpha: d3d11::D3D11_BLEND_ZERO,
        BlendOpAlpha: d3d11::D3D11_BLEND_OP_ADD,
        LogicOp: d3d11::D3D11_LOGIC_OP_NOOP,
        RenderTargetWriteMask: 0,
    }
}

/// Depth and stencil off (pipeline without a depth attachment).
fn no_depth() -> d3d11::D3D11_DEPTH_STENCIL_DESC {
    let face = d3d11::D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: d3d11::D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: d3d11::D3D11_STENCIL_OP_KEEP,
        StencilPassOp: d3d11::D3D11_STENCIL_OP_KEEP,
        StencilFunc: d3d11::D3D11_COMPARISON_ALWAYS,
    };
    d3d11::D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: false.into(),
        DepthWriteMask: d3d11::D3D11_DEPTH_WRITE_MASK_ZERO,
        DepthFunc: d3d11::D3D11_COMPARISON_ALWAYS,
        StencilEnable: false.into(),
        StencilReadMask: 0xff,
        StencilWriteMask: 0xff,
        FrontFace: face,
        BackFace: face,
    }
}

pub(super) unsafe fn create_pipeline_render(
    device: &Device,
    desc: &crate::RenderPipelineDescriptor<PipelineLayout, ShaderModule, PipelineCache>,
) -> Result<RenderPipeline, crate::PipelineError> {
    let start_creation = std::time::Instant::now();
    let device_value = &device.shared.device;
    let bindings_list = &desc.layout.layout_value;

    let (buffers_vertices, step_vertices) = match desc.vertex_processor {
        crate::VertexProcessor::Standard { vertex_buffers, ref vertex_stage } => (vertex_buffers, vertex_stage),
        crate::VertexProcessor::Mesh { .. } => {
            return Err(crate::PipelineError::Linkage(
                wgt::ShaderStages::MESH,
                alloc::string::String::from("Direct3D 11: no mesh shaders"),
            ))
        }
    };

    // Shaders: the VS outputs are adjusted to the FS inputs (aligned registers).
    let vs_compile = shader::compile_vertices(device, step_vertices, desc.fragment_stage.as_ref(), bindings_list)?;
    // Shader objects reused if they already exist for this bytecode (creation is expensive on AMD).
    let key_vs = super::footprint(&[&vs_compile.bytecode]);
    let vs = {
        let mut objects_list = device.shared.objects_shaders.lock();
        if let Some(v) = objects_list.vs.get(&key_vs).cloned() {
            objects_list.resumed += 1;
            v
        } else {
            let mut vs = None;
            unsafe { device_value.CreateVertexShader(&vs_compile.bytecode, None::<&d3d11::ID3D11ClassLinkage>, Some(&mut vs)) }
                .into_device_result("CreateVertexShader")?;
            let vs = vs.ok_or(crate::DeviceError::Unexpected)?;
            objects_list.created += 1;
            objects_list.vs.insert(key_vs, vs.clone());
            vs
        }
    };

    let ps = match desc.fragment_stage {
        Some(ref step_value) => {
            let compile = shader::compile_stage(device, step_value, naga::ShaderStage::Fragment, bindings_list)?;
            let key_tuple = super::footprint(&[&compile.bytecode]);
            let mut objects_list = device.shared.objects_shaders.lock();
            if let Some(p) = objects_list.ps.get(&key_tuple).cloned() {
                objects_list.resumed += 1;
                Some(p)
            } else {
                let mut ps = None;
                unsafe { device_value.CreatePixelShader(&compile.bytecode, None::<&d3d11::ID3D11ClassLinkage>, Some(&mut ps)) }
                    .into_device_result("CreatePixelShader")?;
                let ps = ps.ok_or(crate::DeviceError::Unexpected)?;
                objects_list.created += 1;
                objects_list.ps.insert(key_tuple, ps.clone());
                Some(ps)
            }
        }
        None => None,
    };

    // Vertex layout: one entry per attribute (LOC semantic, index = location).
    let mut vertex_step = ArrayVec::<u32, { crate::MAX_VERTEX_BUFFERS }>::new();
    let mut elements = Vec::new();
    for (slot, buffer_value) in buffers_vertices.iter().enumerate() {
        let Some(buffer_value) = buffer_value else {
            vertex_step.push(0);
            continue;
        };
        vertex_step.push(buffer_value.array_stride as u32);
        let (class_value, instance_step) = match buffer_value.step_mode {
            wgt::VertexStepMode::Vertex => (d3d11::D3D11_INPUT_PER_VERTEX_DATA, 0),
            wgt::VertexStepMode::Instance => (d3d11::D3D11_INPUT_PER_INSTANCE_DATA, 1),
        };
        for attribute in buffer_value.attributes {
            elements.push(d3d11::D3D11_INPUT_ELEMENT_DESC {
                SemanticName: windows::core::PCSTR(shader::SEMANTIC_VERTICES.as_ptr().cast()),
                SemanticIndex: attribute.shader_location,
                Format: conv::format_vertex(attribute.format),
                InputSlot: slot as u32,
                AlignedByteOffset: attribute.offset as u32,
                InputSlotClass: class_value,
                InstanceDataStepRate: instance_step,
            });
        }
    }
    let layout_vertices = if elements.is_empty() {
        None
    } else {
        // Key: VS signature + description of the elements (names, formats, offsets).
        let descr = alloc::format!(
            "{:?}",
            elements.iter().map(|e| (e.SemanticIndex, e.Format.0, e.InputSlot, e.AlignedByteOffset, e.InputSlotClass.0, e.InstanceDataStepRate)).collect::<Vec<_>>()
        );
        let key_tuple = super::footprint(&[&vs_compile.bytecode, descr.as_bytes()]);
        let mut objects_list = device.shared.objects_shaders.lock();
        if let Some(d) = objects_list.layouts.get(&key_tuple).cloned() {
            Some(d)
        } else {
            let mut available = None;
            unsafe { device_value.CreateInputLayout(&elements, &vs_compile.bytecode, Some(&mut available)) }
                .into_device_result("CreateInputLayout")?;
            let d = available.ok_or(crate::DeviceError::Unexpected)?;
            objects_list.layouts.insert(key_tuple, d.clone());
            Some(d)
        }
    };

    // Blend: one description per color target.
    let mut desc_blend = d3d11::D3D11_BLEND_DESC1 {
        AlphaToCoverageEnable: desc.multisample.alpha_to_coverage_enabled.into(),
        IndependentBlendEnable: true.into(),
        RenderTarget: [empty_blend(); 8],
    };
    for (i, target_value) in desc.color_targets.iter().enumerate().take(8) {
        if let Some(target_value) = target_value {
            desc_blend.RenderTarget[i] = conv::blend_target(target_value);
        }
    }
    let mut blend_value = None;
    unsafe { device_value.CreateBlendState1(&desc_blend, Some(&mut blend_value)) }.into_device_result("CreateBlendState1")?;
    let blend_value: d3d11::ID3D11BlendState = blend_value.ok_or(crate::DeviceError::Unexpected)?.into();

    let desc_depth = match desc.depth_stencil {
        Some(ref ds) => conv::depth_stencil_test(ds),
        None => no_depth(),
    };
    let mut depth_value = None;
    unsafe { device_value.CreateDepthStencilState(&desc_depth, Some(&mut depth_value)) }
        .into_device_result("CreateDepthStencilState")?;
    let depth_value = depth_value.ok_or(crate::DeviceError::Unexpected)?;

    // Constant bias on a floating-point depth: D3D11 uses a unit of 2^(e-23) with e
    // the exponent of the triangle's largest depth, OpenGL (NVIDIA, wgpu gles) twice that
    // at any depth: doubled here to get the same images (shadows of the
    // BSP projectors, SetZBias 8).
    let bias_list = desc.depth_stencil.as_ref().map(|ds| {
        let mut b = ds.bias;
        if matches!(ds.format, wgt::TextureFormat::Depth32Float | wgt::TextureFormat::Depth32FloatStencil8) {
            b.constant = b.constant.saturating_mul(2);
        }
        b
    });
    let desc_frame = conv::frame(&desc.primitive, bias_list.as_ref(), desc.multisample.count);
    let mut frame = None;
    unsafe { device_value.CreateRasterizerState1(&desc_frame, Some(&mut frame)) }.into_device_result("CreateRasterizerState1")?;
    let frame: d3d11::ID3D11RasterizerState = frame.ok_or(crate::DeviceError::Unexpected)?.into();

    {
        let o = device.shared.objects_shaders.lock();
        log::info!("Direct3D 11: pipeline {:?} ({:.1} ms); shader objects created {} / reused {}", desc.label, start_creation.elapsed().as_secs_f64() * 1000.0, o.created, o.resumed);
    }
    super::timer_creation("pipeline", desc.label, start_creation);
    Ok(RenderPipeline {
        internal: Arc::new(RenderPipelineInternal {
            vs,
            ps,
            layout_vertices,
            vertex_step,
            topology_value: conv::topology_value(desc.primitive.topology),
            blend_value,
            depth_value,
            frame,
            // 64-bit mask in wgpu, 32 bits in D3D11 (at most 32 samples).
            mask_samples: desc.multisample.mask as u32,
            bindings_list: Arc::clone(bindings_list),
            special_constants: vs_compile.special_constants,
        }),
    })
}

pub(super) unsafe fn create_pipeline_compute(
    device: &Device,
    desc: &crate::ComputePipelineDescriptor<PipelineLayout, ShaderModule, PipelineCache>,
) -> Result<ComputePipeline, crate::PipelineError> {
    let bindings_list = &desc.layout.layout_value;
    let compile = shader::compile_stage(device, &desc.stage, naga::ShaderStage::Compute, bindings_list)?;
    let mut cs = None;
    unsafe { device.shared.device.CreateComputeShader(&compile.bytecode, None::<&d3d11::ID3D11ClassLinkage>, Some(&mut cs)) }
        .into_device_result("CreateComputeShader")?;
    Ok(ComputePipeline {
        internal: Arc::new(ComputePipelineInternal {
            cs: cs.ok_or(crate::DeviceError::Unexpected)?,
            bindings_list: Arc::clone(bindings_list),
            special_constants: compile.special_constants,
        }),
    })
}
