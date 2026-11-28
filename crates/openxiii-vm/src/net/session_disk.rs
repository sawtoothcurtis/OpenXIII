//! The save device's files (profiles, their co-op and mods side files) as the game sees them.
//! Outside a lockstep session they are this machine's files on disk. In a session they are the
//! session's: the host's profile files, sent at the meeting, kept in memory and changed there
//! only, the same on every machine; nothing this machine's disk holds can enter the game. Each
//! write is also copied to disk, never read back during the session: on the host, into its own
//! profile (the session's files are its own); on a guest, into a separate copy of the session's
//! profile (the save folder's coop-sessions), never into the guest's own profile, whose solo
//! saves stay as they were.

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

/// Name prefix of the meeting's entries holding the host's save files.
const PREFIX: &str = "start/save_disk/";

/// A session's files: name (lower case, the save folder's) → content.
#[derive(Default, Clone, Debug)]
pub struct Disk {
    pub files: BTreeMap<String, Vec<u8>>,
    /// The session's profile (the host's): its files are written to this machine's own profile.
    pub host_profile: String,
    /// This machine's save folder and own profile, where the session's writes are copied.
    pub own: Option<(PathBuf, String)>,
    /// This machine is the session's host (its own profile is the session's).
    pub host: bool,
}

/// The save files carried by one lockstep session across its map changes.
#[derive(Default, Clone, Debug)]
pub struct SessionFiles {
    disk: std::sync::Arc<std::sync::Mutex<Option<Disk>>>,
}

impl SessionFiles {
    pub fn set(&self, disk: Option<Disk>) {
        if let Ok(mut current) = self.disk.lock() {
            *current = disk;
        }
    }

    /// This machine's own profile, where the session's writes are copied, when not known yet (a new
    /// VM's save device learns it after the session's files are in).
    pub fn set_own_if_missing(&self, dir: &Path, profile: &str) {
        if profile.is_empty() {
            return;
        }
        self.with(|d| {
            if d.own.is_none() {
                d.own = Some((dir.to_path_buf(), profile.to_string()));
            }
        });
    }

    /// A session's files are in use.
    pub fn active(&self) -> bool {
        self.with(|_| ()).is_some()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Disk) -> R) -> Option<R> {
        self.disk.lock().ok()?.as_mut().map(f)
    }
}

fn key(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Where the session's file `name` is copied on this machine's disk: the host's own profile's
/// file; a guest's copy of the session's profile in `coop-sessions` (never its own profile);
/// None: not copied (another profile's file).
fn mirror_path(d: &Disk, name: &str) -> Option<PathBuf> {
    let (dir, own) = d.own.as_ref()?;
    let host = format!("{}.pro", d.host_profile.to_lowercase());
    let rest = name.strip_prefix(&host)?;
    if d.host {
        return Some(dir.join(format!("{own}.pro{rest}")));
    }
    let copies = dir.join("coop-sessions");
    let _ = std::fs::create_dir_all(&copies);
    Some(copies.join(format!("{}.pro{rest}", d.host_profile)))
}

/// The session's file `name` as it is now, copied to disk (see `mirror_path`).
fn mirror(d: &Disk, name: &str) {
    if let (Some(path), Some(b)) = (mirror_path(d, name), d.files.get(name)) {
        let _ = std::fs::write(path, b);
    }
}

fn disk_write_at(path: &Path, offset: u64, data: &[u8], create: bool) -> bool {
    let f = std::fs::OpenOptions::new().read(true).write(true).create(create).truncate(false).open(path);
    let Ok(mut f) = f else { return false };
    f.seek(std::io::SeekFrom::Start(offset)).is_ok() && f.write_all(data).is_ok()
}

impl SessionFiles {
    /// The whole file (None: none).
    pub fn read(&self, path: &Path) -> Option<Vec<u8>> {
        match self.with(|d| d.files.get(&key(path)).cloned()) {
            Some(found) => found,
            None => std::fs::read(path).ok(),
        }
    }

    /// `len` bytes from `offset` (fewer at the end).
    pub fn read_at(&self, path: &Path, offset: u64, len: usize) -> Option<Vec<u8>> {
        if let Some(found) = self.with(|d| d.files.get(&key(path)).cloned()) {
            let b = found?;
            let from = (offset as usize).min(b.len());
            return Some(b[from..(from + len).min(b.len())].to_vec());
        }
        let mut f = std::fs::File::open(path).ok()?;
        f.seek(std::io::SeekFrom::Start(offset)).ok()?;
        let mut b = vec![0u8; len];
        let n = f.read(&mut b).ok()?;
        b.truncate(n);
        Some(b)
    }

    /// The file's size (0: none).
    pub fn len(&self, path: &Path) -> u64 {
        match self.with(|d| d.files.get(&key(path)).map(|b| b.len() as u64)) {
            Some(n) => n.unwrap_or(0),
            None => std::fs::metadata(path).map_or(0, |m| m.len()),
        }
    }

    /// The file made of `data` (replaced if it was there).
    pub fn write(&self, path: &Path, data: &[u8]) -> bool {
        let done = self.with(|d| {
            let name = key(path);
            d.files.insert(name.clone(), data.to_vec());
            mirror(d, &name);
        });
        match done {
            Some(()) => true,
            None => std::fs::write(path, data).is_ok(),
        }
    }

    /// `data` written at `offset` (the file grows as needed); without `create`, only into a file
    /// that is there.
    pub fn write_at(&self, path: &Path, offset: u64, data: &[u8], create: bool) -> bool {
        let done = self.with(|d| {
            let name = key(path);
            if !create && !d.files.contains_key(&name) {
                return false;
            }
            let b = d.files.entry(name.clone()).or_default();
            let end = offset as usize + data.len();
            if b.len() < end {
                b.resize(end, 0);
            }
            b[offset as usize..end].copy_from_slice(data);
            mirror(d, &name);
            true
        });
        match done {
            Some(ok) => ok,
            None => disk_write_at(path, offset, data, create),
        }
    }

    /// The file removed.
    pub fn remove(&self, path: &Path) {
        let done = self.with(|d| {
            let name = key(path);
            d.files.remove(&name);
            if let Some(own) = mirror_path(d, &name) {
                let _ = std::fs::remove_file(own);
            }
        });
        if done.is_none() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The names of the folder's files (as found on disk outside a session).
    pub fn names(&self, dir: &Path) -> Vec<String> {
        if let Some(n) = self.with(|d| d.files.keys().cloned().collect::<Vec<_>>()) {
            return n;
        }
        std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    }
}

/// The host's side of the meeting: its profile's files (profile and side files) from its save
/// folder, as `start/save_disk/<name>` entries, at most `cap` bytes in all.
pub fn entries(dir: &Path, profile: &str, cap: usize) -> Vec<(String, Vec<u8>)> {
    let prefix = format!("{}.pro", profile.to_lowercase());
    let mut out = Vec::new();
    let mut total = 0;
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| key(p).starts_with(&prefix)).collect();
    found.sort();
    for p in found {
        let Ok(b) = std::fs::read(&p) else { continue };
        if total + b.len() > cap {
            break;
        }
        total += b.len();
        out.push((format!("{PREFIX}{}", key(&p)), b));
    }
    out
}

/// The session's files from the host's entries (see [`entries`]).
pub fn from_entries(entries: &[(String, Vec<u8>)], host_profile: &str, own: Option<(PathBuf, String)>, host: bool) -> Disk {
    let files = entries.iter().filter_map(|(n, b)| Some((n.strip_prefix(PREFIX)?.to_lowercase(), b.clone()))).collect();
    Disk { files, host_profile: host_profile.to_string(), own, host }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In a session the save files are the host's, whatever this machine's disk holds: a slot
    /// read gives the host's bytes, a write changes the session's copy (read back by the game).
    /// On a guest, the writes go to a separate copy of the session's profile, never to the
    /// guest's own profile (its solo saves survive the session byte for byte), even when both
    /// profiles have the same name; on the host, to its own profile.
    #[test]
    fn session_saves_are_the_hosts() {
        let dir = std::env::temp_dir().join(format!("oxiii-session-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The guest's own profile has the host's name (both "XIII", as in the test packages).
        let solo: Vec<u8> = (0..64u8).collect();
        std::fs::write(dir.join("XIII.pro"), &solo).unwrap();
        std::fs::write(dir.join("Own.pro"), vec![1u8; 64]).unwrap();
        let host_files = vec![("start/save_disk/xiii.pro".to_string(), vec![9u8; 64]), ("start/save_disk/xiii.pro.coop-slot2.ini".to_string(), b"[x]".to_vec())];
        let session = SessionFiles::default();
        let disk = |host: bool, own: &str| from_entries(&host_files, "XIII", Some((dir.clone(), own.into())), host);
        let path = dir.join("XIII.pro");
        session.set(Some(disk(false, "XIII")));
        let guest = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_eq!(session.read_at(&path, 8, 4).unwrap(), vec![9u8; 4], "the host's bytes");
            assert_eq!(session.len(&path), 64);
            assert!(session.names(&dir).contains(&"xiii.pro.coop-slot2.ini".to_string()));
            assert!(session.write_at(&path, 60, &[5, 5, 5, 5, 5, 5], false));
            assert_eq!(session.read_at(&path, 60, 8).unwrap(), vec![5u8; 6], "the session's copy grew");
            assert!(!session.write_at(&dir.join("None.pro"), 0, &[1], false), "no file, no create");
            assert_eq!(std::fs::read(dir.join("XIII.pro")).unwrap(), solo, "the guest's own solo saves untouched");
            let copy = std::fs::read(dir.join("coop-sessions").join("XIII.pro")).unwrap();
            assert_eq!(copy.len(), 66);
            assert_eq!(copy[60..66], [5u8; 6], "the session's saves kept apart");
            assert_eq!(copy[..60], [9u8; 60]);
            session.remove(&dir.join("XIII.pro.coop-slot2.ini"));
            assert!(session.read(&dir.join("XIII.pro.coop-slot2.ini")).is_none());
        }));
        // The host: its own profile is the session's.
        session.set(Some(disk(true, "Own")));
        let wrote = session.write_at(&path, 0, &[7, 7], false);
        let own = std::fs::read(dir.join("Own.pro")).unwrap();
        session.set(None);
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = guest {
            std::panic::resume_unwind(e);
        }
        assert!(wrote);
        assert_eq!(own[..2], [7, 7], "the host's own profile gets the session's saves");
    }
}
