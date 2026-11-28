//! Conversions wgpu -> DXGI / D3D11, without calls to the GPU.

use crate::auxil::dxgi::conv as formats;
use windows::Win32::Graphics::{
    Direct3D::{self as d3d, D3D_PRIMITIVE_TOPOLOGY},
    Direct3D11 as d3d11,
    Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_UNKNOWN},
};

/// Typed DXGI format of a texture or a view.
pub(super) fn format_dxgi(format: wgt::TextureFormat) -> DXGI_FORMAT {
    formats::map_texture_format(format)
}

/// Resource format that allows creating the compatible views (linear/sRGB or depth/stencil).
pub(super) fn format_typeless(format: wgt::TextureFormat) -> DXGI_FORMAT {
    formats::map_texture_format_for_resource(format, wgt::TextureUses::RESOURCE, !format.is_depth_stencil_format(), false)
}

/// Format of the read view. For depth/stencil, pass the texture's format.
/// UNKNOWN means that no SRV can represent the requested aspect.
pub(super) fn format_srv(format: wgt::TextureFormat, aspect: wgt::TextureAspect) -> DXGI_FORMAT {
    use wgt::{TextureAspect as A, TextureFormat as F};

    // wgpu-core may already have resolved the plane's format in the view descriptor.
    match (format, aspect) {
        (F::R8Unorm | F::R16Unorm, A::Plane0) | (F::Rg8Unorm | F::Rg16Unorm, A::Plane1) => {
            return format_dxgi(format);
        }
        _ => {}
    }
    let aspects = crate::FormatAspects::new(format, aspect);
    if !aspects.is_one() {
        return DXGI_FORMAT_UNKNOWN;
    }
    let view_value = if format.is_multi_planar_format() {
        format.aspect_specific_format(aspect).map(format_dxgi)
    } else {
        formats::map_texture_format_for_srv_uav(format, aspects)
    };
    view_value.unwrap_or(DXGI_FORMAT_UNKNOWN)
}

/// Typed format of the depth/stencil view, including when the resource is typeless.
pub(super) fn format_dsv(format: wgt::TextureFormat) -> DXGI_FORMAT {
    if format.is_depth_stencil_format() {
        format_dxgi(format)
    } else {
        DXGI_FORMAT_UNKNOWN
    }
}

pub(super) fn format_vertex(format: wgt::VertexFormat) -> DXGI_FORMAT {
    formats::map_vertex_format(format)
}

pub(super) fn format_index(format: wgt::IndexFormat) -> DXGI_FORMAT {
    formats::map_index_format(format)
}

pub(super) fn topology_value(t: wgt::PrimitiveTopology) -> D3D_PRIMITIVE_TOPOLOGY {
    use wgt::PrimitiveTopology as T;
    match t {
        T::PointList => d3d::D3D_PRIMITIVE_TOPOLOGY_POINTLIST,
        T::LineList => d3d::D3D_PRIMITIVE_TOPOLOGY_LINELIST,
        T::LineStrip => d3d::D3D_PRIMITIVE_TOPOLOGY_LINESTRIP,
        T::TriangleList => d3d::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
        T::TriangleStrip => d3d::D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    }
}

pub(super) fn comparison(f: wgt::CompareFunction) -> d3d11::D3D11_COMPARISON_FUNC {
    use wgt::CompareFunction as C;
    match f {
        C::Never => d3d11::D3D11_COMPARISON_NEVER,
        C::Less => d3d11::D3D11_COMPARISON_LESS,
        C::Equal => d3d11::D3D11_COMPARISON_EQUAL,
        C::LessEqual => d3d11::D3D11_COMPARISON_LESS_EQUAL,
        C::Greater => d3d11::D3D11_COMPARISON_GREATER,
        C::NotEqual => d3d11::D3D11_COMPARISON_NOT_EQUAL,
        C::GreaterEqual => d3d11::D3D11_COMPARISON_GREATER_EQUAL,
        C::Always => d3d11::D3D11_COMPARISON_ALWAYS,
    }
}

pub(super) fn factor_blend(f: wgt::BlendFactor, alpha: bool) -> d3d11::D3D11_BLEND {
    use wgt::BlendFactor as F;
    match f {
        F::Zero => d3d11::D3D11_BLEND_ZERO,
        F::One => d3d11::D3D11_BLEND_ONE,
        F::Src if !alpha => d3d11::D3D11_BLEND_SRC_COLOR,
        F::OneMinusSrc if !alpha => d3d11::D3D11_BLEND_INV_SRC_COLOR,
        F::Dst if !alpha => d3d11::D3D11_BLEND_DEST_COLOR,
        F::OneMinusDst if !alpha => d3d11::D3D11_BLEND_INV_DEST_COLOR,
        F::Src | F::SrcAlpha => d3d11::D3D11_BLEND_SRC_ALPHA,
        F::OneMinusSrc | F::OneMinusSrcAlpha => d3d11::D3D11_BLEND_INV_SRC_ALPHA,
        F::Dst | F::DstAlpha => d3d11::D3D11_BLEND_DEST_ALPHA,
        F::OneMinusDst | F::OneMinusDstAlpha => d3d11::D3D11_BLEND_INV_DEST_ALPHA,
        F::SrcAlphaSaturated => d3d11::D3D11_BLEND_SRC_ALPHA_SAT,
        F::Constant => d3d11::D3D11_BLEND_BLEND_FACTOR,
        F::OneMinusConstant => d3d11::D3D11_BLEND_INV_BLEND_FACTOR,
        F::Src1 if !alpha => d3d11::D3D11_BLEND_SRC1_COLOR,
        F::OneMinusSrc1 if !alpha => d3d11::D3D11_BLEND_INV_SRC1_COLOR,
        F::Src1 | F::Src1Alpha => d3d11::D3D11_BLEND_SRC1_ALPHA,
        F::OneMinusSrc1 | F::OneMinusSrc1Alpha => d3d11::D3D11_BLEND_INV_SRC1_ALPHA,
    }
}

pub(super) fn operation_blend(o: wgt::BlendOperation) -> d3d11::D3D11_BLEND_OP {
    use wgt::BlendOperation as O;
    match o {
        O::Add => d3d11::D3D11_BLEND_OP_ADD,
        O::Subtract => d3d11::D3D11_BLEND_OP_SUBTRACT,
        O::ReverseSubtract => d3d11::D3D11_BLEND_OP_REV_SUBTRACT,
        O::Min => d3d11::D3D11_BLEND_OP_MIN,
        O::Max => d3d11::D3D11_BLEND_OP_MAX,
    }
}

pub(super) fn blend_target(c: &wgt::ColorTargetState) -> d3d11::D3D11_RENDER_TARGET_BLEND_DESC1 {
    let blend = c.blend.unwrap_or(wgt::BlendState::REPLACE);
    d3d11::D3D11_RENDER_TARGET_BLEND_DESC1 {
        BlendEnable: c.blend.is_some().into(),
        LogicOpEnable: false.into(),
        SrcBlend: factor_blend(blend.color.src_factor, false),
        DestBlend: factor_blend(blend.color.dst_factor, false),
        BlendOp: operation_blend(blend.color.operation),
        SrcBlendAlpha: factor_blend(blend.alpha.src_factor, true),
        DestBlendAlpha: factor_blend(blend.alpha.dst_factor, true),
        BlendOpAlpha: operation_blend(blend.alpha.operation),
        LogicOp: d3d11::D3D11_LOGIC_OP_NOOP,
        RenderTargetWriteMask: c.write_mask.bits() as u8,
    }
}

pub(super) fn operation_stencil(o: wgt::StencilOperation) -> d3d11::D3D11_STENCIL_OP {
    use wgt::StencilOperation as O;
    match o {
        O::Keep => d3d11::D3D11_STENCIL_OP_KEEP,
        O::Zero => d3d11::D3D11_STENCIL_OP_ZERO,
        O::Replace => d3d11::D3D11_STENCIL_OP_REPLACE,
        O::Invert => d3d11::D3D11_STENCIL_OP_INVERT,
        O::IncrementClamp => d3d11::D3D11_STENCIL_OP_INCR_SAT,
        O::DecrementClamp => d3d11::D3D11_STENCIL_OP_DECR_SAT,
        O::IncrementWrap => d3d11::D3D11_STENCIL_OP_INCR,
        O::DecrementWrap => d3d11::D3D11_STENCIL_OP_DECR,
    }
}

fn face_stencil(face: &wgt::StencilFaceState) -> d3d11::D3D11_DEPTH_STENCILOP_DESC {
    d3d11::D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: operation_stencil(face.fail_op),
        StencilDepthFailOp: operation_stencil(face.depth_fail_op),
        StencilPassOp: operation_stencil(face.pass_op),
        StencilFunc: comparison(face.compare),
    }
}

pub(super) fn depth_stencil_test(d: &wgt::DepthStencilState) -> d3d11::D3D11_DEPTH_STENCIL_DESC {
    d3d11::D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: d.is_depth_enabled().into(),
        DepthWriteMask: if d.depth_write_enabled.unwrap_or_default() { d3d11::D3D11_DEPTH_WRITE_MASK_ALL } else { d3d11::D3D11_DEPTH_WRITE_MASK_ZERO },
        DepthFunc: comparison(d.depth_compare.unwrap_or_default()),
        StencilEnable: d.stencil.is_enabled().into(),
        StencilReadMask: d.stencil.read_mask as u8,
        StencilWriteMask: d.stencil.write_mask as u8,
        FrontFace: face_stencil(&d.stencil.front),
        BackFace: face_stencil(&d.stencil.back),
    }
}

pub(super) fn frame(p: &wgt::PrimitiveState, bias_list: Option<&wgt::DepthBiasState>, samples: u32) -> d3d11::D3D11_RASTERIZER_DESC1 {
    let bias_list = bias_list.copied().unwrap_or_default();
    d3d11::D3D11_RASTERIZER_DESC1 {
        FillMode: match p.polygon_mode {
            wgt::PolygonMode::Fill => d3d11::D3D11_FILL_SOLID,
            wgt::PolygonMode::Line => d3d11::D3D11_FILL_WIREFRAME,
            wgt::PolygonMode::Point => unreachable!("D3D11 does not support POLYGON_MODE_POINT"),
        },
        CullMode: match p.cull_mode {
            Some(wgt::Face::Front) => d3d11::D3D11_CULL_FRONT,
            Some(wgt::Face::Back) => d3d11::D3D11_CULL_BACK,
            None => d3d11::D3D11_CULL_NONE,
        },
        FrontCounterClockwise: (p.front_face == wgt::FrontFace::Ccw).into(),
        DepthBias: bias_list.constant,
        DepthBiasClamp: bias_list.clamp,
        SlopeScaledDepthBias: bias_list.slope_scale,
        DepthClipEnable: (!p.unclipped_depth).into(),
        ScissorEnable: true.into(),
        MultisampleEnable: (samples > 1).into(),
        AntialiasedLineEnable: false.into(),
        ForcedSampleCount: 0,
    }
}

pub(super) fn filter_value(desc: &crate::SamplerDescriptor) -> d3d11::D3D11_FILTER {
    if desc.anisotropy_clamp > 1 {
        return if desc.compare.is_some() { d3d11::D3D11_FILTER_COMPARISON_ANISOTROPIC } else { d3d11::D3D11_FILTER_ANISOTROPIC };
    }

    // Same encoding as D3D11_ENCODE_BASIC_FILTER in the Windows SDK.
    let min = i32::from(desc.min_filter == wgt::FilterMode::Linear);
    let mag = i32::from(desc.mag_filter == wgt::FilterMode::Linear);
    let mip = i32::from(desc.mipmap_filter == wgt::MipmapFilterMode::Linear);
    let comparison = i32::from(desc.compare.is_some());
    d3d11::D3D11_FILTER(
        (min << d3d11::D3D11_MIN_FILTER_SHIFT)
            | (mag << d3d11::D3D11_MAG_FILTER_SHIFT)
            | (mip << d3d11::D3D11_MIP_FILTER_SHIFT)
            | (comparison << d3d11::D3D11_FILTER_REDUCTION_TYPE_SHIFT),
    )
}

pub(super) fn addressing(a: wgt::AddressMode) -> d3d11::D3D11_TEXTURE_ADDRESS_MODE {
    match a {
        wgt::AddressMode::ClampToEdge => d3d11::D3D11_TEXTURE_ADDRESS_CLAMP,
        wgt::AddressMode::Repeat => d3d11::D3D11_TEXTURE_ADDRESS_WRAP,
        wgt::AddressMode::MirrorRepeat => d3d11::D3D11_TEXTURE_ADDRESS_MIRROR,
        wgt::AddressMode::ClampToBorder => d3d11::D3D11_TEXTURE_ADDRESS_BORDER,
    }
}
