//! Improved build only: the co-op characters of each save slot.
//!
//! A slot of the profile (the game's own file, unchanged) does not know who played. When a co-op
//! game writes a slot, each player's character (and look) goes into a small file beside the
//! profile, one per slot, as the mods' scripts memory does (mod_script_save.rs); a slot written
//! outside co-op, or erased, loses its file. When Continue or Load starts a co-op game from the
//! Co-op page, the slot's characters are taken again and the character screen is skipped
//! (character_page.rs, `character_screen_travel`). A slot without the file (written before, or
//! by the original game) falls back to the characters last chosen in co-op, when there are any;
//! otherwise the character screen is shown as for a new game.

use std::path::{Path, PathBuf};

use crate::character_screen::Choice;
use crate::Vm;

/// The file beside profile `profile` keeping slot `slot`'s co-op characters.
pub fn side_file(profile: &Path, slot: i32) -> PathBuf {
    let name = profile.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    profile.with_file_name(format!("{name}.coop-slot{slot}.ini"))
}

/// The file's text for these characters.
pub fn side_text(choices: [Choice; 2]) -> String {
    format!("[CoopCharacters]\r\nPlayer1={}\r\nPlayer2={}\r\n", choices[0].code(), choices[1].code())
}

/// The characters of a side file's text (None: not a co-op slot's file, or a character unknown
/// to this build: the screen is shown).
pub fn read_side_text(text: &str) -> Option<[Choice; 2]> {
    let mut out = [None, None];
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let i = match k.trim().to_ascii_lowercase().as_str() {
            "player1" => 0,
            "player2" => 1,
            _ => continue,
        };
        out[i] = Some(Choice::from_code(v.trim())?);
    }
    Some([out[0]?, out[1]?])
}

impl Vm {
    /// OperationWriteSlot wrote slot `slot` of profile `profile`: its co-op characters beside it
    /// (a game not in co-op, or whose characters were not chosen: the file is removed).
    pub(crate) fn coop_save_write_slot(&mut self, profile: &str, slot: i32) {
        let file = side_file(Path::new(profile), slot);
        let lp = self.local_play();
        let codes = lp.coop.and(lp.characters);
        let Some(codes) = codes else {
            self.session.files.remove(&file);
            return;
        };
        let choices = codes.map(|(c, v)| Choice { character: c as usize, variant: v as usize });
        match self.session.files.write(&file, side_text(choices).as_bytes()) {
            true => self.log.push(format!("Log: Co-op save: slot {slot} keeps {}, {}", choices[0].code(), choices[1].code())),
            false => self.log.push(format!("Log: Co-op save: {}: not written", file.display())),
        }
    }

    /// OperationEraseSlot erased slot `slot`: its file goes too.
    pub(crate) fn coop_save_erase_slot(&mut self, profile: &str, slot: i32) {
        self.session.files.remove(&side_file(Path::new(profile), slot));
    }

    /// OperationReadSlot read slot `slot` (Continue, Load): its co-op characters, if any, for
    /// the travel that follows.
    pub(crate) fn coop_save_read_slot(&mut self, profile: &str, slot: i32) {
        let file = side_file(Path::new(profile), slot);
        let found = self.session.files.read(&file).and_then(|b| String::from_utf8(b).ok()).and_then(|t| read_side_text(&t));
        self.improved.character_screen.slot_characters = Some(found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_file_is_beside_the_profile_per_slot() {
        let p = Path::new("save").join("Player.pro");
        assert_eq!(side_file(&p, 3), Path::new("save").join("Player.pro.coop-slot3.ini"));
    }

    #[test]
    fn characters_read_back() {
        // Every look of every character comes back as it was written.
        let all: Vec<Choice> = crate::character_screen::ROSTER
            .iter()
            .enumerate()
            .flat_map(|(character, r)| (0..r.variants.len()).map(move |variant| Choice { character, variant }))
            .collect();
        for (a, b) in all.iter().zip(all.iter().rev()) {
            let c = [*a, *b];
            assert_eq!(read_side_text(&side_text(c)), Some(c));
        }
        // One player missing or unknown: no characters (the screen is shown).
        assert_eq!(read_side_text("[CoopCharacters]\r\nPlayer1=XIII\r\n"), None);
        assert_eq!(read_side_text("Player1=XIII\nPlayer2=NoSuchOne\n"), None);
        assert_eq!(read_side_text(""), None);
    }

    /// A co-op slot written, then Continue from the Co-op page: the travel goes straight to the
    /// level with the slot's characters. A slot without its file (an older save), and no
    /// characters ever chosen: the travel is held for the screen.
    #[test]
    fn continue_skips_the_screen_with_the_saved_characters() {
        use crate::local_play::{CoopMode, CoopSession, CoopStart};
        let dir = std::env::temp_dir().join(format!("oxiii-coop-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let profile = dir.join("Player.pro").to_string_lossy().into_owned();
        let jones = Choice::from_code("Jones").expect("Jones");
        let kim = Choice::from_code("Kim").expect("Kim");
        let start = |start| CoopSession { mode: CoopMode::SplitScreen, start: Some(start) };

        // Saved during a co-op game with Jones and Kim.
        let mut vm = Vm::default();
        vm.change_local_play(|p| {
            p.coop = Some(start(CoopStart::NewGame));
            p.local_players = 2;
        });
        // Chosen on the screen once the session is on (a new session clears them).
        vm.change_local_play(|p| p.characters = Some([jones, kim].map(|c| (c.character as u8, c.variant as u8))));
        vm.coop_save_write_slot(&profile, 4);
        assert!(side_file(Path::new(&profile), 4).exists());

        // A new run: Continue reads slot 4 and travels to the level.
        let mut vm = Vm::default();
        vm.change_local_play(|p| {
            p.coop = Some(start(CoopStart::Continue));
            p.local_players = 2;
        });
        vm.coop_save_read_slot(&profile, 4);
        let req = vm.browse_request_pub("Hual01a", crate::travel::TRAVEL_ABSOLUTE, Vec::new());
        let out = vm.character_screen_travel(Some(req)).expect("a travel");
        assert!(out.url.map.eq_ignore_ascii_case("Hual01a"), "straight to the level, not the screen");
        assert!(vm.improved.character_screen.held.is_none());
        assert_eq!(vm.local_play().characters, Some([jones, kim].map(|c| (c.character as u8, c.variant as u8))));

        // The slot written again outside co-op: its file goes.
        let mut solo = Vm::default();
        solo.coop_save_write_slot(&profile, 4);
        assert!(!side_file(Path::new(&profile), 4).exists());

        // An older save (no file), nothing remembered: the screen is shown.
        let mut vm = Vm::default();
        vm.change_local_play(|p| {
            p.coop = Some(start(CoopStart::Load));
            p.local_players = 2;
        });
        vm.coop_save_read_slot(&profile, 4);
        let req = vm.browse_request_pub("Hual01a", crate::travel::TRAVEL_ABSOLUTE, Vec::new());
        let out = vm.character_screen_travel(Some(req)).expect("a travel");
        assert!(!out.url.map.eq_ignore_ascii_case("Hual01a"), "the character screen's map");
        assert!(vm.improved.character_screen.held.is_some());
        assert_eq!(vm.local_play().characters, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
