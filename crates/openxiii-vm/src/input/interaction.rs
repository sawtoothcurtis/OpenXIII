//! Player interactions (Engine.dll, UInteractionMaster): keys,
//! typed characters, Tick and drawing go through the global interactions (console) and those
//! of the viewport (GUIController), before UInput's bindings (UEngine::InputEvent).
//! A bNativeEvents interaction is served by its C++ code (GUIController: gui.rs).

use crate::value::{ObjId, Value};
use crate::Vm;

/// Interaction bits (+0x28).
pub(crate) const B_ACTIVE: &str = "bActive";
pub(crate) const B_VISIBLE: &str = "bVisible";

impl Vm {
    fn interaction_list(&self, holder: Option<ObjId>, prop: &str) -> Vec<ObjId> {
        match holder.map(|h| self.prop(h, prop)) {
            Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_object()).collect(),
            _ => Vec::new(),
        }
    }

    /// Global interactions (`global`) or those of the local player's viewport, read again for each
    /// element as Unreal's loop over the TArray does.
    fn interactions(&self, global: bool) -> Vec<ObjId> {
        self.interactions_of(global, None)
    }

    /// Like `interactions`, but the local interactions are those of the given viewport.
    fn interactions_of(&self, global: bool, viewport: Option<ObjId>) -> Vec<ObjId> {
        if global {
            self.interaction_list(self.interaction_master, "GlobalInteractions")
        } else {
            self.interaction_list(viewport.or_else(|| self.local_viewport()), "LocalInteractions")
        }
    }

    /// Viewports whose local interactions the engine walks (MasterProcessKeyType,
    /// MasterProcessTick: Client->Viewports, all of them). With a single viewport, the player's.
    fn viewports_to_walk(&mut self) -> Vec<Option<ObjId>> {
        if self.engine.client.viewports.len() > 1 {
            let v: Vec<Option<ObjId>> = self.engine.client.viewports.clone().into_iter().filter_map(|s| self.find_viewport_object(s)).map(Some).collect();
            if !v.is_empty() {
                return v;
            }
        }
        vec![None]
    }

    /// Window of the local player (UViewport: the Player object of the local controller).
    /// This machine's own window's player viewport (its mouse capture and drag), whatever a
    /// lockstep step is doing.
    pub(crate) fn machine_viewport(&self) -> Option<ObjId> {
        if let Some(v) = self.current_viewport.filter(|&v| self.objects.get(v).is_some_and(|o| o.is_some())) {
            return Some(v);
        }
        let pc = self.input_sys.as_ref().map(|s| s.pc).filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()));
        let pc = pc.or_else(|| self.actors.iter().copied().find(|&a| self.is_a(a, "PlayerController") && self.propr(a, "Player").as_object().is_some()))?;
        self.propr(pc, "Player").as_object()
    }

    /// Lockstep co-op, during a step: the session's answer, the same on every machine (the
    /// viewport the step works on, else the player whose input is being read, else the first
    /// player), never this machine's own window.
    pub(crate) fn local_viewport(&self) -> Option<ObjId> {
        // Viewport being drawn (UGameEngine::Draw(Viewport), see client.rs).
        if let Some(v) = self.current_viewport.filter(|&v| self.objects.get(v).is_some_and(|o| o.is_some())) {
            return Some(v);
        }
        let alive = |p: &ObjId| self.objects.get(*p).is_some_and(|o| o.is_some());
        let pc = self.input_sys.as_ref().map(|s| s.pc).filter(alive);
        let pc = match self.lockstep.is_some() && self.engine.client.session_graphics.in_step {
            true => session_controller(pc, &self.coop.players, alive),
            false => pc.or_else(|| self.actors.iter().copied().find(|&a| self.is_a(a, "PlayerController") && self.propr(a, "Player").as_object().is_some())),
        }?;
        self.propr(pc, "Player").as_object()
    }

    /// Game launched alone (the viewer's choice_level.rs, outside the game logic): the page in
    /// the foreground of the local GUIController when it is the main menu (XIIIMenu) without its new
    /// game video.
    pub fn menu_main(&self) -> Option<ObjId> {
        let page = self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object()).filter(|&p| self.objects.get(p).is_some_and(|x| x.is_some()))?;
        (self.is_a(page, "XIIIMenu") && !self.propr(page, "bPlayingVideo").as_bool()).then_some(page)
    }

    /// Test weapons of the level selection: once the starting
    /// inventory has been given by MapInfo.FirstFrame (bFirstTime false), MapInfo.GiveSomething(class, XIIIPawn)
    /// for each class, each Ammunition of the pawn at MaxAmmo (the weapons stay in the inventory: the start keeps the fists in hand)
    /// None as long as the map has not reached that point; otherwise the number of
    /// classes given.
    pub fn equipment_test(&mut self, classes: &[&str]) -> Option<usize> {
        let map_value = self.actors.iter().copied().find(|&a| self.is_a(a, "MapInfo") && !self.propr(a, "bFirstTime").as_bool())?;
        let live = |vm: &Vm, o: ObjId| vm.objects.get(o).is_some_and(|x| x.is_some());
        let first = self.propr(map_value, "XIIIPawn").as_object().filter(|&p| live(self, p))?;
        // Co-op (OpenXIII addition): every player gets the same equipment, each on his own pawn;
        // the game waits until each player has one.
        let mut pawns = vec![first];
        if self.coop.on {
            for pc in self.coop.players.clone() {
                let pawn = self.propr(pc, "Pawn").as_object().filter(|&p| live(self, p));
                match pawn {
                    Some(p) if !pawns.contains(&p) => pawns.push(p),
                    Some(_) => {}
                    None => return None,
                }
            }
            // Network co-op host: the guests already in the game get the host's choice too (the
            // host's game gives the items; the inventory reaches the guest as any pickup does).
            let pc_class = self.class("PlayerController");
            let remote: Vec<ObjId> = self
                .actors
                .iter()
                .copied()
                .filter(|&a| pc_class.is_some_and(|c| self.objects[a].as_ref().is_some_and(|o| self.is_child_of(o.class, c))))
                .filter(|&a| self.local_player_controller(a).is_none())
                .collect();
            for pc in remote {
                if let Some(p) = self.propr(pc, "Pawn").as_object().filter(|&p| live(self, p) && !pawns.contains(&p)) {
                    pawns.push(p);
                }
            }
        }
        let mut given = 0;
        self.coop.select_equipment = true;
        for pawn_value in pawns {
            given = self.equipment_give(map_value, pawn_value, classes);
            let weapons: Vec<String> = self.inventory_names(pawn_value);
            self.log.push(format!("Log: Select level equipment: {} has {}", self.path_name(pawn_value), weapons.join(", ")));
        }
        Some(given)
    }

    /// Class names of a pawn's inventory chain (the equipment's log).
    fn inventory_names(&self, pawn: ObjId) -> Vec<String> {
        let mut out = Vec::new();
        let mut o = self.propr(pawn, "Inventory").as_object();
        while let Some(i) = o.filter(|&i| self.objects.get(i).is_some_and(|x| x.is_some())) {
            if let Some(obj) = self.objects[i].as_ref() {
                out.push(self.classes[obj.class].name.clone());
            }
            o = self.propr(i, "Inventory").as_object();
            if out.len() > 256 {
                break;
            }
        }
        out
    }

    /// MapInfo.GiveSomething(class, pawn) for each class, then each Ammunition of the pawn at
    /// MaxAmmo; the number of classes given. The weapons go into the inventory only: the player
    /// starts with what the start put in hand (the fists), as the original's start does (the
    /// owner's choice, 15/11: no weapon drawn at the level's start, nor on Retry).
    fn equipment_give(&mut self, map_value: ObjId, pawn_value: ObjId, classes: &[&str]) -> usize {
        let mut given = 0;
        for name_value in classes {
            let Some(c) = self.class(name_value) else { continue };
            if self.call_named(map_value, "GiveSomething", vec![Value::Class(Some(c)), Value::Object(Some(pawn_value))]).is_ok() {
                given += 1;
            }
        }
        self.equipment_fill_ammo(pawn_value);
        given
    }

    /// Each Ammunition of `pawn_value` at the maximum the game allows (Inventory chained from the
    /// pawn).
    pub(crate) fn equipment_fill_ammo(&mut self, pawn_value: ObjId) {
        let mut object_value = self.propr(pawn_value, "Inventory").as_object();
        let mut seen = 0;
        while let Some(o) = object_value.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
            if self.is_a(o, "Ammunition") {
                let max = self.prop(o, "MaxAmmo");
                self.set_prop(o, "AmmoAmount", max);
            }
            object_value = self.propr(o, "Inventory").as_object();
            seen += 1;
            if seen > 256 {
                break;
            }
        }
    }

    /// GUIController of the local viewport's interactions.
    pub(crate) fn gui_local(&self) -> Option<ObjId> {
        self.interaction_list(self.local_viewport(), "LocalInteractions")
            .into_iter()
            .find(|&o| self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "GUIController"))
    }

    /// Script event of an interaction; returned value and out parameters.
    fn interaction_event(&mut self, obj: ObjId, name: &str, args: Vec<Value>) -> (Value, Vec<Option<Value>>) {
        let Some(f) = self.find_function(obj, name, false) else {
            return (Value::Void, Vec::new());
        };
        match self.call(f, obj, args.into_iter().map(Some).collect()) {
            Ok(r) => r,
            Err(e) => {
                self.warn(format!("{name}: {e}"));
                (Value::Void, Vec::new())
            }
        }
    }

    fn native_events(&self, o: ObjId) -> bool {
        self.propr(o, "bNativeEvents").as_bool()
    }

    /// UInteractionMaster::Process_KeyEvent.
    fn process_key_event(&mut self, global: bool, key: &mut u8, action: &mut u8, delta: f32) -> bool {
        let mut i = 0;
        while let Some(&o) = self.interactions(global).get(i) {
            i += 1;
            if self.objects.get(o).is_none_or(|x| x.is_none()) || !self.propr(o, B_ACTIVE).as_bool() {
                continue;
            }
            let handled = if self.native_events(o) {
                self.native_key_event(o, *key, *action, delta)
            } else {
                let (r, outs) = self.interaction_event(o, "KeyEvent", vec![Value::Byte(*key), Value::Byte(*action), Value::Float(delta)]);
                if let Some(Some(k)) = outs.first() {
                    *key = k.as_byte();
                }
                if let Some(Some(a)) = outs.get(1) {
                    *action = a.as_byte();
                }
                r.as_bool()
            };
            if handled {
                return true;
            }
        }
        false
    }

    /// UInteractionMaster::Process_KeyType.
    fn process_key_type(&mut self, global: bool, viewport: Option<ObjId>, key: &mut u8) -> bool {
        let mut i = 0;
        while let Some(&o) = self.interactions_of(global, viewport).get(i) {
            i += 1;
            if self.objects.get(o).is_none_or(|x| x.is_none()) || !self.propr(o, B_ACTIVE).as_bool() {
                continue;
            }
            let handled = if self.native_events(o) {
                self.native_key_type(o, *key)
            } else {
                let (r, outs) = self.interaction_event(o, "KeyType", vec![Value::Byte(*key)]);
                if let Some(Some(k)) = outs.first() {
                    *key = k.as_byte();
                }
                r.as_bool()
            };
            if handled {
                return true;
            }
        }
        false
    }

    /// UInteractionMaster::MasterProcessKeyEvent.
    pub(crate) fn master_key_event(&mut self, key: u8, action: u8, delta: f32) -> bool {
        let Some(master) = self.interaction_master else { return false };
        // Not in the original: Escape while joining the server again gives up (net_reconnect.rs).
        if self.net_reconnect_key(key, action) {
            return true;
        }
        if self.propr(master, "GlobalTimeOutArmed").as_bool() {
            self.set_prop(master, "TimeOfLastInput", Value::Float(self.app_seconds as f32));
        }
        let (mut key, mut action) = (key, action);
        // Co-op: the failure menu in the whole window gets every local player's keys, as that
        // view's own player's (coop_end.rs); otherwise the player's own viewport.
        let target = self.coop_end_key_viewport();
        let kept = self.current_viewport;
        if target.is_some() {
            self.current_viewport = target;
        }
        let handled = self.process_key_event(true, &mut key, &mut action, delta) || self.process_key_event(false, &mut key, &mut action, delta);
        if target.is_some() {
            self.current_viewport = kept;
        }
        handled
    }

    /// UInteractionMaster::MasterProcessKeyType, called by UEngine::Key for a
    /// typed character (WM_CHAR).
    pub fn master_key_type(&mut self, key: u8) -> bool {
        if self.interaction_master.is_none() {
            return false;
        }
        let mut key = key;
        if self.process_key_type(true, None, &mut key) {
            return true;
        }
        for vp in self.viewports_to_walk() {
            if self.process_key_type(false, vp, &mut key) {
                return true;
            }
        }
        false
    }

    /// UInteractionMaster::Process_Tick.
    fn process_tick(&mut self, global: bool, viewport: Option<ObjId>, dt: f32) {
        let mut i = 0;
        while let Some(&o) = self.interactions_of(global, viewport).get(i) {
            i += 1;
            if self.objects.get(o).is_none_or(|x| x.is_none()) || !self.propr(o, "bRequiresTick").as_bool() {
                continue;
            }
            if self.native_events(o) {
                self.native_tick(o, dt);
            } else {
                self.interaction_event(o, "Tick", vec![Value::Float(dt)]);
            }
        }
    }

    /// UInteractionMaster::MasterProcessTick, in UGameEngine::Tick after the
    /// level, with the frame's real duration.
    pub(crate) fn master_tick(&mut self, dt: f32) {
        let Some(master) = self.interaction_master else { return };
        if !self.propr(master, "GlobalTimeOutArmed").as_bool() {
            self.set_prop(master, "TimeOfLastInput", Value::Float(self.app_seconds as f32));
        } else {
            let delay = self.propr(master, "DelayOfGlobalInputInactivityBeforeTimeOut").as_float() as f64;
            let last = self.propr(master, "TimeOfLastInput").as_float() as f64;
            if delay < self.app_seconds - last {
                self.interaction_event(master, "InputTimeOut", vec![]);
                self.set_prop(master, "GlobalTimeOutArmed", Value::Bool(false));
            }
        }
        self.process_tick(true, None, dt);
        for vp in self.viewports_to_walk() {
            self.process_tick(false, vp, dt);
        }
    }

    /// Process_PreRender / Process_PostRender: from the last
    /// interaction to the first, those that are visible.
    fn process_render(&mut self, global: bool, canvas: ObjId, post: bool) {
        let mut i = self.interactions(global).len();
        while i > 0 {
            i -= 1;
            let Some(&o) = self.interactions(global).get(i) else { continue };
            if self.objects.get(o).is_none_or(|x| x.is_none()) || !self.propr(o, B_VISIBLE).as_bool() {
                continue;
            }
            if self.native_events(o) {
                if post {
                    self.native_post_render(o, canvas);
                } else {
                    self.native_pre_render(o, canvas);
                }
            } else {
                self.interaction_event(o, if post { "PostRender" } else { "PreRender" }, vec![Value::Object(Some(canvas))]);
            }
        }
    }

    /// UInteractionMaster::MasterProcessPreRender: before the scene
    /// (FCameraSceneNode::Render); viewport interactions then global ones.
    pub fn master_pre_render(&mut self, canvas: ObjId) {
        if self.interaction_master.is_none() {
            return;
        }
        self.process_render(false, canvas, false);
        self.process_render(true, canvas, false);
    }

    /// UInteractionMaster::MasterProcessPostRender: after the HUD.
    pub fn master_post_render(&mut self, canvas: ObjId) {
        if self.interaction_master.is_none() {
            return;
        }
        self.process_render(false, canvas, true);
        self.process_render(true, canvas, true);
    }

    /// Native methods (vtable +0x70…+0x80) of a bNativeEvents interaction: those of
    /// GUIController; UInteraction does nothing with them.
    fn native_key_event(&mut self, o: ObjId, key: u8, action: u8, delta: f32) -> bool {
        if self.is_a(o, "GUIController") {
            return self.gui_controller_key_event(o, key, action, delta);
        }
        false
    }

    fn native_key_type(&mut self, o: ObjId, key: u8) -> bool {
        if self.is_a(o, "GUIController") {
            return self.gui_controller_key_type(o, key);
        }
        false
    }

    fn native_tick(&mut self, o: ObjId, dt: f32) {
        if self.is_a(o, "GUIController") {
            self.gui_controller_tick(o, dt);
        }
    }

    fn native_pre_render(&mut self, o: ObjId, canvas: ObjId) {
        if self.is_a(o, "GUIController") {
            self.gui_controller_pre_render(o, canvas);
        }
    }

    fn native_post_render(&mut self, o: ObjId, canvas: ObjId) {
        if self.is_a(o, "GUIController") {
            self.gui_controller_post_render(o, canvas);
        }
    }
}

/// Lockstep co-op: the controller a step's "local player" is: `reading` (the player whose input
/// is being read) when it is one of the session's players, else the first living player.
pub(crate) fn session_controller(reading: Option<ObjId>, players: &[ObjId], alive: impl Fn(&ObjId) -> bool) -> Option<ObjId> {
    reading.filter(|p| players.contains(p)).or_else(|| players.iter().copied().find(|p| alive(p)))
}

#[cfg(test)]
mod session_tests {
    use super::session_controller;

    /// A step's local player is a session player on every machine: the one being read, else the
    /// first; never a controller that only this machine has (its own viewport's).
    #[test]
    fn session_local_player_is_a_session_player() {
        let (a, b, own) = (10, 11, 99);
        let players = [a, b];
        assert_eq!(session_controller(Some(b), &players, |_| true), Some(b));
        assert_eq!(session_controller(Some(own), &players, |_| true), Some(a));
        assert_eq!(session_controller(None, &players, |_| true), Some(a));
        assert_eq!(session_controller(None, &players, |p| *p != a), Some(b));
        assert_eq!(session_controller(Some(own), &[], |_| true), None);
    }
}
