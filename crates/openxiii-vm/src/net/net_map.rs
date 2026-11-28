//! Network package map (Core UPackageMap, UnCoreNet.cpp): list of the packages shared
//! with a client, base of their objects and names for replication.
//!
//! UGameEngine::BuildServerMasterMap: the level's package, then each of the
//! ServerPackages ("Server Package: %s"), then that of the GameInfo's class, each through
//! UPackageMap::AddLinker (ignored if PKG_ServerSideOnly, otherwise added then its
//! imported packages, in import order), and finally Compute (cumulative ObjectBase and
//! NameBase, counts limited to the generation known on the other side).

use openxiii_package::Package;

/// PKG_AllowDownload, PKG_ServerSideOnly.
pub const PKG_ALLOW_DOWNLOAD: u32 = 1;
pub const PKG_SERVER_SIDE_ONLY: u32 = 4;

/// FPackageInfo; Clone = copy constructor.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// Package name (LinkerRoot) and file for FNAME= (only if it can be downloaded).
    pub name: String,
    pub url: String,
    pub guid: [u8; 16],
    pub file_size: i32,
    pub object_base: i32,
    pub object_count: i32,
    pub name_index: i32,
    pub name_count: i32,
    pub local_generation: i32,
    pub remote_generation: i32,
    pub package_flags: u32,
    /// Generations of the summary (ExportCount, NameCount) and full counts.
    pub generations: Vec<(i32, i32)>,
    pub export_count: i32,
    pub names_total: i32,
}

/// FGuid::String: four DWORDs in uppercase hexadecimal.
pub fn guid_string(g: &[u8; 16]) -> String {
    (0..4).map(|i| format!("{:08X}", u32::from_le_bytes([g[i * 4], g[i * 4 + 1], g[i * 4 + 2], g[i * 4 + 3]]))).collect()
}

pub fn parse_guid(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..4 {
        // get: the text comes from the network, a non-ASCII character must not cut a slice in the middle.
        let v = u32::from_str_radix(s.get(i * 8..i * 8 + 8)?, 16).ok()?;
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    Some(out)
}

impl PackageInfo {
    /// FPackageInfo::FPackageInfo(Linker).
    pub fn from_package(name: &str, file: &str, package: &Package, file_size: usize) -> Self {
        let flags = package.header.flags;
        PackageInfo {
            name: name.to_string(),
            url: if flags & PKG_ALLOW_DOWNLOAD != 0 { file.to_string() } else { String::new() },
            guid: package.header.guid,
            file_size: file_size as i32,
            object_base: -1,
            object_count: -1,
            name_index: -1,
            name_count: -1,
            local_generation: 0,
            remote_generation: 0,
            package_flags: flags,
            generations: package.header.generations.iter().map(|g| (g.export_count as i32, g.name_count as i32)).collect(),
            export_count: package.exports.len() as i32,
            names_total: package.names.len() as i32,
        }
    }

    /// File name without path (SendPackageMap removes "\", ":", "/").
    pub fn file_name(&self) -> String {
        self.url.rsplit(['\\', ':', '/']).next().unwrap_or("").to_string()
    }

    /// "USES" line (UNetConnection::SendPackageMap).
    pub fn uses_line(&self) -> String {
        format!(
            "USES GUID={} PKG={} FLAGS={} SIZE={} GEN={} FNAME={}",
            guid_string(&self.guid),
            self.name,
            self.package_flags,
            self.file_size,
            self.local_generation,
            self.file_name()
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct PackageMap {
    pub list: Vec<PackageInfo>,
    pub max_object_index: i32,
    pub max_name_index: i32,
}

impl PackageMap {
    /// UPackageMap::AddLinker (continued: imported packages already loaded):
    /// `load` gives a loaded package (name, file, package, size).
    pub fn add_linker(&mut self, name: &str, load: &mut dyn FnMut(&str) -> Option<(String, String, Package, usize)>) -> i32 {
        if let Some(i) = self.list.iter().position(|p| p.name.eq_ignore_ascii_case(name)) {
            return i as i32;
        }
        let Some((pkg_name, file, package, size)) = load(name) else { return -1 };
        if package.header.flags & PKG_SERVER_SIDE_ONLY != 0 {
            return -1;
        }
        let index = self.list.len();
        self.list.push(PackageInfo::from_package(&pkg_name, &file, &package, size));
        for imp in &package.imports {
            let class = package.names.get(imp.class_name as usize).map(|n| n.name.as_str()).unwrap_or("");
            if class.eq_ignore_ascii_case("Package") && imp.package == 0 {
                let dep = package.names.get(imp.object_name as usize).map(|n| n.name.clone()).unwrap_or_default();
                self.add_linker(&dep, load);
            }
        }
        index as i32
    }

    /// UPackageMap::Compute: cumulative bases; generations limited to that of
    /// the other side.
    pub fn compute(&mut self) {
        let (mut objects, mut names) = (0, 0);
        for p in self.list.iter_mut() {
            p.object_base = objects;
            p.name_index = names;
            p.object_count = p.export_count;
            p.name_count = p.names_total;
            p.local_generation = p.generations.len() as i32;
            if p.remote_generation == 0 {
                p.remote_generation = p.local_generation;
            }
            if p.remote_generation < p.local_generation {
                if let Some(&(e, n)) = p.generations.get((p.remote_generation - 1).max(0) as usize) {
                    p.object_count = p.object_count.min(e);
                    p.name_count = p.name_count.min(n);
                }
            }
            objects += p.object_count;
            names += p.name_count;
        }
        self.max_object_index = objects;
        self.max_name_index = names;
    }

    /// UPackageMap::ObjectToIndex: network index of an export (package, export
    /// index).
    pub fn object_to_index(&self, package: &str, export: usize) -> i32 {
        match self.list.iter().find(|p| p.name.eq_ignore_ascii_case(package)) {
            Some(p) if (export as i32) < p.object_count => p.object_base + export as i32,
            _ => -1,
        }
    }

    /// UPackageMap::IndexToObject (continued): (package, export index) of a
    /// network index.
    pub fn index_to_object(&self, mut index: i32) -> Option<(String, usize)> {
        if index < 0 {
            return None;
        }
        for p in &self.list {
            if index < p.object_count {
                return Some((p.name.clone(), index as usize));
            }
            index -= p.object_count;
        }
        None
    }
}

/// What replication looks up in a computed package map: path and class of each
/// counted export (ObjectToIndex / IndexToObject) and the name table (UPackageMap::NameIndices:
/// first network index of each FName, case-insensitive like FNames).
#[derive(Debug, Default)]
pub struct MapIndex {
    pub packages: Vec<MapPackage>,
    pub by_path: std::collections::HashMap<String, u32>,
    pub names: std::collections::HashMap<String, u32>,
    pub max_object: u32,
    pub max_name: u32,
    /// Parent class of each class exported by the map's packages (natives included: Sound, Function…), lowercase.
    pub class_super: std::collections::HashMap<String, String>,
    /// Package of each class exported by the map (short name, lowercase).
    pub class_package: std::collections::HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct MapPackage {
    pub name: String,
    pub object_base: u32,
    pub object_count: u32,
    pub name_base: u32,
    pub name_count: u32,
    /// "Package.Group.Name" and class of each counted export.
    pub exports: Vec<(String, String)>,
    /// Flags of the exports, including RF_LoadForClient/Server read back by CreateExport.
    pub export_flags: Vec<u32>,
    pub names: Vec<String>,
}

/// NameIndices of UPackageMap::Compute (loop): each of the package's first NameCount names
/// takes NameIndex + rank, unless it already has one (the first package in the list wins).
pub fn assign_names<'a>(names: &mut std::collections::HashMap<String, u32>, base: u32, count: u32, package_names: impl Iterator<Item = &'a str>) {
    for (j, n) in package_names.take(count as usize).enumerate() {
        names.entry(n.to_lowercase()).or_insert(base + j as u32);
    }
}

impl MapIndex {
    /// After UPackageMap::Compute: `load` gives the loaded package of that name.
    pub fn build(map: &PackageMap, load: &mut dyn FnMut(&str) -> Option<Package>) -> MapIndex {
        let mut out = MapIndex { max_object: map.max_object_index.max(0) as u32, max_name: map.max_name_index.max(0) as u32, ..Default::default() };
        for info in &map.list {
            let mut mp = MapPackage {
                name: info.name.clone(),
                object_base: info.object_base.max(0) as u32,
                object_count: info.object_count.max(0) as u32,
                name_base: info.name_index.max(0) as u32,
                name_count: info.name_count.max(0) as u32,
                ..Default::default()
            };
            if let Some(p) = load(&info.name) {
                for i in 0..(mp.object_count as usize).min(p.exports.len()) {
                    let path = format!("{}.{}", info.name, p.object_path(openxiii_package::ObjectRef::Export(i)));
                    out.by_path.entry(path.to_lowercase()).or_insert(mp.object_base + i as u32);
                    mp.exports.push((path, p.export_class(i).to_string()));
                    if p.export_class(i).eq_ignore_ascii_case("Class") {
                        let name_value = p.object_name(openxiii_package::ObjectRef::Export(i)).to_lowercase();
                        out.class_package.entry(name_value.clone()).or_insert_with(|| info.name.clone());
                        if p.exports[i].super_ != 0 {
                            out.class_super.entry(name_value).or_insert_with(|| p.object_name(p.exports[i].super_.into()).to_string());
                        }
                    }
                    mp.export_flags.push(p.exports[i].flags);
                }
                assign_names(&mut out.names, mp.name_base, mp.name_count, p.names.iter().map(|n| n.name.as_str()));
                mp.names.extend(p.names.iter().take(mp.name_count as usize).map(|n| n.name.clone()));
            }
            out.packages.push(mp);
        }
        out
    }

    /// UPackageMap::IndexToObject: package (rank in the list) and export.
    pub fn locate(&self, mut index: u32) -> Option<(usize, usize)> {
        for (k, p) in self.packages.iter().enumerate() {
            if index < p.object_count {
                return Some((k, index as usize));
            }
            index -= p.object_count;
        }
        None
    }

    /// Name of a network name index (UPackageMap::SerializeName when reading).
    pub fn name_at(&self, mut index: u32) -> Option<&str> {
        for p in &self.packages {
            if index < p.name_count {
                return p.names.get(index as usize).map(String::as_str);
            }
            index -= p.name_count;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_round_trip() {
        let g = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        assert_eq!(guid_string(&g), "04030201080706050C0B0A09100F0E0D");
        assert_eq!(parse_guid(&guid_string(&g)), Some(g));
    }

    /// Text received from the network: a non-ASCII character across a slice boundary gives None, not a panic.
    #[test]
    fn guid_from_bad_text() {
        assert_eq!(parse_guid("0403020\u{e9}080706050C0B0A09100F0E0D"), None);
        assert_eq!(parse_guid(&"\u{20ac}".repeat(11)), None);
        assert_eq!(parse_guid("0403020"), None);
        assert_eq!(parse_guid("04030201080706050C0B0A09100F0E0D0"), None);
        assert_eq!(parse_guid("+4030201080706050C0B0A09100F0E0D"), None);
    }
}
