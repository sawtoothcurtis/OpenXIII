//! Mods build only: the mods' Lua scripts (mod_script.rs).
//!
//! Without the game: the API table, the sandbox, the instruction budget, the error lines, the
//! timers, `modreload`, a mod without scripts, and no scripts on a network client. With the game
//! (OXIII_GAME = original game folder; skipped when it is not set), on the real Plage01: every
//! function of the API, and the test mod tests/fixtures/mods/first-script.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::mod_script::{ScriptMod, API, BUDGET};
use crate::value::{ObjId, Value};
use crate::{NativeField as Field, Vm};

/// A Mods folder of its own, removed at the end.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Folder {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!("oxiii-scripts-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Folder(p)
    }

    /// Writes `<mod>/scripts/<file>`.
    fn script(&self, m: &str, file: &str, text: impl AsRef<[u8]>) -> &Folder {
        let dir = self.0.join(m).join("scripts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
        self
    }

    fn mods(&self) -> Vec<ScriptMod> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0).unwrap().filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names.into_iter().map(|name| ScriptMod { folder: self.0.join(&name), name }).collect()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A Vm without a game, the scripts of `folder` started.
fn started(folder: &Folder) -> Vm {
    let mut vm = Vm::new();
    vm.set_mod_scripts_here(folder.mods());
    vm.mod_scripts_start();
    vm
}

fn all(vm: &Vm) -> String {
    vm.mod_messages.join("\n")
}

fn count(vm: &Vm, what: &str) -> usize {
    vm.mod_messages.iter().filter(|m| m.contains(what)).count()
}

/// Runs the scripts' frames from `from` to `to` seconds, 30 per second.
fn frames(vm: &mut Vm, from: f32, to: f32) {
    let mut t = from;
    while t <= to + 1e-4 {
        vm.mod_scripts_step(t);
        t += 1.0 / 30.0;
    }
}

#[test]
fn api_table_is_complete_and_registered() {
    // Unique among the globals and among the methods (actor:trigger() and trigger(event) are two).
    let mut names: Vec<(bool, &str)> = API.iter().map(|f| (f.method, f.name)).collect();
    names.sort();
    let n = names.len();
    names.dedup();
    assert_eq!(names.len(), n, "names are unique");
    for f in API.iter().filter(|f| !f.hidden) {
        assert!(!f.about.is_empty() && !f.example.is_empty(), "{} has its description and example", f.name);
    }
    for name in [
        "behaviour",
        "characters_near",
        "camera",
        "camera_back",
        "on",
        "wait",
        "wait_until",
        "after",
        "every",
        "find",
        "player",
        "players",
        "spawn",
        "remove",
        "say",
        "message",
        "log",
        "change",
        "objective",
        "objective_done",
        "trigger",
        "sound",
        "music",
        "next_level",
        "fail",
        "random",
        "class_get",
        "class_set",
    ] {
        assert!(API.iter().any(|f| f.name == name && !f.hidden), "{name} is in the table");
    }
    // Each listed function is a function of the scripts; the hidden ones are not.
    let mut checks: Vec<String> = API.iter().map(|f| format!("log('{}', type({}))\n", f.name, f.name.replace('.', "_"))).collect();
    checks.sort();
    checks.dedup();
    let checks: String = checks.concat();
    let folder = Folder::new();
    folder.script("a", "check.lua", checks);
    let vm = started(&folder);
    for f in API {
        // Methods are called on an actor (actor:go_to(...)), not as globals; a method named as a
        // global function is checked with the global.
        if f.method && API.iter().any(|g| !g.method && g.name == f.name) {
            continue;
        }
        let expected = if f.hidden || f.method { "nil" } else { "function" };
        assert_eq!(count(&vm, &format!(": {} {expected}", f.name)), 1, "{}\n{}", f.name, all(&vm));
    }
}

#[test]
fn sandbox_has_no_files_no_system_no_bytecode() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        for _, name in ipairs({"io", "os", "debug", "package", "require", "load", "loadfile", "dofile", "loadstring", "collectgarbage", "print"}) do
          log("missing", name, _G[name] == nil and rawget(_G, name) == nil and _ENV[name] == nil)
        end
        log("dump", string.dump == nil)
        log("libs", type(string.format), type(table.insert), type(math.floor), type(coroutine.create), type(pairs), type(pcall))
        log("string metatable", tostring(getmetatable("")))
        log("map", map())
        shared = "from a"
        "#,
    );
    folder.script("b", "1.lua", "log('other mod sees', tostring(shared))\n");
    // A precompiled chunk (Lua 5.4 bytecode header) is refused.
    folder.script("c", "1.lua", b"\x1bLua\x54\x00\x19\x93\r\n\x1a\n\x04\x08\x08x\x56\x00\x00".as_slice());
    let vm = started(&folder);
    let text = all(&vm);
    for name in ["io", "os", "debug", "package", "require", "load", "loadfile", "dofile", "loadstring", "collectgarbage", "print"] {
        assert!(text.contains(&format!(": missing {name} true")), "{name}\n{text}");
    }
    assert!(text.contains(": dump true"), "{text}");
    assert!(text.contains(": libs function function function function function function"), "{text}");
    assert!(text.contains(": string metatable false"), "{text}");
    assert!(text.contains(": other mod sees nil"), "each mod has its own globals\n{text}");
    assert!(text.contains("c/scripts/1.lua") && text.contains("binary"), "bytecode refused\n{text}");
    // Negative control: a script that does reach `os` would say so.
    let folder = Folder::new();
    folder.script("a", "1.lua", "log('os here', os ~= nil)");
    let vm = started(&folder);
    assert!(all(&vm).contains("os here false"), "{}", all(&vm));
}

#[test]
fn runaway_scripts_are_stopped_with_one_line() {
    let folder = Folder::new();
    folder.script("a", "1-main.lua", "local n = 0 while true do n = n + 1 end\n");
    folder.script("a", "2-timer.lua", "after(0, function() while true do end end)\n");
    // pcall cannot keep a runaway run going, even from inside a C function (table.sort).
    folder.script("a", "3-pcall.lua", "after(0, function() while true do pcall(function() while true do end end) end end)\n");
    folder.script("a", "4-sort.lua", "after(0, function() while true do pcall(table.sort, {3, 2, 1, 5, 4}, function(x, y) while true do end end) end end)\n");
    folder.script("a", "5-every.lua", "every(0.1, function() while true do end end)\n");
    folder.script("a", "6-alive.lua", "every(0.1, function() ticks = (ticks or 0) + 1 if ticks == 20 then log('still running') end end)\n");
    let started_at = std::time::Instant::now();
    let mut vm = started(&folder);
    frames(&mut vm, 0.0, 3.0);
    let text = all(&vm);
    for f in ["1-main.lua", "2-timer.lua", "3-pcall.lua", "4-sort.lua", "5-every.lua"] {
        let lines = vm.mod_messages.iter().filter(|m| m.contains(f) && m.contains("stopped")).count();
        assert_eq!(lines, 1, "{f}: one line, and the handler is removed\n{text}");
    }
    assert!(text.contains(&format!("more than {BUDGET} instructions")), "{text}");
    assert_eq!(count(&vm, "still running"), 1, "the other scripts go on\n{text}");
    assert!(started_at.elapsed().as_secs() < 60, "never frozen ({:?})", started_at.elapsed());
}

#[test]
fn errors_go_to_the_log_with_file_and_line() {
    let folder = Folder::new();
    folder.script("a", "1-syntax.lua", "log('a')\n\nthis is not lua\n");
    folder.script("a", "2-runtime.lua", "after(1, function()\n  local t = nil\n  log('before')\n  return t.x\nend)\n");
    folder.script("a", "3-api.lua", "after(1, function()\n  spawn{position = {0, 0, 0}}\nend)\n");
    folder.script("a", "4-error.lua", "on('nothing', function() end)\n");
    folder.script("a", "5-after.lua", "after(2, function() log('later') end)\n");
    let mut vm = started(&folder);
    frames(&mut vm, 0.0, 3.0);
    let text = all(&vm);
    assert!(text.contains("a/scripts/1-syntax.lua:3:"), "{text}");
    assert!(text.contains("a/scripts/2-runtime.lua:4:") && text.contains("before"), "{text}");
    assert!(text.contains("a/scripts/3-api.lua:2:") && text.contains("give character = or class ="), "{text}");
    assert!(text.contains("a/scripts/4-error.lua:1:") && text.contains("unknown event \"nothing\""), "{text}");
    assert!(text.contains(": later"), "the game and the other scripts go on\n{text}");
    let file = vm.mod_messages.iter().filter(|m| m.starts_with("warning: script")).count();
    assert_eq!(file, 4, "{text}");
}

#[test]
fn wait_after_every_cancel_on_game_time() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        after(1, function()
          log("after1")
          wait(2)
          log("after3")
        end)
        local n = 0
        every(0.5, function()
          n = n + 1
          log("every" .. n)
          if n == 3 then return false end
        end)
        local id = after(2, function() log("cancelled ran") end)
        log("cancel", cancel(id), cancel(id))
        local h = on("key", "K", function() log("key ran") end)
        cancel(h)
        "#,
    );
    let mut vm = started(&folder);
    let mut times = Vec::new();
    let mut t = 0.0f32;
    while t <= 6.0 {
        let before = vm.mod_messages.len();
        vm.mod_scripts_step(t);
        for m in &vm.mod_messages[before..] {
            times.push((m.rsplit(": ").next().unwrap_or_default().to_string(), t));
        }
        t += 0.1;
    }
    let at = |what: &str| times.iter().find(|(m, _)| m == what).map(|(_, t)| *t).unwrap_or_else(|| panic!("{what}\n{times:?}"));
    let near = |what: &str, t: f32| (at(what) - t).abs() < 0.25;
    assert!(near("after1", 1.0) && near("after3", 3.0), "{times:?}");
    assert!(near("every1", 0.5) && near("every2", 1.0) && near("every3", 1.5), "{times:?}");
    assert!(!times.iter().any(|(m, _)| m == "every4"), "false stops every\n{times:?}");
    assert!(!times.iter().any(|(m, _)| m == "cancelled ran"), "{times:?}");
    assert!(all(&vm).contains("cancel true false"), "{}", all(&vm));
    assert!(vm.mod_script_keys().is_empty(), "the key handler was cancelled");
}

/// A key the game window keeps (F8: co-op character screen) gives one warning naming free keys;
/// a free key gives none.
#[test]
fn a_key_kept_by_the_game_window_is_said() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        "on('key', 'F8', function() end)
on('key', 'K', function() end)
on('key', 'f9', function() end)
",
    );
    let mut vm = started(&folder);
    frames(&mut vm, 0.0, 0.2);
    let text = all(&vm);
    assert_eq!(count(&vm, "is kept by the game window"), 2, "{text}");
    assert!(text.contains("on(\"key\", \"F8\")") && text.contains("co-op character screen"), "{text}");
    assert!(text.contains("on(\"key\", \"f9\")") && text.contains("its own use"), "{text}");
    assert!(text.contains(openxiii_mods::behaviour::FREE_KEYS), "{text}");
    assert!(!text.contains("on(\"key\", \"K\")"), "{text}");
    assert!(
        text.contains("1.lua:1"),
        "where it is written
{text}"
    );
}

/// The prelude's pcall and xpcall give every argument and every value, nils in the middle
/// and last included (a multi-value forwarded before another value keeps only its first).
/// pairs() visits numbers, then strings, then booleans, each in increasing order, the same in
/// every run (Lua's own order of string keys changes with its hash seed); a key removed during
/// the loop is skipped, and __pairs is kept. Negative control: next() keeps Lua's own order, and
/// the same table walked with it is not sorted.
#[test]
fn pairs_visits_keys_in_a_fixed_order() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r##"
        local t = {zebra = 1, apple = 2, mango = 3, [10] = 4, [2] = 5, [true] = 6, kiwi = 7, [false] = 8, [1.5] = 9}
        local out = {}
        for k, v in pairs(t) do out[#out + 1] = tostring(k) .. "=" .. v end
        log("order", table.concat(out, ","))
        local u = {a = 1, b = 2, c = 3}
        local seen = {}
        for k in pairs(u) do seen[#seen + 1] = k; u.b = nil end
        log("removed", table.concat(seen, ","))
        local m = setmetatable({}, {__pairs = function() return function(_, k) if not k then return "only", 1 end end, nil, nil end})
        for k, v in pairs(m) do log("meta", k, v) end
        local big = {}
        for i = 1, 200 do big["key" .. i] = i end
        local raw, sorted = {}, {}
        for k in next, big do raw[#raw + 1] = k end
        for k in pairs(big) do sorted[#sorted + 1] = k end
        local copy = {table.unpack(raw)}
        table.sort(copy)
        log("pairs sorted", table.concat(sorted, ",") == table.concat(copy, ","), "next sorted", table.concat(raw, ",") == table.concat(copy, ","))
        "##,
    );
    let vm = started(&folder);
    let text = all(&vm);
    assert!(text.contains("order 1.5=9,2=5,10=4,apple=2,kiwi=7,mango=3,zebra=1,false=8,true=6"), "{text}");
    assert!(text.contains("removed a,c"), "{text}");
    assert!(text.contains("meta only 1"), "{text}");
    assert!(text.contains("pairs sorted true next sorted false"), "{text}");
}

#[test]
fn pcall_keeps_every_value() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r##"
        local function many(...) return select("#", ...), ... end
        local r = table.pack(pcall(many, 1, nil, 3, nil))
        log("pcall", r.n, r[1], r[2], r[3], r[4], r[5], r[6])
        local x = table.pack(xpcall(many, function(e) return e end, "a", nil, "c"))
        log("xpcall", x.n, x[1], x[2], x[3], x[4], x[5])
        local e = table.pack(pcall(error, {code = 7}))
        log("error", e.n, e[1], type(e[2]))
        "##,
    );
    let vm = started(&folder);
    let text = all(&vm);
    assert!(text.contains("pcall 6 true 4 1 nil 3 nil"), "{text}");
    assert!(text.contains("xpcall 5 true 3 a nil c"), "{text}");
    assert!(text.contains("error 2 false"), "{text}");
}

#[test]
fn modreload_reads_the_files_again() {
    let folder = Folder::new();
    folder.script("a", "1.lua", "log('version one')");
    let mut vm = started(&folder);
    assert_eq!(count(&vm, "version one"), 1);
    folder.script("a", "1.lua", "log('version two')");
    folder.script("a", "2.lua", "log('new file')");
    let out = vm.mod_console(0, "modreload").unwrap();
    assert!(out.contains("2 script(s) started"), "{out}");
    assert_eq!(count(&vm, "version two"), 1, "{}", all(&vm));
    assert_eq!(count(&vm, "new file"), 1, "{}", all(&vm));
    // No mod with scripts: modreload says so.
    let mut plain = Vm::new();
    assert!(plain.mod_console(0, "modreload").unwrap().contains("no mod has scripts"));
}

#[test]
fn a_mod_without_scripts_runs_nothing() {
    let folder = Folder::new();
    std::fs::create_dir_all(folder.0.join("a").join("scripts")).unwrap();
    std::fs::write(folder.0.join("a").join("scripts").join("notes.txt"), "not a script").unwrap();
    let vm = started(&folder);
    assert!(!vm.mod_scripts_running());
    assert_eq!(count(&vm, "script"), 0, "{}", all(&vm));
    let vm = Vm::new();
    assert!(vm.mods.script_sources.is_none() && !vm.mod_scripts_running());
}

#[test]
fn no_scripts_on_a_network_client() {
    use crate::ipdrv::{NetDriver, NetDriverCfg, Sock};
    use crate::net::{ConnState, Connection};
    let folder = Folder::new();
    folder.script("a", "1.lua", "log('ran here')");
    let client = |server_conn: bool| {
        let mut vm = Vm::new();
        vm.set_mod_scripts_here(folder.mods());
        let mut d = NetDriver::new(Sock(None, 0), 0, true, NetDriverCfg::default());
        if server_conn {
            d.server_conn = Some(crate::net_game::NetConn::new(Connection::new("127.0.0.1:7777".parse().unwrap(), ConnState::Open, 0.0, 0)));
        }
        vm.ipdrv.net_driver = Some(d);
        vm.mod_scripts_start();
        vm
    };
    let on_client = client(true);
    assert!(!on_client.mod_scripts_running());
    assert_eq!(count(&on_client, "ran here"), 0);
    // The host of the same game runs them.
    let on_host = client(false);
    assert!(on_host.mod_scripts_running());
    assert_eq!(count(&on_host, "ran here"), 1);
}

// ---------------------------------------------------------------------------------------------
// On the real Plage01.

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/first-script")
}

fn mission_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/beach-mission")
}

/// Plage01 with the [character]s of `toml` and the scripts of `mods`, the player logged in.
fn plage01(game: &Path, toml: &Path, mods: Vec<ScriptMod>) -> (Vm, ObjId, ObjId) {
    level(game, "Plage01", toml, mods)
}

/// Map `map` (found by library::map_path: the game's, or a mod's) started as plage01() does.
fn level(game: &Path, map: &str, toml: &Path, mods: Vec<ScriptMod>) -> (Vm, ObjId, ObjId) {
    let file = openxiii_mods::defaults::parse_mod_file(&std::fs::read_to_string(toml).unwrap(), toml, "test");
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_classes_here(file.new_classes);
    vm.set_mod_defaults_here(file.overrides);
    vm.set_mod_edits_here(file.edits);
    vm.set_mod_behaviour_here(file.actions, file.characters);
    vm.set_mod_scripts_here(mods);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), map);
    vm.url = url.clone();
    vm.load_map(&openxiii_package::library::map_path(game, map)).unwrap();
    let game_class = vm.game_class_for(&url);
    let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
    vm.audio_init(game, map);
    vm.audio_start_level();
    let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
    // Out of the opening cinematic: the player walks.
    let level = vm.level.unwrap();
    vm.set_prop(level, "bCineFrame", Value::Bool(false));
    (vm, pc, pawn)
}

fn put(vm: &mut Vm, pawn: ObjId, at: [f32; 3]) {
    let mut args = [Value::vector(at[0], at[1], at[2])];
    let _ = crate::natives::call(vm, "SetLocation", pawn, &mut args, &[true]);
}

/// Ticks until a line contains `what` (at most `seconds`), the player kept at `at` if given.
fn tick_until(vm: &mut Vm, pawn: ObjId, at: Option<[f32; 3]>, what: &str, seconds: u32) -> bool {
    for frame in 0..(seconds * 30) {
        if let (Some(p), 0) = (at, frame % 15) {
            put(vm, pawn, p);
        }
        // Out of the opening cinematic (without a window it never ends): the player walks.
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

/// Co-op on the real Plage01: a script's goal goes to the first player only (the co-op shows
/// it on the other HUDs itself); without co-op, to every player (negative control).
#[cfg(feature = "improved")]
#[test]
fn coop_goal_goes_to_the_first_player_only() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    for coop in [true, false] {
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let url = crate::travel::start_url(&game.join("System"), "Plage01");
        vm.url = url.clone();
        vm.set_coop(coop);
        vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
        let game_class = vm.game_class_for(&url);
        let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
        vm.begin_play();
        vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap();
        for _ in 0..30 {
            vm.tick(1.0 / 30.0);
        }
        let players = vm.mod_script_player_controllers();
        let targets = vm.mod_script_goal_targets();
        if coop {
            assert_eq!(players.len(), 2, "two players in co-op");
            assert_eq!(targets, vec![vm.coop.players[0]]);
        } else {
            assert_eq!(targets, players);
        }
    }
}

/// The co-op character screen's backdrop map ("Map?OXIIIChoose") runs no script; the same map
/// played runs them (negative control).
#[cfg(feature = "improved")]
#[test]
fn no_script_behind_the_character_screen() {
    let folder = Folder::new();
    folder.script("a", "1.lua", "log('ran')");
    for screen in [true, false] {
        let mut vm = Vm::new();
        vm.url = crate::travel::Url::parse(None, "Plage01", 0);
        if screen {
            vm.url.add_option(crate::character_screen::SCREEN_OPTION);
        }
        vm.set_mod_scripts_here(folder.mods());
        vm.mod_scripts_start();
        assert_eq!(count(&vm, "ran"), usize::from(!screen), "screen {screen}: {}", all(&vm));
    }
}

/// panel{...} on the real Plage01: the game's inset window opens on the player's HUD with the
/// settings given (camera point, target, place, size, text, picture), the second inset comes
/// `after` seconds later, both close after their time; wrong settings are refused with a clear
/// error (negative controls).
#[test]
fn panels_open_the_games_inset_window() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
        on("start", function(p)
          local at = p.position
          local n = panel{
            {from = {at[1] + 200, at[2], at[3] + 80}, look_at = p, seconds = 2, x = 300, y = 60, size = {128, 96}, text = "Who is there?", word = "plage01a", appear = "zoom", sound = "tip", after = 1},
            {from = {at[1], at[2] + 200, at[3] + 80}, look_at = {at[1], at[2], at[3]}, seconds = 1, after = 1},
          }
          log("panel seconds", n)
          for _, bad in ipairs{{look_at = p}, {from = at, look_at = p, appear = "spin"}, {from = at, look_at = p, colour = 1}, {from = at, look_at = p, size = 900}} do
            local ok, e = pcall(panel, bad)
            log("refused", not ok, e)
          end
        end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    let windows = |vm: &Vm| -> Vec<ObjId> {
        vm.actors.iter().copied().filter(|&a| vm.objects.get(a).is_some_and(|o| o.is_some()) && vm.is_a(a, "CWndOnTrigger")).collect()
    };
    assert!(tick_until(&mut vm, pawn, None, "panel seconds", 2), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("panel seconds 2"), "{text}");
    assert_eq!(count(&vm, "refused true"), 4, "{text}");
    assert!(
        text.contains("from = the camera's point")
            && text.contains("appear = \"spin\"")
            && text.contains("unknown setting colour")
            && text.contains("size 900x900"),
        "{text}"
    );
    let tick = |vm: &mut Vm, frames: usize| {
        for _ in 0..frames {
            vm.tick(1.0 / 30.0);
        }
    };
    tick(&mut vm, 15);
    let w = windows(&vm);
    assert_eq!(
        w.len(),
        1,
        "one inset after 0.5 s
{}",
        all(&vm)
    );
    let w = w[0];
    assert_eq!(vm.prop(w, "vScrPosX").as_int(), 300);
    assert_eq!(vm.prop(w, "CWndSizeX").as_int(), 128);
    assert_eq!(vm.prop(w, "CWndSizeY").as_int(), 96);
    assert_eq!(vm.prop(w, "CWndAppearFX").as_int(), 5);
    assert!(vm.prop(w, "iA").as_int() >= 0, "room in the HUD's picture");
    assert_eq!(vm.prop(w, "Tar").as_object(), Some(pawn));
    assert!(!matches!(vm.prop(w, "OnoTexture"), Value::Object(None)), "the picture is set");
    let cam = vm.prop(w, "Cam").as_object().unwrap();
    let [x, _, _] = vm.native_value(cam, Field::Location).as_vector();
    let [px, _, _] = vm.native_value(pawn, Field::Location).as_vector();
    assert!((x - px - 200.0).abs() < 60.0, "camera point {x} for a player at {px}");
    tick(&mut vm, 30);
    assert_eq!(
        windows(&vm).len(),
        2,
        "the second inset comes one second later
{}",
        all(&vm)
    );
    tick(&mut vm, 90);
    assert!(windows(&vm).is_empty(), "both insets closed");
}

/// A mod made of a mod.toml and a scripts folder only is found like any other mod (the Mods
/// page lists every mod found, on or off); turned off there, it is not loaded at the next start,
/// so none of its scripts runs.
#[test]
fn a_script_only_mod_is_listed_and_can_be_turned_off() {
    let folder = Folder::new();
    folder.script("only-scripts", "1.lua", "log('ran')");
    std::fs::write(
        folder.0.join("only-scripts").join("mod.toml"),
        "[mod]
name = \"Only scripts\"
version = \"1.0.0\"
",
    )
    .unwrap();
    let mods = folder.0.as_path();
    let all = openxiii_mods::discover_all(mods);
    assert_eq!(all.mods.len(), 1, "{:?}", all.problems);
    assert!(all.problems.is_empty(), "{:?}", all.problems);
    assert_eq!(all.mods[0].scripts.len(), 1);
    assert_eq!(openxiii_mods::discover(mods).mods.len(), 1, "on by default");
    openxiii_mods::write_disabled(mods, &["only-scripts".to_string()]).unwrap();
    let on = openxiii_mods::discover(mods);
    assert!(on.mods.is_empty() && on.disabled.len() == 1, "turned off: not loaded");
    assert_eq!(openxiii_mods::discover_all(mods).mods.len(), 1, "still listed by the Mods page");
}

/// A network guest's key, as the host receives it (the guest's ServerUpdateSkin with a key):
/// on("key") fires on the host with that player; a key the game window keeps, or a real skin
/// code, is not taken as a key (negative controls). The guest's side: the host's list of keys
/// becomes keys the guest watches.
#[test]
fn a_guest_key_reaches_the_hosts_scripts() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"on("key", "K", function(p) log("key by", p == player()) end) on("key", "F8", function() log("F8 ran") end)"#;
    let (_folder, mut vm, pc, pawn) = plage01_with(&game, "", lua);
    tick_until(&mut vm, pawn, None, "never", 1);
    let f = vm.find_function(pc, "ServerUpdateSkin", false).expect("the game's guest-to-host function");
    let call = |vm: &mut Vm, text: &str| vm.mod_guest_key_call(f, pc, &[Some(Value::Str(text.into()))]);
    assert!(call(&mut vm, "OXIIIMod.Key|K"));
    assert!(tick_until(&mut vm, pawn, None, "key by true", 1), "{}", all(&vm));
    assert!(call(&mut vm, "OXIIIMod.Key|F8"), "taken, not run");
    assert!(!call(&mut vm, "XIII"), "a skin code stays a skin code");
    tick_until(&mut vm, pawn, None, "never", 1);
    assert_eq!(count(&vm, "F8 ran"), 0, "{}", all(&vm));
    // The guest's side (this VM is no network guest: nothing is sent).
    vm.mod_guest_keys_from_host("K, L");
    assert_eq!(vm.mods.guest_keys, vec!["K".to_string(), "L".to_string()]);
    assert!(vm.mod_action_keys().iter().any(|k| k == "L"));
    assert!(!vm.mod_guest_key(pc, "L"), "only a network guest sends keys");
    assert!(crate::mod_script_game::parse_guest_line(&crate::mod_script_keys::keys_line(&["K", "L"])).is_some_and(|(k, _, t)| k == "Keys" && t == "K,L"));
}

/// The test mod comic-insets on the real Plage01: once the opening scene ends, its insets
/// open on the player's HUD, the first with its caption and the second with its picture.
#[test]
fn the_comic_insets_example_runs() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/comic-insets");
    let mods = vec![ScriptMod { name: "comic-insets".into(), folder: dir.clone() }];
    let (mut vm, _pc, pawn) = plage01(&game, &dir.join("mod.toml"), mods);
    tick_until(&mut vm, pawn, None, "never", 1);
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    let windows = |vm: &Vm| -> Vec<ObjId> {
        vm.actors.iter().copied().filter(|&a| vm.objects.get(a).is_some_and(|o| o.is_some()) && vm.is_a(a, "CWndOnTrigger")).collect()
    };
    for _ in 0..75 {
        vm.tick(1.0 / 30.0);
    }
    let w = windows(&vm);
    assert_eq!(
        w.len(),
        2,
        "two insets 2.5 s after the event
{}",
        all(&vm)
    );
    assert_eq!(count(&vm, "warning"), 0, "{}", all(&vm));
}

/// camera{...} on the real Plage01: a path is followed at an even speed over the seconds, an
/// orbit keeps its radius around the target, fov sets every player's field of view and the
/// end of the scene gives back the player's own; wrong settings are refused (negative
/// controls). who:cine gives a line of the scene language to a story character only.
#[test]
fn camera_settings_and_cine_lines() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
        on("start", function(p)
          local at = p.position
          camera{from = {at[1], at[2], at[3] + 100}, look_at = p, path = {{at[1] + 400, at[2], at[3] + 100}}, seconds = 2, fov = 60}
          log("path set")
          wait(3)
          camera{from = {at[1] + 200, at[2], at[3] + 60}, look_at = p, orbit = {turns = 0.25}, seconds = 2}
          log("orbit set")
          wait(3)
          for _, bad in ipairs{{look_at = p, path = {at}}, {look_at = p, from = at, fov = 500}, {look_at = p, from = at, zoom = 2}, {look_at = p, orbit = {}}} do
            local ok, e = pcall(camera, bad)
            log("refused", not ok, e)
          end
          local ok, e = pcall(function() p:cine("Anim Once Rire") end)
          log("cine on the player", ok, e)
          log("done")
        end)
    "#;
    let (_folder, mut vm, pc, pawn) = plage01_with(&game, "", lua);
    let tick = |vm: &mut Vm, frames: usize| {
        for _ in 0..frames {
            vm.tick(1.0 / 30.0);
        }
    };
    assert!(tick_until(&mut vm, pawn, None, "path set", 1), "{}", all(&vm));
    let target = vm.prop(pc, "ViewTarget").as_object().unwrap();
    let [x0, ..] = vm.native_value(target, Field::Location).as_vector();
    let [px, py, _] = vm.native_value(pawn, Field::Location).as_vector();
    assert!((vm.prop(pc, "FovAngle").as_float() - 60.0).abs() < 0.1);
    tick(&mut vm, 30);
    let [x1, ..] = vm.native_value(target, Field::Location).as_vector();
    assert!((x1 - x0 - 200.0).abs() < 40.0, "half the path in half the time: {x0} -> {x1}");
    tick(&mut vm, 45);
    assert_eq!(vm.prop(pc, "ViewTarget").as_object(), Some(pawn), "the scene is over");
    assert!((vm.prop(pc, "FovAngle").as_float() - vm.prop(pc, "DefaultFOV").as_float()).abs() < 0.1, "the player's own field of view");
    assert!(tick_until(&mut vm, pawn, None, "orbit set", 2), "{}", all(&vm));
    tick(&mut vm, 20);
    let target = vm.prop(pc, "ViewTarget").as_object().unwrap();
    let [ox, oy, _] = vm.native_value(target, Field::Location).as_vector();
    let r = ((ox - px).powi(2) + (oy - py).powi(2)).sqrt();
    assert!((r - 200.0).abs() < 40.0 && (oy - py).abs() > 20.0, "around the player at 200: ({ox}, {oy}) r {r}");
    assert!(tick_until(&mut vm, pawn, None, ": done", 5), "{}", all(&vm));
    let text = all(&vm);
    assert_eq!(count(&vm, "refused true"), 4, "{text}");
    assert!(text.contains("cine on the player false") && text.contains("not a story character"), "{text}");
}

/// checkpoint, focus, slow_motion and fade on the real Plage01: the game's save point saves,
/// the focus inset goes to the HUD, the level's time scale changes and comes back, the view's
/// glow goes to black and back; an unknown teleporter is refused (negative control).
#[test]
fn checkpoint_focus_slow_motion_and_fade() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
        on("start", function(p)
          log("bad teleporter", pcall(checkpoint, "Here", "NoSuchTeleporter"))
          checkpoint("On the beach")
          focus{on = p, seconds = 2, zoom = true}
          slow_motion(0.5, 1)
          fade{to = "black", seconds = 1}
          log("effects set")
          wait(2)
          fade{back = true, seconds = 0.5}
          wait(1)
          log("effects done")
        end)
    "#;
    let (_folder, mut vm, pc, pawn) = plage01_with(&game, "", lua);
    let hud = vm.prop(pc, "myHUD").as_object().unwrap();
    assert!(vm.prop(hud, "HudFoc").as_object().is_none());
    assert!(tick_until(&mut vm, pawn, None, "effects set", 1), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("bad teleporter false") && text.contains("no teleporter NoSuchTeleporter"), "{text}");
    let focus = vm.prop(hud, "HudFoc").as_object().expect("the focus inset is on the HUD");
    assert_eq!(vm.prop(focus, "MyFocus").as_object(), Some(pawn));
    let level = vm.level.unwrap();
    assert!((vm.prop(level, "TimeDilation").as_float() - 0.5).abs() < 0.01);
    for _ in 0..70 {
        vm.tick(1.0 / 30.0);
    }
    assert!(vm.prop(pc, "ConstantGlowScale").as_float() < -0.99, "faded to black");
    assert!((vm.prop(level, "TimeDilation").as_float() - 1.0).abs() < 0.01, "normal speed again");
    assert!(vm.log.iter().any(|l| l.contains(":SAVE:")), "the game's save point saved");
    assert!(tick_until(&mut vm, pawn, None, "effects done", 4), "{}", all(&vm));
    assert!(vm.prop(pc, "ConstantGlowScale").as_float().abs() < 0.01, "back from black");
}

/// has_control is false while the level's cut-scene frame is on and true once it is off
/// (negative control: during the scene the waiting handler does not go on).
#[test]
fn has_control_follows_the_cut_scene_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.script("a", "1.lua", "on('start', function() wait(1) log('during', has_control()) wait_until(has_control) log('got control', has_control()) end)");
    let toml = folder.0.join("a").join("mod.toml");
    std::fs::write(
        &toml,
        "[mod]
name = \"A\"
version = \"1\"
",
    )
    .unwrap();
    let (mut vm, _pc, pawn) = plage01(&game, &toml, folder.mods());
    let level = vm.level.unwrap();
    for _ in 0..90 {
        vm.set_prop(level, "bCineFrame", Value::Bool(true));
        vm.tick(1.0 / 30.0);
    }
    assert_eq!(count(&vm, ": during false"), 1, "{}", all(&vm));
    assert_eq!(
        count(&vm, ": got control"),
        0,
        "during the scene
{}",
        all(&vm)
    );
    assert!(tick_until(&mut vm, pawn, None, ": got control true", 1), "{}", all(&vm));
}

/// The example mod: near Janet, her line, three seconds, the guard, his line; ten seconds later
/// he turns hostile; his death is seen.
#[test]
fn example_scene_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = example_dir();
    let mods = vec![ScriptMod { name: "first-script".into(), folder: dir.clone() }];
    let (mut vm, pc, pawn) = plage01(&game, &dir.join("mod.toml"), mods);
    assert!(vm.mod_scripts_running(), "{}", all(&vm));
    assert!(tick_until(&mut vm, pawn, None, "the scene is ready on Plage01", 2), "{}", all(&vm));
    // The end of the opening scene (Objectif91, as its last step fires it): the visitor comes.
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    assert!(tick_until(&mut vm, pawn, None, "spawn character Visitor", 2), "{}", all(&vm));
    let visitor = vm.mod_script_resolve(&crate::mod_script::Who::Name("Visitor".into())).expect("the visitor");
    let at = vm.native_value(visitor, Field::Location).as_vector();
    let start = vm.native_value(pawn, Field::Location).as_vector();
    let far = [start[0] - 2000.0, start[1], start[2]];
    assert!(
        !tick_until(&mut vm, pawn, Some(far), "say Hual02_Carrington_06", 2),
        "far from him, nothing
{}",
        all(&vm)
    );
    let near = [at[0] - 150.0, at[1], at[2]];
    let best = vm.mod_game_line_text("Hual02_Carrington_06").expect("Carrington's line");
    let line = format!("say Hual02_Carrington_06 by {}: \"{best}\"", vm.mod_actor_name(visitor));
    assert!(tick_until(&mut vm, pawn, Some(near), &line, 10), "{}", all(&vm));
    assert!(!vm.mods.live.keys().any(|&a| vm.prop(a, "Mesh") == Value::Asset("XIIIPersos.SpadsM".into())), "the guard waits four seconds");
    assert!(tick_until(&mut vm, pawn, Some(near), "spawn character Guard", 6), "{}", all(&vm));
    let guard = vm.mod_script_resolve(&crate::mod_script::Who::Name("Guard".into())).expect("the guard");
    assert_eq!(vm.prop(guard, "Mesh"), Value::Asset("XIIIPersos.SpadsM".into()));
    assert!(tick_until(&mut vm, pawn, Some(far), "\"Don't move!\"", 10), "{}", all(&vm));
    assert_eq!(count(&vm, "message \"Someone was hiding behind the hut.\""), 1, "{}", all(&vm));
    assert_eq!(vm.mod_script_team(guard), Some(openxiii_mods::behaviour::Team::Neutral));
    assert!(tick_until(&mut vm, pawn, Some(far), "the guard attacks", 12), "{}", all(&vm));
    assert_eq!(vm.mod_script_team(guard), Some(openxiii_mods::behaviour::Team::Enemy));
    // He dies: the death handler.
    vm.set_prop(guard, "Health", Value::Int(0));
    assert!(tick_until(&mut vm, pawn, Some(far), "message \"The guard is down.\"", 2), "{}", all(&vm));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
    // A checkpoint loads the map again: the scripts start again, from their files.
    let n = count(&vm, "the scene is ready");
    let out = vm.mod_console(pc, "modreload").unwrap();
    assert!(out.contains("1 script(s) started"), "{out}");
    assert!(tick_until(&mut vm, pawn, Some(far), "the scene is ready", 2) && count(&vm, "the scene is ready") == n + 1, "{}", all(&vm));
}

/// Every function of the API on Plage01.
#[test]
fn api_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.script(
        "a",
        "api.lua",
        r#"
        local function p(t) return string.format("{%d, %d, %d}", t[1], t[2], t[3]) end
        on("start", function(pl)
          log("start player", pl == player(), #players(), players()[1] == pl)
          local pam = find("Pam")
          log("find PawnName", pam ~= nil, pam and pam.name, pam and pam.class)
          log("find name", find(pam.name) == pam, "find tag", find(pam.tag) ~= nil, "find nobody", find("Nobody") == nil)
          log("health", pl.health > 0, "alive", pl.alive)
          pl.health = 77
          log("health now", pl.health)
          local at = pl.position
          log("position", p(at))
          pam.yaw = 90
          log("yaw", math.floor(pam.yaw + 0.5))
          log("team before", tostring(pam.team))
          pam.team = "friend"
          log("team after", pam.team)
          local ok, err = pcall(function() pl.nothing = 1 end)
          log("unknown field", ok, type(err), err:find("api.lua:18: an actor has no field", 1, true) ~= nil)
          local med = spawn{class = "XIII.MedPick", position = {at[1] + 120, at[2], at[3]}}
          log("spawned", med ~= nil, med and med.class)
          log("remove", remove(med), med.alive, remove(pl))
          say(pam, "A line of my own.")
          message("Hello from a script.")
        end)
        on("event", "ScriptEvent", function(pl) log("event heard", pl == player()) end)
        on("key", "K", function(pl) log("key pressed", pl == player()) end)
        on("death", "Pam", function(who) log("death of", who.name, who.alive) end)
        "#,
    );
    let toml = folder.0.join("a").join("mod.toml");
    std::fs::write(&toml, "[mod]\nname = \"A\"\nversion = \"1\"\n").unwrap();
    let (mut vm, pc, pawn) = plage01(&game, &toml, folder.mods());
    assert!(tick_until(&mut vm, pawn, None, "message \"Hello from a script.\"", 3), "{}", all(&vm));
    let text = all(&vm);
    for want in [
        "start player true 1 true",
        "find PawnName true",
        "find name true find tag true find nobody true",
        "health true alive true",
        "health now 77",
        "yaw 90",
        "team before nil",
        "team after friend",
        "unknown field false string true",
        "spawned true XIII.MedPick",
        "remove true false false",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    assert!(text.contains("warning: Plage01: remove actor") && text.contains("a player cannot be removed"), "{text}");
    assert_eq!(vm.prop(pawn, "Health").as_int(), 77);
    // The position is the point on the ground, as modpos gives it.
    let [x, y, z] = vm.native_value(pawn, Field::Location).as_vector();
    let ground = z - vm.native_prop(pawn, Field::CollisionHeight).as_float();
    assert!(text.contains(&format!("position {{{}, {}, {}}}", x.round(), y.round(), ground.round())), "{text}");
    let pam = vm.mod_script_resolve(&crate::mod_script::Who::Name("Pam".into())).unwrap();
    assert_eq!(vm.mod_script_team(pam), Some(openxiii_mods::behaviour::Team::Friend));
    // say goes through the mods' dialogue lines.
    assert!(tick_until(&mut vm, pawn, None, "\"A line of my own.\"", 5), "{}", all(&vm));
    // An event of the map.
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("ScriptEvent"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    assert!(tick_until(&mut vm, pawn, None, "event heard true", 2), "{}", all(&vm));
    // A key: known to the game window, run by modkey too.
    assert!(vm.mod_action_keys().iter().any(|k| k == "K"));
    assert!(vm.mod_key(pc, "K"));
    assert!(!vm.mod_key(pc, "F12"), "no handler on F12");
    assert!(tick_until(&mut vm, pawn, None, "key pressed true", 1), "{}", all(&vm));
    // A death.
    vm.set_prop(pam, "Health", Value::Int(0));
    let name = vm.mod_actor_name(pam);
    assert!(tick_until(&mut vm, pawn, None, &format!("death of {name} false"), 1), "{}", all(&vm));
    assert!(
        !tick_until(&mut vm, pawn, None, "never", 1) && count(&vm, "death of") == 1,
        "once
{}",
        all(&vm)
    );
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

// ---------------------------------------------------------------------------------------------
// Stage 2, without the game.

/// What a mod changes in the libraries stays its own; the others keep the originals.
#[test]
fn each_mod_has_its_own_libraries() {
    let folder = Folder::new();
    folder.script("a", "1.lua", "string.upper = nil math.pi = 3 table.insert = nil coroutine.wrap = nil log('a changed', string.upper == nil, math.pi)\n");
    folder.script("b", "1.lua", "log('b sees', type(string.upper), math.pi > 3.14, type(table.insert), type(coroutine.wrap), ('x'):upper())\n");
    let vm = started(&folder);
    let text = all(&vm);
    assert!(text.contains(": a changed true 3"), "{text}");
    assert!(text.contains(": b sees function true function function X"), "{text}");
    // Negative control: within one mod, the change is seen.
    let folder = Folder::new();
    folder.script("a", "1.lua", "math.pi = 3");
    folder.script("a", "2.lua", "log('same mod', math.pi)");
    let vm = started(&folder);
    assert!(all(&vm).contains(": same mod 3"), "{}", all(&vm));
}

/// random() has its own generator: the game's random stream (Vm::seed) never moves.
#[test]
fn random_is_the_scripts_own() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        local ok = true
        for i = 1, 200 do
          local f, n, m = random(), random(6), random(-3, 3)
          ok = ok and f >= 0 and f < 1 and n >= 1 and n <= 6 and m >= -3 and m <= 3 and math.type(n) == "integer"
        end
        log("ranges", ok, (pcall(random, 5, 1)))
        "#,
    );
    let mut vm = Vm::new();
    vm.set_mod_scripts_here(folder.mods());
    let seed = vm.seed;
    vm.mod_scripts_start();
    assert!(all(&vm).contains(": ranges true false"), "{}", all(&vm));
    assert_eq!(vm.seed, seed, "the game's random stream did not move");
}

#[test]
fn wait_until_a_condition_or_a_time() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        n = 0
        every(0.5, function() n = n + 1 end)
        after(0, function()
          log("until", wait_until(function() return n >= 3 end))
          log("timeout", wait_until(function() return false end, 1))
        end)
        "#,
    );
    let mut vm = started(&folder);
    frames(&mut vm, 0.0, 4.0);
    let text = all(&vm);
    assert!(text.contains(": until true") && text.contains(": timeout false"), "{text}");
}

/// `save`: kept from one map to the next, brought back with a checkpoint, empty from the menu,
/// in a file of its own next to the profile (the profile is never written by it).
#[test]
fn save_is_kept_with_the_game() {
    let folder = Folder::new();
    folder.script("a", "1.lua", "save.n = (save.n or 0) + 1 save.list = {1, 2} log('n is', save.n)");
    folder.script("b", "1.lua", "log('b has', save.n == nil)");
    let mut vm = Vm::new();
    vm.set_mod_scripts_here(folder.mods());
    vm.engine.save = Some(Default::default());
    vm.url.map = "Plage01".into();
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 1"), 1, "{}", all(&vm));
    assert_eq!(count(&vm, ": b has true"), 1, "each mod has its own save");
    // The next map (the same engine): the memory goes on.
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 2"), 1, "{}", all(&vm));
    // A checkpoint is saved here (n = 2); two more maps; loading the checkpoint brings 2 back.
    vm.mod_script_save_checkpoint(true);
    vm.mod_scripts_start();
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 4"), 1, "{}", all(&vm));
    vm.mod_script_save_load(false);
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 3"), 2, "back to 2, then + 1\n{}", all(&vm));
    // A slot of the profile: the memory goes to a file of its own; the profile is not written.
    let profile = folder.0.join("Player.prf");
    vm.mod_script_save_checkpoint(true);
    vm.mod_script_save_write_slot(profile.to_str().unwrap(), 2, true);
    let side = folder.0.join("Player.prf.mods-slot2.json");
    assert!(side.is_file() && !profile.exists(), "only the side file is written");
    assert!(std::fs::read_to_string(&side).unwrap().contains("\\\"n\\\":3"), "{}", std::fs::read_to_string(&side).unwrap());
    // From the menu the memory starts empty; reading the slot brings it back.
    vm.url.map = "MapMenu".into();
    vm.mod_scripts_start();
    vm.url.map = "Plage01".into();
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 2"), 2, "new game: empty again, 1 at the menu, 2 now\n{}", all(&vm));
    vm.mod_script_save_read_slot(profile.to_str().unwrap(), 2);
    vm.mod_script_save_load(false);
    vm.mod_scripts_start();
    assert_eq!(count(&vm, ": n is 4"), 2, "{}", all(&vm));
    vm.mod_script_save_erase_slot(profile.to_str().unwrap(), 2);
    assert!(!side.exists());
    // A value that cannot be kept: one warning, the memory stays as it was.
    folder.script("a", "1.lua", "save.f = function() end");
    vm.mod_scripts_start();
    assert!(all(&vm).contains("cannot be kept"), "{}", all(&vm));
}

/// The new events and change are checked when they are written.
#[test]
fn methods_and_new_events_are_checked() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        log("global go_to", go_to == nil)
        local ok, e = pcall(on, "enter", {0, 0, 0})
        log("enter needs fn", ok, e:find("last argument") ~= nil)
        log("events", math.type(on("damage", "Nobody", function() end)), math.type(on("pickup", function() end)), math.type(on("enter", {0, 0, 0}, 100, function() end)))
        local ok2, e2 = pcall(change, "Nobody", {health = 10})
        log("change nobody", ok2, e2:find("nobody named Nobody") ~= nil)
        local ok3, e3 = pcall(change, "Nobody", {colour = "red"})
        log("change bad key", ok3, e3:find("colour") ~= nil)
        "#,
    );
    let vm = started(&folder);
    let text = all(&vm);
    for want in
        [": global go_to true", ": enter needs fn false true", ": events integer integer integer", ": change nobody false true", ": change bad key false true"]
    {
        assert!(text.contains(want), "{want}\n{text}");
    }
}

// ---------------------------------------------------------------------------------------------
// Stage 2 on the real Plage01.

/// A test mod with the [character]s of `toml_text` and the script `lua`, on Plage01.
fn plage01_with(game: &Path, toml_text: &str, lua: &str) -> (Folder, Vm, ObjId, ObjId) {
    let folder = Folder::new();
    folder.script("a", "test.lua", lua);
    let toml = folder.0.join("a").join("mod.toml");
    std::fs::write(&toml, format!("[mod]\nname = \"A\"\nversion = \"1\"\n{toml_text}")).unwrap();
    let (vm, pc, pawn) = plage01(game, &toml, folder.mods());
    (folder, vm, pc, pawn)
}

const SOLDIER: &str = "[character.Raider]\nbase = \"XIDPawn.BaseSoldier\"\nteam = \"neutral\"\nproperties = { Mesh = \"XIIIPersos.SpadsM\" }\n[character.Friend]\nbase = \"XIDCine.Cine2\"\nteam = \"friend\"\nproperties = { Mesh = \"XIIIPersos.CarringtonM\" }\n";

/// Orders to a story character (Janet, Cine2) and to a soldier; change; get, set, call.
#[test]
fn orders_and_expert_door_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    local function near(p, what)
      local at = p.position
      for i = 1, 8 do
        local a = spawn{character = what, position = {at[1] + 150 * math.cos(i), at[2] + 150 * math.sin(i), at[3]}}
        if a then return a end
      end
    end
    on("start", function(p)
      local pam = find("Pam")
      local ok0, e0 = pcall(pam.go_to, pam, p)
      log("pam waits", ok0 or tostring(e0):find("own scene") ~= nil)
      local friend = near(p, "Friend")
      wait(1)
      local at = friend.position


      log("go_to", friend:go_to(p.position))
      log("look_at", friend:look_at(p), "play", friend:play("WaitNeutre", true))
      local r = spawn{character = "Raider", position = {-470, -860, 1190}}
      log("soldier", r ~= nil)
      local ok, e = pcall(r.go_to, r, p)
      log("soldier go_to", ok or tostring(e):find("no way") ~= nil)
      log("attack", r:attack(p), r.team)
      log("give", p:give("Shotgun"), p.weapon ~= nil)
      log("get", pam:get("Health") == pam.health, type(pam:get("Mesh")))
      pam:set("GroundSpeed", 321)
      log("set", pam:get("GroundSpeed"))
      local ok2, e2 = pcall(pam.set, pam, "Health", "lots")
      log("set wrong type", ok2, e2:find("Health") ~= nil and e2:find("int") ~= nil)
      local ok3, e3 = pcall(pam.get, pam, "NoSuchThing")
      log("get unknown", ok3, e3:find("no property NoSuchThing") ~= nil)
      log("call", pam:call("IsA", "Pawn"), pam:call("IsA", "Weapon"))
      local ok4, e4 = pcall(pam.call, pam, "IsA", 12)
      log("call wrong type", ok4, e4:find("parameter 1") ~= nil)
      log("class_get", class_get("XIII.Beretta", "ReloadCount") > 0)
      log("change", change("Pam", {look = "Jones", health = 300, team = "enemy"}), pam.health, pam.team)
      objective("Hold the beach")
      objective_done("Hold the beach")
      trigger("ScriptEvent2")
      wait(1)
      -- The game's controller walks it there (CineController2: bMoving, Target, Steering's
      -- acceleration); the walk itself needs the level's collision, not loaded by these tests.
      local c = friend:get("Controller")
      local acc = friend:get("Acceleration")
      log("moving", c:get("bMoving"), tostring(c:get("Target")), math.abs(acc[1]) + math.abs(acc[2]) > 0)
      friend:stop()
      log("follow", friend:follow(p))
      log("kill", r:kill(), r.alive)
    end)
    on("event", "ScriptEvent2", function(p) log("triggered by player", p == player()) end)
    on("damage", "Pam", function(a, n) log("damage", a.name, n) end)
    on("pickup", function(p, class) log("pickup", class) end)
    on("start", function(p) local h = p.position on("enter", h, 100, function(q) log("entered", q == player()) end) end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, SOLDIER, lua);
    assert!(tick_until(&mut vm, pawn, None, ": kill true false", 5), "{}", all(&vm));
    let text = all(&vm);
    for want in [
        ": pam waits true",
        ": go_to true",
        ": follow true",
        ": look_at true play true",
        ": soldier true",
        ": soldier go_to true",
        ": attack true enemy",
        ": give true true",
        ": get true string",
        ": set 321",
        ": set wrong type false true",
        ": get unknown false true",
        ": call true false",
        ": call wrong type false true",
        ": class_get true",
        ": change 1 300 enemy",
        "objective \"Hold the beach\"",
        "objective done \"Hold the beach\"",
        ": triggered by player true",
        ": pickup XIII.FusilPompe",
        ": moving true actor ",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    let pam = vm.mod_script_resolve(&crate::mod_script::Who::Name("Pam".into())).unwrap();
    assert_eq!(vm.prop(pam, "Mesh"), Value::Asset("XIIIPersos.JonesM".into()));
    // Damage, and a zone entered.
    vm.set_prop(pam, "Health", Value::Int(250));
    let damage = format!(": damage {} 50", vm.mod_actor_name(pam));
    assert!(tick_until(&mut vm, pawn, None, &damage, 1), "{}", all(&vm));
    // The zone is where the player stood at the start: entered at once.
    assert_eq!(count(&vm, ": entered true"), 1, "{}", all(&vm));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

/// The instigator of an event: the player who caused it.
#[test]
fn event_instigator_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = "on('event', 'ScriptEvent3', function(p) log('by', p and p.name) end)";
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    vm.tick(1.0 / 30.0);
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("ScriptEvent3"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    let name = vm.mod_actor_name(pawn);
    assert!(tick_until(&mut vm, pawn, None, &format!(": by {name}"), 1), "{}", all(&vm));
}

/// next_level and fail end the mission through the game's EndGame.
#[test]
fn next_level_and_fail_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let (_f, mut vm, _pc, pawn) = plage01_with(&game, "", "on('start', function() next_level('Banque01') end)");
    assert!(tick_until(&mut vm, pawn, None, "next_level Banque01", 2), "{}", all(&vm));
    let game_info = vm.prop(vm.level.unwrap(), "Game").as_object().unwrap();
    let info = vm.prop(game_info, "MapInfo").as_object().unwrap();
    assert_eq!(vm.prop(info, "NextMapLevelWithUnr").as_name(), "Banque01");
    assert!(vm.prop(game_info, "bGameEnded").as_bool(), "{}", all(&vm));
    let (_f, mut vm, _pc, pawn) = plage01_with(&game, "", "on('start', function() fail('The guard escaped.') end)");
    assert!(tick_until(&mut vm, pawn, None, "fail \"The guard escaped.\"", 2), "{}", all(&vm));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

/// The second example: the mission starts with the end of the opening scene, three waves of
/// soldiers come, the beach is held, `save` counts.
#[test]
fn example_mission_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = mission_dir();
    let mods = vec![ScriptMod { name: "beach-mission".into(), folder: dir.clone() }];
    let (mut vm, _pc, pawn) = plage01(&game, &dir.join("mod.toml"), mods);
    vm.engine.save = Some(Default::default());
    vm.tick(1.0 / 30.0);
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    assert!(tick_until(&mut vm, pawn, None, "objective \"Hold the beach\"", 2), "{}", all(&vm));
    for wave in 1..=3 {
        assert!(tick_until(&mut vm, pawn, None, &format!("message \"Wave {wave} of 3\""), 15), "{}", all(&vm));
        // The player holds: the raiders of the wave go down (kept alive himself).
        for _ in 0..30 {
            vm.set_prop(pawn, "Health", Value::Int(100));
            vm.tick(1.0 / 30.0);
        }
        let raiders: Vec<ObjId> = vm.mods.live.keys().copied().filter(|&a| vm.mod_alive(a)).collect();
        assert!(!raiders.is_empty(), "wave {wave}\n{}", all(&vm));
        for r in raiders {
            vm.set_prop(r, "Health", Value::Int(0));
        }
    }
    assert!(tick_until(&mut vm, pawn, None, "The beach is yours (try 1).", 15), "{}", all(&vm));
    assert_eq!(count(&vm, "objective done \"Hold the beach\""), 1);
    let save = vm.engine.mod_script_save.get("beach-mission").cloned().unwrap_or_default();
    assert!(save.contains("\"best\":3") && save.contains("\"tries\":1"), "{save}");
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

/// A network guest: the goals, music and sounds of the host's scripts arrive as the game's
/// own ClientMessage, and the guest's game shows or plays them itself.
#[test]
fn guest_gets_goals_music_and_sounds_on_plage01() {
    use crate::ipdrv::{NetDriver, NetDriverCfg, Sock};
    use crate::mod_script_game::guest_line;
    use crate::net::{ConnState, Connection};
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let (_folder, mut vm, pc, _pawn) = plage01_with(&game, "", "log('host scripts')");
    // From now on this game is a guest of a host.
    let mut d = NetDriver::new(Sock(None, 0), 0, true, NetDriverCfg::default());
    d.server_conn = Some(crate::net_game::NetConn::new(Connection::new("127.0.0.1:7777".parse().unwrap(), ConnState::Open, 0.0, 0)));
    vm.ipdrv.net_driver = Some(d);
    let f = vm.find_function(pc, "ClientMessage", false).unwrap();
    for (kind, n, text) in
        [("Goal", 0, "Hold the beach"), ("Goal", 1, "Hold the beach"), ("Sound", 0, "XIIISound.Vehicles__Sanc3Helico.Sanc3Helico__hSancHelico1Alarm")]
    {
        let args = vec![Some(Value::Str(guest_line(kind, n, text))), Some(Value::Name("None".into()))];
        assert!(vm.mod_script_guest_call(f, pc, &args), "{kind}");
    }
    let text = all(&vm);
    assert_eq!(count(&vm, "guest: goal Hold the beach"), 2, "{text}");
    assert_eq!(count(&vm, "guest: sound XIIISound.Vehicles__Sanc3Helico"), 1, "{text}");
    // Another ClientMessage is left to the game.
    let other = vec![Some(Value::Str("Hello".into())), Some(Value::Name("None".into()))];
    assert!(!vm.mod_script_guest_call(f, pc, &other));
    assert_eq!(count(&vm, "warning: guest"), 0, "{text}");
}

// ---------------------------------------------------------------------------------------------
// Stage 3: behaviours, routes, scenes.

/// A behaviour runs as long as its character lives; behave(nil) stops it. Patrol, a
/// soldier's go_to (the AI's own VaVers), characters_near, and a scene's camera.
#[test]
fn behaviours_routes_and_scenes_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    local function near(p, what, behaviour)
      local at = p.position
      for i = 1, 8 do
        local a = spawn{character = what, position = {at[1] + 150 * math.cos(i), at[2] + 150 * math.sin(i), at[3]}, behaviour = behaviour}
        if a then return a end
      end
    end
    behaviour("Counter", function(self)
      local n = 0
      while true do n = n + 1 log("behaving", self.name, n) wait(0.5) end
    end)
    on("start", function(p)
      local friend = near(p, "Friend", "Counter")
      local r = near(p, "Raider")
      log("near me", #characters_near(p, 400) >= 2, characters_near(p, 400)[1] ~= nil)
      wait(2)
      friend:behave(nil)
      log("stopped")
      wait(1)
      local at = friend.position
      log("patrol", friend:patrol{{at[1] + 100, at[2], at[3]}, {at[1] - 100, at[2], at[3]}})
      local c = friend:get("Controller")
      log("patrolling", c:get("bMoving"), tostring(c:get("Target")))
      local ok, e = pcall(r.go_to, r, {r.position[1] + 60, r.position[2], r.position[3]})
      log("soldier go_to", ok and r:get("Controller"):call("IsInState", "VaVers") or (tostring(e):find("no way") ~= nil))
      r:behave(function(self) wait(0.5) log("own behaviour", self.name) end)
      wait(1)
      camera({at[1], at[2], at[3] + 70}, p, 1)
      log("camera", p:get("Controller"):call("IsInState", "NoControl"), tostring(p:get("Controller"):get("ViewTarget")) ~= tostring(p))
      wait(2)
      log("camera back", p:get("Controller"):call("IsInState", "PlayerWalking"), p:get("Controller"):get("ViewTarget") == p)
      local ok5, e5 = pcall(camera, {at[1], at[2], at[3] - 5000}, p)
      log("camera outside", ok5, tostring(e5):find("outside the level") ~= nil)
      r:kill()
      log("end")
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, SOLDIER, lua);
    assert!(tick_until(&mut vm, pawn, None, ": end", 15), "{}", all(&vm));
    let text = all(&vm);
    for want in [
        ": near me true true",
        ": patrol true",
        ": patrolling true actor ",
        ": soldier go_to true",
        ": own behaviour",
        ": camera true true",
        ": camera back true true",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    // The behaviour ran a few times, and not after behave(nil).
    let stopped = vm.mod_messages.iter().position(|m| m.ends_with(": stopped")).unwrap();
    let runs = vm.mod_messages[..stopped].iter().filter(|m| m.contains(": behaving ")).count();
    assert!(runs >= 3, "{text}");
    assert!(!vm.mod_messages[stopped..].iter().any(|m| m.contains(": behaving ")), "{text}");
    assert_eq!(count(&vm, "warning: script"), 0, "{text}");
}

/// The bodyguard example: he comes after the opening scene and follows the player.
#[test]
fn example_bodyguard_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mods/bodyguard");
    let mods = vec![ScriptMod { name: "bodyguard".into(), folder: dir.clone() }];
    let (mut vm, _pc, pawn) = plage01(&game, &dir.join("mod.toml"), mods);
    vm.tick(1.0 / 30.0);
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("Objectif91"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    assert!(tick_until(&mut vm, pawn, None, "message \"A bodyguard has joined you.\"", 2), "{}", all(&vm));
    tick_until(&mut vm, pawn, None, "never", 3);
    let guard = vm.mod_script_resolve(&crate::mod_script::Who::Name("Bodyguard".into())).expect("the bodyguard");
    assert!(vm.mod_alive(guard));
    assert_eq!(vm.mod_script_team(guard), Some(openxiii_mods::behaviour::Team::Friend));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
}

// ---------------------------------------------------------------------------------------------
// Stage 4: maps of the mods.

/// A mod's map (here a copy of Plage01 under another name, in a mod's maps folder): found
/// after the game's own maps, played with the mod's script; a mod's file named as a game's
/// file leaves the game's in place with a warning.
#[test]
fn a_mod_brings_a_map() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let folder = Folder::new();
    folder.script("a", "level.lua", "campaign{'Plage01', 'ModBeach'} on('start', function(p) log('on the map', map()) end)");
    let maps = folder.0.join("a").join("maps");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::copy(game.join("Maps").join("Plage01.unr"), maps.join("ModBeach.unr")).unwrap();
    std::fs::write(maps.join("Plage01.unr"), "not the game's").unwrap();
    let files: Vec<(String, PathBuf)> = openxiii_mods::own_maps::find(&folder.0.join("a"), "a").into_iter().map(|p| (p.name, p.path)).collect();
    assert_eq!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["ModBeach", "Plage01"]);
    // The mod's Plage01 is named as the game's: a warning (checked here only: the files
    // registered below serve the whole test run).
    let warnings = crate::mod_maps::map_collisions(&game, &files);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("has the name of a file of the game"));
    let files: Vec<(String, PathBuf)> = files.into_iter().filter(|f| f.0 == "ModBeach").collect();
    crate::mod_maps::set_mod_map_files(files.clone());
    openxiii_package::library::set_extra_files(files);
    assert_eq!(openxiii_package::library::map_path(&game, "ModBeach"), maps.join("ModBeach.unr"));
    assert_eq!(openxiii_package::library::map_path(&game, "Plage01.unr"), game.join("Maps").join("Plage01.unr"), "the game's map first");
    assert_eq!(crate::mod_maps::mod_maps().len(), 1);
    let toml = folder.0.join("a").join("mod.toml");
    std::fs::write(
        &toml,
        "[mod]
name = \"A\"
version = \"1\"
",
    )
    .unwrap();
    let (mut vm, _pc, pawn) = level(&game, "ModBeach", &toml, folder.mods());
    assert!(tick_until(&mut vm, pawn, None, ": on the map ModBeach", 2), "{}", all(&vm));
    // The campaign: from Plage01, next_level() goes to ModBeach (the story's travel).
    let (_f2, mut vm2, _pc2, pawn2) = plage01_with(&game, "", "campaign{'Plage01', 'ModBeach'} on('start', function() next_level() end)");
    assert!(tick_until(&mut vm2, pawn2, None, "next_level ModBeach", 2), "{}", all(&vm2));
    let game_info = vm2.prop(vm2.level.unwrap(), "Game").as_object().unwrap();
    let info = vm2.prop(game_info, "MapInfo").as_object().unwrap();
    assert_eq!(vm2.prop(info, "NextMapLevelWithUnr").as_name(), "ModBeach");
}

// ---------------------------------------------------------------------------------------------
// Stage 5: the documentation (docs/modding/script).

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/modding/script")
}

/// The code blocks of language `lang` in the pages of the script documentation: (page, code).
fn doc_blocks(lang: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pages: Vec<PathBuf> =
        std::fs::read_dir(docs_dir()).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
    pages.sort();
    for page in pages {
        let text = std::fs::read_to_string(&page).unwrap().replace("\r\n", "\n");
        let name = page.file_name().unwrap().to_string_lossy().into_owned();
        let mut block: Option<String> = None;
        for line in text.lines() {
            match (&mut block, line.trim_start()) {
                (None, l) if l == format!("```{lang}") => block = Some(String::new()),
                (Some(b), "```") => {
                    out.push((name.clone(), std::mem::take(b)));
                    block = None;
                }
                (Some(b), _) => {
                    b.push_str(line);
                    b.push('\n');
                }
                _ => {}
            }
        }
    }
    out
}

/// The characters the pages use, and a helper: the visitor and a guard come near the player,
/// then the opening scene of the beach ends (Objectif91).
const DOC_TOML: &str = "[character.Visitor]\nbase = \"XIDCine.Cine2\"\nteam = \"friend\"\nproperties = { Mesh = \"XIIIPersos.CarringtonM\", CodeMesh = 16, PawnName = \"Carrington\" }\n[character.Guard]\nbase = \"XIDPawn.BaseSoldier\"\nteam = \"neutral\"\nproperties = { Mesh = \"XIIIPersos.SpadsM\" }\n[character.Raider]\nbase = \"XIDPawn.BaseSoldier\"\nteam = \"enemy\"\nproperties = { Mesh = \"XIIIPersos.SpadsM\" }\n";

const DOC_HELPER: &str = r#"
on("start", function(p)
  local at = p.position
  for i = 1, 8 do
    if spawn{character = "Visitor", position = {at[1] + 150 * math.cos(i), at[2] + 150 * math.sin(i), at[3]}} then break end
  end
  wait(0.5)
  trigger("Objectif91")
end)
"#;

/// Every Lua example of the pages runs on the real beach without an error, and every toml
/// example is read by the game's toml reader without a problem.
#[test]
fn every_example_of_the_docs_runs() {
    let toml_blocks = doc_blocks("toml");
    assert!(toml_blocks.len() >= 5, "{}", toml_blocks.len());
    for (page, code) in &toml_blocks {
        let file = openxiii_mods::defaults::parse_mod_file(code, Path::new(page), "docs");
        assert!(file.problems.is_empty(), "{page}:\n{code}\n{:?}", file.problems);
    }
    let lua_blocks = doc_blocks("lua");
    assert!(lua_blocks.len() >= 10, "{}", lua_blocks.len());
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: the Lua examples are not run");
        return;
    };
    for (page, code) in &lua_blocks {
        let folder = Folder::new();
        folder.script("docs", "0-helper.lua", DOC_HELPER);
        folder.script("docs", "1-example.lua", code);
        let toml = folder.0.join("docs").join("mod.toml");
        std::fs::write(&toml, format!("[mod]\nname = \"Docs\"\nversion = \"1\"\n{DOC_TOML}")).unwrap();
        let (mut vm, _pc, pawn) = plage01(&game, &toml, folder.mods());
        vm.engine.save = Some(Default::default());
        tick_until(&mut vm, pawn, None, "never", 8);
        let text = all(&vm);
        assert_eq!(count(&vm, "warning: script"), 0, "{page}:\n{code}\n{text}");
        assert!(count(&vm, "2 script(s) started") == 1, "{page}\n{text}");
    }
}

/// functions.md is the reference made from the API table.
#[test]
fn the_functions_page_is_up_to_date() {
    let page = std::fs::read_to_string(docs_dir().join("functions.md")).unwrap_or_default().replace("\r\n", "\n");
    assert!(
        page == crate::mod_script::reference_markdown(),
        "docs/modding/script/functions.md is out of date: cargo test -p openxiii-vm --features mods --lib write_the_functions_page -- --ignored"
    );
}

/// Writes functions.md from the API table.
#[test]
#[ignore]
fn write_the_functions_page() {
    std::fs::write(docs_dir().join("functions.md"), crate::mod_script::reference_markdown()).unwrap();
}

// ---------------------------------------------------------------------------------------------
// "when ... then ..." rules given as data (mod_when.rs), for the toml files.

fn text(s: &str) -> crate::mod_when::WhenArg {
    crate::mod_when::WhenArg::Value(crate::mod_when::WhenValue::Text(s.into()))
}

fn rule(
    mod_name: &str,
    event: crate::mod_when::WhenEvent,
    actions: Vec<(&str, Vec<crate::mod_when::WhenArg>)>,
    run: Option<&str>,
    once: bool,
) -> crate::mod_when::WhenRule {
    crate::mod_when::WhenRule {
        mod_name: mod_name.into(),
        map: "Plage01".into(),
        source: format!("{mod_name}/mod.toml line 1"),
        event,
        actions: actions.into_iter().map(|(n, args)| crate::mod_when::WhenAction { name: n.into(), args }).collect(),
        run: run.map(str::to_string),
        once,
        checkpoint: Default::default(),
        control: false,
    }
}

/// Rules of a mod without a scripts folder fire and do each action; `run` calls a function of
/// another mod's script; an unknown action is refused with one line; a timer once.
#[test]
fn when_rules_without_a_script_on_plage01() {
    use crate::mod_when::{WhenArg, WhenEvent, WhenValue};
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let at = WhenValue::List(vec![WhenValue::Int(-470), WhenValue::Int(-860), WhenValue::Int(1190)]);
    let spawn = WhenArg::Value(WhenValue::Table(vec![("character".into(), WhenValue::Text("Visitor".into())), ("position".into(), at)]));
    let rules = vec![
        rule("nolua", WhenEvent::Start, vec![("message", vec![text("rule start")]), ("spawn", vec![spawn])], None, false),
        rule("nolua", WhenEvent::After { seconds: 1.0 }, vec![("log", vec![text("after one second")])], None, false),
        rule("nolua", WhenEvent::Every { seconds: 0.5 }, vec![("log", vec![text("tick once")])], None, true),
        rule(
            "nolua",
            WhenEvent::Event { name: "RuleEvent".into() },
            vec![("objective", vec![text("rule objective")]), ("give", vec![WhenArg::Player, text("Shotgun")]), ("log", vec![text("event done")])],
            None,
            false,
        ),
        rule("nolua", WhenEvent::Death { who: "Visitor".into() }, vec![("message", vec![text("visitor down")]), ("log", vec![WhenArg::Actor])], None, false),
        rule("withlua", WhenEvent::Event { name: "RuleEvent".into() }, vec![], Some("reward"), false),
        rule("nolua", WhenEvent::Start, vec![("explode", vec![])], None, false),
    ];
    let folder = Folder::new();
    folder.script("withlua", "reward.lua", "function reward(p) log('reward', p == player()) end");
    let toml = folder.0.join("withlua").join("mod.toml");
    std::fs::write(&toml, format!("[mod]\nname = \"A\"\nversion = \"1\"\n{DOC_TOML}")).unwrap();
    let file = openxiii_mods::defaults::parse_mod_file(&std::fs::read_to_string(&toml).unwrap(), &toml, "test");
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_behaviour_here(file.actions, file.characters);
    vm.set_mod_scripts_here(folder.mods());
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
    assert!(tick_until(&mut vm, pawn, None, ": after one second", 3), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("Plage01: 6 when rule(s) started"), "{text}");
    assert!(text.contains("unknown action explode"), "{text}");
    assert_eq!(count(&vm, "message \"rule start\""), 1, "{text}");
    assert_eq!(count(&vm, "spawn character Visitor"), 1, "{text}");
    assert_eq!(count(&vm, ": tick once"), 1, "once: one time only\n{text}");
    // The map's event: the actions in order, then the other mod's script function.
    let f = vm.find_function(pawn, "TriggerEvent", false).unwrap();
    let event = Value::Name(crate::value::NameStruct::from_time("RuleEvent"));
    vm.call(f, pawn, vec![Some(event), Some(Value::Object(Some(pawn))), Some(Value::Object(Some(pawn)))]).unwrap();
    assert!(tick_until(&mut vm, pawn, None, ": reward true", 2), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("objective \"rule objective\"") && text.contains("give XIII.FusilPompe") && text.contains(": event done"), "{text}");
    // A death.
    let visitor = vm.mod_script_resolve(&crate::mod_script::Who::Name("Visitor".into())).expect("the visitor");
    vm.set_prop(visitor, "Health", Value::Int(0));
    assert!(tick_until(&mut vm, pawn, None, "message \"visitor down\"", 2), "{}", all(&vm));
    assert!(all(&vm).contains(&format!(": actor {}", vm.mod_actor_name(visitor))), "{}", all(&vm));
    assert_eq!(count(&vm, "warning: script"), 0, "{}", all(&vm));
    // modreload starts the rules again.
    let out = vm.mod_console(pc, "modreload").unwrap();
    assert!(out.contains("script(s) started"), "{out}");
    assert_eq!(count(&vm, "6 when rule(s) started"), 2, "{}", all(&vm));
}

/// No rule runs on a network client.
#[test]
fn when_rules_never_on_a_network_client() {
    use crate::ipdrv::{NetDriver, NetDriverCfg, Sock};
    use crate::mod_when::WhenEvent;
    use crate::net::{ConnState, Connection};
    let rules = vec![rule("nolua", WhenEvent::After { seconds: 0.0 }, vec![("log", vec![text("ran here")])], None, false)];
    let run = |client: bool| {
        let mut vm = Vm::new();
        vm.url.map = "Plage01".into();
        vm.set_mod_when_here(rules.clone());
        if client {
            let mut d = NetDriver::new(Sock(None, 0), 0, true, NetDriverCfg::default());
            d.server_conn = Some(crate::net_game::NetConn::new(Connection::new("127.0.0.1:7777".parse().unwrap(), ConnState::Open, 0.0, 0)));
            vm.ipdrv.net_driver = Some(d);
        }
        vm.mod_scripts_start();
        frames(&mut vm, 0.0, 0.5);
        vm
    };
    let on_client = run(true);
    assert!(!on_client.mod_scripts_running());
    assert_eq!(count(&on_client, "ran here"), 0);
    let on_host = run(false);
    assert_eq!(count(&on_host, ": ran here"), 1, "{}", all(&on_host));
}

/// A death is reported for a character that dies in its very first frame; one already dead
/// before the handler was registered is not.
#[test]
fn death_in_the_first_frame_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    local function near(p, what)
      local at = p.position
      for i = 1, 8 do
        local a = spawn{character = what, position = {at[1] + 150 * math.cos(i), at[2] + 150 * math.sin(i), at[3]}}
        if a then return a end
      end
    end
    on("start", function(p)
      local old = near(p, "Raider")
      old:kill()
      wait(1)
      on("death", "Raider", function(r) log("raider died", r.name) end)
      on("death", "Friend", function(f) log("friend died", f.name) end)
      local f = near(p, "Friend")
      f:kill()
      log("killed", f.name, f.alive)
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, SOLDIER, lua);
    assert!(tick_until(&mut vm, pawn, None, ": friend died", 4), "{}", all(&vm));
    tick_until(&mut vm, pawn, None, "never", 1);
    assert_eq!(count(&vm, ": friend died"), 1, "{}", all(&vm));
    assert_eq!(count(&vm, ": raider died"), 0, "already dead before the handler\n{}", all(&vm));
}

/// The guard dogs example on the cable car station (Hual04a): two guards each get a
/// dog, a new class made from Cine2 with the dog's model, driven by its script without a warning. A character
/// whose [character] gives its own CollisionHeight is put with its own feet on the point
/// (a dog is lower than a man), not with its class's.
#[test]
fn example_guard_dogs_on_hual04a() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/guard-dogs");
    let mods = vec![ScriptMod { name: "guard-dogs".into(), folder: dir.clone() }];
    let (mut vm, _pc, pawn) = level(&game, "Hual04a", &dir.join("mod.toml"), mods);
    assert!(tick_until(&mut vm, pawn, None, "spawn character Dog", 4), "{}", all(&vm));
    tick_until(&mut vm, pawn, None, "never", 3);
    assert_eq!(count(&vm, "spawn character Dog: GuardDog"), 2, "{}", all(&vm));
    assert_eq!(count(&vm, "warning"), 0, "{}", all(&vm));
    let dogs: Vec<ObjId> = vm.actors.iter().copied().filter(|&a| vm.prop_slot(a, "PawnName").is_ok() && matches!(vm.prop(a, "PawnName"), Value::Str(ref s) if s == "Dog")).collect();
    assert_eq!(dogs.len(), 2);
    for &d in &dogs {
        assert!(vm.mod_alive(d));
        assert_eq!(vm.native_prop(d, Field::CollisionHeight).as_float(), 38.0);
        assert_eq!(vm.mod_script_team(d), Some(openxiii_mods::behaviour::Team::Enemy));
    }
    // The spawn point is where the feet are, whatever the character's height.
    let at = [-10484.0f32, -1445.0, 500.0];
    for (what, height) in [(crate::mod_script::SpawnWhat::Character("Dog".into()), 38.0f32), (crate::mod_script::SpawnWhat::Class("XIDCine.Cine2".into()), 0.0)] {
        let a = vm.mod_script_spawn(&what, at, 0.0, None).unwrap();
        let z = vm.native_value(a, Field::Location).as_vector()[2];
        let own = vm.native_prop(a, Field::CollisionHeight).as_float();
        if height > 0.0 {
            assert_eq!(own, height);
        }
        // Negative control: a class alone stands at its class's height (above a dog's).
        assert!((z - (at[2] + own)).abs() < 1.0, "{what}: z {z}, height {own}");
        let _ = vm.destroy_actor(a);
    }
}

/// Every recording of the dog named by the guard dogs example starts a voice when a script
/// plays it by its name (the programs and the single recordings alike), heard at the player.
#[test]
fn guard_dog_recordings_start_voices() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/guard-dogs");
    let (mut vm, _pc, pawn) = level(&game, "Hual04a", &dir.join("mod.toml"), Vec::new());
    vm.tick(1.0 / 30.0);
    let lua = std::fs::read_to_string(dir.join("scripts/dog.lua")).unwrap();
    let mut names: Vec<String> = Vec::new();
    for part in lua.split('"') {
        if part.starts_with("XIIISound.") && !names.iter().any(|n| n == part) {
            names.push(part.to_string());
        }
    }
    assert!(names.len() >= 33, "{names:?}");
    let at = vm.native_value(pawn, Field::Location).as_vector();
    let voices = |vm: &Vm| -> Vec<_> { vm.audio.as_ref().map(|h| h.engine.voices.iter().flatten().map(|v| (v.res, v.wave)).collect()).unwrap_or_default() };
    let mut silent = Vec::new();
    for name in &names {
        // Whatever played before has ended.
        for _ in 0..60 {
            vm.audio.as_mut().unwrap().update_micro(0, at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
            vm.audio_tick(0.05);
        }
        let before = voices(&vm);
        vm.mod_play_sound(pawn, name, "test").unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut started = false;
        for _ in 0..20 {
            vm.audio.as_mut().unwrap().update_micro(0, at, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
            vm.audio_tick(0.05);
            started |= voices(&vm).iter().any(|v| !before.contains(v));
        }
        if !started {
            silent.push(name.clone());
        }
    }
    assert!(silent.is_empty(), "no voice for {silent:?}");
    // A dog's voice is heard from far: 50 metres away (4550 units), where a story character of
    // the game's own class is not (negative control: 1392 units is as far as anyone is heard).
    let far = [at[0] + 4550.0, at[1], at[2]];
    for (what, heard) in [(crate::mod_script::SpawnWhat::Character("Dog".into()), true), (crate::mod_script::SpawnWhat::Class("XIDCine.Cine2".into()), false)] {
        let source = vm.mod_script_spawn(&what, [at[0], at[1], at[2] + 200.0], 0.0, None).unwrap();
        for _ in 0..60 {
            vm.audio.as_mut().unwrap().update_micro(0, far, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
            vm.audio_tick(0.05);
        }
        let before = voices(&vm);
        vm.mod_play_sound(source, "XIIISound.PNJ.DogYelp2", "test").unwrap();
        let mut started = false;
        for _ in 0..20 {
            vm.audio.as_mut().unwrap().update_micro(0, far, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
            vm.audio_tick(0.05);
            started |= voices(&vm).iter().any(|v| !before.contains(v));
        }
        assert_eq!(started, heard, "{what} at 50 m");
        let _ = vm.destroy_actor(source);
    }
}

/// time(), trace(), effect{} and a property of an object the actor holds ("Emitters[0].Disabled").
#[test]
fn time_trace_effect_and_held_objects_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("start", function(p)
      local t0 = time()
      wait(0.5)
      log("time", time() - t0 >= 0.49 and time() - t0 < 0.6)
      local at = p:get("Location")
      log("trace sky", trace(at, {at[1], at[2], at[3] + 1}) == nil)
      local fx = effect{class = "XIDCine.XIIIFireTorchEmitter", position = {at[1], at[2], at[3] + 50}, facing = {0, 0, -1}, on = p, seconds = 2}
      -- From above, down onto the player (the level's own collision is not loaded by these tests).
      local hit = trace({at[1], at[2], at[3] + 300}, at, fx)
      log("trace down", hit ~= nil and hit.actor == p and hit.position[3] > at[3] and hit.normal[3] > 0.5)
      log("effect", fx ~= nil, fx:get("Base") == p, math.floor(fx:get("Rotation").pitch), fx:get("LifeSpan") > 1.9)
      log("held before", fx:get("Emitters[0].Disabled"))
      fx:set("Emitters[0].Disabled", false)
      log("held after", fx:get("Emitters[0].Disabled"))
      local ok, e = pcall(fx.get, fx, "Tag.Disabled")
      log("held wrong", ok, tostring(e):find("holds no object") ~= nil)
      p.charred = 0.5
      local half = p.charred
      p.charred = 3
      log("charred", half, p.charred)
      p.charred = 0
      local ok2, e2 = pcall(effect, {class = "XIII.Beretta", position = at, colour = 1})
      log("effect unknown", ok2, tostring(e2):find("unknown setting") ~= nil)
      wait(2.5)
      log("effect gone", fx.alive)
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": effect gone", 6), "{}", all(&vm));
    let text = all(&vm);
    for want in [
        ": time true",
        ": trace down true",
        ": trace sky true",
        ": effect true true 49152 true",
        ": held before true",
        ": held after false",
        ": held wrong false true",
        ": charred 0.5 1",
        ": effect unknown false true",
        ": effect gone false",
    ] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }
}

/// actors_near(), char_area() and `charred` on the scenery (mods build): the actors around the
/// player in the hut of Plage01 (Pam lying there, the hut's own meshes), nearest first; a mesh of
/// the scenery takes a charred value, and char_area() checks its radius.
#[test]
fn actors_near_and_charred_scenery_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("start", function(p)
      wait(0.2)
      local all = actors_near(p, 600)
      local meshes = actors_near(p, 600, "StaticMeshActor")
      local d1 = 0
      local sorted = true
      for _, a in ipairs(all) do
        local l = a:get("Location")
        local q = p:get("Location")
        local d = math.sqrt((l[1] - q[1]) ^ 2 + (l[2] - q[2]) ^ 2 + (l[3] - q[3]) ^ 2)
        if d < d1 - 0.01 then sorted = false end
        d1 = d
        if a == p then sorted = false end
      end
      local only = #meshes > 0
      for _, a in ipairs(meshes) do if not a:call("IsA", "StaticMeshActor") then only = false end end
      log("near", #all > #meshes, sorted, only)
      local pam = find("Cine0")
      local found = false
      for _, a in ipairs(all) do if a == pam then found = true end end
      log("pam near", found)
      meshes[1].charred = 0.75
      log("mesh charred", meshes[1].charred, meshes[1].name)
      char_area(p:get("Location"), 300)
      char_area(p:get("Location"), 300, 0.5)
      local ok, e = pcall(char_area, p:get("Location"), -1)
      log("char_area wrong", ok, tostring(e):find("radius") ~= nil)
      log("done")
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": done", 4), "{}", all(&vm));
    let text = all(&vm);
    for want in [": near true true true", ": pam near true", ": char_area wrong false true"] {
        assert!(text.contains(want), "missing {want:?} in:\n{text}");
    }
    assert!(text.contains(": mesh charred 0.75 "), "no charred mesh in:\n{text}");
}

/// pin_to() (mods build): an effect pinned to the gun in the player's hands (its third-person
/// actor) is kept at that point of the gun as the player moves; let go (pin_to(nil)) it stays
/// where it was (negative control). A character cannot be pinned.
#[test]
fn pin_to_keeps_an_effect_on_the_gun_in_hand() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("start", function(p)
      p:give("XIII.M16")
      p:get("Controller"):call("SwitchWeapon", 11)
      wait(1.5)
      local h = p:get("Weapon"):get("ThirdPersonActor")
      local fx = effect{class = "XIDCine.XIIIFireTorchEmitter", position = p.position}
      local function off()
        local a, b, r = fx:get("Location"), h:get("Location"), h:get("Rotation")
        local pi, yi = r.pitch * math.pi / 32768, r.yaw * math.pi / 32768
        local f = {math.cos(pi) * math.cos(yi), math.cos(pi) * math.sin(yi), math.sin(pi)}
        local d = {a[1] - b[1], a[2] - b[2], a[3] - b[3]}
        return math.sqrt(d[1] ^ 2 + d[2] ^ 2 + d[3] ^ 2), d[1] * f[1] + d[2] * f[2] + d[3] * f[3]
      end
      fx:pin_to(h, {50, 0, 0})
      wait(0.3)
      log(string.format("pin on %.1f %.1f", off()))
      local before = fx:get("Location")
      log("pin move")
      wait(1)
      local after = fx:get("Location")
      log(string.format("pin moved %.0f %.1f %.1f", math.abs(after[1] - before[1]) + math.abs(after[2] - before[2]), off()))
      fx:pin_to(nil)
      local kept = fx:get("Location")
      log("pin free")
      wait(1)
      local now = fx:get("Location")
      log(string.format("pin left %.1f", math.abs(now[1] - kept[1]) + math.abs(now[2] - kept[2])))
      log("pin character", pcall(function() p:pin_to(h) end))
      log("pin done")
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": pin move", 4), "{}", all(&vm));
    let at = vm.prop(pawn, "Location").as_vector();
    put(&mut vm, pawn, [at[0] + 60.0, at[1], at[2]]);
    assert!(tick_until(&mut vm, pawn, None, ": pin free", 3), "{}", all(&vm));
    let at = vm.prop(pawn, "Location").as_vector();
    put(&mut vm, pawn, [at[0] - 60.0, at[1], at[2]]);
    assert!(tick_until(&mut vm, pawn, None, ": pin done", 3), "{}", all(&vm));
    let text = all(&vm);
    let line = |what: &str| text.lines().find(|l| l.contains(what)).map(|l| l[l.find(what).unwrap() + what.len()..].trim().to_string()).unwrap_or_default();
    let on: Vec<f32> = line(": pin on").split(' ').filter_map(|x| x.parse().ok()).collect();
    assert!(on.len() == 2 && (on[0] - 50.0).abs() < 0.5 && (on[1] - 50.0).abs() < 0.5, "pinned 50 ahead of the gun: {on:?}\n{text}");
    let moved: Vec<f32> = line(": pin moved").split(' ').filter_map(|x| x.parse().ok()).collect();
    assert!(moved.len() == 3 && moved[0] > 5.0 && (moved[1] - 50.0).abs() < 0.5, "it followed the gun: {moved:?}\n{text}");
    let left: f32 = line(": pin left").parse().unwrap_or(-1.0);
    assert!(left == 0.0, "let go, it stays: {left}\n{text}");
    assert!(text.contains(": pin character false"), "{text}");
}

// ---------------------------------------------------------------------------------------------
// More functions (mod_script_more.rs, mod_script_hud.rs), without the game.

/// hud_text/counter/timer/bar/image/line make items of the screen, in order; hud_update changes
/// one; hud_remove takes one or all away; a wrong setting is an error with the line. Negative
/// control: an id never made is not changed (false).
#[test]
fn hud_items_are_made_changed_and_removed() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        log("ids", hud_text{id = "goal", text = "Find the key", x = 0.5, y = 0.1, align = "centre", colour = "yellow"},
          hud_counter{id = "kills", label = "Enemies: ", value = 2, x = 0.05, y = 0.9},
          hud_timer{label = "Bomb: ", seconds = 90, x = 0.8, y = 0.05},
          hud_bar{id = "boss", x = 0.3, y = 0.05, w = 0.4, h = 0.02, value = 0.5, colour = {255, 0, 0}, back = "black"},
          hud_line{id = "cross", from = {0.45, 0.5}, to = {0.55, 0.5}})
        log("update", hud_update("kills", {value = 3}), hud_update("nothing", {value = 1}))
        log("wrong", pcall(hud_text, {id = "x", text = "a", nothing = 1}))
        log("image", pcall(hud_image, {id = "i"}))
        log("remove", hud_remove("cross"), hud_remove("cross"))
        "#,
    );
    let vm = started(&folder);
    let text = all(&vm);
    assert!(text.contains(": ids goal kills hud1 boss cross"), "{text}");
    assert!(text.contains(": update true false"), "{text}");
    assert!(text.contains(": wrong false") && text.contains("unknown setting nothing"), "{text}");
    assert!(text.contains(": image false") && text.contains("give texture"), "{text}");
    assert!(text.contains(": remove true false"), "{text}");
    let items = &vm.mods.hud.items;
    assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["goal", "kills", "hud1", "boss"]);
    assert_eq!(items[0].colour, [255, 220, 0, 255]);
    assert_eq!(items[0].align, 1);
    assert_eq!(items[1].value, 3.0);
    assert_eq!(items[3].back, Some([0, 0, 0, 255]));
    assert!(vm.mods.hud.loading.items.is_empty(), "nothing on the loading screen");
    // A new map starts without them.
    let mut vm = vm;
    vm.mod_scripts_start();
    assert_eq!(vm.mods.hud.items.len(), 4, "the script made them again");
    let folder2 = Folder::new();
    folder2.script("a", "1.lua", "log('none')");
    vm.set_mod_scripts_here(folder2.mods());
    vm.mod_scripts_start();
    assert!(vm.mods.hud.items.is_empty());
}

/// The change of map: map_end then loading run (from, to, failed); what loading draws is the
/// loading screen's, not the game's; hold_start keeps it; loaded and loading_key run while it is
/// held; release_start lets it go. Negative controls: without hold_start the start is not held;
/// hold_start outside a loading handler is an error.
#[test]
fn loading_events_hold_and_release_the_start() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        on("map_end", function(m) log("map_end", m) end)
        on("loading", function(from, to, failed)
          log("loading", from, to, failed)
          hud_text{id = "recap", text = "Previously...", x = 0.1, y = 0.2}
          hold_start()
        end)
        on("loaded", function(m) log("loaded", m) hud_text{id = "prompt", text = "Press", x = 0.4, y = 0.9} end)
        on("loading_key", function() log("key") release_start() end)
        log("outside", pcall(hold_start))
        "#,
    );
    let mut vm = started(&folder);
    assert!(all(&vm).contains(": outside false"), "{}", all(&vm));
    assert!({ vm.url.map = "Plage01".into(); vm.mod_loading_begin("Banque01", true) });
    let text = all(&vm);
    let (a, b) = (text.find(": map_end Plage01").expect("map_end"), text.find(": loading Plage01 Banque01 failed").expect("loading"));
    assert!(a < b, "map_end first\n{text}");
    assert!(vm.mods.hud.items.is_empty(), "the game's screen has nothing");
    assert_eq!(vm.mods.hud.loading.items.len(), 1);
    assert!(!vm.mod_loading_released());
    vm.mod_loading_loaded("Banque01");
    assert!(all(&vm).contains(": loaded Banque01"));
    assert_eq!(vm.mods.hud.loading.items.len(), 2);
    assert!(vm.mod_loading_key(), "the key releases it");
    assert_eq!(count(&vm, ": key"), 1);
    assert!(vm.mod_loading_key(), "still released, the handler is not run again");
    assert_eq!(count(&vm, ": key"), 1);
    vm.mod_loading_end();
    assert!(vm.mods.hud.loading.items.is_empty() && vm.mod_loading_released());
    // Negative control: a loading handler that does not hold.
    let folder = Folder::new();
    folder.script("a", "1.lua", "on('loading', function() hud_text{text = 'x'} end)");
    let mut vm = started(&folder);
    assert!(!vm.mod_loading_begin("Banque01", false));
    assert!(vm.mod_loading_released());
    // No scripts: nothing runs, nothing is held.
    let mut plain = Vm::new();
    assert!(!plain.mod_loading_begin("Banque01", false));
}

/// preload() works in the main code and the start handler only; the toml's preload lists of the
/// map come with it, sorted, each once. Negative control: a later handler is refused.
#[test]
fn preload_in_main_code_only() {
    let folder = Folder::new();
    folder.script(
        "a",
        "1.lua",
        r#"
        log("main", preload("XIIIPersos.SpadsM"), pcall(preload, "Bare"))
        after(1, function() log("later", pcall(preload, "XIIIPersos.Other")) end)
        "#,
    );
    let mut vm = Vm::new();
    let source = openxiii_mods::placement::Source { mod_name: "a".into(), file: "a/mod.toml".into(), line: 1 };
    vm.set_mod_loads_here(vec![
        openxiii_mods::content::LoadList { map: None, sound_banks: vec![], packages: vec![], preload: vec!["XIIIPersos.SpadsM".into(), "AAA.B".into()], source: source.clone() },
        openxiii_mods::content::LoadList { map: Some("Elsewhere".into()), sound_banks: vec![], packages: vec![], preload: vec!["Not.Here".into()], source },
    ]);
    vm.set_mod_scripts_here(folder.mods());
    vm.mod_scripts_start();
    frames(&mut vm, 0.0, 1.5);
    let text = all(&vm);
    assert!(text.contains(": main true false"), "{text}");
    assert!(text.contains(": later false") && text.contains("only in the script's main code"), "{text}");
    assert_eq!(vm.mod_precache_roots(), ["AAA.B", "XIIIPersos.SpadsM"]);
}

/// An asset named in a callback is available at map entry, before the callback runs.
#[test]
fn compiled_scripts_contribute_registered_assets_to_map_loading() {
    let folder = Folder::new();
    folder.script("warmup", "main.lua", r#"
        after(20, function() log("Mod.PreloadCallbackTexture", "Mod.UnknownCallbackAsset") end)
        -- "Mod.PreloadCommentTexture"
    "#);
    let texture = "Mod.PreloadCallbackTexture";
    let comment = "Mod.PreloadCommentTexture";
    for name in [texture, comment] {
        openxiii_assets::mod_textures::set_runtime(name, vec![(1, 1, vec![255; 4])]);
    }
    let mut vm = Vm::new();
    vm.set_mod_scripts_here(folder.mods());
    vm.mod_scripts_start();
    let roots = vm.mod_precache_roots();
    for name in [texture, comment] {
        openxiii_assets::mod_textures::remove_runtime(name);
    }
    assert_eq!(roots, [texture]);
}

/// game(): the mode and language of a game alone (solo, English without a configuration).
#[test]
fn game_info_alone() {
    let folder = Folder::new();
    folder.script("a", "1.lua", "local g = game() log('game', g.mode, g.language, g.difficulty)");
    let vm = started(&folder);
    assert!(all(&vm).contains(": game solo int nil"), "{}", all(&vm));
}

/// text(section, key[, file]) reads the language files (here a French one); nil for a missing
/// key. Negative control: another file name gives nil.
#[test]
fn text_reads_the_language_files() {
    let dir = std::env::temp_dir().join(format!("oxiii-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = crate::config::ConfigCache::new(dir.clone());
    c.language = "frt".into();
    c.replace_file("Engine.frt", "[EndMsgMap]\r\nPlage01=\"A@B|C\"\r\nEmpty=\"\"\r\n");
    let folder = Folder::new();
    folder.script("a", "1.lua", "log('text', text('EndMsgMap', 'plage01'), text('EndMsgMap', 'Empty'), text('EndMsgMap', 'Nope'), text('EndMsgMap', 'Plage01', 'XIII'))");
    let mut vm = Vm { config: Some(c), ..Default::default() };
    vm.set_mod_scripts_here(folder.mods());
    vm.mod_scripts_start();
    assert!(all(&vm).contains(": text A@B|C nil nil nil"), "{}", all(&vm));
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------------------------
// More functions on the real Plage01.

/// on("fire"): each shot of the player's gun is one call (shooter, weapon class), only while a
/// handler exists; on("fire", "Pam") does not hear the player (negative control).
#[test]
fn fire_event_counts_the_shots() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    local n = 0
    on("fire", function(who, class) n = n + 1 log("shot", n, who == player(), class:lower()) end)
    on("fire", "Pam", function() log("pam fired") end)
    on("start", function(p)
      p:give("XIII.M16")
      p:get("Controller"):call("SwitchWeapon", 11)
      wait(1.5)
      log("ready", p.weapon)
    end)
    "#;
    let (_folder, mut vm, pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": ready", 4), "{}", all(&vm));
    let w = vm.prop(pawn, "Weapon").as_object().unwrap();
    for _ in 0..3 {
        vm.set_prop(pc, "bFire", Value::Byte(1));
        let _ = vm.call_named(w, "Fire", vec![Value::Float(0.0)]);
        vm.set_prop(pc, "bFire", Value::Byte(0));
        for _ in 0..20 {
            vm.tick(1.0 / 30.0);
        }
    }
    let text = all(&vm);
    assert!(text.contains(": shot 1 true xiii.m16"), "{text}");
    assert!(text.contains(": shot 3 true xiii.m16") && !text.contains(": shot 4"), "{text}");
    assert!(!text.contains("pam fired"), "{text}");
}

/// on("damage"): who hit, the damage type and the weapon held by who hit; without a cause, nil.
/// Negative control: the old arguments come first (actor, amount).
#[test]
fn damage_event_says_who_and_with_what() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    local low = function(s) return s and s:lower() end
    on("damage", "Raider", function(who, amount, by, kind, weapon)
      log("hurt", who.name == find("Raider").name, amount > 0, by == player(), low(kind), low(weapon))
    end)
    on("start", function(p)
      p:give("XIII.M16")
      p:get("Controller"):call("SwitchWeapon", 11)
      local at = p.position
      local r
      for i = 1, 8 do
        r = r or spawn{character = "Raider", position = {at[1] + 300 * math.cos(i), at[2] + 300 * math.sin(i), at[3]}}
      end
      wait(1.5)
      r:call("TakeDamage", 5, p, r:get("Location"), {0, 0, 0}, "XIII.DTGunned")
      wait(0.5)
      r:call("TakeDamage", 3, nil, r:get("Location"), {0, 0, 0}, "XIII.DTFire")
      wait(0.5)
      log("damage done")
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, SOLDIER, lua);
    assert!(tick_until(&mut vm, pawn, None, ": damage done", 5), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains(": hurt true true true xiii.dtgunned xiii.m16"), "{text}");
    assert!(text.contains(": hurt true true false xiii.dtfire nil"), "{text}");
}

/// is_a, trigger, ammo, has, take and aim on the player and the level's actors.
#[test]
fn actor_shortcuts_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("event", "ShortcutDone", function() log("triggered") end)
    on("start", function(p)
      log("is_a", p:is_a("Pawn"), p:is_a("XIII.XIIIPawn"), p:is_a("Emitter"), p:is_a("No.Such"))
      p:give("XIII.M16")
      p:get("Controller"):call("SwitchWeapon", 11)
      wait(1.5)
      log("has", p:has("XIII.M16"), p:has("XIII.FusilPompe"))
      local before = p.ammo
      p.ammo = 7
      log("ammo", before ~= nil, p.ammo, p:get("Weapon"):get("AmmoType"):get("AmmoAmount"), find("PorteCabane") and find("PorteCabane").ammo)
      log("take", p:take("XIII.M16"), p:has("XIII.M16"))
      local a = p:aim()
      local f = a.from
      log("aim", #a.from, math.abs(a.direction[1] ^ 2 + a.direction[2] ^ 2 + a.direction[3] ^ 2 - 1) < 0.001, f[3] > p.position[3] + 50)
      local d = spawn{class = "Engine.Dispatcher", position = p.position}
      d:set("OutEvents[0]", "ShortcutDone")
      d:trigger()
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": aim", 4), "{}", all(&vm));
    let text = all(&vm);
    for want in [": is_a true true false false", ": has true false", ": ammo true 7 7 nil", ": take 1 false", ": aim 3 true true"] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    assert!(tick_until(&mut vm, pawn, None, ": triggered", 3), "{}", all(&vm));
}

/// message, objective and fade to one player only; sound() gives an id that stop_sound stops
/// once; a screen item is drawn after the HUD. Negative controls: a non-player is refused; no item,
/// nothing more drawn.
#[test]
fn per_player_messages_sounds_and_items_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("start", function(p)
      message("Only you.", p)
      objective("Your goal", p)
      log("not a player", pcall(message, "x", find("Pam")))
      fade{to = "black", seconds = 0.2, player = p}
      local id = sound("XIIISound.Ambient.SancFire", p)
      log("sound", type(id), stop_sound(id), stop_sound(id))
      music_stop()
      log("size", text_size("Hello") > 0, text_size("Hello, Kim") > text_size("Hello"))
      log("sent")
    end)
    "#;
    let (_folder, mut vm, pc, pawn) = plage01_with(&game, "", lua);
    assert!(tick_until(&mut vm, pawn, None, ": sent", 3), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains("message \"Only you.\"") && text.contains("objective \"Your goal\""), "{text}");
    assert!(text.contains(": not a player false"), "{text}");
    assert!(text.contains(": sound number true false"), "{text}");
    assert!(text.contains(": size true true"), "{text}");
    assert!(tick_until(&mut vm, pawn, None, "never", 1) || true);
    assert!(vm.prop(pc, "ConstantGlowScale").as_float() < -0.9, "faded");
    // A screen item: drawn after the game's HUD.
    vm.render_hud(1280, 720);

    vm.mods.hud.items.push(crate::mod_script_hud::Item {
        id: "t".into(),
        kind: crate::mod_script_hud::Kind::Bar,
        x: 0.1,
        y: 0.1,
        w: 0.2,
        h: 0.05,
        text: String::new(),
        value: 0.5,
        ends_at: 0.0,
        font: "XIIIFonts.PoliceF16".into(),
        colour: [255, 0, 0, 255],
        back: Some([0, 0, 0, 255]),
        align: 0,
        texture: String::new(),
        player: None,
    });
    vm.render_hud(1280, 720);
    let red = |vm: &Vm| vm.hud.iter().filter(|d| matches!(d, crate::canvas::HudDraw::Tile { color: [0, 0, 255, 255], material, .. } if material.ends_with("#white"))).count();
    assert_eq!(red(&vm), 1, "the bar's fill, in red");
    vm.mods.hud.items.clear();
    vm.render_hud(1280, 720);
    assert_eq!(red(&vm), 0, "negative control: no item, no bar");
}

/// The flamethrower example on Plage01: given at the start, in hand with its fuel; holding the
/// fire button lights the jet and burns fuel; letting go puts it out. Its script reports no error.
/// Negative control: before the button, the jet is out.
#[test]
fn example_flamethrower_fires_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods/flamethrower");
    let mods = vec![ScriptMod { name: "flamethrower".into(), folder: dir.clone() }];
    let (mut vm, pc, pawn) = level(&game, "Plage01", &dir.join("mod.toml"), mods);
    let weapon = |vm: &Vm| vm.prop(pawn, "Weapon").as_object().map(|w| vm.objects[w].as_ref().map(|o| vm.class_path(o.class)).unwrap_or_default()).unwrap_or_default();
    for _ in 0..(4 * 30) {
        if let Some(level) = vm.level {
            vm.set_prop(level, "bCineFrame", Value::Bool(false));
        }
        vm.tick(1.0 / 30.0);
    }
    assert_eq!(weapon(&vm), "Mod.Flamethrower", "{}", all(&vm));
    let fuel = |vm: &Vm| {
        let w = vm.prop(pawn, "Weapon").as_object().unwrap();
        let a = vm.prop(w, "AmmoType").as_object().unwrap();
        vm.prop(a, "AmmoAmount").as_int()
    };
    assert_eq!(fuel(&vm), 300, "the start fuel");
    // The jets: the flame emitters of the mod in the level, lit or not.
    let lit = |vm: &Vm| {
        let class = vm.class("Mod.FlameJet").unwrap();
        vm.actors
            .iter()
            .filter(|&&a| vm.objects[a].as_ref().is_some_and(|o| vm.is_child_of(o.class, class)))
            .any(|&a| matches!(crate::mod_script_world::property_path(vm, a, "Emitters[0].Disabled"), Ok((o, last)) if !vm.prop(o, &last).as_bool()))
    };
    assert!(!lit(&vm), "negative control: out before the button");
    let w = vm.prop(pawn, "Weapon").as_object().unwrap();
    let mut was_lit = false;
    for f in 0..(2 * 30) {
        vm.set_prop(pc, "bFire", Value::Byte(1));
        if f % 10 == 0 {
            let _ = vm.call_named(w, "Fire", vec![Value::Float(0.0)]);
        }
        vm.tick(1.0 / 30.0);
        was_lit |= lit(&vm);
    }
    assert!(was_lit, "lit while the button is held\n{}", all(&vm));
    assert!(fuel(&vm) < 300, "fuel burnt: {}", fuel(&vm));
    vm.set_prop(pc, "bFire", Value::Byte(0));
    for _ in 0..30 {
        vm.tick(1.0 / 30.0);
    }
    assert!(!lit(&vm), "out once let go\n{}", all(&vm));
    // An error of a part of the script is written "flamethrower: <part>: <why>" (its `guarded`).
    let parts = ["weapon", "player", "burning", "fires", "ground fires", "blazes", "heat", "carried bodies"];
    let errors: Vec<&String> = vm.mod_messages.iter().filter(|m| m.contains("warning: script") || parts.iter().any(|p| m.contains(&format!("flamethrower: {p}: ")))).collect();
    assert!(errors.is_empty(), "{errors:#?}");
}

/// actor:hurt() goes through the game's TakeDamage (the damage handler hears who and with
/// what) and gives the health lost; actor:can_see() is the character's own line of sight.
/// Negative controls: an actor without eyes sees nothing; hurting nothing alive loses nothing.
#[test]
fn hurt_and_can_see_on_plage01() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let lua = r#"
    on("damage", "Raider", function(who, amount, by, kind) log("hit by", by == player(), kind:lower()) end)
    on("start", function(p)
      local at = p.position
      local r
      for i = 1, 8 do
        r = r or spawn{character = "Raider", position = {at[1] + 250 * math.cos(i), at[2] + 250 * math.sin(i), at[3]}}
      end
      wait(1)
      local before = r.health
      local lost = r:hurt(5, p, "XIII.DTFire")
      log("hurt", lost > 0, lost == before - r.health)
      local med = spawn{class = "XIII.MedPick", position = {at[1] + 120, at[2], at[3]}}
      local function raw(a, b) local ok, v = pcall(function() return a:get("Controller"):call("LineOfSightTo", b) end) return ok and v end
      local same = r:can_see(p) == raw(r, p) and p:can_see(r) == raw(p, r) and p:can_see(med) == raw(p, med)
      log("can_see", same, p:can_see(p), med ~= nil and med:can_see(p))
      wait(0.5)
      log("checked")
    end)
    "#;
    let (_folder, mut vm, _pc, pawn) = plage01_with(&game, SOLDIER, lua);
    assert!(tick_until(&mut vm, pawn, None, ": checked", 4), "{}", all(&vm));
    let text = all(&vm);
    assert!(text.contains(": hurt true true"), "{text}");
    assert!(text.contains(": hit by true xiii.dtfire"), "{text}");
    assert!(text.contains(": can_see true true false"), "the game's own line of sight; an actor without eyes sees nothing
{text}");
}
