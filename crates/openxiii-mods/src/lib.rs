//! Mod discovery for OpenXIII.
//!
//! A mod is a sub-folder of the Mods folder that contains a `mod.toml` file:
//!
//! ```toml
//! [mod]
//! name = "Example"
//! version = "1.0.0"
//! author = "Someone"          # optional
//! description = "What it does" # optional
//! openxiii = "0.1.0"          # optional, expected OpenXIII version
//! ```
//!
//! Discovery never fails as a whole: a malformed `mod.toml` produces an error for that mod (file,
//! line, what is wrong) and the mod is skipped; a sub-folder without `mod.toml` produces a warning.
//! A missing or empty Mods folder means no mods.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A file named inside a mod (a glTF buffer or texture URI): `dir` joined with `rel` when `rel`
/// stays below `dir`. An absolute path, a drive (`C:`), or a `..` part gives `None`.
pub(crate) fn path_in(dir: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains(':') || rel.starts_with(['/', '\\']) {
        return None;
    }
    let mut out = dir.to_path_buf();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => out.push(p),
        }
    }
    Some(out)
}

/// Items of an accessor without a buffer view (all zeros, possibly with sparse values): at most
/// this many, since no bytes of the file bound them.
pub(crate) const MAX_ZERO_ACCESSOR_ITEMS: usize = 1 << 20;

/// glTF data check, before anything is read: every buffer view lies inside the bytes of its
/// buffer and every accessor (and its sparse indices and values) inside its buffer view, with at
/// least one item, so no count in the file can ask for more items than the bytes present.
pub(crate) fn check_gltf_data(doc: &gltf::Document, buffers: &[Vec<u8>]) -> Result<(), String> {
    for v in doc.views() {
        let len = buffers.get(v.buffer().index()).map_or(0, Vec::len);
        if v.offset().checked_add(v.length()).is_none_or(|end| end > len) {
            return Err(format!("buffer view {} ({} bytes at {}) goes past the end of its buffer ({len} bytes)", v.index(), v.length(), v.offset()));
        }
    }
    // Bytes taken by `count` items of `size` bytes, `stride` apart, from `offset`.
    let span = |offset: usize, stride: usize, count: usize, size: usize| (count - 1).checked_mul(stride).and_then(|b| b.checked_add(size)).and_then(|b| b.checked_add(offset));
    for a in doc.accessors() {
        let i = a.index();
        if a.count() == 0 {
            return Err(format!("accessor {i} has no items"));
        }
        match a.view() {
            Some(view) => {
                let stride = view.stride().unwrap_or(a.size());
                if span(a.offset(), stride, a.count(), a.size()).is_none_or(|end| end > view.length()) {
                    return Err(format!("accessor {i} has {} items, more than its buffer view's {} bytes hold", a.count(), view.length()));
                }
            }
            None if a.count() > MAX_ZERO_ACCESSOR_ITEMS => {
                return Err(format!("accessor {i} has {} items without a buffer view, at most {MAX_ZERO_ACCESSOR_ITEMS} are supported", a.count()));
            }
            None => {}
        }
        if let Some(sparse) = a.sparse() {
            let n = sparse.count();
            let (ix, values) = (sparse.indices(), sparse.values());
            let ix_size = ix.index_type().size();
            let ok = n >= 1
                && n <= a.count()
                && span(ix.offset(), ix.view().stride().unwrap_or(ix_size), n, ix_size).is_some_and(|end| end <= ix.view().length())
                && span(values.offset(), values.view().stride().unwrap_or(a.size()), n, a.size()).is_some_and(|end| end <= values.view().length());
            if !ok {
                return Err(format!("accessor {i}: its {n} sparse values do not fit in their buffer views"));
            }
        }
    }
    Ok(())
}

/// glTF node hierarchy check: every node has at most one parent and following the parents
/// always ends at a root, so walking the tree terminates.
pub(crate) fn check_gltf_tree(doc: &gltf::Document) -> Result<Vec<Option<usize>>, String> {
    let count = doc.nodes().len();
    let mut parent: Vec<Option<usize>> = vec![None; count];
    for n in doc.nodes() {
        for c in n.children() {
            if parent[c.index()].replace(n.index()).is_some() {
                return Err(format!("node {} has more than one parent", c.index()));
            }
        }
    }
    // 0: not checked yet, 1: on the path being followed, 2: reaches a root.
    let mut state = vec![0u8; count];
    for start in 0..count {
        let mut path = Vec::new();
        let mut i = start;
        loop {
            match state[i] {
                2 => break,
                1 => return Err(format!("node {i} is its own ancestor")),
                _ => {}
            }
            state[i] = 1;
            path.push(i);
            match parent[i] {
                Some(p) => i = p,
                None => break,
            }
        }
        for k in path {
            state[k] = 2;
        }
    }
    Ok(parent)
}

pub mod behaviour;
pub mod content;
pub mod defaults;
pub mod dialogue;
pub mod log;
pub mod looks;
pub mod maps;
pub mod new_classes;
pub mod own_animated;
pub mod own_maps;
pub mod own_arms;
pub mod own_models;
pub mod own_sounds;
pub mod own_textures;
pub mod placement;
pub mod scripts;
pub mod simple;
pub mod names_doc;
pub mod when;
#[cfg(test)]
mod hostile_tests;
pub use defaults::{collect_overrides, DefaultOverride, ModValue};
pub use placement::{Edit, MapEdit};
/// The toml crate the mods' files are read with (the scripts' `change` gives it a table).
pub use toml;

/// Name of the manifest file inside each mod folder.
pub const MANIFEST: &str = "mod.toml";
/// Environment variable that overrides the Mods folder location.
pub const ENV_MODS: &str = "OXIII_MODS";
/// Environment variable that disables mods when set to `1`.
pub const ENV_NO_MODS: &str = "OXIII_NO_MODS";
/// Command-line switch that disables mods.
pub const ARG_NO_MODS: &str = "--no-mods";

/// Contents of the `[mod]` table of a `mod.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// OpenXIII version the mod expects.
    #[serde(default)]
    pub openxiii: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(rename = "mod")]
    info: ModInfo,
}

/// A discovered, valid mod.
#[derive(Debug, Clone, PartialEq)]
pub struct Mod {
    /// Name of the mod's folder (the sort key).
    pub folder_name: String,
    /// Full path of the mod's folder.
    pub folder: PathBuf,
    pub info: ModInfo,
    /// Class default values it changes (`[class."Package.Class"]` tables of mod.toml, then of
    /// the other .toml files of the folder in name order).
    pub overrides: Vec<DefaultOverride>,
    /// Changes to the actors of maps (`[[map.<Map>.add]]`, `remove`, `replace`, `start`), in
    /// file order.
    pub edits: Vec<MapEdit>,
    /// Keys bound to actions (`[[action]]`).
    pub actions: Vec<behaviour::ModAction>,
    /// Mod characters (`[character.<Name>]`).
    pub characters: Vec<behaviour::CharacterDef>,
    /// Sound banks and packages to load (`load_sound_banks`, `load_packages`).
    pub loads: Vec<content::LoadList>,
    /// The mod's own sounds (`[sounds.<Name>]`).
    pub sounds: Vec<own_sounds::OwnSound>,
    /// The mod's own textures (`[textures.<Name>]`).
    pub textures: Vec<own_textures::OwnTexture>,
    /// The mod's own 3D models (`[models.<Name>]`).
    pub models: Vec<own_models::OwnModel>,
    /// The mod's first-person arms made from a game mesh (`[arms.<Name>]`).
    pub arms: Vec<own_arms::OwnArms>,
    /// The mod's new classes (`[classes.<Name>]`).
    pub new_classes: Vec<new_classes::NewClass>,
    /// The mod's Lua scripts (`scripts/*.lua`), in name order.
    pub scripts: Vec<scripts::ScriptFile>,
}

impl Mod {
    /// Fingerprint of the mod's files (every file of its folder and sub-folders: relative path
    /// and contents), 16 hexadecimal digits. Two copies of a mod with the same files have the
    /// same fingerprint; changing any file changes it.
    pub fn fingerprint(&self) -> String {
        let mut files = Vec::new();
        collect_files(&self.folder, &self.folder, &mut files);
        files.sort();
        let mut h = Fnv::new(0);
        for (rel, path) in files {
            h.write(rel.as_bytes());
            h.write(&[0]);
            h.write(&fingerprint_bytes(&rel, std::fs::read(&path).unwrap_or_default()));
            h.write(&[0]);
        }
        format!("{:016X}", h.finish())
    }
}

/// Text files of a mod, whose line ends are not part of its fingerprint.
const TEXT_EXTENSIONS: &[&str] = &["toml", "lua", "txt", "ini", "int", "md", "gltf", "json", "csv"];

/// The bytes of a mod's file `rel` that its fingerprint reads: a text file with its line ends
/// as "\n", so that the same mod copied to Windows, Linux or Android (a copy or a checkout that
/// turned "\r\n" into "\n" or back) is the same mod on the network.
fn fingerprint_bytes(rel: &str, bytes: Vec<u8>) -> Vec<u8> {
    let text = rel.rsplit_once('.').is_some_and(|(_, ext)| TEXT_EXTENSIONS.contains(&ext));
    if !text || !bytes.contains(&b'\r') {
        return bytes;
    }
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if !(b == b'\r' && bytes.get(i + 1) == Some(&b'\n')) {
            out.push(b);
        }
    }
    out
}

/// The file `rel` of a mod folder (a path written in one of its tables, "/" or "\" between
/// folders): each part found whatever its case, as on Windows, so that a mod made on Windows
/// loads the same on Linux and Android. A part that is not there is kept as written.
pub(crate) fn file_in_mod(mod_dir: &Path, rel: &str) -> PathBuf {
    let mut out = mod_dir.to_path_buf();
    // ".." never leaves the mod folder (the tables refuse it; "\" is no separator on Linux).
    for part in rel.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..") {
        let exact = out.join(part);
        let found = if exact.exists() {
            None
        } else {
            std::fs::read_dir(&out).ok().and_then(|it| it.filter_map(Result::ok).map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(part)))
        };
        out = match found {
            Some(name) => out.join(name),
            None => exact,
        };
    }
    out
}

/// Relative paths ("/" separators, lower case) and full paths of the files under `dir`.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>().join("/");
            out.push((rel, path));
        }
    }
}

/// FNV-1a, 64 bits (a fingerprint, not a protection against forgery).
struct Fnv(u64);

impl Fnv {
    fn new(seed: u64) -> Self {
        Fnv(0xcbf2_9ce4_8422_2325 ^ seed)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// What a game says about one of its mods on the network: folder name, version, fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModId {
    pub name: String,
    pub version: String,
    pub fingerprint: String,
}

impl ModId {
    /// "name version (fingerprint)".
    pub fn label(&self) -> String {
        format!("{} {} ({})", self.name, self.version, self.fingerprint)
    }
}

/// The identities of `mods`, in load order.
pub fn mod_ids(mods: &[Mod]) -> Vec<ModId> {
    mods.iter().map(|m| ModId { name: m.folder_name.clone(), version: m.info.version.clone(), fingerprint: m.fingerprint() }).collect()
}

/// The identifier of a set of mods on the network (16 bytes, the GUID of the "Mod" package):
/// the same for the same mods, with the same files, in the same order.
pub fn mods_guid(ids: &[ModId]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (half, seed) in [(0, 0u64), (1, 0x9e37_79b9_7f4a_7c15)] {
        let mut h = Fnv::new(seed);
        for id in ids {
            for part in [&id.name, &id.version, &id.fingerprint] {
                h.write(part.as_bytes());
                h.write(&[0]);
            }
        }
        out[half * 8..half * 8 + 8].copy_from_slice(&h.finish().to_le_bytes());
    }
    out
}

/// Text of the differences between the mods of a server and those of a client, one item per
/// difference, for the message that refuses the client.
pub fn mods_differences(server: &[ModId], client: &[ModId]) -> Vec<String> {
    let mut out = Vec::new();
    for s in server {
        match client.iter().find(|c| c.name.eq_ignore_ascii_case(&s.name)) {
            None => out.push(format!("missing mod {} {}", s.name, s.version)),
            Some(c) if c.version != s.version => out.push(format!("mod {}: version {} instead of {}", s.name, c.version, s.version)),
            Some(c) if c.fingerprint != s.fingerprint => out.push(format!("mod {} {}: its files differ", s.name, s.version)),
            _ => {}
        }
    }
    for c in client {
        if !server.iter().any(|s| s.name.eq_ignore_ascii_case(&c.name)) {
            out.push(format!("extra mod {} {} (the server does not have it)", c.name, c.version));
        }
    }
    if out.is_empty() && server != client {
        out.push("the mods are loaded in another order".into());
    }
    out
}

/// A problem met during discovery. Errors skip the mod; warnings are informational.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A file or a value could not be used (the message says what was skipped).
    Error { file: PathBuf, line: Option<usize>, message: String },
    /// Something was ignored (for example a folder without `mod.toml`).
    Warning { path: PathBuf, message: String },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Error { file, line: Some(line), message } => {
                write!(f, "error: {} (line {line}): {message}", file.display())
            }
            Problem::Error { file, line: None, message } => write!(f, "error: {}: {message}", file.display()),
            Problem::Warning { path, message } => write!(f, "warning: {}: {message}", path.display()),
        }
    }
}

/// Result of a discovery: valid mods sorted by folder name, and the problems met.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Discovery {
    pub mods: Vec<Mod>,
    pub problems: Vec<Problem>,
    /// Folder names of the mods skipped because they are turned off (DISABLED_FILE).
    pub disabled: Vec<String>,
}

/// Two mods that set the same thing: the later one (folder order) wins at load time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Folder name of the mod loaded first (its value is replaced).
    pub first: String,
    /// Folder name of the mod loaded later (its value is kept).
    pub second: String,
    /// What both set: "value XIII.Magnum.ReloadCount", "key F5", "texture Mod.Poster"…
    pub what: String,
}

impl Conflict {
    /// Adds the conflict between mods `first` and `second` on `what` to `out`, unless both are
    /// the same mod (a mod may set a value twice) or it is already there.
    pub(crate) fn note(out: &mut Vec<Conflict>, first: &str, second: &str, what: String) {
        if first.eq_ignore_ascii_case(second) {
            return;
        }
        let c = Conflict { first: first.to_string(), second: second.to_string(), what };
        if !out.contains(&c) {
            out.push(c);
        }
    }

    /// True if mod folder `name` is one of the two.
    pub fn involves(&self, name: &str) -> bool {
        self.first.eq_ignore_ascii_case(name) || self.second.eq_ignore_ascii_case(name)
    }

    /// The other mod of the two, seen from mod folder `name`.
    pub fn other(&self, name: &str) -> &str {
        if self.first.eq_ignore_ascii_case(name) {
            &self.second
        } else {
            &self.first
        }
    }
}

/// The conflicts between `mods` (loaded together, in this order): the same checks as at load
/// time (class values, new classes, map replacements, keys and characters, then the mods' own
/// textures and models defined twice or replacing the same game texture or model).
pub fn find_conflicts(mods: &[Mod]) -> Vec<Conflict> {
    let mut out = Vec::new();
    defaults::collect_overrides_into(mods, &mut out);
    collect_new_classes_into(mods, &mut out);
    collect_edits_into(mods, &mut out);
    collect_behaviour_into(mods, &mut out);
    // Own textures and models: the first one of a name is kept (mod_setup), and the later of
    // two replacements of the same game texture or model is drawn.
    let mut seen: HashMap<String, &str> = HashMap::new();
    for t in mods.iter().flat_map(|m| m.textures.iter()) {
        let name = t.texture_name();
        match seen.get(&format!("t:{}", name.to_ascii_lowercase())) {
            Some(first) => Conflict::note(&mut out, first, &t.source.mod_name, format!("texture {name}")),
            None => {
                seen.insert(format!("t:{}", name.to_ascii_lowercase()), &t.source.mod_name);
                if let Some(r) = &t.replace {
                    let key = format!("tr:{}", r.to_ascii_lowercase());
                    if let Some(first) = seen.get(&key) {
                        Conflict::note(&mut out, first, &t.source.mod_name, format!("replacement of texture {r}"));
                    }
                    seen.insert(key, &t.source.mod_name);
                }
            }
        }
    }
    for m in mods.iter().flat_map(|m| m.models.iter()) {
        let name = m.mesh_name();
        match seen.get(&format!("m:{}", name.to_ascii_lowercase())) {
            Some(first) => Conflict::note(&mut out, first, &m.source.mod_name, format!("model {name}")),
            None => {
                seen.insert(format!("m:{}", name.to_ascii_lowercase()), &m.source.mod_name);
                if let Some(r) = &m.replace {
                    let key = format!("mr:{}", r.to_ascii_lowercase());
                    if let Some(first) = seen.get(&key) {
                        Conflict::note(&mut out, first, &m.source.mod_name, format!("replacement of model {r}"));
                    }
                    seen.insert(key, &m.source.mod_name);
                }
            }
        }
    }
    out
}

/// File of the Mods folder that lists the mods turned off, one folder name per line (`#` starts
/// a comment). Written by the improved build's Mods page; read by `discover`.
pub const DISABLED_FILE: &str = "disabled-mods.txt";

/// Folder names listed in the text of DISABLED_FILE (trimmed, comments and blank lines left out).
pub fn parse_disabled(text: &str) -> Vec<String> {
    text.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// The mods turned off in `folder` (empty when DISABLED_FILE is missing).
pub fn read_disabled(folder: &Path) -> Vec<String> {
    std::fs::read_to_string(folder.join(DISABLED_FILE)).map(|t| parse_disabled(&t)).unwrap_or_default()
}

/// Writes the list of the mods turned off in `folder` (the file is removed when the list is empty).
pub fn write_disabled(folder: &Path, names: &[String]) -> std::io::Result<()> {
    let file = folder.join(DISABLED_FILE);
    if names.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let mut text = String::from("# Mods turned off (one folder name per line). Changes apply at next launch.\r\n");
    for n in names {
        text.push_str(n);
        text.push_str("\r\n");
    }
    std::fs::write(file, text)
}

/// True if folder `name` is in `disabled` (case is ignored, as folder names on Windows).
pub fn is_disabled(disabled: &[String], name: &str) -> bool {
    disabled.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// Folder of the Mods folder that holds the example mods shipped with the game. Its mods are
/// never loaded nor listed: a mod becomes active when its folder is copied into the Mods folder.
pub const EXAMPLES_DIR: &str = "_Examples";

/// Folder of the Mods folder for the modders: the guide, the names catalog and the lists of names
/// (openxiii-vm mod_catalog.rs). Never loaded nor listed either.
pub const GUIDE_DIR: &str = "_Guide";

/// The examples folder an earlier package put inside the Mods folder: never loaded nor listed
/// either, so that an older install does not show it.
pub const OLD_EXAMPLES_FOLDER: &str = "examples";

/// Folder `name` of the Mods folder holds no mod of its own: the examples, the guide, or the
/// examples folder of an earlier package (case is ignored, as on Windows). Such a folder is
/// skipped without a warning.
pub fn is_reserved_folder(name: &str) -> bool {
    [EXAMPLES_DIR, GUIDE_DIR, OLD_EXAMPLES_FOLDER].iter().any(|r| name.eq_ignore_ascii_case(r))
}

/// A mod as the Mods page lists it: its folder, the name and version of its mod.toml, and
/// whether it is turned off.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub folder_name: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
}

/// The mods of `folder` with a readable mod.toml, sorted by folder name, turned off or not (only
/// the manifest is read: listing is cheap and never loads the mods' files). The examples and the
/// guide inside it are not listed ([`is_reserved_folder`]).
pub fn list_mods(folder: &Path) -> Vec<Listing> {
    let disabled = read_disabled(folder);
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<Listing> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let folder_name = e.file_name().to_string_lossy().into_owned();
            if is_reserved_folder(&folder_name) {
                return None;
            }
            let file = e.path().join(MANIFEST);
            let info = parse_manifest(&std::fs::read_to_string(&file).ok()?, &file).ok()?;
            Some(Listing { enabled: !is_disabled(&disabled, &folder_name), folder_name, name: info.name, version: info.version })
        })
        .collect();
    out.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    out
}

/// Folders of the mods the game loaded when it started (openxiii-vm mod_setup.rs): the Mods
/// page marks the rows whose box differs from them. Not set when the mods were not read at
/// startup (--no-mods, the tools).
static LOADED_AT_START: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Records the folders of the mods loaded at startup (first call wins).
pub fn set_loaded_at_start(folders: Vec<String>) {
    let _ = LOADED_AT_START.set(folders);
}

/// The folders of the mods loaded at startup, if the mods were read then.
pub fn loaded_at_start() -> Option<&'static [String]> {
    LOADED_AT_START.get().map(Vec::as_slice)
}

/// The folder of Mods folder `mods` where the game writes the names catalog and the lists of
/// names for the modders, beside the guide ([`GUIDE_DIR`]; made when needed).
pub fn modding_folder(mods: &Path) -> PathBuf {
    mods.join(GUIDE_DIR)
}

/// Mods folder of a system where it is not next to the executable (Android: in shared storage,
/// where a computer can copy mods over USB), set once at startup before the mods are read.
static DEFAULT_FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Sets the Mods folder used when `OXIII_MODS` is not set (first call wins).
pub fn set_default_folder(folder: PathBuf) {
    let _ = DEFAULT_FOLDER.set(folder);
}

/// Mods folder: `OXIII_MODS` if set (and not empty), else the folder given by
/// [`set_default_folder`], else `Mods` next to the executable.
pub fn mods_folder() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(ENV_MODS).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = DEFAULT_FOLDER.get() {
        return Some(dir.clone());
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("Mods"))
}

/// True when mods are turned off by `--no-mods` in `args` or by `OXIII_NO_MODS=1`.
pub fn disabled<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().any(|a| a.as_ref() == ARG_NO_MODS) || std::env::var(ENV_NO_MODS).is_ok_and(|v| v.trim() == "1")
}

/// Parses the text of a `mod.toml`. `file` is only used in the error.
pub fn parse_manifest(text: &str, file: &Path) -> Result<ModInfo, Problem> {
    toml::from_str::<Manifest>(text).map(|m| m.info).map_err(|e| {
        let line = e.span().map(|s| line_of(text, s.start));
        Problem::Error { file: file.to_path_buf(), line, message: format!("{}; mod skipped", e.message().trim()) }
    })
}

/// 1-based line number of byte offset `pos` in `text`.
pub(crate) fn line_of(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    text.as_bytes()[..pos].iter().filter(|&&b| b == b'\n').count() + 1
}

/// Sub-folder of a mod whose `.toml` files are read too (one file per map, by convention).
pub const MAPS_FOLDER: &str = "maps";
/// Sub-folder reserved for the mods' scripts: never read as mod tables.
pub const SCRIPTS_FOLDER: &str = "scripts";

/// The `.toml` files of `dir` (not `mod.toml`), in name order.
fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|it| it.filter_map(Result::ok).map(|e| e.path()).collect()).unwrap_or_default();
    v.retain(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) && p.file_name().is_some_and(|n| n != MANIFEST));
    v.sort();
    v
}

/// Tables of `manifest` (already read as `text`), then of the other `.toml` files of `dir` in
/// name order, then of the `.toml` files of its `maps` sub-folder in name order. The `scripts`
/// sub-folder is not read.
fn read_mod_files(dir: &Path, manifest: &Path, text: &str, mod_name: &str) -> defaults::ModFile {
    let mut all = defaults::parse_mod_file(text, manifest, mod_name);
    let mut extra = toml_files(dir);
    extra.extend(toml_files(&dir.join(MAPS_FOLDER)));
    for file in extra {
        match std::fs::read_to_string(&file) {
            Ok(t) => {
                let f = defaults::parse_mod_file_in(&t, &file, mod_name, dir);
                all.overrides.extend(f.overrides);
                all.edits.extend(f.edits);
                all.actions.extend(f.actions);
                all.characters.extend(f.characters);
                all.loads.extend(f.loads);
                all.sounds.extend(f.sounds);
                all.textures.extend(f.textures);
                all.models.extend(f.models);
                all.arms.extend(f.arms);
                all.new_classes.extend(f.new_classes);
                all.problems.extend(f.problems);
            }
            Err(e) => all.problems.push(Problem::Error { file, line: None, message: format!("cannot read file: {e}; file skipped") }),
        }
    }
    all
}

/// The actions and characters of `mods`. One action per key and one character per name: the
/// later mod wins and a warning says so.
pub fn collect_behaviour(mods: &[Mod]) -> (Vec<behaviour::ModAction>, Vec<behaviour::CharacterDef>, Vec<Problem>) {
    collect_behaviour_into(mods, &mut Vec::new())
}

/// [`collect_behaviour`], also noting in `conflicts` each key or character one mod takes from
/// another.
fn collect_behaviour_into(mods: &[Mod], conflicts: &mut Vec<Conflict>) -> (Vec<behaviour::ModAction>, Vec<behaviour::CharacterDef>, Vec<Problem>) {
    let mut actions: Vec<behaviour::ModAction> = Vec::new();
    let mut characters: Vec<behaviour::CharacterDef> = Vec::new();
    let mut problems = Vec::new();
    for a in mods.iter().flat_map(|m| m.actions.iter()) {
        if let Some(i) = actions.iter().position(|o| o.key.eq_ignore_ascii_case(&a.key)) {
            Conflict::note(conflicts, &actions[i].source.mod_name, &a.source.mod_name, format!("key {}", a.key));
            problems.push(Problem::Warning {
                path: a.source.file.clone(),
                message: format!("line {}: keyboard shortcut {} replaces the one of the same key ({})", a.source.line, a.key, actions[i].source),
            });
            actions.remove(i);
        }
        actions.push(a.clone());
    }
    for c in mods.iter().flat_map(|m| m.characters.iter()) {
        if let Some(i) = characters.iter().position(|o| o.name.eq_ignore_ascii_case(&c.name)) {
            Conflict::note(conflicts, &characters[i].source.mod_name, &c.source.mod_name, format!("character {}", c.name));
            problems.push(Problem::Warning {
                path: c.source.file.clone(),
                message: format!("line {}: character {} replaces the one of the same name ({})", c.source.line, c.name, characters[i].source),
            });
            characters.remove(i);
        }
        characters.push(c.clone());
    }
    (actions, characters, problems)
}

/// The new classes of `mods`. One class per name: the later mod wins and a warning says so.
pub fn collect_new_classes(mods: &[Mod]) -> (Vec<new_classes::NewClass>, Vec<Problem>) {
    collect_new_classes_into(mods, &mut Vec::new())
}

/// [`collect_new_classes`], also noting in `conflicts` each class one mod takes from another.
fn collect_new_classes_into(mods: &[Mod], conflicts: &mut Vec<Conflict>) -> (Vec<new_classes::NewClass>, Vec<Problem>) {
    let mut out: Vec<new_classes::NewClass> = Vec::new();
    let mut problems = Vec::new();
    for c in mods.iter().flat_map(|m| m.new_classes.iter()) {
        if let Some(i) = out.iter().position(|o| o.name.eq_ignore_ascii_case(&c.name)) {
            Conflict::note(conflicts, &out[i].source.mod_name, &c.source.mod_name, format!("class {}", c.class_name()));
            problems.push(Problem::Warning {
                path: c.source.file.clone(),
                message: format!("line {}: class {} replaces the one of the same name ({})", c.source.line, c.class_name(), out[i].source),
            });
            out.remove(i);
        }
        out.push(c.clone());
    }
    // The decorations' class, made when a mod adds a decoration (`model = "…"`).
    let edits: Vec<MapEdit> = mods.iter().flat_map(|m| m.edits.iter().cloned()).collect();
    if let Some(e) = placement::first_decoration(&edits) {
        out.push(new_classes::NewClass { name: placement::DECORATION_NAME.into(), parent: placement::DECORATION_PARENT.into(), source: e.source.clone() });
    }
    (out, problems)
}

/// The class values of the decorations' class (see [`collect_new_classes`]), when a mod adds a
/// decoration: the first class values of [`collect_overrides`].
pub(crate) fn decoration_overrides(mods: &[Mod]) -> Vec<DefaultOverride> {
    let edits: Vec<MapEdit> = mods.iter().flat_map(|m| m.edits.iter().cloned()).collect();
    let Some(e) = placement::first_decoration(&edits) else { return Vec::new() };
    placement::decoration_values()
        .into_iter()
        .map(|(property, value)| DefaultOverride {
            class: placement::DECORATION_CLASS.into(),
            property: property.into(),
            index: None,
            value,
            mod_name: e.source.mod_name.clone(),
            file: e.source.file.clone(),
            line: e.source.line,
            map: None,
            apply_to_placed: false,
        })
        .collect()
}

/// All the map changes of `mods`, in mod order. When two mods replace the same class on the
/// same map, the later one wins and a warning says so.
pub fn collect_edits(mods: &[Mod]) -> (Vec<MapEdit>, Vec<Problem>) {
    collect_edits_into(mods, &mut Vec::new())
}

/// [`collect_edits`], also noting in `conflicts` each class replacement one mod takes from
/// another.
fn collect_edits_into(mods: &[Mod], conflicts: &mut Vec<Conflict>) -> (Vec<MapEdit>, Vec<Problem>) {
    let mut out: Vec<MapEdit> = Vec::new();
    let mut problems = Vec::new();
    for e in mods.iter().flat_map(|m| m.edits.iter()) {
        if let Edit::Replace { from, to } = &e.edit {
            let earlier = out.iter().position(|o| o.map.eq_ignore_ascii_case(&e.map) && matches!(&o.edit, Edit::Replace { from: f, .. } if f.eq_ignore_ascii_case(from)));
            if let Some(i) = earlier {
                Conflict::note(conflicts, &out[i].source.mod_name, &e.source.mod_name, format!("map {}: replacement of {from}", e.map));
                problems.push(Problem::Warning {
                    path: e.source.file.clone(),
                    message: format!("line {}: map {}: replace {from} with {to} replaces \"{}\" ({})", e.source.line, e.map, out[i].describe(), out[i].source),
                });
                out.remove(i);
            }
        }
        out.push(e.clone());
    }
    (out, problems)
}

/// Discovers the mods of `folder`. A missing folder gives no mods and no problems.
pub fn discover(folder: &Path) -> Discovery {
    discover_with(folder, read_disabled(folder))
}

/// [`discover`] of every mod of `folder`, the ones turned off included (the Mods page checks
/// the conflicts of any choice).
pub fn discover_all(folder: &Path) -> Discovery {
    discover_with(folder, Vec::new())
}

fn discover_with(folder: &Path, disabled: Vec<String>) -> Discovery {
    let mut out = Discovery::default();
    let Ok(entries) = std::fs::read_dir(folder) else { return out };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (folder_name, dir) in dirs {
        // The examples, the guide, an older examples folder (earlier packages): never loaded.
        if is_reserved_folder(&folder_name) {
            continue;
        }
        if is_disabled(&disabled, &folder_name) {
            out.disabled.push(folder_name);
            continue;
        }
        let file = dir.join(MANIFEST);
        if !file.is_file() {
            out.problems.push(Problem::Warning { path: dir, message: format!("no {MANIFEST}, folder ignored") });
            continue;
        }
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                out.problems.push(Problem::Error { file, line: None, message: format!("cannot read file: {e}; mod skipped") });
                continue;
            }
        };
        match parse_manifest(&text, &file) {
            Ok(info) => {
                let f = read_mod_files(&dir, &file, &text, &folder_name);
                out.problems.extend(f.problems);
                let scripts = scripts::find(&dir, &folder_name);
                out.mods.push(Mod {
                    folder_name,
                    folder: dir,
                    info,
                    overrides: f.overrides,
                    edits: f.edits,
                    actions: f.actions,
                    characters: f.characters,
                    loads: f.loads,
                    sounds: f.sounds,
                    textures: f.textures,
                    models: f.models,
                    arms: f.arms,
                    new_classes: f.new_classes,
                    scripts,
                });
            }
            Err(p) => out.problems.push(p),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_mod(root: &Path, folder: &str, text: &str) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(MANIFEST), text).unwrap();
    }

    const VALID: &str = "[mod]\nname = \"Example\"\nversion = \"1.0.0\"\nauthor = \"Me\"\ndescription = \"Test mod\"\nopenxiii = \"0.1.0\"\n";

    #[test]
    fn disabled_list_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_disabled(tmp.path()).is_empty());
        write_disabled(tmp.path(), &["b".into(), "Other Mod".into()]).unwrap();
        assert_eq!(read_disabled(tmp.path()), ["b", "Other Mod"]);
        assert_eq!(parse_disabled("# note\n  a  \n\nc # why\n"), ["a", "c"]);
        // An empty list removes the file (twice: no error when it is already gone).
        write_disabled(tmp.path(), &[]).unwrap();
        write_disabled(tmp.path(), &[]).unwrap();
        assert!(!tmp.path().join(DISABLED_FILE).exists());
    }

    #[test]
    fn disabled_mods_are_listed_but_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "a", VALID);
        write_mod(tmp.path(), "b", "[mod]\nname = \"Second\"\nversion = \"2.0\"\n");
        fs::write(tmp.path().join(DISABLED_FILE), "B\n").unwrap();
        let d = discover(tmp.path());
        assert_eq!(d.mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(d.disabled, ["b"]);
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        let l = list_mods(tmp.path());
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].name.as_str(), l[0].version.as_str(), l[0].enabled), ("Example", "1.0.0", true));
        assert_eq!((l[1].name.as_str(), l[1].version.as_str(), l[1].enabled), ("Second", "2.0", false));
    }

    /// Only the mods in the Mods folder count: the examples (Mods\_Examples), the guide
    /// (Mods\_Guide) and an older examples folder (Mods\examples, earlier packages) are neither
    /// loaded nor listed, without a warning; an example copied into the Mods folder is loaded and
    /// listed at once, on, without any list to change. Negative control: the same folder under
    /// another name is read as a mod folder (without a mod.toml: a warning).
    #[test]
    fn only_the_mods_folder_counts() {
        for reserved in [EXAMPLES_DIR, "_examples", GUIDE_DIR, OLD_EXAMPLES_FOLDER] {
            let tmp = tempfile::tempdir().unwrap();
            let folder = tmp.path().join(reserved);
            write_mod(&folder, "first", VALID);
            fs::write(folder.join("README.txt"), "not a mod").unwrap();
            let d = discover(tmp.path());
            assert!(d.mods.is_empty() && d.problems.is_empty(), "{reserved}: {:?}", d.problems);
            assert!(list_mods(tmp.path()).is_empty(), "{reserved}");
        }
        let tmp = tempfile::tempdir().unwrap();
        let examples = tmp.path().join(EXAMPLES_DIR);
        write_mod(&examples, "first", VALID);
        write_mod(&examples, "second", "[mod]\nname = \"Second\"\nversion = \"2.0\"\n");
        let d = discover(tmp.path());
        assert!(d.mods.is_empty() && d.problems.is_empty(), "{:?}", d.problems);
        assert!(list_mods(tmp.path()).is_empty());
        // Copied into Mods: active at the next launch.
        fs::rename(examples.join("second"), tmp.path().join("second")).unwrap();
        assert_eq!(discover(tmp.path()).mods.iter().map(|m| m.folder_name.as_str()).collect::<Vec<_>>(), ["second"]);
        assert_eq!(list_mods(tmp.path()).iter().map(|m| (m.name.as_str(), m.enabled)).collect::<Vec<_>>(), [("Second", true)]);
        // Negative control.
        fs::rename(&examples, tmp.path().join("samples")).unwrap();
        assert!(discover(tmp.path()).problems.iter().any(|p| matches!(p, Problem::Warning { .. })));
    }

    /// The conflicts of a choice of mods name both mods and what they share; a mod setting a
    /// value twice, or mods that do not meet, give none.
    #[test]
    fn conflicts_of_a_choice() {
        let tmp = tempfile::tempdir().unwrap();
        let head = |n: &str| format!("[mod]\nname = \"{n}\"\nversion = \"1\"\n");
        write_mod(tmp.path(), "a", &format!("{}[class.\"XIII.Magnum\"]\nReloadCount = 5\n[classes.Gun]\nparent = \"XIII.Beretta\"\n", head("A")));
        write_mod(tmp.path(), "b", &format!("{}[class.\"XIII.Magnum\"]\nReloadCount = 7\n", head("B")));
        write_mod(tmp.path(), "c", &format!("{}[classes.Gun]\nparent = \"XIII.Magnum\"\n[class.\"XIII.Beretta\"]\nReloadCount = 1\n", head("C")));
        fs::write(tmp.path().join(DISABLED_FILE), "c\n").unwrap();
        // discover_all reads the mods turned off too.
        let all = discover_all(tmp.path()).mods;
        assert_eq!(all.len(), 3);
        assert_eq!(discover(tmp.path()).mods.len(), 2);
        let c = find_conflicts(&all);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!((c[0].first.as_str(), c[0].second.as_str(), c[0].what.as_str()), ("a", "b", "value XIII.Magnum.ReloadCount"));
        assert_eq!((c[1].first.as_str(), c[1].second.as_str(), c[1].what.as_str()), ("a", "c", "class Mod.Gun"));
        assert!(c[1].involves("C") && c[1].other("c") == "a");
        assert!(find_conflicts(&all[..1]).is_empty());
        assert!(find_conflicts(&[all[1].clone(), all[2].clone()]).is_empty());
    }

    #[test]
    fn missing_folder_gives_no_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let d = discover(&tmp.path().join("does-not-exist"));
        assert!(d.mods.is_empty());
        assert!(d.problems.is_empty());
    }

    #[test]
    fn empty_folder_gives_no_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let d = discover(tmp.path());
        assert!(d.mods.is_empty());
        assert!(d.problems.is_empty());
    }

    #[test]
    fn one_valid_mod() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "example", VALID);
        let d = discover(tmp.path());
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        assert_eq!(d.mods.len(), 1);
        let m = &d.mods[0];
        assert_eq!(m.folder_name, "example");
        assert_eq!(m.folder, tmp.path().join("example"));
        assert_eq!(m.info.name, "Example");
        assert_eq!(m.info.version, "1.0.0");
        assert_eq!(m.info.author.as_deref(), Some("Me"));
        assert_eq!(m.info.description.as_deref(), Some("Test mod"));
        assert_eq!(m.info.openxiii.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn optional_fields_may_be_absent() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "min", "[mod]\nname = \"Min\"\nversion = \"0.1\"\n");
        let d = discover(tmp.path());
        assert_eq!(d.mods.len(), 1);
        assert_eq!(d.mods[0].info.author, None);
        assert_eq!(d.mods[0].info.description, None);
        assert_eq!(d.mods[0].info.openxiii, None);
    }

    #[test]
    fn malformed_toml_is_skipped_with_line() {
        let tmp = tempfile::tempdir().unwrap();
        // Line 3 is broken (unterminated string).
        write_mod(tmp.path(), "broken", "[mod]\nname = \"Broken\"\nversion = \"1.0\n");
        write_mod(tmp.path(), "good", VALID);
        let d = discover(tmp.path());
        assert_eq!(d.mods.len(), 1);
        assert_eq!(d.mods[0].folder_name, "good");
        assert_eq!(d.problems.len(), 1);
        match &d.problems[0] {
            Problem::Error { file, line, .. } => {
                assert_eq!(file, &tmp.path().join("broken").join(MANIFEST));
                assert_eq!(*line, Some(3));
            }
            p => panic!("expected an error, got {p:?}"),
        }
        let text = d.problems[0].to_string();
        assert!(text.contains("line 3"), "{text}");
        assert!(text.contains("mod.toml"), "{text}");
    }

    #[test]
    fn missing_required_field_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "noversion", "[mod]\nname = \"X\"\n");
        let d = discover(tmp.path());
        assert!(d.mods.is_empty());
        assert_eq!(d.problems.len(), 1);
        let text = d.problems[0].to_string();
        assert!(text.contains("version"), "{text}");
    }

    #[test]
    fn folder_without_manifest_is_a_warning() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notamod")).unwrap();
        fs::write(tmp.path().join("loose-file.txt"), "x").unwrap();
        let d = discover(tmp.path());
        assert!(d.mods.is_empty());
        assert_eq!(d.problems.len(), 1);
        assert!(matches!(&d.problems[0], Problem::Warning { path, .. } if path == &tmp.path().join("notamod")));
    }

    #[test]
    fn mods_sorted_by_folder_name() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["zeta", "alpha", "Mid", "beta"] {
            write_mod(tmp.path(), f, &format!("[mod]\nname = \"{f}\"\nversion = \"1\"\n"));
        }
        let d = discover(tmp.path());
        let names: Vec<_> = d.mods.iter().map(|m| m.folder_name.as_str()).collect();
        assert_eq!(names, ["Mid", "alpha", "beta", "zeta"]);
    }

    #[test]
    fn class_tables_from_manifest_then_extra_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "w", "[mod]\nname = \"W\"\nversion = \"1\"\n\n[class.\"XIII.Beretta\"]\nReloadCount = 17\n");
        fs::write(tmp.path().join("w").join("b-ammo.toml"), "[class.\"XIII.c9mmAmmo\"]\nSRDamages = 35\n").unwrap();
        fs::write(tmp.path().join("w").join("a-more.toml"), "[class.\"XIII.Beretta\"]\nShotTime = 0.35\n").unwrap();
        fs::write(tmp.path().join("w").join("notes.txt"), "not read").unwrap();
        let d = discover(tmp.path());
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        let o: Vec<_> = d.mods[0].overrides.iter().map(|o| format!("{}.{}={}", o.class, o.property, o.value)).collect();
        assert_eq!(o, ["XIII.Beretta.ReloadCount=17", "XIII.Beretta.ShotTime=0.35", "XIII.c9mmAmmo.SRDamages=35"]);
    }

    /// The `.toml` files of a `maps` sub-folder are read after the others; `scripts` is not.
    #[test]
    fn maps_folder_is_read_and_scripts_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(
            tmp.path(),
            "w",
            "[mod]
name = \"W\"
version = \"1\"
",
        );
        let w = tmp.path().join("w");
        fs::create_dir_all(w.join("maps")).unwrap();
        fs::create_dir_all(w.join("scripts")).unwrap();
        fs::write(
            w.join("characters.toml"),
            "[character.Bodyguard]
look = \"Spads\"
",
        )
        .unwrap();
        fs::write(
            w.join("maps").join("Plage01.toml"),
            "[[map.Plage01.add]]
character = \"Bodyguard\"
position = [1, 2, 3]
",
        )
        .unwrap();
        fs::write(
            w.join("scripts").join("x.toml"),
            "[[map.Plage01.add]]
class = \"XIII.MedPick\"
position = [1, 2, 3]
",
        )
        .unwrap();
        let d = discover(tmp.path());
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        let m = &d.mods[0];
        assert_eq!(m.characters.len(), 1);
        assert_eq!(m.edits.len(), 1, "only the maps folder's entry: {:?}", m.edits);
        assert_eq!(m.edits[0].source.file, w.join("maps").join("Plage01.toml"));
        // Negative control: without the maps folder's file, no entry.
        fs::remove_file(w.join("maps").join("Plage01.toml")).unwrap();
        assert!(discover(tmp.path()).mods[0].edits.is_empty());
    }

    #[test]
    fn bad_value_keeps_the_mod() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "w", "[mod]\nname = \"W\"\nversion = \"1\"\n[class.\"XIII.Beretta\"]\nReloadCount = 17\n\"X[\" = 1\n");
        let d = discover(tmp.path());
        assert_eq!(d.mods.len(), 1);
        assert_eq!(d.mods[0].overrides.len(), 1);
        assert_eq!(d.problems.len(), 1);
        assert!(d.problems[0].to_string().contains("line 6"), "{}", d.problems[0]);
    }

    #[test]
    fn map_edits_from_several_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let head = "[mod]\nname = \"M\"\nversion = \"1\"\n";
        write_mod(tmp.path(), "a", &format!("{head}[[map.Plage01.add]]\nclass = \"XIII.FusilSnipePick\"\nposition = [1, 2, 3]\n[map.Plage01.replace]\n\"XIII.BerettaPick\" = \"XIII.MagnumPick\"\n"));
        write_mod(tmp.path(), "b", &format!("{head}[map.plage01.replace]\n\"XIII.BerettaPick\" = \"XIII.FusilChassePick\"\n[map.Bateau01.replace]\n\"XIII.BerettaPick\" = \"XIII.MagnumPick\"\n"));
        let d = discover(tmp.path());
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        let (edits, problems) = collect_edits(&d.mods);
        let kinds: Vec<String> = edits.iter().map(|e| format!("{} {}", e.map, e.describe())).collect();
        // Mod order, then file order (the maps of one file in name order).
        assert_eq!(kinds, ["Plage01 add XIII.FusilSnipePick at [1, 2, 3]", "Bateau01 replace XIII.BerettaPick with XIII.MagnumPick", "plage01 replace XIII.BerettaPick with XIII.FusilChassePick"]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().contains("replaces \"replace XIII.BerettaPick with XIII.MagnumPick\""), "{}", problems[0]);
    }

    #[test]
    fn decorations_bring_their_class() {
        let tmp = tempfile::tempdir().unwrap();
        let head = "[mod]\nname = \"M\"\nversion = \"1\"\n";
        write_mod(tmp.path(), "a", &format!("{head}[classes.Gun]\nparent = \"XIII.Beretta\"\n"));
        let d = discover(tmp.path());
        // No decoration: no class of its own, no values.
        assert_eq!(collect_new_classes(&d.mods).0.len(), 1);
        assert!(collect_overrides(&d.mods).0.is_empty());
        write_mod(
            tmp.path(),
            "b",
            &format!("{head}[[map.Plage01.add]]\nmodel = \"Mod.Crate\"\nposition = [1, 2, 3]\n[classes.ModelDecoration]\nparent = \"XIII.Beretta\"\n"),
        );
        let d = discover(tmp.path());
        // A mod may not take the decorations' class name.
        assert!(d.problems.iter().any(|p| p.to_string().contains("class of the decorations")), "{:?}", d.problems);
        let (classes, _) = collect_new_classes(&d.mods);
        let decoration = classes.iter().find(|c| c.class_name() == placement::DECORATION_CLASS).expect("decoration class");
        assert_eq!((decoration.parent.as_str(), decoration.source.mod_name.as_str()), (placement::DECORATION_PARENT, "b"));
        let (values, problems) = collect_overrides(&d.mods);
        assert!(problems.is_empty(), "{problems:?}");
        let v: Vec<String> = values.iter().map(|o| format!("{}.{}={}", o.class, o.property, o.value)).collect();
        assert_eq!(v, ["Mod.ModelDecoration.bStatic=false"]);
    }

    /// A mod's scripts are found in name order, listed in its summary, and part of its network
    /// fingerprint: editing one changes the mod's identity.
    #[test]
    fn scripts_found_and_fingerprinted() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "a", "[mod]
name = \"A\"
version = \"1\"
");
        let dir = tmp.path().join("a").join(scripts::FOLDER);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("2-scene.lua"), "log('two')").unwrap();
        fs::write(dir.join("1-setup.lua"), "log('one')").unwrap();
        let found = discover(tmp.path());
        let m = &found.mods[0];
        let labels: Vec<&str> = m.scripts.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["a/scripts/1-setup.lua", "a/scripts/2-scene.lua"]);
        assert!(log::summary(m).contains(&"runs script a/scripts/1-setup.lua".to_string()));
        let before = mod_ids(&found.mods);
        fs::write(dir.join("2-scene.lua"), "log('three')").unwrap();
        let after = mod_ids(&discover(tmp.path()).mods);
        assert_ne!(before[0].fingerprint, after[0].fingerprint);
    }

    #[test]
    fn same_mod_with_windows_or_unix_line_ends() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "a", "[mod]\r\nname = \"A\"\r\nversion = \"1\"\r\n");
        fs::write(tmp.path().join("a").join("x.wav"), "a\r\nb").unwrap();
        let windows = mod_ids(&discover(tmp.path()).mods);
        write_mod(tmp.path(), "a", "[mod]\nname = \"A\"\nversion = \"1\"\n");
        let unix = mod_ids(&discover(tmp.path()).mods);
        assert_eq!(windows, unix);
        // A sound's bytes stay as they are.
        fs::write(tmp.path().join("a").join("x.wav"), "a\nb").unwrap();
        assert_ne!(mod_ids(&discover(tmp.path()).mods), unix);
    }

    #[test]
    fn mod_files_found_whatever_their_case() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Sounds")).unwrap();
        fs::write(tmp.path().join("Sounds").join("Shot.wav"), "x").unwrap();
        let read = |rel: &str| fs::read_to_string(file_in_mod(tmp.path(), rel)).ok();
        for rel in ["Sounds/Shot.wav", "sounds\\shot.WAV", "./sounds/shot.wav", "..\\..\\sounds\\shot.wav"] {
            assert_eq!(read(rel).as_deref(), Some("x"), "{rel}");
        }
        // A missing file is kept as written, inside the folder.
        let missing = file_in_mod(tmp.path(), "../sounds/none.wav");
        assert!(missing.starts_with(tmp.path()) && missing.ends_with("none.wav"));
    }

    #[test]
    fn fingerprints_and_network_identity() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod(tmp.path(), "a", "[mod]\nname = \"A\"\nversion = \"1\"\n");
        write_mod(tmp.path(), "b", "[mod]\nname = \"B\"\nversion = \"2\"\n");
        fs::create_dir_all(tmp.path().join("b").join("sounds")).unwrap();
        fs::write(tmp.path().join("b").join("sounds").join("x.wav"), "abc").unwrap();
        let ids = mod_ids(&discover(tmp.path()).mods);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].fingerprint.len(), 16);
        // Same files: same identity.
        assert_eq!(mod_ids(&discover(tmp.path()).mods), ids);
        let guid = mods_guid(&ids);
        // A file of a sub-folder changes the mod's fingerprint and the network identity.
        fs::write(tmp.path().join("b").join("sounds").join("x.wav"), "abd").unwrap();
        let changed = mod_ids(&discover(tmp.path()).mods);
        assert_eq!(changed[0], ids[0]);
        assert_ne!(changed[1].fingerprint, ids[1].fingerprint);
        assert_ne!(mods_guid(&changed), guid);
        assert_eq!(mods_differences(&ids, &changed), ["mod b 2: its files differ"]);
        // Missing, extra, other version, other order.
        let mut v2 = ids.clone();
        v2[1].version = "3".into();
        assert_eq!(mods_differences(&ids, &v2), ["mod b: version 3 instead of 2"]);
        assert_eq!(mods_differences(&ids, &ids[..1]), ["missing mod b 2"]);
        assert_eq!(mods_differences(&ids[..1], &ids), ["extra mod b 2 (the server does not have it)"]);
        let swapped = vec![ids[1].clone(), ids[0].clone()];
        assert_eq!(mods_differences(&ids, &swapped), ["the mods are loaded in another order"]);
        assert_ne!(mods_guid(&swapped), guid);
        assert!(mods_differences(&ids, &ids).is_empty());
    }

    #[test]
    fn no_mods_switch() {
        assert!(disabled(&["game", "map", "--no-mods"]));
        if std::env::var(ENV_NO_MODS).is_err() {
            assert!(!disabled(&["game", "map"]));
        }
    }
}
