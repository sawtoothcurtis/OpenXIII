//! Viewports of the client (UClient / UWindowsClient) and local players: the PC engine's split screen.
//!
//! UGameEngine::Init creates the player's viewport, then three others with
//! UGameEngine::CreateNewViewport, sets Client+0x78 (number of wanted viewports) to 1
//! and calls UClient::ActiveViewports, which puts the extra viewports into a
//! reserve (+0x7C). SETVIEWPORTNUMBERFORNEXTMAP (UWindowsViewport::Exec) changes
//! +0x78; UGameEngine::LoadMap calls ActiveViewports again then creates one player per viewport
//! ("Spawning new actor for Viewport %s", ULevel::SpawnPlayActor).
//!
//! The client's state survives map changes (EngineState); the UViewport objects
//! ("WindowsViewportN" of WindowsClient0) are those of the current map.

use crate::value::{ObjId, Value};
use crate::Vm;

/// The CRT's __ftol: truncation on 64 bits (FISTP), then the low word; out of range or NaN, the indefinite
/// 0x8000000000000000 gives 0.
/// UClient::PostEditChange, called by UWindowsClient::PostEditChange at startup after reading
/// [WinDrv.WindowsClient]: Brightness (+0x50) brought into [0, 1] and MipFactor (+0x5C) into [-3, 3], through the
/// x87 comparisons (a NaN gives the upper bound).
pub fn client_post_edit_change(brightness: f32, mip_factor: f32) -> (f32, f32) {
    let bound = |v: f32, lo: f32, hi: f32| if v < lo { lo } else if v < hi { v } else { hi };
    (bound(brightness, 0.0, 1.0), bound(mip_factor, -3.0, 3.0))
}

fn ftol(x: f32) -> i32 {
    let t = f64::from(x);
    if t.is_nan() || t.abs() >= 9.223372036854775808e18 {
        0
    } else {
        (t as i64) as i32
    }
}

/// State produced by UViewport::Lock for a viewport, independently of the GPU backend.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ViewportLockState {
    pub current_time: f64,
    pub delta: f32,
    pub hit: bool,
    pub render_flags: u32,
    pub frame_count: u32,
    pub interface: bool,
}

/// Arguments of the logical boundary RenDev::Lock, after the viewport's writes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewportLockCall {
    pub viewport: ObjId,
    pub hit_data: bool,
    pub hit_size: bool,
    pub state: ViewportLockState,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewportLockOutcome {
    pub accepted: bool,
    pub delta: f32,
}

/// Initial request built from the real loaded dimensions and the player's viewport.
#[derive(Debug, Clone, Copy)]
pub struct InitialWindowRequest {
    pub viewport: ObjId,
    pub windowed_size: (i32, i32),
    pub requested_size: (i32, i32),
    pub fullscreen: bool,
}

/// Answer of the real initial host: result and dimensions actually accepted.
#[derive(Debug, Clone, Copy, Default)]
pub struct InitialWindowReply {
    pub result: u32,
    pub accepted_size: Option<(i32, i32)>,
}

impl From<u32> for InitialWindowReply {
    fn from(result: u32) -> Self { Self { result, accepted_size: None } }
}

/// UWindowsClient state kept from one map to the next.
#[derive(Debug, Clone)]
pub struct ClientState {
    /// The layout chosen for this game. Rendering quality still comes from the local device.
    pub session_graphics: crate::video_settings::SessionGraphics,
    /// +0x78: number of wanted viewports (SETVIEWPORTNUMBERFORNEXTMAP).
    pub wanted: i32,
    /// +0x2C Viewports: numbers of the active viewports, in order.
    pub viewports: Vec<u8>,
    /// +0x7C: reserve of inactive viewports.
    pub reserve: Vec<u8>,
    /// +0x98 / +0x9C origin and +0xA0 / +0xA4 size of each viewport, as fractions of the client.
    pub fractions: [[f32; 4]; 4],
    /// EXTENDVIEWPORTBEFOREMENU: the console's viewport takes the whole client, the others a
    /// zero size, until RESTOREVIEWPORTAFTERMENU.
    pub extended: Option<u8>,
    /// Viewport+0x1C4: gamepad (enumeration number) of each active viewport, AttribPadToViewport.
    pub viewport_pads: Vec<Option<usize>>,
    /// Game window open (UWindowsViewport::OpenWindow, "Opened viewport"): UGameEngine::Init
    /// only opens it after the first LoadMap; before that, the viewport's SizeX and SizeY are zero.
    pub window_open: bool,
    /// Improved build: players on this screen for the next multiplayer games (split_versus.rs);
    /// 1 or less: the original game.
    pub local_players: u8,
    /// Improved build: gamepad held by each local player of a split screen, by identity, in
    /// viewport order (split_versus.rs); kept from map to map.
    pub pad_owners: Vec<Option<u64>>,
    /// Improved build: the co-op character screen is on (character_screen.rs). That screen has
    /// no original equivalent, so its two views are always side by side, whatever the window's
    /// shape, the Co-op page's Split setting or Graphics: Original.
    pub character_screen_side_by_side: bool,
    /// Network co-op in lockstep (lockstep.rs): each player's own screen size, in the order of
    /// `viewports`; every viewport then has its player's whole screen.
    pub lockstep_sizes: Option<Vec<(i32, i32)>>,
    /// Improved build: refresh rate (whole hertz) of the screen showing the game window, set by
    /// the viewer; None without a window (dedicated server, captures, tests). An enhanced LAN
    /// host draws at it (Vm::lan_host_tick_rate).
    pub display_hz: Option<u32>,
    /// Request consumed once by the Viewer host; separate from the historical GUI flag.
    initial_window: Option<InitialWindowRequest>,
    /// WindowedViewportX/Y pair actually loaded, missing if incomplete/invalid.
    loaded_windowed_size: Option<(i32, i32)>,
    /// States of the real Lock, one per viewport object (not a global surface counter).
    pub(crate) viewport_locks: std::collections::HashMap<ObjId, ViewportLockState>,
    /// Last rectangle produced for each of the four viewports. The no-op of
    /// UpdateSizeAndOffset keeps it, even when the client's size changes.
    viewport_rects: [std::cell::Cell<[i32; 4]>; 4],
}

impl Default for ClientState {
    /// UGameEngine::Init: WindowsViewport0 then CreateNewViewport ×3 (WindowsViewport1 to 3), +0x78
    /// = 1, ActiveViewports: the last three go into reserve, the last one first.
    fn default() -> Self {
        let mut c = ClientState { loaded_windowed_size: None, initial_window: None, window_open: false, viewport_rects: std::array::from_fn(|_| std::cell::Cell::new([0; 4])), viewport_locks: std::collections::HashMap::new(), wanted: 1, viewports: vec![0, 1, 2, 3], reserve: Vec::new(), fractions: [[0.0, 0.0, 0.5, 0.5]; 4], extended: None, viewport_pads: Vec::new(), local_players: 1, pad_owners: Vec::new(), character_screen_side_by_side: false, session_graphics: Default::default(), lockstep_sizes: None, display_hz: None };
        c.active_viewports();
        c
    }
}

impl ClientState {
    pub(crate) fn set_loaded_windowed_size(&mut self, size: Option<(i32, i32)>) {
        self.loaded_windowed_size = size;
    }

    pub fn initial_window_request(&self) -> Option<InitialWindowRequest> {
        self.initial_window
    }

    /// UClient::ActiveViewports: viewports taken from the reserve (at the start) or given back
    /// to the reserve (the end of the list) up to +0x78, then fractions depending on their number: whole
    /// client; top and bottom; or quarters (top left, top right, bottom left, bottom right).
    pub fn active_viewports(&mut self) {
        let wanted = self.wanted.clamp(1, 4) as usize;
        while self.viewports.len() < wanted && !self.reserve.is_empty() {
            let v = self.reserve.remove(0);
            self.viewports.push(v);
        }
        while self.viewports.len() > wanted {
            if let Some(v) = self.viewports.pop() {
                self.reserve.push(v);
            }
        }
        let layout: &[[f32; 4]] = match self.viewports.len() {
            1 => &[[0.0, 0.0, 1.0, 1.0]],
            2 => &[[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]],
            _ => &[[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5], [0.0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]],
        };
        for (i, &v) in self.viewports.iter().enumerate() {
            if let Some(f) = layout.get(i) {
                self.fractions[v as usize] = *f;
            }
        }
    }

    /// Lockstep co-op: screen size of the player of viewport `slot`.
    pub fn lockstep_size(&self, slot: u8) -> Option<(i32, i32)> {
        let sizes = self.lockstep_sizes.as_ref()?;
        let i = self.viewports.iter().position(|&v| v == slot)?;
        sizes.get(i).copied()
    }

    /// UWindowsViewport::UpdateSizeAndOffset: origin (+0x90, +0x94) and size
    /// (+0x88, +0x8C) in pixels, fractions × client size truncated; nothing if a size
    /// fraction is zero. During EXTENDVIEWPORTBEFOREMENU, the console has the whole client and the
    /// other viewports a zero size.
    /// The last rectangle produced for viewport `index` (zero before the first).
    pub(crate) fn last_viewport_rect(&self, index: usize) -> [i32; 4] {
        self.viewport_rects.get(index).map_or([0; 4], |c| c.get())
    }

    pub fn viewport_rect(&self, slot: u8, client: (i32, i32)) -> [i32; 4] {
        // Lockstep co-op: each player's view is their own whole screen, on every machine.
        if let Some(size) = self.lockstep_size(slot) {
            let rect = [0, 0, size.0, size.1];
            self.viewport_rects[slot as usize & 3].set(rect);
            return rect;
        }
        match self.extended {
            Some(s) if s == slot => return [0, 0, client.0, client.1],
            Some(_) => return [0, 0, 0, 0],
            None => {}
        }
        let index = slot as usize & 3;
        let mut f = self.fractions[index];
        if !(f[2] > 0.0 && f[3] > 0.0) {
            return self.viewport_rects[index].get();
        }
        // Improved build (split_layout.rs): two views side by side on a wide screen, and always
        // on the co-op character screen.
        let side_by_side = self.character_screen_side_by_side || crate::split_layout::side_by_side(client.0, client.1, self.session_graphics);
        if self.viewports.len() == 2 && self.viewports.contains(&slot) && side_by_side {
            f = crate::split_layout::side_by_side_fraction(f);
        }
        let (w, h) = (client.0 as f32, client.1 as f32);
        let rect = [ftol(f[0] * w), ftol(f[1] * h), ftol(f[2] * w), ftol(f[3] * h)];
        self.viewport_rects[index].set(rect);
        rect
    }
}

impl Vm {
    /// Initial producer shared by spawn_play_actors and its tests, at the same logical point.
    fn prepare_initial_window_request(&mut self, viewport: ObjId) {
        self.engine.client.initial_window = Some(InitialWindowRequest {
            viewport, windowed_size: self.engine.client.loaded_windowed_size.unwrap_or(self.viewport),
            requested_size: self.viewport, fullscreen: self.fullscreen,
        });
    }

    /// Same logical site as before: after the first player and before the secondary GUIs.
    fn prepare_first_window(&mut self) {
        // UGameEngine::Init: the game window opens after the first map's player
        // ("Opened viewport", SetRes); before that, GETCURRENTRES returns "0x0". Before it,
        // UViewport::InitInput → UInput::Init: "Input system
        // initialized for %s" (NAME_Init).
        if !self.engine.client.window_open {
            if let Some(name) = self.active_viewport_objects().first().and_then(|&v| self.objects[v].as_ref()).map(|o| o.name.clone()) {
                self.log.push(crate::player_input::InputSystem::init_log(&name));
            }
            // Initial open request: the host consumes the real parameters in resumed.
            if let Some(viewport) = self.active_viewport_objects().first().copied() {
                self.prepare_initial_window_request(viewport);
            }
            // Historical publication/log kept until the GUI/script order is proven.
            self.log.push("Log: Opened viewport".into());
        }
        self.engine.client.window_open = true;
    }

    /// Consumes the real request (OpenWindow).
    /// The presence of the window/device comes from the host, never from window_open (published earlier).
    pub fn open_initial_window<E, R: Into<InitialWindowReply>>(
        &mut self,
        has_window: bool,
        has_renderer: bool,
        host: &mut dyn FnMut(crate::resize::OpenHostCall) -> Result<R, E>,
    ) -> Result<bool, E> {
        let Some(request) = self.engine.client.initial_window else { return Ok(false) };
        let actor = self.propr(request.viewport, "Actor").as_object().is_some();
        let mut state = crate::resize::OpenViewport {
            actor, has_hwnd: has_window, rendev: has_renderer,
            client_f94: request.fullscreen, sizes: [request.windowed_size.0, request.windowed_size.1],
            // Improved build: a window size chosen in the Video menu and saved in XIII.ini (16:9
            // included) opens again as it is, as storage/load.rs already decided for the viewport.
            keep_size: crate::improved::storage::window_size_kept(request.windowed_size.0, request.windowed_size.1, self.desktop),
            // The first call comes before the frames: no Lock in progress and no temporary mode.
            ..Default::default()
        };
        // Real caller Init: parent 0, temporary 0, NewX/Y=-1 (platform positions).
        let viewport_size = &mut self.viewport;
        self.res_table.try_open_window(&mut state, 0, 0, -1, -1, self.desktop, None, false, &mut |call| {
            let device_call = matches!(&call, crate::resize::OpenHostCall::Call(crate::resize::OpenCall::RenderDevice(..)));
            let reply: InitialWindowReply = host(call)?.into();
            // The native callee publishes its SizeX/Y on success. Here the same data comes from
            // the real Window, after the device is accepted, before Gamma/UpdateFrame.
            if device_call && reply.result != 0 {
                if let Some(size) = reply.accepted_size { *viewport_size = size; }
            }
            Ok::<u32, E>(reply.result)
        })?;
        self.engine.client.initial_window = None;
        Ok(true)
    }

    /// UViewport object "WindowsViewportN" of WindowsClient0 for this map (created on demand,
    /// like UWindowsClient::NewViewport).
    pub(crate) fn viewport_object(&mut self, slot: u8) -> Option<ObjId> {
        if let Some(o) = self.find_viewport_object(slot) {
            return Some(o);
        }
        let name = format!("WindowsViewport{slot}");
        let client = "Transient.WindowsClient0";
        let class = self.class("Player")?;
        Some(self.spawn_in(class, client.to_string(), Some(name)))
    }

    /// "WindowsViewportN" object already created for this map.
    pub fn find_viewport_object(&self, slot: u8) -> Option<ObjId> {
        let name = format!("WindowsViewport{slot}");
        let is = |o: ObjId| self.objects.get(o).and_then(|x| x.as_ref()).is_some_and(|x| x.outer == "Transient.WindowsClient0" && x.name == name);
        if let Some((s, o)) = self.viewport_found.get() {
            if s == slot && is(o) {
                return Some(o);
            }
        }
        let r = (0..self.objects.len()).find(|&o| is(o));
        self.viewport_found.set(r.map(|o| (slot, o)));
        r
    }

    /// Active viewports of the client (Client->Viewports): their objects, in order.
    pub fn active_viewport_objects(&mut self) -> Vec<ObjId> {
        let slots = self.engine.client.viewports.clone();
        slots.into_iter().filter_map(|s| self.viewport_object(s)).collect()
    }

    /// UGameEngine::LoadMap: ActiveViewports: one
    /// player per active viewport (GLevel->SpawnPlayActor, role 2), in viewport order.
    /// The first one receives the travel inventory `travel`.
    pub fn spawn_play_actors(&mut self, game: ObjId, portal: &str, options: &str, travel: Option<&str>) -> crate::Result<Vec<ObjId>> {
        let _math = self.session.math.enter();
        if self.coop.on {
            self.engine.client.wanted = self.engine.client.wanted.max(2);
        }
        self.coop_begin_logins();
        self.coop.spawn_portal = portal.to_string();
        // Split-screen multiplayer (split_versus.rs): one viewport per local player.
        self.versus_before_logins(game);
        self.engine.client.active_viewports();
        // End of ActiveViewports: AttribPadToViewport (vtable 0x9C).
        self.attrib_pad_to_viewport();
        // End of UGameEngine::LoadMap: "End LoadMap: %s". On the first map (UGameEngine::Init),
        // the players come after LoadMap; on the next ones, LoadMap creates them before finishing.
        let first = !self.engine.client.window_open;
        if first {
            self.log.push(format!("Log: End LoadMap: {}", self.url.string()));
        }
        let mut players = Vec::new();
        // Co-op (coop.rs): a map reached with the travel inventory gives each player their own
        // inventory of the previous map (the rule of that map, NextMapKeepInventory, applied to
        // every player: coop_story.rs), not the first player's.
        let carried = std::mem::take(&mut self.coop.carried);
        self.coop.arrived = if self.coop.on && travel.is_some() { carried } else { Vec::new() };
        for (i, viewport) in self.active_viewport_objects().into_iter().enumerate() {
            let name = self.objects[viewport].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("Log: Spawning new actor for Viewport {name}"));
            let own = if self.coop.on { self.coop.arrived.get(i).cloned().flatten() } else { None };
            let chosen = match (&own, i) {
                (Some(t), _) => Some(t.as_str()),
                (None, 0) => travel,
                (None, _) => None,
            };
            let info = crate::world::travel_info_named(options, chosen);
            // Local co-op (coop.rs): the other players spawn on the first one's start.
            let extra = if self.coop.on && i > 0 { Some(self.coop_before_extra_login(game)?) } else { None };
            let pc = self.login_viewport(game, portal, options, &info, Some(viewport), 2)?;
            if let Some(saved) = extra {
                self.coop_after_extra_login(game, saved, pc)?;
            }
            if let Some(pc) = pc {
                self.coop_player_logged(pc);
                if self.coop.on {
                    self.coop.own_travel.resize(i + 1, false);
                    self.coop.own_travel[i] = own.is_some();
                }
                players.push(pc);
            }
        }
        // Local co-op (coop_enemies.rs): extra enemies, once the players are there.
        if self.coop.on {
            self.coop_attach_held_weapons();
            self.coop_enemies_start();
        }
        // A VM given its sound subsystem before the map (a lockstep session's start map: the
        // engine kept it from the menus) ends LoadMap with Audio, as a later map does.
        let audio_kept = first && self.audio.is_some();
        if !first || audio_kept {
            self.audio_end_load_map();
        }
        if !first {
            self.log.push(format!("Log: End LoadMap: {}", self.url.string()));
        }
        self.prepare_table_names();
        self.prepare_first_window();
        // UGameEngine::Init: after the player of the first viewport,
        // CreateNewViewport opens the windows WindowsViewport1 to 3 (GUIController and
        // InitializeController: "CREATING ROOT WINDOW: …XIIIRootWindowN"), which ActiveViewports
        // then puts into reserve.
        for slot in 1..=3u8 {
            if self.find_viewport_object(slot).is_none() {
                if let Some(v) = self.viewport_object(slot) {
                    self.open_viewport_window(v);
                    // CreateNewViewport: InitInput after the window (UInput::Init).
                    self.log.push(crate::player_input::InputSystem::init_log(&format!("WindowsViewport{slot}")));
                }
            }
        }
        if first && !audio_kept {
            self.audio_set_viewports();
        }
        Ok(players)
    }

    /// LoadMap with Audio (later maps), after the players: CleanUp and SetViewport,
    /// music of a checkpoint given back to the map (UAudioSaver, only with Audio), then LoadSounds ("SOUND : mem used").
    /// The UAudioSaver is read again after AcceptInventory: the PlayMusic it does during
    /// SpawnPlayActor has written the name of the checkpoint's music into it.
    /// UGameEngine::LoadMap of a network client (pending level): no SpawnPlayActor
    /// (spawns nobody), but the block that follows (
    /// under the sole condition that Audio is non-null) also runs: CleanUp, SetViewport of each
    /// viewport, UAudioSaver, LoadSounds. Without it, the CleanUp at the start of LoadMap
    /// (subsystem kept since the menu) left the client without a microphone or sound.
    pub fn net_client_end_load_map(&mut self) {
        if self.audio.is_some() {
            self.audio_end_load_map();
        }
    }

    pub(crate) fn audio_end_load_map(&mut self) {
        self.audio_set_viewports();
        if self.audio.is_some() {
            self.apply_audio_saver();
        }
        self.audio_load_sounds();
    }

    /// Number (0 to 3) of the viewport of player `pc` (its Player), if it has one.
    pub fn viewport_slot_of(&self, pc: ObjId) -> Option<u8> {
        // Controller already destroyed (the menu's, destroyed at a network client's LoadMap, stays the
        // owner of the UInput until HandleClientPlayer): no viewport.
        self.objects.get(pc)?.as_ref()?;
        let player = self.propr(pc, "Player").as_object()?;
        let name = self.objects.get(player)?.as_ref()?.name.clone();
        name.strip_prefix("WindowsViewport")?.parse().ok()
    }

    /// Horizontal field of view of player `pc`'s picture, `fov` being its FovAngle and (w, h)
    /// its view's size: in local split screen (coop.rs) the old co-op mod's view, 110° across a
    /// 32:9 half; `fov` otherwise. The scene and Console.WorldToScreen (solo_native.rs) use the
    /// same angle, so what the HUD places over the world lines up with the picture.
    pub(crate) fn split_view_angle(&self, pc: Option<ObjId>, fov: f32, w: u32, h: u32) -> f32 {
        let solo_fov = false;
        if self.local_split() && !solo_fov {
            let default_fov = pc.map(|p| self.propr(p, "DefaultFOV").as_float()).filter(|&f| f > 0.0).unwrap_or(85.0);
            crate::coop::split_view_fov_of(fov, default_fov, w, h, crate::display_format::active(self.engine.client.session_graphics))
        } else {
            fov
        }
    }

    /// Rectangle (x, y, width, height) of the viewport of player `pc` in a client of size
    /// `client`.
    pub fn viewport_rect_of(&self, pc: ObjId, client: (i32, i32)) -> [i32; 4] {
        match self.viewport_slot_of(pc) {
            Some(s) => self.engine.client.viewport_rect(s, client),
            None => [0, 0, client.0, client.1],
        }
    }

    /// Split-screen viewport commands (UWindowsViewport::Exec); `None` if `word` is not one of them.
    pub(crate) fn split_screen_exec(&mut self, word: &str, rest: &str) -> Option<String> {
        match word {
            // atoi of the rest into Client+0x78, used at the next LoadMap.
            "SETVIEWPORTNUMBERFORNEXTMAP" => {
                let chars: Vec<char> = rest.chars().collect();
                self.engine.client.wanted = crate::config::atoi(&chars, 0);
            }
            // No effect if +0x78 is 1; otherwise the console's viewport (the first) takes the whole
            // window, the others UpdateSizeAndOffset(0, 0).
            "EXTENDVIEWPORTBEFOREMENU" => {
                if self.engine.client.wanted != 1 {
                    self.engine.client.extended = self.engine.client.viewports.first().copied();
                }
            }
            // No effect if +0x78 is 1; otherwise each viewport takes back its fractions.
            "RESTOREVIEWPORTAFTERMENU" => {
                if self.engine.client.wanted != 1 {
                    self.engine.client.extended = None;
                }
            }
            _ => return None,
        }
        Some(String::new())
    }

    /// UGameEngine::Draw of a player: PlayerCalcView from their position and
    /// rotation; ViewActor (the player if None), position and rotation of the camera.
    pub fn player_calc_view(&mut self, pc: ObjId) -> Option<(Option<ObjId>, [f32; 3], [i32; 3])> {
        let f = self.find_function(pc, "PlayerCalcView", false)?;
        let args = vec![Some(Value::Object(Some(pc))), Some(self.prop(pc, "Location")), Some(self.prop(pc, "Rotation"))];
        match self.call(f, pc, args) {
            Ok((_, outs)) => {
                let actor = outs.first().cloned().flatten().and_then(|v| v.as_object());
                Some((actor, outs.get(1).cloned().flatten()?.as_vector(), outs.get(2).cloned().flatten()?.as_rotator()))
            }
            Err(e) => {
                self.warn(format!("PlayerCalcView: {e}"));
                None
            }
        }
    }

    /// One client frame: UGameEngine::Draw for each active viewport (UWindowsClient::Tick: Repaint vt 0xBC of each viewport, in the order of Client->Viewports),
    /// with the size of its rectangle (UpdateSizeAndOffset): PlayerCalcView of Viewport->Actor,
    /// HUD of this player (PostRender) and scene (FCameraSceneNode, visibility). A viewport of
    /// zero size (during EXTENDVIEWPORTBEFOREMENU) is not drawn (Lock refused).
    pub fn draw_viewports(&mut self, client: (i32, i32)) -> Vec<ViewportFrame> {
        let slots = self.engine.client.viewports.clone();
        slots.into_iter().filter_map(|slot| self.draw_viewport(slot, client)).collect()
    }

    /// UGameEngine::Draw of viewport `slot` (see `draw_viewports`); `None` if it is not
    /// drawn (zero size).
    pub fn draw_viewport(&mut self, slot: u8, client: (i32, i32)) -> Option<ViewportFrame> {
        // Split-screen multiplayer (split_versus.rs): the local players' shared state.
        if self.coop.versus {
            self.versus_frame();
        }
        if !self.coop.on {
            return self.draw_viewport_frame(slot, client);
        }
        // Local co-op (coop_combat.rs): the teammates are not aiming targets while this
        // player's view and HUD are made.
        let viewer = self.viewport_object(slot).and_then(|v| self.propr(v, "Actor").as_object());
        self.coop_gunners_for_view(viewer);
        crate::improved::presentation::prepare_character_view(self, slot);
        let hidden = self.coop_hide_teammate_targets(viewer);
        let frame = self.draw_viewport_frame(slot, client);
        self.coop_restore_teammate_targets(hidden);
        frame
    }

    fn draw_viewport_frame(&mut self, slot: u8, client: (i32, i32)) -> Option<ViewportFrame> {
        // Lockstep co-op: the player's own screen.
        let client = self.engine.client.lockstep_size(slot).unwrap_or(client);
        self.client_size = client;
        self.display_window = client;
        // UGameEngine::Draw: nothing to do while a video plays (VideoPlayerDevice +0x2C == 1).
        if self.engine.video.status == 1 {
            return None;
        }
        let viewport = self.viewport_object(slot)?;
        if self.coop.on {
            self.coop_sync_intro();
        }
        let rect = self.engine.client.viewport_rect(slot, client);
        // UGameEngine::Draw: PlayerCalcView happens before UViewport::Lock, which the zero size makes fail;
        // the frame is then lost: PrecacheOnFlip (RenDev+0x64) reset to 0 unless bSuspendPrecaching.
        let suspended = self.propr(viewport, "bSuspendPrecaching").as_bool();
        if rect[2] <= 0 || rect[3] <= 0 {
            if let Some(pc) = self.propr(viewport, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some())) {
                let _ = self.player_calc_view(pc);
            }
            if !suspended {
                self.precache_on_flip = false;
            }
            return None;
        }
        // bSuspendPrecaching (Player +0x38 bit 2): neither a precache frame nor LevelAction 4, PrecacheOnFlip stays pending.
        let held = suspended && std::mem::take(&mut self.precache_on_flip);
        // UViewport::Lock: CurrentTime and difference with the previous frame (+0x148).
        let lock = self.viewport_lock_device(viewport, false, false, |_| true);
        if !lock.accepted { return None; }
        let delta = lock.delta;
        let pc = self.propr(viewport, "Actor").as_object().filter(|&p| self.objects.get(p).is_some_and(|o| o.is_some()));
        self.current_viewport = Some(viewport);
        let mut view = pc.and_then(|pc| self.player_calc_view(pc));
        let every_slot = false;
        if let (true, Some(v), Some((location, rotation))) = (slot == 0 || every_slot, view.as_mut(), forced_view()) {
            v.1 = location;
            v.2 = rotation;
        }
        if let (Some(v), Some((location, rotation))) = (view.as_mut(), crate::improved::presentation::character_camera(self, slot)) {
            v.0 = None;
            v.1 = location;
            v.2 = rotation;
        }
        let (w, h) = (rect[2] as u32, rect[3] as u32);
        let frame = self.hud_begin(pc, w, h);
        // FCameraSceneNode receives Viewport->Actor.FovAngle (+0x1F8) as is:
        // UpdateMatrices does not clamp it.
        let fov = pc.map(|p| self.propr(p, "FovAngle").as_float()).unwrap_or(85.0);
        let fov = self.split_view_angle(pc, fov, w, h);
        let (fov, scene, lens, cull_fov) = crate::improved::presentation::scene_framing(self, slot, rect, fov);
        let hidden_projectors = crate::improved::presentation::hidden_projectors(self, slot);
        let scene_size = scene.map_or((w, h), |r| (r[2].max(1) as u32, r[3].max(1) as u32));
        if let (Some(pc), Some((actor, location, rotation))) = (pc, view) {
            // UGameEngine::Draw: ViewActor null after PlayerCalcView → Viewport->Actor.
            // Hor+ (display_format.rs) on a whole-window view only: split_view_fov already sets the
            // split-screen views' angle (view_widen_window). The character screen's panel keeps
            // the recorded view's angle, its visibility over the moved picture (cull_fov ≥ fov).
            let node = if scene.is_some() {
                self.camera_scene(Some(pc), actor.or(Some(pc)), location, rotation, cull_fov, scene_size)
            } else {
                self.camera_scene_view(Some(pc), actor.or(Some(pc)), location, rotation, fov, scene_size)
            };
            // CameraRender: OnlyPostRender skips the scene,
            // like the effects below, but keeps the Canvas and the HUD.
            if !self.only_post_render {
                self.render_scene(&node);
                self.scene_bsp_views = self.bsp_views.clone();
            }
        }
        // FCameraSceneNode::Render, after the scene (viewport+0x120 zero): frame shake and
        // colour filter of Viewport->Actor (the flash is read by the viewer).
        let filter = match pc {
            Some(pc) if !self.only_post_render => {
                self.camera_shake_frame(pc);
                self.camera_filter(pc, delta)
            }
            _ => None,
        };
        let precaching = frame.is_some_and(|frame| self.hud_end(frame));
        let mut filter = filter;
        if self.precache_wanted(precaching) {
            // Second frame (precache): new Lock, same viewpoint, full Render.
            let lock = self.viewport_lock_device(viewport, false, false, |_| true);
            if !lock.accepted { return None; }
            let delta = lock.delta;
            self.precache_render = true;
            let frame = self.hud_begin(pc, w, h);
            if let (Some(pc), Some((actor, location, rotation))) = (pc, view) {
                let node = self.camera_scene_view(Some(pc), actor.or(Some(pc)), location, rotation, fov, (w, h));
                if !self.only_post_render {
                    self.render_scene(&node);
                    self.scene_bsp_views = self.bsp_views.clone();
                }
            }
            filter = match pc {
                Some(pc) if !self.only_post_render => {
                    self.camera_shake_frame(pc);
                    self.camera_filter(pc, delta)
                }
                _ => None,
            };
            if let Some(frame) = frame {
                let _ = self.hud_end(frame);
            }
            self.precache_render = false;
        }
        self.current_viewport = None;
        if held {
            self.precache_on_flip = true;
        }
        let anchors = self.hud_anchors_take();
        Some(ViewportFrame { viewport, player: pc, rect, scene, view, fov, lens, hidden_projectors, filter, hud: std::mem::take(&mut self.hud), anchors })
    }
}

/// None outside the plan.
pub static VIEW_PLAN: std::sync::Mutex<Option<([f32; 3], [i32; 3])>> = std::sync::Mutex::new(None);

/// The pawn does not move; everything that depends on the player's view (sky, sprites, BSP
/// visibility, sinus, shadows, weather…) receives this view. Without the variable, nothing
/// changes.
pub fn forced_view() -> Option<([f32; 3], [i32; 3])> {
    if let Some(v) = *VIEW_PLAN.lock().unwrap_or_else(|e| e.into_inner()) {
        return Some(v);
    }
    return None;
}

/// What UGameEngine::Draw produced for a viewport.
#[derive(Debug, Clone)]
pub struct ViewportFrame {
    pub viewport: ObjId,
    pub player: Option<ObjId>,
    /// Origin and size in pixels within the client.
    pub rect: [i32; 4],
    /// Where the scene is drawn when it is not the whole rectangle (pixels within the client;
    /// improved build: the character screen's live-view panel), `fov` being across it.
    pub scene: Option<[i32; 4]>,
    /// ViewActor, camera position and rotation (PlayerCalcView).
    pub view: Option<(Option<ObjId>, [f32; 3], [i32; 3])>,
    pub fov: f32,
    /// Off-centre projection (normalised device units, x right, y up; improved build: the
    /// character screen's live view moved in its panel), else zero.
    pub lens: [f32; 2],
    /// Projectors not drawn in this view (improved build: on the co-op character screen, the
    /// other players' characters' shadows; the views are all made before any is drawn, so
    /// their bHidden of the moment does not tell).
    pub hidden_projectors: Vec<ObjId>,
    /// FCameraSceneNode::Render: colour (B, G, R, A) of the DrawTile of the whole viewport in
    /// FB_Modulate on WhiteTexture, after the scene and before the screen effects and the HUD, if
    /// PlayerController.FilterColor is not neutral grey (see `Vm::camera_filter`).
    pub filter: Option<[u8; 4]>,
    /// Canvas primitives of this viewport, in viewport pixels.
    pub hud: Vec<crate::canvas::HudDraw>,
    /// Lockstep co-op: the HUD's icons over a world point (lockstep_anchor.rs).
    pub anchors: Vec<crate::lockstep_anchor::HudAnchor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_viewports_like_uclient() {
        let mut c = ClientState::default();
        assert_eq!(c.viewports, vec![0]);
        assert_eq!(c.reserve, vec![3, 2, 1]);
        assert_eq!(c.fractions[0], [0.0, 0.0, 1.0, 1.0]);
        c.wanted = 2;
        c.active_viewports();
        // The first one in the reserve (WindowsViewport3) becomes the second viewport, at the bottom.
        assert_eq!(c.viewports, vec![0, 3]);
        assert_eq!(c.fractions[0], [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(c.fractions[3], [0.0, 0.5, 1.0, 0.5]);
        assert_eq!(c.viewport_rect(3, (1920, 1200)), [0, 600, 1920, 600]);
        c.wanted = 4;
        c.active_viewports();
        assert_eq!(c.viewports, vec![0, 3, 2, 1]);
        assert_eq!(c.viewport_rect(1, (800, 600)), [400, 300, 400, 300]);
        c.wanted = 1;
        c.active_viewports();
        assert_eq!(c.viewports, vec![0]);
        assert_eq!(c.reserve, vec![1, 2, 3]);
    }
}

/// What drawing one view leaves in the VM for its frame to be presented (the actors it drew
/// with their owner tests, places and lights, the BSP nodes, the display list, the HUD's
/// drawn actors and portals, the sine modifiers drawn, the render textures' views: the comic
/// panels its HUD updated). Lockstep co-op: every player's view is
/// drawn each step in the same order on every machine, the frame presented being this
/// machine's; its view's output is kept and put back only while that frame is presented.
#[derive(Clone, Default)]
pub struct ViewOutput {
    bsp_views: Vec<crate::bsp_draw::BspView>,
    scene_bsp_views: Vec<crate::bsp_draw::BspView>,
    display: crate::visibility::DisplayList,
    rendered: Vec<ObjId>,
    render_root: crate::types::HashMap<ObjId, ObjId>,
    render_place: crate::types::HashMap<ObjId, crate::render_placement::RenderPlacement>,
    light_selections: crate::types::HashMap<ObjId, std::rc::Rc<crate::visibility::SceneLightSelection>>,
    translucent_order: Vec<crate::visibility::Translucent>,
    canvas_rendered: Vec<ObjId>,
    canvas_actor_calls: Vec<(ObjId, bool, bool, f32)>,
    canvas_portal_calls: Vec<crate::PortalCall>,
    target_rendered: Vec<ObjId>,
    sine_drawn: Vec<(ObjId, String, f32, [f32; 3])>,
    viewport: (i32, i32),
    render_views: std::collections::HashMap<ObjId, Vec<crate::canvas::RenderView>>,
    /// LastRenderDist of each actor the view drew (set by its render, read by the shadows'
    /// update).
    render_dists: Vec<(ObjId, Value)>,
}

impl Vm {
    /// The output of the view just drawn (see `ViewOutput`).
    pub fn view_output(&self) -> ViewOutput {
        ViewOutput {
            bsp_views: self.bsp_views.clone(),
            scene_bsp_views: self.scene_bsp_views.clone(),
            display: self.display.clone(),
            rendered: self.rendered.clone(),
            render_root: self.render_root.clone(),
            render_place: self.render_place.clone(),
            light_selections: self.light_selections.clone(),
            translucent_order: self.translucent_order.clone(),
            canvas_rendered: self.canvas_rendered.clone(),
            canvas_actor_calls: self.canvas_actor_calls.clone(),
            canvas_portal_calls: self.canvas_portal_calls.clone(),
            target_rendered: self.target_rendered.clone(),
            sine_drawn: self.sine_drawn.clone(),
            viewport: self.viewport,
            render_views: self.render_views.clone(),
            render_dists: self.rendered.iter().chain(&self.canvas_rendered).chain(&self.target_rendered).copied().filter(|&a| self.alive(a)).map(|a| (a, self.prop(a, "LastRenderDist"))).collect(),
        }
    }

    /// Testing (lockstep co-op): what the view of `viewer` (a PlayerController) drew of each
    /// player: "VIEWCHECK own_body=… own_weapon=… other_body=… other_weapon=…" (1 = drawn by
    /// the scene or the HUD's DrawActor). The viewer's own body and another player's
    /// first-person weapon are never drawn in a correct view.
    pub fn lockstep_view_check(&self, viewer: Option<ObjId>) -> String {
        let drawn = |a: Option<ObjId>| a.is_some_and(|a| self.rendered.contains(&a) || self.canvas_rendered.contains(&a)) as u8;
        let pcs: Vec<ObjId> = self.actors.iter().copied().filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "PlayerController")).collect();
        let pawn = |pc: ObjId| self.propr(pc, "Pawn").as_object().filter(|&p| self.alive(p));
        let weapon = |pc: ObjId| pawn(pc).and_then(|p| self.propr(p, "Weapon").as_object()).filter(|&w| self.alive(w));
        let (mut ob, mut ow, mut xb, mut xw) = (0, 0, 0, 0);
        for pc in pcs {
            if Some(pc) == viewer {
                ob = drawn(pawn(pc));
                ow = drawn(weapon(pc));
            } else {
                xb = xb.max(drawn(pawn(pc)));
                xw = xw.max(drawn(weapon(pc)));
            }
        }
        let difficulty = self.level.and_then(|l| self.propr(l, "Game").as_object()).filter(|&g| self.alive(g)).map(|g| self.prop(g, "Difficulty"));
        format!("VIEWCHECK t={:.1} own_body={ob} own_weapon={ow} other_body={xb} other_weapon={xw} difficulty={difficulty:?}", self.level_time)
    }

    /// Testing (lockstep co-op): every shadow projector with its owner, place and hidden flag.
    pub fn lockstep_shadow_check(&self) -> String {
        let mut out = format!("SHADOWS t={:.1}", self.level_time);
        for &a in &self.actors {
            if !self.alive(a) || !self.is_a(a, "ShadowProjector") {
                continue;
            }
            let owner = self.propr(a, "Owner").as_object().filter(|&o| self.alive(o));
            let name = owner.and_then(|o| self.objects[o].as_ref().map(|x| x.name.to_string())).unwrap_or_default();
            let at = self.propr(a, "Location").as_vector();
            let oat = owner.map(|o| self.propr(o, "Location").as_vector()).unwrap_or([0.0; 3]);
            out.push_str(&format!(" | {name} at {:.0},{:.0} owner at {:.0},{:.0} hidden {}", at[0], at[1], oat[0], oat[1], self.propr(a, "bHidden").as_bool()));
        }
        out
    }

    /// Puts `o` in place of the VM's view output and returns the one it replaced (swap again
    /// with that to restore the VM as it was).
    pub fn swap_view_output(&mut self, mut o: ViewOutput) -> ViewOutput {
        std::mem::swap(&mut self.bsp_views, &mut o.bsp_views);
        std::mem::swap(&mut self.scene_bsp_views, &mut o.scene_bsp_views);
        std::mem::swap(&mut self.display, &mut o.display);
        std::mem::swap(&mut self.rendered, &mut o.rendered);
        std::mem::swap(&mut self.render_root, &mut o.render_root);
        std::mem::swap(&mut self.render_place, &mut o.render_place);
        std::mem::swap(&mut self.light_selections, &mut o.light_selections);
        std::mem::swap(&mut self.translucent_order, &mut o.translucent_order);
        std::mem::swap(&mut self.canvas_rendered, &mut o.canvas_rendered);
        std::mem::swap(&mut self.canvas_actor_calls, &mut o.canvas_actor_calls);
        std::mem::swap(&mut self.canvas_portal_calls, &mut o.canvas_portal_calls);
        std::mem::swap(&mut self.target_rendered, &mut o.target_rendered);
        std::mem::swap(&mut self.sine_drawn, &mut o.sine_drawn);
        std::mem::swap(&mut self.viewport, &mut o.viewport);
        std::mem::swap(&mut self.render_views, &mut o.render_views);
        // The actors' LastRenderDist as that view left them; the values replaced go back with
        // the output returned.
        for (a, v) in o.render_dists.iter_mut() {
            if self.alive(*a) {
                let old = self.prop(*a, "LastRenderDist");
                self.set_prop(*a, "LastRenderDist", std::mem::replace(v, old));
            }
        }
        o
    }
}

