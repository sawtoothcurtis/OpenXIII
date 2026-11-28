//! Player input reimplemented from Engine.dll and WinDrv.dll: keys and mouse go through
//! UInput's compiled bindings (Button, Pulse, Toggle, Axis, OnRelease, commands and aliases),
//! ReadInput accumulates the axes of held keys, UPlayerInput::DealWithPlayerInputEvent derives
//! aForward, aStrafe, aTurn, aLookUp, bRun and bDuck from them, then APlayerController::Tick calls the
//! script's PlayerTick (movement, rotation, jump) before resetting the axes to zero.
//!
//! Key codes: EInputKey (Interactions.uc), the Windows virtual key codes.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::input::Bindings;
use crate::value::{ObjId, Value};
use crate::Vm;

/// UInput actions (EInputAction). UInput::SetInputAction stores the action and its
/// duration at + 0xEA8 / + 0xEAC, read back by GetInputAction and GetInputDelta;
/// here they are passed as parameters down to the bound commands.
pub const IST_PRESS: u8 = 1;
pub const IST_HOLD: u8 = 2;
pub const IST_RELEASE: u8 = 3;
pub const IST_AXIS: u8 = 4;
/// Typed character (WM_CHAR): not a UInput action, a marker in the event queue.
pub const IST_TYPE: u8 = 0xFF;

/// IK_MouseX, IK_MouseY.
pub const IK_MOUSE_X: u8 = 0xE4;
pub const IK_MOUSE_Y: u8 = 0xE5;

/// Compiled command (FInputCmd and its subclasses, UInput::CompileCommand).
#[derive(Debug, Clone)]
enum Cmd {
    /// 1 on press, 0 on release.
    Button(String),
    /// 1 on press.
    Pulse(String),
    /// Bit 0x80 inverted on press.
    Toggle(String),
    /// (built: Speed +8, SpeedBase +0xC, DeadZone +0x10, Invert
    /// +0x18, integer; Factor +0x14, unused); UInput::DirectAxis replaces Speed with the value
    /// of the gamepad axis.
    Axis { var: String, speed: f32, speed_base: f32, invert: i32, dead_zone: f32, #[allow(dead_code)] factor: f32 },
    /// Command run on release.
    OnRelease(String),
    /// Command run on press.
    Exec(String),
    /// Commands of an alias, with the current action (ExecAliasOptimizedCommand).
    Alias(usize),
}

/// Size of the DirectInput mouse buffer (DIPROP_BUFFERSIZE).
pub const DI_MOUSE_BUFFER: usize = 1024;

/// DirectInput mouse entry: X and Y axes, wheel (and its presses), buttons.
pub(crate) fn is_mouse_device(key: u8) -> bool {
    matches!(key, 1 | 2 | 4 | 0xE4 | 0xE5 | 0xE7 | 0xEC | 0xED)
}

/// UInput state for the local player.
#[derive(Debug, Default)]
pub struct InputSystem {
    /// Commands per key code (0…254).
    keys: Vec<Vec<Cmd>>,
    /// Commands of each alias (at most 40, in User.ini order).
    aliases: Vec<Vec<Cmd>>,
    /// +0xEB0: pressed keys.
    down: Vec<bool>,
    /// Events received since the last frame: (key, action, value).
    pub(crate) queue: Vec<(u8, u8, f32)>,
    /// Input properties (CPF_Input) of the controller: axes (floats) and buttons (bytes).
    axes: Vec<String>,
    buttons: Vec<String>,
    /// Controller of the local player (Viewport->Actor).
    pub(crate) pc: ObjId,
    ready: bool,
    /// Lockstep co-op (lockstep.rs): rank of the player whose own bindings built this UInput.
    pub(crate) profile: Option<usize>,
}

/// ParseLine (Core, without Exact): one line up to `|`, CR or LF (at most `max` − 1 characters;
/// the rest stays for the next call), `|` inside quotes kept, `//` comment skipped up to
/// the end of the line; the `|`, CR and LF that follow are eaten. Nothing when the text is exhausted
/// without a character read.
pub(crate) fn parse_line(s: &[char], pos: &mut usize, max: usize) -> Option<Vec<char>> {
    let at = |i: usize| s.get(i).copied().unwrap_or('\0');
    let (mut out, mut got, mut quoted, mut ignore) = (Vec::new(), false, false, false);
    let mut left = max;
    while !matches!(at(*pos), '\0' | '\n' | '\r') {
        left -= 1;
        if left == 0 {
            break;
        }
        if !quoted && at(*pos) == '/' && at(*pos + 1) == '/' {
            ignore = true;
        }
        if !quoted && at(*pos) == '|' {
            break;
        }
        quoted ^= at(*pos) == '"';
        got = true;
        if !ignore {
            out.push(at(*pos));
        }
        *pos += 1;
    }
    while matches!(at(*pos), '\n' | '\r' | '|') {
        *pos += 1;
    }
    (at(*pos) != '\0' || got).then_some(out)
}

/// ParseLine with Exact (Core, 4th argument true): neither `|` nor `//` comment: one line up to CR or LF (at most `max` − 1 characters); the CR and LF that
/// follow are eaten. Nothing when the text is exhausted without a character read.
pub(crate) fn parse_line_exact(s: &[char], pos: &mut usize, max: usize) -> Option<Vec<char>> {
    let at = |i: usize| s.get(i).copied().unwrap_or('\0');
    let (mut out, mut got) = (Vec::new(), false);
    let mut left = max;
    while !matches!(at(*pos), '\0' | '\n' | '\r') {
        left -= 1;
        if left == 0 {
            break;
        }
        got = true;
        out.push(at(*pos));
        *pos += 1;
    }
    while matches!(at(*pos), '\n' | '\r') {
        *pos += 1;
    }
    (at(*pos) != '\0' || got).then_some(out)
}

/// ParseCommand (Core): spaces and tabs skipped (even on failure), then the word
/// case-insensitively, followed by a non-alphanumeric character, and the spaces that follow it.
pub(crate) fn parse_command(s: &[char], pos: &mut usize, word: &str) -> bool {
    while matches!(s.get(*pos), Some(' ' | '\t')) {
        *pos += 1;
    }
    let w: Vec<char> = word.chars().collect();
    let found = s.len() >= *pos + w.len() && s[*pos..*pos + w.len()].iter().zip(&w).all(|(a, b)| a.eq_ignore_ascii_case(b));
    if !found || s.get(*pos + w.len()).is_some_and(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    *pos += w.len();
    while matches!(s.get(*pos), Some(' ' | '\t')) {
        *pos += 1;
    }
    true
}

/// ParseToken (Core, without escapes): word after the spaces, or text in quotes; at most
/// `max` − 1 characters kept.
pub(crate) fn parse_token(s: &[char], pos: &mut usize, max: usize) -> Option<String> {
    while matches!(s.get(*pos), Some(' ' | '\t')) {
        *pos += 1;
    }
    let mut out = String::new();
    let mut len = 0;
    if s.get(*pos) == Some(&'"') {
        *pos += 1;
        while let Some(&c) = s.get(*pos).filter(|&&c| c != '"' && c != '\0') {
            if len + 1 >= max {
                break;
            }
            out.push(c);
            len += 1;
            *pos += 1;
        }
        if s.get(*pos) == Some(&'"') {
            *pos += 1;
        }
    } else {
        while let Some(&c) = s.get(*pos).filter(|&&c| !matches!(c, ' ' | '\t' | '\0')) {
            if len + 1 < max {
                out.push(c);
                len += 1;
            }
            *pos += 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// appStrfind (Core): `find` case-insensitively, not preceded by a letter or a digit; position
/// after the pattern.
fn app_strfind(s: &[char], find: &str) -> Option<usize> {
    let f: Vec<char> = find.chars().collect();
    let mut alnum = false;
    for i in 0..s.len() {
        let c = s[i].to_ascii_uppercase();
        if c == '\0' {
            break;
        }
        if !alnum && s.len() >= i + f.len() && s[i..i + f.len()].iter().zip(&f).all(|(a, b)| a.eq_ignore_ascii_case(b)) {
            return Some(i + f.len());
        }
        alnum = c.is_ascii_uppercase() || c.is_ascii_digit();
    }
    None
}

/// Whitespace skipped by atof / atoi (isspace).
fn skip_space(s: &[char]) -> usize {
    s.iter().take_while(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')).count()
}

/// atof (msvcr70): numeric prefix (sign, digits, dot, complete exponent) read as a double then
/// stored in the float; 0 without a digit.
fn atof(s: &[char]) -> f32 {
    let start = skip_space(s);
    let mut i = start;
    if matches!(s.get(i), Some('+' | '-')) {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let d = *i;
        while s.get(*i).is_some_and(|c| c.is_ascii_digit()) {
            *i += 1;
        }
        *i - d
    };
    let mut n = digits(&mut i);
    if s.get(i) == Some(&'.') {
        i += 1;
        n += digits(&mut i);
    }
    if n == 0 {
        return 0.0;
    }
    let mantissa = i;
    if matches!(s.get(i), Some('e' | 'E')) {
        i += 1;
        if matches!(s.get(i), Some('+' | '-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            i = mantissa;
        }
    }
    let t: String = s[start..i].iter().collect();
    t.parse::<f64>().map(|v| v as f32).unwrap_or(0.0)
}

/// atoi (msvcr70): sign and digits after the whitespace, without overflow checking.
pub(crate) fn atoi(s: &[char]) -> i32 {
    let mut i = skip_space(s);
    let neg = s.get(i) == Some(&'-');
    if matches!(s.get(i), Some('+' | '-')) {
        i += 1;
    }
    let mut v: i32 = 0;
    while let Some(d) = s.get(i).and_then(|c| c.to_digit(10)) {
        v = v.wrapping_mul(10).wrapping_add(d as i32);
        i += 1;
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

impl InputSystem {

    /// UInput::CompileCommand: lines from ParseLine (0x100 buffer); BUTTON, PULSE,
    /// TOGGLE and AXIS followed by a word (ParseToken, 0x40) that names an input property
    /// (FindButtonOffset, FindAxisOffset), otherwise "Bad Button command";
    /// axis parameters through Parse (SPEED=, SPEEDBASE=, INVERT= integer, DEADZONE=, FACTOR=);
    /// OnRelease; for a key binding (`key_binding`), an alias named by the first word
    /// (IsAnAlias); otherwise the line is a command. Also returns the log lines.
    fn compile(&self, text: &str, alias_names: &[String], key_binding: bool) -> (Vec<Cmd>, Vec<String>) {
        let (mut out, mut log) = (Vec::new(), Vec::new());
        let chars: Vec<char> = text.chars().collect();
        let mut at = 0;
        while let Some(line) = parse_line(&chars, &mut at, 0x100) {
            let mut p = 0;
            let bad = format!("Log: Bad Button command: {}", line.iter().collect::<String>());
            let mut button = |p: &mut usize, make: fn(String) -> Cmd| match parse_token(&line, p, 0x40).and_then(|v| self.button_name(&v)) {
                Some(v) => out.push(make(v)),
                None => log.push(bad.clone()),
            };
            if parse_command(&line, &mut p, "BUTTON") {
                button(&mut p, Cmd::Button);
            } else if parse_command(&line, &mut p, "PULSE") {
                button(&mut p, Cmd::Pulse);
            } else if parse_command(&line, &mut p, "TOGGLE") {
                button(&mut p, Cmd::Toggle);
            } else if parse_command(&line, &mut p, "AXIS") {
                match parse_token(&line, &mut p, 0x40).and_then(|v| self.axis_name(&v)) {
                    Some(var) => {
                        let rest = &line[p..];
                        let float = |k: &str, d: f32| app_strfind(rest, k).map_or(d, |i| atof(&rest[i..]));
                        out.push(Cmd::Axis {
                            var,
                            speed: float("SPEED=", 1.0),
                            speed_base: float("SPEEDBASE=", 0.0),
                            invert: app_strfind(rest, "INVERT=").map_or(1, |i| atoi(&rest[i..])),
                            dead_zone: float("DEADZONE=", 0.0),
                            factor: float("FACTOR=", 1.0),
                        });
                    }
                    None => log.push(bad),
                }
            } else if parse_command(&line, &mut p, "OnRelease") {
                out.push(Cmd::OnRelease(line[p..].iter().collect()));
            } else {
                // IsAnAlias: FName of the first word (FNAME_Find, case-insensitive) among the 40 aliases.
                let mut q = p;
                let alias = parse_token(&line, &mut q, 0x40)
                    .filter(|_| key_binding)
                    .and_then(|w| alias_names.iter().position(|a| !a.eq_ignore_ascii_case("None") && a.eq_ignore_ascii_case(&w)));
                match alias {
                    Some(i) => out.push(Cmd::Alias(i)),
                    None => out.push(Cmd::Exec(line[p..].iter().collect())),
                }
            }
        }
        (out, log)
    }

    /// UInput::Init, last line: "Input system initialized for %s" (NAME_Init), with the viewport's name.
    pub(crate) fn init_log(viewport: &str) -> String {
        format!("Init: Input system initialized for {viewport}")
    }

    /// UInput::Init, compilation part: the controller's input properties
    /// (GetButtonAndAxisOffsets: floats = axes, bytes = buttons), the non-empty aliases
    /// (OptimizeAliases, name other than None) then the non-empty key bindings
    /// (OptimizeBindings, in code order), compiled by CompileCommand; also returns
    /// the compilation's log lines.
    pub(crate) fn build(pc: ObjId, buttons: Vec<String>, axes: Vec<String>, bindings: &Bindings) -> (InputSystem, Vec<String>) {
        let mut sys = InputSystem { down: vec![false; 256], keys: vec![Vec::new(); 255], pc, buttons, axes, ..Default::default() };
        let log = sys.compile_bindings(bindings);
        (sys, log)
    }

    /// Button property (CPF_Input byte) named `v`, case-insensitive.
    fn button_name(&self, v: &str) -> Option<String> {
        self.buttons.iter().find(|b| b.eq_ignore_ascii_case(v)).cloned()
    }

    /// Axis property (CPF_Input float) named `v`, case-insensitive.
    fn axis_name(&self, v: &str) -> Option<String> {
        self.axes.iter().find(|a| a.eq_ignore_ascii_case(v)).cloned()
    }
}

impl InputSystem {
    /// OptimizeAliases then OptimizeBindings: the aliases of the non-empty slots (the name is not
    /// NAME_None) then the non-empty bindings of the 255 keys, in order; returns the log lines.
    pub(crate) fn compile_bindings(&mut self, bindings: &Bindings) -> Vec<String> {
        let alias_names: Vec<String> = bindings.alias_list().iter().map(|(n, _)| n.clone()).collect();
        let mut log = Vec::new();
        for (n, c) in bindings.alias_list() {
            if n.eq_ignore_ascii_case("None") {
                self.aliases.push(Vec::new());
                continue;
            }
            let (cmds, l) = self.compile(c, &alias_names, false);
            self.aliases.push(cmds);
            log.extend(l);
        }
        for (code, name) in KEY_NAMES.iter().enumerate() {
            if let Some(text) = bindings.command(name) {
                let (cmds, l) = self.compile(text, &alias_names, true);
                self.keys[code] = cmds;
                log.extend(l);
            }
        }
        log
    }
}

/// UInput::GetButtonAndAxisOffsets: the CPF_Input properties (flag 4) of the controller's class and of its
/// parent classes, floats = axes, bytes = buttons (names returned: axes, buttons).
pub(crate) fn input_properties(props: &[crate::types::PropDef]) -> (Vec<String>, Vec<String>) {
    let (mut axes, mut buttons) = (Vec::new(), Vec::new());
    for p in props {
        if p.flags & 4 == 0 {
            continue;
        }
        match p.ty {
            crate::types::Type::Float => axes.push(p.name.clone()),
            crate::types::Type::Byte => buttons.push(p.name.clone()),
            _ => {}
        }
    }
    (axes, buttons)
}

impl Vm {
    /// UInput::Init / OptimizeInputBindings: input properties of the controller
    /// (GetButtonAndAxisOffsets: CPF_Input properties, floats = axes, bytes =
    /// buttons), then the 40 aliases (OptimizeAliases) and the 255 bindings
    /// (OptimizeBindings) compiled.
    pub fn input_init(&mut self, pc: ObjId, bindings: &Bindings) {
        let Some(class) = self.objects[pc].as_ref().map(|o| o.class) else { return };
        let (axes, buttons) = input_properties(&self.classes[class].props);
        // Lockstep co-op: a player's keys are their own, on every machine.
        let own = self.lockstep_profile_bindings(pc);
        let bindings = own.as_ref().map_or(bindings, |o| &o.1);
        let (mut sys, log) = InputSystem::build(pc, buttons, axes, bindings);
        self.log.extend(log);
        sys.ready = true;
        sys.profile = own.map(|o| o.0);
        // Each viewport has its UInput (UViewport::InitInput): the first viewport's receives
        // the window's events (input_sys), those of the other local players are separate.
        let primary = self.engine.client.viewports.first().copied();
        let slot = self.viewport_slot_of(pc);
        if slot.is_some() && primary.is_some() && slot != primary {
            self.other_inputs.retain(|s| s.pc != pc);
            self.other_inputs.push(Box::new(sys));
        } else {
            self.input_sys = Some(Box::new(sys));
        }
    }

    /// AActor::execResetInputs: for each viewport of Client->Viewports (+0x2C), in
    /// order, UInput::ResetInput of its UInput (Viewport +0x6C, vtable +0x84). A viewport's UInput
    /// is that of the controller whose Player it is; a viewport without a UInput does nothing
    /// (the original would dereference a null pointer).
    pub(crate) fn reset_all_inputs(&mut self) {
        let slots = self.engine.client.viewports.clone();
        for slot in slots {
            self.reset_slot_input(slot);
        }
    }

    /// UInput::ResetInput of the UInput of a viewport of Client->Viewports (see reset_all_inputs).
    pub(crate) fn reset_slot_input(&mut self, slot: u8) {
        let first = self.engine.client.viewports.first().copied();
        // The main UInput without a known viewport (controller without a Player) is the first one's.
        let primary = self.input_sys.as_ref().is_some_and(|s| match self.viewport_slot_of(s.pc) {
            Some(v) => v == slot,
            None => Some(slot) == first,
        });
        if primary {
            self.reset_input();
        } else if let Some(i) = self.other_inputs.iter().position(|s| self.viewport_slot_of(s.pc) == Some(slot)) {
            let other = self.other_inputs.remove(i);
            let primary = std::mem::replace(&mut self.input_sys, Some(other));
            self.reset_input();
            let other = std::mem::replace(&mut self.input_sys, primary);
            if let Some(o) = other {
                self.other_inputs.insert(i.min(self.other_inputs.len()), o);
            }
        }
    }

    /// UInput::ResetInput: keys released, the controller's input properties (CPF_Input)
    /// set to zero, then UpdateInput(1) of the viewport (windrv_input.rs). (+0xEA8 / +0xEAC reset to
    /// zero: DirectInput state, absent here.)
    pub(crate) fn reset_input(&mut self) {
        let Some(sys) = self.input_sys.as_mut() else { return };
        sys.down.iter_mut().for_each(|d| *d = false);
        let (pc, buttons, axes) = (sys.pc, sys.buttons.clone(), sys.axes.clone());
        if self.objects.get(pc).is_none_or(|o| o.is_none()) {
            return;
        }
        for b in &buttons {
            self.set_prop(pc, b, Value::Byte(0));
        }
        for a in &axes {
            self.set_prop(pc, a, Value::Float(0.0));
        }
        self.update_input(pc, 1, 0.0);
    }

    /// Key event from the window (CauseInputEvent): handled at the start of the frame.
    pub fn input_event(&mut self, key: u8, action: u8, value: f32) {
        if let Some(sys) = self.input_sys.as_mut() {
            // Buffered DirectInput mouse (UWindowsViewport::UWindowsViewport:
            // DIPROP_BUFFERSIZE = 1024 entries), emptied only by UpdateInput (
            // GetDeviceData one entry at a time; DI_BUFFEROVERFLOW is a success, the kept entries
            // are read). When the client's tick does not read (videos, loading), the
            // buffer overflows: DirectInput keeps the oldest entries and loses the new ones.
            if is_mouse_device(key) && sys.queue.iter().filter(|e| is_mouse_device(e.0)).count() >= DI_MOUSE_BUFFER {
                return;
            }
            sys.queue.push((key, action, value));
        }
    }

    /// Key held down for UInput (simulate tools).
    pub fn key_down(&self, key: u8) -> bool {
        self.input_sys.as_ref().is_some_and(|s| s.down.get(key as usize).copied().unwrap_or(false))
    }

    /// EInputKey code of a key name ("LeftMouse", "W"…): UInput::FindKeyName
    /// looks up the FName "IK_<name>" (case-insensitive); GetKeyName returns
    /// the KEY_NAMES entry (the enumeration name without "IK_").
    /// UInput::GetKeyName: the key name without "IK_", empty outside 0..255 (or if the name has three
    /// characters or fewer).
    pub(crate) fn key_name(key: i32) -> &'static str {
        usize::try_from(key).ok().and_then(|k| KEY_NAMES.get(k)).copied().unwrap_or("")
    }

    pub fn key_code(name: &str) -> Option<u8> {
        KEY_NAMES.iter().position(|k| k.eq_ignore_ascii_case(name)).map(|i| i as u8)
    }

    /// UEngine::InputEvent: the interactions (console, menus) first; if one of them
    /// takes the event, UInput only sees the gamepad axes and buttons (200…0xD7) and the
    /// releases (PreProcess only); otherwise PreProcess then Process (bindings).
    /// UGameEngine::InputEvent before that: nothing during a reported network failure
    /// (ConnectionFailure, ReportNetFailure).
    pub(crate) fn input_process(&mut self, pc: ObjId, key: u8, action: u8, value: f32) -> bool {
        if self.ipdrv.connection_failure {
            return false;
        }
        if self.master_key_event(key, action, value) {
            if (200..=0xd7).contains(&key) || action == IST_RELEASE {
                self.input_pre_process(key, action);
            }
            return true;
        }
        if !self.input_pre_process(key, action) {
            return false;
        }
        // UInput::Process: commands bound to the key, with the current action (+0xEA8) and
        // value (+0xEAC); beyond 254, "UInput: Unsupported key id (%d)"; returns 0
        // for a key without a command.
        if key == 255 {
            self.log.push(format!("Log: UInput: Unsupported key id ({key})"));
            return false;
        }
        let Some(cmds) = self.input_sys.as_ref().and_then(|s| s.keys.get(key as usize).cloned()).filter(|c| !c.is_empty()) else {
            return false;
        };
        self.input_run(pc, &cmds, action, value, 0);
        true
    }

    /// UInput::PreProcess: pressed state of the keys; false for a press on an
    /// already pressed key or the release of a released key.
    /// UInput::KeyDown: key pressed (+0xEB0) in the current UInput.
    pub(crate) fn input_key_down(&self, key: impl Into<i32>) -> bool {
        // Signed index clamped to 0..254, including key 255.
        let key = key.into().clamp(0, 254) as usize;
        self.input_sys.as_ref().is_some_and(|s| s.down.get(key).copied().unwrap_or(false))
    }

    /// UInput::DirectAxis: each command bound to the key receives the value (Speed
    /// of axis commands), then runs as IST_Hold with `delta`.
    pub(crate) fn input_direct_axis(&mut self, pc: ObjId, key: u8, value: f32, delta: f32) {
        let Some(sys) = self.input_sys.as_mut() else { return };
        let Some(cmds) = sys.keys.get_mut(key as usize) else { return };
        if cmds.is_empty() {
            return;
        }
        for c in cmds.iter_mut() {
            if let Cmd::Axis { speed, .. } = c {
                *speed = value;
            }
        }
        let cmds = cmds.clone();
        self.input_run(pc, &cmds, IST_HOLD, delta, 0);
    }

    /// Look stick of a gamepad or of the touch controls (`look_stick`, not in the original PC
    /// version, which has no gamepad bindings): read as the console versions read the right
    /// stick, with their bindings (XIIIMenuControlsWindow.PadConfigChanged: "JoyU Axis aTurn
    /// SpeedBase=1.0 DeadZone=0.4", "JoyV Axis aLookup SpeedBase=1.0 DeadZone=0.4 INVERT=-1"),
    /// compiled by UInput::CompileCommand and run like UInput::DirectAxis (input_direct_axis) with the
    /// gamepad's duration (pad_delta). Nothing is added to the key bindings nor written to User.ini;
    /// the joystick path of PlayerInput then applies (look slowed down on a held target, as on
    /// the consoles), where the mouse path does not.
    pub(crate) fn look_stick_axes(&mut self, pc: ObjId) {
        const LOOK: [&str; 2] = ["Axis aTurn SpeedBase=1.0 DeadZone=0.4", "Axis aLookup SpeedBase=1.0 DeadZone=0.4 INVERT=-1"];
        let Some((x, y)) = self.look_stick else { return };
        let Some(sys) = self.input_sys.as_ref().filter(|s| s.pc == pc) else { return };
        let delta = crate::pad::pad_look_delta();
        // Gamepad settings (gamepad_config.rs): look speed as the bindings' SpeedBase, inversion
        // of the vertical axis. The default (1.0, false) leaves the bindings as they are.
        let (speed_factor, invert_y) = self.look_stick_tune;
        let y = if invert_y { -y } else { y };
        // The console's V axis is up positive: with bInverseLook False (DefUser.ini, the consoles'
        // invert option XIIIMenuAdvancedControlsWindow.InvPadChanged), INVERT=-1 then looks up
        // when the stick is pushed up (measured: fed down positive, the view went the wrong way).
        let all: Vec<Vec<Cmd>> = LOOK
            .into_iter()
            .zip([x, -y])
            .map(|(text, value)| {
                let (mut cmds, _) = sys.compile(text, &[], false);
                for c in cmds.iter_mut() {
                    if let Cmd::Axis { speed, speed_base, .. } = c {
                        *speed = value;
                        if speed_factor != 1.0 {
                            *speed_base *= speed_factor;
                        }
                    }
                }
                cmds
            })
            .collect();
        for cmds in all {
            self.input_run(pc, &cmds, IST_HOLD, delta, 0);
        }
    }

    fn input_pre_process(&mut self, key: u8, action: u8) -> bool {
        let Some(sys) = self.input_sys.as_mut() else { return false };
        let k = key as usize;
        if action == IST_PRESS {
            if sys.down[k] {
                return false;
            }
            sys.down[k] = true;
        } else if action == IST_RELEASE {
            if !sys.down[k] {
                return false;
            }
            sys.down[k] = false;
        }
        true
    }

    fn input_run(&mut self, pc: ObjId, cmds: &[Cmd], action: u8, delta: f32, depth: usize) {
        for cmd in cmds {
            match cmd {
                Cmd::Button(v) => {
                    if action == IST_PRESS {
                        self.set_prop(pc, v, Value::Byte(1));
                    } else if action == IST_RELEASE && self.propr(pc, v).as_int() != 0 {
                        self.set_prop(pc, v, Value::Byte(0));
                    }
                }
                Cmd::Pulse(v) => {
                    if action == IST_PRESS {
                        self.set_prop(pc, v, Value::Byte(1));
                    }
                }
                Cmd::Toggle(v) => {
                    if action == IST_PRESS {
                        let b = self.propr(pc, v).as_int() as u8;
                        self.set_prop(pc, v, Value::Byte(b ^ 0x80));
                    }
                }
                Cmd::Axis { var, speed, speed_base, invert, dead_zone, .. } => {
                    // No axis in the menus (bIsInMenu).
                    if self.propr(pc, "bIsInMenu").as_bool() {
                        continue;
                    }
                    if *speed_base > 0.0 {
                        // Gamepad axis: Speed (DirectAxis value) outside the dead zone,
                        // brought back to 0…1 beyond it: v = sign(Speed) × (|Speed| − DeadZone) /
                        // (1 − DeadZone), then sign(v) × Invert × SpeedBase × |v|;
                        // the property is replaced.
                        let sign = |x: f32| if 0.0 < x { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 };
                        let a = speed.abs();
                        if *dead_zone < a {
                            let v = sign(*speed) * (a - dead_zone) / (1.0 - dead_zone);
                            self.set_prop(pc, var, Value::Float(sign(v) * *invert as f32 * speed_base * v.abs()));
                        }
                        continue;
                    }
                    let cur = self.propr(pc, var).as_float();
                    if action == IST_AXIS {
                        self.set_prop(pc, var, Value::Float(delta * *invert as f32 * speed * 0.01 + cur));
                    } else if action == IST_HOLD {
                        self.set_prop(pc, var, Value::Float(delta * *invert as f32 * speed + cur));
                    }
                }
                Cmd::OnRelease(c) => {
                    if action == IST_RELEASE {
                        self.console_command(pc, c);
                    }
                }
                Cmd::Exec(c) => {
                    if action == IST_PRESS {
                        self.console_command(pc, c);
                    }
                }
                Cmd::Alias(i) => {
                    if depth < 8 {
                        let alias = self.input_sys.as_ref().and_then(|s| s.aliases.get(*i).cloned()).unwrap_or_default();
                        self.input_run(pc, &alias, action, delta, depth + 1);
                    }
                }
            }
        }
    }

    /// UInput::ReadInput: axes of the held keys then scaling by 20 / dt;
    /// dt = −1 resets the axes to zero.
    fn input_read(&mut self, pc: ObjId, dt: f32) {
        let Some(sys) = self.input_sys.as_mut() else { return };
        // Axis names taken out for the duration of the axis loops (which do not read input_sys again), then put back:
        // without copying the list every frame.
        if dt == -1.0 {
            let axes = std::mem::take(&mut sys.axes);
            for a in &axes {
                self.set_prop(pc, a, Value::Float(0.0));
            }
            if let Some(s) = self.input_sys.as_mut() {
                s.axes = axes;
            }
            return;
        }
        let held: Vec<u8> = (0..255u8).filter(|&k| sys.down[k as usize]).collect();
        for k in held {
            if let Some(cmds) = self.input_sys.as_ref().and_then(|s| s.keys.get(k as usize).cloned()) {
                self.input_run(pc, &cmds, IST_HOLD, dt, 0);
            }
        }
        let scale = 20.0 / dt;
        let Some(axes) = self.input_sys.as_mut().map(|s| std::mem::take(&mut s.axes)) else { return };
        for a in &axes {
            let v = self.propr(pc, a).as_float();
            self.set_prop(pc, a, Value::Float(scale * v));
        }
        if let Some(s) = self.input_sys.as_mut() {
            s.axes = axes;
        }
    }

    /// Start of the frame, before ULevel::Tick: messages from the local player's window, in their
    /// order. WM_CHAR (UEngine::Key, to the interactions only); keyboard WM_KEYDOWN /
    /// WM_SYSKEYDOWN and WM_KEYUP / WM_SYSKEYUP (UWindowsViewport::ViewportWndProc,
    /// cases 0x100/0x104 and 0x101/0x105): CauseInputEvent right away, so the
    /// bound commands (exec Grab…) act before the Tick of the frame's actors; UpdateInput's
    /// GetKeyState scan then no longer sees a difference. Mouse and gamepad stay in the
    /// queue until UpdateInput, in the controller's Tick (`player_input_device_events`).
    pub(crate) fn player_input_events(&mut self) {
        // Lockstep co-op: this machine's keys only act through its player's commands (lockstep.rs).
        // Keys that arrive while a step waits for the other machine stay queued for the next
        // command; run here, they acted on this machine only.
        if self.lockstep.is_some() {
            return;
        }
        let Some(pc) = self.input_sys.as_ref().filter(|s| s.ready).map(|s| s.pc) else { return };
        if self.objects[pc].is_none() {
            return;
        }
        let queue = self.input_sys.as_mut().map(|s| std::mem::take(&mut s.queue)).unwrap_or_default();
        let keyboard = |key: u8, action: u8| matches!(action, IST_PRESS | IST_RELEASE) && !matches!(key, 0..=7 | 0xC8..=0xD7 | 0xE0..=0xEF);
        let (now, devices): (Vec<_>, Vec<_>) = queue.into_iter().partition(|e| e.1 == IST_TYPE || keyboard(e.0, e.1));
        if let Some(s) = self.input_sys.as_mut() {
            s.queue = devices;
        }
        for (key, action, value) in now {
            if action == IST_TYPE {
                self.master_key_type(key);
            } else if self.objects[pc].is_some() {
                self.input_process(pc, key, action, value);
            }
        }
    }

    /// UWindowsViewport::UpdateInput, in the controller's Tick: buffered
    /// DirectInput mouse data (axes, wheel, buttons → CauseInputEvent) and keyboard
    /// state (GetKeyState), so the presses and releases that arrived since the previous frame.
    fn player_input_device_events(&mut self, pc: ObjId) {
        // Lockstep co-op: what is still queued arrived after this step's command was taken (the
        // step waited for the other machine): it goes into the next command, not straight here.
        if self.lockstep.is_some() {
            return;
        }
        let events = self.input_sys.as_mut().map(|s| std::mem::take(&mut s.queue)).unwrap_or_default();
        for (key, action, value) in events {
            if action == IST_TYPE {
                self.master_key_type(key);
            } else {
                self.input_process(pc, key, action, value);
            }
        }
    }

    /// Input and PlayerTick of the local player, in its APlayerController::Tick.
    pub(crate) fn player_controller_input(&mut self, pc: ObjId, dt: f32) {
        self.player_controller_input_mode(pc, dt, false);
    }

    /// ULevel::Tick while paused (0x10390D2x): for a PlayerController that has a Player, only
    /// Player->ReadInput (vtable +0x6C: UpdateInput then UInput::ReadInput) and
    /// UPlayerInput::DealWithPlayerInputEvent, then all its CPF_Input properties set to zero; no
    /// PlayerTick.
    pub(crate) fn player_controller_input_paused(&mut self, pc: ObjId, dt: f32) {
        self.player_controller_input_mode(pc, dt, true);
    }

    fn player_controller_input_mode(&mut self, pc: ObjId, dt: f32, paused: bool) {
        // Player of another viewport: its UInput (Viewport->Input) for the duration of its Tick.
        if let Some(i) = self.other_inputs.iter().position(|s| s.pc == pc) {
            let other = self.other_inputs.remove(i);
            let primary = std::mem::replace(&mut self.input_sys, Some(other));
            self.player_controller_input_current(pc, dt, paused);
            let other = std::mem::replace(&mut self.input_sys, primary);
            if let Some(o) = other {
                self.other_inputs.insert(i.min(self.other_inputs.len()), o);
            }
            return;
        }
        self.player_controller_input_current(pc, dt, paused);
    }

    fn player_controller_input_current(&mut self, pc: ObjId, dt: f32, paused: bool) {
        if self.input_sys.as_ref().is_none_or(|s| !s.ready || s.pc != pc) || self.objects[pc].is_none() {
            return;
        }
        if self.propr(pc, "PlayerInput").as_object().is_none() {
            self.event_pub(pc, "InitInputSystem", vec![]);
        }
        let Some(input) = self.propr(pc, "PlayerInput").as_object() else { return };
        // UViewport::ReadInput: UpdateInput (vt 0x9C: the viewport's gamepad, then mouse and keyboard)
        // before UInput::ReadInput.
        self.update_input(pc, 0, dt);
        self.player_input_device_events(pc);
        self.input_read(pc, dt);
        self.deal_with_player_input(input, pc, dt);
        if paused {
            // 4 zero bytes at the address of each CPF_Input property (axes and buttons).
            let (buttons, axes) = self.input_sys.as_ref().map(|s| (s.buttons.clone(), s.axes.clone())).unwrap_or_default();
            for b in &buttons {
                self.set_prop(pc, b, Value::Byte(0));
            }
            for a in &axes {
                self.set_prop(pc, a, Value::Float(0.0));
            }
            return;
        }
        // APlayerController::eventPlayerTick.
        self.event_pub(pc, "PlayerTick", vec![Value::Float(dt)]);
        self.input_read(pc, -1.0);
    }

    /// UPlayerInput::SmoothMouse.
    fn smooth_mouse(&self, input: ObjId, a_mouse: f32, delta_time: f32, smooth: &mut f32, borrowed: &mut f32) -> f32 {
        let threshold = self.propr(input, "MouseSmoothThreshold").as_float();
        smooth_mouse(self.propr(input, "bMaxMouseSmoothing").as_bool(), threshold, a_mouse, delta_time, smooth, borrowed)
    }
}

/// UPlayerInput::SmoothMouse without the object: bMaxMouseSmoothing, MouseSmoothThreshold,
/// aMouse, DeltaTime; SmoothMouse and BorrowedMouse updated; returns the smoothed value.
pub(crate) fn smooth_mouse(max_smoothing: bool, threshold: f32, a_mouse: f32, delta_time: f32, smooth: &mut f32, borrowed: &mut f32) -> f32 {
    let old = *smooth;
    if max_smoothing && a_mouse == 0.0 && delta_time < threshold {
        let v = ((threshold - delta_time) * old * 0.5) / threshold;
        *smooth = v;
        *borrowed += v;
        return old;
    }
    if *smooth != 0.0 && a_mouse != 0.0 && (0.0 < *smooth) == (0.0 < a_mouse) {
        let mut v = ((a_mouse + *smooth) - *borrowed) * 0.5;
        if (0.0 < v) != (0.0 < a_mouse) {
            v = if 0.0 < a_mouse { 1.0 } else { -1.0 };
        }
        *smooth = v;
        *borrowed = v - a_mouse;
        return v;
    }
    *smooth = a_mouse;
    *borrowed = 0.0;
    a_mouse
}

/// Dead zone of the gamepad's move stick (the consoles' bindings: DeadZone=0.4) and the push from
/// which it counts as full (sticks that do not quite reach 1.0, circular gates).
pub const MOVE_DEAD_ZONE: f32 = 0.4;
pub const MOVE_FULL: f32 = 0.9;

/// The move stick (`x` right, `y` down, −1…1) as (strafe, forward) fractions of the keys' range:
/// nothing inside the dead zone; beyond it, as much as it is pushed (its length, `MOVE_FULL` and
/// more counting as full), in its direction carried onto the keys' square, so that a full push
/// straight is one key and a full push diagonal two keys, as the keyboard gives.
pub fn analog_move(x: f32, y: f32) -> (f32, f32) {
    let m = x.dhypot(y);
    if !(m > MOVE_DEAD_ZONE) {
        return (0.0, 0.0);
    }
    let amount = ((m - MOVE_DEAD_ZONE) / (MOVE_FULL - MOVE_DEAD_ZONE)).min(1.0);
    let (dx, dy) = (x / m, -y / m);
    let side = dx.abs().max(dy.abs());
    (dx / side * amount, dy / side * amount)
}

impl Vm {
    /// The gamepad move stick of controller `pc` this frame: the first local player's
    /// (`move_stick`, given by the game window), another local player's (`move_sticks`, from
    /// coop_pad_input).
    pub(crate) fn move_stick_of(&self, pc: ObjId) -> Option<(f32, f32)> {
        // Lockstep co-op (lockstep.rs): every player's move stick is their command's.
        let first = self.lockstep.is_none() && !(self.local_split() && self.coop.players.first().is_some_and(|&p| p != pc));
        if first {
            self.move_stick
        } else {
            self.move_sticks.iter().find(|m| m.0 == pc).map(|m| m.1)
        }
    }

    /// UPlayerInput::DealWithPlayerInputEvent.
    pub(crate) fn deal_with_player_input(&mut self, input: ObjId, pc: ObjId, dt: f32) {
        let f = |vm: &Vm, n: &str| vm.propr(pc, n).as_float();
        let fi = |vm: &Vm, n: &str| vm.propr(input, n).as_float();
        let set = |vm: &mut Vm, o: ObjId, n: &str, v: f32| vm.set_prop(o, n, Value::Float(v));
        let k = dt * 59.999_996;
        let look_speed = f(self, "fLookSpeed");
        set(self, pc, "aBaseY", k * f(self, "aBaseY"));
        let mut look_up = k * look_speed * f(self, "aLookUp");
        if self.propr(pc, "bInverseLook").as_bool() {
            look_up = -look_up;
        }
        set(self, pc, "aLookUp", look_up);
        set(self, pc, "aTurn", k * look_speed * f(self, "aTurn"));
        set(self, pc, "aStrafe", k * f(self, "aStrafe"));
        let range = fi(self, "fInputRange");
        let config = self.propr(pc, "ConfigType").as_byte();
        // The statistics are state read back by the gamepad settings.
        if self.propr(input, "bCheckInputRanges").as_bool() {
            for (array, current_value, direction, y, x) in [
                ("MoveRange", "CurrentInput", "CurrentInputAngle", f(self, "aBaseY"), f(self, "aStrafe")),
                ("TurnRange", "CurrentTurnInput", "CurrentTurnInputAngle", f(self, "aTurn"), f(self, "aLookUp")),
            ] {
                // The array is modified in place (at most one element changes) instead of being copied
                // then rewritten in full; empty or missing, it becomes 150 zeros as before.
                if !matches!(self.propr(input, array), Value::Array(v) if !v.is_empty()) {
                    self.set_prop(input, array, Value::Array(vec![Value::Float(0.0); 150]));
                }
                let slot = self.prop_slot(input, array).ok();
                if config != 0 {
                    // FPATAN keeps its precision; the following FMUL rounds to single.
                    let radians = if x != 0.0 && y != 0.0 {
                        ((y / x) as f64).datan()
                    } else if x > 0.0 {
                        0.0
                    } else if x < 0.0 {
                        f32::from_bits(0x40490fdb) as f64
                    } else if y > 0.0 {
                        f32::from_bits(0x3fc90fdb) as f64
                    } else {
                        f32::from_bits(0x4096cbe4) as f64
                    };
                    let mut angle = (radians * f32::from_bits(0x42652ee0) as f64) as f32;
                    if angle < 0.0 { angle = -angle; }
                    if y > 0.0 && x < 0.0 { angle = 180.0 - angle; }
                    if y < 0.0 && x < 0.0 { angle += 180.0; }
                    if y < 0.0 && x > 0.0 { angle = 360.0 - angle; }
                    // FMUL by the double 1/3 then _ftol (truncation), without integer rounding.
                    let hint = ((angle as f64 * f64::from_bits(0x3fd5555555555555)) as f32) as i32;
                    let amplitude = (x * x + y * y).sqrt();
                    set(self, input, current_value, amplitude);
                    self.set_prop(input, direction, Value::Int(hint));
                    if let Some(Value::Array(value_list)) = slot.and_then(|s| self.objects[input].as_mut()?.props.get_mut(s)) {
                        if amplitude > value_list[hint as usize].as_float() {
                            value_list[hint as usize] = Value::Float(amplitude);
                        }
                    }
                } else if let Some(s) = slot {
                    // Writing of the unchanged array, like the previous rewrite (property generation).
                    let _ = self.objects[input].as_mut().and_then(|o| o.props.get_mut(s));
                }
            }
        }
        if config == 1 && f(self, "aTurn") != 0.0 && f(self, "aLookUp") != 0.0 {
            let (turn, up) = (f(self, "aTurn"), f(self, "aLookUp"));
            // appSqrt and the division rounded to 24 bits; power 3 through fyl2x/f2xm1/fscale (in
            // 80 bits) with the intermediate products and addition rounded to 24 bits.
            let mut r = (turn * turn + up * up).sqrt() / range;
            if 1.0 < r {
                r = 1.0;
            }
            let y = ((r as f64).dln() * 3.0) as f32;
            let y = (y as f64 * std::f64::consts::LOG2_E) as f32;
            let integer = y.round_ties_even();
            let p = ((y - integer) as f64).dexp2() as f32;
            let curve = ((p as f64 * 2f64.powi(integer as i32)) * r as f64) as f32;
            set(self, pc, "aTurn", curve * turn);
            set(self, pc, "aLookUp", curve * up);
        }
        // Acceleration of the horizontal rotation (ViewTurnAcc, ViewTurnBoost) and the weapon sway
        // that follows it (WeaponViewTurnAcc).
        let turn = f(self, "aTurn");
        let base_y = f(self, "aBaseY");
        let mut acc = fi(self, "ViewTurnAcc");
        let mut boost = fi(self, "ViewTurnBoost");
        let mut weapon = fi(self, "WeaponViewTurnAcc");
        let sign = self.propr(input, "iMemaTurnSign").as_int();
        if turn <= 0.0 {
            if 0.0 <= turn {
                acc = (acc - dt * 6.0).max(0.0);
                boost = 0.0;
                weapon = if weapon <= 0.0 { (dt * 6.0 + weapon).min(0.0) } else { (weapon - dt * 6.0).max(0.0) };
            } else {
                if sign < 0 {
                    acc = (dt * 3.0 + acc).min(1.0);
                } else {
                    acc = 0.0;
                    boost = 0.0;
                }
                boost = if acc != 1.0 || range * -0.95 <= turn || range * 0.05 <= base_y { 0.0 } else { (dt + boost).min(1.0) };
                self.set_prop(input, "iMemaTurnSign", Value::Int(-1));
                weapon = if weapon <= 0.0 { (weapon - dt * 3.0).max(-1.0) } else { (weapon - dt * 6.0).max(-1.0) };
            }
        } else {
            if sign < 0 {
                acc = 0.0;
                boost = 0.0;
            } else {
                acc = (dt * 3.0 + acc).min(1.0);
            }
            boost = if acc != 1.0 || turn <= range * 0.9 || range * 0.05 <= base_y { 0.0 } else { (dt + boost).min(1.0) };
            self.set_prop(input, "iMemaTurnSign", Value::Int(1));
            weapon = if weapon <= 0.0 { (dt * 6.0 + weapon).min(1.0) } else { (dt * 3.0 + weapon).min(1.0) };
        }
        set(self, input, "ViewTurnAcc", acc);
        set(self, input, "ViewTurnBoost", boost);
        set(self, input, "WeaponViewTurnAcc", weapon);
        set(self, pc, "aTurn", (boost + acc) * turn);
        // Acceleration of the vertical view (ViewUpAcc).
        let up = f(self, "aLookUp");
        let mut up_acc = fi(self, "ViewUpAcc");
        let up_sign = self.propr(input, "iMemaLookUpSign").as_int();
        let new_sign;
        if up <= 0.0 {
            if 0.0 <= up {
                up_acc = (up_acc - dt * 6.0).max(0.0);
                new_sign = up_sign;
            } else if up_sign < 0 {
                up_acc = (dt * 3.0 + up_acc).min(1.0);
                new_sign = -1;
            } else {
                up_acc = 0.0;
                new_sign = -1;
            }
        } else if up_sign < 0 {
            up_acc = 0.0;
            new_sign = 1;
        } else {
            up_acc = (dt * 3.0 + up_acc).min(1.0);
            new_sign = 1;
        }
        set(self, input, "ViewUpAcc", up_acc);
        self.set_prop(input, "iMemaLookUpSign", Value::Int(new_sign));
        set(self, pc, "aLookUp", up_acc * up);
        // FMUL qword: exact product by the double constant, rounded once to 24 bits.
        set(self, pc, "aTurn", crate::ai_move::fmul_d(f(self, "aTurn"), 2.42));
        set(self, pc, "aLookUp", crate::ai_move::fmul_d(f(self, "aLookUp"), -2.42));
        // Digital keys: movement at full range.
        let byte = |vm: &Vm, n: &str| vm.propr(pc, n).as_int() != 0;
        if byte(self, "bUp") {
            set(self, pc, "aBaseY", range);
        }
        if byte(self, "bDown") {
            set(self, pc, "aBaseY", -range);
        }
        if byte(self, "bLeft") {
            set(self, pc, "aStrafe", -range);
        }
        if byte(self, "bRight") {
            set(self, pc, "aStrafe", range);
        }
        // Not in the original PC version: the gamepad's left stick, at the keys' range as far as
        // it is pushed (`analog_move`), whatever the frame rate; the walk / run choice below is
        // then the game's own, as for the consoles' stick.
        if let Some((strafe, forward)) = self.analog_player_movement(pc) {
            if strafe != 0.0 || forward != 0.0 {
                set(self, pc, "aBaseY", forward * range);
                set(self, pc, "aStrafe", strafe * range);
            }
        }
        // View returning to horizontal while walking (bWalkCenterView, after 1 s).
        let pawn = self.propr(pc, "Pawn").as_object();
        let on_ladder = pawn.is_some_and(|p| self.propr(p, "Physics").as_byte() == 11);
        if pawn.is_none() || (!on_ladder && !byte(self, "bFreeLook")) {
            if self.propr(pc, "bWalkCenterView").as_bool() {
                if f(self, "aBaseY") == 0.0 {
                    self.set_prop(pc, "bSnapToLevel", Value::Bool(false));
                    self.set_prop(pc, "bSnapLevel", Value::Byte(0));
                    self.set_prop(input, "bUseSnapTimer", Value::Bool(false));
                } else {
                    if !self.propr(input, "bUseSnapTimer").as_bool() {
                        self.set_prop(input, "bUseSnapTimer", Value::Bool(true));
                        set(self, input, "fSnaptimer", dt);
                    } else {
                        let t = fi(self, "fSnaptimer");
                        set(self, input, "fSnaptimer", dt + t);
                    }
                    if 1.0 < fi(self, "fSnaptimer") {
                        self.set_prop(pc, "bSnapLevel", Value::Byte(1));
                        self.set_prop(pc, "bSnapToLevel", Value::Bool(true));
                    }
                }
            }
        } else {
            self.set_prop(pc, "bSnapLevel", Value::Byte(0));
            self.set_prop(input, "bUseSnapTimer", Value::Bool(false));
        }
        // Run or walk (bRun: slow walk).
        let pct = fi(self, "fPctWalkRun");
        let (by, st) = (f(self, "aBaseY"), f(self, "aStrafe"));
        let walk = if config == 1 {
            by * by + st * st < range * range * pct * pct || byte(self, "bAltRun")
        } else {
            (by.abs() < pct * range && st.abs() < pct * range) || byte(self, "bAltRun")
        };
        self.set_prop(pc, "bRun", Value::Byte(walk as u8));
        // Crouched (toggled by bAltDuck, jumping stands up).
        if self.propr(input, "bForceCrouch").as_bool() {
            self.set_prop(pc, "bDuck", Value::Byte(1));
        } else if pawn.is_some_and(|p| self.propr(p, "bCanCrouch").as_bool()) {
            let mut mem = self.propr(input, "iDuckMem").as_int();
            let alt_duck = self.propr(pc, "bAltDuck").as_int();
            if alt_duck == 1 && self.propr(input, "bDuckCheck").as_bool() {
                self.set_prop(input, "bDuckCheck", Value::Bool(false));
                mem = (mem - 1).abs();
            } else if alt_duck == 0 {
                self.set_prop(input, "bDuckCheck", Value::Bool(true));
            }
            if byte(self, "bAltJump") && 0 < mem {
                self.set_prop(input, "bDuckCheck", Value::Bool(false));
                mem = (mem - 1).abs();
            }
            self.set_prop(input, "iDuckMem", Value::Int(mem));
            self.set_prop(pc, "bDuck", Value::Byte(mem as u8));
        } else {
            self.set_prop(pc, "bDuck", Value::Byte(0));
        }
        // Memory of directions (bWas*) and direction changes (bEdge*).
        for (was, edge, now) in [
            ("bWasForward", "bEdgeForward", 0.0 < by),
            ("bWasBack", "bEdgeBack", by < 0.0),
            ("bWasLeft", "bEdgeLeft", 0.0 < st),
            ("bWasRight", "bEdgeRight", st < 0.0),
        ] {
            let before = self.propr(input, was).as_bool();
            self.set_prop(input, was, Value::Bool(now));
            self.set_prop(input, edge, Value::Bool(before != now));
        }
        // Mouse: sensitivity to the field of view, smoothing.
        let fov = crate::ai_move::fmul_d(f(self, "DesiredFOV"), 0.01111);
        let sens = fov * fi(self, "MouseSensitivity");
        let (mx, my) = (sens * f(self, "aMouseX"), sens * f(self, "aMouseY"));
        set(self, pc, "aMouseX", mx);
        set(self, pc, "aMouseY", my);
        let level_time = self.level.map(|l| self.propr(l, "TimeSeconds").as_float()).unwrap_or(self.time);
        let dilation = self.level.map(|l| self.propr(l, "TimeDilation").as_float()).filter(|&d| d != 0.0).unwrap_or(1.0);
        let since = (level_time - fi(self, "MouseZeroTime")) / dilation;
        let (mut sx, mut bx) = (fi(self, "SmoothMouseX"), fi(self, "BorrowedMouseX"));
        let keep_x = self.smooth_mouse(input, mx, since, &mut sx, &mut bx);
        let (mut sy, mut byy) = (fi(self, "SmoothMouseY"), fi(self, "BorrowedMouseY"));
        let keep_y = self.smooth_mouse(input, my, since, &mut sy, &mut byy);
        // Lockstep co-op: the player's machine integrated the mouse look frame by frame
        // (Vm::lockstep_local_look) and its command gives the view turn of this step; its mouse
        // moves were not sent as events. The smoothed values become what turns the view by that
        // much in UpdateRotation (43, or 21.5 zoomed, × DeltaTime × 0.24 × the value).
        if let Some((yaw, pitch)) = self.lockstep_turn_of(pc).filter(|_| !self.propr(pc, "bIsInMenu").as_bool()) {
            let zoomed = f(self, "DesiredFOV") != f(self, "DefaultFOV");
            let k = if zoomed { 21.5 } else { 43.0 } * dt * 0.24;
            if k != 0.0 {
                // UpdateRotation adds FloatToInt(43 × DeltaTime × aTurn) (truncation towards zero,
                // then AddEqual_IntInt): half a unit away from zero makes it land on the turn
                // sent, whatever the float rounding on the way.
                let away = |n: f32| if n > 0.0 { n + 0.5 } else if n < 0.0 { n - 0.5 } else { 0.0 };
                sx = away(yaw) / k;
                let p = away(pitch) / k;
                sy = if self.propr(input, "bInvertMouse").as_bool() { -p } else { p };
            }
        }
        set(self, input, "BorrowedMouseX", bx);
        set(self, input, "BorrowedMouseY", byy);
        if mx != 0.0 || my != 0.0 {
            set(self, input, "MouseZeroTime", level_time);
        }
        let look = fov * f(self, "aLookUp");
        let turn = fov * f(self, "aTurn");
        set(self, pc, "aLookUp", look);
        set(self, pc, "aTurn", turn);
        if byte(self, "bStrafe") {
            let v = sx + f(self, "aBaseX") + f(self, "aStrafe");
            set(self, pc, "aStrafe", v);
        } else {
            let v = fov * f(self, "aBaseX") + sx + turn;
            set(self, pc, "aTurn", v);
        }
        set(self, pc, "aBaseX", 0.0);
        if !byte(self, "bStrafe") && (self.propr(pc, "bAlwaysMouseLook").as_bool() || byte(self, "bLook")) {
            let mut v = if self.propr(input, "bInvertMouse").as_bool() { look - sy } else { look + sy };
            // FSUB/FADD return the operand's NaN as is (LLVM may write look + (-sy)).
            if sy.is_nan() && !look.is_nan() {
                v = sy;
            }
            set(self, pc, "aLookUp", v);
        } else {
            let v = f(self, "aForward") + sy;
            set(self, pc, "aForward", v);
        }
        set(self, input, "SmoothMouseX", keep_x);
        set(self, input, "SmoothMouseY", keep_y);
        if self.propr(pc, "bSnapLevel").as_int() == 0 {
            if f(self, "aLookUp") == 0.0 {
                if self.propr(pc, "bSnapToLevel").as_bool() && !self.propr(pc, "bAlwaysMouseLook").as_bool() {
                    self.set_prop(pc, "bKeyboardLook", Value::Bool(false));
                    self.set_prop(pc, "bCenterView", Value::Bool(true));
                }
            } else {
                self.set_prop(pc, "bCenterView", Value::Bool(false));
                self.set_prop(pc, "bKeyboardLook", Value::Bool(true));
            }
        } else {
            self.set_prop(pc, "bKeyboardLook", Value::Bool(false));
            self.set_prop(pc, "bCenterView", Value::Bool(true));
        }
        if byte(self, "bFreeLook") {
            self.set_prop(pc, "bKeyboardLook", Value::Bool(true));
            let v = f(self, "aBaseY") * fov * 0.5 + f(self, "aLookUp");
            set(self, pc, "aLookUp", v);
        } else {
            let v = f(self, "aForward") + f(self, "aBaseY");
            set(self, pc, "aForward", v);
        }
        set(self, pc, "aBaseY", 0.0);
        set(self, pc, "aForward", f(self, "aForward") * 4.0);
        set(self, pc, "aStrafe", f(self, "aStrafe") * 4.0);
        set(self, pc, "aLookUp", f(self, "aLookUp") * 0.24);
        set(self, pc, "aTurn", f(self, "aTurn") * 0.24);
        // APlayerController::HandleWalking.
        if pawn.is_some() {
            self.handle_walking(pc);
        }
    }
}

/// UInput::StaticInitInput: the configuration property that the original creates for slot `index` of
/// the EInputKey enumeration: the slot's name without its first three letters ("IK_") and its offset in the object
/// (3 × index + 0xAB) × 4; NAME_None slots are skipped.
#[cfg(test)]
pub(crate) fn key_property(index: usize, enum_name: &str) -> Option<(String, u32)> {
    (enum_name != "None").then(|| (enum_name[3..].to_string(), (3 * index as u32 + 0xAB) * 4))
}

/// Key names (EInputKey from Interactions.uc, without "IK_"), by code.
pub const KEY_NAMES: [&str; 255] = [
    "None",
    "LeftMouse",
    "RightMouse",
    "Cancel",
    "MiddleMouse",
    "Unknown05",
    "Unknown06",
    "Unknown07",
    "Backspace",
    "Tab",
    "Unknown0A",
    "Unknown0B",
    "Unknown0C",
    "Enter",
    "Unknown0E",
    "Unknown0F",
    "Shift",
    "Ctrl",
    "Alt",
    "Pause",
    "CapsLock",
    "Unknown15",
    "Unknown16",
    "Unknown17",
    "Unknown18",
    "Unknown19",
    "Unknown1A",
    "Escape",
    "Unknown1C",
    "Unknown1D",
    "Unknown1E",
    "Unknown1F",
    "Space",
    "PageUp",
    "PageDown",
    "End",
    "Home",
    "Left",
    "Up",
    "Right",
    "Down",
    "Select",
    "Print",
    "Execute",
    "PrintScrn",
    "Insert",
    "Delete",
    "Help",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "Unknown3A",
    "Unknown3B",
    "LesserThan",
    "Unknown3D",
    "Unknown3E",
    "Unknown3F",
    "Unknown40",
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "Unknown5B",
    "Unknown5C",
    "Unknown5D",
    "Unknown5E",
    "Unknown5F",
    "NumPad0",
    "NumPad1",
    "NumPad2",
    "NumPad3",
    "NumPad4",
    "NumPad5",
    "NumPad6",
    "NumPad7",
    "NumPad8",
    "NumPad9",
    "GreyStar",
    "GreyPlus",
    "Separator",
    "GreyMinus",
    "NumPadPeriod",
    "GreySlash",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
    "Unknown88",
    "Unknown89",
    "Unknown8A",
    "Unknown8B",
    "Unknown8C",
    "Unknown8D",
    "Unknown8E",
    "Unknown8F",
    "NumLock",
    "ScrollLock",
    "Unknown92",
    "Unknown93",
    "Unknown94",
    "Unknown95",
    "Unknown96",
    "Unknown97",
    "Unknown98",
    "Unknown99",
    "Unknown9A",
    "Unknown9B",
    "Unknown9C",
    "Unknown9D",
    "Unknown9E",
    "Unknown9F",
    "LShift",
    "RShift",
    "LControl",
    "RControl",
    "LAlt",
    "RAlt",
    "UnknownA6",
    "UnknownA7",
    "UnknownA8",
    "UnknownA9",
    "UnknownAA",
    "UnknownAB",
    "UnknownAC",
    "UnknownAD",
    "UnknownAE",
    "UnknownAF",
    "UnknownB0",
    "UnknownB1",
    "UnknownB2",
    "UnknownB3",
    "UnknownB4",
    "UnknownB5",
    "UnknownB6",
    "UnknownB7",
    "UnknownB8",
    "UnknownB9",
    "Semicolon",
    "Equals",
    "Comma",
    "Minus",
    "Period",
    "Slash",
    "Tilde",
    "UnknownC1",
    "UnknownC2",
    "UnknownC3",
    "UnknownC4",
    "UnknownC5",
    "UnknownC6",
    "UnknownC7",
    "Joy1",
    "Joy2",
    "Joy3",
    "Joy4",
    "Joy5",
    "Joy6",
    "Joy7",
    "Joy8",
    "Joy9",
    "Joy10",
    "Joy11",
    "Joy12",
    "Joy13",
    "Joy14",
    "Joy15",
    "Joy16",
    "UnknownD8",
    "UnknownD9",
    "UnknownDA",
    "LeftBracket",
    "Backslash",
    "RightBracket",
    "SingleQuote",
    "UnknownDF",
    "JoyX",
    "JoyY",
    "UnknownE2",
    "JoyR",
    "MouseX",
    "MouseY",
    "MouseZ",
    "MouseW",
    "JoyU",
    "JoyV",
    "JoyZ",
    "UnknownEB",
    "MouseWheelUp",
    "MouseWheelDown",
    "Unknown10E",
    "Unknown10F",
    "UnknownF0",
    "UnknownF1",
    "UnknownF2",
    "UnknownF3",
    "UnknownF4",
    "UnknownF5",
    "Attn",
    "CrSel",
    "ExSel",
    "ErEof",
    "Play",
    "Zoom",
    "NoName",
    "PA1",
    "OEMClear",
];

/// The gamepad's move stick against the keys (analog_move).
#[cfg(test)]
mod move_stick_tests {
    use super::{analog_move, MOVE_DEAD_ZONE, MOVE_FULL};

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    /// Full push straight = one key (W, S, A, D: the keys' range); full push diagonal on a
    /// circular gate (0.707 on each axis) = two keys (W + D: full range on both); a stick that
    /// stops at 0.9 = full; half push = in between, same direction; dead zone = nothing.
    /// Negative control: the consoles' bindings alone (each axis (|v| − 0.4) / 0.6) give a full
    /// diagonal at 0.51 of the keys and a 0.9 stick at 0.83.
    #[test]
    fn full_push_is_the_keyboard_and_half_push_in_between() {
        let up = (0.0, 1.0);
        assert!(close(analog_move(0.0, -1.0), up), "full forward");
        assert!(close(analog_move(0.0, 1.0), (0.0, -1.0)), "full back");
        assert!(close(analog_move(1.0, 0.0), (1.0, 0.0)), "full right");
        assert!(close(analog_move(-1.0, 0.0), (-1.0, 0.0)), "full left");
        let d = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(analog_move(d, -d), (1.0, 1.0)), "full diagonal = W + D");
        assert!(close(analog_move(-d, d), (-1.0, -1.0)), "full diagonal = S + A");
        assert!(close(analog_move(0.0, -0.9), up), "a stick stopping at 0.9 is full");
        assert!(close(analog_move(0.9 * d, -0.9 * d), (1.0, 1.0)), "0.9 diagonal is full");
        let half = analog_move(0.0, -0.65);
        assert!(half.1 > 0.3 && half.1 < 0.7 && half.0 == 0.0, "half push {half:?}");
        assert!(close(analog_move(0.0, -(MOVE_DEAD_ZONE + MOVE_FULL) / 2.0), (0.0, 0.5)));
        for v in [0.0, 0.1, 0.3, MOVE_DEAD_ZONE] {
            assert_eq!(analog_move(v, 0.0), (0.0, 0.0), "dead zone {v}");
            assert_eq!(analog_move(0.0, v), (0.0, 0.0), "dead zone {v}");
        }
        // Growing with the push, never past the keys.
        let mut last = 0.0;
        for i in 0..=100 {
            let f = analog_move(0.0, -(i as f32) / 100.0).1;
            assert!(f >= last && f <= 1.0, "{i}: {f}");
            last = f;
        }
        let console = |v: f32| if v.abs() > 0.4 { v.signum() * (v.abs() - 0.4) / 0.6 } else { 0.0 };
        assert!(console(d) < 0.6, "negative control: diagonal {}", console(d));
        assert!(console(0.9) < 0.9, "negative control: 0.9 stick {}", console(0.9));
    }
}
