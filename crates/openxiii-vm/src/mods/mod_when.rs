//! Mods build only (feature `mods`): "when ... then ..." rules given as data, for the mods'
//! toml files (`[[map.<Map>.when]]`, read by openxiii-mods), run by the scripts' engine
//! (mod_script.rs).
//!
//! A rule is an event (the same events as the scripts' `on(event, ...)`, plus a timer) and a
//! list of actions, each one a function of the scripts' API called with its arguments (`say`,
//! `spawn`, `give`...), and optionally `run`: a function of the mod's own scripts, called with
//! the event's arguments. The engine registers each rule with the very Lua functions a script
//! would call (`on`, `after`, `every`) and runs its actions through the very API functions:
//! a rule behaves as the script that would say the same, by construction (only where the game
//! has authority, never on a network client; started again with the map and by `modreload`).
//! A mod needs no `scripts` folder for its rules.
//!
//! ```ignore
//! use openxiii_vm::mod_when::*;
//! set_mod_when(vec![WhenRule {
//!     mod_name: "my-mod".into(),
//!     map: "Plage01".into(),
//!     source: "my-mod/mod.toml line 12".into(),
//!     event: WhenEvent::Near { who: "Visitor".into(), distance: Some(250.0) },
//!     actions: vec![
//!         WhenAction { name: "say".into(), args: vec![WhenArg::Actor, WhenArg::Value(WhenValue::Text("Hual02_Carrington_06".into()))] },
//!         WhenAction { name: "give".into(), args: vec![WhenArg::Player, WhenArg::Value(WhenValue::Text("Shotgun".into()))] },
//!     ],
//!     run: Some("reward".into()),
//!     once: true,
//!     checkpoint: Default::default(),
//!     control: false,
//! }]);
//! ```

use std::sync::{Arc, OnceLock};

/// An event of a rule.
#[derive(Debug, Clone, PartialEq)]
pub enum WhenEvent {
    /// The map has started (args: player).
    Start,
    /// A player comes within `distance` (300 by default) of `who` (args: player, actor).
    Near { who: String, distance: Option<f32> },
    /// A player comes within `radius` (300 by default) of a point on the ground (args: player).
    Enter { position: [f32; 3], radius: Option<f32> },
    /// `who` sees a player, with its own eyes (args: actor, player).
    Sees { who: String },
    /// `who` dies (args: actor).
    Death { who: String },
    /// `who` loses health (args: actor, amount).
    Damage { who: String },
    /// A shot of `who` ("all": every character), else of a player (args: actor, class).
    Fire { who: Option<String> },
    /// An item comes into a player's inventory (args: player, class).
    Pickup,
    /// The map fires event `name` (args: player).
    Event { name: String },
    /// A player presses keyboard key `key` (args: player).
    Key { key: String },
    /// Once, `seconds` after the map started (args: none; Player is the main player).
    After { seconds: f32 },
    /// Every `seconds` (args: none; Player is the main player).
    Every { seconds: f32 },
}

/// A plain value.
#[derive(Debug, Clone, PartialEq)]
pub enum WhenValue {
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
    /// A list ({a, b, c}: a position is a list of three numbers).
    List(Vec<WhenValue>),
    /// A table with names ({look = "Jones", health = 300}).
    Table(Vec<(String, WhenValue)>),
}

/// An argument of an action: a value, or what the event gave.
#[derive(Debug, Clone, PartialEq)]
pub enum WhenArg {
    Value(WhenValue),
    /// The player of the event (the main player for a timer or an event that gives none).
    Player,
    /// The actor of the event (near, sees, death, damage).
    Actor,
    /// The amount of health lost (damage).
    Amount,
    /// The class of the item picked up (pickup).
    Class,
}

/// An action: a function of the scripts' API, by its name, with its arguments. A method of
/// the actors (`give`, `go_to`, `kill`...) takes the actor first (an actor, `Player`, `Actor`,
/// or a name found as `find` finds it).
#[derive(Debug, Clone, PartialEq)]
pub struct WhenAction {
    pub name: String,
    pub args: Vec<WhenArg>,
}

/// A "when ... then ..." rule of a mod.
#[derive(Debug, Clone, PartialEq)]
pub struct WhenRule {
    /// Folder name of the mod (its scripts' global table, for `run`).
    pub mod_name: String,
    /// The map, or a selector of maps ("Hual*", "solo", "multi", "*"), as the toml's map tables.
    pub map: String,
    /// Where it was written, for the log ("my-mod/mod.toml line 12").
    pub source: String,
    pub event: WhenEvent,
    pub actions: Vec<WhenAction>,
    /// A global function of the mod's scripts, called after the actions with the event's
    /// arguments (it may wait).
    pub run: Option<String>,
    /// Only the first time.
    pub once: bool,
    /// `from_checkpoint`: only when the level starts from that checkpoint or a later one, as
    /// the toml's other map blocks.
    pub checkpoint: openxiii_mods::placement::Checkpoint,
    /// Wait until the player has control (the level's cut-scene frame, bCineFrame, off for one
    /// second in a row: a level's first frames come before its opening scene) before a `Start` rule acts, or before an `After` / `Every` timer starts counting. The
    /// toml's start, after and every set it; a script's on("start") stays immediate.
    pub control: bool,
}

/// The actions a rule can name today: the functions of the scripts' API that do something.
pub const ACTIONS: &[&str] = &[
    "say",
    "spawn",
    "remove",
    "give",
    "trigger",
    "objective",
    "objective_done",
    "message",
    "sound",
    "music",
    "next_level",
    "fail",
    "change",
    "log",
    "camera",
    "camera_back",
    "go_to",
    "follow",
    "attack",
    "look_at",
    "stop",
    "play",
    "restart",
    "teleport",
    "kill",
    "behave",
    "patrol",
];

static GLOBAL: OnceLock<Arc<Vec<WhenRule>>> = OnceLock::new();

/// Sets the rules of every Vm created afterwards (once, at startup).
pub fn set_mod_when(rules: Vec<WhenRule>) {
    let _ = GLOBAL.set(Arc::new(rules));
}

pub(crate) fn global_mod_when() -> Option<Arc<Vec<WhenRule>>> {
    GLOBAL.get().cloned().filter(|r| !r.is_empty())
}

impl crate::Vm {
    /// The rules of this Vm only (tests). Take effect at the next map start.
    pub fn set_mod_when_here(&mut self, rules: Vec<WhenRule>) {
        self.mods.when_rules = Some(Arc::new(rules)).filter(|r| !r.is_empty());
    }

    /// The rules for the map being played (map selectors as the toml's, and the checkpoint the
    /// level starts from), in their order.
    pub(crate) fn mod_when_here(&self) -> Vec<WhenRule> {
        let map = self.mod_map_name();
        let Some(rules) = self.mods.when_rules.as_ref() else { return Vec::new() };
        let any = openxiii_mods::placement::Checkpoint::Any;
        let checkpoint = if rules.iter().any(|w| w.checkpoint != any) { self.mod_checkpoint_here() } else { 1 };
        rules.iter().filter(|w| openxiii_mods::maps::rank(&w.map, &map).is_some() && w.checkpoint.applies(checkpoint)).cloned().collect()
    }
}

/// The Lua run for each rule: `rule` (event, its arguments, actions, run, once) is data made
/// by `rule_table`; the code is the same for every rule.
pub(crate) const REGISTER: &str = r#"
local env, rule = ...
local roles = {
  start = {"player"}, near = {"player", "actor"}, enter = {"player"}, sees = {"actor", "player"},
  death = {"actor"}, damage = {"actor", "amount"}, fire = {"actor", "class"}, pickup = {"player", "class"}, event = {"player"},
  key = {"player"}, after = {}, every = {},
}
local function value(a, ctx)
  if a.k == "v" then return a.v end
  return ctx[a.k]
end
-- "every Singer" as who: the action for each one of that name (find_all), the dead ones too for
-- remove, the living ones for the orders.
local function every_one(x)
  if type(x) ~= "string" then return nil end
  local name = x:match("^%s*[Ee]very%s+(.-)%s*$")
  if name == nil or name == "" then return nil end
  return find_all(name)
end
local id
local function act(...)
  local ctx = {}
  local given = table.pack(...)
  for i, role in ipairs(roles[rule.event]) do ctx[role] = given[i] end
  if ctx.player == nil then ctx.player = player() end
  if rule.once and id then cancel(id) end
  log("when rule fired")
  for _, a in ipairs(rule.actions) do
    local args = {}
    for i, x in ipairs(a.args) do args[i] = value(x, ctx) end
    local all = every_one(args[1])
    if all and a.method then
      for _, who in ipairs(all) do
        if who.alive then who[a.name](who, table.unpack(args, 2, #a.args)) end
      end
    elseif all and (a.name == "remove" or a.name == "change") then
      for _, who in ipairs(all) do env[a.name](who, table.unpack(args, 2, #a.args)) end
    elseif a.method then
      local who = args[1]
      if type(who) == "string" then who = find(who) end
      if who == nil then error(a.name .. ": nobody of that name", 0) end
      who[a.name](who, table.unpack(args, 2, #a.args))
    else
      local made = env[a.name](table.unpack(args, 1, #a.args))
      -- An enemy a block makes comes for the player at once (a soldier's own AI attacks only
      -- once it has seen or heard him).
      if a.name == "spawn" and made and made.team == "enemy" and ctx.player then
        made:attack(ctx.player)
      end
    end
  end
  if rule.run then
    local f = env[rule.run]
    if type(f) ~= "function" then error("run: the mod's scripts have no function " .. rule.run, 0) end
    return f(table.unpack(given, 1, given.n))
  end
end
local function register()
  if rule.event == "after" then
    id = after(rule.seconds, act)
  elseif rule.event == "every" then
    id = every(rule.seconds, act)
  else
    -- The event's arguments, then the function last (table.unpack in the middle of a list
    -- would give its first value only: a near distance or an enter radius would be lost).
    local args = table.pack(table.unpack(rule.on, 1, rule.on.n))
    args[args.n + 1] = act
    id = on(rule.event, table.unpack(args, 1, args.n + 1))
  end
end
-- The player has control: no cut-scene frame on the level.
local function in_control()
  local p = player()
  if p == nil then return false end
  local ok, cine = pcall(function() return p:get("Level"):get("bCineFrame") end)
  return ok and not cine
end
-- One second in a row without a cut-scene: the first frames of a level come before its opening
-- scene starts.
local function wait_control()
  repeat
    wait_until(in_control)
    wait(1)
  until in_control()
end
if rule.control and rule.event == "start" then
  id = on("start", function(...)
    local given = table.pack(...)
    wait_control()
    return act(table.unpack(given, 1, given.n))
  end)
elseif rule.control then
  on("start", function()
    wait_control()
    register()
  end)
else
  register()
end
"#;

fn value_to_lua(lua: &mlua::Lua, v: &WhenValue) -> mlua::Result<mlua::Value> {
    Ok(match v {
        WhenValue::Bool(b) => mlua::Value::Boolean(*b),
        WhenValue::Int(i) => mlua::Value::Integer(*i),
        WhenValue::Number(n) => mlua::Value::Number(*n),
        WhenValue::Text(t) => mlua::Value::String(lua.create_string(t)?),
        WhenValue::List(items) => {
            let t = lua.create_table()?;
            for x in items {
                t.raw_push(value_to_lua(lua, x)?)?;
            }
            mlua::Value::Table(t)
        }
        WhenValue::Table(fields) => {
            let t = lua.create_table()?;
            for (k, x) in fields {
                t.raw_set(k.as_str(), value_to_lua(lua, x)?)?;
            }
            mlua::Value::Table(t)
        }
    })
}

/// A rule as the data REGISTER reads.
fn rule_table(lua: &mlua::Lua, rule: &WhenRule) -> mlua::Result<Result<mlua::Table, String>> {
    let t = lua.create_table()?;
    let on = lua.create_table()?;
    let mut args: Vec<mlua::Value> = Vec::new();
    let text = |s: &str| lua.create_string(s).map(mlua::Value::String);
    let (event, seconds) = match &rule.event {
        WhenEvent::Start => ("start", None),
        WhenEvent::Near { who, distance } => {
            args.push(text(who)?);
            if let Some(d) = distance {
                args.push(mlua::Value::Number(*d as f64));
            }
            ("near", None)
        }
        WhenEvent::Enter { position, radius } => {
            args.push(mlua::Value::Table(lua.create_sequence_from(position.iter().map(|x| *x as f64))?));
            if let Some(r) = radius {
                args.push(mlua::Value::Number(*r as f64));
            }
            ("enter", None)
        }
        WhenEvent::Sees { who } => {
            args.push(text(who)?);
            ("sees", None)
        }
        WhenEvent::Death { who } => {
            args.push(text(who)?);
            ("death", None)
        }
        WhenEvent::Damage { who } => {
            args.push(text(who)?);
            ("damage", None)
        }
        WhenEvent::Fire { who } => {
            if let Some(w) = who {
                args.push(text(w)?);
            }
            ("fire", None)
        }
        WhenEvent::Pickup => ("pickup", None),
        WhenEvent::Event { name } => {
            args.push(text(name)?);
            ("event", None)
        }
        WhenEvent::Key { key } => {
            args.push(text(key)?);
            ("key", None)
        }
        WhenEvent::After { seconds } => ("after", Some(*seconds)),
        WhenEvent::Every { seconds } => ("every", Some(*seconds)),
    };
    on.raw_set("n", args.len())?;
    for (i, a) in args.into_iter().enumerate() {
        on.raw_set(i + 1, a)?;
    }
    t.raw_set("event", event)?;
    t.raw_set("on", on)?;
    if let Some(s) = seconds {
        t.raw_set("seconds", s)?;
    }
    let actions = lua.create_table()?;
    for a in &rule.actions {
        let found = crate::mod_script::API.iter().find(|f| f.name == a.name && !f.hidden);
        let Some(f) = found.filter(|_| ACTIONS.contains(&a.name.as_str())) else {
            return Ok(Err(format!("unknown action {} (actions: {})", a.name, ACTIONS.join(", "))));
        };
        let at = lua.create_table()?;
        at.raw_set("name", a.name.as_str())?;
        at.raw_set("method", f.method)?;
        let list = lua.create_table()?;
        for x in &a.args {
            let arg = lua.create_table()?;
            match x {
                WhenArg::Value(v) => {
                    arg.raw_set("k", "v")?;
                    arg.raw_set("v", value_to_lua(lua, v)?)?;
                }
                WhenArg::Player => arg.raw_set("k", "player")?,
                WhenArg::Actor => arg.raw_set("k", "actor")?,
                WhenArg::Amount => arg.raw_set("k", "amount")?,
                WhenArg::Class => arg.raw_set("k", "class")?,
            }
            list.raw_push(arg)?;
        }
        at.raw_set("args", list)?;
        actions.raw_push(at)?;
    }
    t.raw_set("actions", actions)?;
    if let Some(r) = &rule.run {
        t.raw_set("run", r.as_str())?;
    }
    t.raw_set("once", rule.once)?;
    t.raw_set("control", rule.control)?;
    Ok(Ok(t))
}

/// Registers `rule` in the scripts' engine `lua`, in the global table `env` of its mod, with
/// the scripts' own `on`, `after` and `every` (REGISTER).
pub(crate) fn register(lua: &mlua::Lua, env: &mlua::Table, rule: &WhenRule) -> Result<(), String> {
    let data = rule_table(lua, rule).map_err(|e| e.to_string())??;
    let chunk = lua.load(REGISTER).set_name(format!("@{}", rule.source)).set_mode(mlua::ChunkMode::Text).set_environment(env.clone());
    chunk.call::<()>((env.clone(), data)).map_err(|e| crate::mod_script::error_text(&e))
}
