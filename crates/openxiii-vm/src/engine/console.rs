//! Console commands as the engine runs them: UWindowsViewport::Exec,
//! UViewport::Exec, UPlayer::Exec, UGameEngine::Exec,
//! UEngine::Exec, UObject::StaticExec for GET
//! and SET, UInput::Exec, UClient::Exec. The text a command writes
//! (FStringOutputDevice) is returned to ConsoleCommand.

use crate::config::CPF_CONFIG;
use crate::types::Type;
use crate::value::{ObjId, Value};
use crate::Vm;

/// ParseCommand: the word (case-insensitive) followed by something other than a
/// letter or a digit; the rest, blanks skipped.
pub(crate) fn parse_command<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let t = s.trim_start_matches([' ', '\t']);
    let (b, w) = (t.as_bytes(), word.as_bytes());
    if b.len() < w.len() || !b[..w.len()].eq_ignore_ascii_case(w) {
        return None;
    }
    let rest = &t[w.len()..];
    if rest.as_bytes().first().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(rest.trim_start_matches([' ', '\t']))
}

/// SETRES's "WxH" (appAtoi of each side): None when one side is zero (the command does nothing).
fn setres_size(rest: &str) -> Option<(i32, i32)> {
    let rest: Vec<char> = rest.chars().collect();
    let w = crate::config::atoi(&rest, 0);
    let h = rest.iter().position(|&c| c == 'x' || c == 'X').map_or(0, |i| crate::config::atoi(&rest, i + 1));
    (w > 0 && h > 0).then_some((w, h))
}

/// ParseToken(Str, FString&, UseEscape = 1): spaces and
/// tabs skipped; a quoted string ("\x" gives x), otherwise a word; false if the
/// token is empty ("" included). Returns the token and the rest of the line.
pub(crate) fn parse_token(s: &str) -> Option<(String, &str)> {
    let t = s.trim_start_matches([' ', '\t']);
    let mut token = String::new();
    let mut it = t.char_indices().peekable();
    let mut end = t.len();
    if t.starts_with('"') {
        it.next();
        end = t.len();
        while let Some(&(i, c)) = it.peek() {
            if c == '"' {
                end = i + 1;
                break;
            }
            it.next();
            let c = if c == '\\' {
                match it.next() {
                    Some((_, e)) => e,
                    None => break,
                }
            } else {
                c
            };
            token.push(c);
        }
    } else {
        for (i, c) in it {
            if c == ' ' || c == '\t' {
                end = i;
                break;
            }
            token.push(c);
        }
    }
    (!token.is_empty()).then(|| (token, &t[end.min(t.len())..]))
}

/// Commands refused by UWindowsViewport::Exec (table of WinDrv.dll) when they do not
/// come from APlayerController::execConsoleCommand and the level does not have bAllowCheat:
/// the typed console and the keys are not allowed them.
const BLOCKED: [&str; 91] = [
    "GOD", "SET", "GET", "NAME", "PAUSE", "TELEPORT", "TELEPORTNEXT", "DANDD", "EQUIPME", "GHOST", "FLY", "WALK", "KILLALL", "KILLALLPAWNS", "KILLPAWNS", "KILLMI",
    "KILLTARGET", "AMBIDX", "IMAWINNER", "IMALOSER", "IMBOSS", "IMANALIEN", "LETMEBREATHE", "LETMEHEAL", "LETMESTUN", "LETMESNIPE", "LETMEPICKLOCKS", "POWERY",
    "WEAPONRY", "THROWWEAPON", "ARMORY", "ALLAMMO", "SUPERHERO", "AAM", "AUTOPICKUP", "CROSSHAIRSIZE", "SETJUMPZ", "CHANGESKIN", "NEUNEU", "ALLBOSSES",
    "LOADEDSOLDIERS", "LOGSOLDIER", "NUKE", "CAUSEEVENT", "GORELEVEL", "LEVELDETAIL", "FREECAMERA", "SETCAMERADIST", "CHEATVIEW", "VIEWCLASS", "VIEWSELF",
    "BEHINDVIEW", "FOV", "TD", "SC", "AVATAR", "SUMMON", "SETSPEED", "SLOMO", "DIFF", "HIDELOG", "WRITETOLOG", "SHOWLOG", "SHOWDEBUG", "EDITACTOR",
    "COPYTOCLIPBOARD", "TAKEFOCUS", "MPLAYER", "HEAT", "RMODE", "STAT", "FOV", "DOHIDEHUD", "START", "OPEN", "RESTART", "EXIT", "RESTARTLEVEL",
    "LOGINTERACTIVES", "TOGGLEFULLSCREEN", "ENDFULLSCREEN", "GETCURRENTRES", "GETCURRENTCOLORDEPTH", "GETCOLORDEPTHS", "GETCURRENTRENDERDEVICE", "GETRES",
    "SETRES", "SETVIEWPORTNUMBERFORNEXTMAP", "EXTENDVIEWPORTBEFOREMENU", "RESTOREVIEWPORTAFTERMENU", "PREFERENCES",
];

impl Vm {
    /// APlayerController::execConsoleCommand: GIsACommandFromPlayerController
    /// is 1 during the command, 0 afterwards.
    pub fn console_command_from_pc(&mut self, pc: ObjId, command: &str) -> (bool, String) {
        self.command_from_pc = true;
        let r = self.console_command_out(pc, command);
        self.command_from_pc = false;
        r
    }

    /// UWindowsViewport::Exec of the window of controller `pc`: the cheat command filter,
    /// UViewport::Exec, then the window's commands (GETCURRENTRES…). True if a
    /// command took it; the written text.
    pub fn console_command_out(&mut self, pc: ObjId, command: &str) -> (bool, String) {
        // Mods build only: the modpos command (mod_placement.rs).
        #[cfg(feature = "mods")]
        if let Some(text) = self.mod_console(pc, command) {
            return (true, text);
        }
        let allow_cheat = self.level.is_some_and(|l| self.propr(l, "bAllowCheat").as_bool());
        if !self.command_from_pc && !allow_cheat && BLOCKED.iter().any(|w| parse_command(command, w).is_some()) {
            return (false, String::new());
        }
        let mut out = String::new();
        if self.viewport_exec(pc, command, &mut out) {
            return (true, out);
        }
        // Lockstep co-op: the other machine's player's window commands are that machine's
        // (each machine keeps its own window, fullscreen and resolution).
        if self.lockstep_foreign(pc) && ["TOGGLEFULLSCREEN", "ENDFULLSCREEN", "SETRES"].iter().any(|w| parse_command(command, w).is_some()) {
            // What a resolution change does to the game itself (the precaching frame, the
            // weather points placed again with the game's random generator) happens on every
            // machine at the same step, the window's part only on that player's own.
            if let Some(rest) = parse_command(command, "SETRES").filter(|_| true) {
                if setres_size(rest).is_some() {
                    self.setres_game_side();
                }
            }
            return (true, String::new());
        }
        match self.windows_viewport_exec(command) {
            Some(text) => (true, text),
            None => (false, out),
        }
    }

    /// UPlayer::Exec of a controller without a viewport (a client's connection), command coming from
    /// APlayerController::execConsoleCommand.
    pub(crate) fn player_exec_out(&mut self, pc: ObjId, command: &str) -> (bool, String) {
        self.command_from_pc = true;
        let mut out = String::new();
        let r = self.player_exec(pc, command, &mut out);
        self.command_from_pc = false;
        (r, out)
    }

    pub fn console_command(&mut self, pc: ObjId, command: &str) -> bool {
        self.console_command_out(pc, command).0
    }

    /// UViewport::Exec: UInput, the player's console (Player.Console), then UPlayer::Exec.
    fn viewport_exec(&mut self, pc: ObjId, command: &str, out: &mut String) -> bool {
        if self.input_exec(command, out) {
            return true;
        }
        let pawn = self.propr(pc, "Pawn").as_object().filter(|&p| self.alive(p));
        if let Some(console) = self.propr(pc, "Player").as_object().and_then(|p| self.propr(p, "Console").as_object()).filter(|&c| self.alive(c)) {
            if self.script_console_exec(console, command, pawn) {
                return true;
            }
        }
        self.player_exec(pc, command, out)
    }

    /// UPlayer::Exec: the game, the HUD, the cheat manager, the controller, its pawn and its
    /// inventory (exec functions, with the pawn as executor), then UGameEngine::Exec.
    fn player_exec(&mut self, pc: ObjId, command: &str, out: &mut String) -> bool {
        // XLevel->Exec (vtable +0x8C, ULevel::Exec: UNetDriver::Exec, demo recording)
        // comes before the player's objects.
        if self.net_driver_exec(command, out) {
            return true;
        }
        let pawn = self.propr(pc, "Pawn").as_object().filter(|&p| self.alive(p));
        let mut targets: Vec<ObjId> = Vec::new();
        targets.extend(self.level.and_then(|l| self.propr(l, "Game").as_object()));
        targets.extend(self.propr(pc, "myHUD").as_object());
        targets.extend(self.propr(pc, "CheatManager").as_object());
        targets.push(pc);
        if let Some(p) = pawn {
            targets.push(p);
        }
        for obj in targets {
            if self.alive(obj) && self.exec_target(obj, command, pawn) {
                return true;
            }
        }
        if let Some(p) = pawn {
            let mut inv = self.propr(p, "Inventory").as_object();
            let mut guard = 0;
            while let (Some(i), true) = (inv, guard < 256) {
                if self.alive(i) && self.exec_target(i, command, pawn) {
                    return true;
                }
                inv = self.propr(i, "Inventory").as_object();
                guard += 1;
            }
        }
        self.engine_exec(command, out)
    }

    /// ScriptConsoleExec of a player object (UPlayer::Exec); tests can record the call and set its result.
    fn exec_target(&mut self, obj: ObjId, command: &str, pawn: Option<ObjId>) -> bool {
        self.script_console_exec(obj, command, pawn)
    }

    /// UInput::Exec: KEYNAME n (name of the key), KEYBINDING Key (its binding).
    fn input_exec(&mut self, command: &str, out: &mut String) -> bool {
        if let Some(rest) = parse_command(command, "KEYNAME") {
            let chars: Vec<char> = rest.chars().collect();
            let n = crate::config::atoi(&chars, 0);
            *out = Self::key_name(n as i32).to_string();
            return true;
        }
        if let Some(rest) = parse_command(command, "KEYBINDING") {
            // FindKeyName receives the whole rest of the line: "Enter x" is not a key.
            if Vm::key_code(rest).is_some() {
                *out = self.config.as_mut().and_then(|c| c.get_string("Engine.Input", rest, "User")).unwrap_or_default();
            }
            return true;
        }
        false
    }

    /// UGameEngine::Exec: travels (OPEN, START), exit, then the game and UEngine::Exec.
    pub fn engine_exec(&mut self, command: &str, out: &mut String) -> bool {
        // OPEN: partial URL; START: absolute URL (SetClientTravel).
        if let Some(url) = parse_command(command, "OPEN") {
            self.client_travel = Some((url.to_string(), crate::travel::TRAVEL_PARTIAL, false));
            return true;
        }
        if let Some(url) = parse_command(command, "START") {
            // Improved build: the Join LAN page's join of a typed address (multi_join.rs).
            let url = crate::improved::navigation::start_url(self, url);
            self.client_travel = Some((url, crate::travel::TRAVEL_ABSOLUTE, false));
            return true;
        }
        // The original keeps two pointers to the command: a local copy (OPEN, START, SERVERTRAVEL, SAY,
        // DISCONNECT, RECONNECT, GETCURRENTTICKRATE, GETMAXTICKRATE) and Cmd (EXIT, QUIT, CANCEL and the rest).
        // SERVERTRAVEL advances the copy even outside a dedicated server; on a dedicated server
        // (GIsServer and not GIsClient), GLevel->GetLevelInfo()->eventServerTravel(rest, 0).
        let mut local = command;
        if let Some(rest) = parse_command(local, "SERVERTRAVEL") {
            if self.ipdrv.dedicated {
                self.engine_server_travel(rest);
                return true;
            }
            local = rest;
        }
        // SAY, dedicated server only: Level.Game.eventBroadcast(NULL, rest, NAME_None).
        if self.ipdrv.dedicated {
            if let Some(rest) = parse_command(local, "SAY") {
                self.engine_say(rest);
                return true;
            }
        }
        // DISCONNECT: Close and FlushNet of the connection to the server, then SetClientTravel(Viewports(0),
        // "?failed", 0, TRAVEL_Absolute); RECONNECT: SetClientTravel to LastURL.
        if parse_command(local, "DISCONNECT").is_some() {
            self.net_close_server_connection();
            self.client_travel = Some(("?failed".into(), crate::travel::TRAVEL_ABSOLUTE, false));
            return true;
        }
        if parse_command(local, "RECONNECT").is_some() {
            self.client_travel = Some((self.url.string(), crate::travel::TRAVEL_ABSOLUTE, false));
            return true;
        }
        if parse_command(command, "EXIT").is_some() || parse_command(command, "QUIT").is_some() {
            // The connection to the server is closed (Close of the control channel, FlushNet) before exiting.
            self.net_close_server_connection();
            out.push_str("Closing by request");
            // appRequestExit (Core): debugf "appRequestExit(%i)" then GIsRequestingExit.
            self.log.push("Log: appRequestExit(0)".into());
            self.quit_requested = true;
            return true;
        }
        // GETCURRENTTICKRATE / GETMAXTICKRATE: Ar.Logf("%f") of CurrentTickRate and of GetMaxTickRate.
        if parse_command(local, "GETCURRENTTICKRATE").is_some() {
            out.push_str(&crate::value::printf_f(self.engine.tick_rate.current));
            return true;
        }
        if parse_command(local, "GETMAXTICKRATE").is_some() {
            out.push_str(&crate::value::printf_f(self.net_max_tick_rate()));
            return true;
        }
        // CANCEL: pending level abandoned (SetProgress "Cancelled Connect Attempt", 2 s),
        // otherwise messages cleared; then CancelPending.
        if parse_command(command, "CANCEL").is_some() {
            if self.ipdrv.pending.is_some() {
                let msg = self.localize("Progress", "CancelledConnect", "Engine", false);
                self.engine_set_progress(&msg, "", 2.0);
            } else {
                self.engine_set_progress("", "", 0.0);
            }
            self.engine_cancel_pending();
            return true;
        }
        // GLevel->Exec: UNetDriver::Exec (KICKPLAYER).
        if self.net_driver_exec(command, out) {
            return true;
        }
        if let Some(game) = self.level.and_then(|l| self.propr(l, "Game").as_object()).filter(|&g| self.alive(g)) {
            if self.script_console_exec(game, command, None) {
                return true;
            }
        }
        // Client of a network game (NetMode 3): "SET Class …" on an actor class that is not a
        // GameInfo is refused (ParseToken, FindObject, IsChildOf of AActor then of AGameInfo).
        if self.level.is_some_and(|l| self.propr(l, "NetMode").as_byte() == 3) {
            if let Some((cname, _)) = parse_command(command, "SET").and_then(parse_token) {
                if let Some(class) = self.class(&cname) {
                    let is = |vm: &Vm, n: &str| vm.class(n).is_some_and(|c| vm.is_child_of(class, c));
                    if is(self, "Actor") && !is(self, "GameInfo") {
                        return false;
                    }
                }
            }
        }
        self.uengine_exec(command, out)
    }

    /// ALevelInfo::eventServerTravel: ProcessEvent of LevelInfo.ServerTravel(URL, bItems = 0).
    fn engine_server_travel(&mut self, url: &str) {
        let Some(level) = self.level else { return };
        if let Some(f) = self.find_function(level, "ServerTravel", false) {
            if let Err(e) = self.call(f, level, vec![Some(Value::Str(url.to_string())), Some(Value::Bool(false))]) {
                self.native_warn(&format!("ServerTravel: {e}"));
            }
        }
    }

    /// AGameInfo::eventBroadcast: ProcessEvent of Game.Broadcast(None, Msg, NAME_None).
    fn engine_say(&mut self, msg: &str) {
        let Some(game) = self.level.and_then(|l| self.propr(l, "Game").as_object()).filter(|&g| self.alive(g)) else { return };
        if let Some(f) = self.find_function(game, "Broadcast", false) {
            if let Err(e) = self.call(f, game, vec![Some(Value::Object(None)), Some(Value::Str(msg.to_string())), Some(Value::Name("None".into()))]) {
                self.native_warn(&format!("Broadcast: {e}"));
            }
        }
    }

    /// UEngine::Exec: UObject::StaticExec (GET, SET), UClient::Exec, the audio (HxAudio::exec), then the engine's
    /// commands (FLUSH, ANTIPORTAL, SHOWBV, CRACKURL). GSys, GExec and the subsystem at +0x34 are queried first by the original
    /// (none of their commands is reproduced here).
    pub(crate) fn uengine_exec(&mut self, command: &str, out: &mut String) -> bool {
        if self.static_exec(command, out) {
            return true;
        }
        if self.client_exec(command) {
            return true;
        }
        // Audio->Exec: ASTAT AUDIO, PAUSESOUNDS, UNPAUSESOUNDS.
        if self.audio.as_mut().is_some_and(|hx| hx.exec(command)) {
            return true;
        }
        if parse_command(command, "FLUSH").is_some() {
            // UEngine::Flush -> UClient::Flush -> RenDev::Flush. The Renderer is owned
            // by the display layer: it drops its caches before the next rendering use.
            self.pending_renderer_flush = true;
            out.push_str("Flushed engine caches");
            return true;
        }
        // ANTIPORTAL n: +0x64 = (appAtoi(rest) == 0).
        if let Some(rest) = parse_command(command, "ANTIPORTAL") {
            self.engine_antiportal = crate::config::atoi(&rest.chars().collect::<Vec<_>>(), 0) == 0;
            return true;
        }
        // STAT: refused at the typed console without bAllowCheat (BLOCKED) and called by no script.
        // SHOWBV: toggles +0x68.
        if parse_command(command, "SHOWBV").is_some() {
            self.engine_show_bv = !self.engine_show_bv;
            return true;
        }
        // CRACKURL url: FURL(NULL, rest, TRAVEL_Absolute), its fields and FURL::String, or "BAD URL".
        if let Some(rest) = parse_command(command, "CRACKURL") {
            let url = crate::travel::Url::parse(None, rest, crate::travel::TRAVEL_ABSOLUTE);
            if url.valid {
                out.push_str(&format!("     Protocol: {}", url.protocol.clone().unwrap_or_else(|| "unreal".into())));
                out.push_str(&format!("         Host: {}", url.host));
                out.push_str(&format!("         Port: {}", url.port.unwrap_or(crate::travel::DEFAULT_PORT)));
                out.push_str(&format!("          Map: {}", url.map));
                out.push_str(&format!("   NumOptions: {}", url.options.len()));
                for (i, o) in url.options.iter().enumerate() {
                    out.push_str(&format!("     Option {i}: {o}"));
                }
                out.push_str(&format!("       Portal: {}", url.portal));
                out.push_str(&format!("       String: '{}'", url.string()));
            } else {
                out.push_str("BAD URL");
            }
            return true;
        }
        // ACTOR LIST: memory sizes of actors and classes (FArchiveCountMem), C++ statistics without
        // an equivalent.
        false
    }

    /// UObject::StaticExec: GET Class Property (exported default value), SET Class
    /// Property Value (UObject::GlobalSetProperty).
    pub(crate) fn static_exec(&mut self, command: &str, out: &mut String) -> bool {
        if self.native_config_exec(command, out) {
            return true;
        }
        if self.input_set_exec(command) {
            return true;
        }
        if let Some(rest) = parse_command(command, "GET") {
            let Some((cname, rest)) = parse_token(rest) else {
                self.log.push(format!("Unrecognized class {}", ""));
                return true;
            };
            let Some(class) = self.class(&cname) else {
                self.log.push(format!("Unrecognized class {cname}"));
                return true;
            };
            let Some((pname, _)) = parse_token(rest) else {
                self.log.push("Unrecognized property ".to_string());
                return true;
            };
            let Some(&slot) = self.classes[class].prop_index.get(&pname.to_lowercase()) else {
                self.log.push(format!("Unrecognized property {pname}"));
                return true;
            };
            let def = self.classes[class].props[slot].clone();
            let cdo = self.classes[class].cdo;
            let v = self.objects[cdo].as_ref().and_then(|o| o.props.get(slot).cloned()).unwrap_or(Value::Void);
            let v = if def.dim > 1 {
                match v {
                    Value::Array(a) => a.first().cloned().unwrap_or(Value::Void),
                    other => other,
                }
            } else {
                v
            };
            *out = self.export_text(&def.ty, def.enumeration.as_deref(), &v, None, 0);
            return true;
        }
        if let Some(rest) = parse_command(command, "SET") {
            let Some((cname, rest)) = parse_token(rest) else {
                return true;
            };
            let Some(class) = self.class(&cname) else {
                self.log.push(format!("Unrecognized class {cname}"));
                return true;
            };
            let Some((pname, rest)) = parse_token(rest) else {
                self.log.push("Unrecognized property ".to_string());
                return true;
            };
            if !self.classes[class].prop_index.contains_key(&pname.to_lowercase()) {
                self.log.push(format!("Unrecognized property {pname}"));
                return true;
            }
            let value = rest.trim_start_matches(' ');
            self.global_set_property(value, class, &pname);
            return true;
        }
        if let Some(rest) = parse_command(command, "OBJ") {
            return self.obj_exec(rest);
        }
        false
    }

    /// UObject::StaticExec, "OBJ". MARK sets flag 0x8000000 on all objects, MARKCHECK logs
    /// those that do not have it (GetFullName), GARBAGE runs CollectGarbage: Rust never frees an object, nothing to do.
    /// The other sub-commands (REFS, HASH, NAMES, CLASSES, DEPENDENCIES, LIST, LINKERS) are listings of the original's full
    /// object table (FArchiveCountMem sizes included) that Rust does not keep: the command is
    /// recognized, without output. An unknown sub-command returns false, as in the original.
    fn obj_exec(&mut self, rest: &str) -> bool {
        if parse_command(rest, "GARBAGE").is_some() {
            return true;
        }
        if parse_command(rest, "MARK").is_some() {
            self.log.push("Marking objects".to_string());
            self.objects_marked = (0..self.objects.len()).filter(|&i| self.objects[i].is_some()).collect();
            return true;
        }
        if parse_command(rest, "MARKCHECK").is_some() {
            self.log.push("Unmarked objects:".to_string());
            for i in 0..self.objects.len() {
                let Some(o) = self.objects[i].as_ref() else { continue };
                if !self.objects_marked.contains(&i) {
                    let name_value = format!("{} {}", self.classes[o.class].name, self.path_name(i));
                    self.log.push(name_value);
                }
            }
            return true;
        }
        ["REFS", "HASH", "NAMES", "CLASSES", "DEPENDENCIES", "LIST", "LINKERS"].iter().any(|c| parse_command(rest, c).is_some())
    }

    /// GET / SET on UHXAUDIOSubsystem, a class without a script whose configuration properties HXAudio.dll creates
    /// (StaticConstructor: MasterVolume, MusicSliderPos…, configuration
    /// "User"): the value lives in the [HXAudio.HXAudioSubsystem] section, which SaveConfig
    /// rewrites after SET.
    fn native_config_exec(&mut self, command: &str, out: &mut String) -> bool {
        let (get, rest) = match (parse_command(command, "GET"), parse_command(command, "SET")) {
            (Some(r), _) => (true, r),
            (_, Some(r)) => (false, r),
            _ => return false,
        };
        let Some((cname, rest)) = parse_token(rest) else { return false };
        if !cname.eq_ignore_ascii_case("HXAudio.HXAudioSubsystem") && !cname.eq_ignore_ascii_case("HXAudioSubsystem") {
            return false;
        }
        let Some((pname, rest)) = parse_token(rest) else {
            self.log.push("Unrecognized property ".to_string());
            return true;
        };
        // The 15 properties of StaticConstructor, WalkSpeed included.
        let Some(&(name, ty, ..)) = openxiii_hx::subsystem::PROPERTIES_CONFIG.iter().find(|p| p.0.eq_ignore_ascii_case(&pname)) else {
            self.log.push(format!("Unrecognized property {pname}"));
            return true;
        };
        let section = "HXAudio.HXAudioSubsystem";
        let Some(c) = self.config.as_mut() else { return true };
        let current = c.get_string(section, name, "User").unwrap_or_default();
        let source = if get { current.clone() } else { rest.trim_start_matches(' ').to_string() };
        let chars: Vec<char> = source.chars().collect();
        let text = match ty {
            b'f' => {
                let v = Value::Float(crate::config::atof(&chars, 0));
                self.export_text(&Type::Float, None, &v, None, 0)
            }
            b'i' => crate::config::atoi(&chars, 0).to_string(),
            // UBoolProperty::ImportText: "1", "True" or GTrue, "0",
            // "False" or GFalse; another word leaves the value unchanged.
            b'b' => {
                let mut v = Value::Bool(current.eq_ignore_ascii_case("True"));
                let _ = self.import_text(&Type::Bool, None, &chars, 0, 0, &mut v);
                if v.as_bool() { "True" } else { "False" }.to_string()
            }
            _ => chars.iter().collect(),
        };
        if get {
            *out = text;
        } else {
            if let Some(c) = self.config.as_mut() {
                c.set_string(section, name, &text, "User");
            }
            self.hx_global_set_property(name, &text);
        }
        true
    }

    /// SET Input Key Command: UInput::StaticInitInput (Engine) gives the native class UInput one config string
    /// property per key name (its Bindings), so StaticExec's SET finds the class and the key and
    /// GlobalSetProperty imports the rest of the line (UStrProperty::ImportText: the whole rest, empty
    /// clears the key) into the bindings, then SaveConfig writes [Engine.Input] of User.ini. The
    /// Controls pages send these then call OptimizeInputBindings, which compiles the bindings again
    /// from the configuration. Without it a key chosen on those pages was shown but neither kept nor used.
    fn input_set_exec(&mut self, command: &str) -> bool {
        let Some(rest) = parse_command(command, "SET") else { return false };
        let Some((cname, rest)) = parse_token(rest) else { return false };
        if !cname.eq_ignore_ascii_case("Input") && !cname.eq_ignore_ascii_case("Engine.Input") {
            return false;
        }
        let Some((key, rest)) = parse_token(rest) else {
            self.log.push("Unrecognized property ".to_string());
            return true;
        };
        let Some(code) = Vm::key_code(&key) else {
            self.log.push(format!("Unrecognized property {key}"));
            return true;
        };
        let name = Self::key_name(i32::from(code)).to_string();
        let value = rest.trim_start_matches(' ');
        let value = value.strip_prefix('"').map_or(value, |v| v.split('"').next().unwrap_or(""));
        if let Some(c) = self.config.as_mut() {
            c.set_string("Engine.Input", &name, value, "User");
        }
        true
    }

    /// UObject::GlobalSetProperty reaching the live UHXAUDIOSubsystem: ImportText of the
    /// new value into the object's field, then its PostEditChange (vtable +0x4C), which
    /// clamps the settings and applies the music position and the master volume line. Without it the
    /// running subsystem kept its start-up settings and the next LoadSounds (each map) put them back.
    fn hx_global_set_property(&mut self, name: &str, text: &str) {
        use openxiii_hx::subsystem::FieldConfig;
        let Some(mut config) = self.audio.as_ref().map(|hx| hx.config.clone()) else { return };
        let chars: Vec<char> = text.chars().collect();
        let (ty, mut v) = match config.field_value(name) {
            Some(FieldConfig::BoolVar(b)) => (Type::Bool, Value::Bool(*b)),
            Some(FieldConfig::FloatVar(f)) => (Type::Float, Value::Float(*f)),
            Some(FieldConfig::Integer(i)) => (Type::Int, Value::Int(*i)),
            Some(FieldConfig::StringVar(s)) => (Type::Str, Value::Str(s.clone())),
            None => return,
        };
        let _ = self.import_text(&ty, None, &chars, 0, 0, &mut v);
        match (config.field_value(name), v) {
            (Some(FieldConfig::BoolVar(b)), Value::Bool(x)) => *b = x,
            (Some(FieldConfig::FloatVar(f)), Value::Float(x)) => *f = x,
            (Some(FieldConfig::Integer(i)), Value::Int(x)) => *i = x,
            (Some(FieldConfig::StringVar(s)), Value::Str(x)) => *s = x,
            _ => {}
        }
        if let Some(hx) = self.audio.as_mut() {
            hx.config = config;
            hx.post_edit_change();
        }
    }

    /// UObject::GlobalSetProperty: the value in each object of the class
    /// and in its default values, then SaveConfig of the default values.
    pub(crate) fn global_set_property(&mut self, value: &str, class: crate::value::ClassId, pname: &str) {
        let slot = self.classes[class].prop_index[&pname.to_lowercase()];
        let def = self.classes[class].props[slot].clone();
        let chars: Vec<char> = value.chars().collect();
        let objects: Vec<ObjId> =
            (0..self.objects.len()).filter(|&o| self.objects[o].as_ref().is_some_and(|x| self.is_child_of(x.class, class)) && !self.is_cdo(o)).collect();
        let cdo = self.classes[class].cdo;
        for o in objects.into_iter().chain(std::iter::once(cdo)) {
            let mut v = self.objects[o].as_ref().map(|x| x.props[slot].clone()).unwrap_or(Value::Void);
            if def.dim > 1 {
                if let Value::Array(items) = &mut v {
                    if let Some(first) = items.first_mut() {
                        self.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, first);
                    }
                }
            } else {
                self.import_text_named(&def.name, &def.ty, def.enumeration.as_deref(), &chars, 0, 1, &mut v);
            }
            if let Some(x) = self.objects[o].as_mut() {
                x.props[slot] = v;
            }
            // Property->ImportText then Object->PostEditChange for each object; the default values only get
            // the ImportText.
            if o != cdo {
                self.post_edit_change(o);
            }
        }
        self.save_config(cdo, CPF_CONFIG);
    }

    /// PostEditChange dispatch: parent Actor then the body of the derived class.
    pub(crate) fn post_edit_change(&mut self, _o: ObjId) {
        if self.is_a(_o, "Actor") {
            self.actor_post_edit_change(_o);
            if self.is_a(_o, "Projector") {
                self.projector_post_edit_change(_o);
            } else if self.is_a(_o, "Emitter") {
                self.emitter_post_edit_change(_o);
            }
        } else if self.is_a(_o, "ParticleEmitter") {
            self.particle_emitter_post_edit_change(_o);
        }
    }

    pub(crate) fn is_cdo(&self, o: ObjId) -> bool {
        self.objects[o].as_ref().is_some_and(|x| self.classes[x.class].cdo == o)
    }

    /// UObject::SaveConfig: the configuration properties in the
    /// class's section (or the object's, PerObjectConfig); a dynamic array becomes a
    /// repeated key. The files stay in memory (the game folder is not modified). Then the
    /// default values of the classes with the same configuration are read again (LoadConfig).
    /// Ranks of classes[class].props in TFieldIterator<UProperty> order: the properties of
    /// the class (order of its Children), then those of each parent class.
    pub(crate) fn field_order(&self, class: crate::ClassId) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.classes[class].props.len());
        let mut end_value = self.classes[class].props.len();
        let mut c = Some(class);
        while let Some(k) = c {
            let start_value = self.classes[k].super_class.map_or(0, |s| self.classes[s].props.len().min(end_value));
            order.extend(start_value..end_value);
            end_value = start_value;
            c = self.classes[k].super_class;
        }
        order
    }

    pub(crate) fn save_config(&mut self, obj: ObjId, flags: u32) {
        let Some(class) = self.objects[obj].as_ref().map(|o| o.class) else { return };
        let per_object = self.classes[class].flags & crate::config::CLASS_PER_OBJECT_CONFIG != 0 && !self.is_cdo(obj);
        let file = self.classes[class].config_name.clone();
        let props = self.classes[class].props.clone();
        // A controller saved while the gamepad is in use (aim_device.rs, not in the original): its
        // iAutoAimMode is the gamepad's, kept in [OpenXIII.Gamepad] AutoAim; the keyboard value
        // stays. (section, value saved, previous keyboard value).
        // The controller of a local player after the first (gamepad_config.rs): its aim values
        // are its own gamepad's; User.ini keeps the first player's (both keys stay as they were).
        let other = self.other_pad_controller(obj);
        let pad_swap = other.is_none() && self.aim_pad_swap(obj);
        let mut other_kept: Vec<(String, String, Option<String>)> = Vec::new();
        let mut other_aim: Option<bool> = None;
        let mut pad_aim: Option<(String, String, Option<String>)> = None;
        let mut pad_rotation: Option<(String, Option<String>)> = None;
        // TFieldIterator (UObject::SaveConfig): the properties of the class, then
        // those of each parent; a new key is thus appended at the end of its section
        // (DecalX, DecalY of XIIIMenuVideoClientWindow before
        // SafeScreenPourcentageX/Y of XIIIWindow).
        for i in self.field_order(class) {
            let p = &props[i];
            if p.flags & flags != flags {
                continue;
            }
            let section = if per_object {
                self.objects[obj].as_ref().map(|o| o.name.clone()).unwrap_or_default()
            } else if p.flags & crate::config::CPF_GLOBAL_CONFIG != 0 {
                let owner = self.prop_owner(class, i);
                self.class_path(owner)
            } else {
                self.class_path(class)
            };
            let v = self.objects[obj].as_ref().map(|o| o.props[i].clone()).unwrap_or(Value::Void);
            if let Type::Array(inner) = &p.ty {
                let items = match &v {
                    Value::Array(a) => a.clone(),
                    _ => Vec::new(),
                };
                // Data and Delta are the same address (0x9C / 0xA4: (buffer, value, value, 0)): nothing is skipped.
                let texts: Vec<String> = items.iter().map(|it| self.export_text(inner, p.enumeration.as_deref(), it, None, crate::travel::EXPORT_ALL)).collect();
                if let Some(c) = self.config.as_mut() {
                    c.replace_key(&section, &p.name, texts, &file);
                }
                continue;
            }
            for k in 0..p.dim {
                let key = if p.dim == 1 { p.name.clone() } else { format!("{}[{k}]", p.name) };
                let e = if p.dim > 1 {
                    match &v {
                        Value::Array(a) => a.get(k).cloned().unwrap_or(Value::Void),
                        other => other.clone(),
                    }
                } else {
                    v.clone()
                };
                let text = self.export_text(&p.ty, p.enumeration.as_deref(), &e, None, crate::travel::EXPORT_ALL);
                if let Some(c) = self.config.as_mut() {
                    if other.is_some() && p.dim == 1 && (key.eq_ignore_ascii_case("iAutoAimMode") || key.eq_ignore_ascii_case("bRotationAssist")) {
                        other_kept.push((section.clone(), key.clone(), c.get_string(&section, &key, &file)));
                        if key.eq_ignore_ascii_case("iAutoAimMode") {
                            other_aim = Some(text.trim() != "0");
                        }
                    }
                    if pad_swap && p.dim == 1 && key.eq_ignore_ascii_case("iAutoAimMode") {
                        pad_aim = Some((section.clone(), text.clone(), c.get_string(&section, &key, &file)));
                    }
                    // bRotationAssist is on because of the gamepad (aim_device.rs): the keyboard value stays.
                    if pad_swap && p.dim == 1 && key.eq_ignore_ascii_case("bRotationAssist") {
                        pad_rotation = Some((section.clone(), c.get_string(&section, &key, &file)));
                    }
                    c.set_string(&section, &key, &text, &file);
                }
            }
        }
        if let (Some((section, text, before)), Some(c)) = (pad_aim, self.config.as_mut()) {
            c.set_string(crate::aim_device::PAD_SECTION, crate::aim_device::PAD_KEY, &text, &file);
            match before {
                Some(b) => c.set_string(&section, "iAutoAimMode", &b, &file),
                None => c.replace_key(&section, "iAutoAimMode", Vec::new(), &file),
            }
        }
        if let Some(rank) = other {
            if let Some(on) = other_aim {
                self.other_aim_saved(rank, on);
            }
            for (section, key, before) in other_kept {
                if let Some(c) = self.config.as_mut() {
                    match before {
                        Some(b) => c.set_string(&section, &key, &b, &file),
                        None => c.replace_key(&section, &key, Vec::new(), &file),
                    }
                }
            }
        }
        if let (Some((section, before)), Some(c)) = (pad_rotation, self.config.as_mut()) {
            match before {
                Some(b) => c.set_string(&section, "bRotationAssist", &b, &file),
                None => c.replace_key(&section, "bRotationAssist", Vec::new(), &file),
            }
        }
        // GConfig->Flush(0, NULL): the changed ini files are written.
        if let Some(c) = self.config.as_mut() {
            c.flush();
        }
        // Root configuration class, then LoadConfig of the default values of its classes.
        let mut root = class;
        while let Some(s) = self.classes[root].super_class {
            if self.classes[s].flags & crate::config::CLASS_CONFIG == 0 {
                break;
            }
            root = s;
        }
        for c in 0..self.classes.len() {
            if self.is_child_of(c, root) {
                let cdo = self.classes[c].cdo;
                let mut values = self.objects[cdo].as_ref().map(|o| o.props.clone()).unwrap_or_default();
                values.with_slice(|v| self.load_config_values(c, v, None));
                if let Some(o) = self.objects[cdo].as_mut() {
                    o.props = values;
                }
            }
        }
    }

    /// UObject::ResetConfig: the keys of the class's section taken from
    /// Default.ini ("System" configuration) or from DefUser.ini ("User"), then LoadConfig of the
    /// default values of its subclasses and of all their objects.
    pub(crate) fn reset_config(&mut self, class: crate::value::ClassId) {
        let file = self.classes[class].config_name.clone();
        let default = if file.eq_ignore_ascii_case("System") {
            "Default.ini"
        } else if file.eq_ignore_ascii_case("User") {
            "DefUser.ini"
        } else {
            return;
        };
        let section = self.class_path(class);
        if let Some(c) = self.config.as_mut() {
            for (k, v) in c.section_pairs(&section, default) {
                c.set_string(&section, &k, &v, &file);
            }
        }
        self.reload_class_config(class);
        // Not in the original: the gamepad's aim assist is kept (aim_device.rs).
        if file.eq_ignore_ascii_case("User") {
            self.aim_config_reloaded();
        }
    }

    /// LoadConfig (Propagate = 1: the parent class first) of the default values of the subclasses of `class`, then of all the objects of
    /// these classes (end of UObject::ResetConfig; for SetUserConfig).
    pub(crate) fn reload_class_config(&mut self, class: crate::value::ClassId) {
        for c in 0..self.classes.len() {
            if self.is_child_of(c, class) {
                let cdo = self.classes[c].cdo;
                let mut values = self.objects[cdo].as_ref().map(|o| o.props.clone()).unwrap_or_default();
                values.with_slice(|v| self.load_config_propagates(c, v, None));
                if let Some(o) = self.objects[cdo].as_mut() {
                    o.props = values;
                }
            }
        }
        for o in 0..self.objects.len() {
            let Some(c) = self.objects[o].as_ref().map(|x| x.class) else { continue };
            if self.is_cdo(o) || !self.is_child_of(c, class) {
                continue;
            }
            let per_object = (self.classes[c].flags & crate::config::CLASS_PER_OBJECT_CONFIG != 0).then(|| {
                let name = self.objects[o].as_ref().map(|x| x.name.clone()).unwrap_or_default();
                (name, self.classes[c].config_name.clone())
            });
            let mut values = self.objects[o].as_ref().map(|x| x.props.clone()).unwrap_or_default();
            values.with_slice(|v| self.load_config_propagates(c, v, per_object.as_ref().map(|(n, p)| (n.as_str(), p.as_str()))));
            if let Some(x) = self.objects[o].as_mut() {
                x.props = values;
            }
        }
    }

    /// UClient::Exec: BRIGHTNESS, CONTRAST, GAMMA (window values, "+" to step).
    fn client_exec(&mut self, command: &str) -> bool {
        self.client_exec_message(command).0
    }

    /// UClient::Exec and the message it sends to the player of the first viewport (APlayerController::eventClientMessage): "Brightness %i" / "Contrast %i" (value × 10 truncated, _ftol2) or "Gamma %1.1f".
    /// The message is returned even without a player (it is then not sent).
    pub(crate) fn client_exec_message(&mut self, command: &str) -> (bool, Option<String>) {
        fn word_key(word: &str) -> &'static str {
            match word {
                "BRIGHTNESS" => "Brightness",
                "CONTRAST" => "Contrast",
                _ => "Gamma",
            }
        }
        for (word, idx) in [("BRIGHTNESS", 0usize), ("CONTRAST", 1), ("GAMMA", 2)] {
            if let Some(rest) = parse_command(command, word) {
                let v = &mut self.client_color[idx];
                // (default, "+" up to, back to, bounds of the given value).
                let (default, step_max, wrap, min, max) = [(0.5f32, 0.9f32, 0.0f32, 0.0f32, 1.0f32), (0.5, 0.9, 0.0, 0.0, 2.0), (1.7, 2.4, 0.5, 0.5, 2.5)][idx];
                if rest.starts_with('+') {
                    *v = if *v < step_max { *v + 0.1 } else { wrap };
                } else if rest.is_empty() {
                    *v = default;
                } else {
                    let chars: Vec<char> = rest.chars().collect();
                    *v = crate::config::atof(&chars, 0).clamp(min, max);
                }
                // Then UpdateGamma (UEngine vt 0x7C → UClient::UpdateGamma:
                // RenDev->UpdateGamma of each viewport, display) and SaveConfig(CPF_Config) of the
                // client ([WinDrv.WindowsClient], "%f").
                let v = *v;
                let text = format!("{v:.6}");
                if let Some(c) = self.config.as_mut() {
                    c.set_string("WinDrv.WindowsClient", word_key(word), &text, "System");
                }
                // End of UClient::Exec: with a Viewports(0)->Actor,
                // APawn::eventClientMessage "Brightness %i" / "Contrast %i"
                // (value × 10 truncated, _ftol2) or "Gamma %1.1f", Type None.
                let text = match idx {
                    0 => format!("Brightness {}", (v * 10.0) as i32),
                    1 => format!("Contrast {}", (v * 10.0) as i32),
                    _ => format!("Gamma {:.1}", v as f64),
                };
                let pc = self.engine.client.viewports.first().copied().and_then(|s| self.find_viewport_object(s)).and_then(|vp| self.propr(vp, "Actor").as_object());
                if let Some(pc) = pc.filter(|&pc| self.alive(pc)) {
                    self.event_pub(pc, "ClientMessage", vec![Value::Str(text.clone()), Value::Name(crate::value::NameStruct::from(String::new()))]);
                }
                return (true, Some(text));
            }
        }
        (false, None)
    }

    /// What SETRES does to the game rather than to the window: UD3DRenderDevice::SetRes asks for
    /// a precaching frame (PrecacheOnFlip, whose drawing places every actor's attachments) and
    /// places the weather points again (three appFrand each).
    fn setres_game_side(&mut self) {
        self.precache_on_flip = true;
        self.rnd_cube_set_res();
    }

    /// UWindowsViewport::Exec after UViewport::Exec: window commands
    /// that answer in the FOutputDevice (ConsoleCommand's FStringOutputDevice).
    pub(crate) fn windows_viewport_exec(&mut self, command: &str) -> Option<String> {
        let word = command.split_whitespace().next().unwrap_or("");
        let rest = command.trim_start().get(word.len()..).unwrap_or("").trim_start();
        // Improved build: LOCALPLAYERS [n], players on this screen (split_versus.rs).
        let command_word = word.to_ascii_uppercase();
        if let Some(out) = crate::improved::navigation::local_players_command(self, &command_word, rest) {
            return Some(out);
        }
        if let Some(out) = self.split_screen_exec(&command_word, rest) {
            return Some(out);
        }
        Some(match word.to_ascii_uppercase().as_str() {
            "GETCURRENTRES" => {
                let (w, h) = self.viewport_size_x_y();
                // Improved build, local split screen (coop.rs, split_versus.rs): the menus ask for
                // the screen resolution (GUIPage.CheckResolution) to switch to 640x480 below it;
                // the player's half (800x300 in an 800x600 window) is not the screen, and taking
                // it shrank the window to 640x480 at the first pause.
                let (w, h) = if self.local_split() && self.engine.client.window_open { self.client_size } else { (w, h) };
                format!("{w}x{h}")
            }
            // ColorBytes (0: 2) × 8.
            "GETCURRENTCOLORDEPTH" => "32".into(),
            "GETCOLORDEPTHS" => "32".into(),
            "GETCURRENTRENDERDEVICE" => "D3DDrv.D3DRenderDevice".into(),
            // UWindowsViewport::Exec: ToggleFullscreen;
            // ENDFULLSCREEN only in fullscreen. The display layer does the toggle.
            "TOGGLEFULLSCREEN" => {
                self.pending_toggle_fullscreen = true;
                String::new()
            }
            "ENDFULLSCREEN" => {
                if self.fullscreen {
                    self.pending_toggle_fullscreen = true;
                }
                String::new()
            }
            // SETRES WxH: SetRenDevRes keeping fullscreen or windowed mode (nothing if one
            // dimension is zero); the display layer resizes the window.
            "SETRES" => {
                if let Some((w, h)) = setres_size(&command.trim_start()[word.len()..]) {
                    // Improved build: the Video page accepted with Borderless (its Resolution
                    // line greyed): the window covers the desktop, the saved sizes stay; what
                    // SETRES does to the game itself still happens.
                    if crate::improved::navigation::setres_keeps_sizes(self) {
                        self.setres_game_side();
                        return Some(String::new());
                    }
                    // UWindowsViewport::ResizeViewport: width rounded to an even
                    // number, size kept in [WinDrv.WindowsClient] (fullscreen or windowed).
                    // SetRenDevRes: in windowed mode, the size is brought to a standard resolution.
                    let (mut w, mut h) = (w, h);
                    // Improved build: a resolution of the monitor offered by the Video menu
                    // (16:9, 16:10...) is kept as it is, and so is the window's own size, which
                    // the menu offers too (a launcher's forced 1024x576: accepting the page
                    // after changing another line keeps the window as it is).
                    let offered = crate::improved::navigation::resolution_offered(self, w, h);
                    if !self.fullscreen && !offered {
                        self.res_table.choose(&mut w, &mut h, self.desktop);
                    }
                    let w = (w + 1) & !1;
                    self.pending_resize = Some((w as u32, h as u32));
                    self.setres_game_side();
                    let (kx, ky) = if self.fullscreen { ("FullscreenViewportX", "FullscreenViewportY") } else { ("WindowedViewportX", "WindowedViewportY") };
                    if let Some(c) = self.config.as_mut() {
                        c.set_string("WinDrv.WindowsClient", kx, &w.to_string(), "System");
                        c.set_string("WinDrv.WindowsClient", ky, &h.to_string(), "System");
                    }
                }
                String::new()
            }
            _ => return None,
        })
    }
}

#[path = "engine_post_edit.rs"]
mod engine_post_edit;

/// Settings pages' console commands reaching the running game (Audio and Controls pages).
/// Negative control: without `input_set_exec` or `hx_global_set_property` these fail.
#[cfg(test)]
mod settings_exec_tests {
    use crate::Vm;

    fn vm_with_user_ini(tag: &str, ini: &str) -> (Vm, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("oxiii-settings-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("User.ini"), ini).unwrap();
        let mut vm = Vm::new();
        vm.config = Some(crate::config::ConfigCache::with_user_dir(dir.clone(), None));
        (vm, dir)
    }

    #[test]
    fn set_input_changes_the_binding_used_and_saved() {
        let (mut vm, dir) = vm_with_user_ini("input", "[Engine.Input]\r\nSpace=Jump\r\nE=Use\r\n");
        let mut out = String::new();
        // The Controls page: the old key cleared, the new one set, any case of the key name.
        assert!(vm.static_exec("SET Input Space", &mut out));
        assert!(vm.static_exec("SET Input f Jump", &mut out));
        let mut get = |vm: &mut Vm, key: &str| {
            let mut s = String::new();
            vm.input_exec(&format!("KEYBINDING {key}"), &mut s);
            s
        };
        assert_eq!(get(&mut vm, "Space"), "");
        assert_eq!(get(&mut vm, "F"), "Jump");
        assert_eq!(get(&mut vm, "E"), "Use", "other keys untouched");
        // OptimizeInputBindings compiles again from the configuration: F jumps, Space no longer.
        let user = vm.config.as_mut().unwrap().file_text("User");
        let b = crate::input::Bindings::parse(&user);
        assert_eq!(b.command("F").as_deref(), Some("Jump"));
        assert_eq!(b.command("Space"), None);
        // An unknown key name changes nothing.
        assert!(vm.static_exec("SET Input NotAKey Fire", &mut out));
        assert_eq!(get(&mut vm, "F"), "Jump");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_audio_settings_reach_the_running_subsystem() {
        use openxiii_hx::subsystem::{Config, HxAudio};
        let (mut vm, dir) = vm_with_user_ini("audio", "[HXAudio.HXAudioSubsystem]\r\nMasterVolume=0.000000\r\nMusicSliderPos=2\r\n");
        vm.audio = Some(HxAudio::new(openxiii_hx::Engine::new(), Config::default()));
        let mut out = String::new();
        assert!(vm.static_exec("set HXAudio.HXAudioSubsystem MusicSliderPos 0", &mut out));
        assert!(vm.static_exec("set HXAudio.HXAudioSubsystem MasterVolume -9", &mut out));
        let hx = vm.audio.as_mut().unwrap();
        assert_eq!(hx.config.music_slider_pos, 0);
        assert_eq!(hx.config.master_volume, -9.0);
        // The next map's LoadSounds applies the settings again: the chosen ones stay.
        hx.post_edit_change();
        assert_eq!((hx.config.music_slider_pos, hx.config.master_volume), (0, -9.0));
        // The Audio page reads them back when it opens.
        let mut s = String::new();
        vm.static_exec("get HXAudio.HXAudioSubsystem MasterVolume", &mut s);
        assert_eq!(s.parse::<f32>().ok(), Some(-9.0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
