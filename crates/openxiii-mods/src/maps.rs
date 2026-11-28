//! Which maps a `[map.<Selector>…]` table is for (docs/modding/README.md, "Maps and checkpoints").
//!
//! - a map name, the map file name without `.unr`: `Plage01`;
//! - the beginning of map names followed by `*`: `Hual*` (Hual01a, Hual01b, Hual02…);
//! - `solo`: the maps of the story (every map but the multiplayer and technical ones);
//! - `multi`: the multiplayer maps (names starting with `DM_`, `CTF_` or `SB_`);
//! - `*`: every map.
//!
//! When several tables give the same value for a map, the most precise one wins: the map's own
//! name, then the longest beginning of name, then `solo` or `multi`, then `*`.

/// Maps that are not levels: the menu, the credits, empty maps.
pub fn is_technical(map: &str) -> bool {
    ["mapmenu", "mapcredits", "entry", "empty"].iter().any(|n| map.eq_ignore_ascii_case(n))
}

/// The multiplayer maps: names starting with DM_, CTF_ or SB_ (deathmatch, capture the flag,
/// sabotage).
pub fn is_multiplayer(map: &str) -> bool {
    let n = map.to_ascii_lowercase();
    ["dm_", "ctf_", "sb_"].iter().any(|p| n.starts_with(p))
}

/// A map of the story.
pub fn is_solo(map: &str) -> bool {
    !is_multiplayer(map) && !is_technical(map)
}

fn name_chars(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A valid selector: a map name, `Name*`, `solo`, `multi` or `*`.
pub fn valid_selector(s: &str) -> bool {
    s == "*" || s.strip_suffix('*').map_or_else(|| name_chars(s), name_chars)
}

/// How precisely `selector` names `map`, None if it is not for that map: higher wins.
pub fn rank(selector: &str, map: &str) -> Option<u32> {
    if selector == "*" {
        return Some(1);
    }
    if selector.eq_ignore_ascii_case("solo") {
        return is_solo(map).then_some(2);
    }
    if selector.eq_ignore_ascii_case("multi") {
        return is_multiplayer(map).then_some(2);
    }
    if let Some(prefix) = selector.strip_suffix('*') {
        let starts = map.len() >= prefix.len() && map.is_char_boundary(prefix.len()) && map[..prefix.len()].eq_ignore_ascii_case(prefix);
        return starts.then(|| 10 + prefix.len() as u32);
    }
    selector.eq_ignore_ascii_case(map).then_some(10_000)
}

/// True if `selector` is for `map`.
pub fn matches(selector: &str, map: &str) -> bool {
    rank(selector, map).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors() {
        assert!(valid_selector("Plage01") && valid_selector("Hual*") && valid_selector("*") && valid_selector("solo"));
        assert!(!valid_selector("Hual**") && !valid_selector("Bad Map") && !valid_selector("") && !valid_selector("*Hual"));
        assert_eq!(rank("plage01", "Plage01"), Some(10_000));
        assert_eq!(rank("Hual*", "Hual01a"), Some(14));
        assert!(rank("Hual01*", "Hual01a") > rank("Hual*", "Hual01a"));
        assert_eq!(rank("Hual*", "Plage01"), None);
        assert_eq!(rank("solo", "Plage01"), Some(2));
        assert_eq!(rank("solo", "DM_Amos"), None);
        assert_eq!(rank("solo", "MapMenu"), None);
        assert_eq!(rank("multi", "ctf_Sanc"), Some(2));
        assert_eq!(rank("multi", "Plage01"), None);
        assert_eq!(rank("*", "MapMenu"), Some(1));
        assert!(rank("Plage01", "Plage01") > rank("Plage*", "Plage01"));
        assert!(rank("P*", "Plage01") > rank("solo", "Plage01"));
        assert!(rank("solo", "Plage01") > rank("*", "Plage01"));
    }
}
