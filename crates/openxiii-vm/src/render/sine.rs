//! USinusModifier (XIII): wave of the vertices of a StaticMesh, advanced when drawing.
//!
//! The rendering of a StaticMesh by FDynamicActor::Render (DrawType DT_StaticMesh),
//! passes each section whose material (UStaticMesh.Materials, not the actor's Skins) is
//! a USinusModifier through FD3DRenderInterface::SetSinusVertexStreams (vtable +0x48), which advances the modifier with appFrand: the game thread's
//! rand() sequence, shared with the script. Once per frame (LastFrameCount ≠ low byte
//! of GEngine->CurrentFrameCount), one draw if RndTimeSpeed; for each drawn section, one
//! draw if RndAmpSpeed. This path is only taken without an outline (UPrimitive+0x28) or cartoon
//! rendering (+0x29, ignored for a bUnlit actor). Sorted sections (material vtable +0x78)
//! come after the others in the game; here they stay in section order, which only
//! changes how the values are spread between modifiers, not the number of draws.
//! Not reimplemented: UMeshEmitters (UMeshEmitter::RenderParticles makes the same call per section),
//! the editor's display flags (selection, ShowFlags 0x400000, wireframe).

use crate::value::{ObjId, Value};
use crate::Vm;
use crate::NativeField as Field;
use openxiii_assets::sine::{SineParams, SineState};
use openxiii_package::ObjectRef;
use std::sync::Arc;

/// Wh reads from a StaticMesh for this path.
#[derive(Debug, Default)]
pub(crate) struct SineMesh {
    /// Empty vertex stream (+0xA8): nothing is drawn.
    empty: bool,
    outline: bool,
    cartoon: bool,
    /// Modifier of each section whose material is a USinusModifier, in order.
    sections: Vec<String>,
    /// Bit 3 of +0x188 (UStaticMesh::PostLoad): at least one material of
    /// UStaticMesh.Materials requires sorting (UMaterial::RequiresSorting, vtable +0x78).
    sorted: bool,
}

impl Vm {
    fn sine_mesh(&mut self, path: &str) -> Option<Arc<SineMesh>> {
        if let Some(m) = self.sine_meshes.get(path) {
            return m.clone();
        }
        let m = self.read_sine_mesh(path).map(Arc::new);
        self.sine_meshes.insert(path.to_string(), m.clone());
        m
    }

    fn read_sine_mesh(&mut self, path: &str) -> Option<SineMesh> {
        let (mut info, materials, all) = {
            let lib = self.library.as_mut()?;
            let (loaded, index) = lib.resolve(path)?;
            if loaded.package.export_class(index) != "StaticMesh" {
                return None;
            }
            let mesh = openxiii_assets::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok()?;
            let qualified = |m: i32| match ObjectRef::from(m) {
                ObjectRef::None => String::new(),
                ObjectRef::Export(_) => format!("{}.{}", loaded.name, loaded.package.object_path(m.into())),
                _ => loaded.package.object_path(m.into()),
            };
            let materials: Vec<String> = (0..mesh.sections.len()).map(|i| mesh.materials.get(i).map_or(String::new(), |&m| qualified(m))).collect();
            let info = SineMesh { empty: mesh.positions.is_empty(), outline: mesh.outline(), cartoon: mesh.cartoon(), sections: Vec::new(), sorted: false };
            // PostLoad walks the whole Materials array, not only the sections.
            let all: Vec<String> = mesh.materials.iter().map(|&m| qualified(m)).collect();
            (info, materials, all)
        };
        info.sorted = all.iter().filter(|m| !m.is_empty()).any(|m| self.material_requires_sorting(m, 0));
        for m in materials.into_iter().filter(|m| !m.is_empty()) {
            if self.sine_modifier(&m) {
                info.sections.push(m.to_ascii_lowercase());
            }
        }
        Some(info)
    }

    /// UMaterial::RequiresSorting (vtable +0x78) of material `path`; see `requires_sorting`.
    fn material_requires_sorting(&mut self, path: &str, depth: u32) -> bool {
        requires_sorting(&mut VmSortGraph(self), &path.to_string(), depth)
    }

    /// FDynamicActor::Init for DT_StaticMesh: does the actor's StaticMesh have a
    /// sorted material (bit 3 of +0x188)? Without a StaticMesh: no.
    pub(crate) fn static_mesh_sorted(&mut self, a: ObjId) -> bool {
        let Value::Asset(path) = self.prop(a, "StaticMesh") else { return false };
        self.sine_mesh(&path).is_some_and(|m| m.sorted)
    }

    /// UObject::IsA(USinusModifier) of the material; its state is created on the first pass.
    fn sine_modifier(&mut self, path: &str) -> bool {
        let key = path.to_ascii_lowercase();
        if self.sine.contains_key(&key) {
            return true;
        }
        let Some(lib) = self.library.as_mut() else { return false };
        let Some(p) = self.sine_props.object(lib, path) else { return false };
        if !p.is_a("SinusModifier") {
            return false;
        }
        let state = (SineParams::read(p), SineState { last_frame: p.byte("LastFrameCount"), ..Default::default() });
        self.sine.insert(key, state);
        true
    }

    /// (called by FDynamicActor::Render) for the sinus modifiers: one
    /// SetSinusVertexStreams per affected section.
    pub(crate) fn sine_render(&mut self, a: ObjId) {
        if self.native_prop(a, Field::DrawType).as_byte() != 8 {
            return;
        }
        // Mesh already read: lookup by name in place, without copying.
        let mesh = match self.native_prop(a, Field::StaticMesh) {
            Value::Asset(path) => match self.sine_meshes.get(path.as_str()) {
                Some(m) => m.clone(),
                None => {
                    let path = path.clone();
                    self.sine_mesh(&path)
                }
            },
            _ => return,
        };
        let Some(mesh) = mesh else { return };
        if mesh.empty || mesh.sections.is_empty() || mesh.outline {
            return;
        }
        // bUnlit (or unlit viewport): the cartoon path is not taken.
        if mesh.cartoon && !self.propr(a, "bUnlit").as_bool() {
            return;
        }
        for key in &mesh.sections {
            self.sine_set_streams(a, key);
        }
    }

    /// Advance of a USinusModifier (openxiii_assets::sine::advance) with
    /// GEngine->FrameLenght (+0xD0) and the low byte of GEngine->CurrentFrameCount (+0xD4).
    fn sine_set_streams(&mut self, a: ObjId, key: &str) {
        let Some((params, mut state)) = self.sine.remove(key) else { return };
        let (frame, dt) = (self.engine_frames as u8, self.engine_delta);
        let amp = openxiii_assets::sine::advance(&params, &mut state, frame, dt, &mut || self.rand());
        // For display: time and amplitude of this call (end).
        self.sine_drawn.push((a, key.to_string(), state.current_time, amp));
        self.sine.insert(key.to_string(), (params, state));
    }
}

/// Class of a material for UMaterial::RequiresSorting (vtable +0x78).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SortKind {
    /// UTexture: bAlphaTexture.
    Texture { alpha: bool },
    /// UShader: OutputBlending.
    Shader { output_blending: u8 },
    /// UFinalBlend: FrameBufferBlending.
    FinalBlend { frame_buffer_blending: u8 },
    /// UModifier: follows its Material.
    Modifier,
    /// The others (UCombiner, UBitmapMaterial, UConstantMaterial…: UMaterial::RequiresSorting returns 0).
    Other,
}

/// Material graph read by RequiresSorting: the VM (asset paths) or the tests.
pub(crate) trait SortGraph {
    type Id: Clone;
    fn kind(&mut self, m: &Self::Id) -> Option<SortKind>;
    /// Material (+0x4C) of a UModifier.
    fn material(&mut self, m: &Self::Id) -> Option<Self::Id>;
    /// UObject::IsA(USinusModifier).
    fn is_sine(&mut self, m: &Self::Id) -> bool;
}

/// UMaterial::RequiresSorting: UTexture bAlphaTexture, UShader OutputBlending other than 0, 1 and 4, UFinalBlend
/// FrameBufferBlending from 1 to 6, UModifier that of its Material, the rest false.
pub(crate) fn requires_sorting<G: SortGraph>(g: &mut G, m: &G::Id, depth: u32) -> bool {
    if depth > 16 {
        return false;
    }
    match g.kind(m) {
        Some(SortKind::Texture { alpha }) => alpha,
        Some(SortKind::Shader { output_blending }) => !matches!(output_blending, 0 | 1 | 4),
        Some(SortKind::FinalBlend { frame_buffer_blending }) => matches!(frame_buffer_blending, 1..=6),
        Some(SortKind::Modifier) => match g.material(m) {
            Some(inner) => requires_sorting(g, &inner, depth + 1),
            None => false,
        },
        _ => false,
    }
}

/// UStaticMesh::PostLoad: bits 3 (a material requires sorting) and 4 (a USinusModifier) of +0x188 from
/// the Materials array (null entries are skipped), then bit 5 is bit 3 | bit 4.
pub(crate) fn mesh_sort_flags<G: SortGraph>(g: &mut G, materials: &[Option<G::Id>]) -> u32 {
    let (mut sorted, mut sine) = (false, false);
    for m in materials.iter().flatten() {
        sorted |= requires_sorting(g, m, 0);
        sine |= g.is_sine(m);
    }
    (sorted as u32) << 3 | (sine as u32) << 4 | ((sorted || sine) as u32) << 5
}

struct VmSortGraph<'a>(&'a mut Vm);

impl SortGraph for VmSortGraph<'_> {
    type Id = String;

    fn kind(&mut self, path: &String) -> Option<SortKind> {
        let vm = &mut *self.0;
        let lib = vm.library.as_mut()?;
        let p = vm.sine_props.object(lib, path)?;
        Some(if p.is_a("Texture") {
            SortKind::Texture { alpha: p.bool("bAlphaTexture") }
        } else if p.is_a("Shader") {
            SortKind::Shader { output_blending: p.byte("OutputBlending") }
        } else if p.is_a("FinalBlend") {
            SortKind::FinalBlend { frame_buffer_blending: p.byte("FrameBufferBlending") }
        } else if p.is_a("Modifier") {
            SortKind::Modifier
        } else {
            SortKind::Other
        })
    }

    fn material(&mut self, path: &String) -> Option<String> {
        let vm = &mut *self.0;
        let lib = vm.library.as_mut()?;
        vm.sine_props.object(lib, path)?.obj("Material")
    }

    fn is_sine(&mut self, path: &String) -> bool {
        let vm = &mut *self.0;
        let Some(lib) = vm.library.as_mut() else { return false };
        vm.sine_props.object(lib, path).is_some_and(|p| p.is_a("SinusModifier"))
    }
}

