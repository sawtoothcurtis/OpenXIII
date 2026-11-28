//! XIII's UnrealScript objects: Function, State, Class and their bytecode.
//!
//! UStruct header (after the tagged properties, except for Class objects which have none):
//! SuperField, Next, ScriptText, Children (compact), FriendlyName (compact name),
//! Line, TextPos, ScriptSize (i32), then the bytecode.

pub mod bytecode;
pub mod decompile;
pub mod objects;

use bytecode::{Expr, Result};
use openxiii_package::{Package, Reader};

#[derive(Debug, Clone)]
pub struct StructHeader {
    pub super_field: i32,
    pub next: i32,
    pub script_text: i32,
    pub children: i32,
    pub friendly_name: i32,
    pub line: i32,
    pub text_pos: i32,
    pub script_size: u32,
}

#[derive(Debug, Clone)]
pub struct ScriptObject {
    pub header: StructHeader,
    pub script: Vec<Expr>,
    /// Bytes remaining after the bytecode (fields specific to Function/State/Class).
    pub tail: Vec<u8>,
}

/// Indices of the package's "None" names (a XIII package may contain several).
pub fn none_indices(package: &Package) -> Vec<i32> {
    (0..package.names.len() as i32).filter(|&i| package.name(i).eq_ignore_ascii_case("None")).collect()
}

/// Reads the UStruct header and bytecode of a Function, State or Class export.
pub fn read_struct(package: &Package, data: &[u8], index: usize) -> Result<ScriptObject> {
    let (object, error) = read_struct_partial(package, data, index)?;
    match error {
        Some(e) => Err(e),
        None => Ok(object),
    }
}

/// Lenient read: returns whatever could be decoded and the bytecode error, if any.
pub fn read_struct_partial(package: &Package, data: &[u8], index: usize) -> Result<(ScriptObject, Option<bytecode::ScriptError>)> {
    let export = &package.exports[index];
    let start = export.serial_offset as usize;
    let end = start + export.serial_size as usize;
    let begin = if package.export_class(index) == "Class" {
        // Class objects have no tagged properties before the header.
        start
    } else {
        package.object_data(data, index)?.end
    };
    let mut r = Reader::new(&data[..end], begin);
    let header = StructHeader {
        super_field: r.compact()?,
        next: r.compact()?,
        script_text: r.compact()?,
        children: r.compact()?,
        friendly_name: r.compact()?,
        line: r.i32()?,
        text_pos: r.i32()?,
        script_size: r.u32()?,
    };
    let (script, error) = bytecode::read_script_partial(&mut r, header.script_size, &none_indices(package));
    let tail = data[r.pos..end].to_vec();
    Ok((ScriptObject { header, script, tail }, error))
}
