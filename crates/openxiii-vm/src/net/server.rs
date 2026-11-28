//! Dedicated server: loop of UServerCommandlet::Main, started by "UCC server URL"
//! (ServerLauncher.exe). Without Direct3D the FPU stays at 53 bits: the x87 computations are those of f64.

/// rdtsc counter → seconds as in Main: fild of the two 32-bit words (2^32 added when the sign
/// bit is set), high × 2^32 + low, then × GSecondsPerCycle.
pub fn cycles_seconds(cycles: u64, seconds_per_cycle: f64) -> f64 {
    let (hi, lo) = ((cycles >> 32) as u32, cycles as u32);
    (hi as f64 * 4294967296.0 + lo as f64) * seconds_per_cycle
}

/// Loop state: OldTime, SecondStartTime, TickCount; CurrentTickRate is the engine's field
/// (UEngine +0xB8), recomputed here.
#[derive(Clone, Debug, Default)]
pub struct ServerLoop {
    pub old_time: f64,
    pub second_start_time: f64,
    pub tick_count: i32,
    pub current_tick_rate: f32,
}

impl ServerLoop {
    /// Before the loop: OldTime = SecondStartTime = appSeconds().
    pub fn new(start: f64) -> Self {
        ServerLoop { old_time: start, second_start_time: start, ..Default::default() }
    }

    /// Duration passed to GEngine->Tick: (float)(NewTime − OldTime).
    pub fn delta(&self, new_time: f64) -> f32 {
        (new_time - self.old_time) as f32
    }

    /// After Tick: OldTime = NewTime, TickCount + 1, and once the second has passed (NewTime >
    /// SecondStartTime + 1.0) CurrentTickRate = TickCount / (NewTime − SecondStartTime).
    pub fn after_tick(&mut self, new_time: f64) {
        self.old_time = new_time;
        self.tick_count += 1;
        if new_time > self.second_start_time + 1.0 {
            self.current_tick_rate = (self.tick_count as f64 / (new_time - self.second_start_time)) as f32;
            self.second_start_time = new_time;
            self.tick_count = 0;
        }
    }

    /// Wait given to appSleep: nothing if MaxTickRate ≤ 0; otherwise 1.0f / MaxTickRate − (now −
    /// NewTime), 0 if that is not positive.
    pub fn sleep_seconds(max_tick_rate: f32, now: f64, new_time: f64) -> Option<f32> {
        if !(max_tick_rate > 0.0) {
            return None;
        }
        let s = 1.0f64 / max_tick_rate as f64 - (now - new_time);
        Some(if 0.0 < s { s as f32 } else { 0.0 })
    }
}

/// UServerCommandlet::StaticConstructor, called on the default object when the class is registered: UCommandlet's
/// boolean word (+0x1D8) with LogToStdout (1), IsServer (2) and LazyLoad (0x10) set, IsClient (4) and IsEditor (8)
/// cleared; the other bits stay.
pub fn server_commandlet_defaults(bits: u32) -> u32 {
    let clears = 0xC;
    (bits & !clears) | 0x13
}

/// One key read from the console (_kbhit then _getch); Q or q requests exit.
pub fn quit_key(key: Option<u8>) -> bool {
    matches!(key, Some(b'Q' | b'q'))
}

/// Console _kbhit / _getch (platform).
pub fn console_key() -> Option<u8> {
    #[cfg(windows)]
    {
        extern "C" {
            fn _kbhit() -> i32;
            fn _getch() -> i32;
        }
        // SAFETY: CRT functions without arguments.
        unsafe {
            if _kbhit() != 0 {
                return Some(_getch() as u8);
            }
        }
    }
    None
}

/// Log name of UCC.exe (FOutputDeviceFile::Serialize): appBaseDir then the value of
/// "LOG="; otherwise the value of "ABSLOG=" alone; otherwise appBaseDir + appPackage ("UCC") + ".log". Parse
/// (appStrfind) only finds a key at the start of a word: "ABSLOG=" does not count as "LOG=".
/// The buffer is 0x400 characters.
pub fn ucc_log_path(cmd_line: &str, base_dir: &str) -> String {
    let mut path: String = base_dir.chars().take(0x3FF).collect();
    let room = 0x400usize.saturating_sub(path.chars().count());
    if let Some(v) = crate::value::parse_fstring(cmd_line, "LOG=") {
        path.extend(v.chars().take(room.saturating_sub(1)));
    } else if let Some(v) = crate::value::parse_fstring(cmd_line, "ABSLOG=") {
        path = v.chars().take(0x3FF).collect();
    } else {
        path.push_str("UCC.log");
    }
    path
}

/// UCC.exe's FOutputDeviceFile: opened at the first message (overwritten, "Log file open, <appTimestamp>"
/// written first), one line "Event: text\r\n" per message, except suppressed events
/// (FName::SafeSuppressed, [Core.System] Suppress=) and Title (0x314); "Log file closed, …" at the end.
pub struct UccLog {
    path: String,
    file: Option<std::fs::File>,
    dead: bool,
    suppressed: Vec<String>,
}

impl UccLog {
    pub fn new(path: String, suppressed: Vec<String>) -> Self {
        UccLog { path, file: None, dead: false, suppressed }
    }

    /// The text written for a message (event, text), or nothing.
    pub fn format(&self, event: &str, data: &str) -> Option<String> {
        if self.suppressed.iter().any(|s| s.eq_ignore_ascii_case(event)) || event.eq_ignore_ascii_case("Title") {
            return None;
        }
        Some(format!("{event}: {data}\r\n"))
    }

    /// One line of the VM log, "Event: text".
    pub fn serialize_line(&mut self, line: &str) {
        let (event, data) = line.split_once(": ").unwrap_or(("Log", line));
        self.serialize(event, data);
    }

    pub fn serialize(&mut self, event: &str, data: &str) {
        if self.suppressed.iter().any(|s| s.eq_ignore_ascii_case(event)) {
            return;
        }
        if self.file.is_none() && !self.dead {
            match std::fs::File::create(&self.path) {
                Ok(f) => {
                    self.file = Some(f);
                    let stamp = chrono::Local::now().format("%m/%d/%y %H:%M:%S").to_string();
                    self.serialize("Log", &format!("Log file open, {stamp}"));
                }
                Err(_) => self.dead = true,
            }
        }
        if let (Some(text), Some(f)) = (self.format(event, data), self.file.as_mut()) {
            use std::io::Write;
            let _ = f.write_all(text.as_bytes());
        }
    }

    /// FOutputDeviceFile::TearDown: "Log file closed, <appTimestamp>" then close.
    pub fn close(&mut self) {
        if self.file.is_some() {
            let stamp = chrono::Local::now().format("%m/%d/%y %H:%M:%S").to_string();
            self.serialize("Log", &format!("Log file closed, {stamp}"));
            self.file = None;
        }
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;

    #[test]
    fn ucc_log_name() {
        let base = r"C:\Games\XIII\System\";
        assert_eq!(ucc_log_path("DM_Hual1?LAN -port=7777", base), r"C:\Games\XIII\System\UCC.log");
        assert_eq!(ucc_log_path("DM_Hual1 log=server.log", base), r"C:\Games\XIII\System\server.log");
        assert_eq!(ucc_log_path("DM_Hual1 LOG=\"a b.log\"", base), r"C:\Games\XIII\System\a b.log");
        assert_eq!(ucc_log_path(r"DM_Hual1 ABSLOG=C:\j.log", base), r"C:\j.log");
    }

    #[test]
    fn ucc_log_lines() {
        let j = UccLog::new(String::new(), vec!["DevNet".into()]);
        assert_eq!(j.format("Log", "Hello").as_deref(), Some("Log: Hello\r\n"));
        assert_eq!(j.format("ScriptLog", "x: y").as_deref(), Some("ScriptLog: x: y\r\n"));
        assert_eq!(j.format("Title", "DuelRoom"), None);
        assert_eq!(j.format("devnet", "z"), None);
    }
}

