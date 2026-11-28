//! Loading of script packages: classes, properties, functions, states, structs.

use crate::types::*;
use crate::value::Value;
use crate::{ObjId, Vm};
use openxiii_package::{ObjectRef, Package};
use openxiii_script::decompile::children;
use openxiii_script::objects::{self, func, prop, PropertyKind};
use std::collections::HashMap;
use std::path::Path;

/// UClient configuration properties read in `client_color`, in the order of their offsets (+0x50, +0x54,
/// +0x58, declared by UClient::StaticConstructor).
pub(crate) const KEYS_COLOR_CLIENT: [&str; 3] = ["Brightness", "Contrast", "Gamma"];

pub struct LoadedPackage {
    pub name: String,
    pub package: Package,
    pub data: Vec<u8>,
}

impl Vm {
    /// Loads all the script packages (.u) of a System folder (and of its subfolders).
    pub fn load_dir(&mut self, dir: &Path) -> std::io::Result<()> {
        let _math = self.session.math.enter();
        if self.system_dir.is_none() {
            self.system_dir = Some(dir.to_path_buf());
        }
        if self.library.is_none() {
            // The System folder is inside the game folder: the whole game is indexed for the data.
            if let Some(game) = dir.parent() {
                self.library = openxiii_package::library::Library::new(game).ok();
                // Mods build only: a mod's map named as a file of the game (mod_maps.rs).
                #[cfg(feature = "mods")]
                self.mod_map_collisions(game);
            }
        }
        let mut files = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d)? {
                let path = entry?.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("u")) {
                    files.push(path);
                }
            }
        }
        files.sort();
        for path in files {
            let data = std::fs::read(&path)?;
            let Ok(package) = Package::parse(&data) else {
                continue;
            };
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
            if self.packages.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                continue;
            }
            self.identify_find_label_package(&name, &data, self.packages.len());
            self.packages.push(LoadedPackage { name, package, data });
        }
        self.canonical_package_names();
        // GConfig: the game's ini, the player's and the language files of the System folder.
        // UPCSaveGameDevice: profiles in "save" next to the game folder (never the game folder itself).
        if self.engine.save.is_none() {
            if let Some(game) = dir.parent() {
                let root = game.parent().unwrap_or(game);
                let dir = crate::save::folder_saves().unwrap_or_else(|| root.join("save"));
                self.engine.save = Some(crate::save::SaveDevice::new(dir, Some(game.join("Save"))));
            }
        }
        if self.config.is_none() {
            self.config = Some(crate::config::ConfigCache::for_session(dir.to_path_buf(), &self.session));
            // UWindowsClient: starting size (fullscreen or window) from XIII.ini.
            if let Some(c) = self.config.as_mut() {
                // Improved build, first launch of the game: the original's defaults, fullscreen
                // at the screen's native size (video_settings.rs).
                crate::improved::storage::apply_first_launch(c);
                let section = "WinDrv.WindowsClient";
                let full = c.get_string(section, "StartupFullscreen", "System").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                let (kx, ky) = if full { ("FullscreenViewportX", "FullscreenViewportY") } else { ("WindowedViewportX", "WindowedViewportY") };
                let x = c.get_string(section, kx, "System").and_then(|v| v.trim().parse().ok());
                let y = c.get_string(section, ky, "System").and_then(|v| v.trim().parse().ok());
                if let (Some(x), Some(y)) = (x, y) {
                    self.viewport = (x, y);
                    // UWindowsViewport::OpenWindow brings the window size back to a standard resolution;
                    // fullscreen keeps its own.
                    // Improved build: a window size chosen in the Video menu (any monitor mode, or
                    // the window's own size, 16:9 included) is kept when it fits the desktop.
                    let kept = crate::improved::storage::window_size_kept(x, y, self.desktop);
                    if !full && !kept {
                        let (mut w, mut h) = (x, y);
                        self.res_table.choose(&mut w, &mut h, self.desktop);
                        self.viewport = (w, h);
                    }
                }
                self.fullscreen = full;
                // OpenWindow reads the window sizes even when the requested rendering is fullscreen.
                let windowed = if full {
                    let wx = c.get_string(section, "WindowedViewportX", "System").and_then(|v| v.trim().parse().ok());
                    let wy = c.get_string(section, "WindowedViewportY", "System").and_then(|v| v.trim().parse().ok());
                    wx.zip(wy)
                } else {
                    x.zip(y)
                };
                self.engine.client.set_loaded_windowed_size(windowed);
                // UClient: Brightness (+0x50), Contrast (+0x54), Gamma (+0x58), configuration
                // properties of [WinDrv.WindowsClient] (UClient::StaticConstructor).
                for (i, key) in KEYS_COLOR_CLIENT.iter().enumerate() {
                    if let Some(v) = c.get_string(section, key, "System") {
                        let chars: Vec<char> = v.chars().collect();
                        self.client_color[i] = crate::config::atof(&chars, 0);
                    }
                }
                // UWindowsClient::PostEditChange → UClient::PostEditChange: Brightness clamped to [0, 1].
                self.client_color[0] = crate::client::client_post_edit_change(self.client_color[0], 0.0).0;
            }
        }
        // GSys->VideoPath ([Core.System] VideoPath, "..\Video"), relative to the System folder.
        if self.engine.video.dir.as_os_str().is_empty() {
            let path = self.config.as_mut().and_then(|c| c.get_string("Core.System", "VideoPath", "System")).unwrap_or_else(|| "../Video".into());
            self.engine.video = crate::video::VideoDevice::new(dir.join(path.replace('\\', "/")));
            self.engine.video.path_text = path.clone();
            // Improved build: the added menus' video frames are made in the background from now
            // on (or read back from the cache), so they are ready when the menu shows them.
            crate::improved::storage::prefetch_menu_pictures(&self.engine.video.dir);
        }
        self.register_classes();
        Ok(())
    }

    fn register_classes(&mut self) {
        for pi in 0..self.packages.len() {
            let p = &self.packages[pi].package;
            for i in 0..p.exports.len() {
                if p.exports[i].serial_size == 0 {
                    continue;
                }
                match p.export_class(i) {
                    "Class" => {
                        let name = p.name(p.exports[i].object_name).to_string();
                        let key = name.to_lowercase();
                        let qualified = format!("{}.{key}", self.packages[pi].name.to_lowercase());
                        if self.class_qualified.contains_key(&qualified) {
                            continue;
                        }
                        let class = objects::read_class(p, &self.packages[pi].data, i).ok();
                        let super_name =
                            class.as_ref().map(|c| c.state.body.header.super_field).filter(|&s| s != 0).map(|s| p.object_name(s.into()).to_string());
                        let flags = class.as_ref().map_or(0, |c| c.class_flags);
                        let config_name = class
                            .as_ref()
                            .map(|c| p.name(c.config_name).to_string())
                            .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("None"))
                            .unwrap_or_else(|| "System".into());
                        // ClassWithin: the Outer of an instance must descend from it (0: Object).
                        if let Some(w) = class.as_ref().map(|c| c.within).filter(|&w| w != 0) {
                            self.class_within.insert(self.classes.len(), p.object_name(w.into()).to_string());
                        }
                        // The short name designates the first class registered with that name.
                        self.class_index.entry(key).or_insert(self.classes.len());
                        self.class_qualified.insert(qualified, self.classes.len());
                        self.classes.push(ClassDef {
                            name,
                            package: pi,
                            export: i,
                            super_name,
                            super_class: None,
                            props: Vec::new(),
                            prop_index: Default::default(),
                            functions: Default::default(),
                            states: Default::default(),
                            cdo: usize::MAX,
                            linked: false,
                            flags,
                            config_name,
                            probe_mask: class.as_ref().map_or(0, |k| k.state.probe_mask),
                        });
                    }
                    "Struct" => {
                        // Several classes can declare a struct with the same name: key "class.struct",
                        // plus the name alone for lookups without an owner.
                        let full = struct_key(&p.object_path(ObjectRef::Export(i)));
                        let name = p.name(p.exports[i].object_name).to_lowercase();
                        self.struct_exports.entry(full).or_insert((pi, i));
                        self.struct_exports.entry(name).or_insert((pi, i));
                    }
                    _ => {}
                }
            }
        }
        // Mods build only: the mods' new classes, made from game classes (mod_classes.rs).
        #[cfg(feature = "mods")]
        self.register_mod_classes();
        for c in 0..self.classes.len() {
            self.link_class(c);
        }
        self.bind_native_fields();
        for c in 0..self.classes.len() {
            self.create_cdo(c);
        }
        // ULinkerLoad: the objects referenced by the default values are loaded with them.
        self.index_script_exports();
        // Mods build only: values of sub-objects (mod_defaults.rs), which need the index.
        #[cfg(feature = "mods")]
        self.apply_mod_subobjects();
        for c in 0..self.classes.len() {
            let cdo = self.classes[c].cdo;
            self.resolve_object_refs(cdo);
        }
        // GNatives: a single exec per number. XIII's Engine.dll does not implement UScriptedTexture
        // (no exported exec): its numbers 472 to 476 are those of MatchMakingManager and of
        // VideoPlayer (476: GetStatus, without which the menu videos would stop at once).
        // XIII's Object.uc declares ResetConfig native(203), the number of "rotator != rotator";
        // Core.dll registers execResetConfig at number 543 (GNatives[0x21F]) and
        // execNotEqual_RotatorRotator at 203. Taking the first one found, every
        // "r != rot(0,0,0)" in the script called ResetConfig and returned false (CineController2,
        // PlayingSequence.Tick: the pawn turned at once towards rWantedRotation).
        for (id, f) in self.functions.iter().enumerate() {
            if f.native != 0 && !self.classes[f.owner].name.eq_ignore_ascii_case("ScriptedTexture") {
                let n = if f.native == 203 && f.name.eq_ignore_ascii_case("ResetConfig") { 543 } else { f.native };
                self.native_index.entry(n).or_insert(id);
            }
        }
    }

    /// Index of the exports whose class is a script class: subobjects of the default
    /// values ("gui.GUIController.GUI_SmallFont", "xiii.BloodShotEmitter.BloodShotEmitterA"…).
    /// Resources (materials, fonts, sounds, meshes…) stay paths (Value::Asset)
    /// that rendering and sound read from their packages.
    fn index_script_exports(&mut self) {
        const RESOURCES: [&str; 8] = ["Material", "Font", "Sound", "Primitive", "MeshAnimation", "Palette", "Level", "Actor"];
        let bases: Vec<crate::value::ClassId> = RESOURCES.iter().filter_map(|n| self.class(n)).collect();
        let mut index = HashMap::new();
        for (pi, lp) in self.packages.iter().enumerate() {
            let p = &lp.package;
            for e in 0..p.exports.len() {
                let class_name = p.export_class(e);
                if class_name.eq_ignore_ascii_case("Class") || p.exports[e].serial_size == 0 {
                    continue;
                }
                let Some(&class) = self.class_index.get(&class_name.to_lowercase()) else {
                    continue;
                };
                if bases.iter().any(|&b| self.is_child_of(class, b)) {
                    continue;
                }
                let path = format!("{}.{}", lp.name, p.object_path(ObjectRef::Export(e))).to_lowercase();
                index.entry(path).or_insert((pi, e));
            }
        }
        self.script_exports = index;
    }

    /// Script object of a package (default subobject), loaded once as
    /// ULinkerLoad does: default values of its class (UObject::InitProperties), then the export's
    /// serialized properties. All references share it (no instantiation of
    /// subobjects in StaticConstructObject); its own references are resolved the same way.
    pub(crate) fn script_object(&mut self, path: &str) -> Option<ObjId> {
        let key = path.to_lowercase();
        if let Some(&id) = self.subobjects.get(&key) {
            return Some(id);
        }
        let &(pi, export) = self.script_exports.get(&key)?;
        let class = self.class_of_ref(pi, self.packages[pi].package.exports[export].class)?;
        let (outer, name) = {
            let lp = &self.packages[pi];
            let p = &lp.package;
            let full = format!("{}.{}", lp.name, p.object_path(openxiii_package::ObjectRef::Export(export)));
            let outer = full.rsplit_once('.').map_or(String::new(), |(o, _)| o.to_string());
            (outer, p.name(p.exports[export].object_name).to_string())
        };
        let id = self.spawn_in(class, outer, Some(name));
        self.subobjects.insert(key, id);
        let props = {
            let lp = &self.packages[pi];
            lp.package.object_data(&lp.data, export).map(|o| o.properties).unwrap_or_default()
        };
        let map = HashMap::new();
        for prop in props {
            let pname = self.packages[pi].package.name(prop.name).to_lowercase();
            let Some(&slot) = self.classes[class].prop_index.get(&pname) else {
                continue;
            };
            let def = self.classes[class].props[slot].clone();
            let value = self.convert_map_value(pi, &def.ty, &prop.value, &map);
            let o = self.objects[id].as_mut().unwrap();
            if def.dim > 1 {
                if let Value::Array(items) = &mut o.props[slot] {
                    if let Some(item) = items.get_mut(prop.array_index as usize) {
                        *item = value;
                    }
                }
            } else if prop.array_index == 0 {
                o.props[slot] = value;
            }
        }
        self.resolve_object_refs(id);
        Some(id)
    }

    /// References of object `id` to package script objects (Value::Asset): the objects.
    pub(crate) fn resolve_object_refs(&mut self, id: ObjId) {
        let Some(mut props) = self.objects.get(id).and_then(|o| o.as_ref()).map(|o| o.props.clone()) else {
            return;
        };
        let mut changed = false;
        for v in props.iter_mut() {
            changed |= self.resolve_value_refs(v);
        }
        if changed {
            if let Some(o) = self.objects[id].as_mut() {
                o.props = props;
            }
        }
    }

    pub(crate) fn resolve_value_refs(&mut self, v: &mut Value) -> bool {
        match v {
            Value::Asset(path) => {
                if !self.script_exports.contains_key(&path.to_lowercase()) {
                    return false;
                }
                let path = path.clone();
                match self.script_object(&path) {
                    Some(id) => {
                        *v = Value::Object(Some(id));
                        true
                    }
                    None => false,
                }
            }
            Value::Array(items) => {
                let mut changed = false;
                for item in items.iter_mut() {
                    changed |= self.resolve_value_refs(item);
                }
                changed
            }
            Value::Struct(_, items) => {
                let mut changed = false;
                for item in items.iter_mut() {
                    changed |= self.resolve_value_refs(item);
                }
                changed
            }
            _ => false,
        }
    }

    /// Type of a property of package `pi`.
    fn property_type(&self, pi: usize, kind: &PropertyKind) -> Type {
        let p = &self.packages[pi].package;
        let name = |r: i32| p.object_name(r.into()).to_string();
        match kind {
            PropertyKind::Byte { .. } => Type::Byte,
            PropertyKind::Int => Type::Int,
            PropertyKind::Bool => Type::Bool,
            PropertyKind::Float => Type::Float,
            PropertyKind::Name => Type::Name,
            PropertyKind::Str => Type::Str,
            PropertyKind::Object { class } => Type::Object(name(*class)),
            PropertyKind::Class { meta_class, .. } => Type::Class(name(*meta_class)),
            PropertyKind::Struct { structure } => Type::Struct(struct_key(&p.object_path((*structure).into()))),
            PropertyKind::Array { inner } => match ObjectRef::from(*inner) {
                ObjectRef::Export(e) => objects::read_property(p, &self.packages[pi].data, e)
                    .map(|q| Type::Array(Box::new(self.property_type(pi, &q.kind))))
                    .unwrap_or(Type::Unknown),
                _ => Type::Unknown,
            },
            PropertyKind::Delegate { .. } => Type::Delegate,
            _ => Type::Unknown,
        }
    }

    fn property_def(&self, pi: usize, export: usize) -> Option<PropDef> {
        let p = &self.packages[pi].package;
        let pr = objects::read_property(p, &self.packages[pi].data, export).ok()?;
        let inner_flags = match &pr.kind {
            PropertyKind::Array { inner } => match ObjectRef::from(*inner) {
                ObjectRef::Export(e) => objects::read_property(p, &self.packages[pi].data, e).map_or(0, |q| q.flags),
                _ => 0,
            },
            _ => 0,
        };
        Some(PropDef {
            name: p.name(p.exports[export].object_name).to_string(),
            ty: self.property_type(pi, &pr.kind),
            dim: pr.array_dim.max(1) as usize,
            flags: pr.flags,
            inner_flags,
            enumeration: self.property_enum(pi, &pr.kind),
            public: p.exports[export].flags & 4 != 0,
        })
    }

    /// Path of the enumeration of a byte property, or of the element of a byte array.
    pub(crate) fn property_enum(&self, pi: usize, kind: &PropertyKind) -> Option<String> {
        let p = &self.packages[pi].package;
        match kind {
            PropertyKind::Byte { enumeration } if *enumeration != 0 => Some(match ObjectRef::from(*enumeration) {
                ObjectRef::Export(_) => format!("{}.{}", self.packages[pi].name, p.object_path((*enumeration).into())),
                _ => p.object_path((*enumeration).into()),
            }),
            PropertyKind::Array { inner } => match ObjectRef::from(*inner) {
                ObjectRef::Export(e) => objects::read_property(p, &self.packages[pi].data, e).ok().and_then(|q| self.property_enum(pi, &q.kind)),
                _ => None,
            },
            _ => None,
        }
    }

    fn link_class(&mut self, c: usize) {
        if self.classes[c].linked {
            return;
        }
        self.classes[c].linked = true;
        // Mods build only: a mod's new class takes everything from its parent (mod_classes.rs).
        #[cfg(feature = "mods")]
        if let Some(parent) = self.mod_class_parent(c) {
            self.link_class(parent);
            self.link_mod_class(c, parent);
            self.clear_slot_cache();
            return;
        }
        let super_class = self.classes[c].super_name.as_ref().and_then(|n| self.class_index.get(&n.to_lowercase()).copied());
        if let Some(s) = super_class {
            self.link_class(s);
        }
        let (pi, export) = (self.classes[c].package, self.classes[c].export);
        let mut props = super_class.map(|s| self.classes[s].props.clone()).unwrap_or_default();
        let mut functions = crate::types::HashMap::default();
        let mut states = crate::types::HashMap::default();
        #[allow(unused_mut)]
        let mut members = {
            let lp = &self.packages[pi];
            let header = objects::read_class(&lp.package, &lp.data, export).map(|k| k.state.body.header.children).unwrap_or(0);
            children(&lp.package, &lp.data, header)
        };
        for m in members {
            let class_name = self.packages[pi].package.export_class(m).to_string();
            if class_name.ends_with("Property") {
                if let Some(d) = self.property_def(pi, m) {
                    props.push(d);
                }
            } else if class_name == "Function" {
                let f = self.function_def(c, None, pi, m);
                functions.insert(f.name.to_lowercase(), self.functions.len());
                self.call_hooks.prepare(&f.name);
                self.functions.push(f);
            } else if class_name == "State" {
                let s = self.state_def(c, pi, m);
                states.insert(s.name.to_lowercase(), s);
            }
        }
        let prop_index = props.iter().enumerate().map(|(i, p)| (p.name.to_lowercase(), i)).collect();
        // UClass::Register: the class takes from its parent the flags of mask 0x6EC (Config,
        // Transient, Localized, SafeReplace, RuntimeStatic, NoUserCreate, PerObjectConfig).
        let inherited = super_class.map_or(0, |s| self.classes[s].flags & 0x6ec);
        let class = &mut self.classes[c];
        class.flags |= inherited;
        class.super_class = super_class;
        class.props = props;
        class.prop_index = prop_index;
        class.functions = functions;
        class.states = states;
        self.clear_slot_cache();
    }

    fn function_def(&self, owner: usize, state: Option<String>, pi: usize, export: usize) -> FunctionDef {
        let lp = &self.packages[pi];
        let p = &lp.package;
        let name = p.name(p.exports[export].object_name).to_string();
        let f = objects::read_function(p, &lp.data, export).ok();
        let mut params = Vec::new();
        let mut locals = Vec::new();
        let mut ret = None;
        if let Some(f) = &f {
            for ch in children(p, &lp.data, f.body.header.children) {
                if !p.export_class(ch).ends_with("Property") {
                    continue;
                }
                let Ok(pr) = objects::read_property(p, &lp.data, ch) else {
                    continue;
                };
                let def = PropDef {
                    name: p.name(p.exports[ch].object_name).to_string(),
                    ty: self.property_type(pi, &pr.kind),
                    dim: pr.array_dim.max(1) as usize,
                    flags: pr.flags,
                    inner_flags: 0,
                    enumeration: None,
                    public: p.exports[ch].flags & 4 != 0,
                };
                if pr.flags & prop::RETURN_PARM != 0 {
                    ret = Some(def.ty.clone());
                    continue;
                }
                if pr.flags & prop::PARM != 0 {
                    params.push(ParamDef {
                        local: locals.len(),
                        out: pr.flags & prop::OUT_PARM != 0,
                        optional: pr.flags & prop::OPTIONAL_PARM != 0,
                        coerce: pr.flags & prop::COERCE_PARM != 0,
                    });
                }
                locals.push(def);
            }
        }
        let local_index = locals.iter().enumerate().map(|(i, l)| (l.name.to_lowercase(), i)).collect();
        // UObject::ProcessInternal runs the bytecode without looking at the
        // function's flags: 0x1000 is missing on "private" functions that nevertheless have
        // a body (PlayerController.ClientShake, the only one of this kind).
        let code = f.as_ref().map(|f| Code::new(f.body.script.clone(), pi));
        FunctionDef {
            name,
            owner,
            state,
            package: pi,
            export,
            flags: f.as_ref().map_or(0, |f| f.flags),
            native: f.as_ref().map_or(0, |f| f.native),
            params,
            locals,
            local_index,
            ret,
            code,
        }
    }

    fn state_def(&mut self, owner: usize, pi: usize, export: usize) -> StateDef {
        let (name, parent, members, script, auto, simulated, ignore_mask, probe_mask) = {
            let lp = &self.packages[pi];
            let p = &lp.package;
            let name = p.name(p.exports[export].object_name).to_string();
            let s = objects::read_state(p, &lp.data, export).ok();
            let super_field = s.as_ref().map(|s| s.body.header.super_field).filter(|&x| x != 0);
            let parent = super_field.map(|x| p.object_name(x.into()).to_string());
            // Path "[Package.]Class.State": the class is the next-to-last element.
            let parent_class = super_field.and_then(|x| {
                let path = p.object_path(x.into());
                let parts: Vec<&str> = path.split('.').collect();
                (parts.len() >= 2).then(|| parts[parts.len() - 2].to_lowercase())
            });
            let members = s.as_ref().map(|s| children(p, &lp.data, s.body.header.children)).unwrap_or_default();
            let script = s.as_ref().map(|s| s.body.script.clone()).unwrap_or_default();
            let auto = s.as_ref().is_some_and(|s| s.flags & 2 != 0);
            let simulated = s.as_ref().is_some_and(|s| s.flags & 4 != 0);
            let ignore_mask = s.as_ref().map(|s| s.ignore_mask).unwrap_or(u64::MAX);
            let probe_mask = s.as_ref().map_or(0, |s| s.probe_mask);
            (name, (parent, parent_class), members, script, auto, simulated, ignore_mask, probe_mask)
        };
        let (parent, parent_class) = parent;
        let mut functions = crate::types::HashMap::default();
        let mut ignores = Vec::new();
        for m in members {
            if self.packages[pi].package.export_class(m) == "Function" {
                let f = self.function_def(owner, Some(name.clone()), pi, m);
                // "Ignored" functions: declared without a body in the state.
                if f.code.is_none() && f.flags & func::NATIVE == 0 {
                    ignores.push(f.name.to_lowercase());
                }
                functions.insert(f.name.to_lowercase(), self.functions.len());
                self.call_hooks.prepare(&f.name);
                self.functions.push(f);
            }
        }
        let mut labels = crate::types::HashMap::default();
        for e in &script {
            if let openxiii_script::bytecode::Kind::LabelTable(table) = &e.kind {
                for (n, off) in table {
                    let n = self.packages[pi].package.name(*n).to_lowercase();
                    if n != "none" {
                        labels.insert(n, *off);
                    }
                }
            }
        }
        let only_stop = script.len() == 1 && matches!(script[0].kind, openxiii_script::bytecode::Kind::Stop);
        let code = (!script.is_empty() && !only_stop).then(|| Code::new(script, pi));
        StateDef { name, parent, parent_class, functions, code, labels, ignores, ignore_mask, probe_mask, auto, simulated }
    }

    /// Zero value of a type.
    pub fn zero(&mut self, ty: &Type, dim: usize) -> Value {
        if dim > 1 {
            let one = self.zero(ty, 1);
            return Value::Array(vec![one; dim]);
        }
        match ty {
            Type::Byte => Value::Byte(0),
            Type::Int => Value::Int(0),
            Type::Bool => Value::Bool(false),
            Type::Float => Value::Float(0.0),
            Type::Name => Value::Name(crate::value::NameStruct::from(String::new())),
            Type::Str => Value::Str(String::new()),
            Type::Object(_) => Value::Object(None),
            Type::Class(_) => Value::Class(None),
            Type::Array(_) => Value::Array(Vec::new()),
            Type::Delegate => Value::Delegate(None, crate::value::NameStruct::EMPTY),
            Type::Struct(name) => {
                let fields: Vec<PropDef> = self.struct_def(name).map(|s| s.fields.clone()).unwrap_or_default();
                let values = fields.iter().map(|f| self.zero(&f.ty, f.dim)).collect();
                // Vector / rotator: inline form (same value, unboxed).
                Value::Struct(name.into(), values).fold()
            }
            Type::Unknown => Value::Void,
        }
    }

    /// Binary order of the fields of an already loaded struct: UStruct::SerializeBin follows
    /// TFieldIterator, which gives the struct's own fields before those of its parent
    /// ("struct Plane extends Vector": W, X, Y, Z). Indices into `fields` (parent first).
    pub(crate) fn serial_order(&self, name: &str) -> Option<Vec<usize>> {
        let mut key = name.to_lowercase();
        if !self.struct_exports.contains_key(&key) {
            key = key.rsplit('.').next().unwrap_or(&key).to_string();
        }
        let def = self.structs.get(&key)?;
        let parent = def.super_struct.as_ref().and_then(|p| self.serial_order(p)).unwrap_or_default();
        let mut order: Vec<usize> = (parent.len()..def.fields.len()).collect();
        order.extend(parent);
        Some(order)
    }

    pub fn struct_def(&mut self, name: &str) -> Option<&StructDef> {
        let mut key = name.to_lowercase();
        if !self.struct_exports.contains_key(&key) {
            key = key.rsplit('.').next().unwrap_or(&key).to_string();
        }
        if !self.structs.contains_key(&key) {
            let (pi, export) = *self.struct_exports.get(&key)?;
            let (members, super_struct) = {
                let lp = &self.packages[pi];
                let s = openxiii_script::read_struct(&lp.package, &lp.data, export).ok()?;
                let parent = (s.header.super_field != 0).then(|| lp.package.object_name(s.header.super_field.into()).to_string());
                (children(&lp.package, &lp.data, s.header.children), parent)
            };
            // Fields of the parent struct first ("struct Plane extends Vector": X, Y, Z, W).
            let mut fields: Vec<PropDef> = match &super_struct {
                Some(parent) => self.struct_def(parent).map(|s| s.fields.clone()).unwrap_or_default(),
                None => Vec::new(),
            };
            fields.extend(members.into_iter().filter_map(|m| self.property_def(pi, m)));
            let index = fields.iter().enumerate().map(|(i, f)| (f.name.to_lowercase(), i)).collect();
            self.structs.insert(key.clone(), StructDef { name: name.to_string(), super_struct, fields, index });
        }
        self.structs.get(&key)
    }

    pub(crate) fn create_cdo(&mut self, c: usize) {
        if self.classes[c].cdo != usize::MAX {
            return;
        }
        let parent = self.classes[c].super_class;
        if let Some(s) = parent {
            self.create_cdo(s);
        }
        let values = self.cdo_values(c);
        let pi = self.classes[c].package;
        let name = format!("Default__{}", self.classes[c].name);
        let outer = self.packages[pi].name.clone();
        self.object_paths.insert(format!("{outer}.{name}").to_lowercase());
        let props = crate::types::Props::new(values, self.case_hot(c));
        self.objects.push(Some(Object { class: c, props, state: None, state_class: None, name, outer }));
        self.classes[c].cdo = self.objects.len() - 1;
        self.set_intrinsics(self.classes[c].cdo);
    }

    /// Default values of a class as UClass::Serialize loads them: those of the parent
    /// class as they are in memory, then the class's serialized defaultproperties
    /// (tagged properties), the configuration (LoadConfig) and the localized texts.
    fn cdo_values(&mut self, c: usize) -> Vec<Value> {
        let parent = self.classes[c].super_class;
        let props = self.classes[c].props.clone();
        let mut values: Vec<Value> = match parent {
            Some(s) => self.objects[self.classes[s].cdo].as_ref().map(|o| o.props.to_vec()).unwrap_or_default(),
            None => Vec::new(),
        };
        for p in props.iter().skip(values.len()) {
            let v = self.zero(&p.ty, p.dim);
            values.push(v);
        }
        // defaultproperties of the class.
        let (pi, export) = (self.classes[c].package, self.classes[c].export);
        let defaults = {
            let lp = &self.packages[pi];
            objects::read_class(&lp.package, &lp.data, export).map(|k| k.defaults).unwrap_or_default()
        };
        // Mods build only: a mod's new class has no serialized defaults of its own (its package
        // and export are its parent's): it keeps the parent's values in memory.
        #[cfg(feature = "mods")]
        let defaults = if self.mod_class_parent(c).is_some() { Vec::new() } else { defaults };
        for d in defaults {
            let pname = self.packages[pi].package.name(d.name).to_lowercase();
            let Some(&slot) = self.classes[c].prop_index.get(&pname) else {
                continue;
            };
            let ty = props[slot].ty.clone();
            let v = self.convert_default(pi, &ty, &d.value);
            if props[slot].dim > 1 {
                if let Value::Array(items) = &mut values[slot] {
                    if let Some(item) = items.get_mut(d.array_index as usize) {
                        *item = v;
                    }
                }
            } else if d.array_index == 0 {
                values[slot] = v;
            }
        }
        // UClass::Serialize: configuration (LoadConfig) then localized texts (LoadLocalized) of the
        // default values, section "Class" of the file "Package.<language>".
        self.load_config_values(c, &mut values, None);
        let (class_name, package) = (self.classes[c].name.clone(), self.packages[pi].name.clone());
        self.load_localized_values(c, &mut values, &class_name, &package);
        // Mods build only: values set by mods, applied last so they win over the package,
        // the .ini and the language files (mod_defaults.rs).
        #[cfg(feature = "mods")]
        self.apply_mod_defaults(c, &mut values);
        values
    }

    /// Class destroyed by UGameEngine::LoadMap's CollectGarbage(RF_Native) then reloaded
    /// (class_gc.rs): its default values start again from those serialized in its package,
    /// on top of those of its parent class in memory (reloaded before it if it is too). The
    /// changes the script had made to "default." disappear. Returns the names of the
    /// properties that change (debugging).
    pub(crate) fn reload_class_defaults(&mut self, c: usize) -> Vec<String> {
        let cdo = self.classes[c].cdo;
        if cdo == usize::MAX || self.objects.get(cdo).is_none_or(|o| o.is_none()) {
            return Vec::new();
        }
        let values = self.cdo_values(c);
        let values = crate::types::Props::new(values, self.case_hot(c));
        let Some(old) = self.objects[cdo].as_mut().map(|o| std::mem::replace(&mut o.props, values)) else { return Vec::new() };
        // Name, Class, Outer… held by the engine; references to script objects (subobjects
        // Emitters, Controls…) resolved as at load time.
        self.set_intrinsics(cdo);
        self.resolve_object_refs(cdo);
        let now = self.objects[cdo].as_ref().map(|o| o.props.clone()).unwrap_or_default();
        now.iter()
            .zip(old.iter())
            .enumerate()
            .filter(|(_, (n, o))| n != o)
            .map(|(i, (n, o))| format!("{}={} (was {})", self.classes[c].props.get(i).map_or("?", |p| p.name.as_str()), self.value_paths(n), self.value_paths(o)))
            .collect()
    }

    /// Value for the debug log, objects designated by Outer.Name.
    fn value_paths(&self, v: &Value) -> String {
        match v {
            Value::Object(Some(o)) => self.objects.get(*o).and_then(|x| x.as_ref()).map_or(format!("#{o}"), |x| format!("{}.{}#{o}", x.outer, x.name)),
            Value::Array(items) => format!("[{}]", items.iter().map(|x| self.value_paths(x)).collect::<Vec<_>>().join(", ")),
            other => format!("{other}"),
        }
    }

    /// Converts a tagged property value to the declared type.
    pub(crate) fn convert_default(&mut self, pi: usize, ty: &Type, v: &openxiii_package::Value) -> Value {
        use openxiii_package::Value as P;
        let path = |vm: &Vm, r: i32| -> String {
            let p = &vm.packages[pi].package;
            match ObjectRef::from(r) {
                ObjectRef::Export(_) => {
                    format!("{}.{}", vm.packages[pi].name, p.object_path(r.into()))
                }
                _ => p.object_path(r.into()),
            }
        };
        match (ty, v) {
            (Type::Bool, P::Bool(b)) => Value::Bool(*b),
            (Type::Byte, P::Byte(b)) => Value::Byte(*b),
            (Type::Int, P::Int(i)) => Value::Int(*i),
            (Type::Float, P::Float(f)) => Value::Float(*f),
            (Type::Str, P::Str(s)) => Value::Str(s.clone()),
            // NAME_None: the VM's empty name.
            (Type::Name, P::Name(n)) => Value::Name(crate::value::NameStruct::from(crate::none_name(self.packages[pi].package.name(*n)))),
            (Type::Class(_), P::Object(r)) => Value::Class(self.class_of_ref(pi, *r)),
            (Type::Object(_), P::Object(r)) => {
                if *r == 0 {
                    Value::Object(None)
                } else {
                    Value::Asset(path(self, *r))
                }
            }
            (Type::Struct(_), P::Vector(x)) => Value::vector(x[0], x[1], x[2]),
            (Type::Struct(_), P::Rotator(r)) => Value::rotator(r[0], r[1], r[2]),
            (Type::Struct(name), P::Raw(raw)) => {
                let mut zero = self.zero(ty, 1);
                // Common immutable structs: float or byte fields in a row (an inline form
                // is unpacked to be written field by field, then packed again).
                if let Some((_, fields)) = zero.struct_mut() {
                    let lname = name.to_lowercase();
                    if lname.ends_with("color") && raw.len() == 4 {
                        for (f, b) in fields.iter_mut().zip(raw.iter()) {
                            *f = Value::Byte(*b);
                        }
                    } else if fields.iter().all(|f| matches!(f, Value::Float(_))) && raw.len() == fields.len() * 4 {
                        let order = self.serial_order(name).unwrap_or_else(|| (0..fields.len()).collect());
                        for (k, &i) in order.iter().enumerate() {
                            fields[i] = Value::Float(f32::from_le_bytes(raw[k * 4..k * 4 + 4].try_into().unwrap()));
                        }
                    } else {
                        // General case: fields in a row, in their binary form (compact indices
                        // for names and objects). Kept only if the whole byte stream is consumed.
                        let mut pos = 0;
                        if let Some(decoded) = self.decode_typed(pi, ty, 1, raw, &mut pos, None) {
                            if pos == raw.len() {
                                return decoded;
                            }
                        }
                    }
                }
                zero.fold()
            }
            (Type::Array(_), P::Raw(raw)) => self.convert_raw_array(pi, ty, raw, None).unwrap_or_else(|| self.zero(ty, 1)),
            _ => self.zero(ty, 1),
        }
    }

    /// Raw serialized dynamic array: count (compact index) then the elements in a row.
    pub(crate) fn convert_raw_array(&mut self, pi: usize, ty: &Type, raw: &[u8], map: Option<&HashMap<usize, ObjId>>) -> Option<Value> {
        let Type::Array(inner) = ty else { return None };
        let mut r = openxiii_package::Reader::new(raw, 0);
        let count = r.compact().ok()?;
        if !(0..=100_000).contains(&count) {
            return None;
        }
        let mut pos = r.pos;
        let mut items = Vec::with_capacity(count as usize);
        for _ in 0..count {
            items.push(self.decode_typed(pi, inner, 1, raw, &mut pos, map)?);
        }
        (pos == raw.len()).then_some(Value::Array(items))
    }

    /// Value serialized in binary (UProperty::SerializeItem) according to its type: static
    /// arrays element by element, dynamic arrays preceded by their count (compact index),
    /// structs field by field according to their definition.
    pub(crate) fn decode_typed(&mut self, pi: usize, ty: &Type, dim: usize, raw: &[u8], pos: &mut usize, map: Option<&HashMap<usize, ObjId>>) -> Option<Value> {
        if dim > 1 {
            let mut items = Vec::with_capacity(dim);
            for _ in 0..dim {
                items.push(self.decode_typed(pi, ty, 1, raw, pos, map)?);
            }
            return Some(Value::Array(items));
        }
        match ty {
            Type::Array(inner) => {
                let mut r = openxiii_package::Reader::new(raw, *pos);
                let count = r.compact().ok()?;
                if !(0..=100_000).contains(&count) {
                    return None;
                }
                *pos = r.pos;
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    items.push(self.decode_typed(pi, inner, 1, raw, pos, map)?);
                }
                Some(Value::Array(items))
            }
            Type::Struct(name) => {
                let fields: Vec<PropDef> = self.struct_def(name)?.fields.clone();
                let mut values = vec![Value::Void; fields.len()];
                for i in self.serial_order(name).unwrap_or_else(|| (0..fields.len()).collect()) {
                    // UStruct::SerializeBin: UProperty::ShouldSerializeValue drops CPF_Transient (0x2000)
                    // fields from a persistent archive (structFlash.bMemDuck).
                    if !should_serialize_value(fields[i].flags, false, true) {
                        values[i] = self.zero(&fields[i].ty, fields[i].dim);
                        continue;
                    }
                    values[i] = self.decode_typed(pi, &fields[i].ty, fields[i].dim, raw, pos, map)?;
                }
                Some(Value::Struct(name.into(), values.into()).fold())
            }
            _ => {
                let mut v = self.zero(ty, 1);
                self.decode_raw(pi, &mut v, raw, pos, map)?;
                Some(v)
            }
        }
    }
}

impl Vm {
    /// Reads `raw[pos..]` into the shape of `value` (zero value of the type), recursively.
    pub(crate) fn decode_raw(&self, pi: usize, value: &mut Value, raw: &[u8], pos: &mut usize, map: Option<&HashMap<usize, ObjId>>) -> Option<()> {
        let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
            let s = raw.get(*pos..*pos + n)?;
            *pos += n;
            Some(s)
        };
        let compact = |pos: &mut usize| -> Option<i32> {
            let mut r = openxiii_package::Reader::new(raw, *pos);
            let v = r.compact().ok()?;
            *pos = r.pos;
            Some(v)
        };
        let valid = |r: i32| {
            let p = &self.packages[pi].package;
            match ObjectRef::from(r) {
                ObjectRef::None => true,
                ObjectRef::Import(i) => i < p.imports.len(),
                ObjectRef::Export(e) => e < p.exports.len(),
            }
        };
        match value {
            Value::Float(f) => *f = f32::from_le_bytes(take(pos, 4)?.try_into().ok()?),
            Value::Int(i) => *i = i32::from_le_bytes(take(pos, 4)?.try_into().ok()?),
            Value::Byte(b) => *b = take(pos, 1)?[0],
            // UBoolProperty::SerializeItem: one byte outside tagged properties.
            Value::Bool(b) => *b = take(pos, 1)?[0] != 0,
            Value::Name(n) => {
                let i = compact(pos)?;
                let p = &self.packages[pi].package;
                *n = if i >= 0 && (i as usize) < p.names.len() {
                    crate::value::NameStruct::from(p.name(i).to_string())
                } else {
                    return None;
                };
                if n.eq_ignore_ascii_case("none") {
                    *n = "".into();
                }
            }
            Value::Class(c) => {
                let r = compact(pos)?;
                if !valid(r) {
                    return None;
                }
                *c = self.class_of_ref(pi, r);
            }
            Value::Str(s) => {
                let len = compact(pos)?;
                if len >= 0 {
                    let bytes = take(pos, len as usize)?;
                    *s = bytes.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
                } else {
                    let bytes = take(pos, (-len) as usize * 2)?;
                    *s = String::from_utf16_lossy(&bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect::<Vec<_>>());
                }
            }
            Value::Object(_) | Value::Asset(_) => {
                let r = compact(pos)?;
                if !valid(r) {
                    return None;
                }
                if let (ObjectRef::Export(e), Some(map)) = (ObjectRef::from(r), map) {
                    if let Some(&id) = map.get(&e) {
                        *value = Value::Object(Some(id));
                        return Some(());
                    }
                }
                *value = if r == 0 {
                    Value::Object(None)
                } else {
                    let p = &self.packages[pi].package;
                    Value::Asset(match ObjectRef::from(r) {
                        ObjectRef::Export(_) => {
                            format!("{}.{}", self.packages[pi].name, p.object_path(r.into()))
                        }
                        _ => p.object_path(r.into()),
                    })
                };
            }
            Value::Struct(name, fields) => {
                for i in self.serial_order(name).unwrap_or_else(|| (0..fields.len()).collect()) {
                    self.decode_raw(pi, fields.get_mut(i)?, raw, pos, map)?;
                }
            }
            // Inline vector / rotator: unpacked, decoded field by field, packed again.
            Value::Vector(_) | Value::Rotator(_) => {
                let (name, fields) = value.struct_mut()?;
                let name = *name;
                for i in self.serial_order(&name).unwrap_or_else(|| (0..fields.len()).collect()) {
                    self.decode_raw(pi, fields.get_mut(i)?, raw, pos, map)?;
                }
                *value = std::mem::replace(value, Value::Void).fold();
            }
            Value::Array(items) => {
                for f in items.iter_mut() {
                    self.decode_raw(pi, f, raw, pos, map)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

/// Key of a struct: "class.struct" in lowercase (the path's last two elements).
pub fn struct_key(path: &str) -> String {
    let parts: Vec<&str> = path.split('.').collect();
    let n = parts.len();
    if n >= 2 {
        format!("{}.{}", parts[n - 2], parts[n - 1]).to_lowercase()
    } else {
        path.to_lowercase()
    }

}

impl Vm {
    /// Name of the script packages as the engine writes it: an FName keeps the spelling of its
    /// first registration in the name table (FName::Init, case-insensitive comparison), and a
    /// package is registered by whoever requests it (name table of the linker that
    /// imports it) before its own file is read. The file name ("xidcine.u") and
    /// the package's entry in its own table ("xidcine") therefore only count as a fallback: the
    /// original's logs write XIII, XIDCine, XIDPawn, XIDMaps, Engine, Core.
    fn canonical_package_names(&mut self) {
        let mut names = Vec::with_capacity(self.packages.len());
        for (i, lp) in self.packages.iter().enumerate() {
            let own = &lp.name;
            // Import of a top-level package by another package.
            let imported = self.packages.iter().enumerate().filter(|(j, _)| *j != i).find_map(|(_, other)| {
                let p = &other.package;
                p.imports.iter().find_map(|imp| {
                    (imp.package == 0 && p.name(imp.class_name).eq_ignore_ascii_case("Package") && p.name(imp.object_name).eq_ignore_ascii_case(own))
                        .then(|| p.name(imp.object_name).to_string())
                })
            });
            // Otherwise, the entry of its own name table (Core, Engine…).
            let listed = || lp.package.names.iter().find(|n| n.name.eq_ignore_ascii_case(own)).map(|n| n.name.clone());
            names.push(imported.or_else(listed).unwrap_or_else(|| own.clone()));
        }
        for (lp, name) in self.packages.iter_mut().zip(names) {
            lp.name = name;
        }
    }
}

/// UProperty::ShouldSerializeValue: `w14` and `w1c` are the archive's words at +0x14 and +0x1C
/// (a package-loading archive: 0 and 1). Flags 0x1000, 0x2000 (CPF_Transient) and 0x20000000 drop the value.
pub(crate) fn should_serialize_value(flags: u32, w14: bool, w1c: bool) -> bool {
    flags & 0x1000 == 0 && (flags & 0x2000 == 0 || !w1c) && (flags & 0x2000_0000 == 0 || !w14)
}

