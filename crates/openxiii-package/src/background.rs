//! Priority of the threads that make pictures in the background (the improved menus' pictures,
//! the co-op arms): below the game's own threads, so that on a computer with few cores the game
//! keeps its frames and the pictures take the time left over.

/// Windows thread priority of the background makers (THREAD_PRIORITY_LOWEST).
#[cfg(windows)]
const LOWEST: i32 = -2;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> isize;
    fn GetThreadPriority(thread: isize) -> i32;
    fn SetThreadPriority(thread: isize, priority: i32) -> i32;
}

/// Lowers the calling thread's priority (on Windows; elsewhere nothing changes).
pub fn lower_this_thread() {
    #[cfg(windows)]
    unsafe {
        SetThreadPriority(GetCurrentThread(), LOWEST);
    }
}

/// The calling thread's priority, to give a helper thread it starts (see `take`).
#[derive(Clone, Copy, Debug)]
pub struct Priority(#[allow(dead_code)] i32);

/// The calling thread's priority.
pub fn current() -> Priority {
    #[cfg(windows)]
    unsafe {
        let p = GetThreadPriority(GetCurrentThread());
        // THREAD_PRIORITY_ERROR_RETURN: keep the default.
        return Priority(if p == 0x7fff_ffff { 0 } else { p });
    }
    #[cfg(not(windows))]
    Priority(0)
}

/// Gives the calling thread priority `p` (a helper thread started by a background maker works
/// at its maker's priority; one started by the game's thread at the game's).
pub fn take(p: Priority) {
    #[cfg(windows)]
    unsafe {
        if p.0 != 0 {
            SetThreadPriority(GetCurrentThread(), p.0);
        }
    }
    #[cfg(not(windows))]
    let _ = p;
}
