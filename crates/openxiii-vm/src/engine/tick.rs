//! Tick of the level and of each actor, reimplemented from Engine.dll: ULevel::Tick,
//! AActor::Tick, AActor::TickAuthoritative, AController::Tick,
//! APlayerController::Tick, AActor::CheckOwnerUpdated /
//! APawn::CheckOwnerUpdated, AActor::UpdateTimers and
//! AController::UpdateTimers.
//!
//! The level handles its dynamic actors one by one, in the order of its list (bStatic
//! actors, placed in front by LoadMap, are never handled), then those created during the frame;
//! an actor whose owner (or, for a pawn, whose controller) has not gone yet is
//! postponed. Each actor does, in this order: UpdateAnimation, Tick, ProcessState
//! (state code and latent actions), timers, LifeSpan, physics, then TickSpecial.

use crate::NativeField as Field;
use crate::value::{ObjId, Value};
use crate::Vm;
use std::collections::HashSet;

// ELevelTick.
const LEVELTICK_VIEWPORTS_ONLY: u8 = 1;
const LEVELTICK_ALL: u8 = 2;

// ENetRole.
const ROLE_SIMULATED_PROXY: u8 = 2;
const ROLE_AUTONOMOUS_PROXY: u8 = 3;
const ROLE_AUTHORITY: u8 = 4;

/// Slots in the properties of each class, including mod classes.
pub(super) struct TickSlots {
    delete_me: Option<usize>,
    ticked: Option<usize>,
    physics: Option<usize>,
    idle_class: Option<IdleClass>,
}

#[derive(Clone, Copy)]
struct IdleClass {
    stamp: u64,
    tick: IdleEvent,
    decoration: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum IdleEvent {
    Missing,
    Returns(u32),
    Script,
}

#[derive(Clone, Copy)]
pub(crate) struct IdleHint {
    class_stamp: u64,
    inert: bool,
    check_order: bool,
}

impl Vm {
    pub(crate) fn idle_event(&self, f: Option<crate::types::FuncId>) -> IdleEvent {
        match f {
            None => IdleEvent::Missing,
            Some(f) => self.empty_tick_offset(f).map_or(IdleEvent::Script, IdleEvent::Returns),
        }
    }

    fn prepare_tick_slots(&mut self) {
        if self.tick_slots.len() != self.classes.len() {
            // Each binding has its own identity, even if a snapshot restores cloned Props
            // or if a class is reloaded with new slots.
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let first = NEXT.fetch_add(self.classes.len() as u64, std::sync::atomic::Ordering::Relaxed);
            use crate::flags_classes::D;
            let special = [D::Pawn, D::Controller, D::Emitter, D::Trail];
            self.tick_slots = self.classes.iter().enumerate().map(|(i, class)| TickSlots {
                delete_me: class.prop_index.get("bdeleteme").copied(),
                ticked: class.prop_index.get("bticked").copied(),
                physics: class.prop_index.get("physics").copied(),
                idle_class: (self.class_is(i, D::Actor) && !special.iter().any(|&flag| self.class_is(i, flag))).then(|| {
                    let tick = std::iter::successors(Some(i), |&c| self.classes[c].super_class)
                        .find_map(|c| self.classes[c].functions.get("tick")).copied();
                    IdleClass { stamp: first + i as u64, tick: self.idle_event(tick), decoration: self.class_is(i, D::Decoration) }
                }),
            }).collect();
        }
    }

    /// Ordinary actor whose class Tick event is empty or missing. Keep
    /// the call to the event for its probes, its depth and its traces. States
    /// and network links may change without writing to the properties.
    fn idle_actor_tick(&mut self, a: ObjId, dt: f32, tick_type: u8) -> bool {
        let object = self.object(a);
        let Some(class) = self.tick_slots.get(object.class).and_then(|slots| slots.idle_class) else { return false };
        if self.ipdrv.net_driver.is_some() || object.state.is_none() && matches!(class.tick, IdleEvent::Script) {
            return false;
        }
        let hint = match object.props.idle_tick.get() {
            Some(hint) if hint.class_stamp == class.stamp => hint,
            _ => {
                let inert = self.role(a) == ROLE_AUTHORITY && self.native_prop(a, Field::RemoteRole).as_byte() != ROLE_AUTONOMOUS_PROXY
                    && !matches!(self.native_prop(a, Field::Mesh), Value::Asset(_)) && self.tick_physics(a) == 0
                    && (!class.decoration || self.propr(a, "Shadow").as_object().is_none())
                    && self.native_prop(a, Field::LifeSpan).as_float() == 0.
                    && self.native_prop(a, Field::TimerRate).as_float() <= 0.
                    && self.native_prop(a, Field::Timer2Rate).as_float() <= 0.;
                let check_order = self.native_prop(a, Field::Stasis).as_bool() || self.native_prop(a, Field::Owner).as_object().is_some();
                let hint = IdleHint { class_stamp: class.stamp, inert, check_order };
                object.props.idle_tick.set(Some(hint));
                hint
            }
        };
        if !hint.inert || self.threads.get(&a).is_some_and(|t| t.code.is_some())
            || self.ipdrv.resolves.contains_key(&a) || self.ipdrv.udp.contains_key(&a) {
            return false;
        }
        // A state can redefine Tick or inherit it from another state. Use the same
        // resolution as ProcessEvent, even when the properties have not changed.
        let tick = if object.state.is_some() { self.tick_state(a) } else { class.tick };
        if matches!(tick, IdleEvent::Script) { return false; }
        // The order between actors and stasis depend on other objects or on time.
        // Leave postponed actors and actors actually in stasis to the normal path.
        if hint.check_order && (self.stasis_skip(a) || !self.check_owner_updated(a)) { return false; }
        self.set_ticked(a);
        if tick_type == LEVELTICK_VIEWPORTS_ONLY { return true; }
        // UpdateAnimation forgets an old instance when the Mesh is removed.
        self.mesh_insts.remove(&a);
        self.event_idle_tick(a, dt, tick);
        true
    }

    /// End of ULevel::Tick, joined by the actors' loop and by the paused loop
    /// without a ServerConnection, TickNetServer (vtable +0x7C) with the frame's
    /// DeltaSeconds, then NetDriver->TickFlush (+0x84). A paused host keeps sending to its clients.
    fn level_tick_network_end(&mut self, dt: f32) {
        let Some(d) = self.ipdrv.net_driver.as_ref() else { return };
        if d.server_conn.is_none() {
            self.net_tick_server(dt);
        }
        self.net_tick_flush();
    }

    /// ULevel::Tick: level time (× TimeDilation, stopped by Pauser), frame
    /// clamped to [0.0005; 0.4] s, then the Tick of each dynamic actor.
    pub fn tick(&mut self, dt: f32) {
        let _math = self.session.math.enter();
        // Not in the original: enhanced multiplayer, the other players drawn smoothly
        // (net_smooth.rs); lockstep co-op back on the menus (lockstep_menus.rs).
        self.net_smooth_tick();
        self.lockstep_menu_back();
        self.net_host_left_notice_tick();
        self.update_local_characters(dt);
        // Window events (UInput): in the original, XIII.exe's loop pumps them
        // after UGameEngine::Tick, so before the next frame and its Level.Pauser (Escape opens the
        // in-game menu, the next frame is already paused).
        // PlayerTick in the controller's Tick.
        if !self.ipdrv.dedicated {
            self.player_input_events();
        }
        // UGameEngine::Tick: GEngine->FrameLenght (+ 0xD0) = frame duration, then
        // CurrentFrameCount + 1.
        self.engine_delta = dt;
        self.engine_frames = self.engine_frames.wrapping_add(1);
        self.target_rendered.clear();
        self.sine_drawn.clear();
        // appSeconds: real time of the frame, without dilation or pause.
        self.app_seconds += dt as f64;
        // GEngine->FrameLenght (UCanvas::execDrawCineFrame).
        self.canvas_state.frame_length = dt;
        // UGameEngine::Tick: Viewports[0]->RI->AnimRndCubeSpr(DeltaSeconds) before
        // the levels (weather, appFrand shared with the script).
        if !self.ipdrv.dedicated {
            self.rnd_cube_anim(dt);
        }
        let level = self.level;
        // UGameEngine::Tick (0x1037DC9x), right before GLevel->Tick: LevelInfo.bDropDetail if
        // DeltaSeconds > 1 / Clamp(MinDesiredFrameRate, 1, 100), bAggressiveLOD with
        // MinDesiredFrameRate − 5.
        if let Some(l) = level.filter(|_| !self.ipdrv.dedicated) {
            let rate = match self.min_desired_frame_rate {
                Some(r) => r,
                None => {
                    let r = self
                        .config
                        .as_mut()
                        .and_then(|c| c.get_string("WinDrv.WindowsClient", "MinDesiredFrameRate", "System"))
                        .and_then(|v| v.trim().parse::<f32>().ok())
                        .unwrap_or(0.0);
                    self.min_desired_frame_rate = Some(r);
                    r
                }
            };
            let clamp = |v: f32| if v >= 1.0 { if v >= 100.0 { 100.0 } else { v } } else { 1.0 };
            let (drop, lod) = (1.0 / clamp(rate) < dt, 1.0 / clamp(rate - 5.0) < dt);
            self.set_prop(l, "bDropDetail", Value::Bool(drop));
            self.set_prop(l, "bAggressiveLOD", Value::Bool(lod));
        }
        // ULevel::Tick: GInitRunaway, then NetDriver->TickDispatch(DeltaSeconds).
        self.runaway = 0;
        self.net_tick_dispatch(dt);
        // ULevel::Tick: TickNetClient (vtable +0x78) on a client.
        if self.ipdrv.net_driver.as_ref().is_some_and(|d| d.server_conn.is_some()) {
            self.net_tick_client();
        }
        let dilation = level.map_or(1.0, |l| self.pf(l, "TimeDilation"));
        let paused = level.is_some_and(|l| self.po(l, "Pauser").is_some());
        let players_only = level.is_some_and(|l| self.pb(l, "bPlayersOnly"));
        let advance_value = advance_time(self.level_time, LEVELTICK_ALL, dt, dilation, paused, players_only);
        if !paused {
            self.time += advance_value.dt_dilate;
            self.level_time = advance_value.time_seconds;
        }
        if let Some(hx) = self.audio.as_mut() {
            hx.time_level = self.level_time;
        }
        if let Some(l) = level {
            self.set_native_prop(l, Field::TimeSeconds, Value::Float(self.level_time as f32));
            // ULevel::UpdateTime (vtable +0xE8): appSystemTime (GetLocalTime) into
            // Year, Month, DayOfWeek, Day, Hour, Minute, Second, Millisecond (StatLog).
            // Lockstep co-op: the session's clock (the host's at the meeting, then the steps'
            // pace), the same on every machine.
            let fields = fields_time(crate::lockstep::level_date(self.lockstep.as_ref().map(|l| l.step), self.session.clock));
            for (name, v) in fields {
                self.set_prop(l, name, Value::Int(v));
            }
        }
        let (tick_type, dt) = (advance_value.tick_type, advance_value.dt_tick);
        self.refresh_colliders();
        // ULevel::Tick: the actors of Actors from iFirstDynamicActor
        // (+0x100) on; before it, the LevelInfo, the default volume and the bStatic actors are not handled.
        let from = self.first_dynamic.min(self.actors.len());
        let actors: Vec<ObjId> = self.actors[from..].to_vec();
        if paused {
            loop_pause(&mut WorldVm { vm: self, dt, tick_type }, actors, true);
            let skip = false;
            if !skip {
                self.level_tick_network_end(dt);
            }
            self.level_ticked = !self.level_ticked;
            self.cleanup_destroyed(false);
            return;
        }
        self.in_tick = true;
        self.newly_spawned.clear();
        self.spawn_order.clear();
        if tick_type != 0 {
            self.prepare_tick_slots();
            loop_actors(&mut WorldVm { vm: self, dt, tick_type }, actors);
        }
        self.in_tick = false;
        self.newly_spawned.clear();
        self.spawn_order.clear();
        // Fix that is not in the original: long hints opened on an already broken box (indices_broken.rs).
        if !self.indices_a_close.is_empty() {
            self.close_indices_broken();
        }
        self.level_tick_network_end(dt);
        // Ticked = !Ticked (0x10390E7x), then CleanupDestroyed(0).
        self.level_ticked = !self.level_ticked;
        self.cleanup_destroyed(false);
        // Mods build only: the looping animation of a mod character's state, kept on
        // (mod_behaviour.rs).
        #[cfg(feature = "mods")]
        if !self.mods.live.is_empty() {
            self.mod_keep_looks();
        }
        // Mods build only: the mods' Lua scripts (mod_script.rs), before the lines they may say.
        #[cfg(feature = "mods")]
        if self.mods.scripts.is_some() {
            let now = self.level.map_or(self.time, |l| self.prop(l, "TimeSeconds").as_float());
            self.mod_scripts_step(now);
            // Actors pinned to a point of another one (pin_to) go there.
            self.mod_pins_apply();
        }
        // Mods build only: lines said by characters (mod_dialogue.rs).
        #[cfg(feature = "mods")]
        if !self.mods.dialogues.says.is_empty() {
            self.mod_dialogues_tick();
        }
        // Not in the original: co-op, the upkeep of this tick (coop_story.rs: a cinematic or a
        // flashback that starts or ends) done before the frame is drawn, so that the window's
        // layout and the views' effects change on the same frame as the scene (it runs at most
        // once per level time; the first view of the frame then finds it done).
        if self.coop.on && !self.coop.players.is_empty() {
            self.coop_combat_frame();
        }
        if self.coop.on {
            self.screen_fx_hidden_views();
        }
    }

    /// ULevel::CleanupDestroyed, called at the end of each ULevel::Tick: as long as
    /// fewer than 128 actors have been destroyed, nothing; otherwise the references from the level's actors
    /// to bDeleteMe actors become None (UClass::CleanupDestroyed: UObjectProperty, and
    /// UDelegateProperty for delegates), then the destroyed actors are deleted.
    pub(crate) fn cleanup_destroyed(&mut self, force: bool) {
        if self.first_deleted.is_empty() || (!force && self.first_deleted.len() < 0x80) {
            return;
        }
        let dead: HashSet<ObjId> = self.first_deleted.iter().copied().collect();
        // Only the actors that point to a destroyed one are written: iter_mut changes the generation
        // of the Props, and all generations changed at once rebuilt the caches of all
        // actors on the next frame (a 10 to 25 ms spike on PC). Same values.
        for &a in &self.actors {
            if let Some(o) = self.objects[a].as_mut() {
                if o.props.iter().any(|v| destroyed_contains(v, &dead)) {
                    o.props.iter_mut().for_each(|v| clear_destroyed(v, &dead));
                }
            }
        }
        // Loop: delete of each actor (vt 0x10); its memory stays readable through
        // the references that the cleanup did not touch (see `freed`).
        for a in std::mem::take(&mut self.first_deleted) {
            if let Some(o) = self.objects[a].take() {
                // ~UObject: ConditionalDestroy then UnhashObject, which removes
                // the object from the table that StaticFindObject reads: its name becomes free again
                // for MakeUniqueObjectName (a destroyed soldier's name can be taken again by a later spawn).
                self.object_paths.remove(&format!("{}.{}", o.outer, o.name).to_lowercase());
                self.freed.insert(a, o);
            }
        }
    }

    /// What the engine's renderer does for each drawn actor: LastRenderTime = TimeSeconds.
    /// A motionless bStasis actor that nobody has drawn for 5 s is no longer handled (see
    /// `stasis_skip`); without rendering (simulate), this applies to all of them.
    pub fn mark_rendered(&mut self, a: ObjId) {
        if self.objects.get(a).is_some_and(|o| o.is_some()) {
            let t = self.level_time as f32;
            self.set_f(a, "LastRenderTime", t);
        }
    }

    /// Actors of the level drawn by the last frame (same criterion as the game:
    /// LastRenderTime == Level.TimeSeconds, set by `mark_rendered`): (object name, short name
    /// of its class).
    pub fn list_drawn(&self) -> Vec<(String, String)> {
        let t = self.level_time as f32;
        self.actors
            .iter()
            .filter_map(|&a| {
                let o = self.objects.get(a)?.as_ref()?;
                (self.native_prop(a, Field::LastRenderTime).as_float() == t).then(|| (o.name.clone(), self.classes[o.class].name.clone()))
            })
            .collect()
    }

    /// Living actor that the level handles (neither destroyed nor bDeleteMe).
    fn ticking(&self, a: ObjId) -> bool {
        let Some(object) = self.objects.get(a).and_then(Option::as_ref) else { return false };
        match self.tick_slots.get(object.class) {
            Some(slots) => !slots.delete_me.and_then(|i| object.props.get(i)).is_some_and(Value::as_bool),
            None => !self.native_prop(a, Field::DeleteMe).as_bool(),
        }
    }

    fn tick_physics(&self, a: ObjId) -> u8 {
        let object = self.object(a);
        match self.tick_slots.get(object.class) {
            Some(slots) => slots.physics.and_then(|i| object.props.get(i)).unwrap_or(&Value::Void).as_byte(),
            None => self.native_prop(a, Field::Physics).as_byte(),
        }
    }

    fn role(&self, a: ObjId) -> u8 {
        self.native_prop(a, Field::Role).as_byte()
    }

    /// AActor::Tick's stasis: bStasis, without physics (or rotating), not drawn for 5 s.
    fn stasis_skip(&self, a: ObjId) -> bool {
        self.native_prop(a, Field::Stasis).as_bool()
            && matches!(self.tick_physics(a), 0 | 5)
            // TimeSeconds (double) − LastRenderTime, rounded to single precision (x87 24 bits).
            && ((self.level_time - self.native_prop(a, Field::LastRenderTime).as_float() as f64) as f32) > 5.0
            && self.level.is_none_or(|l| self.propr(l, "NetMode").as_byte() == 0)
    }

    /// bTicked (bit 0x20000 of + 0x2C) equal to the level's parity: handled in this frame.
    pub(crate) fn is_ticked(&self, a: ObjId) -> bool {
        let object = self.object(a);
        let ticked = match self.tick_slots.get(object.class) {
            Some(slots) => slots.ticked.and_then(|i| object.props.get(i)).is_some_and(Value::as_bool),
            None => self.pb(a, "bTicked"),
        };
        ticked == self.level_ticked
    }

    fn set_ticked(&mut self, a: ObjId) {
        let t = self.level_ticked;
        if let Some(object) = self.objects[a].as_mut() {
            if let Some(slots) = self.tick_slots.get(object.class) {
                // Without changing the Props generation (Props::set_render): bTicked changes
                // every frame for every actor.
                if let Some(i) = slots.ticked {
                    object.props.set_render(i, Value::Bool(t));
                }
                return;
            }
        }
        if let Ok(i) = self.prop_slot(a, "bTicked") {
            if let Some(o) = self.objects[a].as_mut() { o.props.set_render(i, Value::Bool(t)); }
        }
    }

    /// CheckOwnerUpdated (vt 0xD4): false as long as the bTicked of the owner (and, for a pawn,
    /// of its controller) differs from the level's parity.
    fn check_owner_updated(&self, a: ObjId) -> bool {
        let pending = |o: Option<ObjId>| o.is_some_and(|o| self.objects.get(o).is_some_and(|x| x.is_some()) && !self.is_ticked(o));
        if pending(self.native_object(a, Field::Owner)) {
            return false;
        }
        !(self.is(a, crate::flags_classes::D::Pawn) && pending(self.native_object(a, Field::Controller)))
    }

    /// Tick (vt 0x78) of an actor; false if it is postponed.
    fn actor_tick(&mut self, a: ObjId, dt: f32, tick_type: u8, deferred: &mut Vec<ObjId>) -> bool {
        crate::tally::inc(crate::tally::ACTOR_TICKS);
        if !self.tick_ordinary && self.idle_actor_tick(a, dt, tick_type) { return true; }
        if self.is(a, crate::flags_classes::D::PlayerController) {
            self.player_controller_tick(a, dt, tick_type);
            return true;
        }
        if self.is(a, crate::flags_classes::D::Controller) {
            self.controller_tick(a, dt, tick_type);
            return true;
        }
        // AActor::Tick.
        if self.stasis_skip(a) {
            crate::tally::inc(crate::tally::STASIS);
            self.set_ticked(a);
            return true;
        }
        if !self.check_owner_updated(a) {
            deferred.push(a);
            return false;
        }
        self.set_ticked(a);
        // LEVELTICK_ViewportsOnly (bPlayersOnly): only the pawns of a player (vt 0xE4).
        if tick_type == LEVELTICK_VIEWPORTS_ONLY && !(self.is(a, crate::flags_classes::D::Pawn) && self.native_object(a, Field::Controller).is_some_and(|c| self.is(c, crate::flags_classes::D::PlayerController))) {
            return true;
        }
        self.update_animation(a, dt);
        let role = self.role(a);
        if self.native_prop(a, Field::RemoteRole).as_byte() == ROLE_AUTONOMOUS_PROXY {
            let mut top = a;
            for _ in 0..64 {
                match self.native_object(top, Field::Owner) {
                    Some(o) => top = o,
                    None => break,
                }
            }
            // IsA(PlayerController) then LocalPlayerController (vt 0x150,
            // Player is a UViewport); on the server, a client's pawn (Player = its
            // UNetConnection) has no physics here, only through ServerMove.
            if self.is(top, crate::flags_classes::D::PlayerController) && self.local_player_controller(top).is_some() {
                self.tick_authoritative(a, dt);
            } else {
                self.process_state(a, dt);
                self.update_timers(a, dt);
            }
        } else if role < ROLE_AUTONOMOUS_PROXY {
            if role == ROLE_SIMULATED_PROXY {
                self.tick_simulated(a, dt);
            } else if (!self.native_prop(a, Field::DeleteMe).as_bool() && self.tick_physics(a) == crate::physics::PHYS_FALLING)
                || self.tick_physics(a) == crate::physics::PHYS_ROTATING
            {
                self.perform_physics(a, dt);
            }
        } else {
            self.tick_authoritative(a, dt);
        }
        if self.ticking(a) {
            self.tick_special(a, dt);
        }
        // Tests without effect: the class and IpDrv's tables first, `ticking` only for the
        // actors concerned (same result, each step may destroy the actor).
        // AEmitter::Tick: the particle emitters after AActor::Tick.
        if self.is(a, crate::flags_classes::D::Emitter) && self.ticking(a) {
            self.emitter_tick(a, dt);
        }
        // AInternetLink::Tick: end of a resolution after AActor::Tick.
        if !self.ipdrv.resolves.is_empty() && self.ipdrv.resolves.contains_key(&a) && self.ticking(a) {
            self.internet_link_tick(a);
        }
        // AUdpLink::Tick: receiving after AActor::Tick.
        if !self.ipdrv.udp.is_empty() && self.ipdrv.udp.contains_key(&a) && self.ticking(a) {
            self.udp_link_tick(a);
        }
        // ATrail::Tick: the trail after AActor::Tick.
        if self.is(a, crate::flags_classes::D::Trail) && self.ticking(a) {
            self.trail_tick(a, dt);
        }
        true
    }

    /// TickSimulated (vtable +0xDC): AActor::TickSimulated calls TickAuthoritative (vtable +0xD8); a pawn has APawn::TickSimulated.
    pub(crate) fn tick_simulated(&mut self, a: ObjId, dt: f32) {
        if self.is(a, crate::flags_classes::D::Pawn) {
            self.pawn_tick_simulated(a, dt);
        } else {
            self.tick_authoritative(a, dt);
        }
    }

    /// APawn::TickSimulated (another player's pawn on the client side): Acceleration = Velocity.SafeNormal(); Physics according to the volume (water: swimming 3, ladder
    /// with bCanClimbLadders: 11) then bSimulateGravity (walking 1, otherwise flying 4); moveSmooth(Velocity * dt); with bSimulateGravity, if the pawn is not going up
    /// (Velocity.Z <= 0) a downward trace of 8 units (Z < 0) or of half a height (Z = 0) looks for the floor (Time < 1 and Normal.Z >= 0.7); without a floor: Velocity.Z = Gravity.Z * 0.15 if Z = 0,
    /// Velocity += Gravity * 0.5 * dt, Physics = falling (2); finally the Tick event if the probe is active.
    fn pawn_tick_simulated(&mut self, a: ObjId, dt: f32) {
        use crate::physics::{safe_normal, CheckResult};
        let v = self.native_prop(a, Field::Velocity).as_vector();
        let acc = safe_normal(v);
        self.set_native_prop(a, Field::Acceleration, Value::vector(acc[0], acc[1], acc[2]));
        let volume = self.native_object(a, Field::PhysicsVolume);
        let gravity_value = |vm: &Vm| volume.map_or([0.0, 0.0, -950.0], |flight| vm.propr(flight, "Gravity").as_vector());
        let simulated_value = self.pb(a, "bSimulateGravity");
        let physics_list = if volume.is_some_and(|flight| self.pb(flight, "bWaterVolume")) {
            crate::physics::PHYS_SWIMMING
        } else if self.pb(a, "bCanClimbLadders") && volume.is_some_and(|flight| self.is_a(flight, "LadderVolume")) {
            crate::physics::PHYS_LADDER
        } else if simulated_value {
            crate::physics::PHYS_WALKING
        } else {
            crate::physics::PHYS_FLYING
        };
        self.set_native_prop(a, Field::Physics, Value::Byte(physics_list));
        self.move_smooth(a, [v[0] * dt, v[1] * dt, v[2] * dt]);
        if simulated_value {
            let vz = self.native_prop(a, Field::Velocity).as_vector()[2];
            let mut at_floor = false;
            if !(vz > 0.0) && !vz.is_nan() {
                let (cr, ch) = (self.native_prop(a, Field::CollisionRadius).as_float(), self.native_prop(a, Field::CollisionHeight).as_float());
                let hz = if vz == 0.0 { ch * 0.5 } else { 8.0 };
                let loc = self.native_prop(a, Field::Location).as_vector();
                let mut hit = CheckResult::new(1.0);
                self.single_line_check(&mut hit, Some(a), [loc[0], loc[1], loc[2] - hz], loc, 0xDF, [cr, cr, ch]);
                at_floor = hit.time != 1.0 && !(hit.normal[2] < 0.7);
            }
            if !at_floor {
                let g = gravity_value(self);
                let mut vel = self.native_prop(a, Field::Velocity).as_vector();
                if vz == 0.0 {
                    vel[2] = g[2] * 0.15;
                }
                let (tx, ty, tz) = ((g[0] * 0.5) * dt, (g[1] * 0.5) * dt, (g[2] * 0.5) * dt);
                self.set_native_prop(a, Field::Velocity, Value::vector(tx + vel[0], ty + vel[1], tz + vel[2]));
                self.set_native_prop(a, Field::Physics, Value::Byte(crate::physics::PHYS_FALLING));
            }
        }
        self.event_pub(a, "Tick", vec![Value::Float(dt)]);
    }

    /// AActor::TickAuthoritative: Tick, ProcessState, timers, LifeSpan (end of
    /// life: DestroyActor), then physics.
    fn tick_authoritative(&mut self, a: ObjId, dt: f32) {
        self.event_pub(a, "Tick", vec![Value::Float(dt)]);
        self.process_state(a, dt);
        self.update_timers(a, dt);
        if self.objects[a].is_none() {
            return;
        }
        let life = self.native_prop(a, Field::LifeSpan).as_float();
        if life != 0.0 {
            let left = life - dt;
            self.set_f(a, "LifeSpan", left);
            if left <= 0.0001 {
                if let Err(e) = crate::natives::call(self, "Destroy", a, &mut [], &[]) {
                    self.warn(format!("LifeSpan of {}: {e}", self.obj_label(a)));
                }
                return;
            }
        }
        if self.ticking(a) && self.tick_physics(a) != 0 && self.role(a) != ROLE_AUTONOMOUS_PROXY {
            self.perform_physics(a, dt);
        }
    }

    /// AController::Tick: TickAuthoritative, then the pawn turns towards the Focus
    /// (otherwise the FocalPoint) and the controller takes its rotation. A controller in stasis, or whose
    /// pawn is, does nothing.
    fn controller_tick(&mut self, c: ObjId, dt: f32, tick_type: u8) {
        self.set_ticked(c);
        let pawn = self.native_object(c, Field::Pawn);
        if self.native_prop(c, Field::Stasis).as_bool() || pawn.is_some_and(|p| self.stasis_skip(p)) {
            return;
        }
        // Role >= 2, so ROLE_SimulatedProxy too.
        if self.role(c) >= ROLE_SIMULATED_PROXY {
            self.tick_authoritative(c, dt);
        }
        if self.objects[c].is_none() {
            return;
        }
        if self.role(c) == ROLE_AUTHORITY && tick_type == LEVELTICK_ALL {
            self.controller_perception(c, dt);
            if self.objects[c].is_none() {
                return;
            }
        }
        self.controller_rotation(c);
    }

    #[inline(always)]
    fn pc_step(&self, _: &str, _: ObjId, _: f32) -> bool {
        false
    }

    /// APlayerController::Tick: water planes, input and PlayerTick of the local player,
    /// ProcessState, timers, physics, SightCounter. No Tick event.
    fn player_controller_tick(&mut self, pc: ObjId, dt: f32, tick_type: u8) {
        self.set_ticked(pc);
        // DeltaT (+0x5FC): frame duration, for SmoothedAim.
        self.set_prop(pc, "DeltaT", Value::Float(dt));
        self.water_plane_repulsion(pc);
        if self.po(pc, "ViewTarget").is_none() {
            let target = self.native_object(pc, Field::Pawn).unwrap_or(pc);
            self.set_prop(pc, "ViewTarget", Value::Object(Some(target)));
        }
        let role = self.role(pc);
        let local = self.native_object(pc, Field::Player).is_some();
        // RemoteRole == ROLE_AutonomousProxy and not LocalPlayerController
        // (vt 0x150, Player is a UViewport): a client's controller on the server, whose
        // Player is a UNetConnection.
        if self.native_prop(pc, Field::RemoteRole).as_byte() == ROLE_AUTONOMOUS_PROXY && self.local_player_controller(pc).is_none() {
            if !self.pc_step("pc_process_state", pc, dt) {
                self.process_state(pc, dt);
            }
            if !self.pc_step("pc_update_timers", pc, dt) {
                self.update_timers(pc, dt);
            }
        } else if role >= ROLE_SIMULATED_PROXY {
            if local && !self.pc_step("pc_input", pc, dt) {
                self.player_controller_input(pc, dt);
            }
            if self.objects[pc].is_none() {
                return;
            }
            if !self.pc_step("pc_process_state", pc, dt) {
                self.process_state(pc, dt);
            }
            if !self.pc_step("pc_update_timers", pc, dt) {
                self.update_timers(pc, dt);
            }
            if !self.ticking(pc) {
                return;
            }
            if self.tick_physics(pc) != 0 && role != ROLE_AUTONOMOUS_PROXY && !self.pc_step("pc_perform_physics", pc, dt) {
                self.perform_physics(pc, dt);
            }
        }
        if self.objects[pc].is_none() {
            return;
        }
        if role == ROLE_AUTHORITY && tick_type == LEVELTICK_ALL {
            self.player_perception(pc, dt);
        }
    }

    /// Start of APlayerController::Tick: for each water plane of the level (ULevel +
    /// 0x1188) whose volume does not have bSkipWaterRepulsion, if the pawn's eyes are less than 20
    /// from the surface and the pawn within the plane's extent, bDontUseVolumeFog and, when |Velocity.Z| <
    /// 30, a vertical push (1 − 0.05 d) × GEngine->DeltaSeconds × 150 upwards above the
    /// surface, × 105 downwards below it. Window effect: the
    /// global flag turns off the wound effect of Viewports[0]+0x154 (vt 0xA8, (0,
    /// 0.5)) at the start; a plane where 1 − 0.05 d exceeds 0.5 turns it on (1, 0.5) and sets the flag,
    /// unless the pawn's zone has FlashEffectDesc.IsActivated.
    fn water_plane_repulsion(&mut self, pc: ObjId) {
        let Some(pawn) = self.native_object(pc, Field::Pawn) else { return };
        // Split screen: the effects of this player's own view (screen_fx.rs).
        let fx = self.screen_fx_of(pc);
        if fx.underwater {
            fx.set_injured(false, 0.5);
            fx.underwater = false;
        }
        let loc = self.native_prop(pawn, Field::Location).as_vector();
        let eye_z = loc[2] + self.propr(pawn, "EyeHeight").as_float();
        self.set_prop(pc, "bDontUseVolumeFog", Value::Bool(false));
        let planes = self.water_planes.clone();
        for (p, volume) in planes {
            // bSkipWaterRepulsion (+ 0x2CC, bit 0x80).
            if self.propr(volume, "bSkipWaterRepulsion").as_bool() {
                continue;
            }
            let d = (eye_z - p[2]).abs();
            if !(d < 20.0) || !((loc[0] - p[0]).abs() <= p[3]) || !((loc[1] - p[1]).abs() <= p[4]) {
                continue;
            }
            let f = 1.0 - d * 0.05;
            let mut v = self.native_prop(pawn, Field::Velocity).as_vector();
            if v[2].abs() < 30.0 {
                v[2] = if eye_z > p[2] { (f * self.engine_delta) * 150.0 + v[2] } else { v[2] - (f * self.engine_delta) * 105.0 };
                self.set_native_prop(pawn, Field::Velocity, Value::vector(v[0], v[1], v[2]));
            }
            self.set_prop(pc, "bDontUseVolumeFog", Value::Bool(true));
            if f > 0.5 && !self.screen_fx_of(pc).underwater && !self.pawn_zone_flash(pawn) {
                let fx = self.screen_fx_of(pc);
                fx.set_injured(true, 0.5);
                fx.underwater = true;
            }
        }
    }

    /// Pawn.Region.Zone (+0x98) and its FlashEffectDesc.IsActivated (+0x22C, bit 1).
    fn pawn_zone_flash(&self, pawn: ObjId) -> bool {
        let zone = match self.native_value(pawn, Field::Region) {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
            _ => None,
        };
        zone.is_some_and(|z| matches!(self.prop(z, "FlashEffectDesc"), Value::Struct(_, f) if f.first().is_some_and(|b| b.as_bool())))
    }

    /// AActor::UpdateTimers (Timer and Timer2) and AController::UpdateTimers
    /// (Timer3), on the actor's TimerRate / TimerCounter / bTimerLoop: if the rate is
    /// positive, the counter receives dt; when it expires it loses a whole (truncated) number of periods,
    /// a non-looping timer goes to a zero rate, then the event is called.
    fn update_timers(&mut self, a: ObjId, dt: f32) {
        const TIMERS: [(Field, Field, Field, &str); 3] = [
            (Field::TimerRate, Field::TimerCounter, Field::TimerLoop, "Timer"),
            (Field::Timer2Rate, Field::Timer2Counter, Field::Timer2Loop, "Timer2"),
            (Field::Timer3Rate, Field::Timer3Counter, Field::Timer3Loop, "Timer3"),
        ];
        let count = if self.is(a, crate::flags_classes::D::Controller) { 3 } else { 2 };
        for &(rate_name, counter_name, loop_name, event) in &TIMERS[..count] {
            let rate = self.native_prop(a, rate_name).as_float();
            if !(rate > 0.0) {
                continue;
            }
            let counter = self.native_prop(a, counter_name).as_float();
            let run_loop = self.native_prop(a, loop_name).as_bool();
            let (counter, rate, call_value) = step_timer(rate, counter, run_loop, dt);
            self.set_native_prop(a, counter_name, Value::Float(counter));
            self.set_native_prop(a, rate_name, Value::Float(rate));
            if !call_value {
                continue;
            }
            // Timer3: AController::eventTimer3.
            crate::tally::inc(crate::tally::TIMERS);
            self.event_pub(a, event, vec![]);
            if self.objects[a].is_none() {
                return;
            }
        }
    }

    /// AActor::ProcessState: the Poll* native of the current latent action, then the
    /// state code as long as no latent action is started.
    pub(crate) fn process_state(&mut self, a: ObjId, dt: f32) {
        let Some(latent) = self.threads.get(&a).filter(|t| t.code.is_some()).map(|t| t.latent) else { return };
        // Nothing for an actor being destroyed (vt 0x38, AActor::IsPendingKill
        // bDeleteMe). A GotoState done after Destroy, in the same Tick
        // (DialogueManager.STA_HeadAnimation.Tick to STA_PlayingDialogue "EOL"), does not run
        // its state code.
        if self.is(a, crate::flags_classes::D::Actor) && self.native_prop(a, Field::DeleteMe).as_bool() {
            return;
        }
        // Without authority (Role < ROLE_Authority, network client), only a
        // "simulated" state (STATE_Simulated) runs its code and its waits. Actors
        // only: GUI pages (UGUIPage, no Role) have their own pass.
        let simulated_value = self.current_state(a).is_some_and(|(_, sd)| sd.simulated);
        if self.is(a, crate::flags_classes::D::Actor) && !code_state_allowed(self.role(a), simulated_value) {
            return;
        }
        if latent != 0 {
            self.poll_latent(a, latent, dt);
        }
        if self.objects[a].is_none() || self.latent_action(a) != 0 {
            return;
        }
        let r = self.run_thread(a);
        if let Err(e) = r {
            self.warn(format!("state code of {}: {e}", self.obj_label(a)));
            self.threads.remove(&a);
        }
    }

    /// performPhysics (vt 0xFC) depending on the actor: Mover (AMover::performPhysics), pawn
    /// (APawn::performPhysics), other actors (AActor::performPhysics).
    fn perform_physics(&mut self, a: ObjId, dt: f32) {
        if self.collision.is_none() {
            return;
        }
        let physics = self.tick_physics(a);
        crate::tally::inc(crate::tally::PHYSICS + (physics as usize).min(15));
        if self.phys_counts.is_some() && physics != 0 {
            let family = if self.is_a(a, "Mover") { "Mover" } else if self.is_a(a, "Pawn") { "Pawn" } else { "Actor" };
            let class = self.objects[a].as_ref().map(|o| self.classes[o.class].name.clone()).unwrap_or_default();
            if let Some(m) = self.phys_counts.as_mut() {
                *m.entry(format!("{family} phys {physics}")).or_default() += 1;
                *m.entry(format!("  {family} phys {physics} {class}")).or_default() += 1;
            }
        }
        if self.is(a, crate::flags_classes::D::Mover) {
            self.mover_perform_physics(a, dt);
        } else if self.is(a, crate::flags_classes::D::Pawn) {
            if physics != 0 {
                self.pawn_perform_physics(a, dt);
            }
        } else {
            self.actor_perform_physics(a, dt);
        }
    }

    /// TickSpecial (vt 0xE0): APawn::TickSpecial (movement animation, EyeHeight…),
    /// ADecoration::TickSpecial and AProjector::TickSpecial.
    fn tick_special(&mut self, a: ObjId, dt: f32) {
        if self.is(a, crate::flags_classes::D::Pawn) {
            self.pawn_tick_special(a, dt);
        } else if self.is(a, crate::flags_classes::D::Decoration) {
            if let Some(sp) = self.po(a, "Shadow").filter(|&s| self.alive(s)) {
                let now = self.level.map_or(0.0, |l| self.native_prop(l, Field::TimeSeconds).as_float());
                if let Some(force) = shadow_to_detach(now, self.native_prop(a, Field::LastRenderTime).as_float(), self.pb(sp, "bShadowIsStatic")) {
                    self.projector_detach(sp, force);
                }
            }
        } else if self.is(a, crate::flags_classes::D::Projector) && self.tick_physics(a) == 5 {
            // PHYS_Rotating: CalcMatrix (vt 0x14C).
            self.projector_calc_matrix(a);
        }
    }
}

/// A timer of AActor::UpdateTimers whose rate is positive: the counter receives
/// dt; when it expires it loses a whole number (truncated, _ftol) of periods, and a
/// non-looping timer that has done at least one period goes to a zero rate. Returns (counter,
/// rate, event called).
pub(crate) fn step_timer(rate: f32, counter: f32, run_loop: bool, dt: f32) -> (f32, f32, bool) {
    let counter = dt + counter;
    if !(counter >= rate) {
        return (counter, rate, false);
    }
    let periods = (counter / rate) as i32;
    let rate_after = if periods != 0 && !run_loop { 0.0 } else { rate };
    (counter - periods as f32 * rate, rate_after, true)
}

/// AActor::ProcessState: state code allowed if the actor has authority (Role ≥ 4) or
/// if its state is STATE_Simulated.
pub(crate) fn code_state_allowed(role: u8, simulated_value: bool) -> bool {
    role >= 4 || simulated_value
}

/// ADecoration::TickSpecial: a shadow (Shadow) not drawn for more than half
/// a second (Level.TimeSeconds − LastRenderTime > 0.5) and not bShadowIsStatic is detached
/// by a jump to Detach (vtable +0x144) whose argument is replaced by 1: returns
/// Some(force = true) in that case.
pub(crate) fn shadow_to_detach(now_value: f32, last_render: f32, static_value: bool) -> Option<bool> {
    (0.5 < now_value - last_render && !static_value).then_some(true)
}

/// UObjectProperty::CleanupDestroyed, UDelegateProperty::CleanupDestroyed
/// (object and name reset to zero), UArrayProperty and UStructProperty
/// (through UStruct::CleanupDestroyed): a reference to a destroyed actor (IsPendingKill)
/// becomes None, in arrays and structs too.
/// Does the value (or an array or struct element) designate a destroyed actor?
fn destroyed_contains(v: &Value, dead: &HashSet<ObjId>) -> bool {
    match v {
        Value::Object(Some(o)) | Value::Delegate(Some(o), _) => dead.contains(o),
        Value::Array(items) => items.iter().any(|x| destroyed_contains(x, dead)),
        Value::Struct(_, fields) => fields.iter().any(|x| destroyed_contains(x, dead)),
        _ => false,
    }
}

pub(crate) fn clear_destroyed(v: &mut Value, dead: &HashSet<ObjId>) {
    match v {
        Value::Object(Some(o)) if dead.contains(o) => *v = Value::Object(None),
        Value::Delegate(Some(o), _) if dead.contains(o) => *v = Value::Delegate(None, crate::value::NameStruct::EMPTY),
        Value::Array(items) => items.iter_mut().for_each(|x| clear_destroyed(x, dead)),
        Value::Struct(_, fields) => fields.iter_mut().for_each(|x| clear_destroyed(x, dead)),
        _ => {}
    }
}

/// What ULevel::Tick's actor loops ask of the world: the `Vm` in game, a mock in tests.
pub(crate) trait WorldTick {
    /// The actor exists (non-null entry of Actors).
    fn actor_exists(&self, a: ObjId) -> bool;
    /// The actor exists and is not bDeleteMe.
    fn actor_ticking(&self, a: ObjId) -> bool;
    /// bTicked of the actor equal to the level's Ticked parity.
    fn actor_ticked(&self, a: ObjId) -> bool;
    /// Tick (vt +0x78); false if it is postponed (the actor is then added to `postponed`).
    fn tick_actor(&mut self, a: ObjId, postponed: &mut Vec<ObjId>) -> bool;
    /// Actors created since the last call: (creation order in Actors, NewlySpawned list).
    fn created(&mut self) -> (Vec<ObjId>, Vec<ObjId>);
    /// PlayerController with a Player (+0x330): player input while paused.
    fn player_with_player(&self, a: ObjId) -> bool;
    fn always_ticks(&self, a: ObjId) -> bool;
    fn tick_player_pause(&mut self, a: ObjId);
    /// No effect on the game state.
    fn preload(&self, _object: Option<ObjId>, _properties: Option<ObjId>) {}
}

/// ULevel::Tick without pause: the actors of the list, those that an actor's
/// Tick creates at the end of the list (the length of Actors is read again on each turn), then the
/// NewlySpawned list (head first) as long as a pass has ticked an actor. Returns what remains of the list
/// (the most recent last).
pub(crate) fn loop_actors<M: WorldTick>(m: &mut M, actors: Vec<ObjId>) -> Vec<ObjId> {
    let mut deferred: Vec<ObjId> = Vec::new();
    let mut queue = actors;
    let mut updated = 0usize;
    let mut i = 0;
    while i < queue.len() {
        let a = queue[i];
        m.preload(queue.get(i + 8).copied(), queue.get(i + 4).copied());
        i += 1;
        if m.actor_ticking(a) && m.tick_actor(a, &mut deferred) {
            updated += 1;
        }
        // Actors created during the frame are added at the end of the list, and ULevel::SpawnActor
        // (InTick) also puts them at the head of NewlySpawned (XLevel + 0xF4), the list
        // of postponed actors: blood created with a postponed Owner appears there twice, and goes at its
        // creation entry if its Owner has gone in the meantime.
        let (spawned, newly) = m.created();
        queue.extend_from_slice(&spawned);
        deferred.extend(newly);
    }
    // Postponed actors (NewlySpawned list, the last entry first), as long as one goes:
    // those whose bTicked differs from the parity (0x10390C9x). A map actor whose
    // saved bTicked equals the parity is skipped (Banque01: the Cine pawns, postponed behind
    // their CineController created at load time, do not go on the first frame; those of
    // Plage01, saved with bTicked, do).
    while !deferred.is_empty() && updated != 0 {
        updated = 0;
        let list = std::mem::take(&mut deferred);
        for &a in list.iter().rev() {
            if m.actor_ticking(a) && !m.actor_ticked(a) && m.tick_actor(a, &mut deferred) {
                updated += 1;
            }
            let (_, newly) = m.created();
            deferred.extend(newly);
        }
    }
    deferred
}

/// ULevel::Tick while paused: the players' input (without testing
/// bDeleteMe), and the bAlwaysTick actors; an actor created during the loop also goes
/// (`follows_created`, like the original). Returns the remaining NewlySpawned list (the most recent last).
pub(crate) fn loop_pause<M: WorldTick>(m: &mut M, actors: Vec<ObjId>, follows_created: bool) -> Vec<ObjId> {
    let mut list_value: Vec<ObjId> = Vec::new();
    let mut queue = actors;
    let mut i = 0;
    while i < queue.len() {
        let a = queue[i];
        i += 1;
        if m.actor_exists(a) {
            if m.player_with_player(a) {
                m.tick_player_pause(a);
            } else if m.always_ticks(a) && m.actor_ticking(a) {
                m.tick_actor(a, &mut list_value);
            }
        }
        let (spawned, newly) = m.created();
        if follows_created {
            queue.extend_from_slice(&spawned);
        }
        list_value.extend(newly);
    }
    list_value
}

/// The real world of the actor loops.
struct WorldVm<'a> {
    vm: &'a mut Vm,
    dt: f32,
    tick_type: u8,
}

impl WorldTick for WorldVm<'_> {
    fn actor_exists(&self, a: ObjId) -> bool {
        self.vm.objects.get(a).is_some_and(|o| o.is_some())
    }
    fn actor_ticking(&self, a: ObjId) -> bool {
        self.vm.ticking(a)
    }
    fn actor_ticked(&self, a: ObjId) -> bool {
        self.vm.is_ticked(a)
    }
    fn tick_actor(&mut self, a: ObjId, postponed: &mut Vec<ObjId>) -> bool {
        self.vm.actor_tick(a, self.dt, self.tick_type, postponed)
    }
    #[inline]
    fn preload(&self, object_value: Option<ObjId>, properties: Option<ObjId>) {
        #[cfg(target_arch = "x86_64")]
        {
            use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            if let Some(o) = object_value.and_then(|a| self.vm.objects.get(a)) {
                // SAFETY: prefetch of a valid address, without reading or writing.
                unsafe { _mm_prefetch::<_MM_HINT_T0>(o as *const _ as *const i8) };
            }
            if let Some(Some(o)) = properties.and_then(|a| self.vm.objects.get(a)) {
                let p = o.props.first_case();
                if !p.is_null() {
                    unsafe {
                        _mm_prefetch::<_MM_HINT_T0>(p as *const i8);
                        _mm_prefetch::<_MM_HINT_T0>((p as *const i8).wrapping_add(64));
                    }
                }
            }
        }
    }
    fn created(&mut self) -> (Vec<ObjId>, Vec<ObjId>) {
        (std::mem::take(&mut self.vm.spawn_order), std::mem::take(&mut self.vm.newly_spawned))
    }
    fn player_with_player(&self, a: ObjId) -> bool {
        self.vm.is(a, crate::flags_classes::D::PlayerController) && self.vm.native_object(a, Field::Player).is_some()
    }
    fn always_ticks(&self, a: ObjId) -> bool {
        self.vm.pb(a, "bAlwaysTick")
    }
    fn tick_player_pause(&mut self, a: ObjId) {
        self.vm.player_controller_input_paused(a, self.dt);
    }
}

/// ULevel::Tick's time: DeltaSeconds × TimeDilation as a float; without Pauser,
/// TimeSeconds (double) + DeltaSeconds rounded to 24 bits by the FPU; bPlayersOnly brings the tick
/// type back to ViewportsOnly; the step passed to the actors is clamped to [0.0005; 0.4] (up to NaN: 0.4,
/// since the unordered comparison sets C0, C2 and C3 to 1).
pub(crate) struct AdvanceTime {
    pub dt_dilate: f32,
    pub time_seconds: f64,
    pub tick_type: u8,
    pub dt_tick: f32,
}

pub(crate) fn advance_time(time_seconds: f64, tick_type: u8, dt: f32, dilation: f32, paused: bool, players_only: bool) -> AdvanceTime {
    let dt_dilate = dt * dilation;
    // A NaN operand comes out of the FPU as is (sign included).
    let time_seconds = if paused {
        time_seconds
    } else if dt_dilate.is_nan() {
        dt_dilate as f64
    } else {
        ((time_seconds + dt_dilate as f64) as f32) as f64
    };
    let dt_tick = if dt_dilate < 0.0005 {
        0.0005
    } else if dt_dilate < 0.4 {
        dt_dilate
    } else {
        0.4
    };
    AdvanceTime { dt_dilate, time_seconds, tick_type: if players_only { LEVELTICK_VIEWPORTS_ONLY } else { tick_type }, dt_tick }
}

/// The LevelInfo date fields that ULevel::UpdateTime fills: appSystemTime gives
/// Year, Month, DayOfWeek, Day, Hour, Minute, Second, Millisecond, stored under their names.
pub(crate) fn fields_time(sys: [i32; 8]) -> [(&'static str, i32); 8] {
    [("Year", sys[0]), ("Month", sys[1]), ("DayOfWeek", sys[2]), ("Day", sys[3]), ("Hour", sys[4]), ("Minute", sys[5]), ("Second", sys[6]), ("Millisecond", sys[7])]
}

