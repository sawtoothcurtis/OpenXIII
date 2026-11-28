//! Keyboard and mouse of the game window, as in WinDrv.dll: the display layer gives the window messages
//! (keys, characters) and the buffered DirectInput mouse data; the VM keeps the Windows keyboard state
//! (GetKeyState, GetAsyncKeyState) and redoes what UWindowsViewport::ViewportWndProc
//! (see wndproc.rs) and the mouse and keyboard part of UWindowsViewport::UpdateInput do.
//!
//! UpdateInput, after the gamepad (pad.rs):
//! - mouse (outside the editor): GetDeviceData one entry at a time until the
//!   buffer is empty; an entry is only passed on if the viewport's mouse capture (+0x1B4) and
//!   the one seen at the previous entry (global) are non-zero; dwOfs 0 → IK_MouseX,
//!   4 → IK_MouseY (sign inverted), 8 → IK_MouseW then MouseWheelDown / MouseWheelUp pressed and released,
//!   12, 13, 14 → buttons 1, 2, 4 (bit 0x80: press); the other offsets are ignored;
//! - keyboard: for each key 0…255 that the gamepad and the mouse have not
//!   handled, release if UInput thinks it is pressed and GetKeyState says it is not; press if UInput thinks
//!   it is up, GetKeyState says it is down and the window has focus, only when the argument is non-zero
//!   (UInput::ResetInput): UViewport::ReadInput passes 0.
//! With a non-zero argument, the Windows keyboard state is first reset except Num Lock,
//! Caps Lock and Scroll Lock (SetKeyboardState).
//!
//! The fields +0x1B8 and +0x1BC that UpdateInput writes for the gamepad are read back by no code of WinDrv
//! or of the other DLLs (only copied between viewports): they are not kept.

use std::collections::VecDeque;

use crate::player_input::{IST_AXIS, IST_PRESS, IST_RELEASE, IST_TYPE};
use crate::value::ObjId;
use crate::wndproc::{viewport_wnd_proc, WndCall, WndViewport};
use crate::Vm;

/// Registered mouse wheel message (WindowMessageMouseWheel): the display layer does not send any.
const WHEEL_MSG: u32 = 0xC050;

/// What the display layer must do after a window message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinAction {
    Nothing,
    /// UWindowsViewport::ToggleFullscreen.
    FullToggleScreen,
    /// Alt + F4 (function): close.
    Closing,
}

/// What UGameEngine::MouseDelta reads, recorded before a call to UViewport::SetDrag (`Vm::win_state_slides`).
#[derive(Debug, Clone, Copy)]
pub struct StateSlides {
    pub state_value: crate::drag::StateMouseDelta,
    /// Viewports(0)->IsFullscreen().
    pub full_screen: u32,
    /// GetLevelInfo()->Pauser non-null.
    pub pauser: u32,
}

/// WinDrv and Windows state for the local player's window.
#[derive(Debug, Clone)]
pub struct WinInput {
    /// The display layer goes through the window (messages and DirectInput mouse): UpdateInput then reads the
    /// keyboard state again. False when the input events are given directly (CauseInputEvent).
    pub active: bool,
    /// Windows keyboard state per virtual key code: 0x80 pressed, 1 toggled.
    keys: [u8; 256],
    /// The viewport's window has focus (GetFocus).
    pub focus: bool,
    /// Viewport+0x1B4: mouse capture (UWindowsViewport::SetMouseCapture).
    pub capture: u32,
    /// Viewport state kept by SetMouseCapture; bWindowsMouseAvailable (+0x38 bit 0) is copied onto the object
    /// of the player's viewport from the first call on.
    pub capture_vp: crate::wndproc::CaptureViewport,
    pub windows_mouse: Option<bool>,
    /// Capture seen at the last mouse entry read.
    capture_view: u32,
    /// Viewport+0xC4: drag (UViewport::SetDrag, drag.rs), 0 when the viewport is created.
    pub drag: u32,
    /// DirectInput mouse buffer: (dwOfs, dwData).
    mouse: VecDeque<(u32, u32)>,
}

impl Default for WinInput {
    /// Like UWindowsViewport::UWindowsViewport: no capture (+0x1B4 = 0, +0x1C8 = −1);
    /// the capture seen at the last mouse entry at 0.
    fn default() -> Self {
        Self {
            active: false,
            keys: [0; 256],
            focus: true,
            capture: 0,
            capture_vp: crate::wndproc::CaptureViewport { cursor: [-1, -1], ..Default::default() },
            windows_mouse: None,
            capture_view: 0,
            drag: 0,
            mouse: VecDeque::new(),
        }
    }
}

impl WinInput {
    /// GetKeyState: the state byte sign-extended (0xFF80 | toggle when the key is pressed).
    pub fn get_key_state(&self, vk: u32) -> u32 {
        let b = self.keys[(vk & 0xFF) as usize];
        (b as i8 as i16 as u16) as u32
    }

    /// GetAsyncKeyState: bit 15 when the key is pressed.
    pub fn get_async_key_state(&self, vk: u32) -> u32 {
        if self.keys[(vk & 0xFF) as usize] & 0x80 != 0 {
            0x8000
        } else {
            0
        }
    }

    /// Physical key pressed or released: all its codes (generic then left / right) change
    /// state; Num Lock, Caps Lock and Scroll Lock toggle on press.
    pub fn set_key(&mut self, vk: u8, down: bool) {
        let b = &mut self.keys[vk as usize];
        if down {
            if *b & 0x80 == 0 && matches!(vk, 0x14 | 0x90 | 0x91) {
                *b ^= 1;
            }
            *b |= 0x80;
        } else {
            *b &= !0x80;
        }
    }

    /// SetKeyboardState: everything at zero except the bytes of Num Lock, Caps Lock and Scroll Lock.
    pub fn reset_keyboard(&mut self) {
        let kept = [(0x90, self.keys[0x90]), (0x14, self.keys[0x14]), (0x91, self.keys[0x91])];
        self.keys = [0; 256];
        for (k, b) in kept {
            self.keys[k] = b;
        }
    }

    /// UWindowsViewport::SetMouseCapture for the player's window: the game state (capture,
    /// bWindowsMouseAvailable); Windows cursor hidden, centred and confined by the display layer depending on `capture`.
    pub fn set_mouse_capture(&mut self, capture: u32, clip: u32, focus_only: u32) {
        let focus = self.focus;
        let mut host = |c: WndCall| -> u32 {
            match c {
                WndCall::Viewport(0xA0, _) => 1,
                WndCall::Win32("GetFocus", _) => u32::from(focus),
                // ShowCursor: Windows counter, once is enough.
                WndCall::Win32("ShowCursor", a) => if a[0] != 0 { 0 } else { u32::MAX },
                _ => 0,
            }
        };
        crate::wndproc::set_mouse_capture(&mut self.capture_vp, capture, clip, focus_only, [0; 4], [0; 2], &mut host);
        self.capture = self.capture_vp.f1b4;
        self.windows_mouse = Some(self.capture_vp.f38 & 1 != 0);
    }

    /// UViewport::SetDrag (GIsRunning true) of the player's viewport, with UGameEngine::MouseDelta
    /// on state `g` recorded by `Vm::win_state_slides`; returns the old drag.
    pub fn set_drag(&mut self, fresh_list_new: u32, g: &StateSlides) -> u32 {
        let mut drag = self.drag;
        let r = crate::drag::set_drag(&mut drag, fresh_list_new, true, &mut |buttons_list| {
            crate::drag::mouse_delta(&g.state_value, buttons_list, &mut |a| match a {
                crate::drag::CallDrag::IsFullscreen => g.full_screen,
                crate::drag::CallDrag::Pauser => g.pauser,
                crate::drag::CallDrag::SetMouseCapture(c, clip, focus) => {
                    self.set_mouse_capture(c, clip, focus);
                    0
                }
            })
        });
        self.drag = drag;
        r
    }

    /// Entry of the DirectInput mouse buffer (DIPROP_BUFFERSIZE = 1024: when full, new ones are lost).
    /// The same window's state for a new VM (lockstep co-op: the session's VM, then the
    /// menus' after it): the mouse capture and the keys as they are, nothing left to read.
    pub fn carried(&self) -> WinInput {
        let mut w = self.clone();
        w.mouse.clear();
        w
    }

    /// The X and Y movements of the mouse buffer taken out (the other entries stay, in order),
    /// summed in DirectInput units (zero when there is none); None when the buffer is not read now
    /// (no capture).
    pub fn take_mouse_axes(&mut self) -> Option<(f32, f32)> {
        if !self.active || self.capture == 0 {
            return None;
        }
        self.capture_view = self.capture;
        let (mut x, mut y) = (0.0, 0.0);
        self.mouse.retain(|&(ofs, data)| match ofs {
            0 => {
                x += data as i32 as f32;
                false
            }
            4 => {
                y += data as i32 as f32;
                false
            }
            _ => true,
        });
        Some((x, y))
    }

    pub fn push_mouse(&mut self, ofs: u32, data: u32) {
        if self.mouse.len() < crate::player_input::DI_MOUSE_BUFFER {
            self.mouse.push_back((ofs, data));
        }
    }
}

/// CauseInputEvent events (key, action, value) of a DirectInput mouse entry
/// (UpdateInput).
pub fn di_mouse_events(ofs: u32, data: u32) -> Vec<(u8, u8, f32)> {
    let d = data as i32;
    match ofs {
        0 => vec![(0xE4, IST_AXIS, d as f32)],
        4 => vec![(0xE5, IST_AXIS, d.wrapping_neg() as f32)],
        8 => {
            let mut v = vec![(0xE7, IST_AXIS, d as f32)];
            let wheel = if d < 0 {
                Some(0xED)
            } else if d > 0 {
                Some(0xEC)
            } else {
                None
            };
            if let Some(k) = wheel {
                v.push((k, IST_PRESS, 0.0));
                v.push((k, IST_RELEASE, 0.0));
            }
            v
        }
        12..=14 => {
            let key = [1, 2, 4][(ofs - 12) as usize];
            vec![(key, if data as u8 & 0x80 != 0 { IST_PRESS } else { IST_RELEASE }, 0.0)]
        }
        _ => Vec::new(),
    }
}

/// Reading of the mouse buffer: the events passed on, in order; updates
/// the seen capture after each entry.
pub fn update_input_mouse(win: &mut WinInput) -> Vec<(u8, u8, f32)> {
    let mut out = Vec::new();
    while let Some((ofs, data)) = win.mouse.pop_front() {
        if win.capture_view != 0 && win.capture != 0 {
            out.extend(di_mouse_events(ofs, data));
        }
        win.capture_view = win.capture;
    }
    out
}

/// Keyboard reading for an unmarked key: `focus` is a non-zero argument and
/// GetFocus equal to the viewport's window, `down`: UInput::KeyDown read at that moment.
/// Returns the CauseInputEvent action, if there is one.
pub fn update_input_key(win: &WinInput, focus: bool, k: u32, down: bool) -> Option<u8> {
    let os = win.get_key_state(k) & 0x8000 != 0;
    if !down {
        (focus && os).then_some(IST_PRESS)
    } else {
        (!os).then_some(IST_RELEASE)
    }
}

/// What UpdateInput needs around it: the VM in game, or the tests.
pub(crate) trait UpdateInputHost {
    fn win(&mut self) -> &mut WinInput;
    /// Gamepad part (pad.rs); true if the gamepad was read (Poll and GetDeviceState succeeded).
    fn pad(&mut self) -> bool;
    /// UInput::KeyDown, read at the time of the reading.
    fn key_down(&mut self, k: u32) -> bool;
    /// UWindowsViewport::CauseInputEvent: the viewport's UEngine::InputEvent.
    fn cause_input_event(&mut self, k: u8, a: u8, v: f32);
}

/// UWindowsViewport::UpdateInput: keyboard reset (non-zero argument), gamepad,
/// mouse, keyboard reading.
pub(crate) fn update_input_host(h: &mut dyn UpdateInputHost, param: u32) {
    if param != 0 {
        h.win().reset_keyboard();
    }
    let gamepad = h.pad();
    for (k, a, v) in update_input_mouse(h.win()) {
        h.cause_input_event(k, a, v);
    }
    // Keys already handled: buttons and axes of the gamepad read,
    // mouse buttons.
    let mut marked = [false; 256];
    if gamepad {
        for k in (200..216).chain([0xE0, 0xE1, 0xEA, 0xE3, 0xE8, 0xE9]) {
            marked[k] = true;
        }
    }
    for k in [1, 2, 4] {
        marked[k] = true;
    }
    let focus = param != 0 && h.win().focus;
    for k in 0..256u32 {
        if marked[k as usize] {
            continue;
        }
        let down = h.key_down(k);
        if let Some(a) = update_input_key(h.win(), focus, k, down) {
            h.cause_input_event(k as u8, a, 0.0);
        }
    }
}

struct VmHost<'a> {
    vm: &'a mut Vm,
    pc: ObjId,
    dt: f32,
}

impl UpdateInputHost for VmHost<'_> {
    fn win(&mut self) -> &mut WinInput {
        &mut self.vm.win_input
    }
    fn pad(&mut self) -> bool {
        self.vm.update_input_pad(self.pc, self.dt)
    }
    fn key_down(&mut self, k: u32) -> bool {
        self.vm.input_key_down(k as i32)
    }
    fn cause_input_event(&mut self, k: u8, a: u8, v: f32) {
        if self.vm.objects[self.pc].is_some() {
            self.vm.input_process(self.pc, k, a, v);
        }
    }
}

impl Vm {
    /// Message of the player's window (WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_CHAR, WM_SYSCHAR)
    /// passed to ViewportWndProc. For a key, the keyboard state (`set_key`) is already up to date,
    /// as when Windows delivers the message. The events are queued (`input_event`), handled at the start
    /// of the frame like the messages read before the Tick.
    pub fn win_message(&mut self, msg: u32, wparam: u32, lparam: u32) -> WinAction {
        self.win_input.active = true;
        // Client's viewport with an actor (Viewport+0x30): here, the player's UInput exists.
        let ready = self.input_sys.is_some();
        let mut v = WndViewport {
            in_list: ready,
            actor: ready,
            hwnd: 1,
            window_open: true,
            flags: u32::from(self.fullscreen),
            f1b4: self.win_input.capture,
            console: true,
            input: ready,
            rendev: true,
            mouse: true,
            ..Default::default()
        };
        enum Effect {
            Event(u8, u8, f32),
            Reset,
        }
        let slides = self.win_state_slides();
        let mut action = WinAction::Nothing;
        let mut mouse_value = None;
        let mut effects: Vec<Effect> = Vec::new();
        let win = &mut self.win_input;
        let mut host = |c: WndCall| -> u32 {
            match c {
                WndCall::CauseInputEvent(k, a, val) => {
                    effects.push(Effect::Event(k as u8, a as u8, f32::from_bits(val)));
                    0
                }
                WndCall::GetAsyncKeyState(vk) => win.get_async_key_state(vk),
                WndCall::GetKeyState(vk) => win.get_key_state(vk),
                WndCall::Viewport(0xB8, a) => {
                    win.set_mouse_capture(a[0], a[1], a[2]);
                    0
                }
                WndCall::Viewport(0x80, a) => win.set_drag(a[0], &slides),
                // UGameEngine::MousePosition: Viewport.WindowsMouseX / Y.
                WndCall::Engine(0x94, a) => {
                    mouse_value = Some((f32::from_bits(a[1]), f32::from_bits(a[2])));
                    0
                }
                // UEngine::Key (WM_CHAR): queued as a character.
                WndCall::Engine(0x84, a) => {
                    effects.push(Effect::Event(a[0] as u8, IST_TYPE, 0.0));
                    0
                }
                WndCall::InputReset => {
                    effects.push(Effect::Reset);
                    0
                }
                WndCall::ToggleFullscreen => {
                    action = WinAction::FullToggleScreen;
                    0
                }
                WndCall::Closing => {
                    action = WinAction::Closing;
                    0
                }
                // The client returns the current viewport: this one.
                WndCall::Client(0x98, _) => crate::wndproc::VP,
                // Windows windows (DefWindowProc, PostMessage, foreground, cursor, shortcuts), DirectInput
                // (Acquire / Unacquire), RenDev, repaint, log; fullscreen left on loss of focus: done
                // by the display layer (window minimized, restored on return).
                _ => 0,
            }
        };
        viewport_wnd_proc(&mut v, msg, wparam, lparam, WHEEL_MSG, &mut host);
        // Lockstep co-op: this window's mouse goes to its player through their next command (the
        // menus read it on every machine).
        if let (Some(pos), Some(l)) = (mouse_value, self.lockstep.as_mut()) {
            l.local_wmouse.0 = Some(pos);
        } else if let Some((x, y)) = mouse_value {
            if let Some(p) = self.local_viewport() {
                self.set_prop(p, "WindowsMouseX", crate::value::Value::Float(x));
                self.set_prop(p, "WindowsMouseY", crate::value::Value::Float(y));
            }
        }
        self.win_sync_windows_mouse();
        for e in effects {
            match e {
                Effect::Event(k, a, val) => self.input_event(k, a, val),
                Effect::Reset => self.reset_input(),
            }
        }
        action
    }

    /// SetMouseCapture called outside ViewportWndProc: ToggleFullscreen and ResizeViewport
    /// of the display layer.
    pub fn win_set_mouse_capture(&mut self, capture: u32, clip: u32, focus_only: u32) {
        self.win_input.active = true;
        self.win_input.set_mouse_capture(capture, clip, focus_only);
        self.win_sync_windows_mouse();
    }

    /// UViewport::SetDrag of the player's viewport outside ViewportWndProc (ResizeViewport of the display layer: 1 in fullscreen,
    /// 0 in windowed mode); returns the old drag.
    pub fn win_set_drag(&mut self, fresh_list_new: u32) -> u32 {
        self.win_input.active = true;
        let g = self.win_state_slides();
        let r = self.win_input.set_drag(fresh_list_new, &g);
        self.win_sync_windows_mouse();
        r
    }

    /// What MouseDelta reads: client and a viewport (the local player), GLevel, the viewport's bShowWindowsMouse,
    /// Client->CaptureMouse (configuration property of [WinDrv.WindowsClient], UBoolProperty::ImportText; 0
    /// without a value), fullscreen, LevelInfo.Pauser.
    pub(crate) fn win_state_slides(&mut self) -> StateSlides {
        let vp = self.machine_viewport();
        let show = vp.is_some_and(|p| self.propr(p, "bShowWindowsMouse").as_bool());
        let (true_value, false_value) = self.bool_words();
        let capture_mouse = self
            .config
            .as_mut()
            .and_then(|c| {
                let v = c.get_string("WinDrv.WindowsClient", "CaptureMouse", "System")?;
                let t: Vec<char> = v.chars().collect();
                let (tok, _) = crate::config::read_token(&t, 0, 0x400, false, &mut Vec::new())?;
                if tok.eq_ignore_ascii_case("1") || tok.eq_ignore_ascii_case("True") || tok.eq_ignore_ascii_case(&true_value) {
                    Some(1)
                } else if tok.eq_ignore_ascii_case("0") || tok.eq_ignore_ascii_case("False") || tok.eq_ignore_ascii_case(&false_value) {
                    Some(0)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        let pauser = self.level.is_some_and(|l| self.po(l, "Pauser").is_some());
        StateSlides {
            state_value: crate::drag::StateMouseDelta {
                client: true,
                viewports: 1,
                glevel: self.level.is_some(),
                bits_viewport: (self.win_input.capture_vp.f38 & 1) | (u32::from(show) << 1),
                capture_mouse,
            },
            full_screen: u32::from(self.fullscreen),
            pauser: u32::from(pauser),
        }
    }

    /// bWindowsMouseAvailable (Viewport+0x38 bit 0) of the object of the player's viewport, once SetMouseCapture
    /// has been called: the object is recreated on every map, so the value is set again at each reading.
    pub(crate) fn win_sync_windows_mouse(&mut self) {
        if let (Some(b), Some(l)) = (self.win_input.windows_mouse, self.lockstep.as_mut()) {
            if l.local_wmouse.1 != Some(b) {
                l.local_wmouse.1 = Some(b);
            }
            return;
        }
        if let (Some(b), Some(p)) = (self.win_input.windows_mouse, self.local_viewport()) {
            if self.propr(p, "bWindowsMouseAvailable").as_bool() != b {
                self.set_prop(p, "bWindowsMouseAvailable", crate::value::Value::Bool(b));
            }
        }
    }

    /// Focus of the player's window: WM_SETFOCUS (UInput::ResetInput twice) or WM_KILLFOCUS
    /// (SetMouseCapture(0, 0, 0) then UInput::ResetInput) of ViewportWndProc.
    pub fn win_focus(&mut self, focused: bool) {
        self.win_input.focus = focused;
        // Lockstep co-op: the reset is this machine's player's, in their next command.
        if self.lockstep_local_reset() {
            return;
        }
        if self.win_input.active {
            self.win_message(if focused { 7 } else { 8 }, 0, 0);
        }
    }

    /// UWindowsViewport::UpdateInput of the viewport of player `pc`: gamepad (pad.rs), then, when
    /// the display layer goes through the window, mouse and keyboard reading. `param`: 0 from UViewport::ReadInput,
    /// 1 from UInput::ResetInput.
    pub(crate) fn update_input(&mut self, pc: ObjId, param: u32, dt: f32) {
        // Lockstep co-op (lockstep.rs): every player's devices come from the session's commands.
        if self.lockstep.is_some() && self.lockstep_update_input(pc, param) {
            return;
        }
        // Local split screen (coop.rs, split_versus.rs): keyboard and mouse belong to the first player only.
        let extra = self.local_split() && self.coop.players.first().is_some_and(|&first| first != pc);
        if !self.win_input.active || extra {
            self.update_input_pad(pc, dt);
            return;
        }
        self.win_sync_windows_mouse();
        update_input_host(&mut VmHost { vm: self, pc, dt }, param);
    }
}

