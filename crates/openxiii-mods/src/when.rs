//! `[[map.<Map>.when]]`: "when ... then ..." blocks. An event (`event = "near"` and its own
//! settings), then a list of actions in order (`then = [ { say = "...", by = "Pam" }, ... ]`),
//! and optionally `run`: a function of the mod's own scripts. The game runs them with its
//! scripts' engine (openxiii-vm mod_when.rs): a block does what the script that says the same
//! would do, the actions being the scripts' own functions.
//!
//! ```toml
//! [[map.Plage01.when]]
//! event = "near"
//! who = "Pam"
//! distance = 250
//! once = true
//! then = [
//!   { say = "Plage01_Pam_03", by = "actor" },
//!   { give = "Shotgun" },
//! ]
//! ```
//!
//! The words "player", "actor", "amount" and "class" as a value stand for what the event
//! gave (the player near Pam, Pam herself...). Errors (an unknown event, action or setting)
//! skip the block with one message, closest names given.

use crate::looks::{closest, did_you_mean};

/// What starts a block.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start,
    Near { who: String, distance: Option<f32> },
    Enter { position: [f32; 3], radius: Option<f32> },
    Sees { who: String },
    Death { who: String },
    Damage { who: String },
    /// The shots of `who` ("all": every character), or of the players.
    Fire { who: Option<String> },
    Pickup,
    Event { name: String },
    Key { key: String },
    After { seconds: f32 },
    Every { seconds: f32 },
}

/// A plain value of an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
    List(Vec<Value>),
    Table(Vec<(String, Value)>),
}

/// An argument of an action: a value, or what the event gave.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Value(Value),
    Player,
    Actor,
    Amount,
    Class,
}

/// An action: a function of the scripts' API by its name, with its arguments in the API's
/// order (a method of the actors takes the actor first).
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub name: String,
    pub args: Vec<Arg>,
}

/// One `[[map.<Map>.when]]` block.
#[derive(Debug, Clone, PartialEq)]
pub struct When {
    pub event: Event,
    pub actions: Vec<Action>,
    /// A global function of the mod's scripts, called after the actions.
    pub run: Option<String>,
    pub once: bool,
    /// Characters made by `spawn` actions without settings of their own (simple.rs resolves
    /// look names).
    pub spawns: Vec<String>,
    /// `spawn` actions with settings of a character (team, behaviour, weapon...): (the action's
    /// index, the character, its settings), made derived characters by placement.rs.
    pub spawn_keys: Vec<(usize, String, toml::Table)>,
}

/// An event: its word, its settings (required first, with the optional ones after `|`), what
/// it gives, and when it happens (for the docs).
pub struct EventSpec {
    pub name: &'static str,
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
    pub gives: &'static [&'static str],
    pub about: &'static str,
    pub example: &'static str,
}

pub const EVENTS: &[EventSpec] = &[
    EventSpec { name: "start", required: &[], optional: &[], gives: &["player"], about: "XIII can move: the level has started and its opening scene is over", example: "event = \"start\"" },
    EventSpec {
        name: "near",
        required: &["who"],
        optional: &["distance"],
        gives: &["player", "actor"],
        about: "a player comes within distance (300 by default) of a character; not during a cut-scene",
        example: "event = \"near\", who = \"Pam\", distance = 250",
    },
    EventSpec {
        name: "enter",
        required: &["position"],
        optional: &["radius"],
        gives: &["player"],
        about: "a player comes within radius (300 by default) of a point on the ground",
        example: "event = \"enter\", position = [-470, -860, 1190], radius = 200",
    },
    EventSpec { name: "sees", required: &["who"], optional: &[], gives: &["actor", "player"], about: "a character sees a player, with its own eyes", example: "event = \"sees\", who = \"Guard\"" },
    EventSpec { name: "death", required: &["who"], optional: &[], gives: &["actor"], about: "a character dies (who killed it is not known)", example: "event = \"death\", who = \"Visitor\"" },
    EventSpec { name: "damage", required: &["who"], optional: &[], gives: &["actor", "amount"], about: "a character loses health", example: "event = \"damage\", who = \"Visitor\"" },
    EventSpec {
        name: "fire",
        required: &[],
        optional: &["who"],
        gives: &["actor", "class"],
        about: "each shot of a player's gun (without who), of who, or of every character (who = \"all\"): actor is who fired, class the weapon's",
        example: "event = \"fire\"",
    },
    EventSpec { name: "pickup", required: &[], optional: &[], gives: &["player", "class"], about: "an item comes into a player's inventory", example: "event = \"pickup\"" },
    EventSpec {
        name: "event",
        required: &["name"],
        optional: &[],
        gives: &["player"],
        about: "the level fires an event (a trigger, a door, a scene: events.md of the names)",
        example: "event = \"event\", name = \"Objectif91\"",
    },
    EventSpec { name: "key", required: &["key"], optional: &[], gives: &["player"], about: "a player presses a keyboard key (F1-F12, A-Z, 0-9)", example: "event = \"key\", key = \"F9\"" },
    EventSpec { name: "after", required: &["seconds"], optional: &[], gives: &[], about: "once, that many seconds after XIII can move (after the opening scene)", example: "event = \"after\", seconds = 30" },
    EventSpec { name: "every", required: &["seconds"], optional: &[], gives: &[], about: "again and again, every that many seconds once XIII can move", example: "event = \"every\", seconds = 10" },
];

/// An action as written in a `then` list: its word (with its main value), its other settings,
/// what it does and an example (for the docs and the closest names).
pub struct ActionSpec {
    pub name: &'static str,
    /// What the action's own word takes.
    pub value: &'static str,
    pub settings: &'static [&'static str],
    pub about: &'static str,
    pub example: &'static str,
}

/// Every action of a `then` list, the functions of the scripts' API that do something (a test
/// of openxiii-vm checks they are the same as its list).
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec { name: "say", value: "a line of the game (dialogues.md) or your own subtitle", settings: &["by"], about: "a character says a line; by: who says it (the event's character, else the player)", example: "{ say = \"Plage01_Pam_03\", by = \"Pam\" }" },
    ActionSpec { name: "message", value: "a text", settings: &[], about: "shows a message on the screen of every player", example: "{ message = \"The guard is coming.\" }" },
    ActionSpec { name: "objective", value: "a text", settings: &[], about: "shows a new goal, as the game's goals", example: "{ objective = \"Hold the beach\" }" },
    ActionSpec { name: "objective_done", value: "a text", settings: &[], about: "shows a goal as reached", example: "{ objective_done = \"Hold the beach\" }" },
    ActionSpec { name: "give", value: "a weapon (weapons.md) or a class", settings: &["to"], about: "gives a weapon or an item; to: who gets it (the player), who keeps what he holds", example: "{ give = \"Shotgun\" }" },
    ActionSpec {
        name: "spawn",
        value: "a character (characters.md, or a [character.<Name>]) or a class",
        settings: &["position", "yaw", "team", "behaviour", "weapon", "health", "look", "skin", "name", "voice", "can_die"],
        about: "makes a character or an object appear, standing on position; team = \"enemy\" makes one that attacks",
        example: "{ spawn = \"Guard\", position = [-470, -860, 1190], team = \"enemy\" }",
    },
    ActionSpec { name: "remove", value: "who", settings: &[], about: "takes an actor out of the level", example: "{ remove = \"MedPick0\" }" },
    ActionSpec {
        name: "change",
        value: "who",
        settings: &["look", "health", "team", "can_die", "weapon", "name", "voice", "skin", "properties"],
        about: "changes a character now, with the settings of [[map.<Map>.change]]",
        example: "{ change = \"actor\", look = \"Jones\", health = 300 }",
    },
    ActionSpec { name: "trigger", value: "an event of the level (events.md)", settings: &["by"], about: "fires an event of the level: opens its doors, starts its movers and scenes", example: "{ trigger = \"PorteCabane\" }" },
    ActionSpec { name: "sound", value: "a sound (sounds.md) or Mod.<Name>", settings: &["at"], about: "plays a sound; at: an actor or a position (the player)", example: "{ sound = \"Mod.Alarm\", at = \"actor\" }" },
    ActionSpec { name: "music", value: "a music (music.md)", settings: &[], about: "plays a music of the game", example: "{ music = \"XIIIMusic.Combat.hCombat1\" }" },
    ActionSpec { name: "next_level", value: "a map, or true for the next one of the mod's campaign", settings: &[], about: "ends the mission as won and goes on to that map", example: "{ next_level = \"Banque01\" }" },
    ActionSpec { name: "fail", value: "a text", settings: &[], about: "ends the mission as lost, with that text", example: "{ fail = \"The guard escaped.\" }" },
    ActionSpec { name: "log", value: "a text", settings: &[], about: "writes a line in mods-log.txt", example: "{ log = \"Pam was reached\" }" },
    ActionSpec { name: "camera", value: "a position", settings: &["look_at", "seconds"], about: "a scene: every player's view goes there, looking at look_at, for seconds", example: "{ camera = [-300, -900, 1300], look_at = \"actor\", seconds = 4 }" },
    ActionSpec { name: "camera_back", value: "true", settings: &[], about: "ends a scene: the players' own view comes back", example: "{ camera_back = true }" },
    ActionSpec { name: "go_to", value: "an actor or a position", settings: &["who", "run"], about: "a story character walks (run = true: runs) there", example: "{ go_to = [-470, -860, 1190], who = \"Pam\" }" },
    ActionSpec { name: "follow", value: "an actor", settings: &["who"], about: "a story character follows an actor", example: "{ follow = \"player\", who = \"Pam\" }" },
    ActionSpec { name: "attack", value: "an actor", settings: &["who"], about: "a soldier takes that actor as its enemy", example: "{ attack = \"player\", who = \"Guard1\" }" },
    ActionSpec { name: "look_at", value: "an actor or a position", settings: &["who"], about: "a character turns to look at it", example: "{ look_at = \"player\", who = \"Pam\" }" },
    ActionSpec { name: "stop", value: "who", settings: &[], about: "stops a go_to, a follow or a patrol", example: "{ stop = \"Pam\" }" },
    ActionSpec { name: "play", value: "an animation (animations.md)", settings: &["who", "loop"], about: "a character plays an animation", example: "{ play = \"WaitNeutre\", who = \"Pam\", loop = true }" },
    ActionSpec {
        name: "restart",
        value: "who",
        settings: &[],
        about: "a mods character's state look (its animation and its sound) starts again from the beginning",
        example: "{ restart = \"every Singer\" }",
    },
    ActionSpec { name: "teleport", value: "an actor or a position", settings: &["who"], about: "moves an actor there at once", example: "{ teleport = [-470, -860, 1190], who = \"Guard1\" }" },
    ActionSpec { name: "kill", value: "who", settings: &[], about: "the character dies, as from a gun shot", example: "{ kill = \"actor\" }" },
    ActionSpec { name: "behave", value: "a behaviour of the mod's scripts", settings: &["who"], about: "a character runs a behaviour defined by the mod's scripts", example: "{ behave = \"Bodyguard\", who = \"actor\" }" },
    ActionSpec { name: "patrol", value: "a list of positions", settings: &["who", "run"], about: "a character walks from point to point, again and again", example: "{ patrol = [[-470, -860, 1190], [-200, -860, 1190]], who = \"Guard1\" }" },
];

/// The settings of a block besides the event's own.
const BLOCK_SETTINGS: &[&str] = &["event", "once", "then", "run"];

/// The words that stand for what the event gave.
const ROLES: &[&str] = &["player", "actor", "amount", "class"];

impl When {
    /// "when near Pam then say, give (run reward)", for the log.
    pub fn describe(&self) -> String {
        let event = match &self.event {
            Event::Start => "start".to_string(),
            Event::Near { who, .. } => format!("near {who}"),
            Event::Enter { position, .. } => format!("enter [{}, {}, {}]", position[0], position[1], position[2]),
            Event::Sees { who } => format!("{who} sees a player"),
            Event::Death { who } => format!("death of {who}"),
            Event::Damage { who } => format!("damage to {who}"),
            Event::Fire { who: Some(who) } => format!("shot of {who}"),
            Event::Fire { who: None } => "a player's shot".to_string(),
            Event::Pickup => "pickup".to_string(),
            Event::Event { name } => format!("event {name}"),
            Event::Key { key } => format!("key {key}"),
            Event::After { seconds } => format!("after {seconds} s"),
            Event::Every { seconds } => format!("every {seconds} s"),
        };
        let actions: Vec<&str> = self.actions.iter().map(|a| a.name.as_str()).collect();
        let mut s = format!("when {event}");
        if !actions.is_empty() {
            s.push_str(&format!(" then {}", actions.join(", ")));
        }
        if let Some(r) = &self.run {
            s.push_str(&format!(" (run {r})"));
        }
        if self.once {
            s.push_str(" (once)");
        }
        s
    }
}

fn number(v: &toml::Value) -> Option<f32> {
    match v {
        toml::Value::Integer(i) => Some(*i as f32),
        toml::Value::Float(f) => Some(*f as f32),
        _ => None,
    }
}

fn position(v: &toml::Value) -> Option<[f32; 3]> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    Some([number(&a[0])?, number(&a[1])?, number(&a[2])?])
}

fn text<'a>(t: &'a toml::Table, key: &str) -> Result<&'a str, String> {
    match t.get(key) {
        Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(s.trim()),
        Some(v) => Err(format!("{key} = {v} must be a text in quotes")),
        None => Err(format!("{key} = \"...\" is missing")),
    }
}

fn seconds(t: &toml::Table, key: &str) -> Result<f32, String> {
    let v = t.get(key).ok_or_else(|| format!("{key} = <a number> is missing"))?;
    number(v).filter(|n| *n >= 0.0).ok_or_else(|| format!("{key} = {v} must be a number, 0 or more"))
}

fn optional_number(t: &toml::Table, key: &str) -> Result<Option<f32>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(v) => number(v).filter(|n| *n > 0.0).map(Some).ok_or_else(|| format!("{key} = {v} must be a number above 0")),
    }
}

/// A toml value as an action's plain value.
fn plain(v: &toml::Value) -> Result<Value, String> {
    Ok(match v {
        toml::Value::String(s) => Value::Text(s.clone()),
        toml::Value::Integer(i) => Value::Int(*i),
        toml::Value::Float(f) => Value::Number(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Array(a) => Value::List(a.iter().map(plain).collect::<Result<_, _>>()?),
        toml::Value::Table(t) => Value::Table(t.iter().map(|(k, x)| Ok((k.clone(), plain(x)?))).collect::<Result<_, String>>()?),
        toml::Value::Datetime(d) => return Err(format!("{d} is a date; write a text, a number or a list")),
    })
}

/// The event of the block being read, for the words that stand for what it gave.
struct Ctx<'a> {
    event: &'a EventSpec,
}

impl Ctx<'_> {
    /// A value of an action: "player", "actor", "amount" or "class" stand for what the event
    /// gave (an error if it gives no such thing).
    fn arg(&self, v: &toml::Value) -> Result<Arg, String> {
        if let toml::Value::String(s) = v {
            let word = s.trim().to_ascii_lowercase();
            if ROLES.contains(&word.as_str()) {
                // Every event has a player: the main player when it gives none.
                if word != "player" && !self.event.gives.contains(&word.as_str()) {
                    let gives = if self.event.gives.is_empty() { "nothing".to_string() } else { self.event.gives.join(", ") };
                    return Err(format!("\"{word}\": the event {} gives {gives}", self.event.name));
                }
                return Ok(match word.as_str() {
                    "player" => Arg::Player,
                    "actor" => Arg::Actor,
                    "amount" => Arg::Amount,
                    _ => Arg::Class,
                });
            }
        }
        Ok(Arg::Value(plain(v)?))
    }

    /// Who does an action when the block does not say: the event's character, else the player.
    fn default_who(&self) -> Arg {
        if self.event.gives.contains(&"actor") {
            Arg::Actor
        } else {
            Arg::Player
        }
    }

    fn who_or_default(&self, item: &toml::Table, key: &str) -> Result<Arg, String> {
        item.get(key).map_or(Ok(self.default_who()), |v| self.arg(v))
    }
}

/// Reads a `[[map.<Map>.when]]` block (without its `from_checkpoint`, read by placement.rs).
pub fn read(t: &toml::Table) -> Result<When, String> {
    let name = text(t, "event").map_err(|e| format!("{e} (events: {})", EVENTS.iter().map(|e| e.name).collect::<Vec<_>>().join(", ")))?;
    let Some(spec) = EVENTS.iter().find(|e| e.name.eq_ignore_ascii_case(name)) else {
        let close = closest(name, EVENTS.iter().map(|e| e.name), 3);
        let all = EVENTS.iter().map(|e| e.name).collect::<Vec<_>>().join(", ");
        return Err(format!("unknown event \"{name}\"{} (events: {all})", did_you_mean(&close)));
    };
    // Every setting: the block's own and its event's.
    let allowed: Vec<&str> = BLOCK_SETTINGS.iter().chain(spec.required).chain(spec.optional).copied().collect();
    for key in t.keys() {
        if !allowed.contains(&key.as_str()) {
            let close = closest(key, allowed.iter().copied(), 2);
            return Err(format!("unknown setting {key} for event \"{}\"{}", spec.name, did_you_mean(&close)));
        }
    }
    let event = match spec.name {
        "start" => Event::Start,
        "near" => Event::Near { who: text(t, "who")?.to_string(), distance: optional_number(t, "distance")? },
        "enter" => Event::Enter {
            position: t.get("position").and_then(position).ok_or("position = [x, y, z] is missing or is not three numbers")?,
            radius: optional_number(t, "radius")?,
        },
        "sees" => Event::Sees { who: text(t, "who")?.to_string() },
        "death" => Event::Death { who: text(t, "who")?.to_string() },
        "damage" => Event::Damage { who: text(t, "who")?.to_string() },
        "fire" => Event::Fire { who: if t.contains_key("who") { Some(text(t, "who")?.to_string()) } else { None } },
        "pickup" => Event::Pickup,
        "event" => Event::Event { name: text(t, "name")?.to_string() },
        "key" => Event::Key { key: text(t, "key")?.to_string() },
        "after" => Event::After { seconds: seconds(t, "seconds")? },
        _ => Event::Every { seconds: seconds(t, "seconds").and_then(|s| if s > 0.0 { Ok(s) } else { Err("seconds = 0: write a number above 0".to_string()) })? },
    };
    let once = match t.get("once") {
        None => false,
        Some(toml::Value::Boolean(b)) => *b,
        Some(v) => return Err(format!("once = {v} must be true or false")),
    };
    let run = match t.get("run") {
        None => None,
        Some(toml::Value::String(s)) if valid_function(s) => Some(s.trim().to_string()),
        Some(v) => return Err(format!("run = {v} must be the name of a function of the mod's scripts (letters, digits and _)")),
    };
    let ctx = Ctx { event: spec };
    let mut actions = Vec::new();
    let mut spawns = Vec::new();
    let mut spawn_keys = Vec::new();
    match t.get("then") {
        None => {}
        Some(toml::Value::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let toml::Value::Table(item) = item else {
                    return Err(format!("then: item {} must be {{ action = ... }}, for example {{ message = \"Hello\" }}", i + 1));
                };
                let a = action(&ctx, item).map_err(|e| format!("then: item {}: {e}", i + 1))?;
                if a.name == "spawn" {
                    if let Some(Arg::Value(Value::Table(fields))) = a.args.first() {
                        if let Some((_, Value::Text(c))) = fields.iter().find(|(k, _)| k == "character") {
                            let keys: toml::Table = item.iter().filter(|(k, _)| crate::simple::KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
                            if keys.is_empty() {
                                spawns.push(c.clone());
                            } else {
                                spawn_keys.push((actions.len(), c.clone(), keys));
                            }
                        }
                    }
                }
                actions.push(a);
            }
        }
        Some(v) => return Err(format!("then = {v} must be a list: then = [ {{ message = \"Hello\" }} ]")),
    }
    if actions.is_empty() && run.is_none() {
        return Err("nothing to do: give then = [ ... ] or run = \"function\"".into());
    }
    Ok(When { event, actions, run, once, spawns, spawn_keys })
}

fn valid_function(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One item of a `then` list.
fn action(ctx: &Ctx, item: &toml::Table) -> Result<Action, String> {
    let names: Vec<&str> = item.keys().map(String::as_str).filter(|k| ACTIONS.iter().any(|a| a.name == *k)).collect();
    // `look_at` is also a setting of camera.
    let name = match names.as_slice() {
        [one] => *one,
        [_, _] if names.contains(&"camera") && names.contains(&"look_at") => "camera",
        [] => {
            let keys: Vec<&str> = item.keys().map(String::as_str).collect();
            let close: Vec<String> = keys.iter().flat_map(|k| closest(k, ACTIONS.iter().map(|a| a.name), 2)).collect();
            return Err(format!("no action in {{ {} }}{} (actions: {})", keys.join(", "), did_you_mean(&close), ACTIONS.iter().map(|a| a.name).collect::<Vec<_>>().join(", ")));
        }
        _ => return Err(format!("one action per item, not {}: write each one in its own {{ }}", names.join(" and "))),
    };
    let spec = ACTIONS.iter().find(|a| a.name == name).expect("action");
    for key in item.keys().filter(|k| k.as_str() != name) {
        if !spec.settings.contains(&key.as_str()) {
            let close = closest(key, spec.settings.iter().copied(), 2);
            let all = if spec.settings.is_empty() { "none".to_string() } else { spec.settings.join(", ") };
            return Err(format!("unknown setting {key} for {name}{} (its settings: {all})", did_you_mean(&close)));
        }
    }
    let main = &item[name];
    let arg = |v: &toml::Value| ctx.arg(v);
    let text_value = |v: &toml::Value| -> Result<Arg, String> {
        match v {
            toml::Value::String(_) => arg(v),
            _ => Err(format!("{name} = {v} must be a text in quotes")),
        }
    };
    let flag = |key: &str| -> Result<bool, String> {
        match item.get(key) {
            None => Ok(false),
            Some(toml::Value::Boolean(b)) => Ok(*b),
            Some(v) => Err(format!("{key} = {v} must be true or false")),
        }
    };
    let args = match name {
        "say" => vec![ctx.who_or_default(item, "by")?, text_value(main)?],
        "message" | "objective" | "objective_done" | "fail" | "log" | "music" => vec![text_value(main)?],
        "give" => {
            let what = match main {
                // An item of the game by its class ("XIII.MedPick") is taken as written.
                toml::Value::String(s) => crate::simple::resolve_weapon(s).map_err(|close| {
                    format!("give = \"{s}\" is not a weapon name (docs/modding/names/weapons.md){}; write a class (\"Package.Class\") for anything else", did_you_mean(&close))
                })?,
                _ => return Err(format!("give = {main} must be a weapon name in quotes")),
            };
            vec![ctx_player_or(ctx, item, "to")?, Arg::Value(Value::Text(what))]
        }
        "spawn" => {
            let what = match main {
                toml::Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
                _ => return Err(format!("spawn = {main} must be a character name or a class in quotes")),
            };
            let at = item.get("position").and_then(position).ok_or("spawn needs position = [x, y, z] (a point on the ground)")?;
            let mut fields = vec![
                (if what.contains('.') { "class" } else { "character" }.to_string(), Value::Text(what.clone())),
                ("position".to_string(), Value::List(at.iter().map(|x| Value::Number(*x as f64)).collect())),
            ];
            if let Some(y) = item.get("yaw") {
                fields.push(("yaw".into(), Value::Number(number(y).ok_or_else(|| format!("yaw = {y} must be a number of degrees"))? as f64)));
            }
            // The settings of a character (team, behaviour...) are read by placement.rs.
            if what.contains('.') {
                if let Some(k) = item.keys().find(|k| crate::simple::KEYS.contains(&k.as_str())) {
                    return Err(format!("{k} is a setting of a character, not of a class ({what})"));
                }
            }
            vec![Arg::Value(Value::Table(fields))]
        }
        "remove" | "stop" | "kill" | "restart" => vec![arg(main)?],
        "change" => {
            let mut fields = Vec::new();
            for (k, v) in item.iter().filter(|(k, _)| k.as_str() != "change") {
                fields.push((k.clone(), plain(v)?));
            }
            if fields.is_empty() {
                return Err("change: give what changes (look, health, team...)".into());
            }
            vec![arg(main)?, Arg::Value(Value::Table(fields))]
        }
        "trigger" => {
            let mut a = vec![text_value(main)?];
            if let Some(by) = item.get("by") {
                a.push(arg(by)?);
            }
            a
        }
        "sound" => {
            let mut a = vec![text_value(main)?];
            if let Some(at) = item.get("at") {
                a.push(arg(at)?);
            }
            a
        }
        "next_level" => match main {
            toml::Value::Boolean(true) => vec![],
            toml::Value::String(s) if !s.trim().is_empty() => vec![Arg::Value(Value::Text(s.trim().to_string()))],
            _ => return Err(format!("next_level = {main} must be a map in quotes, or true for the next map of the mod's campaign")),
        },
        "camera" => {
            let at = position(main).ok_or_else(|| format!("camera = {main} must be a position [x, y, z]"))?;
            let look = item.get("look_at").ok_or("camera needs look_at = an actor or a position")?;
            let mut a = vec![Arg::Value(Value::List(at.iter().map(|x| Value::Number(*x as f64)).collect())), arg(look)?];
            if let Some(s) = item.get("seconds") {
                a.push(Arg::Value(Value::Number(number(s).ok_or_else(|| format!("seconds = {s} must be a number"))? as f64)));
            }
            a
        }
        "camera_back" => match main {
            toml::Value::Boolean(true) => vec![],
            _ => return Err("write camera_back = true".into()),
        },
        "go_to" | "patrol" => {
            let mut a = vec![ctx.who_or_default(item, "who")?, arg(main)?];
            if flag("run")? {
                a.push(Arg::Value(Value::Text("run".into())));
            }
            a
        }
        "play" => {
            let mut a = vec![ctx.who_or_default(item, "who")?, text_value(main)?];
            if flag("loop")? {
                a.push(Arg::Value(Value::Bool(true)));
            }
            a
        }
        // follow, attack, look_at, teleport, behave: the actor, then the action's value.
        _ => vec![ctx.who_or_default(item, "who")?, arg(main)?],
    };
    Ok(Action { name: name.to_string(), args })
}

/// Who gets something: `key` if written, else the player.
fn ctx_player_or(ctx: &Ctx, item: &toml::Table, key: &str) -> Result<Arg, String> {
    item.get(key).map_or(Ok(Arg::Player), |v| ctx.arg(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> Result<When, String> {
        read(&text.parse::<toml::Table>().unwrap())
    }

    #[test]
    fn near_then_say_and_give() {
        let w = block("event = \"near\"\nwho = \"Pam\"\ndistance = 250\nonce = true\nthen = [\n { say = \"Plage01_Pam_03\" },\n { give = \"Shotgun\", to = \"player\" },\n]").unwrap();
        assert_eq!(w.event, Event::Near { who: "Pam".into(), distance: Some(250.0) });
        assert!(w.once);
        assert_eq!(w.actions[0], Action { name: "say".into(), args: vec![Arg::Actor, Arg::Value(Value::Text("Plage01_Pam_03".into()))] });
        assert_eq!(w.actions[1].args, vec![Arg::Player, Arg::Value(Value::Text("XIII.FusilPompe".into()))]);
        assert_eq!(w.describe(), "when near Pam then say, give (once)");
    }

    /// The multi-line form `[[map.<Map>.when.then]]` reads the same as the inline list.
    #[test]
    fn both_forms_of_then() {
        let inline = "[[map.Plage01.when]]\nevent = \"start\"\nthen = [ { message = \"Hi\" }, { give = \"Shotgun\" } ]\n";
        let tables = "[[map.Plage01.when]]\nevent = \"start\"\n[[map.Plage01.when.then]]\nmessage = \"Hi\"\n[[map.Plage01.when.then]]\ngive = \"Shotgun\"\n";
        let get = |t: &str| {
            let top: toml::Table = t.parse().unwrap();
            read(top["map"]["Plage01"]["when"][0].as_table().unwrap()).unwrap()
        };
        assert_eq!(get(inline), get(tables));
    }

    #[test]
    fn spawn_methods_and_roles() {
        let w = block(
            "event = \"death\"\nwho = \"Visitor\"\nthen = [\n { spawn = \"Guard\", position = [1, 2, 3], team = \"enemy\" },\n { go_to = \"player\", who = \"Pam\", run = true },\n { kill = \"actor\" },\n { trigger = \"PorteCabane\" },\n { camera = [0, 0, 0], look_at = \"actor\", seconds = 3 },\n { next_level = true },\n]",
        )
        .unwrap();
        assert!(w.spawns.is_empty());
        assert_eq!(w.spawn_keys.iter().map(|(i, c, k)| (*i, c.as_str(), k.contains_key("team"))).collect::<Vec<_>>(), [(0, "Guard", true)]);
        let names: Vec<&str> = w.actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["spawn", "go_to", "kill", "trigger", "camera", "next_level"]);
        assert_eq!(w.actions[1].args, vec![Arg::Value(Value::Text("Pam".into())), Arg::Player, Arg::Value(Value::Text("run".into()))]);
        assert_eq!(w.actions[2].args, vec![Arg::Actor]);
        assert!(w.actions[5].args.is_empty());
    }

    /// Each mistake skips the block with one message naming the closest words.
    #[test]
    fn mistakes_name_the_closest_words() {
        let err = |t: &str| block(t).unwrap_err();
        assert!(err("event = \"neer\"\nwho = \"Pam\"\nthen = [{ message = \"x\" }]").contains("did you mean near"));
        assert!(err("event = \"hears\"\nthen = [{ message = \"x\" }]").contains("unknown event \"hears\""));
        assert!(err("event = \"near\"\nwho = \"Pam\"\ndistanse = 3\nthen = [{ message = \"x\" }]").contains("did you mean distance"));
        assert!(err("event = \"start\"\nthen = [{ mesage = \"x\" }]").contains("did you mean message"));
        assert!(err("event = \"start\"\nthen = [{ give = \"Shotgn\" }]").contains("did you mean Shotgun"));
        assert!(err("event = \"start\"\nthen = [{ say = \"x\", bye = \"Pam\" }]").contains("did you mean by"));
        assert!(err("event = \"start\"\nthen = [{ kill = \"actor\" }]").contains("the event start gives player"));
        assert!(err("event = \"near\"\nthen = [{ message = \"x\" }]").contains("who = \"...\" is missing"));
        assert!(err("event = \"start\"").contains("nothing to do"));
        assert!(err("event = \"start\"\nthen = [{ message = \"x\", give = \"Shotgun\" }]").contains("one action per item"));
        // Negative control: the same block written right is read.
        assert!(block("event = \"near\"\nwho = \"Pam\"\nthen = [{ message = \"x\" }]").is_ok());
    }
}
