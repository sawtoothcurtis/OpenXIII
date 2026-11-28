//! Material sounds (Material.FootstepSound… HitSound, NoiseLoudness): UMaterial::GetMaterialSound,
//! UShader::GetMaterialSound, UModifier::GetMaterialSound,
//! GetNoiseLoudness and GetAllMaterialSounds
//! (vtable + 0x88, Shader and Modifier only) that AActor::execTrace
//! calls on the hit material: the empty fields of a Shader or a Modifier are filled once
//! and for all from its sub-materials, which the scripts read afterwards (HitMat.HitSound
//! chooses the impact effect, XIIIAmmo.SetUpImpactEmitter).

use crate::value::Value;
use crate::Vm;

/// Fields read by GetMaterialSound(index): 0 to 3 and 6; 4 and 5 have none.
pub const SOUND_FIELDS: [(usize, &str); 5] = [(0, "FootstepSound"), (1, "XIIIFootStepSound"), (2, "PNJSndStep"), (3, "XIIISndStep"), (6, "HitSound")];

/// Sub-materials of a Shader walked by GetMaterialSound (+0x4C to +0x64).
pub(crate) const SHADER_SUBS: [&str; 7] = ["Diffuse", "Opacity", "Specular", "SpecularityMask", "SelfIllumination", "SelfIlluminationMask", "Detail"];

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Material,
    Shader,
    Modifier,
}

/// Material graph read by the sound functions: the VM (asset paths) or a test (numbered nodes).
pub(crate) trait MaterialGraph {
    type Id: Clone;
    fn kind(&mut self, m: &Self::Id) -> Option<Kind>;
    /// Object (material or sound) of a field, None if it is null.
    fn object(&mut self, m: &Self::Id, field: &str) -> Option<Self::Id>;
    /// NoiseLoudness field of the object.
    fn loudness_field(&mut self, m: &Self::Id) -> f32;
}

/// GetMaterialSound(index): the material's field, or the first non-null result of the sub-materials
/// of a Shader (without looking at its own fields), or that of a Modifier's material.
pub(crate) fn material_sound<G: MaterialGraph>(g: &mut G, path: &G::Id, index: usize, depth: u32) -> Option<G::Id> {
    if depth > 16 {
        return None;
    }
    match g.kind(path)? {
        Kind::Shader => {
            for sub in SHADER_SUBS {
                if let Some(m) = g.object(path, sub) {
                    if let Some(s) = material_sound(g, &m, index, depth + 1) {
                        return Some(s);
                    }
                }
            }
            None
        }
        Kind::Modifier => {
            let m = g.object(path, "Material")?;
            material_sound(g, &m, index, depth + 1)
        }
        Kind::Material => {
            let field = SOUND_FIELDS.iter().find(|(i, _)| *i == index)?.1;
            g.object(path, field)
        }
    }
}

/// GetNoiseLoudness: NoiseLoudness of the material; for a Shader, that of the first
/// non-null sub-material from Diffuse to SelfIlluminationMask, otherwise from Detail; for a
/// Modifier, that of its material; 0 otherwise.
pub(crate) fn material_noise_loudness<G: MaterialGraph>(g: &mut G, path: &G::Id) -> f32 {
    match g.kind(path) {
        Some(Kind::Shader) => {
            for sub in SHADER_SUBS {
                if let Some(m) = g.object(path, sub) {
                    return g.loudness_field(&m);
                }
            }
            0.0
        }
        Some(Kind::Modifier) => match g.object(path, "Material") {
            Some(m) => g.loudness_field(&m),
            None => 0.0,
        },
        _ => g.loudness_field(path),
    }
}

/// Writes of GetAllMaterialSounds: empty sound fields that take GetMaterialSound, and
/// zero NoiseLoudness that takes GetNoiseLoudness. None for a plain UMaterial (nothing to do).
pub(crate) fn sounds_to_fill<G: MaterialGraph>(g: &mut G, path: &G::Id) -> Option<(Vec<(&'static str, Option<G::Id>)>, Option<f32>)> {
    if !matches!(g.kind(path), Some(Kind::Shader | Kind::Modifier)) {
        return None;
    }
    let mut writes = Vec::new();
    for (index, field) in SOUND_FIELDS {
        if g.object(path, field).is_none() {
            writes.push((field, material_sound(g, path, index, 0)));
        }
    }
    let loudness = (g.loudness_field(path) == 0.0).then(|| material_noise_loudness(g, path));
    Some((writes, loudness))
}

struct VmGraph<'a>(&'a mut Vm);

impl MaterialGraph for VmGraph<'_> {
    type Id = String;

    /// Class of material `path` in its package: Shader, Modifier (FinalBlend, TexModifier…)
    /// or another UMaterial.
    fn kind(&mut self, path: &String) -> Option<Kind> {
        let vm = &mut *self.0;
        let class = {
            let library = vm.library.as_mut()?;
            let (loaded, index) = library.resolve(path)?;
            loaded.package.export_class(index).to_string()
        };
        let c = vm.class(&class)?;
        Some(if vm.class("Shader").is_some_and(|s| vm.is_child_of(c, s)) {
            Kind::Shader
        } else if vm.class("Modifier").is_some_and(|m| vm.is_child_of(c, m)) {
            Kind::Modifier
        } else {
            Kind::Material
        })
    }

    fn object(&mut self, path: &String, field: &str) -> Option<String> {
        match self.0.asset_prop(path, field) {
            Some(Value::Asset(p)) => Some(p),
            _ => None,
        }
    }

    /// NoiseLoudness field of the object: its serialized value, otherwise its class's default one
    /// (Material: 0.1).
    fn loudness_field(&mut self, path: &String) -> f32 {
        let vm = &mut *self.0;
        if let Some(v) = vm.asset_prop(path, "NoiseLoudness") {
            return v.as_float();
        }
        let class = vm.library.as_mut().and_then(|l| l.resolve(path).map(|(loaded, index)| loaded.package.export_class(index).to_string()));
        match class.and_then(|c| vm.class(&c)) {
            Some(c) => {
                let cdo = vm.classes[c].cdo;
                vm.propr(cdo, "NoiseLoudness").as_float()
            }
            None => 0.0,
        }
    }
}

impl Vm {
    /// GetAllMaterialSounds on the material hit by a trace: nothing for a
    /// plain UMaterial; for a Shader or a Modifier, each empty sound takes GetMaterialSound and
    /// zero NoiseLoudness takes GetNoiseLoudness, written into the object.
    pub(crate) fn material_fill_sounds(&mut self, path: &str) {
        let path = path.to_string();
        let Some((writes, loudness)) = sounds_to_fill(&mut VmGraph(self), &path) else { return };
        for (field, sound) in writes {
            let v = sound.map_or(Value::Object(None), Value::Asset);
            self.asset_cache.insert((path.to_lowercase(), field.to_lowercase()), v);
        }
        if let Some(v) = loudness {
            self.asset_cache.insert((path.to_lowercase(), "noiseloudness".into()), Value::Float(v));
        }
    }
}

