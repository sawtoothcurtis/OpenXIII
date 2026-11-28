//! Mods build only (feature `mods`): the mods' Lua scripts (`Mods/<mod>/scripts/*.lua`).
//!
//! - **Engine.** Lua 5.4 (mlua, built from the Lua sources). Only the base functions (without
//!   `dofile`, `loadfile`, `load`, `require`, `collectgarbage`) and the `string` (without `dump`),
//!   `table`, `math` and `coroutine` libraries; chunks are loaded as text only (no bytecode).
//!   Each mod has its own global table; the API is shared, read-only through `__index`.
//! - **Where.** Scripts run where the game has authority (solo, split screen, the host or the
//!   server of a network game), never on a network client: the server's actors reach the
//!   clients by replication, as the mods' map changes do (`mod_net_client`).
//! - **When.** The scripts of the map start at the end of its start-up, after the mods' map
//!   changes, each time it is loaded (a checkpoint loads the map again). `modreload` starts
//!   them again from their files.
//! - **Handlers.** `on(event, ...)`, `after` and `every` register Lua functions; each run is a
//!   coroutine resumed on the level's time, so `wait(seconds)` can pause it.
//! - **Safety.** Each run has an instruction budget per frame: past it, the run is stopped (and
//!   its handler or timer removed) with one log line, so a script never freezes the game. Errors
//!   go to the log with the file and the line and never stop the game.
//!
//! Lua functions reach the engine through one scoped "gate" function that exists only while the
//! game runs scripts (`mod_scripts_step`): it is the only path from Lua to the `Vm`.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use mlua::{
    ChunkMode, FromLua, FromLuaMulti, Function, HookTriggers, IntoLua, IntoLuaMulti, Lua, LuaOptions, MultiValue, StdLib, Table, Thread, ThreadStatus,
    UserData, UserDataMethods, VmState,
};
use openxiii_mods::behaviour::Team;

use crate::mod_script_game as G;
use crate::mod_script_hud as H;
use crate::mod_script_more as M;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// Instructions a run of a script may execute in one frame.
pub const BUDGET: i64 = 1_000_000;
/// The budget is checked every this many instructions.
const HOOK_EVERY: u32 = 1000;
/// Memory all the scripts of a map may use.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// `log` lines kept per map (the log file is rewritten at each line).
const LOG_LINES: usize = 1000;
/// spawn: how far below a point the ground is looked for, and the most a point may be above it.
const GROUND_SEARCH: f32 = 2000.0;
const MAX_STEP: f32 = 64.0;
/// Default distance of `on("near", who, distance, fn)`.
const NEAR_DISTANCE: f32 = 300.0;
/// Registry key of the gate function.
const GATE: &str = "openxiii.gate";
/// Registry key of the actors' methods.
const METHODS: &str = "openxiii.methods";
/// The libraries each mod gets its own copy of (a mod cannot change them for the others).
const LIBRARIES: &[&str] = &["string", "table", "math", "coroutine"];

/// A mod that may have scripts: its folder name and folder.
#[derive(Debug, Clone)]
pub struct ScriptMod {
    pub name: String,
    pub folder: PathBuf,
}

static GLOBAL: OnceLock<Arc<Vec<ScriptMod>>> = OnceLock::new();

/// Sets the mods whose scripts every Vm created afterwards runs (once, at startup). Their
/// `scripts` folders are read when a map starts, so `modreload` sees edited and new files.
pub fn set_mod_scripts(mods: Vec<ScriptMod>) {
    let _ = GLOBAL.set(Arc::new(mods));
}

pub(crate) fn global_mod_scripts() -> Option<Arc<Vec<ScriptMod>>> {
    GLOBAL.get().cloned().filter(|m| !m.is_empty())
}

// ---------------------------------------------------------------------------------------------
// The API: one table, with a one-line description and an example for each function.

/// What runs a function of the API.
#[derive(Clone, Copy)]
pub enum ApiKind {
    /// Needs the game (called through the gate).
    Game(fn(&mut Vm, &Lua, MultiValue) -> mlua::Result<MultiValue>),
    /// Only the scripts' own state (handlers, timers).
    Script(fn(&Lua, MultiValue) -> mlua::Result<MultiValue>),
    /// Written in Lua (the prelude defines it).
    Lua,
}

/// A function of the scripts' API.
#[derive(Clone, Copy)]
pub struct ApiFunction {
    pub name: &'static str,
    /// Its arguments, as written in the reference.
    pub args: &'static str,
    pub about: &'static str,
    pub example: &'static str,
    /// Internal (the actor fields): not listed in the reference.
    pub hidden: bool,
    /// A method of the actors (`actor:go_to(...)`), not a global function.
    pub method: bool,
    pub kind: ApiKind,
}

const fn api(name: &'static str, args: &'static str, about: &'static str, example: &'static str, kind: ApiKind) -> ApiFunction {
    ApiFunction { name, args, about, example, hidden: false, method: false, kind }
}

const fn method(name: &'static str, args: &'static str, about: &'static str, example: &'static str, kind: ApiKind) -> ApiFunction {
    ApiFunction { name, args, about, example, hidden: false, method: true, kind }
}

const fn hidden(name: &'static str, kind: ApiKind) -> ApiFunction {
    ApiFunction { name, args: "", about: "", example: "", hidden: true, method: false, kind }
}

/// Every function of the API, in the order of the reference.
pub static API: &[ApiFunction] = &[
    api(
        "on",
        "event, ..., fn",
        "Runs fn when something happens: \"start\" (fn(player)), \"near\" who, distance (fn(player, actor)), \"enter\" position, radius (fn(player)), \"sees\" who (fn(actor, player): the character's own sight, CanSee), \"death\" who (fn(actor)), \"damage\" who (fn(actor, amount, by, damage_type, weapon)), \"fire\" [who] (fn(shooter, weapon_class)), \"pickup\" (fn(player, class)), \"event\" name (fn(player): the player who caused it when the game says, else the main player), \"key\" key (fn(player)), \"map_end\", \"loading\", \"loaded\", \"loading_key\" (the change of map: see the events). Returns an id for cancel.",
        "on(\"near\", \"Visitor\", 250, function(player, visitor) say(visitor, \"Hello.\") end)",
        ApiKind::Script(api_on),
    ),
    api("wait", "seconds", "Pauses the handler for that many seconds of game time.", "wait(2.5)", ApiKind::Lua),
    api("wait_until", "fn[, seconds]", "Pauses the handler until fn() returns true (checked each frame), or for at most that many seconds. Returns true if fn did.", "wait_until(function() return not guard.alive end, 60)", ApiKind::Lua),
    api("has_control", "", "True when a player can move: no cut-scene is playing. The first frames of a level come before its opening scene: wait a moment first.", "wait(1) wait_until(has_control)", ApiKind::Lua),
    api("after", "seconds, fn", "Runs fn once, that many seconds of game time from now. Returns an id for cancel.", "after(10, function() message(\"Ten seconds.\") end)", ApiKind::Script(api_after)),
    api("every", "seconds, fn", "Runs fn every that many seconds of game time; fn returning false stops it. Returns an id for cancel.", "every(1, function() log(player().health) end)", ApiKind::Script(api_every)),
    api("cancel", "id", "Stops a handler of on, after or every.", "local id = every(1, tick) cancel(id)", ApiKind::Script(api_cancel)),
    api("map", "", "The name of the map being played (\"Plage01\").", "if map() ~= \"Plage01\" then return end", ApiKind::Game(api_map)),
    api("find", "who", "The actor named who (its name, then its Tag, then its PawnName; a mods [character] by its name first), or nil.", "local pam = find(\"Pam\")", ApiKind::Game(api_find)),
    api(
        "find_all",
        "who",
        "Every actor named who, as a list: all the mods [character]s of that name on the map, alive or dead; else every actor alive that find would give.",
        "for _, s in ipairs(find_all(\"Singer\")) do s:restart() end",
        ApiKind::Game(api_find_all),
    ),
    api("player", "", "The player (the local one in a split screen game), or nil.", "local p = player()", ApiKind::Game(api_player)),
    api("players", "", "Every player of the game, as a list.", "for _, p in ipairs(players()) do p.health = 100 end", ApiKind::Game(api_players)),
    api(
        "spawn",
        "{character= or class=, position= or near=, yaw=, team=, behaviour=}",
        "Creates a mods [character] or an actor of a class, standing on position (a point on the ground), or near an actor or a point (the first spot around it with room and ground). Returns the actor, or nil.",
        "spawn{character = \"Guard\", position = {-470, -860, 1190}, yaw = 180, team = \"enemy\"}",
        ApiKind::Game(api_spawn),
    ),
    api("remove", "who", "Takes an actor out of the game. Returns true if it was removed.", "remove(\"MedPick0\")", ApiKind::Game(api_remove)),
    api(
        "say",
        "who, line",
        "Who says a line: a line of the game by its name, your own subtitle, or {line=, text=, sound=} as in [[map.<Map>.say]].",
        "say(\"Pam\", \"Plage01_Pam_03\")",
        ApiKind::Game(api_say),
    ),
    api("message", "text[, player]", "Shows a message on the screen of every player, or of that player only.", "message(\"The guard is coming.\")", ApiKind::Game(api_message)),
    api("log", "text", "Writes a line in mods-log.txt.", "log(\"scene started\")", ApiKind::Game(api_log)),
    api("change", "who, {look=, health=, team=, can_die=, weapon=, name=, voice=, skin=, properties=}", "Changes an actor now, with the same words as the toml's [[map.<Map>.change]]. Returns how many actors changed.", "change(\"Pam\", {look = \"Jones\", health = 300})", ApiKind::Game(G::api_change)),
    api("objective", "text[, player]", "Shows a new goal on the screen, as the game's goals are shown (to that player only if given).", "objective(\"Hold the beach\")", ApiKind::Game(G::api_objective)),
    api("objective_done", "text[, player]", "Shows a goal as reached (to that player only if given).", "objective_done(\"Hold the beach\")", ApiKind::Game(G::api_objective_done)),
    api("trigger", "event[, who]", "Fires an event of the map (opens its doors, starts its movers and scenes), caused by who (the player by default).", "trigger(\"PorteCabane\")", ApiKind::Game(G::api_trigger)),
    api("sound", "name[, actor or position]", "Plays a sound of the game or of the mod, from the player, an actor or a position. Returns an id for stop_sound.", "local alarm = sound(\"Mod.Alarm\", {-470, -860, 1190})", ApiKind::Game(G::api_sound)),
    api("music", "name", "Plays a music of the game (a sound of its music banks).", "music(\"XIIIMusic.Combat.hCombat1\")", ApiKind::Game(G::api_music)),
    api("next_level", "[map]", "Ends the mission as won and goes on to that map (\"Banque01\", \"Banque01#Teleporter\", or a mod's map); without a map, the next one of campaign{...}.", "next_level(\"Banque01\")", ApiKind::Game(G::api_next_level)),
    api("campaign", "{map, map, ...}", "Lists the maps of the mod's campaign in order: next_level() then goes to the next one. Write it in every map's script (a script starts again with each map).", "campaign{\"BeachNight\", \"HutAtDawn\"}", ApiKind::Script(api_campaign)),
    api("fail", "reason", "Ends the mission as lost, with that text on the game over screen.", "fail(\"The guard escaped.\")", ApiKind::Game(G::api_fail)),
    api("random", "[m[, n]]", "A random number of the scripts' own (never the game's): random() between 0 and 1, random(n) from 1 to n, random(m, n) from m to n.", "local wave = random(3, 6)", ApiKind::Script(api_random)),
    api("class_get", "class, property", "The default value of a property of a class.", "log(class_get(\"XIII.Beretta\", \"ReloadCount\"))", ApiKind::Game(G::api_class_get)),
    api("class_set", "class, property, value", "Changes the default value of a property of a class (for the actors created afterwards).", "class_set(\"XIII.Beretta\", \"ReloadCount\", 20)", ApiKind::Game(G::api_class_set)),
    method("go_to", "target[, \"run\"]", "A story character (Cine2) walks, or runs, to an actor or a position. Soldiers follow their own AI.", "janet:go_to({-470, -860, 1190})", ApiKind::Game(G::api_go_to)),
    method(
        "follow",
        "actor[, {catch_up = distance}]",
        "The character follows an actor until stop(), finding its way through doors; with catch_up, when it is further than that and no player sees it, it is put back near its leader.",
        "guard:follow(player(), {catch_up = 1500})",
        ApiKind::Game(G::api_follow),
    ),
    method("attack", "actor", "A soldier takes this actor as its enemy (a player: it becomes hostile to the players).", "guard:attack(player())", ApiKind::Game(G::api_attack)),
    method("look_at", "actor or position", "The character turns to look at it.", "janet:look_at(player())", ApiKind::Game(G::api_look_at)),
    method("stop", "", "Stops a go_to or a follow.", "janet:stop()", ApiKind::Game(G::api_stop)),
    method("play", "animation[, loop]", "Plays an animation of the character's model (animations.md).", "janet:play(\"WaitNeutre\", true)", ApiKind::Game(G::api_play)),
    method(
        "restart",
        "",
        "A mods [character] starts the look of its state again from the beginning: its animation and its sound ([character.<Name>.state.<State>]). Returns false for any other actor.",
        "singer:restart()",
        ApiKind::Game(G::api_restart),
    ),
    method("give", "weapon or class", "Gives a weapon (a name of the toml's weapon list, or a class) or an item.", "player():give(\"Shotgun\")", ApiKind::Game(G::api_give)),
    method("teleport", "actor or position", "Moves the actor there at once. Returns false if there is no room.", "guard:teleport({-470, -860, 1190})", ApiKind::Game(G::api_teleport)),
    method("kill", "", "The character dies, as from a gun shot.", "guard:kill()", ApiKind::Game(G::api_kill)),
    method("get", "property", "The value of any property of the actor (characters.md, weapons.md); \"Emitters[0].Disabled\" reads a property of an object it holds.", "log(guard:get(\"GroundSpeed\"))", ApiKind::Game(G::api_get)),
    method("set", "property, value", "Writes any property of the actor (or of an object it holds: \"Emitters[0].Disabled\"); a wrong type gives a clear error.", "guard:set(\"GroundSpeed\", 300)", ApiKind::Game(G::api_set)),
    method("call", "function, ...", "Calls a function of the actor's class, as the game's own scripts call it.", "door:call(\"Trigger\", player(), player())", ApiKind::Game(G::api_call)),
    api("behaviour", "name, fn", "Defines a behaviour: fn(self) drives a character as a handler does (orders, wait, wait_until), until it dies or behaves otherwise.", "behaviour(\"Bodyguard\", function(self) while true do self:follow(player()) wait(1) end end)", ApiKind::Script(api_behaviour)),
    api("actors_near", "position or actor, radius[, class]", "The actors of the level within radius (the scenery's static meshes, its doors and objects, the characters alive or dead; players not included), of that class if given, nearest first.", "for _, a in ipairs(actors_near(player(), 500, \"StaticMeshActor\")) do log(a.name) end", ApiKind::Game(crate::mod_script_world::api_actors_near)),
    api("char_area", "position, radius[, amount]", "The level's own walls, floors and ceilings, and the pieces of scenery too big to be charred as a whole, within radius are drawn charred: amount from 0 to 1 (1 by default; the darkest stays), less towards the edge. A surface drawn in triangles wider than radius is left as it is (a darkened triangle would show far from the fire). It stays as long as the level.", "char_area({-470, -860, 1191}, 300)", ApiKind::Game(crate::mod_script_world::api_char_area)),
    api("characters_near", "position or actor, radius", "The characters alive within radius (players not included), nearest first.", "for _, c in ipairs(characters_near(player(), 1000)) do log(c.name) end", ApiKind::Game(G::api_characters_near)),
    api(
        "camera",
        "position, look_at[, seconds] or {from=, look_at=, seconds=, fov=, follow=, path=, orbit=}",
        "A scene: every player's view goes to position, looking at an actor (followed with the eyes) or a position, with the cinematic frame and the controls frozen; back after seconds if given. With a table: fov = the field of view; follow = an actor the camera moves with; path = {point, ...} the camera goes along from `from` over the seconds; orbit = {radius=, height=, turns=} the camera turns around what it looks at over the seconds.",
        "camera{from = {-300, -900, 1300}, look_at = find(\"Visitor\"), orbit = {turns = 0.5}, seconds = 6}",
        ApiKind::Game(G::api_camera),
    ),
    api(
        "checkpoint",
        "description[, teleporter]",
        "The levels' save point: the game saves here, with that description, and takes up again at the map's teleporter given (one of the map's, as the catalog lists them) or at the map's start. The mod's save table is saved with it.",
        "checkpoint(\"On the beach\")",
        ApiKind::Game(crate::mod_script_effects::api_checkpoint),
    ),
    api(
        "focus",
        "{on=, seconds=, zoom=, zoom_fov=, zoom_distance=, vignette=, danger=}",
        "The levels' \"sixth sense\" inset: a window on every player's screen that shows an actor (or a point), as the levels show a clue or a hidden enemy; zoom = true zooms on it, danger = true is the red danger inset.",
        "focus{on = find(\"Visitor\"), seconds = 3, zoom = true}",
        ApiKind::Game(crate::mod_script_effects::api_focus),
    ),
    api(
        "slow_motion",
        "factor[, seconds]",
        "The game runs factor times as fast (0.5: half speed; 1: normal), for that many seconds if given; slow_motion(1) ends it.",
        "slow_motion(0.4, 3)",
        ApiKind::Game(crate::mod_script_effects::api_slow_motion),
    ),
    api(
        "fade",
        "{to=, seconds=, player=} or {back = true, seconds=, player=}",
        "Every player's view (or that player's only) fades to a colour (\"black\", \"white\", \"red\" or {r, g, b} from 0 to 255) over the seconds, and back with back = true.",
        "fade{to = \"black\", seconds = 2}",
        ApiKind::Game(crate::mod_script_effects::api_fade),
    ),
    api("time", "", "Seconds of game time since the level started: the clock of wait and every, the same at any frame rate.", "local t0 = time()", ApiKind::Game(crate::mod_script_world::api_time)),
    api(
        "trace",
        "from, to[, by]",
        "What the line from one point to another meets first (the level or an actor), as {position=, normal=, actor=, material=} (actor is nil for the level itself; material is the name of the surface's texture or shader), or nil when nothing is in the way. The line starts from actor by (the player by default), which it never meets.",
        "local hit = trace(p.position, {0, 0, 0}) if hit then log(hit.position[3]) end",
        ApiKind::Game(crate::mod_script_world::api_trace),
    ),
    api(
        "heat",
        "{at=, radius=, seconds=, strength=, name=}",
        "Heat shimmer: the view waves a little over a round area around at (a position, or an actor it follows) of that radius, for that many seconds (strength from 0 to 2, 1 by default). A new heat of the same name replaces the earlier one.",
        "heat{at = find(\"Visitor\"), radius = 120, seconds = 5}",
        ApiKind::Game(crate::mod_script_world::api_heat),
    ),
    api(
        "effect",
        "{class=, position=, rotation= or facing=, on=, seconds=}",
        "Puts an actor of a class (an emitter, a projector, a light...) exactly at position (no search for the ground), turned by rotation = {pitch, yaw, roll} in degrees or facing = a direction {x, y, z}; on = an actor that carries it (it moves with it); seconds = how long it stays. Returns the actor, or nil.",
        "effect{class = \"XIII.GrenadBlast\", position = {-470, -860, 1191}, facing = {0, 0, -1}}",
        ApiKind::Game(crate::mod_script_world::api_effect),
    ),
    api("camera_back", "", "Ends a scene: the players' own view and controls come back.", "camera_back()", ApiKind::Game(G::api_camera_back)),
    api(
        "panel",
        "{from=, look_at=, fov=, seconds=, after=, x=, y=, size=, appear=, sound=, text=, word=, see_player=} or a list of them",
        "Comic-book insets as the levels show them: each one a small window with its own camera (from a point, looking at an actor or a point), on every player's screen, one after the other (after = seconds before the next one). The screen is 640x480; size up to 256 (insets shown together share a 256x256 picture: four of 128, the levels' size); appear: none, left, above, stretch, stretch_up, zoom, right, below, top_left, top_right, bottom_left, bottom_right; sound: none, tip, warning, dialog; word: an onomatopoeia picture of the levels (XIIIvignettes). Returns the seconds until the last one closes.",
        "panel{from = {-300, -900, 1300}, look_at = find(\"Visitor\"), seconds = 3, appear = \"zoom\"}",
        ApiKind::Game(crate::mod_script_panel::api_panel),
    ),
    method(
        "cine",
        "line",
        "For experts: one command line of the levels' scene language, given to a story character as the game's own scenes give it (CineController2): \"Anim Once Rire\", \"LookAt XIIIPlayerPawn0\", \"Shoot\", \"Give XIII.Beretta\"... An unknown command is ignored by the game, as in the levels.",
        "visitor:cine(\"Anim Once Rire\")",
        ApiKind::Game(G::api_cine),
    ),
    method(
        "retexture",
        "from, to",
        "Draws the actor (a static mesh of the scenery: a hut, a crate) with texture to where its own is from (the names trace() gives as material); to = \"\" draws it as it was. Only this actor changes.",
        "hut:retexture(\"XIIIPlage.PLwood05\", \"XIIICamp.CSbrule\")",
        ApiKind::Game(crate::mod_script_world::api_retexture),
    ),
    method(
        "pin_to",
        "other[, {forward, right, up}]",
        "The actor (an effect) is kept every frame at that point of the other actor's own frame, until pin_to(nil): a flame at the tip of a gun a character holds (its ThirdPersonActor, placed as the character is posed). In a network game each player sees it on the other actor as his own screen draws it; its rotation stays the script's, sent relative to the view of the character holding it.",
        "jet:pin_to(p:get(\"Weapon\"):get(\"ThirdPersonActor\"), {100, 0, 3})",
        ApiKind::Game(crate::mod_script_world::api_pin_to),
    ),
    method("behave", "name or fn or nil", "The character runs a behaviour (one defined with behaviour(), or a function); nil stops it.", "guard:behave(\"Bodyguard\")", ApiKind::Game(G::api_behave)),
    method("patrol", "{points...}[, \"run\"]", "The character walks from point to point, again and again, until stop() or another order.", "guard:patrol{{-470, -860, 1190}, {-200, -860, 1190}}", ApiKind::Game(G::api_patrol)),
    method("is_a", "class", "True when the actor is of that class or of a class made from it: a full name (\"XIII.Beretta\") or a class's own name (\"Pawn\").", "if a:is_a(\"Pawn\") then log(a.name, \"is a character\") end", ApiKind::Game(M::api_is_a)),
    method("trigger", "[by]", "Triggers the actor (a door, a mover, an emitter: its Trigger), caused by by (the player by default).", "find(\"PorteCabane\"):trigger()", ApiKind::Game(M::api_actor_trigger)),
    method("has", "weapon or class", "True when the character carries that weapon (a name of the toml's weapon list, or a class) or item.", "if player():has(\"Shotgun\") then message(\"You have the shotgun.\") end", ApiKind::Game(M::api_has)),
    method(
        "hurt",
        "amount[, by[, damage_type[, at[, push]]]]",
        "The actor takes damage as from the game's weapons (its TakeDamage: the game decides how much, with the difficulty and armour): caused by by, of damage_type (\"XIII.DTGunned\" by default; \"XIII.DTFire\", \"XIII.DTBite\"...), hit at a point (its middle by default) and pushed by push {x, y, z}. Returns the health a character really lost.",
        "find(\"Guard\"):hurt(25, player(), \"XIII.DTFire\")",
        ApiKind::Game(M::api_hurt),
    ),
    method("can_see", "actor", "True when the character sees that actor with its own eyes (walls hide it); false for an actor without eyes (no controller).", "if dog:can_see(player()) then dog:attack(player()) end", ApiKind::Game(M::api_can_see)),
    method("take", "weapon or class", "Takes that weapon or item away from the character. Returns how many were taken.", "player():take(\"XIII.Beretta\")", ApiKind::Game(M::api_take)),
    method(
        "aim",
        "",
        "Where the character looks: {from =, direction =, hit =, actor =}: the point of its eyes, the direction of its view (where a player aims), the first point the view meets (nil when nothing is within 20000 units) and the actor there (nil for the level itself).",
        "local a = player():aim() if a.hit then effect{class = \"XIII.GrenadBlast\", position = a.hit} end",
        ApiKind::Game(M::api_aim),
    ),
    api("game", "", "What is being played: {difficulty =, language =, mode =}: the difficulty (0 to 3), the language of the game's texts (\"int\", \"frt\", \"det\", \"itt\", \"est\"), and \"solo\", \"split\" (players on one screen) or \"network\".", "if game().mode == \"network\" then log(\"network game\") end", ApiKind::Game(M::api_game)),
    api("preload", "name", "In the script's main code or its on(\"start\") handler only: asks for an object (\"Package.Name\": a mesh, a texture, a sound) to be made ready before the level starts, as preload = [...] in the mod.toml does.", "preload(\"XIIIPersos.SpadsM\")", ApiKind::Game(M::api_preload)),
    api("stop_sound", "id", "Stops a sound that sound() played (sound() gives its id). Returns false when it was stopped already.", "local id = sound(\"XIIISound.Ambient.SancFire\", find(\"Hut\")) after(5, function() stop_sound(id) end)", ApiKind::Game(M::api_stop_sound)),
    api("music_stop", "", "Stops the music.", "music_stop()", ApiKind::Game(M::api_music_stop)),
    api(
        "hud_text",
        "{id=, text=, x=, y=, size=, font=, colour=, align=, player=}",
        "Keeps a text on the screen until hud_remove: x and y from 0 to 1 of the view (0, 0 the top left corner), size \"small\" or \"medium\" (or font = a font of the game), colour = a name or {r, g, b[, a]} (0 to 255), align \"left\", \"centre\" or \"right\" (what x is), player = only on that player's view. Returns its id (the one given, or a new one); an id already shown is replaced.",
        "hud_text{id = \"goal\", text = \"Find the key\", x = 0.5, y = 0.1, align = \"centre\", colour = \"yellow\"}",
        ApiKind::Game(H::api_hud_text),
    ),
    api("hud_counter", "{id=, label=, value=, x=, y=, ...}", "A text showing label then value (a number), as hud_text; hud_update(id, {value = n}) changes it.", "hud_counter{id = \"kills\", label = \"Enemies: \", value = 0, x = 0.05, y = 0.9}", ApiKind::Game(H::api_hud_counter)),
    api("hud_timer", "{id=, label=, seconds=, x=, y=, ...}", "A text counting down from seconds (game time), as minutes:seconds after label, as hud_text; it stops at 0:00.", "hud_timer{id = \"bomb\", label = \"Bomb: \", seconds = 90, x = 0.8, y = 0.05}", ApiKind::Game(H::api_hud_timer)),
    api("hud_bar", "{id=, x=, y=, w=, h=, value=, colour=, back=, player=}", "A bar w wide and h high (0 to 1 of the view), filled from the left to value (0 to 1) with colour, over back if given.", "hud_bar{id = \"boss\", x = 0.3, y = 0.05, w = 0.4, h = 0.02, value = 1, colour = \"red\", back = {40, 40, 40}}", ApiKind::Game(H::api_hud_bar)),
    api("hud_image", "{id=, texture=, x=, y=, w=, h=, colour=, player=}", "A texture of the game or of the mod stretched over w by h (0 to 1 of the view).", "hud_image{id = \"icon\", texture = \"Mod.FlamethrowerIcon\", x = 0.9, y = 0.8, w = 0.08, h = 0.1}", ApiKind::Game(H::api_hud_image)),
    api("hud_line", "{id=, from=, to=, colour=, player=}", "A line from {x, y} to {x, y} (0 to 1 of the view).", "hud_line{id = \"cross\", from = {0.45, 0.5}, to = {0.55, 0.5}, colour = \"green\"}", ApiKind::Game(H::api_hud_line)),
    api("hud_update", "id, {settings}", "Changes what an item of the screen shows (the same settings as when it was made). Returns false when there is no such item.", "hud_update(\"kills\", {value = 3})", ApiKind::Game(H::api_hud_update)),
    api("hud_remove", "[id]", "Takes an item off the screen; without an id, every item of the mod's scripts.", "hud_remove(\"goal\")", ApiKind::Game(H::api_hud_remove)),
    api("text_size", "text[, size or font]", "The width and height of a line of text, as parts of a 640x480 screen (multiply by 640 and 480 for its pixels there).", "local w, h = text_size(\"Hello\", \"small\")", ApiKind::Game(H::api_text_size)),
    api("text", "section, key[, file]", "A text of the game's language files in the player's language (file \"Engine\" by default), or nil.", "local recap = text(\"EndMsgMap\", \"Banque01\")", ApiKind::Game(H::api_text)),
    api("hold_start", "", "In an on(\"loading\") handler: the next level does not start until release_start(); the loading screen shows the hud_... items the loading handlers made.", "on(\"loading\", function() hud_text{text = \"...\", x = 0.5, y = 0.5} hold_start() end)", ApiKind::Game(H::api_hold_start)),
    api("release_start", "", "Lets the held level start (at once when it is ready, otherwise as soon as it is, behind the game's loading animation).", "on(\"loading_key\", function() release_start() end)", ApiKind::Game(H::api_release_start)),
    hidden("actor.get", ApiKind::Game(api_actor_get)),
    hidden("actor.set", ApiKind::Game(api_actor_set)),
    hidden("actor.text", ApiKind::Game(api_actor_text)),
];

/// The fields of an actor, for the reference: name, read/write, description.
pub static ACTOR_FIELDS: &[(&str, bool, &str)] = &[
    ("name", false, "its name in the map (\"Cine0\", \"MedPick2\")"),
    ("class", false, "its class (\"XIDPawn.BaseSoldier\")"),
    ("tag", false, "its Tag"),
    ("alive", false, "false once it is dead or removed"),
    ("health", true, "its health (characters); above 0"),
    ("position", true, "{x, y, z}: the point on the ground where it stands"),
    ("yaw", true, "the direction it faces, in degrees (0 faces +X, 90 faces +Y)"),
    ("team", true, "\"friend\", \"neutral\" or \"enemy\": how a character treats the players (nil: the game's own)"),
    ("weapon", false, "the class of the weapon in its hands (give() gives another)"),
    ("charred", true, "how black it is drawn, from 0 (as the game draws it) to 1 (black): burnt characters and objects. A character's worn pieces (vest, helmet, beret) go with it, never its weapons"),
    ("size", false, "its widest side as it is drawn, in units (a man is about 170 tall): a door, a crate or a whole hall"),
    ("ammo", true, "the ammunition of the weapon in a character's hands (or of a weapon), nil without one"),
    ("muzzle", false, "{x, y, z}: on a weapon held with a FirstPersonModel (p:get(\"Weapon\").muzzle), the front end of that model as its owner sees it in his hands, in first person: the point of the world his view shows there (an effect put there is seen on the model); nil otherwise (no such model, or not drawn on this computer)"),
    ("ground", false, "what is under a character's feet: the name of the level's footsteps there (\"PNJFSNei\" or \"PNJFSNei__hPNJFSNei\": snow, Met: metal, Hrb: grass...), or nil"),
];

/// The events of `on(event, ..., fn)`, for the reference: event, what follows it, what fn
/// receives, when.
pub static EVENTS: &[(&str, &str, &str, &str)] = &[
    ("start", "", "player", "when the map has started and a player is there"),
    ("near", "who, distance", "player, actor", "when a player comes within distance (300 by default) of who; not during a cut-scene"),
    ("enter", "position, radius", "player", "when a player comes within radius of a point"),
    ("sees", "who", "actor, player", "when who sees a player (with its own eyes)"),
    ("death", "who", "actor", "when who dies"),
    ("damage", "who", "actor, amount, by, damage_type, weapon", "when who loses health: by is who hit it (nil when the game does not say), damage_type the kind of damage (\"XIII.DTGunned\"), weapon the class of the weapon by held then"),
    ("fire", "[who]", "shooter, weapon_class", "at each shot of a player (without who), of who, or of every character (\"all\")"),
    ("pickup", "", "player, class", "when an item comes into a player's inventory"),
    ("event", "name", "player", "when the map fires that event (a trigger, a door, a scene); the player who caused it"),
    ("key", "key", "player", "when a player presses the key (A-Z, 0-9, F1, F2, F10, F12...; the game window keeps F3 to F9 and F11)"),
    ("map_end", "", "map", "when the map is left for another one (a new level, a checkpoint loaded, the menus)"),
    ("loading", "", "from_map, to_map, failed", "when the next map starts loading, after map_end; failed is \"failed\" after a lost mission. What it draws with hud_... is the loading screen, and hold_start() keeps it until release_start()"),
    ("loaded", "", "map", "while the start is held: the next map is ready (the handler is the new map's script)"),
    ("loading_key", "", "","while the start is held: at each press of a key, a button or the screen"),
];

/// The reference page of the scripts' functions (docs/modding/script/functions.md), made
/// from the API table.
pub fn reference_markdown() -> String {
    let cell = |s: &str| s.replace('|', r"\|");
    let mut out = String::from("# All the functions\n\n");
    out.push_str("Made from the game's own list of script functions (a test checks it is up to date). The other pages show\n");
    out.push_str("how to use them: [scripts](README.md).\n\n");
    out.push_str("## Events: on(event, ..., fn)\n\n| Event | Then | fn receives | When |\n|---|---|---|---|\n");
    for (e, args, gets, when) in EVENTS {
        let args = if args.is_empty() { "-".to_string() } else { format!("`{args}`") };
        let gets = if gets.is_empty() { "-".to_string() } else { format!("`{gets}`") };
        out.push_str(&format!("| `\"{e}\"` | {args} | {gets} | {when} |\n"));
    }
    out.push_str("\n`on` returns a number that `cancel` takes.\n\n## Functions\n\n| Function | What it does | Example |\n|---|---|---|\n");
    for f in API.iter().filter(|f| !f.hidden && !f.method) {
        out.push_str(&format!("| `{}({})` | {} | `{}` |\n", f.name, f.args, cell(f.about), cell(f.example)));
    }
    out.push_str("\n## Actors\n\nWhat `find`, `spawn`, `player` and the events give is an actor.\n\n| Field | Can be changed | What it is |\n|---|---|---|\n");
    for (name, rw, about) in ACTOR_FIELDS {
        out.push_str(&format!("| `actor.{name}` | {} | {about} |\n", if *rw { "yes" } else { "no" }));
    }
    out.push_str("\n| Order | What it does | Example |\n|---|---|---|\n");
    for f in API.iter().filter(|f| f.method) {
        out.push_str(&format!("| `actor:{}({})` | {} | `{}` |\n", f.name, f.args, cell(f.about), cell(f.example)));
    }
    out.push_str("\n## In a network game\n\n");
    out.push_str("Scripts run on the host only; `players()` lists the guests' characters too. What the scripts show is sent to the\n");
    out.push_str("guests: the actors made by `effect{}` appear on every guest, and what a script does to them afterwards follows them:\n");
    out.push_str("properties written with `set`, functions called with `call` (except `SetOwner` and `SetBase`, which the game sends\n");
    out.push_str("itself), sounds played from them, and their moves (`SetLocation`, `SetRotation`, at most ten times a second). An\n");
    out.push_str("effect that only its owner sees (`bOnlyOwnerSee`: a gun's flame in first person) is placed again at the guest's own\n");
    out.push_str("view every frame, without delay; one owned by a character follows that character. `charred`, `char_area`,\n");
    out.push_str("`retexture` and `heat` are sent as well. A guest who joins late gets the charred actors and areas, the textures and\n");
    out.push_str("every effect still there, as they are now. The mods' log tells once a minute what this costs.\n\n");
    out.push_str("A guest's fire button is not seen on the host (`bFire` stays 0 there): the guest's gun is fired by the host shot\n");
    out.push_str("after shot (its ammunition goes down), and the weapon's `StopFiringTime` changes when the guest lets go of an\n");
    out.push_str("automatic weapon's button.\n");
    out.push_str("\n## Lua\n\nScripts are Lua 5.4. They can use `string`, `table`, `math` and `coroutine` (each mod has its own copy),\n");
    out.push_str("and the base functions without the ones that read files or load code. `save` is the mod's memory\n");
    out.push_str("(numbers, strings, true/false and tables of them).\n");
    out
}

/// The API table as rows (name, arguments, description, example), methods written
/// `actor:name`: for the modding catalog.
pub fn reference_rows() -> Vec<(String, &'static str, &'static str, &'static str)> {
    API.iter().filter(|f| !f.hidden).map(|f| (if f.method { format!("actor:{}", f.name) } else { f.name.to_string() }, f.args, f.about, f.example)).collect()
}

fn op_index(name: &str) -> usize {
    API.iter().position(|f| f.name == name).expect("API function")
}

// ---------------------------------------------------------------------------------------------
// Lua values.

/// An actor of the game, as the scripts see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor(pub ObjId);

impl UserData for Actor {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::Index, |lua, this, key: String| call_game(lua, op_index("actor.get"), (*this, key).into_lua_multi(lua)?));
        m.add_meta_method(mlua::MetaMethod::NewIndex, |lua, this, (key, value): (String, mlua::Value)| {
            call_game(lua, op_index("actor.set"), (*this, key, value).into_lua_multi(lua)?).map(|_| ())
        });
        m.add_meta_method(mlua::MetaMethod::Eq, |_, this, other: mlua::AnyUserData| Ok(other.borrow::<Actor>().is_ok_and(|o| *o == *this)));
        m.add_meta_method(mlua::MetaMethod::ToString, |lua, this, ()| call_game(lua, op_index("actor.text"), (*this,).into_lua_multi(lua)?));
    }
}

impl FromLua for Actor {
    fn from_lua(value: mlua::Value, _: &Lua) -> mlua::Result<Self> {
        match &value {
            mlua::Value::UserData(u) => u.borrow::<Actor>().map(|a| *a).map_err(|_| mlua::Error::runtime("an actor was expected")),
            _ => Err(mlua::Error::runtime(format!("an actor was expected, got {}", value.type_name()))),
        }
    }
}

/// Who an API function is about: an actor, or a name for `find`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Who {
    Name(String),
    Actor(ObjId),
}

impl FromLua for Who {
    fn from_lua(value: mlua::Value, _: &Lua) -> mlua::Result<Self> {
        match &value {
            mlua::Value::String(s) => Ok(Who::Name(s.to_str()?.to_string())),
            mlua::Value::UserData(u) => u.borrow::<Actor>().map(|a| Who::Actor(a.0)).map_err(|_| mlua::Error::runtime("an actor or a name was expected")),
            _ => Err(mlua::Error::runtime(format!("an actor or a name was expected, got {}", value.type_name()))),
        }
    }
}

impl std::fmt::Display for Who {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Who::Name(n) => write!(f, "{n}"),
            Who::Actor(a) => write!(f, "actor #{a}"),
        }
    }
}

/// `{x, y, z}` or `{x = , y = , z = }`.
pub(crate) fn read_position(t: &Table) -> mlua::Result<[f32; 3]> {
    let mut out = [0.0; 3];
    for (i, k) in ["x", "y", "z"].iter().enumerate() {
        let v: Option<f32> = match t.raw_get::<Option<f32>>(i + 1)? {
            Some(v) => Some(v),
            None => t.raw_get(*k)?,
        };
        out[i] = v.ok_or_else(|| mlua::Error::runtime("a position is {x, y, z}"))?;
    }
    Ok(out)
}

fn position_table(lua: &Lua, p: [f32; 3]) -> mlua::Result<Table> {
    lua.create_sequence_from(p.iter().map(|v| v.round()))
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Friend => "friend",
        Team::Neutral => "neutral",
        Team::Enemy => "enemy",
    }
}

pub(crate) fn parse_team(s: &str) -> mlua::Result<Team> {
    match s.to_ascii_lowercase().as_str() {
        "friend" => Ok(Team::Friend),
        "neutral" => Ok(Team::Neutral),
        "enemy" => Ok(Team::Enemy),
        _ => Err(mlua::Error::runtime(format!("team {s:?}: friend, neutral or enemy"))),
    }
}

fn actor_or_nil(lua: &Lua, a: Option<ObjId>) -> mlua::Result<mlua::Value> {
    match a {
        Some(a) => lua.create_userdata(Actor(a)).map(mlua::Value::UserData),
        None => Ok(mlua::Value::Nil),
    }
}

// ---------------------------------------------------------------------------------------------
// The scripts' state (Lua app data).

#[derive(Debug, Clone)]
enum Trigger {
    Start {
        fired: bool,
    },
    Near {
        who: Who,
        distance: f32,
        inside: BTreeSet<ObjId>,
    },
    /// `first_new`: the objects from this index on came after the handler (ObjIds only grow);
    /// a character of before that is already dead when the handler first looks is not reported.
    Death {
        who: Who,
        known: BTreeSet<ObjId>,
        fired: BTreeSet<ObjId>,
        first_new: ObjId,
        primed: bool,
    },
    Damage {
        who: Who,
        last: BTreeMap<ObjId, i32>,
    },
    /// `on("fire"[, who])`: the shots of the players (no who), of every character ("all") or
    /// of who.
    Fire {
        who: Option<Who>,
        seen: crate::mod_script_more::FireSeen,
    },
    /// The map changes (mod_script_hud.rs): "map_end", "loading", "loaded", "loading_key", run
    /// by the window, never by the frames.
    Loading {
        event: &'static str,
    },
    Pickup {
        seen: BTreeMap<ObjId, BTreeSet<ObjId>>,
    },
    Sees {
        who: Who,
        seen: BTreeSet<(ObjId, ObjId)>,
        next: f32,
    },
    Enter {
        at: [f32; 3],
        radius: f32,
        inside: BTreeSet<ObjId>,
    },
    /// `dispatcher`: a Dispatcher of the game with the same Tag, whose Trigger keeps the
    /// event's instigator.
    Event {
        name: String,
        listener: Option<ObjId>,
        dispatcher: Option<ObjId>,
        tried: bool,
    },
    Key {
        key: String,
    },
}

impl Trigger {
    fn describe(&self) -> String {
        match self {
            Trigger::Start { .. } => "start".into(),
            Trigger::Near { who, .. } => format!("near {who}"),
            Trigger::Death { who, .. } => format!("death {who}"),
            Trigger::Damage { who, .. } => format!("damage {who}"),
            Trigger::Fire { who: Some(who), .. } => format!("fire {who}"),
            Trigger::Fire { who: None, .. } => "fire".into(),
            Trigger::Loading { event } => event.to_string(),
            Trigger::Pickup { .. } => "pickup".into(),
            Trigger::Sees { who, .. } => format!("sees {who}"),
            Trigger::Enter { at, radius, .. } => format!("enter {{{}, {}, {}}} {radius}", at[0], at[1], at[2]),
            Trigger::Event { name, .. } => format!("event {name}"),
            Trigger::Key { key } => format!("key {key}"),
        }
    }
}

struct Handler {
    id: u64,
    trigger: Trigger,
    func: Function,
    /// "<file>:<line>" of the `on` call.
    label: String,
}

struct Timer {
    id: u64,
    at: f32,
    period: Option<f32>,
    func: Function,
    label: String,
}

/// A run paused by `wait`.
struct Task {
    thread: Thread,
    wake: f32,
    label: String,
    origin: Option<u64>,
}

/// An argument given to a handler.
#[derive(Debug, Clone)]
enum Arg {
    Actor(ObjId),
    Int(i64),
    Str(String),
    Nil,
}

enum Job {
    Start { func: Function, args: Vec<Arg>, label: String, origin: Option<u64>, repeat: bool },
    Resume { thread: Thread, label: String, origin: Option<u64> },
}

#[derive(Default)]
pub(crate) struct State {
    handlers: Vec<Handler>,
    timers: Vec<Timer>,
    tasks: Vec<Task>,
    next_id: u64,
    /// Level time of the run being made.
    pub(crate) now: f32,
    /// Keys pressed since the last step, with the player who pressed them.
    keys: Vec<(String, Option<ObjId>)>,
    /// Actors taken out by `remove` (their death does not count).
    removed: BTreeSet<ObjId>,
    /// The number of objects at the last frame of the scripts (a handler registered now
    /// knows which characters come after it).
    pub(crate) objects_seen: usize,
    /// Story characters following another: follower -> (leader, next move time).
    pub(crate) follows: BTreeMap<ObjId, Follow>,
    /// Hidden marker actors: of each character given a position, and of the sounds (usize::MAX).
    pub(crate) markers: BTreeMap<ObjId, ObjId>,
    /// behaviour(name, fn): the behaviours defined.
    pub(crate) behaviours: BTreeMap<String, Function>,
    /// Behaviours to start at the next frame: (character, fn).
    pub(crate) behaviours_new: Vec<(ObjId, Function)>,
    /// The behaviour running on each character: its runs' origin id.
    pub(crate) behaving: BTreeMap<ObjId, u64>,
    /// Patrols: character -> (points, next point, run).
    pub(crate) patrols: BTreeMap<ObjId, (Vec<[f32; 3]>, usize, bool)>,
    /// A scene's end (camera(..., seconds)): level time.
    pub(crate) camera_until: Option<f32>,
    /// camera{path =, follow =, orbit =}: the camera that moves (mod_script_camera.rs).
    pub(crate) camera_move: Option<crate::mod_script_camera::CameraMove>,
    /// slow_motion(factor, seconds): the level time at which normal speed comes back.
    pub(crate) slow_until: Option<f32>,
    /// fade{}: the fade under way (mod_script_effects.rs).
    pub(crate) fade: Option<crate::mod_script_effects::Fade>,
    /// campaign{...}: the maps of the mod's campaign, in order.
    pub(crate) campaign: Vec<String>,
    /// Warnings found while registering (a key the game window keeps), written at the next step.
    pub(crate) notes: Vec<String>,
    /// panel{...}: the insets to show, with their level time and the call's place.
    pub(crate) panels: Vec<(f32, crate::mod_script_panel::Panel, String)>,
}

fn api_campaign(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("campaign{\"FirstMap\", \"SecondMap\", ...}"))?;
    let mut maps = Vec::new();
    for i in 1..=t.raw_len() {
        maps.push(t.raw_get::<String>(i)?);
    }
    if maps.is_empty() {
        return Err(mlua::Error::runtime("campaign{...}: give the maps in order"));
    }
    lua.app_data_mut::<State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?.campaign = maps;
    ().into_lua_multi(lua)
}

fn api_behaviour(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (name, f) = <(String, Function)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("behaviour(name, fn)"))?;
    let mut st = lua.app_data_mut::<State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    st.behaviours.insert(name.to_ascii_lowercase(), f);
    ().into_lua_multi(lua)
}

/// A character following another (follow()).
#[derive(Debug, Clone)]
pub(crate) struct Follow {
    pub leader: ObjId,
    /// Level time of the next look at how far the leader is.
    pub next: f32,
    /// When the follower was last given its way: where it stood, where its leader stood, the
    /// level time (to see it is stuck, or its leader has gone elsewhere).
    pub last: Option<([f32; 3], [f32; 3], f32)>,
    /// follow(actor, {catch_up = distance}): beyond it, out of the players' sight, the
    /// follower is put back near its leader.
    pub catch_up: Option<f32>,
    /// A go_to watched until it arrives (then the entry goes), not a follow.
    pub arrive: bool,
}

/// The scripts' state of a Lua state (its app data).
pub(crate) fn state_mut(lua: &Lua) -> Option<mlua::AppDataRefMut<'_, State>> {
    // None while it is borrowed (never a panic).
    lua.try_app_data_mut::<State>().ok().flatten()
}

/// The scripts' own random numbers (xorshift64*): never the game's.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn api_random(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (m, n) =
        <(Option<i64>, Option<i64>)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("random(), random(n) or random(m, n): whole numbers"))?;
    let mut r = lua.app_data_mut::<Random>().ok_or_else(|| mlua::Error::runtime("no random numbers"))?;
    let x = r.next();
    let (lo, hi) = match (m, n) {
        (None, _) => return ((x >> 11) as f64 / (1u64 << 53) as f64).into_lua_multi(lua),
        (Some(n), None) => (1, n),
        (Some(m), Some(n)) => (m, n),
    };
    if lo > hi {
        return Err(mlua::Error::runtime(format!("random({lo}, {hi}): the first number is larger")));
    }
    let span = (hi - lo) as u64 + 1;
    (lo + (x % span) as i64).into_lua_multi(lua)
}

impl State {
    /// Stops the runs paused by wait of handler, timer or behaviour `id`.
    pub(crate) fn stop_runs(&mut self, id: u64) {
        self.tasks.retain(|t| t.origin != Some(id));
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// Instructions left to the run being made.
#[derive(Default)]
struct Budget {
    left: i64,
    exhausted: bool,
    /// Hook calls since the budget ran out (the run could not be paused there).
    over: u32,
}

/// `log` lines written on this map.
#[derive(Default)]
struct LogCount(usize);

/// The Lua state of the current map's scripts.
pub struct ScriptRuntime {
    lua: Lua,
    /// Hidden actors that hear the map's events (destroyed on `modreload`).
    listeners: Vec<ObjId>,
    /// Each mod's global table (its `save` table is kept by the game).
    envs: Vec<(String, Table)>,
}

impl ScriptRuntime {
    /// Each mod's global table, with the mod's folder name.
    pub(crate) fn envs(&self) -> &[(String, Table)] {
        &self.envs
    }
}

/// "<file>:<line>" of the Lua code that called the current Rust function.
pub(crate) fn caller(lua: &Lua) -> String {
    for level in 1..4 {
        let found = lua.inspect_stack(level, |d| {
            let src = d.source();
            let line = d.current_line();
            let file = src.short_src.as_deref().unwrap_or("?").to_string();
            // A "when" rule's code is named after the rule's place in its toml file
            // ("my-mod/mod.toml line 12"): that place alone.
            (src.what != "C").then(|| if file.contains(" line ") { file } else { format!("{file}:{}", line.unwrap_or(0)) })
        });
        match found {
            Some(Some(l)) => return l,
            Some(None) => continue,
            None => break,
        }
    }
    "script".into()
}

/// The text of a script error: "<file>:<line>: <what>", without Lua's traceback.
pub(crate) fn error_text(e: &mlua::Error) -> String {
    fn innermost(e: &mlua::Error) -> &mlua::Error {
        match e {
            mlua::Error::CallbackError { cause, .. } => innermost(cause),
            mlua::Error::WithContext { cause, .. } => innermost(cause),
            e => e,
        }
    }
    fn location(e: &mlua::Error) -> Option<String> {
        match e {
            mlua::Error::CallbackError { traceback, cause } => {
                let inner = location(cause);
                inner.or_else(|| traceback.lines().map(str::trim).find(|l| l.contains(".lua:")).and_then(|l| l.split(": ").next()).map(str::to_string))
            }
            mlua::Error::WithContext { cause, .. } => location(cause),
            _ => None,
        }
    }
    let inner = innermost(e);
    let text = match inner {
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => m.clone(),
        mlua::Error::MemoryError(m) => format!("out of memory ({m})"),
        other => other.to_string(),
    };
    let text = text.lines().take_while(|l| !l.starts_with("stack traceback")).collect::<Vec<_>>().join(" ").trim().to_string();
    match location(e) {
        Some(at) if !text.contains(".lua:") => format!("{at}: {text}"),
        _ => text,
    }
}

/// Calls API function `op` through the gate (an error outside a run of the game).
fn call_game(lua: &Lua, op: usize, args: MultiValue) -> mlua::Result<MultiValue> {
    match lua.named_registry_value::<Option<Function>>(GATE)? {
        Some(gate) => gate.call((op, args)),
        None => Err(mlua::Error::runtime(format!("{} can only be used while the game runs the script", API[op].name))),
    }
}

/// The handlers' functions given by the scripts: what follows the event's own arguments.
fn last_function(args: &mut Vec<mlua::Value>, what: &str) -> mlua::Result<Function> {
    match args.pop() {
        Some(mlua::Value::Function(f)) => Ok(f),
        _ => Err(mlua::Error::runtime(format!("{what}: the last argument must be a function"))),
    }
}

fn api_on(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let mut args: Vec<mlua::Value> = args.into_iter().collect();
    if args.is_empty() {
        return Err(mlua::Error::runtime("on(event, ..., fn): start, near, enter, sees, death, damage, fire, pickup, event, key, map_end, loading, loaded or loading_key"));
    }
    let event = String::from_lua(args.remove(0), lua)?;
    let func = last_function(&mut args, &format!("on(\"{event}\")"))?;
    let mut rest = args.into_iter();
    let mut next = |what: &str| rest.next().ok_or_else(|| mlua::Error::runtime(format!("on(\"{event}\", ...): {what} is missing")));
    let trigger = match event.to_ascii_lowercase().as_str() {
        "start" => Trigger::Start { fired: false },
        "near" => {
            let who = Who::from_lua(next("who")?, lua)?;
            let distance = match rest.next() {
                Some(v) => f32::from_lua(v, lua)?,
                None => NEAR_DISTANCE,
            };
            Trigger::Near { who, distance, inside: BTreeSet::new() }
        }
        "death" => {
            let first_new = lua.app_data_ref::<State>().map_or(0, |s| s.objects_seen);
            Trigger::Death { who: Who::from_lua(next("who")?, lua)?, known: BTreeSet::new(), fired: BTreeSet::new(), first_new, primed: false }
        }
        "damage" => Trigger::Damage { who: Who::from_lua(next("who")?, lua)?, last: BTreeMap::new() },
        "fire" => {
            let who = match rest.next() {
                None | Some(mlua::Value::Nil) => None,
                Some(v) => Some(Who::from_lua(v, lua)?),
            };
            Trigger::Fire { who, seen: BTreeMap::new() }
        }
        "map_end" => Trigger::Loading { event: "map_end" },
        "loading" => Trigger::Loading { event: "loading" },
        "loaded" => Trigger::Loading { event: "loaded" },
        "loading_key" => Trigger::Loading { event: "loading_key" },
        "pickup" => Trigger::Pickup { seen: BTreeMap::new() },
        "sees" => Trigger::Sees { who: Who::from_lua(next("who")?, lua)?, seen: BTreeSet::new(), next: 0.0 },
        "enter" => {
            let at = read_position(&Table::from_lua(next("the position")?, lua)?)?;
            let radius = match rest.next() {
                Some(v) => f32::from_lua(v, lua)?,
                None => NEAR_DISTANCE,
            };
            Trigger::Enter { at, radius, inside: BTreeSet::new() }
        }
        "event" => Trigger::Event { name: String::from_lua(next("the event's name")?, lua)?, listener: None, dispatcher: None, tried: false },
        "key" => {
            let key = String::from_lua(next("the key")?, lua)?;
            // The game window keeps some keys (co-op screen, cheats): said once.
            if let Some(w) = openxiii_mods::behaviour::reserved_key_warning(&key) {
                let line = format!("on(\"key\", \"{key}\") ({}): {w}", caller(lua));
                if let Some(mut st) = lua.app_data_mut::<State>() {
                    st.notes.push(line);
                }
            }
            Trigger::Key { key }
        }
        _ => {
            let names: Vec<&str> = EVENTS.iter().map(|e| e.0).collect();
            return Err(mlua::Error::runtime(format!("on: unknown event {event:?} ({})", names.join(", "))));
        }
    };
    let label = caller(lua);
    let mut st = lua.app_data_mut::<State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    let id = st.new_id();
    st.handlers.push(Handler { id, trigger, func, label });
    id.into_lua_multi(lua)
}

fn add_timer(lua: &Lua, args: MultiValue, repeat: bool) -> mlua::Result<MultiValue> {
    let name = if repeat { "every" } else { "after" };
    let (seconds, func) = <(f32, Function)>::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime(format!("{name}(seconds, fn)")))?;
    if !seconds.is_finite() || seconds < 0.0 || (repeat && seconds <= 0.0) {
        return Err(mlua::Error::runtime(format!("{name}: {seconds} seconds")));
    }
    let label = caller(lua);
    let mut st = lua.app_data_mut::<State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    let id = st.new_id();
    let at = st.now + seconds;
    st.timers.push(Timer { id, at, period: repeat.then_some(seconds), func, label });
    id.into_lua_multi(lua)
}

fn api_after(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add_timer(lua, args, false)
}

fn api_every(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    add_timer(lua, args, true)
}

fn api_cancel(lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let id = u64::from_lua_multi(args, lua)?;
    let mut st = lua.app_data_mut::<State>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    let before = st.handlers.len() + st.timers.len();
    st.handlers.retain(|h| h.id != id);
    st.timers.retain(|t| t.id != id);
    // Its runs paused by wait stop too.
    st.tasks.retain(|t| t.origin != Some(id));
    (before != st.handlers.len() + st.timers.len()).into_lua_multi(lua)
}

fn api_map(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    vm.mod_map_name().into_lua_multi(lua)
}

fn api_find(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let who = Who::from_lua_multi(args, lua)?;
    actor_or_nil(lua, vm.mod_script_resolve(&who))?.into_lua_multi(lua)
}

fn api_find_all(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let name = String::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("find_all(who): a name"))?;
    let t = lua.create_table()?;
    for a in vm.mod_script_resolve_all(&name) {
        t.raw_push(Actor(a))?;
    }
    t.into_lua_multi(lua)
}

fn api_player(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    let p = vm.mod_player_pawn().or_else(|| vm.mod_script_players().first().copied());
    actor_or_nil(lua, p)?.into_lua_multi(lua)
}

fn api_players(vm: &mut Vm, lua: &Lua, _: MultiValue) -> mlua::Result<MultiValue> {
    let list = vm.mod_script_players();
    let t = lua.create_table()?;
    for p in list {
        t.raw_push(Actor(p))?;
    }
    t.into_lua_multi(lua)
}

fn api_spawn(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let t = Table::from_lua_multi(args, lua).map_err(|_| mlua::Error::runtime("spawn{character = or class =, position =, yaw =, team =}"))?;
    for pair in t.pairs::<mlua::Value, mlua::Value>() {
        let (k, _) = pair?;
        let ok = matches!(&k, mlua::Value::String(s) if matches!(s.to_str().map(|s| s.to_string()).as_deref(), Ok("character" | "class" | "position" | "near" | "yaw" | "team" | "behaviour")));
        if !ok {
            return Err(mlua::Error::runtime(format!("spawn: unknown setting {k:?} (character, class, position, near, yaw, team, behaviour)")));
        }
    }
    let character: Option<String> = t.get("character")?;
    let class: Option<String> = t.get("class")?;
    let what = match (character, class) {
        (Some(c), None) => SpawnWhat::Character(c),
        (None, Some(c)) => SpawnWhat::Class(c),
        _ => return Err(mlua::Error::runtime("spawn: give character = or class =")),
    };
    // position: that point; near: an actor or a point, and the first spot around it with room
    // and ground (at 150, 250 then 350 units, eight directions).
    let near = t.get::<mlua::Value>("near")?;
    let spots: Vec<[f32; 3]> = if !near.is_nil() {
        let at = match near {
            mlua::Value::Table(p) => read_position(&p)?,
            other => {
                let a = vm.mod_script_resolve(&Who::from_lua(other, lua)?).ok_or_else(|| mlua::Error::runtime("spawn: near = nobody of that name"))?;
                let [x, y, z] = vm.native_value(a, Field::Location).as_vector();
                [x, y, z - vm.native_prop(a, Field::CollisionHeight).as_float()]
            }
        };
        let mut out = Vec::new();
        for r in [150.0f32, 250.0, 350.0] {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                out.push([at[0] + r * a.dcos(), at[1] + r * a.dsin(), at[2]]);
            }
        }
        out
    } else {
        vec![read_position(&t.get::<Table>("position").map_err(|_| mlua::Error::runtime("spawn: give position = {x, y, z} or near = actor"))?)?]
    };
    let yaw: f32 = t.get::<Option<f32>>("yaw")?.unwrap_or(0.0);
    let team = t.get::<Option<String>>("team")?.map(|s| parse_team(&s)).transpose()?;
    let behaviour = t.get::<mlua::Value>("behaviour")?;
    let behaviour = if behaviour.is_nil() { None } else { Some(G::behaviour_function(lua, behaviour)?) };
    let label = caller(lua);
    let mut last = String::new();
    let mut a = None;
    for position in spots {
        match vm.mod_script_spawn(&what, position, yaw, team) {
            Ok(x) => {
                vm.mod_report(format!("{}: spawn {what}: {} ({label})", vm.mod_map_name(), vm.mod_actor_name(x)));
                a = Some(x);
                break;
            }
            Err(why) => last = why,
        }
    }
    if a.is_none() {
        vm.mod_report(format!("warning: {}: spawn {what}: {last} ({label})", vm.mod_map_name()));
    }
    if let (Some(a), Some(f)) = (a, behaviour) {
        if let Some(mut st) = lua.app_data_mut::<State>() {
            st.behaviours_new.push((a, f));
        }
    }
    actor_or_nil(lua, a)?.into_lua_multi(lua)
}

fn api_remove(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let who = Who::from_lua_multi(args, lua)?;
    let label = caller(lua);
    let removed = match vm.mod_script_resolve(&who) {
        None => {
            vm.mod_report(format!("warning: {}: remove {who}: nobody of that name ({label})", vm.mod_map_name()));
            None
        }
        Some(a) => match vm.mod_script_remove(a) {
            Ok(()) => Some(a),
            Err(why) => {
                vm.mod_report(format!("warning: {}: remove {who}: {why} ({label})", vm.mod_map_name()));
                None
            }
        },
    };
    if let Some(a) = removed {
        if let Some(mut st) = lua.app_data_mut::<State>() {
            st.removed.insert(a);
        }
    }
    removed.is_some().into_lua_multi(lua)
}

fn api_say(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (who, line) = <(Who, mlua::Value)>::from_lua_multi(args, lua)?;
    let (mut game_line, mut text, mut sound) = (None, None, None);
    match line {
        mlua::Value::String(s) => {
            let s = s.to_str()?.to_string();
            let is_line = openxiii_mods::dialogue::parse_line_name(&s).is_some() && vm.mod_game_line_text(&s).is_some();
            if is_line {
                game_line = Some(s);
            } else {
                text = Some(s);
            }
        }
        mlua::Value::Table(t) => {
            game_line = t.get("line")?;
            text = t.get("text")?;
            sound = t.get("sound")?;
            if game_line.is_none() && text.is_none() && sound.is_none() {
                return Err(mlua::Error::runtime("say: give line =, text = or sound ="));
            }
        }
        other => return Err(mlua::Error::runtime(format!("say(who, line): a line was expected, got {}", other.type_name()))),
    }
    let label = caller(lua);
    let ok = vm.mod_script_say(&who, game_line, text, sound, &label);
    ok.into_lua_multi(lua)
}

fn api_message(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (text, who) = <(String, Option<mlua::Value>)>::from_lua_multi(args, lua)?;
    let only = G::only_player(vm, lua, who, "message")?;
    for p in vm.mod_script_player_controllers() {
        if only.is_some_and(|o| vm.native_prop(p, Field::Pawn).as_object() != Some(o)) {
            continue;
        }
        if let Some(f) = vm.find_function(p, "ClientMessage", false) {
            if let Err(e) = vm.call(f, p, vec![Some(Value::Str(text.clone())), None]) {
                vm.mod_report(format!("warning: message {text:?}: {e}"));
            }
        }
    }
    let line = format!("{}: message \"{text}\" ({})", vm.mod_map_name(), caller(lua));
    vm.mod_script_log(line);
    ().into_lua_multi(lua)
}

fn api_log(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    // An actor is written by its name here: its own __tostring would call the game again.
    let parts: Vec<String> = args
        .into_iter()
        .map(|v| match &v {
            mlua::Value::UserData(u) => match u.borrow::<Actor>() {
                Ok(a) => format!("actor {}", vm.mod_actor_name(a.0)),
                Err(_) => "userdata".into(),
            },
            _ => v.to_string().unwrap_or_else(|_| v.type_name().to_string()),
        })
        .collect();
    let mut count = lua.app_data_mut::<LogCount>().ok_or_else(|| mlua::Error::runtime("no scripts state"))?;
    count.0 += 1;
    let n = count.0;
    drop(count);
    if n <= LOG_LINES {
        vm.mod_script_log(format!("script {}: {}", caller(lua), parts.join(" ")));
    } else if n == LOG_LINES + 1 {
        vm.mod_script_log(format!("script: more than {LOG_LINES} log lines on this map, the next ones are not written"));
    }
    ().into_lua_multi(lua)
}

fn api_actor_get(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (actor, key) = <(Actor, String)>::from_lua_multi(args, lua)?;
    let a = actor.0;
    // A method (actor:go_to(...)).
    if API.iter().any(|f| f.method && f.name == key) {
        let methods: Table = lua.named_registry_value(METHODS)?;
        return methods.raw_get::<mlua::Value>(key)?.into_lua_multi(lua);
    }
    let exists = vm.objects.get(a).is_some_and(|o| o.is_some());
    let v: mlua::Value = match key.as_str() {
        "name" => vm.mod_actor_name(a).into_lua(lua)?,
        "alive" => (exists && vm.mod_alive(a)).into_lua(lua)?,
        _ if !exists => mlua::Value::Nil,
        "class" => vm.objects[a].as_ref().map(|o| vm.class_path(o.class)).into_lua(lua)?,
        "tag" => vm.prop(a, "Tag").as_name().into_lua(lua)?,
        "health" => {
            if vm.is_a(a, "Pawn") {
                vm.prop(a, "Health").as_int().into_lua(lua)?
            } else {
                mlua::Value::Nil
            }
        }
        "position" => {
            let [x, y, z] = vm.native_value(a, Field::Location).as_vector();
            let h = vm.native_prop(a, Field::CollisionHeight).as_float();
            mlua::Value::Table(position_table(lua, [x, y, z - h])?)
        }
        "yaw" => {
            let units = vm.native_value(a, Field::Rotation).as_rotator()[1];
            (units.rem_euclid(65536) as f32 * 360.0 / 65536.0).into_lua(lua)?
        }
        "team" => vm.mod_script_team(a).map(team_name).into_lua(lua)?,
        "charred" => (vm.mod_charred.get(&a).copied().unwrap_or(0.0) as f64).into_lua(lua)?,
        // Its widest side as drawn (the render box, as GetBoundingBox gives it), in units.
        "size" => {
            let (lo, hi) = vm.render_bounding_box(a, false);
            ((0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max) as f64).into_lua(lua)?
        }
        "weapon" => G::api_weapon(vm, lua, a)?,
        "ammo" => M::ammo_get(vm, lua, a)?,
        // Where a weapon held with a FirstPersonModel comes out of that model (mod_weapons.rs).
        // Its owner's view: where he sees it (the model is drawn with its own angles).
        "muzzle" => match vm.mod_first_person_muzzle(a) {
            Some(p) => {
                let viewer = vm.prop(a, "Owner").as_object().and_then(|o| vm.prop(o, "Controller").as_object());
                let p = viewer.map_or(p, |c| vm.first_person_seen(c, p));
                mlua::Value::Table(lua.create_sequence_from(p.iter().map(|&v| v as f64))?)
            }
            None => mlua::Value::Nil,
        },
        // The footsteps of the material under its feet (Material.PNJSndStep, what the game's
        // own PlayFootStep reads), by its name.
        "ground" => {
            // What is under its feet now (a line down through the level); without a hit,
            // what its last step met.
            let [x, y, z] = vm.native_value(a, Field::Location).as_vector();
            let h = vm.native_prop(a, Field::CollisionHeight).as_float();
            let mut hit = crate::physics::CheckResult::new(1.0);
            vm.single_line_check(&mut hit, None, [x, y, z - h - 60.0], [x, y, z], crate::physics::TRACE_WORLD, [0.0; 3]);
            let under = if hit.time < 1.0 { hit.material.and_then(|t| vm.collision_materials.get(t as usize).cloned()).filter(|m| !m.is_empty()) } else { None };
            let material = under.or_else(|| match vm.prop_slot(a, "LastCollidedMaterial") {
                Ok(_) => match vm.prop(a, "LastCollidedMaterial") {
                    Value::Asset(m) => Some(m),
                    _ => None,
                },
                Err(_) => None,
            });
            let name = |v: Value| match v {
                Value::Asset(s) => Some(s.rsplit('.').next().unwrap_or(&s).to_string()),
                _ => None,
            };
            // The material's own footsteps (a shader's come from its sub-materials): the
            // step table (PNJSndStep) or, where the level has none, the step sound
            // (FootstepSound); a terrain's are the terrain's.
            let mut step = None;
            if let Some(m) = material {
                vm.material_fill_sounds(&m);
                for field in ["PNJSndStep", "FootstepSound"] {
                    step = step.or_else(|| vm.asset_prop(&m, field).and_then(name));
                }
            }
            if step.is_none() {
                if let Some(floor) = hit.actor.or_else(|| vm.prop(a, "Base").as_object()) {
                    for field in ["PNJSndStep", "FootstepSound"] {
                        if vm.prop_slot(floor, field).is_ok() {
                            step = step.or_else(|| name(vm.prop(floor, field)));
                        }
                    }
                }
            }
            step.into_lua(lua)?
        }
        _ => return Err(mlua::Error::runtime(format!("an actor has no field {key:?} ({})", field_names()))),
    };
    v.into_lua_multi(lua)
}

fn field_names() -> String {
    ACTOR_FIELDS.iter().map(|f| f.0).collect::<Vec<_>>().join(", ")
}

fn api_actor_set(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let (actor, key, value) = <(Actor, String, mlua::Value)>::from_lua_multi(args, lua)?;
    let a = actor.0;
    if !vm.objects.get(a).is_some_and(|o| o.is_some()) || vm.native_prop(a, Field::DeleteMe).as_bool() {
        return Err(mlua::Error::runtime(format!("{key}: this actor is not in the game any more")));
    }
    match key.as_str() {
        "health" => {
            if !vm.is_a(a, "Pawn") {
                return Err(mlua::Error::runtime("health: only characters have health"));
            }
            let h = i32::from_lua(value, lua)?;
            if h <= 0 {
                return Err(mlua::Error::runtime("health: give a value above 0 (remove(who) takes an actor out)"));
            }
            vm.set_prop(a, "Health", Value::Int(h));
        }
        "position" => {
            let p = read_position(&Table::from_lua(value, lua)?)?;
            let h = vm.native_prop(a, Field::CollisionHeight).as_float();
            let mut args = [Value::vector(p[0], p[1], p[2] + h)];
            let ok = crate::natives::call(vm, "SetLocation", a, &mut args, &[true]).map_err(|e| mlua::Error::runtime(format!("position: {e}")))?;
            if !ok.as_bool() {
                return Err(mlua::Error::runtime(format!("position: no room for {} at {{{}, {}, {}}}", vm.mod_actor_name(a), p[0], p[1], p[2])));
            }
        }
        "yaw" => {
            let deg = f32::from_lua(value, lua)?;
            let [p, _, r] = vm.native_value(a, Field::Rotation).as_rotator();
            let units = (deg * 65536.0 / 360.0).round() as i32;
            let mut args = [Value::rotator(p, units.rem_euclid(65536), r)];
            crate::natives::call(vm, "SetRotation", a, &mut args, &[true]).map_err(|e| mlua::Error::runtime(format!("yaw: {e}")))?;
        }
        "team" => {
            let team = Option::<String>::from_lua(value, lua)?.map(|s| parse_team(&s)).transpose()?;
            if !vm.is_a(a, "Pawn") {
                return Err(mlua::Error::runtime("team: only characters have a team"));
            }
            match team {
                Some(t) => {
                    vm.mods.script_teams.insert(a, t);
                }
                None => {
                    vm.mods.script_teams.remove(&a);
                }
            }
        }
        "charred" => {
            let k = f32::from_lua(value, lua)?;
            if !k.is_finite() {
                return Err(mlua::Error::runtime("charred: a number from 0 to 1"));
            }
            let k = k.clamp(0.0, 1.0);
            // A character's worn pieces (a vest, a helmet, a beret: the inventory attachments
            // based on it that are not weapons nor muzzle flashes) go with it.
            let mut targets = vec![a];
            if vm.is_a(a, "Pawn") {
                if let Value::Array(attached) = vm.prop(a, "Attached") {
                    for x in attached.iter().filter_map(Value::as_object) {
                        let worn = vm.objects.get(x).is_some_and(|o| o.is_some())
                            && vm.is_a(x, "InventoryAttachment")
                            && !vm.is_a(x, "WeaponAttachment")
                            && !vm.is_a(x, "MuzzleFlashAttachment")
                            // (not a mod's own piece: the flamethrower's tanks)
                            && vm.objects[x].as_ref().is_some_and(|o| vm.mod_class_net_path(o.class).is_none());
                        if worn {
                            targets.push(x);
                        }
                    }
                }
            }
            for x in targets {
                if k > 0.0 {
                    vm.mod_charred.insert(x, k);
                } else {
                    vm.mod_charred.remove(&x);
                }
                // A static mesh of the scenery is drawn darker too (its own textures, darkened).
                let name = vm.mod_actor_name(x);
                vm.mods.surfaces.set_charred(&name, k);
            }
        }
        "ammo" => M::ammo_set(vm, lua, a, value)?,
        "weapon" => return Err(mlua::Error::runtime("weapon cannot be written: actor:give(weapon) gives one")),
        "name" | "class" | "tag" | "alive" | "ground" | "size" | "muzzle" => return Err(mlua::Error::runtime(format!("{key} cannot be changed"))),
        _ => return Err(mlua::Error::runtime(format!("an actor has no field {key:?} ({})", field_names()))),
    }
    ().into_lua_multi(lua)
}

fn api_actor_text(vm: &mut Vm, lua: &Lua, args: MultiValue) -> mlua::Result<MultiValue> {
    let actor = Actor::from_lua_multi(args, lua)?;
    format!("actor {}", vm.mod_actor_name(actor.0)).into_lua_multi(lua)
}

// ---------------------------------------------------------------------------------------------
// Lua state: sandbox and prelude.

const PRELUDE: &str = r#"
local yield, isyieldable = coroutine.yield, coroutine.isyieldable
local raw_pcall, raw_xpcall, stopping, text, now = pcall, xpcall, __stopping, __error_text, __now
function wait(seconds)
  if seconds ~= nil and type(seconds) ~= "number" then error("wait(seconds): a number was expected", 2) end
  if not isyieldable() then error("wait() works only inside a handler (on, after, every)", 2) end
  return yield(seconds or 0)
end
function wait_until(check, seconds)
  if type(check) ~= "function" then error("wait_until(fn[, seconds]): a function was expected", 2) end
  if seconds ~= nil and type(seconds) ~= "number" then error("wait_until(fn, seconds): a number was expected", 2) end
  if not isyieldable() then error("wait_until() works only inside a handler (on, after, every)", 2) end
  local left = seconds
  while not check() do
    if left ~= nil and left <= 0 then return false end
    local before = now()
    yield(0)
    if left ~= nil then left = left - (now() - before) end
  end
  return true
end
-- The player can move: there is a player and no cut-scene frame on the level.
function has_control()
  local p = player()
  if p == nil then return false end
  local ok, cine = raw_pcall(function() return p:get("Level"):get("bCineFrame") end)
  return ok and not cine
end
-- A run stopped for using too many instructions is not caught by pcall.
local function pass(ok, ...)
  if not ok and stopping() then error((...), 0) end
  -- An error of a game function is given as its text ("file:line: what").
  if not ok then return ok, text((...)), select(2, ...) end
  return ok, ...
end
function pcall(f, ...) return pass(raw_pcall(f, ...)) end
function xpcall(f, h, ...) return pass(raw_xpcall(f, h, ...)) end
-- pairs() in a fixed order: Lua's own order of string keys changes from one run of the game
-- to the next (its hash seed), which would make two machines of a lockstep session part. The
-- keys are taken as the loop starts and visited numbers first, then strings, then booleans,
-- each in increasing order; other keys (tables, actors, functions) after them, in Lua's order.
-- A key removed during the loop is skipped. A table with a __pairs metamethod keeps it.
do
  local raw_next, raw_type, raw_get, sort, getmt = next, type, rawget, table.sort, getmetatable
  local rank = {number = 1, string = 2, boolean = 3}
  local function before(a, b)
    local ra, rb = rank[raw_type(a)] or 4, rank[raw_type(b)] or 4
    if ra ~= rb then return ra < rb end
    if ra == 1 or ra == 2 then return a < b end
    if ra == 3 then return (not a) and b end
    return false
  end
  function pairs(t)
    local mt = getmt(t)
    if raw_type(mt) == "table" and mt.__pairs then return mt.__pairs(t) end
    if raw_type(t) ~= "table" then error("bad argument #1 to 'pairs' (table expected, got " .. raw_type(t) .. ")", 2) end
    local keys, others = {}, {}
    for k in raw_next, t do
      if rank[raw_type(k)] then keys[#keys + 1] = k else others[#others + 1] = k end
    end
    sort(keys, before)
    for _, k in ipairs(others) do keys[#keys + 1] = k end
    local i = 0
    return function()
      while true do
        i = i + 1
        local k = keys[i]
        if k == nil then return nil end
        local v = raw_get(t, k)
        if v ~= nil then return k, v end
      end
    end, t, nil
  end
end
"#;

/// The base functions scripts may not use.
const REMOVED: &[&str] = &["dofile", "loadfile", "load", "loadstring", "require", "collectgarbage", "module", "print"];

/// Lockstep co-op: Lua's math functions replaced by the portable ones (openxiii_math), and its own
/// generator seeded with a fixed value. The `^` operator still uses the C runtime's pow.
fn lockstep_math(lua: &Lua) -> mlua::Result<()> {
    use openxiii_math::SimMath as _;
    let Ok(math) = lua.globals().raw_get::<Table>("math") else { return Ok(()) };
    let one: [(&str, fn(f64) -> f64); 6] = [("sin", f64::dsin), ("cos", f64::dcos), ("tan", f64::dtan), ("asin", f64::dasin), ("acos", f64::dacos), ("exp", f64::dexp)];
    for (name, f) in one {
        math.raw_set(name, lua.create_function(move |_, x: f64| Ok(f(x)))?)?;
    }
    math.raw_set("atan", lua.create_function(|_, (y, x): (f64, Option<f64>)| Ok(y.datan2(x.unwrap_or(1.0))))?)?;
    math.raw_set("log", lua.create_function(|_, (x, b): (f64, Option<f64>)| Ok(match b { None => x.dln(), Some(b) if b == 2.0 => x.dlog2(), Some(b) if b == 10.0 => x.dlog10(), Some(b) => x.dln() / b.dln() }))?)?;
    lua.load("math.randomseed(0x5EED, 0x13)").set_name("=prelude").exec()?;
    Ok(())
}

fn new_lua() -> mlua::Result<Lua> {
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::COROUTINE, LuaOptions::new())?;
    lua.set_memory_limit(MEMORY_LIMIT)?;
    lua.set_app_data(State::default());
    lua.set_app_data(Budget::default());
    lua.set_app_data(LogCount::default());
    let g = lua.globals();
    for name in REMOVED {
        g.raw_set(*name, mlua::Value::Nil)?;
    }
    if let Ok(string) = g.raw_get::<Table>("string") {
        string.raw_set("dump", mlua::Value::Nil)?;
    }
    // The strings' shared metatable cannot be changed by a script.
    lua.load("getmetatable('').__metatable = false").set_name("=prelude").exec()?;
    g.raw_set("__stopping", lua.create_function(|lua, ()| Ok(lua.app_data_ref::<Budget>().is_some_and(|b| b.exhausted)))?)?;
    g.raw_set(
        "__error_text",
        lua.create_function(|lua, v: mlua::Value| match v {
            mlua::Value::Error(e) => error_text(&e).into_lua(lua),
            other => Ok(other),
        })?,
    )?;
    g.raw_set("__now", lua.create_function(|lua, ()| Ok(lua.app_data_ref::<State>().map_or(0.0, |s| s.now)))?)?;
    lua.load(PRELUDE).set_name("=prelude").exec()?;
    g.raw_set("__stopping", mlua::Value::Nil)?;
    g.raw_set("__error_text", mlua::Value::Nil)?;
    g.raw_set("__now", mlua::Value::Nil)?;
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9E37_79B9, |d| d.as_nanos() as u64) | 1;
    // Network co-op in lockstep (lockstep.rs): every machine draws the same numbers and computes
    // the same transcendental functions (the portable ones).
    let seed = if openxiii_math::portable() { 0x9E37_79B9_7F4A_7C15 } else { seed };
    lua.set_app_data(Random(seed));
    if openxiii_math::portable() {
        lockstep_math(&lua)?;
    }
    let methods = lua.create_table()?;
    for (i, f) in API.iter().enumerate() {
        let func = match f.kind {
            ApiKind::Lua => continue,
            ApiKind::Script(run) => lua.create_function(move |lua, args: MultiValue| run(lua, args))?,
            ApiKind::Game(_) if f.hidden => continue,
            ApiKind::Game(_) => lua.create_function(move |lua, args: MultiValue| call_game(lua, i, args))?,
        };
        if f.method {
            methods.raw_set(f.name, func)?;
        } else {
            g.raw_set(f.name, func)?;
        }
    }
    lua.set_named_registry_value(METHODS, methods)?;
    // The instruction budget of each run (see Vm::mod_script_resume).
    lua.set_global_hook(HookTriggers::new().every_nth_instruction(HOOK_EVERY), |lua, _| {
        let Some(mut b) = lua.app_data_mut::<Budget>() else { return Ok(VmState::Continue) };
        b.left -= HOOK_EVERY as i64;
        if b.left > 0 {
            return Ok(VmState::Continue);
        }
        b.exhausted = true;
        b.over += 1;
        // First, pause the run (it is then dropped); where it cannot be paused (inside a
        // function called by the C side), an error, that pcall gives back (prelude).
        if b.over == 1 {
            Ok(VmState::Yield)
        } else {
            Err(mlua::Error::runtime("stopped: too many instructions in one frame"))
        }
    })?;
    Ok(lua)
}

/// The global table of one mod: its own, reading the API and the libraries through `__index`.
/// Each mod has its own copy of the libraries: what it changes in them stays its own.
fn mod_env(lua: &Lua) -> mlua::Result<Table> {
    let env = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.raw_set("__index", lua.globals())?;
    meta.raw_set("__metatable", false)?;
    env.set_metatable(Some(meta))?;
    env.raw_set("_G", env.clone())?;
    for name in LIBRARIES {
        let copy = lua.create_table()?;
        for pair in lua.globals().raw_get::<Table>(*name)?.pairs::<mlua::Value, mlua::Value>() {
            let (k, v) = pair?;
            copy.raw_set(k, v)?;
        }
        env.raw_set(*name, copy)?;
    }
    Ok(env)
}

// ---------------------------------------------------------------------------------------------
// The engine side.

/// What `spawn` creates.
#[derive(Debug, Clone)]
pub(crate) enum SpawnWhat {
    Character(String),
    Class(String),
}

impl std::fmt::Display for SpawnWhat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnWhat::Character(c) => write!(f, "character {c}"),
            SpawnWhat::Class(c) => write!(f, "{c}"),
        }
    }
}

impl Vm {
    /// The mods whose scripts this Vm runs (tests). Takes effect at the next map start.
    pub fn set_mod_scripts_here(&mut self, mods: Vec<ScriptMod>) {
        self.mods.script_sources = Some(Arc::new(mods)).filter(|m| !m.is_empty());
    }

    /// Scripts are running on this map.
    pub fn mod_scripts_running(&self) -> bool {
        self.mods.scripts.is_some()
    }

    /// One line in the log (every call, unlike mod_report which writes a line once per run).
    pub(crate) fn mod_script_log(&mut self, line: String) {
        println!("mods: {line}");
        openxiii_mods::log::note(&format!("mods: {line}"));
        self.mod_messages.push(line);
    }

    /// End of the level start-up (after the mods' map changes): the scripts of this map,
    /// started again from their files.
    pub(crate) fn mod_scripts_start(&mut self) {
        if let Some(old) = self.mods.scripts.take() {
            self.mod_script_keep_saves(&old);
            let markers: Vec<ObjId> = old.lua.app_data_ref::<State>().map(|s| s.markers.values().copied().collect()).unwrap_or_default();
            for l in old.listeners.into_iter().chain(markers) {
                let _ = self.destroy_actor(l);
            }
        }
        self.mods.script_teams.clear();
        self.mod_charred.clear();
        self.mods.heat.clear();
        self.mods.surfaces.set_heat_used(false);
        self.mods.surfaces.clear();
        self.mods.surfaces.clear_charred();
        self.mods.surfaces.clear_char_areas();
        // What was shared with network guests belonged to the previous map.
        self.mods.net_fx = Default::default();
        self.mods.script_attacks.clear();
        self.mods.cine_pending.clear();
        self.mods.locked_doors.clear();
        // The screen items, the hits noted, the sounds and preload() of the previous map (not the
        // loading screen, shown while this map was prepared).
        self.mods.hud.items.clear();
        self.mods.script_events = Default::default();
        if self.mods.script_camera {
            self.mod_script_camera_back();
        }
        // The main menu: a new game or a saved one comes next, the scripts' memory starts empty
        // (a saved game brings its own, mod_script_save.rs).
        if self.mod_map_name().eq_ignore_ascii_case("MapMenu") {
            self.engine.mod_script_save.clear();
        }
        let restore = self.engine.mod_script_save_restore.take();
        if let Some(r) = restore {
            self.engine.mod_script_save = r;
        }
        // A network guest's scripts are the host's; the co-op character screen's backdrop map is
        // not played (a script there would act behind the menu).
        let screen = crate::improved::local_play::character_preview_url(self);
        if self.mod_net_client() || screen {
            return;
        }
        let files: Vec<openxiii_mods::scripts::ScriptFile> =
            self.mods.script_sources.clone().map(|mods| mods.iter().flat_map(|m| openxiii_mods::scripts::find(&m.folder, &m.name)).collect()).unwrap_or_default();
        // The mods' "when ... then ..." rules of this map (mod_when.rs): run by the same engine.
        let rules = self.mod_when_here();
        if files.is_empty() && rules.is_empty() {
            return;
        }
        let lua = match new_lua() {
            Ok(l) => l,
            Err(e) => {
                self.mod_report(format!("warning: scripts: Lua could not start: {e}"));
                return;
            }
        };
        let now = self.mod_level_time();
        if let Some(mut st) = lua.app_data_mut::<State>() {
            st.now = now;
            st.objects_seen = self.objects.len();
        }
        let mut envs: BTreeMap<String, Table> = BTreeMap::new();
        let mut order: Vec<(String, Table)> = Vec::new();
        let mut jobs = Vec::new();
        let map = self.mod_map_name();
        for f in &files {
            let text = match std::fs::read(&f.path) {
                Ok(t) => t,
                Err(e) => {
                    self.mod_report(format!("warning: script {}: cannot read the file: {e}", f.label));
                    continue;
                }
            };
            let env = match self.mod_script_env(&lua, &mut envs, &mut order, &f.mod_name) {
                Ok(e) => e,
                Err(e) => {
                    self.mod_report(format!("warning: script {}: {e}", f.label));
                    continue;
                }
            };
            // Text only: a precompiled chunk is refused.
            let chunk = lua.load(&text[..]).set_name(format!("@{}", f.label)).set_mode(ChunkMode::Text).set_environment(env);
            match chunk.into_function() {
                Ok(func) => {
                    crate::mod_preload::script_names(&text, &mut self.mods.script_events.script_references);
                    jobs.push(Job::Start { func, args: Vec::new(), label: f.label.clone(), origin: None, repeat: false });
                }
                Err(e) => self.mod_report(format!("warning: script {}", with_label(&error_text(&e), &f.label))),
            }
        }
        self.mod_report(format!("{map}: {} script(s) started", jobs.len()));
        let mut registered = 0;
        for rule in &rules {
            let env = match self.mod_script_env(&lua, &mut envs, &mut order, &rule.mod_name) {
                Ok(e) => e,
                Err(e) => {
                    self.mod_report(format!("warning: {map}: when rule ({}): {e}", rule.source));
                    continue;
                }
            };
            match crate::mod_when::register(&lua, &env, rule) {
                Ok(()) => registered += 1,
                Err(why) => self.mod_report(format!("warning: {map}: when rule ({}): {why}; rule not used", rule.source)),
            }
        }
        if !rules.is_empty() {
            self.mod_report(format!("{map}: {registered} when rule(s) started"));
        }
        let mut rt = ScriptRuntime { lua, listeners: Vec::new(), envs: order };
        self.mods.script_events.preload_open = true;
        self.mod_script_run(&mut rt, jobs, now);
        self.mod_script_keep_saves(&rt);
        self.mods.scripts = Some(Box::new(rt));
    }

    /// The global table of mod `name` (made the first time, with its memory `save`).
    fn mod_script_env(&mut self, lua: &Lua, envs: &mut BTreeMap<String, Table>, order: &mut Vec<(String, Table)>, name: &str) -> mlua::Result<Table> {
        if let Some(e) = envs.get(name) {
            return Ok(e.clone());
        }
        let e = mod_env(lua)?;
        // The mod's memory, as the game kept it.
        let kept = self.engine.mod_script_save.get(name).cloned();
        let save = match kept.as_deref().map(|t| crate::mod_script_save::from_json(lua, t)) {
            Some(Ok(t)) => t,
            Some(Err(why)) => {
                self.mod_report(format!("warning: script {name}: its save could not be read ({why}); it starts empty"));
                lua.create_table()?
            }
            None => lua.create_table()?,
        };
        e.raw_set("save", save)?;
        envs.insert(name.to_string(), e.clone());
        order.push((name.to_string(), e.clone()));
        Ok(e)
    }

    /// `modreload`: the scripts of this map, started again from their files.
    pub(crate) fn mod_scripts_reload(&mut self) -> String {
        if self.mod_net_client() {
            return "modreload: scripts run on the server, not on a network client".into();
        }
        if self.mods.script_sources.is_none() && self.mods.when_rules.is_none() {
            return "modreload: no mod has scripts".into();
        }
        let before = self.mod_messages.len();
        self.mod_scripts_start();
        match self.mod_messages[before..].iter().rev().find(|m| m.ends_with("script(s) started")) {
            Some(n) => format!("modreload: {n}"),
            None => "modreload: no script on this map".into(),
        }
    }

    /// Each frame (Level time `now`): triggers, timers and paused runs.
    pub(crate) fn mod_scripts_step(&mut self, now: f32) {
        let Some(mut rt) = self.mods.scripts.take() else { return };
        let jobs = self.mod_script_due(&mut rt, now);
        if !jobs.is_empty() {
            self.mod_script_run(&mut rt, jobs, now);
            self.mod_script_keep_saves(&rt);
        }
        // preload(): in the scripts' main code and their start handlers only (run by now).
        if self.mods.script_events.preload_open && rt.lua.app_data_ref::<State>().is_some_and(|st| !st.handlers.iter().any(|h| matches!(h.trigger, Trigger::Start { fired: false }))) {
            self.mods.script_events.preload_open = false;
        }
        if self.mods.scripts.is_none() {
            self.mods.scripts = Some(rt);
        }
        // Network guests learn the keys of the scripts (mod_script_keys.rs).
        self.mod_keys_tell_guests();
    }

    /// The handlers of a change of map (`on("map_end")`, `on("loading")`...) run now, given
    /// `args` (strings; an empty one is nil). `loading`: the items they make are the loading
    /// screen's (mod_script_hud.rs).
    pub(crate) fn mod_scripts_fire_event(&mut self, event: &str, args: Vec<String>, loading: bool) {
        let Some(mut rt) = self.mods.scripts.take() else { return };
        let jobs: Vec<Job> = match rt.lua.app_data_ref::<State>() {
            Some(st) => st
                .handlers
                .iter()
                .filter(|h| matches!(h.trigger, Trigger::Loading { event: e } if e == event))
                .filter(|_| true)
                .map(|h| Job::Start {
                    func: h.func.clone(),
                    args: args.iter().map(|a| if a.is_empty() { Arg::Nil } else { Arg::Str(a.clone()) }).collect(),
                    label: format!("{} (on {event})", h.label),
                    origin: Some(h.id),
                    repeat: false,
                })
                .collect(),
            None => Vec::new(),
        };
        if !jobs.is_empty() {
            let now = self.mod_level_time();
            self.mods.hud.loading.in_handler = loading;
            self.mod_script_run(&mut rt, jobs, now);
            self.mods.hud.loading.in_handler = false;
            self.mod_script_keep_saves(&rt);
        }
        if self.mods.scripts.is_none() {
            self.mods.scripts = Some(rt);
        }
    }

    /// A key a script may be bound to was pressed (`mod_key`). True if a handler has it.
    pub(crate) fn mod_script_key(&mut self, pc: ObjId, key: &str) -> bool {
        let Some(rt) = self.mods.scripts.as_ref() else { return false };
        let Some(mut st) = rt.lua.app_data_mut::<State>() else { return false };
        let bound = st.handlers.iter().any(|h| matches!(&h.trigger, Trigger::Key { key: k } if k.eq_ignore_ascii_case(key)));
        if bound {
            let pawn = self.native_prop(pc, Field::Pawn).as_object();
            st.keys.push((key.to_string(), pawn));
        }
        bound
    }

    /// Keys the scripts of this map are bound to (`on("key", ...)`).
    pub(crate) fn mod_script_keys(&self) -> Vec<String> {
        let Some(rt) = self.mods.scripts.as_ref() else { return Vec::new() };
        let Some(st) = rt.lua.app_data_ref::<State>() else { return Vec::new() };
        st.handlers.iter().filter_map(|h| if let Trigger::Key { key } = &h.trigger { Some(key.clone()) } else { None }).collect()
    }

    /// The runs due now: handlers whose event happened, timers, paused runs.
    fn mod_script_due(&mut self, rt: &mut ScriptRuntime, now: f32) -> Vec<Job> {
        let lua = rt.lua.clone();
        let Some(mut st) = lua.app_data_mut::<State>() else { return Vec::new() };
        st.now = now;
        let mut jobs = Vec::new();
        let players = self.mod_script_players();
        let first = self.mod_player_pawn().or_else(|| players.first().copied());
        let cinematic = self.level.is_some_and(|l| self.prop(l, "bCineFrame").as_bool());
        let keys = std::mem::take(&mut st.keys);
        let removed = st.removed.clone();
        // Follows and patrols give orders that may place a marker (the scripts' state is
        // borrowed again there): taken out of the state meanwhile.
        let mut follows = std::mem::take(&mut st.follows);
        let mut patrols = std::mem::take(&mut st.patrols);
        let notes = std::mem::take(&mut st.notes);
        let (due_panels, later): (Vec<_>, Vec<_>) = std::mem::take(&mut st.panels).into_iter().partition(|(t, _, _)| *t <= now);
        st.panels = later;
        drop(st);
        for (_, panel, label) in due_panels {
            self.mod_panel_show(&panel, &label);
        }
        for line in notes {
            let map = self.mod_map_name();
            self.mod_report(format!("warning: {map}: {line}"));
        }
        if !follows.is_empty() {
            self.mod_script_follow_step(Some(&lua), &mut follows, now);
        }
        // Patrols: the next point once the last one is reached.
        if !patrols.is_empty() {
            self.mod_script_patrol_step(&lua, &mut patrols);
        }
        st = lua.app_data_mut::<State>().expect("scripts state");
        // An order given meanwhile (a follow or patrol set again) is kept.
        for (k, v) in follows {
            st.follows.entry(k).or_insert(v);
        }
        for (k, v) in patrols {
            st.patrols.entry(k).or_insert(v);
        }
        // A scene's end.
        if st.camera_until.is_some_and(|t| now >= t) {
            st.camera_until = None;
            st.camera_move = None;
            drop(st);
            self.mod_script_camera_back();
            st = lua.app_data_mut::<State>().expect("scripts state");
        }
        // slow_motion(factor, seconds): normal speed again.
        if st.slow_until.is_some_and(|t| now >= t) {
            st.slow_until = None;
            if let Some(l) = self.level {
                self.set_prop(l, "TimeDilation", Value::Float(1.0));
            }
        }
        // A fade goes on.
        if let Some(f) = st.fade.clone() {
            drop(st);
            let going = self.mod_fade_step(&f, now);
            st = lua.app_data_mut::<State>().expect("scripts state");
            if !going {
                st.fade = None;
            }
        }
        // A camera that moves (camera{path =, follow =, orbit =}); none once the scene is over.
        if !self.mods.script_camera {
            st.camera_move = None;
        }
        if let Some(m) = st.camera_move.clone() {
            drop(st);
            let warned = self.mod_camera_step(&lua, &m, now);
            st = lua.app_data_mut::<State>().expect("scripts state");
            if let Some(m) = st.camera_move.as_mut() {
                m.warned = warned;
            }
        }
        // Behaviours: a character dead or gone stops its own.
        let gone: Vec<(ObjId, u64)> = st.behaving.iter().filter(|(a, _)| !self.mod_alive(**a)).map(|(a, id)| (*a, *id)).collect();
        for (a, id) in gone {
            st.behaving.remove(&a);
            st.tasks.retain(|t| t.origin != Some(id));
        }
        for (a, f) in std::mem::take(&mut st.behaviours_new) {
            if let Some(old) = st.behaving.remove(&a) {
                st.tasks.retain(|t| t.origin != Some(old));
            }
            let id = st.new_id();
            st.behaving.insert(a, id);
            jobs.push(Job::Start {
                func: f,
                args: vec![Arg::Actor(a)],
                label: format!("behaviour of {}", self.mod_actor_name(a)),
                origin: Some(id),
                repeat: false,
            });
        }
        if !self.mods.script_attacks.is_empty() {
            self.mod_script_attack_step(now);
        }
        if !self.mods.cine_pending.is_empty() {
            self.mod_cine_pending_step(now);
        }
        for h in st.handlers.iter_mut() {
            let mut fire: Vec<Vec<Arg>> = Vec::new();
            match &mut h.trigger {
                Trigger::Start { fired } => {
                    if !*fired && first.is_some() {
                        *fired = true;
                        fire.push(vec![first.map_or(Arg::Nil, Arg::Actor)]);
                    }
                }
                Trigger::Near { who, distance, inside } => {
                    let target = self.mod_script_resolve(who);
                    for &p in &players {
                        let near = match target {
                            Some(t) if t != p => {
                                let [px, py, pz] = self.native_value(p, Field::Location).as_vector();
                                let [x, y, z] = self.native_value(t, Field::Location).as_vector();
                                ((px - x).powi(2) + (py - y).powi(2) + (pz - z).powi(2)).sqrt() <= *distance
                            }
                            _ => false,
                        };
                        // During a cinematic the player does not walk: coming near waits for
                        // its end, going away counts.
                        if near && !cinematic && inside.insert(p) {
                            fire.push(vec![Arg::Actor(p), target.map_or(Arg::Nil, Arg::Actor)]);
                        } else if !near {
                            inside.remove(&p);
                        }
                    }
                }
                Trigger::Death { who, known, fired, first_new, primed } => {
                    // Every character `who` names, alive or dead (one that died in its first
                    // frame too), and those seen before even if gone since.
                    known.extend(self.mod_script_death_candidates(who));
                    if !*primed {
                        *primed = true;
                        let old: Vec<ObjId> = known.iter().copied().filter(|&a| a < *first_new && !self.mod_alive(a)).collect();
                        fired.extend(old);
                    }
                    let mut dead: Vec<ObjId> = known.iter().copied().filter(|&a| !self.mod_alive(a) && !removed.contains(&a) && !fired.contains(&a)).collect();
                    dead.sort();
                    for a in dead {
                        fired.insert(a);
                        fire.push(vec![Arg::Actor(a)]);
                    }
                }
                Trigger::Damage { who, last } => {
                    if let Some(t) = self.mod_script_resolve(who) {
                        let h = self.prop(t, "Health").as_int();
                        if let Some(&before) = last.get(&t) {
                            if h < before {
                                // Who hit it and with what, when the game said (its TakeDamage).
                                let hit = self.mods.script_events.hits.get(&t).cloned().unwrap_or_default();
                                let s = |v: Option<String>| v.map_or(Arg::Nil, Arg::Str);
                                fire.push(vec![Arg::Actor(t), Arg::Int((before - h) as i64), hit.by.map_or(Arg::Nil, Arg::Actor), s(hit.damage_type), s(hit.weapon)]);
                            }
                        }
                        last.insert(t, h);
                    }
                }
                Trigger::Fire { who, seen } => {
                    for p in self.mod_fire_candidates(who.as_ref()) {
                        let (n, class) = self.mod_fire_shots(p, seen);
                        for _ in 0..n {
                            fire.push(vec![Arg::Actor(p), class.clone().map_or(Arg::Nil, Arg::Str)]);
                        }
                    }
                }
                Trigger::Loading { .. } => {}
                Trigger::Pickup { seen } => {
                    for &p in &players {
                        let items = self.mod_script_inventory(p);
                        match seen.get(&p) {
                            Some(before) => {
                                for &i in items.iter().filter(|i| !before.contains(i)) {
                                    let class = self.objects.get(i).and_then(|o| o.as_ref()).map(|o| self.class_path(o.class)).unwrap_or_default();
                                    fire.push(vec![Arg::Actor(p), Arg::Str(class)]);
                                }
                            }
                            // The first time: what the player already carries is not picked up.
                            None => {}
                        }
                        seen.insert(p, items);
                    }
                }
                Trigger::Enter { at, radius, inside } => {
                    for &p in &players {
                        let [px, py, pz] = self.native_value(p, Field::Location).as_vector();
                        let feet = pz - self.native_prop(p, Field::CollisionHeight).as_float();
                        let d = ((px - at[0]).powi(2) + (py - at[1]).powi(2) + (feet - at[2]).powi(2)).sqrt();
                        let within = d <= *radius;
                        if within && !cinematic && inside.insert(p) {
                            fire.push(vec![Arg::Actor(p)]);
                        } else if !within {
                            inside.remove(&p);
                        }
                    }
                }
                Trigger::Event { name, listener, dispatcher, tried } => {
                    if !*tried {
                        *tried = true;
                        *dispatcher = self.mod_event_dispatcher(name);
                        if let Some(d) = *dispatcher {
                            rt.listeners.push(d);
                        }
                        *listener = self.mod_event_listener(name);
                        match *listener {
                            Some(l) => rt.listeners.push(l),
                            None => {
                                let line = format!("warning: {}: script {}: event {name} cannot be heard", self.mod_map_name(), h.label);
                                self.mod_report(line);
                            }
                        }
                    }
                    if let Some(l) = *listener {
                        if self.objects.get(l).is_some_and(|o| o.is_some()) && self.prop(l, "bEnabled").as_bool() {
                            self.set_prop(l, "bEnabled", Value::Bool(false));
                            // The player who caused it, when the game says (the Dispatcher's
                            // Trigger keeps its EventInstigator); else the main player.
                            let by = dispatcher.and_then(|d| {
                                let i = self.prop(d, "Instigator").as_object();
                                self.set_prop(d, "Instigator", Value::Object(None));
                                i
                            });
                            let by = by.filter(|i| players.contains(i)).or(first);
                            fire.push(vec![by.map_or(Arg::Nil, Arg::Actor)]);
                        }
                    }
                }
                Trigger::Sees { who, seen, next } => {
                    // The character's own sight (Controller.CanSee), four times a second.
                    if now >= *next {
                        *next = now + 0.25;
                        if let Some(t) = self.mod_script_resolve(who).filter(|&t| self.mod_alive(t)) {
                            if let Some(ai) = self.native_prop(t, Field::Controller).as_object() {
                                for &p in &players {
                                    let sees = self.call_named(ai, "CanSee", vec![Value::Object(Some(p))]).is_ok_and(|v| v.as_bool());
                                    if sees && seen.insert((t, p)) {
                                        fire.push(vec![Arg::Actor(t), Arg::Actor(p)]);
                                    } else if !sees {
                                        seen.remove(&(t, p));
                                    }
                                }
                            }
                        }
                    }
                }
                Trigger::Key { key } => {
                    for (k, pawn) in &keys {
                        if k.eq_ignore_ascii_case(key) {
                            fire.push(vec![pawn.map_or(Arg::Nil, Arg::Actor)]);
                        }
                    }
                }
            }
            for args in fire {
                jobs.push(Job::Start {
                    func: h.func.clone(),
                    args,
                    label: format!("{} (on {})", h.label, h.trigger.describe()),
                    origin: Some(h.id),
                    repeat: false,
                });
            }
        }
        for t in st.timers.iter_mut() {
            if t.at <= now {
                jobs.push(Job::Start { func: t.func.clone(), args: Vec::new(), label: t.label.clone(), origin: Some(t.id), repeat: t.period.is_some() });
                // The next time, never in the past (no burst after a pause).
                match t.period {
                    Some(p) => {
                        t.at += p;
                        if t.at <= now {
                            t.at = now + p;
                        }
                    }
                    None => t.at = f32::INFINITY,
                }
            }
        }
        st.timers.retain(|t| t.at.is_finite());
        st.objects_seen = self.objects.len();
        // The hits are noted only while a damage handler exists; those of this frame are used.
        self.mods.script_events.watch_hits = st.handlers.iter().any(|h| matches!(h.trigger, Trigger::Damage { .. }));
        self.mods.script_events.hits.clear();
        let (due, wait): (Vec<Task>, Vec<Task>) = std::mem::take(&mut st.tasks).into_iter().partition(|t| t.wake <= now);
        st.tasks = wait;
        jobs.extend(due.into_iter().map(|t| Job::Resume { thread: t.thread, label: t.label, origin: t.origin }));
        jobs
    }

    /// Runs `jobs`, the gate open.
    fn mod_script_run(&mut self, rt: &mut ScriptRuntime, jobs: Vec<Job>, now: f32) {
        let lua = rt.lua.clone();
        let vm = RefCell::new(&mut *self);
        let mut lines: Vec<String> = Vec::new();
        let r = lua.scope(|scope| {
            let gate = scope.create_function_mut(|lua, (op, args): (usize, MultiValue)| {
                let ApiKind::Game(run) = API.get(op).map(|f| f.kind).ok_or_else(|| mlua::Error::runtime("unknown function"))? else {
                    return Err(mlua::Error::runtime("not a game function"));
                };
                let mut vm = vm.try_borrow_mut().map_err(|_| mlua::Error::runtime("the game is busy"))?;
                run(&mut **vm, lua, args)
            })?;
            lua.set_named_registry_value(GATE, gate)?;
            for job in jobs {
                let (thread, args, label, origin, repeat) = match job {
                    Job::Start { func, args, label, origin, repeat } => {
                        let thread = match lua.create_thread(func) {
                            Ok(t) => t,
                            Err(e) => {
                                lines.push(format!("warning: script {label}: {}", error_text(&e)));
                                continue;
                            }
                        };
                        let mut values = Vec::new();
                        for a in args {
                            values.push(match a {
                                Arg::Actor(a) => mlua::Value::UserData(lua.create_userdata(Actor(a))?),
                                Arg::Int(i) => mlua::Value::Integer(i),
                                Arg::Str(t) => mlua::Value::String(lua.create_string(&t)?),
                                Arg::Nil => mlua::Value::Nil,
                            });
                        }
                        (thread, MultiValue::from_vec(values), label, origin, repeat)
                    }
                    Job::Resume { thread, label, origin } => (thread, MultiValue::new(), label, origin, false),
                };
                // Each run has its budget.
                if let Some(mut b) = lua.app_data_mut::<Budget>() {
                    *b = Budget { left: BUDGET, exhausted: false, over: 0 };
                }
                let result = thread.resume::<MultiValue>(args);
                let exhausted = lua.app_data_ref::<Budget>().is_some_and(|b| b.exhausted);
                let mut st = lua.app_data_mut::<State>().expect("scripts state");
                let mut drop_origin = false;
                match result {
                    _ if exhausted => {
                        lines.push(format!(
                            "warning: script {label}: stopped: more than {BUDGET} instructions in one frame (a loop that never ends?); this handler is removed"
                        ));
                        drop_origin = true;
                    }
                    Err(e) => lines.push(format!("warning: script {}", with_label(&error_text(&e), &label))),
                    Ok(values) if thread.status() == ThreadStatus::Resumable => {
                        let seconds = match values.into_iter().next() {
                            Some(mlua::Value::Number(n)) => n as f32,
                            Some(mlua::Value::Integer(n)) => n as f32,
                            _ => 0.0,
                        };
                        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
                        // At least the next frame.
                        let wake = if seconds > 0.0 { now + seconds } else { now + f32::EPSILON };
                        st.tasks.push(Task { thread, wake, label, origin });
                    }
                    Ok(values) => {
                        // `every`: a function returning false stops.
                        if repeat && matches!(values.into_iter().next(), Some(mlua::Value::Boolean(false))) {
                            drop_origin = true;
                        }
                    }
                }
                if drop_origin {
                    if let Some(id) = origin {
                        st.handlers.retain(|h| h.id != id);
                        st.timers.retain(|t| t.id != id);
                        st.tasks.retain(|t| t.origin != Some(id));
                    }
                }
            }
            Ok(())
        });
        let _ = lua.set_named_registry_value(GATE, mlua::Value::Nil);
        if let Err(e) = r {
            lines.push(format!("warning: scripts: {}", error_text(&e)));
        }
        for l in lines {
            self.mod_report(l);
        }
    }

    // --- Helpers of the API.

    /// The lookup of `who` by the scripts: an actor given as such; else a mods [character] of
    /// that name alive on the map; else the mods' shared lookup of placed actors
    /// (`find_placed`: object name, then characters by Tag or PawnName, then other actors by
    /// Tag), the first one alive in the map's order.
    pub(crate) fn mod_script_resolve(&self, who: &Who) -> Option<ObjId> {
        let name = match who {
            Who::Actor(a) => return Some(*a).filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()) && !self.native_prop(a, Field::DeleteMe).as_bool()),
            Who::Name(n) => n,
        };
        if let Some(b) = self.mods.behaviour.as_ref() {
            let mut live: Vec<ObjId> = self
                .mods.live
                .values()
                .filter(|l| b.characters.get(l.def).is_some_and(|d| d.name.eq_ignore_ascii_case(name)))
                .map(|l| l.pawn)
                .filter(|&p| self.mod_alive(p))
                .collect();
            live.sort();
            if let Some(&p) = live.first() {
                return Some(p);
            }
        }
        self.find_placed(name, None, 0.0).into_iter().find(|&a| self.mod_alive(a))
    }

    /// `find_all`: every mods [character] named `name` still in the game, alive or dead, in the
    /// order they were made; else every actor alive that `find_placed` gives for that name.
    pub(crate) fn mod_script_resolve_all(&self, name: &str) -> Vec<ObjId> {
        if let Some(b) = self.mods.behaviour.as_ref() {
            let mut all: Vec<ObjId> = self
                .mods.live
                .values()
                .filter(|l| b.characters.get(l.def).is_some_and(|d| d.name.eq_ignore_ascii_case(name)))
                .map(|l| l.pawn)
                .filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()) && !self.native_prop(p, Field::DeleteMe).as_bool())
                .collect();
            if !all.is_empty() {
                all.sort();
                return all;
            }
        }
        self.find_placed(name, None, 0.0).into_iter().filter(|&a| self.mod_alive(a)).collect()
    }

    /// The items a pawn carries (its Inventory chain).
    fn mod_script_inventory(&self, pawn: ObjId) -> BTreeSet<ObjId> {
        let mut out = BTreeSet::new();
        let mut item = self.prop(pawn, "Inventory").as_object();
        while let Some(i) = item.filter(|i| out.len() < 256 && self.objects.get(*i).is_some_and(|o| o.is_some())) {
            if !out.insert(i) {
                break;
            }
            item = self.prop(i, "Inventory").as_object();
        }
        out
    }

    /// A Dispatcher of the game tagged `event`: its Trigger keeps the event's instigator.
    fn mod_event_dispatcher(&mut self, event: &str) -> Option<ObjId> {
        let class = self.class("Gameplay.Dispatcher").or_else(|| self.class("Engine.Dispatcher"))?;
        let at = self
            .mod_player_pawn()
            .or_else(|| self.actors.first().copied())
            .map(|a| self.native_value(a, Field::Location))
            .unwrap_or(Value::vector(0.0, 0.0, 0.0));
        let tag = Value::Name(crate::value::NameStruct::from_time(event));
        self.spawn_actor(class, Value::Object(None), Some(tag), at, Value::rotator(0, 0, 0), Value::Object(None), true).ok().flatten()
    }

    /// The characters (and actors) `who` names for a death handler, dead ones included: the
    /// actor given, or the mods [character]s of that name and the actors whose name, Tag or
    /// PawnName it is.
    fn mod_script_death_candidates(&self, who: &Who) -> Vec<ObjId> {
        let name = match who {
            Who::Actor(a) => return vec![*a],
            Who::Name(n) => n,
        };
        let mut out: Vec<ObjId> = Vec::new();
        if let Some(b) = self.mods.behaviour.as_ref() {
            out.extend(self.mods.live.values().filter(|l| b.characters.get(l.def).is_some_and(|d| d.name.eq_ignore_ascii_case(name))).map(|l| l.pawn));
        }
        for &a in &self.actors {
            if self.objects.get(a).is_none_or(|o| o.is_none()) {
                continue;
            }
            let hit = self.mod_actor_name(a).eq_ignore_ascii_case(name)
                || self.prop(a, "Tag").as_name().eq_ignore_ascii_case(name)
                || (self.is_a(a, "Pawn") && self.prop(a, "PawnName").as_name().eq_ignore_ascii_case(name));
            if hit {
                out.push(a);
            }
        }
        out
    }

    /// Player controllers of the game (local and remote ones on a host), in creation order.
    pub(crate) fn mod_script_player_controllers(&self) -> Vec<ObjId> {
        self.actors
            .iter()
            .copied()
            .filter(|&a| {
                self.is_a(a, "PlayerController") && self.objects.get(a).is_some_and(|o| o.is_some()) && !self.native_prop(a, Field::DeleteMe).as_bool()
            })
            .collect()
    }

    /// The players' pawns, alive.
    pub(crate) fn mod_script_players(&self) -> Vec<ObjId> {
        let mut v: Vec<ObjId> = self
            .mod_script_player_controllers()
            .into_iter()
            .filter_map(|pc| self.native_prop(pc, Field::Pawn).as_object())
            .filter(|&p| self.mod_alive(p))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// The team of `a` towards the players: a script's, else its mods [character]'s.
    pub(crate) fn mod_script_team(&self, a: ObjId) -> Option<Team> {
        if let Some(t) = self.mods.script_teams.get(&a) {
            return Some(*t);
        }
        let live = self.mods.live.get(&a)?;
        self.mods.behaviour.as_ref()?.characters.get(live.def)?.team
    }

    /// `spawn`: a mods [character] or an actor of a class standing on `position`.
    pub(crate) fn mod_script_spawn(&mut self, what: &SpawnWhat, position: [f32; 3], yaw: f32, team: Option<Team>) -> Result<ObjId, String> {
        let (class_name, def) = match what {
            SpawnWhat::Class(c) => (c.clone(), None),
            SpawnWhat::Character(name) => {
                let found = self
                    .mods.behaviour
                    .as_ref()
                    .and_then(|b| b.characters.iter().position(|d| d.name.eq_ignore_ascii_case(name)).map(|i| (i, b.characters[i].clone())));
                let Some((i, d)) = found else { return Err(format!("no [character.{name}] in the mods")) };
                (d.base.clone(), Some((i, d)))
            }
        };
        let class = self.class(&class_name).ok_or_else(|| format!("unknown class {class_name}"))?;
        self.create_cdo(class);
        // The character's own height when its [character] gives one (a dog is lower than a man):
        // its feet, not its class's, are put on the ground.
        let own_height = def.as_ref().and_then(|(_, d)| {
            d.properties.iter().find(|v| v.index.is_none() && v.property.eq_ignore_ascii_case("CollisionHeight")).and_then(|v| match v.value {
                openxiii_mods::ModValue::Int(i) => Some(i as f32),
                openxiii_mods::ModValue::Float(f) => Some(f as f32),
                _ => None,
            })
        });
        let height = own_height.filter(|h| *h > 0.0).unwrap_or_else(|| self.native_prop(self.classes[class].cdo, Field::CollisionHeight).as_float());
        let rotation = Value::rotator(0, ((yaw * 65536.0 / 360.0).round() as i32).rem_euclid(65536), 0);
        // The ground under the point: a point a step above it is put on it; a point above a drop
        // is refused (the actor would fall, and a character be hurt by the fall).
        let mut position = position;
        let mut hit = crate::physics::CheckResult::new(1.0);
        let [x, y, z] = position;
        self.single_line_check(&mut hit, None, [x, y, z - GROUND_SEARCH], [x, y, z + 50.0], crate::physics::TRACE_WORLD, [0.0; 3]);
        // Only with the level's whole collision (the game; not the tests without it, where a
        // floor of static meshes is missing).
        if hit.time < 1.0 && self.collision.is_some() {
            let ground = z + 50.0 - hit.time * (GROUND_SEARCH + 50.0);
            if z - ground > MAX_STEP {
                return Err(format!(
                    "no ground at {{{x:.0}, {y:.0}, {z:.0}}}: the ground is {:.0} units lower (it would fall); give a point on the ground (modpos)",
                    z - ground
                ));
            }
            position[2] = ground;
        }
        // Level.bStartup on during the spawn: a character gets its controller as a placed one.
        let level = self.level;
        let startup = level.map(|l| self.prop(l, "bStartup"));
        if let Some(l) = level {
            self.set_prop(l, "bStartup", Value::Bool(true));
        }
        self.mods.spawn_values = def.as_ref().map(|(_, d)| d.properties.clone());
        let r = self.spawn_actor(
            class,
            Value::Object(None),
            None,
            Value::vector(position[0], position[1], position[2] + height),
            rotation,
            Value::Object(None),
            false,
        );
        self.mods.spawn_values = None;
        if let (Some(l), Some(v)) = (level, startup) {
            self.set_prop(l, "bStartup", v);
        }
        let a = match r {
            Ok(Some(a)) => a,
            Ok(None) => return Err("the game could not place it (no room there, or the class cannot be spawned)".into()),
            Err(e) => return Err(e.to_string()),
        };
        let outside = matches!(self.native_value(a, Field::Region), Value::Struct(_, ref f) if matches!(f.get(1), Some(Value::Int(-1))));
        if outside {
            let _ = self.destroy_actor(a);
            return Err("the position is outside the map".into());
        }
        if let Some((i, d)) = def {
            self.mod_take_character(a, i, &d);
            // Its voice and sounds, not loaded with the level for a character only scripts make.
            self.mod_script_character_sounds(i);
        }
        if let Some(t) = team {
            self.mods.script_teams.insert(a, t);
        }
        self.mod_check_look(a, &format!("{}: spawn {what}", self.mod_map_name()), "script");
        Ok(a)
    }

    /// `remove`: takes `a` out (with the AI controller of a character). Not a player, nor a
    /// part of the level.
    pub(crate) fn mod_script_remove(&mut self, a: ObjId) -> Result<(), String> {
        let controller =
            self.native_prop(a, Field::Controller).as_object().filter(|&c| self.is_a(a, "Pawn") && self.objects.get(c).is_some_and(|o| o.is_some()));
        if controller.is_some_and(|c| self.is_a(c, "PlayerController")) || self.is_a(a, "PlayerController") {
            return Err("a player cannot be removed".into());
        }
        if self.propr(a, "bStatic").as_bool() || self.propr(a, "bNoDelete").as_bool() {
            return Err(format!("{} is part of the level (bStatic or bNoDelete)", self.mod_actor_name(a)));
        }
        let name = self.mod_actor_name(a);
        // A mods [character]'s state sound stops with it. It stops being a mods character before
        // it is destroyed: Destroyed sends its controller to another state, whose look (its
        // animation and its sound) must not start again on the pawn being removed.
        self.mod_stop_look_sound(a);
        let live = self.mods.live.remove(&a);
        let refused = match self.destroy_actor(a) {
            Ok(true) => None,
            Ok(false) => Some(format!("the game refused to remove {name}")),
            Err(e) => Some(e.to_string()),
        };
        if let Some(why) = refused {
            if let Some(l) = live {
                self.mods.live.insert(a, l);
            }
            return Err(why);
        }
        if let Some(c) = controller {
            if !self.native_prop(c, Field::DeleteMe).as_bool() {
                let _ = self.destroy_actor(c);
            }
        }
        self.mods.script_teams.remove(&a);
        let map = self.mod_map_name();
        self.mod_report(format!("{map}: remove {name}"));
        Ok(())
    }

    /// `say`: the line goes to the queue of the mods' lines (mod_dialogue.rs), said when no
    /// other line is.
    pub(crate) fn mod_script_say(&mut self, who: &Who, line: Option<String>, text: Option<String>, sound: Option<String>, label: &str) -> bool {
        let map = self.mod_map_name();
        let Some(speaker) = self.mod_script_resolve(who).filter(|&a| self.mod_alive(a)) else {
            self.mod_report(format!("warning: {map}: say: nobody named {who} on this map ({label})"));
            return false;
        };
        let by = match who {
            Who::Name(n) => n.clone(),
            Who::Actor(a) => self.mod_actor_name(*a),
        };
        let say = openxiii_mods::dialogue::Say { by, line, text, sound, when: openxiii_mods::dialogue::When::Start, delay: 0.0, once: true };
        // A voice of another level: its bank is loaded now.
        let voice = if let Some(l) = say.line.as_ref() {
            openxiii_mods::dialogue::parse_line_name(l).map(|(level, _, _)| format!("{level}Voices.{l}"))
        } else {
            say.sound.clone()
        };
        if let Some(path) = voice.filter(|_| self.audio.is_some()) {
            if let Err(why) = self.mod_ensure_sound(&path, &format!("{map}: {}", say.describe())) {
                self.mod_report(format!("warning: {map}: {}: {why} ({label})", say.describe()));
            }
        }
        let now = self.mod_level_time();
        let d = &mut self.mods.dialogues;
        d.says.push(crate::mod_dialogue::LiveSay {
            say,
            source: format!("script {label}"),
            speaker: Some(speaker),
            listener: None,
            done: true,
            inside: false,
            warned: false,
        });
        d.queue.push_back((d.says.len() - 1, speaker, now));
        true
    }
}

fn with_label(text: &str, label: &str) -> String {
    if text.contains(".lua:") {
        text.to_string()
    } else {
        format!("{label}: {text}")
    }
}
