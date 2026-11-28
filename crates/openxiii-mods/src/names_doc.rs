//! The pages of docs/modding/names/ that come only from the built-in tables (no game file):
//! the characters' simple names and voices, the weapons' simple names, the fixed words, and the
//! index. The other pages of that folder are made from the player's game by the VM
//! (openxiii-vm mod_names_doc.rs, `oxiii mods-reference`). A test compares the committed pages
//! with these functions, so the documentation never drifts from what the game accepts.

use crate::looks::{CHARACTER_LOOKS, LOOK_INFO};
use crate::simple::{VOICE_NAMES, WEAPON_NAMES};

/// First line of every page of the folder.
pub const GENERATED_STATIC: &str = "Generated from OpenXIII's own tables: every name below is accepted as written.";
/// Marks a page made from the game's files.
pub const GENERATED_GAME: &str =
    "Written by OpenXIII from the game's own files (again with the row \"Refresh the names catalog\" of the Mods page): identifiers only, no text of the game.";

fn line(cells: &[&str]) -> String {
    format!("| {} |\n", cells.iter().map(|c| c.replace('|', "\\|")).collect::<Vec<_>>().join(" | "))
}

/// docs/modding/names/characters.md: every simple name of a character (`character = "…"`,
/// `look = "…"`, `spawn = "…"`), then the voices.
pub fn characters_page() -> String {
    let mut out = String::from("# Characters\n\n");
    out.push_str(GENERATED_STATIC);
    out.push_str("\n\n");
    out.push_str("The name to write in the settings `character = \"…\"` (to add a character), `look = \"…\"` (to give a\n");
    out.push_str("character this look) or `spawn = \"…\"` (a keyboard shortcut that makes one appear). Names are not\n");
    out.push_str("case-sensitive; the other spellings work too. The model (last column) also works, and so does\n");
    out.push_str("any other model of the game (see [the catalog](../find-names.md)). See\n");
    out.push_str("[spawn-a-character.md](../spawn-a-character.md) and [change-a-character.md](../change-a-character.md).\n\n");
    out.push_str("| Name | Other spellings | Who it is | Met in | Model |\n|---|---|---|---|---|\n");
    for (model, names) in CHARACTER_LOOKS {
        let (who, levels) = LOOK_INFO.iter().find(|(m, _, _)| m == model).map(|(_, w, l)| (*w, *l)).unwrap_or(("", ""));
        let short = model.rsplit('.').next().unwrap_or(model);
        let mut other: Vec<String> = names[1..].iter().map(|s| s.to_string()).collect();
        other.push(short.to_string());
        let met = if levels.is_empty() { "not in the story" } else { levels };
        out.push_str(&line(&[names[0], &other.join(", "), who, met, model]));
    }
    out.push_str("\n## Voices\n\n");
    out.push_str("The voice of a character's shouts (pain, death, alerts): `voice = \"…\"`. A name below, a\n");
    out.push_str("character name of the table above (the voice that character has in the game), or a number.\n\n");
    out.push_str("| Voice | Number |\n|---|---|\n");
    for (code, names) in VOICE_NAMES {
        out.push_str(&line(&[names[0], &code.to_string()]));
    }
    out
}

/// The simple weapon names, the first section of docs/modding/names/weapons.md.
pub fn weapons_section() -> String {
    let mut out = String::from("## Simple names\n\n");
    out.push_str("The name to write in `weapon = \"…\"` (a character's weapon) and in `weapons = [\"…\"]` of\n");
    out.push_str("`[map.<Map>.start]`. The class (last column) works there too, and so does any weapon class\n");
    out.push_str("of the list below.\n\n");
    out.push_str("| Name | Other spellings | Class |\n|---|---|---|\n");
    for (class, names) in WEAPON_NAMES {
        let short = class.rsplit('.').next().unwrap_or(class);
        let mut other: Vec<String> = names[1..].iter().map(|s| s.to_string()).collect();
        if !names.iter().any(|n| n.eq_ignore_ascii_case(short)) {
            other.push(short.to_string());
        }
        out.push_str(&line(&[names[0], &other.join(", "), class]));
    }
    out
}

/// A list of fixed words: (word, meaning).
fn words(out: &mut String, title: &str, intro: &str, list: &[(&str, &str)]) {
    out.push_str(&format!("## {title}\n\n{intro}\n\n| Word | Meaning |\n|---|---|\n"));
    for (w, m) in list {
        out.push_str(&line(&[w, m]));
    }
    out.push('\n');
}

/// docs/modding/names/words.md: every fixed word and key name a mod can write.
pub fn words_page() -> String {
    let mut out = String::from("# Fixed words\n\n");
    out.push_str(GENERATED_STATIC);
    out.push_str("\n\n");
    words(&mut out, "team", "`team = \"…\"`: how a character treats XIII.", &[("friend", "never attacks XIII"), ("neutral", "leaves XIII alone"), ("enemy", "attacks XIII")]);
    words(
        &mut out,
        "behaviour",
        "`behaviour = \"…\"`: what a character a mod adds does.",
        &[("stands", "a calm story character that stays where it is (the default)"), ("patrols", "a soldier with the game's AI"), ("fights", "a soldier that attacks XIII")],
    );
    words(
        &mut out,
        "when (dialogues)",
        "`when = \"…\"` of `[[map.<Map>.say]]`: when the line is said.",
        &[("near", "when XIII comes within `distance` (the default)"), ("start", "when the level starts"), ("event", "when the map triggers `event = \"…\"` (see [events.md](events.md))")],
    );
    words(
        &mut out,
        "Map selectors",
        "The `<Map>` part of `[map.<Map>…]`.",
        &[("Plage01", "one map (see [maps.md](maps.md))"), ("Hual*", "every map whose name starts with Hual"), ("solo", "every map of the story"), ("multi", "every multiplayer map"), ("*", "every map")],
    );
    words(
        &mut out,
        "Keyboard shortcuts",
        "One of these settings in each `[[shortcut]]` block (see [keyboard-shortcuts.md](../keyboard-shortcuts.md)).",
        &[("spawn", "a character appears in front of the player"), ("give", "a weapon for the player"), ("sound", "a sound plays at the player"), ("command", "a console command runs")],
    );
    let events: Vec<(&str, &str)> = crate::when::EVENTS.iter().map(|e| (e.name, e.about)).collect();
    words(&mut out, "Events (reactions)", "`event = \"…\"` of `[[map.<Map>.when]]` (see [reactions.md](../reactions.md)).", &events);
    let actions: Vec<(&str, &str)> = crate::when::ACTIONS.iter().map(|a| (a.name, a.about)).collect();
    words(&mut out, "Actions (reactions)", "Each item of `then = [ … ]` of `[[map.<Map>.when]]`: `{ <action> = … }`.", &actions);
    let mut keys: Vec<String> = (1..=12).map(|n| format!("F{n}")).collect();
    keys.push("A … Z".into());
    keys.push("0 … 9".into());
    keys.push("NumPad0 … NumPad9".into());
    keys.extend(["Insert", "Delete", "Home", "End", "PageUp", "PageDown"].iter().map(|s| s.to_string()));
    out.push_str("## Key names\n\nThe setting `key = \"…\"` of a `[[shortcut]]` block (the cheat build of the game already uses F5, F6, F7 and F11).\n\n");
    out.push_str(&format!("{}\n\n", keys.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ")));
    words(
        &mut out,
        "Weapon fire modes",
        "`WeaponMode = \"…\"` in a weapon's class table.",
        &[("WM_Auto", "fires while the button is held"), ("WM_SemiAuto", "one shot per press"), ("WM_Burst", "three shots per press")],
    );
    words(
        &mut out,
        "How a weapon is held",
        "`FiringMode = \"…\"` in a weapon's class table (how a character holds and fires it).",
        &[
            ("FM_1H", "one hand (9mm, magnum, mini Uzi)"),
            ("FM_2H", "two hands (hunting rifle, pump shotgun, crossbows, harpoon)"),
            ("FM_M16", "assault rifle (M16, Kalashnikov)"),
            ("FM_2HHeavy", "heavy machine gun (M60)"),
            ("FM_Bazook", "bazooka"),
            ("FM_Snipe", "sniper rifle"),
            ("FM_Throw", "thrown (grenades, knives, darts)"),
            ("FM_Fists", "fists"),
            ("FM_Stun", "objects (chairs, bottles)"),
            ("FM_BazookAlt, FM_ARAlt, FM_ShotGunAlt, FM_HarpoonAlt, FM_44Alt", "alternate fire"),
        ],
    );
    words(
        &mut out,
        "Console commands",
        "Press F2 in the game, type, Enter.",
        &[("modwho", "names the character you look at, with an entry to paste"), ("modpos", "your position and facing, for `position` and `yaw`"), ("modkey <key>", "does what the keyboard shortcut of that key does")],
    );
    out.push_str("## Settings of the simple blocks\n\n");
    out.push_str("Settings of a character: `look`, `skin`, `health`, `name`, `voice`, `weapon`, `team`,\n");
    out.push_str("`behaviour`, `can_die`. Added actors: `class`, `model`, `character`, `position`, `yaw`, `rotation`,\n");
    out.push_str("`scale`, `collision`, `properties`, `from_checkpoint`. Changes: `who`, `near`, `radius`.\n");
    out.push_str("Dialogues: `by`, `line`, `text`, `sound`, `when`, `distance`, `event`, `delay`, `once`.\n");
    out.push_str("Reactions: `event`, `who`, `distance`, `position`, `radius`, `name`, `key`, `seconds`, `once`, `then`,\n");
    out.push_str("`run`, `from_checkpoint`; in `then`: `by`, `to`, `at`, `look_at`, `run`, `loop`, `yaw`, `team`.\n");
    out
}

/// docs/modding/names/weapons.md: the weapons' simple names.
pub fn weapons_page() -> String {
    let mut out = String::from("# Weapons

");
    out.push_str(GENERATED_STATIC);
    out.push_str("

");
    out.push_str(&weapons_section());
    out.push_str("
Every weapon class of the game, its ammo and its pickup are listed on your PC in
");
    out.push_str("`Mods/_Guide/names/weapons-and-ammo.md` and in the names catalog (see [README.md](README.md)).
");
    out
}

/// The lists of names: (page, what it lists, the settings that use it, a tiny example, made on
/// the player's PC from the game's files).
pub const PAGES: &[(&str, &str, &str, &str, bool)] = &[
    ("characters.md", "Characters (simple names) and voices", "`character`, `look`, `spawn`, `voice`", "character = \"Jones\"", false),
    ("weapons.md", "Weapons (simple names)", "`weapon`, `weapons`", "weapon = \"Shotgun\"", false),
    ("words.md", "Fixed words: team, behaviour, when, events and actions of reactions, map selectors, key names, fire modes, commands", "many settings", "team = \"enemy\"", false),
    ("weapons-and-ammo.md", "Every weapon class, its ammo and its pickup", "`give`, `[class.\"…\"]`", "give = \"XIII.m16\"", true),
    ("objects.md", "Pickups and objects a mod can place", "`class` of `[[map.<Map>.add]]`", "class = \"XIII.MedPick\"", true),
    ("decorations.md", "Solid models (static meshes)", "`model` of `[[map.<Map>.add]]`", "model = \"Bateau_pack.BAT_caisse01\"", true),
    ("maps.md", "Maps, checkpoints, and the characters each map places", "`[map.<Map>…]`, `from_checkpoint`, `who`, `by`", "who = \"gardien_lostpass\"", true),
    ("events.md", "Events each map triggers", "`event` of `[[map.<Map>.say]]`", "event = \"Objectif91\"", true),
    ("dialogues.md", "Dialogue lines (name, speaker, level)", "`line` of `[[map.<Map>.say]]`", "line = \"Plage01_Pam_00\"", true),
    ("sounds.md", "Sound effects, by group", "`sound`, `[map.<Map>.replace_sounds]`", "sound = \"XIIISound.Doors.Open\"", true),
    ("ambient-sounds.md", "The sounds each map plays as ambience", "`[map.<Map>.replace_sounds]`", "\"XIIIsound.Ambient.Generator1\" = \"Mod.MyHum\"", true),
    ("music.md", "Music of the levels", "`[map.<Map>.replace_sounds]`, `sound`", "\"XIIISound.Music.Banque01\"", true),
    ("animations.md", "Animations of characters and weapons", "`animation` of `[character.<Name>.state.<State>]`", "animation = \"WaitNeutre\"", true),
    ("scripts.md", "Events and actions of when blocks, events and functions of scripts", "`[[map.<Map>.when]]`, the `scripts` folder", "{ give = \"Shotgun\" }", true),
];

/// docs/modding/names/README.md: one line per list.
pub fn index_page() -> String {
    let mut out = String::from("# All the names a mod can use

");
    out.push_str("Every name a mod writes comes from one of these lists. Open a list, search it with Ctrl+F,
");
    out.push_str("copy the name from the first column and paste it into your `.toml` file.

");
    out.push_str("**Here:** the lists OpenXIII defines itself.

");
    out.push_str("| List | What it lists | Used by the settings | Example |
|---|---|---|---|
");
    for (file, what, settings, example, _) in PAGES.iter().filter(|p| !p.4) {
        out.push_str(&line(&[&format!("[{file}]({file})"), what, settings, &format!("`{example}`")]));
    }
    out.push_str("
**On your PC:** the lists made from your own copy of XIII. The game writes them by itself
");
    out.push_str("into the `_Guide` folder of your `Mods` folder (when it starts, if they are missing or
");
    out.push_str("out of date; delete them to have them written again): the names
");
    out.push_str("catalog `Modding catalog.html` (one page with a search box) and these lists in
");
    out.push_str("`Mods/_Guide/names/`. See [find-names.md](../find-names.md).

");
    out.push_str("| List (created on your PC) | What it lists | Used by the settings | Example |
|---|---|---|---|
");
    for (file, what, settings, example, _) in PAGES.iter().filter(|p| p.4) {
        out.push_str(&line(&[&format!("`Mods/_Guide/names/{file}`"), what, settings, &format!("`{example}`")]));
    }
    out.push_str("
The lists made on your PC hold identifiers only (no dialogue text, no subtitle); they are
");
    out.push_str("not part of the repository.
");
    out
}

/// The differences between a committed page and the expected text: None when they match
/// (line endings ignored).
pub fn page_differs(committed: &str, expected: &str) -> Option<String> {
    let a = committed.replace("\r\n", "\n");
    let b = expected.replace("\r\n", "\n");
    if a == b {
        return None;
    }
    let first = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or_else(|| a.lines().count().min(b.lines().count()));
    Some(format!("first difference at line {}: {:?} instead of {:?}", first + 1, a.lines().nth(first).unwrap_or(""), b.lines().nth(first).unwrap_or("")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_look_has_a_description() {
        for (model, _) in CHARACTER_LOOKS {
            assert!(LOOK_INFO.iter().any(|(m, w, _)| m == model && !w.is_empty()), "{model} has no description in LOOK_INFO");
        }
        for (model, _, _) in LOOK_INFO {
            assert!(CHARACTER_LOOKS.iter().any(|(m, _)| m == model), "{model} of LOOK_INFO is not a look");
        }
        let page = characters_page();
        assert!(page.contains("| Jones | MajorJones, JonesM | Major Jones | Banque01, SMarin01 | XIIIPersos.JonesM |"), "{page}");
        assert!(weapons_section().contains("| Shotgun | PumpGun, FusilPompe | XIII.FusilPompe |"));
        assert!(index_page().contains("`Mods/_Guide/names/maps.md`") && index_page().contains("[characters.md](characters.md)"));
        // page_differs: equal pages, and a changed one (negative control).
        assert!(page_differs(&page.replace('\n', "\r\n"), &page).is_none());
        assert!(page_differs(&page.replace("Major Jones |", "Major Jonez |"), &page).is_some());
    }
}
