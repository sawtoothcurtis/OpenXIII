//! The mods log: `Mods/mods-log.txt`, written at every start when the Mods folder exists (and
//! never when it does not). It lists each mod folder found, whether it was loaded, what each mod
//! adds or replaces, then every error and warning, then every line the game printed about the
//! mods during the run ("mods: …"), in order. The file is replaced at each start and rewritten
//! whenever a new line arrives, so it is always complete, even after a crash.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::placement::Edit;
use crate::{Discovery, Mod, Problem, DISABLED_FILE};

/// Name of the log file, inside the Mods folder.
pub const LOG_FILE: &str = "mods-log.txt";

/// Contents of the log: the part written at startup and the lines that came after.
#[derive(Debug, Default, Clone)]
pub struct ModsLog {
    path: PathBuf,
    /// Startup part: the folder, the mods and what they change.
    head: Vec<String>,
    /// Every "mods: …" line of the run (without the prefix), in order.
    lines: Vec<String>,
}

static LOG: Mutex<Option<ModsLog>> = Mutex::new(None);

/// True for a line that reports a problem ("warning: …", "error: …", or one of them after a
/// class or map name).
pub fn is_problem(line: &str) -> bool {
    let l = line.trim_start().to_ascii_lowercase();
    l.starts_with("warning") || l.starts_with("error") || l.contains(": warning") || l.contains(": error") || l.contains(" warning: ") || l.contains(" error: ")
}

impl ModsLog {
    /// The log of `folder` (its file is `folder/mods-log.txt`), with what `found` says.
    pub fn new(folder: &Path, found: &Discovery) -> ModsLog {
        let mut head = vec![
            "OpenXIII mods log".to_string(),
            "==================".to_string(),
            "Written by the game at every start, when this Mods folder exists (the previous log is replaced).".to_string(),
            format!("Mods folder: {}", folder.display()),
            format!(
                "Names a mod can use: {} (made by the game, with the lists of its names folder), and guide/names/ beside it (the modding guide)",
                crate::modding_folder(folder).join("Modding catalog.html").display()
            ),
            String::new(),
        ];
        // Every mod sub-folder, loaded or not, in load order (folder names); the examples and the
        // guide are not mods (crate::is_reserved_folder).
        let mut folders: Vec<(String, PathBuf)> = std::fs::read_dir(folder)
            .map(|d| d.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect())
            .unwrap_or_default();
        folders.retain(|(n, _)| !crate::is_reserved_folder(n));
        folders.sort_by(|a, b| a.0.cmp(&b.0));
        let loaded = found.mods.len();
        let turned_off = |name: &str| found.disabled.iter().any(|d| d.eq_ignore_ascii_case(name));
        let off = folders.iter().filter(|(n, d)| turned_off(n) && !found.mods.iter().any(|m| &m.folder == d)).count();
        let failed = folders.len().saturating_sub(loaded + off);
        head.push(if off == 0 {
            format!("Mod folders found: {} ({loaded} loaded, {failed} not loaded)", folders.len())
        } else {
            format!("Mod folders found: {} ({loaded} loaded, {off} turned off, {failed} not loaded)", folders.len())
        });
        head.push(String::new());
        for (name, dir) in &folders {
            match found.mods.iter().find(|m| &m.folder == dir) {
                Some(m) => {
                    head.push(format!("[loaded] {name}: \"{}\" version {}", m.info.name, m.info.version));
                    for l in summary(m) {
                        head.push(format!("    {l}"));
                    }
                }
                // Turned off on the Mods page: not a failure.
                None if turned_off(name) => {
                    head.push(format!("[turned off] {name}"));
                    head.push(format!("    turned off on the Mods page (listed in {DISABLED_FILE}): not loaded"));
                }
                None => {
                    head.push(format!("[NOT LOADED] {name}"));
                    let reasons: Vec<String> = found.problems.iter().filter(|p| problem_path(p).starts_with(dir)).map(|p| p.to_string()).collect();
                    if reasons.is_empty() {
                        head.push("    reason unknown".to_string());
                    }
                    for r in reasons {
                        head.push(format!("    {r}"));
                    }
                }
            }
            head.push(String::new());
        }
        ModsLog { path: folder.join(LOG_FILE), head, lines: Vec::new() }
    }

    /// Adds one line the game printed about the mods (with or without the "mods: " prefix).
    pub fn push(&mut self, line: &str) {
        let l = line.strip_prefix("mods: ").unwrap_or(line).trim_end();
        self.lines.push(l.to_string());
    }

    /// Text of the log file.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for l in &self.head {
            out.push_str(l);
            out.push_str("\r\n");
        }
        let problems: Vec<&String> = self.lines.iter().filter(|l| is_problem(l)).collect();
        out.push_str(&format!("Errors and warnings: {}\r\n", problems.len()));
        if problems.is_empty() {
            out.push_str("    none\r\n");
        }
        for p in problems {
            out.push_str(&format!("    {p}\r\n"));
        }
        out.push_str("\r\nEverything the game said about the mods during this run, in order:\r\n");
        for l in &self.lines {
            out.push_str(&format!("    {l}\r\n"));
        }
        out
    }

    /// Writes the log file (errors are ignored: the log never stops the game).
    pub fn write(&self) {
        let _ = std::fs::write(&self.path, self.text());
    }
}

/// The file or folder a discovery problem is about.
fn problem_path(p: &Problem) -> &Path {
    match p {
        Problem::Error { file, .. } => file,
        Problem::Warning { path, .. } => path,
    }
}

/// Short list of what `m` adds or replaces, one item per line.
pub fn summary(m: &Mod) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = m.info.description.as_deref().filter(|d| !d.trim().is_empty()) {
        out.push(format!("description: {}", d.trim()));
    }
    if !m.overrides.is_empty() {
        let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
        for o in &m.overrides {
            let key = match &o.map {
                Some(map) => format!("{} (on {map})", o.class),
                None => o.class.clone(),
            };
            *by_class.entry(key).or_default() += 1;
        }
        let list: Vec<String> = by_class.iter().map(|(c, n)| format!("{c} ({n})")).collect();
        out.push(format!("changes {} class value(s): {}", m.overrides.len(), list.join(", ")));
    }
    for c in &m.new_classes {
        out.push(format!("adds class {} (made from {})", c.class_name(), c.parent));
    }
    // Map changes: identical lines counted once ("adds XIII.Medkit on Plage01 (x3)").
    let mut edits: Vec<(String, usize)> = Vec::new();
    for e in &m.edits {
        let map = &e.map;
        let line = match &e.edit {
            Edit::Add { class, .. } => match class.strip_prefix(crate::placement::CHARACTER_PREFIX) {
                Some(name) => format!("places character {name} on {map}"),
                None => format!("adds {class} on {map}"),
            },
            Edit::Say(say) => format!("{} on {map}", say.describe()),
            Edit::When(w) => format!("{} on {map}", w.describe()),
            Edit::RemoveName { name } => format!("removes {name} from {map}"),
            Edit::RemoveNear { class, .. } => format!("removes {class} (near a position) from {map}"),
            Edit::Replace { from, to } => format!("replaces every {from} with {to} on {map}"),
            Edit::StartWeapon { class } => format!("gives {class} at the start of {map}"),
            Edit::StartAmmo { class, amount } => format!("gives {amount} {class} at the start of {map}"),
            Edit::ReplaceSound { from, to } => format!("plays {to} instead of {from} on {map}"),
            Edit::Change(c) => format!("changes {} on {map}", c.who),
        };
        match edits.iter_mut().find(|(l, _)| *l == line) {
            Some((_, n)) => *n += 1,
            None => edits.push((line, 1)),
        }
    }
    for (l, n) in edits {
        out.push(if n > 1 { format!("{l} (x{n})") } else { l });
    }
    for c in &m.characters {
        out.push(format!("adds character {} (from {})", c.name, c.base));
    }
    for a in &m.actions {
        out.push(format!("keyboard shortcut {}", a.key));
    }
    for s in &m.sounds {
        out.push(match &s.replace {
            Some(r) => format!("adds sound {} (played instead of {r})", s.sound_name()),
            None => format!("adds sound {}", s.sound_name()),
        });
    }
    for t in &m.textures {
        out.push(match &t.replace {
            Some(r) => format!("adds texture {} (drawn instead of {r})", t.texture_name()),
            None => format!("adds texture {}", t.texture_name()),
        });
    }
    for x in &m.models {
        out.push(match &x.replace {
            Some(r) => format!("adds model {} (drawn instead of {r})", x.mesh_name()),
            None => format!("adds model {}", x.mesh_name()),
        });
    }
    for l in &m.loads {
        let what: Vec<String> = l.sound_banks.iter().map(|b| format!("sound bank {b}")).chain(l.packages.iter().map(|p| format!("package {p}"))).collect();
        if !what.is_empty() {
            out.push(format!("loads {}{}", what.join(", "), l.map.as_ref().map(|m| format!(" on {m}")).unwrap_or_default()));
        }
    }
    for s in &m.scripts {
        out.push(format!("runs script {}", s.label));
    }
    if out.iter().all(|l| l.starts_with("description:")) {
        out.push("changes nothing".to_string());
    }
    out
}

/// Starts the log of this run: when `folder` exists, writes `folder/mods-log.txt` and keeps it
/// for the lines that follow; otherwise nothing is written.
pub fn start(folder: &Path, found: &Discovery) {
    if !folder.is_dir() {
        return;
    }
    let log = ModsLog::new(folder, found);
    log.write();
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log);
}

/// Adds a line to the log of this run and rewrites the file (nothing when no log was started).
pub fn note(line: &str) {
    let mut g = LOG.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(log) = g.as_mut() {
        log.push(line);
        log.write();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover;
    use std::fs;

    fn write(root: &Path, folder: &str, text: &str) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(crate::MANIFEST), text).unwrap();
    }

    /// A mod turned off (disabled-mods.txt) is listed as turned off, with the reason; without
    /// the file (negative control) the same mod is loaded.
    #[test]
    fn mods_turned_off_say_so() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("Mods");
        write(&mods, "kept", "[mod]
name = \"Kept\"
version = \"1.0\"
");
        write(&mods, "off", "[mod]
name = \"Off\"
version = \"1.0\"
");
        crate::write_disabled(&mods, &["off".to_string()]).unwrap();
        let text = ModsLog::new(&mods, &discover(&mods)).text();
        assert!(text.contains("Mod folders found: 2 (1 loaded, 1 turned off, 0 not loaded)"), "{text}");
        assert!(text.contains("[turned off] off"), "{text}");
        assert!(text.contains("turned off on the Mods page (listed in disabled-mods.txt)"), "{text}");
        assert!(!text.contains("NOT LOADED") && !text.contains("reason unknown"), "{text}");
        crate::write_disabled(&mods, &[]).unwrap();
        let text = ModsLog::new(&mods, &discover(&mods)).text();
        assert!(text.contains("[loaded] off") && !text.contains("turned off"), "{text}");
    }

    #[test]
    fn log_lists_mods_changes_and_problems() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("Mods");
        write(
            &mods,
            "good",
            "[mod]\nname = \"Good\"\nversion = \"1.0\"\ndescription = \"A test\"\n\n[class.\"XIII.Beretta\"]\nReloadCount = 30\nPickupAmmoCount = 30\n\n[map.Plage01.start]\nweapons = [\"XIII.Beretta\"]\n\n[[map.Plage01.add]]\nclass = \"XIII.MedKit\"\nposition = [0, 0, 0]\n\n[[map.Plage01.remove]]\nname = \"Chair0\"\n",
        );
        write(&mods, "broken", "[mod]\nname = \"Broken\"\nversion = \"1.0\n");
        fs::create_dir_all(mods.join("notamod")).unwrap();
        let found = discover(&mods);
        let mut log = ModsLog::new(&mods, &found);
        log.push("mods: warning: XIII.NoSuchClass: unknown class (mod good, good\\mod.toml line 9)");
        log.push("mods: XIII.Beretta.ReloadCount = 30 (mod good, good\\mod.toml line 6)");
        let text = log.text();
        assert!(text.contains("Mod folders found: 3 (1 loaded, 2 not loaded)"), "{text}");
        assert!(text.contains("[loaded] good: \"Good\" version 1.0"), "{text}");
        assert!(text.contains("description: A test"), "{text}");
        assert!(text.contains("changes 2 class value(s): XIII.Beretta (2)"), "{text}");
        assert!(text.contains("gives XIII.Beretta at the start of Plage01"), "{text}");
        assert!(text.contains("adds XIII.MedKit on Plage01"), "{text}");
        assert!(text.contains("removes Chair0 from Plage01"), "{text}");
        assert!(text.contains("[NOT LOADED] broken"), "{text}");
        assert!(text.contains("line 3"), "{text}");
        assert!(text.contains("[NOT LOADED] notamod"), "{text}");
        assert!(text.contains("no mod.toml, folder ignored"), "{text}");
        assert!(text.contains("Errors and warnings: 1"), "{text}");
        assert!(text.contains("    warning: XIII.NoSuchClass: unknown class"), "{text}");
        assert!(text.contains("    XIII.Beretta.ReloadCount = 30"), "{text}");
        // The error-and-warning list shows only the problems.
        let problems = text.split("Errors and warnings").nth(1).unwrap().split("Everything the game said").next().unwrap();
        assert!(!problems.contains("ReloadCount"), "{problems}");
    }

    #[test]
    fn log_written_only_when_the_folder_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("Mods");
        let log = ModsLog::new(&missing, &discover(&missing));
        // `start` writes nothing without a folder.
        start(&missing, &Discovery::default());
        assert!(!missing.exists());
        // With the folder: the file is written, and replaced at the next start.
        fs::create_dir_all(&missing).unwrap();
        fs::write(missing.join(LOG_FILE), "old run").unwrap();
        let log2 = ModsLog::new(&missing, &discover(&missing));
        log2.write();
        let text = fs::read_to_string(missing.join(LOG_FILE)).unwrap();
        assert!(!text.contains("old run"), "{text}");
        assert!(text.contains("Mod folders found: 0 (0 loaded, 0 not loaded)"), "{text}");
        assert!(text.contains("Errors and warnings: 0"), "{text}");
        drop(log);
    }

    #[test]
    fn problem_lines() {
        assert!(is_problem("warning: texture Mod.X defined twice"));
        assert!(is_problem("error: a.toml (line 2): bad"));
        assert!(is_problem("Plage01: warning: no actor named Chair0"));
        assert!(!is_problem("XIII.Beretta.ReloadCount = 30 (mod good)"));
    }
}
