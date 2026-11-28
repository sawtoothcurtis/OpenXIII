//! "Select level" of the game launched alone: each start (beginning of a map or checkpoint) is the
//! loading of a real checkpoint, by the same path as "Load" or "Retry" in
//! the original: the record is written as XIIISaveGameTrigger.DoSave then SaveAtCheckpoint
//! write it, stored in USaveGameDevice (+0x30), then LaunchFromCheckpoint
//! on the device's Tick, LoadFromCheckpoint sets the
//! UAudioSaver flag, Browse loads the map at the portal (FindPlayerStart: Teleporter with this Tag, otherwise the
//! PlayerStart), AcceptInventory reads the record back (health, speed, number, checkpoint
//! music through PlayMusic, CleanMap and the trigger's objectives), LoadMap returns the music to
//! LevelInfo.InitMusic and PlayMusicInit plays it.
//!
//! select_level.tsv (optional data file, see game_data.rs): for each start, the Tag, the music (SoundToLaunch) and the
//! description of its XIIISaveGameTrigger, the checkpoint number, the position of the
//! Teleporter / PlayerStart, the map's MusicVars and the inventory that the progression guarantees
//! at this point (InitialInv of the MapInfos, items given by cutscenes and map scripts, what carries over from one
//! map to the next: DoTravel, NextMapKeepInventory). Optional items picked up while playing are not
//! included. Keys a map's script hands to a guard (picked on the guard's body before the checkpoint) are
//! included with their KeyCodeName: the original's checkpoint record has them.

use crate::save::{checkpoint_raw, store_checkpoint, AudioSaver};
use crate::value::{ObjId, Value};
use crate::Vm;

/// Optional data file (game_data.rs); without it there is no start and the menu is unavailable.
const TABLE_FILE: &str = "select_level/select_level.tsv";

/// True when the Select level table was found (optional game data, see game_data.rs).
pub fn available() -> bool {
    crate::game_data::file(TABLE_FILE).is_some()
}

/// Message shown when the table is missing.
pub const UNAVAILABLE: &str = "Select level is unavailable: its optional data file select_level/select_level.tsv was not found (set OXIII_DATA to a folder that contains it)";

/// A Select level start (one line of select_level.tsv).
#[derive(Debug, Clone)]
pub struct Start {
    pub map_value: String,
    /// TeleporterName of the trigger ("PlayerStart" for the beginning of the map).
    pub portal_value: String,
    /// Tag of the XIIISaveGameTrigger ("None": trigger without a Tag in the map).
    pub tag: String,
    /// SoundToLaunch of the trigger (empty: none).
    pub sound_value: String,
    pub description: String,
    /// GameInfo.CheckpointNumber after DoSave's increment (1: beginning of the map).
    pub number: i32,
    /// Teleporter (or PlayerStart) of the map: position and yaw.
    pub position: [f32; 3],
    pub yaw: i32,
    /// LevelInfo.MusicVars of the map (names).
    pub music_vars: Vec<String>,
    /// Guaranteed inventory, in the order it is given (MapInfo.GiveSomething).
    pub objects_list: Vec<String>,
    /// Objectives at the beginning of the map, in ObjectivesState format (MapInfo.PostBeginPlay).
    pub objectives_start: String,
    /// Objectives at this checkpoint, in ObjectivesState format: those the original wrote at the
    /// same point of a reference game (empty at the beginning of a map).
    pub objectives_checkpoint: String,
}

/// ObjectivesState text of (bCompleted, bPrimary, bAntiGoal) objectives; empty for none.
pub fn objectives_text(goals_list: &[(bool, bool, bool)]) -> String {
    if goals_list.is_empty() {
        return String::new();
    }
    let text_value: Vec<String> = goals_list
        .iter()
        .map(|&(c, p, a)| {
            let fields_list: Vec<&str> = [(c, "bCompleted=True"), (p, "bPrimary=True"), (a, "bAntiGoal=True")].iter().filter(|x| x.0).map(|x| x.1).collect();
            format!("({})", fields_list.join(","))
        })
        .collect();
    format!("({})", text_value.join(","))
}

/// All the starts, in campaign order (none without the optional table).
pub fn starts_list() -> Vec<Start> {
    crate::game_data::file(TABLE_FILE)
        .unwrap_or("")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            let list_value = |s: &str| s.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect::<Vec<_>>();
            Some(Start {
                map_value: c.first()?.to_string(),
                portal_value: c.get(1)?.to_string(),
                tag: c.get(2)?.to_string(),
                sound_value: c.get(3)?.to_string(),
                description: c.get(4)?.to_string(),
                number: c.get(5)?.parse().ok()?,
                position: [c.get(6)?.parse().ok()?, c.get(7)?.parse().ok()?, c.get(8)?.parse().ok()?],
                yaw: c.get(9)?.parse().ok()?,
                music_vars: list_value(c.get(10)?),
                objects_list: list_value(c.get(11).copied().unwrap_or("")),
                objectives_start: c.get(12).copied().unwrap_or("").to_string(),
                objectives_checkpoint: c.get(13).copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// A game weapon or item for the test loadout of Select level: class, displayed name, ammo
/// (primary then secondary: the class's AmmoName and AltAmmoName). Grenades are those of the
/// pickup GrenadPick (GrenadB), the only grenade of the single-player maps (FGrenadB, the frag
/// grenade, is only placed in a multiplayer map).
pub struct Weapon {
    pub class_value: &'static str,
    pub name_value: &'static str,
    /// Where the game localises its name: section and key of XIII.<language> (the pickup's
    /// PickupMessage, or the item's sItemName); `name_value` is the English fallback.
    pub label: (&'static str, &'static str),
    pub ammo: &'static [&'static str],
}

pub const WEAPONS: &[Weapon] = &[
    Weapon { class_value: "XIII.Beretta", name_value: "9mm pistol", label: ("BerettaPick", "PickupMessage"), ammo: &["XIII.c9mmAmmo"] },
    Weapon { class_value: "XIII.magnum", name_value: "44 Special", label: ("MagnumPick", "PickupMessage"), ammo: &["XIII.c44Ammo"] },
    Weapon { class_value: "XIII.m16", name_value: "Assault rifle (M16)", label: ("M16Pick", "PickupMessage"), ammo: &["XIII.M16Ammo", "XIII.M16GrenadAmmo"] },
    Weapon { class_value: "XIII.kalash", name_value: "Kalash", label: ("KalashPick", "PickupMessage"), ammo: &["XIII.KalashAmmo"] },
    Weapon { class_value: "XIII.m60", name_value: "SMG (M60)", label: ("M60Pick", "PickupMessage"), ammo: &["XIII.M60Ammo"] },
    Weapon { class_value: "XIII.MiniUzi", name_value: "Mini Uzi", label: ("MiniUziPick", "PickupMessage"), ammo: &["XIII.MiniUziAmmo"] },
    Weapon { class_value: "XIII.FusilPompe", name_value: "Shotgun", label: ("FusilPompePick", "PickupMessage"), ammo: &["XIII.PompeAmmo", "XIII.ShotGunAltAmmo"] },
    Weapon { class_value: "XIII.FusilChasse", name_value: "Hunting gun", label: ("FusilChassePick", "PickupMessage"), ammo: &["XIII.PompeAmmo", "XIII.HuntGunAltAmmo"] },
    Weapon { class_value: "XIII.FusilSnipe", name_value: "Sniper rifle", label: ("FusilSnipePick", "PickupMessage"), ammo: &["XIII.bmg50Ammo"] },
    Weapon { class_value: "XIII.arbalete", name_value: "Crossbow", label: ("ArbaletePick", "PickupMessage"), ammo: &["XIII.BoltAmmo"] },
    Weapon { class_value: "XIII.ArbaleteX3", name_value: "Crossbow X3", label: ("ArbaleteX3Pick", "PickupMessage"), ammo: &["XIII.BoltAmmo"] },
    Weapon { class_value: "XIII.LHarpon", name_value: "Harpoon gun", label: ("LHarponPick", "PickupMessage"), ammo: &["XIII.HarponAmmo", "XIII.HarponAltAmmo"] },
    Weapon { class_value: "XIII.Bazook", name_value: "Bazooka", label: ("BazookPick", "PickupMessage"), ammo: &["XIII.BazookAmmo", "XIII.BazookRocketAltAmmo"] },
    Weapon { class_value: "XIII.GrenadB", name_value: "Grenades", label: ("GrenadPick", "PickupMessage"), ammo: &["XIII.GrenadAmmo"] },
    Weapon { class_value: "XIII.TKnife", name_value: "Throwing knives", label: ("TKnifePick", "PickupMessage"), ammo: &["XIII.TKnifeAmmo", "XIII.TKnifeAltAmmo"] },
    // Items (XIIIItems, Keys, Med, Armor), given the same way.
    Weapon { class_value: "XIII.Hook", name_value: "Grappling hook", label: ("Hook", "sItemName"), ammo: &[] },
    Weapon { class_value: "XIII.LockPick", name_value: "Lockpick", label: ("LockPick", "sItemName"), ammo: &[] },
    Weapon { class_value: "XIII.MedKit", name_value: "Medkit", label: ("MedKit", "sItemName"), ammo: &[] },
    Weapon { class_value: "XIII.FullMedKit", name_value: "Full medkit", label: ("FullMedKit", "sItemName"), ammo: &[] },
    Weapon { class_value: "XIII.GiletMk1", name_value: "Armor vest", label: ("GiletMk1Pick", "PickupMessage"), ammo: &[] },
    Weapon { class_value: "XIII.Casque", name_value: "Helmet", label: ("CasquePick", "PickupMessage"), ammo: &[] },
];

/// Default test loadout: that of the coop mod (Beretta and M16).
pub const WEAPONS_DEFAULT: u32 = 0b101;

/// Every weapon of `WEAPONS` checked.
pub const WEAPONS_ALL: u32 = (1 << WEAPONS.len()) - 1;

/// Classes to give for the checked weapons (one bit per line of `WEAPONS`): each weapon then its
/// ammo, without duplicates.
pub fn weapon_classes(mask: u32) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for a in WEAPONS.iter().enumerate().filter(|(i, _)| mask & 1 << i != 0).map(|(_, a)| a) {
        for c in std::iter::once(a.class_value).chain(a.ammo.iter().copied()) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// Start from a menu URL: "map" (beginning of the map) or "map#portal".
pub fn start_point(url: &str) -> Option<Start> {
    let (map_value, portal_value) = url.split_once('#').unwrap_or((url, "PlayerStart"));
    starts_list().into_iter().find(|d| d.map_value.eq_ignore_ascii_case(map_value) && d.portal_value.eq_ignore_ascii_case(portal_value))
}

impl Vm {
    /// MapInfo.GiveSomething (XIII MapInfo.uc): an item the pawn lacks is created at its position and given
    /// (GiveTo); ammunition already there receives AddAmmo(default.AmmoAmount).
    pub(crate) fn give_as_mapinfo(&mut self, pawn_value: ObjId, class_value: &str) {
        let Some(c) = self.class(class_value) else {
            self.warn(format!("Select level: class {class_value} not found"));
            return;
        };
        let already = self.call_named(pawn_value, "FindInventoryType", vec![Value::Class(Some(c))]).ok().and_then(|v| v.as_object());
        match already {
            None => {
                let (l, r) = (self.prop(pawn_value, "Location"), self.prop(pawn_value, "Rotation"));
                if let Ok(Some(o)) = self.spawn_actor(c, Value::Object(None), None, l, r, Value::Object(None), true) {
                    let _ = self.call_named(o, "GiveTo", vec![Value::Object(Some(pawn_value))]);
                }
            }
            Some(o) if self.is_a(o, "Ammunition") => {
                let cdo = self.classes[c].cdo;
                let n = self.prop(cdo, "AmmoAmount");
                let _ = self.call_named(o, "AddAmmo", vec![n]);
            }
            Some(_) => {}
        }
    }

    /// One item of a start's guaranteed inventory: `Class`, or `Class/KeyCodeName/Section.Key` for a key the
    /// map's script hands to a character (PRock01a.FirstFrame: GiveSomething to the guard, then
    /// Keys.KeyCodeName and Inventory.ItemName, both travel variables, so the original's checkpoint record
    /// carries them). ItemName is the map script's localized string ([Section] Key of the map's package).
    pub(crate) fn give_item_select_level(&mut self, pawn_value: ObjId, map_value: &str, item: &str) {
        let mut parts = item.split('/');
        let class_value = parts.next().unwrap_or_default();
        self.give_as_mapinfo(pawn_value, class_value);
        let Some(code) = parts.next() else { return };
        let Some(c) = self.class(class_value) else { return };
        let Some(inv) = self.call_named(pawn_value, "FindInventoryType", vec![Value::Class(Some(c))]).ok().and_then(|v| v.as_object()) else { return };
        self.set_prop(inv, "KeyCodeName", Value::Str(code.to_string()));
        if let Some((section, key)) = parts.next().and_then(|n| n.split_once('.')) {
            let text_value = self.localize(section, key, map_value, true);
            if !text_value.is_empty() {
                self.set_prop(inv, "ItemName", Value::Str(text_value));
            }
        }
    }

    /// Property `name_value` of `obj` read back from its text (UProperty::ImportText).
    pub(crate) fn place_text(&mut self, obj: ObjId, name_value: &str, text_value: &str) {
        let Some(class) = self.objects.get(obj).and_then(|o| o.as_ref()).map(|o| o.class) else { return };
        let Some(p) = self.classes[class].props.iter().find(|p| p.name.eq_ignore_ascii_case(name_value)).cloned() else { return };
        let chars: Vec<char> = text_value.chars().collect();
        let mut slot = self.prop(obj, &p.name);
        if self.import_text(&p.ty, p.enumeration.as_deref(), &chars, 0, 0, &mut slot).is_some() {
            self.set_prop(obj, &p.name, slot);
        }
    }

    /// Record of start `d` for player `pc` (their pawn), written as the original writes it when
    /// passing this point: the pawn's inventory becomes the guaranteed inventory (left hand kept; default
    /// weapon left to AddDefaultInventory, as at load time), then DoSave's XIIIThingsToSave
    /// (Tag, health, speed, number, music, objectives), then SaveAtCheckpoint towards the start's
    /// map. Returns the compressed checkpoint (USaveGameDevice +0x30).
    pub fn sheet_select_level(&mut self, pc: ObjId, d: &Start) -> Option<Vec<u8>> {
        let pawn_value = self.propr(pc, "Pawn").as_object()?;
        // The menu's inventory is not the start's (fists and left hand kept).
        let mut objects_list = Vec::new();
        let mut o = self.propr(pawn_value, "Inventory").as_object();
        while let Some(i) = o.filter(|&i| self.objects.get(i).is_some_and(|x| x.is_some())) {
            o = self.propr(i, "Inventory").as_object();
            // The fists stay in hand, as when the original passes this point after starting the
            // map: the default weapon (fists) is given and selected first, and the map's weapons
            // given after it only go into the inventory (Weapon.ClientWeaponSet(true) returns
            // for a human player who holds a weapon). Without them, the first weapon given
            // became Pawn.Weapon and travelled in hand.
            let fists = (self.is_a(i, "Fists") || self.is_a(i, "FistsAmmo"));
            if !self.is_a(i, "XIIILeftHand") && !fists {
                objects_list.push(i);
            }
            if objects_list.len() > 256 {
                break;
            }
        }
        for i in objects_list {
            let _ = self.destroy_actor(i);
        }
        for item in &d.objects_list {
            self.give_item_select_level(pawn_value, &d.map_value, item);
        }
        // XIIISaveGameTrigger.DoSave: XIIIThingsToSave given to the pawn, then its fields.
        let class_value = self.class("XIII.XIIIThingsToSave")?;
        let (l, r) = (self.prop(pawn_value, "Location"), self.prop(pawn_value, "Rotation"));
        let s = self.spawn_actor(class_value, Value::Object(None), None, l, r, Value::Object(None), true).ok()??;
        let _ = self.call_named(s, "GiveTo", vec![Value::Object(Some(pawn_value))]);
        self.set_prop(s, "XIIISaveGameTriggerTag", Value::Name(crate::value::NameStruct::from(d.tag.clone())));
        // Health and speed of a new pawn (default XIIIPlayerPawn): those of the progression without
        // damage; the menu's pawn has SpeedFactorLimit 0 (otherwise the player would not move).
        let cdo = self.class("XIII.XIIIPlayerPawn").map(|c| self.classes[c].cdo);
        if let Some(cdo) = cdo {
            let h = self.prop(cdo, "Health");
            self.set_prop(s, "Health", h);
        }
        self.set_prop(s, "SpeedFactorLimit", Value::Float(1.0));
        self.set_prop(s, "CheckpointNumber", Value::Int(d.number));
        if !d.sound_value.is_empty() {
            self.set_prop(s, "SoundToLaunch", Value::Asset(d.sound_value.clone()));
        }
        // Objectives of the map at this point: those the original wrote at the same point in the
        // reference game (the main objectives of the progression); at the beginning of a map,
        // those of the map (MapInfo.PostBeginPlay).
        let objectives = if d.objectives_checkpoint.is_empty() || d.portal_value.eq_ignore_ascii_case("PlayerStart") {
            d.objectives_start.clone()
        } else {
            d.objectives_checkpoint.clone()
        };
        if !objectives.is_empty() {
            self.place_text(s, "ObjectivesState", &objectives);
        }
        // SaveAtCheckpoint: the pawn then its inventory chain, the player name, then UAudioSaver
        // (name emptied and parameters at −1 outside loading; the map's MusicVars, at rest).
        let mut text_value = String::new();
        self.export_travel(pawn_value, &mut text_value);
        let mut inv = self.propr(pawn_value, "Inventory").as_object();
        let mut kept = 0;
        while let (Some(i), true) = (inv, kept < 256) {
            self.export_travel(i, &mut text_value);
            inv = self.propr(i, "Inventory").as_object();
            kept += 1;
        }
        let name_value = self.propr(pc, "PlayerReplicationInfo").as_object().map(|r| self.propr(r, "PlayerName").as_name()).unwrap_or_default();
        let saver = AudioSaver { vars: d.music_vars.iter().map(|n| (n.clone(), 0, 0)).collect(), ..Default::default() };
        let raw_value = checkpoint_raw(&d.map_value, &d.portal_value, &name_value, &text_value, &saver);
        let dev = self.engine.save.as_mut()?;
        if !store_checkpoint(dev, &raw_value, &d.portal_value, &d.description) {
            return None;
        }
        // Like OperationReadSlot when loading a save: the map's starting point
        // becomes this checkpoint ("Restart level" goes back to it), instead of
        // a starting point from a previous game.
        dev.start = dev.checkpoint.clone();
        dev.start_desc = dev.checkpoint_desc.clone();
        Some(dev.checkpoint.clone())
    }

    /// Start chosen in Select level from the main menu: the start's record stored in the
    /// save device, then LaunchFromCheckpoint like "Load"; the menu pages
    /// closed as when a game is launched (myRoot.CloseAll(true), myRoot.GotoState('')). The
    /// travel starts at the device's next Tick (LoadFromCheckpoint).
    pub fn from_menu_main(&mut self, url: &str) -> bool {
        let Some(page) = crate::improved::navigation::level_selection_page(self) else {
            let page = self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object()).and_then(|p| self.objects.get(p)?.as_ref().map(|o| o.name.clone()));
            self.log.push(format!("Log: Select level: main menu not in front (page {page:?})"));
            return false;
        };
        let Some(pc) = self.local_viewport().and_then(|v| self.propr(v, "Actor").as_object()) else {
            self.log.push("Log: Select level: no local player".into());
            return false;
        };
        let Some(d) = start_point(url) else {
            self.log.push(format!("Log: Select level: start {url} missing from select_level.tsv"));
            return false;
        };
        if self.sheet_select_level(pc, &d).is_none() {
            let pawn_value = self.propr(pc, "Pawn").as_object();
            self.log.push(format!("Log: Select level: record of {url} impossible (pawn: {pawn_value:?})"));
            return false;
        }
        let launches = self.engine.save.as_mut().is_some_and(|dev| dev.launch_from_checkpoint(0, 3));
        if launches {
            if let Some(root) = self.propr(page, "myRoot").as_object() {
                let _ = self.call_named(root, "CloseAll", vec![Value::Bool(true)]);
                let _ = self.goto_state(root, "", "");
            }
        }
        launches
    }
}
