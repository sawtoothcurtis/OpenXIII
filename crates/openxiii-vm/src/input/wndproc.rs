//! UWindowsViewport::ViewportWndProc, outside the editor (GIsEditor = 0), for all
//! messages. The effects are returned as calls to the host (Win32 functions, virtual functions of the engine,
//! of the client, of the viewport, of its UInput and of its RenDev, DirectInput, CauseInputEvent, log), in the
//! original's order, and the written fields of the viewport and of the client are kept in `WndViewport`. The display layer
//! (windrv_input.rs) runs those that affect the game (inputs, UInput::ResetInput, mouse capture, full
//! screen, close); the others are the Windows window.
//!
//! Unreachable in game (GIsEditor is 0 in XIII.exe): the editor branches (editor mouse,
//! buttons, double click, wheel,
//! forwarding to the parent window +0x19C, repaints); the `catch` blocks of the guards
//! are only taken if a callee throws a C++ exception.

/// The viewport itself, when it is an argument (written V in the call text).
pub const VP: u32 = 0x5650_5650;

/// An effect of ViewportWndProc that the host runs (and whose value it returns).
#[derive(Debug, Clone, PartialEq)]
pub enum WndCall {
    /// UWindowsViewport::CauseInputEvent: key, action, value (bits of a float); returns true if the event is taken.
    CauseInputEvent(u32, u32, u32),
    /// UViewport::IsRealtime.
    IsRealtime,
    /// Virtual function of the viewport (vtable +offset) with its arguments: +0x10 ConditionalDestroy, +0x80
    /// UpdateWindowFrame, +0x84 IsFullscreen, +0x88 ResizeViewport, +0x8C Repaint, +0xB0 SetClientSize, +0xB8
    /// SetMouseCapture, +0xBC Repaint.
    Viewport(u32, Vec<u32>),
    /// Virtual function of the client's engine (Client + 0x28), the viewport as first argument (written V): +0x84 Key,
    /// +0x94 MousePosition.
    Engine(u32, Vec<u32>),
    /// Virtual function of the console (GEngine + 0x50).
    Console(u32, Vec<u32>),
    /// Virtual function of the client (GetOuterUClient): +0x94 MakeCurrent, +0x98 GetLastCurrent.
    Client(u32, Vec<u32>),
    /// UInput of the viewport (+0x6C), vtable +0x84: ResetInput.
    InputReset,
    /// RenDev of the viewport (+0x70), vtable +offset.
    RenDev(u32, Vec<u32>),
    /// DirectInput mouse: +0x1C Acquire, +0x20 Unacquire.
    DiMouse(u32),
    /// DirectInput gamepad of the viewport (+0x1C4): +0x1C Acquire, +0x20 Unacquire.
    DiJoy(u32),
    /// GUnIsCdAvailable, vtable +offset.
    Cd(u32),
    /// Win32 function and its arguments.
    Win32(&'static str, Vec<u32>),
    GetAsyncKeyState(u32),
    GetKeyState(u32),
    PostMessage(u32, u32, u32, u32),
    GetDesktopWindow,
    SetForegroundWindow(u32),
    SetActiveWindow(u32),
    SetCursor(u32),
    DefWindowProc(u32, u32, u32, u32),
    /// GetWindowThreadProcessId(hwnd): returns the process identifier.
    WindowProcess(u32),
    /// GetCurrentProcessId (left out of the call text).
    CurrentProcess,
    ToggleFullscreen,
    EndFullscreen,
    /// Function ("EXIT": close).
    Closing,
    /// Function (window size for a client size): width, height, 0.
    SizeWindow(u32, u32, u32),
    /// Function: window, 0.
    H1110a120(u32, u32),
    /// WWindow::MoveWindow(FRect, bRepaint).
    MoveWindow([i32; 4], u32),
    /// Line written to GLog.
    Log(String),
}

impl WndCall {
    /// Call left out of the call text (Display).
    pub fn silencer(&self) -> bool {
        matches!(self, WndCall::IsRealtime | WndCall::CurrentProcess | WndCall::Log(_))
    }
}

impl std::fmt::Display for WndCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let list_value = |f: &mut std::fmt::Formatter<'_>, v: &[u32]| {
            v.iter().try_for_each(|&x| if x == VP { write!(f, " V") } else { write!(f, " {x:x}") })
        };
        match self {
            WndCall::CauseInputEvent(k, a, v) => write!(f, "CauseInputEvent {k:x} {a:x} {v:x}"),
            WndCall::IsRealtime | WndCall::CurrentProcess | WndCall::Log(_) => Ok(()),
            WndCall::Viewport(o, a) => {
                write!(f, "vp {o:x}")?;
                list_value(f, a)
            }
            WndCall::Engine(o, a) => {
                write!(f, "eng {o:x} V")?;
                list_value(f, a)
            }
            WndCall::Console(o, a) => {
                write!(f, "con {o:x}")?;
                list_value(f, a)
            }
            WndCall::Client(o, a) => {
                write!(f, "cli {o:x}")?;
                list_value(f, a)
            }
            WndCall::InputReset => write!(f, "inp 84"),
            WndCall::RenDev(o, a) => {
                write!(f, "rd {o:x}")?;
                list_value(f, a)
            }
            WndCall::DiMouse(o) => write!(f, "dim {o:x}"),
            WndCall::DiJoy(o) => write!(f, "dij {o:x}"),
            WndCall::Cd(o) => write!(f, "cd {o:x}"),
            WndCall::Win32(n, a) => {
                write!(f, "{n}")?;
                list_value(f, a)
            }
            WndCall::GetAsyncKeyState(k) => write!(f, "GetAsyncKeyState {k:x}"),
            WndCall::GetKeyState(k) => write!(f, "GetKeyState {k:x}"),
            WndCall::PostMessage(h, m, w, l) => write!(f, "PostMessageA {h:x} {m:x} {w:x} {l:x}"),
            WndCall::GetDesktopWindow => write!(f, "GetDesktopWindow"),
            WndCall::SetForegroundWindow(h) => write!(f, "SetForegroundWindow {h:x}"),
            WndCall::SetActiveWindow(h) => write!(f, "SetActiveWindow {h:x}"),
            WndCall::SetCursor(c) => write!(f, "SetCursor {c:x}"),
            WndCall::DefWindowProc(h, m, w, l) => write!(f, "DefWindowProcA {h:x} {m:x} {w:x} {l:x}"),
            WndCall::WindowProcess(h) => write!(f, "GetWindowThreadProcessId {h:x}"),
            WndCall::ToggleFullscreen => write!(f, "ToggleFullscreen"),
            WndCall::EndFullscreen => write!(f, "EndFullscreen"),
            WndCall::Closing => write!(f, "H111050d0"),
            WndCall::SizeWindow(w, h, z) => write!(f, "H111056f0 {w:x} {h:x} {z:x}"),
            WndCall::H1110a120(a, b) => write!(f, "H1110a120 {a:x} {b:x}"),
            WndCall::MoveWindow(r, p) => write!(f, "MoveWindow {:x} {:x} {:x} {:x} {p:x}", r[0], r[1], r[2], r[3]),
        }
    }
}

/// What ViewportWndProc reads and writes of the viewport, its client and the globals.
#[derive(Debug, Clone, Default)]
pub struct WndViewport {
    /// The viewport is in the client's list (Client + 0x2C).
    pub in_list: bool,
    /// Viewport + 0x30 (Actor) non-null.
    pub actor: bool,
    /// +0x1A4: processing lock.
    pub lock: u32,
    /// WWindow: handle (+4) and window-open flag (+8).
    pub hwnd: u32,
    pub window_open: bool,
    /// +0x1A8: bit 0 fullscreen; 0x10, 0x80: render modes (WM_SIZE); 0x111: repaint (WM_PAINT).
    pub flags: u32,
    /// +0x1B4: mouse capture.
    pub f1b4: u32,
    pub f38: u32,
    pub b50: u8,
    /// +0x1E4: cursors per shape.
    pub cursors: [u32; 7],
    /// Client + 0x3C, 0x40, 0x44, 0x48: window sizes then fullscreen sizes.
    pub sizes: [u32; 4],
    /// The console of GEngine (+0x50) exists.
    pub console: bool,
    /// +0x84: repaint requested (WM_PAINT).
    pub f84: u32,
    /// +0x198: window state (1 created, 2 closed).
    pub f198: u32,
    /// +0x19C: parent window (editor).
    pub f19c: u32,
    /// +0x1B0: fullscreen to restore (left on loss of focus).
    pub f1b0: u32,
    /// +0x1D0: window rectangle in fullscreen.
    pub rect: [i32; 4],
    /// +0x6C (UInput), +0x70 (RenDev), +0x1C4 (gamepad) non-null; DirectInput mouse.
    pub input: bool,
    pub rendev: bool,
    pub joy: bool,
    pub mouse: bool,
    /// Client + 0xAC: Windows menu loop in progress.
    pub client_ac: u32,
    /// Client + 0xD0…0xD6: identifiers of the global hotkeys.
    pub hotkeys: [u16; 4],
    /// GIsSlowTask, GOperateLoadMap, the CD check flag, GUnIsCdAvailable non-null.
    pub slow_task: bool,
    pub operate_load_map: bool,
    pub cd_flag: bool,
    pub cd: bool,
    /// HWND= from the command line (0: missing).
    pub hwnd_cmd: u32,
    /// DEV_BROADCAST_HDR::dbch_devicetype of WM_DEVICECHANGE's lParam.
    pub devtype: u32,
    /// Name of the viewport (UObject::GetName), for the WM_DISPLAYCHANGE log.
    pub name: String,
}

const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_CHAR: u32 = 0x102;
const WM_SYSKEYDOWN: u32 = 0x104;
const WM_SYSKEYUP: u32 = 0x105;
const WM_SYSCHAR: u32 = 0x106;
const WM_MOUSEMOVE: u32 = 0x200;
const WM_MOUSEWHEEL: u32 = 0x20A;
const IST_PRESS: u32 = 1;
const IST_RELEASE: u32 = 3;

/// ViewportWndProc; `wheel_msg`: the registered mouse wheel message (WindowMessageMouseWheel); `host` runs
/// each effect and returns its value. Returns the message's value.
pub fn viewport_wnd_proc(v: &mut WndViewport, msg: u32, wparam: u32, lparam: u32, wheel_msg: u32, host: &mut dyn FnMut(WndCall) -> u32) -> u32 {
    // Viewport unknown to the client, locked or without an actor: DefWindowProc of the open window.
    if v.lock != 0 || !v.in_list || !v.actor {
        return if v.window_open { host(WndCall::DefWindowProc(v.hwnd, msg, wparam, lparam)) } else { 0 };
    }
    let (mut msg, mut ebx) = (msg, wparam);
    if msg == wheel_msg {
        msg = WM_MOUSEWHEEL;
        ebx = (wparam & 0xFFFF) << 16;
    }
    let edi = lparam;
    // Common start: some messages take the mouse capture, then send the
    // position (UEngine::MousePosition). WM_CREATE (1) is one of them.
    let early = |m: u32| matches!(m, 1 | WM_KEYDOWN | WM_SYSKEYDOWN | 0x201 | 0x204 | 0x207);
    let position = |m: u32| early(m) || matches!(m, WM_MOUSEMOVE | 0x202 | 0x203 | 0x205 | 0x206 | 0x208);
    if position(msg) {
        if early(msg) && v.f1b4 == 0 {
            host(WndCall::Viewport(0xB8, vec![1, 1, 0]));
        }
        let (x, y) = ((lparam & 0xFFFF) as f32, (lparam >> 16) as f32);
        host(WndCall::Engine(0x94, vec![ebx, x.to_bits(), y.to_bits()]));
    }
    // DefWindowProc if the window is open.
    let def = |v: &WndViewport, host: &mut dyn FnMut(WndCall) -> u32, msg: u32, ebx: u32| -> u32 {
        if v.window_open {
            host(WndCall::DefWindowProc(v.hwnd, msg, ebx, edi))
        } else {
            0
        }
    };
    let console = |v: &WndViewport, host: &mut dyn FnMut(WndCall) -> u32, slot: u32| {
        if v.console {
            host(WndCall::Console(slot, vec![0xFFF, 0]));
        }
    };
    let acquire = |v: &WndViewport, host: &mut dyn FnMut(WndCall) -> u32, off: u32| {
        if v.mouse {
            host(WndCall::DiMouse(off));
        }
        if v.joy {
            host(WndCall::DiJoy(off));
        }
    };
    match msg {
        1 => {
            // WM_CREATE.
            v.f198 = 1;
            host(WndCall::Client(0x94, vec![VP]));
            host(WndCall::H1110a120(v.hwnd, 0));
            host(WndCall::Win32("SetFocus", vec![v.hwnd]));
            0
        }
        2 => {
            // WM_DESTROY.
            if v.flags & 1 != 0 {
                host(WndCall::EndFullscreen);
            }
            if v.hwnd_cmd != 0 {
                host(WndCall::Win32("SetParent", vec![v.hwnd, 0]));
                host(WndCall::Win32("SetFocus", vec![v.hwnd_cmd]));
            }
            host(WndCall::Viewport(0x80, vec![0]));
            if v.f198 == 1 {
                v.f198 = 2;
                host(WndCall::Viewport(0x10, vec![1]));
            }
            host(WndCall::Log("Closed viewport".into()));
            0
        }
        5 => {
            // WM_SIZE.
            let (w, h) = (edi & 0xFFFF, edi >> 16);
            if v.flags & 1 != 0 {
                if ebx == 0 && v.window_open {
                    v.lock += 1;
                    host(WndCall::MoveWindow(v.rect, 1));
                    v.lock -= 1;
                }
                return 0;
            }
            if ebx == 0 && v.f1b0 != 0 {
                v.f1b0 = 0;
                host(WndCall::ToggleFullscreen);
                return 0;
            }
            if v.rendev && v.flags & 0x90 != 0 {
                if v.window_open {
                    host(WndCall::SizeWindow(w, h, 0));
                }
                host(WndCall::Viewport(0xB0, vec![w, h]));
            } else {
                host(WndCall::Viewport(0x88, vec![v.flags | 0x20, w, h]));
            }
            0
        }
        6 => {
            // WM_ACTIVATE: DirectInput and the fullscreen global hotkeys.
            match ebx & 0xFFFF {
                0 => {
                    acquire(v, host, 0x20);
                    host(WndCall::Viewport(0x80, vec![0]));
                    if v.flags & 1 != 0 {
                        for id in v.hotkeys {
                            host(WndCall::Win32("UnregisterHotKey", vec![v.hwnd, u32::from(id)]));
                        }
                    }
                }
                1 => {
                    acquire(v, host, 0x1C);
                    if v.flags & 1 != 0 {
                        for (id, (m, k)) in v.hotkeys.into_iter().zip([(1, 0x1B), (1, 9), (2, 0x1B), (2, 9)]) {
                            host(WndCall::Win32("RegisterHotKey", vec![v.hwnd, u32::from(id), m, k]));
                        }
                    }
                }
                _ => {}
            }
            0
        }
        7 => {
            // WM_SETFOCUS.
            if v.input {
                host(WndCall::InputReset);
                host(WndCall::InputReset);
            }
            acquire(v, host, 0x1C);
            let last_one = host(WndCall::Client(0x98, vec![]));
            host(WndCall::Client(0x94, vec![VP]));
            host(WndCall::Viewport(0x8C, vec![]));
            host(WndCall::H1110a120(v.hwnd, 0));
            if v.rendev {
                host(WndCall::RenDev(0x84, vec![VP]));
            }
            if last_one != VP {
                let mut p = v.f19c;
                for _ in 0..3 {
                    p = host(WndCall::Win32("GetParent", vec![p]));
                }
                host(WndCall::PostMessage(p, 0x111, 0x8010, 0));
            }
            0
        }
        8 => {
            // WM_KILLFOCUS.
            let window_value = if ebx != 0 { ebx } else { host(WndCall::Win32("GetForegroundWindow", vec![])) };
            let pid = host(WndCall::WindowProcess(window_value));
            if pid != host(WndCall::CurrentProcess) {
                host(WndCall::RenDev(0x88, vec![]));
            }
            host(WndCall::Viewport(0xB8, vec![0, 0, 0]));
            host(WndCall::Viewport(0x80, vec![0]));
            host(WndCall::InputReset);
            if v.flags & 1 != 0 {
                host(WndCall::Log("WM_KILLFOCUS".into()));
                host(WndCall::EndFullscreen);
                v.lock += 1;
                host(WndCall::Win32("ShowWindow", vec![v.hwnd, 7]));
                v.lock -= 1;
                v.f1b0 = 1;
            }
            acquire(v, host, 0x20);
            host(WndCall::Client(0x94, vec![0]));
            0
        }
        0xF => {
            // WM_PAINT.
            if v.flags & 0x111 != 0 {
                v.f84 = 1;
                host(WndCall::Win32("ValidateRect", vec![v.hwnd, 0]));
                0
            } else {
                1
            }
        }
        0x10 => {
            // WM_CLOSE.
            host(WndCall::Closing);
            def(v, host, msg, ebx)
        }
        0x14 => 0,
        0x1F => {
            // WM_CANCELMODE.
            host(WndCall::Viewport(0x80, vec![0]));
            0
        }
        0x20 => {
            // WM_SETCURSOR: HTCLIENT or slow task, cursor kept.
            if edi & 0xFFFF == 1 || v.slow_task {
                0
            } else {
                def(v, host, msg, ebx)
            }
        }
        0x21 => 1,
        0x48 => {
            // WM_POWER.
            match ebx {
                0 => 0,
                1 => {
                    host(WndCall::Log("Received WM_POWER suspend".into()));
                    u32::MAX
                }
                _ => {
                    host(WndCall::Log("Received WM_POWER".into()));
                    def(v, host, msg, ebx)
                }
            }
        }
        0x7E => {
            // WM_DISPLAYCHANGE.
            host(WndCall::Log(format!("Viewport {}: WM_DisplayChange", v.name)));
            0
        }
        0x112 => wm_syscommand(v, host, ebx, edi),
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let alt = (lparam >> 29) & 1 != 0;
            let mut key = ebx;
            match key {
                9 if alt => {
                    let desktop = host(WndCall::GetDesktopWindow);
                    host(WndCall::SetForegroundWindow(desktop));
                    host(WndCall::SetActiveWindow(desktop));
                    return 0;
                }
                0xD if alt => {
                    host(WndCall::PostMessage(v.hwnd, WM_SYSKEYDOWN, 9, lparam));
                    return 0;
                }
                0x73 if alt => {
                    host(WndCall::Closing);
                    return 0;
                }
                0x10 => key = if host(WndCall::GetAsyncKeyState(0xA0)) & 0xFFFF != 0 { 0xA0 } else { 0xA1 },
                0x11 => {
                    key = if host(WndCall::GetAsyncKeyState(0xA5)) & 0xFFFF != 0 {
                        0xA5
                    } else if host(WndCall::GetAsyncKeyState(0xA2)) & 0xFFFF != 0 {
                        0xA2
                    } else {
                        0xA3
                    }
                }
                0x12 => key = if host(WndCall::GetAsyncKeyState(0xA4)) & 0xFFFF != 0 { 0xA4 } else { 0xA5 },
                0xC => key = 0x65,
                0xD if host(WndCall::GetKeyState(0x12)) & 0x8000 != 0 => {
                    // Alt + Enter: toggles fullscreen, not passed to the game.
                    if v.window_open {
                        console(v, host, 0xA8);
                        host(WndCall::ToggleFullscreen);
                        console(v, host, 0xAC);
                    } else {
                        toggle_without_window(v, host);
                    }
                    let (w, h) = if v.flags & 1 != 0 { (v.sizes[2], v.sizes[3]) } else { (v.sizes[0], v.sizes[1]) };
                    console(v, host, 0xA8);
                    host(WndCall::Viewport(0xB0, vec![w, h]));
                    console(v, host, 0xAC);
                    return 0;
                }
                _ => {}
            }
            if v.window_open && host(WndCall::CauseInputEvent(key, IST_PRESS, 0)) != 0 && host(WndCall::IsRealtime) == 0 {
                host(WndCall::Viewport(0xBC, vec![1]));
            }
            0
        }
        WM_KEYUP | WM_SYSKEYUP => {
            if host(WndCall::CauseInputEvent(ebx, IST_RELEASE, 0)) != 0 && host(WndCall::IsRealtime) == 0 {
                host(WndCall::Viewport(0xBC, vec![1]));
            }
            0
        }
        WM_CHAR | WM_SYSCHAR => {
            if ebx != 0xD && v.window_open && host(WndCall::Engine(0x84, vec![ebx])) != 0 {
                if host(WndCall::IsRealtime) == 0 {
                    host(WndCall::Viewport(0xBC, vec![1]));
                }
                return 0;
            }
            if msg == WM_SYSCHAR {
                def(v, host, msg, ebx)
            } else {
                0
            }
        }
        WM_MOUSEMOVE => {
            if v.f1b4 != 0 && v.f38 & 2 != 0 && v.b50 <= 6 {
                host(WndCall::SetCursor(v.cursors[v.b50 as usize]));
            }
            0
        }
        WM_MOUSEWHEEL | 0x203 => 0,
        0x211 => {
            // WM_ENTERMENULOOP.
            v.client_ac = 1;
            host(WndCall::Viewport(0x80, vec![0]));
            host(WndCall::Viewport(0x90, vec![]));
            acquire(v, host, 0x20);
            0
        }
        0x212 => {
            // WM_EXITMENULOOP.
            v.client_ac = 0;
            acquire(v, host, 0x1C);
            0
        }
        0x219 => {
            // WM_DEVICECHANGE: check of the game CD (DBT_DEVTYP_VOLUME).
            if !v.operate_load_map && v.cd_flag && v.cd {
                let volume = v.devtype == 2;
                if matches!(ebx, 0x8001 | 0x8004 | 0x8003) && volume {
                    host(WndCall::Cd(0x18));
                }
                if ebx == 0x8000 && volume {
                    host(WndCall::Cd(0x1C));
                }
            }
            def(v, host, msg, ebx)
        }
        0x312 => {
            // WM_HOTKEY: Alt + Tab, Alt + Enter, Alt + F4 in fullscreen.
            match (edi >> 16, edi & 0xFFFF) {
                (9, 1) => {
                    let desktop = host(WndCall::GetDesktopWindow);
                    host(WndCall::SetForegroundWindow(desktop));
                    host(WndCall::SetActiveWindow(desktop));
                }
                (0xD, 1) => {
                    host(WndCall::PostMessage(v.hwnd, 0x312, ebx, 0x90001));
                }
                (0x73, _) => {
                    host(WndCall::Closing);
                }
                _ => {}
            }
            0
        }
        _ => def(v, host, msg, ebx),
    }
}

/// Fullscreen toggle without an open window: flag +0x1A8 and capture.
fn toggle_without_window(v: &mut WndViewport, host: &mut dyn FnMut(WndCall) -> u32) {
    let prior = v.flags;
    v.flags ^= 1;
    if v.flags & 1 == 0 && prior & 1 != 0 {
        host(WndCall::Viewport(0xB8, vec![0, 0, 0]));
    }
    if v.flags & 1 != 0 {
        host(WndCall::Viewport(0xB8, vec![1, 1, 0]));
    }
}

/// WM_SYSCOMMAND: screen saver and monitor power-off refused, maximizing and
/// restoring the window.
fn wm_syscommand(v: &mut WndViewport, host: &mut dyn FnMut(WndCall) -> u32, ebx: u32, edi: u32) -> u32 {
    let def = |v: &WndViewport, host: &mut dyn FnMut(WndCall) -> u32| -> u32 {
        if v.window_open {
            host(WndCall::DefWindowProc(v.hwnd, 0x112, ebx, edi))
        } else {
            0
        }
    };
    let sc = ebx & 0xFFF0;
    match sc {
        0xF140 => {
            host(WndCall::Log("Received SC_SCREENSAVE".into()));
            0
        }
        0xF170 => {
            host(WndCall::Log("Received SC_MONITORPOWER".into()));
            0
        }
        // SC_NEXTWINDOW, SC_PREVWINDOW, SC_TASKLIST, 0xF150: refused in fullscreen.
        0xF040 | 0xF050 | 0xF130 | 0xF150 => {
            if host(WndCall::Viewport(0x84, vec![])) != 0 {
                0
            } else {
                def(v, host)
            }
        }
        // SC_MAXIMIZE, SC_RESTORE: client sizes of the current mode; maximizing sets them to 1600 × 1200.
        0xF030 | 0xF120 => {
            let iw = if host(WndCall::Viewport(0x84, vec![])) != 0 { 2 } else { 0 };
            let ih = if host(WndCall::Viewport(0x84, vec![])) != 0 { 3 } else { 1 };
            if sc == 0xF030 {
                v.sizes[iw] = 0x640;
                v.sizes[ih] = 0x4B0;
            }
            if v.window_open {
                if host(WndCall::Viewport(0x84, vec![])) != 0 {
                    v.flags &= !1;
                    host(WndCall::ToggleFullscreen);
                } else {
                    host(WndCall::SizeWindow(v.sizes[iw], v.sizes[ih], 0));
                }
            } else {
                toggle_without_window(v, host);
            }
            host(WndCall::Viewport(0xB0, vec![v.sizes[iw], v.sizes[ih]]));
            host(WndCall::Win32("ShowWindow", vec![v.hwnd, 1]));
            0
        }
        _ => def(v, host),
    }
}

/// What UWindowsViewport::SetMouseCapture reads and writes of the viewport.
#[derive(Debug, Clone, Default)]
pub struct CaptureViewport {
    /// +0x38: UPlayer bits; bit 0 bWindowsMouseAvailable.
    pub f38: u32,
    /// +0x1A8: bit 0 fullscreen.
    pub flags: u32,
    /// +0x1B4: capture.
    pub f1b4: u32,
    /// +0x1C8, +0x1CC: position of the Windows cursor kept during capture (−1: none).
    pub cursor: [i32; 2],
    /// +0xD0, +0xD4, +0xD8: the same position as floats, then 0 (read back by no code).
    pub fd0: [u32; 3],
}

/// UWindowsViewport::SetMouseCapture: `rect`: client rectangle of the window in screen
/// coordinates (GetClientRect then MapWindowPoints), `cursor`: GetCursorPos. bWindowsMouseAvailable is the inverse
/// of the capture; while capturing, the Windows cursor is kept, centred, hidden and confined; otherwise it is released. A
/// "focus only" request without focus only changes bWindowsMouseAvailable.
pub fn set_mouse_capture(
    v: &mut CaptureViewport,
    capture: u32,
    clip: u32,
    focus_only: u32,
    rect: [i32; 4],
    cursor: [i32; 2],
    host: &mut dyn FnMut(WndCall) -> u32,
) {
    v.f38 = (v.f38 & !1) | u32::from(capture == 0);
    let hwnd = host(WndCall::Viewport(0xA0, vec![]));
    if focus_only != 0 && host(WndCall::Win32("GetFocus", vec![])) != hwnd {
        return;
    }
    let clip = clip | capture;
    host(WndCall::Win32("GetClientRect", vec![hwnd]));
    host(WndCall::Win32("MapWindowPoints", vec![hwnd, 0, 2]));
    if capture != 0 {
        if v.cursor[0] == -1 {
            let before_value = host(WndCall::Win32("GetForegroundWindow", vec![]));
            let attach = |host: &mut dyn FnMut(WndCall) -> u32, b: u32| {
                let current_value = host(WndCall::Win32("GetCurrentThreadId", vec![]));
                let thread = host(WndCall::Win32("GetWindowThreadProcessId", vec![before_value, 0]));
                host(WndCall::Win32("AttachThreadInput", vec![thread, current_value, b]));
            };
            if before_value == hwnd {
                attach(host, 1);
            }
            host(WndCall::SetForegroundWindow(hwnd));
            host(WndCall::SetActiveWindow(hwnd));
            if before_value == hwnd {
                attach(host, 0);
            }
            host(WndCall::Win32("GetCursorPos", vec![]));
            v.cursor = cursor;
            v.fd0 = [(cursor[0] as f32).to_bits(), (cursor[1] as f32).to_bits(), 0];
            if host(WndCall::Engine(0xD0, vec![])) == 0 {
                let (x, y) = ((rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2);
                host(WndCall::Win32("SetCursorPos", vec![x as u32, y as u32]));
            }
            host(WndCall::Win32("SetCapture", vec![hwnd]));
            while (host(WndCall::Win32("ShowCursor", vec![0])) as i32) >= 0 {}
        }
    } else {
        if v.flags & 1 == 0 {
            host(WndCall::Win32("ReleaseCapture", vec![]));
        }
        if v.cursor[0] != -1 {
            if host(WndCall::Engine(0xD0, vec![])) == 0 {
                host(WndCall::Win32("SetCursorPos", vec![v.cursor[0] as u32, v.cursor[1] as u32]));
            }
            v.cursor[0] = -1;
            while (host(WndCall::Win32("ShowCursor", vec![1])) as i32) < 0 {}
        }
    }
    let r = if clip != 0 { rect.map(|x| x as u32).to_vec() } else { vec![0] };
    host(WndCall::Win32("ClipCursor", r));
    v.f1b4 = capture;
}
