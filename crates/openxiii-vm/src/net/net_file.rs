//! Package download over the connection:
//! UFileChannel (ReceivedBunch, Tick, Destroy), UChannel::MaxSendBytes,
//! UChannelDownload::ReceiveFile, UDownload::ReceiveData and DownloadDone,
//! UNetConnection::ReceiveFile, UNetPendingLevel::ReceiveNextFile and
//! NotifyReceivedFile. The decisions of each function are in pure functions.
//!
//! Deliberate differences in OpenXIII: downloading is allowed
//! (shipped XIII.ini: AllowDownloads=False); the client only writes into the Cache folder, under the GUID's name, never
//! overwriting a file (the original replaces it); it refuses a file larger than SIZE_MAX or that is not the announced
//! package (signature and GUID). Not reimplemented: the F10 key (Skip/Cancel, TrySkipFile) and compressed
//! packages (never sent: no DownloadManagers with COMPRESSION).

use crate::net::{OutBunch, CHTYPE_FILE};
use crate::net_actor::Which;
use crate::net_map::guid_string;
use crate::netbits::BitReader;
use crate::Vm;
use std::io::{Read, Write};
use std::path::PathBuf;

/// Maximum size of a received package (the game's largest package, XIIIPersos, is 32.7 MiB).
pub const SIZE_MAX: u64 = 96 << 20;
/// PKG_Need (0x8000): package missing, to be received.
const PKG_NEED: u32 = 0x8000;
/// Signature of an Unreal package.
const PACKAGE_TAG: u32 = 0x9E2A_83C1;

/// Sending of a file by the server (UFileChannel opened by the client: SendFileAr +0x68, Filename +0x6C,
/// PackageIndex +0x16C, SentData +0x170).
#[derive(Debug)]
pub struct SendFile {
    pub ch: usize,
    pub package: usize,
    pub name_file: String,
    pub file_value: Option<std::fs::File>,
    pub sent_value: i32,
    /// OpenXIIIDownloadRate: bytes still allowed and driver time of the last credit.
    pub credit: f64,
    pub credit_time: Option<f64>,
}

/// Reception of a package by the client (UChannelDownload: Info +0x30, PackageIndex +0x2C, FileAr +0x44, TempFilename
/// +0x48, Error +0x148, Transfered +0x248).
#[derive(Debug)]
pub struct Download {
    pub ch: usize,
    pub package: usize,
    pub name_value: String,
    pub guid: [u8; 16],
    pub size_value: i32,
    pub temporary: Option<PathBuf>,
    pub file_value: Option<std::fs::File>,
    pub received: i32,
    pub error_value: String,
}

/// UChannel::MaxSendBytes: MaxPacket × 8, minus the packet header if Out is empty, minus Out and
/// MAX_BUNCH_HEADER_BITS + MAX_PACKET_TRAILER_BITS (0x41), divided by 8 (towards zero), at least 0.
pub(crate) fn max_send_bytes_of(max_packet: i32, out_bits: i32) -> i32 {
    let header = if out_bits == 0 { crate::net::MAX_PACKET_HEADER_BITS as i32 } else { 0 };
    let bits = max_packet.wrapping_mul(8).wrapping_sub(header).wrapping_sub(out_bits).wrapping_sub((crate::net::MAX_BUNCH_HEADER_BITS + crate::net::MAX_PACKET_TRAILER_BITS) as i32);
    (bits / 8).max(0)
}

/// UNetPendingLevel::ReceiveNextFile: first package of the map marked PKG_Need.
pub(crate) fn next_package(flags: &[u32]) -> Option<usize> {
    flags.iter().position(|f| f & PKG_NEED != 0)
}

/// What UFileChannel::ReceivedBunch decides on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestFile {
    /// "NetInvalid", closing bunch.
    Refusal,
    /// "SKIP" during sending: the current package is removed from the package map.
    Skips,
    /// Package `k` of the package map is sent.
    Send(usize),
}

/// UFileChannel::ReceivedBunch on the server: without AllowDownloads (driver +0x80), refusal; during a send,
/// only "SKIP" (FString, case-insensitive) is accepted; otherwise an FGuid, then each package with this GUID whose URL is not
/// empty: beyond MaxDownloadSize (if > 0), refusal; NotifySendingFile (accepted) and CreateFileReader (`opens`), otherwise the
/// next package. `packages_list`: (GUID, non-empty URL); `size_value`: GFileManager->FileSize of the URL.
pub(crate) fn request_file(
    allowed: bool,
    in_progress: bool,
    data: &mut BitReader,
    packages_list: &[([u8; 16], bool)],
    max_download_size: i32,
    mut size_value: impl FnMut(usize) -> i32,
    mut accepted: impl FnMut(usize) -> bool,
    mut opens: impl FnMut(usize) -> bool,
) -> RequestFile {
    if !allowed {
        return RequestFile::Refusal;
    }
    if in_progress {
        let text_value = crate::net::read_fstring(data);
        return if !data.error && text_value.eq_ignore_ascii_case("SKIP") { RequestFile::Skips } else { RequestFile::Refusal };
    }
    let bytes = data.read_bytes(16);
    if data.error {
        return RequestFile::Refusal;
    }
    let guid: [u8; 16] = bytes.try_into().unwrap();
    for (k, &(g, url)) in packages_list.iter().enumerate() {
        if g != guid || !url {
            continue;
        }
        if max_download_size > 0 && size_value(k) > max_download_size {
            return RequestFile::Refusal;
        }
        if accepted(k) && opens(k) {
            return RequestFile::Send(k);
        }
    }
    RequestFile::Refusal
}

/// One slice of UFileChannel::Tick: Info.FileSize − SentData remaining, bClose = MaxSendBytes ≥ remainder;
/// bytes sent = min(MaxSendBytes, remainder).
pub(crate) fn slice_value(size_value: i32, sent_value: i32, place: i32) -> (i32, bool) {
    let rest = size_value.wrapping_sub(sent_value);
    (if place > rest { rest } else { place }, place >= rest)
}

/// End of UDownload::DownloadDone, excluding skipped and compressed packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EndDownload {
    /// Error was already set: nothing is tried.
    Already,
    /// LocalizeError key ("NetRefused", "NetSize", "NetMove") or message of an OpenXIII check.
    Error(String),
    /// File moved into the cache.
    Received,
}

/// UDownload::DownloadDone: Transfered = 0 → "NetRefused"; temporary file size different from
/// Info.FileSize → "NetSize"; failed GFileManager->Move → "NetMove". `moves`: Ok, Err(None) for a failed Move,
/// Err(Some(message)) for an OpenXIII check (invalid package, file already there).
pub(crate) fn end_download(already: bool, received: i32, size_temp: i32, size_info: i32, moves: impl FnOnce() -> Result<(), Option<String>>) -> EndDownload {
    if already {
        return EndDownload::Already;
    }
    if received == 0 {
        return EndDownload::Error("NetRefused".into());
    }
    if size_temp != size_info {
        return EndDownload::Error("NetSize".into());
    }
    match moves() {
        Ok(()) => EndDownload::Received,
        Err(None) => EndDownload::Error("NetMove".into()),
        Err(Some(m)) => EndDownload::Error(m),
    }
}

/// UNetPendingLevel state as seen by NotifyReceivedFile: package map flags, FilesNeeded, Error and number of
/// download managers of the connection (+0x3EC8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceivedState {
    pub flags: Vec<u32>,
    pub files_needed: i32,
    pub error_value: String,
    pub handlers: usize,
}

/// UNetPendingLevel::NotifyReceivedFile. Error: with more than one manager, the first one is removed and the
/// next one tries; otherwise Error (if it is empty) = a failure ("DownloadFailed"). Success: PKG_Need removed, FilesNeeded
/// − 1, package removed from the map if it was skipped. True: ReceiveNextFile follows.
pub(crate) fn received_file(e: &mut ReceivedState, index: usize, error_value: &str, skips: bool, failure: impl FnOnce() -> String) -> bool {
    if !error_value.is_empty() {
        if e.handlers > 1 {
            e.handlers -= 1;
            return true;
        }
        if e.error_value.is_empty() {
            e.error_value = failure();
        }
        return false;
    }
    if let Some(f) = e.flags.get_mut(index) {
        *f &= !PKG_NEED;
    }
    e.files_needed -= 1;
    if skips && index < e.flags.len() {
        e.flags.remove(index);
    }
    true
}

/// Progress messages of UDownload::ReceiveData: "ReceiveFile" (or "ReceiveOptionalFile") with the
/// package name, and "ReceiveSize" with FileSize / 1024 (towards zero) and Transfered × 100.0f / FileSize (24-bit x87).
pub(crate) fn progress_reception(fmt_file: &str, fmt_size: &str, name_value: &str, size_value: i32, received: i32) -> (String, String) {
    let pct = if size_value != 0 {
        ((received as f64 * 100.0) as f32 as f64 / size_value as f64) as f32
    } else {
        let p = received as f64 * 100.0;
        if p == 0.0 { f32::NAN } else { (p / 0.0) as f32 }
    };
    let msg1 = crate::value::printf_s(fmt_file, &[name_value]);
    let msg2 = fmt_size.replacen("%i", &(size_value / 1024).to_string(), 1).replacen("%3.1f", &printf_3_1f(pct), 1).replace("%%", "%");
    (msg1, msg2)
}

/// "%3.1f" of _vsnprintf (msvcr70): one decimal, at least 3 characters.
fn printf_3_1f(f: f32) -> String {
    let s = if f.is_finite() { crate::value::printf_f_prec(f, 1) } else { crate::value::printf_f(f) };
    format!("{s:>3}")
}

impl Vm {
    /// Cache folder ([Core.System] CachePath=..\Cache, relative to System).
    pub(crate) fn folder_cache(&self) -> PathBuf {
        // OpenXIII: OXIII_CACHE replaces the Cache folder (never inside the game's files).
        if let Some(d) = openxiii_package::env::var_os("OXIII_CACHE") {
            return PathBuf::from(d);
        }
        let system = self.config.as_ref().map(|c| c.dir().to_path_buf()).unwrap_or_default();
        system.parent().map_or_else(|| system.join("Cache"), |g| g.join("Cache"))
    }

    /// Package already received: the "GUID=…" line of Cache\cache.ini and the file GUID.uxx.
    pub(crate) fn cache_finds(&self, guid: &[u8; 16]) -> Option<PathBuf> {
        let dir = self.folder_cache();
        let g = guid_string(guid);
        let ini = std::fs::read_to_string(dir.join("cache.ini")).ok()?;
        ini.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(&g))).then(|| dir.join(format!("{g}.uxx"))).filter(|p| p.is_file())
    }

    /// Package being received by the client: name, bytes received, announced size.
    pub fn net_download_state(&self) -> Option<(String, u64, u64)> {
        self.net_conn_ref(None).and_then(|c| c.download.as_ref()).map(|t| (t.name_value.clone(), t.received.max(0) as u64, t.size_value.max(0) as u64))
    }

    /// Localized engine error message (LocalizeError(key, "Engine")).
    fn error_engine(&mut self, key_tuple: &str) -> String {
        self.localize("Errors", key_tuple, "Engine", false)
    }

    // ---- Server ----

    /// UFileChannel::ReceivedBunch: on the client, UDownload::ReceiveData; on the server, request for a
    /// package by its GUID, or "SKIP" (request_file).
    pub(crate) fn net_file_bunch(&mut self, which: Which, ch: usize, data: &mut BitReader) {
        if which.is_none() {
            self.net_file_data(ch, data);
            return;
        }
        let allowed = self.ipdrv.net_driver.as_ref().is_some_and(|d| d.cfg.allow_downloads);
        let max = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.cfg.max_download_size);
        let in_progress = self.net_conn_ref(which).and_then(|c| c.sends.iter().position(|e| e.ch == ch));
        let (packages_list, urls): (Vec<([u8; 16], bool)>, Vec<String>) = self.net_conn_ref(which).map(|c| c.package_map.list.iter().map(|p| ((p.guid, !p.url.is_empty()), p.url.clone())).unzip()).unwrap_or_default();
        let mut unlocked = None;
        let decision = request_file(
            allowed,
            in_progress.is_some(),
            data,
            &packages_list,
            max,
            |k| std::fs::metadata(&urls[k]).map_or(-1, |m| m.len().min(i32::MAX as u64) as i32),
            // ULevel::NotifySendingFile: true on the server.
            |_| true,
            |k| {
                unlocked = std::fs::File::open(&urls[k]).ok();
                unlocked.is_some()
            },
        );
        match decision {
            RequestFile::Refusal => {
                let msg = self.error_engine("NetInvalid");
                self.log.push(format!("DevNet: {msg}"));
                if let Some(c) = self.net_conn(which) {
                    c.conn.close_channel(ch);
                }
            }
            RequestFile::Skips => {
                let k = in_progress.unwrap();
                let (name_value, package) = self.net_conn_ref(which).map(|c| (c.sends[k].name_file.clone(), c.sends[k].package)).unwrap();
                self.log.push(format!("Log: User skipped download of '{name_value}'"));
                if let Some(c) = self.net_conn(which) {
                    if package < c.package_map.list.len() {
                        c.package_map.list.remove(package);
                    }
                }
            }
            RequestFile::Send(package) => {
                // appStrncpy(Filename, URL, 256).
                let name_file: String = urls[package].chars().take(0xFF).collect();
                let msg = crate::value::printf_s(&self.localize("Progress", "NetSend", "Engine", false), &[&name_file]);
                self.log.push(format!("DevNet: {msg}"));
                if let Some(c) = self.net_conn(which) {
                    c.sends.push(SendFile { ch, package, name_file, file_value: unlocked, sent_value: 0, credit: 0.0, credit_time: None });
                }
            }
        }
    }

    /// UFileChannel::Tick: Connection->TimeSensitive = 1 for any file channel; on the server, as long
    /// as the channel is ready (UChannel::IsNetReady(ParseParam "lanplay")) and MaxSendBytes is not zero, a reliable
    /// slice of the file, then FlushNet; the last one closes the channel and the file.
    /// (The original does it in the channel loop of UNetConnection::Tick; here just before, for each connection.)
    pub(crate) fn net_files_tick(&mut self, which: Which) {
        let Some(c) = self.net_conn(which) else { return };
        if c.conn.channels.iter().flatten().any(|x| x.ch_type == CHTYPE_FILE) {
            c.conn.time_sensitive = true;
        }
        let chs: Vec<usize> = c.sends.iter().filter(|e| e.file_value.is_some()).map(|e| e.ch).collect();
        let saturated = crate::app_init::parse_param(&crate::config::app_cmd_line(), "lanplay");
        // OpenXIII key OpenXIIIDownloadRate: a credit of bytes per second replaces UNetConnection::IsNetReady (the channel
        // keeps its limit of 127 pending reliable bunches).
        let throughput = self.ipdrv.net_driver.as_ref().map_or(0, |d| d.cfg.download_rate);
        let now = self.ipdrv.net_driver.as_ref().map_or(0.0, |d| d.time);
        if throughput > 0 {
            if let Some(c) = self.net_conn(which) {
                for e in c.sends.iter_mut() {
                    let dt = e.credit_time.map_or(0.0, |t| (now - t).max(0.0));
                    e.credit = (e.credit + dt * throughput as f64).min(throughput as f64);
                    e.credit_time = Some(now);
                }
            }
        }
        for ch in chs {
            loop {
                let ready = if throughput > 0 {
                    self.net_conn_ref(which).is_some_and(|c| !c.conn.channel_out_full(ch) && c.sends.iter().any(|e| e.ch == ch && e.credit > 0.0))
                } else {
                    self.net_channel_ready_sat(which, ch, saturated)
                };
                if !ready {
                    break;
                }
                let Some(c) = self.net_conn(which) else { return };
                let place = c.conn.max_send_bytes() as i32;
                if place == 0 {
                    break;
                }
                let Some(k) = c.sends.iter().position(|e| e.ch == ch && e.file_value.is_some()) else { break };
                let size_value = c.package_map.list.get(c.sends[k].package).map_or(0, |p| p.file_size);
                let e = &mut c.sends[k];
                let (n, end_value) = slice_value(size_value, e.sent_value, place);
                let mut buffer_value = vec![0u8; n.max(0) as usize];
                if let Some(f) = e.file_value.as_mut() {
                    let _ = f.read_exact(&mut buffer_value);
                }
                e.sent_value = e.sent_value.wrapping_add(n);
                e.credit -= n as f64;
                if end_value {
                    e.file_value = None;
                }
                let mut bunch = OutBunch::new(&c.conn, ch, end_value);
                bunch.data.write_bytes(&buffer_value);
                bunch.b_reliable = true;
                c.conn.send_bunch(bunch, false);
                c.conn.flush_net();
                if end_value {
                    break;
                }
            }
        }
    }

    // ---- Client ----

    /// UNetPendingLevel::ReceiveNextFile: the first package marked PKG_Need is requested; otherwise the
    /// connection's download (+0x3EC4) is destroyed.
    pub(crate) fn net_receive_next(&mut self) {
        let flags: Vec<u32> = self.net_conn_ref(None).map(|c| c.package_map.list.iter().map(|p| p.package_flags).collect()).unwrap_or_default();
        match next_package(&flags) {
            Some(k) => self.net_receive_file(k),
            None => {
                if let Some(c) = self.net_conn(None) {
                    c.download = None;
                }
            }
        }
    }

    /// UNetConnection::ReceiveFile then UChannelDownload::ReceiveFile: file channel opened
    /// here, GUID sent in a reliable bunch.
    fn net_receive_file(&mut self, package: usize) {
        let Some(info) = self.net_conn_ref(None).and_then(|c| c.package_map.list.get(package).cloned()) else { return };
        let Some(c) = self.net_conn(None) else { return };
        let Some(ch) = c.conn.create_channel(CHTYPE_FILE, true, None) else {
            let e = self.error_engine("ChAllocate");
            self.net_received_file(package, &info.name, e);
            return;
        };
        let mut bunch = OutBunch::new(&c.conn, ch, false);
        bunch.data.write_bytes(&info.guid);
        bunch.b_reliable = true;
        c.conn.send_bunch(bunch, false);
        c.download = Some(Download {
            ch,
            package,
            name_value: info.name.clone(),
            guid: info.guid,
            size_value: info.file_size,
            temporary: None,
            file_value: None,
            received: 0,
            error_value: String::new(),
        });
    }

    /// UDownload::ReceiveData: on the first block, "Receiving package '%s'" and a temporary file in the
    /// cache (otherwise "NetOpen"); write (otherwise "NetWrite"); Transfered, then the progress messages.
    fn net_file_data(&mut self, ch: usize, data: &mut BitReader) {
        let n = data.num.saturating_sub(data.pos) / 8;
        let bytes_value = data.read_bytes(n);
        let dir = self.folder_cache();
        let Some(t) = self.net_conn(None).and_then(|c| c.download.as_mut()).filter(|t| t.ch == ch) else { return };
        // OpenXIII check: nothing more after an error.
        if !t.error_value.is_empty() {
            return;
        }
        if t.received == 0 && t.file_value.is_none() {
            let name_value = t.name_value.clone();
            self.log.push(format!("DevNet: Receiving package '{name_value}'"));
            let _ = std::fs::create_dir_all(&dir);
            let t = self.net_conn(None).and_then(|c| c.download.as_mut()).unwrap();
            // appCreateTempFilename: a free name in the cache.
            let temp = (0u32..).map(|k| dir.join(format!("Cache{k:04X}.tmp"))).find(|p| !p.exists()).unwrap();
            if let Ok(f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp) {
                t.file_value = Some(f);
                t.temporary = Some(temp);
            }
        }
        let t = self.net_conn(None).and_then(|c| c.download.as_mut()).unwrap();
        if t.file_value.is_none() {
            let e = self.error_engine("NetOpen");
            self.net_conn(None).and_then(|c| c.download.as_mut()).unwrap().error_value = e;
            return;
        }
        // OpenXIII check: nothing beyond the announced size or SIZE_MAX.
        if t.received as i64 + bytes_value.len() as i64 > (t.size_value.max(0) as u64).min(SIZE_MAX) as i64 {
            t.error_value = "size exceeded".into();
            return;
        }
        if t.file_value.as_mut().is_some_and(|f| f.write_all(&bytes_value).is_err()) {
            let temp = t.temporary.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            let e = crate::value::printf_s(&self.error_engine("NetWrite"), &[&temp]);
            self.net_conn(None).and_then(|c| c.download.as_mut()).unwrap().error_value = e;
            return;
        }
        t.received += bytes_value.len() as i32;
        // Notify->NotifyProgress (UNetPendingLevel → Engine->SetProgress), package flag 2: optional.
        let (name_value, received, size_value, package) = (t.name_value.clone(), t.received, t.size_value, t.package);
        let optional = self.net_conn_ref(None).and_then(|c| c.package_map.list.get(package)).is_some_and(|p| p.package_flags & 2 != 0);
        let key_tuple = if optional { "ReceiveOptionalFile" } else { "ReceiveFile" };
        let fmt1 = self.localize("Progress", key_tuple, "Engine", false);
        let fmt2 = self.localize("Progress", "ReceiveSize", "Engine", false);
        let (msg1, msg2) = progress_reception(&fmt1, &fmt2, &name_value, size_value, received);
        self.engine_set_progress(&msg1, &msg2, 4.0);
    }

    /// UFileChannel::Destroy: on the client, UDownload::DownloadDone (end_download);
    /// on the server, the sent file is closed.
    pub(crate) fn net_file_closes(&mut self, which: Which, ch: usize) {
        if which.is_some() {
            if let Some(c) = self.net_conn(which) {
                c.sends.retain(|e| e.ch != ch);
            }
            return;
        }
        let Some(mut t) = self.net_conn(None).and_then(|c| c.download.take()).filter(|t| t.ch == ch) else { return };
        t.file_value = None;
        let dir = self.folder_cache();
        let g = guid_string(&t.guid);
        let dest = dir.join(format!("{g}.uxx"));
        let size_temp = t.temporary.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(-1, |m| m.len().min(i32::MAX as u64) as i32);
        let end_value = end_download(!t.error_value.is_empty(), t.received, size_temp, t.size_value, || {
            // OpenXIII checks before GFileManager->Move: announced package, never overwrite.
            if !valid_package(t.temporary.as_ref(), &t.guid) {
                return Err(Some("invalid package".into()));
            }
            if dest.exists() {
                return Err(None);
            }
            t.temporary.as_ref().filter(|p| std::fs::rename(p, &dest).is_ok()).map(|_| ()).ok_or(None)
        });
        let error_value = match end_value {
            EndDownload::Already => std::mem::take(&mut t.error_value),
            EndDownload::Error(key_tuple) if key_tuple == "NetRefused" => crate::value::printf_s(&self.error_engine("NetRefused"), &[&t.name_value]),
            EndDownload::Error(key_tuple) if key_tuple.starts_with("Net") => self.error_engine(&key_tuple),
            EndDownload::Error(m) => m,
            EndDownload::Received => String::new(),
        };
        if !error_value.is_empty() {
            if let Some(p) = t.temporary.as_ref() {
                let _ = std::fs::remove_file(p);
            }
        } else {
            // Cache\cache.ini [Cache] GUID=value (Info.URL, the USES's FNAME, if it is not empty, otherwise the package name),
            // then the library knows the package.
            let ini = dir.join("cache.ini");
            let mut text_value = std::fs::read_to_string(&ini).unwrap_or_default();
            if !text_value.lines().any(|l| l.trim().eq_ignore_ascii_case("[Cache]")) {
                text_value.push_str("[Cache]\r\n");
            }
            let value_value = self.net_conn_ref(None).and_then(|c| c.package_map.list.get(t.package)).map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| t.name_value.clone());
            text_value.push_str(&format!("{g}={value_value}\r\n"));
            let _ = std::fs::write(&ini, text_value);
            if let Some(lib) = self.library.as_mut() {
                lib.set_file(&t.name_value, dest.clone());
            }
            // Notify->NotifyProgress("Success", "Received '%s'", 4.0).
            self.engine_set_progress("Success", &format!("Received '{}'", t.name_value), 4.0);
        }
        self.net_received_file(t.package, &t.name_value, error_value);
    }

    /// UNetPendingLevel::NotifyReceivedFile (received_file) with a single manager, Engine.ChannelDownload.
    fn net_received_file(&mut self, package: usize, name_value: &str, error_value: String) {
        let Some(p) = self.ipdrv.pending.as_ref() else { return };
        let flags: Vec<u32> = self.net_conn_ref(None).map(|c| c.package_map.list.iter().map(|p| p.package_flags).collect()).unwrap_or_default();
        let mut state_value = ReceivedState { flags, files_needed: p.files_needed, error_value: p.error.clone(), handlers: 1 };
        let fmt = self.error_engine("DownloadFailed");
        let next_one = received_file(&mut state_value, package, &error_value, false, || crate::value::printf_s(&fmt, &[name_value, &error_value]));
        if let Some(c) = self.net_conn(None) {
            for (p, f) in c.package_map.list.iter_mut().zip(&state_value.flags) {
                p.package_flags = *f;
            }
        }
        if let Some(p) = self.ipdrv.pending.as_mut() {
            p.files_needed = state_value.files_needed;
            p.error = state_value.error_value;
        }
        if next_one {
            self.net_receive_next();
        }
    }
}

/// OpenXIII check: the received file is an Unreal package whose GUID is the announced one.
fn valid_package(path_buf: Option<&PathBuf>, guid: &[u8; 16]) -> bool {
    let Some(data) = path_buf.and_then(|p| std::fs::read(p).ok()) else { return false };
    data.len() >= 4 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == PACKAGE_TAG && openxiii_package::Package::parse(&data).is_ok_and(|p| &p.header.guid == guid)
}

