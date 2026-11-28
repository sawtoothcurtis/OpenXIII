//! Lines said by characters: `[[map.<Map>.say]]`.
//!
//! ```toml
//! [[map.Plage01.say]]
//! by = "Pam"                        # who speaks: a [character] of the mods added to this map,
//!                                   # or a character of the map (its name, Tag or PawnName)
//! line = "Hual02_Carrington_04"     # a line of the game, by its name (dialogues.md): its voice
//!                                   # and its subtitle, in the player's language
//! # or a line of your own:
//! # text = "Nice day for a swim."   # the subtitle
//! # sound = "Mod.hello"             # its voice (optional): a sound of the mod or of the game
//! when = "near"                     # near (default): the player comes within `distance`
//!                                   # start: when the level starts
//!                                   # event: when the map triggers `event`
//! distance = 300                    # for when = "near" (300 by default)
//! # event = "Kim_surprise"          # for when = "event": a Tag of the map
//! delay = 0.5                       # seconds between the trigger and the line (optional)
//! once = true                       # false: said again each time it is triggered (near: each
//!                                   # time the player comes back)
//! ```
//!
//! Lines triggered together are said one after the other, in the order of the files, as the
//! game's own dialogues never speak over each other.
//!
//! This module only reads the files; the game's VM says the lines (openxiii-vm, feature `mods`).
//! It also reads the dialogues of the game's localization files (`<Map>.int` and the other
//! languages), for the names of the game's lines and their subtitles.

/// Default `distance` of a line said when the player comes near.
pub const DEFAULT_DISTANCE: f32 = 300.0;

/// When a line is said.
#[derive(Debug, Clone, PartialEq)]
pub enum When {
    /// When the player comes within `distance` of the speaker.
    Near { distance: f32 },
    /// When the level starts.
    Start,
    /// When the map triggers this event (TriggerEvent: the actors whose Tag is this name).
    Event { name: String },
}

/// A `[[map.<Map>.say]]` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Say {
    /// Who speaks: a mod character's name, or a map actor's name, Tag or PawnName.
    pub by: String,
    /// A line of the game by its name ("Hual02_Carrington_04").
    pub line: Option<String>,
    /// Subtitle of the mod's own line (or in place of the game line's own subtitle).
    pub text: Option<String>,
    /// Voice of the mod's own line: "Mod.<Name>" or a game sound "Package.Group.Name".
    pub sound: Option<String>,
    pub when: When,
    pub delay: f32,
    pub once: bool,
}

impl Say {
    /// Short description for log lines.
    pub fn describe(&self) -> String {
        let what = match (&self.line, &self.text, &self.sound) {
            (Some(l), _, _) => l.clone(),
            (None, _, Some(s)) => s.clone(),
            (None, Some(t), None) => format!("\"{t}\""),
            _ => String::new(),
        };
        format!("say {what} by {}", self.by)
    }
}

/// Reads one `[[map.<Map>.say]]` entry.
pub(crate) fn read(t: &toml::Table) -> Result<Say, String> {
    const KEYS: [&str; 9] = ["by", "line", "text", "sound", "when", "distance", "event", "delay", "once"];
    if let Some(k) = t.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key {k} (keys: {})", KEYS.join(", ")));
    }
    let string = |key: &str| -> Result<Option<String>, String> {
        match t.get(key) {
            None => Ok(None),
            Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
            Some(v) => Err(format!("{key} = {v} must be a text")),
        }
    };
    let number = |key: &str| -> Result<Option<f32>, String> {
        match t.get(key) {
            None => Ok(None),
            Some(toml::Value::Integer(i)) if *i >= 0 => Ok(Some(*i as f32)),
            Some(toml::Value::Float(f)) if f.is_finite() && *f >= 0.0 => Ok(Some(*f as f32)),
            Some(v) => Err(format!("{key} = {v} must be a number, 0 or more")),
        }
    };
    let by = string("by")?.ok_or("by = \"<who speaks>\" is missing (a [character] of the mods, or a character of the map)")?;
    let line = string("line")?.map(|l| match l.split_once('.') {
        // "Hual02Voices.Hual02_Carrington_04": the sound's package is not part of the name.
        Some((_, name)) if !name.is_empty() => name.to_string(),
        _ => l,
    });
    if let Some(l) = &line {
        if parse_line_name(l).is_none() {
            return Err(format!("line = \"{l}\" is not the name of a line of the game, such as \"Hual02_Carrington_04\" (see dialogues.md)"));
        }
    }
    let text = string("text")?;
    let sound = string("sound")?;
    if line.is_some() && sound.is_some() {
        return Err("use either line (a line of the game, with its voice) or sound (your own voice), not both".into());
    }
    if line.is_none() && text.is_none() && sound.is_none() {
        return Err("write line = \"<a line of the game>\", or text = \"<subtitle>\" (and sound = \"Mod.<Name>\")".into());
    }
    let distance = number("distance")?;
    let event = string("event")?;
    let when = match string("when")?.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("near") => {
            if event.is_some() {
                return Err("event = ... goes with when = \"event\"".into());
            }
            When::Near { distance: distance.filter(|d| *d > 0.0).unwrap_or(DEFAULT_DISTANCE) }
        }
        Some(w) if distance.is_some() => return Err(format!("distance goes with when = \"near\" (not \"{w}\")")),
        Some("start") if event.is_none() => When::Start,
        Some("event") => When::Event { name: event.ok_or("when = \"event\" needs event = \"<Tag of the map>\"")? },
        Some(w) if event.is_some() => return Err(format!("event = ... goes with when = \"event\" (not \"{w}\")")),
        Some(w) => return Err(format!("when = \"{w}\" must be near, start or event")),
    };
    let once = match t.get("once") {
        None => true,
        Some(toml::Value::Boolean(b)) => *b,
        Some(v) => return Err(format!("once = {v} must be true or false")),
    };
    Ok(Say { by, line, text, sound, when, delay: number("delay")?.unwrap_or(0.0), once })
}

/// Parts of the name of a line of the game, "<Level>_<Speaker>_<NN>" (DialogueManager.Speak:
/// Level.Title, "_", the speaker's PawnName, "_", the sentence index on two digits):
/// (level, speaker, sentence index).
pub fn parse_line_name(name: &str) -> Option<(&str, &str, usize)> {
    let (level, rest) = name.split_once('_')?;
    let (speaker, index) = rest.rsplit_once('_')?;
    if level.is_empty() || speaker.is_empty() || index.len() < 2 || !index.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((level, speaker, index.parse().ok()?))
}

/// Name of a line: DialogueManager.Speak (xidcine): the speaker's PawnName without its "HF" or
/// "NORM" prefix (radio and normal voices with a portrait), the index on two digits.
pub fn line_name(level: &str, pawn_name: &str, index: usize) -> String {
    format!("{level}_{}_{index:02}", speaker_of(pawn_name))
}

/// The speaker part of a line's name for a DialogueManager speaker's PawnName.
pub fn speaker_of(pawn_name: &str) -> &str {
    let lower = pawn_name.to_ascii_lowercase();
    if lower.starts_with("norm") {
        &pawn_name[4..]
    } else if lower.starts_with("hf") {
        &pawn_name[2..]
    } else {
        pawn_name
    }
}

/// A line of the game found in a localization file.
#[derive(Debug, Clone, PartialEq)]
pub struct GameLine {
    /// "Hual02_Carrington_04" (the level as its LevelInfo0 Title gives it).
    pub name: String,
    /// Its section, the DialogueManager that says it ("DialogueManager5").
    pub section: String,
    pub text: String,
}

/// The lines of a map's localization file (`hual02.int`): the non-empty sentences of the
/// `Speakers` of each DialogueManager section, named as DialogueManager.Speak names their
/// voices. A line given by two sections keeps the first one's text.
pub fn game_lines(file_text: &str) -> Vec<GameLine> {
    let mut title = String::new();
    let mut section = String::new();
    let mut out: Vec<GameLine> = Vec::new();
    let mut speakers_of: Vec<(String, String)> = Vec::new();
    for raw in file_text.lines() {
        let line = raw.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if section.eq_ignore_ascii_case("LevelInfo0") && key.trim().eq_ignore_ascii_case("Title") {
            title = value.trim().to_string();
        } else if key.trim().eq_ignore_ascii_case("Speakers") {
            // Some files quote the whole value: Speakers="((PawnName=...))".
            let v = value.trim();
            let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).filter(|v| v.starts_with('(')).unwrap_or(v);
            speakers_of.push((section.clone(), v.to_string()));
        }
    }
    if title.is_empty() {
        return out;
    }
    for (section, value) in speakers_of {
        for (pawn_name, sentences) in parse_speakers(&value) {
            for (i, text) in sentences.into_iter().enumerate() {
                if text.trim().is_empty() || i >= 100 {
                    continue;
                }
                let name = line_name(&title, &pawn_name, i);
                if !out.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
                    out.push(GameLine { name, section: section.clone(), text });
                }
            }
        }
    }
    out
}

/// The subtitle of line `name` in a map's localization file, if it has one.
pub fn game_line_text(file_text: &str, name: &str) -> Option<String> {
    game_lines(file_text).into_iter().find(|l| l.name.eq_ignore_ascii_case(name)).map(|l| l.text)
}

/// `Speakers=((PawnName="A",Sentences=("x","y")),(PawnName="B",...))`: (PawnName, Sentences)
/// of each speaker, in order.
pub fn parse_speakers(value: &str) -> Vec<(String, Vec<String>)> {
    let chars: Vec<char> = value.chars().collect();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut i = 0;
    // A quoted text starting at `i` (on the quote): its content and the position after it.
    let quoted = |mut i: usize| -> (String, usize) {
        let mut s = String::new();
        i += 1;
        while i < chars.len() && chars[i] != '"' {
            if chars[i] == '\\' && i + 1 < chars.len() {
                i += 1;
            }
            s.push(chars[i]);
            i += 1;
        }
        (s, i + 1)
    };
    let starts = |i: usize, word: &str| -> bool {
        let w: Vec<char> = word.chars().collect();
        i + w.len() <= chars.len() && chars[i..i + w.len()].iter().zip(&w).all(|(a, b)| a.eq_ignore_ascii_case(b))
    };
    while i < chars.len() {
        if chars[i] == '"' {
            i = quoted(i).1;
        } else if starts(i, "PawnName=") {
            i += "PawnName=".len();
            let name = if chars.get(i) == Some(&'"') {
                let (s, next) = quoted(i);
                i = next;
                s
            } else {
                let start = i;
                while i < chars.len() && !matches!(chars[i], ',' | ')') {
                    i += 1;
                }
                chars[start..i].iter().collect()
            };
            out.push((name, Vec::new()));
        } else if starts(i, "Sentences=(") {
            i += "Sentences=(".len();
            let mut sentences = Vec::new();
            while i < chars.len() && chars[i] != ')' {
                if chars[i] == '"' {
                    let (s, next) = quoted(i);
                    sentences.push(s);
                    i = next;
                } else if chars[i] == ',' && matches!(chars.get(i + 1), Some(',') | Some(')')) {
                    // An empty element written without quotes.
                    sentences.push(String::new());
                    i += 1;
                } else {
                    i += 1;
                }
            }
            if let Some(last) = out.last_mut() {
                last.1 = sentences;
            } else {
                out.push((String::new(), sentences));
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HUAL02: &str = "[LevelInfo0]\nTitle=hual02\n\n[DialogueManager2]\nSpeakers=((PawnName=\"Carrington\",Sentences=(\"Standwell ! I demand to be released immediately ! STANDWELL !\",\"Hey! You over there.\")),(PawnName=\"XIIIa\",Sentences=(\"\",\"I must question this man...\")))\n\n[DialogueManager5]\nSpeakers=((PawnName=\"Carrington\",Sentences=(\"\",\"\",\"\",\"\",\"Am i dreaming?\")),(PawnName=\"HFMcCall\",Sentences=(\"Over.\")))\n\n[DialogueManager9]\nSpeakers=\"((PawnName=\"HFHom\",Sentences=(\"Janet, it's Frank.\")))\"\n";

    #[test]
    fn lines_of_a_localization_file() {
        let lines = game_lines(HUAL02);
        let names: Vec<&str> = lines.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["hual02_Carrington_00", "hual02_Carrington_01", "hual02_XIIIa_01", "hual02_Carrington_04", "hual02_McCall_00", "hual02_Hom_00"]);
        assert_eq!(game_line_text(HUAL02, "Hual02_Carrington_04").as_deref(), Some("Am i dreaming?"));
        assert_eq!(game_line_text(HUAL02, "HUAL02_carrington_01").as_deref(), Some("Hey! You over there."));
        assert_eq!(lines[3].section, "DialogueManager5");
        assert_eq!(game_line_text(HUAL02, "Hual02_Carrington_02"), None);
    }

    #[test]
    fn line_names() {
        assert_eq!(parse_line_name("Hual02_Carrington_04"), Some(("Hual02", "Carrington", 4)));
        assert_eq!(parse_line_name("SSH101c_SPADS_1b_12"), Some(("SSH101c", "SPADS_1b", 12)));
        assert_eq!(parse_line_name("Hual02_Carrington"), None);
        assert_eq!(parse_line_name("Carrington_4"), None);
        assert_eq!(line_name("Plage01", "NORMPam", 3), "Plage01_Pam_03");
        assert_eq!(line_name("Plage01", "HFJones", 12), "Plage01_Jones_12");
    }

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn entries() {
        let s = read(&table("by = \"Pam\"\nline = \"Hual02Voices.Hual02_Carrington_04\"")).unwrap();
        assert_eq!(s.line.as_deref(), Some("Hual02_Carrington_04"));
        assert_eq!(s.when, When::Near { distance: DEFAULT_DISTANCE });
        assert!(s.once);
        let s = read(&table("by = \"Visitor\"\ntext = \"Hello, son.\"\nsound = \"Mod.hello\"\nwhen = \"event\"\nevent = \"Kim_surprise\"\ndelay = 1.5\nonce = false")).unwrap();
        assert_eq!(s.when, When::Event { name: "Kim_surprise".into() });
        assert_eq!((s.delay, s.once, s.sound.as_deref()), (1.5, false, Some("Mod.hello")));
        let s = read(&table("by = \"Visitor\"\ntext = \"Hi\"\nwhen = \"start\"")).unwrap();
        assert_eq!(s.when, When::Start);
        let s = read(&table("by = \"V\"\ntext = \"Hi\"\ndistance = 120")).unwrap();
        assert_eq!(s.when, When::Near { distance: 120.0 });
        for (bad, why) in [
            ("line = \"Hual02_Carrington_04\"", "by ="),
            ("by = \"V\"", "write line"),
            ("by = \"V\"\nline = \"Carrington\"", "not the name"),
            ("by = \"V\"\nline = \"Hual02_Carrington_04\"\nsound = \"Mod.a\"", "either line"),
            ("by = \"V\"\ntext = \"a\"\nwhen = \"later\"", "near, start or event"),
            ("by = \"V\"\ntext = \"a\"\nwhen = \"event\"", "needs event"),
            ("by = \"V\"\ntext = \"a\"\nwhen = \"start\"\ndistance = 5", "distance goes"),
            ("by = \"V\"\ntext = \"a\"\nevent = \"x\"", "event = ... goes"),
            ("by = \"V\"\ntext = \"a\"\nonce = 1", "true or false"),
            ("by = \"V\"\ntext = \"a\"\nsay = 1", "unknown key say"),
            ("by = \"V\"\ntext = \"a\"\ndelay = -1", "0 or more"),
        ] {
            let e = read(&table(bad)).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }
}
