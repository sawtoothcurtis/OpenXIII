//! Local co-op campaign (improved build only, off by default).
//!
//! The solo campaign runs with two local players, one per viewport (split screen, client.rs).
//! Nothing here runs unless `Vm::set_coop(true)` was called: the solo game is untouched.
//!
//! What the solo scripts do with a second local player, and what co-op changes:
//! - XIIIGameInfo.Login restarts every player on the same PlayerStart: the second pawn cannot
//!   spawn inside the first one and its controller goes to state Dead. Co-op lets it spawn
//!   there, then moves it to a free place next to the first player.
//! - RestartPlayer fires the start's event for each player: co-op fires it for the first only
//!   (checkpoint save and scripted sequences must not start twice).
//! - MapInfo.FindPlayerController returns the head of the controller list, which is the last
//!   player to log in: co-op answers the first player, who is the story player.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;
use crate::types::FuncId;
use crate::value::ObjId;
use crate::{Result, Vm};

/// Co-op state of a Vm.
#[derive(Debug, Default, Clone)]
pub struct Coop {
    /// A generator callback has re-entered its own Timer on this VM.
    pub(crate) generator_busy: bool,
    /// Co-op campaign on.
    pub on: bool,
    /// Split-screen multiplayer game on this map (split_versus.rs).
    pub versus: bool,
    /// Identities of the connected gamepads, by handle, given by the host (split_versus.rs).
    pub(crate) pad_ids: Option<Vec<u64>>,
    /// Split-screen multiplayer (split_versus.rs): the in-game menu of a player other than the
    /// first is being created; ViewPortId of a HUD lowered to 0 meanwhile (HUD, its value); level
    /// time of the last "connect a gamepad" hint.
    pub(crate) versus_menu: bool,
    pub(crate) versus_viewport_id: Option<(ObjId, Value)>,
    pub(crate) versus_hint_time: f64,
    /// Player controllers of the current map, in viewport order (the first is the story player).
    pub players: Vec<ObjId>,
    /// Tests: the partners stay aim targets (aim.rs, coop_aim_partner), as before the fix.
    #[cfg(test)]
    pub(crate) aim_partners_kept: bool,
    /// Button word of each player's gamepad on the previous frame (gamepad_config::word).
    pub(crate) pad_words: Vec<u16>,
    /// Game keys each player's gamepad wanted down on the previous frame (bits, in the order of
    /// `coop_pad_keys`).
    pub(crate) pad_keys_held: Vec<u32>,
    /// Menu keys held by each player's gamepad (bits).
    pub(crate) pad_menu: Vec<u8>,
    /// Gamepad buttons of each player held while their menu was open (bits): not given to the
    /// game until released, so the press that closes the menu does not also crouch or jump.
    pub(crate) pad_hold: Vec<u16>,
    /// A story gift is being repeated for the other players.
    pub(crate) sharing: bool,
    /// Intro sounds (coop_intro_sound): the actor that played the opening sound, and the
    /// closing sound already played.
    pub(crate) intro_actor: Option<Option<ObjId>>,
    pub(crate) intro_closed: bool,
    /// Level time at which each player went down (coop_combat.rs); None: in play.
    pub(crate) down: Vec<Option<f64>>,
    /// Body of each player who is down.
    pub(crate) bodies: Vec<Option<ObjId>>,
    /// Every player is dead: the solo ending runs.
    pub(crate) defeat: bool,
    /// Story part (coop_story.rs): level time of its last frame, cinematic running, viewport
    /// extended for it, players' places on the last frame, travel equipment handed out, a
    /// message of the first player's HUD being repeated.
    pub(crate) story_time: f64,
    pub(crate) cine: bool,
    pub(crate) cine_extended: bool,
    /// Dialogue boxes copied on other local HUDs (coop_story.rs, `coop_dialog_copies_frame`):
    /// (story HUD, its HudDialog, [(other HUD, its copy)]).
    pub(crate) dialog_copies: Vec<(ObjId, ObjId, Vec<(ObjId, ObjId)>)>,
    /// Dialogue boxes laid out (coop_story.rs, `coop_dialog_layout_before`): (HudDialog,
    /// canvas ClipX, ClipY, original text, text with its line breaks).
    pub(crate) dialog_layouts: Vec<(ObjId, f32, f32, String, String)>,
    /// Inside that DrawDlg call (the call itself goes through the hook again).
    pub(crate) dialog_layout_busy: bool,
    /// Player whose cinematic it is (the one the script holds in state NoControl).
    pub(crate) cine_leader: usize,
    /// Player of the last cinematic of the map (its picture is the end of the map's).
    pub(crate) cine_last_leader: usize,
    pub(crate) last_places: Vec<Option<[f32; 3]>>,
    /// The cinematic's player was moved far away at once during the cinematic under way.
    pub(crate) cine_jumped: bool,
    pub(crate) equipped: bool,
    /// The Select level equipment was given on this map (interaction.rs): a player who joins
    /// later gets full ammunition with the copied weapons.
    pub(crate) select_equipment: bool,
    /// Each player's own travel inventory (ExportTravel text, by rank) written when the last map
    /// was left with its items (XIIIGameInfo.ProcessServerTravel, bItems): taken by the next map's
    /// logins (client.rs, spawn_play_actors).
    pub(crate) carried: Vec<Option<String>>,
    /// This map, reached by such a travel: each player's own inventory from the previous map (by
    /// rank), and whether a player got it through their own login (AcceptInventory). A player
    /// without it (a new game, a loaded checkpoint, a Select level start, a player who was not
    /// there) gets the first player's kinds of weapons and items, as before.
    pub(crate) arrived: Vec<Option<String>>,
    pub(crate) own_travel: Vec<bool>,
    /// Tests: the rule of the map being left applied to the first player only, as before
    /// (coop_story.rs, coop_travel_rule_end).
    #[cfg(test)]
    pub(crate) travel_rule_off: bool,
    pub(crate) mirroring: bool,
    /// A script event caused by a player other than the story player is running
    /// (coop_world.rs).
    pub(crate) acting: bool,
    /// An enemy's SeePlayer, HearNoise or DamageAttitudeTo is running (coop_combat.rs).
    pub(crate) ai_event: bool,
    /// Scenery a map wanted hidden while the players were apart: (trigger, rank in its list);
    /// and the trigger's script running on a masked list (coop_world.rs).
    pub(crate) kept_scenery: Vec<(ObjId, usize)>,
    pub(crate) scenery: bool,
    /// Effects of a place given to every player (coop_world.rs): one of their scripts running,
    /// and the dazzles of broken movers on the players other than the script's.
    pub(crate) place_effect: bool,
    pub(crate) dazzles: Vec<crate::coop_world::Dazzle>,
    /// The team's death screen has been given the whole window.
    pub(crate) defeat_shown: bool,
    /// Level time of the last upkeep of the enemies.
    pub(crate) ai_time: f64,
    /// Enhanced network play: a client joining its server again (net_reconnect.rs).
    pub(crate) reconnect: crate::net_reconnect::NetReconnect,
    /// Enhanced network play: this client's host left or was lost; told once the menu is back
    /// (net_reconnect.rs, `net_host_left_notice_tick`).
    pub(crate) host_left_notice: bool,
    pub(crate) net_stats_last: f64,
    /// This machine is back from a network co-op game: its menu is checked on the menu map
    /// (lockstep_menus.rs, `lockstep_menu_back`).
    pub(crate) menu_back_wanted: bool,
    pub(crate) net_stats_time: f64,
    pub(crate) net_state_last: f64,
    pub(crate) net_moves: Vec<(crate::ObjId, [f32; 3], f64, u32, f64, u32)>,
    /// Enhanced multiplayer: drawing of the pawns of players on other machines, smoothed
    /// between their moves (net_smooth.rs).
    pub(crate) net_smooth: Vec<crate::net_smooth::NetSmooth>,
    pub(crate) net_drawn: Vec<(crate::ObjId, [f32; 3], f32, f32, u32, u32, f32, f32)>,
    /// Split screen: screen effects of each local player's view (screen_fx.rs).
    pub(crate) screen_fx: Vec<(ObjId, crate::screen_fx::ScreenFx)>,
    /// Split screen: the views whose effects were drawn since the last level tick.
    pub(crate) screen_fx_drawn: Vec<ObjId>,
    /// Extra enemies (coop_enemies.rs).
    pub enemies: crate::coop_enemies::CoopEnemies,
    /// Flashbacks (coop_flash.rs).
    pub(crate) flash: crate::coop_flash::CoopFlash,
    /// Rules of the scripted story: players held by a scene (coop_rules.rs).
    pub(crate) rules: crate::coop_rules::CoopRules,
    /// Scenes' players and the players parked during a cinematic (coop_cine.rs).
    pub(crate) cine_rules: crate::coop_cine::CoopCine,
    /// A sharing rule's own call is running (coop_shared.rs).
    pub(crate) shared_busy: bool,
    /// Portal of the travel that loaded this map, and the start the second player logged in
    /// at (coop_spawns.rs).
    pub(crate) spawn_portal: String,
    /// Tests: free places judged with the cylinders and close places kept as they are, as
    /// before (negative control).
    #[cfg(test)]
    pub(crate) place_cylinders: bool,
    pub(crate) spawn_anchor: Option<crate::coop_spawns::SpawnAnchor>,
    /// A start with room for one (the duct of PRock03): the second pawn waits on the first
    /// one's start with its collision off until the first one has moved off it (pawn, its
    /// collision flags).
    pub(crate) spawn_waiting: Option<(ObjId, [bool; 3])>,
    /// Level time of the bosses' last look for their target (coop_shared.rs).
    pub(crate) shared_time: f64,
    /// The levels' hints shown to every player (coop_hints.rs).
    pub(crate) hints: crate::coop_hints::CoopHints,
    /// The end of the map shown from the player who ended it (coop_end.rs).
    pub(crate) end: crate::coop_end::CoopEnd,
}

/// What `coop_before_extra_login` changed, to put back after the login.
pub(crate) struct ExtraLogin {
    first_pawn: Option<(ObjId, [bool; 3])>,
    events: Vec<(ObjId, Value)>,
    start_spot_event: Option<Value>,
    /// The checkpoint triggers (XIIISaveGameTrigger) and their collision, off for the login.
    save_triggers: Vec<(ObjId, [bool; 3])>,
    /// GameInfo.bInventorySetUp, false for the login (see coop_before_extra_login).
    inventory_set_up: Option<Value>,
}

/// Places tried for an extra player around the first one: (right, forward) in units. To the
/// right first, then the left, then behind, in front last (in the way of what they look at).
const PLACES: [(f32, f32); 20] = [
    (100.0, 0.0), (-100.0, 0.0), (0.0, -100.0), (0.0, 100.0),
    (160.0, 0.0), (-160.0, 0.0), (0.0, -160.0), (0.0, 160.0),
    (220.0, 0.0), (-220.0, 0.0), (0.0, -220.0), (0.0, 220.0),
    // Small rooms: closer (two pawn radii of 34 apart), last.
    (75.0, 0.0), (-75.0, 0.0), (0.0, -75.0), (0.0, 75.0),
    (55.0, -55.0), (-55.0, -55.0), (55.0, 55.0), (-55.0, 55.0),
];

/// Ground looked for from this far above the teammate's height to this far under it.
/// XIIIRootWindow.CurrentPF of the console menus (the Xbox, whose controller's A / B pictures
/// are tButton[4] / tButton[5]); the PC's is 0.
const CONSOLE_PLATFORM: i32 = 2;
/// Controls page XIIIMenuInGame opens on consoles (not in the PC game) and on PC.
const CONSOLE_CONTROLS_PAGE: &str = "XIDInterf.XIIIMenuInGameControlsWindow";
const PC_CONTROLS_PAGE: &str = "XIDInterf.XIIIMenuInputPC";
const GROUND_ABOVE: f32 = 50.0;
const GROUND_DEPTH: f32 = 140.0;

/// Aspect ratio the game was made for.
pub const REFERENCE_ASPECT: f32 = 4.0 / 3.0;

/// Horizontal field of view (degrees) of a split-screen view `w` × `h`: the game's FovAngle is
/// that of a 4:3 screen, and a wide half screen would only show a thin slice of it. The view
/// keeps the 4:3 height and gains width instead. The scripts still see their own FovAngle.
pub fn split_fov(fov: f32, w: u32, h: u32) -> f32 {
    let aspect = w as f32 / h.max(1) as f32;
    if aspect <= REFERENCE_ASPECT {
        return fov;
    }
    let half = (fov.clamp(1.0, 170.0).to_radians() / 2.0).dtan() * aspect / REFERENCE_ASPECT;
    2.0 * half.datan().to_degrees()
}

/// Half-height of the view (tangent of half the vertical angle) of a split-screen player: that of
/// the old co-op mod, a horizontal 110° in a 32:9 half (a 16:9 window split in two).
const SPLIT_VIEW_TAN_V: f32 = 0.401_658_5; // tan(55°) × 9 / 32

/// Horizontal field of view (degrees) of a player's view in split screen (`w` × `h`), from the
/// controller's FovAngle `fov` and DefaultFOV `default_fov`: the normal view gets the old co-op
/// mod's look (110° across a 32:9 half, the same height of view on other halves), never narrower
/// than the game's own, and a zoom keeps its strength (the same factor on its tangent).
/// split_fov stays the character screen's.
pub fn split_view_fov(fov: f32, default_fov: f32, w: u32, h: u32) -> f32 {
    split_view_fov_of(fov, default_fov, w, h, crate::display_format::active(Default::default()))
}

/// Widest view shape (width / height) whose split-screen view is a solo window's of that shape.
const SOLO_SHAPE_MAX: f32 = 16.0 / 9.0;

/// `split_view_fov`; `solo_shape` (wide-screen display on, Graphics: Enhanced): a view also never
/// narrower than a solo window of its own shape (Hor+, display_format.rs) up to 16:9, so that a
/// side-by-side half of a 32:9 screen (16:9) looks like a 16:9 screen, a half of a 21:9 screen
/// like a 4:3 one; wider halves keep at least the 16:9 view until the old mod's angle is wider.
pub fn split_view_fov_of(fov: f32, default_fov: f32, w: u32, h: u32, solo_shape: bool) -> f32 {
    let aspect = w as f32 / h.max(1) as f32;
    let base = (default_fov.clamp(1.0, 170.0).to_radians() / 2.0).dtan();
    let mut k = (SPLIT_VIEW_TAN_V * aspect / base).max(1.0);
    if solo_shape {
        k = k.max(aspect.min(SOLO_SHAPE_MAX) / REFERENCE_ASPECT);
    }
    let half = (fov.clamp(1.0, 170.0).to_radians() / 2.0).dtan() * k;
    2.0 * half.datan().to_degrees()
}

#[cfg(test)]
#[test]
fn split_view_fov_like_the_old_mod() {
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    // 1920 × 540 and 1366 × 384 halves: 110°, as the old mod.
    assert!(near(split_view_fov(85.0, 85.0, 1920, 540), 110.0), "{}", split_view_fov(85.0, 85.0, 1920, 540));
    assert!(near(split_view_fov(85.0, 85.0, 1366, 384), 110.0));
    // A 4:3 or 16:9 view (four players): never narrower than the game's 85°.
    assert_eq!(split_view_fov(85.0, 85.0, 800, 600), 85.0);
    assert_eq!(split_view_fov(85.0, 85.0, 960, 540), 85.0);
    // A zoom keeps its ratio to the normal view (tangents).
    let zoom = split_view_fov(10.624, 85.0, 1920, 540);
    let ratio = (zoom.to_radians() / 2.0).dtan() / (110f32.to_radians() / 2.0).dtan();
    assert!((ratio - (10.624f32.to_radians() / 2.0).dtan() / (85f32.to_radians() / 2.0).dtan()).abs() < 1e-4);
}

#[cfg(test)]
#[test]
fn split_view_fov_on_the_halfs_own_shape() {
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    let solo_169 = 2.0 * ((42.5f32.to_radians()).dtan() * (16.0 / 9.0) * 0.75).datan().to_degrees();
    // Side by side: a 32:9 screen's halves are 16:9 (a solo 16:9 window's view), a 21:9 screen's
    // narrower than 4:3 (the game's 85°).
    assert!(near(split_view_fov_of(85.0, 85.0, 2560, 1440, true), solo_169), "{}", split_view_fov_of(85.0, 85.0, 2560, 1440, true));
    assert_eq!(split_view_fov_of(85.0, 85.0, 1280, 1080, true), 85.0);
    assert_eq!(split_view_fov_of(85.0, 85.0, 1720, 1440, true), 85.0);
    // Stacked halves of a 16:9 screen: the old mod's 110°, as before.
    assert!(near(split_view_fov_of(85.0, 85.0, 1920, 540, true), 110.0));
    // Without the wide-screen display: exactly as before.
    assert_eq!(split_view_fov_of(85.0, 85.0, 2560, 1440, false), 85.0);
    // A zoom keeps its ratio to the normal view.
    let zoom = split_view_fov_of(10.624, 85.0, 2560, 1440, true);
    let ratio = (zoom.to_radians() / 2.0).dtan() / (solo_169.to_radians() / 2.0).dtan();
    assert!((ratio - (10.624f32.to_radians() / 2.0).dtan() / (85f32.to_radians() / 2.0).dtan()).abs() < 1e-4);
}

impl Vm {
    /// The co-op player `pc` plays on this machine (has a viewport).
    pub fn coop_is_local(&self, pc: ObjId) -> bool {
        self.local_player_controller(pc).is_some()
    }

    /// Co-op players on this machine.
    pub fn coop_local_players(&self) -> usize {
        self.coop.players.iter().filter(|&&pc| self.objects.get(pc).is_some_and(|o| o.is_some()) && self.coop_is_local(pc)).count()
    }

    /// Turns the co-op campaign on or off (before loading a map).
    pub fn set_coop(&mut self, on: bool) {
        self.coop.on = on && crate::improved::local_play::CAMPAIGN_AVAILABLE;
        if self.coop.on {
            self.engine.client.wanted = self.engine.client.wanted.max(2);
        }
    }

    /// Co-op campaign on.
    pub fn coop_on(&self) -> bool {
        self.coop.on
    }

    /// Start of the players' logins on a map.
    pub(crate) fn coop_begin_logins(&mut self) {
        self.coop.players.clear();
        self.coop.down.clear();
        self.coop.bodies.clear();
        self.coop.defeat = false;
        self.coop.intro_actor = None;
        self.coop.intro_closed = false;
        self.coop.pad_menu.clear();
        self.coop.pad_hold.clear();
        self.coop.story_time = -1.0;
        self.coop.cine = false;
        self.coop.cine_extended = false;
        self.coop.dialog_copies.clear();
        self.coop.dialog_layouts.clear();
        self.coop.dialog_layout_busy = false;
        self.coop.cine_leader = 0;
        self.coop.cine_last_leader = 0;
        self.coop.end = Default::default();
        self.coop.last_places.clear();
        self.coop.cine_jumped = false;
        self.coop.equipped = false;
        self.coop.select_equipment = false;
        self.coop.own_travel.clear();
        self.coop.mirroring = false;
        self.coop.acting = false;
        self.coop.ai_event = false;
        self.coop.kept_scenery.clear();
        self.coop.scenery = false;
        self.coop.place_effect = false;
        self.coop.dazzles.clear();
        self.coop.defeat_shown = false;
        self.coop.screen_fx.clear();
        self.coop.flash = Default::default();
        self.coop.rules = Default::default();
        self.coop.cine_rules = Default::default();
        self.coop.spawn_anchor = None;
        self.coop.spawn_waiting = None;
        // A map loaded from the team's death screen (Retry): split screen again.
        if self.coop.on {
            self.engine.client.extended = None;
        }
        self.coop.ai_time = 0.0;
        self.coop_enemies_begin();
    }

    /// A player has logged in (viewport order).
    pub(crate) fn coop_player_logged(&mut self, pc: ObjId) {
        if self.local_split() {
            self.coop.players.push(pc);
        }
    }

    /// Before the login of a player other than the first: the first pawn stops blocking, so
    /// that the new one can spawn on the same start, and the starts lose their event for the
    /// duration of the login.
    pub(crate) fn coop_before_extra_login(&mut self, game: ObjId) -> Result<ExtraLogin> {
        let mut saved = ExtraLogin { first_pawn: None, events: Vec::new(), start_spot_event: None, save_triggers: Vec::new(), inventory_set_up: None };
        // GameInfo.bInventorySetUp: XIIIGameInfo.AcceptInventory sets it at the end of the
        // first player's login. Left on, Weapon.GiveTo would take the next player's travelling
        // weapons for new ones (AmmoType.AddAmmo(PickUpAmmoCount), "don't add ammo as we just
        // travelled" otherwise) and XIIIWeapon's dual setup would differ: the extra player
        // logs in as the first one did, with it off; their own AcceptInventory sets it again.
        let set_up = self.prop(game, "bInventorySetUp");
        if set_up.as_bool() {
            self.set_prop(game, "bInventorySetUp", Value::Bool(false));
        }
        saved.inventory_set_up = Some(set_up);
        if let Some(pawn) = self.coop.players.first().and_then(|&pc| self.prop(pc, "Pawn").as_object()) {
            let flags = ["bCollideActors", "bBlockActors", "bBlockPlayers"].map(|n| self.prop(pawn, n).as_bool());
            self.call_named(pawn, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)])?;
            saved.first_pawn = Some((pawn, flags));
        }
        for a in self.actors.clone() {
            if self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "PlayerStart") {
                let event = self.prop(a, "Event");
                if event.as_bool() {
                    self.set_prop(a, "Event", Value::Name("None".into()));
                    saved.events.push((a, event));
                }
            }
        }
        saved.start_spot_event = Some(self.prop(game, "StartSpotEvent"));
        // The new pawn appears on the map's own start before being placed next to the first
        // player: a checkpoint trigger there (the level's first one) must not save that
        // checkpoint over the one the game was started from (a Select level start, a loaded
        // checkpoint), which Retry would then forget. The triggers do not collide during the
        // login and the placing.
        for a in self.actors.clone() {
            if self.objects.get(a).is_some_and(|o| o.is_some()) && self.is_a(a, "XIIISaveGameTrigger") && self.prop(a, "bCollideActors").as_bool() {
                let flags = ["bCollideActors", "bBlockActors", "bBlockPlayers"].map(|n| self.prop(a, n).as_bool());
                self.call_named(a, "SetCollision", vec![Value::Bool(false), Value::Bool(flags[1]), Value::Bool(flags[2])])?;
                saved.save_triggers.push((a, flags));
            }
        }
        Ok(saved)
    }

    /// After that login: everything put back, and the new pawn moved next to the first one;
    /// the checkpoint triggers collide again once it is in its place.
    pub(crate) fn coop_after_extra_login(&mut self, game: ObjId, mut saved: ExtraLogin, pc: Option<ObjId>) -> Result<()> {
        let triggers = std::mem::take(&mut saved.save_triggers);
        // Back as before the login if AcceptInventory did not run (no pawn).
        if let Some(set_up) = saved.inventory_set_up.take() {
            if set_up.as_bool() && !self.prop(game, "bInventorySetUp").as_bool() {
                self.set_prop(game, "bInventorySetUp", set_up);
            }
        }
        let placed = self.coop_place_extra_login(game, saved, pc);
        for (a, flags) in triggers {
            if self.objects.get(a).is_some_and(|o| o.is_some()) {
                self.call_named(a, "SetCollision", flags.iter().map(|&b| Value::Bool(b)).collect())?;
            }
        }
        placed
    }

    fn coop_place_extra_login(&mut self, game: ObjId, saved: ExtraLogin, pc: Option<ObjId>) -> Result<()> {
        for (a, event) in saved.events {
            self.set_prop(a, "Event", event);
        }
        if let Some(event) = saved.start_spot_event {
            self.set_prop(game, "StartSpotEvent", event);
        }
        let Some((first, flags)) = saved.first_pawn else { return Ok(()) };
        self.call_named(first, "SetCollision", flags.iter().map(|&b| Value::Bool(b)).collect())?;
        let Some(pawn) = pc.and_then(|pc| self.prop(pc, "Pawn").as_object()) else {
            self.log.push("Log: Co-op: the second player has no pawn".to_string());
            return Ok(());
        };
        // A start chosen for this map start or checkpoint (coop_spawns.rs).
        if let Some(pc) = pc {
            if self.coop_spawn_place(first, pawn, pc)? {
                return Ok(());
            }
        }
        for place in self.coop_places_near(first) {
            if self.call_named(pawn, "SetLocation", vec![Value::vector(place[0], place[1], place[2])])?.as_bool() {
                // Facing where the first player looks, or the freest way when that is a wall.
                if let Some(pc) = pc {
                    let yaw = self.coop_open_yaw(pawn, place, self.prop(first, "Rotation").as_rotator()[1]);
                    self.call_named(pawn, "SetRotation", vec![Value::rotator(0, yaw, 0)])?;
                    self.set_prop(pc, "Rotation", Value::rotator(0, yaw, 0));
                }
                self.log.push(format!("Log: Co-op: second player placed at {:.0},{:.0},{:.0}", place[0], place[1], place[2]));
                return Ok(());
            }
        }
        // Room for one: the second player waits on the start itself, without blocking or
        // being blocked (coop_spawn_wait_frame gives the collision back once the first one has
        // moved on). Left where the login put it, the pawn is pushed far away (PRock03: the
        // map's own start).
        let flags = ["bCollideActors", "bBlockActors", "bBlockPlayers"].map(|n| self.prop(pawn, n).as_bool());
        let at = self.prop(first, "Location").as_vector();
        self.call_named(pawn, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)])?;
        self.call_named(pawn, "SetLocation", vec![Value::vector(at[0], at[1], at[2])])?;
        self.coop.spawn_waiting = Some((pawn, flags));
        self.log.push("Log: Co-op: no free place next to the first player, the second one waits on the start".to_string());
        Ok(())
    }

    /// Places around `pawn` where another player can stand: nothing solid between the two (a
    /// wall, a closed door) and ground under the feet (not over a drop, not under a slope).
    pub(crate) fn coop_places_near(&mut self, pawn: ObjId) -> Vec<[f32; 3]> {
        let origin = self.prop(pawn, "Location").as_vector();
        let yaw = self.prop(pawn, "Rotation").as_rotator()[1] as f32 * (std::f32::consts::PI / 32768.0);
        let (forward, right) = ([yaw.dcos(), yaw.dsin()], [-yaw.dsin(), yaw.dcos()]);
        let vector = |p: [f32; 3]| Value::vector(p[0], p[1], p[2]);
        let (radius, height) = (self.prop(pawn, "CollisionRadius").as_float(), self.prop(pawn, "CollisionHeight").as_float());
        let Some(trace) = self.find_function(pawn, "Trace", false) else { return Vec::new() };
        // In water (a start where the player sinks, USA01): beside the teammate at the same
        // height, so that both sink together; there is no ground to stand on.
        let water = self.coop_in_water(pawn);
        let mut out = Vec::new();
        // Each place far enough for the two pawns' boxes not to overlap (coop_place_free): a
        // close place turned by the teammate's yaw can fall inside the box although outside the
        // cylinder; it is pushed out along its own direction just enough.
        let clear = 2.0 * radius + 1.0;
        #[cfg(test)]
        let clear = if self.coop.place_cylinders { 0.0 } else { clear };
        let candidates: Vec<(f32, f32)> = PLACES
            .iter()
            .map(|&(r, f)| {
                let (dx, dy) = (right[0] * r + forward[0] * f, right[1] * r + forward[1] * f);
                let most = dx.abs().max(dy.abs());
                let k = if most > 0.0 && most < clear { clear / most } else { 1.0 };
                (origin[0] + dx * k, origin[1] + dy * k)
            })
            .collect();
        for &(x, y) in &candidates {
            if water {
                continue;
            }
            // A pawn-sized box dropped from a little above the teammate's height: where it
            // lands is where a pawn stands (Actor.Trace with an extent). Under a low ceiling
            // the box starts at the teammate's own height instead.
            for above in [GROUND_ABOVE, 2.0] {
                let (start, end) = ([x, y, origin[2] + above], [x, y, origin[2] - GROUND_DEPTH]);
                let args = vec![
                    Some(vector([0.0; 3])),
                    Some(vector([0.0; 3])),
                    Some(vector(end)),
                    Some(vector(start)),
                    Some(Value::Bool(false)),
                    Some(vector([radius, radius, height])),
                ];
                let Ok((hit, outs)) = self.call(trace, pawn, args) else { continue };
                let (Some(_), Some(Some(at)), Some(Some(normal))) = (hit.as_object(), outs.first(), outs.get(1)) else { continue };
                let (at, normal) = (at.as_vector(), normal.as_vector());
                // Landed at once: the box started inside something. A steep landing is not ground.
                if at[2] > start[2] - 1.0 || normal[2] < 0.7 {
                    continue;
                }
                let place = [x, y, at[2] + 2.0];
                // Nobody standing there: a pawn moved onto another one kills it (a cinematic's
                // actor at the start of PRock01a, the first player at the start of PRock03).
                if self.coop_place_free(place, radius, height) && self.call_named(pawn, "FastTrace", vec![vector(place), vector(origin)]).is_ok_and(|v| v.as_bool()) {
                    out.push(place);
                }
                break;
            }
        }
        // No ground near (water, or a start in the air above it): beside the teammate at the
        // same height.
        if out.is_empty() {
            for &(x, y) in &candidates {
                let place = [x, y, origin[2]];
                if self.coop_place_free(place, radius, height) && self.call_named(pawn, "FastTrace", vec![vector(place), vector(origin)]).is_ok_and(|v| v.as_bool()) {
                    out.push(place);
                }
            }
        }
        out
    }

    /// The menu of player `pc` is open: the GUIController of their viewport (one of its
    /// LocalInteractions) has a page on its MenuStack.
    pub(crate) fn coop_menu_open(&self, pc: ObjId) -> bool {
        let Some(viewport) = self.prop(pc, "Player").as_object().filter(|&v| self.objects.get(v).is_some_and(|o| o.is_some())) else { return false };
        let Value::Array(interactions) = self.prop(viewport, "LocalInteractions") else { return false };
        interactions.iter().filter_map(Value::as_object).any(|o| {
            self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "GUIController") && matches!(self.prop(o, "MenuStack"), Value::Array(pages) if pages.iter().any(|p| p.as_object().is_some()))
        })
    }

    /// Gamepad of a player other than the first, read during that player's UpdateInput (its
    /// UInput is the current one). PC XIII has no gamepad bindings: the gamepad presses the
    /// keys of the keyboard bindings on this player's own UInput. Left stick walking (analog_move), right
    /// stick look, right trigger fire, left trigger alt fire, A jump, B crouch, X reload,
    /// Y grab, LB / RB previous / next weapon, d-pad left / right item, d-pad up or Back
    /// medkit, left stick click walk.
    pub(crate) fn coop_pad_input(&mut self, pc: ObjId, state: crate::pad::PadState) {
        use crate::player_input::{IST_PRESS, IST_RELEASE};
        // Each player has their own menu, in their own half of the screen: Start opens it
        // (Escape on this player's UInput, as the keyboard does for the first player), and
        // while it is open this gamepad moves in it (d-pad or left stick, A to choose, B or
        // Start to go back).
        // The failure menu in the whole window is every player's (coop_end.rs).
        let menu = (self.coop_menu_open(pc) || self.coop_end_menu_open(pc)).then_some(());
        let rank = self.coop.players.iter().position(|&p| p == pc).unwrap_or(0);
        if self.coop.pad_menu.len() <= rank {
            self.coop.pad_menu.resize(rank + 1, 0);
        }
        let bt = state.buttons;
        let (stick, repeats) = self.menu_stick_directions(rank, &state, menu.is_some());
        let menu_keys: [(u8, bool); 6] = if menu.is_some() {
            [
                (0x26, bt[10] || stick[0]),
                (0x28, bt[11] || stick[1]),
                (0x25, bt[12] || stick[2]),
                (0x27, bt[13] || stick[3]),
                (0x0D, bt[0]),
                (0x1B, bt[1] || bt[7]),
            ]
        } else {
            [(0x26, false), (0x28, false), (0x25, false), (0x27, false), (0x0D, false), (0x1B, bt[7])]
        };
        let before = self.coop.pad_menu[rank];
        let mut now = 0u8;
        for (bit, (key, pressed)) in menu_keys.into_iter().enumerate() {
            if pressed {
                now |= 1 << bit;
            }
            if pressed != (before & (1 << bit) != 0) {
                self.input_process(pc, key, if pressed { IST_PRESS } else { IST_RELEASE }, 0.0);
            } else if pressed && bit < 4 {
                // A held direction's repeat: released and pressed again (a second press of a key
                // already down is not taken by every menu).
                if repeats[bit] {
                    self.input_process(pc, key, IST_RELEASE, 0.0);
                    self.input_process(pc, key, IST_PRESS, 0.0);
                }
            }
        }
        self.coop.pad_menu[rank] = now;
        // Buttons pressed in the menu stay the menu's until released (B or Start closing it,
        // A choosing "Back to the game").
        if self.coop.pad_hold.len() <= rank {
            self.coop.pad_hold.resize(rank + 1, 0);
        }
        let pressed = state.buttons.iter().enumerate().fold(0u16, |m, (i, &b)| if b { m | 1 << i } else { m });
        let hold = if menu.is_some() { pressed } else { self.coop.pad_hold[rank] & pressed };
        self.coop.pad_hold[rank] = hold;
        let mut state = state;
        for (i, b) in state.buttons.iter_mut().enumerate() {
            if hold & (1 << i) != 0 {
                *b = false;
            }
        }
        // In a menu or during a cinematic (coop_story.rs) the player stands still.
        let state = if self.coop.cine || menu.is_some() { crate::pad::PadState::default() } else { state };
        let [lx, ly, _, rx, ry, _] = state.axes;
        let versus = self.coop.versus;
        // The buttons follow this player's gamepad settings (gamepad_config.rs; the default is
        // the table this function always had).
        let slot = self.coop.players.iter().position(|&p| p == pc).unwrap_or(0);
        // Lockstep co-op: the player's own gamepad settings, on every machine.
        let settings = self.lockstep_pad_settings(slot).unwrap_or(self.gamepad_players[slot.min(crate::gamepad_config::PLAYERS - 1)]);
        // Their own aim assist (gamepad_config.rs). Lockstep co-op: the aim assist of the device
        // each player uses comes in their own commands (aim_device.rs, `PlayerCmd::aim`), the
        // gamepad's when they take it, the keyboard's when they leave it.
        let lockstep = self.lockstep.is_some();
        if !lockstep {
            self.other_pad_aim(pc, slot, settings.aim_assist);
        }
        if self.coop.pad_words.len() <= slot {
            self.coop.pad_words.resize(slot + 1, 0);
        }
        if self.coop.pad_keys_held.len() <= slot {
            self.coop.pad_keys_held.resize(slot + 1, 0);
        }
        let before = self.coop.pad_words[slot];
        let (keys, wheel, word) = coop_pad_keys(&settings, &state, before, versus);
        self.coop.pad_words[slot] = word;
        let held_before = self.coop.pad_keys_held[slot];
        let mut held = 0u32;
        for (i, (key, wanted)) in keys.into_iter().enumerate() {
            if wanted {
                held |= 1 << i;
            }
            // Lockstep co-op: the player also has their keyboard and mouse, so the gamepad lets
            // go only of the keys it held itself (a key or button held on the keyboard or mouse
            // stays down while the gamepad rests). Split screen: the gamepad is the player's only
            // device, as before.
            let release_ok = !lockstep || held_before & (1 << i) != 0;
            if wanted != self.input_key_down(key) && (wanted || release_ok) {
                self.input_process(pc, key, if wanted { IST_PRESS } else { IST_RELEASE }, 0.0);
            }
        }
        self.coop.pad_keys_held[slot] = held;
        // Wheel notches (LB / RB by default): one step on press.
        for name in wheel {
            if let Some(key) = Vm::key_code(name) {
                self.input_process(pc, key, IST_PRESS, 0.0);
                self.input_process(pc, key, IST_RELEASE, 0.0);
            }
        }
        // Right stick: look, through the game's joystick path (look_stick_axes).
        let dead = |v: i32| if v.abs() < 6554 { 0.0 } else { v as f32 / 32768.0 };
        // Lockstep co-op: a stick its player's machine already turned the view with (frame by
        // frame, in the command's turn) is not read for the view again.
        if !self.lockstep_sticks_turned(pc) {
            let own = std::mem::replace(&mut self.look_stick, Some((dead(rx), dead(ry))));
            let own_tune = std::mem::replace(&mut self.look_stick_tune, settings.look_tune());
            self.look_stick_axes(pc);
            self.look_stick = own;
            self.look_stick_tune = own_tune;
        }
        // Left stick: walking as far as it is pushed, read by this player's
        // DealWithPlayerInputEvent (player_input.rs, analog_move).
        let stick = (lx as f32 / 32768.0, ly as f32 / 32768.0);
        self.record_movement_stick(pc, stick);
    }

    /// Sounds of the intro comic panels (XIIIsound.Interface__BeginMap). The opening sound
    /// lowers the game's sounds (HX variable SoundDesign at −30 dB) and the closing sound,
    /// played when the HUD's player moves, brings them back. Each HUD plays its own set: with
    /// two HUDs the two programs undo each other's work and the map stays 30 dB too quiet
    /// (measured). The set is played once for the team: opening and panel changes from the
    /// first HUD that plays them, the closing sound once, on the same actor. `None`: dropped.
    pub(crate) fn coop_intro_sound(&mut self, call: crate::canvas::AudioCall) -> Option<crate::canvas::AudioCall> {
        use crate::canvas::AudioCall;
        let AudioCall::Play { actor, rolloff, sound, slot, params, check_handler, first_free } = call else { return Some(call) };
        if !sound.contains("Interface__BeginMap.") {
            return Some(AudioCall::Play { actor, rolloff, sound, slot, params, check_handler, first_free });
        }
        let first = *self.coop.intro_actor.get_or_insert(actor);
        if sound.ends_with("hBeginMapPlayerMove") {
            if std::mem::replace(&mut self.coop.intro_closed, true) {
                return None;
            }
            // On the actor that opened, whoever moved first.
            return Some(AudioCall::Play { actor: first, rolloff: first, sound, slot, params, check_handler, first_free });
        }
        if actor != first || self.coop.intro_closed {
            return None;
        }
        Some(AudioCall::Play { actor, rolloff, sound, slot, params, check_handler, first_free })
    }

    /// Intro comic panels of a map. The first HUD whose live panel fills its view ends the intro
    /// for the level (Level.InitialCartoonEffect = 0, XIIIBaseHud.DrawCartoonWindowBis), but each
    /// HUD has its own HudCartoonSFX: the others can no longer finish theirs and would keep
    /// their panels forever. They are released here.
    pub(crate) fn coop_sync_intro(&mut self) {
        self.coop_combat_frame();
        self.coop_enemies_frame();
        let Some(level) = self.level else { return };
        if self.prop(level, "InitialCartoonEffect").as_int() != 0 {
            return;
        }
        for pc in self.coop.players.clone() {
            if self.objects.get(pc).is_none_or(|o| o.is_none()) {
                continue;
            }
            let Some(hud) = self.prop(pc, "MyHUD").as_object() else { continue };
            if !self.is_a(hud, "XIIIBaseHUD") || !self.prop(hud, "HudCartoonSFX").as_bool() {
                continue;
            }
            // A HUD released here has not played the intro's closing sound: played now, as the
            // script would (coop_intro_sound lets it through once for the team).
            if !self.prop(hud, "HideCartoonHud").as_bool() {
                if let Some(owner) = self.prop(hud, "PawnOwner").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
                    let sound = self.prop(hud, "hIntroBeginMapPlayerMove");
                    let _ = self.call_named(owner, "PlayMenu", vec![sound]);
                }
            }
            self.set_prop(hud, "HudCartoonSFX", Value::Bool(false));
            self.set_prop(hud, "HideCartoonHud", Value::Bool(true));
            if let Some(window) = self.prop(hud, "HudCartoonWnd").as_object() {
                let _ = self.call_named(window, "Destroy", vec![]);
                self.set_prop(hud, "HudCartoonWnd", Value::Object(None));
            }
        }
    }

    /// MapInfo.GiveSomething(ItemClass, P): what the story gives to a player (starting
    /// equipment of the map, items handed over by a character or a script).
    pub(crate) fn coop_shares_gift(&self, f: FuncId) -> bool {
        let function = &self.functions[f];
        !self.coop.sharing && function.name == "GiveSomething" && self.classes[function.owner].name.eq_ignore_ascii_case("MapInfo")
    }

    /// After such a gift to one player: the same to every other living player.
    pub(crate) fn coop_share_gift(&mut self, f: FuncId, map_info: ObjId, args: Vec<Option<Value>>) {
        let Some(receiver) = args.get(1).and_then(|a| a.as_ref()).and_then(|v| v.as_object()) else { return };
        // A countdown or Palace01's microphone stays one item (coop_shared.rs).
        if matches!(args.first().and_then(|a| a.as_ref()), Some(Value::Class(Some(c))) if self.coop_single_item(*c)) {
            return;
        }
        let pawns: Vec<ObjId> = self.coop.players.clone().into_iter().filter_map(|pc| self.prop(pc, "Pawn").as_object()).collect();
        if !pawns.contains(&receiver) {
            return;
        }
        self.coop.sharing = true;
        for pawn in pawns {
            if pawn != receiver && self.prop(pawn, "Health").as_int() > 0 {
                let mut again = args.clone();
                again[1] = Some(Value::Object(Some(pawn)));
                if let Err(e) = self.call(f, map_info, again) {
                    self.log.push(format!("Log: Co-op: shared gift failed: {e}"));
                }
            }
        }
        self.coop.sharing = false;
    }

    /// PlayerController.SetPause(bPause) asked by a player's menu (XIIIRootWindow.UWindows.BeginState
    /// and EndState). The PC game pauses when its menu opens in a lone-player game (Level.bLonePlayer,
    /// PC bytecode of BeginState at 142); in a game with several players the menu only sets that
    /// player's bIsInMenu. Local split-screen co-op is a campaign (bLonePlayer) played by several
    /// players: like the original's multiplayer split screen, a menu does not stop the game for the
    /// other player, only bIsInMenu changes. None: anything else (solo, network, other callers).
    fn coop_menu_pause(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<Value> {
        let function = &self.functions[f];
        if function.name != "SetPause" || self.engine.client.viewports.len() < 2 {
            return None;
        }
        if !self.is_a(this, "PlayerController") {
            return None;
        }
        let caller = *self.stack.last()?;
        let caller_fn = &self.functions[caller];
        if !(matches!(caller_fn.name.as_str(), "BeginState" | "EndState") && self.classes[caller_fn.owner].name.eq_ignore_ascii_case("XIIIRootWindow")) {
            return None;
        }
        let pause = args.first().and_then(|a| a.as_ref()).is_some_and(Value::as_bool);
        self.set_prop(this, "bIsInMenu", Value::Bool(pause));
        Some(Value::Bool(true))
    }

    /// XIIIRootWindow.OpenMenu of a gamepad player's menu (split screen, not the mouse player's,
    /// gui.rs gui_without_mouse): the menu's platform (CurrentPF, 0 on PC since BeginState) becomes
    /// the console one before the page is built, so that its footer shows the gamepad's button
    /// pictures (XIIIWindow.Created: tButton[CurrentPF * 2]) and its Controls entry opens the
    /// console Controls page (XIIIMenuInGame: XIIIMenuInGameControlsWindow when CurrentPF > 0), which
    /// the PC game does not ship: that one opens the PC Controls page instead (the page with the
    /// Gamepad entry). Some(result) when the call was redirected.
    fn coop_menu_platform(&mut self, f: FuncId, this: ObjId, args: &[Option<Value>]) -> Option<Value> {
        if self.functions[f].name != "OpenMenu" || !self.is_a(this, "XIIIRootWindow") || !self.gui_without_mouse(this) {
            return None;
        }
        if self.prop(this, "CurrentPF").as_int() != CONSOLE_PLATFORM {
            self.set_prop(this, "CurrentPF", Value::Int(CONSOLE_PLATFORM));
        }
        let page = match args.first() {
            Some(Some(Value::Str(s))) => s.to_string(),
            _ => return None,
        };
        if !page.eq_ignore_ascii_case(CONSOLE_CONTROLS_PAGE) {
            return None;
        }
        let mut pc_args = args.to_vec();
        pc_args[0] = Some(Value::Str(PC_CONTROLS_PAGE.into()));
        self.call(f, this, pc_args).ok().map(|(v, _)| v)
    }

    /// Script calls that co-op answers itself. `None`: the script runs as usual.
    pub(crate) fn coop_answers(&mut self, f: FuncId, _this: ObjId, args: &[Option<Value>]) -> Option<Value> {
        if self.coop_keeps_game(f, args) || self.coop_waits_respawn(f, args) {
            return Some(Value::Void);
        }
        if let Some(v) = self.coop_menu_pause(f, _this, args) {
            return Some(v);
        }
        if let Some(v) = self.coop_menu_platform(f, _this, args) {
            return Some(v);
        }
        let function = &self.functions[f];
        if function.name == "FindPlayerController" && self.classes[function.owner].name.eq_ignore_ascii_case("MapInfo") {
            return self.coop.players.first().map(|&pc| Value::Object(Some(pc)));
        }
        None
    }
}

/// Trigger threshold on the shared Z axis (left minus right).
const TRIGGER: i32 = 16384;
/// Faithful build: movement stick threshold of a co-op player's gamepad (DIJOYSTATE units) for
/// W/S/A/D.

/// Keys of a co-op player's gamepad for one frame: (key, held) in the order they are compared,
/// wheel notches to send, and this frame's button word (gamepad_config::word layout). `before`:
/// the previous frame's word; `versus`: a multiplayer game, where Back shows the scores (F1)
/// whatever its action. The Start button is not here: it opens this player's menu
/// (coop_pad_input).
pub(crate) fn coop_pad_keys(settings: &crate::gamepad_config::PlayerPad, state: &crate::pad::PadState, before: u16, versus: bool) -> (Vec<(u8, bool)>, Vec<&'static str>, u16) {
    use crate::gamepad_config::Button;
    let [lx, ly, lz, ..] = state.axes;
    let b = state.buttons;
    let mut word: u16 = (0..14).filter(|&i| b[i]).fold(0, |w, i| w | 1 << i);
    if lz > TRIGGER {
        word |= Button::LeftTrigger.bit();
    }
    if lz < -TRIGGER {
        word |= Button::RightTrigger.bit();
    }
    let scores = versus && b[6];
    if versus {
        word &= !Button::Back.bit();
    }
    let out = settings.outputs(word, before);
    let held = |k: u8| out.keys.contains(&k);
    // Improved build: the left stick walks as far as it is pushed (coop_pad_input, analog_move),
    // W / S / A / D stay released; the faithful build presses them past a threshold.
    let [up, down, left, right] = crate::improved::input::movement_keys(lx, ly);
    let keys = vec![
        (0x57u8, up),
        (0x53, down),
        (0x41, left),
        (0x44, right),
        (0x20, held(0x20)),
        (0x43, held(0x43)),
        (0x52, held(0x52)),
        (0x45, held(0x45)),
        (0x51, held(0x51)),
        (0x70, scores),
        (0xA0, held(0xA0)),
        (0x46, held(0x46)),
        (0x47, held(0x47)),
        // Fire and alt fire: the left and right mouse buttons (IK_LeftMouse, IK_RightMouse).
        (0x01, out.mouse[0]),
        (0x02, out.mouse[1]),
    ];
    let wheel = out.wheel.iter().map(|&n| if n < 0 { "MouseWheelDown" } else { "MouseWheelUp" }).collect();
    (keys, wheel, word)
}

#[cfg(test)]
mod pad_keys_tests {
    use super::*;
    use crate::gamepad_config::{Button, PlayerPad};

    /// The table coop_pad_input had before the gamepad settings: keys in order, wheel notches.
    fn former(state: &crate::pad::PadState, before: [bool; 2], versus: bool) -> (Vec<(u8, bool)>, Vec<&'static str>) {
        let [lx, ly, lz, ..] = state.axes;
        let b = state.buttons;
        // Improved build: the left stick walks through analog_move, W / S / A / D stay released.
        const STICK: i32 = 11469;
        let keys_stick = !cfg!(feature = "improved");
        let keys = vec![
            (0x57u8, keys_stick && ly < -STICK),
            (0x53, keys_stick && ly > STICK),
            (0x41, keys_stick && lx < -STICK),
            (0x44, keys_stick && lx > STICK),
            (0x20, b[0]),
            (0x43, b[1]),
            (0x52, b[2]),
            (0x45, b[3]),
            (0x51, b[10] || (b[6] && !versus)),
            (0x70, b[6] && versus),
            (0xA0, b[8]),
            (0x46, b[12]),
            (0x47, b[13]),
            (0x01, lz < -TRIGGER),
            (0x02, lz > TRIGGER),
        ];
        let mut wheel = Vec::new();
        for (i, name) in ["MouseWheelDown", "MouseWheelUp"].into_iter().enumerate() {
            if [b[4], b[5]][i] && !before[i] {
                wheel.push(name);
            }
        }
        (keys, wheel)
    }

    /// Default settings: exactly the former table, frame after frame (buttons, sticks and
    /// triggers around their thresholds, co-op and versus).
    #[test]
    fn default_settings_are_the_former_table() {
        let pad = PlayerPad::default();
        let mut seed = 0x0C00_9A11u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let near = [-32767, -16385, -16384, -11470, -11469, 0, 11469, 11470, 16384, 16385, 32767];
        let (mut before_word, mut before_bumpers) = (0u16, [false; 2]);
        for i in 0..3000 {
            let mut state = crate::pad::PadState::default();
            for k in 0..14 {
                state.buttons[k] = next() % 3 == 0;
            }
            for a in 0..3 {
                state.axes[a] = near[(next() as usize) % near.len()];
            }
            let versus = i % 2 == 0;
            let (keys, wheel, word) = coop_pad_keys(&pad, &state, before_word, versus);
            assert_eq!((keys, wheel), former(&state, before_bumpers, versus), "frame {i}");
            before_word = word;
            before_bumpers = [state.buttons[4], state.buttons[5]];
        }
    }

    /// Negative check: A and B exchanged gives other keys (Space on B, C on A).
    #[test]
    fn remapped_differs() {
        let mut pad = PlayerPad::default();
        pad.swap(Button::A, Button::B);
        let mut state = crate::pad::PadState::default();
        state.buttons[0] = true;
        let (keys, _, _) = coop_pad_keys(&pad, &state, 0, false);
        assert_ne!(keys, former(&state, [false; 2], false).0);
        assert!(keys.contains(&(0x43, true)) && keys.contains(&(0x20, false)));
    }
}
