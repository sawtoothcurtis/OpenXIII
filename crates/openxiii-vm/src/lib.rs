//! UnrealScript virtual machine: runs the bytecode of XIII's packages.
//!
//! Directly interprets the expression tree decoded by `openxiii-script`. The
//! object and name references of the bytecode are resolved by name in the
//! package of the current function (properties, functions, classes).

/// Debug environment variable read once (the OXIII_* variables do not change during a
/// run): `env_once!("NAME")` is an Option<&'static str>. Avoids reading the environment
/// block on every function call or trace.
#[macro_export]
macro_rules! env_once {
    ($name:expr) => {{
        static V: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        V.get_or_init(|| openxiii_package::env::var(const { $name }).ok()).as_deref()
    }};
}

#[path = "script/sub_objects.rs"]
mod sub_objects;
#[path = "script/sorting.rs"]
mod sorting;
#[cfg(feature = "mods")]
#[path = "mods/mod_state.rs"]
mod mod_state;
/// Mods build only: class default values changed by mods.
#[cfg(feature = "mods")]
#[path = "mods/mod_defaults.rs"]
pub mod mod_defaults;
/// Mods build only: changes to the actors of maps, start weapons, modpos.
#[cfg(feature = "mods")]
#[path = "mods/mod_placement.rs"]
pub mod mod_placement;
#[cfg(feature = "mods")]
#[path = "mods/mod_change.rs"]
mod mod_change;
/// Mods build only: the skins mods give to actors, shown on network guests.
#[cfg(feature = "mods")]
#[path = "mods/mod_net_skins.rs"]
pub mod mod_net_skins;
/// Mods build only: the shared lookups of a mod's names (`who` of placed actors, `look`).
#[cfg(feature = "mods")]
#[path = "mods/mod_names.rs"]
pub mod mod_names;
/// Mods build only: the names catalog ("Modding catalog.html"), built from the player's game.
#[cfg(feature = "mods")]
#[path = "mods/mod_catalog.rs"]
pub mod mod_catalog;
/// Mods build only: the lists of names written next to the names catalog.
#[cfg(feature = "mods")]
#[path = "mods/mod_names_doc.rs"]
pub mod mod_names_doc;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_catalog_test.rs"]
mod mod_catalog_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_simple_test.rs"]
mod mod_simple_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_net_skins_test.rs"]
mod mod_net_skins_test;
/// Mods build only: keys bound to mod actions, mod characters and their behaviour.
#[cfg(feature = "mods")]
#[path = "mods/mod_behaviour.rs"]
pub mod mod_behaviour;
/// Mods build only: sounds and content from any level.
#[cfg(feature = "mods")]
#[path = "mods/mod_sounds.rs"]
pub mod mod_sounds;
/// Mods build only: lines said by characters (`[[map.<Map>.say]]`).
#[cfg(feature = "mods")]
#[path = "mods/mod_dialogue.rs"]
pub mod mod_dialogue;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_dialogue_test.rs"]
mod mod_dialogue_test;
/// Mods build only: the mods' own sounds (WAV, OGG).
#[cfg(feature = "mods")]
#[path = "mods/mod_own_sounds.rs"]
pub mod mod_own_sounds;
/// Mods build only: the mods' Lua scripts (`scripts/*.lua`).
#[cfg(feature = "mods")]
#[path = "mods/mod_script.rs"]
pub mod mod_script;
/// Mods build only: the scripts' orders to characters and to the game.
#[cfg(feature = "mods")]
#[path = "mods/mod_script_game.rs"]
pub mod mod_script_game;
#[cfg(feature = "mods")]
#[path = "mods/mod_script_panel.rs"]
pub mod mod_script_panel;
#[cfg(feature = "mods")]
#[path = "mods/mod_script_camera.rs"]
pub mod mod_script_camera;
#[cfg(feature = "mods")]
#[path = "mods/mod_script_effects.rs"]
pub mod mod_script_effects;
#[cfg(feature = "mods")]
#[path = "mods/mod_script_keys.rs"]
pub mod mod_script_keys;
/// Mods build only: combat events, shortcuts on actors, game(), preload(), sounds stopped.
#[cfg(feature = "mods")]
#[path = "mods/mod_script_more.rs"]
pub mod mod_script_more;
#[cfg(feature = "mods")]
#[path = "mods/mod_preload.rs"]
mod mod_preload;
/// Mods build only: what scripts keep on the screen, and the loading screen between maps.
#[cfg(feature = "mods")]
#[path = "mods/mod_script_hud.rs"]
pub mod mod_script_hud;
/// Mods build only: textures scripts change on one actor of the scenery.
#[cfg(feature = "mods")]
#[path = "mods/mod_retexture.rs"]
pub mod mod_retexture;
/// Mods build only: what the host's scripts make appear, shown on network guests.
#[cfg(feature = "mods")]
#[path = "mods/mod_net_fx.rs"]
pub mod mod_net_fx;
/// Mods build only: heat shimmer over areas asked by scripts.
#[cfg(feature = "mods")]
#[path = "mods/mod_heat.rs"]
pub mod mod_heat;
/// Mods build only: the scripts' time(), trace() and effect{}.
#[cfg(feature = "mods")]
#[path = "mods/mod_script_world.rs"]
pub mod mod_script_world;
/// Mods build only: the scripts' memory (`save`), kept with the game's checkpoints.
#[cfg(feature = "mods")]
#[path = "mods/mod_script_save.rs"]
pub mod mod_script_save;
/// Mods build only: "when ... then ..." rules of the mods' toml files, run by the scripts' engine.
#[cfg(feature = "mods")]
#[path = "mods/mod_when.rs"]
pub mod mod_when;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_script_test.rs"]
mod mod_script_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_when_toml_test.rs"]
mod mod_when_toml_test;
/// Mods build only: the mods' new classes ("Mod.<Name>").
#[cfg(feature = "mods")]
#[path = "mods/mod_classes.rs"]
pub mod mod_classes;
#[cfg(feature = "mods")]
#[path = "mods/mod_models.rs"]
pub mod mod_models;
#[cfg(feature = "mods")]
#[path = "mods/mod_net.rs"]
pub mod mod_net;
#[cfg(feature = "mods")]
#[path = "mods/mod_setup.rs"]
pub mod mod_setup;
/// Mods build only: weapon settings of mods that are not class properties (animation speeds).
#[cfg(feature = "mods")]
#[path = "mods/mod_weapons.rs"]
pub mod mod_weapons;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_weapons_test.rs"]
mod mod_weapons_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_effects_test.rs"]
mod mod_effects_test;
/// Mods build only: the maps reference page (docs/modding/maps.md).
#[cfg(feature = "mods")]
#[path = "mods/mod_maps.rs"]
pub mod mod_maps;
/// Display settings added by OpenXIII ([OpenXIII.Video] of XIII.ini).
#[path = "gui/video_settings.rs"]
pub mod video_settings;
/// How the players in front of this screen want to play (co-op session, players on this screen).
#[path = "coop/local_play.rs"]
pub mod local_play;
#[cfg(feature = "mods")]
#[path = "mods/mod_character_models.rs"]
mod mod_character_models;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_selectors_test.rs"]
mod mod_selectors_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_additions_test.rs"]
mod mod_additions_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_defaults_test.rs"]
mod mod_defaults_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_placement_test.rs"]
mod mod_placement_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_change_test.rs"]
mod mod_change_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_behaviour_test.rs"]
mod mod_behaviour_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_sounds_test.rs"]
mod mod_sounds_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_own_sounds_test.rs"]
mod mod_own_sounds_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_meshes_test.rs"]
mod mod_meshes_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_classes_test.rs"]
mod mod_classes_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_models_test.rs"]
mod mod_models_test;
#[cfg(all(test, feature = "mods"))]
#[path = "../tests/unit/mods/mod_net_test.rs"]
mod mod_net_test;
#[path = "script/flags_classes.rs"]
mod flags_classes;
#[path = "render/effects_angle.rs"]
pub mod effects_angle;
#[path = "render/display_format.rs"]
pub mod display_format;

pub mod tally {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    pub const EXPRS: usize = 0;
    pub const CALLS: usize = 1;
    pub const NATIVES: usize = 2;
    pub const EVENTS: usize = 3;
    pub const ACTOR_TICKS: usize = 4;
    pub const STASIS: usize = 5;
    pub const TIMERS: usize = 6;
    /// performPhysics per value of Physics (8 + Physics).
    pub const EMITTERS: usize = 7;
    pub const PHYSICS: usize = 8;
    pub const SUB_EMITTERS: usize = 24;
    pub const PARTICLES: usize = 25;
    pub const N: usize = 26;
    pub static C: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
    #[inline]
    pub fn inc(k: usize) {
        let _ = k;
    }
    #[inline]
    pub fn add_value(k: usize, n: u64) {
        let _ = (k, n);
    }
    pub fn read_from() -> [u64; N] {
        std::array::from_fn(|k| C[k].load(Relaxed))
    }
}
#[path = "engine/cheats.rs"]
mod cheats;
#[path = "gameplay/indices_broken.rs"]
mod indices_broken;
#[path = "diagnostics/gc_log.rs"]
mod gc_log;
#[path = "diagnostics/draw_kind.rs"]
pub mod draw_kind;
#[path = "gameplay/demo_dog.rs"]
pub mod demo_dog;
#[path = "gameplay/demo_bar.rs"]
pub mod demo_bar;
#[path = "world/actor_iter.rs"]
pub mod actor_iter;
#[path = "storage/load_order.rs"]
pub mod load_order;
#[path = "engine/xbox_live.rs"]
pub mod xbox_live;
#[path = "gui/loading_anim.rs"]
pub mod loading_anim;
#[path = "engine/main_loop.rs"]
pub mod main_loop;
#[path = "engine/init_engine.rs"]
pub mod init_engine;
#[path = "physics/actor_physics.rs"]
pub mod actor_physics;
#[path = "ai/ai_move.rs"]
pub mod ai_move;
#[path = "gameplay/aim.rs"]
pub mod aim;
#[path = "animation/anim.rs"]
pub mod anim;
#[path = "audio/audio.rs"]
pub mod audio;
#[path = "animation/bone_attach.rs"]
pub mod bone_attach;
#[path = "animation/bone_hit.rs"]
pub mod bone_hit;
#[path = "render/bsp_draw.rs"]
pub mod bsp_draw;
#[path = "ai/bot_native.rs"]
pub mod bot_native;
#[path = "gui/canvas.rs"]
pub mod canvas;
mod improved;
improved::register_modules!(include!("improved/modules.rs"););

#[path = "script/call_hooks.rs"]
mod call_hooks;
#[path = "script/class_gc.rs"]
pub mod class_gc;
#[path = "engine/client.rs"]
pub mod client;
#[path = "coop/coop.rs"]
pub mod coop;
#[path = "coop/coop_spawns.rs"]
pub mod coop_spawns;
#[path = "coop/split_versus.rs"]
pub mod split_versus;
#[path = "coop/split_layout.rs"]
pub mod split_layout;
#[path = "coop/coop_combat.rs"]
mod coop_combat;
#[path = "coop/coop_enemies.rs"]
pub mod coop_enemies;
#[cfg(test)]
#[path = "../tests/unit/coop/coop_enemies_test.rs"]
mod coop_enemies_test;
#[path = "coop/coop_address.rs"]
pub mod coop_address;
#[path = "net/lockstep.rs"]
pub mod lockstep;
#[path = "net/lockstep_menus.rs"]
mod lockstep_menus;
#[path = "net/net_smooth.rs"]
mod net_smooth;
#[path = "net/lockstep_meet.rs"]
pub mod lockstep_meet;
#[path = "net/lockstep_find.rs"]
pub mod lockstep_find;
#[path = "net/lockstep_chat.rs"]
pub mod lockstep_chat;
#[path = "net/net_reconnect.rs"]
pub mod net_reconnect;
#[path = "coop/coop_rules.rs"]
mod coop_rules;
#[path = "coop/coop_generators.rs"]
mod coop_generators;
#[path = "coop/coop_items.rs"]
mod coop_items;
#[path = "coop/coop_shared.rs"]
mod coop_shared;
#[path = "coop/coop_story.rs"]
mod coop_story;
#[path = "coop/coop_cine.rs"]
mod coop_cine;
#[path = "coop/coop_hints.rs"]
mod coop_hints;
#[path = "coop/coop_end.rs"]
mod coop_end;
#[path = "coop/coop_flash.rs"]
mod coop_flash;
#[path = "coop/coop_world.rs"]
mod coop_world;
#[path = "gameplay/cutscene_native.rs"]
pub mod cutscene_native;
#[path = "engine/app_init.rs"]
pub mod app_init;
#[path = "engine/config.rs"]
pub mod config;
#[path = "engine/console.rs"]
pub mod console;
#[path = "gui/gui.rs"]
pub mod gui;
#[path = "gui/gui_native.rs"]
pub mod gui_native;
#[path = "gui/gui_glue.rs"]
mod gui_glue;
#[path = "gui/hud_native.rs"]
pub mod hud_native;
#[path = "ai/ai_natives.rs"]
pub mod ai_natives;
#[path = "ai/reachability.rs"]
pub(crate) mod reachability;
#[path = "ai/path_toward.rs"]
pub(crate) mod path_toward;
#[path = "input/input.rs"]
pub mod input;
#[path = "net/ipdrv.rs"]
pub mod ipdrv;
#[path = "input/interaction.rs"]
pub mod interaction;
#[path = "physics/kdop.rs"]
pub mod kdop;
#[path = "script/layout.rs"]
pub mod layout;
#[path = "storage/load.rs"]
pub mod load;
#[path = "audio/material_sound.rs"]
pub mod material_sound;
#[path = "physics/mover_physics.rs"]
pub mod mover_physics;
#[path = "physics/octree.rs"]
pub mod octree;
#[path = "script/natives.rs"]
pub mod natives;
#[path = "input/force_feedback.rs"]
pub mod force_feedback;
#[path = "input/pad.rs"]
pub mod pad;
#[path = "input/aim_device.rs"]
pub mod aim_device;
#[path = "input/gamepad_config.rs"]
pub mod gamepad_config;
#[path = "ai/navigation.rs"]
pub mod navigation;
#[path = "net/net.rs"]
pub mod net;
#[path = "net/server.rs"]
pub mod server;
#[path = "net/net_file.rs"]
pub mod net_file;
#[path = "net/net_game.rs"]
pub mod net_game;
#[path = "storage/order_original.rs"]
mod order_original;
#[path = "net/net_map.rs"]
pub mod net_map;
#[path = "net/net_actor.rs"]
pub mod net_actor;
#[path = "net/net_rep.rs"]
pub mod net_rep;
#[path = "net/net_replist.rs"]
pub mod net_replist;
#[path = "net/netbits.rs"]
pub mod netbits;
#[path = "net/net_tri.rs"]
pub mod net_tri;
#[cfg(test)]
#[path = "../tests/unit/coop/split_menu_input_test.rs"]
mod split_menu_input_test;
#[path = "render/particles.rs"]
pub mod particles;
#[path = "physics/pawn_physics.rs"]
pub mod pawn_physics;
#[path = "ai/perception.rs"]
pub mod perception;
#[path = "physics/physics.rs"]
pub mod physics;
#[path = "render/projector.rs"]
pub mod projector;
#[path = "input/player_input.rs"]
pub mod player_input;
#[path = "gameplay/player_native.rs"]
pub mod player_native;
#[path = "world/radius_iter.rs"]
pub mod radius_iter;
#[path = "storage/save.rs"]
pub mod save;
#[path = "net/session_disk.rs"]
pub mod session_disk;
/// Lockstep co-op: the periodic full fingerprint (detection of differences, every 5 s).
#[path = "net/lockstep_full.rs"]
pub mod lockstep_full;
/// Lockstep co-op: the other actors drawn between two steps (drawing only).
#[path = "net/lockstep_interp.rs"]
pub mod lockstep_interp;
#[path = "net/lockstep_anchor.rs"]
pub mod lockstep_anchor;
#[path = "storage/zlib11.rs"]
pub mod zlib11;
#[path = "render/screen_fx.rs"]
pub mod screen_fx;
#[path = "render/sine.rs"]
pub mod sine;
#[path = "animation/skel.rs"]
pub mod skel;
#[path = "gameplay/solo_native.rs"]
pub mod solo_native;
#[path = "physics/spider_physics.rs"]
pub mod spider_physics;
#[path = "engine/stat_log.rs"]
pub mod stat_log;
#[path = "physics/static_collision.rs"]
pub mod static_collision;
#[path = "physics/terrain_check.rs"]
pub mod terrain_check;
#[path = "engine/tick.rs"]
pub mod tick;
#[path = "render/trail.rs"]
pub mod trail;
#[path = "render/trail_render.rs"]
pub mod trail_render;
#[path = "render/dyn_light.rs"]
pub mod dyn_light;
#[path = "render/light_wrapper.rs"]
mod light_wrapper;
#[path = "render/scene_lights.rs"]
mod scene_lights;
#[path = "render/scene_light_distribution.rs"]
mod scene_light_distribution;
#[path = "render/scene_light_pass.rs"]
pub mod scene_light_pass;
#[path = "render/dyn_init_cull.rs"]
pub mod dyn_init_cull;
#[path = "storage/game_data.rs"]
pub mod game_data;
#[path = "gameplay/select_level.rs"]
pub mod select_level;
#[path = "render/filter.rs"]
pub mod filter;
#[path = "render/camera_render.rs"]
pub mod camera_render;
#[path = "storage/travel.rs"]
pub mod travel;
#[path = "net/uweb.rs"]
pub mod uweb;
#[path = "script/types.rs"]
pub mod types;
#[path = "script/value.rs"]
pub mod value;
#[path = "script/native_fields.rs"]
mod native_fields;
#[path = "render/render_placement.rs"]
pub mod render_placement;
pub use native_fields::NativeField;
#[path = "render/video.rs"]
pub mod video;
#[path = "input/drag.rs"]
pub mod drag;
#[path = "input/resize.rs"]
pub mod resize;
#[path = "input/windrv_input.rs"]
pub mod windrv_input;
#[path = "input/wndproc.rs"]
pub mod wndproc;
#[path = "render/visibility.rs"]
pub mod visibility;
#[path = "physics/volumes.rs"]
pub mod volumes;
#[path = "render/weather.rs"]
pub mod weather;
#[path = "world/world.rs"]
pub mod world;

use load::LoadedPackage;
use openxiii_package::ObjectRef;
use openxiii_script::bytecode::{Expr, Kind};
use std::collections::HashMap;
use std::fmt;
use std::borrow::Cow;
use std::rc::Rc;
use types::*;
use value::*;

#[derive(Debug, Clone)]
pub enum VmError {
    Unsupported(String),
    UnknownFunction(String),
    UnknownProperty(String),
    UnknownClass(String),
    Native(String),
    TooDeep,
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::Unsupported(s) => write!(f, "not supported: {s}"),
            VmError::UnknownFunction(s) => write!(f, "unknown function: {s}"),
            VmError::UnknownProperty(s) => write!(f, "unknown property: {s}"),
            VmError::UnknownClass(s) => write!(f, "unknown class: {s}"),
            VmError::Native(s) => write!(f, "native: {s}"),
            VmError::TooDeep => write!(f, "recursion too deep"),
        }
    }
}

impl std::error::Error for VmError {}

pub type Result<T> = std::result::Result<T, VmError>;

/// Writable place (target of an assignment or of an `out` parameter).
#[derive(Debug, Clone)]
pub enum Place {
    Local(usize),
    Prop(ObjId, usize),
    Element(Box<Place>, usize),
    Field(Box<Place>, usize),
    /// Length of a dynamic array (`A.Length = n` resizes it).
    Length(Box<Place>),
    /// Assignment through a null context: ignored.
    Nowhere,
}

/// Core natives that call GPropObject->NetDirty(GProperty) on their out parameters (UObject::execAddAdd_*, execSubtractSubtract_*,
/// exec{Add,Subtract,Multiply,Divide}Equal_*, execGetAxes, execGetUnAxes, execInvert).
fn net_dirty_native(name_value: &str) -> bool {
    ["AddAdd_", "SubtractSubtract_", "AddEqual_", "SubtractEqual_", "MultiplyEqual_", "DivideEqual_"].iter().any(|p| name_value.starts_with(p))
        || ["GetAxes", "GetUnAxes", "Invert"].iter().any(|n| name_value.eq_ignore_ascii_case(n))
}

struct Frame {
    this: ObjId,
    function: Option<FuncId>,
    locals: Vec<Value>,
    package: usize,
    switch: Option<Value>,
    iterators: Vec<IteratorState>,
}

/// foreach loop in progress: remaining elements and loop variable.
struct IteratorState {
    items: Vec<Value>,
    index: usize,
    place: Place,
    /// Other `out` variables of the iterator (TraceActors): the element is then an array
    /// [loop variable, extras…].
    extras: Vec<Place>,
    body: usize,
    /// Iterator walked during the loop (AllActors, RadiusActors…), otherwise `items`.
    lazy: Option<actor_iter::LazyIter>,
}

/// End of execution of a sequence of statements.
pub(crate) enum Flow {
    Return(Value),
    Stop,
    /// State code suspended by a latent function; resumes at the given statement.
    Suspend(usize),
}

/// Latent action requested by a native called from state code.
#[derive(Debug, Clone)]
pub enum Latent {
    /// FinishAnim: waits for the end of the sequence on the LatentFloat channel (PollFinishAnim).
    FinishAnim,
}

/// A DrawPortal call as the test reads it back.
#[derive(Clone, Debug)]
pub struct PortalCall {
    pub rect: [i32; 4],
    pub actor: Option<ObjId>,
    pub location: [f32; 3],
    pub rotation: [i32; 3],
    pub fov: i32,
    pub clear_z: bool,
    pub fov_angle: f32,
}

pub struct Vm {
    native_slots: Vec<native_fields::NativeSlots>,
    tick_slots: Vec<tick::TickSlots>,
    /// prop_slot cache: (class, name as requested, slot) stored by class and address of the
    /// name; an entry is only used if the class and the name's text are the same. Looking up the
    /// lowercase name in prop_index on every property read cost ~10% of a frame.
    /// The name is kept in the entry itself (32 bytes at most; longer names are not
    /// cached): the comparison no longer reads a string elsewhere in memory.
    slot_cache: std::cell::RefCell<Vec<SlotEntry>>,
    /// prop_slot of the code's literal names (`SlotLiteral`).
    slot_literals: std::cell::RefCell<Vec<SlotLiteral>>,
    /// Probe masks of each object's current state (`Vm::masks_state`), by ObjId: kept
    /// as long as the state's name, the declaring class, the object's class and the number of loaded
    /// classes are the same. Each event used to read the state again (lowercase + state table).
    states_resolved: std::cell::RefCell<Vec<Option<StateResolved>>>,
    /// `states_resolved` used (true outside tests: some tests modify the state tables in place).
    pub(crate) cache_states: bool,
    pub packages: Vec<LoadedPackage>,
    pub classes: Vec<ClassDef>,
    pub class_index: HashMap<String, ClassId>,
    /// Classes by "package.class" path (lowercase): two packages can declare a class
    /// with the same name (XIDPawn.Johansson and XIIIPersos.Johansson, XIII.Mouette and XIIIPersos.Mouette).
    pub class_qualified: HashMap<String, ClassId>,
    pub functions: Vec<FunctionDef>,
    call_hooks: call_hooks::CallHooks,
    pub struct_exports: HashMap<String, (usize, usize)>,
    pub structs: HashMap<String, StructDef>,
    pub objects: Vec<Option<Object>>,
    /// `Log` messages.
    pub log: Vec<String>,
    /// Failed assertions and warnings ("Accessed None"…).
    pub warnings: Vec<String>,
    pub(crate) native_index: types::HashMap<u16, FuncId>,
    depth: usize,
    /// RF_InSingularFunc: objects running a singular function.
    in_singular: std::collections::HashSet<ObjId>,
    /// Seed of the pseudo-random generator (Rand, FRand).
    pub(crate) seed: u32,
    /// Particle emitters: native state (particles.rs).
    pub particles: particles::ParticleSystems,
    /// Load order of the emulated linker (load_order.rs): objects already loaded, for the order
    /// of PostLoads (draws of the ParticleEmitters and Emitters) at load time and in game.
    pub(crate) load_order: Option<load_order::LoadOrder>,
    /// Script objects loaded from a package (subobjects of default values:
    /// ParticleEmitters, fonts and menu controls…), by lowercase path.
    pub(crate) subobjects: HashMap<String, ObjId>,
    /// Paths (lowercase) of the created objects, for UObject::MakeUniqueObjectName.
    object_paths: std::collections::HashSet<String>,
    /// Actor whose state code is running (to locate its warnings).
    pub(crate) state_code_obj: Option<ObjId>,
    /// UClass::ClassUnique: counter of object names for each class.
    class_unique: HashMap<ClassId, u16>,
    /// Names (lowercase) of the loaded packages' tables, for an FName known but outside the map (SerializeName), with
    /// the number of packages they come from.
    pub(crate) names_loaded: std::cell::RefCell<(usize, std::collections::HashSet<String>)>,
    /// Original indices (GObjObjects) of a map's objects, recorded from the original: lowercase path → index.
    /// Empty: object walks follow the creation order (see `order_original.rs`).
    pub(crate) order_original: HashMap<String, u32>,
    /// Paths of the map's Sound objects in GObjObjects index order (FillSoundsArray, `order_original.rs`); empty: no record.
    pub(crate) order_sounds: Vec<String>,
    /// Walk of `order_sounds` already done by `audio_sound_by_cuuid` (see `IndexSounds`); reset
    /// to zero when `order_sounds` or `audio_sounds` changes.
    pub(crate) index_sounds: audio::IndexSounds,
    /// Last map whose record was chosen (to know where the next travel comes from).
    pub(crate) order_map: Option<String>,
    /// Objects carrying the RF 0x8000000 flag set by the "OBJ MARK" console command (UObject::StaticExec).
    pub(crate) objects_marked: std::collections::HashSet<ObjId>,
    /// ClassWithin declared by a class (class name; missing: Object).
    pub(crate) class_within: HashMap<ClassId, String>,
    /// Name of the last evaluated `class'Name'` constant whose class is not in the VM (StaticMesh, Mesh, Font, Class…): the class
    /// requested from DynamicLoadObject, which Value::Class(None) does not tell.
    pub(crate) meta_const: Option<String>,
    /// Parent of each class of the loaded packages (lowercase name), computed at the first DynamicLoadObject.
    pub(crate) native_class_parents: Option<HashMap<String, String>>,
    /// Travel to the same map (UGameEngine::LoadMap: URL.Map == LastURL.Map): the
    /// CollectGarbage before the load is skipped, only the one after remains; what the reloaded
    /// level refers to therefore survives (classes and values of its objects, class_gc.rs).
    pub(crate) same_map_roots: Option<Vec<Value>>,
    /// Objects created by `new` (UObject::execNew): neither RF_RootSet nor RF_Native, the travel's
    /// garbage collection only keeps them if they are reachable (class_gc.rs).
    pub(crate) objects_new: std::collections::HashSet<ObjId>,
    /// IpDrv.dll: sockets of the AUdpLinks and the listening TcpNetDriver (ipdrv.rs).
    pub ipdrv: ipdrv::IpDrv,
    /// UXboxLive created and initialized at startup (XboxLiveDevice=Engine.XboxLive, xbox_live.rs).
    pub xbox_live: xbox_live::XboxLive,
    /// System folder of the script packages (DLLs of the native classes, class_gc.rs).
    pub(crate) system_dir: Option<std::path::PathBuf>,
    /// FName table (lowercase) for FName(…, FNAME_Find): names of the loaded packages (the
    /// script ones, the map and its imports), then those added during the game. Built at the
    /// first lookup.
    pub(crate) name_table: Option<std::collections::HashMap<String, String>>,
    /// Part of the name table that only depends on the loaded packages and the map (names of the
    /// packages, then of those the map imports, read from disk), prepared at the end of LoadMap
    /// (`prepare_table_names`) with the key of what it read: reused by `canonical_name` if the
    /// key is the same at the first StringToName, otherwise rebuilt as before.
    pub(crate) table_names_packages: Option<(KeyNames, std::collections::HashMap<String, String>)>,
    /// File of the loaded map (its imports complete the name table).
    pub(crate) map_file: Option<std::path::PathBuf>,
    /// World: living actors, elapsed time, current state code per actor.
    pub actors: Vec<ObjId>,
    /// Membership in `actors` (one boolean per object), rebuilt on demand after each
    /// change of the list (actors_changed).
    actor_member: std::cell::RefCell<Option<Vec<bool>>>,
    /// ULevel::iFirstDynamicActor (+0x100): start of the non-static actors in `actors`
    /// (DynamicActors, ULevel::Tick), set by UGameEngine::LoadMap.
    pub first_dynamic: usize,
    /// Number of head slots (among the file's Actors(0) and Actors(1)) occupied by a loaded actor.
    pub(crate) level_head: usize,
    /// The actor array at ULevel+0x1124 (read last by ULevel::Serialize): ULevel::CleanLightActors destroys its hidden actors.
    pub(crate) light_actor_cases: Vec<Option<ObjId>>,
    pub time: f32,
    /// Ordered by actor (order of the level's actor list, like ULevel::Tick): the
    /// result does not depend on a HashMap's random order.
    pub(crate) threads: world::ThreadsState,
    /// Timers by (actor, number): 1 = Timer, 2 = Timer2 (XIII-specific).
    pub(crate) pending_latent: Option<Latent>,
    pub(crate) in_state_code: bool,
    /// The last script function called finished without writing anything into its result (Return
    /// followed by Nothing, or end of code): see execLet.
    return_nothing: bool,
    /// Places of the out parameters copied back by the last call (execLet: if one of them is the
    /// destination of the Let, the copy-back, done after the result is written, wins).
    last_outputs: Vec<(Place, Value)>,
    /// Missing natives encountered (name → number of calls).
    pub missing_natives: HashMap<String, usize>,
    pub native_counts: Option<HashMap<String, u64>>,
    pub rand_counts: Option<HashMap<String, u64>>,
    /// AActor::execReplaceATextureByAnOther: textures replaced everywhere (source, replacement),
    /// in call order, for display.
    pub texture_replacements: Vec<(String, String)>,
    /// UClient::MinDesiredFrameRate (Client + 0x70, [WinDrv.WindowsClient] in XIII.ini), read once.
    pub(crate) min_desired_frame_rate: Option<f32>,
    /// GameInfo created by start_game whose InitGame (URL options) waits for its turn in
    /// begin_play (UGameEngine::LoadMap: after InitExecution and bBegunPlay).
    pub(crate) pending_init_game: Option<(ObjId, String)>,
    pub level: Option<ObjId>,
    /// Stack of the running functions (diagnostics).
    pub stack: Vec<FuncId>,
    /// Object of each function in `stack` (Stack.Object, written at the head of ScriptWarnings).
    pub(crate) this_stack: Vec<ObjId>,
    /// Blocking geometry of the map (BSP, static objects, terrain).
    pub collision: Option<openxiii_assets::collision::CollisionMesh>,
    /// Skeletal mesh instances (animation channels) per actor.
    pub mesh_insts: types::HashMap<ObjId, anim::MeshInstance>,
    /// Collision shapes of the movable objects (Movers), recomputed when they move.
    pub movers: std::collections::BTreeMap<ObjId, world::MoverShape>,
    /// Navigation network: links (ReachSpecs) leaving each navigation point.
    pub nav: std::collections::BTreeMap<ObjId, Vec<navigation::ReachSpec>>,
    /// CurrentPath of the controllers: point and index of the followed ReachSpec.
    pub(crate) current_paths: std::collections::BTreeMap<ObjId, (ObjId, usize)>,
    /// GReachedDestinationXYMargin / HeightMargin (set by XIDPawn's FindBestPathToward).
    pub(crate) reach_xy_margin: f32,
    pub(crate) reach_height_margin: f32,
    /// Static list of the points already returned by AController::SetPath.
    pub(crate) set_path_pending: [Option<ObjId>; 4],
    /// Brushes (UModel) of the map's volumes, per actor: their contacts go through the BSP.
    pub brushes: HashMap<ObjId, openxiii_assets::bsp::BspModel>,
    /// ULevel+0x1188 (filled by ULevel::PostLoad): water planes of the bWaterVolume volumes,
    /// center X, Y, height, half-widths X, Y and the volume.
    pub water_planes: Vec<([f32; 5], ObjId)>,
    /// GEngine->DeltaSeconds (+ 0xD0) of the current frame.
    pub(crate) engine_delta: f32,
    /// GEngine->CurrentFrameCount (+ 0xD4), increased at the start of UGameEngine::Tick.
    pub engine_frames: u32,
    /// USinusModifier: state of each modifier (lowercase path), meshes read and
    /// material properties (sine.rs).
    pub sine: HashMap<String, (openxiii_assets::sine::SineParams, openxiii_assets::sine::SineState)>,
    /// FxHash: one lookup per drawn actor and per frame, never iterated.
    pub(crate) sine_meshes: types::HashMap<String, Option<std::sync::Arc<sine::SineMesh>>>,
    pub(crate) sine_props: openxiii_assets::props::PropCache,
    /// SetSinusVertexStreams of the frame (actor, modifier, CurrentTime, amplitude), in
    /// draw order; emptied at the start of UGameEngine::Tick.
    pub sine_drawn: Vec<(ObjId, String, f32, [f32; 3])>,
    /// Location, Rotation, DrawScale, DrawScale3D and PrePivot of each actor at its first
    /// FDynamicActor::Render: CalculateLighting of a bIgnoreDynLight StaticMeshInstance is
    /// only done at the first render (stream revision at most 1), where the actor was.
    pub first_render: types::HashMap<ObjId, render_placement::RenderPlacement>,
    /// Bounding box of each brush in the world (GetCollisionBoundingBox), at the start of the tick.
    pub(crate) brush_boxes: Vec<(ObjId, [f32; 3], [f32; 3])>,
    /// Inputs of the boxes in `brush_boxes` (volumes.rs refresh_brush_boxes) and buffer.
    pub(crate) brush_box_keys: Vec<(ObjId, [u32; 27])>,
    pub(crate) brush_box_keys_buffer: Vec<(ObjId, [u32; 27])>,
    /// BSP of the map (UModel::PointRegion of the zones) and actor of each of its zones.
    pub level_bsp: Option<openxiii_assets::bsp::BspModel>,
    /// Package of the map (index in `packages`).
    pub(crate) map_package: Option<usize>,
    /// "Start LoadMap" already written for the current travel (engine_travel_tick).
    pub(crate) start_loadmap_logged: bool,
    /// gc_log.rs: packages imported per package (names), and GC2's pending "Unloading" lines.
    pub(crate) gc_imports_cache: HashMap<String, Vec<String>>,
    pub(crate) gc_pending_unload: Option<Vec<String>>,
    /// ULevel.Terrains and their collision data (`terrain_check`).
    pub(crate) terrains: Vec<terrain_check::TerrainCol>,
    pub phys_counts: Option<std::collections::BTreeMap<String, u64>>,
    /// Collision shapes of the actors whose primitive is a StaticMesh, and collision
    /// triangles read for each mesh.
    /// StaticMesh shapes (FxHash: never iterated, one lookup per trace and per actor every frame).
    pub(crate) sm_shapes: crate::types::HashMap<ObjId, static_collision::SmShape>,
    /// Generation of each actor's properties at the last computation of its shape (static_collision.rs).
    pub(crate) sm_generations: crate::types::HashMap<ObjId, u64>,
    pub(crate) sm_local: HashMap<String, Option<std::sync::Arc<static_collision::LocalSets>>>,
    /// Classes found by `is_a`, by the given name.
    pub(crate) class_cache: std::cell::RefCell<crate::types::HashMap<String, ClassId>>,
    /// Answers of is_a by (object class, requested name), emptied when classes are linked.
    pub(crate) is_a_cache: std::cell::RefCell<Vec<IsAEntry>>,
    /// IsA flags of the tick per class (flags_classes.rs), emptied with the slots.
    pub(crate) flags_classes: std::cell::RefCell<Vec<u16>>,
    /// Functions found by find_function, emptied when classes are linked.
    pub(crate) func_cache: std::cell::RefCell<Vec<FuncEntry>>,
    /// place_in: slot of instance variables by (package, reference, class) and of
    /// locals by (package, reference, function).
    var_slot_cache: std::collections::HashMap<(usize, i32, ClassId), usize, std::hash::BuildHasherDefault<crate::types::FxHasher>>,
    /// Direct table in front of `var_slot_cache` (one slot per hash of (package, reference,
    /// class), a single comparison): same answers, never emptied like it.
    var_slot_direct: Vec<VarSlotEntry>,
    pub(crate) var_slot_direct_active: bool,
    local_slot_cache: std::collections::HashMap<(usize, i32, usize), usize, std::hash::BuildHasherDefault<crate::types::FxHasher>>,
    /// place_in: slot of a struct field by (package, member reference, struct name:
    /// address and length of the kept text, see value::NameStruct).
    member_cache: std::collections::HashMap<(usize, i32, usize, usize), usize, std::hash::BuildHasherDefault<crate::types::FxHasher>>,
    /// class_of_ref by (package, reference), with the number of classes at the time of computation.
    /// FinalFunction: function by (package, reference), with the generation of the computation.
    final_cache: std::collections::HashMap<(usize, i32), (Option<FuncId>, usize), std::hash::BuildHasherDefault<crate::types::FxHasher>>,
    class_ref_cache: std::cell::RefCell<std::collections::HashMap<(usize, i32), (Option<ClassId>, usize), std::hash::BuildHasherDefault<crate::types::FxHasher>>>,
    /// call_inner: zero values of the locals, parameters and name of each function, prepared at the
    /// first call (by FuncId).
    call_preps: Vec<Option<Rc<CallPrep>>>,
    /// Core and XIDCine packages whose bytecode suits find_label_direct.
    find_label_packages: [Option<usize>; 2],
    /// Lists of script calls returned to be reused (locals, native values,
    /// supplied arguments, arguments).
    pool_values: Vec<Vec<Value>>,
    pool_bools: Vec<Vec<bool>>,
    pool_args: Vec<Vec<Option<Value>>>,
    /// Lists of out parameters (places of call_with_exprs, outputs of call_inner).
    pool_places: Vec<Vec<Option<Place>>>,
    /// place_in called by place_probed: the Unsupported error will be ignored.
    place_probe: bool,
    pool_outs: Vec<Vec<Option<Value>>>,
    /// Label (index in `collision_materials`) of each material of the BSP surfaces.
    pub(crate) bsp_material_tags: std::cell::OnceCell<HashMap<i32, u32>>,
    pub zone_actors: Vec<Option<ObjId>>,
    /// BSP nodes drawn by the last root scene and its sky scenes (`bsp_draw`).
    pub bsp_views: Vec<bsp_draw::BspView>,
    /// `bsp_views` of a player's last view (UGameEngine::Draw), kept before the
    /// HUD's DrawPortals and render textures: the scenery of its frame is culled according to them.
    pub scene_bsp_views: Vec<bsp_draw::BspView>,
    /// The level's display list (ULevel+0x1130), see visibility.rs.
    pub display: visibility::DisplayList,
    /// Dynamic lights of the level (ULevel+0x1118), see dyn_light.rs.
    pub dyn_lights: Vec<Option<ObjId>>,
    /// Linked list of the level's FLight* (ULevel+0x113C): slots of `dyn_lights`, head first.
    pub dyn_light_list: Vec<usize>,
    /// FLight cache per slot: removal keeps its metadata, see the light wrapper test.
    dyn_light_cache: Vec<Option<openxiii_assets::lighting::Light>>,
    /// Last engine frame whose scene lights were updated (static), see scene_lights.rs.
    pub(crate) lights_frame: u32,
    /// LightType of the first `static_lights` lights of the level's model, and UModel+0x134 (lights read again from the actors).
    pub(crate) static_light_types: (Vec<u8>, bool),
    pub(crate) scene_light_model: scene_light_pass::SceneLightModel,
    /// Cache of each real Model PostLoad, key (package, export).
    pub(crate) model_light_caches: HashMap<(usize, usize), Vec<openxiii_assets::lighting::Light>>,
    /// Mixed Model/Dynamic Level113C; producer RefreshAll/Add/Remove of C.
    pub(crate) scene_light_chain: Vec<scene_light_pass::SceneLightId>,
    /// Original brush of each surface of the level's BSP (FBspSurf::Actor).
    pub surf_actors: Vec<Option<ObjId>>,
    /// Polygons (UModel::Polys) of the brushes of the portals and anti-portals.
    pub brush_polys: HashMap<ObjId, Vec<openxiii_assets::bsp::Poly>>,
    /// Actors drawn by the last frame (FDynamicActor::Render), in order.
    pub rendered: Vec<ObjId>,
    /// Actor of the list whose FDynamicActor::Render drew each actor
    /// (itself, or the base of an actor attached to bones): the attached actor follows its base's list.
    pub render_root: types::HashMap<ObjId, ObjId>,
    /// Location, Rotation, DrawScale, DrawScale3D and PrePivot of each actor at the time of its
    /// FDynamicActor::Render: the deferred weapon is drawn at the scene's place, not at the one
    /// that RenderOverlays gives it afterwards (HUD.PostRender, after the scene).
    pub render_place: types::HashMap<ObjId, render_placement::RenderPlacement>,
    /// Last light selection of each drawn actor (Vm::scene_light_selection).
    pub(crate) light_selections: types::HashMap<ObjId, std::rc::Rc<visibility::SceneLightSelection>>,
    /// List of translucent items of the last root scene, from farthest to
    /// closest: BSP nodes with a sorted material and translucent actors (draw order).
    pub translucent_order: Vec<crate::visibility::Translucent>,
    /// UMaterial::RequiresSorting of the material of each BSP surface (bsp_draw.rs).
    pub(crate) surf_sorting: Option<Vec<bool>>,
    /// Actors drawn by UCanvas::DrawActor during the last HUD.
    pub canvas_rendered: Vec<ObjId>,
    /// UCanvas::execDrawActor calls of the frame: (actor, WireFrame, ClearZ, DisplayFOV in degrees).
    pub canvas_actor_calls: Vec<(ObjId, bool, bool, f32)>,
    /// Lockstep co-op: a scene made for drawing only (`preview_scene`), which must leave the
    /// game's state as it was (no LastRenderTime, pose, sine or emitter advance, no script).
    pub render_preview: bool,
    /// Lockstep co-op: the view being made is never drawn (another player's view, or this
    /// machine's on a step whose output no frame presents): only what the game reads back is made
    /// (visibility, LastRenderTime and LastRenderDist, poses, sines, emitters, attachments,
    /// scripts), not the scene's lights and their passes.
    pub view_state_only: bool,
    /// Lockstep co-op: the actors' places of the last two steps, for drawing between them.
    pub interp_places: crate::lockstep_interp::Places,
    /// Lockstep co-op, drawing only: the HUD's icons over a world point made by the view being
    /// drawn (`lockstep_anchor.rs`), moved at present onto the camera the frame draws.
    pub hud_anchors: Vec<crate::lockstep_anchor::HudAnchor>,
    /// The script depth of the function whose last WorldToScreen opened an anchor (its
    /// primitives belong to that point until it returns or projects another).
    pub(crate) hud_anchor_depth: Option<usize>,
    /// Lockstep co-op: the frame being drawn shows a drawing-only scene (`preview_scene`): the
    /// actors the step did not draw are posed without advancing their animation (`peek_frame`).
    pub preview_drawing: bool,
    /// `render_preview`: the actors the step's own scene drew (their bone attachments are where
    /// that render placed them).
    pub preview_step_drawn: std::collections::HashSet<ObjId, std::hash::BuildHasherDefault<types::FxHasher>>,
    /// DrawPortal calls (draw_portal test): rectangle, actor, location, rotation, FOV, ClearZ, the player's FovAngle during the scene.
    pub canvas_portal_calls: Vec<PortalCall>,
    /// Scenes rendered by RenderTargetMaterial.Update (render_target_update test).
    pub render_target_scenes: u32,
    /// Actors drawn into a render texture (URenderTargetMaterial::Update) since the start
    /// of the frame; Update is called during the Tick, before the HUD.
    pub target_rendered: Vec<ObjId>,
    /// Probe events disabled by Disable (bit = `world::PROBES`), reset to zero by GotoState.
    pub(crate) probe_disabled: types::HashMap<ObjId, u64>,
    pub(crate) tick_ordinary: bool,
    /// URL of the current map (GetLocalURL, GetURLMap, relative travels).
    pub url: travel::Url,
    /// Pending ClientTravel of the local player (URL, ETravelType, bItems).
    pub(crate) client_travel: Option<(String, u8, bool)>,
    /// GConfig: the game's and the player's ini files, language files (.int, .frt).
    pub config: Option<config::ConfigCache>,
    /// Names of the enumerations already read, by path (lowercase).
    pub(crate) enum_cache: HashMap<String, Option<std::rc::Rc<Vec<String>>>>,
    /// RF_StateChanged: set by a successful GotoState, cleared before EndState / BeginState.
    pub(crate) state_changed: std::collections::HashSet<ObjId>,
    /// RF_InEndState: EndState in progress (no nested EndState).
    pub(crate) in_end_state: std::collections::HashSet<ObjId>,
    pub(crate) trace_steps: Option<String>,
    pub(crate) trace_states: bool,
    /// Long hints opened on an already broken object, closed at the end of the tick (indices_broken.rs).
    pub(crate) indices_a_close: Vec<ObjId>,
    /// DefaultPhysicsVolume of the map.
    pub default_volume: Option<ObjId>,
    /// ULevel::TimeSeconds (double), starting from the TimeSeconds saved in the LevelInfo.
    pub level_time: f64,
    /// During ULevel::Tick: the created actors go into `newly_spawned` (NewlySpawned list).
    pub(crate) in_tick: bool,
    /// NewlySpawned list (XLevel + 0xF4): the actor enters it at the very end of SpawnActor, after SetInitialState and FindBase;
    /// ULevel::Tick walks it last entry first.
    pub(crate) newly_spawned: Vec<ObjId>,
    /// Actors created during the frame in the order of Actors.Add (start of SpawnActor): ULevel::Tick's main loop
    /// handles them at their place in Actors.
    pub(crate) spawn_order: Vec<ObjId>,
    /// ULevel + 0xFC (Ticked): parity of the frame, toggled at the end of each ULevel::Tick; an
    /// actor has been handled in the frame when its bTicked equals it. Zero for a new level.
    pub(crate) level_ticked: bool,
    /// ULevel + 0xF0 (FirstDeleted): actors destroyed since the last CleanupDestroyed.
    pub(crate) first_deleted: Vec<ObjId>,
    /// Actors deleted by CleanupDestroyed (`delete`): the engine only sets to NULL
    /// the references held by the level's actors; a reference from another object (Interaction)
    /// still points to the freed memory, which keeps its values. XIIIPlayerInteraction.
    /// ResetInteractions thus reads TargetActor.bDeleteMe (true) and forgets it. Read-only.
    pub(crate) freed: HashMap<ObjId, Object>,
    /// Bone of the last Trace / TraceActors with TRACETYPE_RequestBones (GetLastTraceBone).
    pub(crate) last_trace_bone: String,
    /// Indices removed from `actors` by ULevel::DestroyActor, in order (cursors of the iterators).
    pub(crate) actor_removals: Vec<usize>,
    /// Collision boxes of the bones per skeletal mesh (see `bone_hit`).
    /// UPrimitive::BoundingBox per mesh or StaticMesh (see `radius_iter`).
    pub(crate) primitive_boxes: HashMap<String, Option<([f32; 3], [f32; 3])>>,
    /// Local spheres of the UPrimitive exports for the library's current revision.
    primitive_spheres: HashMap<String, [f32; 4]>,
    sphere_library_revision: Option<u64>,
    /// USkeletalMeshInstance+0x248: render box computed by RecomputeBoundingVolume.
    pub(crate) skel_render_boxes: HashMap<ObjId, ([f32; 3], [f32; 3])>,
    /// Skeletal meshes loaded for RecomputeBoundingVolume.
    pub(crate) skel_models: HashMap<String, Option<std::rc::Rc<openxiii_assets::character::CharacterModel>>>,
    /// Attachments (TagAliases, TagNames, TagCoords) per skeletal mesh (see `bone_attach`).
    #[allow(clippy::type_complexity)]
    /// Trails (ATrail): vertices and sections, read by the renderer.
    pub trails: HashMap<ObjId, trail::TrailState>,
    /// Collision hash (FCollisionOctree).
    pub(crate) octree: octree::Octree,
    pub(crate) attach_tags_cache: HashMap<String, Option<std::rc::Rc<(Vec<String>, Vec<String>, Vec<openxiii_assets::coords::Coords>)>>>,
    pub(crate) bone_boxes: HashMap<String, Option<std::rc::Rc<Vec<openxiii_assets::skeletal_collision::BoneBox>>>>,
    /// HUD draw commands of the last frame (see `canvas`).
    pub hud: Vec<canvas::HudDraw>,
    /// Sounds requested since the last read (the interface empties the list).
    pub sounds: Vec<canvas::AudioCall>,
    /// HX audio subsystem (UHXAUDIOSubsystem); without it the calls stay in `sounds`.
    pub audio: Option<openxiii_hx::subsystem::HxAudio>,
    /// Sound objects already resolved (path → HX resource).
    pub audio_sounds: std::collections::HashMap<String, Option<openxiii_hx::subsystem::SoundRef>>,
    /// Materials of the map's surfaces, indexed by the label of the collision triangles.
    pub collision_materials: Vec<String>,
    /// C++ state of the open AStatLogFiles (stat_log.rs).
    pub(crate) stat_log_files: HashMap<ObjId, crate::stat_log::StatLogFileState>,
    /// VariableMap of the UWebRequests and ReplacementMap of the UWebResponses (uweb.rs).
    pub(crate) web_maps: HashMap<ObjId, crate::uweb::WebMap>,
    pub(crate) web_subst: HashMap<ObjId, crate::uweb::WebMap>,
    /// Loaded MeshAnimation objects (lowercase path).
    pub(crate) anim_sets: HashMap<String, Option<std::rc::Rc<anim::AnimSet>>>,
    /// Loaded skeletal meshes (lowercase path): default animation, bones.
    pub(crate) mesh_infos: HashMap<String, Option<std::rc::Rc<anim::MeshInfo>>>,
    pub(crate) canvas: Option<ObjId>,
    /// Native state of UCanvas (material sizes, static UFinalBlend, frame duration).
    pub canvas_state: canvas::CanvasState,
    pub(crate) fonts: HashMap<String, Option<std::rc::Rc<openxiii_assets::font::Font>>>,
    /// Position (bytes) of the current statement, to locate warnings.
    pub(crate) cur_offset: u32,
    /// GRunawayCount: jumps (execJump, execJumpIfNot) since GInitRunaway,
    /// called at the start of ULevel::Tick and by UGameEngine::LoadMap.
    pub(crate) runaway: u32,
    /// End (in the bytecode) of the current function calls: Stack.Code as seen by a native
    /// that writes through Stack.Logf.
    pub(crate) call_ends: Vec<u32>,
    /// Place evaluated as a destination (Stack.Step with a null Result: assignment, out
    /// parameter); a read (eval) resets it to false for its subexpressions.
    writing: bool,
    /// Access to the data packages (textures, sounds…) to read their properties from the script.
    pub library: Option<openxiii_package::library::Library>,
    asset_cache: HashMap<(String, String), Value>,
    /// Exports of the script packages whose class is a script class (lowercase path
    /// "package.outer.name" → package, export).
    pub(crate) script_exports: HashMap<String, (usize, usize)>,
    /// UGameEngine subsystems that survive the map change (saves, music).
    pub engine: save::EngineState,
    /// EXIT / QUIT requested by a command (appRequestExit).
    pub quit_requested: bool,
    /// GIsACommandFromPlayerController.
    pub command_from_pc: bool,
    /// SETRES requested (width, height), to be applied by the display.
    pub pending_resize: Option<(u32, u32)>,
    /// UD3DRenderDevice::Flush pending: PrecacheOnFlip (RenDev+0x64). SetRes
    /// (device creation with the window, SETRES) calls Flush; the next frame
    /// is drawn with LevelAction = LEVACT_Precaching (UGameEngine::Draw).
    pub precache_on_flip: bool,
    /// Viewport+0x158: precache frame in progress (second FCameraSceneNode of
    /// UGameEngine::Draw); the scene is then seen from zone 0, without a frustum.
    pub precache_render: bool,
    /// Viewport+0x148 of the last UViewport::Lock (difference with the previous lock), read again by
    /// the FCameraSceneNode::Render of the DrawPortals in the same frame.
    pub(crate) viewport_delta: f32,
    pub draws_before_done: std::collections::HashSet<ObjId>,
    /// Player window in fullscreen (BLIT_Fullscreen): StartupFullscreen of
    /// [WinDrv.WindowsClient] at startup.
    pub fullscreen: bool,
    /// TOGGLEFULLSCREEN / ENDFULLSCREEN requested (UWindowsViewport::ToggleFullscreen).
    pub pending_toggle_fullscreen: bool,
    /// Display mode chosen in the Video menu (improved build), to be applied by the display
    /// (with pending_resize when the page also sent SETRES).
    pub pending_display_mode: Option<video_settings::FullscreenMode>,
    /// FLUSH (UEngine::Exec): resource caches dropped by the owning Renderer.
    pub pending_renderer_flush: bool,
    /// UEngine (+0x8C and +0x90 of the object, +0x64 and +0x68 of the FExec subobject): ANTIPORTAL n (true for n = 0) and SHOWBV
    /// (toggle), read by the original's renderer.
    pub engine_antiportal: bool,
    pub engine_show_bv: bool,
    /// UClient: Brightness, Contrast, Gamma.
    pub client_color: [f32; 3],
    /// The srand of HXAudio's first CUUID reads the clock (the game, [`Vm::seed_from_clock`]);
    /// off, the sequence continues (tests and tools).
    pub(crate) hx_srand_clock: bool,
    /// Seed given to that srand: the host's in a lockstep session (`start/seed`).
    pub(crate) hx_srand_given: Option<u32>,
    /// LoadSounds already done for the current map (by LoadMap), PlayMusicInit not yet.
    pub(crate) audio_sounds_loaded: bool,
    /// Display modes of the adapter (width, height, bits per pixel), in the order in which
    /// Direct3D enumerates them; supplied by the display.
    pub display_modes: Vec<(u32, u32, u32)>,
    /// The screen's native resolution (its preferred mode), supplied by the display: the improved
    /// Video menu offers nothing larger (video_settings::menu_resolutions). None: not known.
    pub screen_native: Option<(u32, u32)>,
    /// What the display device accepts for the improved Video menu's quality lines (MSAA counts,
    /// largest render scale and anisotropy); supplied by the renderer.
    pub video_caps: video_settings::VideoCaps,
    /// appSeconds(): the engine clock (real seconds, without TimeDilation or pause).
    pub app_seconds: f64,
    /// UViewport+0x140 (CurrentTime) of each viewport at its last Lock (see `viewport_lock`).
    pub(crate) viewport_times: HashMap<ObjId, f64>,
    /// Size of the player's viewport (UViewport::SizeX/SizeY): that of the window, otherwise XIII.ini.
    /// Size of the client window at the last UGameEngine::Draw: that of the viewports that
    /// UGameEngine::Tick takes again for the microphones (UpdateMicro).
    pub client_size: (i32, i32),
    /// Size of the whole window drawn this frame (display_format.rs): the solo frame's, or the
    /// client's in split screen. (0, 0) before the first frame.
    pub display_window: (i32, i32),
    pub(crate) menu_layout: gui::MenuLayout,
    /// Touch screens (Android): the finger drives the menus, no menu cursor is drawn (gui.rs, canvas.rs).
    pub touch_hides_cursor: bool,
    pub viewport: (i32, i32),
    /// Desktop size (GetDeviceCaps HORZRES, VERTRES) and WinDrv's table of standard resolutions.
    pub desktop: (i32, i32),
    pub res_table: crate::resize::ResTable,
    /// UViewport+0x120 (UViewport::SetOnlyPostRender): FCameraSceneNode::Render
    /// skips the scene and only keeps PostRender.
    pub only_post_render: bool,
    /// Last object found by find_viewport_object (viewport number, object), checked again on each call.
    pub(crate) viewport_found: std::cell::Cell<Option<(u8, ObjId)>>,
    /// Projectors (AProjector): render info and the BSP nodes that carry them.
    pub projectors: projector::Projectors,
    /// Viewport->Actor during HUD.PostRender (render_hud): camera of the DrawPortals.
    pub(crate) render_player: Option<ObjId>,
    pub interaction_master: Option<ObjId>,
    /// Input of the local player (UInput), created by `input_init`.
    pub input_sys: Option<Box<player_input::InputSystem>>,
    /// Look stick of a gamepad or of the touch controls (x right, y down, −1…1), read each
    /// frame through the console versions' joystick bindings (player_input.rs, `look_stick_axes`);
    /// None: keyboard and mouse (the original PC version), nothing read.
    pub look_stick: Option<(f32, f32)>,
    /// The first local player's move stick (the gamepad's left stick, x right and y down, −1…1),
    /// given by the game window this frame, walked at the keys' range as far as it is pushed
    /// (player_input.rs, nalog_move); None: keys (keyboard, touch controls).
    pub move_stick: Option<(f32, f32)>,
    /// The other local players' move sticks (coop.rs, coop_pad_input), by controller.
    pub move_sticks: Vec<(ObjId, (f32, f32))>,
    /// Gamepad settings applied to `look_stick` (gamepad_config.rs): look speed factor (SpeedBase
    /// of the console bindings) and vertical inversion. (1.0, false), the default, changes nothing.
    pub look_stick_tune: (f32, bool),
    /// Network co-op in lockstep (lockstep.rs): the simulation's side of the session.
    pub lockstep: Option<Box<lockstep::SimSide>>,
    pub session: lockstep::SessionContext,
    /// Lockstep co-op, a new VM after a session: the menu pages kept before it, for the first
    /// root window made (lockstep_menus.rs, give_saved_menu_stack_to).
    pub lockstep_menus: Option<Vec<String>>,
    pub(crate) remembered_menus: lockstep_menus::RememberedMenus,
    /// Lockstep co-op: the animated textures' state the emitters' texture animation reads and
    /// advances (game state), apart from the one this machine's drawing advances.
    pub(crate) lockstep_materials: Option<Box<openxiii_assets::material::Materials>>,
    /// Gamepad settings of the local players (gamepad_config.rs), set by the game window each
    /// frame: local multiplayer reads them for the gamepads of the players after the first.
    pub gamepad_players: [gamepad_config::PlayerPad; gamepad_config::PLAYERS],
    /// Aim assist of the local players after the first (gamepad_config.rs).
    pub(crate) other_aim: gamepad_config::OtherAim,
    /// UInput of the other local players' viewports (split screen, see client.rs).
    pub other_inputs: Vec<Box<player_input::InputSystem>>,
    /// Viewport that UGameEngine::Draw draws (local interactions, Canvas), see client.rs.
    pub current_viewport: Option<ObjId>,
    /// DirectInput gamepads (WinDrv): state given by the display, see pad.rs.
    pub pads: pad::Pads,
    /// Input device whose aim assist applies (aim_device.rs; not in the original PC version).
    pub(crate) aim_device: aim_device::AimDevice,
    /// UForceFeedbackManager::Instance: absent from the original PC game (None), created by `enable_vibrations`.
    pub force_feedback: Option<force_feedback::Manager>,
    /// Viewports whose UpdateInput has already read the gamepad once (Viewport+0x1C0).
    pub(crate) pad_seen: std::collections::HashSet<u8>,
    /// Keyboard and mouse of the player's window (WinDrv, see windrv_input.rs).
    pub win_input: windrv_input::WinInput,
    /// Camera views of the render textures (RenderTargetMaterial object -> camera).
    pub render_views: HashMap<ObjId, Vec<canvas::RenderView>>,
    /// Number of calls to RenderTargetMaterial.Update (RenderView::serial).
    pub render_serial: u64,
    /// Clipboard, viewport and screen effects written by the `solo_native` natives.
    pub solo: solo_native::SoloState,
    /// Screen effects of FD3DRenderInterface (poison, blur, wound, underwater).
    pub screen_fx: screen_fx::ScreenFx,
    /// Weather of FD3DRenderInterface (RndCubeSpr).
    pub weather: weather::RndCubeSpr,
    /// Android Edit panel cheat (not in the original): the player's pawn and the multiplier of
    /// its walking speed, applied in physWalking (see pawn_physics.rs). None: normal game.
    pub cheat_speed: Option<(ObjId, f32)>,
    #[cfg(feature = "mods")]
    pub(crate) mods: mod_state::ModState,
    /// Mods build only: lines reported about those values (applied, ignored).
    #[cfg(feature = "mods")]
    pub mod_messages: Vec<String>,
    pub(crate) improved: improved::State,
    /// What the players chose in the menus (local_play.rs).
    pub(crate) local_play: local_play::LocalPlayCopy,
    /// Mods build only: state changes redirected into a character's allowed states.
    #[cfg(feature = "mods")]
    pub mod_redirects: usize,
    /// Mods build only: sound banks loaded for the mods on this level.
    #[cfg(feature = "mods")]
    pub mod_banks: Vec<mod_sounds::BankLoad>,
    /// Mods build only: classes of the Select level test weapons given in this game, whose
    /// banks come back on each level (or reload) where a player still carries one (mod_sounds.rs).
    #[cfg(feature = "mods")]
    pub select_level_classes: Vec<String>,
    /// Local co-op campaign (coop.rs): off unless asked for.
    pub coop: coop::Coop,
    /// Mods build only: how charred scripts made an actor (0: as the game draws it, 1: black);
    /// the viewer darkens its cartoon colour by it.
    #[cfg(feature = "mods")]
    pub mod_charred: HashMap<ObjId, f32>,
    /// Mods build only: who looks through the view whose particles are being drawn (its view
    /// actor), so that the mods' own effects keep the bOwnerNoSee / bOnlyOwnerSee filters.
    #[cfg(feature = "mods")]
    pub mod_particle_viewer: Option<ObjId>,
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {

    pub fn new() -> Self {
        Vm {
            native_slots: Vec::new(),
            tick_slots: Vec::new(),
            slot_cache: std::cell::RefCell::new(vec![SlotEntry::EMPTY; SLOT_CACHE]),
            slot_literals: std::cell::RefCell::new(vec![SlotLiteral::EMPTY; SLOT_LITERALS]),
            states_resolved: Default::default(),
            cache_states: !cfg!(test),
            packages: Vec::new(),
            classes: Vec::new(),
            class_index: HashMap::new(),
            class_qualified: HashMap::new(),
            functions: Vec::new(),
            call_hooks: call_hooks::CallHooks::default(),
            struct_exports: HashMap::new(),
            structs: HashMap::new(),
            objects: Vec::new(),
            log: Vec::new(),
            warnings: Vec::new(),
            native_index: Default::default(),
            depth: 0,
            in_singular: std::collections::HashSet::new(),
            seed: 0x1234_5678,
            particles: Default::default(),
            load_order: None,
            subobjects: HashMap::new(),
            object_paths: std::collections::HashSet::new(),
            class_unique: HashMap::new(),
            names_loaded: Default::default(),
            order_original: HashMap::new(),
            order_sounds: Vec::new(),
            index_sounds: Default::default(),
            order_map: None,
            objects_marked: std::collections::HashSet::new(),
            class_within: HashMap::new(),
            meta_const: None,
            native_class_parents: None,
            same_map_roots: None,
            objects_new: Default::default(),
            system_dir: None,
            ipdrv: Default::default(),
            xbox_live: { let mut x = xbox_live::XboxLive::default(); x.init(); x },
            state_code_obj: None,
            name_table: None,
            table_names_packages: None,
            map_file: None,
            actor_member: Default::default(),
            actors: Vec::new(),
            first_dynamic: 0,
            level_head: 2,
            light_actor_cases: Vec::new(),
            time: 0.0,
            threads: Default::default(),
            pending_latent: None,
            in_state_code: false,
            return_nothing: false,
            last_outputs: Vec::new(),
            missing_natives: HashMap::new(),
            native_counts: None,
            rand_counts: None,
            texture_replacements: Vec::new(),
            min_desired_frame_rate: None,
            pending_init_game: None,
            level: None,
            stack: Vec::new(),
            this_stack: Vec::new(),
            collision: None,
            mesh_insts: Default::default(),
            movers: Default::default(),
            nav: Default::default(),
            current_paths: Default::default(),
            reach_xy_margin: 0.0,
            reach_height_margin: 0.0,
            set_path_pending: [None; 4],
            brushes: HashMap::new(),
            water_planes: Vec::new(),
            engine_delta: 0.0,
            engine_frames: 0,
            sine: HashMap::new(),
            sine_meshes: types::HashMap::default(),
            sine_props: openxiii_assets::props::PropCache::new(),
            sine_drawn: Vec::new(),
            first_render: types::HashMap::default(),
            brush_boxes: Vec::new(),
            brush_box_keys: Vec::new(),
            brush_box_keys_buffer: Vec::new(),
            level_bsp: None,
            map_package: None,
            start_loadmap_logged: false,
            gc_imports_cache: HashMap::new(),
            gc_pending_unload: None,
            terrains: Vec::new(),
            phys_counts: None,
            sm_shapes: Default::default(),
            sm_generations: Default::default(),
            sm_local: HashMap::new(),
            class_cache: Default::default(),
            is_a_cache: std::cell::RefCell::new(vec![IsAEntry::EMPTY; IS_A_CACHE]),
            flags_classes: Default::default(),
            func_cache: std::cell::RefCell::new(vec![FuncEntry::EMPTY; FUNC_CACHE]),
            var_slot_cache: Default::default(),
            var_slot_direct: vec![VarSlotEntry::EMPTY; VAR_SLOT_DIRECT],
            var_slot_direct_active: true,
            local_slot_cache: Default::default(),
            member_cache: Default::default(),
            class_ref_cache: Default::default(),
            final_cache: Default::default(),
            call_preps: Vec::new(),
            find_label_packages: [None; 2],
            pool_values: Vec::new(),
            pool_bools: Vec::new(),
            pool_args: Vec::new(),
            pool_places: Vec::new(),
            place_probe: false,
            pool_outs: Vec::new(),
            bsp_material_tags: std::cell::OnceCell::new(),
            zone_actors: Vec::new(),
            bsp_views: Vec::new(),
            scene_bsp_views: Vec::new(),
            display: Default::default(),
            dyn_lights: Vec::new(),
            dyn_light_list: Vec::new(),
            dyn_light_cache: Vec::new(),
            lights_frame: 0,
            static_light_types: (Vec::new(), false),
            scene_light_model: Default::default(),
            model_light_caches: HashMap::new(),
            scene_light_chain: Vec::new(),
            surf_actors: Vec::new(),
            brush_polys: HashMap::new(),
            rendered: Vec::new(),
            render_root: Default::default(),
            render_place: types::HashMap::default(),
            light_selections: types::HashMap::default(),
            translucent_order: Vec::new(),
            surf_sorting: None,
            canvas_rendered: Vec::new(),
            canvas_actor_calls: Vec::new(),
            render_preview: false,
            view_state_only: false,
            interp_places: Default::default(),
            hud_anchors: Vec::new(),
            hud_anchor_depth: None,
            preview_drawing: false,
            preview_step_drawn: Default::default(),
            canvas_portal_calls: Vec::new(),
            render_target_scenes: 0,
            target_rendered: Vec::new(),
            probe_disabled: Default::default(),
            tick_ordinary: false,
            url: Default::default(),
            client_travel: None,
            config: None,
            enum_cache: HashMap::new(),
            state_changed: Default::default(),
            in_end_state: Default::default(),
            trace_steps: None,
            trace_states: false,
            indices_a_close: Vec::new(),
            default_volume: None,
            level_time: 0.0,
            in_tick: false,
            newly_spawned: Vec::new(),
            spawn_order: Vec::new(),
            level_ticked: false,
            first_deleted: Vec::new(),
            freed: HashMap::new(),
            last_trace_bone: String::new(),
            actor_removals: Vec::new(),
            bone_boxes: HashMap::new(),
            trails: HashMap::new(),
            octree: Default::default(),
            attach_tags_cache: HashMap::new(),
            primitive_boxes: HashMap::new(),
            primitive_spheres: HashMap::new(),
            sphere_library_revision: None,
            skel_render_boxes: HashMap::new(),
            skel_models: HashMap::new(),
            hud: Vec::new(),
            sounds: Vec::new(),
            audio: None,
            audio_sounds: std::collections::HashMap::new(),
            collision_materials: Vec::new(),
            stat_log_files: HashMap::new(),
            web_maps: HashMap::new(),
            web_subst: HashMap::new(),
            anim_sets: HashMap::new(),
            mesh_infos: HashMap::new(),
            canvas: None,
            canvas_state: canvas::CanvasState::default(),
            fonts: HashMap::new(),
            cur_offset: 0,
            runaway: 0,
            call_ends: Vec::new(),
            writing: false,
            library: None,
            asset_cache: HashMap::new(),
            script_exports: HashMap::new(),
            engine: Default::default(),
            quit_requested: false,
            command_from_pc: false,
            pending_resize: None,
            precache_on_flip: true,
            precache_render: false,
            viewport_delta: 0.0,
            draws_before_done: Default::default(),
            fullscreen: false,
            pending_toggle_fullscreen: false,
            pending_display_mode: None,
            pending_renderer_flush: false,
            engine_antiportal: false,
            engine_show_bv: false,
            client_color: [0.5, 0.5, 1.7],
            hx_srand_clock: false,
            hx_srand_given: None,
            audio_sounds_loaded: false,
            display_modes: Vec::new(),
            screen_native: None,
            video_caps: Default::default(),
            app_seconds: 0.0,
            client_size: (1280, 720),
            display_window: (0, 0),
            menu_layout: Default::default(),
            touch_hides_cursor: false,
            viewport_times: HashMap::new(),
            viewport: (800, 600),
            desktop: (1920, 1080),
            res_table: Default::default(),
            only_post_render: false,
            viewport_found: std::cell::Cell::new(None),
            projectors: projector::Projectors::new(),
            render_player: None,
            interaction_master: None,
            input_sys: None,
            look_stick: None,
            move_stick: None,
            move_sticks: Vec::new(),
            look_stick_tune: (1.0, false),
            lockstep: None,
            session: Default::default(),
            lockstep_menus: None,
            remembered_menus: Default::default(),
            lockstep_materials: None,
            gamepad_players: Default::default(),
            other_aim: Default::default(),
            other_inputs: Vec::new(),
            current_viewport: None,
            pads: Default::default(),
            aim_device: Default::default(),
            force_feedback: None,
            pad_seen: Default::default(),
            win_input: Default::default(),
            render_views: HashMap::new(),
            render_serial: 0,
            solo: Default::default(),
            screen_fx: Default::default(),
            weather: Default::default(),
            cheat_speed: None,
            #[cfg(feature = "mods")]
            mods: mod_state::ModState::new(),
            #[cfg(feature = "mods")]
            mod_messages: Vec::new(),
            improved: improved::State::default(),
            local_play: Default::default(),
            #[cfg(feature = "mods")]
            mod_redirects: 0,
            #[cfg(feature = "mods")]
            mod_banks: Vec::new(),
            #[cfg(feature = "mods")]
            select_level_classes: Vec::new(),
            coop: coop::Coop::default(),
            #[cfg(feature = "mods")]
            mod_charred: HashMap::new(),
            #[cfg(feature = "mods")]
            mod_particle_viewer: None,
        }
    }

    pub fn class(&self, name: &str) -> Option<ClassId> {
        // Full path: the class of this package (StaticFindObject with the Outer).
        if let (Some((package, _)), Some(short)) = (name.split_once('.'), name.rsplit('.').next()) {
            if let Some(&c) = self.class_qualified.get(&format!("{package}.{short}").to_lowercase()) {
                return Some(c);
            }
        }
        let name = name.rsplit('.').next().unwrap_or(name);
        self.class_index.get(&name.to_lowercase()).copied()
    }

    /// Class designated by an object reference of package `pi` (import "Package.Class" or
    /// export of the package), resolved with its package.
    pub(crate) fn class_of_ref(&self, pi: usize, r: i32) -> Option<ClassId> {
        // Kept as long as no package or class has been added.
        let generation = self.packages.len() ^ self.classes.len() << 16 ^ self.class_index.len() << 32 ^ self.class_qualified.len() << 48;
        if let Ok(cache) = self.class_ref_cache.try_borrow() {
            if let Some(&(c, g)) = cache.get(&(pi, r)) {
                if g == generation {
                    return c;
                }
            }
        }
        let c = self.class_of_ref_compute(pi, r);
        if let Ok(mut cache) = self.class_ref_cache.try_borrow_mut() {
            cache.insert((pi, r), (c, generation));
        }
        c
    }

    fn class_of_ref_compute(&self, pi: usize, r: i32) -> Option<ClassId> {
        let p = &self.packages[pi].package;
        let path = match ObjectRef::from(r) {
            ObjectRef::None => return None,
            ObjectRef::Export(_) => format!("{}.{}", self.packages[pi].name, p.object_path(r.into())),
            ObjectRef::Import(_) => p.object_path(r.into()),
        };
        self.class(&path)
    }

    pub fn is_child_of(&self, mut c: ClassId, parent: ClassId) -> bool {
        loop {
            if c == parent {
                return true;
            }
            match self.classes[c].super_class {
                Some(s) => c = s,
                None => return false,
            }
        }
    }

    /// Creates an instance (copy of the class's default values). Outer like the engine: the
    /// map's package for an actor (ULevel::SpawnActor, the level's GetOuter()), otherwise the
    /// Transient package (objects created by the engine: InteractionMaster, Canvas…).
    pub fn spawn(&mut self, class: ClassId) -> ObjId {
        let outer = match (self.class("Actor"), self.map_package) {
            (Some(actor), Some(pi)) if self.is_child_of(class, actor) => self.packages[pi].name.clone(),
            _ => "Transient".to_string(),
        };
        self.spawn_in(class, outer, None)
    }

    /// UObject::GetPathName: "Outer.Name".
    pub fn path_name(&self, id: ObjId) -> String {
        match self.objects.get(id).and_then(|o| o.as_ref()) {
            Some(o) if o.outer.is_empty() => o.name.clone(),
            Some(o) => format!("{}.{}", o.outer, o.name),
            None => "None".into(),
        }
    }

    /// FName(…, FNAME_Add): the name now exists in the table.
    pub(crate) fn add_name(&mut self, name: &str) {
        if let Some(t) = self.name_table.as_mut() {
            t.entry(name.to_lowercase()).or_insert_with(|| name.to_string());
        }
    }

    /// FName(…, FNAME_Find): is the name in the name table? The packages loaded by the
    /// game add their whole name table (ULinkerLoad): script packages, map, and the
    /// packages it imports (recursively).
    pub(crate) fn name_exists(&mut self, name: &str) -> bool {
        self.canonical_name(name).is_some()
    }

    /// Spelling of the name in the table (FNameEntry: that of its first registration, the name created
    /// by StringToName "gamma" is "Gamma" if the package writes Gamma), None if it does not exist.
    pub(crate) fn canonical_name(&mut self, name: &str) -> Option<String> {
        if self.name_table.is_none() {
            let key_tuple = self.key_names();
            let mut table = match self.table_names_packages.take() {
                Some((c, t)) if c == key_tuple => t,
                _ => self.names_packages(),
            };
            for o in self.objects.iter().flatten() {
                table.entry(o.name.to_lowercase()).or_insert_with(|| o.name.clone());
            }
            self.name_table = Some(table);
        }
        self.name_table.as_ref().and_then(|t| t.get(&name.to_lowercase()).cloned())
    }

    /// What `names_packages` reads: the map and the loaded packages, in order.
    fn key_names(&self) -> KeyNames {
        (self.map_file.clone(), self.packages.iter().map(|p| p.name.clone()).collect())
    }

    /// End of UGameEngine::LoadMap: the part of the name table that only depends on the packages
    /// (otherwise read from disk at the game's first StringToName, in the middle of play: 20 to 45 ms at the
    /// first dialogue of a map). Nothing is changed in the VM; the first StringToName takes
    /// this table if the key has not changed.
    pub(crate) fn prepare_table_names(&mut self) {
        if self.name_table.is_some() {
            return;
        }
        let key_tuple = self.key_names();
        if self.table_names_packages.as_ref().is_some_and(|(c, _)| *c == key_tuple) {
            return;
        }
        let t = self.names_packages();
        self.table_names_packages = Some((key_tuple, t));
    }

    /// Names of the loaded packages, then of the packages the map imports (recursively, read from
    /// disk); the kept spelling is that of the first registration.
    fn names_packages(&self) -> std::collections::HashMap<String, String> {
        {
            let mut table: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            for p in &self.packages {
                for n in &p.package.names {
                    table.entry(n.name.to_lowercase()).or_insert_with(|| n.name.clone());
                }
                table.entry(p.name.to_lowercase()).or_insert_with(|| p.name.clone());
            }
            if let Some(map) = self.map_file.clone() {
                // The game folder (the System folder's parent; a mod's map is not in the game's Maps).
                let game = self.system_dir.as_ref().and_then(|d| d.parent()).or_else(|| map.parent().and_then(|d| d.parent())).map(|d| d.to_path_buf());
                if let Some(mut library) = game.and_then(|g| openxiii_package::library::Library::new(&g).ok()) {
                    let mut pending: Vec<String> = Vec::new();
                    let mut seen = std::collections::HashSet::new();
                    let push_imports = |p: &openxiii_package::Package, pending: &mut Vec<String>| {
                        for i in &p.imports {
                            if i.package == 0 && p.name(i.class_name).eq_ignore_ascii_case("Package") {
                                pending.push(p.name(i.object_name).to_string());
                            }
                        }
                    };
                    if let Some(p) = self.packages.iter().find(|p| Some(p.name.as_str()) == map.file_stem().and_then(|s| s.to_str())) {
                        push_imports(&p.package, &mut pending);
                    }
                    while let Some(name) = pending.pop() {
                        if !seen.insert(name.to_lowercase()) || self.packages.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                            continue;
                        }
                        if let Some(lp) = library.get(&name) {
                            for n in &lp.package.names {
                                table.entry(n.name.to_lowercase()).or_insert_with(|| n.name.clone());
                            }
                            push_imports(&lp.package, &mut pending);
                        }
                    }
                }
            }
            table
        }
    }

    /// ClassUnique of each class, by name, for the next map: only those that
    /// survive the travel's CollectGarbage (class_gc.rs); the others will be reloaded, their
    /// counter starts again from 0.
    pub(crate) fn class_unique_by_name(&self) -> std::collections::HashMap<String, u16> {
        let alive = self.classes_surviving_travel();
        let mut out: std::collections::HashMap<String, u16> =
            self.engine.class_unique.iter().filter(|(n, _)| self.class(n).is_some_and(|c| alive.contains(&c))).map(|(n, &u)| (n.clone(), u)).collect();
        for (&c, &n) in &self.class_unique {
            if alive.contains(&c) {
                out.insert(self.classes[c].name.to_lowercase(), n);
            }
        }
        out
    }

    /// UObject::MakeUniqueObjectName: class name without its trailing
    /// digits, followed by ClassUnique++ until a name is free in the Outer.
    fn make_unique_name(&mut self, class: ClassId, outer: &str) -> String {
        let base = self.classes[class].name.trim_end_matches(|c: char| c.is_ascii_digit()).to_string();
        loop {
            // Counter taken from the previous map (EngineState, travel) at first use.
            let key_tuple = self.classes[class].name.to_lowercase();
            let carried = self.engine.class_unique.get(&key_tuple).copied().unwrap_or_else(|| crate::class_gc::first_number(&key_tuple));
            let unique = self.class_unique.entry(class).or_insert(carried);
            let n = *unique;
            *unique = unique.wrapping_add(1);
            let number = n.to_string();
            // appStrncpy(Result, Base, 0x42 - len) then the number: 64 characters at most.
            let keep = (0x42 - number.len()).saturating_sub(1).min(base.len());
            let name = format!("{}{number}", &base[..keep]);
            if !self.object_paths.contains(&format!("{outer}.{name}").to_lowercase()) {
                return name;
            }
        }
    }

    /// Checks of UObject::StaticAllocateObject before creation: an abstract class
    /// (ClassFlags & 1) is not instantiated ("Abstract"), and the Outer must descend from ClassWithin
    /// ("NotWithin"); without an Outer, `new` takes the transient package, which is only an Object. Returns the key of
    /// the Core.int error ([Errors]) when creation is refused.
    pub(crate) fn creation_refused(&self, class: ClassId, outer: Option<ObjId>) -> Option<&'static str> {
        if self.classes[class].flags & 1 != 0 {
            return Some("Abstract");
        }
        let within = self.class_within.get(&class).and_then(|n| self.class(n))?;
        if self.classes[within].name.eq_ignore_ascii_case("Object") {
            return None;
        }
        let ok = match outer {
            Some(o) => self.is_child_of(self.object(o).class, within),
            None => false,
        };
        (!ok).then_some("NotWithin")
    }

    /// UObject::execNew after reading its parameters: the created object (or None) and the
    /// script warnings, in order. Forbidden flags are reported ("new: Flags %08X not
    /// allowed") before creation; StaticAllocateObject refuses an abstract class and an
    /// Outer that does not descend from ClassWithin; StaticConstructObject without a SubobjectRoot, InitProperties takes
    /// the object itself.
    pub(crate) fn exec_new(&mut self, outer: Option<ObjId>, name: Option<String>, flag_bits: u32, class_value: Option<ClassId>) -> (Option<ObjId>, Vec<WarningNew>) {
        let mut warnings_list = Vec::new();
        if flag_bits & 0xFF87_BF7A != 0 {
            warnings_list.push(WarningNew { key_tuple: None, text_value: format!("new: Flags {:08X} not allowed", flag_bits & 0xFF87_BF7A) });
        }
        // UObject::StaticAllocateObject, null class: "Empty class for object %s", on execNew's Error (the stack).
        let Some(c) = class_value else {
            warnings_list.push(WarningNew { key_tuple: None, text_value: format!("Empty class for object {}", name.as_deref().unwrap_or("None")) });
            return (None, warnings_list);
        };
        if let Some(key_tuple) = self.creation_refused(c, outer) {
            let model = self.localize("Errors", key_tuple, "Core", false);
            let (request, class_value) = (name.clone().unwrap_or_else(|| "None".into()), self.classes[c].name.clone());
            let outer_class = outer.map_or_else(|| "Package".to_string(), |o| self.classes[self.object(o).class].name.clone());
            let within = self.class_within.get(&c).cloned().unwrap_or_else(|| "Object".into());
            let text_value = match key_tuple {
                "Abstract" => crate::value::printf_s(&model, &[&request, &class_value]),
                _ => crate::value::printf_s(&model, &[&class_value, &request, &outer_class, &within]),
            };
            warnings_list.push(WarningNew { key_tuple: Some(key_tuple), text_value });
            return (None, warnings_list);
        }
        let path = outer.map_or_else(|| "Transient".to_string(), |o| self.path_name(o));
        let id = self.spawn_bare(c, path, name);
        if let Some(o) = outer {
            self.set_prop(id, "Outer", Value::Object(Some(o)));
        }
        self.objects_new.insert(id);
        self.instantiate_sub_objects(id, id);
        (Some(id), warnings_list)
    }

    /// Creates an instance in `outer`, under `name` (loaded object) or a unique name; when constructed
    /// (without a name), its instanced subobjects get their copies (`instantiate_sub_objects`).
    pub fn spawn_in(&mut self, class: ClassId, outer: String, name: Option<String>) -> ObjId {
        let constructed = name.is_none();
        let id = self.spawn_bare(class, outer, name);
        if constructed {
            self.instantiate_sub_objects(id, id);
        }
        id
    }

    /// Slot of the property stored in the object itself (types::Props) for a class: bDeleteMe.
    pub(crate) fn case_hot(&self, class: ClassId) -> usize {
        self.classes.get(class).and_then(|c| c.prop_index.get("bdeleteme").copied()).unwrap_or(usize::MAX)
    }

    /// `spawn_in` without copying the instanced subobjects (execNew does it once the Outer is set).
    pub(crate) fn spawn_bare(&mut self, class: ClassId, outer: String, name: Option<String>) -> ObjId {
        let cdo = self.classes[class].cdo;
        #[allow(unused_mut)]
        let mut props = self.objects[cdo].as_ref().map(|o| o.props.clone()).unwrap_or_default();
        let constructed = name.is_none();
        let name = name.unwrap_or_else(|| self.make_unique_name(class, &outer));
        self.object_paths.insert(format!("{outer}.{name}").to_lowercase());
        self.add_name(&name);
        self.objects.push(Some(Object { class, props, state: None, state_class: None, name, outer }));
        let id = self.objects.len() - 1;
        self.set_intrinsics(id);
        // ParticleEmitters of an Emitter's default values: copies of its own for an Emitter built
        // in game; a loaded object (RF_NeedLoad) keeps the templates until its properties.
        if self.class("Emitter").is_some_and(|e| self.is_child_of(class, e)) {
            if constructed {
                self.construct_emitters(id);
            } else {
                self.instance_emitters(id);
            }
        }
        // AInternetLink::AInternetLink: native fields (Socket -1…) and InitSockets.
        if self.class("InternetLink").is_some_and(|c| self.is_child_of(class, c)) {
            self.internet_link_construct(id);
        }
        id
    }

    /// Native Object properties held by the engine: Class and Name.
    pub(crate) fn set_intrinsics(&mut self, id: ObjId) {
        let (class, name) = {
            let o = self.object(id);
            (o.class, o.name.clone())
        };
        // Object's properties (Class, Name) are the first of every class; a class may declare
        // another "Name" property (ParticleEmitter's string) that prop_slot, which returns the last
        // declared one, must not confuse with it.
        if let Some(oc) = self.class("Object") {
            for (name_value, value_value) in [("class", Value::Class(Some(class))), ("name", Value::Name(crate::value::NameStruct::from(name)))] {
                if let Some(&slot) = self.classes[oc].prop_index.get(name_value) {
                    if let Some(v) = self.objects[id].as_mut().and_then(|o| o.props.get_mut(slot)) {
                        *v = value_value;
                    }
                }
            }
        }
    }

    fn object(&self, id: ObjId) -> &Object {
        self.objects[id].as_ref().or_else(|| self.freed.get(&id)).unwrap_or_else(|| panic!("destroyed object {id} (stack: {})", self.backtrace()))
    }

    /// Readable call stack ("Class.Function < …").
    pub fn backtrace(&self) -> String {
        self.stack
            .iter()
            .rev()
            .take(6)
            .map(|&f| format!("{}.{}", self.classes[self.functions[f].owner].name, self.functions[f].name))
            .collect::<Vec<_>>()
            .join(" < ")
    }

    pub(crate) fn current_function(&self) -> String {
        self.stack.last().map(|&f| format!("{}.{}", self.classes[self.functions[f].owner].name, self.functions[f].name)).unwrap_or_else(|| "state code".into())
    }

    /// Script warning like FFrame::Logf ("%s (%s:%04X) %s"):
    /// function (with its state) and position in the code, which the game writes as a ScriptWarning.
    fn script_warn(&mut self, offset: u32, message: &str) {
        self.script_warn_in(self.stack.len(), offset, message);
    }

    /// Script warning in the name of function `stack[depth - 1]` (state code if 0).
    fn script_warn_in(&mut self, depth: usize, offset: u32, message: &str) {
        let (kind, at, package) = match depth.checked_sub(1).and_then(|i| self.stack.get(i)) {
            Some(&f) => {
                let f = &self.functions[f];
                let at = match &f.state {
                    Some(state) => format!("{}.{}.{}", self.classes[f.owner].name, state, f.name),
                    None => format!("{}.{}", self.classes[f.owner].name, f.name),
                };
                ("Function", at, self.packages[f.package].name.clone())
            }
            // State code: Stack.Node is the UState ("State XIDPawn.BaseSoldier.Attente").
            None => match self.state_code_obj.and_then(|o| self.objects.get(o)).and_then(|o| o.as_ref()) {
                Some(o) => {
                    let c = o.state_class.unwrap_or(o.class);
                    let state = o.state.clone().unwrap_or_default();
                    ("State", format!("{}.{state}", self.classes[c].name), self.packages[self.classes[c].package].name.clone())
                }
                None => ("State", "state code".into(), String::new()),
            },
        };
        // In the log, at its place among the ScriptLogs, preceded as in XIII.log by the object
        // ("Class Package.Object", FFrame::Logf → FOutputDevice: Object->GetClass()->GetName(),
        // Object->GetPathName()): Stack.Object in a function, the state code's object otherwise.
        // Always, like FFrame::Serialize: the list of warnings (summary) is bounded, not
        // the log (a campaign keeps the same VM and quickly exceeds 10,000 warnings).
        let who = self.frame_object_full_name();
        let (_, line) = frame_serialize(NAME_SCRIPT_WARNING, false, &who, &format!("{kind} {package}.{at}"), offset, message);
        self.log.push(format!("ScriptWarning: {line}"));
        self.warn(format!("{at}:{offset:04X} {message}"));
    }

    /// Stack.Logf of a native: script warning in the caller's frame, at the end
    /// of the call expression.
    pub(crate) fn native_warn(&mut self, message: &str) {
        let offset = self.call_ends.last().copied().unwrap_or(self.cur_offset);
        self.native_warn_at(offset, message);
    }

    /// Like `native_warn`, at the given offset (execEnable warns before going past the EndFunctionParms).
    pub(crate) fn native_warn_at(&mut self, offset: u32, message: &str) {
        let native = self.stack.last().copied().filter(|&f| self.functions[f].native != 0 || self.functions[f].flags & openxiii_script::objects::func::NATIVE != 0);
        let this = native.and_then(|_| self.this_stack.pop());
        if native.is_some() {
            self.stack.pop();
        }
        self.script_warn(offset, message);
        if let Some(f) = native {
            self.stack.push(f);
        }
        if let Some(o) = this {
            self.this_stack.push(o);
        }
    }

    /// Object->GetFullName() of the current frame ("Class Package.Object"): Stack.Object in a function,
    /// the state code's object otherwise; "None" without an object, like UObject::GetFullName.
    fn frame_object_full_name(&self) -> String {
        let obj = if self.stack.is_empty() { self.state_code_obj } else { self.this_stack.last().copied() };
        match obj.and_then(|o| self.objects.get(o).and_then(|x| x.as_ref()).map(|x| (o, x.class))) {
            Some((o, c)) => format!("{} {}", self.classes[c].name, self.path_name(o)),
            None => "None".into(),
        }
    }

    /// FFrame::Serialize with NAME_Critical: GError->Logf("%s (%s:%04X) %s"), which
    /// stops the original game; the VM writes it to the log and to the warnings.
    fn script_critical(&mut self, offset: u32, message: &str) {
        let at = self.current_function();
        let who = self.frame_object_full_name();
        let (_, line) = frame_serialize(NAME_CRITICAL, false, &who, &at, offset, message);
        self.log.push(format!("Critical: {line}"));
        self.warn(format!("{at}:{offset:04X} {message} (fatal error in the original game)"));
    }

    /// execJump and execJumpIfNot: beyond a million jumps
    /// since GInitRunaway, "Runaway loop detected (over 1000000 iterations)" as NAME_Critical
    /// (unless -norunaway) and the counter starts again from zero.
    fn runaway_step(&mut self, offset: u32) -> Result<()> {
        self.runaway += 1;
        if self.runaway > 1_000_000 {
            self.runaway = 0;
            let message = "Runaway loop detected (over 1000000 iterations)";
            self.script_critical(offset, message);
            return Err(VmError::Unsupported(message.into()));
        }
        Ok(())
    }

    fn warn(&mut self, message: String) {
        if self.warnings.len() < 10_000 {
            self.warnings.push(message);
        }
    }

    // ---- Resolution of bytecode references ----

    /// prop_slot of a bytecode instance variable (package, property reference) on
    /// `obj`, kept per class.
    pub(crate) fn var_slot(&mut self, package: usize, r: i32, obj: ObjId) -> Result<usize> {
        let class = self.object(obj).class;
        let direct = self.var_slot_direct_active;
        let h = VarSlotEntry::case(package, r, class);
        if direct {
            let e = &self.var_slot_direct[h];
            if e.package == package as u32 && e.r == r && (e.class == class as u32) {
                return Ok(e.slot as usize);
            }
        }
        let key = (package, r, class);
        let slot = match self.var_slot_cache.get(&key) {
            Some(&slot) => slot,
            None => {
                let name = self.ref_name(package, r);
                let slot = self.prop_slot(obj, &name)?;
                self.var_slot_cache.insert(key, slot);
                slot
            }
        };
        if direct && package < u32::MAX as usize && class < u32::MAX as usize && slot <= u32::MAX as usize {
            self.var_slot_direct[h] = VarSlotEntry { package: package as u32, r, class: class as u32, slot: slot as u32 };
        }
        Ok(slot)
    }

    fn ref_name(&self, package: usize, r: i32) -> String {
        self.packages[package].package.object_name(r.into()).to_string()
    }

    fn name_text(&self, package: usize, n: i32) -> String {
        let n = self.packages[package].package.name(n);
        if n.eq_ignore_ascii_case("None") {
            String::new()
        } else {
            n.to_string()
        }
    }

    pub(crate) fn prop_slot(&self, obj: ObjId, name: &str) -> Result<usize> {
        let class = self.object(obj).class;
        // Literal name from the code (text in the executable's image, fixed for the whole game):
        // compact table of 16 bytes per entry (64 KB), key = class + address and length of the
        // text, without comparing its bytes; names missing from the class are kept there too.
        if let Some(key_tuple) = key_literal(name).filter(|_| class <= u32::MAX as usize) {
            let h = slot_literal_case(class, key_tuple);
            if let Ok(t) = self.slot_literals.try_borrow() {
                let e = t[h];
                let class_matches = e.class == class as u32;
                if e.key_tuple == key_tuple && class_matches {
                    return if e.slot == SLOT_ABSENT { Err(VmError::UnknownProperty(format!("{}.{name}", self.classes[class].name))) } else { Ok(e.slot as usize) };
                }
            }
            let found = with_lower(name, |key| self.classes[class].prop_index.get(key).copied());
            if let (Ok(mut t), true) = (self.slot_literals.try_borrow_mut(), found.is_none_or(|s| s < SLOT_ABSENT as usize)) {
                t[h] = SlotLiteral { class: class as u32, slot: found.map_or(SLOT_ABSENT, |s| s as u32), key_tuple };
            }
            return found.ok_or_else(|| VmError::UnknownProperty(format!("{}.{name}", self.classes[class].name)));
        }
        let h = (class.wrapping_mul(0x9E37_79B9) ^ (name.as_ptr() as usize >> 3) ^ name.len().wrapping_mul(0x85EB_CA6B)).wrapping_mul(0x2545_F491) >> 7;
        let h = h & (SLOT_CACHE - 1);
        if let Ok(cache) = self.slot_cache.try_borrow() {
            let e = &cache[h];
            if e.class == class && e.name_value() == Some(name.as_bytes()) {
                return Ok(e.slot as usize);
            }
        }
        let found = with_lower(name, |key| self.classes[class].prop_index.get(key).copied());
        if let (Some(slot), Ok(mut cache)) = (found, self.slot_cache.try_borrow_mut()) {
            if let Some(e) = SlotEntry::new(class, name, slot) {
                cache[h] = e;
            }
        }
        found.ok_or_else(|| VmError::UnknownProperty(format!("{}.{name}", self.classes[class].name)))
    }

    /// Linking a class has replaced its property index.
    pub(crate) fn clear_slot_cache(&mut self) {
        self.native_slots.clear();
        self.tick_slots.clear();
        for e in self.slot_cache.borrow_mut().iter_mut() {
            *e = SlotEntry::EMPTY;
        }
        for e in self.slot_literals.borrow_mut().iter_mut() {
            *e = SlotLiteral::EMPTY;
        }
        for e in self.is_a_cache.borrow_mut().iter_mut() {
            *e = IsAEntry::EMPTY;
        }
        self.flags_classes.borrow_mut().clear();
        self.sm_generations.clear();
        self.particles.params_kept.clear();
        self.particles.render_actors_valid.set(false);
        for e in self.func_cache.borrow_mut().iter_mut() {
            *e = FuncEntry::EMPTY;
        }
        self.call_preps.clear();
        self.member_cache.clear();
        self.class_ref_cache.borrow_mut().clear();
        self.final_cache.clear();
        self.particles.emitter_slots.borrow_mut().clear();
    }

    /// Property by name, without copying (Void if missing).
    pub fn propr(&self, obj: ObjId, name: &str) -> &Value {
        static VOID: Value = Value::Void;
        match (self.prop_slot(obj, name), self.objects.get(obj).and_then(|o| o.as_ref()).or_else(|| self.freed.get(&obj))) {
            (Ok(slot), Some(o)) => o.props.get(slot).unwrap_or(&VOID),
            _ => &VOID,
        }
    }

    /// Current state of an object (StateFrame->StateNode). (ProbeMask, IgnoreMask) of the current
    /// state of `obj` (None: no state, or state not found), like `current_state`, kept per object
    /// (`states_resolved`).
    pub(crate) fn masks_state(&self, obj: ObjId) -> Option<(u64, u64)> {
        let fresh_value = || self.current_state(obj).map(|(_, sd)| (sd.probe_mask, sd.ignore_mask));
        if !self.cache_states {
            return fresh_value();
        }
        let o = self.objects.get(obj)?.as_ref()?;
        let state_value = o.state.as_deref()?;
        let nb_classes = self.classes.len();
        if let Ok(cache) = self.states_resolved.try_borrow() {
            if let Some(Some(e)) = cache.get(obj) {
                if e.class_value == o.class && e.state_class == o.state_class && e.nb_classes == nb_classes && (&*e.state_value == state_value) {
                    return e.masks;
                }
            }
        }
        let masks = fresh_value();
        if let Ok(mut cache) = self.states_resolved.try_borrow_mut() {
            if cache.len() <= obj {
                cache.resize(obj + 1, None);
            }
            cache[obj] = Some(StateResolved { state_value: state_value.into(), state_class: o.state_class, class_value: o.class, nb_classes, masks, tick: None });
        }
        masks
    }

    /// Tick event of the current state of `obj` for the idle tick (tick.rs idle_actor_tick):
    /// `idle_event(find_function(obj, "Tick", false))`, kept in the object's entry
    /// (`states_resolved`, same keys as `masks_state`, plus the number of functions and classes
    /// like find_function's cache).
    pub(crate) fn tick_state(&self, obj: ObjId) -> crate::tick::IdleEvent {
        let fresh_value = || self.idle_event(self.find_function(obj, "Tick", false));
        if !self.cache_states {
            return fresh_value();
        }
        let Some(o) = self.objects.get(obj).and_then(Option::as_ref) else { return fresh_value() };
        let Some(state_value) = o.state.as_deref() else { return fresh_value() };
        let nb_classes = self.classes.len();
        let generation = self.functions.len() + nb_classes;
        let valid_value = |e: &StateResolved| e.class_value == o.class && e.state_class == o.state_class && e.nb_classes == nb_classes && (&*e.state_value == state_value);
        if let Ok(cache) = self.states_resolved.try_borrow() {
            if let Some(Some(e)) = cache.get(obj) {
                if let Some((g, t)) = e.tick.filter(|_| valid_value(e)) {
                    if g == generation {
                        return t;
                    }
                }
            }
        }
        let t = fresh_value();
        let masks = self.current_state(obj).map(|(_, sd)| (sd.probe_mask, sd.ignore_mask));
        if let Ok(mut cache) = self.states_resolved.try_borrow_mut() {
            if cache.len() <= obj {
                cache.resize(obj + 1, None);
            }
            match cache[obj].as_mut().filter(|e| valid_value(e)) {
                Some(e) => e.tick = Some((generation, t)),
                None => cache[obj] = Some(StateResolved { state_value: state_value.into(), state_class: o.state_class, class_value: o.class, nb_classes, masks, tick: Some((generation, t)) }),
            }
        }
        t
    }

    pub(crate) fn current_state(&self, obj: ObjId) -> Option<(ClassId, &types::StateDef)> {
        let o = self.objects.get(obj)?.as_ref()?;
        with_lower(o.state.as_ref()?, |state| {
            if let Some(sd) = o.state_class.and_then(|c| self.classes[c].states.get(state)) {
                return Some((o.state_class?, sd));
            }
            self.find_state(o.class, state)
        })
    }

    /// Names of the StateNode and its SuperFields: the current state then its parents, or the class and
    /// its parent classes outside any state.
    /// Does state_chain_names(obj) contain `name` (case-insensitive)? Same walk, without copying the names.
    pub(crate) fn state_chain_has(&self, obj: ObjId, name: &str) -> bool {
        let Some(o) = self.objects.get(obj).and_then(|o| o.as_ref()) else {
            return false;
        };
        if o.state.is_some() {
            let mut state = self.current_state(obj);
            let mut n = 0;
            while let (Some((ci, sd)), true) = (state, n < 32) {
                if sd.name.eq_ignore_ascii_case(name) {
                    return true;
                }
                n += 1;
                state = self.parent_state(ci, sd);
            }
            false
        } else {
            let mut c = Some(o.class);
            while let Some(ci) = c {
                if self.classes[ci].name.eq_ignore_ascii_case(name) {
                    return true;
                }
                c = self.classes[ci].super_class;
            }
            false
        }
    }

    pub(crate) fn state_chain_names(&self, obj: ObjId) -> Vec<String> {
        let mut names = Vec::new();
        let Some(o) = self.objects.get(obj).and_then(|o| o.as_ref()) else {
            return names;
        };
        if o.state.is_some() {
            let mut state = self.current_state(obj);
            while let (Some((ci, sd)), true) = (state, names.len() < 32) {
                names.push(sd.name.clone());
                state = self.parent_state(ci, sd);
            }
        } else {
            let mut c = Some(o.class);
            while let Some(ci) = c {
                names.push(self.classes[ci].name.clone());
                c = self.classes[ci].super_class;
            }
        }
        names
    }

    /// UObject::FindState: the state with this name declared by the class or,
    /// failing that, by the closest parent class.
    pub(crate) fn find_state(&self, class: ClassId, name: &str) -> Option<(ClassId, &types::StateDef)> {
        with_lower(name, |name| {
            let mut c = Some(class);
            while let Some(ci) = c {
                if let Some(sd) = self.classes[ci].states.get(name) {
                    return Some((ci, sd));
                }
                c = self.classes[ci].super_class;
            }
            None
        })
    }

    /// Parent state (UState::SuperField) of a state declared by class `ci`.
    pub(crate) fn parent_state(&self, ci: ClassId, sd: &types::StateDef) -> Option<(ClassId, &types::StateDef)> {
        let name = sd.parent.as_ref()?.to_lowercase();
        if let Some(&pc) = sd.parent_class.as_ref().and_then(|c| self.class_index.get(c)) {
            if let Some(p) = self.classes[pc].states.get(&name) {
                return Some((pc, p));
            }
        }
        // Parent's class unknown: lookup by name, above `ci` for a state with the same name.
        let mut c = if name == sd.name.to_lowercase() { self.classes[ci].super_class } else { Some(ci) };
        while let Some(x) = c {
            if let Some(p) = self.classes[x].states.get(&name) {
                return Some((x, p));
            }
            c = self.classes[x].super_class;
        }
        None
    }

    /// Function called by name on an object: current state (and its parents), then classes.
    pub fn find_function(&self, obj: ObjId, name: &str, global: bool) -> Option<FuncId> {
        // Answer already found for (class, state and class of the object's state, requested name):
        // the name and state are kept in the entry; emptied when classes are linked.
        let o = self.object(obj);
        let state_value = if global { None } else { o.state.as_deref() };
        let sc = if global { usize::MAX } else { o.state_class.unwrap_or(usize::MAX - 1) };
        let generation = self.functions.len() + self.classes.len();
        let h = FuncEntry::case(o.class, name, state_value);
        if let Ok(cache) = self.func_cache.try_borrow() {
            if let Some(r) = cache[h].response(o.class, sc, state_value, name, global, generation) {
                return r;
            }
        }
        let r = with_lower(name, |key| self.find_function_lower(obj, key, global));
        if let (Some(e), Ok(mut cache)) = (FuncEntry::new(o.class, sc, state_value, name, global, r, generation), self.func_cache.try_borrow_mut()) {
            cache[h] = e;
        }
        r
    }

    fn find_function_lower(&self, obj: ObjId, key: &str, global: bool) -> Option<FuncId> {
        let o = self.object(obj);
        if !global {
            let mut state = self.current_state(obj);
            let mut guard = 0;
            while let (Some((ci, sd)), true) = (state, guard < 32) {
                guard += 1;
                if let Some(&f) = sd.functions.get(key) {
                    return Some(f);
                }
                state = self.parent_state(ci, sd);
            }
        }
        let mut c = Some(o.class);
        while let Some(ci) = c {
            if let Some(&f) = self.classes[ci].functions.get(key) {
                return Some(f);
            }
            c = self.classes[ci].super_class;
        }
        None
    }

    /// Function designated by a "[Package.]Class[.State].Function" path (final/super call).
    fn function_by_path(&self, path: &str) -> Option<FuncId> {
        let parts: Vec<&str> = path.split('.').collect();
        let fname = parts.last()?.to_lowercase();
        for k in (0..parts.len() - 1).rev() {
            if let Some(&c) = self.class_index.get(&parts[k].to_lowercase()) {
                if k + 2 < parts.len() {
                    let state = parts[k + 1].to_lowercase();
                    return self.classes[c].states.get(&state).and_then(|s| s.functions.get(&fname)).copied();
                }
                let mut ci = Some(c);
                while let Some(x) = ci {
                    if let Some(&f) = self.classes[x].functions.get(&fname) {
                        return Some(f);
                    }
                    ci = self.classes[x].super_class;
                }
                return None;
            }
        }
        None
    }

    // ---- Places ----

    /// Only copies the requested value, never the arrays/structs that contain it.
    pub fn get(&self, frame_locals: &[Value], place: &Place) -> Value {
        if let Place::Length(inner) = place {
            return Value::Int(match self.value_at(frame_locals, inner).as_deref() {
                Some(Value::Array(items)) => items.len() as i32,
                _ => 0,
            });
        }
        self.value_at(frame_locals, place).map_or(Value::Void, Cow::into_owned)
    }

    /// Value in place (reference), or made on the fly for a field of an inline vector / rotator
    /// (`Value::Vector` / `Value::Rotator` have no `Value` for their fields).
    fn value_at<'a>(&'a self, locals: &'a [Value], place: &Place) -> Option<Cow<'a, Value>> {
        match place {
            Place::Local(i) => locals.get(*i).map(Cow::Borrowed),
            // Keeps the reads of the freed object and the error of the unknown object.
            Place::Prop(o, s) => self.object(*o).props.get(*s).map(Cow::Borrowed),
            Place::Element(p, i) => element_read(self.value_at(locals, p)?, *i),
            Place::Field(p, i) => read_field(self.value_at(locals, p)?, *i),
            // A length is a temporary integer, never a container.
            Place::Length(_) | Place::Nowhere => None,
        }
    }

    fn slot_ref<'a>(&'a self, locals: &'a [Value], place: &Place) -> Option<Cow<'a, Value>> {
        match place {
            Place::Local(i) => locals.get(*i).map(Cow::Borrowed),
            Place::Prop(o, s) => self.objects.get(*o)?.as_ref().or_else(|| self.freed.get(o))?.props.get(*s).map(Cow::Borrowed),
            Place::Element(p, i) => element_read(self.slot_ref(locals, p)?, *i),
            Place::Field(p, i) => read_field(self.slot_ref(locals, p)?, *i),
            Place::Length(_) | Place::Nowhere => None,
        }
    }

    fn slot_mut<'a>(&'a mut self, locals: &'a mut [Value], place: &Place) -> Option<&'a mut Value> {
        match place {
            Place::Local(i) => locals.get_mut(*i),
            Place::Prop(o, s) => self.objects[*o].as_mut()?.props.get_mut(*s),
            // Negative index (usize::MAX): null destination, the write is abandoned
            // (execDynArrayElement, null GPropAddr); `*i + 1` wrapped to 0 and emptied
            // the array.
            Place::Element(_, i) if *i == usize::MAX => None,
            Place::Element(p, i) => match self.slot_mut(locals, p)? {
                Value::Array(items) => {
                    if *i >= items.len() {
                        // Dynamic array: assigning beyond the end grows it.
                        let filler = items.first().cloned().map(|v| zero_like(&v)).unwrap_or(Value::Void);
                        items.resize(*i + 1, filler);
                    }
                    items.get_mut(*i)
                }
                _ => None,
            },
            // Field of an inline vector / rotator: the struct is unpacked in place (same value).
            Place::Field(p, i) => self.slot_mut(locals, p)?.struct_mut()?.1.get_mut(*i),
            Place::Length(_) | Place::Nowhere => None,
        }
    }

    pub fn set(&mut self, function: Option<FuncId>, locals: &mut [Value], place: &Place, value: Value) {
        if let Place::Length(inner) = place {
            // UObject::execLet, length of an array: AddZeroed of the
            // missing elements, or Remove from the end.
            let zero = self.array_zero(function, inner);
            if let Some(Value::Array(items)) = self.slot_mut(locals, inner) {
                let n = value.as_int().max(0) as usize;
                let filler = zero.unwrap_or_else(|| items.first().map(zero_like).unwrap_or(Value::Void));
                items.resize(n, filler);
            }
            return;
        }
        // Field of an inline vector / rotator that receives the inline form of its fields (Float /
        // Int): written into the inline form (same value as the Struct unpacked then written).
        if let Place::Field(p, i) = place {
            match (self.slot_mut(locals, p), &value) {
                (Some(Value::Vector(a)), Value::Float(f)) if *i < 3 => {
                    a[*i] = *f;
                    return;
                }
                (Some(Value::Rotator(a)), Value::Int(n)) if *i < 3 => {
                    a[*i] = *n;
                    return;
                }
                _ => {}
            }
        }
        if let Some(slot) = self.slot_mut(locals, place) {
            *slot = value;
        }
    }

    /// Type and static dimension of the value of a place.
    fn place_def(&mut self, function: Option<FuncId>, place: &Place) -> Option<(Type, usize)> {
        match place {
            Place::Local(i) => function.and_then(|f| self.functions[f].locals.get(*i)).map(|d| (d.ty.clone(), d.dim)),
            Place::Prop(o, s) => {
                let c = self.objects.get(*o)?.as_ref()?.class;
                self.classes[c].props.get(*s).map(|d| (d.ty.clone(), d.dim))
            }
            Place::Element(p, _) => match self.place_def(function, p)? {
                (t, dim) if dim > 1 => Some((t, 1)),
                (Type::Array(inner), _) => Some((*inner, 1)),
                _ => None,
            },
            Place::Field(p, i) => match self.place_def(function, p)? {
                (Type::Struct(name), 1) => self.struct_def(&name)?.fields.get(*i).map(|d| (d.ty.clone(), d.dim)),
                _ => None,
            },
            Place::Length(_) | Place::Nowhere => None,
        }
    }

    /// Zero element (UArrayProperty::Inner) of the dynamic array at place `place`.
    fn array_zero(&mut self, function: Option<FuncId>, place: &Place) -> Option<Value> {
        match self.place_def(function, place)? {
            (Type::Array(inner), 1) => Some(self.zero(&inner, 1)),
            _ => None,
        }
    }

    /// UObject::execDynArrayElement evaluated as the destination of an
    /// assignment: an index beyond the end grows the array (FArray::AddZeroed).
    fn grow_for_write(&mut self, function: Option<FuncId>, locals: &mut [Value], place: &Place) {
        match place {
            Place::Element(p, i) => {
                self.grow_for_write(function, locals, p);
                if *i == usize::MAX {
                    return;
                }
                let Some(zero) = self.array_zero(function, p) else { return };
                if let Some(Value::Array(items)) = self.slot_mut(locals, p) {
                    if items.len() <= *i {
                        items.resize(*i + 1, zero);
                    }
                }
            }
            Place::Field(p, _) | Place::Length(p) => self.grow_for_write(function, locals, p),
            _ => {}
        }
    }

    fn place(&mut self, e: &Expr, frame: &mut Frame) -> Result<Place> {
        let this = frame.this;
        let writing = std::mem::replace(&mut self.writing, true);
        let place = self.place_in(e, frame, this);
        self.writing = writing;
        place
    }

    /// UObject::NetDirty (vtable +0x68) called by execLet, execLetBool, the compound operators (+=, ++, GetAxes, Invert…) and by
    /// CallFunction for each out parameter of a script function, when the destination is an object variable:
    /// GPropObject->NetDirty(GProperty). GProperty is the root instance variable of the destination (a struct field or an
    /// array element does not change GProperty). Only AActor overrides it.
    fn net_dirty_place(&mut self, place: &Place) {
        let mut p = place;
        let (o, s) = loop {
            match p {
                Place::Prop(o, s) => break (*o, *s),
                Place::Element(inner, _) | Place::Field(inner, _) | Place::Length(inner) => p = inner,
                Place::Local(_) | Place::Nowhere => return,
            }
        };
        let flags = self.objects.get(o).and_then(|x| x.as_ref()).and_then(|x| self.classes[x.class].props.get(s)).map(|d| d.flags);
        self.actor_net_dirty(o, flags);
    }

    /// AActor::NetDirty(UProperty*): a Net property (CPF_Net, 0x20) written on an actor sets bNetDirty
    /// (+0x30, bit 0x20); the "Net dirtied by property" message goes to GNull (discarded). `flags`: PropertyFlags (None: no property).
    pub(crate) fn actor_net_dirty(&mut self, a: ObjId, flags: Option<u32>) {
        // Class flag instead of is_a by name; bNetDirty already true: nothing to write (same value).
        if flags.is_some_and(|f| f & openxiii_script::objects::prop::NET != 0)
            && self.is(a, crate::flags_classes::D::Actor)
            && !self.propr(a, "bNetDirty").as_bool()
        {
            self.set_prop(a, "bNetDirty", Value::Bool(true));
        }
    }

    /// Name of the property (GProperty) of a variable expression, for warnings.
    fn variable_name(&mut self, e: &Expr, frame: &Frame) -> String {
        match &e.kind {
            Kind::LocalVariable(r) | Kind::InstanceVariable(r) | Kind::DefaultVariable(r) | Kind::StructMember(r, _) => self.ref_name(frame.package, *r),
            Kind::Context(_, _, _, member) | Kind::ClassContext(_, _, _, member) => self.variable_name(member, frame),
            Kind::BoolVariable(x) | Kind::Skip(_, x) => self.variable_name(x, frame),
            _ => String::new(),
        }
    }

    /// Place of an expression, with members resolved on `target`.
    fn place_in(&mut self, e: &Expr, frame: &mut Frame, target: ObjId) -> Result<Place> {
        match &e.kind {
            Kind::LocalVariable(r) => {
                // Place already found for (package, reference, function): the name is no longer
                // rebuilt or looked up on each access.
                let key = (frame.package, *r, frame.function.map_or(usize::MAX, |f| f));
                if let Some(&index) = self.local_slot_cache.get(&key) {
                    return Ok(Place::Local(index));
                }
                let name = self.ref_name(frame.package, *r).to_lowercase();
                let index = frame
                    .function
                    .and_then(|f| self.functions[f].local_index.get(&name).copied())
                    .ok_or_else(|| VmError::UnknownProperty(format!("local {name}")))?;
                self.local_slot_cache.insert(key, index);
                Ok(Place::Local(index))
            }
            Kind::InstanceVariable(r) => {
                let slot = self.var_slot(frame.package, *r, target)?;
                Ok(Place::Prop(target, slot))
            }
            Kind::DefaultVariable(r) => {
                let cdo = self.classes[self.object(target).class].cdo;
                let slot = self.var_slot(frame.package, *r, cdo)?;
                Ok(Place::Prop(cdo, slot))
            }
            Kind::BoolVariable(x) | Kind::Skip(_, x) => self.place_in(x, frame, target),
            Kind::ArrayElement(index, array) => {
                // UObject::execArrayElement: out-of-bounds index brought back into the static array.
                let i = self.eval(index, frame)?.as_int();
                let base = self.place_in(array, frame, target)?;
                // The bound only depends on the length. Copying the elements here duplicated
                // the whole array (strings and structs included) before reading a single slot.
                let dim = match self.value_at(&frame.locals, &base).as_deref() {
                    Some(Value::Array(items)) => items.len() as i32,
                    _ => 1,
                };
                // Warning if the property exists, Stack.Code after the array.
                if (i < 0 || i >= dim) && !matches!(base, Place::Nowhere) {
                    self.script_warn(e.end, &format!("Accessed array out of bounds ({i}/{dim})"));
                }
                Ok(Place::Element(Box::new(base), i.clamp(0, (dim - 1).max(0)) as usize))
            }
            Kind::DynArrayElement(index, array) => {
                // UObject::execDynArrayElement: negative index outside the array (zero value).
                let i = self.eval(index, frame)?.as_int();
                let base = self.place_in(array, frame, target)?;
                // Outside the array when reading, or negative index, Stack.Code after the array.
                if let Some(Value::Array(items)) = self.slot_ref(&frame.locals, &base).as_deref() {
                    let n = items.len() as i32;
                    if i < 0 || (i >= n && !self.writing) {
                        let name = self.variable_name(array, frame);
                        self.script_warn(e.end, &format!("Accessed array '{name}' out of bounds ({i}/{n})"));
                    }
                }
                let i = if i < 0 { usize::MAX } else { i as usize };
                Ok(Place::Element(Box::new(base), i))
            }
            Kind::StructMember(member, x) => Ok(match self.struct_member_place(*member, x, frame, target)? {
                Some((base, index)) => Place::Field(Box::new(base), index),
                None => Place::Nowhere,
            }),
            Kind::Context(object, _, _, member) => match self.eval(object, frame)? {
                Value::Object(Some(o)) => self.place_in(member, frame, o),
                _ => {
                    // UObject::execContext: Stack.Code is on the jump that follows the object.
                    self.script_warn(member.offset - 3, "Accessed None");
                    Ok(Place::Nowhere)
                }
            },
            Kind::ClassContext(class, _, _, member) => match self.eval(class, frame)? {
                Value::Class(Some(c)) => {
                    let cdo = self.classes[c].cdo;
                    self.place_in(member, frame, cdo)
                }
                _ => {
                    // UObject::execClassContext.
                    self.script_warn(member.offset - 3, "Accessed null class context");
                    Ok(Place::Nowhere)
                }
            },
            Kind::DynArrayLength(x) => Ok(Place::Length(Box::new(self.place_in(x, frame, target)?))),
            // Error expected by the caller (place_probe): without text or stack, it ignores it.
            _ if self.place_probe => Err(VmError::Unsupported(String::new())),
            other => Err(VmError::Unsupported(format!("slot {:?} in {}", std::mem::discriminant(other), self.backtrace()))),
        }
    }

    /// Place of a struct field (StructMember): the struct's place and the field's slot,
    /// None for Place::Nowhere.
    fn struct_member_place(&mut self, member: i32, x: &Expr, frame: &mut Frame, target: ObjId) -> Result<Option<(Place, usize)>> {
        let member = &member;
        {
            {
                // execStructMember evaluates the struct into a buffer (non-null Result):
                // a dynamic array element is read there (log and zero outside the array, never
                // AddZeroed), even when the field is a destination.
                let writing = std::mem::replace(&mut self.writing, false);
                let base = self.place_in(x, frame, target);
                self.writing = writing;
                let base = base?;
                // Struct name read without copying the struct (get copied it whole).
                let struct_name = match self.value_at(&frame.locals, &base).as_deref().and_then(Value::struct_view) {
                    Some((n, _)) => n,
                    // A[i].Field written outside the array: null GPropAddr (the array's Result non-null), nothing is written or grown.
                    _ => match (&base, self.writing) {
                        (Place::Element(..), true) => return Ok(None),
                        // A[i].Field read outside the array: execStructMember reads the field
                        // in its buffer, which execDynArrayElement has zeroed.
                        (Place::Element(p, _), false) => match self.array_zero(frame.function, p).as_ref().and_then(Value::struct_view) {
                            Some((n, _)) => n,
                            _ => return Ok(None),
                        },
                        _ => return Ok(None),
                    },
                };
                // Field slot already found for (package, member reference, struct name):
                // neither the member's name nor the struct's table is read again by text.
                let key_tuple = (frame.package, *member, struct_name.as_ptr() as usize, struct_name.len());
                if let Some(&index) = self.member_cache.get(&key_tuple) {
                    return Ok(Some((base, index)));
                }
                let field = self.ref_name(frame.package, *member);
                let index = self
                    .struct_def(&struct_name)
                    .and_then(|s| s.index.get(&field.to_lowercase()).copied())
                    .ok_or_else(|| VmError::UnknownProperty(format!("{struct_name}.{field}")))?;
                self.member_cache.insert(key_tuple, index);
                Ok(Some((base, index)))
            }
        }
    }

    // ---- Evaluation ----

    fn eval(&mut self, e: &Expr, frame: &mut Frame) -> Result<Value> {
        let this = frame.this;
        self.eval_in(e, frame, this)
    }

    /// place_in whose caller ignores the Unsupported error (temporary value): error without text.
    fn place_probed<T>(&mut self, f: impl FnOnce(&mut Self, &mut Frame) -> Result<T>, frame: &mut Frame) -> Result<T> {
        let probe_value = std::mem::replace(&mut self.place_probe, true);
        let r = f(self, frame);
        self.place_probe = probe_value;
        r
    }

    /// Evaluates an expression; instance variables and calls are resolved on `target`
    /// (the context object), arguments and indices still being evaluated in the current frame.
    fn eval_in(&mut self, e: &Expr, frame: &mut Frame, target: ObjId) -> Result<Value> {
        let writing = std::mem::replace(&mut self.writing, false);
        let probe_value = std::mem::replace(&mut self.place_probe, false);
        let v = self.eval_value(e, frame, target);
        self.writing = writing;
        self.place_probe = probe_value;
        v
    }

    fn eval_value(&mut self, e: &Expr, frame: &mut Frame, target: ObjId) -> Result<Value> {
        tally::inc(tally::EXPRS);
        use Kind::*;
        Ok(match &e.kind {
            LocalVariable(_) | InstanceVariable(_) | DefaultVariable(_) | ArrayElement(..) => {
                let place = self.place_in(e, frame, target)?;
                self.get(&frame.locals, &place)
            }
            DynArrayElement(..) => {
                let place = self.place_in(e, frame, target)?;
                // UObject::execDynArrayElement: outside the array when reading, the
                // result is set to zero (zero element of the type, None for an object).
                match &place {
                    Place::Element(p, _) if self.slot_ref(&frame.locals, &place).is_none() && !matches!(**p, Place::Nowhere) => {
                        self.array_zero(frame.function, p).unwrap_or(Value::Void)
                    }
                    _ => self.get(&frame.locals, &place),
                }
            }
            // Place of the field without building it (no Box): the struct and the field's slot.
            StructMember(member, x) => match self.place_probed(|vm, frame| vm.struct_member_place(*member, x, frame, target), frame) {
                // Field of an element read outside the array: that of the type's zero element.
                Ok(Some((b, index))) if self.slot_ref(&frame.locals, &b).is_none() && matches!(b, Place::Element(..)) => match &b {
                    Place::Element(p, _) => self.array_zero(frame.function, p).and_then(|z| z.field_value(index)).unwrap_or(Value::Void),
                    _ => Value::Void,
                },
                // Struct through a null context: the struct's buffer is zeroed (execContext's
                // Result), the field read is the zero of its type.
                Ok(None) => self.default_for(e, frame),
                Ok(Some((b, index))) => self.value_at(&frame.locals, &b).and_then(|s| s.field_value(index)).unwrap_or(Value::Void),
                // Field of a temporary value (e.g. the result of a call: (A Cross B).Z).
                Err(VmError::Unsupported(_)) => match self.eval_in(x, frame, target)?.struct_view() {
                    Some((name, fields)) => {
                        let field = self.ref_name(frame.package, *member).to_lowercase();
                        let index = self.struct_def(&name).and_then(|d| d.index.get(&field).copied());
                        index.and_then(|i| fields.get(i).cloned()).unwrap_or(Value::Void)
                    }
                    _ => Value::Void,
                },
                Err(other) => return Err(other),
            },
            BoolVariable(x) | Skip(_, x) => self.eval_in(x, frame, target)?,
            Let(a, b) | LetBool(a, b) | LetDelegate(a, b) => {
                // UObject::execLet: the destination is evaluated before the value.
                let place = self.place(a, frame)?;
                // UObject::execLet and execLetBool: GPropObject->NetDirty(GProperty) (not execLetDelegate).
                if !matches!(e.kind, LetDelegate(..)) {
                    self.net_dirty_place(&place);
                }
                // UObject::execLet: null GPropAddr after the destination (None context).
                if matches!(e.kind, Let(..)) && place_is_null(&place) {
                    self.script_warn(b.offset, "Attempt to assigned variable through None");
                }
                self.grow_for_write(frame.function, &mut frame.locals, &place);
                self.return_nothing = false;
                self.last_outputs.clear();
                let v = self.eval(b, frame)?;
                // UObject::execLet: the value is evaluated directly into GPropAddr;
                // a script function that ends without a value (ProcessInternal, EX_Nothing) leaves
                // the destination as it is (DetectionVolume.Trigger: PersoEnContact).
                let is_call = |k: &Kind| matches!(k, VirtualFunction(..) | FinalFunction(..) | GlobalFunction(..) | DynArrayLength(_));
                let call_value = is_call(&b.kind) || matches!(&b.kind, Context(_, _, _, m) | ClassContext(_, _, _, m) if is_call(&m.kind));
                // The call writes its result into the destination (Result = GPropAddr), then copies back its
                // out parameters: if one of them is this destination (x = f(x)), it has the last word.
                let is_call_output = |k: &Kind| matches!(k, VirtualFunction(..) | FinalFunction(..) | GlobalFunction(..) | NativeFunction(..));
                let a_outputs = is_call_output(&b.kind) || matches!(&b.kind, Context(_, _, _, m) | ClassContext(_, _, _, m) if is_call_output(&m.kind));
                if !(call_value && self.return_nothing) {
                    self.set(frame.function, &mut frame.locals, &place, v);
                }
                if a_outputs {
                    // An output that overlaps the destination (the same variable, a field of the destination
                    // struct, the struct that contains the destination) is copied back after the result.
                    let outputs = std::mem::take(&mut self.last_outputs);
                    for (p, val) in outputs {
                        if place_overlaps(&p, &place) {
                            self.set(frame.function, &mut frame.locals, &p, val);
                        }
                    }
                }
                self.return_nothing = false;
                Value::Void
            }
            SelfObject => Value::Object(Some(target)),
            NoObject => Value::Object(None),
            IntConst(i) => Value::Int(*i),
            IntZero => Value::Int(0),
            IntOne => Value::Int(1),
            IntConstByte(b) => Value::Int(*b as i32),
            ByteConst(b) => Value::Byte(*b),
            FloatConst(f) => Value::Float(*f),
            True => Value::Bool(true),
            False => Value::Bool(false),
            StringConst(s) | UnicodeStringConst(s) => Value::Str(s.clone()),
            NameConst(n) => {
                // name_text without copying.
                let n = self.packages[frame.package].package.name(*n);
                Value::Name(crate::value::NameStruct::from_time(if n.eq_ignore_ascii_case("None") { "" } else { n }))
            }
            VectorConst(v) => Value::vector(v[0], v[1], v[2]),
            RotationConst(r) => Value::rotator(r[0], r[1], r[2]),
            ObjectConst(r) => self.object_const(frame.package, *r),
            Nothing => Value::Void,
            Context(object, _, _, member) => match self.eval(object, frame)? {
                Value::Object(Some(o)) => self.eval_in(member, frame, o)?,
                // Data object (texture…): reading its properties in its package.
                Value::Asset(path) if matches!(member.kind, Kind::InstanceVariable(_)) => {
                    let Kind::InstanceVariable(r) = member.kind else { unreachable!() };
                    let name = self.ref_name(frame.package, r);
                    match self.asset_prop(&path, &name) {
                        Some(v) => v,
                        // Unserialized field: default value of the object's class
                        // (Material.NoiseLoudness = 0.1 for a material that does not give it).
                        None => match self.asset_class_default(&path, &name) {
                            Some(v) => v,
                            None => self.default_for(member, frame),
                        },
                    }
                }
                Value::Class(Some(c)) => {
                    let cdo = self.classes[c].cdo;
                    self.eval_in(member, frame, cdo)?
                }
                _ => {
                    // UObject::execContext: Stack.Code is on the jump that follows the object.
                    // The result is zeroed (so execLet writes it, see return_nothing).
                    self.script_warn(member.offset - 3, "Accessed None");
                    self.return_nothing = false;
                    self.default_for(member, frame)
                }
            },
            ClassContext(class, _, _, member) => match self.eval(class, frame)? {
                Value::Class(Some(c)) => {
                    let cdo = self.classes[c].cdo;
                    self.eval_in(member, frame, cdo)?
                }
                _ => {
                    // UObject::execClassContext.
                    // Null class: Result zeroed over the member's size (like execContext),
                    // the destination of a Let receives 0 (and not Void).
                    self.script_warn(member.offset - 3, "Accessed null class context");
                    self.default_for(member, frame)
                }
            },
            Conversion(code, x) => {
                let v = self.eval(x, frame)?;
                // UObject::execStringToName: FName(…, FNAME_Find); a
                // NAME_None result (name missing from the table, empty string or "None" itself)
                // gives a warning (Stack.Code after the string).
                if let (0x5A, Value::Str(s)) = (*code, &v) {
                    if s.is_empty() || s.eq_ignore_ascii_case("None") || !self.name_exists(s) {
                        let s = s.clone();
                        self.script_warn(e.end, &format!("cannot convert {s} to name, not found"));
                        return Ok(Value::Name(crate::value::NameStruct::from(String::new())));
                    }
                    // FName(…, FNAME_Find) returns the existing entry: its spelling, not the string's.
                    let canonical = self.canonical_name(s).unwrap_or_else(|| s.clone());
                    return Ok(Value::Name(crate::value::NameStruct::from(canonical)));
                }
                self.convert(*code, v)
            }
            DynamicCast(class, x) => {
                let wanted = self.class_of_ref(frame.package, *class);
                match self.eval(x, frame)? {
                    Value::Object(Some(o)) if wanted.is_some_and(|w| self.is_child_of(self.object(o).class, w)) => Value::Object(Some(o)),
                    // NetConnection: native class without a script; a client's Player on the host is one.
                    Value::Object(Some(o)) if wanted.is_none() && self.net_is_remote_player(o) && self.ref_name(frame.package, *class).eq_ignore_ascii_case("NetConnection") => Value::Object(Some(o)),
                    Value::Object(_) => Value::Object(None),
                    // A class object (UClass) is neither an actor nor a script object: the
                    // cast only keeps it towards Object / Field / Struct / State / Class.
                    Value::Class(c) => {
                        let name = self.ref_name(frame.package, *class);
                        if ["Object", "Field", "Struct", "State", "Class"].iter().any(|n| name.eq_ignore_ascii_case(n)) {
                            Value::Class(c)
                        } else {
                            Value::Object(None)
                        }
                    }
                    other => other,
                }
            }
            MetaCast(class, x) => {
                let wanted = self.class_of_ref(frame.package, *class);
                match self.eval(x, frame)? {
                    Value::Class(Some(c)) if wanted.is_some_and(|w| self.is_child_of(c, w)) => Value::Class(Some(c)),
                    #[allow(unused_variables)]
                    other => {
                        Value::Class(None)
                    }
                }
            }
            StructCmpEq(_, a, b) => {
                let (a, b) = (self.eval(a, frame)?, self.eval(b, frame)?);
                Value::Bool(struct_identical(&a, &b))
            }
            StructCmpNe(_, a, b) => {
                let (a, b) = (self.eval(a, frame)?, self.eval(b, frame)?);
                Value::Bool(!struct_identical(&a, &b))
            }
            DynArrayLength(x) => match self.place_probed(|vm, frame| vm.place_in(x, frame, target), frame) {
                // UObject::execDynArrayLength: null GPropAddr (array through a
                // null context, element at a negative index), Result is not written: the destination of a
                // Let keeps its value (like a script function that returns nothing).
                Ok(place) if place_is_null(&place) => {
                    self.return_nothing = true;
                    Value::Int(0)
                }
                Ok(place) => match (&place, self.value_at(&frame.locals, &place).as_deref()) {
                    (Place::Length(_), _) => Value::Int(0),
                    (_, Some(Value::Array(items))) => Value::Int(items.len() as i32),
                    _ => Value::Int(0),
                },
                Err(VmError::Unsupported(_)) => match self.eval_in(x, frame, target)? {
                    Value::Array(items) => Value::Int(items.len() as i32),
                    _ => Value::Int(0),
                },
                Err(other) => return Err(other),
            },
            DynArrayInsert(array, index, count) => {
                // UObject::execDynArrayInsert: FArray::InsertZeroed; the
                // messages go through Stack.Logf (script warning with the function).
                let place = self.place(array, frame)?;
                let i = self.eval(index, frame)?.as_int();
                let n = self.eval(count, frame)?.as_int();
                // Null address or zero count, nothing; negative count, message.
                if place_is_null(&place) || n == 0 {
                    return Ok(Value::Void);
                }
                if n < 0 {
                    self.script_warn(e.end, "Attempt to insert a negative number of elements");
                    return Ok(Value::Void);
                }
                let zero = self.array_zero(frame.function, &place);
                let mut message = None;
                if let Some(Value::Array(items)) = self.slot_mut(&mut frame.locals, &place) {
                    let len = items.len() as i32;
                    let at = if i < 0 || i > len {
                        message = Some(format!("Attempt to insert {n} elements at {i} an {len}-element array"));
                        i.clamp(0, len)
                    } else {
                        i
                    } as usize;
                    let filler = zero.unwrap_or_else(|| items.first().map(zero_like).unwrap_or(Value::Void));
                    for _ in 0..n {
                        items.insert(at, filler.clone());
                    }
                }
                if let Some(m) = message {
                    self.script_warn(e.end, &m);
                }
                Value::Void
            }
            // UObject::execDynArrayRemove: nothing for 0 elements; a negative
            // count or a range outside the array is reported (Stack.Logf), the range is brought back
            // into the array, then FArray::Remove.
            DynArrayRemove(array, index, count) => {
                let place = self.place(array, frame)?;
                let i = self.eval(index, frame)?.as_int();
                let n = self.eval(count, frame)?.as_int();
                let len = match self.slot_ref(&frame.locals, &place).as_deref() {
                    Some(Value::Array(items)) => Some(items.len() as i32),
                    _ => None,
                };
                let Some(len) = len else { return Ok(Value::Void) };
                if n == 0 {
                    return Ok(Value::Void);
                }
                if n < 0 {
                    self.script_warn(e.end, "Attempt to remove a negative number of elements");
                    return Ok(Value::Void);
                }
                let (mut i, mut n) = (i, n);
                if i < 0 || i >= len || i + n > len {
                    if n == 1 {
                        self.script_warn(e.end, &format!("Attempt to remove element {i} in an {len}-element array"));
                    } else {
                        self.script_warn(e.end, &format!("Attempt to remove elements {i} through {} in an {len}-element array", i + n - 1));
                    }
                    i = i.clamp(0, len);
                    if i + n > len {
                        n = len - i;
                    }
                }
                if let Some(Value::Array(items)) = self.slot_mut(&mut frame.locals, &place) {
                    if n > 0 {
                        items.drain(i as usize..(i + n) as usize);
                    }
                }
                Value::Void
            }
            New(outer, name, flags, class) => {
                // UObject::execNew: Outer, name, flags then class. Outer
                // missing: the object running the code (this unless Index == INDEX_NONE,
                // then the Transient package); empty name: MakeUniqueObjectName.
                let outer = match self.eval(outer, frame)? {
                    Value::Object(Some(o)) => Some(o),
                    _ if matches!(outer.kind, Nothing) => Some(target),
                    _ => None,
                };
                let name = match self.eval(name, frame)? {
                    Value::Str(n) if !n.is_empty() => Some(n),
                    _ => None,
                };
                // Forbidden flags are reported ("new: Flags %08X not allowed") after the class
                // is evaluated; StaticConstructObject only keeps Flags & 0x784085.
                let flag_bits = self.eval(flags, frame)?.as_int() as u32;
                let class_value = self.eval(class, frame)?;
                let (object_value, warnings_list) = self.exec_new(outer, name, flag_bits, class_value.as_class());
                for a in warnings_list {
                    self.script_warn(e.end, &a.text_value);
                }
                Value::Object(object_value)
            }
            // UObject::execDelegateProperty: Object = this only if the name is not None.
            DelegateProperty(n) => {
                let name_value = self.name_text(frame.package, *n);
                if name_value.is_empty() {
                    Value::Delegate(None, name_value.into())
                } else {
                    Value::Delegate(Some(target), name_value.into())
                }
            }
            VirtualFunction(name, params) => {
                // Name read in the package without copying (name_text).
                let f = {
                    let n = self.packages[frame.package].package.name(*name);
                    self.find_function(target, if n.eq_ignore_ascii_case("None") { "" } else { n }, false)
                };
                match f {
                    Some(f) => self.call_at(e.end, f, target, params, frame)?,
                    None => {
                        // Function ignored by the state or nonexistent.
                        let name = self.name_text(frame.package, *name);
                        self.warn(format!("function not found: {name}"));
                        Value::Void
                    }
                }
            }
            GlobalFunction(name, params) => {
                let name = self.name_text(frame.package, *name);
                let f = self.find_function(target, &name, true).ok_or_else(|| VmError::UnknownFunction(name))?;
                self.call_at(e.end, f, target, params, frame)?
            }
            FinalFunction(r, params) => {
                // Kept as long as no package, class or function has been added.
                let generation = self.packages.len() ^ self.classes.len() << 16 ^ self.class_index.len() << 32 ^ self.functions.len() << 48;
                let f = match self.final_cache.get(&(frame.package, *r)) {
                    Some(&(f, g)) if g == generation => f,
                    _ => {
                        let path = self.packages[frame.package].package.object_path((*r).into());
                        let f = self.function_by_path(&path);
                        self.final_cache.insert((frame.package, *r), (f, generation));
                        f
                    }
                };
                let f = f.ok_or_else(|| VmError::UnknownFunction(self.packages[frame.package].package.object_path((*r).into())))?;
                self.call_at(e.end, f, target, params, frame)?
            }
            DelegateFunction(property, name, params) => {
                // UObject::execDelegateFunction, like UObject::ProcessDelegate
                // for engine calls: a delegate bound to an object being
                // destroyed (IsPendingKill, vtable +0x38: bDeleteMe) is reset to zero,
                // then `this`'s default function is called.
                let pname = self.ref_name(frame.package, *property);
                let slot = self.prop_slot(target, &pname).ok();
                let bound = slot.map(|s| self.object(target).props[s].clone());
                if let (Some(s), Some(Value::Delegate(Some(o), _))) = (slot, &bound) {
                    if !self.alive(*o) || self.native_prop(*o, crate::NativeField::DeleteMe).as_bool() {
                        self.objects[target].as_mut().unwrap().props[s] = Value::Delegate(None, crate::value::NameStruct::EMPTY);
                    }
                }
                let bound = slot.map(|s| self.object(target).props[s].clone());
                let (obj, fname) = match bound {
                    Some(Value::Delegate(Some(o), f)) if !f.is_empty() => (o, f.to_string()),
                    _ => (target, self.name_text(frame.package, *name)),
                };
                match self.find_function(obj, &fname, false) {
                    Some(f) => self.call_with_exprs(f, obj, params, frame)?,
                    None => Value::Void,
                }
            }
            NativeFunction(index, params) => {
                let f = *self.native_index.get(index).ok_or_else(|| VmError::UnknownFunction(format!("native({index})")))?;
                // && and ||: the second operand (preceded by Skip) is only evaluated if needed.
                if params.len() == 2 && matches!(params[1].kind, Skip(..)) {
                    let name = &self.functions[f].name;
                    let short = if name.eq_ignore_ascii_case("AndAnd_BoolBool") {
                        Some(false)
                    } else if name.eq_ignore_ascii_case("OrOr_BoolBool") {
                        Some(true)
                    } else {
                        None
                    };
                    if let Some(stop) = short {
                        let first = self.eval(&params[0], frame)?.as_bool();
                        return Ok(Value::Bool(if first == stop { stop } else { self.eval(&params[1], frame)?.as_bool() }));
                    }
                }
                self.call_at(e.end, f, target, params, frame)?
            }
            EatString(x) => {
                self.eval(x, frame)?;
                Value::Void
            }
            other => return Err(VmError::Unsupported(format!("expression {:?}", std::mem::discriminant(other)))),
        })
    }

    /// Zero value returned by an access through a null context.
    /// Property `name` of data object `path` ("Package.Group.Object"), read in its package.
    pub fn asset_prop(&mut self, path: &str, name: &str) -> Option<Value> {
        // Local Terrain PostLoad creates an actual G16 object from legacy words.
        // Keep its reference observable through the same asset-property access as disk textures.
        if let Some(map) = self.terrains.iter().filter_map(|t| t.generated_map.as_ref()).find_map(|(p, map)| p.eq_ignore_ascii_case(path).then_some(map)) {
            if name.eq_ignore_ascii_case("Format") { return Some(Value::Byte(map.format)); }
            if name.eq_ignore_ascii_case("USize") { return Some(Value::Int(map.width)); }
            if name.eq_ignore_ascii_case("VSize") { return Some(Value::Int(map.height)); }
        }
        // Mods build: the size of a mod's own texture ("Mod.<Name>", a PNG of the mod), which no
        // package holds (scripts that draw a texture whole, such as DrawTraceBullet, read it).
        #[cfg(feature = "mods")]
        if path.len() > 4 && path[..4].eq_ignore_ascii_case("Mod.") && (name.eq_ignore_ascii_case("USize") || name.eq_ignore_ascii_case("VSize")) {
            if let Some((w, h)) = openxiii_assets::mod_textures::get(path).and_then(|m| m.first().map(|l| (l.0, l.1))) {
                return Some(Value::Int(if name.eq_ignore_ascii_case("USize") { w as i32 } else { h as i32 }));
            }
        }
        let key = (path.to_lowercase(), name.to_lowercase());
        if let Some(v) = self.asset_cache.get(&key) {
            return Some(v.clone());
        }
        let library = self.library.as_mut()?;
        let (loaded, index) = library.resolve(path)?;
        let data = loaded.package.object_data(&loaded.data, index).ok()?;
        use openxiii_package::Value as P;
        let value = match data.get(&loaded.package, name)? {
            P::Byte(b) => Value::Byte(*b),
            P::Int(i) => Value::Int(*i),
            P::Bool(b) => Value::Bool(*b),
            P::Float(f) => Value::Float(*f),
            P::Name(n) => Value::Name(crate::value::NameStruct::from(none_name(loaded.package.name(*n)))),
            P::Str(s) => Value::Str(s.clone()),
            P::Vector(x) => Value::vector(x[0], x[1], x[2]),
            P::Object(r) => match ObjectRef::from(*r) {
                ObjectRef::None => Value::Object(None),
                ObjectRef::Export(_) => Value::Asset(format!("{}.{}", loaded.name, loaded.package.object_path((*r).into()))),
                _ => Value::Asset(loaded.package.object_path((*r).into())),
            },
            _ => return None,
        };
        self.asset_cache.insert(key, value.clone());
        Some(value)
    }

    /// Property `name` of the default object of a data object's class (`path`).
    fn asset_class_default(&mut self, path: &str, name: &str) -> Option<Value> {
        let class = {
            let library = self.library.as_mut()?;
            let (loaded, index) = library.resolve(path)?;
            loaded.package.export_class(index).to_string()
        };
        let c = self.class(&class)?;
        let cdo = self.classes[c].cdo;
        self.prop_slot(cdo, name).ok()?;
        Some(self.propr(cdo, name).clone())
    }

    /// Type and static dimension of the variable designated by a member expression (for the
    /// zero value of an access through a null context: execContext zeroes Result over the
    /// size of the member's type, whatever its access: variable, array element, field…).
    fn member_type(&mut self, e: &Expr, frame: &Frame) -> Option<(Type, usize)> {
        match &e.kind {
            Kind::BoolVariable(_) => Some((Type::Bool, 1)),
            Kind::InstanceVariable(r) | Kind::DefaultVariable(r) => {
                let name = self.ref_name(frame.package, *r).to_lowercase();
                self.classes.iter().find_map(|c| c.prop_index.get(&name).map(|&i| (c.props[i].ty.clone(), c.props[i].dim)))
            }
            Kind::LocalVariable(r) => {
                let name = self.ref_name(frame.package, *r).to_lowercase();
                let f = frame.function?;
                let i = self.functions[f].local_index.get(&name).copied()?;
                let d = &self.functions[f].locals[i];
                Some((d.ty.clone(), d.dim))
            }
            Kind::Skip(_, x) => self.member_type(x, frame),
            Kind::Context(_, _, _, m) | Kind::ClassContext(_, _, _, m) => self.member_type(m, frame),
            Kind::ArrayElement(_, arr) | Kind::DynArrayElement(_, arr) => match self.member_type(arr, frame)? {
                (t, dim) if dim > 1 => Some((t, 1)),
                (Type::Array(inner), _) => Some((*inner, 1)),
                _ => None,
            },
            Kind::StructMember(r, x) => {
                let (t, _) = self.member_type(x, frame)?;
                let Type::Struct(name) = t else { return None };
                let field = self.ref_name(frame.package, *r).to_lowercase();
                let def = self.struct_def(&name)?;
                let i = def.index.get(&field).copied()?;
                Some((def.fields[i].ty.clone(), def.fields[i].dim))
            }
            Kind::DynArrayLength(_) => Some((Type::Int, 1)),
            _ => None,
        }
    }

    fn default_for(&mut self, member: &Expr, frame: &Frame) -> Value {
        match &member.kind {
            Kind::BoolVariable(_) => Value::Bool(false),
            Kind::InstanceVariable(_) | Kind::DefaultVariable(_) | Kind::LocalVariable(_) | Kind::ArrayElement(..) | Kind::DynArrayElement(..) | Kind::StructMember(..) | Kind::DynArrayLength(_) | Kind::Context(..) | Kind::ClassContext(..) | Kind::Skip(..) => {
                match self.member_type(member, frame) {
                    Some((t, dim)) => self.zero(&t, dim),
                    None => Value::Void,
                }
            }
            // Call on a null context: execContext zeroes the result over the size of the return
            // type (null object, and not Void: `A.Arme() == None` is true).
            Kind::VirtualFunction(r, _) | Kind::GlobalFunction(r, _) => {
                let name = self.name_text(frame.package, *r);
                let ret = self.functions.iter().find(|f| f.name.eq_ignore_ascii_case(&name) && f.ret.is_some()).and_then(|f| f.ret.clone());
                ret.map_or(Value::Void, |t| self.zero(&t, 1))
            }
            Kind::FinalFunction(r, _) => {
                let path = self.packages[frame.package].package.object_path((*r).into());
                let ret = self.function_by_path(&path).and_then(|f| self.functions[f].ret.clone());
                ret.map_or(Value::Void, |t| self.zero(&t, 1))
            }
            _ => Value::Void,
        }
    }

    fn object_const(&mut self, package: usize, r: i32) -> Value {
        if r == 0 {
            return Value::Object(None);
        }
        let p = &self.packages[package].package;
        let is_class_ref = match ObjectRef::from(r) {
            ObjectRef::Import(i) => p.name(p.imports[i].class_name).eq_ignore_ascii_case("Class"),
            ObjectRef::Export(i) => p.export_class(i).eq_ignore_ascii_case("Class"),
            ObjectRef::None => false,
        };
        if is_class_ref {
            let c = self.class_of_ref(package, r);
            // The requested class of a following DynamicLoadObject: its name when the VM does not have that class.
            self.meta_const = if c.is_none() { Some(self.packages[package].package.object_name(r.into()).to_string()) } else { None };
            return Value::Class(c);
        }
        let path = match ObjectRef::from(r) {
            ObjectRef::Export(_) => format!("{}.{}", self.packages[package].name, p.object_path(r.into())),
            _ => p.object_path(r.into()),
        };
        // Script object of a package (default subobject): the loaded object.
        if let Some(id) = self.script_object(&path) {
            return Value::Object(Some(id));
        }
        Value::Asset(path)
    }

    fn convert(&mut self, code: u8, v: Value) -> Value {
        match code {
            // RotatorToVector: FRotator::Vector (GMath's sine table).
            0x39 => {
                let d = crate::ai_move::rotator_vector(v.as_rotator());
                Value::vector(d[0], d[1], d[2])
            }
            // execFloatToInt and execFloatToByte: _ftol (
            // 64-bit truncation), then the low 32 or 8 bits.
            0x44 => Value::Int(crate::value::ftol(v.as_float()) as i32),
            0x43 => Value::Byte(crate::value::ftol(v.as_float()) as u8),
            // execStringToInt, execStringToByte: appAtoi;
            // execStringToFloat: appAtof.
            0x4A => Value::Int(crate::config::atoi(&v.as_name().chars().collect::<Vec<_>>(), 0)),
            0x49 => Value::Byte(crate::config::atoi(&v.as_name().chars().collect::<Vec<_>>(), 0) as u8),
            0x4C => Value::Float(crate::config::atof(&v.as_name().chars().collect::<Vec<_>>(), 0)),
            0x3A | 0x41 => Value::Int(v.as_int()),
            // execRotatorToBool: a component whose low 16 bits are not zero.
            0x51 => Value::Bool(v.as_rotator().iter().any(|c| c & 0xffff != 0)),
            0x3B | 0x3E | 0x45 | 0x47 | 0x48 => Value::Bool(v.as_bool()),
            // execStringToBool: "True" or GTrue, "False" or GFalse (case-insensitive;
            // Core.frt keeps True and False), otherwise appAtoi != 0.
            0x4B => Value::Bool({
                let s = v.as_name();
                if s.eq_ignore_ascii_case("True") {
                    true
                } else if s.eq_ignore_ascii_case("False") {
                    false
                } else {
                    crate::config::atoi(&s.chars().collect::<Vec<_>>(), 0) != 0
                }
            }),
            // execVectorToBool: a component other than 0.
            0x4F => Value::Bool(v.as_vector() != [0.0; 3]),
            0x3C | 0x3F | 0x42 => Value::Float(v.as_float()),
            0x3D | 0x40 => Value::Byte(v.as_byte()),
            // UObject::execFloatToString and execVectorToString:
            // FString::Printf("%f") and "%f,%f,%f" (msvcr70's _vsnprintf).
            0x55 => Value::Str(crate::value::printf_f(v.as_float())),
            0x58 => {
                let [x, y, z] = v.as_vector();
                Value::Str(format!("{},{},{}", crate::value::printf_f(x), crate::value::printf_f(y), crate::value::printf_f(z)))
            }
            // execRotatorToString: "%i,%i,%i" of the components & 0xFFFF.
            0x59 => {
                let [p, y, r] = v.as_rotator();
                Value::Str(format!("{},{},{}", p & 0xffff, y & 0xffff, r & 0xffff))
            }
            0x52..=0x59 => Value::Str(match &v {
                // UObject::execObjectToString: GetPathName; a class is an object
                // of its package ("XIIIMP.HunterPlayer").
                Value::Object(Some(o)) => self.path_name(*o),
                Value::Class(Some(c)) => format!("{}.{}", self.packages[self.classes[*c].package].name, self.classes[*c].name),
                other => other.to_string(),
            }),
            0x5A => Value::Name(crate::value::NameStruct::from(v.as_name())),
            // execStringToVector and execStringToRotator: appAtof /
            // appAtoi of the start, then after each "," found by appStrstr.
            0x4D | 0x4E => {
                let chars: Vec<char> = v.as_name().chars().collect();
                let first = chars.iter().position(|&c| c == ',');
                let second = first.and_then(|i| chars[i + 1..].iter().position(|&c| c == ',').map(|j| i + 1 + j));
                if code == 0x4D {
                    let f = |p: Option<usize>| p.map_or(0.0, |p| crate::config::atof(&chars, p));
                    Value::vector(f(Some(0)), f(first.map(|i| i + 1)), f(second.map(|i| i + 1)))
                } else {
                    let f = |p: Option<usize>| p.map_or(0, |p| crate::config::atoi(&chars, p));
                    Value::rotator(f(Some(0)), f(first.map(|i| i + 1)), f(second.map(|i| i + 1)))
                }
            }
            // VectorToRotator: FVector::Rotation.
            0x50 => {
                let r = crate::physics::vector_rotation(v.as_vector());
                Value::rotator(r[0], r[1], r[2])
            }
            _ => v,
        }
    }

    // ---- Calls ----

    /// Call of a function from the bytecode, `end` being the end of the call expression.
    fn call_at(&mut self, end: u32, f: FuncId, this: ObjId, params: &[Expr], frame: &mut Frame) -> Result<Value> {
        self.call_ends.push(end);
        let r = self.call_with_exprs(f, this, params, frame);
        self.call_ends.pop();
        r
    }

    fn call_with_exprs(&mut self, f: FuncId, this: ObjId, params: &[Expr], frame: &mut Frame) -> Result<Value> {
        let prep = self.call_prep(f);
        // Resolved operator (natives::fast_operator): operands evaluated in order, then the same
        // computation as call_inner → natives::operator, without a frame or lists. The call tracking
        // tools (traces, depth, native counts) keep the full path.
        if let Some(op) = prep.fast {
            let tracking = self.depth > 250
                || self.native_counts.is_some()

               ;
            let gives = |i: usize| params.get(i).is_some_and(|e| !matches!(e.kind, Kind::Nothing));
            let v = match op {
                natives::OpFast::Un(calc) if !tracking && gives(0) => Some(calc(&self.eval(&params[0], frame)?)),
                natives::OpFast::Bin(calc) if !tracking && gives(0) && gives(1) => {
                    let a = self.eval(&params[0], frame)?;
                    let b = self.eval(&params[1], frame)?;
                    Some(calc(&a, &b))
                }
                _ => None,
            };
            if let Some(v) = v {
                tally::inc(tally::CALLS);
                // As after call_inner for a native: result written, no output to copy back.
                self.return_nothing = false;
                self.last_outputs.clear();
                return Ok(v);
            }
        }
        let defs = prep.params.clone();
        let follow = prep.has_out;
        // NetDirty on the destination of an out parameter: CallFunction for a script function; for a native, the
        // only ones that call it (compound operators, ++, --, GetAxes, GetUnAxes, Invert).
        let nd_out = {
            let d = &self.functions[f];
            let native = (d.native != 0 || d.flags & openxiii_script::objects::func::NATIVE != 0) && d.code.is_none();
            !native || net_dirty_native(&d.name)
        };
        let mut args = self.pool_args.pop().unwrap_or_default();
        args.clear();
        let mut places = if follow { self.pool_places.pop().unwrap_or_default() } else { Vec::new() };
        places.clear();
        for (i, def) in defs.iter().enumerate() {
            let expr = params.get(i);
            let omitted = expr.is_none_or(|e| matches!(e.kind, Kind::Nothing));
            if omitted {
                args.push(None);
                if follow {
                    places.push(None);
                }
                continue;
            }
            let expr = expr.unwrap();
            if def.out {
                // Out parameter receiving a non-writable value: plain value, without copy-back.
                // CallFunction and P_GET_*_REF evaluate the parameter into a buffer (non-null Result):
                // a dynamic array element outside the array is read there (warning, zero,
                // null GPropAddr), never grown; without an address, nothing is copied back after the call.
                let this = frame.this;
                let writing = std::mem::replace(&mut self.writing, false);
                let place = self.place_probed(|vm, frame| vm.place_in(expr, frame, this), frame);
                self.writing = writing;
                match place {
                    Ok(place) => match {
                        if nd_out {
                            self.net_dirty_place(&place);
                        }
                        (&place, self.slot_ref(&frame.locals, &place))
                    } {
                        (Place::Length(_), _) => {
                            args.push(Some(self.get(&frame.locals, &place)));
                            places.push(Some(place));
                        }
                        (_, Some(v)) => {
                            args.push(Some(v.into_owned()));
                            places.push(Some(place));
                        }
                        (_, None) => {
                            args.push(Some(self.default_for(expr, frame)));
                            places.push(None);
                        }
                    },
                    Err(VmError::Unsupported(_)) => {
                        args.push(Some(self.eval(expr, frame)?));
                        places.push(None);
                    }
                    Err(e) => return Err(e),
                }
            } else {
                args.push(Some(self.eval(expr, frame)?));
                if follow {
                    places.push(None);
                }
            }
        }
        // A native receives its out parameters by reference (P_GET_*_REF: GPropAddr) and only reads them
        // once all the parameters have been evaluated: `A += A++` adds the already incremented A. A
        // script function copies them into its frame parameter by parameter (CallFunction).
        let d = &self.functions[f];
        if (d.native != 0 || d.flags & openxiii_script::objects::func::NATIVE != 0) && d.code.is_none() {
            for (arg, place) in args.iter_mut().zip(&places) {
                if let (Some(a), Some(p)) = (arg.as_mut(), place) {
                    if let Some(v) = self.slot_ref(&frame.locals, p) {
                        *a = v.into_owned();
                    }
                }
            }
        }
        let (result, mut outs) = self.call(f, this, args)?;
        // CallFunction copies the out parameters back from the last to the first: if two
        // parameters overlap, the first one has the last word.
        let mut copied = std::mem::take(&mut self.last_outputs);
        copied.clear();
        for (place, value) in places.drain(..).zip(outs.drain(..)).rev() {
            if let (Some(place), Some(value)) = (place, value) {
                self.set(frame.function, &mut frame.locals, &place, value.clone());
                copied.push((place, value));
            }
        }
        self.last_outputs = copied;
        // Lists returned for the next calls.
        if follow && self.pool_places.len() < 64 {
            self.pool_places.push(places);
            self.pool_outs.push(outs);
        }
        Ok(result)
    }

    /// Preparation of the call of `f`: zero values of its locals (Vm::zero of each type, as
    /// the call created them), parameters, string locals and name. Redone if a struct has been loaded
    /// or declared since (a struct still unknown gave a zero without fields).
    fn call_prep(&mut self, f: FuncId) -> Rc<CallPrep> {
        let generation = self.structs.len() + self.struct_exports.len();
        if let Some(Some(p)) = self.call_preps.get(f) {
            if p.generation == generation && p.name.as_ref() == self.functions[f].name {
                return p.clone();
            }
        }
        let defs: Vec<(Type, usize)> = self.functions[f].locals.iter().map(|l| (l.ty.clone(), l.dim)).collect();
        let zeros: Vec<Value> = defs.iter().map(|(t, d)| self.zero(t, *d)).collect();
        let d = &self.functions[f];
        let operator = d.name.contains('_') && self.classes[d.owner].name.eq_ignore_ascii_case("Object");
        // Same conditions as call_inner's native path (native with iNative: never
        // sent by ProcessRemoteFunction), not singular, no out parameter or string coercion.
        // The untyped Not_Pre spelling historically takes the full call path.
        let fast = (operator
            && d.native != 0
            && d.code.as_ref().is_none_or(|c| c.statements.iter().all(|s| matches!(s.kind, Kind::NativeParm(_) | Kind::Nothing)))
            && d.flags & openxiii_script::objects::func::SINGULAR == 0
            && d.name != "Not_Pre"
            && !d.params.iter().any(|p| p.out || p.coerce))
            .then(|| natives::fast_operator(&d.name, d.params.len()))
            .flatten();
        let prep = Rc::new(CallPrep {
            zeros,
            params: d.params.clone().into(),
            str_locals: defs.iter().map(|(t, _)| *t == Type::Str).collect(),
            name: Rc::from(d.name.as_str()),
            operator,
            dispatch: natives::dispatch(&d.name),
            fast,
            find_label: d.name == "FindLabel" && self.find_label_original(f),
            has_out: d.params.iter().any(|p| p.out),
            probe: crate::world::probe_bit(&d.name),
            others: (0..defs.len()).filter(|i| !d.params.iter().any(|p| p.local == *i)).collect(),
            // After the zeros: Vm::zero may have loaded the structs of the locals.
            generation: self.structs.len() + self.struct_exports.len(),
        });
        if self.call_preps.len() <= f {
            self.call_preps.resize(f + 1, None);
        }
        self.call_preps[f] = Some(prep.clone());
        prep
    }

    fn call_inner(&mut self, f: FuncId, this: ObjId, args: Vec<Option<Value>>) -> Result<(Value, Vec<Option<Value>>)> {
        tally::inc(tally::CALLS);
        if self.depth > 250 {
            let trace: Vec<String> =
                self.stack.iter().rev().take(24).map(|&f| format!("{}.{}", self.classes[self.functions[f].owner].name, self.functions[f].name)).collect();
            self.warn(format!("recursion too deep: {}", trace.join(" < ")));
            return Err(VmError::TooDeep);
        }
        let prep = self.call_prep(f);
        let params = prep.params.clone();
        let (code, native, flags, package) = {
            let d = &self.functions[f];
            (d.code.clone(), d.native, d.flags, d.package)
        };
        // Lists reused from a previous call (without a new allocation), returned at the end.
        // Zero locals (Vm::zero) except the received parameters: their zero is not copied only to
        // be replaced at once (a struct's zero is an allocated list).
        let mut locals: Vec<Value> = self.pool_values.pop().unwrap_or_default();
        locals.clear();
        locals.resize(prep.zeros.len(), Value::Void);
        for &i in prep.others.iter() {
            locals[i] = prep.zeros[i].clone();
        }
        let mut supplied = self.pool_bools.pop().unwrap_or_default();
        supplied.clear();
        let mut args = args;
        let keep_value = false;
        for (i, p) in params.iter().enumerate() {
            let given = if keep_value { args.get(i).cloned().flatten() } else { args.get_mut(i).and_then(|a| a.take()) };
            supplied.push(given.is_some());
            if let Some(mut v) = given {
                if p.coerce && prep.str_locals[p.local] && !matches!(v, Value::Str(_)) {
                    v = Value::Str(match &v {
                        Value::Object(Some(o)) => self.object(*o).name.clone(),
                        other => other.to_string(),
                    });
                }
                locals[p.local] = v;
            } else {
                locals[p.local] = prep.zeros[p.local].clone();
            }
        }
        let is_native = native != 0 || flags & openxiii_script::objects::func::NATIVE != 0;
        // AActor::ProcessRemoteFunction: script function (ProcessInternal) or
        // native without iNative (CallFunction), only on the network; sent or absorbed, it does not
        // run here.
        if self.ipdrv.net_driver.is_some() && (native == 0 || !is_native) && self.net_process_remote_function(f, this, &locals) {
            let ret = self.functions[f].ret.clone();
            let outs = if prep.has_out { params.iter().map(|p| if p.out { Some(locals[p.local].clone()) } else { None }).collect() } else { Vec::new() };
            return Ok((ret.map(|t| self.zero(&t, 1)).unwrap_or(Value::Void), outs));
        }
        // UObject::ProcessInternal: a script function whose name is a probe
        // event (names 300 to 363) does not run if the ProbeMask of the object's state frame does not have
        // its bit (the state's `ignores`, Disable); also for a direct call from the script (XIIIDispatcher's
        // Dispatch ignores UnTrigger in Banque01).
        if !is_native && code.is_some() {
            let ignore = prep.probe.is_some_and(|bit| self.probe_blocked_bit(this, bit));
            if ignore {
                let ret = self.functions[f].ret.clone();
                let outs = if prep.has_out { params.iter().map(|p| if p.out { Some(locals[p.local].clone()) } else { None }).collect() } else { Vec::new() };
                return Ok((ret.map(|t| self.zero(&t, 1)).unwrap_or(Value::Void), outs));
            }
        }
        // Mods build: a script function a mod turns off (mod_weapons.rs, ImpactEffectsBySurface).
        #[cfg(feature = "mods")]
        if !is_native && self.mod_skip_function(f, this) {
            let ret = self.functions[f].ret.clone();
            let outs = if prep.has_out { params.iter().map(|p| if p.out { Some(locals[p.local].clone()) } else { None }).collect() } else { Vec::new() };
            return Ok((ret.map(|t| self.zero(&t, 1)).unwrap_or(Value::Void), outs));
        }
        self.depth += 1;
        self.stack.push(f);
        self.this_stack.push(this);
        let result = if is_native && code.as_ref().is_none_or(|c| c.statements.iter().all(|s| matches!(s.kind, Kind::NativeParm(_) | Kind::Nothing))) {
            let name = prep.name.clone();
            let mut values: Vec<Value> = self.pool_values.pop().unwrap_or_default();
            values.clear();
            values.extend(params.iter().map(|p| std::mem::replace(&mut locals[p.local], Value::Void)));
            if let Some(counts) = self.native_counts.as_mut() {
                let owner = &self.classes[self.functions[f].owner].name;
                *counts.entry(format!("{owner}.{name}")).or_default() += 1;
            }
            let direct = if prep.operator { natives::operator(&name, &mut values) } else { None };
            let dispatch = prep.dispatch;
            let r = match direct.unwrap_or_else(|| natives::call_dispatches(self, &name, dispatch, this, &mut values, &supplied)) {
                Err(VmError::Native(message)) => {
                    // Missing native: counted, and the zero value of the return type.
                    *self.missing_natives.entry(name.to_string()).or_default() += 1;
                    let _ = message;
                    let ret = self.functions[f].ret.clone();
                    Ok(ret.map(|t| self.zero(&t, 1)).unwrap_or(Value::Void))
                }
                other => other,
            };
            for (p, v) in params.iter().zip(values.drain(..)) {
                locals[p.local] = v;
            }
            if self.pool_values.len() < 64 {
                self.pool_values.push(values);
            }
            // A native always writes its result.
            self.return_nothing = false;
            r
        } else if let Some(line) = prep.find_label.then(|| self.find_label_direct(this, package, &locals[params[0].local])).flatten() {
            self.return_nothing = false;
            Ok(Value::Int(line))
        } else if let Some(code) = code {
            let mut frame = Frame { this, function: Some(f), locals, package, switch: None, iterators: Vec::new() };
            // A function called from state code cannot be suspended.
            let state_code = std::mem::replace(&mut self.in_state_code, false);
            let r = self.run(&code, &mut frame, 0).map(|flow| match flow {
                Flow::Return(v) => v,
                Flow::Stop | Flow::Suspend(_) => Value::Void,
            });
            self.in_state_code = state_code;
            locals = frame.locals;
            // UObject::ProcessInternal: the expression that follows EX_Return is evaluated
            // into the caller's result buffer; EX_Nothing (end of function, "return;")
            // writes nothing there. Outside an assignment this buffer is a zero temporary (P_GET_*),
            // hence the zero value of the type; execLet keeps the destination (return_nothing).
            let ret = self.functions[f].ret.clone();
            match (r, ret) {
                (Ok(Value::Void), Some(t)) => {
                    self.return_nothing = true;
                    Ok(self.zero(&t, 1))
                }
                (r, _) => {
                    self.return_nothing = false;
                    r
                }
            }
        } else {
            Ok(Value::Void)
        };
        self.depth -= 1;
        // Lockstep co-op: the function that projected a HUD icon's world point returns.
        if self.hud_anchor_depth == Some(self.stack.len()) {
            self.hud_anchor_close();
        }
        self.stack.pop();
        self.this_stack.pop();
        let outs = if prep.has_out {
            let mut outs = self.pool_outs.pop().unwrap_or_default();
            outs.clear();
            outs.extend(params.iter().map(|p| if p.out { Some(locals[p.local].clone()) } else { None }));
            outs
        } else {
            Vec::new()
        };
        if self.pool_values.len() < 64 {
            locals.clear();
            self.pool_values.push(locals);
            self.pool_bools.push(supplied);
            args.clear();
            self.pool_args.push(args);
        }
        Ok((result?, outs))
    }

    /// Call by name with values (external entry point).
    pub fn call_named(&mut self, this: ObjId, name: &str, args: Vec<Value>) -> Result<Value> {
        let f = self.find_function(this, name, false).ok_or_else(|| VmError::UnknownFunction(name.to_string()))?;
        Ok(self.call(f, this, args.into_iter().map(Some).collect())?.0)
    }

    fn set_iteration(&mut self, frame: &mut Frame, place: &Place, extras: &[Place], v: Value) {
        match v {
            Value::Array(parts) if !extras.is_empty() => {
                let mut parts = parts.into_iter();
                if let Some(main) = parts.next() {
                    self.set(frame.function, &mut frame.locals, place, main);
                }
                for (extra, value) in extras.iter().zip(parts.by_ref()) {
                    self.set(frame.function, &mut frame.locals, extra, value);
                }
                // TraceActors with TRACETYPE_RequestBones: the contact's bone.
                if let Some(Value::Name(bone)) = parts.next() {
                    self.last_trace_bone = String::from(bone);
                }
            }
            // Element without a contact of the class (TraceActors): the loop body runs, the
            // out variables keep their value.
            Value::Void => {}
            v => self.set(frame.function, &mut frame.locals, place, v),
        }
    }

    pub(crate) fn run(&mut self, code: &Rc<Code>, frame: &mut Frame, start: usize) -> Result<Flow> {
        let mut pc = start;
        let jump = |pc: &mut usize, target: u16| -> Result<()> {
            *pc = *code.at.get(&(target as u32)).ok_or_else(|| VmError::Unsupported(format!("jump to {target}")))?;
            Ok(())
        };
        while pc < code.statements.len() {
            let e = &code.statements[pc];
            pc += 1;
            self.cur_offset = e.offset;
            if let Some(f) = &self.trace_steps {
                if self.current_function().eq_ignore_ascii_case(f) {
                    let line = format!("STEP {} @{} {:?}", f, e.offset, frame.locals);
                    self.log.push(line);
                }
            }
            match &e.kind {
                Kind::Return(x) => return Ok(Flow::Return(self.eval(x, frame)?)),
                Kind::Stop => return Ok(Flow::Stop),
                Kind::Jump(t) => {
                    // The original logs with Stack.Code after the opcode (offset + 1).
                    self.runaway_step(e.offset + 1)?;
                    jump(&mut pc, *t)?
                }
                Kind::JumpIfNot(t, cond) => {
                    self.runaway_step(e.offset + 1)?;
                    // The condition is evaluated only once, traced or not: evaluating its
                    // arguments again for the trace drew FRand twice (FRand() < 0.5), redid
                    // the effects of the calls and out parameters, and bypassed short-circuiting.
                    let true_value = self.eval(cond, frame)?.as_bool();
                    if self.trace_steps.as_ref().is_some_and(|f| self.current_function().eq_ignore_ascii_case(f)) {
                        self.log.push(format!("COND @{} {true_value}", e.offset));
                    }
                    if !true_value {
                        jump(&mut pc, *t)?;
                    }
                }
                Kind::Switch(_, x) => {
                    // UObject::execSwitch: walks the chain of `case`s up to the first one that
                    // matches (or `default`) and resumes right after it.
                    let value = self.eval(x, frame)?;
                    while let Some(Kind::Case(next, case)) = code.statements.get(pc).map(|s| &s.kind) {
                        match case {
                            Some(c) => {
                                let c = self.eval(c, frame)?;
                                if values_equal(&value, &c) {
                                    pc += 1;
                                    break;
                                }
                                jump(&mut pc, *next)?;
                            }
                            None => {
                                pc += 1;
                                break;
                            }
                        }
                    }
                    frame.switch = Some(value);
                }
                // UObject::execCase reached in sequence (previous case without break): skipped.
                // UObject::execCase: the case's expression is evaluated into a buffer,
                // the result discarded (its warnings, rand() draws and effects happen);
                // the default (0xFFFF) has none.
                Kind::Case(_, Some(x)) => {
                    self.eval(x, frame)?;
                }
                Kind::Case(..) => {}
                // UObject::execAssert: Stack.Logf(NAME_Critical, "Assertion failed,
                // line %i"), fatal error (FFrame::Serialize → GError); here the call stops.
                Kind::Assert(line, x) => {
                    if !self.eval(x, frame)?.as_bool() {
                        let message = format!("Assertion failed, line {line}");
                        self.script_critical(e.end, &message);
                        return Err(VmError::Unsupported(message));
                    }
                }
                Kind::LabelTable(_) | Kind::Nothing | Kind::EndFunctionParms => {}
                Kind::Iterator(call, end) => {
                    let (items, place, extras, mut lazy) = self.iterator_items(call, frame)?;
                    let first = match lazy.as_mut() {
                        Some(it) => self.lazy_next(it).map(|o| Value::Object(Some(o))),
                        None => items.first().cloned(),
                    };
                    frame.iterators.push(IteratorState { items, index: 0, place: place.clone(), extras: extras.clone(), body: pc, lazy });
                    match first {
                        Some(v) => self.set_iteration(frame, &place, &extras, v),
                        None => {
                            // Each step of Engine's iterators starts with *OutActor = NULL: the
                            // variable is None when the list is exhausted (e.g.).
                            self.set(frame.function, &mut frame.locals, &place, Value::Object(None));
                            jump(&mut pc, *end)?;
                        }
                    }
                }
                Kind::IteratorNext => {
                    let mut lazy = frame.iterators.last_mut().and_then(|it| {
                        it.index += 1;
                        it.lazy.take()
                    });
                    let next = match lazy.as_mut() {
                        Some(l) => self.lazy_next(l).map(|o| Value::Object(Some(o))),
                        None => frame.iterators.last().and_then(|it| it.items.get(it.index).cloned()),
                    };
                    if let Some(it) = frame.iterators.last_mut() {
                        if lazy.is_some() {
                            it.lazy = lazy;
                        }
                        if let Some(v) = next {
                            let (place, extras, body) = (it.place.clone(), it.extras.clone(), it.body);
                            self.set_iteration(frame, &place, &extras, v);
                            pc = body;
                        } else {
                            let place = it.place.clone();
                            self.set(frame.function, &mut frame.locals, &place, Value::Object(None));
                        }
                    }
                }
                Kind::IteratorPop => {
                    frame.iterators.pop();
                }
                Kind::GotoLabel(x) => {
                    // execGotoLabel -> UObject::GotoLabel: the label may be in the code of a
                    // parent state; the code then changes (the state code resumes there).
                    let label = self.eval(x, frame)?.as_name();
                    let this = frame.this;
                    match self.state_label(this, &label) {
                        Some((target, i)) if Rc::ptr_eq(&target, code) => pc = i,
                        _ => {
                            // execGotoLabel: Stack.Logf if the label is missing.
                            if !self.goto_label(this, &label) {
                                self.script_warn(e.end, &format!("GotoLabel ({}): Label not found", if label.is_empty() { "None" } else { &label }));
                            }
                            // In state code, Stack is the StateFrame whose Code has just changed;
                            // in a function, execGotoLabel does not touch its own Stack.
                            if self.in_state_code {
                                return Ok(Flow::Stop);
                            }
                        }
                    }
                }
                _ => {
                    self.eval(e, frame)?;
                    if self.in_state_code {
                        if let Some(latent) = self.pending_latent.take() {
                            self.start_latent(frame.this, latent);
                        }
                        // GotoState from state code: this code is left.
                        if self.threads.get(&frame.this).is_some_and(|t| t.restart) {
                            return Ok(Flow::Stop);
                        }
                        // Latent action started (StateFrame->LatentAction): ProcessState
                        // stops after the statement.
                        if self.latent_action(frame.this) != 0 {
                            return Ok(Flow::Suspend(pc));
                        }
                    }
                }
            }
        }
        Ok(Flow::Return(Value::Void))
    }
}

/// Name from a name table in the VM: NAME_None ("None") becomes the empty name.
/// Null GPropAddr after evaluating a destination: null context, or element of a dynamic
/// array at a negative index (execDynArrayElement: GPropAddr = 0), and what depends
/// on it (element or field of a null place: null GProperty or GPropAddr, nothing is computed).
/// Same place (same variable, same element, same field).
fn place_eq(a: &Place, b: &Place) -> bool {
    match (a, b) {
        (Place::Local(i), Place::Local(j)) => i == j,
        (Place::Prop(o, s), Place::Prop(p, t)) => o == p && s == t,
        (Place::Element(x, i), Place::Element(y, j)) => i == j && *i != usize::MAX && place_eq(x, y),
        (Place::Field(x, i), Place::Field(y, j)) => i == j && place_eq(x, y),
        (Place::Length(x), Place::Length(y)) => place_eq(x, y),
        _ => false,
    }
}

/// A place is the same memory as the other or contains the other (struct and its field,
/// array and its element).
fn place_overlaps(a: &Place, b: &Place) -> bool {
    fn contains_list(a: &Place, b: &Place) -> bool {
        if place_eq(a, b) {
            return true;
        }
        match b {
            Place::Element(x, i) if *i != usize::MAX => contains_list(a, x),
            Place::Field(x, _) => contains_list(a, x),
            _ => false,
        }
    }
    contains_list(a, b) || contains_list(b, a)
}

fn place_is_null(place: &Place) -> bool {
    match place {
        Place::Nowhere => true,
        Place::Element(_, i) if *i == usize::MAX => true,
        Place::Element(p, _) | Place::Field(p, _) | Place::Length(p) => place_is_null(p),
        _ => false,
    }
}

pub(crate) fn none_name(n: &str) -> String {
    if n.eq_ignore_ascii_case("None") {
        String::new()
    } else {
        n.to_string()
    }
}

/// Equality for switch/case (numbers compared numerically, names case-insensitively).
/// Comparison of a `case` (UObject::execSwitch, Core.dll): strings compared with appStricmp
/// (case-insensitive), other types byte by byte.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Name(x), Value::Name(y)) => x.eq_ignore_ascii_case(y),
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        // appMemcmp on 4 bytes: 0 and −0 differ.
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        // 4-byte appMemcmp: NoObject (Object None) and a null class are the same null pointer.
        (Value::Object(None) | Value::Class(None), Value::Object(None) | Value::Class(None)) => true,
        (Value::Float(_), _) | (_, Value::Float(_)) => a.as_float() == b.as_float(),
        (Value::Int(_) | Value::Byte(_) | Value::Bool(_), Value::Int(_) | Value::Byte(_) | Value::Bool(_)) => a.as_int() == b.as_int(),
        _ => a == b,
    }
}

/// Element i of a read array (in place or made on the fly).
fn element_read(v: Cow<'_, Value>, i: usize) -> Option<Cow<'_, Value>> {
    match v {
        Cow::Borrowed(Value::Array(items)) => items.get(i).map(Cow::Borrowed),
        Cow::Owned(Value::Array(items)) => items.into_iter().nth(i).map(Cow::Owned),
        _ => None,
    }
}

/// Field i of a read struct: in place for a Struct, made on the fly for an inline vector /
/// rotator.
fn read_field(v: Cow<'_, Value>, i: usize) -> Option<Cow<'_, Value>> {
    match v {
        Cow::Borrowed(Value::Struct(_, fields)) => fields.get(i).map(Cow::Borrowed),
        Cow::Borrowed(v) => v.field_value(i).map(Cow::Owned),
        Cow::Owned(v) => v.field_value(i).map(Cow::Owned),
    }
}

/// Zero value of the same shape as an existing value.
fn zero_like(v: &Value) -> Value {
    match v {
        Value::Byte(_) => Value::Byte(0),
        Value::Int(_) => Value::Int(0),
        Value::Bool(_) => Value::Bool(false),
        Value::Float(_) => Value::Float(0.0),
        Value::Name(_) => Value::Name(crate::value::NameStruct::from(String::new())),
        Value::Str(_) => Value::Str(String::new()),
        Value::Object(_) | Value::Asset(_) => Value::Object(None),
        Value::Class(_) => Value::Class(None),
        Value::Struct(n, f) => Value::Struct(n.clone(), f.iter().map(zero_like).collect()),
        Value::Vector(_) => Value::Vector([0.0; 3]),
        Value::Rotator(_) => Value::Rotator([0; 3]),
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Delegate(..) => Value::Delegate(None, crate::value::NameStruct::EMPTY),
        Value::Void => Value::Void,
    }
}

/// UObject::execStructCmpEq / execStructCmpNe:
/// UStruct::StructCompare, Identical of each property of the struct (type name
/// ignored): UByteProperty, UIntProperty, UBoolProperty
/// (bit of the mask) and the others like `identical`.
fn struct_identical(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Struct(_, x), Value::Struct(_, y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| identical(p, q)),
        _ => match (a.struct_view(), b.struct_view()) {
            (Some((_, x)), Some((_, y))) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| identical(p, q)),
            _ => false,
        },
    }
}

/// UProperty::Identical of a property value (Core): UStrProperty through
/// appStricmp (_stricmp, case-insensitive); UNameProperty by FName index, so
/// case-insensitive, None and the empty name being the same; UFloatProperty by != (0 and −0
/// equal); UObjectProperty and UDelegateProperty by pointer;
/// UArrayProperty: same number of elements, then Identical of the Inner;
/// UStructProperty: field by field; UByteProperty, UIntProperty
/// and UBoolProperty by value.
pub(crate) fn identical(a: &Value, b: &Value) -> bool {
    fn name_key(n: &str) -> &str {
        if n.is_empty() {
            "None"
        } else {
            n
        }
    }
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        (Value::Name(x), Value::Name(y)) => name_key(x).eq_ignore_ascii_case(name_key(y)),
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Struct(..) | Value::Vector(_) | Value::Rotator(_), Value::Struct(..) | Value::Vector(_) | Value::Rotator(_)) => struct_identical(a, b),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| identical(p, q)),
        (Value::Asset(x), Value::Asset(y)) => x.eq_ignore_ascii_case(y),
        (Value::Object(None) | Value::Class(None), Value::Object(None) | Value::Class(None)) => true,
        (Value::Delegate(o, f), Value::Delegate(p, g)) => o == p && f.eq_ignore_ascii_case(g),
        _ => a == b,
    }
}

/// Entry of the prop_slot cache: class, requested name (bytes, in the entry) and slot.
#[derive(Clone, Copy)]
pub(crate) struct SlotEntry {
    class: ClassId,
    slot: u32,
    len: u8,
    bytes_value: [u8; 32],
}

impl SlotEntry {
    const EMPTY: SlotEntry = SlotEntry { class: usize::MAX, slot: 0, len: 0, bytes_value: [0; 32] };

    fn new(class: ClassId, name: &str, slot: usize) -> Option<SlotEntry> {
        let b = name.as_bytes();
        if b.len() > 32 || slot > u32::MAX as usize {
            return None;
        }
        let mut bytes_value = [0; 32];
        bytes_value[..b.len()].copy_from_slice(b);
        Some(SlotEntry { class, slot: slot as u32, len: b.len() as u8, bytes_value })
    }

    fn name_value(&self) -> Option<&[u8]> {
        (self.class != usize::MAX).then(|| &self.bytes_value[..self.len as usize])
    }
}

/// Entry of the is_a cache: object class, requested class name (bytes) and answer. Only
/// existing classes go into it (a still unknown name may be loaded later).
#[derive(Clone, Copy)]
pub(crate) struct IsAEntry {
    class: ClassId,
    response: bool,
    len: u8,
    bytes_value: [u8; 32],
}

pub(crate) const IS_A_CACHE: usize = 4096;

impl IsAEntry {
    pub(crate) const EMPTY: IsAEntry = IsAEntry { class: usize::MAX, response: false, len: 0, bytes_value: [0; 32] };

    pub(crate) fn case(class: ClassId, name: &str) -> usize {
        let h = (class.wrapping_mul(0x9E37_79B9) ^ (name.as_ptr() as usize >> 3) ^ name.len().wrapping_mul(0x85EB_CA6B)).wrapping_mul(0x2545_F491) >> 7;
        h & (IS_A_CACHE - 1)
    }

    pub(crate) fn new(class: ClassId, name: &str, response: bool) -> Option<IsAEntry> {
        let b = name.as_bytes();
        if b.len() > 32 {
            return None;
        }
        let mut bytes_value = [0; 32];
        bytes_value[..b.len()].copy_from_slice(b);
        Some(IsAEntry { class, response, len: b.len() as u8, bytes_value })
    }

    pub(crate) fn response(&self, class: ClassId, name: &str) -> Option<bool> {
        (self.class == class && &self.bytes_value[..self.len as usize] == name.as_bytes()).then_some(self.response)
    }
}

/// Entry of the find_function cache: object class, class and name of its state (absent for a
/// global call), requested name and function found (or none).
#[derive(Clone, Copy)]
pub(crate) struct FuncEntry {
    class: ClassId,
    state_class: ClassId,
    global: bool,
    to_state: bool,
    response: Option<FuncId>,
    /// Number of loaded functions and classes at the time of the answer (a function or a
    /// class added since makes it stale).
    generation: usize,
    len_state: u8,
    len_name: u8,
    state_value: [u8; 32],
    name_value: [u8; 32],
}

pub(crate) const FUNC_CACHE: usize = 4096;

impl FuncEntry {
    pub(crate) const EMPTY: FuncEntry =
        FuncEntry { class: usize::MAX, state_class: 0, global: false, to_state: false, response: None, generation: 0, len_state: 0, len_name: 0, state_value: [0; 32], name_value: [0; 32] };

    fn case(class: ClassId, name: &str, state_value: Option<&str>) -> usize {
        let e = state_value.map_or(0, |s| s.len().wrapping_mul(0x27D4_EB2F) ^ s.as_bytes().first().copied().unwrap_or(0) as usize);
        let h = (class.wrapping_mul(0x9E37_79B9) ^ (name.as_ptr() as usize >> 3) ^ name.len().wrapping_mul(0x85EB_CA6B) ^ e).wrapping_mul(0x2545_F491) >> 7;
        h & (FUNC_CACHE - 1)
    }

    fn new(class: ClassId, state_class: ClassId, state_value: Option<&str>, name: &str, global: bool, response: Option<FuncId>, generation: usize) -> Option<FuncEntry> {
        let (n, s) = (name.as_bytes(), state_value.unwrap_or("").as_bytes());
        if n.len() > 32 || s.len() > 32 {
            return None;
        }
        let mut e = FuncEntry { class, state_class, global, to_state: state_value.is_some(), response, generation, len_state: s.len() as u8, len_name: n.len() as u8, state_value: [0; 32], name_value: [0; 32] };
        e.name_value[..n.len()].copy_from_slice(n);
        e.state_value[..s.len()].copy_from_slice(s);
        Some(e)
    }

    fn response(&self, class: ClassId, state_class: ClassId, state_value: Option<&str>, name: &str, global: bool, generation: usize) -> Option<Option<FuncId>> {
        (self.class == class
            && self.generation == generation
            && self.state_class == state_class
            && self.global == global
            && self.to_state == state_value.is_some()
            && &self.name_value[..self.len_name as usize] == name.as_bytes()
            && &self.state_value[..self.len_state as usize] == state_value.unwrap_or("").as_bytes())
        .then_some(self.response)
    }
}

/// What call_inner prepares once per function (see `Vm::call_prep`).
pub(crate) struct CallPrep {
    zeros: Vec<Value>,
    params: Rc<[types::ParamDef]>,
    str_locals: Vec<bool>,
    name: Rc<str>,
    /// Object operator (natives::operator): called without going through the dispatchers.
    operator: bool,
    /// Dispatchers of natives::call that can handle this native (the generated dispatch table).
    dispatch: natives::NativeRoute,
    /// Operator without out parameters resolved once (natives::fast_operator): computed
    /// directly by call_with_exprs, without a call frame.
    fast: Option<natives::OpFast>,
    /// CineController2.FindLabel of the checked package.
    find_label: bool,
    /// At least one out parameter (otherwise the call returns no list of outputs).
    has_out: bool,
    /// Probe bit of the function's name (world::probe_bit), looked up once.
    probe: Option<u32>,
    /// Locals that are not parameters (reset to zero on each call).
    others: Vec<usize>,
    generation: usize,
}

/// `f(lowercase name)`: buffer on the stack for a short ASCII name (names of properties,
/// states, functions), otherwise `to_lowercase` (same result, without allocation in the common
/// case).
pub(crate) fn with_lower<R>(name: &str, f: impl FnOnce(&str) -> R) -> R {
    let b = name.as_bytes();
    if b.len() <= 64 && b.is_ascii() {
        if !b.iter().any(|c| c.is_ascii_uppercase()) {
            return f(name);
        }
        let mut buf = [0u8; 64];
        for (d, c) in buf.iter_mut().zip(b) {
            *d = c.to_ascii_lowercase();
        }
        // ASCII (checked above, and ASCII lowercase stays ASCII): always valid
        // UTF-8, without validating it again on each call.
        // SAFETY: `b` is ASCII, and `to_ascii_lowercase` of an ASCII byte is ASCII.
        return f(unsafe { std::str::from_utf8_unchecked(&buf[..b.len()]) });
    }
    f(&name.to_lowercase())
}

impl Vm {
    /// To be called after any modification of `actors`.
    pub(crate) fn actors_changed(&self) {
        *self.actor_member.borrow_mut() = None;
        self.particles.render_actors_valid.set(false);
    }
}

impl Vm {
    /// Debugging: (state, position of the state code, latent action) of an object.
    pub fn script_snapshot(&self, o: ObjId) -> (String, usize, u16) {
        let state = self.objects.get(o).and_then(|x| x.as_ref()).and_then(|x| x.state.clone()).unwrap_or_default();
        let (pc, latent) = self.threads.get(&o).map_or((0, 0), |t| (t.pc, t.latent));
        (state, pc, latent)
    }
}

#[path = "render/terrain_arrays.rs"]
mod terrain_arrays;

/// Script warning of UObject::execNew: `key_tuple` is the key of the Core.int error ([Errors]) when
/// creation is refused, `text_value` the localized message.
pub(crate) struct WarningNew {
    pub key_tuple: Option<&'static str>,
    pub text_value: String,
}

#[path = "script/classes_natives_table.rs"]
mod classes_natives_table;

pub(crate) const NAME_CRITICAL: u32 = 0x2F9;
pub(crate) const NAME_SCRIPT_WARNING: u32 = 0x301;

/// Device that FFrame::Serialize writes to: GLog with this EName, or GError (fatal error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameSink {
    Log(u32),
    Error,
}

/// FFrame::Serialize: "%s (%s:%04X) %s" with
/// Object->GetFullName(), Node->GetFullName(), Code - Node->Script.Data and the message, to
/// GLog->Logf(NAME_ScriptWarning) except for NAME_Critical or under GIsStrict (GError->Logf).
pub(crate) fn frame_serialize(event: u32, strict: bool, object: &str, node: &str, offset: u32, message: &str) -> (FrameSink, String) {
    let sink = if event != NAME_CRITICAL && !strict { FrameSink::Log(NAME_SCRIPT_WARNING) } else { FrameSink::Error };
    (sink, format!("{object} ({node}:{offset:04X}) {message}"))
}

#[inline(always)]
pub(crate) fn mutant(_name: &str) -> bool {
    false
}

/// Slots of `Vm::var_slot_direct` (power of two).
const VAR_SLOT_DIRECT: usize = 2048;

/// Entry of `Vm::var_slot_direct` (package u32::MAX: empty).
#[derive(Clone, Copy)]
struct VarSlotEntry {
    package: u32,
    r: i32,
    class: u32,
    slot: u32,
}

impl VarSlotEntry {
    const EMPTY: VarSlotEntry = VarSlotEntry { package: u32::MAX, r: 0, class: 0, slot: 0 };

    #[inline]
    fn case(package: usize, r: i32, class: usize) -> usize {
        let h = (package as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (r as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (class as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        (h >> 40) as usize & (VAR_SLOT_DIRECT - 1)
    }
}

/// Entry of `Vm::states_resolved`.
#[derive(Clone)]
pub(crate) struct StateResolved {
    state_value: Box<str>,
    state_class: Option<ClassId>,
    class_value: ClassId,
    nb_classes: usize,
    masks: Option<(u64, u64)>,
    /// Tick of the state (`Vm::tick_state`) and number of functions + classes at the time of the lookup.
    tick: Option<(usize, crate::tick::IdleEvent)>,
}

/// Key of `Vm::table_names_packages`: the map and the names of the loaded packages, in order.
pub(crate) type KeyNames = (Option<std::path::PathBuf>, Vec<String>);

/// Size of the prop_slot cache (power of 2).
const SLOT_CACHE: usize = 8192;

/// Entry of prop_slot's table of literal names: class, slot (SLOT_ABSENT: the
/// class does not have this property) and key of the name (address | length << 48; 0: empty).
#[derive(Clone, Copy)]
pub(crate) struct SlotLiteral {
    class: u32,
    slot: u32,
    key_tuple: u64,
}

impl SlotLiteral {
    const EMPTY: SlotLiteral = SlotLiteral { class: 0, slot: 0, key_tuple: 0 };
}

const SLOT_LITERALS: usize = 4096;
const SLOT_ABSENT: u32 = u32::MAX;

/// Key of a name whose text is in the executable's image (literal from the code, never freed or
/// rewritten): the same address and length always designate the same text. None otherwise
/// (allocated text: it may be freed and another string written at the same address).
#[inline]
fn key_literal(name: &str) -> Option<u64> {
    let p = name.as_ptr() as usize;
    let (start_value, end_value) = bounds_image();
    (p >= start_value && p + name.len() <= end_value && name.len() < 1 << 16 && (p as u64) < 1 << 48).then(|| p as u64 | (name.len() as u64) << 48)
}

#[inline]
fn slot_literal_case(class: ClassId, key_tuple: u64) -> usize {
    let c = class as u64;
    ((c.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ key_tuple.wrapping_mul(0xC2B2_AE3D_27D4_EB4F)) >> 52) as usize & (SLOT_LITERALS - 1)
}

/// Addresses of the executable's image (PE header of the main module), read once.
fn bounds_image() -> (usize, usize) {
    static B: std::sync::OnceLock<(usize, usize)> = std::sync::OnceLock::new();
    *B.get_or_init(|| {
        #[cfg(windows)]
        unsafe {
            #[link(name = "kernel32")]
            extern "system" {
                fn GetModuleHandleW(name_value: *const u16) -> *const u8;
            }
            let base = GetModuleHandleW(std::ptr::null());
            if base.is_null() {
                return (0, 0);
            }
            let lfanew = std::ptr::read_unaligned(base.add(0x3C) as *const u32) as usize;
            let nt = base.add(lfanew);
            if std::ptr::read_unaligned(nt as *const u32) != 0x4550 {
                return (0, 0);
            }
            // IMAGE_OPTIONAL_HEADER(64).SizeOfImage: optional header at +24, field at +56.
            let size_value = std::ptr::read_unaligned(nt.add(24 + 56) as *const u32) as usize;
            (base as usize, base as usize + size_value)
        }
        #[cfg(not(windows))]
        {
            (0, 0)
        }
    })
}

