//! Recording of commands into a `Command` list.
//!
//! The list is the contract between recording (this file) and execution (`queue.rs`).
//! Here we never talk to the GPU: we copy what is needed (COM objects, offsets) so that
//! execution has nothing left to look up. wgpu's barriers have no equivalent in D3D11
//! (the driver tracks hazards itself): they are ignored.

use alloc::{string::String, sync::Arc, vec::Vec};
use core::{mem, ops::Range};

use arrayvec::ArrayVec;
use windows::Win32::Graphics::{Direct3D11 as d3d11, Dxgi::Common::DXGI_FORMAT};

use super::{
    AccelerationStructure, Api, BindGroup, Buffer, CommandBuffer, CommandEncoder, ComputePipeline,
    ComputePipelineInternal, DeviceResult, LayoutBindings, Bound, PipelineLayout, QuerySet, RenderPipeline,
    RenderPipelineInternal, Texture, TextureView,
};

/// Dynamic offsets of a bind group: up to 3 inside the command itself (OpenXIII never has
/// more than one), beyond that in a box. A command stays small (see the size of `Command`).
pub enum Offsets {
    Short(u8, [wgt::DynamicOffset; 3]),
    Long(alloc::boxed::Box<[wgt::DynamicOffset]>),
}

impl Offsets {
    fn from_time(d: &[wgt::DynamicOffset]) -> Self {
        if d.len() <= 3 {
            let mut v = [0; 3];
            v[..d.len()].copy_from_slice(d);
            Self::Short(d.len() as u8, v)
        } else {
            Self::Long(d.into())
        }
    }
}

impl core::ops::Deref for Offsets {
    type Target = [wgt::DynamicOffset];
    fn deref(&self) -> &[wgt::DynamicOffset] {
        match self {
            Self::Short(n, v) => &v[..*n as usize],
            Self::Long(v) => v,
        }
    }
}

/// Color targets of a pass (boxed: 8 attachments would make every command huge).
pub type Colors = alloc::boxed::Box<ArrayVec<Option<AttachColor>, { crate::MAX_COLOR_ATTACHMENTS }>>;

/// Color target of a pass.
#[derive(Clone)]
pub struct AttachColor {
    pub rtv: d3d11::ID3D11RenderTargetView,
    /// Address of the resource: read views of this resource are unbound at the start of the pass.
    pub resource: usize,
    /// Clear color if the operation is LOAD_CLEAR.
    pub clear_color: Option<[f32; 4]>,
    /// Multisample resolve at the end of the pass.
    pub resolve: Option<Resolution>,
}

/// ResolveSubresource(dst, dst_sub, src, src_sub, format) at the end of the pass.
#[derive(Clone)]
pub struct Resolution {
    pub src: d3d11::ID3D11Resource,
    pub src_sub: u32,
    pub dst: d3d11::ID3D11Resource,
    pub dst_sub: u32,
    pub format: DXGI_FORMAT,
}

/// Depth/stencil target of a pass.
#[derive(Clone)]
pub struct AttachDepth {
    pub dsv: d3d11::ID3D11DepthStencilView,
    /// Address of the resource if the pass writes it (a read-only DSV may stay bound for reading).
    pub written_resource: usize,
    pub clear_depth: Option<f32>,
    pub clear_stencil: Option<u8>,
}

/// Render pass open during recording.
pub struct PassOpen {
    pub resolutions: Vec<Resolution>,
    pub timestamp_end: Option<(d3d11::ID3D11Query, u32)>,
}

/// A recorded command, replayed by `queue.rs`.
pub enum Command {
    // --- copies ---
    ClearBuffer { buffer_value: Buffer, range_value: crate::MemoryRange },
    // Boxed copies: rare, and large (two resources and a region).
    CopyBuffer(alloc::boxed::Box<(Buffer, Buffer, crate::BufferCopy)>),
    CopyBufferToTexture(alloc::boxed::Box<(Buffer, Texture, crate::BufferTextureCopy)>),
    CopyTextureToBuffer(alloc::boxed::Box<(Texture, Buffer, crate::BufferTextureCopy)>),
    CopyTexture(alloc::boxed::Box<(Texture, Texture, crate::TextureCopy)>),

    // --- render passes ---
    /// Sets the targets (OMSetRenderTargets), clears what must be cleared, viewport and scissor at the
    /// size of the pass. First unbinds from all stages any read view of a texture that became a target.
    StartPass {
        colors: Colors,
        depth_value: Option<AttachDepth>,
        size_value: wgt::Extent3d,
    },
    /// End of pass: resolves, if any.
    EndPass { resolutions: Vec<Resolution> },
    PipelineRender(Arc<RenderPipelineInternal>),
    /// Binds a bind group: registers taken from `layout.groups[hint]`, resources from `bound_list`
    /// (same order), dynamic offsets in the order of the dynamic entries.
    Group { hint: u32, layout_value: Arc<LayoutBindings>, bound_list: Arc<[Bound]>, offsets: Offsets, compute: bool },
    BufferIndex { buffer_value: Raw<d3d11::ID3D11Buffer>, format: DXGI_FORMAT, shift_amount: u32 },
    BufferVertices { slot: u32, buffer_value: Raw<d3d11::ID3D11Buffer>, shift_amount: u32 },
    /// Viewport in pixels (origin at the top left, like wgpu and D3D11).
    Window { rect: crate::Rect<f32>, depth_value: Range<f32> },
    Scissor { rect: crate::Rect<u32> },
    RefStencil(u32),
    ConstantsBlend([f32; 4]),
    Draw { first_vertex_value: u32, nb_vertices: u32, first_instance_value: u32, nb_instances: u32 },
    DrawIndexed { first_index_value: u32, nb_index: u32, vertex_base: i32, first_instance_value: u32, nb_instances: u32 },
    /// Arguments read from a buffer created with D3D11_RESOURCE_MISC_DRAWINDIRECT_ARGS (resources.rs).
    DrawIndirect { buffer_value: Raw<d3d11::ID3D11Buffer>, shift_amount: u32, nb: u32, indexed: bool },

    // --- compute ---
    PipelineCompute(Arc<ComputePipelineInternal>),
    Compute([u32; 3]),
    ComputeIndirect { buffer_value: Raw<d3d11::ID3D11Buffer>, shift_amount: u32 },
    /// End of compute pass: unbind the UAVs so that what follows can read.
    EndCompute,

    // --- queries (see sync.rs for their meaning) ---
    StartQuery { query: d3d11::ID3D11Query },
    EndQuery { query: d3d11::ID3D11Query },
    Timestamp { query: d3d11::ID3D11Query },
    CopyResultsQueries(alloc::boxed::Box<ResultsQueries>),

    // --- labels (debugging tools) ---
    StartGroupDebug(String),
    EndGroupDebug,
    Marker(String),
}

/// COM object held without a reference (neither AddRef on recording nor Release when the
/// list is freed): wgpu-core keeps every resource of a list alive until it has finished executing.
/// One AddRef/Release per buffer and per draw went through the driver.
pub struct Raw<T: windows::core::Interface>(mem::ManuallyDrop<T>);

impl<T: windows::core::Interface> Raw<T> {
    pub fn from_time(x: &T) -> Self {
        // SAFETY: copy of the interface pointer without AddRef; never released (ManuallyDrop).
        Self(mem::ManuallyDrop::new(unsafe { mem::transmute_copy(x) }))
    }
}

impl<T: windows::core::Interface> core::ops::Deref for Raw<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// CopyResultsQueries: queries to read back and where to write their results.
pub struct ResultsQueries {
    pub queries: Vec<d3d11::ID3D11Query>,
    pub ty: wgt::QueryType,
    pub dst: Buffer,
    pub shift_amount: u64,
    pub step_size: u64,
}

// One command per draw, binding or state change: copied when recorded and walked at
// execution as when freed. Keep the large cases boxed.
const _: () = assert!(mem::size_of::<Command>() <= 64);

fn raw_buffer(b: &Buffer) -> Raw<d3d11::ID3D11Buffer> {
    Raw::from_time(b.raw.as_ref().expect("Direct3D 11: CPU buffer used as a GPU buffer"))
}

impl crate::CommandEncoder for CommandEncoder {
    type A = Api;

    unsafe fn begin_encoding(&mut self, label: crate::Label) -> DeviceResult<()> {
        self.commands.clear();
        Ok(())
    }
    unsafe fn discard_encoding(&mut self) {
        self.commands.clear();
        self.pass = None;
    }
    unsafe fn end_encoding(&mut self) -> DeviceResult<CommandBuffer> {
        Ok(CommandBuffer { commands: mem::take(&mut self.commands) })
    }
    unsafe fn reset_all<I>(&mut self, command_buffers: I)
    where
        I: Iterator<Item = CommandBuffer>,
    {
        // The largest list returned serves the next encoding (no reallocations while
        // filling it every frame).
        for mut cb in command_buffers {
            if cb.commands.capacity() > self.commands.capacity() {
                cb.commands.clear();
                mem::swap(&mut self.commands, &mut cb.commands);
            }
        }
    }

    unsafe fn transition_buffers<'a, T>(&mut self, barriers: T)
    where
        T: Iterator<Item = crate::BufferBarrier<'a, Buffer>>,
    {
    }
    unsafe fn transition_textures<'a, T>(&mut self, barriers: T)
    where
        T: Iterator<Item = crate::TextureBarrier<'a, Texture>>,
    {
    }

    unsafe fn clear_buffer(&mut self, buffer: &Buffer, range: crate::MemoryRange) {
        self.commands.push(Command::ClearBuffer { buffer_value: buffer.clone(), range_value: range });
    }

    unsafe fn copy_buffer_to_buffer<T>(&mut self, src: &Buffer, dst: &Buffer, regions: T)
    where
        T: Iterator<Item = crate::BufferCopy>,
    {
        for region in regions {
            self.commands.push(Command::CopyBuffer(alloc::boxed::Box::new((src.clone(), dst.clone(), region))));
        }
    }

    unsafe fn copy_texture_to_texture<T>(&mut self, src: &Texture, src_usage: wgt::TextureUses, dst: &Texture, regions: T)
    where
        T: Iterator<Item = crate::TextureCopy>,
    {
        for region in regions {
            self.commands.push(Command::CopyTexture(alloc::boxed::Box::new((src.clone(), dst.clone(), region))));
        }
    }

    unsafe fn copy_buffer_to_texture<T>(&mut self, src: &Buffer, dst: &Texture, regions: T)
    where
        T: Iterator<Item = crate::BufferTextureCopy>,
    {
        for region in regions {
            self.commands.push(Command::CopyBufferToTexture(alloc::boxed::Box::new((src.clone(), dst.clone(), region))));
        }
    }

    unsafe fn copy_texture_to_buffer<T>(&mut self, src: &Texture, src_usage: wgt::TextureUses, dst: &Buffer, regions: T)
    where
        T: Iterator<Item = crate::BufferTextureCopy>,
    {
        for region in regions {
            self.commands.push(Command::CopyTextureToBuffer(alloc::boxed::Box::new((src.clone(), dst.clone(), region))));
        }
    }

    unsafe fn begin_query(&mut self, set: &QuerySet, index: u32) {
        self.commands.push(Command::StartQuery { query: set.raw[index as usize].clone() });
    }
    unsafe fn end_query(&mut self, set: &QuerySet, index: u32) {
        self.commands.push(Command::EndQuery { query: set.raw[index as usize].clone() });
    }
    unsafe fn write_timestamp(&mut self, set: &QuerySet, index: u32) {
        self.commands.push(Command::Timestamp { query: set.raw[index as usize].clone() });
    }
    unsafe fn read_acceleration_structure_compact_size(&mut self, acceleration_structure: &AccelerationStructure, buf: &Buffer) {}
    unsafe fn reset_queries(&mut self, set: &QuerySet, range: Range<u32>) {}
    unsafe fn copy_query_results(&mut self, set: &QuerySet, range: Range<u32>, buffer: &Buffer, offset: wgt::BufferAddress, stride: wgt::BufferSize) {
        self.commands.push(Command::CopyResultsQueries(alloc::boxed::Box::new(ResultsQueries {
            queries: set.raw[range.start as usize..range.end as usize].to_vec(),
            ty: set.ty,
            dst: buffer.clone(),
            shift_amount: offset,
            step_size: stride.get(),
        })));
    }

    // --- rendering ---

    unsafe fn begin_render_pass(&mut self, desc: &crate::RenderPassDescriptor<QuerySet, TextureView>) -> DeviceResult<()> {
        let mut colors = Colors::default();
        let mut resolutions = Vec::new();
        for c in desc.color_attachments {
            colors.push(c.as_ref().map(|c| {
                let view_value = c.target.view;
                let resolve = c.resolve_target.as_ref().map(|r| Resolution {
                    src: view_value.texture.clone(),
                    src_sub: view_value.mips.start + view_value.layers.start * view_value.nb_mips_texture,
                    dst: r.view.texture.clone(),
                    dst_sub: r.view.mips.start + r.view.layers.start * r.view.nb_mips_texture,
                    format: super::conv::format_dxgi(view_value.format),
                });
                if let Some(r) = &resolve {
                    resolutions.push(r.clone());
                }
                AttachColor {
                    rtv: view_value.rtv.clone().expect("Direct3D 11: view without an RTV used as a target"),
                    resource: windows::core::Interface::as_raw(&view_value.texture) as usize,
                    clear_color: c.ops.contains(crate::AttachmentOps::LOAD_CLEAR).then(|| {
                        let v = c.clear_value;
                        [v.r as f32, v.g as f32, v.b as f32, v.a as f32]
                    }),
                    resolve,
                }
            }));
        }
        let depth_value = desc.depth_stencil_attachment.as_ref().map(|d| {
            let view_value = d.target.view;
            let writes = d.target.usage.contains(wgt::TextureUses::DEPTH_STENCIL_WRITE);
            AttachDepth {
                dsv: if writes { view_value.dsv.clone() } else { view_value.dsv_read.clone().or_else(|| view_value.dsv.clone()) }
                    .expect("Direct3D 11: view without a DSV used as depth"),
                written_resource: if writes { windows::core::Interface::as_raw(&view_value.texture) as usize } else { 0 },
                clear_depth: d.depth_ops.contains(crate::AttachmentOps::LOAD_CLEAR).then_some(d.clear_value.0),
                clear_stencil: d.stencil_ops.contains(crate::AttachmentOps::LOAD_CLEAR).then_some(d.clear_value.1 as u8),
            }
        });
        if let Some(t) = &desc.timestamp_writes {
            if let Some(i) = t.beginning_of_pass_write_index {
                self.commands.push(Command::Timestamp { query: t.query_set.raw[i as usize].clone() });
            }
        }
        self.commands.push(Command::StartPass { colors, depth_value, size_value: desc.extent });
        self.pass = Some(PassOpen {
            resolutions,
            timestamp_end: desc.timestamp_writes.as_ref().and_then(|t| {
                t.end_of_pass_write_index.map(|i| (t.query_set.raw[i as usize].clone(), i))
            }),
        });
        Ok(())
    }

    unsafe fn end_render_pass(&mut self) {
        if let Some(p) = self.pass.take() {
            self.commands.push(Command::EndPass { resolutions: p.resolutions });
            if let Some((q, _)) = p.timestamp_end {
                self.commands.push(Command::Timestamp { query: q });
            }
        }
    }

    unsafe fn set_bind_group(&mut self, layout: &PipelineLayout, index: u32, group: &BindGroup, dynamic_offsets: &[wgt::DynamicOffset]) {
        self.commands.push(Command::Group {
            hint: index,
            layout_value: layout.layout_value.clone(),
            bound_list: group.bound_list.clone(),
            offsets: Offsets::from_time(dynamic_offsets),
            compute: self.compute,
        });
    }
    unsafe fn set_immediates(&mut self, layout: &PipelineLayout, offset_bytes: u32, data: &[u32]) {
        unreachable!("Direct3D 11: no immediate constants (feature not announced)")
    }

    unsafe fn insert_debug_marker(&mut self, label: &str) {
        self.commands.push(Command::Marker(String::from(label)));
    }
    unsafe fn begin_debug_marker(&mut self, group_label: &str) {
        self.commands.push(Command::StartGroupDebug(String::from(group_label)));
    }
    unsafe fn end_debug_marker(&mut self) {
        self.commands.push(Command::EndGroupDebug);
    }

    unsafe fn set_render_pipeline(&mut self, pipeline: &RenderPipeline) {
        self.commands.push(Command::PipelineRender(pipeline.internal.clone()));
    }

    unsafe fn set_index_buffer<'a>(&mut self, binding: crate::BufferBinding<'a, Buffer>, format: wgt::IndexFormat) {
        self.commands.push(Command::BufferIndex {
            buffer_value: raw_buffer(binding.buffer),
            format: super::conv::format_index(format),
            shift_amount: binding.offset as u32,
        });
    }
    unsafe fn set_vertex_buffer<'a>(&mut self, index: u32, binding: crate::BufferBinding<'a, Buffer>) {
        self.commands.push(Command::BufferVertices { slot: index, buffer_value: raw_buffer(binding.buffer), shift_amount: binding.offset as u32 });
    }
    unsafe fn set_viewport(&mut self, rect: &crate::Rect<f32>, depth_range: Range<f32>) {
        self.commands.push(Command::Window { rect: rect.clone(), depth_value: depth_range });
    }
    unsafe fn set_scissor_rect(&mut self, rect: &crate::Rect<u32>) {
        self.commands.push(Command::Scissor { rect: rect.clone() });
    }
    unsafe fn set_stencil_reference(&mut self, value: u32) {
        self.commands.push(Command::RefStencil(value));
    }
    unsafe fn set_blend_constants(&mut self, color: &[f32; 4]) {
        self.commands.push(Command::ConstantsBlend(*color));
    }

    unsafe fn draw(&mut self, first_vertex: u32, vertex_count: u32, first_instance: u32, instance_count: u32) {
        self.commands.push(Command::Draw { first_vertex_value: first_vertex, nb_vertices: vertex_count, first_instance_value: first_instance, nb_instances: instance_count });
    }
    unsafe fn draw_indexed(&mut self, first_index: u32, index_count: u32, base_vertex: i32, first_instance: u32, instance_count: u32) {
        self.commands.push(Command::DrawIndexed {
            first_index_value: first_index,
            nb_index: index_count,
            vertex_base: base_vertex,
            first_instance_value: first_instance,
            nb_instances: instance_count,
        });
    }
    unsafe fn draw_mesh_tasks(&mut self, group_count_x: u32, group_count_y: u32, group_count_z: u32) {
        unreachable!("Direct3D 11: no mesh shaders")
    }
    unsafe fn draw_indirect(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, draw_count: u32) {
        self.commands.push(Command::DrawIndirect { buffer_value: raw_buffer(buffer), shift_amount: offset as u32, nb: draw_count, indexed: false });
    }
    unsafe fn draw_indexed_indirect(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, draw_count: u32) {
        self.commands.push(Command::DrawIndirect { buffer_value: raw_buffer(buffer), shift_amount: offset as u32, nb: draw_count, indexed: true });
    }
    unsafe fn draw_mesh_tasks_indirect(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, draw_count: u32) {
        unreachable!("Direct3D 11: no mesh shaders")
    }
    unsafe fn draw_indirect_count(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, count_buffer: &Buffer, count_offset: wgt::BufferAddress, max_count: u32) {
        unreachable!("Direct3D 11: no counted indirect draws (feature not announced)")
    }
    unsafe fn draw_indexed_indirect_count(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, count_buffer: &Buffer, count_offset: wgt::BufferAddress, max_count: u32) {
        unreachable!("Direct3D 11: no counted indirect draws (feature not announced)")
    }
    unsafe fn draw_mesh_tasks_indirect_count(&mut self, buffer: &Buffer, offset: wgt::BufferAddress, count_buffer: &Buffer, count_offset: wgt::BufferAddress, max_count: u32) {
        unreachable!("Direct3D 11: no mesh shaders")
    }

    // --- compute ---

    unsafe fn begin_compute_pass(&mut self, desc: &crate::ComputePassDescriptor<QuerySet>) {
        self.compute = true;
        if let Some(t) = &desc.timestamp_writes {
            if let Some(i) = t.beginning_of_pass_write_index {
                self.commands.push(Command::Timestamp { query: t.query_set.raw[i as usize].clone() });
            }
            self.timestamp_end_compute = t.end_of_pass_write_index.map(|i| t.query_set.raw[i as usize].clone());
        }
    }
    unsafe fn end_compute_pass(&mut self) {
        self.compute = false;
        self.commands.push(Command::EndCompute);
        if let Some(q) = self.timestamp_end_compute.take() {
            self.commands.push(Command::Timestamp { query: q });
        }
    }
    unsafe fn set_compute_pipeline(&mut self, pipeline: &ComputePipeline) {
        self.commands.push(Command::PipelineCompute(pipeline.internal.clone()));
    }
    unsafe fn dispatch_workgroups(&mut self, count: [u32; 3]) {
        self.commands.push(Command::Compute(count));
    }
    unsafe fn dispatch_workgroups_indirect(&mut self, buffer: &Buffer, offset: wgt::BufferAddress) {
        self.commands.push(Command::ComputeIndirect { buffer_value: raw_buffer(buffer), shift_amount: offset as u32 });
    }

    // --- ray tracing: absent ---

    unsafe fn build_acceleration_structures<'a, T>(&mut self, _descriptor_count: u32, descriptors: T)
    where
        Api: 'a,
        T: IntoIterator<Item = crate::BuildAccelerationStructureDescriptor<'a, Buffer, AccelerationStructure>>,
    {
        unreachable!("Direct3D 11: no ray tracing")
    }
    unsafe fn place_acceleration_structure_barrier(&mut self, _barriers: crate::AccelerationStructureBarrier) {}
    unsafe fn copy_acceleration_structure_to_acceleration_structure(
        &mut self,
        src: &AccelerationStructure,
        dst: &AccelerationStructure,
        copy: wgt::AccelerationStructureCopy,
    ) {
        unreachable!("Direct3D 11: no ray tracing")
    }
    unsafe fn set_acceleration_structure_dependencies(command_buffers: &[&CommandBuffer], dependencies: &[&AccelerationStructure]) {}
    unsafe fn begin_ray_tracing_pass(&mut self, _desc: &crate::RayTracingPassDescriptor) {}
    unsafe fn end_ray_tracing_pass(&mut self) {}
    unsafe fn set_ray_tracing_pipeline(&mut self, _pipeline: &super::RayTracingPipeline) {}
    unsafe fn trace_rays(
        &mut self,
        _count: [u32; 3],
        _ray_generation_group_data: crate::PipelineGroupData<Buffer>,
        _miss_group_data: crate::PipelineGroupData<Buffer>,
        _intersection_group_data: crate::PipelineGroupData<Buffer>,
    ) {
        unreachable!("Direct3D 11: no ray tracing")
    }
}
