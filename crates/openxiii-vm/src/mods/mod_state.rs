//! Per-VM state for installed mods and their active map.

use crate::value::{ClassId, ObjId};
use crate::{
    mod_behaviour, mod_classes, mod_defaults, mod_dialogue,
    mod_heat, mod_net, mod_net_fx, mod_net_skins,
    mod_own_sounds, mod_placement, mod_retexture, mod_script,
    mod_script_hud, mod_script_more, mod_sounds, mod_when,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

pub(crate) struct ModState {
    /// Class default values set by mods (mod_defaults.rs).
    pub(crate) defaults: Option<mod_defaults::ModDefaults>,
    /// FiringMode set by a mod on a weapon class, for its attachment class
    /// (mod_weapons.rs).
    pub(crate) attach_firing: HashMap<ClassId, openxiii_mods::DefaultOverride>,
    /// The checkpoint the level started from, set at the start of begin_play
    /// (mod_maps.rs), before a loaded checkpoint destroys save points (CleanMap).
    pub(crate) checkpoint: Option<u32>,
    /// Weapon slots (InventoryGroup) given to the mods' new weapon classes
    /// (mod_weapons.rs), slot → class.
    pub(crate) weapon_slots: BTreeMap<u8, ClassId>,
    /// The first-person model attached to each weapon (mod_weapons.rs).
    pub(crate) fp_models: HashMap<ObjId, ObjId>,
    /// Weapons whose first-person model was placed on their idle pose.
    pub(crate) fp_idle: HashSet<ObjId>,
    /// The front end of each static mesh asked (mod_weapons.rs mesh_front), by path.
    pub(crate) mesh_fronts: HashMap<String, Option<[f32; 3]>>,
    /// Where the beam of each tracer texture ends (mod_weapons.rs tracer_texture_end).
    pub(crate) tracer_ends: HashMap<String, [f32; 2]>,
    /// The scene of the view being drawn (its camera) and the view's size, for the tracer of a
    /// weapon held with a FirstPersonModel (mod_weapons.rs mod_tracer_size); None before a view.
    pub(crate) view_projection: Option<(crate::visibility::SceneNode, (i32, i32))>,
    /// The latest scene of each player's view (the PlayerController drawing it) and its size,
    /// for the muzzle of the model in his hands as he sees it (mod_weapons.rs).
    pub(crate) views: HashMap<ObjId, (crate::visibility::SceneNode, (i32, i32))>,
    /// Tracers drawn this HUD for such a weapon: (index in Vm::hud, weapon, end of the beam in
    /// its texture, the texture), placed again after the HUD (mod_weapons.rs mod_tracer_fixup).
    pub(crate) tracer_tiles: Vec<(usize, ObjId, [f32; 2], String)>,
    /// Changes to the actors of maps (mod_placement.rs).
    pub(crate) edits: Option<Arc<Vec<openxiii_mods::MapEdit>>>,
    /// Actors to spawn at the end of the level start-up.
    pub(crate) pending: Vec<mod_placement::PendingSpawn>,
    /// Placed characters a `change` gives a team or can_die (pawn, index of
    /// their derived [character]), taken at the end of the level start-up (mod_change.rs).
    pub(crate) changed_characters: Vec<(ObjId, usize)>,
    /// Skins mods gave to actors (host), the skins messages to send to the
    /// guests (connection, channel, actor), and skins that reached a guest before their actor
    /// (by channel) (mod_net_skins.rs).
    pub(crate) net_skins: HashMap<ObjId, mod_net_skins::NetSkins>,
    pub(crate) net_skin_outbox: Vec<(usize, usize, ObjId)>,
    pub(crate) net_skin_pending: HashMap<usize, mod_net_skins::NetSkins>,
    /// The scripts' effects shared with network guests (mod_net_fx.rs).
    pub(crate) net_fx: mod_net_fx::NetFx,
    /// The scripts' combat events, preload() and sounds (mod_script_more.rs).
    pub(crate) script_events: mod_script_more::More,
    /// The scripts' screen items and the loading screen (mod_script_hud.rs).
    pub(crate) hud: mod_script_hud::Hud,
    /// Values for the actor being spawned by a mod, set before its BeginPlay.
    pub(crate) spawn_values: Option<Vec<openxiii_mods::placement::ActorValue>>,
    /// Keys bound to actions and mod characters (mod_behaviour.rs).
    pub(crate) behaviour: Option<Arc<mod_behaviour::ModBehaviour>>,
    /// Mod characters alive, by pawn.
    pub(crate) live: BTreeMap<ObjId, mod_behaviour::LiveCharacter>,
    /// AI controller of each mod character -> its pawn.
    pub(crate) live_controllers: BTreeMap<ObjId, ObjId>,
    /// Lines said by characters on this level (mod_dialogue.rs).
    pub(crate) dialogues: mod_dialogue::ModDialogues,
    /// Sound banks and packages to load (mod_sounds.rs).
    pub(crate) loads: Option<Arc<Vec<openxiii_mods::content::LoadList>>>,
    /// Sounds of the game replaced on this level (`[map.<Map>.replace_sounds]`):
    /// lower-case name of the game's sound → the sound played instead (mod_sounds.rs).
    pub(crate) sound_swaps: HashMap<String, String>,
    /// The mods' own sounds, decoded (mod_own_sounds.rs).
    pub(crate) own_sounds: Option<Arc<Vec<mod_own_sounds::OwnLoaded>>>,
    /// The mods' new classes to register (mod_classes.rs).
    pub(crate) new_classes: Option<Arc<Vec<openxiii_mods::new_classes::NewClass>>>,
    /// Network identity of this Vm's mods, for tests (mod_net.rs).
    pub(crate) network: Option<Arc<mod_net::ModNetwork>>,
    /// Each mod's new class -> the game class it is made from.
    pub(crate) class_parents: HashMap<ClassId, ClassId>,
    /// The mods whose Lua scripts run (mod_script.rs).
    pub(crate) script_sources: Option<Arc<Vec<mod_script::ScriptMod>>>,
    /// The Lua scripts of this map, running.
    pub(crate) scripts: Option<Box<mod_script::ScriptRuntime>>,
    /// Teams given by scripts to characters (pawn -> team towards the players).
    pub(crate) script_teams: HashMap<ObjId, openxiii_mods::behaviour::Team>,
    pub(crate) surfaces: Arc<mod_retexture::SurfaceEffects>,
    /// Areas of heat shimmer asked by scripts (mod_heat.rs).
    pub(crate) heat: Vec<mod_heat::HeatZone>,
    /// `attack` orders given again until the soldier's AI holds them
    /// (pawn, target, until this level time).
    pub(crate) script_attacks: Vec<(ObjId, ObjId, f32)>,
    /// A scene of the scripts holds the players' views (camera()).
    pub(crate) script_camera: bool,
    /// Orders to story characters given before they were set up
    /// (pawn, command, until this level time).
    pub(crate) cine_pending: Vec<(ObjId, String, f32)>,
    /// A network guest, the keys the host's scripts react to (sent by the
    /// host); the host, the remote players already told and the keys told (mod_script_keys.rs).
    pub(crate) guest_keys: Vec<String>,
    pub(crate) keys_told: (Vec<ObjId>, Vec<String>),
    /// The mods' "when ... then ..." rules (mod_when.rs).
    pub(crate) when_rules: Option<Arc<Vec<mod_when::WhenRule>>>,
    /// Locked doors whose nearby path nodes the scripts closed (door, nodes).
    pub(crate) locked_doors: Vec<(ObjId, Vec<ObjId>)>,
}

impl ModState {
    pub(crate) fn new() -> Self {
        Self {
            defaults: mod_defaults::global_mod_defaults(),
            attach_firing: HashMap::new(),
            checkpoint: None,
            weapon_slots: Default::default(),
            fp_models: HashMap::new(),
            fp_idle: Default::default(),
            mesh_fronts: HashMap::new(),
            tracer_ends: HashMap::new(),
            view_projection: None,
            views: HashMap::new(),
            tracer_tiles: Vec::new(),
            edits: mod_placement::global_mod_edits(),
            pending: Vec::new(),
            changed_characters: Vec::new(),
            net_skins: HashMap::new(),
            net_skin_outbox: Vec::new(),
            net_skin_pending: HashMap::new(),
            net_fx: Default::default(),
            script_events: Default::default(),
            hud: Default::default(),
            spawn_values: None,
            behaviour: mod_behaviour::global_mod_behaviour(),
            live: BTreeMap::new(),
            live_controllers: BTreeMap::new(),
            dialogues: Default::default(),
            loads: mod_sounds::global_mod_loads(),
            sound_swaps: HashMap::new(),
            own_sounds: mod_own_sounds::global_own_sounds(),
            new_classes: mod_classes::global_mod_classes(),
            network: None,
            class_parents: HashMap::new(),
            script_sources: mod_script::global_mod_scripts(),
            scripts: None,
            script_teams: HashMap::new(),
            surfaces: Default::default(),
            heat: Vec::new(),
            script_attacks: Vec::new(),
            script_camera: false,
            cine_pending: Vec::new(),
            guest_keys: Vec::new(),
            keys_told: (Vec::new(), Vec::new()),
            when_rules: mod_when::global_mod_when(),
            locked_doors: Vec::new(),
        }
    }
}
