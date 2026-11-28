//! Mods build only: the mods' new classes ("Mod.<Name>", `[classes.<Name>]` tables).
//!
//! A new class is a subclass of a game class that declares nothing of its own: the parent's
//! functions and states run for it (they are found by walking up the class chain, as for any
//! subclass), and its default values start from the parent's default values as they are in
//! memory (the mods' values on the parent included). Its own values come from the mods'
//! `[class."Mod.<Name>"]` tables, applied like any class's (mod_defaults.rs).
//!
//! The class is registered with the classes of the packages, before they are linked, under
//! "Mod.<Name>", under its short name and under "<parent's package>.<Name>" (the path the
//! engine code builds from a class's package). It has no export of its own: it keeps the
//! parent's package and export, and the loader never reads members or serialized default
//! values for it (`link_class`, `cdo_values`).

use std::sync::{Arc, OnceLock};

use openxiii_mods::new_classes::{NewClass, PACKAGE};

use crate::types::ClassDef;
use crate::value::ClassId;
use crate::Vm;

static GLOBAL: OnceLock<Arc<Vec<NewClass>>> = OnceLock::new();

/// Sets the new classes of every Vm created afterwards (once, at startup, before any game
/// script is loaded). A second call is ignored.
pub fn set_mod_classes(list: Vec<NewClass>) {
    let _ = GLOBAL.set(Arc::new(list));
}

pub(crate) fn global_mod_classes() -> Option<Arc<Vec<NewClass>>> {
    GLOBAL.get().cloned().filter(|l| !l.is_empty())
}

impl Vm {
    /// New classes of this Vm only (tests). Must be called before the classes are loaded.
    pub fn set_mod_classes_here(&mut self, list: Vec<NewClass>) {
        self.mods.new_classes = Some(Arc::new(list)).filter(|l| !l.is_empty());
    }

    /// The game class a mod's new class is made from (None for the game's classes).
    pub(crate) fn mod_class_parent(&self, c: ClassId) -> Option<ClassId> {
        self.mods.class_parents.get(&c).copied()
    }

    /// "Mod", the package part of a mod's new class's name (None for the game's classes).
    pub(crate) fn mod_class_package(&self, c: ClassId) -> Option<&'static str> {
        self.mods.class_parents.contains_key(&c).then_some(PACKAGE)
    }

    /// In register_classes, once the packages' classes are known and before they are linked:
    /// one class per mod's new class whose parent is a class of the game's packages.
    pub(crate) fn register_mod_classes(&mut self) {
        let Some(list) = self.mods.new_classes.clone() else { return };
        for n in list.iter() {
            let qualified = n.class_name().to_lowercase();
            if self.class_qualified.contains_key(&qualified) {
                continue;
            }
            let Some(&parent) = self.class_qualified.get(&n.parent.to_lowercase()) else {
                self.mod_report(format!("warning: new class {}: {} is not a class of the game's packages; class not made ({})", n.class_name(), n.parent, n.source));
                continue;
            };
            let key = n.name.to_lowercase();
            if let Some(&other) = self.class_index.get(&key) {
                let other = format!("{}.{}", self.packages[self.classes[other].package].name, self.classes[other].name);
                self.mod_report(format!("warning: new class {}: the game already has a class named {} ({other}); class not made ({})", n.class_name(), n.name, n.source));
                continue;
            }
            let p = &self.classes[parent];
            let package = self.packages[p.package].name.to_lowercase();
            let def = ClassDef {
                name: n.name.clone(),
                package: p.package,
                export: p.export,
                super_name: Some(p.name.clone()),
                super_class: None,
                props: Vec::new(),
                prop_index: Default::default(),
                functions: Default::default(),
                states: Default::default(),
                cdo: usize::MAX,
                linked: false,
                // CLASS_Abstract (1) cleared: a new class is made to be spawned.
                flags: p.flags & !1,
                config_name: p.config_name.clone(),
                probe_mask: p.probe_mask,
            };
            let id = self.classes.len();
            self.classes.push(def);
            if let Some(w) = self.class_within.get(&parent).cloned() {
                self.class_within.insert(id, w);
            }
            self.class_index.insert(key.clone(), id);
            self.class_qualified.insert(qualified, id);
            self.class_qualified.entry(format!("{package}.{key}")).or_insert(id);
            self.mods.class_parents.insert(id, parent);
            self.mod_report(format!("new class {} made from {} ({})", n.class_name(), n.parent, n.source));
        }
    }

    /// In link_class, for a mod's new class whose parent is linked: the parent's properties, no
    /// functions or states of its own.
    pub(crate) fn link_mod_class(&mut self, c: ClassId, parent: ClassId) {
        let (props, prop_index, probe_mask) = {
            let p = &self.classes[parent];
            (p.props.clone(), p.prop_index.clone(), p.probe_mask)
        };
        let class = &mut self.classes[c];
        class.super_class = Some(parent);
        class.props = props;
        class.prop_index = prop_index;
        class.functions = Default::default();
        class.states = Default::default();
        class.probe_mask = probe_mask;
    }
}
