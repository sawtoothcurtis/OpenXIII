//! Script classes that survive travel: UGameEngine::LoadMap purges the old
//! level through UObject::CollectGarbage(RF_Native); only native objects and what they
//! reach remain. A script class that nothing reaches is destroyed, then reloaded when the
//! next level uses it: UClass::Serialize then resets ClassUnique to 0.
//! Hence "Map.XIIISoloMutator0" on every map (GameInfo only refers to it by a string,
//! MutatorClass) while XIIIGameInfo, XIIIPlayerController… (named in the bytecode of the menus and
//! of the console) continue their numbering.
//!
//! Roots: the classes registered by a DLL (RF_Native, exported PrivateStaticClass symbol)
//! and the live objects outside the map's package (interactions, console, menus, transient
//! objects). A class reaches its parent class, its default values, the types of its
//! properties (class, metaclass, struct, enum) and what the bytecode of its
//! functions and states refers to (object constants, casts, properties and final functions of other
//! classes, whose Outer is the class). An object reaches its class and the objects and classes of
//! its properties. The level's actors are purged by force (RF_…0x400) and hold nothing.
//!
//! LoadMap collects twice: before loading, only if the map changes (
//! appStricmp(URL.Map, LastURL.Map), all actors eliminated except Entry's), then after
//! (the actors of the new level's package missing from its Actors eliminated). Reloading
//! the same map (checkpoint after a death) therefore only does the second one: what the reloaded
//! level refers to stays loaded and keeps its ClassUnique ("Hual04c.XIIISoloMutator1",
//! "ApacheTopRotor1" in the original's log).

use crate::types::Type;
use crate::value::{ClassId, ObjId, Value};
use crate::Vm;
use openxiii_script::bytecode::{Expr, Kind};
use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

/// Native classes ("package.class") of the DLLs of the System folder: exports
/// ?PrivateStaticClass@<A|U><Class>@@… of each <Package>.dll.
pub(crate) fn native_classes(system: &Path) -> &'static HashSet<String> {
    static NATIVES: OnceLock<HashSet<String>> = OnceLock::new();
    NATIVES.get_or_init(|| {
        let mut out = HashSet::new();
        let Ok(dir) = std::fs::read_dir(system) else { return out };
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
                continue;
            }
            let package = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let Ok(data) = std::fs::read(&path) else { continue };
            for name in pe_export_names(&data) {
                if let Some(rest) = name.strip_prefix("?PrivateStaticClass@") {
                    if let Some(cpp) = rest.split("@@").next() {
                        if cpp.len() > 1 {
                            out.insert(format!("{package}.{}", cpp[1..].to_lowercase()));
                        }
                    }
                }
            }
        }
        out
    })
}

/// Names of the export table of a PE (IMAGE_EXPORT_DIRECTORY).
fn pe_export_names(d: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let mut out = Vec::new();
    let Some(pe) = u32_at(0x3C) else { return out };
    if d.get(pe..pe + 4) != Some(b"PE\0\0") {
        return out;
    }
    let (Some(sections), Some(opt_size)) = (u16_at(pe + 6), u16_at(pe + 20)) else { return out };
    let opt = pe + 24;
    // PE32: directory 0 (exports) at +96 of the optional header.
    let Some(export_rva) = u32_at(opt + 96) else { return out };
    let table = opt + opt_size;
    let file_of = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let (vsize, va, raw) = (u32_at(s + 8)?, u32_at(s + 12)?, u32_at(s + 20)?);
            (va <= rva && rva < va + vsize.max(1)).then(|| rva - va + raw)
        })
    };
    let Some(ed) = file_of(export_rva) else { return out };
    let (Some(count), Some(names)) = (u32_at(ed + 0x18), u32_at(ed + 0x20).and_then(|r| file_of(r))) else { return out };
    for i in 0..count {
        let Some(at) = u32_at(names + i * 4).and_then(|r| file_of(r)) else { continue };
        let end = d[at..].iter().position(|&b| b == 0).map_or(d.len(), |e| at + e);
        out.push(String::from_utf8_lossy(&d[at..end]).into_owned());
    }
    out
}

/// Entry level: UGameEngine::Init loads it (LoadMap(Entry), log "Start LoadMap: Entry") and keeps it for the whole session
/// (GEntry, UGameEngine+0x1B4, held for the garbage collector by UGameEngine::Serialize); OpenXIII does not load it. The objects
/// it spawns take number 0 of their class: GameInfo0 (LoadMap), GameReplicationInfo0 (GameInfo.PreBeginPlay), Mutator0
/// ("Base Mutator is Entry.Mutator0"), BroadcastHandler0 and AccessControl0 (GameInfo.InitGame, Engine.u bytecode): the same classes
/// therefore start at 1 in the maps (BroadcastHandler1, AccessControl1, GameReplicationInfo1 in single-player).
pub(crate) const NAMES_ENTRY: &[&str] = &["gameinfo", "gamereplicationinfo", "mutator", "broadcasthandler", "accesscontrol"];

/// Loading screen: XIII.exe constructs UEngLoadingAnimationThread right after GEngine->Init (WinMain, then the
/// constructor), which creates two UFinalBlends in the transient package (Transient.FinalBlend0 and FinalBlend1).
/// OpenXIII does not create these objects: the session's FinalBlend name
/// counter therefore starts at 2 (first `new class'FinalBlend'` of the script: ShadowProjector.ShadowMaterial).
pub(crate) const NAMES_SCREEN_LOADING: &[(&str, u16)] = &[("finalblend", 2)];

/// First name number of a class in the session (lowercase class), before any creation by OpenXIII.
pub(crate) fn first_number(class_value: &str) -> u16 {
    NAMES_SCREEN_LOADING.iter().find(|(c, _)| *c == class_value).map_or(u16::from(NAMES_ENTRY.contains(&class_value)), |&(_, n)| n)
}

/// Classes of Entry's objects (its 26 exports and the objects above): Entry keeps them loaded from one map to the next.
const CLASSES_ENTRY: &[&str] = &["GameInfo", "GameReplicationInfo", "Mutator", "BroadcastHandler", "AccessControl", "LevelInfo", "DefaultPhysicsVolume",
    "Brush", "Camera", "PlayerStart", "Model", "Polys", "LevelSummary", "Level"];

impl Vm {
    /// Classes still loaded after the travel's CollectGarbage(RF_Native).
    pub fn classes_surviving_travel(&self) -> HashSet<ClassId> {
        self.travel_gc().0
    }

    /// The travel's CollectGarbage(RF_Native): classes and objects outside the map that remain. The
    /// controls and pages of the menus (GUIComponent) are not roots: only reached through
    /// another root (GUIController.MenuStack, open pages, their Controls). A closed page
    /// goes away with its class, whose ClassUnique restarts from 0 on reload (back to
    /// MapMenu through XIIIMenu0, XIIIMenuSelectProfile0, XIIIMenuContinue0, as in the original).
    pub fn travel_gc(&self) -> (HashSet<ClassId>, HashSet<ObjId>) {
        let map = self.map_package.map(|p| self.packages[p].name.to_lowercase());
        let in_map = |o: &crate::types::Object| map.as_ref().is_some_and(|m| o.outer.split('.').next().is_some_and(|p| p.eq_ignore_ascii_case(m)));
        let cdos: HashSet<ObjId> = self.classes.iter().map(|c| c.cdo).collect();
        let mut classes: Vec<ClassId> = Vec::new();
        let mut objects: Vec<ObjId> = Vec::new();
        if let Some(system) = self.system_dir.as_deref() {
            let natives = native_classes(system);
            for (c, def) in self.classes.iter().enumerate() {
                if natives.contains(&format!("{}.{}", self.packages[def.package].name.to_lowercase(), def.name.to_lowercase())) {
                    classes.push(c);
                }
            }
        }
        // GEntry: the classes of Entry's objects stay loaded (UGameEngine::Serialize holds it during collection).
        classes.extend(CLASSES_ENTRY.iter().filter_map(|n| self.class(n)));
        // Neither the menus' controls, nor the objects they created (Outer = a control: the
        // VideoPlayer of XIIIMenuContinue, which the collection's UObject::Serialize links to its page through
        // its Outer): they only remain if they are reached.
        let gui = self.class("GUIComponent");
        let is_gui = |o: &crate::types::Object| gui.is_some_and(|g| self.is_child_of(o.class, g));
        let paths_gui: HashSet<String> = self.objects.iter().flatten().filter(|o| is_gui(o)).map(|o| format!("{}.{}", o.outer, o.name).to_lowercase()).collect();
        for (id, o) in self.objects.iter().enumerate() {
            if let Some(o) = o {
                // Nor an object created by `new` (neither RF_RootSet nor RF_Native: XIIIMenuData of an old
                // GUIController, which held the page classes through its ClassObj).
                if !in_map(o) && !cdos.contains(&id) && !is_gui(o) && !paths_gui.contains(&o.outer.to_lowercase()) && !self.objects_new.contains(&id) {
                    objects.push(id);
                }
            }
        }
        for v in self.same_map_roots.iter().flatten() {
            self.gc_value(v, &mut classes, &mut objects, &in_map);
        }
        let mut seen_c: HashSet<ClassId> = HashSet::new();
        let mut seen_o: HashSet<ObjId> = HashSet::new();
        loop {
            if let Some(o) = objects.pop() {
                if !seen_o.insert(o) {
                    continue;
                }
                let Some(obj) = self.objects.get(o).and_then(|x| x.as_ref()) else { continue };
                classes.push(obj.class);
                for v in obj.props.iter() {
                    self.gc_value(v, &mut classes, &mut objects, &in_map);
                }
                continue;
            }
            let Some(c) = classes.pop() else { break };
            if !seen_c.insert(c) {
                continue;
            }
            let def = &self.classes[c];
            if let Some(s) = def.super_class {
                classes.push(s);
            }
            if let Some(cdo) = self.objects.get(def.cdo).and_then(|x| x.as_ref()) {
                for v in cdo.props.iter() {
                    self.gc_value(v, &mut classes, &mut objects, &in_map);
                }
            }
            for p in &def.props {
                self.gc_type(&p.ty, &mut classes);
                if let Some(e) = &p.enumeration {
                    self.gc_path(e, &mut classes);
                }
            }
            let mut funcs: Vec<usize> = def.functions.values().copied().collect();
            for st in def.states.values() {
                funcs.extend(st.functions.values().copied());
                if let Some(code) = &st.code {
                    self.gc_code(code, &mut classes);
                }
            }
            for f in funcs {
                let f = &self.functions[f];
                for l in &f.locals {
                    self.gc_type(&l.ty, &mut classes);
                }
                if let Some(t) = &f.ret {
                    self.gc_type(t, &mut classes);
                }
                if let Some(code) = &f.code {
                    self.gc_code(code, &mut classes);
                }
            }
        }
        (seen_c, seen_o)
    }

    /// What the reloaded level holds at the second CollectGarbage: the class of each object of the
    /// map's package (the actors outside ULevel::Actors are eliminated) and the serialized
    /// values of its properties, before any script (collection precedes BeginPlay).
    pub(crate) fn level_gc_roots(&mut self) -> Vec<Value> {
        let Some(pi) = self.map_package else { return Vec::new() };
        let level: Option<HashSet<usize>> = self.level_actor_list(pi).map(|l| l.into_iter().collect());
        let actor = self.class("Actor");
        let none = std::collections::HashMap::new();
        let mut out = Vec::new();
        for e in 0..self.packages[pi].package.exports.len() {
            let export = &self.packages[pi].package.exports[e];
            if export.serial_size == 0 {
                continue;
            }
            let Some(class) = self.class_of_ref(pi, export.class) else { continue };
            if actor.is_some_and(|a| self.is_child_of(class, a)) && level.as_ref().is_some_and(|l| !l.contains(&e)) {
                continue;
            }
            out.push(Value::Class(Some(class)));
            let props = {
                let lp = &self.packages[pi];
                lp.package.object_data(&lp.data, e).map(|o| o.properties).unwrap_or_default()
            };
            for prop in props {
                let pname = self.packages[pi].package.name(prop.name).to_lowercase();
                let Some(&slot) = self.classes[class].prop_index.get(&pname) else { continue };
                let ty = self.classes[class].props[slot].ty.clone();
                if matches!(ty, Type::Class(_) | Type::Object(_) | Type::Struct(_) | Type::Array(_)) {
                    out.push(self.convert_map_value(pi, &ty, &prop.value, &none));
                }
            }
        }
        out
    }

    fn gc_value(&self, v: &Value, classes: &mut Vec<ClassId>, objects: &mut Vec<ObjId>, in_map: &impl Fn(&crate::types::Object) -> bool) {
        match v {
            Value::Object(Some(o)) | Value::Delegate(Some(o), _) => {
                if self.objects.get(*o).and_then(|x| x.as_ref()).is_some_and(|x| !in_map(x)) {
                    objects.push(*o);
                }
            }
            Value::Class(Some(c)) => classes.push(*c),
            Value::Struct(_, fields) => {
                for f in fields.iter() {
                    self.gc_value(f, classes, objects, in_map);
                }
            }
            Value::Array(fields) => {
                for f in fields {
                    self.gc_value(f, classes, objects, in_map);
                }
            }
            _ => {}
        }
    }

    fn gc_type(&self, t: &Type, classes: &mut Vec<ClassId>) {
        match t {
            Type::Object(n) | Type::Class(n) => classes.extend(self.class(n)),
            // Struct "class.struct": its Outer is the class that declares it.
            Type::Struct(n) => classes.extend(n.split('.').next().and_then(|c| self.class(c))),
            Type::Array(inner) => self.gc_type(inner, classes),
            _ => {}
        }
    }

    /// Path "Package.Class[.Member…]": the class (Outer of its members), if it is one.
    fn gc_path(&self, path: &str, classes: &mut Vec<ClassId>) {
        let mut it = path.split('.');
        if let (Some(p), Some(c)) = (it.next(), it.next()) {
            if let Some(&id) = self.class_qualified.get(&format!("{p}.{c}").to_lowercase()) {
                classes.push(id);
            }
        }
    }

    /// Objects (not classes) the loaded classes' script code names: Texture'…' / Material'…'
    /// constants and the string literals of DynamicLoadObject-style paths ("Package.Name"), as
    /// full paths. The renderer prepares the materials among them at load, like the original,
    /// whose LoadMap has these objects in memory with their packages.
    pub fn script_object_constants(&self) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut walk = |code: &crate::types::Code| {
            let Some(pkg) = self.packages.get(code.package) else { return };
            let p = &pkg.package;
            let mut visit = |e: &Expr| match &e.kind {
                Kind::ObjectConst(r) => {
                    let class_name = match openxiii_package::ObjectRef::from(*r) {
                        openxiii_package::ObjectRef::Import(i) => p.name(p.imports[i].class_name).to_string(),
                        openxiii_package::ObjectRef::Export(i) => p.export_class(i).to_string(),
                        openxiii_package::ObjectRef::None => return,
                    };
                    if class_name.eq_ignore_ascii_case("Class") {
                        return;
                    }
                    let path = match openxiii_package::ObjectRef::from(*r) {
                        openxiii_package::ObjectRef::Export(_) => format!("{}.{}", pkg.name, p.object_path((*r).into())),
                        _ => p.object_path((*r).into()),
                    };
                    out.insert(path);
                }
                Kind::StringConst(s) if s.contains('.') && !s.contains(' ') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') => {
                    out.insert(s.clone());
                }
                _ => {}
            };
            for s in &code.statements {
                s.walk(&mut visit);
            }
        };
        for def in &self.classes {
            for st in def.states.values() {
                if let Some(code) = &st.code {
                    walk(code);
                }
                for &f in st.functions.values() {
                    if let Some(code) = &self.functions[f].code {
                        walk(code);
                    }
                }
            }
            for &f in def.functions.values() {
                if let Some(code) = &self.functions[f].code {
                    walk(code);
                }
            }
        }
        out.into_iter().collect()
    }

    fn gc_code(&self, code: &crate::types::Code, classes: &mut Vec<ClassId>) {
        let Some(pkg) = self.packages.get(code.package) else { return };
        let path = |r: i32| -> Option<String> {
            match openxiii_package::ObjectRef::from(r) {
                openxiii_package::ObjectRef::None => None,
                openxiii_package::ObjectRef::Export(_) => Some(format!("{}.{}", pkg.name, pkg.package.object_path(r.into()))),
                _ => Some(pkg.package.object_path(r.into())),
            }
        };
        let mut visit = |e: &Expr| {
            use Kind::*;
            let r = match &e.kind {
                // DelegateProperty (0x44) carries an FName, not an object: reading it as an object
                // kept XIIIMenu alive through XIIIMenuSave (name 285 taken for XIIIMenu.ContinueButton).
                ObjectConst(r) | DynamicCast(r, _) | MetaCast(r, _) | InstanceVariable(r) | DefaultVariable(r) | FinalFunction(r, _) | StructMember(r, _) | StructCmpEq(r, _, _) | StructCmpNe(r, _, _) => *r,
                DelegateFunction(r, _, _) => *r,
                _ => return,
            };
            if let Some(p) = path(r) {
                self.gc_path(&p, classes);
            }
        };
        for s in &code.statements {
            s.walk(&mut visit);
        }
    }
}

