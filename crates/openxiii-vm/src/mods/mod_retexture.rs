//! Mods build only (feature `mods`): textures a script changes on one actor of the scenery
//! (`actor:retexture(from, to)`). The viewer draws the static mesh of that actor with `to` where
//! its own material is `from`; nothing else changes, and nothing at all without a script asking.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Script-driven changes to the scenery of one game, shared with its renderer.
#[derive(Debug, Default)]
pub struct SurfaceEffects {
    changes: RwLock<Option<HashMap<String, Vec<(String, String)>>>>,
    generation: AtomicU64,
    charred: RwLock<Option<HashMap<String, f32>>>,
    areas: RwLock<Vec<([f32; 3], f32, f32)>>,
    areas_generation: AtomicU64,
    heat: std::sync::atomic::AtomicBool,
}

impl SurfaceEffects {
    pub fn heat_used(&self) -> bool {
        self.heat.load(Ordering::Relaxed)
    }

    pub(crate) fn set_heat_used(&self, on: bool) {
        self.heat.store(on, Ordering::Relaxed);
    }

    /// `actor`'s material `from` is drawn as `to` from now on (`to` empty: as its own again).
    pub fn set(&self, actor: &str, from: &str, to: &str) {
        let mut guard = self.changes.write().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        let list = map.entry(actor.to_lowercase()).or_default();
        list.retain(|(f, _)| !f.eq_ignore_ascii_case(from));
        if !to.is_empty() {
            list.push((from.to_string(), to.to_string()));
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// What `owner`'s material `material` is drawn as, when a script changed it.
    pub fn lookup(&self, owner: &str, material: &str) -> Option<String> {
        let guard = self.changes.read().unwrap_or_else(|e| e.into_inner());
        let list = guard.as_ref()?.get(owner)?;
        list.iter().find(|(f, _)| f.eq_ignore_ascii_case(material)).map(|(_, t)| t.clone())
    }

    /// True when no actor has a changed texture (the viewer then does nothing more).
    pub fn is_empty(&self) -> bool {
        self.changes.read().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(HashMap::is_empty)
    }

    /// Changes at every change.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Every change forgotten (a new level, the scripts started again).
    pub fn clear(&self) {
        let mut guard = self.changes.write().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|m| !m.is_empty()) {
            *guard = None;
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `actor` is drawn this much charred from now on (0: as the game draws it).
    pub fn set_charred(&self, actor: &str, k: f32) {
        let mut guard = self.charred.write().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if k > 0.0 {
            map.insert(actor.to_lowercase(), k);
        } else {
            map.remove(&actor.to_lowercase());
        }
    }

    /// How charred `owner` (an actor of the scenery, lower case) is drawn, when a script charred it.
    pub fn charred(&self, owner: &str) -> Option<f32> {
        let guard = self.charred.read().unwrap_or_else(|e| e.into_inner());
        guard.as_ref()?.get(owner).copied()
    }

    /// True when no actor of the scenery is charred.
    pub fn no_charred(&self) -> bool {
        self.charred.read().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(HashMap::is_empty)
    }

    /// Every charred actor forgotten (a new level, the scripts started again).
    pub fn clear_charred(&self) {
        *self.charred.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The level's surfaces within `radius` of `at` are drawn `k` charred (the darkest wins).
    pub fn char_area(&self, at: [f32; 3], radius: f32, k: f32) {
        let mut areas = self.areas.write().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = areas.iter_mut().find(|a| a.0 == at && a.1 == radius) {
            if a.2 >= k {
                return;
            }
            a.2 = k;
        } else {
            areas.push((at, radius, k));
        }
        self.areas_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// The charred areas of the level's surfaces.
    pub fn char_areas(&self) -> Vec<([f32; 3], f32, f32)> {
        self.areas.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Changes at every new charred area.
    pub fn char_areas_generation(&self) -> u64 {
        self.areas_generation.load(Ordering::Relaxed)
    }

    /// Every charred area forgotten (a new level, the scripts started again).
    pub fn clear_char_areas(&self) {
        let mut areas = self.areas.write().unwrap_or_else(|e| e.into_inner());
        if !areas.is_empty() {
            areas.clear();
            self.areas_generation.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl crate::Vm {
    /// The renderer reads the same surface effects that scripts update on this VM.
    pub fn mod_surface_effects(&self) -> &std::sync::Arc<SurfaceEffects> {
        &self.mods.surfaces
    }
}
