//! Mods build only (feature `mods`): the lists of names written next to the names catalog,
//! in `Mods/_Guide/names/` on the player's computer (mod_catalog.rs). The lists of OpenXIII's own
//! tables (characters, weapons, fixed words, and the index) come from openxiii-mods'
//! names_doc.rs, the same text as docs/modding/names/; the others are made from the catalog's
//! data, read from the player's own game: identifiers only, never a subtitle or any text of a
//! dialogue.

use std::collections::BTreeMap;

use openxiii_mods::names_doc as own;

use crate::mod_catalog::Catalog;

fn line(cells: &[&str]) -> String {
    format!("| {} |\n", cells.iter().map(|c| c.replace('|', "\\|").replace(['\r', '\n'], " ")).collect::<Vec<_>>().join(" | "))
}

fn page(title: &str, intro: &str) -> String {
    format!("# {title}\n\n{}\n\n{intro}\n\n", own::GENERATED_GAME)
}

/// "Package.Group" of "Package.Group.Name" (or the package alone).
fn group_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(g, _)| g)
}

/// Lines grouped under a heading per key, in order.
/// Rows under one `## group` heading each; `head` starts every group (a table's header, or nothing).
fn grouped<'a>(head: &str, items: impl Iterator<Item = (&'a str, String)>) -> String {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (key, row) in items {
        groups.entry(key.to_string()).or_default().push(row);
    }
    let mut out = String::new();
    for (key, rows) in groups {
        out.push_str(&format!("## {key}\n\n{head}"));
        for r in rows {
            out.push_str(&r);
        }
        out.push('\n');
    }
    out
}

/// The reactions of the toml (`[[map.<Map>.when]]`: events, actions) and the scripts' events
/// and functions, as rows (paste, name, kind, arguments or settings, what it does): for the
/// catalog's Scripts tab and scripts.md.
pub(crate) fn script_rows() -> Vec<[String; 5]> {
    let mut out = Vec::new();
    for e in openxiii_mods::when::EVENTS {
        let settings: Vec<&str> = e.required.iter().chain(e.optional).copied().collect();
        let gives = if e.gives.is_empty() { "nothing".to_string() } else { e.gives.join(", ") };
        out.push([e.example.to_string(), e.name.to_string(), "when block: event".into(), settings.join(", "), format!("{}; gives {gives}", e.about)]);
    }
    for a in openxiii_mods::when::ACTIONS {
        let settings = if a.settings.is_empty() { a.value.to_string() } else { format!("{}; {}", a.value, a.settings.join(", ")) };
        out.push([a.example.to_string(), a.name.to_string(), "when block: action".into(), settings, a.about.to_string()]);
    }
    for (e, args, gets, when) in crate::mod_script::EVENTS {
        let then = if args.is_empty() { String::new() } else { format!(", {args}") };
        out.push([format!("on(\"{e}\"{then}, function({gets}) end)"), e.to_string(), "script: event".into(), args.to_string(), format!("{when}; the function receives {}", if gets.is_empty() { "nothing" } else { gets })]);
    }
    for (name, args, about, example) in crate::mod_script::reference_rows() {
        out.push([example.to_string(), name, "script: function".into(), args.to_string(), about.to_string()]);
    }
    out
}

/// scripts.md: the same rows as the catalog's Scripts tab, one table per kind.
fn scripts_page() -> String {
    let mut out = format!("# Reactions and scripts\n\n{}\n\n", own::GENERATED_STATIC);
    out.push_str("The events and actions of `[[map.<Map>.when]]` blocks (docs/modding/reactions.md of the modding guide),\n");
    out.push_str("then the events and functions of the mods' Lua scripts (`scripts` folder of a mod).\n\n");
    let rows = script_rows();
    for (kind, title, cols) in [
        ("when block: event", "When blocks: events", "| Event | Its settings | When, and what it gives | Example |"),
        ("when block: action", "When blocks: actions", "| Action | Its value; its settings | What it does | Example |"),
        ("script: event", "Scripts: events of on(event, ..., fn)", "| Event | Then | When | Example |"),
        ("script: function", "Scripts: functions", "| Function | Arguments | What it does | Example |"),
    ] {
        out.push_str(&format!("## {title}\n\n{cols}\n|---|---|---|---|\n"));
        for r in rows.iter().filter(|r| r[2] == kind) {
            out.push_str(&line(&[&format!("`{}`", r[1]), &r[3], &r[4], &format!("`{}`", r[0])]));
        }
        out.push('\n');
    }
    out
}

/// Every page of `Mods/_Guide/names/`: (file, text).
pub fn name_pages(c: &Catalog) -> Vec<(&'static str, String)> {
    let mut pages = vec![("README.md", own::index_page()), ("characters.md", own::characters_page()), ("weapons.md", own::weapons_page()), ("words.md", own::words_page())];

    let mut w = page("Weapons and ammo", "Every weapon class of the game, with its pickup and its ammo, then every ammo class. A class goes in `give = \"…\"`, `[class.\"…\"]` and, for a pickup, in `class = \"…\"` of `[[map.<Map>.add]]`.");
    w.push_str("## Weapons\n\n| Class | Name in the game | Simple name | Pickup | Ammo |\n|---|---|---|---|---|\n");
    for x in &c.weapons {
        w.push_str(&line(&[&x.class, &x.name, openxiii_mods::simple::weapon_name(&x.class).unwrap_or(""), &x.pickup, &x.ammo]));
    }
    w.push_str("\n## Ammo\n\n| Class | Name in the game |\n|---|---|\n");
    for (class, name) in &c.ammo {
        w.push_str(&line(&[class, name]));
    }
    pages.push(("weapons-and-ammo.md", w));

    let mut o = page("Objects and pickups", "Classes a mod can place with `class = \"…\"` in `[[map.<Map>.add]]`.");
    o.push_str("| Class | Kind | Gives |\n|---|---|---|\n");
    for x in &c.objects {
        let gives = if x.name.is_empty() { x.gives.clone() } else { format!("{} ({})", x.gives, x.name) };
        o.push_str(&line(&[&x.class, x.kind, &gives]));
    }
    pages.push(("objects.md", o));

    let mut d = page("Decorations", "Solid models (static meshes) a mod can place with `model = \"…\"` in `[[map.<Map>.add]]`, by package. Search with Ctrl+F.");
    d.push_str(&grouped("", c.decorations.iter().map(|m| (m.split('.').next().unwrap_or(m), format!("- `{m}`\n")))));
    pages.push(("decorations.md", d));

    let mut m = page("Maps", "Every map: the name for `[map.<Map>…]`, its checkpoints (`from_checkpoint`) and the characters it places, with the `who` value that finds each one alone (`[[map.<Map>.change]]`, `by` of `[[map.<Map>.say]]`).");
    for (e, kind) in &c.maps {
        m.push_str(&format!("## {} ({kind})\n\n", e.name));
        if !e.saves.is_empty() {
            let cps: Vec<String> = e.saves.iter().enumerate().map(|(i, s)| format!("{} {}", i + 1, s.description.trim())).collect();
            m.push_str(&format!("Checkpoints: {}.\n\n", cps.join(", ")));
        }
        if e.characters.is_empty() {
            m.push_str("No character placed.\n\n");
            continue;
        }
        m.push_str("| who | Actor | Class | Model | Health |\n|---|---|---|---|---|\n");
        for p in &e.characters {
            m.push_str(&line(&[&p.who, &p.name, &p.class, &p.model, &p.health.map(|h| h.to_string()).unwrap_or_default()]));
        }
        m.push('\n');
    }
    pages.push(("maps.md", m));

    let mut ev = page("Events", "The events each map triggers: the names `event = \"…\"` of a line said `when = \"event\"` can wait for.");
    for (e, _) in c.maps.iter().filter(|(e, _)| !e.events.is_empty()) {
        ev.push_str(&format!("## {}\n\n{}\n\n", e.name, e.events.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")));
    }
    pages.push(("events.md", ev));

    let mut dl = page("Dialogue lines", "The lines of the game a character can say: `line = \"…\"` in `[[map.<Map>.say]]`, with who says it in the game. The subtitle is shown in the game in the player's language.");
    dl.push_str(&grouped("| Line | Said by | Kind |\n|---|---|---|\n", c.dialogues.iter().map(|l| (l.level.as_str(), line(&[&format!("`{}`", l.name), &l.speaker, if l.voice { "voice" } else { "subtitle only" }])))));
    pages.push(("dialogues.md", dl));

    let mut s = page("Sound effects", "Every sound of the game but the music, by group: `sound = \"…\"`, `[map.<Map>.replace_sounds]`. Search with Ctrl+F.");
    s.push_str(&grouped("", c.sounds.iter().map(|x| (group_of(x), format!("- `{x}`\n")))));
    pages.push(("sounds.md", s));

    let mut a = page("Ambient sounds", "The sounds each map's actors play as ambience (their `AmbientSound`): the names `[map.<Map>.replace_sounds]` replaces.");
    for (e, _) in c.maps.iter().filter(|(e, _)| !e.ambient.is_empty()) {
        a.push_str(&format!("## {}\n\n", e.name));
        for (sound, n) in &e.ambient {
            a.push_str(&format!("- `{sound}`{}\n", if *n > 1 { format!(" ({n} actors)") } else { String::new() }));
        }
        a.push('\n');
    }
    pages.push(("ambient-sounds.md", a));

    let mut mu = page("Music", "The music of the levels (the `Music…` groups of `XIIISound`), by group.");
    mu.push_str(&grouped("", c.music.iter().map(|x| (group_of(x), format!("- `{x}`\n")))));
    pages.push(("music.md", mu));

    let mut an = page("Animations", "The animations of the characters and of the weapons seen in first person, by animation set: `animation = \"…\"` of `[character.<Name>.state.<State>]`.");
    for (set, users, seqs) in &c.animations {
        an.push_str(&format!("## {set}\n\nUsed by: {}\n\n{}\n\n", users.join(", "), seqs.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")));
    }
    pages.push(("animations.md", an));
    pages.push(("scripts.md", scripts_page()));
    pages
}
