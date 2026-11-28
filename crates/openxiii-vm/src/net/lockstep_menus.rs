//! Lockstep co-op (lockstep.rs): the menus this machine leaves for a network co-op game and
//! comes back to.
//!
//! - The menu pages XIIIRootWindow.CloseAll keeps when the menu map is left (SaveMenuStack),
//!   noted while on the menus and given back to the new VM of the menus after a session, so
//!   that its UWindows.BeginState restores them (RestoreMenuStack) instead of the start pages.
//! - The menu map back after a game whose travels left the root window in its menu state.

use crate::value::{ObjId, Value};
use crate::Vm;

/// The menu stack observed before leaving for a lockstep session.
#[derive(Default)]
pub(crate) struct RememberedMenus {
    /// Last non-empty list of pages, kept even if CloseAll later saves an empty stack.
    pages: Option<Vec<String>>,
    /// Last SaveMenuStack object examined, including empty lists.
    source: Option<ObjId>,
}

impl Vm {
    /// The menu map after a network co-op game (`lockstep_menu_back_wanted`), with no menu page
    /// open after a second: its menus' root window may have been left in its menu state
    /// (UWindows) by the travels of the game (measured: the menu map's ShowMenu found the root
    /// window already in UWindows, so BeginState never opened a page: a white screen until
    /// Escape). It leaves that state and the menu is asked for again.
    pub(crate) fn lockstep_menu_back(&mut self) {
        if !self.coop.menu_back_wanted {
            return;
        }
        let map = self.url.map.to_ascii_lowercase();
        if !map.starts_with("mapmenu") {
            return;
        }
        if self.level_time < 1.0 {
            return;
        }
        self.coop.menu_back_wanted = false;
        let roots: Vec<ObjId> = (0..self.objects.len())
            .filter(|&o| self.objects[o].as_ref().is_some_and(|x| !x.name.starts_with("Default__")) && self.is_a(o, "XIIIRootWindow"))
            .collect();
        let open = roots.iter().any(|&o| matches!(self.prop(o, "MenuStack"), Value::Array(p) if p.iter().any(|x| x.as_object().is_some())));
        if open {
            return;
        }
        for &o in &roots {
            if self.current_state(o).is_some_and(|(_, s)| s.name.eq_ignore_ascii_case("UWindows")) {
                let _ = self.call_named(o, "GotoState", vec![Value::Name("None".into())]);
            }
        }
        if let Some(pc) = self.find_viewport_object(0).and_then(|v| self.prop(v, "Actor").as_object()).filter(|&p| self.alive(p)) {
            let _ = self.call_named(pc, "ShowMenu", vec![]);
        }
        self.log.push("Log: Co-op: the menu shown again after the session (its root window was left in its menu state)".into());
    }

    /// Lockstep co-op: this machine goes back to its menus from a network game's page or map
    /// (the co-op character screen): the menus' root window is checked (`lockstep_menu_back`).
    pub fn lockstep_menu_back_wanted(&mut self) {
        self.coop.menu_back_wanted = true;
    }

    /// The menu pages XIIIRootWindow.CloseAll kept when the menu map was left
    /// (SaveMenuStack.Menu: each page's class and parameters), None without any.
    pub fn saved_menu_stack(&self) -> Option<Vec<String>> {
        self.saved_menu_stack_now().or_else(|| self.remembered_menus.pages.clone())
    }

    /// Notes the menu pages the root window keeps when its kept list changes (read only; once
    /// a frame from the viewer on the menus, outside lockstep sessions).
    pub fn note_menus_left(&mut self) {
        let Some(root) = self.gui_local() else { return };
        let saved = self.propr(root, "SaveMenuStack").as_object();
        if saved == self.remembered_menus.source {
            return;
        }
        self.remembered_menus.source = saved;
        if let Some(m) = self.saved_menu_stack_now() {
            self.remembered_menus.pages = Some(m);
        }
    }

    pub(crate) fn saved_menu_stack_now(&self) -> Option<Vec<String>> {
        // The first player's root window (on a network game's page the viewport's interactions
        // are not the menu's: the first root window made, as in the menus).
        let root = self.gui_local().filter(|&r| self.is_a(r, "XIIIRootWindow")).or_else(|| {
            (0..self.objects.len()).find(|&o| self.objects[o].as_ref().is_some_and(|x| !x.name.starts_with("Default__")) && self.is_a(o, "XIIIRootWindow"))
        })?;
        let saved = self.propr(root, "SaveMenuStack").as_object().filter(|&s| self.alive(s))?;
        let Value::Array(list) = self.prop(saved, "Menu") else { return None };
        let menus: Vec<String> = list.iter().filter_map(|v| if let Value::Str(s) = v { Some(s.to_string()) } else { None }).collect();
        (!menus.is_empty()).then_some(menus)
    }

    /// Lockstep co-op, the menus of a new VM after a session: the pages kept when this
    /// machine's menus were left for the session, given to this VM's root window as its
    /// SaveMenuStack, so that the menu map's UWindows.BeginState restores them
    /// (RestoreMenuStack), as on any return to the menus, instead of the start pages.
    /// The same for this VM's menus (a network game that did not start, back to the menus).
    pub fn give_saved_menu_stack(&mut self, menus: &[String]) -> bool {
        let root = self.gui_local().filter(|&r| self.is_a(r, "XIIIRootWindow")).or_else(|| {
            (0..self.objects.len()).find(|&o| self.objects[o].as_ref().is_some_and(|x| !x.name.starts_with("Default__")) && self.is_a(o, "XIIIRootWindow"))
        });
        root.is_some_and(|r| self.give_saved_menu_stack_to(r, menus))
    }

    pub(crate) fn give_saved_menu_stack_to(&mut self, root: ObjId, menus: &[String]) -> bool {
        if !self.is_a(root, "XIIIRootWindow") {
            return false;
        }
        let Some(class) = self.class("XIIISaveMenuStack") else { return false };
        let (Some(saved), _) = self.exec_new(None, None, 0, Some(class)) else { return false };
        self.set_prop(saved, "Menu", Value::Array(menus.iter().map(|m| Value::Str(m.as_str().into())).collect()));
        self.set_prop(root, "SaveMenuStack", Value::Object(Some(saved)));
        true
    }
}
