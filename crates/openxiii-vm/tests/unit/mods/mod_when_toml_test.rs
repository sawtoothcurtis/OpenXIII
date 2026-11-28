//! Mods build only: the toml's `[[map.<Map>.when]]` blocks (openxiii-mods when.rs) read from a
//! mod's file and run by the scripts' engine (mod_when.rs, mod_setup::when_rules).
//!
//! Without the game: the actions of the toml are the engine's, a block keeps its
//! from_checkpoint. With the game (OXIII_GAME; skipped when it is not set), on the real
//! Plage01: start -> give, after -> message, near -> say, key -> spawn, death of a named
//! character -> a map event, the event -> a log and `run` (a function of the mod's script), a
//! mod without a scripts folder, a wrong block skipped with one line; and the test mod
//! tests/fixtures/mods/when-then.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::mod_script::ScriptMod;
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// A Mods folder of its own, removed at the end.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Folder {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!("oxiii-when-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Folder(p)
    }

    /// Writes `<mod>/mod.toml`.
    fn toml(&self, m: &str, text: &str) -> PathBuf {
        let dir = self.0.join(m);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mod.toml");
        std::fs::write(&file, format!("[mod]\nname = \"{m}\"\nversion = \"1\"\n{text}")).unwrap();
        file
    }

    /// Writes `<mod>/scripts/<file>`.
    fn script(&self, m: &str, file: &str, text: &str) {
        let dir = self.0.join(m).join("scripts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn all(vm: &Vm) -> String {
    vm.mod_messages.join("\n")
}

fn count(vm: &Vm, what: &str) -> usize {
    vm.mod_messages.iter().filter(|m| m.contains(what)).count()
}

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// The mods' files read as at startup (mod_setup): their map changes, characters (look names
/// resolved) and when rules.
fn read(files: &[(&str, PathBuf)]) -> (Vec<openxiii_mods::MapEdit>, Vec<openxiii_mods::behaviour::ModAction>, Vec<openxiii_mods::behaviour::CharacterDef>, Vec<String>) {
    let (mut edits, mut actions, mut characters, mut problems) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (name, file) in files {
        let f = openxiii_mods::defaults::parse_mod_file(&std::fs::read_to_string(file).unwrap(), file, name);
        edits.extend(f.edits);
        actions.extend(f.actions);
        characters.extend(f.characters);
        problems.extend(f.problems.iter().map(|p| p.to_string()));
    }
    problems.extend(openxiii_mods::simple::resolve(&edits, &actions, &mut characters).iter().map(|p| p.to_string()));
    (edits, actions, characters, problems)
}

/// Plage01 with the mods of `folder` (their toml files and scripts), the player logged in.
fn plage01(game: &Path, folder: &Folder, names: &[&str]) -> (Vm, ObjId, ObjId, Vec<String>) {
    let files: Vec<(&str, PathBuf)> = names.iter().map(|n| (*n, folder.0.join(n).join("mod.toml"))).collect();
    let (edits, actions, characters, problems) = read(&files);
    let rules = crate::mod_setup::when_rules(&edits).into_iter().map(|(r, _)| r).collect();
    let scripted: Vec<ScriptMod> = names.iter().filter(|n| folder.0.join(n).join("scripts").is_dir()).map(|n| ScriptMod { name: n.to_string(), folder: folder.0.join(n) }).collect();
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(edits);
    vm.set_mod_behaviour_here(actions, characters);
    vm.set_mod_scripts_here(scripted);
    vm.set_mod_when_here(rules);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    let level = vm.level.unwrap();
    vm.set_prop(level, "bCineFrame", Value::Bool(false));
    (vm, pc, pawn, problems)
}

/// The end of Plage01's opening scene, as its last step does (it fires Objectif91): the
/// level's cut-scene frame goes off and XIII can move.
fn end_opening_scene(vm: &mut Vm, pawn: ObjId) {
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
}

fn put(vm: &mut Vm, pawn: ObjId, at: [f32; 3]) {
    let mut args = [Value::vector(at[0], at[1], at[2])];
    let _ = crate::natives::call(vm, "SetLocation", pawn, &mut args, &[true]);
}

/// Ticks until a line contains `what` (at most `seconds`), the player kept at `at` if given.
fn tick_until(vm: &mut Vm, pawn: ObjId, at: Option<[f32; 3]>, what: &str, seconds: f32) -> bool {
    for frame in 0..(seconds * 30.0) as u32 {
        if let (Some(p), 0) = (at, frame % 15) {
            put(vm, pawn, p);
        }
        if let Some(level) = vm.level {
            vm.set_prop(level, "bCineFrame", Value::Bool(false));
        }
        vm.tick(1.0 / 30.0);
        if count(vm, what) > 0 {
            return true;
        }
    }
    false
}

/// The actions a toml block can name are the engine's own (mod_when::ACTIONS), each a function
/// of the scripts' API.
#[test]
fn toml_actions_are_the_engines() {
    let toml: Vec<&str> = openxiii_mods::when::ACTIONS.iter().map(|a| a.name).collect();
    let mut engine: Vec<&str> = crate::mod_when::ACTIONS.to_vec();
    let mut sorted = toml.clone();
    sorted.sort_unstable();
    engine.sort_unstable();
    assert_eq!(sorted, engine, "the toml's actions (openxiii-mods when.rs) and the engine's (mod_when.rs)");
    for a in &toml {
        assert!(crate::mod_script::API.iter().any(|f| f.name == *a && !f.hidden), "{a} is a function of the scripts");
    }
    // The events: the scripts' own, plus the timers; not the change of map (a map's rule ends
    // with its map, and the next map's starts after it).
    let change_of_map = ["map_end", "loading", "loaded", "loading_key"];
    let events: Vec<&str> = openxiii_mods::when::EVENTS.iter().map(|e| e.name).collect();
    for (e, ..) in crate::mod_script::EVENTS.iter().filter(|e| !change_of_map.contains(&e.0)) {
        assert!(events.contains(e), "event {e}");
    }
    assert!(change_of_map.iter().all(|e| crate::mod_script::EVENTS.iter().any(|s| s.0 == *e) && !events.contains(e)));
    assert_eq!(events.len(), crate::mod_script::EVENTS.len() - change_of_map.len() + 2, "{events:?}");
}

/// A block keeps its map selector, its from_checkpoint and where it was written.
#[test]
fn blocks_become_rules() {
    let folder = Folder::new();
    let file = folder.toml("m", "[[map.\"Hual*\".when]]\nevent = \"after\"\nseconds = 2\nfrom_checkpoint = 3\nthen = [ { message = \"later\" } ]\n");
    let (edits, _, _, problems) = read(&[("m", file)]);
    assert!(problems.is_empty(), "{problems:?}");
    let rules = crate::mod_setup::when_rules(&edits);
    assert_eq!(rules.len(), 1);
    let (r, about) = &rules[0];
    assert_eq!((r.map.as_str(), r.mod_name.as_str(), r.source.as_str()), ("Hual*", "m", "m/mod.toml line 4"));
    assert_eq!(r.checkpoint, openxiii_mods::placement::Checkpoint::From(3));
    assert_eq!(r.event, crate::mod_when::WhenEvent::After { seconds: 2.0 });
    assert_eq!(about, "when after 2 s then message");
}

const RULES: &str = r#"
[[map.Plage01.when]]
event = "start"
then = [ { give = "Shotgun" }, { message = "rules ready" } ]

[[map.Plage01.when]]
event = "after"
seconds = 1
then = [ { message = "one second" } ]

[[map.Plage01.when]]
event = "near"
who = "Pam"
distance = 300
once = true
then = [ { say = "Plage01_Pam_03" } ]

[[map.Plage01.when]]
event = "key"
key = "K"
then = [ { spawn = "Carrington", position = [-470, -860, 1190], team = "neutral" } ]

[[map.Plage01.when]]
event = "death"
who = "Carrington"
then = [ { trigger = "WhenGuardDown" } ]

[[map.Plage01.when]]
event = "event"
name = "WhenGuardDown"
then = [ { log = "the map event came" } ]
run = "reward"

[[map.Plage01.when]]
event = "start"
then = [ { explode = true } ]
"#;

/// Each event of the toml on the real Plage01, with its negative control.
#[test]
fn when_blocks_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.toml("rules", RULES);
    folder.script("rules", "reward.lua", "function reward(p) log('reward for', p == player()) end");
    // A mod without a scripts folder: its blocks run all the same.
    folder.toml("plain", "[[map.Plage01.when]]\nevent = \"start\"\nthen = [ { message = \"plain mod here\" } ]\n");
    let (mut vm, pc, pawn, problems) = plage01(&game, &folder, &["plain", "rules"]);
    // The wrong block: one line with the closest words, the others kept.
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("no action in { explode }") && problems[0].contains("block skipped"), "{problems:?}");
    end_opening_scene(&mut vm, pawn);
    assert!(tick_until(&mut vm, pawn, None, "message \"rules ready\"", 3.0), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("7 when rule(s) started") || text.contains("when rule(s) started"), "{text}");
    assert!(text.contains("give XIII.FusilPompe"), "start -> give\n{text}");
    assert_eq!(count(&vm, "message \"plain mod here\""), 1, "{text}");
    // after: not yet, then once.
    assert_eq!(count(&vm, "message \"one second\""), 0, "{text}");
    assert!(tick_until(&mut vm, pawn, None, "message \"one second\"", 2.0), "{}", all(&vm));
    let start = vm.native_value(pawn, Field::Location).as_vector();
    let far = [start[0] - 3000.0, start[1], start[2]];
    // key: no guard before the key, one after.
    assert_eq!(count(&vm, "spawn character Carrington"), 0, "{}", all(&vm));
    assert!(vm.mod_script_key(pc, "K"));
    assert!(tick_until(&mut vm, pawn, Some(far), "spawn character Carrington", 2.0), "{}", all(&vm));
    let guard = vm.mod_script_resolve(&crate::mod_script::Who::Name("Carrington".into())).expect("the general");
    assert_eq!(vm.prop(guard, "Mesh"), Value::Asset("XIIIPersos.CarringtonM".into()), "the simple name Carrington is the general");
    // death of the named character -> the map's event -> the log and the script's function.
    assert_eq!(count(&vm, "trigger WhenGuardDown"), 0, "{}", all(&vm));
    // A second alive (the death rule finds him), then he dies.
    tick_until(&mut vm, pawn, Some(far), "nothing", 1.0);
    assert_eq!(count(&vm, "trigger WhenGuardDown"), 0, "{}", all(&vm));
    vm.set_prop(guard, "Health", Value::Int(0));
    assert!(tick_until(&mut vm, pawn, Some(far), ": reward for true", 3.0), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("trigger WhenGuardDown") && text.contains(": the map event came"), "{text}");
    assert!(count(&vm, "when rule fired") >= 6, "each rule says when it fires\n{text}");
    assert_eq!(count(&vm, "warning: script"), 0, "{text}");
}

/// The test mod tests/fixtures/mods/when-then on Plage01, as played: nothing during the opening
/// scene; once XIII can move, the shotgun, the message and Carrington at the hut; near him his
/// line and the goal (not from XIII's mark, 280 units away); L: an enemy who takes XIII as his
/// enemy at once; his death: the message.
#[test]
fn example_when_then_on_plage01() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/when-then");
    let file = dir.join("mod.toml");
    let (edits, actions, characters, problems) = read(&[("when-then", file)]);
    assert!(problems.is_empty(), "{problems:?}");
    let rules = crate::mod_setup::when_rules(&edits);
    assert!(rules.len() >= 4, "{rules:?}");
    let guard_def = characters.iter().find(|c| c.like.as_deref() == Some("Guard")).expect("the example's guard");
    assert_eq!(guard_def.team, Some(openxiii_mods::behaviour::Team::Enemy));
    assert_eq!(guard_def.base, openxiii_mods::simple::SOLDIER_CLASS, "team = enemy: a soldier that fights");
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: the example is only read");
        return;
    };
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_behaviour_here(actions, characters);
    vm.set_mod_when_here(rules.into_iter().map(|(r, _)| r).collect());
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "Plage01");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    // During the opening scene (its cut-scene frame on): nothing.
    let level = vm.level.unwrap();
    for _ in 0..90 {
        vm.set_prop(level, "bCineFrame", Value::Bool(true));
        vm.tick(1.0 / 30.0);
    }
    assert_eq!(count(&vm, "when rule fired"), 0, "{}", all(&vm));
    end_opening_scene(&mut vm, pawn);
    assert!(tick_until(&mut vm, pawn, None, "message \"Someone waits for you at the hut.\"", 3.0), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("give XIII.FusilPompe") && text.contains("spawn character Carrington"), "{text}");
    // Carrington's line near him, then the goal; not from XIII's mark.
    let general = vm.mod_script_resolve(&crate::mod_script::Who::Name("Carrington".into())).expect("Carrington");
    assert!(!tick_until(&mut vm, pawn, None, "objective \"Hold the beach\"", 1.0), "{}", all(&vm));
    let at = vm.native_value(general, Field::Location).as_vector();
    assert!(tick_until(&mut vm, pawn, Some([at[0] - 100.0, at[1], at[2]]), "objective \"Hold the beach\"", 3.0), "{}", all(&vm));
    assert!(all(&vm).contains("say Hual02_Carrington_06 by"), "{}", all(&vm));
    // L: the guard, an enemy whose AI takes XIII as its enemy.
    let far = [at[0] - 600.0, at[1], at[2]];
    assert!(vm.mod_script_resolve(&crate::mod_script::Who::Name("Guard".into())).is_none());
    assert!(vm.mod_script_key(pc, "L"));
    assert!(tick_until(&mut vm, pawn, Some(far), "spawn character Guard", 2.0), "{}", all(&vm));
    let guard = vm.mod_script_resolve(&crate::mod_script::Who::Name("Guard".into())).expect("the guard, found by who = Guard");
    tick_until(&mut vm, pawn, Some(far), "nothing", 2.0);
    let ai = vm.native_prop(guard, Field::Controller).as_object().expect("the guard's AI");
    assert_eq!(vm.prop(ai, "Enemy").as_object(), Some(pawn), "the guard attacks XIII\n{}", all(&vm));
    assert_eq!(count(&vm, "The guard is down."), 0);
    vm.set_prop(guard, "Health", Value::Int(0));
    assert!(tick_until(&mut vm, pawn, Some(far), "message \"The guard is down.\"", 3.0), "{}", all(&vm));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

/// near fires within its distance only: a character 270 units away with distance = 130 does
/// not trigger it; once he is moved next to XIII (teleport, a key's action), it does. (Without
/// a window the opening scene holds XIII at its start: the character is the one that moves.)
#[test]
fn near_keeps_its_distance_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.toml(
        "near",
        "[[map.Plage01.when]]\nevent = \"key\"\nkey = \"K\"\nthen = [ { spawn = \"Carrington\", position = [-470, -860, 1190] } ]\n\n[[map.Plage01.when]]\nevent = \"key\"\nkey = \"J\"\nthen = [ { teleport = [-658, -860, 1190], who = \"Carrington\" } ]\n\n[[map.Plage01.when]]\nevent = \"near\"\nwho = \"Carrington\"\ndistance = 130\nthen = [ { log = \"near fired\" } ]\n",
    );
    let (mut vm, pc, pawn, problems) = plage01(&game, &folder, &["near"]);
    assert!(problems.is_empty(), "{problems:?}");
    // The end of the opening scene: near waits for it.
    end_opening_scene(&mut vm, pawn);
    assert!(vm.mod_script_key(pc, "K"));
    assert!(tick_until(&mut vm, pawn, None, "spawn character Carrington", 2.0), "{}", all(&vm));
    let general = vm.mod_script_resolve(&crate::mod_script::Who::Name("Carrington".into())).expect("Carrington");
    let distance = |vm: &Vm| {
        let (g, p) = (vm.native_value(general, Field::Location).as_vector(), vm.native_value(pawn, Field::Location).as_vector());
        ((g[0] - p[0]).powi(2) + (g[1] - p[1]).powi(2)).sqrt()
    };
    assert!(!tick_until(&mut vm, pawn, None, ": near fired", 2.0), "{} units away\n{}", distance(&vm), all(&vm));
    assert!(distance(&vm) > 200.0, "{}", distance(&vm));
    assert!(vm.mod_script_key(pc, "J"));
    assert!(tick_until(&mut vm, pawn, None, ": near fired", 2.0), "{} units away\n{}", distance(&vm), all(&vm));
    assert!(distance(&vm) < 130.0);
}

/// A block's start waits until XIII can move: nothing while the level's cut-scene frame is on,
/// then once, one second after it went off; a script's on("start") runs at once (negative
/// control: the raw event).
#[test]
fn start_waits_for_control_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.toml("m", "[[map.Plage01.when]]\nevent = \"start\"\nthen = [ { log = \"block start\" } ]\n");
    folder.script("m", "s.lua", "on('start', function() log('script start') end)");
    let (mut vm, _pc, pawn, problems) = plage01(&game, &folder, &["m"]);
    assert!(problems.is_empty(), "{problems:?}");
    let level = vm.level.unwrap();
    for _ in 0..120 {
        vm.set_prop(level, "bCineFrame", Value::Bool(true));
        vm.tick(1.0 / 30.0);
    }
    assert_eq!(count(&vm, ": script start"), 1, "{}", all(&vm));
    assert_eq!(count(&vm, ": block start"), 0, "during the scene\n{}", all(&vm));
    end_opening_scene(&mut vm, pawn);
    assert!(!tick_until(&mut vm, pawn, None, ": block start", 0.7), "not before a second\n{}", all(&vm));
    assert!(tick_until(&mut vm, pawn, None, ": block start", 2.0), "{}", all(&vm));
    tick_until(&mut vm, pawn, None, "nothing", 1.0);
    assert_eq!(count(&vm, ": block start"), 1);
}
