//! Shaders: naga -> HLSL (shader model 5.0) -> FXC bytecode.
//!
//! - naga options (`naga::back::hlsl::Options`): `shader_model: ShaderModel::V5_0`,
//!   `binding_map: bindings.map_naga.clone()` (registers fixed by pipelines.rs),
//!   `fake_missing_bindings: false`, `special_constants_binding: Some(BindTarget { space: 0,
//!   register: REGISTRY_SPECIAL_CONSTANTS, .. })`: naga reads first vertex / first instance there
//!   (D3D's SV_VertexID and SV_InstanceID do not include the bases) and the number of workgroups in compute.
//!   `restrict_indexing` / `force_loop_bounding` follow the module's `runtime_checks`, like dx12.
//!   `Compile::special_constants` = the HLSL reads `_NagaConstants.` (vertex_index, instance_index
//!   or num_workgroups).
//! - Pipeline constants (`step.constants`): `naga::back::pipeline_constants::process_overrides`.
//! - `compile_vertices`: the vertex stage receives the fragment entry point (`hlsl::FragmentEntryPoint`,
//!   like dx12's `load_shader`); naga then only keeps as outputs what the fragment reads, in the
//!   same register order. Without this the fragment's signature does not line up in D3D11.
//! - naga 30 writes two things for SM 5.1 that FXC refuses in SM 5.0; `adapt_sm50` rewrites them:
//!   * samplers go through a heap (`nagaSamplerHeap[2048]` + `StructuredBuffer<uint>`
//!     of indices per group): becomes `SamplerState x : register(sN)`, N = register from `map_naga`;
//!     sampler arrays (binding_array) are refused (clean error);
//!   * `ConstantBuffer<T> name: register(bN);` (special constants) becomes
//!     `cbuffer nameCb : register(bN) { T name; }`; the `name.field` accesses stay valid.
//!   Immediate constants (push constants) are refused: no register is provided.
//! - Compilation: D3DCompile (d3dcompiler_47.dll, always present on Windows 10) with the target
//!   vs_5_0 / ps_5_0 / cs_5_0, ENABLE_STRICTNESS | OPTIMIZATION_LEVEL3. (DEBUG + SKIP_OPTIMIZATION
//!   with InstanceFlags::DEBUG: not done, `Shared` does not keep the instance's flags.)
//!   Error -> PipelineError::Linkage(stage, FXC message), HLSL in the log (log::error!).
//! - Private cache (`CACHE`): (final HLSL, entry point, target) -> bytecode; the same compilation requested
//!   by several pipeline variants only calls FXC once.
//! - Vertex input: the VS input signature (for CreateInputLayout) is the bytecode itself;
//!   naga names the attributes `LOC0`, `LOC1`… (LOC semantic, index = location): pipelines.rs uses it.

use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write as _;

use naga::back::hlsl;
use parking_lot::Mutex;
use windows::{
    core::PCSTR,
    Win32::Graphics::Direct3D::{Fxc, ID3DBlob},
};

use super::{Device, LayoutBindings, ShaderModule, REGISTRY_SPECIAL_CONSTANTS};

/// Result of a compilation.
#[derive(Clone)]
pub(super) struct Compile {
    pub bytecode: Vec<u8>,
    /// The shader reads the special constants (register REGISTRY_SPECIAL_CONSTANTS).
    pub special_constants: bool,
}

/// HLSL semantic of the vertex attributes written by naga (index = location): `LOC{n}`
/// (naga 30, back/hlsl/writer.rs, LOCATION_SEMANTIC).
pub(super) const SEMANTIC_VERTICES: &core::ffi::CStr = c"LOC";

/// Space given to naga's sampler index buffers, to recognize their lines and
/// remove them (they do not exist in D3D11).
const SPACE_INDICES_SAMPLERS: u8 = u8::MAX;
const HEAP_SAMPLERS: &str = "nagaSamplerHeap";
const HEAP_SAMPLERS_COMPARISON: &str = "nagaComparisonSamplerHeap";
/// Name of the special constants variable written by naga (writer.rs, SPECIAL_CBUF_VAR).
const VAR_SPECIAL_CONSTANTS: &str = "_NagaConstants";

/// Compilation cache: (final HLSL, entry point, target) -> result. OpenXIII creates many
/// pipeline variants at load time with the same shaders.
static CACHE: Mutex<BTreeMap<(String, String, &'static str), Compile>> = Mutex::new(BTreeMap::new());
/// Beyond this, the cache is emptied (memory safeguard).
const CACHE_MAX: usize = 4096;

pub(super) unsafe fn create_module(
    device: &Device,
    desc: &crate::ShaderModuleDescriptor,
    shader: crate::ShaderInput,
) -> Result<ShaderModule, crate::ShaderError> {
    match shader {
        crate::ShaderInput::Naga(naga) => Ok(ShaderModule { naga, runtime_checks: desc.runtime_checks }),
        _ => Err(crate::ShaderError::Compilation(String::from("Direct3D 11: naga shaders only"))),
    }
}

/// Compiles a stage for a given binding layout (fragment, compute, or vertex without
/// reducing its outputs).
pub(super) fn compile_stage(
    device: &Device,
    step_value: &crate::ProgrammableStage<ShaderModule>,
    naga_stage: naga::ShaderStage,
    bindings_list: &LayoutBindings,
) -> Result<Compile, crate::PipelineError> {
    compile_with(step_value, naga_stage, None, bindings_list)
}

/// Compiles the vertex stage of a render pipeline, reduced to the inputs of `fragment` if there is one.
pub(super) fn compile_vertices(
    device: &Device,
    vertices_list: &crate::ProgrammableStage<ShaderModule>,
    fragment: Option<&crate::ProgrammableStage<ShaderModule>>,
    bindings_list: &LayoutBindings,
) -> Result<Compile, crate::PipelineError> {
    let ep = match fragment {
        Some(fs) => Some(
            hlsl::FragmentEntryPoint::new(&fs.module.naga.module, fs.entry_point)
                .ok_or(crate::PipelineError::EntryPoint(naga::ShaderStage::Fragment))?,
        ),
        None => None,
    };
    compile_with(vertices_list, naga::ShaderStage::Vertex, ep.as_ref(), bindings_list)
}

fn compile_with(
    step_value: &crate::ProgrammableStage<ShaderModule>,
    naga_stage: naga::ShaderStage,
    fragment: Option<&hlsl::FragmentEntryPoint<'_>>,
    bindings_list: &LayoutBindings,
) -> Result<Compile, crate::PipelineError> {
    let stage = crate::auxil::map_naga_stage(naga_stage);
    let naga_shader = &step_value.module.naga;
    let checks = &step_value.module.runtime_checks;

    let (module, info) = naga::back::pipeline_constants::process_overrides(
        &naga_shader.module,
        &naga_shader.info,
        Some((naga_stage, step_value.entry_point)),
        step_value.constants,
    )
    .map_err(|e| crate::PipelineError::PipelineConstants(stage, format!("HLSL: {e:?}")))?;

    // naga stops (expect) on immediate constants without a register: there is no room for them here.
    if module.global_variables.iter().any(|(_, g)| g.space == naga::AddressSpace::Immediate) {
        return Err(crate::PipelineError::Linkage(
            stage,
            String::from("Direct3D 11: immediate constants not supported"),
        ));
    }

    // One dummy entry per group that has a sampler, otherwise naga stops; the corresponding
    // lines are removed by `adapt_sm50`.
    let mut indices_samplers = hlsl::SamplerIndexBufferBindingMap::new();
    for (_, g) in module.global_variables.iter() {
        let Some(ref b) = g.binding else { continue };
        let sampler = match module.types[g.ty].inner {
            naga::TypeInner::Sampler { .. } => true,
            naga::TypeInner::BindingArray { base, .. } => {
                matches!(module.types[base].inner, naga::TypeInner::Sampler { .. })
            }
            _ => false,
        };
        if sampler {
            indices_samplers.insert(
                hlsl::SamplerIndexBufferKey { group: b.group },
                target_value(SPACE_INDICES_SAMPLERS, b.group),
            );
        }
    }

    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V5_0,
        binding_map: bindings_list.map_naga.clone(),
        fake_missing_bindings: false,
        special_constants_binding: Some(target_value(0, REGISTRY_SPECIAL_CONSTANTS)),
        sampler_buffer_binding_map: indices_samplers,
        zero_initialize_workgroup_memory: step_value.zero_initialize_workgroup_memory,
        restrict_indexing: checks.bounds_checks,
        force_loop_bounding: checks.force_loop_bounding,
        ..hlsl::Options::default()
    };
    let options_pipeline = hlsl::PipelineOptions {
        entry_point: Some((naga_stage, step_value.entry_point.to_string())),
    };

    let mut source = String::new();
    let mut reflection = hlsl::Writer::new(&mut source, &options, &options_pipeline)
        .write(&module, &info, fragment)
        .map_err(|e| crate::PipelineError::Linkage(stage, format!("HLSL: {e:?}")))?;
    let input_value = match reflection.entry_point_names.pop() {
        Some(Ok(name_value)) => name_value,
        Some(Err(e)) => return Err(crate::PipelineError::Linkage(stage, format!("{e}"))),
        None => return Err(crate::PipelineError::EntryPoint(naga_stage)),
    };

    let source = adapt_sm50(&source).map_err(|e| {
        log::error!("Direct3D 11: HLSL of {input_value:?} ({naga_stage:?}) cannot be adapted: {e}\n{source}");
        crate::PipelineError::Linkage(stage, format!("Direct3D 11: {e}"))
    })?;
    let special_constants = source.contains(&format!("{VAR_SPECIAL_CONSTANTS}."));

    let target_fxc = match naga_stage {
        naga::ShaderStage::Vertex => "vs_5_0",
        naga::ShaderStage::Fragment => "ps_5_0",
        naga::ShaderStage::Compute => "cs_5_0",
        _ => return Err(crate::PipelineError::EntryPoint(naga_stage)),
    };

    let key_tuple = (source, input_value, target_fxc);
    if let Some(c) = CACHE.lock().get(&key_tuple) {
        return Ok(c.clone());
    }
    let file_value = file_cache(&key_tuple.0, &key_tuple.1, target_fxc);
    let read_from = |f: &std::path::PathBuf| std::fs::read(f).ok().filter(|b| b.starts_with(b"DXBC"));
    let delivers = || file_value.as_ref().and_then(|f| Some(folder_shipped()?.join(f.file_name()?))).and_then(|f| read_from(&f));
    let bytecode = match file_value.as_ref().and_then(read_from).or_else(delivers) {
        Some(b) => b,
        None => {
            let start_value = std::time::Instant::now();
            let b = compile_fxc(&key_tuple.0, &key_tuple.1, target_fxc).map_err(|e| {
                log::error!("Direct3D 11: FXC rejects {:?} ({naga_stage:?}): {e}\n{}", key_tuple.1, key_tuple.0);
                crate::PipelineError::Linkage(stage, e)
            })?;
            log::info!("Direct3D 11: FXC {} {} in {:.1} ms", key_tuple.1, target_fxc, start_value.elapsed().as_secs_f64() * 1000.0);
            if let Some(f) = &file_value {
                // Written next to it then renamed: a cache file is always complete.
                let provisional = f.with_extension("partiel");
                if std::fs::write(&provisional, &b).is_ok() {
                    let _ = std::fs::rename(&provisional, f);
                }
            }
            b
        }
    };
    let compile = Compile { bytecode, special_constants };
    let mut cache = CACHE.lock();
    if cache.len() >= CACHE_MAX {
        cache.clear();
    }
    cache.insert(key_tuple, compile.clone());
    Ok(compile)
}

/// Disk cache of the FXC bytecode (%LOCALAPPDATA%\OpenXIII\cache-hlsl): FXC is slow on old CPUs (~10 s
/// more when loading Plage01, 0.2 s hitches at the first burst of fire). Key: final HLSL, entry point,
/// target and d3dcompiler version (64-bit FNV-1a).
fn file_cache(source: &str, input_value: &str, target_fxc: &str) -> Option<std::path::PathBuf> {
    static FOLDER: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    let folder = FOLDER.get_or_init(|| {
        let d = std::path::Path::new(&std::env::var_os("LOCALAPPDATA")?).join("OpenXIII").join("cache-hlsl");
        std::fs::create_dir_all(&d).ok()?;
        Some(d)
    });
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for bytes_value in [source.as_bytes(), b"\0", input_value.as_bytes(), b"\0", target_fxc.as_bytes(), b"\0d3dcompiler_47 O3 v1"] {
        for &o in bytes_value {
            h ^= o as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    folder.as_ref().map(|d| d.join(format!("{h:016x}.dxbc")))
}

/// Cache shipped with the game (cache-hlsl folder next to the exe, read-only): DXBC bytecode
/// independent of the GPU, filled by replaying the whole game; avoids FXC on the first launch.
fn folder_shipped() -> Option<&'static std::path::Path> {
    static FOLDER: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    FOLDER
        .get_or_init(|| {
            let d = std::env::current_exe().ok()?.parent()?.join("cache-hlsl");
            d.is_dir().then_some(d)
        })
        .as_deref()
}

fn target_value(space: u8, register: u32) -> hlsl::BindTarget {
    hlsl::BindTarget {
        space,
        register,
        binding_array_size: None,
        dynamic_storage_buffer_offsets_index: None,
        restrict_indexing: false,
    }
}

/// Rewrites naga's HLSL (written for SM 5.1 on these two points) into HLSL that FXC accepts in SM 5.0.
fn adapt_sm50(hlsl: &str) -> Result<String, String> {
    let heap = format!("{HEAP_SAMPLERS}[2048]");
    let heap_comparison = format!("{HEAP_SAMPLERS_COMPARISON}[2048]");
    let end_indices = format!(", space{SPACE_INDICES_SAMPLERS});");
    let mut output = String::with_capacity(hlsl.len());
    for line_value in hlsl.lines() {
        let t = line_value.trim_start();
        // Sampler heap and index buffers: removed.
        if (t.starts_with("SamplerState ") && t.contains(&heap))
            || (t.starts_with("SamplerComparisonState ") && t.contains(&heap_comparison))
            || (t.starts_with("StructuredBuffer<uint> ") && t.ends_with(&end_indices))
        {
            continue;
        }
        // `static const SamplerState s = nagaSamplerHeap[nagaGroup0SamplerIndexArray[R]];`
        if let Some(rest_value) = t.strip_prefix("static const ") {
            if let Some(l) = sampler_direct(rest_value) {
                output.push_str(&l?);
                output.push('\n');
                continue;
            }
        }
        // `ConstantBuffer<T> name: register(bN);`
        if let Some(rest_value) = t.strip_prefix("ConstantBuffer<") {
            output.push_str(&buffer_constants(rest_value)?);
            output.push('\n');
            continue;
        }
        output.push_str(line_value);
        output.push('\n');
    }
    if output.contains(HEAP_SAMPLERS) || output.contains(HEAP_SAMPLERS_COMPARISON) {
        return Err(String::from("sampler arrays are not supported in SM 5.0"));
    }
    Ok(output)
}

/// `SamplerState s = nagaSamplerHeap[idx[R]];` (after `static const `) -> `SamplerState s : register(sR);`.
/// `None` if the line is not a sampler declaration.
fn sampler_direct(rest_value: &str) -> Option<Result<String, String>> {
    let (ty, r, name_heap) = if let Some(r) = rest_value.strip_prefix("SamplerState ") {
        ("SamplerState", r, HEAP_SAMPLERS)
    } else if let Some(r) = rest_value.strip_prefix("SamplerComparisonState ") {
        ("SamplerComparisonState", r, HEAP_SAMPLERS_COMPARISON)
    } else {
        return None;
    };
    let (name_value, value_value) = r.split_once(" = ")?;
    let inner_value = value_value.trim_end().strip_prefix(name_heap)?.strip_prefix('[')?.strip_suffix("]];")?;
    let (_, registry) = inner_value.rsplit_once('[')?;
    Some(match registry.parse::<u32>() {
        Ok(n) => Ok(format!("{ty} {name_value} : register(s{n});")),
        Err(_) => Err(format!("unreadable sampler register: {rest_value}")),
    })
}

/// `T> name: register(bN);` (after `ConstantBuffer<`) -> `cbuffer nameCb : register(bN) { T name; }`.
fn buffer_constants(rest_value: &str) -> Result<String, String> {
    let error_value = || format!("unreadable ConstantBuffer declaration: ConstantBuffer<{rest_value}");
    let (ty, after_value) = rest_value.split_once('>').ok_or_else(error_value)?;
    let (name_value, registry) = after_value.split_once(':').ok_or_else(error_value)?;
    let name_value = name_value.trim();
    let registry = registry.trim().strip_suffix(';').ok_or_else(error_value)?;
    if !registry.starts_with("register(b") || registry.contains("space") || name_value.is_empty() {
        return Err(error_value());
    }
    Ok(format!("cbuffer {name_value}Cb : {registry} {{ {ty} {name_value}; }}"))
}

/// D3DCompile (d3dcompiler_47.dll) in SM 5.0.
fn compile_fxc(source: &str, input_value: &str, target_value: &str) -> Result<Vec<u8>, String> {
    profiling::scope!("dx11::compiler_fxc");
    let input_c = format!("{input_value}\0");
    let target_c = format!("{target_value}\0");
    let flag_bits = Fxc::D3DCOMPILE_ENABLE_STRICTNESS | Fxc::D3DCOMPILE_OPTIMIZATION_LEVEL3;
    let mut code: Option<ID3DBlob> = None;
    let mut errors_list: Option<ID3DBlob> = None;
    let r = unsafe {
        Fxc::D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(input_c.as_ptr()),
            PCSTR(target_c.as_ptr()),
            flag_bits,
            0,
            &mut code,
            Some(&mut errors_list),
        )
    };
    match (r, code) {
        (Ok(()), Some(code)) => Ok(bytes_value(&code).to_vec()),
        (r, _) => {
            let mut message = match r {
                Ok(()) => String::from("FXC D3DCompile: no code returned"),
                Err(e) => format!("FXC D3DCompile ({e})"),
            };
            if let Some(e) = errors_list {
                let text_value = String::from_utf8_lossy(bytes_value(&e));
                let _ = write!(message, " : {}", text_value.trim_end_matches('\0'));
            }
            Err(message)
        }
    }
}

fn bytes_value(blob: &ID3DBlob) -> &[u8] {
    unsafe { core::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) }
}
