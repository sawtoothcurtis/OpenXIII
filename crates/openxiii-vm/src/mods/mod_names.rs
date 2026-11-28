//! Mods build only (feature `mods`): the two lookups a mod's names go through, shared by the
//! `[[map.<Map>.change]]` entries (mod_change.rs), the names catalog (mod_catalog.rs) and any
//! other code that turns a name written by a modder into something of the game.
//!
//! - [`Vm::find_placed`]: the actors of the loaded map that a `who` value names.
//! - [`character_look`]: the character model (`Mesh`) a `look` value names, with the table of
//!   the friendly names in [`character_looks`].

use crate::value::ObjId;
use crate::Vm;

impl Vm {
    /// The actors of the loaded map that `who` names (case ignored), as `who = "…"` of a
    /// `[[map.<Map>.change]]` entry finds them. The first rule that finds any wins:
    ///
    /// 1. the actor whose object name is `who` ("XIIIBaseSoldier12"; unique in a map);
    /// 2. the characters (Pawns) whose `Tag` or `PawnName` is `who`: characters come before the
    ///    other actors because a scene's dispatcher and triggers often share the character's
    ///    Tag (PRock03's "gardien_lostpass" is also a XIIIDispatcher2's Tag);
    /// 3. the other actors whose `Tag` is `who`.
    ///
    /// Destroyed actors (`bDeleteMe`) are left out. With `near`, only the actors within
    /// `radius` units of that point are kept (the actor's Location).
    pub fn find_placed(&self, who: &str, near: Option<[f32; 3]>, radius: f32) -> Vec<ObjId> {
        self.mod_find_placed(who, near, radius)
    }

    /// The `who` value to suggest for actor `a`: its PawnName (characters) or its Tag when
    /// [`Vm::find_placed`] finds `a` alone with it, else its object name, which is unique in a
    /// map. Used by the catalog and by `modwho`, so that both print the same value.
    pub fn friendly_who(&self, a: ObjId) -> String {
        let text = |v: crate::value::Value| match v {
            crate::value::Value::Str(s) => s.trim().to_string(),
            v => v.as_name(),
        };
        let pawn_name = if self.is_a(a, "Pawn") { text(self.prop(a, "PawnName")) } else { String::new() };
        let tag = text(self.prop(a, "Tag"));
        let alone = |w: &str| !w.is_empty() && !w.eq_ignore_ascii_case("None") && self.find_placed(w, None, 0.0) == vec![a];
        match [pawn_name, tag].into_iter().find(|w| alone(w)) {
            Some(w) => w,
            None => self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.name.clone()).unwrap_or_default(),
        }
    }
}

/// The friendly names of the character models: (model, names), the first name being the one
/// shown and suggested. A model of XIIIPersos also answers to its short name with or without
/// the final "M" ("Spads", "SpadsM" for XIIIPersos.SpadsM), which this table does not repeat.
pub fn character_looks() -> &'static [(&'static str, &'static [&'static str])] {
    openxiii_mods::looks::CHARACTER_LOOKS
}

/// The character model (`Mesh`, "Package.Name") that a `look = "…"` value names: a full name
/// ("XIIIPersos.JonesM") as written, else a friendly name of [`character_looks`] or a model's
/// short name, case ignored. Err: up to three of the closest friendly names, for "did you mean".
///
/// A full name is returned without checking that the game has it; the caller loads it.
pub fn character_look(name: &str) -> Result<String, Vec<String>> {
    openxiii_mods::looks::resolve(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_by_friendly_short_and_full_name() {
        assert_eq!(character_look("jones").as_deref(), Ok("XIIIPersos.JonesM"));
        assert_eq!(character_look("Guard").as_deref(), Ok("XIIIPersos.GardienM"));
        assert_eq!(character_look("GardienM").as_deref(), Ok("XIIIPersos.GardienM"));
        assert_eq!(character_look("XIIIPersos.SpadsM").as_deref(), Ok("XIIIPersos.SpadsM"));
        // Negative control: a wrong name is refused, with the closest names.
        let why = character_look("Jone").unwrap_err();
        assert!(why.iter().any(|n| n == "Jones"), "{why:?}");
        assert!(character_looks().iter().any(|(m, names)| *m == "XIIIPersos.JonesM" && names[0] == "Jones"));
    }
}
