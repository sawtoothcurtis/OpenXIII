//! Mods build only (feature `mods`): the maps reference page (docs/modding/maps.md), written by
//! `oxiii mods-reference`: every map of the game's Maps folder, read from the map itself.
//!
//! - Solo maps, in the order of the story: from the first map, each map's `MapInfo` names the
//!   next one (`NextMapLevelWithUnr`, MapInfo.uc).
//! - Multiplayer maps: the other maps whose name starts with DM_, CTF_ or SB_.
//! - Technical maps: the others (menu, credits, empty maps).
//!
//! For each map: its level title (`LevelInfo.Title`), where the player appears at the start
//! (`PlayerStart`), and its save points (`XIIISaveGameTrigger`): XIIISaveGameTrigger.DoSave adds
//! one to `XIIIGameInfo.CheckpointNumber` each time the player saves, so the save made when the
//! map is entered is number 1 and the checkpoints reached in the map are 2, 3…

use std::path::{Path, PathBuf};

static MOD_FILES: std::sync::OnceLock<Vec<(String, PathBuf)>> = std::sync::OnceLock::new();

/// The mods' maps and packages (name, file), set once at startup (mod_setup.rs).
pub fn set_mod_map_files(files: Vec<(String, PathBuf)>) {
    let _ = MOD_FILES.set(files);
}

/// The maps the mods bring (name, file), in mod order: for the menus that list maps.
pub fn mod_maps() -> Vec<(String, PathBuf)> {
    MOD_FILES.get().into_iter().flatten().filter(|(_, p)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("unr"))).cloned().collect()
}

use crate::{NativeField as Field, Vm};

/// One save point of a map.
#[derive(Debug, Clone)]
pub struct SavePoint {
    pub name: String,
    pub tag: String,
    pub teleporter: String,
    pub description: String,
    /// Where the trigger stands.
    pub position: [f32; 3],
    /// Where the player appears when the game is loaded there (the Teleporter it names).
    pub start: Option<[f32; 3]>,
    /// Saved when the map is entered (triggered by an event, not touched).
    pub at_entry: bool,
}

/// One character (Pawn) placed by a map, as the level designer saved it (no script run).
#[derive(Debug, Clone, Default)]
pub struct PlacedCharacter {
    /// Object name, unique in the map ("XIIIBaseSoldier12").
    pub name: String,
    pub class: String,
    /// In-game name (`PawnName`), empty when none.
    pub pawn_name: String,
    /// `Tag`, empty when None.
    pub tag: String,
    /// Model (`Mesh`, "Package.Name"), empty when none.
    pub model: String,
    pub health: Option<i32>,
    /// `CodeMesh`: the voice of its shouts (BaseSoldier), when it has one.
    pub voice: Option<i32>,
    pub position: [f32; 3],
    /// The shortest `who` value that finds this character alone ([`Vm::find_placed`]): its
    /// PawnName or Tag when no other actor answers to it, else its object name.
    pub who: String,
}

/// What maps.md says about one map.
#[derive(Debug, Clone, Default)]
pub struct MapEntry {
    pub name: String,
    pub title: String,
    pub next: Option<String>,
    pub starts: Vec<[f32; 3]>,
    pub saves: Vec<SavePoint>,
    /// The ambient sounds of the map's actors (AmbientSound), with how many actors play each.
    pub ambient: Vec<(String, usize)>,
    /// The characters the map places, in the map's order (filled by `read_map_entry`).
    pub characters: Vec<PlacedCharacter>,
    /// The events the map's actors trigger (their `Event` values), sorted: the names a line
    /// said `when = "event"` can wait for (filled by `read_map_entry`).
    pub events: Vec<String>,
}

use openxiii_mods::maps::{is_multiplayer, is_technical};

/// The map named by a `NextMapLevelWithUnr` value ("Banque01.unr?…" or "Banque01#portal").
fn map_of_url(url: &str) -> Option<String> {
    let s = url.split(['?', '#']).next()?.trim();
    let s = s.rsplit(['/', '\\']).next()?;
    let s = s.strip_suffix(".unr").or_else(|| s.strip_suffix(".UNR")).unwrap_or(s);
    (!s.is_empty()).then(|| s.to_string())
}

impl Vm {
    /// The map `name` of `game`, loaded (its actors placed, no script run), with its characters.
    pub(crate) fn read_map_entry(game: &Path, name: &str) -> Result<MapEntry, String> {
        // The game's own map: the mods of a running game are left out.
        let mut vm = crate::mod_catalog::game_only_vm();
        vm.load_dir(&game.join("System")).map_err(|e| e.to_string())?;
        vm.read_map_here(game, name)
    }

    /// The map `name` of `game` loaded in this Vm (its System folder already loaded; a map loaded
    /// before is taken away first, as a travel does), with its characters and events.
    pub(crate) fn read_map_here(&mut self, game: &Path, name: &str) -> Result<MapEntry, String> {
        if self.map_package.is_some() {
            self.unload_level();
        }
        self.url = crate::travel::start_url(&game.join("System"), name);
        self.load_map(&game.join("Maps").join(format!("{name}.unr"))).map_err(|e| format!("{name}: {e}"))?;
        let mut entry = self.map_entry_here(name);
        entry.characters = self.placed_characters_list();
        entry.events = self.map_events_list();
        Ok(entry)
    }

    /// The events the actors of the loaded map trigger (`Event`), sorted, without repeats.
    pub(crate) fn map_events_list(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for &a in &self.actors {
            if self.objects[a].is_none() {
                continue;
            }
            let e = self.prop(a, "Event").as_name();
            if !e.is_empty() && !e.eq_ignore_ascii_case("None") && !out.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
                out.push(e);
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        out
    }

    /// The characters (Pawns) placed in the map loaded in this Vm, in the map's order.
    pub(crate) fn placed_characters_list(&self) -> Vec<PlacedCharacter> {
        let none = |s: String| if s.eq_ignore_ascii_case("None") { String::new() } else { s };
        let mut out = Vec::new();
        for &a in &self.actors {
            let Some(o) = self.objects[a].as_ref() else { continue };
            if !self.is_a(a, "Pawn") || self.propr(a, "bDeleteMe").as_bool() {
                continue;
            }
            let pawn_name = match self.prop(a, "PawnName") {
                crate::value::Value::Str(s) => none(s.trim().to_string()),
                v => none(v.as_name()),
            };
            let tag = none(self.prop(a, "Tag").as_name());
            let model = crate::mod_character_models::object_name(self, &self.native_value(a, Field::Mesh))
                .or_else(|| crate::mod_character_models::object_name(self, &self.prop(a, "Mesh")))
                .unwrap_or_default();
            let health = match self.prop(a, "Health") {
                crate::value::Value::Int(h) => Some(h),
                _ => None,
            };
            // The friendliest value that finds this character and nothing else.
            let who = self.friendly_who(a);
            out.push(PlacedCharacter {
                name: o.name.clone(),
                class: self.class_path(o.class),
                pawn_name,
                tag,
                model,
                health,
                voice: match self.prop(a, "CodeMesh") {
                    crate::value::Value::Int(v) => Some(v),
                    _ => None,
                },
                position: self.native_value(a, Field::Location).as_vector(),
                who,
            });
        }
        out
    }

    /// The checkpoint the level starts from, numbered as maps.md does: the save point whose
    /// teleporter is the URL's portal (a checkpoint is loaded at `Map#Teleporter`, select_level.rs),
    /// 1 (the map's start) otherwise.
    pub(crate) fn mod_checkpoint_here(&self) -> u32 {
        // Read at the start of begin_play: a loaded checkpoint then destroys save points of the
        // map (XIIISaveGameTrigger.CleanMap, ActorsToDestroy), which would change the order.
        if let Some(n) = self.mods.checkpoint {
            return n;
        }
        let portal = self.url.portal.clone();
        if portal.is_empty() || portal.eq_ignore_ascii_case("PlayerStart") {
            return 1;
        }
        let entry = self.map_entry_here("");
        entry.saves.iter().position(|s| s.teleporter.eq_ignore_ascii_case(&portal)).map_or(1, |i| i as u32 + 1)
    }

    /// What maps.md says about the map loaded in this Vm.
    pub(crate) fn map_entry_here(&self, name: &str) -> MapEntry {
        let vm = self;
        let mut entry = MapEntry { name: name.to_string(), ..Default::default() };
        let text = |v: crate::value::Value| match v {
            crate::value::Value::Str(s) => s,
            v @ crate::value::Value::Name(_) => v.as_name().to_string(),
            other => format!("{other}"),
        };
        let at = |vm: &Vm, a: usize| vm.native_value(a, Field::Location).as_vector();
        let mut teleporters = Vec::new();
        for &a in &vm.actors {
            if vm.objects[a].is_none() {
                continue;
            }
            if vm.is_a(a, "LevelInfo") {
                entry.title = text(vm.prop(a, "Title"));
            } else if vm.is_a(a, "MapInfo") {
                entry.next = map_of_url(&text(vm.prop(a, "NextMapLevelWithUnr")));
            } else if vm.is_a(a, "XIIISaveGameTrigger") {
                entry.saves.push(SavePoint {
                    name: vm.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default(),
                    tag: text(vm.prop(a, "Tag")),
                    teleporter: text(vm.prop(a, "TeleporterName")),
                    description: text(vm.prop(a, "SaveDescription")),
                    position: at(vm, a),
                    start: None,
                    // The map entry's save point starts the player at the map's PlayerStart.
                    at_entry: {
                        let t = text(vm.prop(a, "TeleporterName"));
                        t.is_empty() || t.eq_ignore_ascii_case("PlayerStart")
                    },
                });
            } else if vm.is_a(a, "PlayerStart") {
                entry.starts.push(at(vm, a));
            }
            if vm.is_a(a, "Teleporter") {
                teleporters.push((text(vm.prop(a, "Tag")), at(vm, a)));
            }
            if let crate::value::Value::Asset(s) = vm.prop(a, "AmbientSound") {
                match entry.ambient.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&s)) {
                    Some((_, k)) => *k += 1,
                    None => entry.ambient.push((s, 1)),
                }
            }
        }
        entry.ambient.sort_by_key(|(n, _)| n.to_lowercase());
        let first_start = entry.starts.first().copied();
        for s in &mut entry.saves {
            s.start = teleporters.iter().find(|(t, _)| !s.teleporter.is_empty() && t.eq_ignore_ascii_case(&s.teleporter)).map(|(_, p)| *p);
            if s.at_entry && s.start.is_none() {
                s.start = first_start;
            }
        }
        // The map entry's save first, then the checkpoints by their description, which the level
        // designers numbered in the order they are reached ("Winslow Bank 1", "Winslow Bank 2"),
        // then by teleporter and object name.
        entry.saves.sort_by(|a, b| {
            (!a.at_entry, natural(&a.description), natural(&a.teleporter), natural(&a.name)).cmp(&(
                !b.at_entry,
                natural(&b.description),
                natural(&b.teleporter),
                natural(&b.name),
            ))
        });
        entry
    }

    /// Markdown reference of the maps (docs/modding/maps.md), from the maps of `game`.
    pub fn maps_reference(game: &Path) -> Result<String, String> {
        let names = crate::mod_catalog::map_names(game)?;
        let entries = crate::mod_catalog::read_maps(game, &names, 1, &|_, _| {});
        let (story, multi, technical) = map_groups(&entries);

        let mut out = String::new();
        out.push_str("# Map reference\n\n");
        out.push_str("All the names a mod can use: [names/](names/README.md).\n\n");
        out.push_str("Generated by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`)\n");
        out.push_str("from the game's own maps. Use these map names (the file names without `.unr`) in\n");
        out.push_str("`[map.<Map>…]` tables, and the checkpoint numbers in `from_checkpoint` and\n");
        out.push_str("`[map.<Map>.checkpoint.<N>.start]` (see [expert/maps-and-checkpoints.md](expert/maps-and-checkpoints.md)).\n\n");
        out.push_str(&format!(
            "{} maps: {} solo maps in the order of the story, {} multiplayer maps, {} technical maps.\n\n",
            entries.len(),
            story.len(),
            multi.len(),
            technical.len()
        ));
        out.push_str("**Checkpoints.** A save point (`XIIISaveGameTrigger`) saves the game when the player touches it,\n");
        out.push_str("or when the map is entered for the one the map's start triggers. Each save adds one to the\n");
        out.push_str("level's checkpoint number (`XIIIGameInfo.CheckpointNumber`): the map entry's save is 1, the\n");
        out.push_str("checkpoints reached in the map are 2, 3… in the order the player reaches them. The numbers\n");
        out.push_str("below follow the save points' descriptions, which the level designers numbered in the order\n");
        out.push_str("they are reached (\"Winslow Bank 1\", \"Winslow Bank 2\"). Loading a checkpoint loads the map\n");
        out.push_str("again and starts the player at the save point's teleporter.\n\n");
        out.push_str("Contents: [Solo maps](#solo-maps), [Multiplayer maps](#multiplayer-maps), [Technical maps](#technical-maps).\n\n");
        out.push_str("## Solo maps\n\n| # | Map | Level title | Next map | Checkpoints |\n|---|---|---|---|---|\n");
        for (k, &i) in story.iter().enumerate() {
            let e = &entries[i];
            out.push_str(&format!(
                "| {} | [{}](#{}) | {} | {} | {} |\n",
                k + 1,
                e.name,
                e.name.to_lowercase(),
                e.title,
                e.next.clone().unwrap_or_default(),
                e.saves.len()
            ));
        }
        out.push('\n');
        for &i in &story {
            map_section(&mut out, &entries[i]);
        }
        out.push_str("## Multiplayer maps\n\n| Map | Level title | Player starts |\n|---|---|---|\n");
        for &i in &multi {
            let e = &entries[i];
            out.push_str(&format!("| {} | {} | {} |\n", e.name, e.title, e.starts.len()));
        }
        out.push_str("\n## Technical maps\n\n| Map | Level title |\n|---|---|\n");
        for &i in &technical {
            let e = &entries[i];
            out.push_str(&format!("| {} | {} |\n", e.name, e.title));
        }
        Ok(out)
    }
}

/// The maps of `entries` in three groups of indices: the solo maps in the order of the story
/// (from the solo map no other map leads to, through NextMapLevelWithUnr), the multiplayer
/// maps, the technical maps.
pub(crate) fn map_groups(entries: &[MapEntry]) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let find = |n: &str| entries.iter().position(|e| e.name.eq_ignore_ascii_case(n));
    let solo_candidates: Vec<usize> =
        (0..entries.len()).filter(|&i| !is_multiplayer(&entries[i].name) && entries[i].next.as_deref().and_then(find).is_some()).collect();
    let pointed: Vec<usize> = entries.iter().filter_map(|e| e.next.as_deref().and_then(find)).collect();
    let mut story = Vec::new();
    if let Some(&first) = solo_candidates.iter().find(|i| !pointed.contains(i)) {
        let mut k = Some(first);
        while let Some(i) = k {
            if story.contains(&i) || is_multiplayer(&entries[i].name) || is_technical(&entries[i].name) {
                break;
            }
            story.push(i);
            k = entries[i].next.as_deref().and_then(find);
        }
    }
    let multi: Vec<usize> = (0..entries.len()).filter(|i| !story.contains(i) && is_multiplayer(&entries[*i].name)).collect();
    let technical: Vec<usize> = (0..entries.len()).filter(|i| !story.contains(i) && !multi.contains(i)).collect();
    (story, multi, technical)
}

fn pos(p: [f32; 3]) -> String {
    format!("[{:.0}, {:.0}, {:.0}]", p[0], p[1], p[2])
}

/// "checkpoint10" after "checkpoint9": runs of digits compared as numbers.
fn natural(s: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push((std::mem::take(&mut text), digits.parse().unwrap_or(0)));
                digits.clear();
            }
            text.push(c);
        }
    }
    out.push((text, digits.parse().unwrap_or(0)));
    out
}

fn map_section(out: &mut String, e: &MapEntry) {
    out.push_str(&format!("### {}\n\n", e.name));
    out.push_str(&format!("Level title: {}. ", if e.title.is_empty() { "none" } else { &e.title }));
    match &e.next {
        Some(n) if !is_technical(n) => out.push_str(&format!("Next map: {n}.\n\n")),
        _ => out.push_str("Last map of the story.\n\n"),
    }
    if !e.starts.is_empty() {
        let list: Vec<String> = e.starts.iter().map(|p| pos(*p)).collect();
        out.push_str(&format!("Player start: {}.\n\n", list.join(", ")));
    }
    if !e.ambient.is_empty() {
        // `[map.<Map>.replace_sounds]` names them (README.md, "Replacing a sound on one map").
        let list: Vec<String> = e.ambient.iter().map(|(s, n)| if *n > 1 { format!("`{s}` ({n} actors)") } else { format!("`{s}`") }).collect();
        out.push_str(&format!("Ambient sounds: {}.\n\n", list.join(", ")));
    }
    if e.saves.is_empty() {
        out.push_str("No save point.\n\n");
        return;
    }
    out.push_str("| Checkpoint | Save point | Teleporter | Player appears at | Trigger at | Description |\n|---|---|---|---|---|---|\n");
    let mut n = 0;
    for s in &e.saves {
        // Each save adds one to the level's number (DoSave), from 0 when the map is loaded.
        n += 1;
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            n,
            s.name,
            if s.teleporter.is_empty() { "-" } else { &s.teleporter },
            s.start.map(pos).unwrap_or_else(|| "-".into()),
            pos(s.position),
            s.description.replace('|', "\\|")
        ));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Option<std::path::PathBuf> {
        let g = std::path::PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
        g.join("System").exists().then_some(g)
    }

    #[test]
    fn urls_and_names() {
        assert_eq!(map_of_url("Banque01.unr?Game=XIII.XIIIGameInfo").as_deref(), Some("Banque01"));
        assert_eq!(map_of_url("hual01a#start").as_deref(), Some("hual01a"));
        assert_eq!(map_of_url("").as_deref(), None);
        assert!(natural("Major Jones 2") < natural("Major Jones 10"));
        assert!(is_multiplayer("ctf_Sanc") && is_multiplayer("DM_Amos") && !is_multiplayer("SB"));
    }

    /// Save points of two maps in the order the game numbers them (their descriptions), the
    /// map entry's first; Toits01's objects are not in that order.
    #[test]
    fn save_points_in_play_order() {
        let Some(game) = game() else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let toits = Vm::read_map_entry(&game, "Toits01").unwrap();
        let d: Vec<&str> = toits.saves.iter().map(|s| s.description.as_str()).collect();
        assert_eq!(d, ["Major Jones", "Major Jones 1", "Major Jones 2", "Major Jones 3"]);
        assert!(toits.saves[0].at_entry && !toits.saves[1].at_entry);
        assert_eq!(toits.saves[1].teleporter, "Toits_CheckPointbis");
        assert!(toits.saves[1].start.is_some());
        let banque = Vm::read_map_entry(&game, "Banque01").unwrap();
        assert_eq!(banque.next.as_deref(), Some("Amos01"));
        assert_eq!(banque.saves.iter().map(|s| s.teleporter.as_str()).collect::<Vec<_>>(), ["PlayerStart", "Teleporter_bricole", "Teleporter_otage"]);
    }
}

/// The warnings for the mods' files named as files of the game.
pub(crate) fn map_collisions(game: &Path, files: &[(String, PathBuf)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, path) in files {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let game_file = ["Maps", "MapsUser", "System", "Textures", "TexturesPC", "StaticMeshes", "Sounds", "Animations", "Music"]
            .iter()
            .map(|d| game.join(d).join(format!("{name}.{ext}")))
            .find(|p| p.is_file());
        if let Some(g) = game_file {
            out.push(format!("warning: {} has the name of a file of the game ({}): the game's file is used; rename the mod's", path.display(), g.display()));
        }
    }
    out
}

impl Vm {
    /// A map or package of a mod with the name of a file of the game: the game's file is used,
    /// and the log says so (once).
    pub(crate) fn mod_map_collisions(&mut self, game: &Path) {
        for line in map_collisions(game, MOD_FILES.get().map(Vec::as_slice).unwrap_or_default()) {
            self.mod_report(line);
        }
    }
}
