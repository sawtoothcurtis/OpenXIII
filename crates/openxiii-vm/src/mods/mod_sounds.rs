//! Mods build only (feature `mods`): sounds and content from any level (docs/modding/README.md).
//!
//! The game loads, for each map, the shared sound bank `Fix.hxc` and the map's own bank
//! (`Sounds/PC/<level title>.hxc`); a sound whose resource is in another map's bank plays
//! nothing. When a level's sounds are loaded (end of `audio_load_sounds`), the mods build also
//! loads, once for the level:
//! - the banks listed by `load_sound_banks` (every map, or this map);
//! - for each sound named in a mod value (class, map addition, character, action), the smallest
//!   bank that holds its resource, if no loaded bank has it;
//! - for each character voice a mod asks for (`CodeMesh` values), the banks that hold the
//!   sounds of that voice in the onomatopoeia sets of the `Onomatopees` package (pain, death,
//!   shouts…).
//!
//! These banks are unloaded with the map's own bank (`audio_clean_up`). Which bank holds which
//! resource comes from the banks' indexes (a small block at the end of each file), read once.
//!
//! Textures, meshes, sounds and other non-script objects named in a mod value are resolved in
//! the game's packages (`mod_asset_value`), which loads their package when needed, as the
//! game's DynamicLoadObject does.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use openxiii_assets::hx::Cuuid;
use openxiii_mods::content::LoadList;
use openxiii_mods::ModValue;

use crate::value::{ObjId, Value};
use crate::Vm;

/// Which banks hold which resources.
#[derive(Debug, Default)]
pub struct BankIndex {
    /// (bank name, file size in bytes), sorted by name.
    pub banks: Vec<(String, u64)>,
    /// Resource -> banks (indexes in `banks`) that hold it.
    pub by_id: HashMap<Cuuid, Vec<usize>>,
    /// Resource -> (kind, resources it uses: the waves of a sound, the sounds of a program).
    pub entries: HashMap<Cuuid, (String, Vec<Cuuid>)>,
}

impl BankIndex {
    /// The waves (`WaveFileIdObj`) a resource can play, through the resources it uses.
    pub fn waves_of(&self, id: Cuuid) -> Vec<Cuuid> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut todo = vec![id];
        while let Some(i) = todo.pop() {
            if !seen.insert(i) {
                continue;
            }
            match self.entries.get(&i) {
                Some((kind, _)) if kind.ends_with("WaveFileIdObj") => out.push(i),
                Some((_, links)) => todo.extend(links.iter().copied()),
                None => {}
            }
        }
        out.sort();
        out
    }

    /// The smallest bank holding resource `id`.
    pub fn smallest_with(&self, id: Cuuid) -> Option<&str> {
        let list = self.by_id.get(&id)?;
        list.iter().min_by_key(|&&i| (self.banks[i].1, self.banks[i].0.to_lowercase())).map(|&i| self.banks[i].0.as_str())
    }
}

/// Resources listed in the index of bank `path` (the u32 at the start gives its offset): id,
/// kind, and the resources it uses (links, and the waves of its language variants).
fn read_bank_ids(path: &Path) -> Option<Vec<(Cuuid, String, Vec<Cuuid>)>> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 4];
    f.read_exact(&mut head).ok()?;
    f.seek(SeekFrom::Start(u32::from_le_bytes(head) as u64)).ok()?;
    let mut data = Vec::new();
    f.read_to_end(&mut data).ok()?;
    let mut p = 0usize;
    let u32_at = |p: &mut usize| -> Option<u32> {
        let v = u32::from_le_bytes(data.get(*p..*p + 4)?.try_into().ok()?);
        *p += 4;
        Some(v)
    };
    if data.get(0..4)? != b"INDX" {
        return None;
    }
    p += 4;
    let index_type = u32_at(&mut p)?;
    let count = u32_at(&mut p)?;
    if index_type == 1 {
        return None;
    }
    let mut ids = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = u32_at(&mut p)? as usize;
        let kind: String = data.get(p..p + len)?.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        p += len;
        let id = Cuuid(u32_at(&mut p)?, u32_at(&mut p)?);
        p += 12; // offset, size, unknown
        let mut uses = Vec::new();
        for _ in 0..u32_at(&mut p)? {
            uses.push(Cuuid(u32_at(&mut p)?, u32_at(&mut p)?));
        }
        // Language variants: code, ?, wave.
        for _ in 0..u32_at(&mut p)? {
            let (_, _, a, b) = (u32_at(&mut p)?, u32_at(&mut p)?, u32_at(&mut p)?, u32_at(&mut p)?);
            uses.push(Cuuid(a, b));
        }
        ids.push((id, kind, uses));
    }
    Some(ids)
}

static INDEX: Mutex<Option<(PathBuf, Arc<BankIndex>)>> = Mutex::new(None);

/// Index of the banks of folder `dir` (Sounds/PC), read once per folder.
pub fn bank_index(dir: &Path) -> Arc<BankIndex> {
    let mut g = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((d, i)) = g.as_ref() {
        if d == dir {
            return i.clone();
        }
    }
    let mut files: Vec<(String, PathBuf, u64)> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("hxc")))
                .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p.clone(), std::fs::metadata(&p).ok()?.len())))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|f| f.0.to_lowercase());
    let mut index = BankIndex::default();
    for (name, path, size) in files {
        let i = index.banks.len();
        index.banks.push((name, size));
        for (id, kind, uses) in read_bank_ids(&path).unwrap_or_default() {
            let list = index.by_id.entry(id).or_default();
            if !list.contains(&i) {
                list.push(i);
            }
            index.entries.entry(id).or_insert((kind, uses));
        }
    }
    let index = Arc::new(index);
    *g = Some((dir.to_path_buf(), index.clone()));
    index
}

static GLOBAL: OnceLock<Arc<Vec<LoadList>>> = OnceLock::new();

/// Sets the load lists used by every Vm created afterwards (once, at startup).
pub fn set_mod_loads(list: Vec<LoadList>) {
    let _ = GLOBAL.set(Arc::new(list));
}

pub(crate) fn global_mod_loads() -> Option<Arc<Vec<LoadList>>> {
    GLOBAL.get().cloned().filter(|l| !l.is_empty())
}

/// "Sound'Package.Group.Name'" or "Package.Group.Name" -> "Package.Group.Name".
pub(crate) fn object_path(text: &str) -> &str {
    let t = text.trim();
    match (t.find('\''), t.ends_with('\'')) {
        (Some(i), true) if i + 1 < t.len() - 1 => &t[i + 1..t.len() - 1],
        _ => t,
    }
}

/// Opening paragraph of the generated references.
const GENERATED: &str = "Generated by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`)\n\
from the original game's files.\n\n";

/// One bank loaded for the mods: (name, load time in ms, file size in bytes).
pub type BankLoad = (String, f64, u64);

impl Vm {
    /// Load lists of this Vm only (tests).
    pub fn set_mod_loads_here(&mut self, list: Vec<LoadList>) {
        self.mods.loads = Some(Arc::new(list)).filter(|l| !l.is_empty());
    }

    /// Sounds/PC folder of the game, from where the library finds the XIIISound package.
    pub(crate) fn mod_bank_dir(&self) -> Option<PathBuf> {
        let file = self.library.as_ref()?.file_of("XIIISound")?;
        Some(file.parent()?.join("PC"))
    }

    /// Resource of the game already loaded in the audio engine.
    fn mod_resource_loaded(&self, id: Cuuid) -> bool {
        self.audio.as_ref().is_some_and(|h| h.engine.registry.contains_key(&id))
    }

    /// Loads sound bank `name` for the mods (once per level). True if it is loaded now.
    pub(crate) fn mod_load_bank(&mut self, name: &str, why: &str) -> bool {
        if self.mod_banks.iter().any(|(b, _, _)| b.eq_ignore_ascii_case(name)) {
            return true;
        }
        let Some(dir) = self.mod_bank_dir() else { return false };
        let path = dir.join(format!("{name}.hxc"));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let Some(game) = dir.parent().and_then(|d| d.parent()).map(Path::to_path_buf) else { return false };
        let Some(hx) = self.audio.as_mut() else { return false };
        let t = Instant::now();
        let loaded = hx.engine.load_banks(&game, &[name]);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if loaded.is_empty() {
            self.mod_report(format!("warning: sound bank {name} could not be loaded ({why})"));
            return false;
        }
        self.mod_report(format!("sound bank {name} loaded for {why} ({} KB, {ms:.1} ms)", size / 1024));
        self.mod_banks.push((name.to_string(), ms, size));
        true
    }

    /// Makes sound `path` playable: its resource, if no loaded bank has it, comes with the
    /// smallest bank that holds it. Err: why not.
    pub(crate) fn mod_ensure_sound(&mut self, path: &str, why: &str) -> Result<(), String> {
        let path = object_path(path).to_string();
        let Some(s) = self.audio_sound_ref(&path) else {
            return Err(format!("{path} is not a sound of the game's packages"));
        };
        if self.audio.is_none() || self.mod_resource_loaded(s.cuuid) {
            return Ok(());
        }
        let Some(dir) = self.mod_bank_dir() else { return Ok(()) };
        let index = bank_index(&dir);
        let Some(bank) = index.smallest_with(s.cuuid).map(str::to_string) else {
            return Err(format!("no sound bank holds {path}"));
        };
        if self.mod_load_bank(&bank, why) {
            Ok(())
        } else {
            Err(format!("sound bank {bank} of {path} could not be loaded"))
        }
    }

    /// Sounds of voice `code_mesh` in the onomatopoeia sets (PNJOno, DeathOno) of the
    /// Onomatopees package.
    pub(crate) fn mod_voice_sounds(&mut self, code_mesh: i32) -> Vec<(String, Cuuid)> {
        let onos: Vec<String> = {
            let Some(lib) = self.library.as_mut() else { return Vec::new() };
            let Some(p) = lib.get("Onomatopees") else { return Vec::new() };
            (0..p.package.exports.len())
                .filter(|&i| matches!(p.package.export_class(i), "PNJOno" | "DeathOno"))
                .map(|i| format!("{}.{}", p.name, p.package.object_path(openxiii_package::ObjectRef::Export(i))))
                .collect()
        };
        let mut out = Vec::new();
        for ono in onos {
            let Some(set) = self.audio_ono(&ono) else { continue };
            if let Some(t) = set.timbres.get(code_mesh.max(0) as usize) {
                for s in t.iter().flatten() {
                    if !out.iter().any(|(_, id)| *id == s.cuuid) {
                        out.push((s.path.clone(), s.cuuid));
                    }
                }
            }
        }
        out
    }

    /// Sounds named in the mods' values, and the voices (CodeMesh) they ask for.
    fn mod_named_sounds(&mut self) -> (Vec<String>, Vec<i32>) {
        let mut texts: Vec<String> = Vec::new();
        let mut voices: Vec<i32> = Vec::new();
        fn walk(v: &ModValue, texts: &mut Vec<String>) {
            match v {
                ModValue::Str(s) => texts.push(s.clone()),
                ModValue::Array(items) => items.iter().for_each(|i| walk(i, texts)),
                ModValue::Table(fields) => fields.iter().for_each(|(_, i)| walk(i, texts)),
                _ => {}
            }
        }
        let mut voice = |prop: &str, v: &ModValue| {
            if prop.eq_ignore_ascii_case("CodeMesh") {
                if let ModValue::Int(i) = v {
                    voices.push(*i as i32);
                }
            }
        };
        if let Some(t) = self.mods.defaults.clone() {
            let map = self.mod_map_name().to_lowercase();
            let lists = t.classes_all().chain(t.maps_of(&map));
            for o in lists {
                walk(&o.value, &mut texts);
                voice(&o.property, &o.value);
            }
        }
        if self.mods.edits.is_some() {
            for e in self.mod_edits_here().iter() {
                if let openxiii_mods::Edit::Add { properties, .. } = &e.edit {
                    for p in properties {
                        walk(&p.value, &mut texts);
                        voice(&p.property, &p.value);
                    }
                }
            }
        }
        if let Some(b) = self.mods.behaviour.clone() {
            // The characters this map may show: placed on it, made by a shortcut (any map) or
            // by a change. One only a script creates gets its sounds when it is created
            // (mod_script_character_sounds), not on every map.
            let mut used: Vec<String> = b.actions.iter().filter_map(|a| if let openxiii_mods::behaviour::ActionKind::Spawn { what, .. } = &a.kind { Some(what.clone()) } else { None }).collect();
            if self.mods.edits.is_some() {
                for e in self.mod_edits_here() {
                    match &e.edit {
                        openxiii_mods::Edit::Add { class, .. } => used.extend(class.strip_prefix(openxiii_mods::placement::CHARACTER_PREFIX).map(str::to_string)),
                        openxiii_mods::Edit::Change(c) => used.extend(c.character.clone()),
                        _ => {}
                    }
                }
            }
            let scripted_only = self.mods.script_sources.is_some() || self.mods.when_rules.is_some();
            for c in b.characters.iter().filter(|c| !scripted_only || used.iter().any(|u| u.eq_ignore_ascii_case(&c.name))) {
                for p in &c.properties {
                    walk(&p.value, &mut texts);
                    voice(&p.property, &p.value);
                }
                texts.extend(c.states.iter().filter_map(|s| s.sound.clone()));
            }
            for a in &b.actions {
                if let openxiii_mods::behaviour::ActionKind::Sound { sound } = &a.kind {
                    texts.push(sound.clone());
                }
            }
        }
        // Only the texts that are sounds of the game's packages.
        let mut sounds = Vec::new();
        for t in texts {
            let path = object_path(&t).to_string();
            if path.contains('.') && !sounds.contains(&path) && self.audio_sound_ref(&path).is_some() {
                sounds.push(path);
            }
        }
        voices.sort();
        voices.dedup();
        (sounds, voices)
    }

    /// The sounds of mods [character] `index`, made now by a script (its voice: CodeMesh, its
    /// states' sounds), which were not loaded with the level.
    pub(crate) fn mod_script_character_sounds(&mut self, index: usize) {
        if self.audio.is_none() {
            return;
        }
        let Some(c) = self.mods.behaviour.as_ref().and_then(|b| b.characters.get(index).cloned()) else { return };
        let mut paths: Vec<String> = c.states.iter().filter_map(|s| s.sound.clone()).collect();
        for p in &c.properties {
            if let (true, ModValue::Int(v)) = (p.property.eq_ignore_ascii_case("CodeMesh"), &p.value) {
                paths.extend(self.mod_voice_sounds(*v as i32).into_iter().map(|(path, _)| path));
            }
        }
        for path in paths {
            if let Err(why) = self.mod_ensure_sound(&path, &format!("character {}", c.name)) {
                self.mod_report(format!("warning: character {}: {why}", c.name));
            }
        }
    }

    /// End of audio_load_sounds: the banks the mods need on this level.
    pub(crate) fn mod_level_sounds(&mut self) {
        self.mod_register_own_sounds();
        self.select_level_sounds_again();
        let map = self.mod_map_name();
        if let Some(loads) = self.mods.loads.clone() {
            for l in loads.iter().filter(|l| l.map.as_deref().is_none_or(|m| openxiii_mods::maps::matches(m, &map))) {
                for b in &l.sound_banks {
                    self.mod_load_bank(b, &format!("load_sound_banks ({})", l.source));
                }
                for p in &l.packages {
                    let ok = self.library.as_mut().is_some_and(|lib| lib.get(p).is_some());
                    if ok {
                        self.mod_report(format!("package {p} loaded ({})", l.source));
                    } else {
                        self.mod_report(format!("warning: package {p} not found ({})", l.source));
                    }
                }
            }
        }
        if self.mods.defaults.is_none() && self.mods.edits.is_none() && self.mods.behaviour.is_none() && self.mods.own_sounds.is_none() {
            return;
        }
        let (sounds, voices) = self.mod_named_sounds();
        for s in sounds {
            if let Err(why) = self.mod_ensure_sound(&s, &format!("sound {s}")) {
                self.mod_report(format!("warning: {why}"));
            }
        }
        for v in voices {
            for (path, _) in self.mod_voice_sounds(v) {
                if let Err(why) = self.mod_ensure_sound(&path, &format!("voice CodeMesh {v}")) {
                    self.mod_report(format!("warning: voice CodeMesh {v}: {why}"));
                }
            }
        }
        // The game classes the mods bring to this level keep their own sounds (a sniper rifle's
        // shot on a map without sniper rifles): their banks come too.
        for c in self.mod_classes_here() {
            self.load_class_sounds(c, 2);
        }
        // The voices of the lines said on this map (`[[map.<Map>.say]]`), from any level.
        self.mod_dialogue_sounds();
        // The sounds playing instead of the game's (`[map.<Map>.replace_sounds]`): their banks,
        // then the ambient sounds that the level started before those banks were there.
        let swaps: Vec<(String, String)> = self
            .mod_edits_here()
            .iter()
            .filter_map(|e| match &e.edit {
                openxiii_mods::Edit::ReplaceSound { from, to } => Some((object_path(from).to_string(), object_path(to).to_string())),
                _ => None,
            })
            .collect();
        for (from, to) in &swaps {
            if let Err(why) = self.mod_ensure_sound(to, &format!("{map}: sound {from} plays as {to}")) {
                self.mod_report(format!("warning: {map}: sound {from} plays as {to}: {why}"));
            }
        }
        if !swaps.is_empty() {
            let actors: Vec<ObjId> = self.actors.clone();
            for a in actors {
                let swapped = matches!(self.prop(a, "AmbientSound"), Value::Asset(p) if self.mods.sound_swaps.contains_key(&p.to_ascii_lowercase()));
                if !swapped {
                    continue;
                }
                let Some(state) = self.audio_actor_state(a) else { continue };
                if let Some(hx) = self.audio.as_mut() {
                    hx.restart_ambient_actor(&state);
                }
            }
        }
    }

    /// End of the level start-up: the sounds of the game replaced on this map
    /// (`[map.<Map>.replace_sounds]`; for one sound replaced twice, the last entry read wins).
    pub(crate) fn mod_sound_swaps_start(&mut self) {
        self.mods.sound_swaps.clear();
        let map = self.mod_map_name();
        for e in self.mod_edits_here() {
            let openxiii_mods::Edit::ReplaceSound { from, to } = &e.edit else { continue };
            let from = object_path(from).to_string();
            let to = object_path(to).to_string();
            self.mods.sound_swaps.insert(from.to_ascii_lowercase(), to.clone());
            self.mod_report(format!("{map}: {} ({})", e.describe(), e.source));
        }
    }

    /// The classes the mods bring to the level being played: added, given at the start,
    /// replacing, the mods' new classes, and the base and spawned classes of mod characters and
    /// key actions.
    fn mod_classes_here(&mut self) -> Vec<crate::value::ClassId> {
        let mut names: Vec<String> = Vec::new();
        for e in self.mod_edits_here() {
            match &e.edit {
                openxiii_mods::Edit::Add { class, .. } => names.push(class.clone()),
                openxiii_mods::Edit::Replace { to, .. } => names.push(to.clone()),
                openxiii_mods::Edit::StartWeapon { class } => names.push(class.clone()),
                _ => {}
            }
        }
        if let Some(b) = self.mods.behaviour.clone() {
            names.extend(b.characters.iter().map(|c| c.base.clone()));
            for a in &b.actions {
                match &a.kind {
                    openxiii_mods::behaviour::ActionKind::Spawn { what, .. } if what.contains('.') => names.push(what.clone()),
                    openxiii_mods::behaviour::ActionKind::Give { class } => names.push(class.clone()),
                    _ => {}
                }
            }
        }
        if let Some(n) = self.mods.new_classes.clone() {
            names.extend(n.iter().map(|c| c.class_name()));
        }
        let mut out = Vec::new();
        for n in names {
            if let Some(c) = self.class(&n) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Improved build: the sound banks the default sounds of class `class` need on this level
    /// (its Sound values, and those of the classes it names: a weapon's attachment and ammo, a
    /// pickup's weapon), loaded as for the sounds named by mods (mod_ensure_sound). Used for the
    /// classes mods bring and for the test weapons of Select level. Returns the banks loaded.
    pub fn load_class_sounds(&mut self, class: crate::value::ClassId, depth: u8) -> usize {
        if self.audio.is_none() {
            return 0;
        }
        self.create_cdo(class);
        let cdo = self.classes[class].cdo;
        let path = self.class_path(class);
        let before = self.mod_banks.len();
        let props = self.classes[class].props.clone();
        // Pickups respawn only in multiplayer games: their respawn sound needs no bank in solo.
        let solo = !openxiii_mods::maps::is_multiplayer(&self.mod_map_name());
        for p in &props {
            match &p.ty {
                crate::types::Type::Object(meta) if meta.to_ascii_lowercase().ends_with("sound") && !(solo && p.name.eq_ignore_ascii_case("hRespawnSound")) => {
                    if let Value::Asset(sound) = self.prop(cdo, &p.name) {
                        // A sound of no bank (or not a game sound): nothing to load, no warning.
                        let _ = self.mod_ensure_sound(&sound, &format!("{path}.{}", p.name));
                    }
                }
                crate::types::Type::Class(_) if depth > 0 && ["AttachmentClass", "AmmoName", "AltAmmoName", "InventoryType"].iter().any(|n| p.name.eq_ignore_ascii_case(n)) => {
                    if let Value::Class(Some(k)) = self.prop(cdo, &p.name) {
                        if k != class {
                            self.load_class_sounds(k, depth - 1);
                        }
                    }
                }
                _ => {}
            }
        }
        self.mod_banks.len() - before
    }

    /// load_class_sounds of classes named "Package.Class" (Select level's test weapons). The
    /// classes are kept for the rest of the game: the banks leave with the map's own (level
    /// change, checkpoint reload, KillAllSounds) while the players keep the weapons, so they are
    /// loaded again with each level's sounds (`select_level_sounds_again`).
    pub fn load_class_sounds_named(&mut self, names: &[&str]) -> usize {
        let mut n = 0;
        for name in names {
            if !self.select_level_classes.iter().any(|k| k.eq_ignore_ascii_case(name)) {
                self.select_level_classes.push((*name).to_string());
            }
            if let Some(c) = self.class(name) {
                n += self.load_class_sounds(c, 2);
            }
        }
        n
    }

    /// End of each level's sound loading (and after KillAllSounds): the banks of the Select level
    /// test weapons that a player still carries, unloaded with the previous map's sounds. Back on
    /// the menu map the game is over: the list is emptied. Returns the banks loaded.
    pub(crate) fn select_level_sounds_again(&mut self) -> usize {
        if self.select_level_classes.is_empty() {
            return 0;
        }
        if self.mod_map_name().to_ascii_lowercase().starts_with("mapmenu") {
            self.select_level_classes.clear();
            return 0;
        }
        let carried = self.select_level_carried();
        let mut n = 0;
        for &c in &carried {
            n += self.load_class_sounds(c, 2);
        }
        n
    }

    /// Select level test weapon classes that a player's pawn (XIIIPlayerPawn) still carries,
    /// from their Pawn.Inventory chains.
    pub(crate) fn select_level_carried(&mut self) -> Vec<crate::value::ClassId> {
        // The players' inventories (Pawn.Inventory chains).
        let mut held: Vec<crate::value::ClassId> = Vec::new();
        let pawns: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "XIIIPlayerPawn")).collect();
        for p in pawns {
            let mut inv = self.prop(p, "Inventory").as_object();
            for _ in 0..256 {
                let Some(i) = inv.filter(|&i| self.objects.get(i).is_some_and(|o| o.is_some())) else { break };
                if let Some(o) = self.objects[i].as_ref() {
                    held.push(o.class);
                }
                inv = self.prop(i, "Inventory").as_object();
            }
        }
        self
            .select_level_classes
            .clone()
            .iter()
            .filter_map(|name| self.class(name))
            .filter(|&c| held.iter().any(|&h| self.is_child_of(h, c)))
            .collect()
    }

    /// Start of audio_clean_up (level change): the banks loaded for the mods leave with the
    /// map's own bank.
    pub(crate) fn mod_unload_banks(&mut self) {
        let banks = std::mem::take(&mut self.mod_banks);
        if let Some(hx) = self.audio.as_mut() {
            for (b, _, _) in &banks {
                hx.engine.unload_bank(b);
            }
        }
    }

    /// A non-script object (sound, texture, mesh…) named `text` in the game's packages, for a
    /// property whose class `meta` is not an actor class (actors are script objects of the
    /// level; sounds, textures and meshes are held by path): its package is loaded when needed.
    pub(crate) fn mod_asset_value(&mut self, meta: &str, text: &str) -> Option<Result<Value, String>> {
        let actor = self.class("Engine.Actor");
        if self.class(meta).zip(actor).is_some_and(|(c, a)| self.is_child_of(c, a)) {
            return None;
        }
        let path = object_path(text);
        if path.eq_ignore_ascii_case("None") {
            return Some(Ok(Value::Object(None)));
        }
        // A mod's own texture, 3D model or sound ("Mod.<Name>").
        if openxiii_assets::mod_textures::get(path).is_some() || openxiii_assets::mod_meshes::get(path).is_some() || openxiii_assets::mod_skeletal::get(path).is_some() || self.mod_sound_ref(path).is_some() {
            return Some(Ok(Value::Asset(path.to_string())));
        }
        let found = self.library.as_mut().and_then(|lib| lib.resolve(path).map(|(lp, i)| lp.package.export_class(i).to_string()));
        Some(match found {
            Some(_) => Ok(Value::Asset(path.to_string())),
            None => Err(format!("{path} is not an object of the game's packages")),
        })
    }

    /// Generated references of the game's sounds and content (docs/modding): `sounds.md` with
    /// `sounds/<Package>.md`, and `content.md` with `content/<Package>.md`. Paths relative to
    /// the output folder, with their text.
    pub fn content_references(&mut self) -> Result<Vec<(PathBuf, String)>, String> {
        let game = self.system_dir.as_ref().and_then(|d| d.parent()).map(Path::to_path_buf).ok_or("no game folder")?;
        let index = bank_index(&game.join("Sounds").join("PC"));
        let mut out = Vec::new();
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
        // Voices: readable names of the CodeMesh values (MeshSkinList).
        let mut voice_names: Vec<(i32, String)> = Vec::new();
        if let Some(c) = self.class("MeshSkinList") {
            self.create_cdo(c);
            if let Value::Array(items) = self.prop(self.classes[c].cdo, "MeshSkinListInfo") {
                for it in items {
                    if let Value::Struct(_, fields) = &it {
                        let name = fields.iter().find_map(|f| if let Value::Str(s) = f { (!s.is_empty()).then(|| s.clone()) } else { None });
                        let code = fields.iter().rev().find_map(|f| if let Value::Int(i) = f { Some(*i) } else { None });
                        if let (Some(n), Some(c)) = (name, code) {
                            if !voice_names.iter().any(|(k, m)| *k == c && *m == n) {
                                voice_names.push((c, n));
                            }
                        }
                    }
                }
            }
        }
        // Sounds, one page per package.
        let mut sound_index = String::from("# Sound reference\n\n");
        sound_index.push_str("All the names a mod can use: [names/](names/README.md).\n\n");
        sound_index.push_str(GENERATED);
        sound_index.push_str("Sounds are named `Package.Group.Name`. A sound plays only if a loaded sound bank holds its\n");
        sound_index.push_str("resource; the mods build loads the bank of any sound named in a mod value (expert/sounds.md,\n");
        sound_index.push_str("\"Sounds from any level\"). The bank column says which banks hold each sound (the shared\n");
        sound_index.push_str("bank `Fix` is always loaded; a map's own bank is named after its level title).\n\n");
        sound_index.push_str("## Packages\n\n| Package | Sounds |\n|---|---|\n");
        let mut pages = Vec::new();
        for pkg in files("Sounds", "uax") {
            let names: Vec<String> = {
                let Some(lib) = self.library.as_mut() else { break };
                let Some(p) = lib.get(&pkg) else { continue };
                (0..p.package.exports.len())
                    .filter(|&i| p.package.export_class(i) == "Sound")
                    .map(|i| format!("{}.{}", p.name, p.package.object_path(openxiii_package::ObjectRef::Export(i))))
                    .collect()
            };
            let mut page = format!("# Sounds of {pkg}\n\n{GENERATED}| Sound | Banks |\n|---|---|\n");
            for n in &names {
                let banks = self.audio_sound_ref(n).and_then(|s| index.by_id.get(&s.cuuid)).map(|l| l.iter().map(|&i| index.banks[i].0.clone()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                page.push_str(&format!("| {n} | {banks} |\n"));
            }
            sound_index.push_str(&format!("| [{pkg}](sounds/{pkg}.md) | {} |\n", names.len()));
            pages.push((PathBuf::from("sounds").join(format!("{pkg}.md")), page));
        }
        sound_index.push_str("\n## Character voices\n\n");
        sound_index.push_str("A character's pain, death and other shouts are chosen by its `CodeMesh` value (and its\n");
        sound_index.push_str("timbre, `NumeroTimbre`) in the onomatopoeia sets of the `Onomatopees` package. Give an added\n");
        sound_index.push_str("character the `CodeMesh` of the voice it should have; the mods build loads the banks that voice\n");
        sound_index.push_str("needs. Names from the game's skin list (MeshSkinList):\n\n| CodeMesh | Character | Sounds | Banks needed (besides Fix) |\n|---|---|---|---|\n");
        let mut codes: Vec<i32> = voice_names.iter().map(|(c, _)| *c).collect();
        codes.sort();
        codes.dedup();
        for c in codes {
            let names: Vec<&str> = voice_names.iter().filter(|(k, _)| *k == c).map(|(_, n)| n.as_str()).collect();
            let sounds = self.mod_voice_sounds(c);
            let mut banks: Vec<String> = sounds.iter().filter_map(|(_, id)| index.smallest_with(*id)).filter(|b| !b.eq_ignore_ascii_case("Fix")).map(str::to_string).collect();
            banks.sort();
            banks.dedup();
            sound_index.push_str(&format!("| {c} | {} | {} | {} |\n", names.join(", "), sounds.len(), banks.join(", ")));
        }
        sound_index.push_str("\n## Sound banks\n\n| Bank | Size (KB) | Resources |\n|---|---|---|\n");
        for (i, (b, size)) in index.banks.iter().enumerate() {
            let n = index.by_id.values().filter(|l| l.contains(&i)).count();
            sound_index.push_str(&format!("| {b} | {} | {n} |\n", size / 1024));
        }
        out.push((PathBuf::from("sounds.md"), sound_index));
        out.extend(pages);
        // Content: textures, static meshes, animations and characters' meshes, one page per package.
        let mut content = String::from("# Content reference\n\n");
        content.push_str("All the names a mod can use: [names/](names/README.md).\n\n");
        content.push_str(GENERATED);
        content.push_str("Textures, meshes and other objects are named `Package.Group.Name`. A mod can name any of\n");
        content.push_str("them in a value (for example a `Mesh` or `StaticMesh` property, or a decoration added with\n");
        content.push_str("`[[map.<Map>.add]]`): its package is loaded when needed (expert/content-and-network.md).\n\n");
        content.push_str("| Folder | Package | Objects by kind |\n|---|---|---|\n");
        // Script internals of the content packages of System/PC are left out.
        let script = |k: &str| k.is_empty() || k.ends_with("Property") || ["Class", "Function", "State", "Struct", "Enum", "Const", "TextBuffer", "Package"].contains(&k);
        for (dir, ext) in [("TexturesPC", "utx"), ("StaticMeshes", "usx"), ("system/PC", "u")] {
            for pkg in files(dir, ext) {
                let list: Vec<(String, String)> = {
                    let Some(lib) = self.library.as_mut() else { break };
                    let Some(p) = lib.get(&pkg) else { continue };
                    (0..p.package.exports.len())
                        .filter(|&i| !script(p.package.export_class(i)))
                        .map(|i| (p.package.export_class(i).to_string(), format!("{}.{}", p.name, p.package.object_path(openxiii_package::ObjectRef::Export(i)))))
                        .collect()
                };
                if list.is_empty() {
                    continue;
                }
                let mut kinds: Vec<(String, usize)> = Vec::new();
                for (k, _) in &list {
                    match kinds.iter_mut().find(|(n, _)| n == k) {
                        Some(e) => e.1 += 1,
                        None => kinds.push((k.clone(), 1)),
                    }
                }
                kinds.sort();
                let summary: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
                content.push_str(&format!("| {dir} | [{pkg}](content/{pkg}.md) | {} |\n", summary.join(", ")));
                let mut page = format!("# {pkg} ({dir})\n\n{GENERATED}");
                for (k, _) in &kinds {
                    page.push_str(&format!("## {k}\n\n"));
                    for (_, n) in list.iter().filter(|(x, _)| x == k) {
                        page.push_str(&format!("- {n}\n"));
                    }
                    page.push('\n');
                }
                out.push((PathBuf::from("content").join(format!("{pkg}.md")), page));
            }
        }
        out.push((PathBuf::from("content.md"), content));
        Ok(out)
    }

    /// The sound action: plays `path` at the player, after loading its bank if needed.
    pub(crate) fn mod_play_sound(&mut self, pawn: ObjId, path: &str, why: &str) -> Result<(), String> {
        self.mod_play_native("PlaySound", pawn, path, why)
    }

    /// Plays music `path` (the game's PlayMusic), loading its bank first if needed.
    pub(crate) fn mod_play_music(&mut self, pawn: ObjId, path: &str, why: &str) -> Result<(), String> {
        self.mod_play_native("PlayMusic", pawn, path, why)
    }

    fn mod_play_native(&mut self, native: &str, pawn: ObjId, path: &str, why: &str) -> Result<(), String> {
        self.mod_ensure_sound(path, why)?;
        let mut args = [Value::Asset(object_path(path).to_string())];
        crate::natives::call(self, native, pawn, &mut args, &[true]).map(|_| ()).map_err(|e| e.to_string())
    }
}
