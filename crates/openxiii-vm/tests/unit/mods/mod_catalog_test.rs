//! Mods build only: the data of the names catalog on real maps (OXIII_GAME = game folder;
//! skipped when it is not set), and the page made from it.

use std::path::PathBuf;

use crate::mod_catalog::{catalog_html, Catalog, LookRow};
use crate::mod_maps::{MapEntry, PlacedCharacter};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

/// PRock03 lists the guard who loses his pass, with his model, and `who` finds him alone;
/// Plage01 (negative control) does not list him, and a name one letter off finds nothing.
#[test]
fn placed_characters_of_prock03() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let prock = Vm::read_map_entry(&game, "PRock03").unwrap();
    assert!(!prock.characters.is_empty());
    let guard: Vec<&PlacedCharacter> = prock.characters.iter().filter(|p| p.who.eq_ignore_ascii_case("gardien_lostpass")).collect();
    assert_eq!(guard.len(), 1, "{:#?}", prock.characters.iter().map(|p| (&p.who, &p.name, &p.model)).collect::<Vec<_>>());
    let guard = guard[0];
    assert_eq!(guard.model, "XIIIPersos.GardienM");
    assert!(guard.pawn_name.eq_ignore_ascii_case("gardien_lostpass") || guard.tag.eq_ignore_ascii_case("gardien_lostpass"));
    assert!(guard.class.contains('.'), "{}", guard.class);
    assert!(guard.health.is_some_and(|h| h > 0));
    // Every `who` names its character alone; when it is not the PawnName or the Tag, it is the
    // object name (two characters sharing a PawnName).
    for p in &prock.characters {
        assert!(!p.who.is_empty());
        if !p.who.eq_ignore_ascii_case(&p.pawn_name) && !p.who.eq_ignore_ascii_case(&p.tag) {
            assert_eq!(p.who, p.name);
        }
    }
    // The same lookup a mod's `who` goes through, on the loaded map.
    let mut vm = crate::mod_catalog::game_only_vm();
    vm.load_dir(&game.join("System")).unwrap();
    vm.url = crate::travel::start_url(&game.join("System"), "PRock03");
    vm.load_map(&game.join("Maps").join("PRock03.unr")).unwrap();
    let found = vm.find_placed("gardien_lostpass", None, 0.0);
    assert_eq!(found.len(), 1);
    assert_eq!(vm.objects[found[0]].as_ref().unwrap().name, guard.name);
    assert!(vm.find_placed("gardien_lostpas", None, 0.0).is_empty());
    // Negative control: another map.
    let plage = Vm::read_map_entry(&game, "Plage01").unwrap();
    assert!(!plage.characters.is_empty());
    assert!(!plage.characters.iter().any(|p| p.who.eq_ignore_ascii_case("gardien_lostpass") || p.pawn_name.eq_ignore_ascii_case("gardien_lostpass")));
}

/// The page carries the rows to paste, with nothing that would end its script early.
#[test]
fn page_from_a_small_catalog() {
    let guard = PlacedCharacter {
        name: "XIIIBaseSoldier3".into(),
        class: "XIDPawn.BaseSoldier".into(),
        pawn_name: "gardien_lostpass".into(),
        tag: "gardien_lostpass".into(),
        model: "XIIIPersos.GardienM".into(),
        health: Some(100),
        position: [1.0, 2.0, 3.0],
        who: "gardien_lostpass".into(),
        voice: None,
    };
    let c = Catalog {
        language: "int".into(),
        looks: vec![LookRow {
            model: "XIIIPersos.JonesM".into(),
            names: vec!["Jones".into()],
            wearers: vec!["Major Jones".into()],
            placed: 1,
            maps: vec!["Toits01".into()],
        }],
        maps: vec![(MapEntry { name: "PRock03".into(), title: "Prison </script> <b>".into(), characters: vec![guard], ..Default::default() }, "solo")],
        sounds: vec!["XIIISound.Doors.Open".into()],
        ..Default::default()
    };
    let html = catalog_html(&c);
    assert!(!html.contains("/*CATALOG_DATA*/"));
    assert!(html.contains(r#"look = \"Jones\""#));
    assert!(html.contains(r#"[[map.PRock03.change]]\nwho = \"gardien_lostpass\""#));
    assert!(html.contains("XIIIPersos.GardienM"));
    let data = &html[html.find("catalog-data").unwrap()..];
    let data = &data[..data.find("</script>").unwrap()];
    assert!(!data.contains("<b>") && data.contains("\\u003c/script"));
    // Negative control: without the guard, the page does not name him.
    let mut empty = c.clone();
    empty.maps[0].0.characters.clear();
    assert!(!catalog_html(&empty).contains("gardien_lostpass"));
}

#[test]
fn measure_voices_and_weapons() {
    let (Some(game), true) = (game(), false) else { return };
    let names = crate::mod_catalog::map_names(&game).unwrap();
    let maps = crate::mod_catalog::read_maps(&game, &names, 2, &|_, _| {});
    let mut by_model: std::collections::BTreeMap<String, std::collections::BTreeMap<i32, usize>> = Default::default();
    for m in &maps {
        for p in &m.characters {
            if let Some(v) = p.voice {
                *by_model.entry(p.model.clone()).or_default().entry(v).or_default() += 1;
            }
        }
    }
    for (model, voices) in by_model {
        eprintln!("VOICE {model} {voices:?}");
    }
}

#[test]
fn measure_catalog_phases() {
    let (Some(game), true) = (game(), false) else { return };
    let t = std::time::Instant::now();
    let mut vm = crate::mod_catalog::game_only_vm();
    vm.load_dir(&game.join("System")).unwrap();
    let load_dir = t.elapsed();
    let t = std::time::Instant::now();
    let mut c = crate::mod_catalog::Catalog::default();
    vm.catalog_from_packages(&mut c).unwrap();
    let packages = t.elapsed();
    let mut per_map = Vec::new();
    for n in ["Plage01", "PRock03", "Palace01", "DM_Amos"] {
        let t = std::time::Instant::now();
        let mut v = crate::mod_catalog::game_only_vm();
        v.load_dir(&game.join("System")).unwrap();
        let a = t.elapsed();
        v.url = crate::travel::start_url(&game.join("System"), n);
        v.load_map(&game.join("Maps").join(format!("{n}.unr"))).unwrap();
        let b = t.elapsed();
        let e = v.map_entry_here(n);
        let _ = (e, v.placed_characters_list());
        per_map.push(format!("{n}: load_dir {:?}, load_map {:?}, read {:?}", a, b - a, t.elapsed() - b));
    }
    let names = crate::mod_catalog::map_names(&game).unwrap();
    for threads in [1usize, 4, 8, 16] {
        let t = std::time::Instant::now();
        let _ = crate::mod_catalog::read_maps(&game, &names, threads, &|_, _| {});
        eprintln!("MEASURE read_maps {} maps on {threads} threads: {:?}", names.len(), t.elapsed());
    }
    eprintln!("MEASURE load_dir {load_dir:?}, packages {packages:?}");
    for l in per_map {
        eprintln!("MEASURE {l}");
    }
}

/// One Vm reading several maps in a row (Vm::unload_level between them, as read_maps does)
/// gives exactly what a fresh Vm gives for each map: same entries, characters and events.
#[test]
fn a_reused_vm_reads_maps_like_a_fresh_one() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let maps = ["PRock03", "Plage01", "Palace01", "DM_Amos", "Hual01a", "PRock03"];
    let mut vm = crate::mod_catalog::game_only_vm();
    vm.load_dir(&game.join("System")).unwrap();
    for m in maps {
        let reused = vm.read_map_here(&game, m).unwrap();
        let fresh = Vm::read_map_entry(&game, m).unwrap();
        assert_eq!(format!("{reused:?}"), format!("{fresh:?}"), "{m}");
        assert!(!fresh.characters.is_empty() || m == "DM_Amos", "{m}");
    }
    // PRock03's events and its guard, read a second time after other maps.
    let prock = vm.read_map_here(&game, "PRock03").unwrap();
    assert!(prock.characters.iter().any(|p| p.who == "gardien_lostpass"));
    assert!(!prock.events.is_empty());
    // Negative control: two different maps do not give the same entry.
    let plage = vm.read_map_here(&game, "Plage01").unwrap();
    assert_ne!(format!("{plage:?}"), format!("{prock:?}"));
}

/// The lists made on the player's computer, from the real game: a sample of each, no text of
/// a dialogue, and a negative control (without PRock03, its guard is not listed).
#[test]
fn name_pages_from_the_game() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let c = Vm::build_catalog(&game, &|_| {}).unwrap();
    let pages: std::collections::HashMap<&str, String> = crate::mod_names_doc::name_pages(&c).into_iter().collect();
    for file in ["README.md", "characters.md", "weapons.md", "words.md", "weapons-and-ammo.md", "objects.md", "decorations.md", "maps.md", "events.md", "dialogues.md", "sounds.md", "ambient-sounds.md", "music.md", "animations.md"] {
        assert!(pages.get(file).is_some_and(|p| p.len() > 200), "{file}");
    }
    let maps = &pages["maps.md"];
    let prock = &maps[maps.find("## PRock03").unwrap()..];
    let prock = &prock[..prock[3..].find("\n## ").map_or(prock.len(), |i| i + 3)];
    assert!(prock.contains("| gardien_lostpass |") && prock.contains("XIIIPersos.GardienM"), "{prock}");
    assert!(pages["music.md"].contains("`XIIISound.Music.Banque01`"));
    assert!(pages["dialogues.md"].contains("`Plage01_Pam_00`"));
    assert!(pages["weapons-and-ammo.md"].contains("| XIII.FusilPompe |") && pages["weapons-and-ammo.md"].contains("XIII.c9mmAmmo"));
    assert!(pages["events.md"].contains("## PRock03"));
    assert!(pages["animations.md"].contains("Used by:"));
    // No text of a dialogue anywhere: none of the subtitles appears in any page.
    for l in c.dialogues.iter().filter(|l| l.subtitle.trim().len() > 20) {
        for (file, p) in &pages {
            assert!(!p.contains(l.subtitle.trim()), "{file} holds the subtitle of {}", l.name);
        }
    }
    // Negative control.
    let mut without = c.clone();
    without.maps.retain(|(e, _)| e.name != "PRock03");
    let pages = crate::mod_names_doc::name_pages(&without);
    assert!(!pages.iter().any(|(f, p)| *f == "maps.md" && p.contains("gardien_lostpass")));
}

/// The catalog's stamp: written with the lists, current for this game, not for another game
/// or another stamp (made again then), and only one writer at a time.
#[test]
fn catalog_stamp_and_lists() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let dir = std::env::temp_dir().join(format!("oxiii-catalog-stamp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let guide = openxiii_mods::modding_folder(&dir.join("Mods"));
    let file = guide.join(crate::mod_catalog::FILE_NAME);
    assert!(!crate::mod_catalog::catalog_is_current(&file, &game), "missing: to be made");
    Vm::write_catalog(&game, &file, &|_| {}).unwrap();
    assert!(crate::mod_catalog::catalog_is_current(&file, &game));
    assert!(guide.join("names").join("maps.md").is_file());
    assert!(guide.ends_with("Mods/_Guide"), "{}", guide.display());
    assert!(!crate::mod_catalog::catalog_is_current(&file, &dir), "another game folder: made again");
    // Another stamp (another OpenXIII): made again.
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replacen("format ", "format 0", 1)).unwrap();
    assert!(!crate::mod_catalog::catalog_is_current(&file, &game));
    // Lists missing: made again.
    std::fs::write(&file, &text).unwrap();
    assert!(crate::mod_catalog::catalog_is_current(&file, &game));
    std::fs::remove_file(guide.join("names").join("README.md")).unwrap();
    assert!(!crate::mod_catalog::catalog_is_current(&file, &game));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Scripts tab and scripts.md: every event and action of the toml's when blocks, every event
/// and function of the scripts, each with its example; the catalog's format is that of a
/// page with the tab (an older catalog is written again).
#[test]
fn scripts_tab_and_page() {
    let rows = crate::mod_names_doc::script_rows();
    let has = |kind: &str, name: &str| rows.iter().any(|r| r[2] == kind && r[1] == name && !r[0].is_empty());
    for e in openxiii_mods::when::EVENTS {
        assert!(has("when block: event", e.name), "{}", e.name);
    }
    for a in openxiii_mods::when::ACTIONS {
        assert!(has("when block: action", a.name), "{}", a.name);
    }
    for (e, ..) in crate::mod_script::EVENTS {
        assert!(has("script: event", e), "{e}");
    }
    for (name, ..) in crate::mod_script::reference_rows() {
        assert!(has("script: function", &name), "{name}");
    }
    let html = catalog_html(&Catalog::default());
    assert!(html.contains("{ id: \"scripts\", title: \"Scripts\""));
    let data = &html[html.find("catalog-data").unwrap()..];
    assert!(data.contains("\"scripts\":[") && data.contains("player():give(\\\"Shotgun\\\")") && data.contains("when block: action"));
    let pages = crate::mod_names_doc::name_pages(&Catalog::default());
    let (_, page) = pages.iter().find(|(f, _)| *f == "scripts.md").expect("scripts.md");
    for name in ["`near`", "`give`", "`camera_back`", "`actor:go_to`", "`wait_until`"] {
        assert!(page.contains(name), "{name} in scripts.md");
    }
    assert!(crate::mod_catalog::catalog_stamp(std::path::Path::new("x")).contains(" format 3 "));
    // Negative control: a name nobody has is not found.
    assert!(!has("script: function", "explode") && !page.contains("`explode`"));
}
