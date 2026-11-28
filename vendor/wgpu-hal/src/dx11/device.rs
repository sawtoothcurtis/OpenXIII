//! `Device` trait: each method delegates to the file that implements it. Do not write
//! logic here; code that needs one more method adds it in its own file.

use alloc::vec::Vec;
use core::time::Duration;

use super::{
    pipelines, resources, shader, sync, AccelerationStructure, Api, BindGroup, BindGroupLayout, Buffer,
    CommandEncoder, ComputePipeline, Device, DeviceResult, Fence, PipelineCache, PipelineLayout, QuerySet,
    Queue, RayTracingPipeline, RenderPipeline, Sampler, ShaderModule, Texture, TextureView,
};

impl crate::Device for Device {
    type A = Api;

    // --- buffers, textures, views, samplers: resources.rs ---

    unsafe fn create_buffer(&self, desc: &crate::BufferDescriptor) -> DeviceResult<Buffer> {
        unsafe { resources::create_buffer_impl(self, desc) }
    }
    unsafe fn destroy_buffer(&self, buffer: Buffer) {
        resources::return_buffer(buffer);
    }
    unsafe fn add_raw_buffer(&self, _buffer: &Buffer) {}

    unsafe fn map_buffer(&self, buffer: &Buffer, range: crate::MemoryRange) -> DeviceResult<crate::BufferMapping> {
        unsafe { resources::open_buffer(self, buffer, range) }
    }
    unsafe fn unmap_buffer(&self, buffer: &Buffer) {
        unsafe { resources::close_buffer(self, buffer) }
    }
    unsafe fn flush_mapped_ranges<I>(&self, buffer: &Buffer, ranges: I)
    where
        I: Iterator<Item = crate::MemoryRange>,
    {
        // Coherent CPU copies (is_coherent = true): nothing to do.
    }
    unsafe fn invalidate_mapped_ranges<I>(&self, buffer: &Buffer, ranges: I)
    where
        I: Iterator<Item = crate::MemoryRange>,
    {
    }

    unsafe fn create_texture(&self, desc: &crate::TextureDescriptor) -> DeviceResult<Texture> {
        let start_value = std::time::Instant::now();
        let r = unsafe { resources::create_texture_impl(self, desc) };
        super::timer_creation("texture", desc.label, start_value);
        r
    }
    unsafe fn destroy_texture(&self, texture: Texture) {}
    unsafe fn add_raw_texture(&self, _texture: &Texture) {}

    unsafe fn create_texture_view(&self, texture: &Texture, desc: &crate::TextureViewDescriptor) -> DeviceResult<TextureView> {
        unsafe { resources::create_view(self, texture, desc) }
    }
    unsafe fn destroy_texture_view(&self, view: TextureView) {}
    unsafe fn create_sampler(&self, desc: &crate::SamplerDescriptor) -> DeviceResult<Sampler> {
        unsafe { resources::create_sampler_impl(self, desc) }
    }
    unsafe fn destroy_sampler(&self, sampler: Sampler) {}

    // --- recording: command.rs ---

    unsafe fn create_command_encoder(&self, desc: &crate::CommandEncoderDescriptor<Queue>) -> DeviceResult<CommandEncoder> {
        Ok(CommandEncoder {
            shared: self.shared.clone(),
            commands: Vec::new(),
            pass: None,
            compute: false,
            timestamp_end_compute: None,
        })
    }

    // --- bindings and pipelines: pipelines.rs, shaders: shader.rs ---

    unsafe fn create_bind_group_layout(&self, desc: &crate::BindGroupLayoutDescriptor) -> DeviceResult<BindGroupLayout> {
        Ok(BindGroupLayout { inputs: desc.entries.to_vec() })
    }
    unsafe fn destroy_bind_group_layout(&self, bg_layout: BindGroupLayout) {}
    unsafe fn create_pipeline_layout(&self, desc: &crate::PipelineLayoutDescriptor<BindGroupLayout>) -> DeviceResult<PipelineLayout> {
        unsafe { pipelines::create_layout(self, desc) }
    }
    unsafe fn destroy_pipeline_layout(&self, pipeline_layout: PipelineLayout) {}
    unsafe fn create_bind_group(
        &self,
        desc: &crate::BindGroupDescriptor<BindGroupLayout, Buffer, Sampler, TextureView, AccelerationStructure>,
    ) -> DeviceResult<BindGroup> {
        unsafe { pipelines::create_group(self, desc) }
    }
    unsafe fn destroy_bind_group(&self, group: BindGroup) {}

    unsafe fn create_shader_module(&self, desc: &crate::ShaderModuleDescriptor, shader: crate::ShaderInput) -> Result<ShaderModule, crate::ShaderError> {
        unsafe { shader::create_module(self, desc, shader) }
    }
    unsafe fn destroy_shader_module(&self, module: ShaderModule) {}
    unsafe fn create_render_pipeline(
        &self,
        desc: &crate::RenderPipelineDescriptor<PipelineLayout, ShaderModule, PipelineCache>,
    ) -> Result<RenderPipeline, crate::PipelineError> {
        unsafe { pipelines::create_pipeline_render(self, desc) }
    }
    unsafe fn destroy_render_pipeline(&self, pipeline: RenderPipeline) {}
    unsafe fn create_compute_pipeline(
        &self,
        desc: &crate::ComputePipelineDescriptor<PipelineLayout, ShaderModule, PipelineCache>,
    ) -> Result<ComputePipeline, crate::PipelineError> {
        unsafe { pipelines::create_pipeline_compute(self, desc) }
    }
    unsafe fn destroy_compute_pipeline(&self, pipeline: ComputePipeline) {}
    unsafe fn create_ray_tracing_pipeline(
        &self,
        desc: &crate::RayTracingPipelineDescriptor<PipelineLayout, ShaderModule, PipelineCache>,
    ) -> Result<RayTracingPipeline, crate::PipelineError> {
        Err(crate::PipelineError::Linkage(wgt::ShaderStages::all(), alloc::string::String::from("no ray tracing on Direct3D 11")))
    }
    unsafe fn destroy_ray_tracing_pipeline(&self, pipeline: RayTracingPipeline) {}
    unsafe fn get_raytracing_pipeline_group_data(&self, pipeline: &RayTracingPipeline, groups: core::ops::Range<u32>) -> Result<Vec<u8>, crate::DeviceError> {
        Err(crate::DeviceError::Unexpected)
    }
    unsafe fn create_pipeline_cache(&self, desc: &crate::PipelineCacheDescriptor<'_>) -> Result<PipelineCache, crate::PipelineCacheError> {
        Ok(PipelineCache)
    }
    unsafe fn destroy_pipeline_cache(&self, cache: PipelineCache) {}

    // --- queries and end fences: sync.rs ---

    unsafe fn create_query_set(&self, desc: &wgt::QuerySetDescriptor<crate::Label>) -> DeviceResult<QuerySet> {
        unsafe { sync::create_queries(self, desc) }
    }
    unsafe fn destroy_query_set(&self, set: QuerySet) {}
    unsafe fn create_fence(&self) -> DeviceResult<Fence> {
        Ok(Fence {
            reach_result: core::sync::atomic::AtomicU64::new(0),
            in_wait: parking_lot::Mutex::new(Vec::new()),
            requested: core::sync::atomic::AtomicU64::new(0),
            last_record: parking_lot::Mutex::new(None),
        })
    }
    unsafe fn destroy_fence(&self, fence: Fence) {}
    unsafe fn get_fence_value(&self, fence: &Fence) -> DeviceResult<crate::FenceValue> {
        unsafe { sync::value_reached(self, fence) }
    }
    unsafe fn wait(&self, fence: &Fence, value: crate::FenceValue, timeout: Option<Duration>) -> DeviceResult<bool> {
        let r = unsafe { sync::wait_value(self, fence, value, timeout) };
        r
    }

    unsafe fn start_graphics_debugger_capture(&self) -> bool {
        false
    }
    unsafe fn stop_graphics_debugger_capture(&self) {}

    // --- ray tracing: absent in Direct3D 11 ---

    unsafe fn create_acceleration_structure(&self, desc: &crate::AccelerationStructureDescriptor) -> DeviceResult<AccelerationStructure> {
        Err(crate::DeviceError::Unexpected)
    }
    unsafe fn get_acceleration_structure_build_sizes<'a>(
        &self,
        _desc: &crate::GetAccelerationStructureBuildSizesDescriptor<'a, Buffer>,
    ) -> crate::AccelerationStructureBuildSizes {
        Default::default()
    }
    unsafe fn get_acceleration_structure_device_address(&self, _acceleration_structure: &AccelerationStructure) -> wgt::BufferAddress {
        0
    }
    unsafe fn destroy_acceleration_structure(&self, _acceleration_structure: AccelerationStructure) {}
    fn tlas_instance_to_bytes(&self, instance: crate::TlasInstance) -> Vec<u8> {
        Vec::new()
    }

    fn get_internal_counters(&self) -> wgt::HalCounters {
        Default::default()
    }
    fn check_if_oom(&self) -> DeviceResult<()> {
        Ok(())
    }
}
