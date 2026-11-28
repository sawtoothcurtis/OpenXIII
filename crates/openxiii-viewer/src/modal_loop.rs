//! Windows: while the player drags or resizes the window, Windows runs its own modal loop
//! (between WM_ENTERSIZEMOVE and WM_EXITSIZEMOVE) and the event loop gets no turn: no frame, no
//! step. A lockstep session's other machine then waits for this one. A timer set for that loop
//! asks for a repaint at the steps' pace; the repaint is the event loop's RedrawRequested, so the
//! frames, and the steps, go on as with the window still, as they do minimized.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};

    type Hwnd = *mut c_void;

    #[link(name = "comctl32")]
    extern "system" {
        fn SetWindowSubclass(hwnd: Hwnd, proc_: SubclassProc, id: usize, data: usize) -> i32;
        fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    }
    #[link(name = "user32")]
    extern "system" {
        fn SetTimer(hwnd: Hwnd, id: usize, ms: u32, func: *const c_void) -> usize;
        fn KillTimer(hwnd: Hwnd, id: usize) -> i32;
        fn InvalidateRect(hwnd: Hwnd, rect: *const c_void, erase: i32) -> i32;
    }

    type SubclassProc = unsafe extern "system" fn(Hwnd, u32, usize, isize, usize, usize) -> isize;

    const WM_TIMER: u32 = 0x0113;
    const WM_ENTERSIZEMOVE: u32 = 0x0231;
    const WM_EXITSIZEMOVE: u32 = 0x0232;
    const SUBCLASS_ID: usize = 0x0E13;
    const TIMER_ID: usize = 0x0E13;
    /// The steps' pace (ms).
    const PERIOD_MS: u32 = 1000 / openxiii_vm::lockstep::STEP_HZ;

    /// Repaints asked during modal loops so far (read by the tests).
    pub static REPAINTS: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn proc_(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize, _id: usize, _data: usize) -> isize {
        match msg {
            WM_ENTERSIZEMOVE if ENABLED.load(Ordering::Relaxed) != 0 => {
                SetTimer(hwnd, TIMER_ID, PERIOD_MS, std::ptr::null());
            }
            WM_EXITSIZEMOVE => {
                KillTimer(hwnd, TIMER_ID);
            }
            WM_TIMER if wparam == TIMER_ID => {
                REPAINTS.fetch_add(1, Ordering::Relaxed);
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    static ENABLED: AtomicU32 = AtomicU32::new(1);

    /// The window `hwnd` keeps its frames going through move and size loops (`timer` false:
    /// testing only, the loop's frames stop as before).
    pub fn install(hwnd: Hwnd, timer: bool) -> bool {
        ENABLED.store(u32::from(timer), Ordering::Relaxed);
        // SAFETY: subclassing a window of this thread with a procedure that lives as long as the
        // program; DefSubclassProc passes every message on.
        unsafe { SetWindowSubclass(hwnd, proc_, SUBCLASS_ID, 0) != 0 }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[link(name = "user32")]
        extern "system" {
            fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: *mut c_void, inst: *mut c_void, param: *mut c_void) -> Hwnd;
            fn DestroyWindow(hwnd: Hwnd) -> i32;
            fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
            fn PeekMessageW(msg: *mut [usize; 6], hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
            fn DispatchMessageW(msg: *const [usize; 6]) -> isize;
        }

        /// A hidden window (never shown) in a move loop: a repaint asked about every step while
        /// the loop lasts, none after it ends. Negative control: without the timer, none.
        #[test]
        fn move_loop_keeps_asking_for_frames() {
            let run = |timer: bool| {
                let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
                // SAFETY: a plain hidden window of this thread, destroyed at the end.
                unsafe {
                    let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 64, 64, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                    assert!(!hwnd.is_null());
                    assert!(install(hwnd, timer));
                    let pump = |ms: u64| {
                        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                        let mut m = [0usize; 6];
                        while std::time::Instant::now() < end {
                            while PeekMessageW(&mut m, std::ptr::null_mut(), 0, 0, 1) != 0 {
                                DispatchMessageW(&m);
                            }
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                    };
                    let before = REPAINTS.load(Ordering::Relaxed);
                    SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
                    pump(500);
                    let during = REPAINTS.load(Ordering::Relaxed) - before;
                    SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
                    let at_end = REPAINTS.load(Ordering::Relaxed);
                    pump(200);
                    let after = REPAINTS.load(Ordering::Relaxed) - at_end;
                    DestroyWindow(hwnd);
                    (during, after)
                }
            };
            let (during, after) = run(true);
            // Windows' timers are 15.6 ms at worst: 500 ms give 20 to 30 ticks.
            assert!(during >= 15, "{during} repaints asked in 500 ms of a move loop");
            assert_eq!(after, 0, "none once the loop ended");
            let (during, _) = run(false);
            assert_eq!(during, 0, "negative control: no timer, no frame");
        }
    }
}

#[cfg(windows)]
pub use imp::install;

/// The game's window keeps its frames going through Windows' move and size loops.
#[cfg(windows)]
pub fn keep_frames_while_dragged(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(h) = window.window_handle() {
        if let RawWindowHandle::Win32(w) = h.as_raw() {
            install(w.hwnd.get() as *mut std::ffi::c_void, true);
        }
    }
}
