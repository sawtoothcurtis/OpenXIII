//! Execution: replays the `Command` list (command.rs) on the immediate context.
//!
//! This is the core of the speed gain: only send D3D11 what changes (shaders, state
//! objects, constant buffers with offsets, views, samplers, vertex and
//! index buffers). `StateContext` keeps what is set, from one submission to the next. Objects are
//! compared by address: an object bound to the context is held there by D3D11, so its address
//! cannot be reused by another object while it is bound.
//!
//! Method: precomputed state objects, constants by offset (11.1), no allocation
//! per draw.

use alloc::{sync::Arc, vec};
use core::mem::ManuallyDrop;

use windows::{
    core::Interface as _,
    Win32::{
        Foundation::RECT,
        Graphics::Direct3D11 as d3d11,
    },
};

use super::{
    command::Resolution, Api, Buffer, Command, CommandBuffer, ComputePipelineInternal, DeviceResult, Fence, Bound,
    Queue, RenderPipelineInternal, Shared, Surface, SurfaceTexture, Texture,
};

/// Stages: 0 = vertex, 1 = pixel, 2 = compute.
const NB_STAGES: usize = 3;
const NB_CONSTANTS: usize = 14;
/// Tracked read registers (beyond these, set without tracking).
const READ_COUNT: usize = 32;
const NB_SAMPLERS: usize = 16;
const NB_WRITE: usize = 8;
const NB_VERTICES: usize = crate::MAX_VERTEX_BUFFERS;
/// Size of the special constants buffer (16 constants, the minimum of VSSetConstantBuffers1).
const SPECIAL_SIZE: u32 = 256;

/// What is set on the immediate context (to avoid useless calls).
pub struct StateContext {
    render: Option<Arc<RenderPipelineInternal>>,
    compute: Option<Arc<ComputePipelineInternal>>,
    blend_constants: [f32; 4],
    ref_stencil: u32,
    /// (buffer, first constant, count) per stage and register.
    constants: [[(usize, u32, u32); NB_CONSTANTS]; NB_STAGES],
    read_value: [[usize; READ_COUNT]; NB_STAGES],
    /// Resource of each read view set (0: none).
    read_resource: [[usize; READ_COUNT]; NB_STAGES],
    samplers: [[usize; NB_SAMPLERS]; NB_STAGES],
    write_compute: [usize; NB_WRITE],
    vertices_list: [Option<d3d11::ID3D11Buffer>; NB_VERTICES],
    vertices_offset: [u32; NB_VERTICES],
    /// Stride actually set for each slot.
    vertices_stride: [u32; NB_VERTICES],
    /// Vertex slots to set again before the next draw.
    vertices_dirty: u32,
    index: (usize, i32, u32),
    /// Buffer of naga's special constants (first vertex, first instance, other).
    special: Option<d3d11::ID3D11Buffer>,
    special_values: [u32; 4],
    /// Upload ring (DYNAMIC buffer, size) and position of the next write.
    ring_value: Option<(d3d11::ID3D11Buffer, u64)>,
    ring_pos: u64,
    /// Places in the ring of the current submission's writes, in copy order.
    sends: alloc::collections::VecDeque<u32>,
    /// Targets set (addresses of the RTVs, of the DSV, count) and generation of `Shared`'s targets.
    targets_placed: Option<([usize; crate::MAX_COLOR_ATTACHMENTS + 2], u64)>,
    target_count: (u32, u32),
    /// Passes or dispatches recorded on the context since its last Flush (Present flushes).
    drawn_since_flush: bool,
}

impl Default for StateContext {
    fn default() -> Self {
        StateContext {
            render: None,
            compute: None,
            blend_constants: [0.0; 4],
            ref_stencil: 0,
            constants: [[(0, 0, 0); NB_CONSTANTS]; NB_STAGES],
            read_value: [[0; READ_COUNT]; NB_STAGES],
            read_resource: [[0; READ_COUNT]; NB_STAGES],
            samplers: [[0; NB_SAMPLERS]; NB_STAGES],
            write_compute: [0; NB_WRITE],
            vertices_list: Default::default(),
            vertices_offset: [0; NB_VERTICES],
            vertices_stride: [0; NB_VERTICES],
            vertices_dirty: 0,
            index: (0, 0, 0),
            special: None,
            special_values: [u32::MAX; 4],
            ring_value: None,
            ring_pos: u64::MAX,
            sends: alloc::collections::VecDeque::new(),
            targets_placed: None,
            target_count: (0, 0),
            drawn_since_flush: false,
        }
    }
}

fn address<T: windows::core::Interface>(x: &T) -> usize {
    x.as_raw() as usize
}

/// Lends a COM object as `Option<T>` without AddRef/Release (for binding arrays).
fn borrow<T: windows::core::Interface>(x: &T) -> ManuallyDrop<Option<T>> {
    // SAFETY: `Option<T>` has the same representation as a non-null interface pointer; the copy
    // is never released (ManuallyDrop) and does not outlive `x`.
    ManuallyDrop::new(Some(unsafe { core::mem::transmute_copy(x) }))
}

fn rounded_256(n: u64) -> u64 {
    (n + 255) & !255
}

unsafe fn place_constants(ctx: &d3d11::ID3D11DeviceContext1, stage: usize, registry: u32, buffer_value: &d3d11::ID3D11Buffer, first_one: u32, count_value: u32) {
    call_value(if stage == 0 { 0 } else { 1 });
    let b = borrow(buffer_value);
    let p: *const Option<d3d11::ID3D11Buffer> = &*b;
    let (f, n) = ([first_one], [count_value]);
    unsafe {
        match stage {
            0 => ctx.VSSetConstantBuffers1(registry, 1, Some(p), Some(f.as_ptr()), Some(n.as_ptr())),
            1 => ctx.PSSetConstantBuffers1(registry, 1, Some(p), Some(f.as_ptr()), Some(n.as_ptr())),
            _ => ctx.CSSetConstantBuffers1(registry, 1, Some(p), Some(f.as_ptr()), Some(n.as_ptr())),
        }
    }
}

unsafe fn read_place(ctx: &d3d11::ID3D11DeviceContext1, stage: usize, registry: u32, view_value: Option<&d3d11::ID3D11ShaderResourceView>) {
    call_value(2);
    let v = match view_value {
        Some(v) => borrow(v),
        None => ManuallyDrop::new(None),
    };
    let s = core::slice::from_ref(&*v);
    unsafe {
        match stage {
            0 => ctx.VSSetShaderResources(registry, Some(s)),
            1 => ctx.PSSetShaderResources(registry, Some(s)),
            _ => ctx.CSSetShaderResources(registry, Some(s)),
        }
    }
}

unsafe fn place_sampler(ctx: &d3d11::ID3D11DeviceContext1, stage: usize, registry: u32, e: &d3d11::ID3D11SamplerState) {
    call_value(3);
    let v = borrow(e);
    let s = core::slice::from_ref(&*v);
    unsafe {
        match stage {
            0 => ctx.VSSetSamplers(registry, Some(s)),
            1 => ctx.PSSetSamplers(registry, Some(s)),
            _ => ctx.CSSetSamplers(registry, Some(s)),
        }
    }
}

/// Aspect of a copy -> wgpu aspect (block size of depth formats).
fn aspect(a: crate::FormatAspects) -> Option<wgt::TextureAspect> {
    if a.contains(crate::FormatAspects::DEPTH) && !a.contains(crate::FormatAspects::STENCIL) {
        Some(wgt::TextureAspect::DepthOnly)
    } else if a.contains(crate::FormatAspects::STENCIL) && !a.contains(crate::FormatAspects::DEPTH) {
        Some(wgt::TextureAspect::StencilOnly)
    } else {
        None
    }
}

/// Layout of a buffer <-> texture copy: (bytes per row of blocks, rows of blocks per
/// layer, useful bytes of a row, rows of blocks of the region).
fn layout_copy(format: wgt::TextureFormat, r: &crate::BufferTextureCopy) -> (u32, u32, u32, u32) {
    let (bw, bh) = format.block_dimensions();
    let size_block = format.block_copy_size(aspect(r.texture_base.aspect)).unwrap_or(4);
    let blocks_large = r.size.width.div_ceil(bw);
    let lines_list = r.size.height.div_ceil(bh);
    let useful = blocks_large * size_block;
    let by_line = r.buffer_layout.bytes_per_row.unwrap_or(useful);
    let by_layer = r.buffer_layout.rows_per_image.map(|h| h.div_ceil(bh)).unwrap_or(lines_list);
    (by_line, by_layer, useful, lines_list)
}

impl StateContext {
    /// Unbinds all read and write views (start of pass: a texture being read may
    /// become a target).
    unsafe fn unbind_reads(&mut self, ctx: &d3d11::ID3D11DeviceContext1) {
        for stage in 0..NB_STAGES {
            if let Some(last_one) = self.read_value[stage].iter().rposition(|&v| v != 0) {
                let empty_value: [Option<d3d11::ID3D11ShaderResourceView>; READ_COUNT] = Default::default();
                let s = &empty_value[..=last_one];
                unsafe {
                    match stage {
                        0 => ctx.VSSetShaderResources(0, Some(s)),
                        1 => ctx.PSSetShaderResources(0, Some(s)),
                        _ => ctx.CSSetShaderResources(0, Some(s)),
                    }
                }
                self.read_value[stage] = [0; READ_COUNT];
            }
        }
        unsafe { self.unbind_writes(ctx) };
    }

    /// Start of pass: only unbinds the read views of the resources that become targets (D3D11 would set them to
    /// null by itself, without the state tracked here knowing). Unbinding everything cost ~80 µs per pass on an
    /// older GPU, plus setting all the textures again afterwards.
    unsafe fn unbind_targets(&mut self, ctx: &d3d11::ID3D11DeviceContext1, targets_list: &[usize]) {
        static ALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *ALL.get_or_init(|| false) {
            return unsafe { self.unbind_reads(ctx) };
        }
        for stage in 0..NB_STAGES {
            for r in 0..READ_COUNT {
                let res = self.read_resource[stage][r];
                if res != 0 && self.read_value[stage][r] != 0 && targets_list.contains(&res) {
                    unsafe { read_place(ctx, stage, r as u32, None) };
                    self.read_value[stage][r] = 0;
                    self.read_resource[stage][r] = 0;
                }
            }
        }
        unsafe { self.unbind_writes(ctx) };
    }

    unsafe fn unbind_writes(&mut self, ctx: &d3d11::ID3D11DeviceContext1) {
        if let Some(last_one) = self.write_compute.iter().rposition(|&v| v != 0) {
            let empty_value: [Option<d3d11::ID3D11UnorderedAccessView>; NB_WRITE] = Default::default();
            unsafe { ctx.CSSetUnorderedAccessViews(0, last_one as u32 + 1, Some(empty_value.as_ptr()), None) };
            self.write_compute = [0; NB_WRITE];
        }
    }

    unsafe fn group_value(
        &mut self,
        ctx: &d3d11::ID3D11DeviceContext1,
        slots: &[super::Slot],
        bound_list: &[Bound],
        offsets: &[wgt::DynamicOffset],
        compute: bool,
    ) {
        for (emp, bound_entry) in slots.iter().zip(bound_list) {
            let stages_list: &[usize] = if compute {
                if emp.stages_list.contains(wgt::ShaderStages::COMPUTE) { &[2] } else { &[] }
            } else {
                match (emp.stages_list.contains(wgt::ShaderStages::VERTEX), emp.stages_list.contains(wgt::ShaderStages::FRAGMENT)) {
                    (true, true) => &[0, 1],
                    (true, false) => &[0],
                    (false, true) => &[1],
                    (false, false) => &[],
                }
            };
            let r = emp.registry;
            match bound_entry {
                Bound::Constants { buffer_value, shift_amount, size_value } => {
                    let dyn_ = emp.dynamic_rank.and_then(|i| offsets.get(i as usize)).copied().unwrap_or(0) as u64;
                    let first_one = ((shift_amount + dyn_) / 16) as u32;
                    let count_value = (rounded_256(*size_value) / 16).min(4096) as u32;
                    let a = address(buffer_value);
                    for &e in stages_list {
                        let tracking = (r as usize) < NB_CONSTANTS;
                        if tracking && self.constants[e][r as usize] == (a, first_one, count_value) {
                            continue;
                        }
                        unsafe { place_constants(ctx, e, r, buffer_value, first_one, count_value) };
                        if tracking {
                            self.constants[e][r as usize] = (a, first_one, count_value);
                        }
                    }
                }
                Bound::ReadSlot(view_value, resource) => {
                    let a = address(view_value);
                    for &e in stages_list {
                        let tracking = (r as usize) < READ_COUNT;
                        if tracking && self.read_value[e][r as usize] == a {
                            continue;
                        }
                        unsafe { read_place(ctx, e, r, Some(view_value)) };
                        if tracking {
                            self.read_value[e][r as usize] = a;
                            self.read_resource[e][r as usize] = *resource;
                        }
                    }
                }
                Bound::SamplerSlot(sample) => {
                    let a = address(sample);
                    for &e in stages_list {
                        let tracking = (r as usize) < NB_SAMPLERS;
                        if tracking && self.samplers[e][r as usize] == a {
                            continue;
                        }
                        unsafe { place_sampler(ctx, e, r, sample) };
                        if tracking {
                            self.samplers[e][r as usize] = a;
                        }
                    }
                }
                Bound::WriteSlot(view_value) => {
                    if compute {
                        let a = address(view_value);
                        if (r as usize) < NB_WRITE && self.write_compute[r as usize] == a {
                            continue;
                        }
                        let v = borrow(view_value);
                        unsafe { ctx.CSSetUnorderedAccessViews(r, 1, Some(core::ptr::from_ref(&*v)), None) };
                        if (r as usize) < NB_WRITE {
                            self.write_compute[r as usize] = a;
                        }
                    } else {
                        // Writes from pixel shaders (OMSetRenderTargetsAndUnorderedAccessViews):
                        // not used by OpenXIII; to do if a storage view is ever used for rendering.
                        log::warn!("Direct3D 11: writable view in a render pass ignored");
                    }
                }
            }
        }
    }

    unsafe fn pipeline_render(&mut self, ctx: &d3d11::ID3D11DeviceContext1, p: &Arc<RenderPipelineInternal>) {
        if self.render.as_ref().is_some_and(|a| Arc::ptr_eq(a, p)) {
            return;
        }
        let prior = self.render.take();
        let change = |f: &dyn Fn(&RenderPipelineInternal) -> usize| prior.as_ref().map_or(true, |a| f(a) != f(p));
        unsafe {
            if change(&|q| address(&q.vs)) {
                ctx.VSSetShader(&p.vs, None);
            }
            if change(&|q| q.ps.as_ref().map_or(0, address)) {
                ctx.PSSetShader(p.ps.as_ref(), None);
            }
            if change(&|q| q.layout_vertices.as_ref().map_or(0, address)) {
                ctx.IASetInputLayout(p.layout_vertices.as_ref());
            }
            if change(&|q| q.topology_value.0 as usize) {
                ctx.IASetPrimitiveTopology(p.topology_value);
            }
            if change(&|q| address(&q.blend_value)) || change(&|q| q.mask_samples as usize) {
                ctx.OMSetBlendState(&p.blend_value, Some(&self.blend_constants), p.mask_samples);
            }
            if change(&|q| address(&q.depth_value)) {
                ctx.OMSetDepthStencilState(&p.depth_value, self.ref_stencil);
            }
            if change(&|q| address(&q.frame)) {
                ctx.RSSetState(&p.frame);
            }
        }
        for (i, step_size) in self.vertices_stride.iter().enumerate() {
            let wanted = p.vertex_step.get(i).copied().unwrap_or(0);
            if self.vertices_list[i].is_some() && *step_size != wanted {
                self.vertices_dirty |= 1 << i;
            }
        }
        call_value(6);
        self.render = Some(p.clone());
    }

    /// Before a draw: pending vertex buffers, special constants.
    unsafe fn before_draw(&mut self, shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, first_one: i32, instance: u32) {
        call_value(7);
        let Some(p) = self.render.clone() else { return };
        while self.vertices_dirty != 0 {
            let i = self.vertices_dirty.trailing_zeros() as usize;
            self.vertices_dirty &= !(1 << i);
            let step_size = p.vertex_step.get(i).copied().unwrap_or(0);
            let shift_amount = self.vertices_offset[i];
            call_value(4);
            unsafe { ctx.IASetVertexBuffers(i as u32, 1, Some(core::ptr::from_ref(&self.vertices_list[i])), Some(&step_size), Some(&shift_amount)) };
            self.vertices_stride[i] = step_size;
        }
        if p.special_constants {
            unsafe { self.special(shared, ctx, [first_one as u32, instance, 0, 0], 0) };
        }
    }

    /// Writes and binds naga's special constants (register b13) if they change.
    unsafe fn special(&mut self, shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, value_list: [u32; 4], stage: usize) {
        if self.special.is_none() {
            let desc = d3d11::D3D11_BUFFER_DESC {
                ByteWidth: SPECIAL_SIZE,
                Usage: d3d11::D3D11_USAGE_DEFAULT,
                BindFlags: d3d11::D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                ..Default::default()
            };
            let mut b = None;
            if unsafe { shared.device.CreateBuffer(&desc, None, Some(&mut b)) }.is_err() {
                log::error!("Direct3D 11: cannot create the special constants buffer");
                return;
            }
            self.special = b;
        }
        let buffer_value = self.special.clone().unwrap();
        if value_list != self.special_values {
            // Whole buffer (no box): a partial update of a constant buffer is
            // ignored without a flag (see write_buffer).
            let mut everything = [0u32; SPECIAL_SIZE as usize / 4];
            everything[..4].copy_from_slice(&value_list);
            call_value(5);
            unsafe { ctx.UpdateSubresource1(&buffer_value, 0, None, everything.as_ptr().cast(), 0, 0, 0) };
            self.special_values = value_list;
        }
        let r = super::REGISTRY_SPECIAL_CONSTANTS as usize;
        let a = address(&buffer_value);
        if self.constants[stage][r] != (a, 0, 16) {
            unsafe { place_constants(ctx, stage, r as u32, &buffer_value, 0, 16) };
            self.constants[stage][r] = (a, 0, 16);
        }
    }
}

/// Writes `data_value` into a GPU buffer (and into its readback copy if it has one).
///
/// Constant buffer: D3D 11.1 only accepts a partial update (UpdateSubresource1 with a
/// box) with D3D11_COPY_NO_OVERWRITE or DISCARD, which do not respect the order of frames
/// still in flight (without a flag, nothing is written). The data therefore goes through a
/// buffer without binding (ordered update), then through a copy into the constant buffer,
/// also ordered with the draws.
unsafe fn write_buffer(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, dst: &Buffer, shift_amount: u64, data_value: &[u8]) {
    if data_value.is_empty() {
        return;
    }
    if dst.raw.is_some() && split_active() {
        let mut o = BYTES_COUNT.lock();
        let k = usize::from(dst.usage.contains(wgt::BufferUses::UNIFORM));
        o[k].0 += 1;
        o[k].1 += data_value.len() as u64;
    }
    if let Some(raw) = &dst.raw {
        let bbox = d3d11::D3D11_BOX { left: shift_amount as u32, top: 0, front: 0, right: (shift_amount + data_value.len() as u64) as u32, bottom: 1, back: 1 };
        if dst.usage.contains(wgt::BufferUses::UNIFORM) {
            let mut pass = shared.pass_constants.lock();
            if pass.as_ref().is_none_or(|p| p.1 < data_value.len() as u64) {
                let size_value = (data_value.len() as u64).next_power_of_two().max(64 << 10);
                let desc = d3d11::D3D11_BUFFER_DESC { ByteWidth: size_value as u32, Usage: d3d11::D3D11_USAGE_DEFAULT, ..Default::default() };
                let mut b = None;
                if unsafe { shared.device.CreateBuffer(&desc, None, Some(&mut b)) }.is_err() {
                    log::error!("Direct3D 11: cannot create the constants staging buffer");
                    return;
                }
                *pass = b.map(|b| (b, size_value));
            }
            let (p, _) = pass.as_ref().unwrap();
            let source = d3d11::D3D11_BOX { left: 0, top: 0, front: 0, right: data_value.len() as u32, bottom: 1, back: 1 };
            unsafe {
                ctx.UpdateSubresource1(p, 0, Some(&source), data_value.as_ptr().cast(), 0, 0, 0);
                ctx.CopySubresourceRegion(raw, 0, shift_amount as u32, 0, 0, p, 0, Some(&source));
            }
        } else {
            unsafe { ctx.UpdateSubresource1(raw, 0, Some(&bbox), data_value.as_ptr().cast(), 0, 0, 0) };
        }
        if let Some(read_value) = &dst.read_value {
            unsafe { ctx.CopySubresourceRegion(read_value, 0, shift_amount as u32, 0, 0, raw, 0, Some(&bbox)) };
        }
    } else if let Some(shadow) = &dst.shadow {
        let mut o = shadow.lock();
        o[shift_amount as usize..shift_amount as usize + data_value.len()].copy_from_slice(data_value);
    }
}

static FLUSHES_BEFORE_WRITES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// A submission that writes resources (queue.write_buffer / write_texture uploads, copies,
/// clears) while draws of an earlier submission are still unflushed on the context: the context
/// is flushed first. Some drivers (AMD Radeon HD 7500G, 2015 driver) let such a write reach the
/// destination before the earlier, unflushed draws have read it: in split screen, the top
/// view's HUD then used the bottom view's vertices and constants (written at the head of the
/// bottom view's submission). A legal D3D11 order, so the cost is paid only when it can matter:
/// one Flush per extra submission that writes (split-screen views after the first, portals),
/// none for a frame drawn in a single submission (Present flushes).
fn flush_before_writes() -> bool {
    true
}

fn writes_resources(command_buffers: &[&CommandBuffer]) -> bool {
    command_buffers.iter().any(|cb| {
        cb.commands.iter().any(|c| {
            matches!(
                c,
                Command::ClearBuffer { .. } | Command::CopyBuffer(_) | Command::CopyBufferToTexture(_) | Command::CopyTexture(_)
            )
        })
    })
}

/// Upload through the ring: write of a CPU buffer (wgpu-core writes) into a GPU buffer.
fn by_ring(src: &Buffer, dst: &Buffer) -> bool {
    src.raw.is_none() && src.shadow.is_some() && dst.raw.is_some()
}

/// Before replaying a submission: all its CPU → GPU writes copied at once into the upload ring
/// (DYNAMIC buffer, a single Map: NO_OVERWRITE in sequence, DISCARD when it is full; a written
/// area is never rewritten before the DISCARD, which gives new memory). The copies then take
/// their piece through CopySubresourceRegion, in their place in the order: one
/// UpdateSubresource per write cost ~13 µs on PC (character vertices, 17 MB/s;
unsafe fn prepare_sends(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, state_value: &mut StateContext, command_buffers: &[&CommandBuffer]) {
    static CUT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    state_value.sends.clear();
    if *CUT.get_or_init(|| false) {
        return;
    }
    let aligned = |n: u64| (n + 15) & !15;
    let mut total = 0u64;
    for cb in command_buffers {
        for c in &cb.commands {
            if let Command::CopyBuffer(b) = c {
                let (src, dst, region) = (&b.0, &b.1, &b.2);
                if by_ring(src, dst) {
                    total += aligned(region.size.get());
                }
            }
        }
    }
    if total == 0 {
        return;
    }
    if state_value.ring_value.as_ref().is_none_or(|a| a.1 < total) {
        let size_value = total.next_power_of_two().max(8 << 20);
        let desc = d3d11::D3D11_BUFFER_DESC {
            ByteWidth: size_value as u32,
            Usage: d3d11::D3D11_USAGE_DYNAMIC,
            BindFlags: d3d11::D3D11_BIND_VERTEX_BUFFER.0 as u32,
            CPUAccessFlags: d3d11::D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut b = None;
        if unsafe { shared.device.CreateBuffer(&desc, None, Some(&mut b)) }.is_err() {
            log::error!("Direct3D 11: cannot create the upload ring ({size_value} bytes)");
            state_value.ring_value = None;
            return;
        }
        state_value.ring_value = b.map(|b| (b, size_value));
        state_value.ring_pos = u64::MAX;
    }
    let (ring_value, size_value) = state_value.ring_value.clone().unwrap();
    let mode = if state_value.ring_pos.saturating_add(total) > size_value {
        state_value.ring_pos = 0;
        d3d11::D3D11_MAP_WRITE_DISCARD
    } else {
        d3d11::D3D11_MAP_WRITE_NO_OVERWRITE
    };
    let mut m = d3d11::D3D11_MAPPED_SUBRESOURCE::default();
    if unsafe { ctx.Map(&ring_value, 0, mode, 0, Some(&mut m)) }.is_err() {
        log::error!("Direct3D 11: cannot map the upload ring");
        return;
    }
    let base = m.pData.cast::<u8>();
    let mut pos = state_value.ring_pos;
    for cb in command_buffers {
        for c in &cb.commands {
            if let Command::CopyBuffer(b) = c {
                let (src, dst, region) = (&b.0, &b.1, &b.2);
                if by_ring(src, dst) {
                    let n = region.size.get();
                    let o = src.shadow.as_ref().unwrap().lock();
                    // SAFETY: pos + n ≤ size (total counted above, aligned to 16).
                    unsafe { core::ptr::copy_nonoverlapping(o[region.src_offset as usize..].as_ptr(), base.add(pos as usize), n as usize) };
                    state_value.sends.push_back(pos as u32);
                    pos += aligned(n);
                }
            }
        }
    }
    unsafe { ctx.Unmap(&ring_value, 0) };
    state_value.ring_pos = pos;
}

unsafe fn copy_buffer(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, state_value: &mut StateContext, src: &Buffer, dst: &Buffer, r: &crate::BufferCopy) {
    let size_value = r.size.get();
    // CPU buffer → GPU: writes from queue.write_buffer (wgpu-core's staging buffers).
    if src.raw.is_none() {
    }
    if by_ring(src, dst) {
        if let (Some(pos), Some((ring_value, _)), Some(d)) = (state_value.sends.pop_front(), &state_value.ring_value, &dst.raw) {
            let bbox = d3d11::D3D11_BOX { left: pos, top: 0, front: 0, right: pos + size_value as u32, bottom: 1, back: 1 };
            unsafe { ctx.CopySubresourceRegion(d, 0, r.dst_offset as u32, 0, 0, ring_value, 0, Some(&bbox)) };
            if let Some(read_value) = &dst.read_value {
                let b2 = d3d11::D3D11_BOX { left: r.dst_offset as u32, top: 0, front: 0, right: (r.dst_offset + size_value) as u32, bottom: 1, back: 1 };
                unsafe { ctx.CopySubresourceRegion(read_value, 0, r.dst_offset as u32, 0, 0, d, 0, Some(&b2)) };
            }
            if split_active() {
                let mut o = BYTES_COUNT.lock();
                let k = usize::from(dst.usage.contains(wgt::BufferUses::UNIFORM));
                o[k].0 += 1;
                o[k].1 += size_value;
            }
            return;
        }
    }
    match (&src.raw, &dst.raw) {
        (None, _) => {
            let shadow = src.shadow.as_ref().expect("Direct3D 11: buffer without contents");
            let o = shadow.lock();
            let piece = &o[r.src_offset as usize..(r.src_offset + size_value) as usize];
            unsafe { write_buffer(shared, ctx, dst, r.dst_offset, piece) };
        }
        (Some(s), Some(d)) => {
            let bbox = d3d11::D3D11_BOX { left: r.src_offset as u32, top: 0, front: 0, right: (r.src_offset + size_value) as u32, bottom: 1, back: 1 };
            unsafe { ctx.CopySubresourceRegion(d, 0, r.dst_offset as u32, 0, 0, s, 0, Some(&bbox)) };
            if let Some(read_value) = &dst.read_value {
                let b2 = d3d11::D3D11_BOX { left: r.dst_offset as u32, top: 0, front: 0, right: (r.dst_offset + size_value) as u32, bottom: 1, back: 1 };
                unsafe { ctx.CopySubresourceRegion(read_value, 0, r.dst_offset as u32, 0, 0, d, 0, Some(&b2)) };
            }
        }
        (Some(_), None) => super::to_do!("not supported: copy from a GPU buffer to a CPU buffer"),
    }
}

unsafe fn copy_buffer_to_texture(ctx: &d3d11::ID3D11DeviceContext1, src: &Buffer, dst: &Texture, r: &crate::BufferTextureCopy) {
    let Some(shadow) = &src.shadow else {
        super::to_do!("not supported: copy from a GPU buffer to a texture");
    };
    let o = shadow.lock();
    let (by_line, by_layer, _, _) = layout_copy(dst.format, r);
    let base = &r.texture_base;
    let three_d = dst.dimension == wgt::TextureDimension::D3;
    let layers = if three_d { 1 } else { r.size.depth };
    for z in 0..layers {
        let start_value = r.buffer_layout.offset as usize + (z * by_layer * by_line) as usize;
        let (sub, front, back) = if three_d {
            (dst.sub_resource(base.mip_level, 0), base.origin.z, base.origin.z + r.size.depth)
        } else {
            (dst.sub_resource(base.mip_level, base.array_layer + z), 0, 1)
        };
        let bbox = d3d11::D3D11_BOX {
            left: base.origin.x,
            top: base.origin.y,
            front,
            right: base.origin.x + r.size.width,
            bottom: base.origin.y + r.size.height,
            back,
        };
        unsafe { ctx.UpdateSubresource(&dst.raw, sub, Some(&bbox), o[start_value..].as_ptr().cast(), by_line, by_layer * by_line) };
    }
}

unsafe fn copy_texture_to_buffer(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, src: &Texture, dst: &Buffer, r: &crate::BufferTextureCopy) {
    let Ok(t2d) = src.raw.cast::<d3d11::ID3D11Texture2D>() else {
        super::to_do!("not supported: reading a non-2D texture");
    };
    let mut d = d3d11::D3D11_TEXTURE2D_DESC::default();
    unsafe { t2d.GetDesc(&mut d) };
    let desc = d3d11::D3D11_TEXTURE2D_DESC {
        Width: r.size.width,
        Height: r.size.height,
        MipLevels: 1,
        ArraySize: 1,
        Format: d.Format,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: d3d11::D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: d3d11::D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut pass = None;
    if unsafe { shared.device.CreateTexture2D(&desc, None, Some(&mut pass)) }.is_err() {
        log::error!("Direct3D 11: cannot create the readback texture");
        return;
    }
    let pass = pass.unwrap();
    let (by_line, by_layer, useful, lines_list) = layout_copy(src.format, r);
    let base = &r.texture_base;
    let mut buffer_value = vec![0u8; ((lines_list.max(1) - 1) * by_line + useful) as usize];
    for z in 0..r.size.depth {
        let bbox = d3d11::D3D11_BOX {
            left: base.origin.x,
            top: base.origin.y,
            front: 0,
            right: base.origin.x + r.size.width,
            bottom: base.origin.y + r.size.height,
            back: 1,
        };
        unsafe { ctx.CopySubresourceRegion(&pass, 0, 0, 0, 0, &src.raw, src.sub_resource(base.mip_level, base.array_layer + z), Some(&bbox)) };
        let mut m = d3d11::D3D11_MAPPED_SUBRESOURCE::default();
        if unsafe { ctx.Map(&pass, 0, d3d11::D3D11_MAP_READ, 0, Some(&mut m)) }.is_err() {
            log::error!("Direct3D 11: texture readback failed");
            return;
        }
        for y in 0..lines_list {
            // SAFETY: the staging texture has `lines_list` rows of `RowPitch` bytes.
            let line_value = unsafe { core::slice::from_raw_parts((m.pData as *const u8).add((y * m.RowPitch) as usize), useful as usize) };
            let o = (y * by_line) as usize;
            buffer_value[o..o + useful as usize].copy_from_slice(line_value);
        }
        unsafe { ctx.Unmap(&pass, 0) };
        let shift_amount = r.buffer_layout.offset + (z * by_layer * by_line) as u64;
        unsafe { write_buffer(shared, ctx, dst, shift_amount, &buffer_value) };
    }
}

unsafe fn copy_texture(ctx: &d3d11::ID3D11DeviceContext1, src: &Texture, dst: &Texture, r: &crate::TextureCopy) {
    let three_d = src.dimension == wgt::TextureDimension::D3;
    let layers = if three_d { 1 } else { r.size.depth };
    for z in 0..layers {
        let (s_sub, d_sub, front, back, dz) = if three_d {
            (src.sub_resource(r.src_base.mip_level, 0), dst.sub_resource(r.dst_base.mip_level, 0), r.src_base.origin.z, r.src_base.origin.z + r.size.depth, r.dst_base.origin.z)
        } else {
            (src.sub_resource(r.src_base.mip_level, r.src_base.array_layer + z), dst.sub_resource(r.dst_base.mip_level, r.dst_base.array_layer + z), 0, 1, 0)
        };
        let bbox = d3d11::D3D11_BOX {
            left: r.src_base.origin.x,
            top: r.src_base.origin.y,
            front,
            right: r.src_base.origin.x + r.size.width,
            bottom: r.src_base.origin.y + r.size.height,
            back,
        };
        unsafe { ctx.CopySubresourceRegion(&dst.raw, d_sub, r.dst_base.origin.x, r.dst_base.origin.y, dz, &src.raw, s_sub, Some(&bbox)) };
    }
}

unsafe fn clear_buffer(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, buffer_value: &Buffer, range_value: &crate::MemoryRange) {
    if buffer_value.raw.is_none() {
        if let Some(o) = &buffer_value.shadow {
            o.lock()[range_value.start as usize..range_value.end as usize].fill(0);
        }
        return;
    }
    let mut start_value = range_value.start;
    while start_value < range_value.end {
        let n = (range_value.end - start_value).min(super::ZERO_BUFFER_SIZE as u64);
        unsafe { write_buffer(shared, ctx, buffer_value, start_value, &shared.zeros[..n as usize]) };
        start_value += n;
    }
}

unsafe fn resolve(ctx: &d3d11::ID3D11DeviceContext1, r: &Resolution) {
    unsafe { ctx.ResolveSubresource(&r.dst, r.dst_sub, &r.src, r.src_sub, r.format) };
}

/// Replays a list of commands. The context is already locked by the caller.
const KINDS: [&str; 8] = ["copies", "pass start", "pipeline", "group", "buffers", "draw", "pass end", "other"];

fn kind_index(c: &Command) -> usize {
    match c {
        Command::ClearBuffer { .. }
        | Command::CopyBuffer(_)
        | Command::CopyBufferToTexture(_)
        | Command::CopyTextureToBuffer(_)
        | Command::CopyTexture(_) => 0,
        Command::StartPass { .. } => 1,
        Command::PipelineRender(_) => 2,
        Command::Group { .. } => 3,
        Command::BufferIndex { .. } | Command::BufferVertices { .. } => 4,
        Command::Draw { .. } | Command::DrawIndexed { .. } | Command::DrawIndirect { .. } => 5,
        Command::EndPass { .. } => 6,
        _ => 7,
    }
}

static SPLIT: parking_lot::Mutex<[(f64, u64); 8]> = parking_lot::Mutex::new([(0.0, 0); 8]);
const PHASES_PASS: [&str; 5] = ["unbind", "targets", "clear color", "clear depth", "viewport"];
static SPLIT_PASS: parking_lot::Mutex<[f64; 5]> = parking_lot::Mutex::new([0.0; 5]);

const CALLS_NAMES: [&str; 8] = ["VS constants", "PS constants", "views", "samplers", "vertices", "special", "pipelines", "draws"];
static CALLS: [core::sync::atomic::AtomicU64; 8] = [const { core::sync::atomic::AtomicU64::new(0) }; 8];

fn call_value(k: usize) {
    if split_active() {
        CALLS[k].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
}

fn mark_value(t: &mut Option<std::time::Instant>, phase: usize) {
    if let Some(start_value) = t {
        SPLIT_PASS.lock()[phase] += start_value.elapsed().as_secs_f64() * 1000.0;
        *t = Some(std::time::Instant::now());
    }
}

fn split_active() -> bool {
    static SPLIT_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SPLIT_ENABLED.get_or_init(|| false)
}

/// Writes (count, bytes): ordinary buffers, constant buffers.
static BYTES_COUNT: parking_lot::Mutex<[(u64, u64); 2]> = parking_lot::Mutex::new([(0, 0); 2]);

pub(super) unsafe fn execute(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, state_value: &mut StateContext, commands: &[Command]) {
    if split_active() {
        let mut d = [(0.0f64, 0u64); 8];
        for c in commands {
            let t = std::time::Instant::now();
            unsafe { execute_one(shared, ctx, state_value, c) };
            let k = kind_index(c);
            d[k].0 += t.elapsed().as_secs_f64() * 1000.0;
            d[k].1 += 1;
        }
        let mut g = SPLIT.lock();
        for (a, b) in g.iter_mut().zip(d) {
            a.0 += b.0;
            a.1 += b.1;
        }
        return;
    }
    for c in commands {
        unsafe { execute_one(shared, ctx, state_value, c) };
    }
}

unsafe fn execute_one(shared: &Shared, ctx: &d3d11::ID3D11DeviceContext1, state_value: &mut StateContext, c: &Command) {
    {
        match c {
            Command::ClearBuffer { buffer_value, range_value } => unsafe { clear_buffer(shared, ctx, buffer_value, range_value) },
            Command::CopyBuffer(c) => unsafe { copy_buffer(shared, ctx, state_value, &c.0, &c.1, &c.2) },
            Command::CopyBufferToTexture(c) => unsafe { copy_buffer_to_texture(ctx, &c.0, &c.1, &c.2) },
            Command::CopyTextureToBuffer(c) => unsafe { copy_texture_to_buffer(shared, ctx, &c.0, &c.1, &c.2) },
            Command::CopyTexture(c) => unsafe { copy_texture(ctx, &c.0, &c.1, &c.2) },

            Command::StartPass { colors, depth_value, size_value } => {
                state_value.drawn_since_flush = true;
                let mut t = split_active().then(std::time::Instant::now);
                state_value.target_count.0 += 1;
                let mut resources = [0usize; crate::MAX_COLOR_ATTACHMENTS + 1];
                for (i, c) in colors.iter().enumerate() {
                    resources[i] = c.as_ref().map_or(0, |c| c.resource);
                }
                resources[crate::MAX_COLOR_ATTACHMENTS] = depth_value.as_ref().map_or(0, |p| p.written_resource);
                unsafe { state_value.unbind_targets(ctx, &resources) };
                let mut targets_list: [Option<d3d11::ID3D11RenderTargetView>; crate::MAX_COLOR_ATTACHMENTS] = Default::default();
                let mut n = 0;
                for (i, c) in colors.iter().enumerate() {
                    if let Some(c) = c {
                        targets_list[i] = Some(c.rtv.clone());
                        n = i + 1;
                    }
                }
                let dsv = depth_value.as_ref().map(|p| &p.dsv);
                mark_value(&mut t, 0);
                // Targets already set (neighbouring passes on the same target: sky, map, comic panel, first-person
                // view): not set again.
                let mut placed = [0usize; crate::MAX_COLOR_ATTACHMENTS + 2];
                for (i, c) in targets_list[..n].iter().enumerate() {
                    placed[i] = c.as_ref().map_or(0, address);
                }
                placed[crate::MAX_COLOR_ATTACHMENTS] = dsv.map_or(0, address);
                placed[crate::MAX_COLOR_ATTACHMENTS + 1] = n;
                let generation = shared.targets_generation.load(core::sync::atomic::Ordering::Acquire);
                static ALWAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                let always = *ALWAYS.get_or_init(|| false);
                if always || state_value.targets_placed != Some((placed, generation)) {
                    unsafe { ctx.OMSetRenderTargets(Some(&targets_list[..n]), dsv) };
                    state_value.targets_placed = Some((placed, generation));
                    state_value.target_count.1 += 1;
                }
                mark_value(&mut t, 1);
                for c in colors.iter().flatten() {
                    if let Some(color_value) = &c.clear_color {
                        unsafe { ctx.ClearRenderTargetView(&c.rtv, color_value) };
                    }
                }
                mark_value(&mut t, 2);
                if let Some(p) = depth_value {
                    let mut flag_bits = 0;
                    if p.clear_depth.is_some() {
                        flag_bits |= d3d11::D3D11_CLEAR_DEPTH.0;
                    }
                    if p.clear_stencil.is_some() {
                        flag_bits |= d3d11::D3D11_CLEAR_STENCIL.0;
                    }
                    if flag_bits != 0 {
                        unsafe { ctx.ClearDepthStencilView(&p.dsv, flag_bits, p.clear_depth.unwrap_or(1.0), p.clear_stencil.unwrap_or(0)) };
                    }
                }
                mark_value(&mut t, 3);
                let window_value = d3d11::D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: size_value.width as f32, Height: size_value.height as f32, MinDepth: 0.0, MaxDepth: 1.0 };
                let rect = RECT { left: 0, top: 0, right: size_value.width as i32, bottom: size_value.height as i32 };
                unsafe {
                    ctx.RSSetViewports(Some(&[window_value]));
                    ctx.RSSetScissorRects(Some(&[rect]));
                }
                mark_value(&mut t, 4);
            }
            Command::EndPass { resolutions } => {
                for r in resolutions {
                    unsafe { resolve(ctx, r) };
                }
            }
            Command::PipelineRender(p) => {
                unsafe { state_value.pipeline_render(ctx, p) }
            }
            Command::Group { hint, layout_value, bound_list, offsets, compute } => {
                if let Some(slots) = layout_value.groups.get(*hint as usize) {
                    unsafe { state_value.group_value(ctx, slots, bound_list, offsets, *compute) };
                }
            }
            Command::BufferIndex { buffer_value, format, shift_amount } => {
                let key_tuple = (address(&**buffer_value), format.0, *shift_amount);
                if state_value.index != key_tuple {
                    unsafe { ctx.IASetIndexBuffer(&**buffer_value, *format, *shift_amount) };
                    state_value.index = key_tuple;
                }
            }
            Command::BufferVertices { slot, buffer_value, shift_amount } => {
                let i = *slot as usize;
                if state_value.vertices_list[i].as_ref().map(address) != Some(address(&**buffer_value)) || state_value.vertices_offset[i] != *shift_amount {
                    state_value.vertices_list[i] = Some((**buffer_value).clone());
                    state_value.vertices_offset[i] = *shift_amount;
                    state_value.vertices_dirty |= 1 << i;
                }
            }
            Command::Window { rect, depth_value } => {
                let f = d3d11::D3D11_VIEWPORT { TopLeftX: rect.x, TopLeftY: rect.y, Width: rect.w, Height: rect.h, MinDepth: depth_value.start, MaxDepth: depth_value.end };
                unsafe { ctx.RSSetViewports(Some(&[f])) };
            }
            Command::Scissor { rect } => {
                let r = RECT { left: rect.x as i32, top: rect.y as i32, right: (rect.x + rect.w) as i32, bottom: (rect.y + rect.h) as i32 };
                unsafe { ctx.RSSetScissorRects(Some(&[r])) };
            }
            Command::RefStencil(v) => {
                if state_value.ref_stencil != *v {
                    state_value.ref_stencil = *v;
                    if let Some(p) = &state_value.render {
                        unsafe { ctx.OMSetDepthStencilState(&p.depth_value, *v) };
                    }
                }
            }
            Command::ConstantsBlend(c) => {
                if state_value.blend_constants != *c {
                    state_value.blend_constants = *c;
                    if let Some(p) = &state_value.render {
                        unsafe { ctx.OMSetBlendState(&p.blend_value, Some(c), p.mask_samples) };
                    }
                }
            }
            Command::Draw { first_vertex_value, nb_vertices, first_instance_value, nb_instances } => unsafe {
                state_value.before_draw(shared, ctx, *first_vertex_value as i32, *first_instance_value);
                ctx.DrawInstanced(*nb_vertices, *nb_instances, *first_vertex_value, *first_instance_value);
            },
            Command::DrawIndexed { first_index_value, nb_index, vertex_base, first_instance_value, nb_instances } => unsafe {
                state_value.before_draw(shared, ctx, *vertex_base, *first_instance_value);
                ctx.DrawIndexedInstanced(*nb_index, *nb_instances, *first_index_value, *vertex_base, *first_instance_value);
            },
            Command::DrawIndirect { buffer_value, shift_amount, nb, indexed } => {
                // The bases of indirect draws are not known here: special constants at 0.
                unsafe { state_value.before_draw(shared, ctx, 0, 0) };
                let step_size = if *indexed { 20 } else { 16 };
                for i in 0..*nb {
                    unsafe {
                        if *indexed {
                            ctx.DrawIndexedInstancedIndirect(&**buffer_value, shift_amount + i * step_size);
                        } else {
                            ctx.DrawInstancedIndirect(&**buffer_value, shift_amount + i * step_size);
                        }
                    }
                }
            }

            Command::PipelineCompute(p) => {
                state_value.drawn_since_flush = true;
                if !state_value.compute.as_ref().is_some_and(|a| Arc::ptr_eq(a, p)) {
                    unsafe { ctx.CSSetShader(&p.cs, None) };
                    state_value.compute = Some(p.clone());
                }
            }
            Command::Compute(n) => {
                if state_value.compute.as_ref().is_some_and(|p| p.special_constants) {
                    unsafe { state_value.special(shared, ctx, [n[0], n[1], n[2], 0], 2) };
                }
                unsafe { ctx.Dispatch(n[0], n[1], n[2]) };
            }
            Command::ComputeIndirect { buffer_value, shift_amount } => unsafe { ctx.DispatchIndirect(&**buffer_value, *shift_amount) },
            Command::EndCompute => unsafe { state_value.unbind_writes(ctx) },

            Command::StartQuery { query } => unsafe { ctx.Begin(query) },
            Command::EndQuery { query } | Command::Timestamp { query } => unsafe { ctx.End(query) },
            Command::CopyResultsQueries(r) => unsafe {
                super::sync::write_results(shared, ctx, &r.queries, r.ty, &r.dst, r.shift_amount, r.step_size)
            },

            // Labels: nothing (ID3DUserDefinedAnnotation costs even with no tool listening).
            Command::StartGroupDebug(name_value) => {
            }
            Command::EndGroupDebug => {
            }
            Command::Marker(_) => {}
        }
    }
}

/// D3D11 debug layer (InstanceFlags::VALIDATION and graphics tools installed): its
/// messages go to the log after each submission.
unsafe fn log_debug(shared: &Shared) {
    let Ok(file) = shared.device.cast::<d3d11::ID3D11InfoQueue>() else { return };
    let n = unsafe { file.GetNumStoredMessages() };
    for i in 0..n.min(64) {
        let mut size_value = 0usize;
        if unsafe { file.GetMessage(i, None, &mut size_value) }.is_err() || size_value == 0 {
            continue;
        }
        let mut place = vec![0u64; size_value.div_ceil(8)];
        let m = place.as_mut_ptr().cast::<d3d11::D3D11_MESSAGE>();
        if unsafe { file.GetMessage(i, Some(m), &mut size_value) }.is_ok() {
            // SAFETY: GetMessage has filled the message and its description in `place`.
            let m = unsafe { &*m };
            let text_value = unsafe { core::slice::from_raw_parts(m.pDescription, m.DescriptionByteLength) };
            log::warn!("D3D11 [{:?}] {}", m.Severity, alloc::string::String::from_utf8_lossy(text_value).trim_end_matches('\0'));
        }
    }
    unsafe { file.ClearStoredMessages() };
}

fn count_targets(state_value: &mut StateContext) {
    let _c = core::mem::take(&mut state_value.target_count);
}

/// Sampled every 2 s in the wgpu log (OXIII_LOG_WGPU=1), or, as soon as the renderer calls
/// `record_hal`, per frame in its "perf hal" lines (game time windows).
static HAL: parking_lot::Mutex<Option<(std::time::Instant, f64, u64, u64)>> = parking_lot::Mutex::new(None);
static RECORD_BY_RENDER: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn timer_hal(ms: f64, orders: usize) {
    static ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ACTIVE.get_or_init(|| false) {
        return;
    }
    let duration = {
        let mut c = HAL.lock();
        let (start_value, total, submissions, n) = c.get_or_insert((std::time::Instant::now(), 0.0, 0, 0));
        *total += ms;
        *submissions += 1;
        *n += orders as u64;
        start_value.elapsed().as_secs_f64()
    };
    if duration >= 2.0 && !RECORD_BY_RENDER.load(core::sync::atomic::Ordering::Relaxed) {
        for l in record(duration, "/s") {
            log::warn!("{l}");
        }
    }
}

/// Lines of the sample (values divided by `div`, seconds or frames), counters reset to zero.
fn record(div: f64, unit: &str) -> alloc::vec::Vec<alloc::string::String> {
    use alloc::{format, string::String, vec::Vec};
    let mut lines_list = Vec::new();
    let (total, submissions, n) = HAL.lock().take().map_or((0.0, 0, 0), |c| (c.1, c.2, c.3));
    let flushes = FLUSHES_BEFORE_WRITES.swap(0, core::sync::atomic::Ordering::Relaxed);
    lines_list.push(format!(
        "hal submission{unit}: time {:.3} ms, submissions {:.2}, commands {:.1}, flushes before writes {:.2}",
        total / div,
        submissions as f64 / div,
        n as f64 / div,
        flushes as f64 / div
    ));
    let mut d = SPLIT.lock();
    if d.iter().any(|x| x.1 > 0) {
        let t: Vec<String> = KINDS.iter().zip(d.iter()).map(|(s, (ms, n))| format!("{s} {:.3} ms ({:.0})", ms / div, *n as f64 / div)).collect();
        lines_list.push(format!("hal breakdown{unit}: {}", t.join(", ")));
        *d = [(0.0, 0); 8];
        let mut o = BYTES_COUNT.lock();
        lines_list.push(format!(
            "hal writes{unit}: plain {:.1} {:.0} KiB, constants {:.1} {:.0} KiB",
            o[0].0 as f64 / div,
            o[0].1 as f64 / div / 1024.0,
            o[1].0 as f64 / div,
            o[1].1 as f64 / div / 1024.0
        ));
        *o = [(0, 0); 2];
        let mut dp = SPLIT_PASS.lock();
        let t: Vec<String> = PHASES_PASS.iter().zip(dp.iter()).map(|(s, ms)| format!("{s} {:.3}", ms / div)).collect();
        lines_list.push(format!("hal pass start{unit} (ms): {}", t.join(", ")));
        *dp = [0.0; 5];
        let t: Vec<String> = CALLS_NAMES.iter().zip(CALLS.iter()).map(|(s, n)| format!("{s} {:.1}", n.swap(0, core::sync::atomic::Ordering::Relaxed) as f64 / div)).collect();
        lines_list.push(format!("hal calls{unit}: {}", t.join(", ")));
    }
    lines_list
}

impl crate::Queue for Queue {
    type A = Api;

    unsafe fn submit(
        &self,
        command_buffers: &[&CommandBuffer],
        _surface_textures: &[&SurfaceTexture],
        (fence, fence_value): (&Fence, crate::FenceValue),
    ) -> DeviceResult<()> {
        let ctx = self.shared.context.lock();
        let mut state_value = self.state_value.lock();
        let start_value = std::time::Instant::now();
        let mut n = 0;
        // Targets kept only from one pass to the next within the same submission (in between, Present
        // or a resize may unbind them).
        state_value.targets_placed = None;
        if state_value.drawn_since_flush && writes_resources(command_buffers) && flush_before_writes() {
            unsafe { ctx.Flush() };
            state_value.drawn_since_flush = false;
            FLUSHES_BEFORE_WRITES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        unsafe { prepare_sends(&self.shared, &ctx, &mut state_value, command_buffers) };
        for cb in command_buffers {
            n += cb.commands.len();
            unsafe { execute(&self.shared, &ctx, &mut state_value, &cb.commands) };
        }
        let ms = start_value.elapsed().as_secs_f64() * 1000.0;
        timer_hal(ms, n);
        count_targets(&mut state_value);
        if ms >= 20.0 {
            super::timer_creation("submission", Some(&alloc::format!("{n} commands")), start_value);
        }
        unsafe { log_debug(&self.shared) };
        unsafe { super::sync::report(&self.shared, &ctx, fence, fence_value) }
    }

    unsafe fn present(&self, surface: &Surface, texture: SurfaceTexture) -> Result<(), crate::SurfaceError> {
        let _ctx = self.shared.context.lock();
        // DXGI's flip model unbinds the back buffer on Present: targets to set again.
        {
            let mut e = self.state_value.lock();
            e.targets_placed = None;
            // Present flushes the immediate context.
            e.drawn_since_flush = false;
        }
        let r = unsafe { super::surface::present_value(surface, texture) };
        r
    }

    unsafe fn get_timestamp_period(&self) -> f32 {
        unsafe { super::sync::period_timestamp(&self.shared) }
    }

    unsafe fn wait_for_idle(&self) -> Result<(), crate::DeviceError> {
        unsafe { super::sync::wait_all(&self.shared) }
    }
}
