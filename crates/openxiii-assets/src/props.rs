//! Object properties with their class defaults, as the engine sees them
//! after loading: defaultproperties of each class in the chain (from Object to the object's
//! class), then the object's serialized properties. Object references are returned as
//! full paths "Package.Group.Object".

use openxiii_package::library::Library;
use openxiii_package::{ObjectRef, Package, Value};
use std::collections::HashMap;

/// Resolved property value.
#[derive(Debug, Clone)]
pub enum PVal {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Obj(Option<String>),
    Rot([i32; 3]),
    Vec3([f32; 3]),
    Raw(Vec<u8>),
    Other,
}

/// Tables read on every draw (properties by name): fast hash, same content.
pub type FastMap<K, V> = std::collections::HashMap<K, V, rustc_hash::FxBuildHasher>;

/// A loaded object: its class chain and merged properties.
#[derive(Debug, Clone)]
pub struct Props {
    /// From the object's class up to Object.
    pub chain: Vec<String>,
    pub values: FastMap<String, PVal>,
}

pub fn qualified(package_name: &str, package: &Package, r: i32) -> String {
    match ObjectRef::from(r) {
        ObjectRef::Export(_) => format!("{package_name}.{}", package.object_path(r.into())),
        _ => package.object_path(r.into()),
    }
}

fn convert(package_name: &str, package: &Package, v: &Value) -> PVal {
    match v {
        Value::Byte(b) => PVal::Byte(*b),
        Value::Int(i) => PVal::Int(*i),
        Value::Bool(b) => PVal::Bool(*b),
        Value::Float(f) => PVal::Float(*f),
        Value::Object(r) => PVal::Obj((*r != 0).then(|| qualified(package_name, package, *r))),
        Value::Rotator(r) => PVal::Rot(*r),
        Value::Vector(v) => PVal::Vec3(*v),
        Value::Raw(b) => PVal::Raw(b.clone()),
        _ => PVal::Other,
    }
}

impl Props {
    pub fn is_a(&self, class: &str) -> bool {
        self.chain.iter().any(|c| c.eq_ignore_ascii_case(class))
    }
    pub fn get(&self, name: &str) -> Option<&PVal> {
        // Same key as name.to_ascii_lowercase(), without allocating: the name as is if it has
        // no ASCII uppercase letter, otherwise its lowercase copy in a stack buffer.
        let b = name.as_bytes();
        if !b.iter().any(u8::is_ascii_uppercase) {
            return self.values.get(name);
        }
        let mut buffer_value = [0u8; 64];
        match buffer_value.get_mut(..b.len()) {
            Some(t) => {
                for (d, s) in t.iter_mut().zip(b) {
                    *d = s.to_ascii_lowercase();
                }
                // ASCII lowercase of UTF-8 text: always valid UTF-8.
                self.values.get(std::str::from_utf8(t).ok()?)
            }
            None => self.values.get(&name.to_ascii_lowercase()),
        }
    }
    pub fn obj(&self, name: &str) -> Option<String> {
        match self.get(name) {
            Some(PVal::Obj(o)) => o.clone(),
            _ => None,
        }
    }
    pub fn byte(&self, name: &str) -> u8 {
        match self.get(name) {
            Some(PVal::Byte(b)) => *b,
            Some(PVal::Int(i)) => *i as u8,
            _ => 0,
        }
    }
    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(PVal::Int(i)) => *i,
            Some(PVal::Byte(b)) => *b as i32,
            _ => 0,
        }
    }
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(PVal::Bool(true)))
    }
    pub fn float(&self, name: &str) -> f32 {
        match self.get(name) {
            Some(PVal::Float(f)) => *f,
            _ => 0.0,
        }
    }
    pub fn float_or(&self, name: &str, default: f32) -> f32 {
        match self.get(name) {
            Some(PVal::Float(f)) => *f,
            _ => default,
        }
    }
    pub fn rot(&self, name: &str) -> [i32; 3] {
        match self.get(name) {
            Some(PVal::Rot(r)) => *r,
            _ => [0; 3],
        }
    }
    pub fn vec3(&self, name: &str) -> Option<[f32; 3]> {
        match self.get(name) {
            Some(PVal::Vec3(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn raw(&self, name: &str) -> Option<&[u8]> {
        match self.get(name) {
            Some(PVal::Raw(b)) => Some(b),
            _ => None,
        }
    }
    /// Color struct (serialized R, G, B, A) as D3DCOLOR 0xAARRGGBB (FColor::operator DWORD).
    pub fn color(&self, name: &str) -> u32 {
        match self.get(name) {
            Some(PVal::Raw(b)) if b.len() >= 4 => (b[3] as u32) << 24 | (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32,
            _ => 0,
        }
    }
    pub fn matrix(&self, name: &str) -> [f32; 16] {
        match self.get(name) {
            Some(PVal::Raw(b)) if b.len() >= 64 => std::array::from_fn(|i| f32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])),
            _ => [0.0; 16],
        }
    }
}

/// Cache of loaded classes and objects.
#[derive(Default)]
pub struct PropCache {
    classes: FastMap<String, Option<Props>>,
    objects: FastMap<String, Option<Props>>,
}

impl PropCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Chain of classes and merged defaultproperties of a class ("Engine.Texture").
    pub fn class(&mut self, library: &mut Library, class_path: &str) -> Option<Props> {
        let key = class_path.to_ascii_lowercase();
        if let Some(c) = self.classes.get(&key) {
            return c.clone();
        }
        let info = (|| {
            let (name, super_path, defaults) = {
                let (loaded, index) = library.resolve(class_path)?;
                let p = &loaded.package;
                let export = &p.exports[index];
                let name = p.object_name(ObjectRef::Export(index)).to_string();
                let super_path = (export.super_ != 0).then(|| qualified(&loaded.name, p, export.super_));
                let defaults: Vec<(String, PVal)> = openxiii_script::objects::read_class(p, &loaded.data, index)
                    .map(|c| {
                        c.defaults
                            .iter()
                            .filter(|d| d.array_index == 0)
                            .map(|d| (p.name(d.name).to_ascii_lowercase(), convert(&loaded.name, p, &d.value)))
                            .collect()
                    })
                    .unwrap_or_default();
                (name, super_path, defaults)
            };
            let mut props = match super_path {
                Some(s) => self.class(library, &s).unwrap_or(Props { chain: Vec::new(), values: Default::default() }),
                None => Props { chain: Vec::new(), values: Default::default() },
            };
            props.chain.insert(0, name);
            for (k, v) in defaults {
                props.values.insert(k, v);
            }
            Some(props)
        })();
        self.classes.insert(key, info.clone());
        info
    }

    /// An object (export) with its class defaults.
    pub fn object(&mut self, library: &mut Library, path: &str) -> Option<&Props> {
        let key = path.to_ascii_lowercase();
        if !self.objects.contains_key(&key) {
            let obj = (|| {
                let (class_path, own) = {
                    let (loaded, index) = library.resolve(path)?;
                    let p = &loaded.package;
                    let class_ref = p.exports[index].class;
                    let class_path = if class_ref == 0 { "Core.Class".to_string() } else { qualified(&loaded.name, p, class_ref) };
                    let o = p.object_data(&loaded.data, index).ok()?;
                    let own: Vec<(String, PVal)> = o
                        .properties
                        .iter()
                        .filter(|q| q.array_index == 0)
                        .map(|q| (p.name(q.name).to_ascii_lowercase(), convert(&loaded.name, p, &q.value)))
                        .collect();
                    (class_path, own)
                };
                let mut props = self.class(library, &class_path)?;
                for (k, v) in own {
                    props.values.insert(k, v);
                }
                Some(props)
            })();
            // Mods build only: a mod's new texture ("Mod.<Name>", in no package) is a Texture
            // with the class defaults and the size of its image, so the materials draw it like
            // a game texture (its pixels come from mod_textures).
            #[cfg(feature = "mods")]
            let obj = obj.or_else(|| self.mod_texture(library, path));
            self.objects.insert(key.clone(), obj);
        }
        self.objects.get(&key).and_then(Option::as_ref)
    }

    /// A mod's texture `path` as a Texture object (mods build only).
    #[cfg(feature = "mods")]
    fn mod_texture(&mut self, library: &mut Library, path: &str) -> Option<Props> {
        let mips = crate::mod_textures::get(path)?;
        let &(width, height, ref rgba) = mips.first()?;
        let mut props = self.class(library, "Engine.Texture")?;
        props.values.insert("usize".into(), PVal::Int(width as i32));
        props.values.insert("vsize".into(), PVal::Int(height as i32));
        // Transparent pixels: drawn with their alpha, as a game texture with an alpha channel.
        let alpha = rgba.chunks_exact(4).any(|p| p[3] < 255);
        props.values.insert("balphatexture".into(), PVal::Bool(alpha));
        // `clamp = true`: read at its edge outside the image (marks projected on walls).
        if crate::mod_textures::clamped(path) {
            props.values.insert("uclampmode".into(), PVal::Byte(1));
            props.values.insert("vclampmode".into(), PVal::Byte(1));
        }
        Some(props)
    }
}

/// Allocation-free Props::get against the previous lookup (to_ascii_lowercase key).
#[cfg(test)]
#[test]
fn get_identical_to_key_in_lowercase() {
    let long = "SomeVeryLongPropertyName".repeat(4);
    let keys_list = ["usize", "vsize", "animnext", "é", "Éclat", "x", "", long.as_str(), &long.to_ascii_lowercase()];
    let props = Props { chain: vec!["Object".into()], values: keys_list.iter().enumerate().map(|(i, k)| (k.to_ascii_lowercase(), PVal::Int(i as i32))).collect() };
    let names_list = ["USize", "usize", "VSIZE", "AnimNext", "animNEXT", "é", "É", "Éclat", "éclat", "ÉCLAT", "X", "x", "", "Absent", &long, &long.to_ascii_lowercase(), &long.to_ascii_uppercase(), "64_characters_exactly_ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmno", "64_characters_exactly_ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnop"];
    for n in names_list {
        let want = props.values.get(&n.to_ascii_lowercase()).map(|v| format!("{v:?}"));
        assert_eq!(props.get(n).map(|v| format!("{v:?}")), want, "{n}");
    }
    assert!(props.get("USize").is_some() && props.get(&long).is_some() && props.get("ÉCLAT").is_some() && props.get("éCLAT").is_none());
}