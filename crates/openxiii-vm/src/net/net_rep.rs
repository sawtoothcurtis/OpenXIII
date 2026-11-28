//! Replication: what the network knows about a class and how a value goes into a bunch.
//!
//! UClass::Link: NetFields = CPF_Net (0x20) properties and FUNC_Net
//! (0x10) functions without a SuperField declared by the class, sorted by export index (compares
//! _LinkerIndex); ClassReps = those of the parent class then one record per element of
//! each own CPF_Net property, in Children order (RepIndex, UProperty+0x42).
//! UStruct::Link: RepOwner of a property = the last following property
//! (TFieldIterator: the class then its parents) whose condition has the same bytes; the
//! comparison is done here on the read expression, within the same package only.
//! UPackageMap::GetClassNetCache: FieldNetIndex = FieldsBase (parent) + rank;
//! new ConditionIndex if RepOwner is itself, otherwise RepOwner's; RepProperties = those
//! of the parent then the properties of the classes without CLASS_NativeReplication (0x800).
//! NetSerializeItem: byte on appCeilLogTwo(number of values of the
//! enum) bits (8 without an enum), int and float on 4 bytes, bool on 1 bit, name through
//! UPackageMap::SerializeName, object through UPackageMapLevel::SerializeObject,
//! compressed vector, rotator as high bytes, plane as SWORDs, other
//! structs field by field, dynamic arrays never.

use crate::net_map::MapIndex;
#[path = "net_names_codes.rs"]
mod net_names_codes;
use net_names_codes::NAMES_CODES;
use crate::netbits::{ceil_log_two, BitReader, BitWriter};
use crate::types::{Code, FuncId, Type};
use crate::value::{ClassId, ObjId, Value};
use crate::Vm;
use openxiii_package::{ObjectRef, Package};
use openxiii_script::decompile::children;
use openxiii_script::objects::{self, func, prop};
use std::collections::HashMap;
use std::rc::Rc;

pub const CLASS_NATIVE_REPLICATION: u32 = 0x800;

/// How NetSerializeItem writes a value.
#[derive(Debug, Clone)]
pub enum NetTy {
    /// Number of bits (appCeilLogTwo of the number of values, or 8).
    Byte(u32),
    Int,
    Bool,
    Float,
    Name,
    Str,
    /// Object or class; expected class ("Forged object" otherwise).
    Object(String),
    Delegate,
    Vector,
    Rotator,
    Plane,
    /// Other struct: (rank of the field in Value::Struct, dimension, type), TFieldIterator order.
    Struct(Vec<(usize, usize, NetTy)>),
    /// Dynamic array, map, fixed array: nothing.
    Skip,
}

/// Replicated property of a class.
#[derive(Debug)]
pub struct NetProp {
    pub slot: usize,
    pub name: String,
    /// Class that declares it, export of the property, RepOffset in that class's script.
    pub owner: ClassId,
    pub export: usize,
    pub rep_offset: u16,
    /// RepOwner: (class that declares it, slot).
    pub rep_owner: (ClassId, usize),
    pub rep_index: usize,
    pub dim: usize,
    pub ty: NetTy,
    pub flags: u32,
}

/// Replicated function (FUNC_Net without a parent function).
#[derive(Debug)]
pub struct NetFunc {
    pub func: FuncId,
    pub name: String,
    pub owner: ClassId,
    pub export: usize,
    pub rep_offset: u16,
    pub flags: u32,
    /// Parameters (local, dimension, type, bool?) in TFieldIterator order.
    pub params: Vec<(usize, usize, NetTy, bool)>,
}

#[derive(Debug, Clone)]
pub enum NetField {
    Prop(Rc<NetProp>),
    Func(Rc<NetFunc>),
}

impl NetField {
    pub fn same(&self, other: &NetField) -> bool {
        match (self, other) {
            (NetField::Prop(a), NetField::Prop(b)) => a.owner == b.owner && a.slot == b.slot,
            (NetField::Func(a), NetField::Func(b)) => a.func == b.func,
            _ => false,
        }
    }
    fn owner_export(&self) -> (ClassId, usize) {
        match self {
            NetField::Prop(p) => (p.owner, p.export),
            NetField::Func(f) => (f.owner, f.export),
        }
    }
}

/// Replication specific to a class (UClass::Link, UStruct::Link).
#[derive(Debug, Default)]
pub struct NetClass {
    /// NetFields of the class itself, sorted by export.
    pub own: Vec<NetField>,
    /// ClassReps: (slot, element), parents first.
    pub reps: Vec<(usize, usize)>,
    /// Replicated properties of the class and its parents, per slot.
    pub props: HashMap<usize, Rc<NetProp>>,
    /// TFieldIterator<UProperty> order of the CPF_Net properties (class then parents) and key of
    /// their condition.
    pub iter: Vec<(usize, String)>,
    /// Script of the class (conditions of the replication block).
    pub code: Option<Rc<Code>>,
    pub native_rep: bool,
    /// Slots of the CPF_Config properties (ConfigLink).
    pub config_slots: Vec<usize>,
}

/// FClassNetCache entry: FFieldNetCache(Field, FieldNetIndex, ConditionIndex).
#[derive(Debug, Clone)]
pub struct FieldCache {
    pub field: NetField,
    pub net_index: u32,
    pub condition: i32,
}

/// FClassNetCache of a class for a package map (FClassNetCache(UClass*),
/// filled by UPackageMap::GetClassNetCache).
#[derive(Debug)]
pub struct ClassNetCache {
    pub sup: Option<Rc<ClassNetCache>>,
    pub fields_base: u32,
    pub fields: Vec<FieldCache>,
    pub rep_condition_count: i32,
    pub rep_properties: Vec<FieldCache>,
}

impl ClassNetCache {
    /// FClassNetCache::GetMaxIndex.
    pub fn max_index(&self) -> u32 {
        self.fields_base + self.fields.len() as u32
    }

    /// FClassNetCache::GetFromIndex: the class, then its parents.
    pub fn from_index(&self, i: u32) -> Option<&FieldCache> {
        let mut c = Some(self);
        while let Some(x) = c {
            if x.fields_base <= i && i < x.max_index() {
                return x.fields.get((i - x.fields_base) as usize);
            }
            c = x.sup.as_deref();
        }
        None
    }

    /// FClassNetCache::GetFromField: the class, then its parents.
    pub fn from_field(&self, f: &NetField) -> Option<&FieldCache> {
        let mut c = Some(self);
        while let Some(x) = c {
            if let Some(found) = x.fields.iter().find(|e| e.field.same(f)) {
                return Some(found);
            }
            c = x.sup.as_deref();
        }
        None
    }

    pub fn from_prop(&self, owner: ClassId, slot: usize) -> Option<&FieldCache> {
        let mut c = Some(self);
        while let Some(x) = c {
            if let Some(found) = x.fields.iter().find(|e| matches!(&e.field, NetField::Prop(p) if p.owner == owner && p.slot == slot)) {
                return Some(found);
            }
            c = x.sup.as_deref();
        }
        None
    }
}

/// Replication caches kept by the VM.
#[derive(Debug, Default)]
pub struct RepCache {
    pub classes: HashMap<ClassId, Rc<NetClass>>,
    /// Objects placed in the map by path (lowercase), for IndexToObject.
    pub map_objects: Option<HashMap<String, ObjId>>,
    /// ULevel::NetTag (+0x108).
    pub net_tag: i32,
}

/// Comparison key of a condition: the expression without its positions, its object and name references replaced by the path of
/// the object or the text of the name. In the original the loaded script contains object pointers (SerializeExpr resolves the package's indices):
/// two conditions from different packages are the same bytes if they refer to the same objects (seen on the 764 classes of Core,
/// Engine, Gameplay, XIII and XIDPawn: XIIIPlayerReplicationInfo shares PlayerReplicationInfo's condition).
fn condition_key(code: &Code, offset: u16, package: &Package, package_name: &str) -> Option<String> {
    let &i = code.at.get(&(offset as u32))?;
    let text = format!("{:?}", code.statements[i].kind);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    // Tokens whose first integer is an object reference; NameConst: index into the name table.
    const OBJECTS: [&str; 14] = [
        "LocalVariable(", "InstanceVariable(", "DefaultVariable(", "NativeParm(", "MetaCast(", "DynamicCast(", "ObjectConst(", "StructMember(",
        "StructCmpEq(", "StructCmpNe(", "VirtualFunction(", "FinalFunction(", "GlobalFunction(", "DelegateProperty(",
    ];
    let object_value = |r: i32| -> String {
        match ObjectRef::from(r) {
            ObjectRef::None => "None".to_string(),
            ObjectRef::Import(k) if k < package.imports.len() => package.object_path(ObjectRef::Import(k)).to_ascii_lowercase(),
            ObjectRef::Export(k) if k < package.exports.len() => format!("{}.{}", package_name, package.object_path(ObjectRef::Export(k))).to_ascii_lowercase(),
            _ => format!("?{r}"),
        }
    };
    loop {
        // Positions (offset: / end: ) removed as before; first integer of the object tokens and of NameConst replaced.
        let pos = rest.find("offset: ").map(|x| (x, 8usize, 0u8)).into_iter().chain(rest.find("end: ").map(|y| (y, 5usize, 0u8))).min_by_key(|t| t.0);
        let name_value = OBJECTS.iter().filter_map(|t| rest.find(t).map(|x| (x, t.len(), 1u8))).chain(rest.find("NameConst(").map(|x| (x, 10usize, 2u8))).min_by_key(|t| t.0);
        let Some((at, skip, kind)) = [pos, name_value].into_iter().flatten().min_by_key(|t| t.0) else { break };
        out.push_str(&rest[..at + if kind == 0 { 0 } else { skip }]);
        rest = &rest[at + skip..];
        let end_value = rest.find(|c: char| !(c.is_ascii_digit() || c == '-')).unwrap_or(rest.len());
        match kind {
            0 => rest = rest[end_value..].trim_start_matches(|c: char| c.is_ascii_digit()),
            1 => {
                out.push_str(&object_value(rest[..end_value].parse().unwrap_or(0)));
                rest = &rest[end_value..];
            }
            _ => {
                let n: usize = rest[..end_value].parse().unwrap_or(usize::MAX);
                out.push_str(&package.names.get(n).map_or("?".to_string(), |x| x.name.to_ascii_lowercase()));
                rest = &rest[end_value..];
            }
        }
    }
    out.push_str(rest);
    Some(out)
}

/// appRound (fistp, round to even): outside i32, infinity or NaN give the undefined integer
/// 0x80000000.
fn app_round(f: f32) -> i32 {
    let r = f.round_ties_even();
    if (-2147483648.0..2147483648.0).contains(&r) {
        r as i32
    } else {
        i32::MIN
    }
}

impl Vm {
    /// Is the name (lowercase) in the table of a loaded package? The FNames of these packages already exist when
    /// UPackageMap::Compute sizes NameIndices.
    pub(crate) fn name_load(&self, key_tuple: &str) -> bool {
        let mut c = self.names_loaded.borrow_mut();
        if c.0 != self.packages.len() {
            c.1 = self.packages.iter().flat_map(|p| p.package.names.iter().map(|n| n.name.to_lowercase())).collect();
            // Core's hard-coded names (NAME_Default…): always in the FName table.
            c.1.extend(NAMES_CODES.iter().map(|n| n.to_lowercase()));
            c.0 = self.packages.len();
        }
        c.1.contains(key_tuple)
    }

    /// UClass::Link / UStruct::Link of a class, for the network.
    pub(crate) fn net_class(&mut self, c: ClassId) -> Rc<NetClass> {
        if let Some(n) = self.ipdrv.rep.classes.get(&c) {
            return n.clone();
        }
        let sup = self.classes[c].super_class.map(|s| self.net_class(s));
        let (pi, export) = (self.classes[c].package, self.classes[c].export);
        let super_len = self.classes[c].super_class.map_or(0, |s| self.classes[s].props.len());
        let (members, script, class_flags) = {
            let lp = &self.packages[pi];
            match objects::read_class(&lp.package, &lp.data, export) {
                Ok(k) => (children(&lp.package, &lp.data, k.state.body.header.children), k.state.body.script, k.class_flags),
                Err(_) => (Vec::new(), Vec::new(), 0),
            }
        };
        let code = (!script.is_empty()).then(|| Code::new(script, pi));
        let mut out = NetClass {
            reps: sup.as_ref().map(|s| s.reps.clone()).unwrap_or_default(),
            props: sup.as_ref().map(|s| s.props.clone()).unwrap_or_default(),
            code: code.clone(),
            native_rep: class_flags & CLASS_NATIVE_REPLICATION != 0,
            config_slots: sup.as_ref().map(|s| s.config_slots.clone()).unwrap_or_default(),
            ..Default::default()
        };
        // Own properties in Children order (that of ClassDef.props after the parent).
        let mut slot = super_len;
        let mut own_iter: Vec<(usize, String)> = Vec::new();
        let mut own_props: Vec<(usize, usize, u16, u32)> = Vec::new();
        let mut own_funcs: Vec<(usize, u16, u32)> = Vec::new();
        for m in members {
            let class_name = self.packages[pi].package.export_class(m).to_string();
            if class_name.ends_with("Property") {
                let Ok(pr) = objects::read_property(&self.packages[pi].package, &self.packages[pi].data, m) else { continue };
                if pr.flags & prop::CONFIG != 0 {
                    out.config_slots.push(slot);
                }
                if pr.flags & prop::NET != 0 {
                    let off = pr.rep_offset.unwrap_or(0);
                    let key = code.as_ref().and_then(|k| condition_key(k, off, &self.packages[pi].package, &self.packages[pi].name)).unwrap_or_else(|| format!("{c}:{slot}"));
                    own_iter.push((slot, key));
                    own_props.push((slot, m, off, pr.flags));
                }
                slot += 1;
            } else if class_name == "Function" {
                let lp = &self.packages[pi];
                if let Ok(f) = objects::read_function(&lp.package, &lp.data, m) {
                    if f.flags & func::NET != 0 && f.body.header.super_field == 0 {
                        own_funcs.push((m, f.rep_offset.unwrap_or(0), f.flags));
                    }
                }
            }
        }
        out.iter = own_iter.clone();
        if let Some(s) = &sup {
            out.iter.extend(s.iter.iter().cloned());
        }
        let mut fields: Vec<NetField> = Vec::new();
        for (k, &(slot, m, off, flags)) in own_props.iter().enumerate() {
            let key = &own_iter[k].1;
            let owner_slot = out.iter[k..].iter().rev().find(|(_, q)| q == key).map_or(slot, |(s, _)| *s);
            let owner_class = self.slot_owner(c, owner_slot);
            let def = self.classes[c].props[slot].clone();
            let ty = self.net_ty(&def.ty, def.enumeration.as_deref());
            let rep_index = out.reps.len();
            for e in 0..def.dim {
                out.reps.push((slot, e));
            }
            let p = Rc::new(NetProp { slot, name: def.name.clone(), owner: c, export: m, rep_offset: off, rep_owner: (owner_class, owner_slot), rep_index, dim: def.dim, ty, flags });
            out.props.insert(slot, p.clone());
            fields.push(NetField::Prop(p));
        }
        for (m, off, flags) in own_funcs {
            let name = self.packages[pi].package.name(self.packages[pi].package.exports[m].object_name).to_string();
            let Some(&fid) = self.classes[c].functions.get(&name.to_lowercase()) else { continue };
            let params = self.net_params(fid);
            fields.push(NetField::Func(Rc::new(NetFunc { func: fid, name, owner: c, export: m, rep_offset: off, flags, params })));
        }
        fields.sort_by_key(|f| f.owner_export().1);
        out.own = fields;
        let rc = Rc::new(out);
        self.ipdrv.rep.classes.insert(c, rc.clone());
        rc
    }

    /// Class that declares slot `slot` of the properties of `c`.
    pub(crate) fn slot_owner(&self, mut c: ClassId, slot: usize) -> ClassId {
        while let Some(s) = self.classes[c].super_class {
            if self.classes[s].props.len() <= slot {
                break;
            }
            c = s;
        }
        c
    }

    /// Parameters of a replicated function, with the enum of each byte.
    fn net_params(&mut self, f: FuncId) -> Vec<(usize, usize, NetTy, bool)> {
        let (pi, export) = (self.functions[f].package, self.functions[f].export);
        let kinds: Vec<(objects::PropertyKind, u32)> = {
            let lp = &self.packages[pi];
            let Ok(fun) = objects::read_function(&lp.package, &lp.data, export) else { return Vec::new() };
            children(&lp.package, &lp.data, fun.body.header.children)
                .into_iter()
                .filter(|&ch| lp.package.export_class(ch).ends_with("Property"))
                .filter_map(|ch| objects::read_property(&lp.package, &lp.data, ch).ok())
                .filter(|p| p.flags & (prop::PARM | prop::RETURN_PARM) == prop::PARM)
                .map(|p| (p.kind, p.flags))
                .collect()
        };
        let params = self.functions[f].params.clone();
        let mut out = Vec::new();
        for (p, (kind, _)) in params.iter().zip(kinds) {
            let def = self.functions[f].locals[p.local].clone();
            let enumeration = self.property_enum(pi, &kind);
            let ty = self.net_ty(&def.ty, enumeration.as_deref());
            out.push((p.local, def.dim, ty, def.ty == Type::Bool));
        }
        out
    }

    pub(crate) fn net_ty(&mut self, ty: &Type, enumeration: Option<&str>) -> NetTy {
        match ty {
            Type::Byte => NetTy::Byte(enumeration.and_then(|e| self.enum_names(e)).map_or(8, |n| ceil_log_two(n.len() as u32))),
            Type::Int => NetTy::Int,
            Type::Bool => NetTy::Bool,
            Type::Float => NetTy::Float,
            Type::Name => NetTy::Name,
            Type::Str => NetTy::Str,
            Type::Object(c) => NetTy::Object(c.clone()),
            // UClassProperty: PropertyClass is UClass (the metaclass is not checked by
            // SerializeObject, only IsA(UClass)).
            Type::Class(_) => NetTy::Object("Class".into()),
            Type::Delegate => NetTy::Delegate,
            Type::Struct(name) => match name.rsplit('.').next().unwrap_or(name).to_ascii_lowercase().as_str() {
                "vector" => NetTy::Vector,
                "rotator" => NetTy::Rotator,
                "plane" => NetTy::Plane,
                _ => {
                    let fields: Vec<(Type, usize, Option<String>)> = self.struct_def(name).map(|s| s.fields.iter().map(|f| (f.ty.clone(), f.dim, f.enumeration.clone())).collect()).unwrap_or_default();
                    let order = self.serial_order(name).unwrap_or_else(|| (0..fields.len()).collect());
                    let mut out = Vec::new();
                    for i in order {
                        if let Some((t, dim, e)) = fields.get(i).cloned() {
                            let nt = self.net_ty(&t, e.as_deref());
                            out.push((i, dim, nt));
                        }
                    }
                    NetTy::Struct(out)
                }
            },
            Type::Array(_) | Type::Unknown => NetTy::Skip,
        }
    }

    /// UPackageMap::GetClassNetCache for map `index`: None if the class is not in it.
    pub(crate) fn class_net_cache(&mut self, index: &MapIndex, cache: &mut HashMap<ClassId, Rc<ClassNetCache>>, c: ClassId) -> Option<Rc<ClassNetCache>> {
        if let Some(x) = cache.get(&c) {
            return Some(x.clone());
        }
        if self.export_net_index(index, self.classes[c].package, self.classes[c].export).is_none() {
            return None;
        }
        let sup = self.classes[c].super_class.and_then(|s| self.class_net_cache(index, cache, s));
        let net = self.net_class(c);
        let fields_base = sup.as_ref().map_or(0, |s| s.max_index());
        let mut count = sup.as_ref().map_or(0, |s| s.rep_condition_count);
        let mut fields: Vec<FieldCache> = Vec::new();
        for f in &net.own {
            let (owner, export) = f.owner_export();
            if self.export_net_index(index, self.classes[owner].package, export).is_none() {
                continue;
            }
            let net_index = fields_base + fields.len() as u32;
            let mut condition = -1;
            if let NetField::Prop(p) = f {
                let (oc, os) = p.rep_owner;
                let owner_export = self.net_class(oc).props.get(&os).map(|q| q.export);
                let owner_mapped = owner_export.is_some_and(|e| self.export_net_index(index, self.classes[oc].package, e).is_some());
                if (oc, os) == (p.owner, p.slot) || !owner_mapped {
                    condition = count;
                    count += 1;
                }
            }
            fields.push(FieldCache { field: f.clone(), net_index, condition });
        }
        let mut result = ClassNetCache { sup: sup.clone(), fields_base, fields, rep_condition_count: count, rep_properties: sup.as_ref().map(|s| s.rep_properties.clone()).unwrap_or_default() };
        for i in 0..result.fields.len() {
            let NetField::Prop(p) = result.fields[i].field.clone() else { continue };
            if result.fields[i].condition == -1 {
                let (oc, os) = p.rep_owner;
                let cond = result.from_prop(oc, os).map_or(-1, |e| e.condition);
                result.fields[i].condition = cond;
            }
            if !self.net_class(p.owner).native_rep {
                result.rep_properties.push(result.fields[i].clone());
            }
        }
        let rc = Rc::new(result);
        cache.insert(c, rc.clone());
        Some(rc)
    }

    /// UPackageMap::ObjectToIndex of an export of a VM package.
    pub(crate) fn export_net_index(&self, index: &MapIndex, pi: usize, export: usize) -> Option<u32> {
        let name = &self.packages.get(pi)?.name;
        let p = index.packages.iter().find(|p| p.name.eq_ignore_ascii_case(name))?;
        ((export as u32) < p.object_count).then(|| p.object_base + export as u32)
    }

    /// UPackageMap::ObjectToIndex of a class (the export of its package; a mod's new class: its
    /// export in the mods' package).
    pub(crate) fn class_net_index(&self, index: &MapIndex, class: crate::value::ClassId) -> Option<u32> {
        #[cfg(feature = "mods")]
        if let Some(p) = self.mod_class_net_path(class) {
            return index.by_path.get(&p.to_lowercase()).copied();
        }
        self.export_net_index(index, self.classes[class].package, self.classes[class].export)
    }

    /// Path "Package.Group.Name" of an object value, for ObjectToIndex.
    fn net_object_path(&self, v: &Value) -> Option<String> {
        // Mods build only: a mod's new class is in the mods' package, not its parent's (mod_net.rs).
        #[cfg(feature = "mods")]
        if let Value::Class(Some(c)) = v {
            if let Some(p) = self.mod_class_net_path(*c) {
                return Some(p);
            }
        }
        match v {
            Value::Class(Some(c)) => Some(format!("{}.{}", self.packages[self.classes[*c].package].name, self.classes[*c].name)),
            Value::Object(Some(o)) => self.objects.get(*o)?.as_ref().map(|x| if x.outer.is_empty() { x.name.clone() } else { format!("{}.{}", x.outer, x.name) }),
            Value::Asset(p) => Some(p.clone()),
            _ => None,
        }
    }

    /// UPackageMapLevel::CanSerializeObject: a dynamic actor is serializable if it has a channel.
    pub(crate) fn net_can_serialize(&self, channels: &HashMap<ObjId, (usize, bool)>, v: &Value) -> bool {
        match v {
            Value::Object(Some(o)) if self.objects.get(*o).is_some_and(|x| x.is_some()) && self.net_dynamic_actor(*o) => channels.contains_key(o),
            _ => true,
        }
    }

    /// Non-static actor (neither bStatic nor bNoDelete): designated by its channel.
    pub(crate) fn net_dynamic_actor(&self, o: ObjId) -> bool {
        self.is_a(o, "Actor") && !self.propr(o, "bStatic").as_bool() && !self.propr(o, "bNoDelete").as_bool()
    }

    /// UPackageMapLevel::SerializeObject when writing, ObjectToIndex. `channels`: channel (and acknowledged open) of each actor. True if the object
    /// is designated ("mapped"): a dynamic actor without a channel or with an unacknowledged open is
    /// not, an object outside the package map is (index 1023).
    pub(crate) fn net_write_object(&self, w: &mut BitWriter, index: &MapIndex, channels: &HashMap<ObjId, (usize, bool)>, v: &Value) -> bool {
        if let Value::Object(Some(o)) = v {
            if self.objects.get(*o).is_some_and(|x| x.is_some()) && self.net_dynamic_actor(*o) {
                w.write_bit(true);
                let (ch, acked) = channels.get(o).copied().unwrap_or((0, false));
                w.write_int(ch as u32, crate::net::MAX_CHANNELS as u32);
                return acked;
            }
        }
        let found = self.net_object_path(v).and_then(|p| index.by_path.get(&p.to_lowercase()).copied());
        match (v, found) {
            (_, Some(i)) => {
                w.write_bit(false);
                w.write_int(i, index.max_object);
            }
            (Value::Object(None) | Value::Class(None) | Value::Void, None) => {
                w.write_bit(true);
                w.write_int(0, crate::net::MAX_CHANNELS as u32);
            }
            _ => {
                w.write_bit(true);
                w.write_int(u32::MAX, crate::net::MAX_CHANNELS as u32);
            }
        }
        true
    }

    /// UPackageMapLevel::SerializeObject when reading: `channel_actor` gives
    /// the actor of an open actor channel (not closing); IndexToObject.
    pub(crate) fn net_read_object(&mut self, r: &mut BitReader, index: &MapIndex, channel_actor: &dyn Fn(usize) -> Option<ObjId>, expected: &str) -> Value {
        let by_channel = r.read_bit();
        // Class of the export read (an object not loaded by the VM, sound, texture…: Value::Asset).
        let mut asset_class = None;
        let v = if !by_channel {
            let i = r.read_int(index.max_object);
            if r.error {
                return Value::Object(None);
            }
            asset_class = index.locate(i).and_then(|(k, e)| index.packages[k].exports.get(e)).map(|(_, c)| c.clone());
            self.net_index_to_object(index, i)
        } else {
            let ch = r.read_int(crate::net::MAX_CHANNELS as u32);
            if ch == 0 || r.error || ch as usize >= crate::net::MAX_CHANNELS {
                return Value::Object(None);
            }
            Value::Object(channel_actor(ch as usize))
        };
        // IsA of the expected class, otherwise "Forged object".
        let ok = match &v {
            // Read by its index: the export's class (a script object, Function…, has no class in the VM).
            Value::Object(Some(_)) if asset_class.is_some() => self.net_class_is(index, asset_class.as_deref().unwrap(), expected),
            Value::Object(Some(o)) => expected.is_empty() || self.is_a(*o, expected),
            // A UClass object: IsA(Class, State, Struct, Field or Object) only (the metaclass of a UClassProperty is
            // not checked).
            Value::Class(Some(_)) => self.net_class_is(index, "Class", expected),
            // IsA of the export's class (Sound, Function…): its parents through the map's classes, then those of the VM.
            Value::Asset(_) => asset_class.as_deref().is_none_or(|c| self.net_class_is(index, c, expected)),
            _ => true,
        };
        if !ok {
            // UObject::GetFullName: "Class Path".
            let got = match &v {
                Value::Object(Some(o)) if asset_class.is_some() => format!("{} {}", asset_class.as_deref().unwrap(), self.net_object_path(&v).unwrap_or_else(|| self.path_name(*o))),
                Value::Object(Some(o)) => format!("{} {}", self.objects[*o].as_ref().map_or(String::new(), |x| self.classes[x.class].name.clone()), self.path_name(*o)),
                Value::Asset(p) => format!("{} {p}", asset_class.as_deref().unwrap_or("Object")),
                _ => format!("Class {}", self.net_object_path(&v).unwrap_or_default()),
            };
            let want = self.class(expected).map_or_else(
                // Intrinsic class without an export (Class, Function…): Core.
                || format!("{}.{expected}", index.class_package.get(&expected.to_lowercase()).map_or("Core", |p| p.as_str())),
                |c| format!("{}.{}", self.packages[self.classes[c].package].name, self.classes[c].name),
            );
            self.log.push(format!("Log: Forged object: got {got}, expecting Class {want}"));
            return Value::Object(None);
        }
        v
    }

    /// Class `class` (short name) is `expected` or derives from it: parents of the classes exported by the map (natives included),
    /// otherwise from the VM.
    fn net_class_is(&self, index: &MapIndex, class: &str, expected: &str) -> bool {
        // Every object is an Object (UObject::IsA).
        if expected.is_empty() || expected.eq_ignore_ascii_case("Object") {
            return true;
        }
        let mut n = class.to_string();
        for _ in 0..64 {
            if n.eq_ignore_ascii_case(expected) {
                return true;
            }
            let low_value = n.to_lowercase();
            // Core's intrinsic classes, without an export in Core.u.
            let intrinsic = match low_value.as_str() {
                "class" => Some("State"),
                "state" | "function" => Some("Struct"),
                "struct" | "enum" | "const" | "property" => Some("Field"),
                "field" | "package" | "textbuffer" => Some("Object"),
                x if x.ends_with("property") => Some("Property"),
                _ => None,
            };
            let natives = true;
            let native_parent = || {
                let found = crate::classes_natives_table::PARENTS_NATIVES.iter().find(|(c, _)| c.eq_ignore_ascii_case(&n));
                found.map(|(_, p)| p.to_string()).filter(|p| natives && !p.is_empty())
            };
            let parent_value = index.class_super.get(&low_value).cloned()
                .or_else(|| self.class(&n).and_then(|c| self.classes[c].super_class).map(|s| self.classes[s].name.clone()))
                .or_else(native_parent)
                .or_else(|| intrinsic.map(String::from));
            match parent_value {
                Some(m) => n = m,
                None => return false,
            }
        }
        false
    }

    /// UPackageMap::IndexToObject(Index, 1): the loaded object of the export.
    pub(crate) fn net_index_to_object(&mut self, index: &MapIndex, i: u32) -> Value {
        let Some((k, e)) = index.locate(i) else { return Value::Object(None) };
        // CreateExport refuses exports absent from the client/server context.
        if index.packages[k].export_flags.get(e).is_some_and(|flags| flags & 0x30000 == 0) {
            return Value::Object(None);
        }
        let Some((path, class)) = index.packages[k].exports.get(e).cloned() else { return Value::Object(None) };
        if class.eq_ignore_ascii_case("Class") {
            return Value::Class(self.class(&path));
        }
        let map = self.map_package.map(|p| self.packages[p].name.to_lowercase());
        if map.as_deref() == Some(&index.packages[k].name.to_lowercase()) {
            if self.ipdrv.rep.map_objects.is_none() {
                let m: HashMap<String, ObjId> = (0..self.objects.len())
                    .filter_map(|o| self.objects[o].as_ref().map(|x| (format!("{}.{}", x.outer, x.name).to_lowercase(), o)))
                    .collect();
                self.ipdrv.rep.map_objects = Some(m);
            }
            let found = self.ipdrv.rep.map_objects.as_ref().and_then(|m| m.get(&path.to_lowercase()).copied());
            return Value::Object(found.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())));
        }
        if self.script_exports.contains_key(&path.to_lowercase()) {
            return Value::Object(self.script_object(&path));
        }
        Value::Asset(path)
    }

    /// NetSerializeItem when writing; true if everything designated is ("Mapped").
    pub(crate) fn net_write_value(&self, w: &mut BitWriter, index: &MapIndex, channels: &HashMap<ObjId, (usize, bool)>, ty: &NetTy, v: &Value) -> bool {
        match ty {
            // UByteProperty::NetSerializeItem.
            NetTy::Byte(bits) => {
                w.write_bits(&[v.as_byte()], *bits as usize);
                true
            }
            // UIntProperty::NetSerializeItem.
            NetTy::Int => {
                w.write_bytes(&v.as_int().to_le_bytes());
                true
            }
            // UFloatProperty::NetSerializeItem.
            NetTy::Float => {
                w.write_bytes(&v.as_float().to_bits().to_le_bytes());
                true
            }
            // UBoolProperty::NetSerializeItem.
            NetTy::Bool => {
                w.write_bit(v.as_bool());
                true
            }
            // UProperty::NetSerializeItem → UPackageMap::SerializeName.
            NetTy::Name => {
                let n = v.as_name();
                let key_tuple = if n.is_empty() { "none".to_string() } else { n.to_lowercase() };
                // NameIndices[Name] is -1 for an FName already in the table but absent from the map's packages
                // (then writes 0xFFFFFFFF: SerializeInt only keeps its leading bits); a name created
                // after Compute goes beyond NameIndices and is MaxNameIndex.
                let i = match index.names.get(&key_tuple) {
                    Some(&i) => i,
                    None if self.name_load(&key_tuple) => u32::MAX,
                    None => index.max_name,
                };
                w.write_int(i, index.max_name + 1);
                i != index.max_name
            }
            // UProperty::NetSerializeItem: SerializeItem, operator<<(FString).
            NetTy::Str => {
                crate::net::write_fstring(w, &v.as_name());
                true
            }
            // UObjectProperty::NetSerializeItem.
            // (FOutBunch::operator<< for a UObject*, for an FName.)
            NetTy::Object(_) => self.net_write_object(w, index, channels, v),
            // UDelegateProperty::NetSerializeItem: object (FOutBunch::operator<<)
            // then name.
            NetTy::Delegate => {
                let (o, n) = match v {
                    Value::Delegate(o, n) => (Value::Object(*o), Value::Name(crate::value::NameStruct::from(n.clone()))),
                    _ => (Value::Object(None), Value::Name(crate::value::NameStruct::from(String::new()))),
                };
                self.net_write_object(w, index, channels, &o);
                self.net_write_value(w, index, channels, &NetTy::Name, &n);
                true
            }
            // UStructProperty::NetSerializeItem, Vector: rounded components, Bits =
            // clamp(appCeilLogTwo(max |c| + 1), 1, 20) − 1, then c + 2^(Bits+1) on 2^(Bits+2).
            // |c| through neg then signed max: |0x80000000| stays negative and does not count.
            NetTy::Vector => {
                let f = v.as_vector();
                let (x, y, z) = (app_round(f[0]), app_round(f[1]), app_round(f[2]));
                let m = x.wrapping_abs().max(y.wrapping_abs()).max(z.wrapping_abs());
                let bits = ceil_log_two(m.wrapping_add(1) as u32).clamp(1, 20) - 1;
                w.write_int(bits, 20);
                let bias = 1i32 << (bits + 1);
                let max = 1u32 << (bits + 2);
                for c in [x, y, z] {
                    w.write_int(c.wrapping_add(bias) as u32, max);
                }
                true
            }
            // Rotator: high byte of each component, preceded by a bit.
            NetTy::Rotator => {
                let r = v.as_rotator();
                for c in r {
                    let b = (c >> 8) as u8;
                    w.write_bit(b != 0);
                    if b != 0 {
                        w.write_bytes(&[b]);
                    }
                }
                true
            }
            // Plane: four rounded SWORDs.
            NetTy::Plane => {
                if let Value::Struct(_, f) = v {
                    for c in f.iter().take(4) {
                        w.write_bytes(&(app_round(c.as_float()) as i16).to_le_bytes());
                    }
                }
                true
            }
            // Other struct: each property known to the map, element by element.
            NetTy::Struct(fields) => {
                if let Some((_, vals)) = v.struct_view() {
                    for (i, dim, t) in fields {
                        let Some(fv) = vals.get(*i) else { continue };
                        match fv {
                            Value::Array(items) if *dim > 1 => {
                                for it in items {
                                    self.net_write_value(w, index, channels, t, it);
                                }
                            }
                            _ => {
                                self.net_write_value(w, index, channels, t, fv);
                            }
                        }
                    }
                }
                true
            }
            // UArrayProperty, UMapProperty, UFixedArrayProperty
            // nothing.
            NetTy::Skip => true,
        }
    }

    /// NetSerializeItem when reading: the received value (`old` gives the shape of a struct).
    pub(crate) fn net_read_value(&mut self, r: &mut BitReader, index: &MapIndex, channel_actor: &dyn Fn(usize) -> Option<ObjId>, ty: &NetTy, old: &Value) -> Value {
        match ty {
            NetTy::Byte(bits) => Value::Byte(r.read_bits(*bits as usize).first().copied().unwrap_or_else(|| old.as_byte())),
            NetTy::Int => {
                let b = r.read_bytes(4);
                Value::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            }
            NetTy::Float => {
                let b = r.read_bytes(4);
                Value::Float(f32::from_bits(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
            }
            NetTy::Bool => Value::Bool(r.read_bit()),
            NetTy::Name => {
                let i = r.read_int(index.max_name + 1);
                let n = if i < index.max_name && !r.error { index.name_at(i).unwrap_or("None").to_string() } else { "None".to_string() };
                Value::Name(crate::value::NameStruct::from(n))
            }
            NetTy::Str => Value::Str(crate::net::read_fstring(r)),
            // FInBunch::operator<<: UObject* and FName through the map.
            NetTy::Object(c) => self.net_read_object(r, index, channel_actor, c),
            NetTy::Delegate => {
                let o = self.net_read_object(r, index, channel_actor, "Object").as_object();
                let n = self.net_read_value(r, index, channel_actor, &NetTy::Name, &Value::Void).as_name();
                Value::Delegate(o, n.into())
            }
            NetTy::Vector => {
                let bits = r.read_int(20);
                let bias = 1i32 << (bits + 1);
                let max = 1u32 << (bits + 2);
                let mut c = [0f32; 3];
                for x in c.iter_mut() {
                    *x = (r.read_int(max) as i32).wrapping_sub(bias) as f32;
                }
                Value::vector(c[0], c[1], c[2])
            }
            NetTy::Rotator => {
                let mut c = [0i32; 3];
                for x in c.iter_mut() {
                    if r.read_bit() {
                        *x = (r.read_bytes(1)[0] as i32) << 8;
                    }
                }
                Value::rotator(c[0], c[1], c[2])
            }
            NetTy::Plane => {
                let mut f = Vec::new();
                for _ in 0..4 {
                    let b = r.read_bytes(2);
                    f.push(Value::Float(i16::from_le_bytes([b[0], b[1]]) as f32));
                }
                let name = match old {
                    Value::Struct(n, _) => n.clone(),
                    _ => "object.plane".into(),
                };
                Value::Struct(name, f.into())
            }
            NetTy::Struct(fields) => {
                let mut v = old.clone();
                if let Some((_, vals)) = v.struct_mut() {
                    for (i, dim, t) in fields {
                        let Some(cur) = vals.get(*i).cloned() else { continue };
                        let new = match cur {
                            Value::Array(items) if *dim > 1 => Value::Array(items.iter().map(|it| self.net_read_value(r, index, channel_actor, t, it)).collect()),
                            other => self.net_read_value(r, index, channel_actor, t, &other),
                        };
                        vals[*i] = new;
                    }
                }
                v.fold()
            }
            NetTy::Skip => old.clone(),
        }
    }
}

/// UProperty::Identical of a value: strings and names ignoring case, floats by equality.
pub fn net_identical(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        (Value::Name(x), Value::Name(y)) => {
            let none = |s: &str| s.is_empty() || s.eq_ignore_ascii_case("None");
            x.eq_ignore_ascii_case(y) || (none(x) && none(y))
        }
        (Value::Struct(_, x), Value::Struct(_, y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| net_identical(p, q)),
        // Inline vector / rotator: like the struct they are (name ignored).
        _ if a.is_struct() && b.is_struct() => match (a.struct_view(), b.struct_view()) {
            (Some((_, x)), Some((_, y))) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| net_identical(p, q)),
            _ => false,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| net_identical(p, q)),
        // UObjectProperty::Identical compares the pointers: None (NoObject, Value::Object(None)) passed to a class parameter is
        // the same null pointer as the null class (otherwise an extra "non-zero" bit and object are written).
        (Value::Object(None) | Value::Class(None), Value::Object(None) | Value::Class(None)) => true,
        _ => a == b,
    }
}

