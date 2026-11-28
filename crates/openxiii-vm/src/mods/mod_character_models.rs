//! Mods build only: docs/modding/character-models.md, the animated character models of the game
//! (package XIIIPersos) with the characters that wear them in the maps, read from the maps
//! themselves: the class, the in-game name (`PawnName`) and the skin of each placed character.
//! A model's file name is often a short or French word; this page says who it really is.

use std::collections::BTreeMap;
use std::path::Path;

use crate::value::Value;
use crate::{NativeField as Field, Vm};

/// Class and model names whose look is not what the name suggests, checked in the game.
const MISLEADING: &[(&str, &str)] = &[
    ("`XIDPawn.BaseSoldier` without a `Mesh`", "XIII himself (`XIIIPersos.XIIIM`); the same for `XIDCine.Cine2`, `XIDCine.ChoPawn` and `XIII.XIIICorpse`: in the maps the level designer gives their characters their own `Mesh`, and those that keep this one are XIII himself in cutscenes"),
    ("`XIDPawn.FBI_Helico01a`", "a uniformed guard (`XIIIPersos.GardienM`), not an FBI agent; the FBI agents are `XIIIPersos.FbiM` and `XIIIPersos.FBI2M`"),
    ("`XIIIPersos.GardienM`", "a uniformed guard with a star badge (\"gardien\": guard), worn by the guards of Banque01 and of the prison (PRock01a, PRock03)"),
    ("`XIIIPersos.MiocheM`", "a bald man in an orange vest, the killer of Plage01 (\"mioche\" means kid, but it is not a child)"),
    ("`XIIIPersos.KKKM`", "a hooded figure in a long yellow robe (the hooded men of Sanc02a, Sanc02b, Sanc03 and PRock)"),
    ("`XIII.MitraillGuard`", "invisible (`DrawType` `DT_None`): the gunner of a mounted machine gun, not a guard to place"),
];

/// One character placed in a map.
struct Placed {
    map: String,
    class: String,
    name: String,
    skin: Option<String>,
}

/// Who wears a model, gathered from every map.
#[derive(Default)]
struct Wearers {
    /// In-game names, with how many characters carry each.
    names: BTreeMap<String, usize>,
    classes: BTreeMap<String, usize>,
    maps: Vec<String>,
    skins: BTreeMap<String, usize>,
    count: usize,
    /// Characters without a name of their own.
    unnamed: usize,
}

/// Readable name of an object value ("XIIIPersos.SpadsM"), None for None.
pub(crate) fn object_name(vm: &Vm, v: &Value) -> Option<String> {
    match v {
        Value::Object(Some(o)) => vm.objects.get(*o).and_then(|x| x.as_ref()).map(|x| format!("{}.{}", x.outer, x.name)),
        // Meshes and textures are kept by name.
        Value::Asset(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

impl Vm {
    /// The characters placed in the map loaded in this Vm whose look is an animated model.
    fn placed_characters_here(&self, map: &str) -> Vec<(String, Placed)> {
        let mut out = Vec::new();
        for &a in &self.actors {
            let Some(o) = self.objects[a].as_ref() else { continue };
            if !self.is_a(a, "Pawn") {
                continue;
            }
            let Some(mesh) = object_name(self, &self.native_value(a, Field::Mesh)).or_else(|| object_name(self, &self.prop(a, "Mesh"))) else { continue };
            let name = match self.prop(a, "PawnName") {
                Value::Str(s) => s,
                _ => String::new(),
            };
            let skin = match self.prop(a, "Skins") {
                Value::Array(items) => items.first().and_then(|v| object_name(self, v)),
                _ => None,
            };
            out.push((mesh, Placed { map: map.to_string(), class: self.class_path(o.class), name, skin }));
        }
        out
    }

    /// Markdown page of the character models (docs/modding/character-models.md), from the
    /// packages and the maps of `game`.
    pub fn character_models_reference(game: &Path) -> Result<String, String> {
        // The models: every skeletal mesh of XIIIPersos.
        let mut base = Vm::new();
        base.ipdrv.offline = true;
        base.load_dir(&game.join("System")).map_err(|e| e.to_string())?;
        let mut models: Vec<String> = {
            let lib = base.library.as_mut().ok_or("no package library")?;
            let p = lib.get("XIIIPersos").ok_or("package XIIIPersos not found")?;
            (0..p.package.exports.len())
                .filter(|&i| p.package.export_class(i) == "SkeletalMesh")
                .map(|i| format!("XIIIPersos.{}", p.package.object_path(openxiii_package::ObjectRef::Export(i))))
                .collect()
        };
        models.sort_by_key(|m| m.to_lowercase());
        // The classes whose default look is each model.
        let mut defaults: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(pawn) = base.class("Engine.Pawn") {
            for c in 0..base.classes.len() {
                if !base.is_child_of(c, pawn) {
                    continue;
                }
                base.create_cdo(c);
                let cdo = base.classes[c].cdo;
                let parent_mesh = base.classes[c].super_class.map(|s| {
                    base.create_cdo(s);
                    base.prop(base.classes[s].cdo, "Mesh")
                });
                let mesh = base.prop(cdo, "Mesh");
                // Only the class that sets it, not each subclass that inherits it.
                if parent_mesh.as_ref() == Some(&mesh) {
                    continue;
                }
                if let Some(m) = object_name(&base, &mesh) {
                    defaults.entry(m.to_lowercase()).or_default().push(base.class_path(c));
                }
            }
        }
        // The maps.
        let mut names: Vec<String> = std::fs::read_dir(game.join("Maps"))
            .map_err(|e| format!("{}: {e}", game.join("Maps").display()))?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.path().file_name().map(|n| n.to_string_lossy().to_string()))
            .filter_map(|n| n.strip_suffix(".unr").or_else(|| n.strip_suffix(".UNR")).map(str::to_string))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        let mut worn: BTreeMap<String, Wearers> = BTreeMap::new();
        for n in &names {
            let mut vm = Vm::new();
            vm.ipdrv.offline = true;
            if vm.load_dir(&game.join("System")).is_err() {
                continue;
            }
            vm.url = crate::travel::start_url(&game.join("System"), n);
            if vm.load_map(&game.join("Maps").join(format!("{n}.unr"))).is_err() {
                continue;
            }
            for (mesh, p) in vm.placed_characters_here(n) {
                let w = worn.entry(mesh.to_lowercase()).or_default();
                w.count += 1;
                // "Someone" is the default PawnName of XIIIPawn: a character the level designer
                // did not name.
                let name = p.name.replace(['|', '\r', '\n'], " ").trim().to_string();
                if name.is_empty() || name.eq_ignore_ascii_case("Someone") || name.eq_ignore_ascii_case("None") {
                    w.unnamed += 1;
                } else {
                    *w.names.entry(name).or_default() += 1;
                }
                *w.classes.entry(p.class).or_default() += 1;
                if let Some(s) = p.skin {
                    *w.skins.entry(s).or_default() += 1;
                }
                if !w.maps.contains(&p.map) {
                    w.maps.push(p.map);
                }
            }
        }
        let list = |m: &BTreeMap<String, usize>| m.iter().map(|(k, n)| if *n > 1 { format!("{k} ({n})") } else { k.clone() }).collect::<Vec<_>>().join(", ");
        let mut out = String::new();
        out.push_str("# Character models\n\n");
        out.push_str("All the names a mod can use: [names/](names/README.md).\n\n");
        out.push_str("Generated by `oxiii mods-reference <game folder> docs/modding` (oxiii built with `--features mods`)\n");
        out.push_str("from the game's own packages and maps.\n\n");
        out.push_str("The animated models of the characters (package XIIIPersos) are named after a short or French\n");
        out.push_str("word, not always after the character: this page says who wears each one in the game. A\n");
        out.push_str("character's look is its `Mesh` (and its skin, `Skins[0]`), not its class: give\n");
        out.push_str("`properties = { Mesh = \"XIIIPersos.SpadsM\" }` to a character a mod adds (expert/characters.md, \"Changing\n");
        out.push_str("characters\"). The classes are in [characters.md](characters.md), the animations and bones of\n");
        out.push_str("each model in [animations.md](animations.md).\n\n");
        out.push_str("Columns: the in-game names (`PawnName`) and the classes of the characters placed in the maps\n");
        out.push_str("with this model (how many when more than one), the maps, the skins they wear besides the\n");
        out.push_str("model's own, and the classes whose default look it is. A model nobody wears in the maps is\n");
        out.push_str("still a valid `Mesh`, but it was only seen in cutscenes, in multiplayer or not at all: check it\n");
        out.push_str("in the game before using it.\n\n");
        out.push_str(&format!("**`{0}` is XIII himself.** It is the default `Mesh` of several classes (`XIDPawn.BaseSoldier`,\n", "XIIIPersos.XIIIM"));
        out.push_str("`XIDCine.Cine2`, `XIII.XIIICorpse`…): a character made from them without a `Mesh` looks like XIII.\n\n");
        out.push_str("| Model (`Mesh`) | In-game names | Classes in the maps | Maps | Skins | Default look of |\n|---|---|---|---|---|---|\n");
        for m in &models {
            let key = m.to_lowercase();
            let empty = Wearers::default();
            let w = worn.get(&key).unwrap_or(&empty);
            let shown = if key == "xiiipersos.xiiim" { format!("**`{m}`** (XIII)") } else { format!("`{m}`") };
            let names = if w.count == 0 {
                "(not placed in any map)".to_string()
            } else {
                let mut n = list(&w.names);
                if w.unnamed > 0 {
                    n = if n.is_empty() { format!("unnamed ({})", w.unnamed) } else { format!("{n}; unnamed ({})", w.unnamed) };
                }
                n
            };
            out.push_str(&format!(
                "| {shown} | {} | {} | {} | {} | {} |\n",
                if names.is_empty() { "-".into() } else { names },
                if w.classes.is_empty() { "-".into() } else { list(&w.classes) },
                if w.maps.is_empty() { "-".into() } else { w.maps.join(", ") },
                if w.skins.is_empty() { "-".into() } else { list(&w.skins) },
                defaults.get(&key).map(|v| v.join(", ")).unwrap_or_else(|| "-".into()),
            ));
        }
        out.push('\n');
        out.push_str("## Names that mislead\n\n");
        out.push_str("Checked in the game, with characters added by a mod on Plage01:\n\n");
        for (what, note) in MISLEADING {
            out.push_str(&format!("- {what}: {note}\n"));
        }
        out.push('\n');
        Ok(out)
    }
}
