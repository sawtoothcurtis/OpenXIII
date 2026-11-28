//! Changes to the actors of a map: `[[map.<Map>.add]]`, `[[map.<Map>.remove]]`,
//! `[map.<Map>.replace]` and `[map.<Map>.start]`; `[[map.<Map>.say]]` (lines said by
//! characters) is read by dialogue.rs.
//!
//! ```toml
//! [[map.Plage01.add]]                  # add an actor
//! class = "XIII.FusilSnipePick"
//! position = [-6518, -11741, 900]      # point on the ground, as printed by the modpos command
//! yaw = 90                             # degrees, optional
//! properties = { Health = 300 }        # optional, values of this actor only
//!
//! [[map.Plage01.remove]]               # remove an actor placed by the map, by its name…
//! name = "BerettaPick3"
//! [[map.Plage01.remove]]               # …or the actors of a class near a point
//! class = "XIII.BerettaPick"
//! near = [-6518, -11741, 900]
//! radius = 300                         # optional, 100 by default
//!
//! [map.Plage01.replace]                # every actor of exactly this class becomes another one
//! "XIII.BerettaPick" = "XIII.FusilChassePick"
//!
//! [map.Plage01.start]                  # given to the player when the level starts
//! weapons = ["XIII.Beretta", "XIII.FusilChasse"]
//! ammo = { "XIII.c9mmAmmo" = 50 }      # optional: amount of these ammo types afterwards
//!
//! [[map.PRock03.change]]               # change one actor placed by the map, which keeps its
//! who = "Cine2"                        # place and role: its name, Tag or PawnName (modwho)
//! look = "Jones"                       # a character model, full or friendly name (looks.rs)
//! ```
//!
//! This module only reads the files; the game's VM applies the changes (openxiii-vm, feature
//! `mods`).

use std::fmt;
use std::path::{Path, PathBuf};

use toml::Spanned;

use crate::defaults::{convert, parse_key, valid_class_name};
use crate::{line_of, ModValue, Problem};

/// Class of an `add` entry that names a [character.<Name>] (`character = "<Name>"`): this
/// prefix, then the name.
pub const CHARACTER_PREFIX: &str = "Character.";

/// Default `radius` of a `remove` by class and position.
pub const DEFAULT_RADIUS: f32 = 100.0;

/// Class of the decorations added with `model = "…"`: a new class made from the game's
/// StaticMeshActor (drawn with its StaticMesh, blocks players, characters and shots as the map's
/// own decorations do), that can be spawned.
pub const DECORATION_CLASS: &str = "Mod.ModelDecoration";
/// Short name of [`DECORATION_CLASS`].
pub const DECORATION_NAME: &str = "ModelDecoration";
/// The game class the decorations are made from.
pub const DECORATION_PARENT: &str = "Engine.StaticMeshActor";

/// The values [`DECORATION_CLASS`] changes on its parent: the map's StaticMeshActors are part
/// of the level (`bStatic`), which the engine refuses to spawn during play.
pub fn decoration_values() -> Vec<(&'static str, ModValue)> {
    vec![("bStatic", ModValue::Bool(false))]
}

/// The first change of `edits` that adds a decoration ([`DECORATION_CLASS`]), if any.
pub fn first_decoration(edits: &[MapEdit]) -> Option<&MapEdit> {
    edits.iter().find(|e| matches!(&e.edit, Edit::Add { class, .. } if class.eq_ignore_ascii_case(DECORATION_CLASS)))
}

/// Where a change comes from, for log lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub mod_name: String,
    pub file: PathBuf,
    pub line: usize,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mod {}, {} line {}", self.mod_name, self.file.display(), self.line)
    }
}

/// One value of an added actor (`properties`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorValue {
    pub property: String,
    pub index: Option<usize>,
    pub value: ModValue,
}

impl ActorValue {
    /// Property as written, with its index: "Health", "Prop[2]".
    pub fn property_label(&self) -> String {
        match self.index {
            Some(i) => format!("{}[{i}]", self.property),
            None => self.property.clone(),
        }
    }
}

/// What a change does.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Spawn an actor of `class` standing on `position` (point on the ground).
    Add { class: String, position: [f32; 3], yaw: Option<f32>, properties: Vec<ActorValue> },
    /// Remove the actor placed by the map with this name.
    RemoveName { name: String },
    /// Remove the actors of `class` (and subclasses) whose position is within `radius` of `near`.
    RemoveNear { class: String, near: [f32; 3], radius: f32 },
    /// Every actor of exactly class `from` becomes an actor of class `to` (same place, rotation,
    /// Tag and Event).
    Replace { from: String, to: String },
    /// Weapon given to the player when the level starts.
    StartWeapon { class: String },
    /// Amount of an ammo type the player has when the level starts (after the weapons).
    StartAmmo { class: String, amount: i32 },
    /// A line said by a character (`[[map.<Map>.say]]`, dialogue.rs).
    Say(Box<crate::dialogue::Say>),
    /// A "when ... then ..." block (`[[map.<Map>.when]]`, when.rs), run by the scripts' engine.
    When(Box<crate::when::When>),
    /// `[map.<Map>.replace_sounds]`: on this map, sound `from` of the game (`Package.Group.Name`)
    /// plays as `to` (another sound of the game, or a mod's own `Mod.<Name>`): the ambient
    /// sounds of the map's actors and every sound played by its name.
    ReplaceSound { from: String, to: String },
    /// `[[map.<Map>.change]]`: values written on actors placed by the map, which keep their
    /// place and their role.
    Change(Box<Change>),
}

/// A `[[map.<Map>.change]]` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// The actor's name ("Cine2"), else a character's Tag or PawnName, else another actor's
    /// Tag (case ignored).
    pub who: String,
    /// Among the actors `who` finds, only those within `radius` of this point.
    pub near: Option<[f32; 3]>,
    pub radius: f32,
    /// A character model, as written: "XIIIPersos.JonesM" or a friendly name ("Jones",
    /// looks.rs). Resolved by the game, which checks it exists.
    pub look: Option<String>,
    /// `Skins[0]` drawn on the model (a material of the game's packages or a mod's own
    /// texture). Without it, a new look is drawn with the model's own skin: the skins the map
    /// gave the actor are taken away.
    pub skin: Option<String>,
    pub health: Option<i32>,
    /// Raw values (`properties = { ... }`), written last; `name`, `voice` and `weapon` are
    /// translated into the first of them (simple.rs).
    pub properties: Vec<ActorValue>,
    /// `team` or `can_die`: the derived [character] the actor becomes (its name).
    pub character: Option<String>,
}

impl Change {
    /// "change Cine2: look Jones, health 200".
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(l) = &self.look {
            parts.push(format!("look {l}"));
        }
        if let Some(s) = &self.skin {
            parts.push(format!("skin {s}"));
        }
        if let Some(h) = self.health {
            parts.push(format!("health {h}"));
        }
        for p in &self.properties {
            parts.push(format!("{} = {}", p.property_label(), p.value));
        }
        let near = match self.near {
            Some(p) => format!(" within {} of [{}, {}, {}]", self.radius, p[0], p[1], p[2]),
            None => String::new(),
        };
        format!("change {}{near}: {}", self.who, parts.join(", "))
    }
}

/// From which save point of the map a change is made (maps.md numbers them: 1 is the map's
/// start, 2, 3… the checkpoints in the order they are reached).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Checkpoint {
    /// Whatever save point the level starts from.
    #[default]
    Any,
    /// `from_checkpoint = N`: when the level starts from checkpoint N or a later one.
    From(u32),
    /// `[map.<Map>.checkpoint.<N>.start]`: when the level starts from checkpoint N.
    At(u32),
}

impl Checkpoint {
    /// True if a level started from checkpoint `n` gets the change.
    pub fn applies(self, n: u32) -> bool {
        match self {
            Checkpoint::Any => true,
            Checkpoint::From(k) => n >= k,
            Checkpoint::At(k) => n == k,
        }
    }
}

/// A change to the actors of one map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapEdit {
    /// Map name as written ("Plage01"), or a selector ("Hual*", "solo", "*").
    pub map: String,
    pub edit: Edit,
    pub source: Source,
    /// The save points the level must start from.
    pub checkpoint: Checkpoint,
}

impl MapEdit {
    /// Short description for log lines: "add XIII.FusilSnipePick at [-6518, -11741, 900]".
    pub fn describe(&self) -> String {
        let v = |p: &[f32; 3]| format!("[{}, {}, {}]", p[0], p[1], p[2]);
        match &self.edit {
            Edit::Add { class, position, .. } => format!("add {class} at {}", v(position)),
            Edit::RemoveName { name } => format!("remove {name}"),
            Edit::RemoveNear { class, near, radius } => format!("remove {class} within {radius} of {}", v(near)),
            Edit::Replace { from, to } => format!("replace {from} with {to}"),
            Edit::StartWeapon { class } => format!("start weapon {class}"),
            Edit::StartAmmo { class, amount } => format!("start ammo {class} = {amount}"),
            Edit::Say(say) => say.describe(),
            Edit::When(w) => w.describe(),
            Edit::ReplaceSound { from, to } => format!("sound {from} plays as {to}"),
            Edit::Change(c) => c.describe(),
        }
    }
}

/// The `map.<Map>` keys this module reads (the `class` tables are read by defaults.rs).
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct MapActors {
    #[serde(default)]
    pub add: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    pub remove: Vec<Spanned<toml::Table>>,
    #[serde(default)]
    pub replace: Option<Spanned<toml::Table>>,
    #[serde(default)]
    pub start: Option<Spanned<toml::Table>>,
    /// `[[map.<Map>.say]]` (dialogue.rs).
    #[serde(default)]
    pub say: Vec<Spanned<toml::Table>>,
    /// `[[map.<Map>.change]]`: values of actors placed by the map.
    #[serde(default)]
    pub change: Vec<Spanned<toml::Table>>,
    /// `[map.<Map>.replace_sounds]`: "game sound" = "game sound or Mod.<Name>".
    #[serde(default)]
    pub replace_sounds: Option<Spanned<toml::Table>>,
    /// `[[map.<Map>.when]]` (when.rs).
    #[serde(default)]
    pub when: Vec<Spanned<toml::Table>>,
    /// `[map.<Map>.checkpoint.<N>.start]`: (N as written, its start table).
    #[serde(skip)]
    pub checkpoint_start: Vec<(String, Spanned<toml::Table>)>,
}

/// Key of add and remove entries: from which checkpoint the change is made.
const FROM_CHECKPOINT: &str = "from_checkpoint";

/// A checkpoint number as written: 1 or more.
fn checkpoint_number(v: &toml::Value) -> Option<u32> {
    match v {
        toml::Value::Integer(n) if (1..=999).contains(n) => Some(*n as u32),
        _ => None,
    }
}

/// A `change` entry given as a table by a script (`change(who, {...})`): read exactly as a
/// `[[map.<Map>.change]]` entry, `who` included.
pub fn read_change(t: &toml::Table) -> Result<(Change, crate::simple::SimpleKeys), String> {
    Reader { text: "", file: Path::new("script"), mod_name: "script" }.change(t)
}

/// The file being read.
pub(crate) struct Reader<'a> {
    pub text: &'a str,
    pub file: &'a Path,
    pub mod_name: &'a str,
}

impl Reader<'_> {
    fn source(&self, line: usize) -> Source {
        Source { mod_name: self.mod_name.to_string(), file: self.file.to_path_buf(), line }
    }

    fn error(&self, line: usize, message: String) -> Problem {
        Problem::Error { file: self.file.to_path_buf(), line: Some(line), message }
    }

    fn line(&self, start: usize) -> usize {
        line_of(self.text, start)
    }

    /// The actor changes of map `map`. A wrong entry gives an error (with its line) and is
    /// skipped; the others are kept.
    /// An `add` with `character` and simple keys, and a `change` with `team` or `can_die`, add
    /// a derived definition to `characters` (simple.rs).
    pub fn map_actors(
        &self,
        map: &str,
        m: MapActors,
        out: &mut Vec<MapEdit>,
        characters: &mut Vec<crate::behaviour::CharacterDef>,
        problems: &mut Vec<Problem>,
    ) {
        let push = |out: &mut Vec<MapEdit>, line: usize, edit: Edit, checkpoint: Checkpoint| {
            out.push(MapEdit { map: map.to_string(), edit, source: self.source(line), checkpoint })
        };
        // add and remove entries: `from_checkpoint = N` (optional) and the change itself.
        let entry_checkpoint = |t: &toml::Table| -> Result<(toml::Table, Checkpoint), String> {
            let mut t = t.clone();
            let cp = match t.remove(FROM_CHECKPOINT) {
                None => Checkpoint::Any,
                Some(v) => Checkpoint::From(
                    checkpoint_number(&v).ok_or_else(|| format!("{FROM_CHECKPOINT} = {v} must be a checkpoint number, 1 or more (see maps.md)"))?,
                ),
            };
            Ok((t, cp))
        };
        for entry in m.add {
            let line = self.line(entry.span().start);
            match entry_checkpoint(entry.get_ref()).and_then(|(t, cp)| self.add(&t).map(|e| (e, cp))) {
                Ok(((mut edit, keys), cp)) => {
                    if let (Edit::Add { class, .. }, false) = (&mut edit, keys.is_empty()) {
                        if let Some(like) = class.strip_prefix(CHARACTER_PREFIX).map(str::to_string) {
                            let derived = crate::behaviour::derived_name(&like, self.file, line);
                            characters.push(crate::behaviour::derived_def(&derived, &like, keys, self.source(line)));
                            *class = format!("{CHARACTER_PREFIX}{derived}");
                        }
                    }
                    push(out, line, edit, cp)
                }
                Err(why) => problems.push(self.error(line, format!("map {map}: add: {why}; entry skipped"))),
            }
        }
        for entry in m.remove {
            let line = self.line(entry.span().start);
            match entry_checkpoint(entry.get_ref()).and_then(|(t, cp)| self.remove(&t).map(|e| (e, cp))) {
                Ok((edit, cp)) => push(out, line, edit, cp),
                Err(why) => problems.push(self.error(line, format!("map {map}: remove: {why}; entry skipped"))),
            }
        }
        for entry in m.say {
            let line = self.line(entry.span().start);
            match entry_checkpoint(entry.get_ref()).and_then(|(t, cp)| crate::dialogue::read(&t).map(|s| (s, cp))) {
                Ok((say, cp)) => push(out, line, Edit::Say(Box::new(say)), cp),
                Err(why) => problems.push(self.error(line, format!("map {map}: say: {why}; entry skipped"))),
            }
        }
        for entry in m.when {
            let line = self.line(entry.span().start);
            match entry_checkpoint(entry.get_ref()).and_then(|(t, cp)| crate::when::read(&t).map(|w| (w, cp))) {
                Ok((mut w, cp)) => {
                    // A spawn with settings of a character: a derived character (as an add with
                    // the same settings), its name in the level (PawnName) the spawn's word so
                    // that `who` finds it; the action names the derived character.
                    let mut failed = None;
                    for (i, (index, what, keys)) in std::mem::take(&mut w.spawn_keys).into_iter().enumerate() {
                        let mut keys = keys;
                        keys.entry("name").or_insert_with(|| toml::Value::String(what.clone()));
                        match crate::simple::read(&keys) {
                            Ok(k) => {
                                let derived = format!("{}#{i}", crate::behaviour::derived_name(&what, self.file, line));
                                characters.push(crate::behaviour::derived_def(&derived, &what, k, self.source(line)));
                                if let Some(crate::when::Arg::Value(crate::when::Value::Table(fields))) = w.actions[index].args.first_mut() {
                                    for (k, v) in fields.iter_mut() {
                                        if k == "character" {
                                            *v = crate::when::Value::Text(derived.clone());
                                        }
                                    }
                                }
                            }
                            Err(why) => failed = Some(format!("then: item {}: spawn: {why}", index + 1)),
                        }
                    }
                    if let Some(why) = failed {
                        problems.push(self.error(line, format!("map {map}: when: {why}; block skipped")));
                        continue;
                    }
                    if let crate::when::Event::Key { key } = &w.event {
                        if let Some(why) = crate::behaviour::reserved_key_warning(key) {
                            problems.push(Problem::Warning { path: self.file.to_path_buf(), message: format!("line {line}: map {map}: when: {why}") });
                        }
                    }
                    push(out, line, Edit::When(Box::new(w)), cp)
                }
                Err(why) => problems.push(self.error(line, format!("map {map}: when: {why}; block skipped"))),
            }
        }
        for entry in m.change {
            let line = self.line(entry.span().start);
            match entry_checkpoint(entry.get_ref()).and_then(|(t, cp)| self.change(&t).map(|c| (c, cp))) {
                Ok(((mut c, keys), cp)) => {
                    // team and can_die: the actor becomes a mod character (its AI asks the team).
                    if keys.team.is_some() || keys.can_die.is_some() {
                        let derived = crate::behaviour::derived_name(&format!("change {}", c.who), self.file, line);
                        let mut def = crate::behaviour::derived_def(&derived, "", keys, self.source(line));
                        def.like = None;
                        def.team = def.keys.team;
                        def.can_die = def.keys.can_die.unwrap_or(true);
                        characters.push(def);
                        c.character = Some(derived);
                    }
                    push(out, line, Edit::Change(Box::new(c)), cp)
                }
                Err(why) => problems.push(self.error(line, format!("map {map}: change: {why}; entry skipped"))),
            }
        }
        if let Some(table) = m.replace {
            let line = self.line(table.span().start);
            for (from, to) in table.get_ref() {
                match (to, valid_class_name(from)) {
                    (toml::Value::String(to), true) if valid_class_name(to) => {
                        push(out, line, Edit::Replace { from: from.clone(), to: to.clone() }, Checkpoint::Any)
                    }
                    _ => problems
                        .push(self.error(line, format!("map {map}: replace: write \"Package.Class\" = \"Package.Class\" (got {from} = {to}); entry skipped"))),
                }
            }
        }
        if let Some(table) = m.replace_sounds {
            let line = self.line(table.span().start);
            // A sound name: "Package.Group.Name" (two dots or more), or a mod's own "Mod.<Name>".
            let sound_name = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace) && (s.matches('.').count() >= 2 || s.starts_with(crate::own_sounds::PREFIX));
            for (from, to) in table.get_ref() {
                match to {
                    toml::Value::String(to) if sound_name(from) && !from.starts_with(crate::own_sounds::PREFIX) && sound_name(to) => {
                        push(out, line, Edit::ReplaceSound { from: from.clone(), to: to.clone() }, Checkpoint::Any)
                    }
                    _ => problems.push(self.error(line, format!("map {map}: replace_sounds: write \"Package.Group.Sound\" = \"Package.Group.Sound\" or \"Mod.<Name>\" (got {from} = {to}); entry skipped"))),
                }
            }
        }
        if let Some(table) = m.start {
            self.start(map, &table, Checkpoint::Any, out, problems);
        }
        for (n, table) in m.checkpoint_start {
            match n.parse::<u32>().ok().filter(|k| (1..=999).contains(k)) {
                Some(k) => self.start(map, &table, Checkpoint::At(k), out, problems),
                None => problems.push(self.error(
                    self.line(table.span().start),
                    format!("map {map}: checkpoint.{n}: \"{n}\" is not a checkpoint number (1 or more, see maps.md); skipped"),
                )),
            }
        }
    }

    /// A `start` table: weapons and ammo given when the level starts (from `checkpoint`).
    fn start(&self, map: &str, table: &Spanned<toml::Table>, checkpoint: Checkpoint, out: &mut Vec<MapEdit>, problems: &mut Vec<Problem>) {
        let line = self.line(table.span().start);
        let push = |out: &mut Vec<MapEdit>, edit: Edit| out.push(MapEdit { map: map.to_string(), edit, source: self.source(line), checkpoint });
        let scope = match checkpoint {
            Checkpoint::At(k) => format!("checkpoint.{k}.start"),
            _ => "start".to_string(),
        };
        // Weapons first: the ammo amounts apply to what the weapons gave.
        let mut entries: Vec<(&String, &toml::Value)> = table.get_ref().iter().collect();
        entries.sort_by_key(|(k, _)| *k != "weapons");
        for (key, value) in entries {
            match (key.as_str(), value) {
                ("weapons", toml::Value::Array(items)) => {
                    for item in items {
                        match item {
                            // A class, or a friendly weapon name (simple.rs).
                            toml::Value::String(c) => match crate::simple::resolve_weapon(c) {
                                Ok(class) if valid_class_name(&class) => push(out, Edit::StartWeapon { class }),
                                Ok(_) => problems.push(self.error(
                                    line,
                                    format!("map {map}: {scope}: weapon {item} must be \"Package.Class\" or a weapon name such as \"Pistol\"; skipped"),
                                )),
                                Err(close) => problems.push(self.error(
                                    line,
                                    format!(
                                        "map {map}: {scope}: weapon {item} is not a weapon{} (docs/modding/names/weapons.md, or the names catalog); skipped",
                                        crate::looks::did_you_mean(&close)
                                    ),
                                )),
                            },
                            _ => problems.push(self.error(
                                line,
                                format!("map {map}: {scope}: weapon {item} must be \"Package.Class\" or a weapon name such as \"Pistol\"; skipped"),
                            )),
                        }
                    }
                }
                ("ammo", toml::Value::Table(t)) => {
                    for (class, amount) in t {
                        match (amount, valid_class_name(class)) {
                            (toml::Value::Integer(n), true) if (0..=i32::MAX as i64).contains(n) => {
                                push(out, Edit::StartAmmo { class: class.clone(), amount: *n as i32 })
                            }
                            _ => problems
                                .push(self.error(line, format!("map {map}: {scope}: ammo \"{class}\" = {amount}: write \"Package.Class\" = amount; skipped"))),
                        }
                    }
                }
                _ => problems.push(self.error(line, format!("map {map}: {scope}: unknown key {key} (use weapons = [...] and ammo = {{ ... }}); skipped"))),
            }
        }
    }

    fn add(&self, t: &toml::Table) -> Result<(Edit, crate::simple::SimpleKeys), String> {
        let mut allowed = vec!["class", "model", "character", "position", "yaw", "rotation", "scale", "collision", "properties", FROM_CHECKPOINT];
        allowed.extend_from_slice(crate::simple::KEYS);
        known_keys(t, &allowed)?;
        let keys = crate::simple::read(t)?;
        if !keys.is_empty() && !t.contains_key("character") {
            return Err(format!("{} go with character = \"<Name>\" (a look name such as \"Jones\", or a [character.<Name>])", crate::simple::KEYS.join(", ")));
        }
        // `character`: a [character.<Name>] of the mods (its base class, team, states, looks).
        let character = match t.get("character") {
            None => None,
            Some(_) if t.contains_key("class") || t.contains_key("model") => return Err("use one of class, model or character".into()),
            Some(toml::Value::String(n)) if !n.trim().is_empty() && !n.contains('.') => Some(n.trim().to_string()),
            Some(v) => return Err(format!("character = {v} must be a look name such as \"Jones\" or the name of a [character.<Name>] table")),
        };
        // `model`: a solid decoration drawn with that static mesh (a mod's model or a game mesh).
        let model = match (t.get("model"), t.get("class")) {
            (None, _) => None,
            (Some(_), Some(_)) => return Err("use either class (an actor of a game class) or model (a decoration), not both".into()),
            (Some(toml::Value::String(m)), None) if m.contains('.') && !m.trim().ends_with('.') => Some(m.trim().to_string()),
            (Some(v), None) => return Err(format!("model = {v} must be a static mesh \"Mod.<Name>\" or \"Package.Group.Name\"")),
        };
        let class = match (&model, &character) {
            (_, Some(n)) => format!("{CHARACTER_PREFIX}{n}"),
            (Some(_), None) => DECORATION_CLASS.to_string(),
            (None, None) => class_key(t, "class").map_err(|e| format!("{e} (or model = \"Mod.<Name>\" for a decoration, or character = \"<Name>\")"))?,
        };
        let position = vector(t, "position")?.ok_or("position = [x, y, z] is missing")?;
        let yaw = match t.get("yaw") {
            None => None,
            Some(v) => Some(number(v).ok_or("yaw must be a number of degrees")?),
        };
        let mut properties = Vec::new();
        if let Some(m) = model {
            properties.push(ActorValue { property: "StaticMesh".into(), index: None, value: ModValue::Str(m) });
        }
        if t.contains_key("rotation") && t.contains_key("yaw") {
            return Err("use either yaw or rotation = [pitch, yaw, roll], not both".into());
        }
        properties.extend(placement_values(t)?);
        match t.get("properties") {
            None => {}
            Some(toml::Value::Table(p)) => {
                for (key, value) in p {
                    let (property, index) = parse_key(key).ok_or_else(|| format!("properties: \"{key}\" is not a property name"))?;
                    let value = convert(value).map_err(|e| format!("properties.{key}: {e}"))?;
                    properties.push(ActorValue { property, index, value });
                }
            }
            Some(_) => return Err("properties must be a table, for example { Health = 300 }".into()),
        }
        Ok((Edit::Add { class, position, yaw, properties }, keys))
    }

    fn change(&self, t: &toml::Table) -> Result<(Change, crate::simple::SimpleKeys), String> {
        if t.contains_key("behaviour") {
            return Err("behaviour cannot be changed on a character the map places: it keeps its role (its class and AI); use team, or add a character with [[map.<Map>.add]]".into());
        }
        known_keys(t, &["who", "near", "radius", "look", "skin", "health", "name", "voice", "weapon", "team", "can_die", "properties", FROM_CHECKPOINT])?;
        let keys = crate::simple::read(t)?;
        let text = |key: &str| -> Result<Option<String>, String> {
            match t.get(key) {
                None => Ok(None),
                Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
                Some(v) => Err(format!("{key} = {v} must be a name in quotes")),
            }
        };
        let who = text("who")?.ok_or("who = \"…\" is missing (the actor's name, Tag or PawnName, as printed by the modwho command)")?;
        let near = vector(t, "near")?;
        let radius = match (t.get("radius"), near) {
            (None, _) => DEFAULT_RADIUS,
            (Some(_), None) => return Err("radius goes with near = [x, y, z]".into()),
            (Some(v), Some(_)) => number(v).filter(|r| *r > 0.0).ok_or("radius must be a positive number")?,
        };
        let look = text("look")?;
        let skin = text("skin")?;
        if let Some(l) = &look {
            // A short name is checked here (the full names are checked by the game).
            if let Err(close) = crate::looks::resolve(l) {
                return Err(format!("look = \"{l}\" is not a character model{} (write a model name such as \"XIIIPersos.JonesM\", or a name of character-models.md)", crate::looks::did_you_mean(&close)));
            }
        }
        if skin.as_deref().is_some_and(|s| !s.contains('.')) {
            return Err(format!("skin = \"{}\" must be a material \"Package.Group.Name\" or a texture \"Mod.<Name>\"", skin.unwrap_or_default()));
        }
        let health = match t.get("health") {
            None => None,
            Some(toml::Value::Integer(h)) if (1..=i32::MAX as i64).contains(h) => Some(*h as i32),
            Some(v) => return Err(format!("health = {v} must be a whole number, 1 or more")),
        };
        // name, voice and weapon: their values, before the raw properties (which win).
        let simple = crate::simple::SimpleKeys { name: keys.name.clone(), voice: keys.voice, weapon: keys.weapon.clone(), ..Default::default() };
        let mut properties = simple.values();
        match t.get("properties") {
            None => {}
            Some(toml::Value::Table(p)) => {
                for (key, value) in p {
                    let (property, index) = parse_key(key).ok_or_else(|| format!("properties: \"{key}\" is not a property name"))?;
                    let value = convert(value).map_err(|e| format!("properties.{key}: {e}"))?;
                    properties.push(ActorValue { property, index, value });
                }
            }
            Some(_) => return Err("properties must be a table, for example { GroundSpeed = 300 }".into()),
        }
        if look.is_none() && skin.is_none() && health.is_none() && properties.is_empty() && keys.team.is_none() && keys.can_die.is_none() {
            return Err("nothing to change: give look, skin, health, name, voice, weapon, team, can_die or properties".into());
        }
        Ok((Change { who, near, radius, look, skin, health, properties, character: None }, keys))
    }

    fn remove(&self, t: &toml::Table) -> Result<Edit, String> {
        known_keys(t, &["name", "class", "near", "radius", FROM_CHECKPOINT])?;
        match t.get("name") {
            Some(toml::Value::String(name)) => {
                if t.len() > 1 {
                    return Err("use either name, or class with near (and radius)".into());
                }
                Ok(Edit::RemoveName { name: name.clone() })
            }
            Some(_) => Err("name must be a string (the actor's name, such as \"BerettaPick3\")".into()),
            None => {
                let class = class_key(t, "class").map_err(|_| "write name = \"…\", or class = \"Package.Class\" with near = [x, y, z]".to_string())?;
                let near = vector(t, "near")?.ok_or("near = [x, y, z] is missing")?;
                let radius = match t.get("radius") {
                    None => DEFAULT_RADIUS,
                    Some(v) => number(v).filter(|r| *r > 0.0).ok_or("radius must be a positive number")?,
                };
                Ok(Edit::RemoveNear { class, near, radius })
            }
        }
    }
}

/// Game rotation units in a full turn (65536) per degree.
const UNITS_PER_DEGREE: f64 = 65536.0 / 360.0;

/// The actor values of an added actor's `rotation`, `scale` and `collision` (written before its
/// own `properties`, which can change them again):
/// - `rotation = [pitch, yaw, roll]` in degrees: Rotation (pitch raises the front, yaw turns
///   left, roll tilts to the right);
/// - `scale = 2.0` (DrawScale) or `scale = [x, y, z]` (DrawScale3D, per axis);
/// - `collision = true`: the actor blocks players, characters and shots (bCollideActors,
///   bBlockActors, bBlockPlayers, bBlockZeroExtentTraces, bBlockNonZeroExtentTraces);
///   `false`: nothing collides with it.
pub fn placement_values(t: &toml::Table) -> Result<Vec<ActorValue>, String> {
    let mut out = Vec::new();
    let value = |property: &str, value: ModValue| ActorValue { property: property.into(), index: None, value };
    if let Some(v) = t.get("rotation") {
        let angles = match v {
            toml::Value::Array(items) if items.len() == 3 => items.iter().map(number).collect::<Option<Vec<f32>>>(),
            _ => None,
        }
        .ok_or("rotation must be [pitch, yaw, roll] in degrees")?;
        let units = |d: f32| ModValue::Int((d as f64 * UNITS_PER_DEGREE).round() as i64);
        out.push(value(
            "Rotation",
            ModValue::Table(vec![("Pitch".into(), units(angles[0])), ("Yaw".into(), units(angles[1])), ("Roll".into(), units(angles[2]))]),
        ));
    }
    if let Some(v) = t.get("scale") {
        match v {
            toml::Value::Array(items) if items.len() == 3 => {
                let s = items.iter().map(number).collect::<Option<Vec<f32>>>().filter(|s| s.iter().all(|x| *x > 0.0)).ok_or("scale = [x, y, z] must be three positive numbers")?;
                let f = |x: f32| ModValue::Float(x as f64);
                out.push(value("DrawScale3D", ModValue::Table(vec![("X".into(), f(s[0])), ("Y".into(), f(s[1])), ("Z".into(), f(s[2]))])));
            }
            v => {
                let s = number(v).filter(|x| *x > 0.0).ok_or("scale must be a positive number, or [x, y, z]")?;
                out.push(value("DrawScale", ModValue::Float(s as f64)));
            }
        }
    }
    if let Some(v) = t.get("collision") {
        let toml::Value::Boolean(on) = v else { return Err("collision must be true or false".into()) };
        for p in ["bCollideActors", "bBlockActors", "bBlockPlayers", "bBlockZeroExtentTraces", "bBlockNonZeroExtentTraces"] {
            out.push(value(p, ModValue::Bool(*on)));
        }
    }
    Ok(out)
}

fn known_keys(t: &toml::Table, keys: &[&str]) -> Result<(), String> {
    match t.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown key {k} (keys: {})", keys.join(", "))),
        None => Ok(()),
    }
}

fn class_key(t: &toml::Table, key: &str) -> Result<String, String> {
    match t.get(key) {
        Some(toml::Value::String(c)) if valid_class_name(c) => Ok(c.clone()),
        Some(v) => Err(format!("{key} = {v} must be \"Package.Class\"")),
        None => Err(format!("{key} = \"Package.Class\" is missing")),
    }
}

fn number(v: &toml::Value) -> Option<f32> {
    match v {
        toml::Value::Integer(i) => Some(*i as f32),
        toml::Value::Float(f) if f.is_finite() => Some(*f as f32),
        _ => None,
    }
}

fn vector(t: &toml::Table, key: &str) -> Result<Option<[f32; 3]>, String> {
    let Some(v) = t.get(key) else { return Ok(None) };
    let toml::Value::Array(items) = v else { return Err(format!("{key} must be [x, y, z]")) };
    let n: Vec<f32> = items.iter().filter_map(number).collect();
    if items.len() != 3 || n.len() != 3 {
        return Err(format!("{key} must be three numbers [x, y, z]"));
    }
    Ok(Some([n[0], n[1], n[2]]))
}

#[cfg(test)]
mod tests {
    use crate::defaults::parse_mod_file;
    use crate::placement::{Edit, DEFAULT_RADIUS};
    use crate::{ModValue, Problem};
    use std::path::Path;

    fn parse(text: &str) -> (Vec<crate::placement::MapEdit>, Vec<Problem>) {
        let f = parse_mod_file(text, Path::new("m/mod.toml"), "m");
        (f.edits, f.problems)
    }

    #[test]
    fn all_kinds() {
        let text = r#"
[[map.Plage01.add]]
class = "XIII.FusilSnipePick"
position = [-6518, -11741.5, 900]
yaw = 90

[[map.Plage01.add]]
class = "XIDPawn.BaseSoldier"
position = [1, 2, 3]
properties = { Health = 300, "Prop[1]" = true }

[[map.Plage01.remove]]
name = "BerettaPick3"

[[map.Plage01.remove]]
class = "XIII.BerettaPick"
near = [10, 20, 30]

[map.Plage01.replace]
"XIII.BerettaPick" = "XIII.FusilChassePick"

[map.Plage01.start]
weapons = ["XIII.Beretta", "XIII.FusilChasse"]
ammo = { "XIII.c9mmAmmo" = 50 }

[map.Plage01.class."XIII.Beretta"]
ReloadCount = 17
"#;
        let (e, p) = parse(text);
        assert!(p.is_empty(), "{p:?}");
        let kinds: Vec<&Edit> = e.iter().map(|x| &x.edit).collect();
        assert_eq!(kinds.len(), 8, "{kinds:?}");
        assert_eq!(*kinds[0], Edit::Add { class: "XIII.FusilSnipePick".into(), position: [-6518.0, -11741.5, 900.0], yaw: Some(90.0), properties: vec![] });
        assert_eq!(e[0].source.line, 2);
        assert_eq!(e[0].map, "Plage01");
        match kinds[1] {
            Edit::Add { properties, yaw: None, .. } => {
                assert_eq!(properties.len(), 2);
                assert_eq!(properties[0].property, "Health");
                assert_eq!(properties[0].value, ModValue::Int(300));
                assert_eq!(properties[1].property_label(), "Prop[1]");
            }
            k => panic!("{k:?}"),
        }
        assert_eq!(*kinds[2], Edit::RemoveName { name: "BerettaPick3".into() });
        assert_eq!(*kinds[3], Edit::RemoveNear { class: "XIII.BerettaPick".into(), near: [10.0, 20.0, 30.0], radius: DEFAULT_RADIUS });
        assert_eq!(*kinds[4], Edit::Replace { from: "XIII.BerettaPick".into(), to: "XIII.FusilChassePick".into() });
        assert_eq!(*kinds[5], Edit::StartWeapon { class: "XIII.Beretta".into() });
        assert_eq!(*kinds[6], Edit::StartWeapon { class: "XIII.FusilChasse".into() });
        assert_eq!(*kinds[7], Edit::StartAmmo { class: "XIII.c9mmAmmo".into(), amount: 50 });
        assert_eq!(e[2].describe(), "remove BerettaPick3");
    }

    #[test]
    fn errors_have_lines_and_skip_only_the_entry() {
        let text = r#"
[[map.Plage01.add]]
class = "FusilSnipePick"
position = [1, 2, 3]

[[map.Plage01.add]]
class = "XIII.FusilSnipePick"
position = [1, 2]

[[map.Plage01.add]]
class = "XIII.FusilSnipePick"
position = [1, 2, 3]
colour = "red"

[[map.Plage01.remove]]
class = "XIII.BerettaPick"

[[map.Plage01.remove]]
name = "A"
near = [1, 2, 3]

[[map.Plage01.add]]
class = "XIII.FusilSnipePick"
position = [4, 5, 6]

[map.Plage01.start]
weapons = ["Raygun"]
lives = 3
"#;
        let (e, p) = parse(text);
        assert_eq!(e.len(), 1, "{e:?}");
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        let has = |line: usize, what: &str| all.iter().any(|x| x.contains(&format!("(line {line})")) && x.contains(what));
        assert!(has(2, "must be \"Package.Class\""), "{all:?}");
        assert!(has(6, "three numbers"), "{all:?}");
        assert!(has(10, "unknown key colour"), "{all:?}");
        assert!(has(15, "near = [x, y, z] is missing"), "{all:?}");
        assert!(has(18, "either name"), "{all:?}");
        assert!(all.iter().any(|x| x.contains("weapon \"Raygun\" is not a weapon")), "{all:?}");
        assert!(all.iter().any(|x| x.contains("unknown key lives")), "{all:?}");
    }

    #[test]
    fn rotation_scale_collision() {
        let text = r#"
[[map.Plage01.add]]
model = "Mod.Crate"
position = [1, 2, 3]
rotation = [0, 90, 45]
scale = [1, 2, 0.5]
collision = false

[[map.Plage01.add]]
class = "XIII.MedPick"
position = [1, 2, 3]
scale = 2
properties = { DrawScale = 3.0 }

[[map.Plage01.add]]
class = "XIII.MedPick"
position = [1, 2, 3]
yaw = 90
rotation = [0, 90, 0]

[[map.Plage01.add]]
class = "XIII.MedPick"
position = [1, 2, 3]
scale = -1
"#;
        let (e, p) = parse(text);
        assert_eq!(e.len(), 2, "{e:?}");
        let Edit::Add { properties, yaw, .. } = &e[0].edit else { panic!() };
        assert_eq!(*yaw, None);
        let get = |name: &str| properties.iter().find(|v| v.property == name).map(|v| v.value.clone());
        assert_eq!(get("Rotation"), Some(ModValue::Table(vec![("Pitch".into(), ModValue::Int(0)), ("Yaw".into(), ModValue::Int(16384)), ("Roll".into(), ModValue::Int(8192))])));
        assert_eq!(get("DrawScale3D"), Some(ModValue::Table(vec![("X".into(), ModValue::Float(1.0)), ("Y".into(), ModValue::Float(2.0)), ("Z".into(), ModValue::Float(0.5))])));
        assert_eq!(get("bBlockPlayers"), Some(ModValue::Bool(false)));
        assert_eq!(get("bCollideActors"), Some(ModValue::Bool(false)));
        // `properties` come after the placement values: they win.
        let Edit::Add { properties, .. } = &e[1].edit else { panic!() };
        let scales: Vec<&ModValue> = properties.iter().filter(|v| v.property == "DrawScale").map(|v| &v.value).collect();
        assert_eq!(scales, vec![&ModValue::Float(2.0), &ModValue::Float(3.0)]);
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        assert!(all.iter().any(|x| x.contains("not both")), "{all:?}");
        assert!(all.iter().any(|x| x.contains("positive number")), "{all:?}");
    }

    #[test]
    fn decorations() {
        let text = r#"
[[map.Plage01.add]]
model = "Mod.Crate"
position = [1, 2, 3]
properties = { DrawScale = 2.0 }

[[map.Plage01.add]]
model = "Mod.Crate"
class = "XIII.MedPick"
position = [1, 2, 3]

[[map.Plage01.add]]
model = "Crate"
position = [1, 2, 3]
"#;
        let (e, p) = parse(text);
        assert_eq!(e.len(), 1, "{e:?}");
        match &e[0].edit {
            Edit::Add { class, properties, .. } => {
                assert_eq!(class, crate::placement::DECORATION_CLASS);
                // The model first, then the entry's own values.
                assert_eq!((properties[0].property.as_str(), &properties[0].value), ("StaticMesh", &ModValue::Str("Mod.Crate".into())));
                assert_eq!(properties[1].property, "DrawScale");
            }
            k => panic!("{k:?}"),
        }
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        assert!(all.iter().any(|x| x.contains("(line 7)") && x.contains("not both")), "{all:?}");
        assert!(all.iter().any(|x| x.contains("(line 12)") && x.contains("static mesh")), "{all:?}");
        assert_eq!(crate::placement::first_decoration(&e).map(|d| d.source.line), Some(2));
    }

    #[test]
    fn change_entries() {
        use crate::placement::Change;
        let text = r#"
[[map.PRock03.change]]
who = "Cine2"
look = "Jones"

[[map.PRock03.change]]
who = "gardien_lostpass"
near = [10, 20, 30]
radius = 50
look = "XIIIPersos.SpadsM"
skin = "XIIIPersos.Skins.SpadsNeigeTex"
health = 200
properties = { GroundSpeed = 300 }
from_checkpoint = 2

[[map.PRock03.change]]
look = "Jones"

[[map.PRock03.change]]
who = "Cine2"
look = "Jnes"

[[map.PRock03.change]]
who = "Cine2"

[[map.PRock03.change]]
who = "Cine2"
radius = 10
health = 5

[[map.PRock03.change]]
who = "Cine2"
health = 0

[[map.PRock03.change]]
who = "Cine2"
health = 5
colour = "red"
"#;
        let (e, p) = parse(text);
        assert_eq!(e.len(), 2, "{e:?}");
        let Edit::Change(c) = &e[0].edit else { panic!("{e:?}") };
        assert_eq!(
            **c,
            Change {
                who: "Cine2".into(),
                near: None,
                radius: DEFAULT_RADIUS,
                look: Some("Jones".into()),
                skin: None,
                health: None,
                properties: vec![],
                character: None
            }
        );
        assert_eq!(e[0].source.line, 2);
        assert_eq!(e[0].describe(), "change Cine2: look Jones");
        let Edit::Change(c) = &e[1].edit else { panic!() };
        assert_eq!((c.near, c.radius, c.health), (Some([10.0, 20.0, 30.0]), 50.0, Some(200)));
        assert_eq!(c.skin.as_deref(), Some("XIIIPersos.Skins.SpadsNeigeTex"));
        assert_eq!((c.properties[0].property.as_str(), &c.properties[0].value), ("GroundSpeed", &ModValue::Int(300)));
        assert_eq!(e[1].checkpoint, crate::placement::Checkpoint::From(2));
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        assert_eq!(all.len(), 6, "{all:?}");
        let has = |line: usize, what: &str| all.iter().any(|x| x.contains(&format!("(line {line})")) && x.contains(what));
        assert!(has(16, "who = \"…\" is missing"), "{all:?}");
        assert!(has(19, "did you mean Jones"), "{all:?}");
        assert!(has(23, "nothing to change"), "{all:?}");
        assert!(has(26, "radius goes with near"), "{all:?}");
        assert!(has(31, "1 or more"), "{all:?}");
        assert!(has(35, "unknown key colour"), "{all:?}");
    }

    #[test]
    fn typo_in_map_section_is_a_file_error() {
        let (e, p) = parse("[[map.Plage01.ad]]\nclass = \"XIII.FusilSnipePick\"\nposition = [1, 2, 3]\n");
        assert!(e.is_empty());
        assert!(matches!(&p[0], Problem::Error { line: Some(1), .. }), "{p:?}");
    }

    #[test]
    fn checkpoint_conditions() {
        use crate::placement::Checkpoint;
        let text = r#"
[[map.Banque01.add]]
class = "XIII.MedPick"
position = [1, 2, 3]
from_checkpoint = 2

[[map.Banque01.remove]]
name = "MedPick0"
from_checkpoint = 3

[map.Banque01.checkpoint.2.start]
weapons = ["XIII.m16"]

[[map.Banque01.add]]
class = "XIII.MedPick"
position = [1, 2, 3]
from_checkpoint = 0

[map.Banque01.checkpoint.x.start]
weapons = ["XIII.m16"]
"#;
        let (e, p) = parse(text);
        let cps: Vec<Checkpoint> = e.iter().map(|x| x.checkpoint).collect();
        assert_eq!(cps, [Checkpoint::From(2), Checkpoint::From(3), Checkpoint::At(2)], "{e:?}");
        assert!(Checkpoint::From(2).applies(3) && !Checkpoint::From(2).applies(1));
        assert!(Checkpoint::At(2).applies(2) && !Checkpoint::At(2).applies(3) && Checkpoint::Any.applies(1));
        let all: Vec<String> = p.iter().map(|x| x.to_string()).collect();
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(all.iter().any(|x| x.contains("from_checkpoint = 0")), "{all:?}");
        assert!(all.iter().any(|x| x.contains("checkpoint.x")), "{all:?}");
    }
}