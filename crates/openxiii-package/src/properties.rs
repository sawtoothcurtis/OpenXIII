//! Tagged properties serialized at the start of each object.
//!
//! UE2 format: name (compact index), info byte (type on 4 bits, size code
//! on 3 bits, bit 7 = array or boolean value), optional struct name,
//! size, array index, then the value. The list ends with the name "None".
//! XIII quirk: booleans carry a size code of 5 followed by a 0 byte.

use crate::{Error, Package, Reader, Result};

pub const RF_HAS_STACK: u32 = 0x0200_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyType {
    Byte = 1,
    Int = 2,
    Bool = 3,
    Float = 4,
    Object = 5,
    Name = 6,
    String = 7,
    Class = 8,
    Array = 9,
    Struct = 10,
    Vector = 11,
    Rotator = 12,
    Str = 13,
    Map = 14,
    FixedArray = 15,
}

impl PropertyType {
    fn from_info(info: u8) -> Option<Self> {
        use PropertyType::*;
        Some(match info & 0x0F {
            1 => Byte,
            2 => Int,
            3 => Bool,
            4 => Float,
            5 => Object,
            6 => Name,
            7 => String,
            8 => Class,
            9 => Array,
            10 => Struct,
            11 => Vector,
            12 => Rotator,
            13 => Str,
            14 => Map,
            15 => FixedArray,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Object(i32),
    Name(i32),
    Str(String),
    Vector([f32; 3]),
    Rotator([i32; 3]),
    /// Struct, dynamic array or undecoded type: raw bytes.
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: i32,
    pub kind: PropertyType,
    pub struct_name: Option<i32>,
    pub array_index: u32,
    pub value: Value,
}

/// State frame of a scripted actor or object (RF_HasStack flag).
#[derive(Debug, Clone)]
pub struct StateFrame {
    pub node: i32,
    pub state_node: i32,
    pub probe_mask: u64,
    pub latent_action: u32,
    pub offset: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ObjectData {
    pub state: Option<StateFrame>,
    pub properties: Vec<Property>,
    /// Absolute file position of the end of the properties.
    pub end: usize,
}

/// UObject::Serialize, RF_HasStack object: Node, StateNode (object references), ProbeMask (8 bytes), LatentAction
/// (4 bytes), then Offset (compact index) when Node is not null.
pub fn read_state_frame(r: &mut Reader) -> Result<StateFrame> {
    let node = r.compact()?;
    let state_node = r.compact()?;
    let probe_mask = r.u32()? as u64 | (r.u32()? as u64) << 32;
    let latent_action = r.u32()?;
    let offset = if node != 0 { Some(r.compact()?) } else { None };
    Ok(StateFrame { node, state_node, probe_mask, latent_action, offset })
}

impl Package {
    /// Reads the state frame and properties of export `index` (0-based).
    /// `Class` objects are not handled (their serialization differs).
    pub fn object_data(&self, data: &[u8], index: usize) -> Result<ObjectData> {
        let export = &self.exports[index];
        let start = export.serial_offset as usize;
        let end = start + export.serial_size as usize;
        let mut r = Reader::new(&data[..end], start);
        let state = if export.flags & RF_HAS_STACK != 0 {
            Some(read_state_frame(&mut r)?)
        } else {
            None
        };
        let properties = self.read_properties(&mut r)?;
        Ok(ObjectData { state, properties, end: r.pos })
    }

    /// Reads a list of tagged properties between `start` and `end` (e.g. a class's
    /// defaultproperties); returns the properties and the end position.
    pub fn properties_at(&self, data: &[u8], start: usize, end: usize) -> Result<(Vec<Property>, usize)> {
        let mut r = Reader::new(&data[..end], start);
        let properties = self.read_properties(&mut r)?;
        Ok((properties, r.pos))
    }

    fn read_properties(&self, r: &mut Reader) -> Result<Vec<Property>> {
        let mut properties = Vec::new();
        loop {
            let name = r.compact()?;
            if !(0..self.names.len() as i32).contains(&name) {
                return Err(Error::BadIndex(name));
            }
            if self.name(name).eq_ignore_ascii_case("None") {
                return Ok(properties);
            }
            let info = r.u8()?;
            let kind = PropertyType::from_info(info).ok_or(Error::BadProperty(r.pos - 1))?;
            let struct_name = if kind == PropertyType::Struct { Some(r.compact()?) } else { None };
            let size = match (info >> 4) & 7 {
                0 => 1,
                1 => 2,
                2 => 4,
                3 => 12,
                4 => 16,
                5 => r.u8()? as usize,
                6 => r.u16()? as usize,
                _ => r.u32()? as usize,
            };
            let flag = info & 0x80 != 0;
            let array_index = if flag && kind != PropertyType::Bool { array_index(r)? } else { 0 };
            let value_start = r.pos;
            let mut v = Reader::new(r.data, value_start);
            let raw = |r: &mut Reader| -> Result<Value> { Ok(Value::Raw(r.take(size)?.to_vec())) };
            let value = match kind {
                PropertyType::Bool => Value::Bool(flag),
                PropertyType::Byte if size == 1 => Value::Byte(v.u8()?),
                PropertyType::Int if size == 4 => Value::Int(v.i32()?),
                PropertyType::Float if size == 4 => Value::Float(f32::from_bits(v.u32()?)),
                PropertyType::Object | PropertyType::Class => Value::Object(v.compact()?),
                PropertyType::Name => Value::Name(v.compact()?),
                PropertyType::Str | PropertyType::String => Value::Str(v.string()?),
                PropertyType::Struct if size == 12 && self.struct_is(struct_name, "Vector") => {
                    Value::Vector([0; 3].map(|_| f32::from_bits(v.u32().unwrap_or(0))))
                }
                PropertyType::Struct if size == 12 && self.struct_is(struct_name, "Rotator") => Value::Rotator([0; 3].map(|_| v.i32().unwrap_or(0))),
                _ => raw(&mut v)?,
            };
            if !matches!(value, Value::Raw(_)) && v.pos > value_start + size {
                return Err(Error::BadProperty(value_start));
            }
            r.take(size)?;
            properties.push(Property { name, kind, struct_name, array_index, value });
        }
    }

    fn struct_is(&self, name: Option<i32>, expected: &str) -> bool {
        name.is_some_and(|n| self.name(n).eq_ignore_ascii_case(expected))
    }

    /// Readable representation of a value, with names and objects resolved.
    pub fn display_value(&self, value: &Value) -> String {
        match value {
            Value::Byte(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Object(0) => "None".into(),
            Value::Object(o) => self.object_path((*o).into()),
            Value::Name(n) => format!("'{}'", self.name(*n)),
            Value::Str(s) => format!("{s:?}"),
            Value::Vector([x, y, z]) => format!("(X={x},Y={y},Z={z})"),
            Value::Rotator([p, y, r]) => format!("(Pitch={p},Yaw={y},Roll={r})"),
            Value::Raw(bytes) => {
                let hex: String = bytes.iter().take(32).map(|b| format!("{b:02x}")).collect();
                let more = if bytes.len() > 32 { "…" } else { "" };
                format!("[{} bytes] {hex}{more}", bytes.len())
            }
        }
    }
}

fn array_index(r: &mut Reader) -> Result<u32> {
    let b0 = r.u8()? as u32;
    Ok(if b0 & 0x80 == 0 {
        b0
    } else if b0 & 0xC0 == 0x80 {
        ((b0 & 0x7F) << 8) | r.u8()? as u32
    } else {
        ((b0 & 0x3F) << 24) | (r.u8()? as u32) << 16 | (r.u8()? as u32) << 8 | r.u8()? as u32
    })
}

impl ObjectData {
    /// First property with this name (array index 0).
    pub fn get<'a>(&'a self, package: &Package, name: &str) -> Option<&'a Value> {
        self.properties.iter().find(|p| p.array_index == 0 && package.name(p.name).eq_ignore_ascii_case(name)).map(|p| &p.value)
    }
}
