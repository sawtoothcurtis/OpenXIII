//! Mods build only (feature `mods`): the names catalog, one self-contained page
//! ("Modding catalog.html") that the game writes from the player's own copy of XIII, so that a
//! modder finds every name a mod can use without developer tools: the characters' looks, the
//! weapons, the objects and pickups, the decorations (static meshes), the maps with their
//! checkpoints and the characters they place, the sounds, the music and the dialogue lines.
//!
//! The page has a search box that filters every tab as one types, and each row carries the
//! exact text to paste into a mod with a copy button. It needs no network and no other file.
//! The full property tables stay in the reference pages of `oxiii mods-reference`.
//!
//! Everything is read from the game's files with mods left out (a mod's own classes and values
//! are not listed); nothing here runs unless the player asks for the catalog
//! (`--mods-catalog`, or the Mods page's row).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::mod_dialogue::DialogueLine;
use crate::mod_maps::MapEntry;
use crate::value::{ClassId, Value};
use crate::Vm;

/// File name of the catalog, written in the guide folder of the Mods folder (Mods\_Guide).
pub const FILE_NAME: &str = "Modding catalog.html";

/// The page, with `/*CATALOG_DATA*/` replaced by the data (JSON).
const TEMPLATE: &str = include_str!("mod_catalog.html");

/// A character model and who wears it.
#[derive(Debug, Clone, Default)]
pub struct LookRow {
    /// "XIIIPersos.JonesM".
    pub model: String,
    /// Friendly names (mod_names::character_looks), the first one suggested.
    pub names: Vec<String>,
    /// In-game names (PawnName) of the characters the maps place with this model.
    pub wearers: Vec<String>,
    /// Characters placed with this model, named or not.
    pub placed: usize,
    pub maps: Vec<String>,
}

/// A weapon class.
#[derive(Debug, Clone, Default)]
pub struct WeaponRow {
    pub class: String,
    /// `ItemName`, in the game's language.
    pub name: String,
    pub pickup: String,
    pub ammo: String,
}

/// A class a mod can place with `[[map.<Map>.add]]` `class = "…"`: a pickup or an object.
#[derive(Debug, Clone, Default)]
pub struct ObjectRow {
    pub class: String,
    /// The name of what it gives (`ItemName` of its `InventoryType`), if any.
    pub name: String,
    /// "weapon pickup", "ammo pickup", "pickup" or "object".
    pub kind: &'static str,
    /// The class it gives (`InventoryType`), if any.
    pub gives: String,
}

/// Everything the page shows.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Language of the texts (game's localization extension: "int", "frt"…).
    pub language: String,
    pub looks: Vec<LookRow>,
    pub weapons: Vec<WeaponRow>,
    pub objects: Vec<ObjectRow>,
    /// Static meshes ("Package.Group.Name").
    pub decorations: Vec<String>,
    /// Maps: solo maps in the order of the story, then multiplayer, then technical ones.
    pub maps: Vec<(MapEntry, &'static str)>,
    /// Sounds ("Package.Group.Name"), music left out.
    pub sounds: Vec<String>,
    /// The music of the levels (XIIISound's groups whose name starts with "Music").
    pub music: Vec<String>,
    pub dialogues: Vec<DialogueLine>,
    /// Ammo classes: (class, name in the game's language).
    pub ammo: Vec<(String, String)>,
    /// Animation sets: (set, classes that use it, its animations).
    pub animations: Vec<(String, Vec<String>, Vec<String>)>,
}

/// What `write_catalog` wrote.
#[derive(Debug, Clone)]
pub struct Written {
    pub file: PathBuf,
    pub bytes: usize,
    pub seconds: f32,
}

/// Where the catalog goes by default: the guide folder of the Mods folder (Mods\_Guide, made
/// when needed), on any computer. The lists of names go in its `names` folder.
pub fn default_file() -> Option<PathBuf> {
    Some(openxiii_mods::modding_folder(&openxiii_mods::mods_folder()?).join(FILE_NAME))
}

/// Version of the catalog's content: raised when what it lists changes.
const CATALOG_FORMAT: u32 = 3;

/// The stamp written in the catalog: this OpenXIII (its version, the format, the date of its
/// executable) and the game folder it was made from (a hash, not the path). A catalog with
/// another stamp is made again.
pub fn catalog_stamp(game: &Path) -> String {
    let exe = std::env::current_exe().ok().and_then(|e| std::fs::metadata(e).ok()).and_then(|m| m.modified().ok());
    let exe = exe.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let path = game.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    format!("openxiii {} format {CATALOG_FORMAT} exe {exe:x} game {h:016x}", env!("CARGO_PKG_VERSION"))
}

/// The stamp a catalog file holds, if any.
pub fn stamp_of(file: &Path) -> Option<String> {
    let text = std::fs::read(file).ok()?;
    let head = String::from_utf8_lossy(&text[..text.len().min(4096)]).into_owned();
    let start = head.find(STAMP_START)? + STAMP_START.len();
    let end = head[start..].find('"')? + start;
    Some(head[start..end].to_string())
}

const STAMP_START: &str = "<meta name=\"openxiii-catalog\" content=\"";

/// True when `file` is a catalog of this OpenXIII and this game folder, with its lists.
pub fn catalog_is_current(file: &Path, game: &Path) -> bool {
    stamp_of(file).is_some_and(|s| s == catalog_stamp(game)) && file.parent().is_some_and(|d| d.join(NAMES_FOLDER).join("README.md").is_file())
}

/// Folder of the lists of names, next to the catalog.
pub const NAMES_FOLDER: &str = "names";

/// A catalog being written (one at a time: the Mods page's row waits for the startup's).
static WRITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True while a catalog is being written.
pub fn is_writing() -> bool {
    WRITING.load(Ordering::Relaxed)
}

/// At startup (the game a player starts, on a computer): writes the catalog and the lists in
/// the guide folder (Mods\_Guide) on a thread of its own when they are missing or out of date. None when
/// nothing is to be done.
pub fn refresh_in_background(game: PathBuf) -> Option<std::thread::JoinHandle<()>> {
    let file = default_file()?;
    if catalog_is_current(&file, &game) || is_writing() {
        return None;
    }
    std::thread::Builder::new()
        .name("names catalog".into())
        .spawn(move || match Vm::write_catalog(&game, &file, &|_| {}) {
            Ok(w) => openxiii_mods::log::note(&format!("names catalog written: {} ({} KB in {:.1} s)", w.file.display(), w.bytes / 1024, w.seconds)),
            Err(e) => openxiii_mods::log::note(&format!("warning: names catalog: {e}")),
        })
        .ok()
}

/// A Vm on the game's own content: the mods loaded by the running game are left out.
pub(crate) fn game_only_vm() -> Vm {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.mods.defaults = None;
    vm.mods.edits = None;
    vm.mods.behaviour = None;
    vm.mods.loads = None;
    vm.mods.new_classes = None;
    vm
}

/// Is a music sound: XIIISound's groups "Music" and "Music__<Level>".
fn is_music(sound: &str) -> bool {
    let mut parts = sound.split('.');
    let package = parts.next().unwrap_or_default();
    let group = parts.next().unwrap_or_default();
    package.eq_ignore_ascii_case("XIIISound") && group.to_ascii_lowercase().starts_with("music") && parts.next().is_some()
}

/// The `.unr` maps of `game`, sorted.
pub(crate) fn map_names(game: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = std::fs::read_dir(game.join("Maps"))
        .map_err(|e| format!("{}: {e}", game.join("Maps").display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.path().file_name().map(|n| n.to_string_lossy().to_string()))
        .filter_map(|n| n.strip_suffix(".unr").or_else(|| n.strip_suffix(".UNR")).map(str::to_string))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

/// Reads every map of `names` (mod_maps::read_map_entry) on up to `threads` threads, each map
/// in a Vm of its own. Calls `progress(done, total)` after each map.
pub(crate) fn read_maps(game: &Path, names: &[String], threads: usize, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<MapEntry> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<MapEntry>>> = Mutex::new(vec![None; names.len()]);
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, names.len().max(1)) {
            s.spawn(|| {
                // One Vm per thread: the System folder is loaded once, and each map replaces
                // the previous one (Vm::unload_level, as a travel). A Vm that failed is replaced.
                let mut vm: Option<Vm> = None;
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(n) = names.get(i) else { break };
                    if vm.is_none() {
                        let mut v = game_only_vm();
                        if v.load_dir(&game.join("System")).is_ok() {
                            vm = Some(v);
                        }
                    }
                    let read = match vm.as_mut() {
                        Some(v) => v.read_map_here(game, n),
                        None => Err("the System folder could not be read".to_string()),
                    };
                    let e = read.unwrap_or_else(|why| {
                        vm = None;
                        MapEntry { name: n.clone(), title: format!("(not read: {why})"), ..Default::default() }
                    });
                    out.lock().unwrap_or_else(|p| p.into_inner())[i] = Some(e);
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, names.len());
                }
            });
        }
    });
    out.into_inner().unwrap_or_else(|p| p.into_inner()).into_iter().flatten().collect()
}

/// Threads for reading the maps: half the processor's threads, 1 to 4 (each map takes a Vm of
/// its own, a few hundred MB at most).
fn map_threads() -> usize {
    // Measured on a 16-thread PC with one Vm per thread (64 maps): 1 thread 13.7 s, 4 threads
    // 6.0 s, 8 threads 10.8 s, 16 threads 59.5 s: past 4 the threads slow each other down.
    std::thread::available_parallelism().map_or(1, |n| n.get() / 2).clamp(1, 4)
}

impl Vm {
    /// The folder of the game this Vm loaded (the parent of its System folder).
    pub fn game_folder(&self) -> Option<PathBuf> {
        self.system_dir.as_ref().and_then(|d| d.parent()).map(Path::to_path_buf)
    }

    /// Exports of class `kind` in package `package` ("Package.Group.Name"), if it loads.
    fn catalog_exports(&mut self, package: &str, kind: &str) -> Vec<String> {
        let Some(lib) = self.library.as_mut() else { return Vec::new() };
        let Some(p) = lib.get(package) else { return Vec::new() };
        (0..p.package.exports.len())
            .filter(|&i| p.package.export_class(i) == kind)
            .map(|i| format!("{}.{}", p.name, p.package.object_path(openxiii_package::ObjectRef::Export(i))))
            .collect()
    }

    /// Classes a mod can create: not abstract, not bStatic or bNoDelete (ULevel::SpawnActor's
    /// refusals), subclasses of `root` (included).
    fn catalog_classes(&mut self, root: &str) -> Vec<ClassId> {
        let Some(root) = self.class(root) else { return Vec::new() };
        let mut out = Vec::new();
        for c in 0..self.classes.len() {
            if !self.is_child_of(c, root) || self.classes[c].flags & 1 != 0 {
                continue;
            }
            self.create_cdo(c);
            let cdo = self.classes[c].cdo;
            if self.propr(cdo, "bStatic").as_bool() || self.propr(cdo, "bNoDelete").as_bool() {
                continue;
            }
            out.push(c);
        }
        out.sort_by_key(|&c| self.class_path(c).to_lowercase());
        out
    }

    fn catalog_text(&self, obj: crate::value::ObjId, prop: &str) -> String {
        match self.prop(obj, prop) {
            Value::Str(s) => s.trim().to_string(),
            _ => String::new(),
        }
    }

    fn catalog_class_name(&self, v: Value) -> String {
        match v {
            Value::Class(Some(k)) => self.class_path(k),
            _ => String::new(),
        }
    }

    /// `ItemName` of the default object of class `k`.
    fn catalog_item_name(&mut self, k: Option<ClassId>) -> String {
        let Some(k) = k else { return String::new() };
        self.create_cdo(k);
        self.catalog_text(self.classes[k].cdo, "ItemName")
    }

    /// The weapons, objects, decorations, sounds and dialogues (no map loaded).
    pub(crate) fn catalog_from_packages(&mut self, c: &mut Catalog) -> Result<(), String> {
        c.language = self.config.as_ref().map_or("int", |k| k.language.as_str()).to_string();
        let game = self.system_dir.as_ref().and_then(|d| d.parent()).map(Path::to_path_buf).ok_or("no game folder")?;
        // Weapons.
        for k in self.catalog_classes("XIII.XIIIWeapon") {
            let cdo = self.classes[k].cdo;
            let row = WeaponRow {
                class: self.class_path(k),
                name: self.catalog_text(cdo, "ItemName"),
                pickup: self.catalog_class_name(self.prop(cdo, "PickupClass")),
                ammo: self.catalog_class_name(self.prop(cdo, "AmmoName")),
            };
            c.weapons.push(row);
        }
        // Ammo.
        for k in self.catalog_classes("Engine.Ammunition") {
            let cdo = self.classes[k].cdo;
            let row = (self.class_path(k), self.catalog_text(cdo, "ItemName"));
            c.ammo.push(row);
        }
        // Pickups, then the other objects.
        let weapon = self.class("XIII.XIIIWeapon");
        let ammo = self.class("Engine.Ammunition");
        for k in self.catalog_classes("Engine.Pickup") {
            let cdo = self.classes[k].cdo;
            let given = self.prop(cdo, "InventoryType").as_class();
            let kind = match given {
                Some(g) if weapon.is_some_and(|w| self.is_child_of(g, w)) => "weapon pickup",
                Some(g) if ammo.is_some_and(|a| self.is_child_of(g, a)) => "ammo pickup",
                _ => "pickup",
            };
            let name = self.catalog_item_name(given);
            let gives = given.map(|g| self.class_path(g)).unwrap_or_default();
            c.objects.push(ObjectRow { class: self.class_path(k), name, kind, gives });
        }
        // A weapon without a PickupClass of its own: the pickup that gives it.
        for w in &mut c.weapons {
            if w.pickup.is_empty() {
                if let Some(o) = c.objects.iter().find(|o| o.kind == "weapon pickup" && o.gives.eq_ignore_ascii_case(&w.class)) {
                    w.pickup = o.class.clone();
                }
            }
        }
        for k in self.catalog_classes("Engine.Decoration") {
            c.objects.push(ObjectRow { class: self.class_path(k), name: String::new(), kind: "object", gives: String::new() });
        }
        // Decorations: the static meshes of the StaticMeshes folder.
        let files = |dir: &str, ext: &str| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(game.join(dir))
                .map(|it| {
                    it.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
                        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by_key(|s| s.to_lowercase());
            v
        };
        for pkg in files("StaticMeshes", "usx") {
            let list = self.catalog_exports(&pkg, "StaticMesh");
            c.decorations.extend(list);
        }
        // Sounds and music.
        for pkg in files("Sounds", "uax") {
            for s in self.catalog_exports(&pkg, "Sound") {
                if is_music(&s) {
                    c.music.push(s);
                } else {
                    c.sounds.push(s);
                }
            }
        }
        // Dialogues, in the game's language.
        let lang = c.language.clone();
        c.dialogues = self.dialogue_lines(&lang)?;
        c.animations = self.catalog_animations();
        Ok(())
    }

    /// The animation sets of the weapons seen in first person and of the characters: (set, the
    /// classes that use it, its animations), as animations.md lists them.
    fn catalog_animations(&mut self) -> Vec<(String, Vec<String>, Vec<String>)> {
        let mut sets: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for root in ["XIII.XIIIWeapon", "Engine.Pawn"] {
            let Some(root) = self.class(root) else { continue };
            let classes: Vec<ClassId> = (0..self.classes.len())
                .filter(|&c| c == root || self.is_child_of(c, root))
                .filter(|&c| !self.packages[self.classes[c].package].name.eq_ignore_ascii_case("XIIIMP"))
                .collect();
            for c in classes {
                self.create_cdo(c);
                let cdo = self.classes[c].cdo;
                // A weapon names its first-person mesh in MeshName; other classes give their Mesh.
                let mesh = match (self.prop(cdo, "MeshName"), self.prop(cdo, "Mesh")) {
                    (Value::Str(m), _) if !m.is_empty() => m,
                    (_, Value::Asset(m)) => m,
                    _ => continue,
                };
                let Some(anim) = self.mesh_info(&mesh).and_then(|i| i.default_anim.clone()) else { continue };
                let path = self.class_path(c);
                let users = sets.entry(anim).or_default();
                if !users.contains(&path) {
                    users.push(path);
                }
            }
        }
        let mut out = Vec::new();
        for (anim, mut users) in sets {
            let Some(set) = self.anim_set(&anim) else { continue };
            users.sort_by_key(|u| u.to_lowercase());
            let seqs = set.seqs.iter().map(|s| s.name.clone()).collect();
            out.push((anim, users, seqs));
        }
        out
    }

    /// The catalog of the game in `game`. `progress` gets short English lines ("maps 12/64").
    pub fn build_catalog(game: &Path, progress: &(dyn Fn(&str) + Sync)) -> Result<Catalog, String> {
        let mut c = Catalog::default();
        let mut vm = game_only_vm();
        vm.load_dir(&openxiii_package::system_dir(game)).map_err(|e| e.to_string())?;
        progress("classes and packages");
        vm.catalog_from_packages(&mut c)?;
        // The character models: XIIIPersos's animated models, and those the friendly names use.
        let mut models = vm.catalog_exports("XIIIPersos", "SkeletalMesh");
        for (m, _) in crate::mod_names::character_looks() {
            if !models.iter().any(|x| x.eq_ignore_ascii_case(m)) {
                models.push(m.to_string());
            }
        }
        drop(vm);
        // The maps, each in a Vm of its own.
        let names = map_names(game)?;
        let entries = read_maps(game, &names, map_threads(), &|done, total| progress(&format!("maps {done}/{total}")));
        let (story, multi, technical) = crate::mod_maps::map_groups(&entries);
        for (list, kind) in [(&story, "solo"), (&multi, "multiplayer"), (&technical, "technical")] {
            for &i in list.iter() {
                c.maps.push((entries[i].clone(), kind));
            }
        }
        // Who wears each model.
        models.sort_by_key(|m| m.to_lowercase());
        for m in models {
            let names = crate::mod_names::character_looks()
                .iter()
                .find(|(x, _)| x.eq_ignore_ascii_case(&m))
                .map(|(_, n)| n.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            let mut row = LookRow { model: m, names, ..Default::default() };
            for (e, _) in &c.maps {
                for p in e.characters.iter().filter(|p| p.model.eq_ignore_ascii_case(&row.model)) {
                    row.placed += 1;
                    let n = &p.pawn_name;
                    // "Someone" is XIIIPawn's default PawnName: a character without a name.
                    if !n.is_empty()
                        && !n.eq_ignore_ascii_case("Someone")
                        && !n.eq_ignore_ascii_case("None")
                        && !row.wearers.iter().any(|w| w.eq_ignore_ascii_case(n))
                    {
                        row.wearers.push(n.clone());
                    }
                    if !row.maps.contains(&e.name) {
                        row.maps.push(e.name.clone());
                    }
                }
            }
            c.looks.push(row);
        }
        Ok(c)
    }

    /// Builds the catalog of the game in `game` and writes it to `file` (one HTML page).
    pub fn write_catalog(game: &Path, file: &Path, progress: &(dyn Fn(&str) + Sync)) -> Result<Written, String> {
        if WRITING.swap(true, Ordering::AcqRel) {
            return Err("the names catalog is already being written".into());
        }
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                WRITING.store(false, Ordering::Release);
            }
        }
        let _done = Done;
        let start = std::time::Instant::now();
        let c = Vm::build_catalog(game, progress)?;
        let html = catalog_html(&c).replacen("/*STAMP*/", &catalog_stamp(game), 1);
        let dir = file.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_default();
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        // The lists first, the catalog last: its stamp says that everything is there.
        let names = dir.join(NAMES_FOLDER);
        std::fs::create_dir_all(&names).map_err(|e| format!("{}: {e}", names.display()))?;
        for (page, text) in crate::mod_names_doc::name_pages(&c) {
            std::fs::write(names.join(page), text).map_err(|e| format!("{}: {e}", names.join(page).display()))?;
        }
        std::fs::write(file, &html).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(Written { file: file.to_path_buf(), bytes: html.len(), seconds: start.elapsed().as_secs_f32() })
    }
}

/// A JSON string.
fn js(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Never "</script>" or "<!--" inside the page's script.
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON array of rows, each an array of strings.
fn rows<T>(items: &[T], row: impl Fn(&T) -> Vec<String>) -> String {
    let lines: Vec<String> = items.iter().map(|it| format!("[{}]", row(it).iter().map(|s| js(s)).collect::<Vec<_>>().join(","))).collect();
    format!("[\n{}\n]", lines.join(",\n"))
}

fn position(p: [f32; 3]) -> String {
    format!("[{:.0}, {:.0}, {:.0}]", p[0], p[1], p[2])
}

/// The page for catalog `c`. Rows: `[text to paste, columns…]`; the page knows each tab's columns.
pub fn catalog_html(c: &Catalog) -> String {
    let looks = rows(&c.looks, |l| {
        let paste = format!("look = \"{}\"", l.names.first().cloned().unwrap_or_else(|| l.model.clone()));
        let worn = if l.placed == 0 {
            "not placed in any map".to_string()
        } else if l.wearers.is_empty() {
            format!("{} unnamed", l.placed)
        } else {
            l.wearers.join(", ")
        };
        let who = openxiii_mods::looks::look_info(&l.model).map_or(String::new(), |(w, _)| w.to_string());
        vec![paste, l.names.join(", "), who, l.model.clone(), worn, l.maps.join(", ")]
    });
    // The simple name when the weapon has one (simple.rs), else the class.
    let weapons = rows(&c.weapons, |w| {
        let simple = openxiii_mods::simple::weapon_name(&w.class).unwrap_or(&w.class);
        vec![format!("weapon = \"{simple}\""), w.name.clone(), w.class.clone(), w.pickup.clone(), w.ammo.clone()]
    });
    let objects = rows(&c.objects, |o| vec![format!("class = \"{}\"", o.class), o.name.clone(), o.class.clone(), o.kind.to_string()]);
    let decorations = rows(&c.decorations, |d| vec![format!("model = \"{d}\""), d.clone()]);
    let sounds = rows(&c.sounds, |s| vec![format!("\"{s}\""), s.clone()]);
    let music = rows(&c.music, |s| vec![format!("\"{s}\""), s.clone()]);
    let dialogues = rows(&c.dialogues, |d| {
        vec![
            format!("line = \"{}\"", d.name),
            d.name.clone(),
            d.speaker.clone(),
            d.subtitle.clone(),
            if d.voice { "voice".into() } else { "subtitle only".into() },
        ]
    });
    // Scripts: the toml's reactions and the scripts' events and functions (mod_names_doc.rs).
    let scripts = rows(&crate::mod_names_doc::script_rows(), |r| r.to_vec());
    // Maps: [name, title, kind, checkpoints [[n, description]], characters [[paste, who, actor, class, PawnName, Tag, model, health, position]]].
    let maps: Vec<String> = c
        .maps
        .iter()
        .map(|(e, kind)| {
            let saves: Vec<String> = e.saves.iter().enumerate().map(|(i, s)| format!("[{},{}]", i + 1, js(&s.description))).collect();
            let people: Vec<String> = e
                .characters
                .iter()
                .map(|p| {
                    let paste = format!("[[map.{}.change]]\nwho = \"{}\"", e.name, p.who);
                    let health = p.health.map(|h| h.to_string()).unwrap_or_default();
                    let cells = [
                        paste,
                        p.who.clone(),
                        p.name.clone(),
                        p.class.clone(),
                        p.pawn_name.clone(),
                        p.tag.clone(),
                        p.model.clone(),
                        health,
                        position(p.position),
                    ];
                    format!("[{}]", cells.iter().map(|s| js(s)).collect::<Vec<_>>().join(","))
                })
                .collect();
            format!("[{},{},{},[{}],[\n{}\n]]", js(&e.name), js(&e.title), js(kind), saves.join(","), people.join(",\n"))
        })
        .collect();
    let data = format!(
        "{{\"language\":{},\"looks\":{looks},\n\"weapons\":{weapons},\n\"objects\":{objects},\n\"decorations\":{decorations},\n\"maps\":[\n{}\n],\n\"sounds\":{sounds},\n\"music\":{music},\n\"dialogues\":{dialogues},\n\"scripts\":{scripts}}}",
        js(&c.language),
        maps.join(",\n")
    );
    TEMPLATE.replacen("/*CATALOG_DATA*/", &data, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_strings_are_safe_in_a_script() {
        assert_eq!(js("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        assert!(!js("</script><!--").contains('<'));
        assert!(is_music("XIIISound.Music__Banque01.Banque01__hAttaque") && is_music("XIIISound.Music.Banque01"));
        assert!(!is_music("XIIISound.Weapons.Music") && !is_music("Plage01Voices.Music.x") && !is_music("XIIISound.Music"));
    }
}

/// True when `dir` is inside the docs of a source repository (an ancestor named "docs" below an
/// ancestor that holds ".git"): the lists made from the game's files are not written there
/// unless asked for explicitly (`--into-repository`), so that they never end up in the
/// repository by mistake.
pub fn inside_repository_docs(dir: &Path) -> bool {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut docs = false;
    for a in dir.ancestors() {
        if a.file_name().is_some_and(|n| n.eq_ignore_ascii_case("docs")) {
            docs = true;
        }
        if docs && a.join(".git").exists() {
            return true;
        }
    }
    false
}
