//! Header reader for the Unreal Engine 2 packages used by XIII.
//!
//! XIII (PC, 2003) writes packages of version 100, licensee 58. The layout of the
//! header and of the name, import and export tables is standard UE2; this module
//! does not read the serialized object contents yet.

use std::fmt;

pub mod background;
pub mod env;
pub mod events;
pub mod loadings;
pub mod library;
mod properties;
pub use properties::*;

/// The game's `System` folder. Installs differ in its case (the Steam release names it `system`,
/// other copies `System`): on a case-sensitive file system (Linux, Android) the folder actually
/// present is taken, whatever its case. Without one, `game/System`.
pub fn system_dir(game: &std::path::Path) -> std::path::PathBuf {
    let exact = game.join("System");
    if exact.is_dir() {
        return exact;
    }
    std::fs::read_dir(game)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case("System")) && e.path().is_dir())
        .map_or(exact, |e| e.path())
}

/// The entry `name` of `dir`, whatever its case: `dir/name` when it exists, otherwise the first
/// entry of `dir` whose name matches ignoring ASCII case (installs differ: the GOG and Steam
/// releases name files `engine.u` and `core.u` where other copies have `Engine.u`), otherwise
/// `dir/name`. On a case-insensitive file system (Windows) this is always `dir/name`.
pub fn child_any_case(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let exact = dir.join(name);
    if exact.exists() {
        return exact;
    }
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map_or(exact, |e| e.path())
}

#[cfg(test)]
mod any_case_tests {
    use super::*;

    /// A lowercase copy of the game (`system/engine.u`, `maps/mapmenu.unr`) is found from the
    /// names the code uses; a missing entry gives the plain joined path, which does not exist.
    #[test]
    fn entries_found_whatever_their_case() {
        let base = std::env::temp_dir().join(format!("oxiii-any-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("system")).unwrap();
        std::fs::create_dir_all(base.join("maps")).unwrap();
        std::fs::write(base.join("system").join("engine.u"), b"").unwrap();
        std::fs::write(base.join("maps").join("mapmenu.unr"), b"").unwrap();
        let system = system_dir(&base);
        assert!(system.is_dir(), "system folder");
        assert!(child_any_case(&system, "Engine.u").is_file(), "Engine.u as engine.u");
        assert!(child_any_case(&child_any_case(&base, "Maps"), "MapMenu.unr").is_file(), "Maps/MapMenu.unr as maps/mapmenu.unr");
        assert!(child_any_case(&system, "engine.u").is_file(), "exact name");
        // Negative control: an entry that is not there is not invented.
        let missing = child_any_case(&system, "XIII.ini");
        assert_eq!(missing, system.join("XIII.ini"));
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}

pub const PACKAGE_TAG: u32 = 0x9E2A_83C1;

#[derive(Debug)]
pub enum Error {
    BadTag(u32),
    Eof { offset: usize, wanted: usize },
    BadName(usize),
    BadIndex(i32),
    BadProperty(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadTag(t) => write!(f, "invalid package signature 0x{t:08X}"),
            Error::Eof { offset, wanted } => {
                write!(f, "end of file at 0x{offset:X} ({wanted} bytes expected)")
            }
            Error::BadName(o) => write!(f, "invalid name at 0x{o:X}"),
            Error::BadIndex(i) => write!(f, "invalid object or name index {i}"),
            Error::BadProperty(o) => write!(f, "invalid property at 0x{o:X}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Little-endian read cursor over a buffer.
pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
}

/// FArchive::AtEnd: Tell() differs from INDEX_NONE and Tell() >= TotalSize().
pub fn archive_at_end(tell: i32, total: i32) -> bool {
    tell != -1 && tell >= total
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    /// FBufferReader::TotalSize: buffer size.
    pub fn total_size(&self) -> usize {
        self.data.len()
    }

    /// FBufferReader::AtEnd: the FArchive::AtEnd rule applied to the cursor and buffer size.
    pub fn at_end(&self) -> bool {
        archive_at_end(self.pos as i32, self.total_size() as i32)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or(Error::Eof { offset: self.pos, wanted: n })?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn guid(&mut self) -> Result<[u8; 16]> {
        Ok(self.take(16)?.try_into().unwrap())
    }

    /// Unreal compact integer (FCompactIndex): sign 0x80 and continuation 0x40 in the first byte (6 bits), then
    /// 7 bits per byte with 0x80 for continuation; the fifth byte is taken whole. As in the original, the shifts and sign change
    /// overflow on 32 bits (a compact index of i32::MIN stays i32::MIN), without panicking.
    pub fn compact(&mut self) -> Result<i32> {
        let b0 = self.u8()?;
        let mut v: i32 = 0;
        if b0 & 0x40 != 0 {
            let b1 = self.u8()?;
            if b1 & 0x80 != 0 {
                let b2 = self.u8()?;
                if b2 & 0x80 != 0 {
                    let b3 = self.u8()?;
                    if b3 & 0x80 != 0 {
                        v = self.u8()? as i32;
                    }
                    v = ((b3 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
                }
                v = ((b2 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
            }
            v = ((b1 & 0x7F) as i32).wrapping_add(v.wrapping_shl(7));
        }
        v = ((b0 & 0x3F) as i32).wrapping_add(v.wrapping_shl(6));
        Ok(if b0 & 0x80 != 0 { v.wrapping_neg() } else { v })
    }

    /// FString: compact length (terminator included), negative for UTF-16.
    pub fn string(&mut self) -> Result<String> {
        let start = self.pos;
        let len = self.compact()?;
        if len == 0 {
            return Ok(String::new());
        }
        let text = if len > 0 {
            let bytes = self.take(len as usize)?;
            bytes[..bytes.len() - 1].iter().map(|&b| b as char).collect()
        } else {
            let bytes = self.take(len.unsigned_abs() as usize * 2)?;
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units[..units.len() - 1])
        };
        if text.contains('\0') {
            return Err(Error::BadName(start));
        }
        Ok(text)
    }
}

#[derive(Debug, Clone)]
pub struct Generation {
    pub export_count: u32,
    pub name_count: u32,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: u16,
    pub licensee: u16,
    pub flags: u32,
    pub name_count: u32,
    pub name_offset: u32,
    pub export_count: u32,
    pub export_offset: u32,
    pub import_count: u32,
    pub import_offset: u32,
    pub guid: [u8; 16],
    pub generations: Vec<Generation>,
}

#[derive(Debug, Clone)]
pub struct Name {
    pub name: String,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub class_package: i32,
    pub class_name: i32,
    pub package: i32,
    pub object_name: i32,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub class: i32,
    pub super_: i32,
    pub package: i32,
    pub object_name: i32,
    pub flags: u32,
    pub serial_size: i32,
    pub serial_offset: i32,
}

/// Object reference: 0 = none, < 0 = import, > 0 = export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRef {
    None,
    Import(usize),
    Export(usize),
}

impl From<i32> for ObjectRef {
    fn from(i: i32) -> Self {
        match i {
            0 => ObjectRef::None,
            i if i < 0 => ObjectRef::Import((-i - 1) as usize),
            i => ObjectRef::Export((i - 1) as usize),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub header: Header,
    pub names: Vec<Name>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

impl Package {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data, 0);
        let tag = r.u32()?;
        if tag != PACKAGE_TAG {
            return Err(Error::BadTag(tag));
        }
        let version = r.u16()?;
        let licensee = r.u16()?;
        let flags = r.u32()?;
        let name_count = r.u32()?;
        let name_offset = r.u32()?;
        let export_count = r.u32()?;
        let export_offset = r.u32()?;
        let import_count = r.u32()?;
        let import_offset = r.u32()?;
        let guid = r.guid()?;
        let generation_count = r.u32()?;
        let generations = (0..generation_count.min(1024)).map(|_| Ok(Generation { export_count: r.u32()?, name_count: r.u32()? })).collect::<Result<_>>()?;
        let header = Header { version, licensee, flags, name_count, name_offset, export_count, export_offset, import_count, import_offset, guid, generations };

        let mut r = Reader::new(data, name_offset as usize);
        // ULinkerLoad constructor: a name with neither RF_LoadForClient nor RF_LoadForServer
        // (the game runs as client and server, not as editor) becomes NAME_None; only editor names
        // (u2viewport0, texturebrowser…) have just RF_LoadForEdit.
        let names = (0..name_count)
            .map(|_| {
                let (name, flags) = (r.string()?, r.u32()?);
                let name = if flags & 0x30000 == 0 { "None".to_string() } else { name };
                Ok(Name { name, flags })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut r = Reader::new(data, import_offset as usize);
        let imports = (0..import_count)
            .map(|_| Ok(Import { class_package: r.compact()?, class_name: r.compact()?, package: r.i32()?, object_name: r.compact()? }))
            .collect::<Result<Vec<_>>>()?;

        let mut r = Reader::new(data, export_offset as usize);
        let exports = (0..export_count)
            .map(|_| {
                let class = r.compact()?;
                let super_ = r.compact()?;
                let package = r.i32()?;
                let object_name = r.compact()?;
                let flags = r.u32()?;
                let serial_size = r.compact()?;
                let serial_offset = if serial_size > 0 { r.compact()? } else { 0 };
                Ok(Export { class, super_, package, object_name, flags, serial_size, serial_offset })
            })
            .collect::<Result<Vec<_>>>()?;

        let package = Package { header, names, imports, exports };
        package.validate(data.len())?;
        Ok(package)
    }

    fn validate(&self, len: usize) -> Result<()> {
        let names = self.names.len() as i32;
        let check_name = |i: i32| {
            if (0..names).contains(&i) {
                Ok(())
            } else {
                Err(Error::BadIndex(i))
            }
        };
        let check_ref = |i: i32| {
            let ok = match ObjectRef::from(i) {
                ObjectRef::None => true,
                ObjectRef::Import(n) => n < self.imports.len(),
                ObjectRef::Export(n) => n < self.exports.len(),
            };
            if ok {
                Ok(())
            } else {
                Err(Error::BadIndex(i))
            }
        };
        for i in &self.imports {
            check_name(i.class_package)?;
            check_name(i.class_name)?;
            check_name(i.object_name)?;
            check_ref(i.package)?;
        }
        for e in &self.exports {
            check_name(e.object_name)?;
            check_ref(e.class)?;
            check_ref(e.super_)?;
            check_ref(e.package)?;
            let end = e.serial_offset as i64 + e.serial_size as i64;
            if e.serial_size < 0 || e.serial_offset < 0 || end > len as i64 {
                return Err(Error::Eof { offset: e.serial_offset.max(0) as usize, wanted: e.serial_size.max(0) as usize });
            }
        }
        self.check_outers()
    }

    /// Following the outers (`package` of an import or export) always ends at a top-level object:
    /// a damaged file whose outers loop would otherwise hang every path lookup.
    fn check_outers(&self) -> Result<()> {
        let (ni, ne) = (self.imports.len(), self.exports.len());
        // Imports first, then exports, in one index space.
        let outer = |k: usize| if k < ni { self.imports[k].package } else { self.exports[k - ni].package };
        let slot = |r: i32| match ObjectRef::from(r) {
            ObjectRef::None => None,
            ObjectRef::Import(n) => Some(n),
            ObjectRef::Export(n) => Some(ni + n),
        };
        // 0: not seen, 1: on the chain being followed, 2: ends at a top-level object.
        let mut state = vec![0u8; ni + ne];
        for start in 0..ni + ne {
            let mut chain = Vec::new();
            let mut k = start;
            loop {
                match state[k] {
                    2 => break,
                    1 => return Err(Error::BadIndex(outer(k))),
                    _ => {}
                }
                state[k] = 1;
                chain.push(k);
                match slot(outer(k)) {
                    Some(next) => k = next,
                    None => break,
                }
            }
            for c in chain {
                state[c] = 2;
            }
        }
        Ok(())
    }

    pub fn name(&self, index: i32) -> &str {
        self.names.get(index as usize).map_or("<?>", |n| n.name.as_str())
    }

    /// Plain name of a referenced object; "Class" for native classes (null reference).
    pub fn object_name(&self, r: ObjectRef) -> &str {
        match r {
            ObjectRef::None => "Class",
            ObjectRef::Import(i) => self.name(self.imports[i].object_name),
            ObjectRef::Export(i) => self.name(self.exports[i].object_name),
        }
    }

    /// Full path "Package.Group.Object" of an import or export.
    pub fn object_path(&self, r: ObjectRef) -> String {
        let mut parts = Vec::new();
        let mut current = r;
        while current != ObjectRef::None && parts.len() < 64 {
            parts.push(self.object_name(current).to_string());
            current = match current {
                ObjectRef::Import(i) => self.imports[i].package.into(),
                ObjectRef::Export(i) => self.exports[i].package.into(),
                ObjectRef::None => ObjectRef::None,
            };
        }
        parts.reverse();
        parts.join(".")
    }

    pub fn import_class(&self, i: usize) -> String {
        let imp = &self.imports[i];
        format!("{}.{}", self.name(imp.class_package), self.name(imp.class_name))
    }

    pub fn export_class(&self, i: usize) -> &str {
        self.object_name(self.exports[i].class.into())
    }

    /// ULinkerLoad::GetExportClassPackage: package of the class of export `i`: the Outer name of an
    /// imported class, `own` (the package itself) for a class of this package, "Core" for a null (intrinsic) class.
    pub fn export_class_package<'a>(&'a self, i: usize, own: &'a str) -> &'a str {
        match ObjectRef::from(self.exports[i].class) {
            ObjectRef::Import(c) => self.object_name(self.imports[c].package.into()),
            ObjectRef::Export(_) => own,
            ObjectRef::None => "Core",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outers that loop (import 0 inside import 1 inside import 0) are refused; a chain that ends passes.
    #[test]
    fn looping_outers_refused() {
        let header = Header { version: 100, licensee: 58, flags: 0, name_count: 1, name_offset: 0, export_count: 0, export_offset: 0, import_count: 2, import_offset: 0, guid: [0; 16], generations: Vec::new() };
        let import = |package: i32| Import { class_package: 0, class_name: 0, package, object_name: 0 };
        let names = vec![Name { name: "None".into(), flags: 0x30000 }];
        let chained = Package { header: header.clone(), names: names.clone(), imports: vec![import(0), import(-1)], exports: Vec::new() };
        assert!(chained.validate(0).is_ok());
        let looping = Package { header, names, imports: vec![import(-2), import(-1)], exports: Vec::new() };
        assert!(looping.validate(0).is_err());
    }

    #[test]
    fn compact_index() {
        // The last three: fifth byte taken whole, then i32::MIN and its overflowing negation.
        let cases: &[(&[u8], i32)] = &[(&[0x00], 0), (&[0x3F], 63), (&[0x81], -1), (&[0x40, 0x01], 64), (&[0x7F, 0x7F], 8191), (&[0x40, 0x80, 0x01], 8192),
            (&[0x40, 0x80, 0x80, 0x80, 0xFF], (0xFFu32 << 27) as i32), (&[0x40, 0x80, 0x80, 0x80, 0x10], i32::MIN), (&[0xC0, 0x80, 0x80, 0x80, 0x10], i32::MIN)];
        for (bytes, expected) in cases {
            assert_eq!(Reader::new(bytes, 0).compact().unwrap(), *expected, "{bytes:?}");
        }
    }

    #[test]
    fn object_refs() {
        assert_eq!(ObjectRef::from(0), ObjectRef::None);
        assert_eq!(ObjectRef::from(-1), ObjectRef::Import(0));
        assert_eq!(ObjectRef::from(3), ObjectRef::Export(2));
    }
}

