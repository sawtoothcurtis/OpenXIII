//! Loading a map's `.hxc` banks into the engine.

use crate::engine::Engine;
use openxiii_assets::hx::HxBank;
use std::path::Path;

impl Engine {
    /// Loads `<game>/Sounds/PC/<name>.hxc` for each name; returns the names loaded.
    pub fn load_banks(&mut self, game: &Path, names: &[&str]) -> Vec<String> {
        self.load_banks_direct(game, names).0
    }

    /// Same loading; distinguishes direct creations from dependencies for
    /// LoadMapSounds' line tables.
    pub(crate) fn load_banks_direct(&mut self, game: &Path, names: &[&str]) -> (Vec<String>, std::collections::HashSet<openxiii_assets::hx::Cuuid>) {
        let dir = game.join("Sounds").join("PC");
        let mut loaded = Vec::new();
        let mut direct = std::collections::HashSet::new();
        for name in names {
            let path = dir.join(format!("{name}.hxc"));
            let Ok(data) = std::fs::read(&path) else {
                continue;
            };
            openxiii_package::events::file_read(&path, data.len());
            let Ok(bank) = HxBank::parse_with_path(&data, &path) else {
                continue;
            };
            let len = |w: &openxiii_assets::hx::Wave| openxiii_assets::audio::wave_length(&path, &data, w);
            let (res0, waves0): (std::collections::HashSet<_>, std::collections::HashSet<_>) =
                (self.registry.keys().copied().collect(), self.waves.keys().copied().collect());
            self.add_bank(&data, &bank, &len, Some(&path));
            // Objects created by the loader (LoadMapSounds: each entry that is not
            // a CPCWaveFileIdObj and not yet loaded, then its dependencies):
            // Object registry by CUUID (self.registry): singleton, add, lookup; references (bank objects stay until unloading);
            // CIdObj constructor.
            // Each CIdObj constructed draws a CUUID (one rand() of the game
            // thread) before receiving the one from the file. An entry that is not a
            // CPCWaveFileIdObj and not yet loaded is created (one draw), then its dependencies
            // without an object (counted references whose object is null) are created in
            // turn, up to ten passes: the wave of a CPCWavResData (direct link, or language
            // variant) is only created if neither an already loaded bank nor a previous entry
            // brought it. E.g. Hual02: 2 × 240 + 39 = 519 draws; Fix: 2 × 377 + 80 = 834; Toits01 and
            // Hual01b: one wave already in Fix (4b152152-45d3ee31), 517 draws for Toits01.
            // The chosen wave is the one used when playing the sound (WavRes::wave_for: the object's variants,
            // not those of the index); a dependency missing from the bank's index is not
            // created (loader returns null, no draw): defaultmulti cites 469
            // missing "fr" variants and keeps its "en" waves.
            let in_index: std::collections::HashSet<_> = bank.entries.iter().map(|e| e.cuuid).collect();
            let mut present: std::collections::HashSet<_> = res0.iter().chain(waves0.iter()).copied().collect();
            let mut order: Vec<_> = bank.entries.iter().filter(|e| e.class != "CPCWaveFileIdObj").map(|e| e.cuuid).collect();
            order.sort();
            for id in order {
                if !present.insert(id) {
                    continue;
                }
                direct.insert(id);
                let _ = self.cuuid_rand();
                // Global snapshot of the unresolved references, then
                // loading of that frontier only, at most ten times.
                // A wave revealed by the tenth pass waits for a later
                // direct load: the index skips CPCWaveFileIdObj entries.
                for _ in 0..10 {
                    let dependencies: std::collections::BTreeSet<_> = self.registry.iter()
                        .filter(|(id, _)| present.contains(id))
                        .flat_map(|(_, r)| match r {
                            crate::engine::ResData::Wav(w) => vec![w.wave_for(self.language)],
                            crate::engine::ResData::Program(p) => p.links.clone(),
                            crate::engine::ResData::Actor(_) => Vec::new(),
                        })
                        .filter(|dependency| !present.contains(dependency))
                        .collect();
                    if dependencies.is_empty() {
                        break;
                    }
                    for dependency in dependencies {
                        if in_index.contains(&dependency) && present.insert(dependency) {
                            let _ = self.cuuid_rand();
                        }
                    }
                }
            }
            // Only waves created as dependencies exist (variants in other languages
            // and waves nothing cites stay in the file).
            self.waves.retain(|k, _| waves0.contains(k) || present.contains(k));
            let mut res: Vec<_> = self.registry.keys().filter(|k| !res0.contains(k)).copied().collect();
            let mut waves: Vec<_> = self.waves.keys().filter(|k| !waves0.contains(k)).copied().collect();
            res.sort();
            waves.sort();
            self.banks.push((name.to_string(), res, waves));
            loaded.push(name.to_string());
        }
        (loaded, direct)
    }

    /// Unloads bank `name`: its resources and waves leave the engine
    /// (UnLoadMapSounds, the bank's objects destroyed).
    pub fn unload_bank(&mut self, name: &str) {
        let Some(i) = self.banks.iter().position(|(n, _, _)| n.eq_ignore_ascii_case(name)) else {
            return;
        };
        let (_, res, waves) = self.banks.remove(i);
        for id in res {
            self.registry.remove(&id);
        }
        for id in waves {
            self.waves.remove(&id);
        }
    }

    /// Program containing handler `name` (searched in every bank).
    pub fn find_handler_program(&self, name: &str) -> Option<openxiii_assets::hx::Cuuid> {
        let mut ids: Vec<_> = self
            .registry
            .iter()
            .filter_map(|(id, r)| match r {
                crate::engine::ResData::Program(p) if p.handler_index(name.as_bytes()).is_some() => Some(*id),
                _ => None,
            })
            .collect();
        ids.sort();
        ids.first().copied()
    }
}
