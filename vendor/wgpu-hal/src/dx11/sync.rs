//! Synchronization of D3D11 submissions through EVENT queries, and reading of the results.

use alloc::vec::Vec;
use core::{ptr, sync::atomic::Ordering, time::Duration};
use std::{thread, time::Instant};

use arrayvec::ArrayVec;
use windows::{
    core::{Interface, BOOL, GUID},
    Win32::{
        Foundation::{S_FALSE, S_OK},
        Graphics::Direct3D11::*,
    },
};

use super::{Buffer, Device, DeviceResult, Fence, QuerySet, Shared};
use crate::{auxil::dxgi::result::HResult, DeviceError, FenceValue};

fn create_query(shared: &Shared, ty: D3D11_QUERY) -> DeviceResult<ID3D11Query> {
    let desc = D3D11_QUERY_DESC {
        Query: ty,
        MiscFlags: 0,
    };
    let mut query = None;
    unsafe { shared.device.CreateQuery(&desc, Some(&mut query)) }.into_device_result("CreateQuery")?;
    query.ok_or(DeviceError::Unexpected)
}

// T must be the result type of this query. The windows-rs GetData wrapper
// confuses S_FALSE (pending) and S_OK: keep the COM method's HRESULT.
unsafe fn probe<T: Default>(ctx: &ID3D11DeviceContext, query: &ID3D11Query) -> DeviceResult<Option<T>> {
    let mut result_value = T::default();
    let hr = unsafe {
        (ctx.vtable().GetData)(
            ctx.as_raw(),
            query.as_raw(),
            ptr::from_mut(&mut result_value).cast(),
            size_of::<T>() as u32,
            D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
        )
    };
    match hr {
        S_OK => Ok(Some(result_value)),
        S_FALSE => Ok(None),
        _ => {
            hr.ok().into_device_result("GetData")?;
            Err(DeviceError::Unexpected)
        }
    }
}

fn finished_event(ctx: &ID3D11DeviceContext, query: &ID3D11Query) -> DeviceResult<bool> {
    Ok(unsafe { probe::<BOOL>(ctx, query)? }.is_some_and(|v| v.as_bool()))
}

const PAUSE: Duration = Duration::from_micros(200);

fn wait_retry(test_value: &mut u8, rest: Duration) {
    if *test_value < 4 {
        *test_value += 1;
        thread::yield_now();
    } else {
        thread::sleep(rest);
    }
}

// Finished queries stay at the start of the vector, in value order. At most four are
// kept for the next submissions, without sharing objects between devices.
fn record(
    ctx: &ID3D11DeviceContext,
    fence: &Fence,
    inputs: &mut Vec<(FenceValue, ID3D11Query)>,
) -> DeviceResult<FenceValue> {
    let mut value_value = fence.reach_result.load(Ordering::Acquire);
    let mut finished = inputs.partition_point(|(v, _)| *v <= value_value);
    for (v, query) in &inputs[finished..] {
        if !finished_event(ctx, query)? {
            break;
        }
        value_value = *v;
        finished += 1;
    }
    fence.reach_result.store(value_value, Ordering::Release);
    inputs.drain(..finished.saturating_sub(4));
    Ok(value_value)
}

/// End of a submission: the value is only noted. The EVENT query that covers it is set
/// at the next poll (`emit`), a single one for all the submissions made between two polls:
/// setting and polling a query on every submission cost ~0.2 ms per submission on PC,
/// and a frame has several of them (shadows, comic panels).
pub(super) unsafe fn report(
    _shared: &Shared,
    ctx: &ID3D11DeviceContext1,
    fence: &Fence,
    value_value: FenceValue,
) -> DeviceResult<()> {
    fence.requested.fetch_max(value_value, Ordering::AcqRel);
    // No Flush on every submission: Present flushes the context, and `wait_value` does a Flush
    // before waiting (readbacks, captures).
    static EMPTY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *EMPTY.get_or_init(|| false) {
        unsafe { ctx.Flush() };
    }
    Ok(())
}

/// Sets the query for the highest submitted value if it is not set yet (context locked).
fn emit(shared: &Shared, ctx: &ID3D11DeviceContext, fence: &Fence, inputs: &mut Vec<(FenceValue, ID3D11Query)>) -> DeviceResult<()> {
    let value_value = fence.requested.load(Ordering::Acquire);
    let reach_result = fence.reach_result.load(Ordering::Acquire);
    if value_value <= inputs.last().map_or(reach_result, |(v, _)| *v) {
        return Ok(());
    }
    let query = if inputs.first().is_some_and(|(v, _)| *v <= reach_result) {
        inputs.remove(0).1
    } else {
        create_query(shared, D3D11_QUERY_EVENT)?
    };
    unsafe { ctx.End(&query) };
    inputs.push((value_value, query));
    Ok(())
}

/// Reached value. wgpu-core asks for it after every submission: the driver is polled
/// at most once per millisecond (otherwise the last polled value is returned); `wait_value`
/// always polls.
pub(super) unsafe fn value_reached(device: &Device, fence: &Fence) -> DeviceResult<FenceValue> {
    {
        let mut last_one = fence.last_record.lock();
        if last_one.is_some_and(|t| t.elapsed() < Duration::from_millis(1)) {
            return Ok(fence.reach_result.load(Ordering::Acquire));
        }
        *last_one = Some(Instant::now());
    }
    let ctx = device.shared.context.lock();
    let mut inputs = fence.in_wait.lock();
    emit(&device.shared, &ctx, fence, &mut inputs)?;
    record(&ctx, fence, &mut inputs)
}

pub(super) unsafe fn wait_value(
    device: &Device,
    fence: &Fence,
    value_value: FenceValue,
    wait_delay: Option<Duration>,
) -> DeviceResult<bool> {
    let start_value = Instant::now();
    let mut test_value = 0;
    let mut first_one = true;
    loop {
        if fence.reach_result.load(Ordering::Acquire) >= value_value {
            return Ok(true);
        }
        {
            let ctx = match wait_delay {
                Some(wait_delay) => {
                    let Some(ctx) = device
                        .shared
                        .context
                        .try_lock_for(wait_delay.saturating_sub(start_value.elapsed()))
                    else {
                        return Ok(fence.reach_result.load(Ordering::Acquire) >= value_value);
                    };
                    ctx
                }
                None => device.shared.context.lock(),
            };
            let mut inputs = fence.in_wait.lock();
            emit(&device.shared, &ctx, fence, &mut inputs)?;
            if record(&ctx, fence, &mut inputs)? >= value_value {
                return Ok(true);
            }
            drop(inputs);
            if first_one {
                unsafe { ctx.Flush() };
                first_one = false;
            }
        }
        let remaining = wait_delay.map_or(PAUSE, |d| d.saturating_sub(start_value.elapsed()));
        if remaining.is_zero() {
            return Ok(false);
        }
        wait_retry(&mut test_value, remaining.min(PAUSE));
    }
}

pub(super) unsafe fn wait_all(shared: &Shared) -> DeviceResult<()> {
    let query = create_query(shared, D3D11_QUERY_EVENT)?;
    {
        let ctx = shared.context.lock();
        unsafe {
            ctx.End(&query);
            ctx.Flush();
        }
    }
    let mut test_value = 0;
    while !finished_event(&shared.context.lock(), &query)? {
        wait_retry(&mut test_value, PAUSE);
    }
    Ok(())
}

pub(super) unsafe fn create_queries(
    device: &Device,
    desc: &wgt::QuerySetDescriptor<crate::Label>,
) -> DeviceResult<QuerySet> {
    let ty = match desc.ty {
        wgt::QueryType::Occlusion => D3D11_QUERY_OCCLUSION,
        wgt::QueryType::Timestamp => D3D11_QUERY_TIMESTAMP,
        wgt::QueryType::PipelineStatistics(_) => D3D11_QUERY_PIPELINE_STATISTICS,
    };
    let raw = (0..desc.count)
        .map(|_| create_query(&device.shared, ty))
        .collect::<DeviceResult<_>>()?;
    Ok(QuerySet { raw, ty: desc.ty })
}

// The caller has already issued End. Flush the commands only once if the result is late.
unsafe fn read_from<T: Default>(ctx: &ID3D11DeviceContext, query: &ID3D11Query) -> DeviceResult<T> {
    let mut test_value = 0;
    loop {
        if let Some(value_value) = unsafe { probe(ctx, query)? } {
            return Ok(value_value);
        }
        if test_value == 0 {
            unsafe { ctx.Flush() };
        }
        wait_retry(&mut test_value, PAUSE);
    }
}

// The private data belongs to the COM device and disappears with it.
const KEY_PERIOD: GUID = GUID::from_u128(0x5b4ce975_3ca3_4a5a_a716_e9444e6ded42);

fn measure_period(shared: &Shared) -> DeviceResult<f32> {
    let ctx = shared.context.lock();
    let mut period = 0.0f32;
    let mut size_value = size_of::<f32>() as u32;
    if unsafe {
        shared
            .device
            .GetPrivateData(&KEY_PERIOD, &mut size_value, Some(ptr::from_mut(&mut period).cast()))
    }
    .is_ok()
    {
        return Ok(period);
    }

    let query = create_query(shared, D3D11_QUERY_TIMESTAMP_DISJOINT)?;
    unsafe {
        ctx.Begin(&query);
        ctx.End(&query);
    }
    let measure: D3D11_QUERY_DATA_TIMESTAMP_DISJOINT = unsafe { read_from(&ctx, &query)? };
    if measure.Disjoint.as_bool() || measure.Frequency == 0 {
        return Ok(0.0); // The next request can measure again.
    }
    period = (1e9 / measure.Frequency as f64) as f32;
    unsafe {
        shared.device.SetPrivateData(
            &KEY_PERIOD,
            size_of::<f32>() as u32,
            Some(ptr::from_ref(&period).cast()),
        )
    }
    .into_device_result("SetPrivateData(period)")?;
    Ok(period)
}

pub(super) unsafe fn period_timestamp(shared: &Shared) -> f32 {
    measure_period(shared).unwrap_or(0.0)
}

fn counters(
    ctx: &ID3D11DeviceContext,
    query: &ID3D11Query,
    ty: wgt::QueryType,
) -> DeviceResult<ArrayVec<u64, 5>> {
    use wgt::PipelineStatisticsTypes as Stat;
    let mut words_list = ArrayVec::new();
    match ty {
        wgt::QueryType::Occlusion => words_list.push(u64::from(unsafe { read_from::<u64>(ctx, query)? } != 0)),
        wgt::QueryType::Timestamp => words_list.push(unsafe { read_from::<u64>(ctx, query)? }),
        wgt::QueryType::PipelineStatistics(flags) => {
            let stats: D3D11_QUERY_DATA_PIPELINE_STATISTICS = unsafe { read_from(ctx, query)? };
            for (flag, value_value) in [
                (Stat::VERTEX_SHADER_INVOCATIONS, stats.VSInvocations),
                (Stat::CLIPPER_INVOCATIONS, stats.CInvocations),
                (Stat::CLIPPER_PRIMITIVES_OUT, stats.CPrimitives),
                (Stat::FRAGMENT_SHADER_INVOCATIONS, stats.PSInvocations),
                (Stat::COMPUTE_SHADER_INVOCATIONS, stats.CSInvocations),
            ] {
                if flags.contains(flag) {
                    words_list.push(value_value);
                }
            }
        }
    }
    Ok(words_list)
}

/// queue.rs already holds the context's lock. wgpu-core separately clears the
/// slots that were never issued and only passes queries ended by End here.
pub(super) unsafe fn write_results(
    _shared: &Shared,
    ctx: &ID3D11DeviceContext1,
    queries: &[ID3D11Query],
    ty: wgt::QueryType,
    dst: &Buffer,
    shift_amount: u64,
    step_size: u64,
) {
    for (i, query) in queries.iter().enumerate() {
        let words_list = match counters(ctx, query, ty) {
            Ok(words_list) => words_list,
            Err(err) => {
                log::error!("Reading Direct3D 11 query results: {err}");
                return;
            }
        };
        let bytes_value: &[u8] = bytemuck::cast_slice(&words_list);
        if bytes_value.is_empty() {
            continue;
        }
        let start_value = shift_amount + i as u64 * step_size;
        let end_value = start_value + bytes_value.len() as u64;
        // Ranges are validated by wgpu-core; D3D11 buffer sizes fit in a UINT.
        debug_assert!(end_value <= dst.size && end_value <= u64::from(u32::MAX));
        if let Some(shadow) = &dst.shadow {
            shadow.lock()[start_value as usize..end_value as usize].copy_from_slice(bytes_value);
        } else if let Some(raw) = &dst.raw {
            let zone = D3D11_BOX {
                left: start_value as u32,
                right: end_value as u32,
                top: 0,
                bottom: 1,
                front: 0,
                back: 1,
            };
            unsafe { ctx.UpdateSubresource1(raw, 0, Some(&zone), bytes_value.as_ptr().cast(), 0, 0, 0) };
        }
    }
}
