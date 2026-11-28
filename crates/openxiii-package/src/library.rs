//! Set of packages loaded on demand from the game folders, with resolution of
//! "Package.Group.Object" object paths to an export.

use crate::{ObjectRef, Package};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

const EXTENSIONS: &[&str] = &["u", "usx", "utx", "uax", "ukx", "unr"];

/// Package files from outside the game's folders (the mods' maps and their packages, and the
/// MapsUser folder, registered by the mods build at startup): name -> file. Empty otherwise,
/// so that the game's own files are the only ones found.
static EXTRA: std::sync::OnceLock<Vec<(String, PathBuf)>> = std::sync::OnceLock::new();

/// Registers package files found outside the game's folders (once, at startup). A name the
/// game already has is not taken (the caller warns).
pub fn set_extra_files(files: Vec<(String, PathBuf)>) {
    let _ = EXTRA.set(files);
}

fn extra_file(name: &str) -> Option<&'static PathBuf> {
    EXTRA.get()?.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, p)| p)
}

/// THE file of map `file` ("Plage01.unr", or "Plage01"): the game's Maps folder; when files
/// were registered with `set_extra_files` (the mods build), then the MapsUser folder (as the
/// game's Paths list it) and the registered files (the mods' maps); else the Maps folder's
/// path (which the caller then reports as missing).
pub fn map_path(game: &Path, file: &str) -> PathBuf {
    let file = if file.contains('.') { file.to_string() } else { format!("{file}.unr") };
    let own = game.join("Maps").join(&file);
    if own.is_file() || EXTRA.get().is_none() {
        return own;
    }
    let user = game.join("MapsUser").join(&file);
    if user.is_file() {
        return user;
    }
    let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
    extra_file(stem).cloned().unwrap_or(own)
}

pub struct LoadedPackage {
    pub name: String,
    pub path: PathBuf,
    pub data: Vec<u8>,
    pub package: Package,
}

struct CachedPackage {
    loaded: LoadedPackage,
    index: OnceLock<ExportIndex>,
}

impl CachedPackage {
    fn new(loaded: LoadedPackage) -> Arc<Self> {
        Arc::new(Self { loaded, index: OnceLock::new() })
    }
}

pub struct Library {
    revision: u64,
    /// Lowercase package name -> file.
    files: HashMap<String, PathBuf>,
    loaded: HashMap<String, Option<Arc<CachedPackage>>>,
    /// Files prepared for this map by the other library. Only a real lookup promotes
    /// one to `loaded`, so a persistent VM can release the rest on travel.
    prepared: HashMap<String, Arc<CachedPackage>>,
    /// `resolve` results already found: lowercase path -> (lowercase package, export). Otherwise the
    /// lookup walks every export and builds its path on each call.
    resolved: HashMap<String, (String, usize)>,
}

/// Export index of one package, each list in increasing order.
#[derive(Default)]
struct ExportIndex {
    /// Path inside the package (`object_path`), lowercase -> exports.
    paths: HashMap<String, Vec<usize>>,
    /// Lowercase export name -> exports.
    names: HashMap<String, Vec<usize>>,
}

impl ExportIndex {
    fn build(p: &Package) -> Self {
        let mut index = ExportIndex::default();
        for i in 0..p.exports.len() {
            index.paths.entry(p.object_path(ObjectRef::Export(i)).to_ascii_lowercase()).or_default().push(i);
            index.names.entry(p.name(p.exports[i].object_name).to_ascii_lowercase()).or_default().push(i);
        }
        index
    }

    /// Exports whose path equals `rest` or `path` (case-insensitive), in increasing order.
    fn exact(&self, rest: &str, path: &str) -> Vec<usize> {
        let (a, b) = (rest.to_ascii_lowercase(), path.to_ascii_lowercase());
        let mut v: Vec<usize> = self.paths.get(&a).into_iter().flatten().copied().collect();
        if b != a {
            v.extend(self.paths.get(&b).into_iter().flatten().copied());
            v.sort_unstable();
            v.dedup();
        }
        v
    }

    fn named(&self, name: &str) -> &[usize] {
        self.names.get(&name.to_ascii_lowercase()).map_or(&[], Vec::as_slice)
    }
}

impl Library {
    /// The same package files, nothing loaded yet: a library another thread can read on its own.
    pub fn fresh_copy(&self) -> Library {
        Library { revision: 0, files: self.files.clone(), loaded: HashMap::new(), resolved: HashMap::new(), prepared: HashMap::new() }
    }

    /// Indexes the packages in the game's usual subfolders (System, StaticMeshes, …).
    pub fn new(game: &Path) -> std::io::Result<Self> {
        let mut files = HashMap::new();
        // Game folders and their subfolders (e.g. System/PC, where the PC packages live).
        let mut dirs = Vec::new();
        for dir in std::fs::read_dir(game)? {
            let dir = dir?.path();
            if dir.is_dir() {
                for sub in std::fs::read_dir(&dir)? {
                    let sub = sub?.path();
                    if sub.is_dir() {
                        dirs.push(sub);
                    }
                }
                dirs.insert(0, dir);
            }
        }
        for dir in dirs {
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
                if ext.as_deref().is_some_and(|e| EXTENSIONS.contains(&e)) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        files.entry(stem.to_ascii_lowercase()).or_insert(path);
                    }
                }
            }
        }
        // Registered files (mods build): after the game's own, which keep their names.
        for (name, path) in EXTRA.get().into_iter().flatten() {
            files.entry(name.to_ascii_lowercase()).or_insert_with(|| path.clone());
        }
        Ok(Library { revision: next_revision(), files, loaded: HashMap::new(), resolved: HashMap::new(), prepared: HashMap::new() })
    }

    /// Identifies the indexed content, even after the whole library has been replaced.
    pub fn revision(&self) -> u64 { self.revision }

    /// Adds a package already in memory (e.g. the map itself, which may contain its own objects).
    pub fn insert(&mut self, name: &str, path: PathBuf, data: Vec<u8>, package: Package) {
        let p = LoadedPackage { name: name.to_string(), path, data, package };
        let key = name.to_ascii_lowercase();
        self.prepared.remove(&key);
        self.loaded.insert(key, Some(CachedPackage::new(p)));
        self.resolved.clear();
        self.revision = next_revision();
    }

    /// The file of package `name` becomes `path` (package received into the cache, Cache\GUID.uxx); the package
    /// already read under that name is forgotten.
    pub fn set_file(&mut self, name: &str, path: PathBuf) {
        let key = name.to_ascii_lowercase();
        self.files.insert(key.clone(), path);
        self.loaded.remove(&key);
        self.prepared.remove(&key);
        self.resolved.clear();
        self.revision = next_revision();
    }

    /// Forgets package `name` (tests: package missing on the client).
    pub fn forget(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        self.files.remove(&key);
        self.loaded.remove(&key);
        self.prepared.remove(&key);
        self.resolved.clear();
        self.revision = next_revision();
    }

    /// File of package `name` (case-insensitive), if it exists in the game folders.
    pub fn file_of(&self, name: &str) -> Option<&PathBuf> {
        self.files.get(&name.to_ascii_lowercase())
    }

    /// Whether package `name` is already read (without reading it).
    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded.get(&name.to_ascii_lowercase()).is_some_and(Option::is_some)
    }

    pub fn get(&mut self, name: &str) -> Option<&LoadedPackage> {
        Some(&self.cached(name)?.loaded)
    }

    fn cached(&mut self, name: &str) -> Option<&Arc<CachedPackage>> {
        let key = name.to_ascii_lowercase();
        self.loaded.entry(key.clone()).or_insert_with(|| {
            self.prepared.remove(&key).or_else(|| {
                let path = self.files.get(&key)?;
                let mut ch = crate::loadings::start_value(crate::loadings::PACKAGE);
                ch.name_value(|| name.to_string());
                let data = std::fs::read(path).ok()?;
                crate::events::file_read(path, data.len());
                if let Ok(mut reads) = crate::loadings::READ_PACKAGES.lock() {
                    reads.push(name.to_string());
                }
                let package = Package::parse(&data).ok()?;
                Some(CachedPackage::new(LoadedPackage { name: name.to_string(), path: path.clone(), data, package }))
            })
        }).as_ref()
    }

    /// Finds the export designated by a full path; returns (package, export index).
    /// Like `resolve`, the export of class `class` first (StaticFindObject with a class:
    /// a group and an object may share a name, "XIIIFonts.XIIIConsoleFont").
    pub fn resolve_class(&mut self, path: &str, class: &str) -> Option<(&LoadedPackage, usize)> {
        let (package_name, rest) = path.split_once('.')?;
        let index = {
            let (p, x) = self.indexed(package_name)?;
            let is_class = |i: &usize| p.export_class(*i).eq_ignore_ascii_case(class);
            // ULinkerLoad::Create → FindExportIndex with
            // ExportOuterIndex = −1: name and class, the group is ignored ("XIIIMenuStart.continue01"
            // is "interface_home.continue01", images of the main menu tiles).
            let name = rest.rsplit('.').next().unwrap_or(rest);
            let exact = x.exact(rest, path);
            exact.iter().copied().find(is_class).or_else(|| x.named(name).iter().copied().find(is_class)).or_else(|| exact.first().copied())?
        };
        Some((self.get(package_name)?, index))
    }

    /// Reads package `name` and builds its export index right away (loading screen)
    /// instead of on the first lookup mid-game. Does not change what is found.
    pub fn warm_up(&mut self, name: &str) {
        let _ = self.indexed(name);
    }

    /// Prepare the level and the script packages loaded with it, including their
    /// transitive imports. Both consumers can use the same immutable bytes and export
    /// index. A download or mod override in the receiving library keeps its own file.
    pub fn preload_map<'a>(&mut self, level_packages: impl IntoIterator<Item = &'a str>, mut other: Option<&mut Library>) {
        if let Some(other) = other.as_deref_mut() {
            other.prepared.clear();
        }
        self.preload_imports(level_packages.into_iter().map(str::to_owned), other);
    }

    /// Read and index the transitive imports of every package already loaded.
    /// Returns how many additional package files were requested.
    pub fn read_imported_packages(&mut self) -> usize {
        let roots: Vec<_> = self.loaded.values().flatten().map(|p| p.loaded.name.clone()).collect();
        self.preload_imports(roots, None)
    }

    fn preload_imports(&mut self, roots: impl IntoIterator<Item = String>, mut other: Option<&mut Library>) -> usize {
        let mut pending: BTreeSet<_> = roots.into_iter().map(|name| (name.to_ascii_lowercase(), name)).collect();
        let mut visited = HashSet::new();
        let mut reads = 0;
        while let Some((key, name)) = pending.pop_first() {
            if !visited.insert(key.clone()) {
                continue;
            }
            if !self.loaded.contains_key(&key) {
                if !self.files.contains_key(&key) {
                    continue;
                }
                reads += 1;
            }
            let Some(cached) = self.cached(&name) else { continue };
            let package = &cached.loaded.package;
            cached.index.get_or_init(|| ExportIndex::build(package));
            if let Some(other) = other.as_deref_mut() {
                if !other.loaded.contains_key(&key) && other.files.get(&key) == Some(&cached.loaded.path) {
                    other.prepared.insert(key, Arc::clone(cached));
                }
            }
            for import in &package.imports {
                if import.package == 0 && package.name(import.class_name).eq_ignore_ascii_case("Package") {
                    let name = package.name(import.object_name);
                    let key = name.to_ascii_lowercase();
                    if !visited.contains(&key) {
                        pending.insert((key, name.to_owned()));
                    }
                }
            }
        }
        reads
    }

    /// Full paths of the objects the read script packages (.u) import whose class is one of
    /// `classes` (UObject::VerifyImport loads them with the package in the original).
    pub fn imported_objects(&self, classes: &[&str]) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        for lp in self.loaded.values().flatten() {
            let lp = &lp.loaded;
            if !lp.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("u")) {
                continue;
            }
            let p = &lp.package;
            for (i, imp) in p.imports.iter().enumerate() {
                if classes.iter().any(|c| p.name(imp.class_name).eq_ignore_ascii_case(c)) {
                    out.insert(p.object_path((-(i as i32) - 1).into()));
                }
            }
        }
        out.into_iter().collect()
    }

    /// Package `name` and its export index, built on its first lookup.
    fn indexed(&mut self, name: &str) -> Option<(&Package, &ExportIndex)> {
        let cached = self.cached(name)?;
        let package = &cached.loaded.package;
        Some((package, cached.index.get_or_init(|| ExportIndex::build(package))))
    }

    pub fn resolve(&mut self, path: &str) -> Option<(&LoadedPackage, usize)> {
        let key = path.to_ascii_lowercase();
        if let Some((package, index)) = self.resolved.get(&key) {
            let index = *index;
            let package = package.clone();
            return self.get(&package).map(|l| (l, index));
        }
        let (package_name, _) = path.split_once('.')?;
        let index = self.resolve_uncached(path)?;
        self.resolved.insert(key, (package_name.to_ascii_lowercase(), index));
        self.get(package_name).map(|l| (l, index))
    }

    fn resolve_uncached(&mut self, path: &str) -> Option<usize> {
        let mut ch = crate::loadings::start_value(crate::loadings::OBJECT);
        ch.name_value(|| path.to_string());
        let (package_name, rest) = path.split_once('.')?;
        let (p, x) = self.indexed(package_name)?;
        let group = |i: &usize| p.export_class(*i).eq_ignore_ascii_case("Package");
        let exact = x.exact(rest, path);
        // A group (Package) only matches when there is neither an object with that path nor one with that name:
        // "XIIIMenu.Loading", loaded as a Texture by XIIIBaseHud.DrawLevelAction, is the texture
        // Loading.Loading (ULinkerLoad::Create looks up name and class), not the Loading group.
        let index = exact
            .iter()
            .copied()
            .find(|i| !group(i))
            .or_else(|| {
                // UObject::StaticLoadObject goes through ULinkerLoad::Create, which looks the export up by
                // name across the whole package, ignoring the group ("XIIIMenuStart.sortie"
                // finds "interface_home.sortie").
                let name = rest.rsplit('.').next().unwrap_or(rest);
                x.named(name).iter().copied().find(|i| !group(i))
            })
            .or_else(|| exact.first().copied())?;
        Some(index)
    }
}
