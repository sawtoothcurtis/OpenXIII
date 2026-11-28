//! Direct3D 11 for OpenXIII, in place of wgpu's DirectX 12 backend (`dx11` feature).
//!
//! wgpu-core sees `hal::api::Dx12`; this module is what answers. Nothing else in wgpu changes.
//!
//! Layout (one file per part):
//! - `conv.rs`        conversions wgpu -> DXGI / D3D11
//! - `instance.rs`    DXGI factory, list of adapters, surface creation
//! - `adapter.rs`     device opening, capabilities, limits, formats
//! - `resources.rs`  buffers (+ CPU copy), textures, views, samplers
//! - `shader.rs`      naga -> HLSL (SM 5.0) -> FXC, registers
//! - `pipelines.rs`   bind group layouts, bind groups, pipelines, state objects
//! - `command.rs`     recording of commands (`Command` list)
//! - `queue.rs`       execution of the list on the immediate context, state cache
//! - `sync.rs`        end fences (event queries), waiting, queries
//! - `surface.rs`     DXGI swap chain, acquisition, presentation
//! - `device.rs`      implementation of the `Device` trait, delegating to the files above
//! - `dxgi_lib.rs`    loading of dxgi.dll, used by `auxil::dxgi::factory`
//!
//! Execution model: like the GL backend. `CommandEncoder` fills a `Vec<Command>`;
//! `Queue::submit` replays it on the immediate context (`Shared::context`, under a lock), the only
//! place that talks to the GPU during a frame. No deferred contexts (slow on
//! old drivers).
//!
//! Common rules:
//! - Level 11_0 minimum, and constant buffer offsetting (D3D 11.1 `VSSetConstantBuffers1`) required:
//!   an adapter without it is not offered (OpenXIII then falls back to OpenGL).
//! - Bindings: each bind group entry receives a fixed register per kind (b#, t#, s#, u#), the same
//!   in all stages, assigned by `pipelines::create_layout` and described by
//!   `LayoutBindings`; `shader.rs` gives the same map to naga.
//! - COM objects are kept as they are (clones = AddRef); all types are declared
//!   Send + Sync by hand, like the DX12 backend: access to the context goes through the lock.

#![allow(dead_code, unused_variables)]

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{ops::Range, sync::atomic::AtomicU64};

use arrayvec::ArrayVec;
use parking_lot::{Mutex, RwLock};
use windows::Win32::{
    Foundation::HWND,
    Graphics::{Direct3D, Direct3D11 as d3d11, Dxgi},
};

use crate::auxil::dxgi::factory::DxgiFactory;

mod adapter;
mod command;
mod conv;
mod device;
mod dxgi_lib;
mod instance;
mod pipelines;
mod queue;
mod resources;
mod shader;
mod surface;
mod sync;

pub use command::Command;
pub(crate) use dxgi_lib::DxgiLib;

// ---------------------------------------------------------------------------------------------
// Names expected by the rest of wgpu-hal under `cfg(dx12)` (lib.rs: D3D12 telemetry).
// ---------------------------------------------------------------------------------------------

/// Device creation error (wgpu telemetry; not used by OpenXIII).
#[derive(Debug)]
pub enum CreateDeviceError {
    /// D3D11CreateDevice failed.
    Hresult(windows_core::HRESULT),
}
/// Direct3D feature level (telemetry).
pub type FeatureLevel = Direct3D::D3D_FEATURE_LEVEL;
/// Shader model (telemetry): always 5.0 here.
#[derive(Debug, Clone, Copy)]
pub struct ShaderModel;

// ---------------------------------------------------------------------------------------------
// Api
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Api;

impl crate::Api for Api {
    const VARIANT: wgt::Backend = wgt::Backend::Dx12;

    type Instance = Instance;
    type Surface = Surface;
    type Adapter = Adapter;
    type Device = Device;

    type Queue = Queue;
    type CommandEncoder = CommandEncoder;
    type CommandBuffer = CommandBuffer;

    type Buffer = Buffer;
    type Texture = Texture;
    type SurfaceTexture = SurfaceTexture;
    type TextureView = TextureView;
    type Sampler = Sampler;
    type QuerySet = QuerySet;
    type Fence = Fence;

    type BindGroupLayout = BindGroupLayout;
    type BindGroup = BindGroup;
    type PipelineLayout = PipelineLayout;
    type ShaderModule = ShaderModule;
    type RenderPipeline = RenderPipeline;
    type ComputePipeline = ComputePipeline;
    type RayTracingPipeline = RayTracingPipeline;
    type PipelineCache = PipelineCache;

    type AccelerationStructure = AccelerationStructure;
}

crate::impl_dyn_resource!(
    Adapter,
    AccelerationStructure,
    BindGroup,
    BindGroupLayout,
    Buffer,
    CommandBuffer,
    CommandEncoder,
    ComputePipeline,
    Device,
    Fence,
    Instance,
    PipelineCache,
    PipelineLayout,
    QuerySet,
    Queue,
    RayTracingPipeline,
    RenderPipeline,
    Sampler,
    ShaderModule,
    Surface,
    SurfaceTexture,
    Texture,
    TextureView
);

impl crate::DynAccelerationStructure for AccelerationStructure {}
impl crate::DynBindGroup for BindGroup {}
impl crate::DynBindGroupLayout for BindGroupLayout {}
impl crate::DynBuffer for Buffer {}
impl crate::DynCommandBuffer for CommandBuffer {}
impl crate::DynComputePipeline for ComputePipeline {}
impl crate::DynFence for Fence {}
impl crate::DynPipelineCache for PipelineCache {}
impl crate::DynPipelineLayout for PipelineLayout {}
impl crate::DynQuerySet for QuerySet {}
impl crate::DynRenderPipeline for RenderPipeline {}
impl crate::DynRayTracingPipeline for RayTracingPipeline {}
impl crate::DynSampler for Sampler {}
impl crate::DynShaderModule for ShaderModule {}
impl crate::DynSurfaceTexture for SurfaceTexture {}
impl crate::DynTexture for Texture {}
impl crate::DynTextureView for TextureView {}

impl core::borrow::Borrow<Texture> for SurfaceTexture {
    fn borrow(&self) -> &Texture {
        &self.texture
    }
}
impl core::borrow::Borrow<dyn crate::DynTexture> for SurfaceTexture {
    fn borrow(&self) -> &dyn crate::DynTexture {
        &self.texture
    }
}

/// Declares Send + Sync by hand (D3D11 COM objects; access to the context under a lock).
macro_rules! sendable {
    ($($t:ty),* $(,)?) => { $( unsafe impl Send for $t {} unsafe impl Sync for $t {} )* };
}
sendable!(
    Instance, Surface, Adapter, Shared, Device, Queue, CommandEncoder, CommandBuffer, Buffer, Texture,
    SurfaceTexture, TextureView, Sampler, QuerySet, Fence, BindGroupLayout, BindGroup, PipelineLayout,
    ShaderModule, RenderPipeline, ComputePipeline, RayTracingPipeline, PipelineCache, AccelerationStructure,
);

type DeviceResult<T> = Result<T, crate::DeviceError>;

// ---------------------------------------------------------------------------------------------
// Instance, surface, adapter (instance.rs, adapter.rs, surface.rs)
// ---------------------------------------------------------------------------------------------

pub struct Instance {
    pub(super) factory: DxgiFactory,
    pub(super) _lib_dxgi: DxgiLib,
    pub(super) flags: wgt::InstanceFlags,
}

/// Window (HWND) and its swap chain, created by `Surface::configure` (surface.rs).
pub struct Surface {
    pub(super) factory: DxgiFactory,
    pub(super) hwnd: HWND,
    pub(super) swap_chain: RwLock<Option<SwapChain>>,
}

/// Configured swap chain (surface.rs).
pub struct SwapChain {
    pub(super) raw: Dxgi::IDXGISwapChain,
    /// Back buffer (a single image in DISCARD mode; rebuilt after ResizeBuffers).
    pub(super) back_buffer: d3d11::ID3D11Texture2D,
    pub(super) format: wgt::TextureFormat,
    pub(super) size: wgt::Extent3d,
    /// Sync interval passed to Present (0 = no vsync).
    pub(super) sync_interval: u32,
    pub(super) present_flags: Dxgi::DXGI_PRESENT,
}

pub struct Adapter {
    pub(super) raw: Dxgi::IDXGIAdapter1,
    pub(super) feature_level: Direct3D::D3D_FEATURE_LEVEL,
    pub(super) info: wgt::AdapterInfo,
    pub(super) flags: wgt::InstanceFlags,
}

/// What the device and the queue share: the device (free-threaded) and the
/// immediate context (one thread at a time).
pub struct Shared {
    pub(super) device: d3d11::ID3D11Device1,
    pub(super) context: Mutex<d3d11::ID3D11DeviceContext1>,
    pub(super) feature_level: Direct3D::D3D_FEATURE_LEVEL,
    /// Zero buffer reused by clear_buffer (UpdateSubresource1), size ZERO_BUFFER_SIZE.
    pub(super) zeros: Vec<u8>,
    /// Staging buffer for writes into constant buffers (see queue::write_buffer):
    /// (DEFAULT buffer without binding, size).
    pub(super) pass_constants: Mutex<Option<(d3d11::ID3D11Buffer, u64)>>,
    /// Changes when a target is removed from the context outside the queue (surface.rs): the targets
    /// set that the queue believes are in place must be set again.
    pub(super) targets_generation: core::sync::atomic::AtomicU64,
    /// Shader objects and input layouts already created, by bytecode hash: creation
    /// by the driver costs 70 to 160 ms on an older GPU and many
    /// pipelines only differ by their states.
    pub(super) objects_shaders: Mutex<ObjectsShaders>,
    /// Address (ID3D11Resource) of the configured swap chain's back buffer, 0 without a swap chain.
    pub(super) buffer_back: core::sync::atomic::AtomicUsize,
    /// Emptied before any ResizeBuffers or swap chain change (`surface::unbind_targets`), which requires
    /// that no reference to the back buffer remains.
    pub(super) views_back: Mutex<Vec<(alloc::string::String, TextureView)>>,
}

/// Cache of the device's shader objects (see `Shared::objects_shaders`).
#[derive(Default)]
pub struct ObjectsShaders {
    pub vs: hashbrown::HashMap<u64, d3d11::ID3D11VertexShader>,
    pub ps: hashbrown::HashMap<u64, d3d11::ID3D11PixelShader>,
    pub layouts: hashbrown::HashMap<u64, d3d11::ID3D11InputLayout>,
    pub created: u32,
    pub resumed: u32,
}

/// 64-bit FNV-1a hash (keys of the object caches).
pub(super) fn footprint(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for &o in *p {
            h ^= o as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub(super) const ZERO_BUFFER_SIZE: usize = 256 << 10;
/// Constant register reserved for naga's special constants (first vertex / instance).
pub(super) const REGISTRY_SPECIAL_CONSTANTS: u32 = 13;
/// Alignment of constant offsets: 16 constants of 16 bytes (D3D 11.1 rule).
pub(super) const ALIGNMENT_CONSTANTS: u64 = 256;

pub struct Device {
    pub(super) shared: Arc<Shared>,
}

pub struct Queue {
    pub(super) shared: Arc<Shared>,
    /// State of bindings and states on the immediate context, kept from one submission to the next
    /// so as not to resend what has not changed (queue.rs).
    pub(super) state_value: Mutex<queue::StateContext>,
}

// ---------------------------------------------------------------------------------------------
// Commands (command.rs and queue.rs)
// ---------------------------------------------------------------------------------------------

pub struct CommandEncoder {
    pub(super) shared: Arc<Shared>,
    pub(super) commands: Vec<Command>,
    /// Render pass in progress (to close properly and resolve multisampling).
    pub(super) pass: Option<command::PassOpen>,
    /// Compute pass in progress (bind groups then go to the compute registers).
    pub(super) compute: bool,
    pub(super) timestamp_end_compute: Option<d3d11::ID3D11Query>,
}

pub struct CommandBuffer {
    pub(super) commands: Vec<Command>,
}

// ---------------------------------------------------------------------------------------------
// Resources (resources.rs)
// ---------------------------------------------------------------------------------------------

/// Buffer. `raw` is absent for a purely "CPU" buffer (MAP_WRITE | COPY_SRC, the case
/// of wgpu-core's staging buffers for write_buffer): its content lives in `shadow`, and the
/// copy to a real buffer becomes an UpdateSubresource1 at execution time.
#[derive(Clone)]
pub struct Buffer {
    pub(super) raw: Option<d3d11::ID3D11Buffer>,
    pub(super) size: wgt::BufferAddress,
    pub(super) usage: wgt::BufferUses,
    /// CPU copy (MAP_WRITE / MAP_READ buffers).
    pub(super) shadow: Option<Arc<Mutex<Vec<u8>>>>,
    /// Readback buffer (STAGING, CPU_ACCESS_READ) for MAP_READ.
    pub(super) read_value: Option<d3d11::ID3D11Buffer>,
}

#[derive(Clone)]
pub struct Texture {
    pub(super) raw: d3d11::ID3D11Resource,
    pub(super) format: wgt::TextureFormat,
    pub(super) dimension: wgt::TextureDimension,
    pub(super) size: wgt::Extent3d,
    pub(super) mip_level_count: u32,
    pub(super) sample_count: u32,
    pub(super) usage: wgt::TextureUses,
}

impl Texture {
    /// Number of layers (arrays and cubes), 1 for a 3D texture.
    pub(super) fn layers(&self) -> u32 {
        match self.dimension {
            wgt::TextureDimension::D3 => 1,
            _ => self.size.depth_or_array_layers,
        }
    }
    /// D3D11 subresource index (mip + layer * number of mips).
    pub(super) fn sub_resource(&self, mip: u32, layer: u32) -> u32 {
        mip + layer * self.mip_level_count
    }
}

pub struct SurfaceTexture {
    pub(super) texture: Texture,
}

/// Texture view: each view only exists if the texture's usage allows it.
#[derive(Clone)]
pub struct TextureView {
    pub(super) texture: d3d11::ID3D11Resource,
    pub(super) format: wgt::TextureFormat,
    pub(super) dimension: wgt::TextureViewDimension,
    pub(super) mips: Range<u32>,
    pub(super) layers: Range<u32>,
    /// Number of mips of the whole texture (computing subresource indices).
    pub(super) nb_mips_texture: u32,
    /// Size of the view's first mip (for viewports and targets).
    pub(super) size_value: wgt::Extent3d,
    pub(super) srv: Option<d3d11::ID3D11ShaderResourceView>,
    pub(super) rtv: Option<d3d11::ID3D11RenderTargetView>,
    pub(super) dsv: Option<d3d11::ID3D11DepthStencilView>,
    /// Read-only DSV (depth and/or stencil), for passes that read the depth.
    pub(super) dsv_read: Option<d3d11::ID3D11DepthStencilView>,
    pub(super) uav: Option<d3d11::ID3D11UnorderedAccessView>,
}

pub struct Sampler {
    pub(super) raw: d3d11::ID3D11SamplerState,
}

pub struct QuerySet {
    pub(super) raw: Vec<d3d11::ID3D11Query>,
    pub(super) ty: wgt::QueryType,
}

/// End fence: D3D11 has no fence on these drivers; each submission sets an event
/// query, and the reached value advances when the query has finished (sync.rs).
pub struct Fence {
    pub(super) reach_result: AtomicU64,
    pub(super) in_wait: Mutex<Vec<(crate::FenceValue, d3d11::ID3D11Query)>>,
    /// Highest submitted value: its query is only set (End) at the next poll (sync.rs).
    pub(super) requested: AtomicU64,
    /// Time of the last poll of the driver (spaced-out polls, see sync::value_reached).
    pub(super) last_record: Mutex<Option<std::time::Instant>>,
}

// ---------------------------------------------------------------------------------------------
// Bindings and pipelines (shader.rs and pipelines.rs)
// ---------------------------------------------------------------------------------------------

pub struct BindGroupLayout {
    pub(super) inputs: Vec<wgt::BindGroupLayoutEntry>,
}

/// D3D11 register kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// b#: constant buffer (uniform).
    Constants,
    /// t#: texture or read-only buffer (SRV).
    ReadSlot,
    /// s#: sampler.
    SamplerSlot,
    /// u#: write (UAV), pixel and compute only at level 11_0.
    WriteSlot,
}

/// Register assigned to a bind group entry.
#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub binding: u32,
    pub kind_index: Kind,
    pub registry: u32,
    pub stages_list: wgt::ShaderStages,
    /// Rank among the bind group's dynamic offsets (entry order), if dynamic.
    pub dynamic_rank: Option<u32>,
}

/// Registers of a whole pipeline layout, shared by the pipeline, the shaders and the queue.
pub struct LayoutBindings {
    /// `groups[g]` = registers of the entries of group g, in the order of its layout's entries.
    pub groups: Vec<Vec<Slot>>,
    /// Map given to naga: (group, binding) -> register.
    pub map_naga: naga::back::hlsl::BindingMap,
    /// Number of registers used per kind (to size the binding arrays).
    pub nb_constants: u32,
    pub read_count: u32,
    pub nb_samplers: u32,
    pub nb_write: u32,
}

pub struct PipelineLayout {
    pub(super) layout_value: Arc<LayoutBindings>,
}

/// A resource bound in a bind group, in the order of the layout's entries.
#[derive(Clone)]
pub enum Bound {
    /// Constant buffer: offset and size in bytes (dynamic offset added at execution).
    Constants { buffer_value: d3d11::ID3D11Buffer, shift_amount: u64, size_value: u64 },
    /// Read view and address of its resource (to unbind only the views of a target).
    ReadSlot(d3d11::ID3D11ShaderResourceView, usize),
    SamplerSlot(d3d11::ID3D11SamplerState),
    WriteSlot(d3d11::ID3D11UnorderedAccessView),
}

pub struct BindGroup {
    pub(super) bound_list: Arc<[Bound]>,
}

/// Shader module: the naga module is kept; HLSL compilation happens when the
/// pipeline is created, once the register layout is known (shader.rs).
pub struct ShaderModule {
    pub(super) naga: crate::NagaShader,
    pub(super) runtime_checks: wgt::ShaderRuntimeChecks,
}

/// Everything a draw sets on the context, prepared once (pipelines.rs).
pub struct RenderPipelineInternal {
    pub vs: d3d11::ID3D11VertexShader,
    pub ps: Option<d3d11::ID3D11PixelShader>,
    pub layout_vertices: Option<d3d11::ID3D11InputLayout>,
    /// Stride of each vertex buffer (index = slot).
    pub vertex_step: ArrayVec<u32, { crate::MAX_VERTEX_BUFFERS }>,
    pub topology_value: Direct3D::D3D_PRIMITIVE_TOPOLOGY,
    pub blend_value: d3d11::ID3D11BlendState,
    pub depth_value: d3d11::ID3D11DepthStencilState,
    pub frame: d3d11::ID3D11RasterizerState,
    pub mask_samples: u32,
    pub bindings_list: Arc<LayoutBindings>,
    /// The vertex shader reads first vertex / first instance (register REGISTRY_SPECIAL_CONSTANTS).
    pub special_constants: bool,
}

pub struct RenderPipeline {
    pub(super) internal: Arc<RenderPipelineInternal>,
}

pub struct ComputePipelineInternal {
    pub cs: d3d11::ID3D11ComputeShader,
    pub bindings_list: Arc<LayoutBindings>,
    /// The shader reads the number of workgroups (register REGISTRY_SPECIAL_CONSTANTS).
    pub special_constants: bool,
}

pub struct ComputePipeline {
    pub(super) internal: Arc<ComputePipelineInternal>,
}

pub struct RayTracingPipeline;
pub struct PipelineCache;
pub struct AccelerationStructure;

impl core::fmt::Debug for Instance { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Instance") } }
impl core::fmt::Debug for Surface { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Surface") } }
impl core::fmt::Debug for Adapter { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Adapter") } }
impl core::fmt::Debug for Device { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Device") } }
impl core::fmt::Debug for Queue { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Queue") } }
impl core::fmt::Debug for CommandEncoder { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::CommandEncoder") } }
impl core::fmt::Debug for CommandBuffer { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::CommandBuffer") } }
impl core::fmt::Debug for Buffer { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Buffer") } }
impl core::fmt::Debug for Texture { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Texture") } }
impl core::fmt::Debug for SurfaceTexture { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::SurfaceTexture") } }
impl core::fmt::Debug for TextureView { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::TextureView") } }
impl core::fmt::Debug for Sampler { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Sampler") } }
impl core::fmt::Debug for QuerySet { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::QuerySet") } }
impl core::fmt::Debug for Fence { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::Fence") } }
impl core::fmt::Debug for BindGroupLayout { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::BindGroupLayout") } }
impl core::fmt::Debug for BindGroup { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::BindGroup") } }
impl core::fmt::Debug for PipelineLayout { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::PipelineLayout") } }
impl core::fmt::Debug for ShaderModule { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::ShaderModule") } }
impl core::fmt::Debug for RenderPipeline { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::RenderPipeline") } }
impl core::fmt::Debug for ComputePipeline { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::ComputePipeline") } }
impl core::fmt::Debug for RayTracingPipeline { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::RayTracingPipeline") } }
impl core::fmt::Debug for PipelineCache { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::PipelineCache") } }
impl core::fmt::Debug for AccelerationStructure { fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { f.write_str("dx11::AccelerationStructure") } }

/// Message for a part that is not implemented yet.
macro_rules! to_do {
    ($block_value:literal) => { todo!(concat!("Direct3D 11, ", $block_value)) };
}
pub(crate) use to_do;

/// OXIII_LOG_WGPU=2: slow creations (pipelines, textures) with the time since the first one,
/// to spot hitches during a game.
pub(super) fn timer_creation(kind_index: &str, name_value: crate::Label, start_value: std::time::Instant) {
    static ORIGINAL: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let original = *ORIGINAL.get_or_init(|| start_value);
    let ms = start_value.elapsed().as_secs_f64() * 1000.0;
    if ms >= 2.0 {
        log::info!("Direct3D 11: {kind_index} {:?} in {ms:.1} ms (t = {:.2} s)", name_value, start_value.duration_since(original).as_secs_f64());
    }
}
