//! Windows: the game's window takes the foreground once it is open, as the game's own window
//! did (WinDrv: SetFocus on WM_CREATE, SetForegroundWindow and SetActiveWindow when the mouse is
//! captured). The window is shown before the render device and the first map are made, which
//! takes seconds without the message queue being read; a window (unlike a fullscreen one, which
//! winit activates again when it goes fullscreen) then stayed in the background: no keys, the
//! world paused, until the player clicked it, and Alt+Enter and Alt+F4 went to another window.
//! The same happened after any long stall, through Windows' ghost window
//! ([`keep_focus_through_stalls`]).

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn GetForegroundWindow() -> *mut c_void;
        fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
        fn IsIconic(hwnd: *mut c_void) -> i32;
        fn DisableProcessWindowsGhosting();
    }

    /// A window that reads no message for 5 seconds (the first frames at a high render scale,
    /// a map being loaded) is replaced on the screen by a "Not responding" ghost window owned
    /// by Windows, which takes the foreground: the keys, Alt+Enter and Alt+F4 then went to that
    /// ghost, and the game's window could stay without the focus until the player clicked it.
    /// Without ghosting, the window keeps the foreground and its keys wait in its queue.
    pub fn keep_focus_through_stalls() {
        // SAFETY: no argument; affects only this process's windows.
        unsafe { DisableProcessWindowsGhosting() }
    }

    fn hwnd_of(window: &winit::window::Window) -> Option<*mut c_void> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(w) => Some(w.hwnd.get() as *mut c_void),
            _ => None,
        }
    }

    /// The window is brought to the foreground unless it already is, is hidden or minimized.
    /// Windows may refuse SetForegroundWindow to a process that has not had input since it
    /// started (the foreground lock); winit's focus_window is then the fallback. Returns
    /// whether the window is the foreground one afterwards.
    pub fn bring_to_front(window: &winit::window::Window) -> bool {
        let Some(hwnd) = hwnd_of(window) else { return false };
        // SAFETY: plain user32 calls on this thread's own window handle.
        unsafe {
            if GetForegroundWindow() == hwnd {
                return true;
            }
            if window.is_visible() == Some(false) || IsIconic(hwnd) != 0 {
                return false;
            }
            if SetForegroundWindow(hwnd) == 0 || GetForegroundWindow() != hwnd {
                window.focus_window();
            }
            GetForegroundWindow() == hwnd
        }
    }
}

#[cfg(windows)]
pub use imp::{bring_to_front, keep_focus_through_stalls};

/// Other systems: no ghost windows.
#[cfg(not(windows))]
pub fn keep_focus_through_stalls() {}

/// Other systems: the window manager gives the focus to a new window.
#[cfg(not(windows))]
pub fn bring_to_front(_window: &winit::window::Window) -> bool {
    true
}
