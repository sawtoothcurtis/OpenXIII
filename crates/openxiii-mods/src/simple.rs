//! Simple words for characters (level 1): keys a modder writes instead of class names and
//! raw property values. They are translated, when the mods are read, into the values the game
//! already uses; nothing new runs in the game for them.
//!
//! ```toml
//! [[map.Plage01.add]]
//! character = "Jones"          # a friendly look name: no [character] table needed
//! position = [-470, -860, 1190]
//! weapon = "Pistol"            # optional simple keys (below)
//! ```
//!
//! | Key | Becomes |
//! |---|---|
//! | `look` | `Mesh` (looks.rs) |
//! | `skin` | `Skins = [<skin>]` (the first skin; Skins is a list) |
//! | `health` | `Health` |
//! | `name` | `PawnName` |
//! | `voice` | `CodeMesh`, the set of shouts of BaseSoldier (MeshSkinList's names, or a look name) |
//! | `weapon` | `InitialInventory[0] = (Inventory = <class>, Count = 2)`, which BaseSoldier.InitializeInventory gives at the start of play (soldiers and story characters, Cine2) |
//! | `team` | the [character]'s team (friend, neutral, enemy) |
//! | `behaviour` | the class: "stands" = XIDCine.Cine2 (a calm story character), "patrols" = XIDPawn.BaseSoldier (the soldiers' AI), "fights" = XIDPawn.BaseSoldier with team enemy |
//! | `can_die` | the [character]'s can_die |
//!
//! Raw `properties` written in the same entry are written after these values, so they win.

use crate::behaviour::{ActionKind, CharacterDef, ModAction, Team};
use crate::looks;
use crate::placement::{ActorValue, Edit, MapEdit, Source, CHARACTER_PREFIX};
use crate::{ModValue, Problem};

/// The simple keys, in the order the documentation lists them.
pub const KEYS: &[&str] = &["look", "skin", "health", "name", "voice", "weapon", "team", "behaviour", "can_die"];

/// Class of a character that "stands": a story character of the game, calm, that stays where it
/// is (what the cinematic characters of the maps are).
pub const STANDS_CLASS: &str = "XIDCine.Cine2";
/// Class of the characters that "patrol" or "fight": the game's soldier and its AI.
pub const SOLDIER_CLASS: &str = "XIDPawn.BaseSoldier";
/// Ammo count of a `weapon` (InitialInventory's Count): the soldiers of the maps carry 2.
pub const WEAPON_COUNT: i64 = 2;

/// What a character does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    Stands,
    Patrols,
    Fights,
}

impl Behaviour {
    pub fn class(self) -> &'static str {
        match self {
            Behaviour::Stands => STANDS_CLASS,
            Behaviour::Patrols | Behaviour::Fights => SOLDIER_CLASS,
        }
    }

    /// The team it implies when none is written: a standing character is a friend, a fighting
    /// one an enemy; a patrolling soldier keeps the game's own alliances.
    pub fn team(self) -> Option<Team> {
        match self {
            Behaviour::Stands => Some(Team::Friend),
            Behaviour::Patrols => None,
            Behaviour::Fights => Some(Team::Enemy),
        }
    }
}

/// The simple keys of one table, checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimpleKeys {
    /// Model ("XIIIPersos.JonesM"), resolved.
    pub look: Option<String>,
    pub skin: Option<String>,
    pub health: Option<i32>,
    pub name: Option<String>,
    /// CodeMesh, resolved.
    pub voice: Option<i32>,
    /// Weapon class ("XIII.Beretta"), resolved.
    pub weapon: Option<String>,
    pub team: Option<Team>,
    pub behaviour: Option<Behaviour>,
    pub can_die: Option<bool>,
}

/// Friendly names of the weapons a character can carry or the player be given: (class, names),
/// the first name suggested. A class name ("XIII.Beretta") is always accepted as well.
pub const WEAPON_NAMES: &[(&str, &[&str])] = &[
    ("XIII.Beretta", &["Pistol", "Beretta", "9mm"]),
    ("XIII.magnum", &["Magnum", "44Special", "Revolver"]),
    ("XIII.FusilChasse", &["HuntingRifle", "HuntingGun"]),
    ("XIII.FusilPompe", &["Shotgun", "PumpGun"]),
    ("XIII.FusilSnipe", &["SniperRifle", "Sniper"]),
    ("XIII.m16", &["AssaultRifle", "M16"]),
    ("XIII.kalash", &["Kalashnikov", "Kalash", "AK47"]),
    ("XIII.m60", &["MachineGun", "M60"]),
    ("XIII.MiniUzi", &["Uzi", "MiniUzi"]),
    ("XIII.arbalete", &["Crossbow"]),
    ("XIII.ArbaleteX3", &["CrossbowX3", "TripleCrossbow"]),
    ("XIII.Bazook", &["Bazooka", "RocketLauncher"]),
    ("XIII.LHarpon", &["HarpoonGun", "Harpoon"]),
    ("XIII.TKnife", &["ThrowingKnife", "Knife"]),
    ("XIII.Fists", &["Fists"]),
];

/// The weapon class `name` names: a class ("Package.Class") as written, else a friendly name
/// (case ignored). Err: the closest friendly names.
pub fn resolve_weapon(name: &str) -> Result<String, Vec<String>> {
    let name = name.trim();
    if name.contains('.') {
        return Ok(name.to_string());
    }
    for (class, names) in WEAPON_NAMES {
        let short = class.rsplit('.').next().unwrap_or(class);
        if short.eq_ignore_ascii_case(name) || names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Ok(class.to_string());
        }
    }
    Err(looks::closest(name, WEAPON_NAMES.iter().flat_map(|(_, n)| n.iter().copied()), 3))
}

/// The friendly name of weapon class `class`, if it has one.
pub fn weapon_name(class: &str) -> Option<&'static str> {
    WEAPON_NAMES.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)).map(|(_, n)| n[0])
}

/// Voices (CodeMesh values) by name: the names of the game's skin list (MeshSkinList's
/// SkinReadableName, with its CodeMesh), first name suggested.
pub const VOICE_NAMES: &[(i32, &[&str])] = &[
    (9, &["XIII"]),
    (6, &["Densetsu"]),
    (8, &["Armael"]),
    (13, &["John"]),
    (14, &["Shinyaku"]),
    (16, &["Carrington"]),
    (27, &["Mongoose"]),
    (28, &["Robert"]),
    (29, &["Trystan"]),
    (34, &["Roger"]),
];

/// The voice the maps give the characters of each look (model, CodeMesh): the value most of
/// the characters placed with that model carry, measured on every map of the game. 9 is the
/// soldiers' default (BaseSoldier's CodeMesh): another value of the same look wins over it.
pub const LOOK_VOICES: &[(&str, i32)] = &[
    ("XIIIPersos.AmosM", 15),
    ("XIIIPersos.Bank1M", 18),
    ("XIIIPersos.Bank2M", 18),
    ("XIIIPersos.CarringtonM", 16),
    ("XIIIPersos.CorsaireM", 22),
    ("XIIIPersos.DocM", 9),
    ("XIIIPersos.Employee1M", 21),
    ("XIIIPersos.Employee2M", 21),
    ("XIIIPersos.FBI2M", 3),
    ("XIIIPersos.FbiM", 3),
    ("XIIIPersos.FouM", 19),
    ("XIIIPersos.GalbrainM", 23),
    ("XIIIPersos.GardienM", 8),
    ("XIIIPersos.GiM", 5),
    ("XIIIPersos.GroflicM", 8),
    ("XIIIPersos.JessicaM", 20),
    ("XIIIPersos.JohanssonM", 25),
    // A tie in the maps (8 once, 26 once): 26, the voice of her two other looks.
    ("XIIIPersos.JonesM", 26),
    ("XIIIPersos.KKKM", 6),
    ("XIIIPersos.KimM", 20),
    ("XIIIPersos.MangousteM", 27),
    ("XIIIPersos.McCallM", 28),
    ("XIIIPersos.MecanoM", 10),
    ("XIIIPersos.MiocheM", 13),
    ("XIIIPersos.PachaM", 29),
    ("XIIIPersos.PamM", 35),
    ("XIIIPersos.QuinnM", 26),
    ("XIIIPersos.SMM", 10),
    ("XIIIPersos.SheridanM", 30),
    ("XIIIPersos.SoireeM", 26),
    // 26 for Jones in disguise; the SPADS soldiers keep BaseSoldier's 9.
    ("XIIIPersos.SpadsM", 9),
    ("XIIIPersos.StandKM", 31),
    ("XIIIPersos.StandwellM", 31),
    ("XIIIPersos.Tueur1M", 11),
    ("XIIIPersos.Tueur2M", 12),
    ("XIIIPersos.Tueur3M", 13),
    ("XIIIPersos.Tueur4M", 14),
    ("XIIIPersos.Tueur5M", 14),
    ("XIIIPersos.Tueur6M", 11),
    ("XIIIPersos.WaxM", 32),
    ("XIIIPersos.WigM", 2),
    ("XIIIPersos.WillardM", 33),
    ("XIIIPersos.WinslowM", 34),
    ("XIIIPersos.XIIIM", 8),
    ("XIIIPersos.XIIIMilitM", 9),
    ("XIIIPersos.ratM", 36),
    ("XIIIPersosG.JonesMajM", 26),
    ("XIIIPersosG.MigM", 1),
];

/// The voice of look `model` in the game, if the characters of that model have one.
pub fn look_voice(model: &str) -> Option<i32> {
    LOOK_VOICES.iter().find(|(m, _)| m.eq_ignore_ascii_case(model)).map(|(_, v)| *v).or_else(|| {
        // MeshSkinList pairs a model and a voice for the multiplayer skins.
        const SKINS: &[(&str, i32)] = &[
            ("XIIIPersos.XIIIMilitM", 9),
            ("XIIIPersos.DanhsuM", 14),
            ("XIIIPersos.ScandiM", 29),
            ("XIIIPersos.CarringtonM", 16),
            ("XIIIPersos.GaminM", 13),
            ("XIIIPersos.NiheiM", 6),
            ("XIIIPersos.FrenchyM", 34),
            ("XIIIPersos.Frenchy2M", 28),
        ];
        SKINS.iter().find(|(m, _)| m.eq_ignore_ascii_case(model)).map(|(_, v)| *v)
    })
}

/// The CodeMesh `name` names: a number, a voice name, else a look name (the voice of that look).
pub fn resolve_voice(name: &str) -> Result<i32, Vec<String>> {
    let name = name.trim();
    if let Ok(n) = name.parse::<i32>() {
        return Ok(n);
    }
    if let Some((v, _)) = VOICE_NAMES.iter().find(|(_, n)| n.iter().any(|x| x.eq_ignore_ascii_case(name))) {
        return Ok(*v);
    }
    if let Some(v) = looks::resolve(name).ok().filter(|_| !name.contains('.')).and_then(|m| look_voice(&m)) {
        return Ok(v);
    }
    let with_voice = looks::CHARACTER_LOOKS.iter().filter(|(m, _)| look_voice(m).is_some()).flat_map(|(_, n)| n.iter().copied());
    Err(looks::closest(name, VOICE_NAMES.iter().flat_map(|(_, n)| n.iter().copied()).chain(with_voice), 3))
}

/// The look's first friendly name ("Jones" for XIIIPersos.JonesM).
pub fn look_name(model: &str) -> Option<&'static str> {
    looks::CHARACTER_LOOKS.iter().find(|(m, _)| m.eq_ignore_ascii_case(model)).map(|(_, n)| n[0])
}

fn text(t: &toml::Table, key: &str) -> Result<Option<String>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Some(v) => Err(format!("{key} = {v} must be a name in quotes")),
    }
}

/// The team `s` names.
pub fn team(s: &str) -> Result<Team, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "friend" => Ok(Team::Friend),
        "neutral" => Ok(Team::Neutral),
        "enemy" => Ok(Team::Enemy),
        _ => Err(format!("team = {s:?} must be friend, neutral or enemy")),
    }
}

/// Reads the simple keys of `t` (the other keys are left to the caller). An unknown name gives
/// the closest valid names.
pub fn read(t: &toml::Table) -> Result<SimpleKeys, String> {
    let mut k = SimpleKeys::default();
    if let Some(l) = text(t, "look")? {
        k.look = Some(
            looks::resolve(&l)
                .map_err(|close| format!("look = \"{l}\" is not a character model{} (docs/modding/names/characters.md, or the names catalog)", looks::did_you_mean(&close)))?,
        );
    }
    if let Some(s) = text(t, "skin")? {
        if !s.contains('.') {
            return Err(format!("skin = \"{s}\" must be a material \"Package.Group.Name\" or a texture \"Mod.<Name>\""));
        }
        k.skin = Some(s);
    }
    k.health = match t.get("health") {
        None => None,
        Some(toml::Value::Integer(h)) if (1..=i32::MAX as i64).contains(h) => Some(*h as i32),
        Some(v) => return Err(format!("health = {v} must be a whole number, 1 or more")),
    };
    k.name = text(t, "name")?;
    k.voice = match t.get("voice") {
        None => None,
        Some(toml::Value::Integer(n)) if (0..=i32::MAX as i64).contains(n) => Some(*n as i32),
        Some(toml::Value::String(s)) => Some(resolve_voice(s).map_err(|close| format!("voice = \"{s}\" is not a voice{}", looks::did_you_mean(&close)))?),
        Some(v) => return Err(format!("voice = {v} must be a voice name such as \"Carrington\"")),
    };
    if let Some(w) = text(t, "weapon")? {
        k.weapon = Some(
            resolve_weapon(&w)
                .map_err(|close| format!("weapon = \"{w}\" is not a weapon{} (docs/modding/names/weapons.md, or the names catalog)", looks::did_you_mean(&close)))?,
        );
    }
    k.team = text(t, "team")?.map(|s| team(&s)).transpose()?;
    k.behaviour = match text(t, "behaviour")?.map(|s| s.to_ascii_lowercase()) {
        None => None,
        Some(s) if s == "stands" => Some(Behaviour::Stands),
        Some(s) if s == "patrols" => Some(Behaviour::Patrols),
        Some(s) if s == "fights" => Some(Behaviour::Fights),
        Some(s) => {
            let close = looks::closest(&s, ["stands", "patrols", "fights"], 1);
            return Err(format!("behaviour = \"{s}\" must be stands, patrols or fights{}", looks::did_you_mean(&close)));
        }
    };
    k.can_die = match t.get("can_die") {
        None => None,
        Some(toml::Value::Boolean(b)) => Some(*b),
        Some(v) => return Err(format!("can_die = {v} must be true or false")),
    };
    Ok(k)
}

impl SimpleKeys {
    pub fn is_empty(&self) -> bool {
        *self == SimpleKeys::default()
    }

    /// The raw values these keys stand for, in this order: Mesh, Skins, Health, PawnName,
    /// CodeMesh, InitialInventory[0].
    pub fn values(&self) -> Vec<ActorValue> {
        let mut out = Vec::new();
        let mut push = |property: &str, index: Option<usize>, value: ModValue| out.push(ActorValue { property: property.into(), index, value });
        if let Some(l) = &self.look {
            push("Mesh", None, ModValue::Str(l.clone()));
        }
        if let Some(s) = &self.skin {
            // Skins is a dynamic array: the whole list, with this one skin.
            push("Skins", None, ModValue::Array(vec![ModValue::Str(s.clone())]));
        }
        if let Some(h) = self.health {
            push("Health", None, ModValue::Int(h as i64));
        }
        if let Some(n) = &self.name {
            push("PawnName", None, ModValue::Str(n.clone()));
        }
        if let Some(v) = self.voice {
            push("CodeMesh", None, ModValue::Int(v as i64));
        }
        if let Some(w) = &self.weapon {
            push(
                "InitialInventory",
                Some(0),
                ModValue::Table(vec![("Inventory".into(), ModValue::Str(format!("Class'{w}'"))), ("Count".into(), ModValue::Int(WEAPON_COUNT))]),
            );
        }
        out
    }

    /// These keys with what a look implies when not written: the voice the game gives that
    /// look (look_voice).
    pub fn with_defaults(&self) -> SimpleKeys {
        let mut k = self.clone();
        if k.voice.is_none() {
            k.voice = k.look.as_deref().and_then(look_voice);
        }
        // An enemy attacks: a story character (stands) has no fighting AI, so `team = "enemy"`
        // alone makes a soldier that fights.
        if k.behaviour.is_none() && k.team == Some(Team::Enemy) {
            k.behaviour = Some(Behaviour::Fights);
        }
        k
    }

    /// The keys of `other` given, over these.
    pub fn over(&self, other: &SimpleKeys) -> SimpleKeys {
        SimpleKeys {
            look: other.look.clone().or_else(|| self.look.clone()),
            skin: other.skin.clone().or_else(|| self.skin.clone()),
            health: other.health.or(self.health),
            name: other.name.clone().or_else(|| self.name.clone()),
            voice: other.voice.or(self.voice),
            weapon: other.weapon.clone().or_else(|| self.weapon.clone()),
            team: other.team.or(self.team),
            behaviour: other.behaviour.or(self.behaviour),
            can_die: other.can_die.or(self.can_die),
        }
    }
}

/// The character a look name stands for when no mod has a `[character.<Name>]` of that name:
/// a calm story character (XIDCine.Cine2, a friend) with that look, the voice the game gives
/// it, and the look's name as its in-game name (PawnName).
pub fn look_character(name: &str, model: &str, source: Source) -> CharacterDef {
    let keys = SimpleKeys { look: Some(model.to_string()), name: Some(look_name(model).unwrap_or(name).to_string()), ..Default::default() }.with_defaults();
    CharacterDef {
        name: name.to_string(),
        base: STANDS_CLASS.to_string(),
        team: Some(Team::Friend),
        allowed_states: Vec::new(),
        can_die: true,
        death_animation: None,
        properties: keys.values(),
        states: Vec::new(),
        source,
        keys,
        like: None,
    }
}

/// Once every mod is read: the characters named by `add` entries and `spawn` actions that are
/// look names without a `[character.<Name>]` table become [`look_character`]s, and each derived
/// definition (an entry's simple keys) gets the character it starts from, with its keys over
/// it: `behaviour` sets the class, `team` and `can_die` the behaviour, the other keys values
/// written after the character's own (raw `properties` of the entry come after them all).
/// An unknown name gives one warning with the closest names; the game then skips the entry.
pub fn resolve(edits: &[MapEdit], actions: &[ModAction], characters: &mut Vec<CharacterDef>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut wanted: Vec<(String, Source)> = Vec::new();
    for e in edits {
        if let Edit::Add { class, .. } = &e.edit {
            if let Some(n) = class.strip_prefix(CHARACTER_PREFIX) {
                wanted.push((n.to_string(), e.source.clone()));
            }
        }
        // The characters `spawn` actions of when blocks make.
        if let Edit::When(w) = &e.edit {
            wanted.extend(w.spawns.iter().map(|n| (n.clone(), e.source.clone())));
        }
    }
    for a in actions {
        if let ActionKind::Spawn { what, .. } = &a.kind {
            if !what.contains('.') {
                wanted.push((what.clone(), a.source.clone()));
            }
        }
    }
    for c in characters.iter() {
        if let Some(l) = c.like.as_ref().filter(|l| !l.is_empty()) {
            wanted.push((l.clone(), c.source.clone()));
        }
    }
    for (n, source) in wanted {
        if n.contains('@') || characters.iter().any(|c| c.name.eq_ignore_ascii_case(&n)) {
            continue;
        }
        match looks::resolve(&n) {
            Ok(model) if !n.contains('.') => characters.push(look_character(&n, &model, source)),
            _ => {
                let names: Vec<String> = characters.iter().filter(|c| c.like.is_none() && !c.name.contains('@')).map(|c| c.name.clone()).collect();
                let mut all: Vec<&str> = looks::look_names().collect();
                all.extend(names.iter().map(String::as_str));
                let close = looks::closest(&n, all, 3);
                problems.push(Problem::Warning {
                    path: source.file.clone(),
                    message: format!(
                        "line {}: character {n:?} is neither a character name (docs/modding/names/characters.md, or the names catalog) nor a [character.{n}] of the mods{}",
                        source.line,
                        looks::did_you_mean(&close)
                    ),
                });
            }
        }
    }
    for i in 0..characters.len() {
        let Some(like) = characters[i].like.clone().filter(|l| !l.is_empty()) else { continue };
        let Some(base) = characters.iter().find(|c| c.like.is_none() && c.name.eq_ignore_ascii_case(&like)).cloned() else { continue };
        let d = &mut characters[i];
        let keys = d.keys.with_defaults();
        d.base = keys.behaviour.map_or(base.base.clone(), |b| b.class().to_string());
        d.team = keys.team.or_else(|| keys.behaviour.and_then(Behaviour::team)).or(base.team);
        d.can_die = keys.can_die.unwrap_or(base.can_die);
        d.allowed_states = base.allowed_states.clone();
        d.death_animation = base.death_animation.clone();
        d.states = base.states.clone();
        let mut properties = base.properties.clone();
        properties.extend(keys.values());
        d.properties = properties;
        d.keys = base.keys.over(&keys);
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(s: &str) -> toml::Table {
        s.parse().unwrap()
    }

    /// The characters of a mod file after the game's resolving (look names, simple keys).
    fn resolved_chars(text: &str) -> Vec<CharacterDef> {
        let mut f = crate::defaults::parse_mod_file(text, std::path::Path::new("mod.toml"), "m");
        assert!(f.problems.is_empty(), "{:?}", f.problems);
        let problems = resolve(&f.edits, &f.actions, &mut f.characters);
        assert!(problems.is_empty(), "{problems:?}");
        f.characters
    }

    /// `team = "enemy"` alone makes a soldier that fights (a story character cannot); another
    /// team keeps the calm story character; an explicit behaviour wins.
    #[test]
    fn an_enemy_fights() {
        let base = |text: &str| resolved_chars(text).into_iter().find(|c| c.like.is_some()).map(|c| (c.base, c.team)).unwrap();
        let add = |keys: &str| format!("[[map.Plage01.add]]\ncharacter = \"Jones\"\nposition = [0, 0, 0]\n{keys}\n");
        assert_eq!(base(&add("team = \"enemy\"")), (SOLDIER_CLASS.to_string(), Some(Team::Enemy)));
        assert_eq!(base(&add("team = \"neutral\"")), (STANDS_CLASS.to_string(), Some(Team::Neutral)));
        assert_eq!(base(&add("team = \"enemy\"\nbehaviour = \"stands\"")).0, STANDS_CLASS);
        // A when block's spawn with settings: the same derived character, named as the spawn.
        let chars = resolved_chars("[[map.Plage01.when]]\nevent = \"key\"\nkey = \"K\"\nthen = [ { spawn = \"Guard\", position = [0, 0, 0], team = \"enemy\", weapon = \"Pistol\" } ]\n");
        let guard = chars.iter().find(|c| c.like.as_deref() == Some("Guard")).expect("derived guard");
        assert_eq!((guard.base.as_str(), guard.team), (SOLDIER_CLASS, Some(Team::Enemy)));
        assert!(guard.name.starts_with("Guard@mod.toml:"), "{}", guard.name);
        assert_eq!(guard.keys.name.as_deref(), Some("Guard"), "found by who = \"Guard\"");
    }

    #[test]
    fn names_resolve_with_did_you_mean() {
        assert_eq!(resolve_weapon("pistol").as_deref(), Ok("XIII.Beretta"));
        assert_eq!(resolve_weapon("Beretta").as_deref(), Ok("XIII.Beretta"));
        assert_eq!(resolve_weapon("XIII.Bazook").as_deref(), Ok("XIII.Bazook"));
        assert!(resolve_weapon("Shotgnu").unwrap_err().iter().any(|n| n == "Shotgun"));
        assert_eq!(resolve_voice("Carrington"), Ok(16));
        assert_eq!(resolve_voice("12"), Ok(12));
        assert!(resolve_voice("Nobody").is_err());
        assert_eq!(weapon_name("XIII.FusilSnipe"), Some("SniperRifle"));
    }

    #[test]
    fn keys_become_values() {
        let k = read(&table(
            r#"look = "Jones"
health = 300
name = "Major"
voice = "Carrington"
weapon = "Shotgun"
team = "enemy"
behaviour = "Fights"
can_die = false"#,
        ))
        .unwrap();
        assert_eq!(k.look.as_deref(), Some("XIIIPersos.JonesM"));
        assert_eq!((k.team, k.behaviour, k.can_die), (Some(Team::Enemy), Some(Behaviour::Fights), Some(false)));
        let v = k.values();
        let names: Vec<&str> = v.iter().map(|a| a.property.as_str()).collect();
        assert_eq!(names, ["Mesh", "Health", "PawnName", "CodeMesh", "InitialInventory"]);
        assert_eq!(v[4].index, Some(0));
        assert_eq!(v[4].value, ModValue::Table(vec![("Inventory".into(), ModValue::Str("Class'XIII.FusilPompe'".into())), ("Count".into(), ModValue::Int(2))]));
        // Errors name the closest valid values.
        for (bad, hint) in [
            ("look = \"Jnoes\"", "look"),
            ("weapon = \"Shotgnu\"", "Shotgun"),
            ("behaviour = \"stand\"", "stands"),
            ("team = \"ally\"", "friend"),
            ("health = 0", "1 or more"),
        ] {
            let why = read(&table(bad)).unwrap_err();
            assert!(why.contains(hint), "{bad}: {why}");
        }
        assert!(read(&table("x = 1")).unwrap().is_empty());
    }

    fn resolved(text: &str) -> (crate::defaults::ModFile, Vec<Problem>) {
        let mut f = crate::defaults::parse_mod_file(text, std::path::Path::new("m/mod.toml"), "m");
        let problems = resolve(&f.edits, &f.actions, &mut f.characters);
        (f, problems)
    }

    fn def<'a>(f: &'a crate::defaults::ModFile, name: &str) -> &'a CharacterDef {
        f.characters.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name}: {:?}", f.characters.iter().map(|c| &c.name).collect::<Vec<_>>()))
    }

    fn value<'a>(d: &'a CharacterDef, property: &str) -> Option<&'a ModValue> {
        // The last value written wins.
        d.properties.iter().rev().find(|a| a.property == property).map(|a| &a.value)
    }

    /// `character = "Jones"` alone makes a calm Jones; simple keys of an entry go over it; a
    /// `[character]` table of the same name wins over the look name; raw properties win.
    #[test]
    fn look_names_and_derived_characters() {
        let (f, problems) = resolved(
            r#"
[[map.Plage01.add]]
character = "Jones"
position = [0, 0, 0]

[[map.Plage01.add]]
character = "Jones"
position = [100, 0, 0]
weapon = "Pistol"
behaviour = "fights"
health = 500
properties = { Health = 600 }

[[action]]
key = "K"
spawn = "Carrington"
team = "neutral"
"#,
        );
        assert!(problems.is_empty() && f.problems.is_empty(), "{problems:?} {:?}", f.problems);
        let jones = def(&f, "Jones");
        assert_eq!((jones.base.as_str(), jones.team, jones.like.as_deref()), (STANDS_CLASS, Some(Team::Friend), None));
        assert_eq!(value(jones, "Mesh"), Some(&ModValue::Str("XIIIPersos.JonesM".into())));
        assert_eq!(value(jones, "PawnName"), Some(&ModValue::Str("Jones".into())));
        // The second entry: its own derived definition, a fighting soldier with a pistol.
        let Edit::Add { class, properties, .. } = &f.edits[1].edit else { panic!() };
        assert_eq!(class, "Character.Jones@mod.toml:6");
        let fighter = def(&f, "Jones@mod.toml:6");
        assert_eq!((fighter.base.as_str(), fighter.team), (SOLDIER_CLASS, Some(Team::Enemy)));
        assert_eq!(value(fighter, "Mesh"), Some(&ModValue::Str("XIIIPersos.JonesM".into())));
        assert_eq!(value(fighter, "Health"), Some(&ModValue::Int(500)));
        assert!(matches!(value(fighter, "InitialInventory"), Some(ModValue::Table(t)) if t[0].1 == ModValue::Str("Class'XIII.Beretta'".into())));
        // The entry's raw properties stay on the entry: written after the character's values.
        assert_eq!(properties[0].value, ModValue::Int(600));
        // The spawn: Carrington, neutral, his own voice (MeshSkinList's CodeMesh 16).
        let ActionKind::Spawn { what, .. } = &f.actions[0].kind else { panic!() };
        let carrington = def(&f, what);
        assert_eq!(carrington.team, Some(Team::Neutral));
        assert_eq!(value(carrington, "CodeMesh"), Some(&ModValue::Int(16)));

        // A table of the same name wins over the look name.
        let (f, problems) = resolved(
            r#"
[character.Jones]
base = "XIDPawn.BaseSoldier"
properties = { Mesh = "XIIIPersos.SpadsM" }

[[map.Plage01.add]]
character = "Jones"
position = [0, 0, 0]
"#,
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(f.characters.len(), 1);
        assert_eq!(def(&f, "Jones").base, "XIDPawn.BaseSoldier");

        // Negative controls: an unknown name gives one warning with the closest names, and a
        // simple key without a character is refused.
        let (f, problems) = resolved(
            r#"
[[map.Plage01.add]]
character = "Jomes"
position = [0, 0, 0]

[[map.Plage01.add]]
class = "XIII.MedPick"
position = [0, 0, 0]
look = "Jones"
"#,
        );
        assert!(f.characters.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].to_string().contains("did you mean Jones"), "{}", problems[0]);
        assert!(f.problems.iter().any(|p| p.to_string().contains("go with character")), "{:?}", f.problems);
    }

    /// `[[map.<Map>.change]]`: name, voice and weapon become values before the raw properties;
    /// team and can_die a derived character; behaviour is refused.
    #[test]
    fn change_keys() {
        let (f, _) = resolved(
            r#"
[[map.PRock03.change]]
who = "gardien_lostpass"
name = "Major"
weapon = "Shotgun"
team = "enemy"
can_die = false
properties = { PawnName = "Raw" }

[[map.PRock03.change]]
who = "gardien_lostpass"
behaviour = "fights"
"#,
        );
        let Edit::Change(c) = &f.edits[0].edit else { panic!() };
        let names: Vec<&str> = c.properties.iter().map(|p| p.property.as_str()).collect();
        assert_eq!(names, ["PawnName", "InitialInventory", "PawnName"]);
        let d = def(&f, c.character.as_deref().unwrap());
        assert_eq!((d.team, d.can_die), (Some(Team::Enemy), false));
        assert_eq!(f.edits.len(), 1);
        assert!(f.problems.iter().any(|p| p.to_string().contains("behaviour cannot be changed")), "{:?}", f.problems);
    }
}
