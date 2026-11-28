//! "Play" mode: the virtual machine runs the map's logic; keyboard and
//! mouse go through the engine's input (UInput, PlayerInput, PlayerTick) and the camera is
//! the player controller's (PlayerCalcView), as in UGameEngine::Tick and Draw.

use openxiii_vm::NativeField as Field;
use crate::prop_geometry::{placement_bits, ActorGeometry, Pose, WorldGeometry};
use openxiii_assets::coords::Coords;
use openxiii_assets::world::Vertex;
use openxiii_vm::value::{ObjId, Value};
use openxiii_vm::Vm;
use std::path::Path;

/// Actor tables and sets read every frame: fast hashing (same content, no
/// result depends on their order).
type Table<K, V> = std::collections::HashMap<K, V, rustc_hash::FxBuildHasher>;
type Ens<T> = std::collections::HashSet<T, rustc_hash::FxBuildHasher>;

pub struct Game {
    pub vm: Vm,
    pub pawn: Option<ObjId>,
    pub controller: Option<ObjId>,
    /// Actors read from the map file: their StaticMeshes are in the static geometry.
    baked: Ens<ObjId>,
    /// Lowercase names of these actors.
    baked_names: Table<ObjId, String>,
    /// StaticMesh of these actors at load time (the one in the static geometry).
    baked_meshes: Table<ObjId, Value>,
    /// Location, Rotation, DrawScale, DrawScale3D and PrePivot of these actors at load time.
    baked_placements: Table<ObjId, [Value; 5]>,
    /// Loaded map (package name).
    pub map_name: String,
    /// Colour filter of the last frame (R, G, B, A), see `camera_filter`.
    pub color_filter: Option<[u8; 4]>,
    /// Last changed meshes built (`replaced_world`).
    replaced: Vec<(String, String)>,
    /// Actors and lighting of the map kept for the replaced scenery (`replaced_world_with`).
    base_replacements: Option<openxiii_assets::world::BaseReplacements>,
    /// `hidden_baked` sets kept as long as their inputs are the same.
    caches: std::cell::RefCell<CacheCaches>,
    /// Names of the light pass actors kept from one frame to the next (`scene_light_leaves`).
    names_lights: std::cell::RefCell<crate::lights_scene::Names>,
    /// Models already loaded (per mesh), to rebuild the list of drawn actors.
    models: std::cell::RefCell<std::collections::HashMap<String, Option<std::rc::Rc<Model>>>>,
    /// Last PlayerCalcView: ViewActor, position, rotation.
    view: Option<(Option<ObjId>, [f32; 3], [i32; 3])>,
    /// HXAudio's clock in game: HXAudio compares its due times with timeGetTime taken at Update,
    /// the PC's real time and not level time. Set by the window
    /// loop; when absent (captures, tests), sound follows level time.
    pub clock_hx: Option<(std::time::Instant, f64)>,
    /// Key bindings, for the controller a host gives later (network client).
    bindings: openxiii_vm::input::Bindings,
}

/// Prepare the same map packages for the renderer and the simulation during loading. A new VM of
/// the game (not of a test): the game thread's rand() seeded from the clock, as the original does
/// at every launch ([`Vm::seed_from_clock`]).
pub(crate) fn launch_seed(vm: &mut Vm) {
    vm.seed_from_clock();
}

fn warm_up_packages(vm: &mut Vm, library: &mut openxiii_package::library::Library) {
    library.preload_map(vm.packages.iter().map(|p| p.name.as_str()), vm.library.as_mut());
}

/// Command-line choices belonging to the game that is starting.
#[derive(Default)]
pub struct StartupOptions {
    pub math: openxiii_math::Mode,
    pub language: Option<(String, String)>,
    pub local_players: Option<u8>,
}

impl Game {
    /// Shapes of the Movers, producer shared by loading and its test.
    fn install_mover_shapes(vm: &mut Vm, library: &mut openxiii_package::library::Library) {
        // Collision shapes of the Movers (doors, lifts): they follow their movements.
        if let Some(mover) = vm.class("Mover") {
            let movers: Vec<_> = vm.actors.iter().copied().filter(|&a| vm.objects[a].as_ref().is_some_and(|o| vm.is_child_of(o.class, mover))).collect();
            for a in movers {
                if let Value::Asset(mesh) = vm.native_value(a, Field::StaticMesh) {
                    if let Some(tris) = openxiii_assets::world::static_mesh_triangles(library, &mesh) {
                        vm.set_mover_shape(a, tris);
                    }
                }
            }
        }
    }

    /// Lowercase names of the map's Mover actors, to exclude from the static geometry.
    pub fn mover_names(game: &Path, map: &str) -> Result<std::collections::HashSet<String>, String> {
        let mut vm = Vm::new();
        vm.session.math = openxiii_math::Mode::current();
        vm.load_dir(&openxiii_package::system_dir(&game)).map_err(|e| e.to_string())?;
        vm.load_map(&openxiii_package::library::map_path(game, map)).map_err(|e| e.to_string())?;
        let mover = vm.class("Mover");
        Ok(vm
            .actors
            .iter()
            .filter(|&&a| mover.is_some_and(|m| vm.objects[a].as_ref().is_some_and(|o| vm.is_child_of(o.class, m))))
            .filter_map(|&a| vm.objects[a].as_ref().map(|o| o.name.to_lowercase()))
            .collect())
    }

    /// Map of URL `url` (UGameEngine::LoadMap); `travel`: inventory of the player coming from another
    /// map (ExportTravel text).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        game: &Path,
        url: &openxiii_vm::travel::Url,
        collision: Vec<[[f32; 3]; 3]>,
        tags: Vec<u32>,
        materials: Vec<String>,
        library: &mut openxiii_package::library::Library,
        bindings: &openxiii_vm::input::Bindings,
        travel: Option<&str>,
        engine: Option<Box<openxiii_vm::save::EngineState>>,
        reuse: Option<Vm>,
        startup: StartupOptions,
    ) -> Result<Self, String> {
        // Travel: the previous map's VM, its level unloaded (Vm::unload_level); what
        // the engine created once (interactions, console, menus, viewports) stays there.
        let reused = reuse.is_some();
        let mut vm = reuse.unwrap_or_else(|| {
            let mut vm = Vm::default();
            vm.session.math = startup.math;
            vm.session.language = startup.language;
            launch_seed(&mut vm);
            vm
        });
        let _math = vm.session.math.enter();
        // Travels to a network address go through the engine's pending level.
        vm.ipdrv.engine_pending = true;
        // Map download allowed (deliberate deviation, openxiii_vm::net_file).
        vm.ipdrv.downloads = true;
        // Map of a pending level (UGameEngine::LoadMap with Pending): network client.
        let net_client = vm.ipdrv.pending.is_some();
        // UGameEngine subsystems (saves, music) kept from one map to the next.
        let first_one = engine.is_none();
        if let Some(mut e) = engine {
            vm.config = e.config.take();
            e.client.session_graphics = vm.engine.client.session_graphics;
            vm.engine = *e;
        }
        if !reused {
            vm.load_dir(&openxiii_package::system_dir(&game)).map_err(|e| e.to_string())?;
            if first_one {
                // First start (no engine carried over from a previous map): XIII.exe's InitEngine.
                vm.init_engine_config();
            }
        }
        let file = openxiii_vm::travel::map_file(url);
        let map_owned = file.rsplit_once('.').map_or(file.clone(), |(m, _)| m.to_string());
        let map = map_owned.as_str();
        vm.url = url.clone();
        vm.load_map(&openxiii_package::library::map_path(&game, &file)).map_err(|e| e.to_string())?;
        // Actors whose StaticMesh is given by the map (level::mesh_actors): they are in the static
        // geometry; those whose mesh comes from the class (pickups…) are not.
        let in_map: std::collections::HashSet<String> = library
            .get(map)
            .map(|l| openxiii_assets::level::mesh_actors(map, &l.package, &l.data).into_iter().map(|m| m.actor.to_lowercase()).collect())
            .unwrap_or_default();
        let baked_names: Table<ObjId, String> =
            vm.actors.iter().filter_map(|&a| vm.objects[a].as_ref().map(|o| (a, o.name.to_lowercase()))).filter(|(_, n)| in_map.contains(n)).collect();
        let baked = baked_names.keys().copied().collect();
        let baked_meshes = baked_names.keys().map(|&a| (a, vm.native_value(a, Field::StaticMesh))).collect();
        let baked_placements = baked_names.keys().map(|&a| (a, Self::placement(&vm, a))).collect();
        let tags = vm.adopt_world_materials(&materials, &tags);
        vm.collision = Some(openxiii_assets::collision::CollisionMesh::new_tagged(collision, tags));
        Self::install_mover_shapes(&mut vm, library);
        if net_client {
            // Client: NM_Client before BeginPlay, no GameInfo; non-static actors destroyed
            // and roles swapped, then JOIN; the controller will come from the host.
            if let Some(l) = vm.level {
                vm.set_prop(l, "NetMode", Value::Byte(3));
            }
            // Improved build: not the co-op character screen (its state left with the last map).
            set_character_screen_url(&mut vm, url);
            vm.net_client_purge();
            vm.begin_play();
            vm.net_after_pending_load();
            vm.load_map_save_url_config();
            // End of UGameEngine::LoadMap: micros and sounds of the map for the
            // subsystem kept since the menu (cleared by the CleanUp at the start of LoadMap).
            vm.net_client_end_load_map();
            vm.audio_init(game, map);
            vm.audio_start_level();
            // Loading: written before the lines the viewer prints afterwards itself.
            crate::game_log::write_value(&mut vm.log);
            crate::game_log::empty();
            warm_up_packages(&mut vm, library);
            return Ok(Game {
                vm,
                pawn: None,
                controller: None,
                baked,
                baked_names,
                baked_meshes,
                baked_placements,
                map_name: map.to_string(),
                color_filter: None,
                replaced: Vec::new(),
                base_replacements: None,
            caches: Default::default(),
            names_lights: Default::default(),
                models: Default::default(),
                view: None,
                clock_hx: None,
                bindings: bindings.clone(),
            });
        }
        // UGameEngine::LoadMap: InitGame receives the URL options ("?Name=XIII?Class=…").
        let game_class = vm.game_class_for(&url);
        let gi = vm.start_game(&game_class, &url.options_string()).map_err(|e| e.to_string())?;
        vm.begin_play();
        // What the players chose in the menus (local_play.rs): the menu's map ends a co-op session,
        // another map logs the session it belongs to.
        vm.local_play_map_start(&url.map);
        // --local-players N: players on this screen for the multiplayer games (split_versus.rs).
        vm.apply_startup_local_players(startup.local_players);
        let split = vm.local_play().split_screen();
        if split {
            // Not on the menu's own map: one view there, as in the solo game.
            if url.map.to_ascii_lowercase().starts_with("mapmenu") {
                vm.engine.client.wanted = 1;
            } else {
                vm.set_coop(true);
            }
        } else if vm.coop_on() {
            // The VM is kept from one map to the next: a co-op session that ended (back to the
            // main menu, then a solo game) leaves one player and one view.
            vm.set_coop(false);
            vm.engine.client.wanted = 1;
        }
        // Improved build: the co-op character screen ("Hual02?OXIIIChoose").
        set_character_screen_url(&mut vm, url);
        let players = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), travel).map_err(|e| e.to_string())?;
        let pc = players.first().copied();
        let pawn = pc.and_then(|pc| vm.native_prop(pc, Field::Pawn).as_object());
        vm.load_map_save_url_config();
        // Input of each local player (UInput of its viewport) with the bindings of User.ini /
        // DefUser.ini (or those a profile or SET Input changed, in GConfig).
        let from_config = vm.config.as_mut().map(|c| openxiii_vm::input::Bindings::parse(&c.file_text("User")));
        for &p in &players {
            match from_config.as_ref().filter(|b| !b.alias_list().is_empty()) {
                Some(b) => vm.input_init(p, b),
                None => vm.input_init(p, bindings),
            }
        }
        // UGameEngine::Init, first map only: GLevel->DetailChange(RenDev->HighDetailActors).
        if !reused {
            let high = vm.high_detail_actors();
            vm.detail_change(high);
        }
        // UGameEngine::Init: InitAudio after the map and its player (audio_init).
        vm.audio_init(game, map);
        vm.audio_start_level();
        // End of UGameEngine::Browse: LoadFromCheckpoint clears UAudioSaver's flag.
        vm.checkpoint_browse_done();
        crate::game_log::write_value(&mut vm.log);
        crate::game_log::empty();
        let controller = pc;
        warm_up_packages(&mut vm, library);
        Ok(Game {
            vm,
            pawn,
            controller,
            baked,
            baked_names,
            baked_meshes,
            baked_placements,
            map_name: map.to_string(),
            color_filter: None,
            replaced: Vec::new(),
            base_replacements: None,
            caches: Default::default(),
            names_lights: Default::default(),
            models: Default::default(),
            view: None,
            clock_hx: None,
            bindings: bindings.clone(),
        })
    }

    /// One engine frame (UGameEngine::Tick): the level advances by the elapsed real time (ULevel::Tick
    /// clamps it), the player's input and PlayerTick included, then the sound.
    pub fn update(&mut self, dt: f32) {
        self.vm.audio_listeners_from_viewports();
        self.vm.tick(dt);
        self.follow_viewport_controller();
        if let Some((start, base)) = self.clock_hx {
            self.vm.audio_set_time(base + start.elapsed().as_secs_f64());
        }
        self.vm.audio_tick(dt);
        crate::game_log::write_value(&mut self.vm.log);
        self.vm.warnings.clear();
    }

    /// Network client: the first viewport's controller is the one the host gave
    /// (UNetConnection::HandleClientPlayer); its input is prepared on arrival, and the pawn
    /// follows its own.
    fn follow_viewport_controller(&mut self) {
        if self.vm.level.is_none_or(|l| self.vm.propr(l, "NetMode").as_byte() != 3) {
            return;
        }
        let pc = self.vm.find_viewport_object(0).and_then(|v| self.vm.propr(v, "Actor").as_object()).filter(|&p| self.vm.objects.get(p).is_some_and(|o| o.is_some()));
        let Some(pc) = pc else { return };
        if self.controller != Some(pc) {
            if self.controller.is_some_and(|c| self.vm.objects.get(c).is_some_and(|o| o.is_some())) {
                return;
            }
            self.controller = Some(pc);
            let from_config = self.vm.config.as_mut().map(|c| openxiii_vm::input::Bindings::parse(&c.file_text("User")));
            match from_config.filter(|b| !b.alias_list().is_empty()) {
                Some(b) => self.vm.input_init(pc, &b),
                None => self.vm.input_init(pc, &self.bindings.clone()),
            }
        }
        self.pawn = self.vm.native_prop(pc, Field::Pawn).as_object();
    }

    /// Key or button (EInputKey code) pressed or released: to UInput, handled at the start of the
    /// next frame like the window's messages.
    pub fn key(&mut self, code: u8, pressed: bool) {
        let action = if pressed { openxiii_vm::player_input::IST_PRESS } else { openxiii_vm::player_input::IST_RELEASE };
        self.vm.input_event(code, action, 0.0);
    }

    /// Typed character (WM_CHAR → UEngine::Key), at the start of the next frame.
    pub fn key_type(&mut self, ch: u8) {
        self.vm.input_event(ch, openxiii_vm::player_input::IST_TYPE, 0.0);
    }

    /// Analogue axis (mouse: IK_MouseX, IK_MouseY, IK_MouseW).
    pub fn axis(&mut self, code: u8, delta: f32) {
        self.vm.input_event(code, openxiii_vm::player_input::IST_AXIS, delta);
    }

    /// UGameEngine::Draw: the controller's PlayerCalcView, starting from its position
    /// and rotation. Camera position and rotation for the frame.
    pub fn calc_view(&mut self) -> Option<([f32; 3], [i32; 3])> {
        let pc = self.controller?;
        let f = self.vm.find_function(pc, "PlayerCalcView", false)?;
        let args = vec![Some(Value::Object(Some(pc))), Some(self.vm.native_value(pc, Field::Location)), Some(self.vm.native_value(pc, Field::Rotation))];
        match self.vm.call(f, pc, args) {
            Ok((_, outs)) => {
                let view_actor = outs.first().cloned().flatten().and_then(|v| v.as_object());
                let mut location = outs.get(1).cloned().flatten()?.as_vector();
                let mut rotation = outs.get(2).cloned().flatten()?.as_rotator();
                if let Some((l, r)) = openxiii_vm::client::forced_view() {
                    (location, rotation) = (l, r);
                }
                // Improved build: the co-op character screen's fixed camera (first player).
                let view_actor = character_camera(&self.vm, view_actor, &mut location, &mut rotation);
                self.view = Some((view_actor, location, rotation));
                Some((location, rotation))
            }
            Err(e) => {
                eprintln!("PlayerCalcView: {e}");
                None
            }
        }
    }

    /// Screen effects of the frame (UGameEngine::Draw settings, then Apply2DEffect) for the view
    /// of the last PlayerCalcView.
    pub fn screen_fx(&mut self) -> Vec<openxiii_vm::screen_fx::FxPass> {
        let origin = self.view.map(|(_, o, _)| o);
        self.vm.screen_fx_frame(self.controller, origin)
    }

    /// HUD frame: Canvas primitives (UCanvas::Update, HUD.PostRender) and camera views
    /// of the render textures.
    pub fn hud(&mut self, width: u32, height: u32) -> (Vec<openxiii_vm::canvas::HudDraw>, Vec<(String, Vec<crate::renderer::TargetView>)>) {
        self.vm.render_hud(width, height);
        let quads = self.vm.hud.clone();
        let targets = self.render_targets();
        (quads, targets)
    }

    /// Camera views of the render textures (RenderTargetMaterial) the HUD requested.
    pub fn render_targets(&self) -> Vec<(String, Vec<crate::renderer::TargetView>)> {
        let deg = |r: i32| {
            let r = r.rem_euclid(65536);
            (if r > 32768 { r - 65536 } else { r }) as f32 * 360.0 / 65536.0
        };
        self
            .vm
            .render_views
            .iter()
            .map(|(id, views)| {
                let views = views
                    .iter()
                    .map(|v| crate::renderer::TargetView {
                        rect: v.rect.map(|k| k as f32),
                        camera: crate::renderer::Camera { position: v.location, yaw: deg(v.rotation[1]), pitch: deg(v.rotation[0]), roll: deg(v.rotation[2]) },
                        fov: v.fov,
                        filter: v.filter,
                        highlight: v.highlight,
                        high_light: v.high_light,
                        filter_texture: v.filter_texture.clone(),
                        force_clear: v.force_clear,
                        serial: v.serial,
                    })
                    .collect();
                (format!("#{id}"), views)
            })
            .collect()
    }

    /// Active viewports of the client (split screen: more than one).
    pub fn active_viewports(&self) -> Vec<u8> {
        self.vm.engine.client.viewports.clone()
    }

    /// UGameEngine::Draw of a split-screen viewport (client size `size`): camera,
    /// HUD and visibility of this viewport; the camera becomes the current frame's.
    pub fn draw_viewport(&mut self, slot: u8, size: (u32, u32)) -> Option<openxiii_vm::client::ViewportFrame> {
        let f = self.vm.draw_viewport(slot, (size.0 as i32, size.1 as i32))?;
        if f.view.is_some() {
            self.view = f.view;
        }
        Some(f)
    }

    /// Puts `view` in place of the last PlayerCalcView kept for the frame and returns the one
    /// it replaced (lockstep co-op: this machine's view while its frame is presented).
    pub fn swap_view(&mut self, view: Option<(Option<ObjId>, [f32; 3], [i32; 3])>) -> Option<(Option<ObjId>, [f32; 3], [i32; 3])> {
        std::mem::replace(&mut self.view, view)
    }

    /// Player's horizontal field of view (FovAngle of their controller).
    pub fn fov(&self) -> Option<f32> {
        let c = self.controller?;
        // FCameraSceneNode receives FovAngle as is, unclamped.
        Some(self.vm.propr(c, "FovAngle").as_float())
    }

}

#[cfg(feature = "mods")]
thread_local! {
    /// Mods build only: `Vm::mod_charred` of the characters' ranks given by the last
    /// `character_cartoon_in` (empty when no actor is charred).
    static MOD_CHARRED: std::cell::RefCell<Vec<f32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Mods build only: how charred the actor of rank `owner` is (0 to 1).
#[cfg(feature = "mods")]
pub(crate) fn mod_charred(owner: usize) -> f32 {
    MOD_CHARRED.with(|c| c.borrow().get(owner).copied().unwrap_or(0.0))
}

/// CPU positions of the meshes of a view. Hidden actors share zero vertices.
pub struct CharacterFrame<'a> {
    meshes: Vec<FrameMesh>,
    collapsed: &'a [Vertex],
    pub vertex_count: usize,
}

struct FrameMesh {
    first: usize,
    count: usize,
    vertices: Option<MeshVertices>,
}

enum MeshVertices {
    Animated(Vec<Vertex>),
    Skeletal(std::rc::Rc<PosedVertices>),
    Shared(std::rc::Rc<Vec<Vertex>>),
}

impl MeshVertices {
    /// Allocation shared from one frame to the next (same Rc: same vertices); None for
    /// vertices rebuilt this frame.
    fn shared_id(&self) -> Option<std::rc::Rc<dyn std::any::Any>> {
        match self {
            Self::Animated(_) => None,
            Self::Skeletal(pose) => Some(pose.clone()),
            Self::Shared(shared) => Some(shared.clone()),
        }
    }

    fn slice(&self) -> &[Vertex] {
        match self {
            Self::Animated(v) => v,
            Self::Skeletal(pose) => &pose.vertices,
            Self::Shared(v) => v,
        }
    }
}

impl CharacterFrame<'_> {
    pub fn mesh(&self, owner: usize) -> &[Vertex] {
        let mesh = &self.meshes[owner];
        mesh.vertices.as_ref().map_or(&self.collapsed[..mesh.count], MeshVertices::slice)
    }

    pub fn visible_meshes(&self) -> impl Iterator<Item = (usize, &[Vertex])> {
        self.meshes.iter().filter_map(|mesh| Some((mesh.first, mesh.vertices.as_ref()?.slice())))
    }

    /// `visible_meshes` with the shared vertex allocation (same Rc from one frame to the next:
    /// same vertices); None for vertices rebuilt this frame.
    pub fn visible_meshes_keyed(&self) -> impl Iterator<Item = (usize, &[Vertex], Option<std::rc::Rc<dyn std::any::Any>>)> {
        self.meshes.iter().filter_map(|mesh| {
            let vertices = mesh.vertices.as_ref()?;
            Some((mesh.first, vertices.slice(), vertices.shared_id()))
        })
    }
}

#[derive(Clone, Copy)]
enum MeshView { Main, Targets, Shadows }

/// Result of `Game::characters_merge`.
pub struct Merge {
    /// Length of the previous vertex array (vertices of new models beyond).
    pub vertices_list: usize,
    /// First rank whose batches changed (insertion in the middle, changed model);
    /// usize::MAX: previous ranks unchanged, new actors only at the end.
    pub since_index: usize,
    /// Previous rank of each rank (None: inserted or resurrected).
    pub prior: Vec<Option<usize>>,
    detail: String,
    /// Only rank flags changed: nothing to give the renderer.
    pub simple: bool,
}

/// Displayed characters: VM actor and loaded model.
pub struct Characters {
    models: Vec<std::rc::Rc<Model>>,
    actors: Vec<(ObjId, usize)>,
    /// First and last rank of each ObjId, in this geometry's immutable list.
    ranks: Table<ObjId, (usize, usize)>,
    /// Presence in the level and in the view, indexed by ObjId for each traversal.
    visible: std::cell::RefCell<Vec<u8>>,
    /// Length of the vertex array, hidden actors included (and, after `Game::characters_merge`,
    /// abandoned vertices of a replaced model that no index reads any more).
    vertex_count: usize,
    /// First vertex of each rank: the sum of the previous models in a rebuild;
    /// at the end of the array for an actor inserted or changed by `Game::characters_merge`.
    bases: Vec<usize>,
    /// First index of each rank in the index array (`Game::character_layout`), like
    /// `bases`: the indices of an inserted or changed rank go to the end, those of the other ranks do not
    /// move (only new indices are uploaded, the following batches are kept).
    ibases: Vec<usize>,
    /// Length of the index array (abandoned indices of a replaced model included).
    index_len: usize,
    collapsed: Vec<Vertex>,
    /// Version of the list and its materials at construction.
    signature: u64,
    /// StaticMeshInstance (Actor+0x140) of each StaticMesh actor that has one.
    instances: std::collections::HashMap<ObjId, std::rc::Rc<openxiii_assets::static_mesh::MeshInstance>>,
    /// Scenery entries shared by the cartoon and the lights.
    props: std::cell::RefCell<ViewProps>,
    /// Scenery vertices kept per model and placement (drawing and shadows).
    prop_cache: std::cell::RefCell<std::collections::HashMap<(usize, [u32; 12]), std::rc::Rc<Vec<openxiii_assets::world::Vertex>>>>,
    /// Last deformation per skeletal actor for the view and its silhouette.
    /// The list and its models are immutable; rebuilding them discards these entries.
    skeletal_geometry: std::cell::RefCell<Table<usize, SkeletalGeometry>>,
    /// Actors of the list that left the level since it was built (`Game::characters_to_day`),
    /// and per rank: kept at their rank, hidden like an actor absent from the frame.
    removed: Vec<ObjId>,
    removes: Vec<bool>,
    /// Model of each mesh (lowercase name) in `models`, None if it could not be read
    /// (`Game::characters_add` reuses the models already in the list).
    model_index: std::collections::HashMap<String, Option<usize>>,
}

struct SkeletalGeometry {
    placement: [u32; 12],
    bases: Option<Vec<Coords>>,
    vertices: std::rc::Rc<PosedVertices>,
}

/// Box of the exact pose of this view. A silhouette alone does not request it.
struct PosedVertices {
    vertices: Vec<Vertex>,
    bounds: std::cell::OnceCell<([f32; 3], [f32; 3])>,
}

fn check_generations() -> bool {
    false
}

fn check_kept(kept: Option<bool>, fresh_value: bool, a: ObjId) {
    if kept.is_some_and(|g| g != fresh_value) {
        panic!("CHECK_GENERATIONS: kept value of actor {a} differs from the computation");
    }
}

/// Second term of `Vm::draw_list` (the actor's bIgnoreVignetteAlpha, or its base's for an
/// actor attached to bones) without reading the two flags of the deferred actors.
fn ignore_vignette_alpha(vm: &Vm, a: ObjId) -> bool {
    let a = vm.render_root.get(&a).copied().filter(|&r| vm.objects.get(r).is_some_and(|o| o.is_some())).unwrap_or(a);
    vm.native_prop(a, Field::IgnoreVignetteAlpha).as_bool()
}

fn vertex_bounds(vertices: &[Vertex]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for vertex in vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(vertex.position[k]);
            hi[k] = hi[k].max(vertex.position[k]);
        }
    }
    (lo, hi)
}

impl PosedVertices {
    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        *self.bounds.get_or_init(|| vertex_bounds(&self.vertices))
    }
}

/// Bone frames are the skinning inputs. Including the sign of zeros and
/// NaN bits, two poses only share vertices if their inputs match.
fn same_bases(left: Option<&[Coords]>, right: Option<&[Coords]>) -> bool {
    let bits = |c: &Coords| [c.origin, c.x, c.y, c.z].map(|v| v.map(f32::to_bits));
    match (left, right) {
        (None, None) => true,
        (Some(a), Some(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| bits(a) == bits(b)),
        _ => false,
    }
}

/// Ranks of the scenery only, in the same order as the Characters batches. The geometry
/// stays specific to each rank, even if an ObjId appears several times in the list.
struct ViewProps {
    by_owner: Vec<usize>,
    rows: Vec<ViewProp>,
    refreshes: Table<ObjId, Option<std::rc::Rc<openxiii_vm::visibility::DynRefresh>>>,
}

#[derive(Default)]
struct ViewProp {
    owner: usize,
    geometry: ActorGeometry,
    inputs: PropInputs,
}

impl ViewProps {
    fn new(actors: &[(ObjId, usize)], models: &[std::rc::Rc<Model>]) -> Self {
        let mut props = Self { by_owner: vec![usize::MAX; actors.len()], rows: Vec::new(), refreshes: Table::default() };
        for (owner, &(actor, model)) in actors.iter().enumerate() {
            if matches!(&*models[model], Model::Prop(_)) {
                props.by_owner[owner] = props.rows.len();
                props.rows.push(ViewProp { owner, ..ViewProp::default() });
                props.refreshes.insert(actor, None);
            }
        }
        props
    }

    /// Each call reads the VM's current draws again: the same vertices can serve
    /// another view. Going back from the Canvas to the first pass, the first draw
    /// wins, even if it contains no matrix.
    fn index_draws(&mut self, display: &openxiii_vm::visibility::DisplayList) {
        if self.rows.is_empty() { return; }
        for slot in self.refreshes.values_mut() { *slot = None; }
        let scene = display.scene_light_passes.iter().rev().flat_map(|pass| pass.draws.iter().rev());
        for draw in display.canvas_source_draws.iter().rev().chain(scene) {
            if let Some(slot) = self.refreshes.get_mut(&draw.actor) {
                slot.clone_from(&draw.refresh);
            }
        }
    }
}

#[derive(Default)]
struct PropInputs {
    generation: Option<(u64, u64)>,
    placement: Option<openxiii_assets::level::MeshActor>,
    location: [f32; 3],
    light_group: u8,
    glow: u8,
    unlit: bool,
    ignore_dyn: bool,
    no_ambient: bool,
    pivot_ambient: bool,
    leaves: Vec<i32>,
}

impl PropInputs {
    fn read(&mut self, vm: &Vm, actor: ObjId, restoration: u64) {
        let generation = Some((vm.objects[actor].as_ref().unwrap().props.generation(), restoration));
        if self.generation == generation { return; }
        self.generation = generation;
        self.placement = None;
        self.location = vm.native_prop(actor, Field::Location).as_vector();
        self.light_group = vm.native_prop(actor, Field::StaticLightGroup).as_int() as u8;
        self.glow = vm.native_prop(actor, Field::AmbientGlow).as_byte();
        self.unlit = vm.native_prop(actor, Field::Unlit).as_bool();
        self.ignore_dyn = vm.native_prop(actor, Field::IgnoreDynLight).as_bool();
        self.no_ambient = vm.propr(actor, "bNoAmbientLight").as_bool();
        self.pivot_ambient = vm.native_prop(actor, Field::UseOnlyPivotAmbient).as_bool();
        self.leaves.clear();
        if let Value::Array(v) = vm.native_prop(actor, Field::Leaves) {
            self.leaves.extend(v.iter().map(Value::as_int));
        }
    }
}

impl Characters {
    /// Actors and their Skins, to know whether the list must be rebuilt.
    pub fn signature(&self) -> u64 {
        self.signature
    }

    /// Actors removed from the list without rebuilding it (`Game::characters_to_day`).
    pub fn removed(&self) -> &[ObjId] {
        &self.removed
    }

    fn skeletal_vertices(
        &self,
        owner: usize,
        model: &openxiii_assets::character::CharacterModel,
        placement: &openxiii_assets::level::MeshActor,
        bases: Option<Vec<Coords>>,
    ) -> std::rc::Rc<PosedVertices> {
        let key = placement_bits(placement);
        let mut entries = self.skeletal_geometry.borrow_mut();
        if let Some(last) = entries.get(&owner) {
            if last.placement == key && same_bases(last.bases.as_deref(), bases.as_deref()) {
                return last.vertices.clone();
            }
        }
        let vertices = match &bases {
            Some(bases) => model.vertices_bases(bases, placement),
            None => model.vertices(None, placement),
        };
        let vertices = std::rc::Rc::new(PosedVertices { vertices, bounds: Default::default() });
        entries.insert(owner, SkeletalGeometry { placement: key, bases, vertices: vertices.clone() });
        vertices
    }

    /// `model.vertices(placement)` of model `m`, kept as long as the placement is the same.
    fn prop_vertices(&self, m: usize, model: &openxiii_assets::character::PropModel, placement: &openxiii_assets::level::MeshActor) -> std::rc::Rc<Vec<openxiii_assets::world::Vertex>> {
        let key = (m, placement_bits(placement));
        let mut cache = self.prop_cache.borrow_mut();
        if let Some(v) = cache.get(&key) {
            return v.clone();
        }
        if cache.len() >= 4096 {
            cache.clear();
        }
        let v = std::rc::Rc::new(model.vertices(placement));
        cache.insert(key, v.clone());
        v
    }
}

/// PropModel of a StaticMesh rank (WorldGeometry::of_prop).
fn prop_of(m: &std::rc::Rc<Model>) -> &openxiii_assets::character::PropModel {
    match &**m {
        Model::Prop(p) => p,
        Model::Skeletal(_) => unreachable!("StaticMesh rank"),
    }
}

enum Model {
    Skeletal(openxiii_assets::character::CharacterModel),
    Prop(openxiii_assets::character::PropModel),
}

impl Model {
    fn vertex_count(&self) -> usize {
        match self {
            Model::Skeletal(m) => m.vertex_count(),
            Model::Prop(m) => m.vertex_count(),
        }
    }
    fn batches(&self) -> &Vec<(String, Vec<u32>)> {
        match self {
            Model::Skeletal(m) => &m.batches,
            Model::Prop(m) => &m.batches,
        }
    }
}

impl Game {
    /// Actors that may be drawn outside the static geometry and their mesh: skeletal
    /// meshes, StaticMeshes of Movers and of DT_StaticMesh actors created after loading
    /// (weapons, dropped items, attachments). Those of each frame are those of `drawn_actors`.
    fn drawn_meshes(&self) -> Vec<(ObjId, String, bool)> {
        let hide_bodies = false;
        let player_body = |a: ObjId| self.vm.objects[a].is_some() && self.vm.is_a(a, "XIIIPlayerPawn");
        self.vm.actors.iter().filter_map(|&a| {
            self.vm.objects[a].as_ref()?;
            if hide_bodies
                && (player_body(a)
                    || ["Owner", "Base"].iter().any(|p| self.vm.prop(a, p).as_object().is_some_and(|o| self.vm.objects.get(o).is_some_and(|x| x.is_some()) && player_body(o))))
            {
                return None;
            }
            let moved = self.baked.contains(&a) && self.baked_moved(a);
            let (mesh, prop) = self.drawn_mesh(a, self.vm.movers.contains_key(&a), moved)?;
            Some((a, mesh.to_owned(), prop))
        }).collect()
    }

    fn drawn_mesh(&self, a: ObjId, mover: bool, moved: bool) -> Option<(&str, bool)> {
        if mover || ((!self.baked.contains(&a) || moved) && self.vm.native_prop(a, Field::DrawType).as_byte() == 8) {
            if let Value::Asset(m) = self.vm.native_prop(a, Field::StaticMesh) {
                return Some((m, true));
            }
        }
        if !mover {
            // Improved build: a co-op player's first-person mesh with their character's arms
            // (the VM's coop_looks.rs).
            if let Some(m) = player_arms_mesh(&self.vm, a) {
                return Some((m, false));
            }
            if let Value::Asset(m) = self.vm.native_prop(a, Field::Mesh) {
                return Some((m, false));
            }
        }
        None
    }

    /// Static geometry actor whose script changed the StaticMesh (Amos01: DisplayScreen):
    /// the engine draws the actor's current mesh, so it leaves the static geometry.
    fn baked_mesh_changed(&self, a: ObjId) -> bool {
        self.baked_meshes.get(&a).is_some_and(|m| m != self.vm.native_prop(a, Field::StaticMesh))
    }

    fn placement(vm: &Vm, a: ObjId) -> [Value; 5] {
        [Field::Location, Field::Rotation, Field::DrawScale, Field::DrawScale3D, Field::PrePivot].map(|p| vm.native_value(a, p))
    }

    /// Static geometry actor the script has moved (Plage01's HelicoDeco placed by its
    /// cinematic): the engine draws it where it is, so it leaves the static geometry.
    fn baked_moved(&self, a: ObjId) -> bool {
        // Compared field by field without copying (Self::placement copied five values per actor).
        const FIELDS: [Field; 5] = [Field::Location, Field::Rotation, Field::DrawScale, Field::DrawScale3D, Field::PrePivot];
        self.baked_placements.get(&a).is_some_and(|p| p.iter().zip(FIELDS).any(|(v, field)| v != self.vm.native_prop(a, field)))
    }

    /// Geometry of the map's actors whose script changed the StaticMesh, when this list
    /// changes (or always with `force`, for a new renderer): Some(None) if there are none left,
    /// None if nothing changed.
    pub fn replaced_world(&mut self, library: &mut openxiii_package::library::Library, force: bool) -> Option<Option<openxiii_assets::world::World>> {
        let moved = self.baked_moved_image();
        self.replaced_world_with(library, force, &moved)
    }

    /// Live static geometry actors that moved (`baked_moved`), computed once for
    /// the steps of a frame that read them (signature, changed meshes, hidden actors): the
    /// actors' placement does not change between them (no script runs).
    pub fn baked_moved_image(&self) -> std::rc::Rc<Ens<ObjId>> {
        let mut caches = self.caches.borrow_mut();
        let cache = &mut caches.placements;
        // The slots follow the immutable order of self.baked, specific to this map.
        cache.generations.resize(self.baked.len(), None);
        for (&a, previous) in self.baked.iter().zip(&mut cache.generations) {
            let generation = self.vm.objects.get(a).and_then(Option::as_ref).map(|o| o.props.generation());
            if generation == *previous {
                continue;
            }
            *previous = generation;
            let moved = generation.is_some() && self.baked_moved(a);
            if moved != cache.moved.contains(&a) {
                let set = std::rc::Rc::make_mut(&mut cache.moved);
                if moved {
                    set.insert(a);
                } else {
                    set.remove(&a);
                }
            }
        }
        cache.moved.clone()
    }

    /// Geometry of every static geometry actor of the map with its current mesh: the size the
    /// replaced-meshes buffers are made with at load, so a script swapping meshes (broken or
    /// destroyed versions) rewrites them instead of making new ones while playing.
    pub fn replaced_capacity(&mut self, library: &mut openxiii_package::library::Library) -> Option<openxiii_assets::world::World> {
        let list: Vec<(String, String)> = self
            .baked
            .iter()
            .filter(|&&a| self.vm.objects.get(a).is_some_and(|o| o.is_some()))
            .filter_map(|&a| match self.vm.native_value(a, Field::StaticMesh) {
                Value::Asset(m) if !m.is_empty() => Some((self.baked_names.get(&a)?.clone(), m)),
                _ => None,
            })
            .collect();
        if list.is_empty() {
            return None;
        }
        openxiii_assets::world::build_mesh_replacements_kept(library, &self.map_name, &list, &mut self.base_replacements)
    }

    /// `replaced_world` with the moved actors of `baked_moved_image`.
    pub fn replaced_world_with(&mut self, library: &mut openxiii_package::library::Library, force: bool, moved: &Ens<ObjId>) -> Option<Option<openxiii_assets::world::World>> {
        let mut list: Vec<(String, String)> = self
            .baked
            .iter()
            .filter(|&&a| self.vm.objects.get(a).is_some_and(|o| o.is_some()) && self.baked_mesh_changed(a) && !moved.contains(&a))
            .filter_map(|&a| match self.vm.native_value(a, Field::StaticMesh) {
                Value::Asset(m) if !m.is_empty() => Some((self.baked_names.get(&a)?.clone(), m)),
                _ => None,
            })
            .collect();
        list.sort();
        if list == self.replaced && !force {
            // The map's actors and lighting prepared from the level's first frame (and not on the
            // first broken shutter, which paid for it): no effect on the game.
            if self.base_replacements.is_none() && !self.baked.is_empty() {
                let _ = openxiii_assets::world::build_mesh_replacements_kept(library, &self.map_name, &[], &mut self.base_replacements);
            }
            return None;
        }
        self.replaced = list;
        if self.replaced.is_empty() {
            return Some(None);
        }
        let list_value = &self.replaced[..];
        let w = openxiii_assets::world::build_mesh_replacements_kept(library, &self.map_name, list_value, &mut self.base_replacements);
        Some(w)
    }

    /// Actors of `drawn_meshes`, to compare with the built list.
    pub fn characters_signature(&self) -> u64 {
        self.characters_signature_with(&self.baked_moved_image())
    }

    /// Version of the ordered list of drawable actors and of their meshes and Skins.
    /// Each Game has its own list, including after a travel that reuses the VM.
    pub fn characters_signature_with(&self, moved: &Ens<ObjId>) -> u64 {
        let mut caches = self.caches.borrow_mut();
        let list = &mut caches.characters_list;
        let objects = self.vm.objects.len();
        list.actors.resize_with(objects, ActorStamp::default);
        // Only Movers and moved scenery actors set these two bits.
        // The other actors no longer need to look up both tables in every view.
        const MOVER: u8 = 1;
        const DISPLACED: u8 = 2;
        list.membership.clear();
        list.membership.resize(objects, 0);
        for &id in self.vm.movers.keys() {
            if let Some(bits) = list.membership.get_mut(id) { *bits |= MOVER; }
        }
        for &id in moved {
            if let Some(bits) = list.membership.get_mut(id) { *bits |= DISPLACED; }
        }
        let models = self.models.borrow();
        if models.len() != list.model_count {
            list.available.clear();
            list.model_count = models.len();
        }
        let mut changed = list.version == 0;
        let mut was_reread = false;
        let mut reread_existing = false;
        // Actors whose mesh was read or lost by this call (ActorStamp::value_read).
        let mut read_count: Vec<ObjId> = Vec::new();
        let revived = characters_revived();
        let mut count = 0;
        for &a in &self.vm.actors {
            let Some(object) = &self.vm.objects[a] else { continue };
            let stamp = &mut list.actors[a];
            let generation = object.props.generation();
            let membership = list.membership[a];
            let models_changed = stamp.model_count != list.model_count;
            if stamp.generation != generation || stamp.membership != membership || models_changed {
                stamp.model_count = list.model_count;
                stamp.generation = generation;
                stamp.membership = membership;
                let mesh = self.drawn_mesh(a, membership & MOVER != 0, membership & DISPLACED != 0);
                // An unrelated write (LastRenderTime, for example) requires neither
                // a path copy nor a new model lookup.
                let mut available = |name: &str| {
                    if let Some(&known) = list.available.get(name) { return known; }
                    let known = models.get(&name.to_lowercase()).is_none_or(|model| model.is_some());
                    list.available.insert(name.to_owned(), known);
                    known
                };
                let matches = mesh.zip(stamp.mesh.as_deref()).is_some_and(|((name, prop), old)| {
                    old.prop == prop && old.matches(&self.vm, a) && (!models_changed || available(name))
                });
                if !matches {
                    // Actor without a mesh before and after: neither re-read nor changed, not recorded.
                    if dbg_reason() && (stamp.mesh.is_some() || mesh.is_some()) {
                        let before_value = match stamp.mesh.as_deref() {
                            Some(old) => format!("known mesh (changed: {})", old.differences(&self.vm, a)),
                            None => "without mesh".to_owned(),
                        };
                        let name_value = mesh.map_or("none", |m| m.0);
                        dbg_reread(format!("{}: {before_value} -> {name_value}{}", dbg_class(&self.vm, a, None), if models_changed { " (models changed)" } else { "" }));
                    }
                    if stamp.mesh.is_some() || mesh.is_some() {
                        read_count.push(a);
                    }
                    if let Some((_, prop)) = mesh.filter(|(name, _)| available(name)) {
                        reread_existing |= stamp.mesh.is_some();
                        stamp.mesh = Some(Box::new(MeshInputs::read(&self.vm, a, prop)));
                        changed = true;
                        was_reread = true;
                    } else {
                        // Mesh lost: the actor leaves list.order; with the fix, the list
                        // hides it at its rank like an actor that left (characters_to_day), without re-reading.
                        if !revived {
                            was_reread |= stamp.mesh.is_some();
                            reread_existing |= stamp.mesh.is_some();
                        }
                        stamp.mesh = None;
                    }
                }
            }
            if stamp.mesh.is_some() {
                if list.order.get(count) != Some(&a) {
                    if count < list.order.len() {
                        list.order[count] = a;
                    } else {
                        list.order.push(a);
                    }
                    changed = true;
                }
                count += 1;
            }
        }
        changed |= list.order.len() != count;
        list.order.truncate(count);
        if changed {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            list.version = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if was_reread {
                list.reread_version = list.version;
            }
            if reread_existing {
                list.reread_existing_version = list.version;
            }
        }
        for a in read_count {
            list.actors[a].value_read = list.version;
        }
        list.version
    }

    /// List `chars` still valid for this frame (otherwise: `characters` must be redone). When the
    /// signature only changes because actors of the list left the level (broken windows and shutters,
    /// pickups taken), the list is kept: each actor that left stays at its rank, marked removed
    /// (`Characters::removes`) and hidden like an actor absent from the frame (vertices collapsed to a
    /// point); the others keep their draw order. Rebuilding the list redid every model and re-uploaded
    /// every vertex on each broken window. A mesh re-read or lost, an added actor, or a removed actor
    /// back in the level (index reused by a new object) forces a rebuild.
    fn dbg_actor(&self, a: ObjId, moved: &Ens<ObjId>, rank: Option<usize>) -> String {
        let mesh = if self.vm.objects.get(a).is_some_and(|o| o.is_some()) {
            self.drawn_mesh(a, self.vm.movers.contains_key(&a), moved.contains(&a)).map_or("none", |m| m.0)
        } else {
            "object destroyed"
        };
        format!("{} mesh {mesh}", dbg_class(&self.vm, a, rank))
    }

    pub fn characters_to_day(&self, chars: &mut Characters, moved: &Ens<ObjId>) -> bool {
        let signature = self.characters_signature_with(moved);
        let in_level = || {
            let mut v = vec![false; self.vm.objects.len()];
            for &a in &self.vm.actors {
                if let Some(slot) = v.get_mut(a) { *slot = true; }
            }
            v
        };
        let revived = characters_revived();
        if signature == chars.signature {
            if chars.removed.iter().all(|&a| self.vm.objects.get(a).is_none_or(|o| o.is_none())) {
                return true;
            }
            // With the fix, a removed actor still in the level without a mesh (muzzle flash
            // turned off) is hidden like one that left: only a removed actor with a mesh counts.
            let caches = self.caches.borrow();
            let stamps = &caches.characters_list.actors;
            let a_mesh = |a: ObjId| !revived || stamps.get(a).is_some_and(|s| s.mesh.is_some());
            if revived && !chars.removed.iter().any(|&a| a_mesh(a) && self.vm.objects.get(a).is_some_and(|o| o.is_some())) {
                return true;
            }
            let level_value = in_level();
            let returned = chars.removed.iter().find(|&&a| level_value.get(a).is_some_and(|&b| b) && a_mesh(a));
            if let Some(&a) = returned.filter(|_| dbg_reason()) {
                let rank = chars.actors.iter().position(|&(x, _)| x == a);
                dbg_note(format!("up_to_date: equal signature but a removed actor came back into the level (index taken again): {}", self.dbg_actor(a, moved, rank)));
            }
            return returned.is_none();
        }
        let caches = self.caches.borrow();
        let list = &caches.characters_list;
        if list.reread_version > chars.signature {
            if dbg_reason() {
                dbg_note(format!("up_to_date: reread_version {} > signature {} (mesh reread or lost)", list.reread_version, chars.signature));
            }
            return false;
        }
        let level_value = in_level();
        let present = |a: ObjId| level_value.get(a).is_some_and(|&b| b);
        // With the fix: an actor of the level that has a read mesh (hence in list.order).
        let a_mesh = |a: ObjId| !revived || list.actors.get(a).is_some_and(|s| s.mesh.is_some());
        // Order of the actors still in the level unchanged, each skipped actor gone (or, with
        // the fix, still in the level without a mesh: hidden at its rank the same way).
        let mut next = 0;
        let mut gone = Vec::new();
        for (owner, &(a, _)) in chars.actors.iter().enumerate() {
            if chars.removes[owner] {
                if present(a) && a_mesh(a) {
                    if dbg_reason() {
                        dbg_note(format!("up_to_date: removed actor came back (index taken again): {}", self.dbg_actor(a, moved, Some(owner))));
                    }
                    return false;
                }
            } else if list.order.get(next) == Some(&a) {
                next += 1;
            } else if !present(a) || !a_mesh(a) {
                gone.push(owner);
            } else {
                if dbg_reason() {
                    let want = list.order.get(next).map_or("end of the list".to_owned(), |&b| self.dbg_actor(b, moved, None));
                    let pos = list.order.iter().position(|&b| b == a);
                    dbg_note(format!("up_to_date: actor present out of order: {}; expected at rank {next} of the order: {want}; rank in the new order {pos:?}", self.dbg_actor(a, moved, Some(owner))));
                }
                return false;
            }
        }
        if next != list.order.len() {
            if dbg_reason() {
                let n: Vec<String> = list.order[next..].iter().take(6).map(|&b| self.dbg_actor(b, moved, None)).collect();
                dbg_note(format!("up_to_date: next {next} != list.order.len() {} ({} new: {})", list.order.len(), list.order.len() - next, n.join(" | ")));
            }
            return false;
        }
        drop(caches);
        for owner in gone {
            chars.removes[owner] = true;
            chars.removed.push(chars.actors[owner].0);
        }
        chars.signature = signature;
        true
    }

    /// Each rank keeps its actor; an actor that left or has no mesh (muzzle flash off) is hidden at
    /// its rank; a hidden actor that gets its mesh back at its place becomes visible again (same
    /// model); a new actor is inserted at its place in the order, and a kept actor re-read with
    /// another model (broken shutter) takes that model at its rank. The vertices of a new model go to
    /// the end of the buffer (`Characters::bases`): the batches stay in the order of a rebuild, only
    /// their indices change. None: rebuild (a listed actor placed elsewhere in the order, unreadable
    /// model).
    pub fn characters_merge(&self, chars: &mut Characters, moved: &Ens<ObjId>, library: &mut openxiii_package::library::Library) -> Option<Merge> {
        let signature = self.characters_signature_with(moved);
        if signature == chars.signature {
            if dbg_reason() {
                dbg_note("merge: equal signature (removed with a mesh in the level)".to_owned());
            }
            return None;
        }
        enum StepMode { Kept(usize), Cache(usize), Revives(usize), Change(usize), Inserts(ObjId) }
        let caches = self.caches.borrow();
        let list = &caches.characters_list;
        let mut level_value = vec![false; self.vm.objects.len()];
        for &a in &self.vm.actors {
            if let Some(slot) = level_value.get_mut(a) {
                *slot = true;
            }
        }
        // Actor present in list.order: in the level, alive, with a read mesh.
        let draws = |a: ObjId| {
            level_value.get(a).is_some_and(|&b| b) && self.vm.objects.get(a).is_some_and(|o| o.is_some()) && list.actors.get(a).is_some_and(|s| s.mesh.is_some())
        };
        // Mesh of `a` read as `characters` does (drawn_meshes).
        let mesh_value = |a: ObjId| self.drawn_mesh(a, self.vm.movers.contains_key(&a), self.baked.contains(&a) && self.baked_moved(a));
        let mut plan = Vec::with_capacity(chars.actors.len() + 4);
        let mut next = 0;
        for (owner, &(a, m)) in chars.actors.iter().enumerate() {
            if !draws(a) {
                plan.push(StepMode::Cache(owner));
                continue;
            }
            loop {
                match list.order.get(next) {
                    Some(&b) if b == a => break,
                    Some(&b) if chars.ranks.get(&b).is_none() => {
                        plan.push(StepMode::Inserts(b));
                        next += 1;
                    }
                    other_one => {
                        if dbg_reason() {
                            let seen_flag = other_one.map_or("end of the order".to_owned(), |&b| self.dbg_actor(b, moved, chars.ranks.get(&b).map(|r| r.0)));
                            dbg_note(format!("merge: listed actor elsewhere in the order: {} expected, found {seen_flag}", self.dbg_actor(a, moved, Some(owner))));
                        }
                        return None;
                    }
                }
            }
            next += 1;
            let was_reread = chars.removes[owner] || list.actors[a].value_read > chars.signature;
            let same_check = || {
                mesh_value(a).is_some_and(|(mesh, prop)| {
                    chars.model_index.get(&mesh.to_lowercase()) == Some(&Some(m)) && matches!(&*chars.models[m], Model::Prop(_)) == prop
                })
            };
            plan.push(if !was_reread {
                StepMode::Kept(owner)
            } else if !same_check() {
                StepMode::Change(owner)
            } else if chars.removes[owner] {
                StepMode::Revives(owner)
            } else {
                StepMode::Kept(owner)
            });
        }
        for &b in &list.order[next..] {
            if chars.ranks.get(&b).is_some() {
                if dbg_reason() {
                    dbg_note(format!("merge: listed actor came back after the end of the order: {}", self.dbg_actor(b, moved, chars.ranks.get(&b).map(|r| r.0))));
                }
                return None;
            }
            plan.push(StepMode::Inserts(b));
        }
        drop(caches);
        // Short path: only hidden ranks or resurrected scenery (StaticMesh), same
        // number of ranks. Batches, indices and vertices unchanged (a scenery's material comes from its
        // model, not from the Skins): only the rank flags change, like a removal in
        // `characters_to_day`, without redoing the layout or the vertex image.
        let simple = plan.len() == chars.actors.len()
           
            && plan.iter().all(|step_size| match *step_size {
                StepMode::Kept(_) | StepMode::Cache(_) => true,
                StepMode::Revives(o) => matches!(&*chars.models[chars.actors[o].1], Model::Prop(_)),
                _ => false,
            });
        if simple {
            let bad = revives_bad_rank();
            let mut prior: Vec<Option<usize>> = (0..chars.actors.len()).map(Some).collect();
            let (mut nb_cache, mut nb_res) = (0, 0);
            let mut described = Vec::new();
            for step_size in plan {
                match step_size {
                    StepMode::Cache(o) if !chars.removes[o] => {
                        nb_cache += 1;
                        chars.removes[o] = true;
                    }
                    StepMode::Revives(o) => {
                        nb_res += 1;
                        chars.removes[o] = bad;
                        prior[o] = None;
                        let a = chars.actors[o].0;
                        if described.len() < 6 {
                            described.push(format!("revived {}", self.dbg_actor(a, moved, Some(o))));
                        }
                        chars.instances.remove(&a);
                        if let Value::Asset(path) = self.vm.prop(a, "StaticMeshInstance") {
                            let inst = library
                                .resolve(&path)
                                .and_then(|(loaded, index)| openxiii_assets::static_mesh::MeshInstance::read(&loaded.package, &loaded.data, index).ok());
                            if let Some(inst) = inst {
                                chars.instances.insert(a, std::rc::Rc::new(inst));
                            }
                        }
                    }
                    _ => {}
                }
            }
            chars.removed = chars.actors.iter().zip(&chars.removes).filter(|(_, &r)| r).map(|(&(a, _), _)| a).collect();
            chars.signature = signature;
            let detail = if dbg_reason() { format!("short path: {nb_cache} hidden, {nb_res} revived: {}", described.join(" | ")) } else { String::new() };
            return Some(Merge { vertices_list: chars.vertex_count, since_index: usize::MAX, prior, detail, simple: true });
        }
        // Models of the inserted or changed actors (as in `characters`) before any modification.
        let mut models_list: Table<ObjId, Option<usize>> = Table::default();
        let t_models = std::time::Instant::now();
        for step_size in &plan {
            let a = match *step_size {
                StepMode::Change(o) => chars.actors[o].0,
                StepMode::Inserts(b) => b,
                _ => continue,
            };
            let slot = mesh_value(a).and_then(|(mesh, prop)| self.model_list(chars, mesh, prop, library));
            if slot.is_none() && matches!(step_size, StepMode::Change(_)) {
                if dbg_reason() {
                    dbg_note(format!("merge: unreadable model for a changed actor: {}", self.dbg_actor(a, moved, None)));
                }
                return None;
            }
            models_list.insert(a, slot);
        }
        let ms_models = t_models.elapsed().as_secs_f64() * 1000.0;
        let before_value = chars.vertex_count;
        let bad = revives_bad_rank();
        let order_broken = false;
        let mut actors = Vec::with_capacity(plan.len());
        let mut removes = Vec::with_capacity(plan.len());
        let mut bases = Vec::with_capacity(plan.len());
        let mut ibases = Vec::with_capacity(plan.len());
        let mut index_len = chars.index_len;
        let mut prior: Vec<Option<usize>> = Vec::with_capacity(plan.len());
        let mut vertex_count = chars.vertex_count;
        let mut since_index = usize::MAX;
        let mut first_inserts: Option<usize> = None;
        let (mut nb_cache, mut nb_res, mut nb_change, mut nb_ins) = (0, 0, 0, 0);
        let mut geo_before = std::mem::take(&mut *chars.skeletal_geometry.borrow_mut());
        let mut geo = Table::default();
        let mut reread_list = Vec::new();
        let mut described = Vec::new();
        for step_size in plan {
            let k = actors.len();
            let old_rank = match step_size {
                StepMode::Inserts(_) => None,
                StepMode::Kept(o) | StepMode::Cache(o) | StepMode::Revives(o) | StepMode::Change(o) => Some(o),
            };
            if old_rank.is_some() {
                if let Some(i) = first_inserts.take() {
                    since_index = since_index.min(i);
                }
            }
            match step_size {
                StepMode::Kept(o) | StepMode::Cache(o) => {
                    let cache = matches!(step_size, StepMode::Cache(_));
                    if cache && !chars.removes[o] {
                        nb_cache += 1;
                    }
                    actors.push(chars.actors[o]);
                    removes.push(chars.removes[o] || cache);
                    bases.push(chars.bases[o]);
                    ibases.push(chars.ibases[o]);
                    prior.push(Some(o));
                    if let Some(g) = geo_before.remove(&o) {
                        geo.insert(k, g);
                    }
                }
                StepMode::Revives(o) => {
                    nb_res += 1;
                    actors.push(chars.actors[o]);
                    removes.push(bad);
                    bases.push(chars.bases[o]);
                    ibases.push(chars.ibases[o]);
                    prior.push(None);
                    reread_list.push(chars.actors[o].0);
                    if described.len() < 6 {
                        described.push(format!("revived {}", self.dbg_actor(chars.actors[o].0, moved, Some(k))));
                    }
                    if order_broken && k > 0 {
                        actors.swap(k - 1, k);
                        removes.swap(k - 1, k);
                        bases.swap(k - 1, k);
                        ibases.swap(k - 1, k);
                        prior.swap(k - 1, k);
                        if let Some(g) = geo.remove(&(k - 1)) {
                            geo.insert(k, g);
                        }
                        since_index = since_index.min(k - 1);
                    }
                }
                StepMode::Change(o) => {
                    nb_change += 1;
                    let a = chars.actors[o].0;
                    let m = models_list.get(&a).copied().flatten().expect("model read above");
                    actors.push((a, m));
                    removes.push(false);
                    bases.push(vertex_count);
                    vertex_count += chars.models[m].vertex_count();
                    ibases.push(index_len);
                    index_len += model_indices(&chars.models[m]);
                    // As in a rebuild: the cartoon direction follows the kept actor.
                    prior.push(Some(o));
                    reread_list.push(a);
                    since_index = since_index.min(k);
                    if described.len() < 6 {
                        described.push(format!("changed {}", self.dbg_actor(a, moved, Some(k))));
                    }
                }
                StepMode::Inserts(b) => {
                    let Some(m) = models_list.get(&b).copied().flatten() else { continue };
                    nb_ins += 1;
                    actors.push((b, m));
                    removes.push(false);
                    bases.push(vertex_count);
                    vertex_count += chars.models[m].vertex_count();
                    ibases.push(index_len);
                    index_len += model_indices(&chars.models[m]);
                    prior.push(None);
                    reread_list.push(b);
                    first_inserts.get_or_insert(k);
                    if described.len() < 6 {
                        described.push(format!("inserted {}", self.dbg_actor(b, moved, Some(k))));
                    }
                }
            }
        }
        let detail = if dbg_reason() {
            format!("{nb_cache} hidden, {nb_res} revived, {nb_change} changed, {nb_ins} inserted; {} -> {} ranks, {before_value} -> {vertex_count} vertices; models {ms_models:.3} ms: {}", chars.actors.len(), actors.len(), described.join(" | "))
        } else {
            String::new()
        };
        let mut ranks = Table::default();
        for (k, &(a, _)) in actors.iter().enumerate() {
            ranks.entry(a).or_insert((k, k)).1 = k;
        }
        chars.removed = actors.iter().zip(&removes).filter(|(_, &r)| r).map(|(&(a, _), _)| a).collect();
        chars.actors = actors;
        chars.removes = removes;
        chars.bases = bases;
        chars.ibases = ibases;
        chars.index_len = index_len;
        chars.ranks = ranks;
        chars.vertex_count = vertex_count;
        *chars.skeletal_geometry.borrow_mut() = geo;
        // StaticMeshInstance read again for each inserted, resurrected or changed actor (as in `characters`).
        for a in reread_list {
            chars.instances.remove(&a);
            let Some(&(k, _)) = chars.ranks.get(&a) else { continue };
            if !matches!(&*chars.models[chars.actors[k].1], Model::Prop(_)) {
                continue;
            }
            if let Value::Asset(path) = self.vm.prop(a, "StaticMeshInstance") {
                let inst = library
                    .resolve(&path)
                    .and_then(|(loaded, index)| openxiii_assets::static_mesh::MeshInstance::read(&loaded.package, &loaded.data, index).ok());
                if let Some(inst) = inst {
                    chars.instances.insert(a, std::rc::Rc::new(inst));
                }
            }
        }
        let largest_mesh = chars.models.iter().map(|m| m.vertex_count()).max().unwrap_or(0);
        if chars.collapsed.len() < largest_mesh {
            chars.collapsed.resize(largest_mesh, Vertex::default());
        }
        *chars.props.borrow_mut() = ViewProps::new(&chars.actors, &chars.models);
        chars.signature = signature;
        Some(Merge { vertices_list: before_value, since_index, prior, detail, simple: false })
    }

    /// Model of mesh `mesh` in `chars.models` (loaded as in `characters` if needed).
    fn model_list(&self, chars: &mut Characters, mesh: &str, prop: bool, library: &mut openxiii_package::library::Library) -> Option<usize> {
        let key = mesh.to_lowercase();
        if let Some(&slot) = chars.model_index.get(&key) {
            return slot;
        }
        let model = self
            .models
            .borrow_mut()
            .entry(key.clone())
            .or_insert_with(|| {
                let model = if prop {
                    openxiii_assets::character::PropModel::load(library, mesh).map(Model::Prop)
                } else {
                    openxiii_assets::character::CharacterModel::load(library, mesh).map(Model::Skeletal)
                };
                model.map(std::rc::Rc::new)
            })
            .clone();
        let slot = model.map(|m| {
            chars.models.push(m);
            chars.models.len() - 1
        });
        chars.model_index.insert(key, slot);
        slot
    }

    /// List `chars` extended without rebuilding it, when `characters_to_day` refuses it only
    /// because actors appeared (enemy, dropped weapon, debris): the list's actors keep their
    /// order (those that left stay removed at their rank) and the new ones come after them in
    /// level order, hence at the end of the list, as in a rebuild (`characters` follows
    /// `drawn_meshes`: same ranks, same batches and same first vertices for the kept actors).
    /// Returns (actor count, vertex count) before the addition; None: rebuild needed (mesh
    /// re-read or lost, actor back, appearance in the middle of the list).
    pub fn characters_add(&self, chars: &mut Characters, moved: &Ens<ObjId>, library: &mut openxiii_package::library::Library) -> Option<(usize, usize)> {
        let signature = self.characters_signature_with(moved);
        let caches = self.caches.borrow();
        let list = &caches.characters_list;
        if signature == chars.signature || list.reread_existing_version > chars.signature {
            if dbg_reason() {
                if signature == chars.signature {
                    dbg_note("add: equal signature (removed actor came back)".to_owned());
                } else {
                    dbg_note(format!("add: reread_existing_version {} > signature {} (mesh of an already listed actor reread or lost)", list.reread_existing_version, chars.signature));
                }
            }
            return None;
        }
        let mut level_value = vec![false; self.vm.objects.len()];
        for &a in &self.vm.actors {
            if let Some(slot) = level_value.get_mut(a) {
                *slot = true;
            }
        }
        let present = |a: ObjId| level_value.get(a).is_some_and(|&b| b);
        let mut next = 0;
        let mut gone = Vec::new();
        for (owner, &(a, _)) in chars.actors.iter().enumerate() {
            if chars.removes[owner] {
                if present(a) {
                    if dbg_reason() {
                        dbg_note(format!("add: removed actor came back (index taken again): {}", self.dbg_actor(a, moved, Some(owner))));
                    }
                    return None;
                }
            } else if list.order.get(next) == Some(&a) {
                next += 1;
            } else if !present(a) {
                gone.push(owner);
            } else {
                if dbg_reason() {
                    let want = list.order.get(next).map_or("end of the list".to_owned(), |&b| self.dbg_actor(b, moved, None));
                    let pos = list.order.iter().position(|&b| b == a);
                    dbg_note(format!("add: actor present out of order (appeared in the middle): {}; in its place at rank {next} of the order: {want}; rank in the new order {pos:?}", self.dbg_actor(a, moved, Some(owner))));
                }
                return None;
            }
        }
        let new_list: Vec<ObjId> = list.order[next..].to_vec();
        if new_list.is_empty() || new_list.iter().any(|a| chars.ranks.get(a).is_some()) {
            if dbg_reason() {
                if new_list.is_empty() {
                    dbg_note(format!("add: nothing new (next {next} = length of the order, {} gone)", gone.len()));
                } else {
                    let d: Vec<String> = new_list.iter().filter_map(|a| chars.ranks.get(a).map(|r| self.dbg_actor(*a, moved, Some(r.0)))).take(6).collect();
                    dbg_note(format!("add: new one already in ranks (already listed, removed): {}", d.join(" | ")));
                }
            }
            return None;
        }
        drop(caches);
        let before_value = (chars.actors.len(), chars.vertex_count);
        for owner in gone {
            chars.removes[owner] = true;
            chars.removed.push(chars.actors[owner].0);
        }
        // Same reading as `characters` (drawn_meshes, then each mesh's model).
        for a in new_list {
            let moved = self.baked.contains(&a) && self.baked_moved(a);
            let Some((mesh, prop)) = self.drawn_mesh(a, self.vm.movers.contains_key(&a), moved) else { continue };
            let key = mesh.to_lowercase();
            let slot = match chars.model_index.get(&key) {
                Some(&slot) => slot,
                None => {
                    let model = self
                        .models
                        .borrow_mut()
                        .entry(key.clone())
                        .or_insert_with(|| {
                            let model = if prop {
                                openxiii_assets::character::PropModel::load(library, mesh).map(Model::Prop)
                            } else {
                                openxiii_assets::character::CharacterModel::load(library, mesh).map(Model::Skeletal)
                            };
                            model.map(std::rc::Rc::new)
                        })
                        .clone();
                    let slot = model.map(|m| {
                        chars.models.push(m);
                        chars.models.len() - 1
                    });
                    chars.model_index.insert(key, slot);
                    slot
                }
            };
            let Some(m) = slot else { continue };
            let k = chars.actors.len();
            chars.actors.push((a, m));
            chars.removes.push(false);
            chars.bases.push(chars.vertex_count);
            chars.ibases.push(chars.index_len);
            chars.index_len += model_indices(&chars.models[m]);
            chars.ranks.entry(a).or_insert((k, k)).1 = k;
            chars.vertex_count += chars.models[m].vertex_count();
            if matches!(&*chars.models[m], Model::Prop(_)) {
                if let Value::Asset(path) = self.vm.prop(a, "StaticMeshInstance") {
                    let inst = library
                        .resolve(&path)
                        .and_then(|(loaded, index)| openxiii_assets::static_mesh::MeshInstance::read(&loaded.package, &loaded.data, index).ok());
                    if let Some(inst) = inst {
                        chars.instances.insert(a, std::rc::Rc::new(inst));
                    }
                }
            }
        }
        let largest_mesh = chars.models.iter().map(|m| m.vertex_count()).max().unwrap_or(0);
        if chars.collapsed.len() < largest_mesh {
            chars.collapsed.resize(largest_mesh, Vertex::default());
        }
        *chars.props.borrow_mut() = ViewProps::new(&chars.actors, &chars.models);
        chars.signature = signature;
        Some(before_value)
    }

    /// Character list of the frame given to the renderer: kept (`characters_to_day`),
    /// extended at the end (`characters_add`, only the new vertices uploaded) or rebuilt.
    pub fn list_characters(&mut self, r: &mut crate::renderer::Renderer, library: &mut openxiii_package::library::Library, chars: &mut Option<Characters>, moved: &Ens<ObjId>) {
        if chars.as_mut().is_some_and(|c| self.characters_to_day(c, moved)) {
            if dbg_reason() {
                dbg_empty();
            }
            return;
        }
        if let Some(c) = chars.as_mut().filter(|_| characters_revived()) {
            let t_fusion = std::time::Instant::now();
            if let Some(f) = self.characters_merge(c, moved, library) {
                let ms_fusion = t_fusion.elapsed().as_secs_f64() * 1000.0;
                if f.simple {
                    if dbg_reason() {
                        dbg_write("merge (flags only)", &f.detail);
                    }
                    r.cartoon_dirs_follow(c.actors.len(), |k| f.prior.get(k).copied().flatten());
                    return;
                }
                let t_layout = std::time::Instant::now();
                let (indices, batches, owners) = self.character_layout(c);
                let t_frame = std::time::Instant::now();
                let frame = self.character_frame(c);
                let t_send = std::time::Instant::now();
                let done = if f.simple {
                    true
                } else if f.since_index == usize::MAX {
                    r.extend_dynamic(&frame, &indices, &batches, &owners, f.vertices_list)
                } else {
                    r.relayout_dynamic(&frame, &indices, &batches, &owners, f.vertices_list, f.since_index)
                };
                if !done {
                    r.set_dynamic(&frame, &indices, &batches, &owners, library);
                }
                if dbg_reason() {
                    let path_buf = match (done, f.since_index == usize::MAX) {
                        _ if f.simple => "merge (flags only, checked)".to_owned(),
                        (true, true) => "merge (extend_dynamic)".to_owned(),
                        (true, false) => format!("merge (relayout_dynamic from rank {})", f.since_index),
                        (false, _) => format!("merge then refused ({}) -> set_dynamic", crate::renderer::reason_extend()),
                    };
                    let ms = |a: std::time::Instant, b: std::time::Instant| (b - a).as_secs_f64() * 1000.0;
                    let end_value = std::time::Instant::now();
                    dbg_write(&path_buf, &format!("{}; time: merge {ms_fusion:.3} (signature and plan included), layout {:.3}, vertex image {:.3}, renderer {:.3} ms", f.detail, ms(t_layout, t_frame), ms(t_frame, t_send), ms(t_send, end_value)));
                }
                // Cartoon direction: follows each actor to its new rank; none for an
                // inserted or resurrected actor (absent from the previous list of a rebuild).
                r.cartoon_dirs_follow(c.actors.len(), |k| f.prior.get(k).copied().flatten());
                return;
            }
        } else if let Some(c) = chars.as_mut() {
            if let Some((actors_list, vertices_list)) = self.characters_add(c, moved, library) {
                let (indices, batches, owners) = self.character_layout(c);
                let frame = self.character_frame(c);
                let extended = r.extend_dynamic(&frame, &indices, &batches, &owners, vertices_list);
                if !extended {
                    r.set_dynamic(&frame, &indices, &batches, &owners, library);
                }
                if dbg_reason() {
                    let n: Vec<String> = (actors_list..c.actors.len()).take(6).map(|k| self.dbg_actor(c.actors[k].0, moved, Some(k))).collect();
                    let path_buf = if extended { "addition (extend_dynamic)".to_owned() } else { format!("add then extend refused ({}) -> set_dynamic", crate::renderer::reason_extend()) };
                    dbg_write(&path_buf, &format!("{} actors added ({} -> {}): {}", c.actors.len() - actors_list, actors_list, c.actors.len(), n.join(" | ")));
                }
                r.cartoon_dirs_follow(c.actors.len(), |k| (k < actors_list).then_some(k));
                return;
            }
        }
        if dbg_reason() {
            let path_buf = if chars.is_some() { "full rebuild" } else { "first list" };
            dbg_write(path_buf, "");
        }
        let before_value: Option<Table<ObjId, usize>> = chars.as_ref().map(|c| {
            let mut t = Table::default();
            for (k, &(a, _)) in c.actors.iter().enumerate() {
                if !c.removes[k] {
                    t.entry(a).or_insert(k);
                }
            }
            t
        });
        let c = self.characters(library);
        let (indices, batches, owners) = self.character_layout(&c);
        r.set_dynamic(&self.character_frame(&c), &indices, &batches, &owners, library);
        if let Some(before_value) = before_value.filter(|_| true) {
            r.cartoon_dirs_follow(c.actors.len(), |k| before_value.get(&c.actors[k].0).copied());
        }
        *chars = Some(c);
    }

    /// Loads the models of the actors of `drawn_meshes`.
    pub fn characters(&self, library: &mut openxiii_package::library::Library) -> Characters {
        let mut models = Vec::new();
        let mut index: std::collections::HashMap<String, Option<usize>> = std::collections::HashMap::new();
        let mut actors = Vec::new();
        for (a, mesh, prop) in self.drawn_meshes() {
            let key = mesh.to_lowercase();
            let slot = *index.entry(key.clone()).or_insert_with(|| {
                let model = self
                    .models
                    .borrow_mut()
                    .entry(key)
                    .or_insert_with(|| {
                        let model = if prop {
                            openxiii_assets::character::PropModel::load(library, &mesh).map(Model::Prop)
                        } else {
                            openxiii_assets::character::CharacterModel::load(library, &mesh).map(Model::Skeletal)
                        };
                        model.map(std::rc::Rc::new)
                    })
                    .clone();
                model.map(|m| {
                    models.push(m);
                    models.len() - 1
                })
            });
            if let Some(m) = slot {
                actors.push((a, m));
            }
        }
        let mut instances = std::collections::HashMap::new();
        for &(a, m) in &actors {
            if !matches!(&*models[m], Model::Prop(_)) {
                continue;
            }
            if let Value::Asset(path) = self.vm.prop(a, "StaticMeshInstance") {
                let inst = library
                    .resolve(&path)
                    .and_then(|(loaded, index)| openxiii_assets::static_mesh::MeshInstance::read(&loaded.package, &loaded.data, index).ok());
                if let Some(inst) = inst {
                    instances.insert(a, std::rc::Rc::new(inst));
                }
            }
        }
        let vertex_count = actors.iter().map(|&(_, index)| models[index].vertex_count()).sum();
        let largest_mesh = models.iter().map(|m| m.vertex_count()).max().unwrap_or(0);
        let collapsed = vec![Vertex::default(); largest_mesh];
        let mut ranks = Table::default();
        for (k, &(a, _)) in actors.iter().enumerate() {
            ranks.entry(a).or_insert((k, k)).1 = k;
        }
        let props = std::cell::RefCell::new(ViewProps::new(&actors, &models));
        let removes = vec![false; actors.len()];
        let mut bases = Vec::with_capacity(actors.len());
        let mut ibases = Vec::with_capacity(actors.len());
        let (mut base, mut index_len) = (0, 0);
        for &(_, index) in &actors {
            bases.push(base);
            base += models[index].vertex_count();
            ibases.push(index_len);
            index_len += model_indices(&models[index]);
        }
        Characters { models, actors, ranks, visible: Default::default(), vertex_count, bases, ibases, index_len, collapsed, signature: self.characters_signature(), instances, props, prop_cache: Default::default(), skeletal_geometry: Default::default(), removed: Vec::new(), removes, model_index: index }
    }

    fn actor_placement(&self, a: ObjId, mesh: &str) -> openxiii_assets::level::MeshActor {
        Self::placement_of(openxiii_vm::render_placement::RenderPlacement::read(&self.vm, a), mesh)
    }

    /// The placement captured at draw time, independent of later writes.
    fn drawn_placement(&self, a: ObjId) -> openxiii_assets::level::MeshActor {
        match self.vm.render_place.get(&a) {
            Some(p) => Self::placement_of(*p, ""),
            None => self.actor_placement(a, ""),
        }
    }

    fn placement_of(p: openxiii_vm::render_placement::RenderPlacement, mesh: &str) -> openxiii_assets::level::MeshActor {
        openxiii_assets::level::MeshActor {
            actor: String::new(), mesh: mesh.to_string(), location: p.location,
            rotation: p.rotation, scale: p.scale, pre_pivot: p.pre_pivot, blocks: false,
        }
    }

    /// USkeletalMeshInstance::GetTexture: Skins[index] (AActor::GetSkin) if it is
    /// set, otherwise the mesh texture, otherwise Skins[0], otherwise the mesh's first texture.
    /// None: the mesh texture (default batch).
    fn skeletal_texture(&self, a: ObjId, index: i32, textures: &[String]) -> Option<String> {
        let skins: Vec<Value> = match self.vm.native_value(a, Field::Skins) {
            Value::Array(v) => v,
            _ => Vec::new(),
        };
        let path = |v: &Value| match v {
            Value::Asset(p) if !p.is_empty() => Some(p.clone()),
            _ => None,
        };
        let pose = |i: usize| skins.get(i).and_then(path).is_some();
        match openxiii_vm::Vm::texture_source(true, skins.len(), pose, textures.len(), index) {
            openxiii_vm::solo_native::TextureSource::Skin(i) => Some(path(&skins[i]).unwrap_or_default()),
            openxiii_vm::solo_native::TextureSource::Material(i) if usize::try_from(index) == Ok(i) => None,
            openxiii_vm::solo_native::TextureSource::Material(i) => textures.get(i).cloned(),
            openxiii_vm::solo_native::TextureSource::Nothing => textures.first().cloned(),
        }
    }

    /// Fixed indices and batches of all characters (vertices in `frame` order).
    /// Also returns, for each batch, the rank of its actor.
    pub fn character_layout(&self, chars: &Characters) -> (Vec<u32>, Vec<openxiii_assets::world::Batch>, Vec<usize>) {
        // Indices of each rank starting from `ibases` (consecutive in a rebuild); the
        // batches stay in rank order. Abandoned slots stay at zero (no batch).
        let mut indices = vec![0u32; chars.index_len];
        let mut batches = Vec::new();
        let mut owners = Vec::new();
        for (k, &(_, m)) in chars.actors.iter().enumerate() {
            let model = &chars.models[m];
            let base = chars.bases[k] as u32;
            let mut pos = chars.ibases[k];
            for (bi, (material, local)) in model.batches().iter().enumerate() {
                // USkeletalMeshInstance::GetTexture: the actor's Skins.
                let material = match &**model {
                    Model::Skeletal(m) => skeletal_material(material, self.skeletal_texture(chars.actors[k].0, m.batch_textures[bi], &m.textures)),
                    Model::Prop(_) => material.clone(),
                };
                let first_index = pos as u32;
                for (dst, i) in indices[pos..pos + local.len()].iter_mut().zip(local.iter()) {
                    *dst = base + i;
                }
                pos += local.len();
                batches.push(openxiii_assets::world::Batch { material, first_index, index_count: local.len() as u32 });
                owners.push(k);
            }
        }
        (indices, batches, owners)
    }

    /// Every data object (texture, material, mesh…) named by a property of any VM object, the
    /// default objects of every loaded class included (effects, pickups, HUD and fonts spawned
    /// later): what the level references, all in memory after the original's LoadMap. Read only:
    /// no property is written and no object is made.
    pub fn precache_roots(&self) -> Vec<String> {
        fn walk(v: &Value, out: &mut std::collections::HashSet<String>) {
            match v {
                Value::Asset(p) => {
                    if !p.is_empty() && !out.contains(p) {
                        out.insert(p.clone());
                    }
                }
                Value::Array(items) => items.iter().for_each(|x| walk(x, out)),
                Value::Struct(_, fields) => fields.iter().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut out = std::collections::HashSet::new();
        for o in self.vm.objects.iter().flatten() {
            for v in o.props.iter() {
                walk(v, &mut out);
            }
        }
        // The objects the loaded classes' script code names (Texture'…' constants, paths given
        // to DynamicLoadObject): the walk keeps the materials among them.
        for p in self.vm.script_object_constants() {
            out.insert(format!("@{p}"));
        }
        #[cfg(feature = "mods")]
        out.extend(self.vm.mod_precache_roots());
        // The script's render textures already made (the HUD's comic panel material, made in
        // XIIIBaseHud.PostBeginPlay): "#n", as the renderer names them.
        for (id, o) in self.vm.objects.iter().enumerate() {
            if o.is_some() && self.vm.is_a(id, "RenderTargetMaterial") {
                out.insert(format!("#{id}"));
            }
        }
        let mut list: Vec<String> = out.into_iter().collect();
        list.sort();
        list
    }

    fn drawn_view(&self) -> std::rc::Rc<DrawnActors> {
        self.caches.borrow_mut().drawn.get([&self.vm.rendered, &self.vm.canvas_rendered, &self.vm.target_rendered])
    }

    /// The frame's SetSinusVertexStreams for the static geometry actors: (actor name,
    /// modifier, CurrentTime, amplitude), in draw order (Vm::sine_drawn).
    pub fn sine_drawn(&self) -> Vec<(String, String, f32, [f32; 3])> {
        self.vm.sine_drawn.iter().filter_map(|(a, key, t, amp)| self.baked_names.get(a).map(|n| (n.clone(), key.clone(), *t, *amp))).collect()
    }

    /// Deferred actors (bDelayDisplay, 's deferred list): drawn after the
    /// scene, depth cleared; at 50° (DELAYED_FOV) with bSpecialDelayFov.
    pub fn first_person(&self, chars: &Characters) -> Vec<u8> {
        chars
            .actors
            .iter()
            .zip(&chars.removes)
            .map(|(&(a, model), &removes)| {
                if self.vm.objects[a].is_none() || removes {
                    return 0;
                }
                match self.vm.draw_list(a).0 {
                    0 => 0,
                    1 => crate::renderer::DELAYED,
                    _ if matches!(chars.models.get(model).map(|m| &**m), Some(Model::Skeletal(m)) if m.reaches_eye) => crate::renderer::DELAYED_FOV_EYE,
                    _ => crate::renderer::DELAYED_FOV,
                }
            })
            .collect()
    }

    /// bIgnoreVignetteAlpha of each animated actor (list set aside by when
    /// UseVignetteHighlight).
    pub fn ignore_vignette(&self, chars: &Characters) -> Vec<bool> {
        chars.actors.iter().zip(&chars.removes).map(|(&(a, _), &removes)| !removes && self.vm.objects[a].is_some() && ignore_vignette_alpha(&self.vm, a)).collect()
    }

    /// Ranks of the last scene's translucent list (Vm::translucent_order, from
    /// farthest to nearest) for the order of the renderer's sorted passes.
    pub fn translucent_keys(&self, chars: &Characters) -> crate::renderer::TranslucentKeys {
        let mut keys = crate::renderer::TranslucentKeys::default();
        for (p, t) in self.vm.translucent_order.iter().enumerate() {
            match *t {
                openxiii_vm::visibility::Translucent::Node(n) => {
                    keys.nodes.insert(n as u32, p as i64);
                }
                openxiii_vm::visibility::Translucent::Actor(a) => {
                    keys.actors.insert(a, p as i64);
                    if let Some(&(_, k)) = chars.ranks.get(&a) {
                        keys.characters.insert(k, p as i64);
                    } else if let Some(n) = self.baked_names.get(&a) {
                        keys.baked.insert(n.clone(), p as i64);
                    }
                }
            }
        }
        keys
    }

    /// Static geometry actors (names) that have bIgnoreVignetteAlpha.
    /// The actors are recomputed on every call; the set of names (shared through Rc with the
    /// renderer) is only rebuilt if they change.
    pub fn vignette_baked(&self) -> std::rc::Rc<std::collections::HashSet<String>> {
        let mut caches = self.caches.borrow_mut();
        let c = &mut *caches;
        // bIgnoreVignetteAlpha only read again if the base's Props changed.
        let restoration = c.prop_restoration;
        if c.states_vignette.len() != self.baked.len() {
            c.states_vignette.clear();
            c.states_vignette.resize(self.baked.len(), None);
        }
        c.scratch_ids.clear();
        for (i, &a) in self.baked.iter().enumerate() {
            if !self.vm.objects.get(a).is_some_and(|o| o.is_some()) {
                continue;
            }
            let root = self.vm.render_root.get(&a).copied().filter(|&r| self.vm.objects.get(r).is_some_and(|o| o.is_some())).unwrap_or(a);
            let key_tuple = (a, root, self.vm.objects[root].as_ref().map_or(0, |o| o.props.generation()), restoration);
            let ignore = match c.states_vignette[i] {
                Some((k, v)) if k == key_tuple && !check_generations() && crate::prop_geometry::kept_active() => v,
                _ => {
                    let v = ignore_vignette_alpha(&self.vm, a);
                    check_kept(c.states_vignette[i].filter(|(k, _)| *k == key_tuple).map(|(_, v)| v), v, a);
                    c.states_vignette[i] = Some((key_tuple, v));
                    v
                }
            };
            if ignore {
                c.scratch_ids.push(a);
            }
        }
        if c.scratch_ids != c.vignette_ids {
            std::mem::swap(&mut c.scratch_ids, &mut c.vignette_ids);
            c.vignette = std::rc::Rc::new(c.vignette_ids.iter().filter_map(|a| self.baked_names.get(a).cloned()).collect());
        }
        caches.vignette.clone()
    }

    /// Screen flash of FCameraSceneNode::Render for PlayerController `pc`:
    /// if FlashScale.X < 1, DrawTile with colour FColor(FPlane(FlashFog, 1 − FlashScale.X)), each
    /// value clamped to [0, 1] (FColor::FColor: fistp(v × 255 − 0.5) clamped).
    pub fn flash_tile(&self, pc: Option<ObjId>) -> Option<[u8; 4]> {
        let pc = pc.filter(|&c| self.vm.objects.get(c).is_some_and(|o| o.is_some()) && self.vm.is_a(c, "PlayerController"))?;
        let scale = self.vm.propr(pc, "FlashScale").as_vector();
        let fog = self.vm.propr(pc, "FlashFog").as_vector();
        let [b, g, r, a] = openxiii_vm::filter::screen_flash_tile(scale[0], fog)?;
        Some([r, g, b, a])
    }

    /// Vignette of the view's PlayerController.
    pub fn vignette(&self) -> openxiii_vm::visibility::Vignette {
        self.vm.vignette(self.controller)
    }

    /// Projectors of the BSP nodes seen from `camera`. Node drawn: in the draw lists
    /// of the last visibility traversal (`bsp_visible`); without a traversal, render vertex stream
    /// and a vertex inside the view cone.
    pub fn projector_draws(
        &mut self,
        camera: &crate::renderer::Camera,
        model: Option<&openxiii_assets::model::Model>,
        fov: f32,
    ) -> Vec<openxiii_vm::projector::ProjectorDraw> {
        let Some(model) = model else { return Vec::new() };
        let [fwd, _, _] = camera.axes();
        let eye = camera.position;
        let fov = openxiii_vm::display_format::widen_fov(fov, self.vm.view_widen_window(), self.vm.engine.client.session_graphics);
        let half = ((fov.to_radians() * 0.5).tan() * 1.25).atan();
        let cos = half.cos();
        // Only draws the projectors of the nodes in the scene's draw lists
        // (bsp_draw.rs: no invisible surfaces, portals or sky backdrops):
        // the list of the view's last traversal and of its sky scenes when there is one.
        // Same nodes as `bsp_visible` (root, or one of the sky scenes within the root's length),
        // read without copying the lists or the actor names.
        // The views are taken for the duration of the VM calls below (which do not read them),
        // then put back.
        let views = std::mem::take(&mut self.vm.scene_bsp_views);
        let nodes = self.vm.level_bsp.as_ref().map(|m| m.nodes.len());
        let records = views.iter().find(|v| v.sky_zone.is_none()).filter(|root| nodes == Some(root.visible.len()));
        let drawn = |n: usize| -> bool {
            if let Some(root) = records {
                return root.visible.get(n).is_some_and(|&b| b)
                    || (n < root.visible.len() && views.iter().filter(|v| v.sky_zone.is_some()).any(|v| v.visible.get(n).is_some_and(|&b| b)));
            }
            let Some(node) = model.nodes.get(n) else { return false };
            if node.vertex_stream < 0 || node.vertex_index < 0 {
                return false;
            }
            let Some(stream) = model.vertex_streams.get(node.vertex_stream as usize) else { return false };
            (0..node.num_vertices as usize).any(|k| {
                stream.get(node.vertex_index as usize + k).is_some_and(|v| {
                    let d = [v.position[0] - eye[0], v.position[1] - eye[1], v.position[2] - eye[2]];
                    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    l > 0.0 && (d[0] * fwd[0] + d[1] * fwd[1] + d[2] * fwd[2]) / l >= cos
                })
            })
        };
        let mut out = self.vm.terrain_projector_draws();
        out.extend(self.vm.bsp_projector_draws(&drawn));
        self.vm.scene_bsp_views = views;
        let actors = self.drawn_view();
        out.extend(self.vm.actor_projector_draws(&actors.members));
        out
    }

    /// Particles of the frame seen from `camera` (AEmitter::Render of each Emitter).
    pub fn particle_sprites(&mut self, camera: &crate::renderer::Camera) -> (Vec<openxiii_vm::particles::SpriteBatch>, Vec<openxiii_vm::particles::MeshBatch>) {
        let viewer = self.view.and_then(|v| v.0).or(self.controller);
        self.particle_sprites_for(camera, viewer)
    }

    /// The particles of one view, seen by `viewer` (a split-screen viewport's player): mods'
    /// effects shown only to their owner, or hidden from him, follow it.
    #[cfg_attr(not(feature = "mods"), allow(unused_variables))]
    pub fn particle_sprites_for(&mut self, camera: &crate::renderer::Camera, viewer: Option<ObjId>) -> (Vec<openxiii_vm::particles::SpriteBatch>, Vec<openxiii_vm::particles::MeshBatch>) {
        #[cfg(feature = "mods")]
        {
            self.vm.mod_particle_viewer = viewer.or(self.controller);
        }
        self.vm.particle_frame(&camera.view_frame())
    }

    /// Dynamic lights of the level (FLight::Refresh of each actor this frame).
    pub fn dyn_light_records(&self) -> Vec<openxiii_assets::model::LightRecord> {
        let out = self.vm.dyn_light_records();
        out
    }

    /// Trails drawn this frame (ATrail::Render).
    pub fn trail_batches(&mut self, camera: &crate::renderer::Camera) -> Vec<openxiii_vm::trail_render::TrailBatch> {
        let drawn = self.drawn_view();
        let mut out = self.vm.trail_frame(&drawn.members);
        // Actors drawn only by a sky scene: that scene's camera (FSkySceneNode).
        let mut sky_value: std::collections::HashMap<ObjId, crate::renderer::Camera> = std::collections::HashMap::new();
        if let Some(root) = self.vm.scene_bsp_views.iter().find(|v| v.sky_zone.is_none()) {
            let root_value: std::collections::HashSet<ObjId> = root.actors.iter().copied().collect();
            for v in self.vm.scene_bsp_views.iter().filter(|v| v.sky_zone.is_some()) {
                let za = v.sky_zone.and_then(|z| usize::try_from(z).ok()).and_then(|z| self.vm.zone_actors.get(z).copied().flatten());
                let Some(za) = za else { continue };
                let l = self.vm.native_value(za, Field::Location).as_vector();
                let r = self.vm.native_value(za, Field::Rotation).as_rotator();
                let sky = camera.sky_view(l, r);
                for &a in v.actors.iter().filter(|a| !root_value.contains(a)) {
                    sky_value.entry(a).or_insert(sky);
                }
            }
        }
        out.extend(crate::sprites::batches(&mut self.vm, camera, &drawn.order, &|a| sky_value.get(&a).copied()));
        out
    }

    /// Light passes distinct from those of the main scene, portals and targets.
    pub fn scene_light_leaves(&self, chars: &Characters) -> crate::lights_scene::Views {
        crate::lights_scene::convert_kept(&self.vm, |a| chars.ranks.get(&a).map(|&(first, _)| first),
            &self.baked_names, &mut self.names_lights.borrow_mut(), &mut self.caches.borrow_mut().lights_list, chars.signature())
    }

    /// In a view, the actor's first draw provides its matrix (scene before Canvas).
    /// Flags, leaves and lights stay reads of this view.
    fn prop_geometry(&self, row: &mut ViewProp, actor: ObjId, model: &std::rc::Rc<Model>,
                     verts: &CharacterFrame<'_>, refresh: Option<&openxiii_vm::visibility::DynRefresh>) -> std::rc::Rc<WorldGeometry> {
        let owner = row.owner;
        let slot = &mut row.geometry.current;
        if let Some(refresh) = refresh {
            let key = Pose::Refresh { matrix: refresh.matrix.map(f32::to_bits), visible: verts.meshes[owner].vertices.is_some() };
            slot.get(key, || {
                let matrix = std::array::from_fn(|r| std::array::from_fn(|c| refresh.matrix[r * 4 + c]));
                WorldGeometry::new(verts.mesh(owner).iter().map(|v| (
                    openxiii_assets::lighting::transform_point(&matrix, v.position),
                    openxiii_assets::lighting::transform_vector(&matrix, v.normal),
                )))
            })
        } else {
            let placement = row.inputs.placement.get_or_insert_with(|| self.actor_placement(actor, ""));
            slot.get(Pose::Placement(placement_bits(placement)), || WorldGeometry::of_prop(model.clone(), prop_of, placement))
        }
    }

    /// `character_cartoon` written into `out` (the previous frame's): the mesh names
    /// and leaf lists already allocated are reused.
    pub fn character_cartoon_in(&self, chars: &Characters, verts: &CharacterFrame<'_>, out: &mut Vec<Option<openxiii_assets::cartoon::CharacterCartoon>>) {
        // Mods build only: how charred scripts made each of these actors, by rank.
        #[cfg(feature = "mods")]
        MOD_CHARRED.with(|c| {
            let mut c = c.borrow_mut();
            c.clear();
            if !self.vm.mod_charred.is_empty() {
                c.extend(chars.actors.iter().map(|&(a, _)| self.vm.mod_charred.get(&a).copied().unwrap_or(0.0)));
            }
        });
        let mut props = chars.props.borrow_mut();
        props.index_draws(&self.vm.display);
        let restoration = self.caches.borrow().prop_restoration;
        let ViewProps { by_owner, rows, refreshes } = &mut *props;
        out.truncate(chars.actors.len());
        out.resize_with(chars.actors.len(), || None);
        for (owner, &(a, m)) in chars.actors.iter().enumerate() {
            let slice = verts.mesh(owner);
            let model = &*chars.models[m];
            let mesh_field = if matches!(model, Model::Prop(_)) { Field::StaticMesh } else { Field::Mesh };
            if self.vm.objects[a].is_none() || chars.removes[owner] {
                out[owner] = None;
                continue;
            }
            let Value::Asset(mesh) = self.vm.native_prop(a, mesh_field) else {
                out[owner] = None;
                continue;
            };
            let slot = out[owner].get_or_insert_with(|| openxiii_assets::cartoon::CharacterCartoon {
                mesh: String::new(), light_group: 0, location: [0.0; 3], sphere: [0.0; 4], leaves: Vec::new(), unlit: false,
            });
            slot.leaves.clear();
            let ((lo, hi), location, light_group, unlit) = if let Model::Prop(_) = model {
                let row = &mut rows[by_owner[owner]];
                row.inputs.read(&self.vm, a, restoration);
                let geometry = self.prop_geometry(row, a, &chars.models[m], verts, refreshes[&a].as_deref());
                slot.leaves.extend_from_slice(&row.inputs.leaves);
                // No light for bUnlit nor bIgnoreDynLight in the game.
                ((geometry.min, geometry.max), row.inputs.location, row.inputs.light_group, row.inputs.unlit || row.inputs.ignore_dyn)
            } else {
                let bounds = if let Some(MeshVertices::Skeletal(pose)) = &verts.meshes[owner].vertices {
                    pose.bounds()
                } else if verts.meshes[owner].vertices.is_none() && !slice.is_empty() {
                    // All the vertices of the hidden skeleton are Vertex::default().
                    ([0.0; 3], [0.0; 3])
                } else {
                    vertex_bounds(slice)
                };
                // Vm::actor_leaves without an intermediate list.
                if let Value::Array(v) = self.vm.native_prop(a, Field::Leaves) {
                    slot.leaves.extend(v.iter().map(Value::as_int));
                }
                (bounds, self.vm.native_prop(a, Field::Location).as_vector(),
                    self.vm.native_prop(a, Field::StaticLightGroup).as_int() as u8,
                    self.vm.native_prop(a, Field::Unlit).as_bool() || self.vm.native_prop(a, Field::IgnoreDynLight).as_bool())
            };
            let c = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
            let r = [0, 1, 2].map(|k| (hi[k] - lo[k]) * 0.5);
            let sphere = [c[0], c[1], c[2], (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt()];
            slot.mesh.clone_from(mesh);
            slot.light_group = light_group;
            slot.location = location;
            slot.sphere = sphere;
            slot.unlit = unlit;
        }
    }

    /// AShadowProjector::UpdateShadow of the drawn actors that have a shadow (Pawn.Shadow,
    /// Decoration.Shadow), then the silhouettes to draw, by actor rank in `chars`.
    pub fn update_shadows(
        &mut self,
        chars: &Characters,
        eye: [f32; 3],
        dir: &dyn Fn(usize) -> Option<[f32; 3]>,
        frame: u32,
        view: Option<u8>,
    ) -> Vec<crate::renderer::ShadowRender> {
        self.vm.projectors.shadow_jobs.clear();
        // More shadow targets than D3DDrv's eight with Graphics: Enhanced
        // (openxiii_vm::projector::shadow_slot_count).
        self.vm.projectors.more_shadow_targets = enhanced_shadow_settings(&self.vm);
        if let Some(v) = view {
            self.vm.shadow_view_drawn(frame, v);
        }
        let drawn = self.drawn_view();
        let far = far_shadow_scale(
            enhanced_shadow_settings(&self.vm),
            self.controller.map(|c| (self.vm.propr(c, "FovAngle").as_float(), self.vm.propr(c, "DefaultFOV").as_float())),
        );
        for (k, &(a, _)) in chars.actors.iter().enumerate() {
            if self.vm.objects[a].is_none() || chars.removes[k] || !drawn.members.contains(&a) {
                continue;
            }
            let Some(sp) = self.vm.propr(a, "Shadow").as_object().filter(|&s| self.vm.objects.get(s).is_some_and(|o| o.is_some())) else { continue };
            // LastRenderDist just set by FDynamicActor::Render (is called after
            // the owner is rendered): distance to the camera of the scene that drew it, that
            // of a comic panel for a corpse only the kill cam shows.
            let _ = eye;
            let dist = self.vm.propr(a, "LastRenderDist").as_float();
            // AShadowProjector::UpdateShadow: a StaticMesh owner gives the light
            // direction kept in its StaticMeshInstance (+0x68), none when it has no instance
            // (light straight down): a placed vehicle decoration has none.
            let static_mesh = self.vm.native_prop(a, Field::DrawType).as_byte() == 8;
            let has_instance = matches!(self.vm.prop(a, "StaticMeshInstance"), Value::Asset(ref p) if !p.is_empty());
            let light = if static_mesh && !has_instance { None } else { dir(k) };
            // Split screen: one update per frame, by the nearest view (Vm::update_shadow_in_view).
            match view {
                Some(v) => self.vm.update_shadow_in_view(sp, dist / far, light, frame, v),
                None => self.vm.update_shadow(sp, dist / far, light, frame),
            }
        }
        let jobs = std::mem::take(&mut self.vm.projectors.shadow_jobs);
        jobs.into_iter()
            .filter_map(|j| {
                let &(owner, _) = chars.ranks.get(&j.owner)?;
                // UShadowBitmapMaterial::Update: then each actor in the owner's
                // Attached that has an AttachmentBone and is a WeaponAttachment (the weapon it
                // carries) is drawn into the silhouette.
                let attached = match self.vm.prop(j.owner, "Attached") {
                    Value::Array(list) => list
                        .iter()
                        .filter_map(|v| v.as_object())
                        .filter(|&w| self.vm.objects.get(w).is_some_and(|o| o.is_some()))
                        .filter(|&w| {
                            let bone = self.vm.prop(w, "AttachmentBone").as_name();
                            !bone.is_empty() && !bone.eq_ignore_ascii_case("None")
                        })
                        .filter(|&w| self.vm.is_a(w, "WeaponAttachment"))
                        .filter_map(|w| chars.ranks.get(&w).map(|&(rank, _)| rank))
                        .collect(),
                    _ => Vec::new(),
                };
                Some(crate::renderer::ShadowRender {
                    slot: j.slot,
                    owner,
                    attached,
                    location: j.location,
                    light_dir: j.light_dir,
                    usize: j.usize,
                })
            })
            .collect()
    }

    /// Map actors whose StaticMesh is in the static geometry and that must no longer be
    /// drawn: destroyed, out of the level's display list (bHidden or DT_None at the
    /// last RefreshDynDisp), or a DrawType other than DT_StaticMesh (lowercase names).
    pub fn hidden_baked(&self) -> std::rc::Rc<std::collections::HashSet<String>> {
        self.hidden_baked_with(&self.baked_moved_image())
    }

    /// `hidden_baked` with the moved actors of `baked_moved_image`. The hidden actors are
    /// recomputed on every call; the intermediate sets and the returned set of names
    /// are only rebuilt if their inputs changed (exact comparison), which is rare.
    pub fn hidden_baked_with(&self, moved: &Ens<ObjId>) -> std::rc::Rc<std::collections::HashSet<String>> {
        let mut caches = self.caches.borrow_mut();
        let c = &mut *caches;
        if c.actors_source != self.vm.actors {
            c.actors_source.clone_from(&self.vm.actors);
            // vm.actors as a set: one contains per map actor used to walk the whole list.
            c.actors_list = self.vm.actors.iter().copied().collect();
        }
        let entries = &self.vm.display.entries;
        if !(c.lists_source.len() == entries.len() && entries.iter().map(|e| e.actor).eq(c.lists_source.iter().copied())) {
            c.lists_source = entries.iter().map(|e| e.actor).collect();
            c.lists = entries.iter().filter_map(|e| e.actor).collect();
        }
        let (actors, listed) = (&c.actors_list, &c.lists);
        // DrawType and StaticMesh only read again if the actor's Props changed.
        let restoration = c.prop_restoration;
        if c.states_caches.len() != self.baked.len() {
            c.states_caches.clear();
            c.states_caches.resize(self.baked.len(), None);
        }
        c.scratch_ids.clear();
        for (i, &a) in self.baked.iter().enumerate() {
            let cache = match self.vm.objects.get(a).and_then(|o| o.as_ref()) {
                None => true,
                Some(o) => !actors.contains(&a) || !listed.contains(&a) || {
                    let key_tuple = (a, o.props.generation(), restoration);
                    match c.states_caches[i] {
                        Some((k, v)) if k == key_tuple && !check_generations() && crate::prop_geometry::kept_active() => v,
                        _ => {
                            let v = self.vm.native_prop(a, Field::DrawType).as_byte() != 8 || self.baked_mesh_changed(a);
                            check_kept(c.states_caches[i].filter(|(k, _)| *k == key_tuple).map(|(_, v)| v), v, a);
                            c.states_caches[i] = Some((key_tuple, v));
                            v
                        }
                    }
                } || moved.contains(&a),
            };
            if cache {
                c.scratch_ids.push(a);
            }
        }
        if c.scratch_ids != c.caches_ids {
            std::mem::swap(&mut c.scratch_ids, &mut c.caches_ids);
            c.caches = std::rc::Rc::new(c.caches_ids.iter().filter_map(|a| self.baked_names.get(a).cloned()).collect());
        }
        c.caches.clone()
    }

    /// Lighting of the StaticMeshes drawn outside the static geometry, per actor of
    /// `chars`; `verts`: vertices of `character_frame`.
    pub fn prop_lights(&self, chars: &Characters, verts: &CharacterFrame<'_>) -> Vec<Option<crate::renderer::PropLight>> {
        let mut props = chars.props.borrow_mut();
        props.index_draws(&self.vm.display);
        let ViewProps { rows, refreshes, .. } = &mut *props;
        let restoration = self.caches.borrow().prop_restoration;
        let mut names = self.names_lights.borrow_mut();
        let mut out = vec![None; chars.actors.len()];
        for row in rows {
            let owner = row.owner;
            let (a, m) = chars.actors[owner];
            let Model::Prop(model) = &*chars.models[m] else { unreachable!("StaticMesh rank") };
            if chars.removes[owner] { continue; }
            let Some(object) = &self.vm.objects[a] else { continue };
            if verts.meshes[owner].count == 0 { continue; }
            row.inputs.read(&self.vm, a, restoration);
            let geometry = self.prop_geometry(row, a, &chars.models[m], verts, refreshes[&a].as_deref());
            let first_placement = self.vm.first_render.get(&a);
            // CalculateLighting of a bIgnoreDynLight instance: on the actor's first render.
            let first = first_placement.filter(|_| row.inputs.ignore_dyn).map(|p| {
                let placement = Self::placement_of(*p, "");
                row.geometry.first_render.get(Pose::Placement(placement_bits(&placement)), || {
                    WorldGeometry::new(chars.prop_vertices(m, model, &placement).iter().map(|v| (v.position, v.normal)))
                })
            });
            let (bmin, bmax) = (geometry.min, geometry.max);
            out[owner] = Some(crate::renderer::PropLight {
                name: names.object_value(a, &object.name),
                first_vertex: verts.meshes[owner].first,
                instance: chars.instances.get(&a).cloned(),
                first_rendered: first_placement.is_some(),
                geometry: first.unwrap_or(geometry),
                bmin,
                bmax,
                light_group: row.inputs.light_group,
                glow: row.inputs.glow,
                unlit: row.inputs.unlit,
                ignore_dyn: row.inputs.ignore_dyn,
                no_ambient: row.inputs.no_ambient,
                pivot_ambient: row.inputs.pivot_ambient,
                location: row.inputs.location,
                zone: match self.vm.native_value(a, Field::Region) {
                    openxiii_vm::value::Value::Struct(_, f) => f.get(2).map(|z| z.as_int() as u8),
                    _ => None,
                },
                leaves: row.inputs.leaves.clone(),
            });
        }
        out
    }

    /// Scene frame (FCameraSceneNode, FLevelSceneNode::Render): the engine's visibility
    /// (visibility.rs) gives LastRenderTime and LastRenderDist to the drawn actors. Viewpoint:
    /// the last PlayerCalcView (ViewActor, position, rotation), otherwise the given camera.
    pub fn mark_rendered(&mut self, _chars: &Characters, eye: [f32; 3], yaw: f32, pitch: f32, fov: f32) {
        self.scene_visibility(eye, yaw, pitch, fov);
    }

    /// Colour filter of FCameraSceneNode::Render after the scene (main viewport), kept
    /// for the frame as R, G, B, A.
    pub fn camera_filter(&mut self) {
        self.color_filter = self.vm.main_camera_filter(self.controller).map(|c| [c[2], c[1], c[0], c[3]]);
    }

    /// FCameraSceneNode::Render of a frame: UCanvas::Update and PreRender of the
    /// interactions, the scene (`scene_visibility`), then HUD.PostRender and PostRender of the
    /// interactions; returns the Canvas primitives and the render texture views.
    pub fn render_frame(&mut self, width: u32, height: u32, eye: [f32; 3], yaw: f32, pitch: f32, fov: f32) -> (Vec<openxiii_vm::canvas::HudDraw>, Vec<(String, Vec<crate::renderer::TargetView>)>) {
        self.vm.display_window = (width as i32, height as i32);
        let frame = self.vm.hud_begin(None, width, height);
        self.scene_visibility(eye, yaw, pitch, fov);
        self.camera_filter();
        let precaching = frame.is_some_and(|frame| self.vm.hud_end(frame));
        // UGameEngine::Draw: second precache frame (Vm::precache_wanted).
        if self.vm.precache_wanted(precaching) {
            self.vm.precache_render = true;
            let frame = self.vm.hud_begin(None, width, height);
            self.scene_visibility(eye, yaw, pitch, fov);
            self.camera_filter();
            if let Some(frame) = frame {
                let _ = self.vm.hud_end(frame);
            }
            self.vm.precache_render = false;
        }
        let quads = self.vm.hud.clone();
        let targets = self.render_targets();
        (quads, targets)
    }

    fn scene_visibility(&mut self, eye: [f32; 3], yaw: f32, pitch: f32, fov: f32) {
        let (view_actor, origin, rotation) = self.view.unwrap_or_else(|| {
            let r = |d: f32| (d * 65536.0 / 360.0).round() as i32;
            (self.pawn, eye, [r(pitch), r(yaw), 0])
        });
        // Viewport size: that of the last HUD frame (UViewport::SizeX, SizeY).
        let size = (self.vm.viewport.0.max(1) as u32, self.vm.viewport.1.max(1) as u32);
        // UGameEngine::Draw: null ViewActor after PlayerCalcView → Viewport->Actor.
        let node = self.vm.camera_scene_view(self.controller, view_actor.or(self.controller), origin, rotation, fov, size);
        self.vm.render_scene(&node);
        self.vm.scene_bsp_views = self.vm.bsp_views.clone();
    }

    /// BSP nodes and static geometry actors drawn by the player's last view, and
    /// by its sky scenes combined (the viewer's sky pass); None without a traversal.
    pub fn bsp_visible(&self) -> Option<crate::renderer::SceneryVisible> {
        let views = &self.vm.scene_bsp_views;
        let root = views.iter().find(|v| v.sky_zone.is_none())?;
        // List from another map (before the first traversal after a travel): nothing culled.
        if self.vm.level_bsp.as_ref().map(|m| m.nodes.len()) != Some(root.visible.len()) {
            return None;
        }
        let names = |actors: &[ObjId], out: &mut std::collections::HashSet<String>| out.extend(actors.iter().filter_map(|a| self.baked_names.get(a).cloned()));
        let mut d = crate::renderer::SceneryVisible { root: root.visible.clone(), sky: vec![false; root.visible.len()], ..Default::default() };
        names(&root.actors, &mut d.root_owners);
        d.root_actors.extend(root.actors.iter().copied());
        let terrains = |ts: &[ObjId], out: &mut std::collections::HashSet<String>| {
            out.extend(ts.iter().filter_map(|&t| self.vm.objects.get(t).and_then(|o| o.as_ref()).map(|o| o.name.to_lowercase())))
        };
        terrains(&root.terrains, &mut d.root_terrains);
        for v in views.iter().filter(|v| v.sky_zone.is_some()) {
            for (o, &b) in d.sky.iter_mut().zip(&v.visible) {
                *o |= b;
            }
            names(&v.actors, &mut d.sky_owners);
            d.sky_actors.extend(v.actors.iter().copied());
            terrains(&v.terrains, &mut d.sky_terrains);
        }
        Some(d)
    }

    /// Weather of the frame (DrawRndCubeSpr) seen from the same point as
    /// `mark_rendered`, with this view's CameraToWorld.
    pub fn weather(&self, eye: [f32; 3], yaw: f32, pitch: f32) -> (Option<openxiii_vm::weather::WeatherDraw>, [[f32; 4]; 4]) {
        let (origin, rotation) = self.view.map(|(_, o, r)| (o, r)).unwrap_or_else(|| {
            let r = |d: f32| (d * 65536.0 / 360.0).round() as i32;
            (eye, [r(pitch), r(yaw), 0])
        });
        let m = openxiii_vm::weather::camera_to_world(origin, rotation);
        (self.vm.weather_draw(&m), m)
    }

    /// Vertices of all characters at the current time (position, rotation, animation).
    pub fn character_frame<'a>(&mut self, chars: &'a Characters) -> CharacterFrame<'a> {
        // Actors drawn by the view's scene (FDynamicActor::Render, which also places the
        // bone attachments with USkeletalMesh::SetAttachmentLocation) and by the HUD; not those of the
        // render textures alone (URenderTargetMaterial::Update draws its own list): otherwise the
        // comic panel of the guard's fall (Plage00) would show XIII's pawn in the view.
        self.character_frame_of(chars, MeshView::Main)
    }

    /// Shadow silhouette vertices: the pose of each actor drawn by a scene of the frame
    /// (main view, Canvas, render textures); the shadow of an actor that only a comic panel
    /// shows thus follows its pose as in the engine (the owner's USkeletalMeshInstance).
    pub fn for_each_shadow_mesh(
        &mut self,
        chars: &Characters,
        jobs: &[crate::renderer::ShadowRender],
        mut upload: impl FnMut(usize, &[openxiii_assets::world::Vertex], Option<std::rc::Rc<dyn std::any::Any>>),
    ) {
        // Keep the order of the poses for every view that shares the animation cache.
        // Only shadow owners are posed.
        self.visit_character_meshes(chars, MeshView::Shadows, jobs, |_, first, count, vertices| {
            if count != 0 {
                upload(first, vertices.as_ref().map_or(&chars.collapsed[..count], MeshVertices::slice), vertices.as_ref().and_then(MeshVertices::shared_id));
            }
        });
    }

    /// Vertices of the characters drawn by the frame's render textures (target_rendered),
    /// to render their views; None if there are none.
    pub fn character_frame_targets<'a>(&mut self, chars: &'a Characters) -> Option<CharacterFrame<'a>> {
        if self.vm.target_rendered.is_empty() {
            return None;
        }
        Some(self.character_frame_of(chars, MeshView::Targets))
    }

    fn character_frame_of<'a>(&mut self, chars: &'a Characters, view: MeshView) -> CharacterFrame<'a> {
        let mut frame = CharacterFrame {
            meshes: Vec::with_capacity(chars.actors.len()),
            collapsed: &chars.collapsed,
            vertex_count: chars.vertex_count,
        };
        // The CharacterFrame's props come from the immutable local model or the hidden array.
        self.visit_character_meshes(chars, view, &[], |_, first, count, vertices| {
            frame.meshes.push(FrameMesh { first, count, vertices });
        });
        frame
    }

    fn visit_character_meshes(
        &mut self,
        chars: &Characters,
        view: MeshView,
        jobs: &[crate::renderer::ShadowRender],
        mut visit: impl FnMut(usize, usize, usize, Option<MeshVertices>),
    ) {
        let mut visible = chars.visible.borrow_mut();
        visible.clear();
        visible.resize(self.vm.objects.len(), 0);
        for &a in &self.vm.actors {
            if let Some(slot) = visible.get_mut(a) { *slot = 1; }
        }
        let mut include = |a: &ObjId| {
            if let Some(slot) = visible.get_mut(*a) { *slot |= 2; }
        };
        if !matches!(view, MeshView::Targets) {
            self.vm.rendered.iter().chain(&self.vm.canvas_rendered).for_each(&mut include);
        }
        if !matches!(view, MeshView::Main) {
            self.vm.target_rendered.iter().for_each(include);
        }
        for (owner, &(a, index)) in chars.actors.iter().enumerate() {
            let model = &*chars.models[index];
            let first = chars.bases[owner];
            let count = model.vertex_count();
            if matches!(view, MeshView::Shadows) && !jobs.iter().any(|job| job.owner == owner || job.attached.contains(&owner)) {
                continue;
            }
            // FDynamicActor::Render only draws a mesh in DT_Mesh: the carried
            // corpse (XIIIPawn in state Grabbed, DT_None) stays listed and rendered, but invisible.
            let hidden = self.vm.objects[a].is_none()
                || chars.removes[owner]
                || visible[a] != 3
                || (matches!(model, Model::Skeletal(_)) && self.vm.native_prop(a, Field::DrawType).as_byte() != 2);
            if hidden {
                // Actor not drawn this frame: vertices collapsed to an invisible point.
                visit(owner, first, count, None);
                continue;
            }
            match model {
                Model::Skeletal(m) => {
                    // World → mesh frame of GetFrame (skel.rs actor_coords): origin
                    // −(Location + PrePivot), PrePivot in world coordinates, without rotation or
                    // scale (unlike StaticMeshes).
                    let mut placement = self.drawn_placement(a);
                    let pp = placement.pre_pivot;
                    placement.location = [placement.location[0] + pp[0], placement.location[1] + pp[1], placement.location[2] + pp[2]];
                    placement.pre_pivot = [0.0; 3];
                    let advance = false;
                    // Lockstep co-op: drawing never writes a pose (get_frame updates the cached pose,
                    // which the next step's bone attachments read): this machine draws only its own
                    // view, so it would pose only the actors that view shows and the machines differ.
                    let peek = (self.vm.lockstep_on() || (self.vm.preview_drawing && !self.vm.preview_step_drawn.contains(&a))) && !advance;
                    let bases = if peek { self.vm.peek_frame(a) } else { self.vm.get_frame(a) };
                    let mesh = chars.skeletal_vertices(owner, m, &placement, bases);
                    visit(owner, first, count, Some(MeshVertices::Skeletal(mesh)));
                }
                Model::Prop(m) => {
                    let placement = if matches!(view, MeshView::Shadows) {
                        self.drawn_placement(a)
                    } else {
                        // The Renderer's matrix places the props, their vertices stay local.
                        Self::placement_of(openxiii_vm::render_placement::RenderPlacement {
                            location: [0.0; 3], rotation: [0; 3], scale: [1.0; 3], pre_pivot: [0.0; 3],
                        }, "")
                    };
                    let vertices = chars.prop_vertices(index, m, &placement);
                    // SinusModifier sections (Movers included): waving from the actor's SetSinusVertexStreams
                    // this frame (Vm::sine_drawn).
                    let drawn: Vec<(&str, f32, [f32; 3])> = if m.sine.is_empty() {
                        Vec::new()
                    } else {
                        self.vm.sine_drawn.iter().filter(|r| r.0 == a).map(|(_, key, t, amp)| (key.as_str(), *t, *amp)).collect()
                    };
                    if drawn.is_empty() {
                        visit(owner, first, count, Some(MeshVertices::Shared(vertices)));
                    } else {
                        let mut waved = vertices.to_vec();
                        m.wave(&placement, &drawn, &mut waved);
                        visit(owner, first, count, Some(MeshVertices::Animated(waved)));
                    }
                },
            }
        }
    }
}

/// Inputs and results of the previous call of `Game::hidden_baked_with`.
#[derive(Default)]
struct CacheCaches {
    drawn: DrawnActorsCache,
    /// Restoration of the VM's Props by a campaign screenshot.
    prop_restoration: u64,
    placements: PlacementCache,
    characters_list: CharacterList,
    /// Copy of `vm.actors` and its set.
    actors_source: Vec<ObjId>,
    actors_list: Ens<ObjId>,
    /// Actors of the display list entries (`vm.display.entries`) and their set.
    lists_source: Vec<Option<ObjId>>,
    lists: Ens<ObjId>,
    /// Hidden actors and the set of their names.
    scratch_ids: Vec<ObjId>,
    caches_ids: Vec<ObjId>,
    caches: std::rc::Rc<std::collections::HashSet<String>>,
    /// Per actor of `baked` (same rank): (actor, generation of its Props, restoration) and
    /// "DrawType other than DT_StaticMesh or StaticMesh changed" computed with them.
    states_caches: Vec<Option<((ObjId, u64, u64), bool)>>,
    /// Per actor of `baked`: (actor, base, generation of the base's Props, restoration)
    /// and bIgnoreVignetteAlpha read with them.
    states_vignette: Vec<Option<((ObjId, ObjId, u64, u64), bool)>>,
    /// Light lists of the previous frame's views (`scene_light_leaves`).
    lights_list: crate::lights_scene::KeptLightCache,
    /// Actors of `vignette_baked` and the set of their names.
    vignette_ids: Vec<ObjId>,
    vignette: std::rc::Rc<std::collections::HashSet<String>>,
}

#[derive(Default, Clone)]
struct DrawnActors {
    members: std::collections::HashSet<ObjId>,
    order: Vec<ObjId>,
}

#[derive(Default)]
struct DrawnActorsCache {
    sources: [Vec<ObjId>; 3],
    view: std::rc::Rc<DrawnActors>,
}

impl DrawnActorsCache {
    /// Scripts can replace a single list (Canvas, target or scene) without
    /// changing the frame number. Their content and order define the view.
    fn get(&mut self, sources: [&[ObjId]; 3]) -> std::rc::Rc<DrawnActors> {
        if !self.sources.iter().zip(sources).all(|(old, new)| old.as_slice() == new) {
            let view = std::rc::Rc::make_mut(&mut self.view);
            view.members.clear();
            view.order.clear();
            for (old, source) in self.sources.iter_mut().zip(sources) {
                old.clear();
                old.extend_from_slice(source);
                for &actor in source {
                    if view.members.insert(actor) {
                        view.order.push(actor);
                    }
                }
            }
        }
        self.view.clone()
    }
}

/// Farther shadows (improved build, Graphics: Enhanced; not in the original game): in XIII a
/// shadow fades out between ShadowTransDist (1000) and ShadowMaxDist (1500) of LastRenderDist,
/// the plain camera distance, whatever the zoom. Here the distance
/// given to UpdateShadow is divided by FAR_SHADOW_SCALE and by the zoom of the view
/// (tan(DefaultFOV / 2) / tan(FovAngle / 2), at least 1), so a shadow lasts four times
/// farther and stays on an enemy seen through a scope. 1 (original behaviour) otherwise.
fn far_shadow_scale(enhanced: bool, fov: Option<(f32, f32)>) -> f32 {
    if !enhanced {
        return 1.0;
    }
    let zoom = fov.map_or(1.0, |(fov, default)| {
        let t = |d: f32| (d.clamp(1.0, 170.0).to_radians() * 0.5).tan();
        (t(default) / t(fov)).max(1.0)
    });
    FAR_SHADOW_SCALE * zoom
}

/// How many times farther shadows reach with farther shadows on.
const FAR_SHADOW_SCALE: f32 = 4.0;

#[cfg(test)]
#[test]
fn far_shadow_original_unchanged() {
    for fov in [None, Some((85.0, 85.0)), Some((20.0, 85.0)), Some((120.0, 85.0)), Some((0.0, 0.0))] {
        assert_eq!(far_shadow_scale(false, fov), 1.0);
    }
}

#[cfg(test)]
#[test]
fn far_shadow_enhanced_reach_and_zoom() {
    assert_eq!(far_shadow_scale(true, None), 4.0);
    assert!((far_shadow_scale(true, Some((85.0, 85.0))) - 4.0).abs() < 1e-5);
    let scoped = far_shadow_scale(true, Some((20.0, 85.0)));
    // tan(42.5°) / tan(10°) = 5.197
    assert!((scoped - 4.0 * 5.197).abs() < 0.01, "{scoped}");
    for fov in [86.0, 100.0, 120.0, 170.0, 179.0] {
        assert!(far_shadow_scale(true, Some((fov, 85.0))) >= 4.0);
    }
}

#[cfg(test)]
#[test]
fn drawn_view_tracks_order_across_canvas_and_targets() {
    let mut cache = DrawnActorsCache::default();
    let first = cache.get([&[4, 2], &[4, 5], &[3, 2]]);
    assert_eq!(first.order, [4, 2, 5, 3]);
    assert!(std::rc::Rc::ptr_eq(&first, &cache.get([&[4, 2], &[4, 5], &[3, 2]])));
    let next = cache.get([&[2, 4], &[], &[3]]);
    assert_eq!(next.order, [2, 4, 3]);
    assert!(!next.members.contains(&5) && first.members.contains(&5));
}

#[derive(Default)]
struct PlacementCache {
    generations: Vec<Option<u64>>,
    moved: std::rc::Rc<Ens<ObjId>>,
}

#[derive(Default)]
struct CharacterList {
    actors: Vec<ActorStamp>,
    membership: Vec<u8>,
    order: Vec<ObjId>,
    version: u64,
    model_count: usize,
    available: Table<String, bool>,
    /// Last version where a mesh was re-read or lost (not a simple removal); versions
    /// increase, the signature may be recomputed several times between two frames.
    reread_version: u64,
    /// Like `reread_version`, without the meshes read for an actor that had none
    /// (appearance): `Game::characters_add`.
    reread_existing_version: u64,
}

#[derive(Default)]
struct ActorStamp {
    generation: u64,
    model_count: usize,
    membership: u8,
    mesh: Option<Box<MeshInputs>>,
    /// Version of the list where `mesh` was last read or lost (`Game::characters_add`).
    value_read: u64,
}

struct MeshInputs {
    prop: bool,
    values: [Value; 3],
    /// Improved build: the mesh drawn instead of the actor's own (a co-op player's arms,
    /// Vm::arms_mesh_override), which can come after the actor was first listed.
    arms: Option<String>,
}

impl MeshInputs {
    const FIELDS: [Field; 3] = [Field::StaticMesh, Field::Mesh, Field::Skins];

    fn read(vm: &Vm, a: ObjId, prop: bool) -> Self {
        Self { prop, values: Self::FIELDS.map(|field| vm.native_value(a, field)), arms: arms_of(vm, a) }
    }

    fn matches(&self, vm: &Vm, a: ObjId) -> bool {
        self.values.iter().zip(Self::FIELDS).all(|(v, field)| same_value(v, vm.native_prop(a, field))) && self.arms.as_deref() == arms_of(vm, a).as_deref()
    }

    fn differences(&self, vm: &Vm, a: ObjId) -> String {
        let names_list = ["StaticMesh", "Mesh", "Skins"];
        let d: Vec<String> = self
            .values
            .iter()
            .zip(Self::FIELDS)
            .zip(names_list)
            .filter(|((v, field), _)| !same_value(v, vm.native_prop(a, *field)))
            .map(|((v, field), n)| format!("{n} {v} -> {}", vm.native_prop(a, field)))
            .collect();
        let mut d = d;
        if self.arms.as_deref() != arms_of(vm, a).as_deref() {
            d.push(format!("arms {:?} -> {:?}", self.arms, arms_of(vm, a)));
        }
        if d.is_empty() { "no field".to_owned() } else { d.join(", ") }
    }
}

/// The co-op arms drawn for actor `a` instead of its mesh (improved build), else None.
fn arms_of(vm: &Vm, a: ObjId) -> Option<String> {
    player_arms_mesh(vm, a).map(str::to_owned)
}

/// Number of indices of all batches of a model (space the rank takes in the index array).
fn model_indices(m: &Model) -> usize {
    m.batches().iter().map(|(_, local)| local.len()).sum()
}

/// By default, it is hidden at its rank then made visible again at the same rank (order of a rebuild),
/// without re-uploading everything.
pub(crate) fn characters_revived() -> bool {
    true

}

fn revives_bad_rank() -> bool {
    false
}

/// Like the old fingerprint, distinguishes float bits, including in the Skins.
fn same_value(a: &Value, b: &Value) -> bool {
    let values = |x: &[Value], y: &[Value]| x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_value(x, y));
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Array(x), Value::Array(y)) => values(x, y),
        (Value::Struct(k, x), Value::Struct(l, y)) => k == l && values(x, y),
        // Inline vector / rotator: compared as the struct they are (float bits).
        _ if a.is_struct() && b.is_struct() => match (a.struct_view(), b.struct_view()) {
            (Some((k, x)), Some((l, y))) => k == l && values(&x, &y),
            _ => false,
        },
        _ => a == b,
    }
}

// The reasons recorded by characters_to_day, characters_add and the signature (re-read
// meshes) wait for the line written by list_characters; a kept list clears them.

/// Launch time (App::started), to timestamp the log lines.
pub static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

thread_local! {
    static DBG_NOTES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static DBG_REREAD: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn dbg_reason() -> bool {
    false
}

fn dbg_note(s: String) {
    DBG_NOTES.with(|n| n.borrow_mut().push(s));
}

fn dbg_reread(s: String) {
    DBG_REREAD.with(|n| {
        let mut n = n.borrow_mut();
        if n.len() < 64 {
            n.push(s);
        }
    });
}

fn dbg_empty() {
    DBG_NOTES.with(|n| n.borrow_mut().clear());
    DBG_REREAD.with(|n| n.borrow_mut().clear());
}

/// Simulated screenshot time (main.rs), when there is no window.
pub static TIME_CAPTURE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x7fc0_0000);

pub fn dbg_time() -> f32 {
    START.get().map_or_else(|| f32::from_bits(TIME_CAPTURE.load(std::sync::atomic::Ordering::Relaxed)), |d| d.elapsed().as_secs_f32())
}

fn dbg_class(vm: &Vm, a: ObjId, rank: Option<usize>) -> String {
    let class_value = vm.objects.get(a).and_then(|o| o.as_ref()).and_then(|o| vm.classes.get(o.class)).map_or("?", |c| c.name.as_str());
    match rank {
        Some(k) => format!("#{a} {class_value} rank {k}"),
        None => format!("#{a} {class_value}"),
    }
}

fn dbg_write(path_buf: &str, detail: &str) {
    let notes = DBG_NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()));
    let reread_count = DBG_REREAD.with(|n| std::mem::take(&mut *n.borrow_mut()));
    let mut s = format!("list reason t={:.3} path={path_buf}", dbg_time());
    for n in &notes {
        s.push_str(" || ");
        s.push_str(n);
    }
    if !reread_count.is_empty() {
        s.push_str(&format!(" || meshes reread since the last line ({}): {}", reread_count.len(), reread_count.join(" | ")));
    }
    if !detail.is_empty() {
        s.push_str(" || ");
        s.push_str(detail);
    }
    crate::renderer::diag(&s);
}

/// Material of a skeletal batch: the actor's texture `skin` for it (Skins), else its own. Mods
/// build only: a mod model's outline shell batch keeps its marker, followed by the texture its
/// part is drawn with (the outline rule depends on it).
fn skeletal_material(material: &str, skin: Option<String>) -> String {
    #[cfg(feature = "mods")]
    if let Some(part) = openxiii_assets::mod_meshes::outline_shell_of(material) {
        return format!("{}{}", openxiii_assets::mod_meshes::OUTLINE_SHELL, skin.unwrap_or_else(|| part.to_string()));
    }
    skin.unwrap_or_else(|| material.to_string())
}

register_viewer_layer!("world");
