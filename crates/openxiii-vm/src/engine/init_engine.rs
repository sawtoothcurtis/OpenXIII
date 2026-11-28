//! InitEngine of XIII.exe (pieces; UnEngineWin.h): what the
//! game's startup decides and writes before the first Tick. FirstRun ([FirstRun] of the system .ini, forced to 0 by
//! -FirstRun): below 220, each save "..\Save\*.usa" gets the name "Saved game" in
//! [UnrealShare.UnrealSlotMenu] SlotNames[i] of User.ini if it is empty; below 829, the render driver goes back to
//! D3DDrv.D3DRenderDevice; FirstRun is then rewritten to at least 829. "consolecommand=" and "testrendev=" on the
//! command line are tools that stop before the engine; -safe, readini and -changevideo open
//! the configuration wizard. The engine is the class ini:Engine.Engine.GameEngine (EditorEngine in the editor),
//! constructed then Init; with no window open after Init, it is destroyed and the game stops.

use crate::app_init::parse_param;
use crate::value::{app_strfind, parse_fstring};

/// What the startup environment gives InitEngine.
#[derive(Clone, Debug, Default)]
pub struct InitEngineEnv<'a> {
    /// [FirstRun] FirstRun of the system .ini (None: key missing, GetInt leaves 0).
    pub first_run: Option<i32>,
    pub cmd_line: &'a str,
    /// Names of the files found by FindFiles("..\Save\*.usa").
    pub saves: &'a [String],
    pub editor: bool,
    pub client: bool,
    /// GIsWindowOpened after UGameEngine::Init.
    pub window_opened: bool,
    /// Return value of the wizard's DoModal (0: cancelled).
    pub wizard_ok: bool,
    /// Init of the render driver tried by testrendev=.
    pub device_ok: bool,
}

/// One action of InitEngine, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// GConfig->GetInt("FirstRun", "FirstRun", v, NULL).
    GetFirstRun,
    /// GFileManager->FindFiles("..\Save\*.usa", 1, 0).
    FindSaves,
    /// GetStr / SetString of [UnrealShare.UnrealSlotMenu] SlotNames[i] in User.ini.
    GetSlotName(i32),
    SetSlotName(i32),
    /// GExec->Exec(command, GLog).
    Exec(String),
    /// testrendev=: LoadClass of the driver, SetInt(driver, "DescFlags", 2), Flush(0), construction, Init (vtable +0x74)
    /// and destruction if it fails, then the Detected.ini file.
    Detect { device: String, ok: bool },
    /// SetString("Engine.Engine", "GameRenderDevice", "D3DDrv.D3DRenderDevice", NULL).
    ResetRenderDevice,
    /// Wizard shown at this page ("safe" or "video"); DoModal and, if it is accepted, the logo screen.
    Wizard { page: &'static str, ok: bool },
    /// GFileManager->CreateFileWriter("Running.ini") then its destruction.
    RunningIni,
    /// GConfig->SetInt("FirstRun", "FirstRun", v, NULL).
    SetFirstRun(i32),
    /// StaticLoadClass(ini:Engine.Engine.GameEngine / EditorEngine, LOAD_NoFail), ConstructObject, Init.
    Engine { class: &'static str },
    /// No window open: the engine is destroyed.
    DestroyEngine,
}

/// appAtoi (MSVCR70's atoi): blanks, sign, digits.
pub(crate) fn atoi(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0B', '\x0C']);
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i32 = 0;
    for c in s.bytes().take_while(u8::is_ascii_digit) {
        v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// InitEngine: actions in order and returned engine (false: InitEngine returns NULL and WinMain stops). `slot_name`
/// reads SlotNames[i] of User.ini.
pub fn init_engine(env: &InitEngineEnv, mut slot_name: impl FnMut(i32) -> String) -> (Vec<Action>, bool) {
    let mut out = vec![Action::GetFirstRun];
    let mut first_run = env.first_run.unwrap_or(0);
    if parse_param(env.cmd_line, "FirstRun") {
        first_run = 0;
    }
    if first_run < 220 {
        out.push(Action::FindSaves);
        for name in env.saves {
            let i = atoi(name.get(4..).unwrap_or(""));
            out.push(Action::GetSlotName(i));
            if slot_name(i).is_empty() {
                out.push(Action::SetSlotName(i));
            }
        }
    }
    if let Some(cmd) = parse_fstring(env.cmd_line, "consolecommand=") {
        out.push(Action::Exec(cmd));
        return (out, false);
    }
    if let Some(device) = parse_fstring(env.cmd_line, "testrendev=") {
        out.push(Action::Detect { device, ok: env.device_ok });
        return (out, false);
    }
    if !env.editor && env.client {
        let page = if parse_param(env.cmd_line, "safe") || app_strfind(env.cmd_line, "readini").is_some() {
            Some("safe")
        } else if first_run < 829 {
            out.push(Action::ResetRenderDevice);
            None
        } else if parse_param(env.cmd_line, "changevideo") {
            Some("video")
        } else {
            None
        };
        if let Some(page) = page {
            out.push(Action::Wizard { page, ok: env.wizard_ok });
            if !env.wizard_ok {
                return (out, false);
            }
        }
    }
    out.push(Action::RunningIni);
    out.push(Action::SetFirstRun(first_run.max(829)));
    out.push(Action::Engine { class: if env.editor { "EditorEngine" } else { "GameEngine" } });
    if !env.editor && !env.window_opened {
        out.push(Action::DestroyEngine);
        return (out, false);
    }
    (out, true)
}

impl crate::Vm {
    /// What InitEngine writes to the configuration when the game starts in a window (WinMain → InitEngine, before the
    /// engine): slot names of the saves in "..\Save" below FirstRun 220, render driver below 829,
    /// FirstRun raised to 829. The wizard (-safe, readini, -changevideo) and the consolecommand= / testrendev= tools
    /// are not reproduced (platform and tools). The changed files are written by the next flush
    /// of the cache, as in the original.
    pub fn init_engine_config(&mut self) -> Vec<Action> {
        let Some(c) = self.config.as_mut() else { return Vec::new() };
        // GetInt: the value read by appAtoi if the key exists, otherwise 0.
        let first_run = c.get_string("FirstRun", "FirstRun", "System").map(|v| atoi(&v));
        // FindFiles("..\Save\*.usa") from System: names in FindFirstFile order (NTFS, case-insensitive).
        let mut saves: Vec<String> = std::fs::read_dir(c.dir().join("..").join("Save"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().ends_with(".usa"))
            .collect();
        saves.sort_by_key(|n| n.to_ascii_lowercase());
        let cmd = crate::config::app_cmd_line();
        let env = InitEngineEnv { first_run, cmd_line: &cmd, saves: &saves, editor: false, client: true, window_opened: true, wizard_ok: true, device_ok: false };
        let (actions, _) = init_engine(&env, |i| c.get_string("UnrealShare.UnrealSlotMenu", &format!("SlotNames[{i}]"), "User").unwrap_or_default());
        for a in &actions {
            match a {
                Action::SetSlotName(i) => c.set_string("UnrealShare.UnrealSlotMenu", &format!("SlotNames[{i}]"), "Saved game", "User"),
                Action::ResetRenderDevice => c.set_string("Engine.Engine", "GameRenderDevice", "D3DDrv.D3DRenderDevice", "System"),
                Action::SetFirstRun(v) => c.set_string("FirstRun", "FirstRun", &v.to_string(), "System"),
                _ => {}
            }
        }
        actions
    }
}

