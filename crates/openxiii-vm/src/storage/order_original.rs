//! Iteration order of the original's objects (GObjObjects).
//!
//! The original walks the objects by GObjObjects index (TObjectIterator, FObjectIterator). After a garbage collection,
//! AddObject reuses the free indices as a stack (GObjAvailable): the objects of a level loaded after another no longer follow
//! the creation order. Rust only instantiates the actors and cannot recompute these indices; they are recorded in the
//! original (one line per object: map, index, class, path) and read here. Without a record, the creation order is kept.
//! The sites that depend on it: ULevel::PostLoad (volumes), UGameEngine::LoadMap (EndMap of the interactions), APickup::AddMyMarker (not reimplemented), UHXAUDIOSubsystem::FillSoundsArray.

use crate::value::ObjId;
use crate::Vm;

impl Vm {
    /// Reads a record of lines `map;index;class;path` (header optional) for map `map_value` (case-insensitive).
    pub fn load_order_original(&mut self, csv: &str, map_value: &str) {
        self.order_original.clear();
        for l in csv.lines() {
            let c: Vec<&str> = l.split(';').collect();
            if c.len() < 4 || !c[0].eq_ignore_ascii_case(map_value) {
                continue;
            }
            if let Ok(i) = c[1].trim().parse::<u32>() {
                self.order_original.insert(c[3].trim().to_lowercase(), i);
            }
        }
    }

    /// Reads the Sound objects of map `map_value` in index order: a full `map;index;class;path` record (Sound class, sorted by
    /// index) or the reduced file `map;path` (already in order). The subsystem's array (FillSoundsArray) only keeps the
    /// first 5000 (0x1388).
    pub fn load_order_sounds(&mut self, csv: &str, map_value: &str) {
        let mut sounds_list: Vec<(u32, String)> = Vec::new();
        for (rank, l) in csv.lines().enumerate() {
            let c: Vec<&str> = l.split(';').collect();
            if c.len() < 2 || !c[0].eq_ignore_ascii_case(map_value) {
                continue;
            }
            if c.len() >= 4 {
                if c[2] == "Sound" {
                    if let Ok(i) = c[1].trim().parse::<u32>() {
                        sounds_list.push((i, c[3].trim().to_string()));
                    }
                }
            } else {
                sounds_list.push((rank as u32, c[1].trim().to_string()));
            }
        }
        sounds_list.sort();
        let kept_list = openxiii_hx::subsystem::fill_array_sounds(&vec![Some(true); sounds_list.len()]);
        self.order_sounds = kept_list.into_iter().map(|i| std::mem::take(&mut sounds_list[i].1)).collect();
        self.index_sounds = Default::default();
    }

    /// The records only keep the volumes and the interactions (object_order/embedded_*.csv files).
    pub(crate) fn order_original_for(&mut self, map_value: &str) {
        self.order_sounds.clear();
        self.index_sounds = Default::default();
        let previous = self.order_map.replace(map_value.to_string());
        // (record of volumes and interactions, record of Sounds): pass 1 or chain, as the original walked the objects.
        // Optional game data (game_data.rs): without these records the creation order is kept.
        let data = |rel: &str| crate::game_data::file(rel).unwrap_or("");
        let pass1 = (data("object_order/embedded_pass1.csv"), data("object_order/sounds_pass1.csv"));
        let string = (data("object_order/embedded_chain.csv"), data("object_order/sounds_chain.csv"));
        static MISSING_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if pass1.0.is_empty() && string.0.is_empty() && !MISSING_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            self.log.push("Original object index order: no record found (optional data object_order/, see OXIII_DATA), creation order kept".to_string());
        }
        // Depending on the actual route: first load of the session (Entry → map) = pass 1; travel map → map following the
        // campaign chain = chain; any other route has no record, the creation order is kept.
        let mut by_route = || match previous.as_deref() {
            None => pass1,
            Some(p) if p.eq_ignore_ascii_case("entry") => pass1,
            Some(p) if Self::predecessor_string(map_value).is_some_and(|c| c.eq_ignore_ascii_case(p)) => string,
            Some(p) => {
                self.log.push(format!("Original index order: no record for the route {p} -> {map_value}, creation order kept"));
                ("", "")
            }
        };
        let (text_value, sounds_list) = by_route();
        self.load_order_original(text_value, map_value);
        self.load_order_sounds(sounds_list, map_value);
    }

    /// Map that precedes `map_value` in the recorded campaign chain (object_order/chain_maps.txt).
    fn predecessor_string(map_value: &str) -> Option<&'static str> {
        let list_value: Vec<&'static str> = crate::game_data::file("object_order/chain_maps.txt").unwrap_or("").lines().map(str::trim).collect();
        let i = list_value.iter().position(|c| c.eq_ignore_ascii_case(map_value))?;
        i.checked_sub(1).map(|j| list_value[j])
    }

    /// Sort key of an object: original index if recorded, otherwise after all recorded ones in creation order.
    pub(crate) fn key_order(&self, id: ObjId) -> (u32, ObjId) {
        let Some(o) = self.objects.get(id).and_then(|o| o.as_ref()) else { return (u32::MAX, id) };
        let path_buf = format!("{}.{}", o.outer, o.name).to_lowercase();
        (self.order_original.get(&path_buf).copied().unwrap_or(u32::MAX), id)
    }

    /// Sorts the objects in the original's iteration order when a record is loaded (otherwise changes nothing).
    pub(crate) fn sort_original(&self, ids: &mut [ObjId]) {
        if !self.order_original.is_empty() {
            ids.sort_by_key(|&o| self.key_order(o));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn sounds_in_order_index_and_capped() {
        let mut vm = Vm::new();
        vm.load_order_sounds("map;9;Sound;P.B
map;2;Sound;P.A
map;5;Interaction;P.I
other;1;Sound;P.Z", "MAP");
        assert_eq!(vm.order_sounds, vec!["P.A".to_string(), "P.B".to_string()]);
        let mut big = String::new();
        for i in 0..5003 {
            big.push_str(&format!("map;{};Sound;P.S{i}
", 5003 - i));
        }
        vm.load_order_sounds(&big, "map");
        assert_eq!(vm.order_sounds.len(), 5000, "FillSoundsArray stops at 0x1388 sounds");
        assert_eq!(vm.order_sounds[0], "P.S5002", "lowest index first");
        vm.load_order_sounds("map;P.X
map;P.Y", "map");
        assert_eq!(vm.order_sounds, vec!["P.X".to_string(), "P.Y".to_string()], "reduced file: the file's order");
    }

    #[test]
    fn record_sorted_and_without_record_change_nothing() {
        let mut vm = Vm::new();
        let mut ids = vec![3usize, 1, 2];
        vm.sort_original(&mut ids);
        assert_eq!(ids, vec![3, 1, 2], "without a record: the received order is kept");
        vm.load_order_original("map;5;Interaction;Pkg.A\nmap;2;Interaction;Pkg.B\nother;1;Interaction;Pkg.C", "MAP");
        assert_eq!(vm.order_original.len(), 2);
    }

    #[test]
    fn pass_chosen_by_route() {
        if crate::game_data::test_file("object_order/embedded_pass1.csv").is_none() || crate::game_data::test_file("object_order/chain_maps.txt").is_none() {
            return;
        }
        let mut vm = Vm::new();
        // first load of the session: pass 1
        vm.order_original_for("Banque01");
        let pass1 = vm.order_original.clone();
        assert!(!pass1.is_empty());
        // travel Banque01 -> Amos01, following the chain: chain (a record other than pass 1)
        vm.order_original_for("Amos01");
        assert!(!vm.order_original.is_empty());
        assert!(vm.log.is_empty());
        // route outside the chain: no record and a warning
        vm.order_original_for("Hual02");
        assert!(vm.order_original.is_empty());
        assert_eq!(vm.log.len(), 1);
        assert!(vm.log[0].contains("Amos01 -> Hual02"));
        // after Entry: pass 1
        let mut vm = Vm::new();
        vm.order_original_for("Entry");
        vm.order_original_for("Plage01");
        assert!(!vm.order_original.is_empty() && vm.log.is_empty());
    }
}
