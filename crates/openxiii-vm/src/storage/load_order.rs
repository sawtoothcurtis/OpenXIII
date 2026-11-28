//! Load order of the UE2 linker (XIII's Core.dll), for the order of the PostLoads: emulation
//! of ULinkerLoad::CreateExport, IndexToObject, CreateImport,
//! Preload and the passes of UObject::EndLoad, on the
//! packages' tables, without creating anything in the VM.
//!
//! XIII's EndLoad does not empty GObjLoaded between rounds: it redoes Preload passes on
//! all RF_NeedLoad objects as long as objects are created, then calls ConditionalPostLoad on
//! each one in GObjLoaded order, i.e. creation order. What an object refers to while
//! serializing is created (CreateExport) when the reference is read; CreateExport
//! immediately preloads the object's class, Preload first preloads a struct's SuperField,
//! and UStruct::Link preloads each of its children.
//!
//! Modelled serializations: UField (SuperField, Next), UStruct (ScriptText, Children, bytecode
//! references, Link), UClass (dependencies, ClassWithin, default values), properties (class,
//! struct, enum, Inner…), objects with tagged properties (state frame included), ULevel (actor
//! list). Native data (textures, meshes, sounds, BSP) creates no class or
//! template: it is not read.

use crate::types::Type;
use crate::Vm;
use openxiii_package::{ObjectRef, Package};
use std::collections::HashMap;

/// A linker: package of the VM or of a library, tables and data.
#[derive(Debug, Clone)]
struct Linker {
    name: String,
    package: std::rc::Rc<Package>,
    /// Package of the VM (its bytes stay there), otherwise bytes of the library's package.
    vm: Option<usize>,
    data: Option<std::rc::Rc<Vec<u8>>>,
    /// Export path (without the package, lowercase) → index.
    paths: HashMap<String, Vec<usize>>,
}

/// Created object (GObjLoaded entry).
#[derive(Debug, Clone)]
pub struct Node {
    pub linker: usize,
    pub export: usize,
    /// "Package.Group.Name".
    pub path: String,
    /// Name of the object's class ("Class", "SpriteEmitter"…).
    pub class: String,
    /// Object whose serialization created it (debugging).
    pub creator: Option<usize>,
}

#[derive(Debug, Default, Clone)]
pub struct LoadOrder {
    linkers: Vec<Linker>,
    by_name: HashMap<String, Option<usize>>,
    created: HashMap<(usize, usize), usize>,
    pub nodes: Vec<Node>,
    need_load: Vec<bool>,
    post_loaded: Vec<bool>,
    /// GObjLoaded of the current load (node indices) and BeginLoad depth.
    loaded: Vec<usize>,
    begin_count: u32,
    /// Nodes in the order of their PostLoads.
    pub post_loads: Vec<usize>,
    /// Nodes in the order of their serialization (Preload that reads the object).
    pub preloads: Vec<usize>,
    /// Objects being serialized.
    stack: Vec<usize>,
}

impl Vm {
    /// Linker of package `name` (GetPackageLinker), created when needed.
    fn lo_linker(&mut self, lo: &mut LoadOrder, name: &str) -> Option<usize> {
        let key = name.to_ascii_lowercase();
        if let Some(&l) = lo.by_name.get(&key) {
            return l;
        }
        let found = if let Some(pi) = self.packages.iter().position(|p| p.name.eq_ignore_ascii_case(name)) {
            Some((std::rc::Rc::new(self.packages[pi].package.clone()), Some(pi), None))
        } else {
            self.library.as_mut().and_then(|lib| lib.get(name)).map(|p| (std::rc::Rc::new(p.package.clone()), None, Some(std::rc::Rc::new(p.data.clone()))))
        };
        let l = found.map(|(package, vm, data)| {
            let mut paths: HashMap<String, Vec<usize>> = HashMap::new();
            for e in 0..package.exports.len() {
                paths.entry(package.object_path(ObjectRef::Export(e)).to_ascii_lowercase()).or_default().push(e);
            }
            lo.linkers.push(Linker { name: name.to_string(), package, vm, data, paths });
            lo.linkers.len() - 1
        });
        lo.by_name.insert(key, l);
        l
    }

    /// `f` with the package and the bytes of linker `l`: those of a VM package are lent for the
    /// duration of the call (without copying).
    fn lo_with_data<R>(&mut self, lo: &LoadOrder, l: usize, f: impl FnOnce(&mut Vm, &Package, &[u8]) -> R) -> R {
        let pkg = lo.linkers[l].package.clone();
        match (lo.linkers[l].vm, &lo.linkers[l].data) {
            (Some(pi), _) => {
                let data = std::mem::take(&mut self.packages[pi].data);
                let r = f(self, &pkg, &data);
                self.packages[pi].data = data;
                r
            }
            (None, Some(d)) => {
                let d = d.clone();
                f(self, &pkg, &d)
            }
            (None, None) => f(self, &pkg, &[]),
        }
    }

    /// UObject::BeginLoad.
    fn lo_begin(&mut self, lo: &mut LoadOrder) {
        lo.begin_count += 1;
    }

    /// UObject::EndLoad: Preload passes until nothing more is created,
    /// then ConditionalPostLoad in GObjLoaded order.
    fn lo_end(&mut self, lo: &mut LoadOrder) {
        if lo.begin_count == 1 {
            let mut done = 0;
            while done != lo.loaded.len() {
                let n = lo.loaded.len();
                for i in 0..n {
                    let node = lo.loaded[i];
                    if lo.need_load[node] {
                        self.lo_preload(lo, node);
                    }
                }
                if n == lo.loaded.len() {
                    for i in 0..n {
                        let node = lo.loaded[i];
                        if !lo.post_loaded[node] {
                            lo.post_loaded[node] = true;
                            lo.post_loads.push(node);
                        }
                    }
                }
                done = n;
            }
            lo.loaded.clear();
        }
        lo.begin_count -= 1;
    }

    /// IndexToObject: export created, import resolved in its package.
    fn lo_index(&mut self, lo: &mut LoadOrder, l: usize, r: i32) -> Option<usize> {
        match ObjectRef::from(r) {
            ObjectRef::None => None,
            ObjectRef::Export(e) => self.lo_create_export(lo, l, e),
            ObjectRef::Import(i) => {
                // VerifyImport (nested BeginLoad / EndLoad), then CreateExport
                // in the source linker.
                self.lo_begin(lo);
                let target = self.lo_verify_import(lo, l, i);
                self.lo_end(lo);
                let (tl, te) = target?;
                self.lo_create_export(lo, tl, te)
            }
        }
    }

    /// VerifyImport: linker of the outermost package, then the export with the same path (and the
    /// same class if there are several). A package, or an intrinsic class without an export, has
    /// no loaded object.
    fn lo_verify_import(&mut self, lo: &mut LoadOrder, l: usize, i: usize) -> Option<(usize, usize)> {
        let pkg = lo.linkers[l].package.clone();
        let imp = &pkg.imports[i];
        if imp.package == 0 {
            return None;
        }
        // Outermost package and path below it.
        let mut parts = vec![pkg.name(imp.object_name).to_string()];
        let mut outer = imp.package;
        let outermost;
        loop {
            match ObjectRef::from(outer) {
                ObjectRef::Import(j) => {
                    let o = &pkg.imports[j];
                    if o.package == 0 {
                        outermost = pkg.name(o.object_name).to_string();
                        break;
                    }
                    parts.push(pkg.name(o.object_name).to_string());
                    outer = o.package;
                }
                _ => return None,
            }
        }
        parts.reverse();
        let path = parts.join(".").to_ascii_lowercase();
        let class = pkg.name(imp.class_name).to_string();
        let tl = self.lo_linker(lo, &outermost)?;
        let tp = lo.linkers[tl].package.clone();
        let list = lo.linkers[tl].paths.get(&path)?.clone();
        // ULinkerLoad::VerifyImport: same class name and same class package (GetExportClassPackage); several: the ExportHash chain returns the last one registered; none: failure, except VertMesh which is
        // looked up again as SkeletalMesh. (No import of the game's packages falls into these edge cases:
        // crates/openxiii-package/tests/measure_verify_import_core2.rs.)
        let class_package = pkg.name(imp.class_package).to_string();
        let own = lo.linkers[tl].name.clone();
        let pick = |cls: &str| list.iter().copied().rev().find(|&e| tp.export_class(e).eq_ignore_ascii_case(cls) && tp.export_class_package(e, &own).eq_ignore_ascii_case(&class_package));
        let e = pick(&class).or_else(|| if class.eq_ignore_ascii_case("VertMesh") { pick("SkeletalMesh") } else { None })?;
        Some((tl, e))
    }

    /// Is class `name` (short name) `wanted` or a subclass of `wanted`?
    fn lo_class_derives(&self, name: &str, wanted: &str) -> bool {
        let mut c = self.class(name);
        while let Some(id) = c {
            if self.classes[id].name.eq_ignore_ascii_case(wanted) {
                return true;
            }
            c = self.classes[id].super_class;
        }
        false
    }

    /// CreateExport.
    fn lo_create_export(&mut self, lo: &mut LoadOrder, l: usize, e: usize) -> Option<usize> {
        if let Some(&n) = lo.created.get(&(l, e)) {
            return Some(n);
        }
        let pkg = lo.linkers[l].package.clone();
        let ex = &pkg.exports[e];
        // CreateExport: only an export whose flags have a bit of the linker's LoadFlags (RF_LoadForClient | RF_LoadForServer,
        // 0x30000, read from ULinkerLoad+0xA0) is created; the others (ScriptText, editor Camera…) return NULL.
        if ex.flags & 0x30000 == 0 {
            return None;
        }
        // CreateExport: an object of class Camera or PlayerInput (FName 0xA2 / 0xA3) is not created.
        if matches!(pkg.export_class(e).to_ascii_lowercase().as_str(), "camera" | "playerinput") {
            return None;
        }
        // Class of the object, preloaded right away.
        if let Some(c) = self.lo_index(lo, l, ex.class) {
            self.lo_preload(lo, c);
        }
        if ex.package != 0 {
            self.lo_index(lo, l, ex.package);
        }
        if let Some(&n) = lo.created.get(&(l, e)) {
            return Some(n);
        }
        let n = lo.nodes.len();
        let path = format!("{}.{}", lo.linkers[l].name, pkg.object_path(ObjectRef::Export(e)));
        let creator = lo.stack.last().copied();
        lo.nodes.push(Node { linker: l, export: e, path, class: pkg.export_class(e).to_string(), creator });
        lo.need_load.push(true);
        lo.post_loaded.push(false);
        lo.created.insert((l, e), n);
        lo.loaded.push(n);
        // UStruct: SuperField.
        if is_struct_class(pkg.export_class(e)) && ex.super_ != 0 {
            self.lo_index(lo, l, ex.super_);
        }
        Some(n)
    }

    /// ULinkerLoad::Preload: SuperField first, then Serialize.
    fn lo_preload(&mut self, lo: &mut LoadOrder, n: usize) {
        if !lo.need_load[n] {
            return;
        }
        let (l, e) = (lo.nodes[n].linker, lo.nodes[n].export);
        let pkg = lo.linkers[l].package.clone();
        if is_struct_class(&lo.nodes[n].class) && pkg.exports[e].super_ != 0 {
            if let Some(s) = self.lo_index(lo, l, pkg.exports[e].super_) {
                self.lo_preload(lo, s);
            }
        }
        lo.need_load[n] = false;
        lo.preloads.push(n);
        lo.stack.push(n);
        self.lo_serialize(lo, n);
        lo.stack.pop();
    }

    /// Serialize of the object: the references read in order.
    fn lo_serialize(&mut self, lo: &mut LoadOrder, n: usize) {
        let (l, e) = (lo.nodes[n].linker, lo.nodes[n].export);
        let class = lo.nodes[n].class.clone();
        let linker_name = lo.linkers[l].name.clone();
        let refs_then = self.lo_with_data(lo, l, |vm, pkg, data| vm.lo_object_refs(pkg, &linker_name, data, e, &class));
        for step in refs_then {
            match step {
                Step::Ref(r) => {
                    self.lo_index(lo, l, r);
                }
                Step::Preload(r) => {
                    if let Some(x) = self.lo_index(lo, l, r) {
                        self.lo_preload(lo, x);
                    }
                }
                Step::LinkChildren(first) => {
                    // UStruct::Link: preloads each child of the chain as long as its Outer is
                    // this struct; Next is read from the preloaded child.
                    let mut r = first;
                    let mut guard = 0;
                    while r != 0 && guard < 100_000 {
                        guard += 1;
                        let Some(x) = self.lo_index(lo, l, r) else { break };
                        self.lo_preload(lo, x);
                        self.lo_property_link(lo, x);
                        let (cl, ce) = (lo.nodes[x].linker, lo.nodes[x].export);
                        let cp = lo.linkers[cl].package.clone();
                        if cl != l || cp.exports[ce].package != (e as i32 + 1) {
                            break;
                        }
                        r = self.lo_with_data(lo, cl, |_, pkg, data| field_next(pkg, data, ce));
                    }
                }
            }
        }
    }

    /// UObject::PostLoad: ObjectFlags | RF_DebugPostLoad
    /// (0x20000000, read back by ConditionalPostLoad); in the editor only, a name with spaces is reported
    /// (appMsgf, except GIsUCC) then renamed spaces → "_" with ObjectFlags | 0x80004. In game GIsEditor is 0:
    /// the VM does not keep these flags (a node's PostLoad is recorded by `post_loaded`). Returns (flags, new
    /// name, message).
    pub(crate) fn object_post_load(flags: u32, name: &str, is_editor: bool, is_ucc: bool) -> (u32, Option<String>, bool) {
        let flags = flags | 0x2000_0000;
        if !is_editor || !name.contains(' ') {
            return (flags, None, false);
        }
        (flags | 0x80004, Some(name.replace(' ', "_")), !is_ucc)
    }

    /// UProperty::Link of a property preloaded by UStruct::Link: UArrayProperty /
    /// UFixedArrayProperty preload Inner and link it, UMapProperty
    /// Key and Value, UStructProperty preloads Struct.
    fn lo_property_link(&mut self, lo: &mut LoadOrder, n: usize) {
        if !lo.nodes[n].class.ends_with("Property") {
            return;
        }
        let (l, e) = (lo.nodes[n].linker, lo.nodes[n].export);
        let Ok(p) = self.lo_with_data(lo, l, |_, pkg, data| openxiii_script::objects::read_property(pkg, data, e)) else { return };
        use openxiii_script::objects::PropertyKind::*;
        let (links, preloads): (Vec<i32>, Vec<i32>) = match p.kind {
            Array { inner } | FixedArray { inner, .. } => (vec![inner], vec![]),
            Map { key, value } => (vec![key, value], vec![]),
            Struct { structure } => (vec![], vec![structure]),
            _ => (vec![], vec![]),
        };
        for r in links {
            if let Some(x) = self.lo_index(lo, l, r) {
                self.lo_preload(lo, x);
                self.lo_property_link(lo, x);
            }
        }
        for r in preloads {
            if let Some(x) = self.lo_index(lo, l, r) {
                self.lo_preload(lo, x);
            }
        }
    }

    /// Steps of the serialization of an export of a VM package.
    fn lo_object_refs(&mut self, pkg: &Package, pkg_name: &str, data: &[u8], e: usize, class: &str) -> Vec<Step> {
        use openxiii_script::objects;
        let mut out = Vec::new();
        match class {
            "Class" | "Function" | "State" | "Struct" => {
                let Ok((body, _)) = openxiii_script::read_struct_partial(pkg, data, e) else { return out };
                if class != "Class" {
                    // UObject::Serialize: tagged properties of a native object (none).
                }
                let h = &body.header;
                out.push(Step::Ref(h.super_field));
                out.push(Step::Ref(h.next));
                out.push(Step::Ref(h.script_text));
                out.push(Step::Ref(h.children));
                for s in &body.script {
                    s.walk(&mut |x| {
                        if let Some(r) = expr_ref(x) {
                            out.push(Step::Ref(r));
                        }
                    });
                }
                // Link: SuperField preloaded, then the children.
                out.push(Step::Preload(h.super_field));
                out.push(Step::LinkChildren(h.children));
                if class == "Class" {
                    if let Ok(c) = objects::read_class(pkg, data, e) {
                        for d in &c.dependencies {
                            out.push(Step::Ref(d.class));
                        }
                        out.push(Step::Ref(c.within));
                        // Two classes can have the same short name (XIIIPersos.Johansson, XIDPawn.Johansson): the owner is qualified.
                        let owner = format!("{pkg_name}.{}", pkg.object_name(ObjectRef::Export(e)));
                        self.lo_tagged_refs(pkg, &owner, &c.defaults, &mut out);
                    }
                }
            }
            c if c.ends_with("Property") => {
                if let Ok(p) = objects::read_property(pkg, data, e) {
                    out.push(Step::Ref(p.super_field));
                    out.push(Step::Ref(p.next));
                    use objects::PropertyKind::*;
                    match p.kind {
                        Byte { enumeration } => out.push(Step::Ref(enumeration)),
                        Object { class } => out.push(Step::Ref(class)),
                        Class { class, meta_class } => {
                            out.push(Step::Ref(class));
                            out.push(Step::Ref(meta_class));
                        }
                        Struct { structure } => out.push(Step::Ref(structure)),
                        Array { inner } => out.push(Step::Ref(inner)),
                        Map { key, value } => {
                            out.push(Step::Ref(key));
                            out.push(Step::Ref(value));
                        }
                        Delegate { function } => out.push(Step::Ref(function)),
                        FixedArray { inner, .. } => out.push(Step::Ref(inner)),
                        _ => {}
                    }
                }
            }
            "Enum" | "Const" | "TextBuffer" => {
                let Ok(od) = pkg.object_data(data, e) else { return out };
                let mut r = openxiii_package::Reader::new(data, od.end);
                if class != "TextBuffer" {
                    out.push(Step::Ref(r.compact().unwrap_or(0)));
                    out.push(Step::Ref(r.compact().unwrap_or(0)));
                }
            }
            _ => {
                let Ok(od) = pkg.object_data(data, e) else { return out };
                if let Some(st) = &od.state {
                    out.push(Step::Ref(st.node));
                    out.push(Step::Ref(st.state_node));
                    out.push(Step::Preload(st.node));
                }
                // Class of the object, qualified by its package (import: "Package.Class"; export: this package).
                let qualified = match ObjectRef::from(pkg.exports[e].class) {
                    ObjectRef::Import(k) if k < pkg.imports.len() => pkg.object_path(ObjectRef::Import(k)),
                    ObjectRef::Export(k) if k < pkg.exports.len() => format!("{pkg_name}.{}", pkg.object_name(ObjectRef::Export(k))),
                    _ => class.to_string(),
                };
                self.lo_tagged_refs(pkg, &qualified, &od.properties, &mut out);
                // ANavigationPoint::Serialize: after the properties, newPathList (FReachSpec: Start then End); these references
                // create the navigation points that the level's actor list does not mention (seen on Hual01b, Kello01a, PRock03, SSH101b).
                if let (Some(nav), Some(c)) = (self.class("NavigationPoint"), self.class(&qualified)) {
                    if self.is_child_of(c, nav) {
                        let end = (pkg.exports[e].serial_offset + pkg.exports[e].serial_size) as usize;
                        let mut r = openxiii_package::Reader::new(&data[..end.min(data.len())], od.end);
                        for (start, target, _) in crate::navigation::read_reach_specs(&mut r) {
                            out.push(Step::Ref(start));
                            out.push(Step::Ref(target));
                        }
                    }
                }
                // ULevel: the actor list.
                if class == "Level" {
                    let end = (pkg.exports[e].serial_offset + pkg.exports[e].serial_size) as usize;
                    let mut r = openxiii_package::Reader::new(&data[..end], od.end);
                    if let (Ok(n), Ok(_)) = (r.i32(), r.i32()) {
                        for _ in 0..n.max(0) {
                            match r.compact() {
                                Ok(x) => out.push(Step::Ref(x)),
                                Err(_) => break,
                            }
                        }
                    }
                    // The level's model (Level.Model) follows the actor list: created, hence post-loaded,
                    // after the last actor (USA02: its FLight::Init draws right after the AEmitter::PostLoad
                    // of the last Emitter).
                    if let Some(m) = openxiii_assets::model::level_model(pkg, data) {
                        out.push(Step::Ref(m as i32 + 1));
                    }
                }
            }
        }
        out
    }

    /// References of tagged properties (default values of a class `class_export`, or
    /// properties of an object): objects and classes, structs and arrays decoded by their type.
    fn lo_tagged_refs(&mut self, pkg: &Package, owner: &str, props: &[openxiii_package::Property], out: &mut Vec<Step>) {
        use openxiii_package::Value as P;
        let owner = self.class(owner);
        for p in props {
            match &p.value {
                P::Object(r) => out.push(Step::Ref(*r)),
                P::Raw(raw) => {
                    let pname = pkg.name(p.name).to_ascii_lowercase();
                    let ty = owner.and_then(|c| self.classes[c].prop_index.get(&pname).map(|&s| self.classes[c].props[s].ty.clone()));
                    let ty = match ty {
                        Some(t) => t,
                        // UStaticMesh::Materials: native array of FStaticMeshMaterial (byte
                        // EnableCollision, then the material).
                        None if pname == "materials" => {
                            let mut r = openxiii_package::Reader::new(raw, 0);
                            let n = r.compact().unwrap_or(0);
                            for _ in 0..n.clamp(0, 10_000) {
                                if r.u8().is_err() {
                                    break;
                                }
                                match r.compact() {
                                    Ok(x) => out.push(Step::Ref(x)),
                                    Err(_) => break,
                                }
                            }
                            continue;
                        }
                        None => continue,
                    };
                    let mut pos = 0;
                    let mut refs = Vec::new();
                    if self.lo_typed_refs(&ty, raw, &mut pos, &mut refs).is_some() {
                        out.extend(refs.into_iter().map(Step::Ref));
                    }
                }
                _ => {}
            }
        }
    }

    /// References of a binary value (UProperty::SerializeItem) from its type.
    fn lo_typed_refs(&mut self, ty: &Type, raw: &[u8], pos: &mut usize, out: &mut Vec<i32>) -> Option<()> {
        let compact = |pos: &mut usize| -> Option<i32> {
            let mut r = openxiii_package::Reader::new(raw, *pos);
            let v = r.compact().ok()?;
            *pos = r.pos;
            Some(v)
        };
        match ty {
            Type::Float | Type::Int => *pos += 4,
            Type::Byte | Type::Bool => *pos += 1,
            Type::Name => {
                compact(pos)?;
            }
            Type::Str => {
                let len = compact(pos)?;
                *pos += if len >= 0 { len as usize } else { (-len) as usize * 2 };
            }
            Type::Object(_) | Type::Class(_) => out.push(compact(pos)?),
            Type::Array(inner) => {
                let n = compact(pos)?;
                for _ in 0..n.clamp(0, 100_000) {
                    self.lo_typed_refs(inner, raw, pos, out)?;
                }
            }
            Type::Struct(name) => {
                let fields = self.struct_def(name)?.fields.clone();
                for i in self.serial_order(name).unwrap_or_else(|| (0..fields.len()).collect()) {
                    if fields[i].flags & 0x2000 != 0 {
                        continue;
                    }
                    for _ in 0..fields[i].dim.max(1) {
                        self.lo_typed_refs(&fields[i].ty, raw, pos, out)?;
                    }
                }
            }
            _ => return None,
        }
        (*pos <= raw.len()).then_some(())
    }

    /// UObject::StaticLoadObject of an object "Package.Path": Begin/EndLoad around
    /// ULinkerLoad::Create (export of this path, of this class if given).
    pub fn lo_load(&mut self, lo: &mut LoadOrder, path: &str, class: Option<&str>) -> Option<usize> {
        let (pkg_name, rest) = path.split_once('.')?;
        self.lo_begin(lo);
        let n = (|| {
            let l = self.lo_linker(lo, pkg_name)?;
            let p = lo.linkers[l].package.clone();
            let list = lo.linkers[l].paths.get(&rest.to_ascii_lowercase()).cloned().or_else(|| {
                // Create searches by name in the whole package.
                let last = rest.rsplit('.').next().unwrap_or(rest);
                let v: Vec<usize> = (0..p.exports.len()).filter(|&e| p.object_name(ObjectRef::Export(e)).eq_ignore_ascii_case(last)).collect();
                (!v.is_empty()).then_some(v)
            })?;
            // ULinkerLoad::Create → FindExportIndex: the export with this name whose class has the requested name
            // (hash table), failing that whose parent class has it (linear search); no fallback to an export of another
            // class (a native class without a script half is not loaded).
            let e = match class {
                None => list.first().copied()?,
                Some(c) => list.iter().copied().find(|&e| p.export_class(e).eq_ignore_ascii_case(c)).or_else(|| list.iter().copied().find(|&e| self.lo_class_derives(p.export_class(e), c)))?,
            };
            self.lo_create_export(lo, l, e)
        })();
        self.lo_end(lo);
        n
    }

    /// Roots taken from a load list of the original (lines "Type;Path;Class"): after
    /// the given roots, each line of the list that our loads have not yet
    /// produced and whose object does not exist becomes a root (the script's DynamicLoadObject);
    /// we stop at the first order difference. Returns the added roots.
    pub fn load_order_oracle_roots(&mut self, roots: &[(String, Option<String>)], oracle: &[String]) -> (Vec<String>, Vec<String>) {
        let mut lo = LoadOrder::default();
        for (path, class) in roots {
            self.lo_load(&mut lo, path, class.as_deref());
        }
        let mut added = Vec::new();
        loop {
            let ours = self.lo_lines(&lo);
            let same = ours.iter().zip(oracle).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count();
            if same >= oracle.len() || same < ours.len() {
                return (added, ours);
            }
            let row: Vec<&str> = oracle[same].split(';').collect();
            let (path, class) = (row.get(1).copied().unwrap_or(""), row.get(2).copied().unwrap_or(""));
            if lo.nodes.iter().any(|n| n.path.eq_ignore_ascii_case(path)) || added.len() > 2000 {
                return (added, ours);
            }
            added.push(format!("{class}:{path}"));
            self.lo_load(&mut lo, path, Some(class));
        }
    }

    /// `oxiii load-order`: PostLoads of the Classes, Emitters and ParticleEmitters of the given
    /// roots, in order ("Type Path Class").
    pub fn load_order_report(&mut self, roots: &[(String, Option<String>)]) -> Vec<String> {
        let mut lo = LoadOrder::default();
        for (path, class) in roots {
            self.lo_load(&mut lo, path, class.as_deref());
        }
        self.lo_lines(&lo)
    }

    fn lo_lines(&self, lo: &LoadOrder) -> Vec<String> {
        let pe = self.class("ParticleEmitter");
        let em = self.class("Emitter");
        let mut out = Vec::new();
        let all = false;
        for &n in &lo.post_loads {
            let node = &lo.nodes[n];
            if all {
                let by = node.creator.map(|c| lo.nodes[c].path.clone()).unwrap_or_default();
                out.push(format!("*;{};{};{by}", node.path, node.class));
                continue;
            }
            let kind = if node.class == "Class" {
                "Class"
            } else if let Some(c) = self.class(&node.class) {
                if pe.is_some_and(|p| self.is_child_of(c, p)) {
                    "ParticleEmitter"
                } else if em.is_some_and(|p| self.is_child_of(c, p)) {
                    "Emitter"
                } else {
                    continue;
                }
            } else {
                continue;
            };
            out.push(format!("{kind};{};{}", node.path, node.class));
        }
        out
    }
}

/// Startup loads before the first map, "Class:Path" in order: the engine's class
/// (Engine.GameEngine, loaded by the launcher), then the loop of UEngine::Init
/// for each UClass of GObjObjects without a linker (native class registered by
/// its DLL, Core, Engine, GUI, XIIIMP, XIDCine, Xiii…), StaticLoadObject of its script half;
/// those already loaded by a previous one are not listed. The order is that of the
/// registration of the native classes (DLL initializers), as in the original
/// game, where it gives the exact order of the startup PostLoads
/// and of the map. UGameEngine::Init's LoadMap(Entry) loads nothing more (its Cameras are
/// not created).
pub const BOOT_ROOTS: &[&str] = &[
    "Class:Engine.GameEngine",
    "Class:GUI.GUICheckBoxButton",
    "Class:GUI.GUISlider",
    "Class:GUI.GUIVertScrollBar",
    "Class:GUI.GUIImage",
    "Class:GUI.GUIListBox",
    "Class:XIIIMP.TheDuck",
    "Class:XIIIMP.BotController",
    "Class:XIDCine.CineController2",
    "Class:XIDCine.PositionInfo",
    "Class:XIII.CWndSFXTrigger",
    "Class:Engine.MatchMakingManager",
    "Class:Engine.Cubemap",
    "Class:Engine.SparkEmitter",
    "Class:Engine.SequEditProps",
    "Class:Engine.AnimEditProps",
    "Class:Engine.MeshEditProps",
    "Class:Engine.ParticleMaterial",
    "Class:Engine.SinusModifier",
    "Class:Engine.TexMatrix",
    "Class:Engine.TexEnvMap",
    "Class:Engine.TexOscillator",
    "Class:Engine.TexRotator",
    "Class:Engine.TexScaler",
    "Class:Engine.TexPanner",
    "Class:Engine.FadeColor",
    "Class:Engine.ConstantColor",
    "Class:Engine.Combiner",
    "Class:Engine.ColorModifier",
    "Class:Engine.Console",
    "Class:Core.Commandlet",
    "Class:Engine.BeamEmitter",
    "Class:Engine.MapList",
    "Class:Engine.AutoLadder",
    "Class:Engine.LineOfSightTrigger",
    "Class:Engine.Door",
    "Class:Engine.Vehicle",
    "Class:Engine.InternetInfo",
    "Class:Engine.LiftCenter",
    "Class:Engine.WarpZoneMarker",
    "Class:Engine.Scout",
    "Class:Engine.Camera",
    "Class:Engine.Note",
    "Class:Engine.PolyMarker",
    "Class:Engine.ClipMarker",
];

/// PostLoad of a loaded object: ParticleEmitter (Reset, two draws) or Emitter (VRand + FRand).
#[derive(Debug, Clone)]
pub enum PostLoad {
    ParticleEmitter { linker_name: String, export: usize, path: String },
    Emitter { linker_name: String, export: usize, path: String },
    /// UModel::PostLoad: FLight::Init of the static lights of the level's model.
    Model { linker_name: String, export: usize, path: String },
}

impl Vm {
    /// Runs on the state of the emulated linker (created on the first call with the startup
    /// loads) and returns the ParticleEmitter and Emitter PostLoads it produced.
    pub(crate) fn lo_run(&mut self, f: impl FnOnce(&mut Vm, &mut LoadOrder)) -> Vec<PostLoad> {
        // After a travel, the state comes from the previous map (EngineState).
        if self.load_order.is_none() {
            self.load_order = self.engine.load_order.take();
        }
        let fresh = self.load_order.is_none();
        let mut lo = match self.load_order.take() {
            Some(lo) => lo,
            None => {
                let mut lo = LoadOrder::default();
                for r in BOOT_ROOTS {
                    let (c, p) = r.split_once(':').unwrap_or(("", r));
                    self.lo_load(&mut lo, p, Some(c));
                }
                lo
            }
        };
        // The startup PostLoads count with the first load.
        let start = if fresh { 0 } else { lo.post_loads.len() };
        f(self, &mut lo);
        let pe = self.class("ParticleEmitter");
        let em = self.class("Emitter");
        let mut out = Vec::new();
        for &n in &lo.post_loads[start..] {
            let node = &lo.nodes[n];
            let linker_name = lo.linkers[node.linker].name.clone();
            if node.class.eq_ignore_ascii_case("Model") {
                out.push(PostLoad::Model { linker_name, export: node.export, path: node.path.clone() });
                continue;
            }
            let Some(c) = self.class(&node.class) else { continue };
            if pe.is_some_and(|p| self.is_child_of(c, p)) {
                out.push(PostLoad::ParticleEmitter { linker_name, export: node.export, path: node.path.clone() });
            } else if em.is_some_and(|p| self.is_child_of(c, p)) {
                out.push(PostLoad::Emitter { linker_name, export: node.export, path: node.path.clone() });
            }
        }
        self.load_order = Some(lo);
        out
    }
}

impl Vm {
    /// UGameEngine::LoadMap, CollectGarbage: only the classes that survive the
    /// travel (class_gc.rs) and their members (functions, properties, templates) stay loaded; the objects of the
    /// map and of the data packages will be recreated (and their PostLoads redone) if they are used again.
    pub(crate) fn lo_prune_for_travel(&self, lo: LoadOrder) -> LoadOrder {
        let alive: Vec<String> = self.classes_surviving_travel().into_iter().map(|c| format!("{}.{}", self.packages[self.classes[c].package].name, self.classes[c].name).to_ascii_lowercase()).collect();
        let alive: std::collections::HashSet<String> = alive.into_iter().collect();
        let keep = |path: &str| -> bool {
            let p = path.to_ascii_lowercase();
            let mut parts = p.splitn(3, '.');
            match (parts.next(), parts.next()) {
                (Some(a), Some(b)) => alive.contains(&format!("{a}.{b}")),
                _ => false,
            }
        };
        // Linkers of the map left and of the data packages: forgotten (ResetLoaders); a
        // later load creates a new one. Their objects do not survive.
        let mut linkers = lo.linkers;
        let mut by_name = lo.by_name;
        for (i, k) in linkers.iter_mut().enumerate() {
            let gone = match k.vm {
                Some(pi) => self.packages.get(pi).is_none_or(|p| p.name.starts_with('\u{1}') || Some(pi) == self.map_package),
                None => true,
            };
            if gone {
                k.data = None;
                k.vm = None;
                by_name.retain(|_, v| *v != Some(i));
            }
        }
        let mut out = LoadOrder { linkers, by_name, ..Default::default() };
        let mut remap: HashMap<usize, usize> = HashMap::new();
        for (i, n) in lo.nodes.into_iter().enumerate() {
            if !keep(&n.path) {
                continue;
            }
            let j = out.nodes.len();
            remap.insert(i, j);
            out.created.insert((n.linker, n.export), j);
            out.need_load.push(lo.need_load[i]);
            out.post_loaded.push(lo.post_loaded[i]);
            out.nodes.push(Node { creator: None, ..n });
        }
        out.post_loads = lo.post_loads.iter().filter_map(|i| remap.get(i).copied()).collect();
        out
    }
}

enum Step {
    Ref(i32),
    Preload(i32),
    LinkChildren(i32),
}

fn is_struct_class(c: &str) -> bool {
    matches!(c, "Class" | "Function" | "State" | "Struct")
}

/// Next field (UField) of a struct child.
fn field_next(pkg: &Package, data: &[u8], e: usize) -> i32 {
    let class = pkg.export_class(e);
    if is_struct_class(class) {
        return openxiii_script::read_struct_partial(pkg, data, e).map(|(b, _)| b.header.next).unwrap_or(0);
    }
    let Ok(od) = pkg.object_data(data, e) else { return 0 };
    let mut r = openxiii_package::Reader::new(data, od.end);
    let _super = r.compact().unwrap_or(0);
    r.compact().unwrap_or(0)
}

/// Object reference of a bytecode expression (read before its sub-expressions).
fn expr_ref(e: &openxiii_script::bytecode::Expr) -> Option<i32> {
    use openxiii_script::bytecode::Kind::*;
    match &e.kind {
        LocalVariable(r) | InstanceVariable(r) | DefaultVariable(r) | MetaCast(r, _) | FinalFunction(r, _) | ObjectConst(r) | NativeParm(r) | DynamicCast(r, _) | StructCmpEq(r, _, _) | StructCmpNe(r, _, _) | StructMember(r, _) | DelegateFunction(r, _, _) => Some(*r),
        _ => None,
    }
}
