//! Configuration and localization: .ini, .int and .frt files (FConfigCacheIni), UObject::
//! LoadConfig, UObject::LoadLocalized, Localize
//! and the properties' text import (UProperty::ImportText).
//!
//! A class's default values receive its configuration then its localized texts when it is
//! loaded (UClass::Serialize); a map's actors receive the texts of their section
//! in "Map.<language>" (UGameEngine::LoadLevelLocalizedTexts). The game is ANSI: the
//! files are read byte by byte (Latin-1), like the packages' strings.

use crate::types::{PropDef, Type};
use crate::value::{ClassId, ObjId, Value};
use crate::Vm;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

/// Folder where the player keeps their ini files (the launcher's play mode): FConfigCacheIni::Flush writes
/// the changed files there, and they are read from there before those of the System folder. Never set for
/// headless runs and tests: they read the game's ini files as they are and write nothing.
static FOLDER_PLAYER: OnceLock<PathBuf> = OnceLock::new();

/// The game's command line (program name included), read by appInit for INI= and USERINI= (`with_command_line`).
static LINE_COMMAND: OnceLock<String> = OnceLock::new();

/// Sets the game's command line, before loading.
pub fn set_line_command(cmd: String) {
    let _ = LINE_COMMAND.set(cmd);
}

/// appCmdLine: the command line without the program name (appInit, app_init::line_command).
pub fn app_cmd_line() -> String {
    crate::app_init::line_command(LINE_COMMAND.get().map_or("XIII.exe", |s| s.as_str())).to_string()
}

/// Sets the player's ini folder, before the game is loaded.
pub fn set_folder_player(dir: PathBuf) {
    let _ = FOLDER_PLAYER.set(dir);
}

/// The player's ini folder, when set.
pub fn folder_player() -> Option<PathBuf> {
    FOLDER_PLAYER.get().cloned()
}

// EClassFlags (16 bits in XIII's packages).
pub const CLASS_CONFIG: u32 = 0x4;
pub const CLASS_LOCALIZED: u32 = 0x20;
pub const CLASS_PER_OBJECT_CONFIG: u32 = 0x400;
// EPropertyFlags.
pub const CPF_NATIVE: u32 = 0x1000;
pub const CPF_TRANSIENT: u32 = 0x2000;
pub const CPF_CONFIG: u32 = 0x4000;
pub const CPF_LOCALIZED: u32 = 0x8000;
pub const CPF_GLOBAL_CONFIG: u32 = 0x40000;
// Import flags (EPropertyPortFlags).
const PPF_DELIMITED: u32 = 2;
const PPF_LOCALIZED_ONLY: u32 = 8;

/// File read like FConfigFile::Read: lines separated by CR / LF, "[Section]", then
/// "Key=Value" without stripping spaces; a value entirely in quotes loses them.
/// Sections and keys are case-insensitive; a repeated section continues the first one.
#[derive(Default, Debug, Clone)]
pub struct ConfigFile {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl ConfigFile {
    pub fn parse(text: &str) -> Self {
        let mut f = ConfigFile::default();
        let mut current: Option<usize> = None;
        for line in text.split(['\r', '\n']) {
            if line.is_empty() {
                continue;
            }
            if line.len() >= 2 && line.starts_with('[') && line.ends_with(']') {
                let name = &line[1..line.len() - 1];
                current = Some(match f.sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    Some(i) => i,
                    None => {
                        f.sections.push((name.to_string(), Vec::new()));
                        f.sections.len() - 1
                    }
                });
            } else if let Some(c) = current {
                if let Some(eq) = line.find('=') {
                    let mut value = &line[eq + 1..];
                    // FConfigFile::Read: first and last character '"' (the
                    // same one for a value made of a single quote, which becomes empty).
                    if value.starts_with('"') && value.ends_with('"') {
                        value = if value.len() >= 2 { &value[1..value.len() - 1] } else { "" };
                    }
                    f.sections[c].1.push((line[..eq].to_string(), value.to_string()));
                }
            }
        }
        f
    }

    pub(crate) fn section(&self, name: &str) -> Option<&Vec<(String, String)>> {
        self.sections.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, s)| s)
    }

    /// Text of the file: "[Section]\r\n", "Key=Value\r\n", "\r\n".
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (name, pairs) in &self.sections {
            out.push_str(&format!("[{name}]\r\n"));
            for (k, v) in pairs {
                out.push_str(&format!("{k}={v}\r\n"));
            }
            out.push_str("\r\n");
        }
        out
    }

    pub fn replace_key(&mut self, section: &str, key: &str, values: Vec<String>) {
        let i = match self.sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(section)) {
            Some(i) => i,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        let pairs = &mut self.sections[i].1;
        pairs.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        pairs.extend(values.into_iter().map(|v| (key.to_string(), v)));
    }

    pub fn replace_section(&mut self, section: &str, pairs: Vec<(String, String)>) {
        match self.sections.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(section)) {
            Some(s) => s.1 = pairs,
            None => self.sections.push((section.to_string(), pairs)),
        }
    }

    /// SetString: the value that Find returns (the key's last one), otherwise an added pair. A value
    /// equal to the old one ignoring case is not replaced (FConfigCacheIni::SetString: appStricmp): returns false in that case, the file has not changed.
    pub fn set(&mut self, section: &str, key: &str, value: &str) -> bool {
        let i = match self.sections.iter().position(|(n, _)| n.eq_ignore_ascii_case(section)) {
            Some(i) => i,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        let pairs = &mut self.sections[i].1;
        match pairs.iter_mut().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(p) if p.1.eq_ignore_ascii_case(value) => false,
            Some(p) => {
                p.1 = value.to_string();
                true
            }
            None => {
                pairs.push((key.to_string(), value.to_string()));
                true
            }
        }
    }
}

/// GConfig (FConfigCacheIni): files of the System folder read on demand.
#[derive(Debug, Clone)]
pub struct ConfigCache {
    dir: PathBuf,
    files: HashMap<String, Option<ConfigFile>>,
    /// The player's ini folder (`set_folder_player`), read first and the only one written.
    user_dir: Option<PathBuf>,
    /// FConfigFile::Dirty: files changed since the last Flush (key → file name).
    dirty: HashMap<String, String>,
    /// The game's ini ("System").
    pub ini: String,
    /// The player's ini ("User"): User.ini, which the engine creates from DefUser.ini.
    pub user_ini: String,
    /// UObject::GetLanguage: [Engine.Engine] Language of the game's ini ("int" otherwise).
    pub language: String,
    /// Nothing is written to disk (lockstep co-op: the guest plays with the host's User.ini).
    pub read_only: bool,
    /// The session's settings files are in the cache (`apply_session_ini`).
    session: bool,
}

/// Lockstep co-op: `file`'s sections that describe a machine ([`crate::lockstep::machine_local_section`])
/// become `own`'s (dropped when `own` has none).
fn keep_machine_sections(file: &mut ConfigFile, own: &ConfigFile) {
    let theirs: Vec<String> = file.sections.iter().map(|(n, _)| n.clone()).filter(|n| crate::lockstep::machine_local_section(n)).collect();
    for n in theirs {
        file.replace_section(&n, Vec::new());
    }
    for (n, pairs) in own.sections.iter().filter(|(n, _)| crate::lockstep::machine_local_section(n)) {
        file.replace_section(n, pairs.clone());
    }
}

impl ConfigCache {
    pub fn new(dir: PathBuf) -> Self {
        Self::with_user_dir(dir, FOLDER_PLAYER.get().cloned())
    }

    /// Like `new`, with the given player ini folder.
    pub fn with_user_dir(dir: PathBuf, user_dir: Option<PathBuf>) -> Self {
        let cmd = LINE_COMMAND.get().cloned().unwrap_or_else(|| "XIII.exe".to_string());
        Self::with_command_line(dir, user_dir, &cmd, true)
    }

    /// appInit (exact model tested in app_init.rs): the game's ini (INI=, otherwise XIII.ini) and the player's
    /// (USERINI=, otherwise User.ini) according to the command line `cmd` (program name included); a missing ini is created
    /// from Default.ini or DefUser.ini when `create` (the game: `with_user_dir`). The created file only exists in memory, marked changed: only `flush` writes it, and
    /// only into the player's folder (never into the game's folder).
    pub fn with_command_line(dir: PathBuf, user_dir: Option<PathBuf>, cmd: &str, create: bool) -> Self {
        Self::build(dir, user_dir, cmd, create, None)
    }

    pub(crate) fn for_session(dir: PathBuf, session: &crate::lockstep::SessionContext) -> Self {
        let cmd = LINE_COMMAND.get().map(String::as_str).unwrap_or("XIII.exe");
        Self::build(dir, FOLDER_PLAYER.get().cloned(), cmd, true, Some(session))
    }

    fn build(dir: PathBuf, user_dir: Option<PathBuf>, cmd: &str, create: bool, session: Option<&crate::lockstep::SessionContext>) -> Self {
        // The disk as appInit sees it: the .ini files of the System folder, the player's on top.
        let mut files_list: HashMap<String, Vec<u8>> = HashMap::new();
        for d in std::iter::once(&dir).chain(user_dir.iter()) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let name_value = e.file_name().to_string_lossy().to_lowercase();
                if name_value.ends_with(".ini") {
                    if let Ok(b) = std::fs::read(e.path()) {
                        files_list.insert(name_value, b);
                    }
                }
            }
        }
        let r = crate::app_init::app_init("XIII", cmd, create, &files_list, &|_, _| None);
        let mut c = ConfigCache { dir, files: HashMap::new(), user_dir, dirty: HashMap::new(), ini: r.ini, user_ini: r.user_ini, language: "int".into(), read_only: false, session: false };
        for (name_value, content) in r.created {
            for who in ["System", "User"] {
                let n = c.file_name(who);
                if crate::app_init::name_file(&n) == name_value {
                    c.files.insert(n.to_lowercase(), Some(ConfigFile::parse(&decode_file_text(&content))));
                    c.dirty.insert(n.to_lowercase(), n);
                }
            }
        }
        c.languages_installation();
        c.apply_session_ini(session.and_then(|s| s.ini.clone()));
        if let Some(l) = c.get_string("Engine.Engine", "Language", "System").filter(|l| !l.is_empty()) {
            c.language = l;
        }
        // Lockstep co-op: the session's language (the host's), in memory only.
        if let Some((text, sound)) = session.and_then(|s| s.language.clone()) {
            c.language = text.clone();
            for (who, section, key, value) in [("System", "Engine.Engine", "Language", text), ("User", "HXAudio.HXAudioSubsystem", "SoundLanguage", sound)] {
                c.file(who);
                let k = c.file_key(who);
                if let Some(Some(f)) = c.files.get_mut(&k) {
                    f.set(section, key, &value);
                }
            }
        }
        c
    }

    /// Language of the texts ([Engine.Engine] Language of the game's ini) and of the voices ([HXAudio.HXAudioSubsystem] SoundLanguage of
    /// the player's ini) taken from the game installation, never from the copies in the player's folder. The original has only one
    /// set of files, in its installation's System folder, which the installer writes in its language (GOG deletes XIII.ini on
    /// each language change): texts and voices there always follow the same installation. OpenXIII's player folder, on the other hand,
    /// serves every installation chosen in the launcher: a User.ini created with a French installation and an
    /// XIII.ini copied from an English installation gave French voices and English texts. Each value comes
    /// from the first file present in the installation, like appInit: XIII.ini otherwise Default.ini, User.ini otherwise DefUser.ini;
    /// it replaces the one in the player's copy in memory only (the file is not marked changed).
    fn languages_installation(&mut self) {
        let Some(player_value) = self.user_dir.clone() else { return };
        for (who, model, section, key_tuple) in [("System", "Default.ini", "Engine.Engine", "Language"), ("User", "DefUser.ini", "HXAudio.HXAudioSubsystem", "SoundLanguage")] {
            let name_value = self.file_name(who);
            if !player_value.join(&name_value).exists() {
                continue;
            }
            let Some(bytes_value) = [name_value.as_str(), model].iter().find_map(|n| std::fs::read(self.dir.join(n)).ok()) else { continue };
            let installation = ConfigFile::parse(&decode_file_text(&bytes_value));
            let Some(value_value) = installation.section(section).and_then(|s| s.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key_tuple))).map(|(_, v)| v.clone()) else { continue };
            self.file(who);
            let k = self.file_key(who);
            if let Some(Some(f)) = self.files.get_mut(&k) {
                f.set(section, key_tuple, &value_value);
            }
        }
    }

    /// Lockstep co-op: the session's settings files (`SessionContext::ini`) in place of this
    /// machine's, this machine's own sections kept; the cache then writes nothing to disk and never
    /// reads these two files again (an unloading Flush keeps them).
    fn apply_session_ini(&mut self, files: Option<(String, Option<String>)>) {
        let Some((user, system)) = files else { return };
        for (who, text) in [("User", Some(user)), ("System", system)] {
            let Some(text) = text else { continue };
            let mut session = ConfigFile::parse(&text);
            {
                let local = self.file(who).cloned().unwrap_or_default();
                keep_machine_sections(&mut session, &local);
            }
            let k = self.file_key(who);
            self.dirty.remove(&k);
            self.files.insert(k, Some(session));
        }
        self.read_only = true;
        self.session = true;
    }

    /// System folder, base of the files' relative paths.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// FConfigCacheIni::Find: "System" and "User" designate the two ini files; a name without
    /// an extension is an .ini.
    fn file(&mut self, name: &str) -> Option<&ConfigFile> {
        let name = self.file_name(name);
        let key = name.to_lowercase();
        if !self.files.contains_key(&key) {
            let player_value = self.user_dir.as_ref().map(|d| d.join(&name)).filter(|p| p.exists());
            let f = std::fs::read(player_value.unwrap_or_else(|| self.dir.join(&name))).ok().map(|b| ConfigFile::parse(&decode_file_text(&b)));
            self.files.insert(key.clone(), f);
        }
        self.files.get(&key).and_then(|f| f.as_ref())
    }

    fn file_key(&self, name: &str) -> String {
        self.file_name(name).to_lowercase()
    }

    /// Name of the file designated by `name` ("System", "User", name without extension).
    /// Any other name is reduced to a bare file name: it comes from a class's ConfigName or a
    /// script string, so from a package that may have been downloaded from a server or shipped
    /// by a mod, and must never read or write outside the System and player folders.
    fn file_name(&self, name: &str) -> String {
        if name.eq_ignore_ascii_case("System") {
            self.ini.clone()
        } else if name.eq_ignore_ascii_case("User") {
            self.user_ini.clone()
        } else {
            let base = name.rsplit(['/', '\\', ':']).next().unwrap_or_default();
            if base.contains('.') {
                base.to_string()
            } else {
                format!("{base}.ini")
            }
        }
    }

    fn mark_dirty(&mut self, name: &str) {
        let n = self.file_name(name);
        // DefUser.ini changed: the engine writes it under the name User.ini (UGameEngine::Init copies it).
        let n = if n.eq_ignore_ascii_case("DefUser.ini") { "User.ini".to_string() } else { n };
        self.dirty.insert(n.to_lowercase(), n);
    }

    /// FConfigCacheIni::Flush(0, NULL) (vtable +0x38, called by UObject::SaveConfig, and on exit): each changed file is written in full (FConfigFile::Write,
    /// "[Section]", "Key=Value", CRLF, ANSI bytes). Only into the player's folder.
    pub fn flush(&mut self) {
        self.flush_file(false, None);
    }

    /// FConfigCacheIni::Flush(Read, Filename): without a name, like `flush`; with a name, only the file with that name (case-insensitive comparison) is written if it has changed.
    /// `unload` (Read non-zero) then removes the named file from the cache, or all of them (Flush(1, NULL) of the destructor): the next access reads it again from disk,
    /// so the file just written if there was one. An unknown name does nothing.
    pub fn flush_file(&mut self, unload: bool, name: Option<&str>) {
        let target_value = name.map(|n| self.file_key(n));
        let concerns = |k: &String| target_value.as_ref().is_none_or(|c| k == c);
        if let Some(dir) = self.user_dir.clone().filter(|_| !self.read_only) {
            let a_write: Vec<(String, String)> = self.dirty.iter().filter(|(k, _)| concerns(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
            for (key, name_value) in a_write {
                self.dirty.remove(&key);
                let src = if key == "user.ini" && !self.files.contains_key("user.ini") { self.user_ini.to_lowercase() } else { key.clone() };
                let Some(Some(f)) = self.files.get(&src) else { continue };
                let bytes: Vec<u8> = f.to_text().chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join(&name_value), bytes);
            }
        } else {
            self.dirty.retain(|k, _| !concerns(k));
        }
        if unload {
            // A session never reads its settings again: what the disk holds is this machine's.
            let session = self.session;
            match &target_value {
                Some(_) if session => {}
                Some(c) => {
                    self.files.remove(c);
                }
                None if session => {}
                None => self.files.clear(),
            }
        }
    }

    /// Text of a loaded configuration file (empty if it does not exist).
    pub fn file_text(&mut self, name: &str) -> String {
        self.file(name).map(|f| f.to_text()).unwrap_or_default()
    }

    /// Content of a file replaced in memory (FConfigFile read again from a text);
    /// nothing is written to disk.
    pub fn replace_file(&mut self, name: &str, text: &str) {
        let key = self.file_key(name);
        let mut file = ConfigFile::parse(text);
        // Lockstep co-op: a profile's settings (the host's) replace the session's file, this
        // machine's own sections stay (its sound volume, window, devices).
        if self.session {
            let own = self.files.get(&key).cloned().flatten().unwrap_or_default();
            keep_machine_sections(&mut file, &own);
        }
        self.files.insert(key, Some(file));
    }

    /// UObject::ResetConfig: the section taken again from the default file (`default`), in memory.
    pub fn reset_section(&mut self, section: &str, file: &str, default: &str) {
        self.mark_dirty(file);
        let pairs = self.section_pairs(section, default);
        self.file(file);
        let k = self.file_key(file);
        if let Some(Some(f)) = self.files.get_mut(&k) {
            f.replace_section(section, pairs);
        }
    }

    /// Configuration array (SaveConfig): the key removed from the section, then one pair per
    /// element.
    pub fn replace_key(&mut self, section: &str, key: &str, values: Vec<String>, file: &str) {
        self.mark_dirty(file);
        self.file(file);
        let k = self.file_key(file);
        if !matches!(self.files.get(&k), Some(Some(_))) {
            self.files.insert(k.clone(), Some(ConfigFile::default()));
        }
        if let Some(Some(f)) = self.files.get_mut(&k) {
            f.replace_key(section, key, values);
        }
    }

    /// GConfig->SetString.
    pub fn set_string(&mut self, section: &str, key: &str, value: &str, file: &str) {
        self.file(file);
        let k = self.file_key(file);
        // The file is only marked changed if the value is (FConfigCacheIni::SetString: appStricmp, the case of an equal value stays that of the file).
        let changed = if let Some(Some(f)) = self.files.get_mut(&k) {
            f.set(section, key, value)
        } else {
            let mut f = ConfigFile::default();
            let c = f.set(section, key, value);
            self.files.insert(k, Some(f));
            c
        };
        if changed {
            self.mark_dirty(file);
        }
    }

    /// FConfigCacheIni::EmptySection (vtable +0x24): the file's section (case ignored) loses its elements; the file is only marked changed
    /// (FConfigFile::Dirty) if it had any. A missing file or section: nothing. (Called by IpDrv for "Engine.MatchMakingServers".)
    pub fn empty_section(&mut self, section: &str, file: &str) {
        self.file(file);
        let k = self.file_key(file);
        let emptied = match self.files.get_mut(&k) {
            Some(Some(f)) => match f.sections.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(section)) {
                Some(s) if !s.1.is_empty() => {
                    s.1.clear();
                    true
                }
                _ => false,
            },
            _ => false,
        };
        if emptied {
            self.mark_dirty(file);
        }
    }

    /// The file is marked changed since the last Flush (FConfigFile::Dirty, used by tests).
    #[cfg(test)]
    pub(crate) fn is_modified(&self, file: &str) -> bool {
        self.dirty.contains_key(&self.file_name(file).to_lowercase())
    }

    /// GetString: the value added last for this key (TMultiMap::Find).
    pub fn get_string(&mut self, section: &str, key: &str, file: &str) -> Option<String> {
        let s = self.file(file)?.section(section)?;
        s.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone())
    }

    /// Pairs of a section, in file order.
    pub fn section_pairs(&mut self, section: &str, file: &str) -> Vec<(String, String)> {
        self.file(file).and_then(|f| f.section(section)).cloned().unwrap_or_default()
    }

    /// Section present: all the values of the key, in file order (MultiFind, which
    /// LoadConfig walks backwards).
    pub fn multi_find(&mut self, section: &str, key: &str, file: &str) -> Option<Vec<String>> {
        let s = self.file(file)?.section(section)?;
        Some(s.iter().filter(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).collect())
    }

    /// Localize: GConfig->GetString(Section, Key, buffer of 0x400, "Package.language")
    /// (at most 1023 characters); missing and not `optional`: the trace "No localization: …" and
    /// "<?language?Package.Section.Key?>"; missing and `optional`: empty string.
    pub fn localize_text(&mut self, section: &str, key: &str, package: &str, lang: &str, optional: bool) -> (String, Option<String>) {
        let file = format!("{package}.{lang}");
        match self.get_string(section, key, &file) {
            Some(v) => (v.chars().take(0x3FF).collect(), None),
            None if optional => (String::new(), None),
            None => (format!("<?{lang}?{package}.{section}.{key}?>"), Some(format!("No localization: {package}.{section}.{key} ({lang})"))),
        }
    }

    /// Localize2: value from "Package.<language>", [Section] Key.
    pub fn localize2(&mut self, section: &str, key: &str, package: &str) -> Option<String> {
        let file = format!("{package}.{}", self.language);
        self.get_string(section, key, &file)
    }
}

fn at(text: &[char], i: usize) -> char {
    text.get(i).copied().unwrap_or('\0')
}

/// appAtoi (atoi): spaces, sign, digits.
pub(crate) fn atoi(text: &[char], mut p: usize) -> i32 {
    while matches!(at(text, p), ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') {
        p += 1;
    }
    let neg = at(text, p) == '-';
    if matches!(at(text, p), '-' | '+') {
        p += 1;
    }
    let mut v: i64 = 0;
    while at(text, p).is_ascii_digit() {
        v = (v * 10 + at(text, p) as i64 - '0' as i64) & 0xffff_ffff;
        p += 1;
    }
    let v = v as u32 as i32;
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

/// appAtof (atof): longest prefix that is a decimal number.
pub(crate) fn atof(text: &[char], mut p: usize) -> f32 {
    while matches!(at(text, p), ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c') {
        p += 1;
    }
    let start = p;
    if matches!(at(text, p), '-' | '+') {
        p += 1;
    }
    while at(text, p).is_ascii_digit() {
        p += 1;
    }
    if at(text, p) == '.' {
        p += 1;
        while at(text, p).is_ascii_digit() {
            p += 1;
        }
    }
    if matches!(at(text, p), 'e' | 'E') {
        let mut q = p + 1;
        if matches!(at(text, q), '-' | '+') {
            q += 1;
        }
        if at(text, q).is_ascii_digit() {
            while at(text, q).is_ascii_digit() {
                q += 1;
            }
            p = q;
        }
    }
    let s: String = text[start..p].iter().collect();
    s.parse::<f64>().map(|v| v as f32).unwrap_or(0.0)
}

fn from_hex(c: char) -> u32 {
    c.to_digit(16).unwrap_or(0)
}

/// ReadToken: string in quotes (escapes \\ and \XX),
/// identifier [A-Za-z0-9_-] (and "." if `dotted`), otherwise a single character (without advancing).
pub(crate) fn read_token(text: &[char], pos: usize, max: usize, dotted: bool, log: &mut Vec<String>) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut p = pos;
    let mut count = 0usize;
    let c = at(text, p);
    if c == '"' {
        p += 1;
        loop {
            let c = at(text, p);
            if c == '\0' || c == '"' || c == '\r' || c == '\n' || count + 1 >= max {
                break;
            }
            if c != '\\' {
                out.push(c);
                count += 1;
                p += 1;
                continue;
            }
            p += 1;
            if at(text, p) == '\\' {
                out.push('\\');
                p += 1;
            } else {
                out.push(char::from((from_hex(at(text, p)) * 16 + from_hex(at(text, p + 1))) as u8));
                p += 2;
            }
            count += 1;
        }
        if count + 1 == max {
            log.push("ReadToken: Quoted string too long".into());
            return None;
        }
        if at(text, p) != '"' {
            log.push("ReadToken: Bad quoted string".into());
            return None;
        }
        // The token is a C string: a NUL byte (or the end of the text) ends it.
        return Some((cstr(out), p + 1));
    }
    if c.is_ascii_alphanumeric() {
        loop {
            let c = at(text, p);
            let ok = c.is_ascii_alphanumeric() || c == '_' || c == '-' || (dotted && c == '.');
            if !ok || count + 1 >= max {
                break;
            }
            out.push(c);
            count += 1;
            p += 1;
        }
        if count + 1 == max {
            log.push("ReadToken: Alphanumeric overflow".into());
            return None;
        }
        return Some((out, p));
    }
    out.push(c);
    Some((cstr(out), p))
}

/// Text up to the first NUL (the original's buffer is a C string).
fn cstr(mut s: String) -> String {
    if let Some(i) = s.find('\0') {
        s.truncate(i);
    }
    s
}

impl Vm {
    /// "Package.Class" path of a class (GetPathName).
    pub(crate) fn class_path(&self, c: ClassId) -> String {
        // Mods build only: "Mod.<Name>" for a mod's new class (mod_classes.rs).
        #[cfg(feature = "mods")]
        if let Some(p) = self.mod_class_package(c) {
            return format!("{p}.{}", self.classes[c].name);
        }
        format!("{}.{}", self.packages[self.classes[c].package].name, self.classes[c].name)
    }

    /// Class that declares property number `i` of `class` (inherited properties first).
    pub(crate) fn prop_owner(&self, class: ClassId, i: usize) -> ClassId {
        let mut c = class;
        while let Some(s) = self.classes[c].super_class {
            if self.classes[s].props.len() > i {
                c = s;
            } else {
                break;
            }
        }
        c
    }

    /// Names of an enumeration (UEnum::Names) from its path "Package.[Class.]Enum".
    pub(crate) fn enum_names(&mut self, path: &str) -> Option<Rc<Vec<String>>> {
        let key = path.to_lowercase();
        if let Some(v) = self.enum_cache.get(&key) {
            return v.clone();
        }
        let found = (|| {
            let (package, rest) = path.split_once('.')?;
            let lp = self.packages.iter().find(|p| p.name.eq_ignore_ascii_case(package))?;
            let p = &lp.package;
            let export = (0..p.exports.len())
                .find(|&i| p.export_class(i) == "Enum" && p.object_path(openxiii_package::ObjectRef::Export(i)).eq_ignore_ascii_case(rest))?;
            let names = openxiii_script::objects::read_enum(p, &lp.data, export).ok()?;
            Some(Rc::new(names.iter().map(|&n| p.name(n).to_string()).collect::<Vec<_>>()))
        })();
        self.enum_cache.insert(key, found.clone());
        found
    }

    /// Localize: text from "Package.<language>"; missing, a trace and
    /// "<?language?Package.Section.Key?>" (an empty string if `optional`).
    pub fn localize(&mut self, section: &str, key: &str, package: &str, optional: bool) -> String {
        let Some(c) = self.config.as_mut() else {
            return key.to_string();
        };
        let lang = c.language.clone();
        let (text_value, trace) = c.localize_text(section, key, package, &lang, optional);
        if let Some(t) = trace {
            self.log.push(t);
        }
        text_value
    }

    /// UObject::LoadConfig on a class's values: keys of the section
    /// "Package.Class" (that of the class that declares a globalconfig property), or of the
    /// object's name for a PerObjectConfig object (`per_object` = name, package).
    pub(crate) fn load_config_values(&mut self, class: ClassId, values: &mut [Value], per_object: Option<(&str, &str)>) {
        self.load_config_class(class, class, values, per_object);
    }

    /// UObject::LoadConfig with Propagate = 1 (ResetConfig): the parent class first (if it carries CLASS_Config, otherwise the recursion
    /// stops there), then the class itself; the file is always that of the object's class (`class`).
    pub(crate) fn load_config_propagates(&mut self, class: ClassId, values: &mut [Value], per_object: Option<(&str, &str)>) {
        let mut string = Vec::new();
        let mut k = Some(class);
        while let Some(c) = k {
            if self.classes[c].flags & CLASS_CONFIG == 0 {
                break;
            }
            string.push(c);
            k = self.classes[c].super_class;
        }
        for &c in string.iter().rev() {
            self.load_config_class(c, class, values, per_object);
        }
    }

    fn load_config_class(&mut self, class: ClassId, class_object: ClassId, values: &mut [Value], per_object: Option<(&str, &str)>) {
        if self.classes[class].flags & CLASS_CONFIG == 0 || self.config.is_none() {
            return;
        }
        let file = match per_object {
            Some((_, package)) => package.to_string(),
            None => self.classes[class_object].config_name.clone(),
        };
        let props = self.classes[class].props.clone();
        for (i, p) in props.iter().enumerate() {
            if p.flags & CPF_CONFIG == 0 {
                continue;
            }
            // The values are those of the object's class: a property of a parent class is found there by its name.
            let si = if class == class_object {
                i
            } else {
                match self.classes[class_object].prop_index.get(&p.name.to_lowercase()) {
                    Some(&s) => s,
                    None => continue,
                }
            };
            if si >= values.len() {
                continue;
            }
            let section = match per_object {
                Some((name, _)) => name.to_string(),
                None => {
                    let base = if p.flags & CPF_GLOBAL_CONFIG != 0 { self.prop_owner(class, i) } else { class };
                    self.class_path(base)
                }
            };
            if let Type::Array(inner) = &p.ty {
                // Section present: the array is rebuilt with all the values of the key.
                let Some(list) = self.config.as_mut().and_then(|c| c.multi_find(&section, &p.name, &file)) else {
                    continue;
                };
                let mut items = Vec::with_capacity(list.len());
                for text in &list {
                    let mut v = self.zero(inner, 1);
                    let chars: Vec<char> = text.chars().collect();
                    self.import_text(inner, p.enumeration.as_deref(), &chars, 0, 0, &mut v);
                    items.push(v);
                }
                values[si] = Value::Array(items);
                continue;
            }
            for k in 0..p.dim {
                let key = if p.dim == 1 { p.name.clone() } else { format!("{}[{k}]", p.name) };
                let Some(text) = self.config.as_mut().and_then(|c| c.get_string(&section, &key, &file)) else {
                    continue;
                };
                let chars: Vec<char> = text.chars().collect();
                let slot = element(&mut values[si], p.dim, k);
                if let Some(slot) = slot {
                    self.import_text(&p.ty, p.enumeration.as_deref(), &chars, 0, 0, slot);
                }
            }
        }
    }

    /// UObject::LoadLocalized: localized properties from [`section`] of
    /// "`package`.<language>" (key "Name" or "Name[i]"), imported without rebuilding the arrays.
    pub(crate) fn load_localized_values(&mut self, class: ClassId, values: &mut [Value], section: &str, package: &str) {
        if self.classes[class].flags & CLASS_LOCALIZED == 0 || self.config.is_none() {
            return;
        }
        let props = self.classes[class].props.clone();
        for (i, p) in props.iter().enumerate() {
            if p.flags & CPF_LOCALIZED == 0 || i >= values.len() {
                continue;
            }
            for k in 0..p.dim {
                let key = if p.dim == 1 { p.name.clone() } else { format!("{}[{k}]", p.name) };
                let Some(text) = self.config.as_mut().and_then(|c| c.localize2(section, &key, package)) else {
                    continue;
                };
                let chars: Vec<char> = text.chars().collect();
                if let Some(slot) = element(&mut values[i], p.dim, k) {
                    self.import_text(&p.ty, p.enumeration.as_deref(), &chars, 0, PPF_LOCALIZED_ONLY, slot);
                }
            }
        }
    }

    /// UGameEngine::LoadLevelLocalizedTexts: texts of each actor
    /// of the map, section = its name, file = the map.
    pub(crate) fn load_level_localized(&mut self, actors: &[ObjId], map: &str) {
        // No effect without a level or without an Outer.
        // The already loaded map is represented by level; an empty map represents the missing Outer.
        if self.level.is_none() || map.is_empty() {
            return;
        }
        let classes: Vec<Option<u32>> = actors.iter().map(|&a| self.objects[a].as_ref().map(|o| self.classes[o.class].flags)).collect();
        for i in localized_actors(&classes) {
            let a = actors[i];
            let Some((class, name)) = self.objects[a].as_ref().map(|o| (o.class, o.name.clone())) else {
                continue;
            };
            let mut values = std::mem::take(&mut self.objects[a].as_mut().unwrap().props);
            values.with_slice(|v| self.load_localized_values(class, v, &name, map));
            if let Some(o) = self.objects[a].as_mut() {
                o.props = values;
            }
        }
        // All the LevelSummary objects, including subclasses and other maps.
        let summaries: Vec<(ObjId, String)> = self.objects.iter().enumerate().filter_map(|(id, object)| {
            let object = object.as_ref()?;
            self.is_a(id, "LevelSummary").then(|| (id, object.outer.rsplit('.').next().unwrap_or_default().to_string()))
        }).collect();
        for (summary, package) in summaries {
            let title = if let Some(cache) = self.config.as_mut() {
                let language = cache.language.clone();
                cache.localize_text("LevelInfo0", "Title", &package, &language, true).0
            } else {
                String::new()
            };
            // Optional=1 and empty string: the old title stays intact.
            if !title.is_empty() {
                self.set_prop(summary, "Title", Value::Str(title));
            }
        }
    }

    /// GTrue / GFalse: localized "True" and "False" (Core.<language>, [General]).
    pub(crate) fn bool_words(&mut self) -> (String, String) {
        let c = self.config.as_mut();
        match c {
            Some(c) => (
                c.localize2("General", "True", "Core").unwrap_or_else(|| "True".into()),
                c.localize2("General", "False", "Core").unwrap_or_else(|| "False".into()),
            ),
            None => ("True".into(), "False".into()),
        }
    }

    /// UProperty::ImportText: reads a value of type `ty` at `text[pos..]` into `slot`; the position
    /// after the value, or None on failure (the value may stay half filled).
    pub(crate) fn import_text(&mut self, ty: &Type, enumeration: Option<&str>, text: &[char], pos: usize, flags: u32, slot: &mut Value) -> Option<usize> {
        self.import_text_named("", ty, enumeration, text, pos, flags, slot)
    }

    /// ImportText of a property named `prop` (the name that error messages give:
    /// "Unknown struct item … in property <name>").
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn import_text_named(&mut self, prop: &str, ty: &Type, enumeration: Option<&str>, text: &[char], pos: usize, flags: u32, slot: &mut Value) -> Option<usize> {
        let mut log = Vec::new();
        let r = self.import_inner(prop, ty, enumeration, text, pos, flags, slot, &mut log);
        self.log.extend(log);
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn import_inner(
        &mut self,
        prop: &str,
        ty: &Type,
        enumeration: Option<&str>,
        text: &[char],
        pos: usize,
        flags: u32,
        slot: &mut Value,
        log: &mut Vec<String>,
    ) -> Option<usize> {
        match ty {
            // UIntProperty::ImportText: without a digit, nothing is read but it is not
            // a failure.
            Type::Int => {
                let c = at(text, pos);
                if c == '-' || c.is_ascii_digit() {
                    *slot = Value::Int(atoi(text, pos));
                }
                let mut p = pos;
                while at(text, p) == '-' || at(text, p).is_ascii_digit() {
                    p += 1;
                }
                Some(p)
            }
            // UFloatProperty::ImportText.
            Type::Float => {
                *slot = Value::Float(atof(text, pos));
                let mut p = pos;
                while !matches!(at(text, p), '\0' | ',' | ')' | '\r' | '\n') {
                    p += 1;
                }
                Some(p)
            }
            // UBoolProperty::ImportText.
            Type::Bool => {
                let (tok, p) = read_token(text, pos, 0x400, false, log)?;
                let (t, f) = self.bool_words();
                if tok.eq_ignore_ascii_case("1") || tok.eq_ignore_ascii_case("True") || tok.eq_ignore_ascii_case(&t) {
                    *slot = Value::Bool(true);
                } else if tok.eq_ignore_ascii_case("0") || tok.eq_ignore_ascii_case("False") || tok.eq_ignore_ascii_case(&f) {
                    *slot = Value::Bool(false);
                } else {
                    return None;
                }
                Some(p)
            }
            // UByteProperty::ImportText: name of the enumeration value, otherwise digits
            // (after the token read).
            Type::Byte => {
                let mut p = pos;
                if let Some(names) = enumeration.and_then(|e| self.enum_names(e)) {
                    let (tok, after) = read_token(text, pos, 0x400, false, log)?;
                    p = after;
                    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(&tok)) {
                        *slot = Value::Byte(i as u8);
                        return Some(p);
                    }
                }
                if !at(text, p).is_ascii_digit() {
                    return None;
                }
                *slot = Value::Byte(atoi(text, p) as u8);
                while at(text, p).is_ascii_digit() {
                    p += 1;
                }
                Some(p)
            }
            // UNameProperty::ImportText.
            Type::Name => {
                let (tok, p) = read_token(text, pos, 0x400, false, log)?;
                // FName(tok, FNAME_Add): a name already in the table keeps its spelling
                *slot = Value::Name(crate::value::NameStruct::from(self.canonical_name(&tok).unwrap_or(tok)));
                Some(p)
            }
            // UStrProperty::ImportText: outside a struct or an array, the whole
            // rest of the text (without advancing); inside, a token.
            Type::Str => {
                if flags & PPF_DELIMITED == 0 {
                    *slot = Value::Str(text[pos.min(text.len())..].iter().collect());
                    return Some(pos);
                }
                let (tok, p) = read_token(text, pos, 0x1000, false, log)?;
                *slot = Value::Str(tok);
                Some(p)
            }
            // UObjectProperty / UClassProperty::ImportText.
            Type::Object(meta) => self.import_object_ref(meta, text, pos, flags, slot, log),
            Type::Class(_) => {
                let (tok, p) = read_token(text, pos, 0x400, true, log)?;
                let is_class = matches!(ty, Type::Class(_));
                if tok.eq_ignore_ascii_case("None") {
                    *slot = if is_class { Value::Class(None) } else { Value::Object(None) };
                    return Some(p);
                }
                let mut q = p;
                while at(text, q) == ' ' {
                    q += 1;
                }
                if at(text, q) != '\'' {
                    let v = self.find_object_value(ty, None, &tok);
                    // UClassProperty: a class found outside the metaclass gives None but keeps the position.
                    let found = if is_class { self.class(&tok).is_some() } else { !matches!(v, Value::Object(None)) };
                    *slot = v;
                    // The engine returns the position one character after the end of the name.
                    return found.then_some(q + 1);
                }
                let (path, r) = read_token(text, q + 1, 0x400, true, log)?;
                if at(text, r) != '\'' {
                    return None;
                }
                let v = self.find_object_value(ty, Some(&tok), &path);
                let found = if is_class { self.class(&path).is_some() } else { !matches!(v, Value::Object(None)) };
                *slot = v;
                found.then_some(r + 1)
            }
            Type::Struct(name) => self.import_struct(prop, name, text, pos, flags, slot, log),
            // UArrayProperty::ImportText: in "localized only" mode, the
            // existing elements are rewritten without rebuilding the array.
            Type::Array(inner) => {
                if at(text, pos) != '(' {
                    return None;
                }
                if !matches!(slot, Value::Array(_)) {
                    *slot = Value::Array(Vec::new());
                }
                if flags & PPF_LOCALIZED_ONLY == 0 {
                    if let Value::Array(items) = slot {
                        items.clear();
                    }
                }
                let mut index: isize = -1;
                let mut p = pos;
                loop {
                    p += 1;
                    if at(text, p) == ')' {
                        break;
                    }
                    let zero = self.zero(inner, 1);
                    let Value::Array(items) = slot else { return None };
                    if flags & PPF_LOCALIZED_ONLY == 0 {
                        items.push(zero);
                        index = items.len() as isize - 1;
                    } else {
                        index += 1;
                        while items.len() <= index as usize {
                            items.push(zero.clone());
                        }
                    }
                    let mut item = std::mem::replace(&mut items[index as usize], Value::Void);
                    let r = self.import_inner(prop, inner, enumeration, text, p, flags | PPF_DELIMITED, &mut item, log);
                    if let Value::Array(items) = slot {
                        items[index as usize] = item;
                    }
                    p = r?;
                    if at(text, p) != ',' {
                        break;
                    }
                }
                (at(text, p) == ')').then_some(p + 1)
            }
            Type::Delegate | Type::Unknown => None,
        }
    }

    /// UStructProperty::ImportText: "(Field=Value,Field[i]=Value…)".
    fn import_struct(&mut self, prop: &str, name: &str, text: &[char], pos: usize, flags: u32, slot: &mut Value, log: &mut Vec<String>) -> Option<usize> {
        if at(text, pos) != '(' {
            log.push("ImportText: Struct missing '('".into());
            return None;
        }
        let fields: Vec<PropDef> = self.struct_def(name).map(|s| s.fields.clone()).unwrap_or_default();
        if !slot.is_struct() {
            *slot = self.zero(&Type::Struct(name.to_string()), 1);
        }
        // Inline vector / rotator: unpacked to be written field by field (same value).
        if let Some(s) = slot.unfold() {
            *slot = s;
        }
        let mut p = pos + 1;
        loop {
            if at(text, p) == ')' {
                return Some(p + 1);
            }
            let start = p;
            while p - start < 0x42 && !matches!(at(text, p), '\0' | '=' | '[') {
                p += 1;
            }
            let field: String = text[start..p].iter().collect();
            let mut index = 0usize;
            if at(text, p) == '[' {
                let q = p + 1;
                let mut r = q;
                while at(text, r).is_ascii_digit() {
                    r += 1;
                }
                if at(text, r) != ']' {
                    log.push("ImportText: Illegal array element".into());
                    return None;
                }
                index = atoi(text, q).max(0) as usize;
                p = r + 1;
            }
            if at(text, p) != '=' {
                log.push("ImportText: Illegal or missing key name".into());
                return None;
            }
            p += 1;
            // UProperty::Port: neither transient nor native (without a category), and not "Class".
            let found = fields.iter().position(|f| {
                f.name.eq_ignore_ascii_case(&field) && index < f.dim && !f.name.eq_ignore_ascii_case("Class") && f.flags & (CPF_TRANSIENT | CPF_NATIVE) == 0
            });
            match found {
                Some(fi) => {
                    let f = fields[fi].clone();
                    let Value::Struct(_, values) = slot else { return None };
                    let target = element(&mut values[fi], f.dim, index)?;
                    let mut v = std::mem::replace(target, Value::Void);
                    let r = self.import_inner(&f.name, &f.ty, f.enumeration.as_deref(), text, p, flags | PPF_DELIMITED, &mut v, log);
                    if let Value::Struct(_, values) = slot {
                        if let Some(target) = element(&mut values[fi], f.dim, index) {
                            *target = v;
                        }
                    }
                    p = r?;
                }
                None => {
                    log.push(format!("ImportText: Unknown struct item {field} in property {prop}"));
                    // Skipped value: up to the end of the line, or "," / ")" outside parentheses.
                    let mut depth = 0i32;
                    loop {
                        let c = at(text, p);
                        if c == '\0' || c == '\n' || c == '\r' || (depth <= 0 && (c == ')' || c == ',')) {
                            break;
                        }
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth < 0 {
                                log.push("ImportText: Bad parenthesised struct".into());
                                return None;
                            }
                        }
                        p += 1;
                    }
                    if depth > 0 {
                        log.push("ImportText: Incomplete parenthesised struct".into());
                        return None;
                    }
                }
            }
            if at(text, p) == ',' {
                p += 1;
                continue;
            }
            if at(text, p) != ')' {
                log.push("ImportText: Bad termination".into());
                return None;
            }
        }
    }

    /// UObjectProperty::ImportText: "None", a name (StaticFindObject filtered by the property's class) or
    /// "Class'Path'" (class looked up by its name, object filtered by that class; PPF 4: warnings for an
    /// unresolved reference or cast, or a class that does not fit). The object is written even if it is null, except
    /// when the text's class is unknown or the text is malformed.
    fn import_object_ref(&mut self, meta: &str, text: &[char], pos: usize, flags: u32, slot: &mut Value, log: &mut Vec<String>) -> Option<usize> {
        let (tok, p) = read_token(text, pos, 0x400, true, log)?;
        if tok.eq_ignore_ascii_case("None") {
            *slot = Value::Object(None);
            return Some(p);
        }
        let filter_value = self.class(meta);
        let mut q = p;
        while at(text, q) == ' ' {
            q += 1;
        }
        if at(text, q) != '\'' {
            let v = self.finds_object(filter_value, &tok);
            *slot = v.clone();
            if !matches!(v, Value::Object(None)) {
                return Some(q + 1);
            }
            if flags & 4 != 0 {
                log.push(format!("{meta}: unresolved reference to '{tok}'"));
            }
            return None;
        }
        let (path, r) = read_token(text, q + 1, 0x400, true, log)?;
        if at(text, r) != '\'' {
            return None;
        }
        let Some(class_value) = self.class(&tok) else {
            // StaticFindObject(UClass, ANY_PACKAGE, class) also finds the native classes without a script class
            // in the VM (Sound, Texture, StaticMesh…): their objects are resources (`Value::Asset`), never VM
            // objects; StaticFindObject therefore finds none and StaticLoadObject(class, NULL, path, NULL,
            // LOAD_NoWarn, NULL) loads them from their package.
            if let Some(name_value) = crate::natives::class_native_known(self, &tok) {
                return self.import_resource(meta, &name_value, &path, r, flags, slot, log);
            }
            if flags & 4 != 0 {
                log.push(format!("{meta}: unresolved cast in '{tok}'"));
            }
            return None;
        };
        let v = self.finds_object(Some(class_value), &path);
        // No VM object under this name: StaticLoadObject looks for it in its package, as for a native class.
        if matches!(v, Value::Asset(_)) {
            let name_value = self.classes[class_value].name.clone();
            return self.import_resource(meta, &name_value, &path, r, flags, slot, log);
        }
        *slot = v.clone();
        if let Value::Object(Some(o)) = &v {
            if flags & 4 != 0 {
                let c = self.objects[*o].as_ref().map(|x| x.class);
                if let (Some(c), Some(f)) = (c, filter_value) {
                    if !self.is_child_of(c, f) {
                        log.push(format!("{meta}: bad cast in '{path}'"));
                    }
                }
            }
        }
        if !matches!(v, Value::Object(None)) {
            return Some(r + 1);
        }
        if flags & 4 != 0 {
            log.push(format!("{meta}: unresolved reference to '{path}'"));
        }
        None
    }

    /// End of UObjectProperty::ImportText for "Class'Path'" of a native class without objects in
    /// the VM: StaticLoadObject writes its result into the property; if loaded, PPF 4 checks that it is of the
    /// property's class (IsA, "bad cast") and reading continues after the quote; otherwise PPF 4 warns of an
    /// unresolved reference and ImportText returns NULL. StaticLoadObject's lines (LOAD_NoWarn) go to the log.
    #[allow(clippy::too_many_arguments)]
    fn import_resource(&mut self, meta: &str, class_value: &str, path: &str, r: usize, flags: u32, slot: &mut Value, log: &mut Vec<String>) -> Option<usize> {
        let (load, lines_list) = crate::natives::load_resource(self, class_value, path);
        log.extend(lines_list);
        match load {
            Some(c) => {
                *slot = Value::Asset(path.to_string());
                if flags & 4 != 0 && !crate::natives::class_is(self, &c, meta) {
                    log.push(format!("{meta}: bad cast in '{path}'"));
                }
                Some(r + 1)
            }
            None => {
                *slot = Value::Object(None);
                if flags & 4 != 0 {
                    log.push(format!("{meta}: unresolved reference to '{path}'"));
                }
                None
            }
        }
    }

    /// StaticFindObject(Class, ANY_PACKAGE, path) on VM objects: the most recent one with this name whose class fits
    /// and whose full path is the one given (a dotted path); a name that only another class has gives None;
    /// a name unknown to the VM is a resource (texture, sound…) returned as `Value::Asset`.
    fn finds_object(&mut self, filter_value: Option<crate::value::ClassId>, path_buf: &str) -> Value {
        let last_one = path_buf.rsplit('.').next().unwrap_or(path_buf);
        let mut name_present = false;
        for o in (0..self.objects.len()).rev() {
            let Some(ob) = self.objects[o].as_ref() else { continue };
            if !ob.name.eq_ignore_ascii_case(last_one) || ob.name.starts_with("Default__") {
                continue;
            }
            if path_buf.contains('.') && !self.path_name(o).eq_ignore_ascii_case(path_buf) {
                continue;
            }
            name_present = true;
            let class_value = ob.class;
            if filter_value.is_none_or(|f| self.is_child_of(class_value, f)) {
                return Value::Object(Some(o));
            }
        }
        if name_present {
            Value::Object(None)
        } else {
            Value::Asset(path_buf.to_string())
        }
    }

    /// StaticFindObject(Class, ANY_PACKAGE, path) for an ini reference: a class, an
    /// actor or script object by its name, otherwise a resource (texture, sound…).
    fn find_object_value(&mut self, ty: &Type, class: Option<&str>, path: &str) -> Value {
        let wants_class = matches!(ty, Type::Class(_)) || class.is_some_and(|c| c.eq_ignore_ascii_case("Class"));
        if wants_class {
            return match self.class(path) {
                Some(c) => {
                    // UClassProperty: the class must descend from the metaclass.
                    let ok = match ty {
                        Type::Class(meta) => self.class(meta).is_none_or(|m| self.is_child_of(c, m)),
                        _ => true,
                    };
                    if matches!(ty, Type::Class(_)) {
                        Value::Class(ok.then_some(c))
                    } else {
                        Value::Class(Some(c))
                    }
                }
                None => {
                    if matches!(ty, Type::Class(_)) {
                        Value::Class(None)
                    } else {
                        Value::Object(None)
                    }
                }
            };
        }
        let last = path.rsplit('.').next().unwrap_or(path);
        // StaticFindObject returns the most recent of the objects with this name (HashObject puts it at the head of the chain).
        if let Some(o) = self.objects.iter().rposition(|o| o.as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(last) && !o.name.starts_with("Default__"))) {
            return Value::Object(Some(o));
        }
        Value::Asset(path.to_string())
    }
}

/// appLoadFileToString: a UTF-16 file (FF FE little-endian or FE FF big-endian mark, even length)
/// is converted character by character (above 0xFF: 0x7F, appToAnsi without a range table); otherwise the bytes are kept as
/// they are. The ini readers stop at the first null byte: the text ends there.
pub(crate) fn decode_file_text(b: &[u8]) -> String {
    let mut out = String::new();
    let utf16 = b.len() > 1 && b.len() % 2 == 0 && ((b[0] == 0xFF && b[1] == 0xFE) || (b[0] == 0xFE && b[1] == 0xFF));
    if utf16 {
        let small = b[0] == 0xFF;
        for k in 1..b.len() / 2 {
            let (lo, hi) = if small { (b[2 * k], b[2 * k + 1]) } else { (b[2 * k + 1], b[2 * k]) };
            let unit = lo as u32 + hi as u32 * 256;
            let c = if unit > 0xFF { 0x7F } else { unit as u8 };
            if c == 0 {
                break;
            }
            out.push(c as char);
        }
    } else {
        for &c in b {
            if c == 0 {
                break;
            }
            out.push(c as char);
        }
    }
    out
}

/// Element `k` of a property (static array if `dim` > 1).
fn element(v: &mut Value, dim: usize, k: usize) -> Option<&mut Value> {
    if dim <= 1 {
        return (k == 0).then_some(v);
    }
    match v {
        Value::Array(items) => items.get_mut(k),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigCache;

    /// SETRES then SaveConfig: XIII.ini written into the player's folder (never into System),
    /// read again at the next start; without a player folder, nothing is written.
    #[test]
    fn ini_player_writes_and_reread() {
        let base = std::env::temp_dir().join(format!("oxiii-ini-{}", std::process::id()));
        let (system, player_value) = (base.join("System"), base.join("player"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("XIII.ini"), "[WinDrv.WindowsClient]\r\nFullscreenViewportX=800\r\nFullscreenViewportY=600\r\n\r\n").unwrap();
        let mut c = ConfigCache::with_user_dir(system.clone(), None);
        c.set_string("WinDrv.WindowsClient", "FullscreenViewportX", "1024", "System");
        c.flush();
        assert!(!player_value.exists());
        let mut c = ConfigCache::with_user_dir(system.clone(), Some(player_value.clone()));
        c.set_string("WinDrv.WindowsClient", "FullscreenViewportX", "1024", "System");
        c.set_string("WinDrv.WindowsClient", "FullscreenViewportY", "768", "System");
        c.flush();
        let text_value = std::fs::read_to_string(system.join("XIII.ini")).unwrap();
        assert!(text_value.contains("FullscreenViewportX=800"));
        let mut c = ConfigCache::with_user_dir(system, Some(player_value));
        assert_eq!(c.get_string("WinDrv.WindowsClient", "FullscreenViewportX", "System").as_deref(), Some("1024"));
        assert_eq!(c.get_string("WinDrv.WindowsClient", "FullscreenViewportY", "System").as_deref(), Some("768"));
        let _ = std::fs::remove_dir_all(base);
    }

    /// Lockstep co-op: a cache made in a session reads the host's User.ini and game ini (whatever
    /// this machine's files say), keeps this machine's own sections (window, render and sound
    /// devices), writes nothing to disk and keeps the session's files over an unloading Flush.
    #[test]
    fn session_ini_is_the_hosts_but_machine_sections() {
        let base = std::env::temp_dir().join(format!("oxiii-ini-session-{}", std::process::id()));
        let (system, player) = (base.join("System"), base.join("player"));
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(&player).unwrap();
        std::fs::write(player.join("XIII.ini"), "[WinDrv.WindowsClient]

WindowedViewportX=1280

[XIII.M16]

Ammo=10

[HXAudio.HXAudioSubsystem]

MusicVolume=0.2

").unwrap();
        std::fs::write(player.join("User.ini"), "[Engine.HUD]

HudScale=1

[XIII.Parental]

Lock=False

").unwrap();
        let host_system = "[WinDrv.WindowsClient]

WindowedViewportX=640

[XIII.M16]

Ammo=30

[HXAudio.HXAudioSubsystem]

MusicVolume=0.9

";
        let host_user = "[Engine.HUD]

HudScale=2

[XIII.Parental]

Lock=True

";
        let mut c = ConfigCache::with_user_dir(system.clone(), Some(player.clone()));
        c.apply_session_ini(Some((host_user.into(), Some(host_system.into()))));
        fn get(c: &mut ConfigCache, s: &str, k: &str, f: &str) -> Option<String> {
            c.get_string(s, k, f)
        }
        assert_eq!(get(&mut c, "XIII.M16", "Ammo", "System").as_deref(), Some("30"), "game section: the host's");
        assert_eq!(get(&mut c, "XIII.Parental", "Lock", "User").as_deref(), Some("True"), "user section: the host's");
        assert_eq!(get(&mut c, "WinDrv.WindowsClient", "WindowedViewportX", "System").as_deref(), Some("1280"), "window: this machine's");
        assert_eq!(get(&mut c, "HXAudio.HXAudioSubsystem", "MusicVolume", "System").as_deref(), Some("0.2"), "sound device: this machine's");
        c.set_string("XIII.M16", "Ammo", "31", "System");
        c.flush_file(true, None);
        assert_eq!(get(&mut c, "XIII.M16", "Ammo", "System").as_deref(), Some("31"), "kept over an unloading Flush");
        assert!(std::fs::read_to_string(player.join("XIII.ini")).unwrap().contains("Ammo=10"), "nothing written");
        // A profile's settings loaded during the session (OperationReadUserConfig): the host's,
        // this machine's sound volume kept.
        c.replace_file("System", "[XIII.M16]\r\nAmmo=40\r\n[HXAudio.HXAudioSubsystem]\r\nMusicVolume=1.0\r\n");
        assert_eq!(get(&mut c, "XIII.M16", "Ammo", "System").as_deref(), Some("40"));
        assert_eq!(get(&mut c, "HXAudio.HXAudioSubsystem", "MusicVolume", "System").as_deref(), Some("0.2"), "this machine's volume kept");
        // No session: this machine's files.
        let mut c = ConfigCache::with_user_dir(system, Some(player));
        c.apply_session_ini(None);
        assert_eq!(get(&mut c, "XIII.M16", "Ammo", "System").as_deref(), Some("10"));
        let _ = std::fs::remove_dir_all(base);
    }

    /// A ConfigName or ini name from a package (server, mod) never leaves the System and player folders.
    #[test]
    fn ini_name_stays_in_folders() {
        let c = ConfigCache::with_user_dir(std::path::PathBuf::from("System"), None);
        assert_eq!(c.file_name("XIIIMod"), "XIIIMod.ini");
        assert_eq!(c.file_name("Mod.int"), "Mod.int");
        assert_eq!(c.file_name(r"..\..\Windows\evil"), "evil.ini");
        assert_eq!(c.file_name(r"C:\evil.bat"), "evil.bat");
        assert_eq!(c.file_name("/etc/evil.cfg"), "evil.cfg");
        // ".." names the folder itself, never a file: nothing can be read or written there.
        assert_eq!(c.file_name(".."), "..");
        assert_eq!(c.file_name("C:"), ".ini");
    }
}

/// The actors that UGameEngine::LoadLevelLocalizedTexts localizes: in order, those that exist and whose
/// class has CLASS_Localized (ClassFlags & 0x20).
pub(crate) fn localized_actors(class_flags: &[Option<u32>]) -> Vec<usize> {
    class_flags.iter().enumerate().filter(|(_, f)| f.is_some_and(|f| f & CLASS_LOCALIZED != 0)).map(|(i, _)| i).collect()
}

