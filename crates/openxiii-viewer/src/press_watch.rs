//! Improved build: presses watched by a thread while the main thread is blocked building a level
//! behind the loading recap (main.rs, recap_prepare). The window's events are not read meanwhile,
//! so the keys, mouse buttons and XInput gamepad buttons are read directly, while the game's window
//! is in front. A key or button already down when the watch starts is a press only once released
//! and pressed again. A press sets the shared flag (the hold thread then shows the loading
//! animation: Renderer::hold_switch) and its instant is kept.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Inputs down now (keys and mouse buttons by virtual key code, then gamepad buttons) compared with
/// those down before: true when one is newly down. `before` becomes `now`.
pub fn new_press(before: &mut Vec<bool>, now: Vec<bool>) -> bool {
    let pressed = now.iter().enumerate().any(|(i, &d)| d && !before.get(i).copied().unwrap_or(false));
    *before = now;
    pressed
}

/// The watching thread; stopped by `stop` (or when dropped).
pub struct PressWatch {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// When the first press came.
    pub at: Arc<Mutex<Option<Instant>>>,
}

impl PressWatch {
    /// Starts watching; `flag` is set on the first press. None where inputs cannot be read
    /// outside the event loop.
    pub fn start(flag: Arc<AtomicBool>) -> Option<PressWatch> {
        let read = platform::reader()?;
        let stop = Arc::new(AtomicBool::new(false));
        let at = Arc::new(Mutex::new(None));
        let (stop_thread, at_thread) = (stop.clone(), at.clone());
        let thread = std::thread::Builder::new()
            .name("press watch".into())
            .spawn(move || {
                let mut before = read();
                while !stop_thread.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    if !platform::in_front() {
                        // In the background: what is pressed elsewhere is not for the game.
                        before = read();
                        continue;
                    }
                    if new_press(&mut before, read()) {
                        if let Ok(mut a) = at_thread.lock() {
                            a.get_or_insert_with(Instant::now);
                        }
                        flag.store(true, Ordering::Release);
                    }
                }
            })
            .ok()?;
        Some(PressWatch { stop, thread: Some(thread), at })
    }

    /// Stops the thread; the instant of the first press, if any.
    pub fn stop(mut self) -> Option<Instant> {
        self.halt();
        self.at.lock().ok().and_then(|a| *a)
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PressWatch {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(windows)]
mod platform {
    #[repr(C)]
    #[derive(Default)]
    struct XInputState {
        packet: u32,
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        thumbs: [i16; 4],
    }
    type GetState = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(window: isize, process: *mut u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(name: *const u8) -> *mut std::ffi::c_void;
        fn GetProcAddress(module: *mut std::ffi::c_void, name: *const u8) -> *mut std::ffi::c_void;
        fn GetCurrentProcessId() -> u32;
    }

    /// XInputGetState of the system's XInput, if present.
    fn xinput() -> Option<GetState> {
        for dll in [&b"xinput1_4.dll\0"[..], b"xinput9_1_0.dll\0"] {
            // SAFETY: null-terminated names; the function has XInputGetState's signature.
            unsafe {
                let m = LoadLibraryA(dll.as_ptr());
                if !m.is_null() {
                    let f = GetProcAddress(m, b"XInputGetState\0".as_ptr());
                    if !f.is_null() {
                        return Some(std::mem::transmute::<*mut std::ffi::c_void, GetState>(f));
                    }
                }
            }
        }
        None
    }

    /// Reads the keys and mouse buttons (virtual key codes 1 to 254) and the buttons and triggers
    /// of the four XInput gamepads.
    pub fn reader() -> Option<impl Fn() -> Vec<bool> + Send + 'static> {
        let pad = xinput();
        Some(move || {
            let mut v: Vec<bool> = (1..255).map(|k| unsafe { GetAsyncKeyState(k) as u16 & 0x8000 != 0 }).collect();
            for user in 0..4 {
                let mut s = XInputState::default();
                // SAFETY: XInputGetState writes one XINPUT_STATE.
                let ok = pad.is_some_and(|f| unsafe { f(user, &mut s) } == 0);
                v.extend((0..16).map(|b| ok && s.buttons & (1 << b) != 0));
                v.push(ok && s.left_trigger > 127);
                v.push(ok && s.right_trigger > 127);
            }
            v
        })
    }

    /// A window of this process is in front.
    pub fn in_front() -> bool {
        let mut pid = 0;
        // SAFETY: plain queries.
        unsafe {
            let w = GetForegroundWindow();
            w != 0 && GetWindowThreadProcessId(w, &mut pid) != 0 && pid == GetCurrentProcessId()
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// No reading outside the event loop: a press during the preparation acts once it is done.
    pub fn reader() -> Option<fn() -> Vec<bool>> {
        None
    }

    pub fn in_front() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only an input newly down is a press: one held from before is not, until released and
    /// pressed again; an input released is not.
    #[test]
    fn new_presses_only() {
        let mut before = vec![false, true, false];
        assert!(!new_press(&mut before, vec![false, true, false]), "held from before");
        assert!(!new_press(&mut before, vec![false, false, false]), "a release");
        assert!(new_press(&mut before, vec![false, true, false]), "pressed again");
        assert!(!new_press(&mut before, vec![false, true, false]), "still held");
        assert!(new_press(&mut before, vec![true, true, false]));
        // A longer list (a gamepad plugged meanwhile) with a button down is a press.
        let mut before = vec![false];
        assert!(new_press(&mut before, vec![false, true]));
    }

    /// The hold's turn (Renderer::hold_start): each kept frame `step` ms in order, then again
    /// from the first; nothing to show without frames. The loading animation shown after a
    /// press during the preparation starts its own turn from its first frame.
    #[test]
    fn the_hold_turns_through_its_frames() {
        use crate::renderer::held_index;
        assert_eq!(held_index(0, 500, 100), None);
        assert_eq!(held_index(3, 0, 100), Some(0));
        assert_eq!(held_index(3, 250, 100), Some(2));
        assert_eq!(held_index(3, 350, 100), Some(0));
        assert_eq!(held_index(1, 99_999, 100), Some(0));
        assert_eq!(held_index(2, 7, 0), Some(1), "a zero step counts as 1 ms");
    }
}
