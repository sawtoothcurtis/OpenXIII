//! Game log on Android: stdout and stderr (OpenXIII's println!) go to logcat
//! (tag "OpenXIII") and to an OpenXIII.log file, recreated on each launch.

use std::ffi::{c_char, CString};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::path::Path;

#[link(name = "log")]
extern "C" {
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

extern "C" {
    fn pipe(fds: *mut i32) -> i32;
    fn dup2(old: i32, new: i32) -> i32;
}

const ANDROID_LOG_INFO: i32 = 4;

/// `timed`: a second file the lines are added to, each with the seconds since the launch
/// (test builds: a file the tester can reach and send back).
pub fn redirect(file_value: &Path, timed: Option<&Path>) {
    let mut fds = [0i32; 2];
    if unsafe { pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    unsafe {
        dup2(fds[1], 1);
        dup2(fds[1], 2);
    }
    if let Some(d) = file_value.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut log = std::fs::File::create(file_value).ok();
    let mut timed = timed.and_then(|p| std::fs::OpenOptions::new().create(true).append(true).open(p).ok());
    let started = std::time::Instant::now();
    if let Some(f) = &mut timed {
        let _ = writeln!(f, "---- launch ----");
    }
    let read_value = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    std::thread::spawn(move || {
        let tag = CString::new("OpenXIII").unwrap();
        for line_value in BufReader::new(read_value).lines() {
            let Ok(line_value) = line_value else { break };
            if let Some(f) = &mut log {
                let _ = writeln!(f, "{line_value}");
            }
            if let Some(f) = &mut timed {
                let _ = writeln!(f, "{:8.3} {line_value}", started.elapsed().as_secs_f64());
            }
            if let Ok(c) = CString::new(line_value) {
                unsafe { __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), c.as_ptr()) };
            }
        }
    });
}
