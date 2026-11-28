//! Natives of StatLog / StatLogFile (game statistics, Engine.dll AStatLog.cpp) and
//! a few isolated natives: UObject::GetEnum, AActor::MoveCacheEntry, AHUD::Draw3DLine,
//! APlayerController::GetEntryLevel.
//!
//! XIII does not log its games (bLocalLog and bWorldLog absent from the ini files, hence false): these
//! natives only run if a script creates a StatLog. The files are written as the
//! engine does (`..\Logs`, relative to the System folder); the natives that launch an external program
//! (appCreateProc, appLaunchURL) launch nothing here.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;
use crate::{ObjId, Vm};
use std::io::Write;

/// C++ state of an AStatLogFile: LogAr (FArchive* of the file) and Context (FMD5Context*).
#[derive(Default)]
pub struct StatLogFileState {
    file: Option<std::fs::File>,
    md5: Option<Md5>,
}

impl Vm {
    /// System folder (base of the engine's relative paths, GFileManager).
    fn system_dir(&self) -> std::path::PathBuf {
        self.config.as_ref().map(|c| c.dir().to_path_buf()).unwrap_or_default()
    }

    pub(crate) fn stat_log_native(&mut self, name: &str, this: ObjId, args: &mut [Value]) -> Option<Value> {
        let s = |i: usize| args.get(i).map(|v| v.to_string()).unwrap_or_default();
        Some(match name {
            // UObject::execGetEnum: E.Names(i) if E is a UEnum and
            // 0 <= i < Names.Num(), otherwise None.
            "GetEnum" => {
                let i = args.get(1).map_or(0, |v| v.as_int());
                let names = match args.first() {
                    Some(Value::Asset(path)) => self.enum_names(path),
                    _ => None,
                };
                // None: FName 0 (as in the original), the VM's empty name (== '' or 'None').
                Value::Name(crate::value::NameStruct::from(enum_get(names.as_deref().map(|v| v.as_slice()), i)))
            }
            // APlayerController::execGetEntryLevel: LevelInfo of GEntry (the Entry
            // map that UGameEngine::Init loads alongside the game). The VM does not load Entry:
            // None (no XIII script calls it).
            "GetEntryLevel" => Value::Object(None),
            // AActor::execMoveCacheEntry: [Cache] Guid of GSys->CachePath\cache.ini;
            // without an entry, False. With an entry, the engine moves the cache file to the
            // Paths folder with the same extension: not reimplemented (XIII has no cache).
            "MoveCacheEntry" => {
                let cache = self.config.as_mut().and_then(|c| c.get_string("Core.System", "CachePath", "System")).unwrap_or_else(|| "..\\Cache".into());
                let ini = self.system_dir().join(cache.replace('\\', "/")).join("cache.ini");
                let found = std::fs::read(&ini)
                    .ok()
                    .map(|b| crate::config::ConfigFile::parse(&b.iter().map(|&c| c as char).collect::<String>()))
                    .and_then(|f| f.section("Cache").and_then(|sec| sec.iter().find(|(k, _)| k.eq_ignore_ascii_case(&s(0))).map(|(_, v)| v.clone())));
                if found.is_some() {
                    self.warn(format!("MoveCacheEntry {}: cache entry found, move not reimplemented", s(0)));
                }
                Value::Bool(false)
            }
            // AHUD::execDraw3DLine: debug line drawn directly into the
            // viewport's RenderInterface (FLineBatcher), outside the Canvas: nothing in the game state.
            "Draw3DLine" if self.is_a(this, "HUD") => Value::Void,
            // AStatLog::execGetGMTRef → appGetGMTRef: offset of
            // the time zone's standard time in whole hours, "+%1.1f" if positive, "%1.1f" otherwise.
            "GetGMTRef" if self.is_a(this, "StatLog") => Value::Str(gmt_ref_text(standard_offset_seconds())),
            // AStatLog::execGetMapFileName: XLevel->URL.Map.
            "GetMapFileName" if self.is_a(this, "StatLog") => Value::Str(self.url.map.clone()),
            // AStatLog::execLogMutator: LogGameSpecial("GameMutator",
            // M.Class.GetFullName()).
            "LogMutator" if self.is_a(this, "StatLog") => {
                let m = args.first().and_then(|v| v.as_object());
                let full = m.map(|m| self.class_full_name(m)).unwrap_or_default();
                self.event_pub(this, "LogGameSpecial", vec![Value::Str("GameMutator".into()), Value::Str(full)]);
                Value::Void
            }
            // AStatLog::execInitialCheck: LogGameSpecial("GameClass", class of Game),
            // then for each package that contains a class (in GObjObjects order):
            // MD5 of NAME.u in uppercase + its size, LogGameSpecial2("CodePackageChecksum",
            // "Package.u", md5); then the same with Package.dll, the .u string staying in front.
            "InitialCheck" if self.is_a(this, "StatLog") => {
                let game = args.first().and_then(|v| v.as_object());
                let full = game.map(|g| self.class_full_name(g)).unwrap_or_default();
                self.event_pub(this, "LogGameSpecial", vec![Value::Str("GameClass".into()), Value::Str(full)]);
                let mut packages: Vec<String> = Vec::new();
                for c in &self.classes {
                    let p = self.packages[c.package].name.clone();
                    if !packages.iter().any(|x| x.eq_ignore_ascii_case(&p)) {
                        packages.push(p);
                    }
                }
                let dir = self.system_dir();
                let size = |f: &str| std::fs::metadata(dir.join(f)).map_or(-1, |m| m.len() as i32);
                for (file, md5) in code_package_checksums(&packages, &size) {
                    self.event_pub(this, "LogGameSpecial2", vec![Value::Str("CodePackageChecksum".into()), Value::Str(file), Value::Str(md5)]);
                }
                Value::Void
            }
            // AStatLog::execGetPlayerChecksum: "NoChecksum" without ngWorldSecret,
            // otherwise MD5(PlayerReplicationInfo.PlayerName + ngWorldSecret), Latin-1 bytes, in hexadecimal.
            "GetPlayerChecksum" if self.is_a(this, "StatLog") => {
                let p = args.first().and_then(|v| v.as_object());
                let secret = p.map(|p| self.prop(p, "ngWorldSecret").to_string()).unwrap_or_default();
                let out = if secret.is_empty() {
                    "NoChecksum".to_string()
                } else {
                    let name = p.and_then(|p| self.propr(p, "PlayerReplicationInfo").as_object()).map(|r| self.prop(r, "PlayerName").to_string()).unwrap_or_default();
                    hex(&Md5::digest(&name.chars().chain(secret.chars()).map(|c| c as u32 as u8).collect::<Vec<u8>>()))
                };
                if let Some(a) = args.get_mut(1) {
                    *a = Value::Str(out);
                }
                Value::Void
            }
            // AStatLog::execBatchLocal, execExecuteLocalLogBatcher,
            // execExecuteSilentLogBatcher, execExecuteWorldLogBatcher,
            // execBrowseRelativeLocalURL: launch a program or a web page
            // (appCreateProc, appLaunchURL). Nothing here; ExecuteWorldLogBatcher keeps its lines.
            "BatchLocal" | "ExecuteLocalLogBatcher" | "ExecuteSilentLogBatcher" | "BrowseRelativeLocalURL" if self.is_a(this, "StatLog") || name == "BatchLocal" || name == "BrowseRelativeLocalURL" => Value::Void,
            "ExecuteWorldLogBatcher" if self.is_a(this, "StatLog") => {
                // Without a launched program: the engine's "Failed to launch batcher" branch, then
                // bWorldBatcherError copied into the class's default object.
                self.log.push("Log: ngWorldStats: ExecuteWorldLogBatcher".into());
                self.log.push("Log: ngWorldStats: Failed to launch batcher.".into());
                self.set_prop(this, "bWorldBatcherError", Value::Bool(true));
                self.log.push("Log: bWorldBatchError is 1".into());
                // GetDefaultObject of the static class AStatLog.
                if let Some(c) = self.class("StatLog") {
                    let cdo = self.classes[c].cdo;
                    self.set_prop(cdo, "bWorldBatcherError", Value::Bool(true));
                }
                // SaveConfig(CPF_Config, NULL) of the object.
                self.save_config(this, crate::config::CPF_CONFIG);
                Value::Void
            }
            // AStatLogFile::execOpenLog: creates ..\Logs, opens StatLogFile for writing,
            // and an MD5 context if bWorld.
            "OpenLog" if self.is_a(this, "StatLogFile") => {
                let dir = self.system_dir();
                let _ = std::fs::create_dir_all(dir.join("../Logs"));
                let path = log_path(&dir, &self.prop(this, "StatLogFile").to_string());
                let st = StatLogFileState { file: path.and_then(|p| std::fs::File::create(p).ok()), md5: self.propr(this, "bWorld").as_bool().then(Md5::new) };
                self.stat_log_files.insert(this, st);
                Value::Void
            }
            // AStatLogFile::execCloseLog: closes the file then renames it to
            // StatLogFinal (replaces it, even if it is read-only).
            "CloseLog" if self.is_a(this, "StatLogFile") => {
                self.stat_log_files.remove(&this);
                let dir = self.system_dir();
                let from = log_path(&dir, &self.prop(this, "StatLogFile").to_string());
                let to = log_path(&dir, &self.prop(this, "StatLogFinal").to_string());
                if let (Some(from), Some(to)) = (from, to) {
                    let _ = std::fs::rename(from, to);
                }
                Value::Void
            }
            // AStatLogFile::execWatermark: MD5Update(EventString + "\r\n").
            "Watermark" if self.is_a(this, "StatLogFile") => {
                let text = format!("{}\r\n", s(0));
                if let Some(m) = self.stat_log_files.get_mut(&this).and_then(|st| st.md5.as_mut()) {
                    m.update(&text.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>());
                }
                Value::Void
            }
            // AStatLogFile::execGetChecksum: MD5Final, then the 16 bytes as %02x appended to Checksum.
            // The original also mixes in a private key of the online stats service before MD5Final; that service no
            // longer exists and the key is not reproduced here, so the checksum is the MD5 of the watermarked lines only.
            "GetChecksum" if self.is_a(this, "StatLogFile") => {
                let digest = self.stat_log_files.get_mut(&this).and_then(|st| st.md5.take()).map(|m| m.finish());
                let mut out = args.first().map(|v| v.to_string()).unwrap_or_default();
                out.push_str(&hex(&digest.unwrap_or_else(|| Md5::new().finish())));
                if let Some(a) = args.get_mut(0) {
                    *a = Value::Str(out);
                }
                Value::Void
            }
            // AStatLogFile::execFileFlush: LogAr->Flush().
            "FileFlush" if self.is_a(this, "StatLogFile") => {
                if let Some(f) = self.stat_log_files.get_mut(&this).and_then(|st| st.file.as_mut()) {
                    let _ = f.flush();
                }
                Value::Void
            }
            // AStatLogFile::execFileLog: EventString + "\r\n"; with bWorld,
            // each character XOR 0xA7 (written with %c: a zero result leaves nothing in the string).
            "FileLog" if self.is_a(this, "StatLogFile") => {
                let text = format!("{}\r\n", s(0));
                let world = self.propr(this, "bWorld").as_bool();
                let bytes: Vec<u8> = if world { text.chars().map(|c| (c as u32 as u8) ^ 0xA7).filter(|&b| b != 0).collect() } else { text.chars().map(|c| c as u32 as u8).collect() };
                if let Some(f) = self.stat_log_files.get_mut(&this).and_then(|st| st.file.as_mut()) {
                    let _ = f.write_all(&bytes);
                }
                Value::Void
            }
            _ => return None,
        })
    }

    /// UObject::GetFullName of an object's class: "Class Package.Name".
    fn class_full_name(&self, o: ObjId) -> String {
        let Some(obj) = self.objects.get(o).and_then(|x| x.as_ref()) else { return "None".into() };
        let c = &self.classes[obj.class];
        format!("Class {}.{}", self.packages[c.package].name, c.name)
    }
}

/// Path of a stat log file, relative to the System folder (`..\Logs\Game_1234.tmp` in the scripts).
/// The name is a script property, and scripts can come from a server or a mod: the file must stay
/// inside the game's Logs folder. An absolute path, a drive, or a `..` leaving Logs gives `None`
/// (no file opened, no rename).
pub(crate) fn log_path(system: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    // Components below the game folder (the parent of System), resolved without touching the disk.
    let mut parts: Vec<&str> = vec!["System"];
    for c in name.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c if c.contains(':') => return None,
            c => parts.push(c),
        }
    }
    if name.starts_with(['/', '\\']) || parts.len() < 2 || !parts[0].eq_ignore_ascii_case("Logs") {
        return None;
    }
    Some(parts.iter().fold(system.join(".."), |p, c| p.join(c)))
}

/// appGetGMTRef: t − mktime(gmtime(t)) in seconds (see `standard_offset_seconds`), divided by 3600 towards zero.
pub(crate) fn gmt_ref_text(offset_seconds: i32) -> String {
    let h = (offset_seconds / 3600) as f64;
    if h > 0.0 {
        format!("+{h:.1}")
    } else {
        format!("{h:.1}")
    }
}

/// mktime(gmtime(t)): gmtime's tm_isdst is 0, so mktime reads the UTC fields as the time zone's standard time and the offset obtained
/// is that of standard time, without daylight saving. The smaller of the January and July offsets.
fn standard_offset_seconds() -> i32 {
    use chrono::{Datelike, Offset, TimeZone};
    let year = chrono::Local::now().year();
    let at = |month| chrono::Local.with_ymd_and_hms(year, month, 1, 12, 0, 0).single().map_or(0, |d| d.offset().fix().local_minus_utc());
    at(1).min(at(7))
}

/// Package loop of AStatLog::execInitialCheck: for "Name.u" then "Name.dll", the file size (−1 if missing), the string T = UPPERCASE
/// of the names seen so far (the ".u" one continues for ".dll") + the size, its MD5 in hexadecimal; (file, MD5) for each file present.
pub(crate) fn code_package_checksums(packages: &[String], size: &dyn Fn(&str) -> i32) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in packages {
        let mut upper = String::new();
        for ext in [".u", ".dll"] {
            let file = format!("{p}{ext}");
            upper.extend(file.chars().map(|c| if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c }));
            let n = size(&file);
            if n != -1 {
                out.push((file, hex(&Md5::digest(format!("{upper}{n}").as_bytes()))));
            }
        }
    }
    out
}

pub(crate) fn hex(d: &[u8; 16]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// MD5 (RFC 1321), like Core's appMD5Init / appMD5Update / appMD5Final.
#[derive(Clone)]
pub struct Md5 {
    state: [u32; 4],
    len: u64,
    buf: Vec<u8>,
}

impl Md5 {
    pub fn new() -> Self {
        Md5 { state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476], len: 0, buf: Vec::new() }
    }

    pub fn digest(data: &[u8]) -> [u8; 16] {
        let mut m = Md5::new();
        m.update(data);
        m.finish()
    }

    pub fn update(&mut self, data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        self.buf.extend_from_slice(data);
        let full = self.buf.len() / 64 * 64;
        let blocks: Vec<u8> = self.buf.drain(..full).collect();
        for b in blocks.chunks(64) {
            self.block(b);
        }
    }

    pub fn finish(mut self) -> [u8; 16] {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.buf.len() + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_le_bytes());
        let len = self.len;
        self.update(&pad);
        self.len = len;
        let mut out = [0u8; 16];
        for (i, w) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn block(&mut self, b: &[u8]) {
        const S: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).dsin().abs() * 4294967296.0) as u32).collect();
        let m: Vec<u32> = (0..16).map(|i| u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])).collect();
        let [mut a, mut bb, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((bb & c) | (!bb & d), i),
                1 => ((d & bb) | (!d & c), (5 * i + 1) % 16),
                2 => (bb ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (bb | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = bb;
            bb = bb.wrapping_add(a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(S[i]));
            a = t;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(bb);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }
}

impl Default for Md5 {
    fn default() -> Self {
        Md5::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_rfc1321() {
        assert_eq!(hex(&Md5::digest(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&Md5::digest(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        let long = b"12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(hex(&Md5::digest(long)), "57edf4a22be3c955ac49da2e2107b67a");
        let mut m = Md5::new();
        m.update(&long[..33]);
        m.update(&long[33..]);
        assert_eq!(hex(&m.finish()), "57edf4a22be3c955ac49da2e2107b67a");
    }
}

/// UObject::execGetEnum once the UEnum is found: name `i` of Names, otherwise NAME_None (empty) for an
/// object that is not a UEnum, a negative index or one outside the list.
#[cfg(test)]
mod log_path_tests {
    use super::log_path;
    use std::path::Path;

    /// The scripts' names stay in Logs; anything leaving it is refused.
    #[test]
    fn only_inside_logs() {
        let sys = Path::new("game/System");
        assert_eq!(log_path(sys, r"..\Logs\Game_12.tmp"), Some(Path::new("game/System/..").join("Logs").join("Game_12.tmp")));
        assert_eq!(log_path(sys, "../Logs/x y.log"), Some(Path::new("game/System/..").join("Logs").join("x y.log")));
        assert!(log_path(sys, r"..\Logs\..\..\evil.txt").is_none());
        assert!(log_path(sys, r"..\..\evil.txt").is_none());
        assert!(log_path(sys, r"C:\Windows\evil.txt").is_none());
        assert!(log_path(sys, r"\\server\share\x").is_none());
        assert!(log_path(sys, "/etc/evil").is_none());
        assert!(log_path(sys, "evil.txt").is_none());
        assert!(log_path(sys, r"..\Logs").is_none());
        assert!(log_path(sys, "").is_none());
    }
}

pub(crate) fn enum_get(names: Option<&[String]>, i: i32) -> String {
    names.filter(|_| i >= 0).and_then(|n| n.get(i as usize).cloned()).unwrap_or_default()
}

