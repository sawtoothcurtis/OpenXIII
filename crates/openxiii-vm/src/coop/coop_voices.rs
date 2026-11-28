//! Improved build with mods only: a co-op player who plays another character than XIII
//! shouts with that character's voice.
//!
//! The player's pawn keeps XIII's sound events (hXIIIPaf when hit, hXIIIJump, the footsteps'
//! landing and end of run, LadderClimb): their noises (impacts, heartbeat, medikit, jump
//! effort) stay as they are. Only the waves of XIII's voice are swapped, for that pawn only
//! (`Engine::wave_swaps`):
//! - the hit grunts become the character's own hurt shouts, drawn as the game draws an
//!   enemy's (the `hPNJHurt` table of Onomatopees for the character's voice, CodeMesh, and
//!   timbre), and the character's death shout (`hPNJDeath1`) when the hit kills;
//! - the voice sounds no other character has (jump, landing, end of run, ladder, panting)
//!   are silent, unless a mod brings its own sound for them: `[sounds.Coop_<Character>_<Kind>…]`
//!   (Kind: Hurt, Death, Jump, Land, RunEnd, Ladder, Breath; Character: the roster id, e.g.
//!   Jones, or a variant's code, e.g. Killer.3), which also replaces the game's hurt and death
//!   shouts when given.
//! - Kim and Amos have shouts in the game's files that no table uses (they stay silent in the
//!   game): those are used.
//!
//! XIII himself, and the solo game, are untouched.

use std::rc::Rc;

use openxiii_assets::hx::Cuuid;
use openxiii_hx::engine::{ResData, WaveSwaps};

use crate::character_screen::{Choice, ROSTER};
use crate::{ObjId, Vm};

/// XIII's voice waves, by kind (the waves of XIIIPerso that have one recording per language).
const XIII_VOICE: &[(&str, &[&str])] = &[
    ("Hurt", &["OnoPafBull1", "OnoPafBull2", "OnoPafBull3", "OnoPafBull4", "OnoPafBull5", "OnoHurt_XIII_01", "OnoHurt_XIII_03", "OnoHurt_XIII_04"]),
    ("Jump", &["OnoJump0", "OnoJump1", "OnoJump2"]),
    ("Land", &["OnoLand0", "OnoLand1", "OnoLand2"]),
    ("RunEnd", &["OnoRunEnd1", "OnoRunEnd2", "OnoRunEnd3"]),
    ("Ladder", &["OnoLadder1", "OnoLadder2"]),
    ("Breath", &["Haletement1"]),
];

/// Characters whose voice (CodeMesh) has no entry in the shout tables: their unused shouts.
const UNUSED_SHOUTS: &[(&str, &[&str], &[&str])] = &[
    ("Kim", &["XIIISound.PNJ.Hurt_Kim_00", "XIIISound.PNJ.Hurt_Kim_01"], &["XIIISound.PNJ.Dth1_Kim_00"]),
    ("Amos", &["XIIISound.PNJ.Hurt_Amos_00", "XIIISound.PNJ.Hurt_Amos_01"], &["XIIISound.PNJ.Dth1_Amos_00"]),
];

/// The voice of a character's model (CodeMesh), as the game's maps give it.
pub fn character_voice(choice: Choice) -> Option<i32> {
    openxiii_mods::simple::look_voice(choice.variant_data()?.mesh)
}

impl Vm {
    /// Once per level tick for each co-op player's pawn (`coop_looks_frame`): the pawn of a
    /// player who is not XIII gets its character's voice, once per pawn.
    pub(crate) fn coop_voice(&mut self, pawn: ObjId, choice: Choice) {
        // XIII, in every outfit, keeps his own voice.
        if ROSTER.get(choice.character).is_none_or(|c| c.id == "XIII") {
            return;
        }
        if self.audio.is_none() {
            return;
        }
        // One of the game's three timbres for the soldiers, the same on every machine.
        let timbre = self.prop(pawn, "PlayerReplicationInfo").as_object().map_or(0, |r| self.prop(r, "PlayerID").as_int()).rem_euclid(3);
        let tag = format!("{} {timbre}", choice.code());
        let alive = self.prop(pawn, "Health").as_int() > 0;
        if let Some(s) = self.audio.as_ref().and_then(|hx| hx.actor_wave_swaps.get(&(pawn as u32))) {
            if s.tag == tag {
                // A living pawn shouts its hurt shouts again: a pawn that respawned in the
                // same object, after a death that made its shouts death shouts.
                if alive && s.dead.get() {
                    s.dead.set(false);
                }
                return;
            }
        }
        let mut swaps = self.coop_voice_swaps(choice, timbre);
        swaps.tag = tag;
        swaps.hit_sound = match self.prop(pawn, "hHitSound") {
            crate::value::Value::Asset(p) => self.audio_sound_ref(&p).map(|s| s.cuuid),
            _ => None,
        };
        let summary =
            format!("{} hurt, {} death", swaps.waves.values().map(Vec::len).max().unwrap_or(0), swaps.death.values().map(Vec::len).max().unwrap_or(0));
        if let Some(hx) = self.audio.as_mut() {
            hx.actor_wave_swaps.insert(pawn as u32, Rc::new(swaps));
        }
        self.log.push(format!("Log: Co-op: {} speaks as {} ({summary})", self.obj_label(pawn), choice.code()));
    }

    /// The swap table of `choice`'s voice (timbre `timbre` of three).
    fn coop_voice_swaps(&mut self, choice: Choice, timbre: i32) -> WaveSwaps {
        let mut hurt = self.coop_own_voice(choice, "Hurt");
        let mut death = self.coop_own_voice(choice, "Death");
        let id = ROSTER.get(choice.character).map_or("", |c| c.id);
        if let Some(&(_, h, d)) = UNUSED_SHOUTS.iter().find(|(c, _, _)| *c == id) {
            if hurt.is_empty() {
                hurt = self.coop_game_sounds(h.iter().map(|s| s.to_string()).collect());
            }
            if death.is_empty() {
                death = self.coop_game_sounds(d.iter().map(|s| s.to_string()).collect());
            }
        } else if let Some(code_mesh) = character_voice(choice) {
            if hurt.is_empty() {
                hurt = self.coop_table_shouts("Onomatopees.hPNJHurt", code_mesh, timbre);
            }
            if death.is_empty() {
                death = self.coop_table_shouts("Onomatopees.hPNJDeath1", code_mesh, timbre);
            }
        }
        // A character with hurt shouts but no death shout (or the reverse) uses the other one.
        if death.is_empty() {
            death = hurt.clone();
        }
        if hurt.is_empty() {
            hurt = death.clone();
        }
        let mut swaps = WaveSwaps::default();
        for &(kind, names) in XIII_VOICE {
            let to = if kind == "Hurt" { hurt.clone() } else { self.coop_own_voice(choice, kind) };
            for from in self.coop_waves_named(names) {
                if kind == "Hurt" {
                    swaps.death.insert(from, death.clone());
                }
                swaps.waves.insert(from, to.clone());
            }
        }
        swaps
    }

    /// Tests: how many hurt and death shouts `choice` gets (timbre `timbre`).
    #[cfg(test)]
    pub(crate) fn coop_voice_swaps_for_test(&mut self, choice: Choice, timbre: i32) -> (usize, usize) {
        let s = self.coop_voice_swaps(choice, timbre);
        (s.waves.values().map(Vec::len).max().unwrap_or(0), s.death.values().map(Vec::len).max().unwrap_or(0))
    }

    /// Tests: the game's hurt and death shouts `choice` gets (timbre `timbre`), by path.
    #[cfg(test)]
    pub(crate) fn coop_voice_paths_for_test(&mut self, choice: Choice, timbre: i32) -> (Vec<String>, Vec<String>) {
        let id = ROSTER.get(choice.character).map_or("", |c| c.id);
        if let Some(&(_, h, d)) = UNUSED_SHOUTS.iter().find(|(c, _, _)| *c == id) {
            return (h.iter().map(|s| s.to_string()).collect(), d.iter().map(|s| s.to_string()).collect());
        }
        let Some(code_mesh) = character_voice(choice) else { return (Vec::new(), Vec::new()) };
        (self.coop_table_paths("Onomatopees.hPNJHurt", code_mesh, timbre), self.coop_table_paths("Onomatopees.hPNJDeath1", code_mesh, timbre))
    }

    /// Wave resources of the loaded banks with one of these names (XIII's voice).
    fn coop_waves_named(&self, names: &[&str]) -> Vec<Cuuid> {
        let Some(hx) = self.audio.as_ref() else { return Vec::new() };
        let mut out: Vec<Cuuid> = hx
            .engine
            .registry
            .iter()
            .filter_map(|(id, r)| match r {
                ResData::Wav(w) if names.iter().any(|n| w.name.eq_ignore_ascii_case(n)) => Some(*id),
                _ => None,
            })
            .collect();
        out.sort();
        out
    }

    /// The shouts of voice `code_mesh`, timbre `timbre`, in shout table `table` (the timbre's
    /// four places, or the first timbre's when that one is empty), made playable.
    fn coop_table_shouts(&mut self, table: &str, code_mesh: i32, timbre: i32) -> Vec<Cuuid> {
        let paths = self.coop_table_paths(table, code_mesh, timbre);
        self.coop_game_sounds(paths)
    }

    /// The paths of the shouts of `coop_table_shouts`.
    fn coop_table_paths(&mut self, table: &str, code_mesh: i32, timbre: i32) -> Vec<String> {
        let Some(set) = self.audio_ono(table) else { return Vec::new() };
        let Some(t) = set.timbres.get(code_mesh.max(0) as usize) else { return Vec::new() };
        let group = |g: usize| t[g * 4..g * 4 + 4].iter().flatten().map(|s| s.path.clone()).collect::<Vec<String>>();
        let mut paths = group(timbre.clamp(0, 2) as usize);
        if paths.is_empty() {
            paths = group(0);
        }
        paths
    }

    /// Game sounds by path, their banks loaded if this level lacks them; their resources.
    fn coop_game_sounds(&mut self, paths: Vec<String>) -> Vec<Cuuid> {
        let mut out = Vec::new();
        for p in paths {
            if let Err(why) = self.mod_ensure_sound(&p, "co-op voice") {
                self.log.push(format!("Log: Co-op voice: {why}"));
                continue;
            }
            if let Some(s) = self.audio_sound_ref(&p) {
                if !out.contains(&s.cuuid) {
                    out.push(s.cuuid);
                }
            }
        }
        out
    }

    /// A mod's own sounds for `choice`'s `kind` (`Coop_<code or id>_<kind>…`), registered on
    /// this level.
    fn coop_own_voice(&self, choice: Choice, kind: &str) -> Vec<Cuuid> {
        let Some(own) = self.mods.own_sounds.as_ref() else { return Vec::new() };
        let id = ROSTER.get(choice.character).map_or("XIII", |c| c.id);
        let registered = |r: Cuuid| self.audio.as_ref().is_some_and(|h| h.engine.registry.contains_key(&r));
        for who in [choice.code(), id.to_string()] {
            let prefix = format!("Coop_{who}_{kind}").to_ascii_lowercase();
            let found: Vec<Cuuid> = own
                .iter()
                .filter(|o| {
                    let n = o.def.name.to_ascii_lowercase();
                    // "Coop_Jones_Jump", "Coop_Jones_Jump2", not "Coop_Jones_JumpBig".
                    n.strip_prefix(&prefix).is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit() || c == '_'))
                })
                .map(|o| o.resource)
                .filter(|&r| registered(r))
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_roster_character_but_xiii_has_a_voice_or_unused_shouts() {
        for (c, ch) in ROSTER.iter().enumerate().skip(1) {
            for v in 0..ch.variants.len() {
                let choice = Choice { character: c, variant: v };
                let unused = UNUSED_SHOUTS.iter().any(|(id, _, _)| *id == ch.id);
                assert!(unused || character_voice(choice).is_some(), "{}", choice.code());
            }
        }
    }

    #[test]
    fn soldiers_have_their_soldiers_voice() {
        // The voice family the game's shout files are named after: SPADS 9, GI 5, FBI 3,
        // conspirators 6, guards 8, killers 11 to 14.
        for (id, voices) in [("Spads", &[9][..]), ("GI", &[5]), ("FBI", &[3]), ("Conspirator", &[6]), ("Policeman", &[8]), ("Killer", &[11, 12, 13, 14])] {
            let c = ROSTER.iter().position(|r| r.id == id).unwrap();
            for v in 0..ROSTER[c].variants.len() {
                let voice = character_voice(Choice { character: c, variant: v }).unwrap();
                assert!(voices.contains(&voice), "{id}.{v}: voice {voice}");
            }
        }
    }

    #[test]
    fn kim_and_amos_voices_are_the_silent_ones() {
        // Their CodeMesh (20, 15) has no entry in the game's shout tables.
        let kim = ROSTER.iter().position(|c| c.id == "Kim").unwrap();
        let amos = ROSTER.iter().position(|c| c.id == "Amos").unwrap();
        assert_eq!(character_voice(Choice { character: kim, variant: 0 }), Some(20));
        assert_eq!(character_voice(Choice { character: amos, variant: 0 }), Some(15));
    }
}
