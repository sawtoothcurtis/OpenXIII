//! Fields specific to script objects: Function, State, Class, properties, Enum, Const.

use crate::{read_struct, ScriptObject};
use openxiii_package::{Package, Reader, Result as PackageResult};

/// XIII function flags (24 bits, numbering specific to this UE2 version,
/// established by comparing compiled functions with the keywords in the sources).
pub mod func {
    pub const FINAL: u32 = 0x00_0001;
    pub const ITERATOR: u32 = 0x00_0002;
    pub const LATENT: u32 = 0x00_0004;
    pub const PRE_OPERATOR: u32 = 0x00_0008;
    /// Replicated function: a u16 RepOffset follows the flags.
    pub const NET: u32 = 0x00_0010;
    pub const NET_RELIABLE: u32 = 0x00_0020;
    pub const SIMULATED: u32 = 0x00_0040;
    pub const EXEC: u32 = 0x00_0080;
    pub const NATIVE: u32 = 0x00_0100;
    pub const EVENT: u32 = 0x00_0200;
    pub const OPERATOR: u32 = 0x00_0400;
    pub const STATIC: u32 = 0x00_0800;
    /// Absent from "private" functions even when they have a body (ClientShake):
    /// the VM does not use it to decide whether to run the bytecode.
    pub const DEFINED: u32 = 0x00_1000;
    pub const PRIVATE: u32 = 0x00_2000;
    pub const DELEGATE: u32 = 0x00_8000;
    pub const SINGULAR: u32 = 0x01_0000;
}

/// Property flags (standard UE2 numbering, checked on Parm/Edit/Net).
pub mod prop {
    pub const EDIT: u32 = 0x0000_0001;
    pub const CONST: u32 = 0x0000_0002;
    pub const INPUT: u32 = 0x0000_0004;
    pub const EXPORT_OBJECT: u32 = 0x0000_0008;
    pub const OPTIONAL_PARM: u32 = 0x0000_0010;
    pub const NET: u32 = 0x0000_0020;
    pub const CONST_REF: u32 = 0x0000_0040;
    pub const PARM: u32 = 0x0000_0080;
    pub const OUT_PARM: u32 = 0x0000_0100;
    pub const SKIP_PARM: u32 = 0x0000_0200;
    pub const RETURN_PARM: u32 = 0x0000_0400;
    pub const COERCE_PARM: u32 = 0x0000_0800;
    pub const NATIVE: u32 = 0x0000_1000;
    pub const TRANSIENT: u32 = 0x0000_2000;
    pub const CONFIG: u32 = 0x0000_4000;
    pub const LOCALIZED: u32 = 0x0000_8000;
    pub const TRAVEL: u32 = 0x0001_0000;
    pub const EDIT_CONST: u32 = 0x0002_0000;
    pub const GLOBAL_CONFIG: u32 = 0x0004_0000;
}

#[derive(Debug, Clone)]
pub struct Function {
    pub body: ScriptObject,
    pub native: u16,
    pub precedence: u8,
    pub flags: u32,
    pub rep_offset: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct State {
    pub body: ScriptObject,
    pub probe_mask: u64,
    pub ignore_mask: u64,
    pub label_table_offset: u16,
    pub flags: u16,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub class: i32,
    pub deep: u32,
    pub script_text_crc: u32,
}

#[derive(Debug, Clone)]
pub struct Class {
    pub state: State,
    pub class_flags: u32,
    pub guid: [u8; 16],
    pub dependencies: Vec<Dependency>,
    pub package_imports: Vec<i32>,
    pub within: i32,
    pub config_name: i32,
    pub hide_categories: Vec<i32>,
    /// File position of the start of the defaultproperties (tagged properties).
    pub defaults_offset: usize,
    pub defaults: Vec<openxiii_package::Property>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyKind {
    Byte { enumeration: i32 },
    Int,
    Bool,
    Float,
    Object { class: i32 },
    Class { class: i32, meta_class: i32 },
    Name,
    Str,
    Struct { structure: i32 },
    Array { inner: i32 },
    Map { key: i32, value: i32 },
    Delegate { function: i32 },
    FixedArray { inner: i32, count: i32 },
}

#[derive(Debug, Clone)]
pub struct Property {
    pub super_field: i32,
    pub next: i32,
    pub array_dim: u16,
    pub flags: u32,
    pub category: i32,
    pub rep_offset: Option<u16>,
    pub kind: PropertyKind,
}

#[derive(Debug)]
pub enum ObjectError {
    Script(crate::bytecode::ScriptError),
    Package(openxiii_package::Error),
    /// Unexpected bytes remaining at the end of the object.
    Trailing(usize),
    NotAProperty(String),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectError::Script(e) => write!(f, "{e}"),
            ObjectError::Package(e) => write!(f, "{e}"),
            ObjectError::Trailing(n) => write!(f, "{n} bytes not read at the end of the object"),
            ObjectError::NotAProperty(c) => write!(f, "class {c} not supported"),
        }
    }
}

impl std::error::Error for ObjectError {}

impl From<crate::bytecode::ScriptError> for ObjectError {
    fn from(e: crate::bytecode::ScriptError) -> Self {
        ObjectError::Script(e)
    }
}

impl From<openxiii_package::Error> for ObjectError {
    fn from(e: openxiii_package::Error) -> Self {
        ObjectError::Package(e)
    }
}

pub type Result<T> = std::result::Result<T, ObjectError>;

fn tail_reader(body: &ScriptObject) -> Reader<'_> {
    Reader::new(&body.tail, 0)
}

/// Without the development tools (release packages): never a mutant.
fn mutant(_name_value: &str) -> bool {
    false
}

fn finish(r: &Reader, len: usize) -> Result<()> {
    if r.pos == len {
        Ok(())
    } else {
        Err(ObjectError::Trailing(len - r.pos))
    }
}

fn u24(r: &mut Reader) -> PackageResult<u32> {
    Ok(r.u16()? as u32 | (r.u8()? as u32) << 16)
}

pub fn read_function(package: &Package, data: &[u8], index: usize) -> Result<Function> {
    let body = read_struct(package, data, index)?;
    let mut r = tail_reader(&body);
    let native = r.u16()?;
    let precedence = r.u8()?;
    let flags = u24(&mut r)?;
    let rep_offset = if flags & func::NET != 0 { Some(r.u16()?) } else { None };
    finish(&r, body.tail.len())?;
    Ok(Function { body, native, precedence, flags, rep_offset })
}

fn state_fields(r: &mut Reader) -> PackageResult<(u64, u64, u16, u16)> {
    let probe = r.u32()? as u64 | (r.u32()? as u64) << 32;
    let ignore = r.u32()? as u64 | (r.u32()? as u64) << 32;
    Ok((probe, ignore, r.u16()?, r.u16()?))
}

pub fn read_state(package: &Package, data: &[u8], index: usize) -> Result<State> {
    let body = read_struct(package, data, index)?;
    let mut r = tail_reader(&body);
    let (probe_mask, ignore_mask, label_table_offset, flags) = state_fields(&mut r)?;
    finish(&r, body.tail.len())?;
    Ok(State { body, probe_mask, ignore_mask, label_table_offset, flags })
}

pub fn read_class(package: &Package, data: &[u8], index: usize) -> Result<Class> {
    let body = read_struct(package, data, index)?;
    let tail_start = {
        let e = &package.exports[index];
        e.serial_offset as usize + e.serial_size as usize - body.tail.len()
    };
    let mut r = tail_reader(&body);
    let (probe_mask, ignore_mask, label_table_offset, flags) = state_fields(&mut r)?;
    // XIII writes ClassFlags on 16 bits (32 in standard UE2).
    let class_flags = r.u16()? as u32;
    let guid: [u8; 16] = r.take(16)?.try_into().unwrap();
    let dependencies =
        (0..r.compact()?).map(|_| Ok(Dependency { class: r.compact()?, deep: r.u32()?, script_text_crc: r.u32()? })).collect::<PackageResult<Vec<_>>>()?;
    let package_imports = (0..r.compact()?).map(|_| r.compact()).collect::<PackageResult<Vec<_>>>()?;
    let within = r.compact()?;
    let config_name = r.compact()?;
    let hide_categories = (0..r.compact()?).map(|_| r.compact()).collect::<PackageResult<Vec<_>>>()?;
    let defaults_offset = tail_start + r.pos;
    let end = tail_start + body.tail.len();
    let (defaults, defaults_end) = package.properties_at(data, defaults_offset, end)?;
    if defaults_end != end {
        return Err(ObjectError::Trailing(end - defaults_end));
    }
    let state = State { body, probe_mask, ignore_mask, label_table_offset, flags };
    Ok(Class { state, class_flags, guid, dependencies, package_imports, within, config_name, hide_categories, defaults_offset, defaults })
}

pub fn read_property(package: &Package, data: &[u8], index: usize) -> Result<Property> {
    let class = package.export_class(index).to_string();
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let start = package.object_data(data, index)?.end;
    let mut r = Reader::new(&data[..end], start);
    let super_field = r.compact()?;
    let next = r.compact()?;
    let array_dim = r.u16()?;
    let flags = r.u32()?;
    let category = r.compact()?;
    let rep_offset = if flags & prop::NET != 0 { Some(r.u16()?) } else { None };
    let kind = match class.as_str() {
        "ByteProperty" => PropertyKind::Byte { enumeration: r.compact()? },
        "IntProperty" => PropertyKind::Int,
        "BoolProperty" => PropertyKind::Bool,
        "FloatProperty" => PropertyKind::Float,
        "ObjectProperty" => PropertyKind::Object { class: r.compact()? },
        "ClassProperty" => PropertyKind::Class { class: r.compact()?, meta_class: r.compact()? },
        "NameProperty" => PropertyKind::Name,
        "StrProperty" => PropertyKind::Str,
        "StructProperty" => PropertyKind::Struct { structure: r.compact()? },
        "ArrayProperty" => PropertyKind::Array { inner: r.compact()? },
        "MapProperty" => PropertyKind::Map { key: r.compact()?, value: r.compact()? },
        "DelegateProperty" => PropertyKind::Delegate { function: r.compact()? },
        "FixedArrayProperty" => PropertyKind::FixedArray { inner: r.compact()?, count: r.i32()? },
        other => return Err(ObjectError::NotAProperty(other.to_string())),
    };
    finish(&r, end)?;
    Ok(Property { super_field, next, array_dim, flags, category, rep_offset, kind })
}

/// Enum: UField then an array of names.
pub fn read_enum(package: &Package, data: &[u8], index: usize) -> Result<Vec<i32>> {
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], package.object_data(data, index)?.end);
    r.compact()?;
    r.compact()?;
    #[allow(unused_mut)]
    let mut names = (0..r.compact()?).map(|_| r.compact()).collect::<PackageResult<Vec<_>>>()?;
    finish(&r, end)?;
    Ok(names)
}

/// Const: UField then a text value.
pub fn read_const(package: &Package, data: &[u8], index: usize) -> Result<String> {
    let export = &package.exports[index];
    let end = export.serial_offset as usize + export.serial_size as usize;
    let mut r = Reader::new(&data[..end], package.object_data(data, index)?.end);
    r.compact()?;
    r.compact()?;
    #[allow(unused_mut)]
    let mut value = r.string()?;
    finish(&r, end)?;
    Ok(value)
}
