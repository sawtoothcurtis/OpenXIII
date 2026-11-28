//! Every `toml` block of the task pages of docs/modding (README.md and the pages it links,
//! not the expert pages nor the generated reference pages) is read as a mod file, then resolved
//! as the game does at startup: no error and no warning allowed.

use std::path::{Path, PathBuf};

/// The task pages: README.md and the pages written for the simple recipes.
const PAGES: &[&str] = &[
    "README.md",
    "spawn-a-character.md",
    "change-a-character.md",
    "change-a-weapon.md",
    "add-a-weapon.md",
    "add-an-object-or-decoration.md",
    "sounds-ambience-music.md",
    "make-a-character-talk.md",
    "find-names.md",
    "keyboard-shortcuts.md",
    "reactions.md",
    "multiplayer-and-coop.md",
    "troubleshooting.md",
];

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/modding")
}

/// The `toml` blocks of `text`: (line of the block's first line, text).
fn toml_blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        match &mut current {
            None if t == "```toml" => current = Some((i + 2, String::new())),
            Some(_) if t == "```" => out.push(current.take().unwrap()),
            Some((_, body)) => {
                body.push_str(line);
                body.push('\n');
            }
            None => {}
        }
    }
    out
}

#[test]
fn every_recipe_of_the_task_pages_loads_without_warnings() {
    let mut count = 0;
    let mut failures = Vec::new();
    for page in PAGES {
        let path = docs().join(page);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for (line, block) in toml_blocks(&text) {
            count += 1;
            let mut f = openxiii_mods::defaults::parse_mod_file(&block, &path, "docs");
            let mut problems: Vec<String> = f.problems.iter().map(|p| p.to_string()).collect();
            problems.extend(openxiii_mods::simple::resolve(&f.edits, &f.actions, &mut f.characters).iter().map(|p| p.to_string()));
            if !problems.is_empty() {
                failures.push(format!("{page}, block at line {line}: {problems:?}"));
            }
        }
    }
    assert!(count >= 15, "only {count} recipes found");
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Negative control: a block with a wrong name is caught.
#[test]
fn a_wrong_recipe_is_caught() {
    let block = "[[map.Plage01.add]]\ncharacter = \"Jomes\"\nposition = [0, 0, 0]\n";
    let mut f = openxiii_mods::defaults::parse_mod_file(block, Path::new("x.md"), "docs");
    let problems = openxiii_mods::simple::resolve(&f.edits, &f.actions, &mut f.characters);
    assert_eq!(problems.len(), 1);
    let (_, blocks) = (0, toml_blocks("text\n```toml\na = 1\n```\n"));
    assert_eq!(blocks, [(3, "a = 1\n".to_string())]);
}

/// Every example mod of the repository loads without errors or warnings, as the game reads it
/// (examples/mods; beach-weapons keeps its map in maps/Plage01.toml).
#[test]
fn example_mods_load_cleanly() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods");
    let found = openxiii_mods::discover_all(&dir);
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert!(found.mods.len() >= 8);
    let (edits, _) = openxiii_mods::collect_edits(&found.mods);
    let (actions, mut characters, _) = openxiii_mods::collect_behaviour(&found.mods);
    let problems = openxiii_mods::simple::resolve(&edits, &actions, &mut characters);
    assert!(problems.is_empty(), "{problems:?}");
    let sniper = found.mods.iter().find(|m| m.folder_name == "beach-weapons").unwrap();
    assert_eq!(sniper.edits.len(), 5, "{:?}", sniper.edits);
    assert!(characters.iter().any(|c| c.name == "Singer" && c.base == "XIDPawn.BaseSoldier"));
}

/// The committed pages of docs/modding/names/ are exactly what the game's own tables give
/// (OXIII_WRITE_NAME_PAGES=1 writes them first, after a change of the tables).
#[test]
fn committed_name_pages_match_the_tables() {
    use openxiii_mods::names_doc as n;
    let dir = docs().join("names");
    let pages: [(&str, String); 4] = [("README.md", n::index_page()), ("characters.md", n::characters_page()), ("weapons.md", n::weapons_page()), ("words.md", n::words_page())];
    if std::env::var_os("OXIII_WRITE_NAME_PAGES").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in &pages {
            std::fs::write(dir.join(file), text).unwrap();
        }
    }
    for (file, text) in &pages {
        let committed = std::fs::read_to_string(dir.join(file)).unwrap_or_else(|e| panic!("names/{file}: {e}"));
        if let Some(why) = n::page_differs(&committed, text) {
            panic!("docs/modding/names/{file} differs from the tables ({why}); run the tests with OXIII_WRITE_NAME_PAGES=1");
        }
    }
}

/// `[[shortcut]]` and `[[action]]` give the same keyboard shortcuts; an unknown block is
/// reported (negative control).
#[test]
fn shortcut_and_action_blocks_are_the_same() {
    let read = |text: &str| openxiii_mods::defaults::parse_mod_file(text, Path::new("m/mod.toml"), "m");
    let body = "key = \"K\"\nspawn = \"Jones\"\nin_front_of_player = 200\n";
    let a = read(&format!("[[action]]\n{body}"));
    let s = read(&format!("[[shortcut]]\n{body}"));
    assert!(a.problems.is_empty() && s.problems.is_empty(), "{:?} {:?}", a.problems, s.problems);
    assert_eq!(a.actions.iter().map(|x| (&x.key, &x.kind)).collect::<Vec<_>>(), s.actions.iter().map(|x| (&x.key, &x.kind)).collect::<Vec<_>>());
    assert_eq!(a.actions.len(), 1);
    let both = read(&format!("[[action]]\n{body}\n[[shortcut]]\nkey = \"L\"\ncommand = \"modpos\"\n"));
    assert_eq!(both.actions.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), ["K", "L"]);
    let wrong = read(&format!("[[shortcuts]]\n{body}"));
    assert!(wrong.actions.is_empty());
    assert!(wrong.problems.iter().any(|p| p.to_string().contains("unknown block shortcuts") && p.to_string().contains("did you mean shortcut")), "{:?}", wrong.problems);
}

/// The words of `table` (rows "| `word` |" of reactions.md) that are missing from `page`.
fn missing_rows<'a>(page: &str, words: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    words.filter(|w| !page.contains(&format!("| `{w}` |"))).collect()
}

/// reactions.md has a row for every event and every action a when block can name (the tables
/// of openxiii-mods when.rs), and the words "hears" and "use", which do not exist, are not in it.
#[test]
fn the_reactions_page_lists_every_event_and_action() {
    let page = std::fs::read_to_string(docs().join("reactions.md")).unwrap();
    let events = missing_rows(&page, openxiii_mods::when::EVENTS.iter().map(|e| e.name));
    let actions = missing_rows(&page, openxiii_mods::when::ACTIONS.iter().map(|a| a.name));
    assert!(events.is_empty() && actions.is_empty(), "missing from reactions.md: events {events:?}, actions {actions:?}");
    assert!(!page.contains("`hears`") && !page.contains("`use`"));
    // Negative control: a page without the table misses them all.
    assert_eq!(missing_rows("# Empty", openxiii_mods::when::ACTIONS.iter().map(|a| a.name)).len(), openxiii_mods::when::ACTIONS.len());
}

/// No shipped example binds a key the game window keeps (it would never run), in its mod.toml or
/// in its scripts (`on("key", "U", ...)`); such a key gives a warning when the mod is read, with
/// free keys named.
#[test]
fn no_example_uses_a_kept_key() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mods");
    let mut keys = Vec::new();
    for dir in std::fs::read_dir(&examples).unwrap().filter_map(Result::ok).map(|e| e.path()).filter(|p| p.join("mod.toml").is_file()) {
        let file = dir.join("mod.toml");
        let f = openxiii_mods::defaults::parse_mod_file(&std::fs::read_to_string(&file).unwrap(), &file, "example");
        keys.extend(f.actions.iter().map(|a| (a.key.clone(), file.display().to_string())));
        for e in &f.edits {
            if let openxiii_mods::Edit::When(w) = &e.edit {
                if let openxiii_mods::when::Event::Key { key } = &w.event {
                    keys.push((key.clone(), file.display().to_string()));
                }
            }
        }
        // The keys a script names: on("key", "U", ...).
        for script in std::fs::read_dir(dir.join("scripts")).into_iter().flatten().filter_map(Result::ok).map(|e| e.path()) {
            let text = std::fs::read_to_string(&script).unwrap_or_default();
            for part in text.split("on(\"key\", \"").skip(1) {
                if let Some(key) = part.split('"').next() {
                    keys.push((key.to_string(), script.display().to_string()));
                }
            }
        }
    }
    assert!(keys.len() >= 2, "{keys:?}");
    for (key, file) in &keys {
        assert!(openxiii_mods::behaviour::reserved_key(key).is_none(), "{file}: key {key} is kept by the game window");
    }
    // Negative control: F9 and F8 are reported when the mod is read, with the free keys.
    for key in ["F9", "F8"] {
        let shortcut = format!("[[shortcut]]\nkey = \"{key}\"\ncommand = \"modpos\"\n");
        let f = openxiii_mods::defaults::parse_mod_file(&shortcut, Path::new("m.toml"), "m");
        let text: Vec<String> = f.problems.iter().map(|p| p.to_string()).collect();
        assert!(text.iter().any(|t| t.contains(&format!("key {key} is kept by the game window")) && t.contains("free key (K, L")), "{text:?}");
        let when = format!("[[map.Plage01.when]]\nevent = \"key\"\nkey = \"{key}\"\nthen = [ {{ message = \"x\" }} ]\n");
        let f = openxiii_mods::defaults::parse_mod_file(&when, Path::new("m.toml"), "m");
        assert!(f.problems.iter().any(|p| p.to_string().contains("is kept by the game window")), "{:?}", f.problems);
    }
}
