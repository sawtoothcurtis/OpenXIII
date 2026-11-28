//! Mods build only (feature `mods`): weapon settings that are not properties of the game's
//! classes, written in the same `[class."Package.Class"]` and `[map.<Map>.class."..."]` tables
//! (docs/modding/README.md, "Weapon animation speeds").
//!
//! The game plays the first-person animations of a weapon at a rate written in its scripts
//! (`PlayAnim('Fire', 1.0)`), and the weapon waits for their end: the next shot comes when the
//! firing animation ends (`NormalFire.AnimEnd` calls `Finish`), the magazine is full when the
//! reload animation ends, the weapon is in hand when the select animation ends. The player's rate
//! of fire, reload time and weapon change time are therefore the lengths of these animations.
//!
//! - `FireAnimRate`, `ReloadAnimRate`, `SelectAnimRate`: multipliers of the rate of the
//!   animations a weapon of the class plays in `PlayFiring`/`PlayAltFiring` (and the other firing
//!   functions), in `PlayReloading`, and in `PlaySelect`/`TweenDown`. 1.0 is the original game;
//!   2.0 plays them twice as fast.
//!
//! Which animation is which comes from the script function that calls the PlayAnim native, the
//! same functions for every weapon of the game. Without mods, or without these values, nothing is
//! changed: the native gets the script's own sequence and rate.
//!
//! - `FirstPersonFrom = "XIII.m16"`: the weapon keeps its scripts (its behaviour) but is seen in
//!   first person as another weapon: that weapon's hands and gun mesh (`MeshName`), place in front
//!   of the eye and muzzle flash are copied into the class defaults. The scripts still play their
//!   own animation names; a name the new mesh does not have is played as the closest one it has
//!   (`FireLast` as `Fire`, `WaitActVide` as `WaitAct` then `Wait`, `Fire` as `Fire1`…), or as
//!   given by `FirstPersonAnimations = { FireLast = "Fire" }`.

use openxiii_mods::{DefaultOverride, ModValue};

use crate::value::{ClassId, ObjId, Value};
use crate::Vm;

/// First-person look of another weapon.
pub const FIRST_PERSON_FROM: &str = "FirstPersonFrom";
/// Animation names: the name the scripts play = the name of the first-person mesh.
pub const FIRST_PERSON_ANIMATIONS: &str = "FirstPersonAnimations";

/// The class defaults that make a weapon's first-person look (Weapon.uc, XIIIWeapon.uc): its
/// hands and gun mesh, its place in front of the eye, where its shots start, its muzzle flashes.
const LOOK: &[&str] = &[
    "MeshName",
    "PlayerViewOffset",
    "DrawScale",
    "FireOffset",
    "AltFireOffset",
    "FirstPersonMFClass",
    "FPMFRelativeLoc",
    "FPMFRelativeRot",
    "bDrawMuzzleFlash",
    "MuzzleScale",
    "MuzzleFlashSize",
    "MFTexture",
    "FlashOffsetX",
    "FlashOffsetY",
    "FlashLength",
];

/// Firing animations speed (rate of fire).
pub const FIRE_ANIM_RATE: &str = "FireAnimRate";
/// Reload animation speed.
pub const RELOAD_ANIM_RATE: &str = "ReloadAnimRate";
/// Select (draw) and put down animations speed.
pub const SELECT_ANIM_RATE: &str = "SelectAnimRate";

/// Settings read by mods and not by the game's scripts: they are not written into the class
/// defaults (the classes have no such property).
pub(crate) const MOD_ONLY: &[&str] = &[
    FIRE_ANIM_RATE,
    RELOAD_ANIM_RATE,
    SELECT_ANIM_RATE,
    FIRST_PERSON_FROM,
    FIRST_PERSON_ANIMATIONS,
    HUD_ICON,
    FIRST_PERSON_MODEL,
    FIRST_PERSON_MODEL_BONE,
    FIRST_PERSON_MODEL_OFFSET,
    FIRST_PERSON_MODEL_ROTATION,
    FIRST_PERSON_MODEL_SCALE,
    FIRST_PERSON_HIDE_BONES,
    TRACER_COLOR,
    IMPACT_EFFECTS_BY_SURFACE,
];

/// Colour of the tracer a weapon draws in first person ({ R, G, B, A }, 0 to 255). The game
/// draws it with a fixed orange colour (XIIIWeapon.DrawTraceBullet: DrawColor 255, 200, 100),
/// which tints the ammo's TBTexture; white draws the texture with its own colours.
pub const TRACER_COLOR: &str = "TracerColor";

/// `false` on an ammo, projectile or weapon attachment class: the impact effects of its class
/// values (ImpactEmitterMem, and DecalProjectorClass for bullets) are used on every surface. The
/// game otherwise chooses them by the surface hit (SetUpImpactEmitter of XIIIAmmo,
/// XIIIBulletsAmmo, XIIIProjectile and XIIIWeaponAttachment), which is then not run for that
/// class; the impact sound of the surface is still played.
pub const IMPACT_EFFECTS_BY_SURFACE: &str = "ImpactEffectsBySurface";

/// A static mesh held in first person instead of the parent's gun (mod_first_person_model).
pub const FIRST_PERSON_MODEL: &str = "FirstPersonModel";
/// The bone of the hands mesh it is attached to (default: the weapon's GetBaseWeaponBone).
pub const FIRST_PERSON_MODEL_BONE: &str = "FirstPersonModelBone";
/// Its place relative to that bone, { X, Y, Z }.
pub const FIRST_PERSON_MODEL_OFFSET: &str = "FirstPersonModelOffset";
/// Its rotation relative to that bone, { Pitch, Yaw, Roll } (65536 = a full turn).
pub const FIRST_PERSON_MODEL_ROTATION: &str = "FirstPersonModelRotation";
/// Its scale (DrawScale).
pub const FIRST_PERSON_MODEL_SCALE: &str = "FirstPersonModelScale";
/// Bones of the hands mesh shrunk to nothing (the parent's gun).
pub const FIRST_PERSON_HIDE_BONES: &str = "FirstPersonHideBones";
/// Scale of those bones: too small to be seen, not 0 so that they keep their direction. The
/// renderer recognises it to leave their points without an outline (character.rs).
const HIDDEN_BONE_SCALE: f32 = openxiii_assets::mod_skeletal::HIDDEN_BONE_SCALE;

/// The weapon's icon in the HUD (weapon list, pickup message): a texture, "Mod.<Name>" for a
/// mod's PNG.
pub const HUD_ICON: &str = "HudIcon";

/// Weapon slots (InventoryGroup) no weapon of the game uses: the game's weapons use 0 to 17
/// (1 is Weapon.uc's default, the throwing knives' and darts'), 20 and 24 (XIIIMP.MPBomb). The
/// native weapon list (AHUD::execDrawWeaponsList) has 25 places. The HUD icon sheet
/// (XIIIMenu.HUD.Weapon1Icons, 4 icons of 64 × 32 per row) draws a weapon's icon at its slot's
/// place, and the items' icons (IconNumber 18 to 29) in the same places: a new weapon's icon is
/// therefore drawn by weapon, never by place on the sheet (mod_weapon_icon, mod_hud_tile).
pub(crate) const FREE_SLOTS: [u8; 5] = [18, 19, 21, 22, 23];

/// The HUD icon sheet of the weapons (XIIIBaseHud.HudWIcons).
const ICON_SHEET: &str = "weapon1icons";

/// The HUD icon of a mod's new weapon without a HudIcon: a plain grey pistol outline, so that it
/// is never shown with the icon of the game weapon it is made from.
pub const DEFAULT_ICON: &str = "Mod.DefaultWeaponIcon";

/// DEFAULT_ICON's image: 64 × 32 RGBA, one mip level.
pub(crate) fn default_icon() -> Vec<(u32, u32, Vec<u8>)> {
    let (w, h) = (64u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut put = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        rgba[i..i + 4].copy_from_slice(&[210, 210, 210, 255]);
    };
    // Outline of a pistol: barrel and slide (x 12..52, y 8..15), grip (x 36..45, y 15..26).
    let shape = |x: u32, y: u32| ((12..52).contains(&x) && (8..15).contains(&y)) || ((36..45).contains(&x) && (15..26).contains(&y));
    for y in 0..h {
        for x in 0..w {
            let inside = shape(x, y);
            let edge = inside && (!shape(x.wrapping_sub(1), y) || !shape(x + 1, y) || !shape(x, y.wrapping_sub(1)) || !shape(x, y + 1));
            if edge {
                put(x, y);
            }
        }
    }
    vec![(w, h, rgba)]
}

/// Names to try, in order, for an animation `name` the mesh does not have: without the endings
/// the scripts add to a base animation (`Vide` empty magazine, `Last` last round, `Act` active
/// idle), without a variant number (`Fire2`), or with the first variant (`Fire1`).
pub(crate) fn fallback_names(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !s.eq_ignore_ascii_case(name) && !out.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    };
    let mut base = name.to_string();
    loop {
        let lower = base.to_ascii_lowercase();
        let Some(cut) = ["vide", "last", "act"].iter().find(|e| lower.ends_with(*e) && lower.len() > e.len()).map(|e| base.len() - e.len()) else { break };
        base.truncate(cut);
        push(base.clone());
    }
    let no_digit = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if no_digit.len() < name.len() {
        push(no_digit.to_string());
    }
    for b in [name.to_string(), base.clone()] {
        if !b.ends_with(|c: char| c.is_ascii_digit()) {
            push(format!("{b}1"));
        }
    }
    out
}

/// True for a mod-only setting name (any case).
pub(crate) fn is_mod_only(property: &str) -> bool {
    MOD_ONLY.iter().any(|p| p.eq_ignore_ascii_case(property))
}

/// Highest speed accepted: beyond it an animation lasts less than a frame and the game's
/// end-of-animation events would come at every frame anyway.
const MAX_RATE: f64 = 20.0;

/// Which first-person animation of a weapon is being played.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnimKind {
    Fire,
    Reload,
    Select,
}

impl AnimKind {
    fn setting(self) -> &'static str {
        match self {
            AnimKind::Fire => FIRE_ANIM_RATE,
            AnimKind::Reload => RELOAD_ANIM_RATE,
            AnimKind::Select => SELECT_ANIM_RATE,
        }
    }
}

/// The animation kind from the script function that plays it (and its state for BeginState).
/// The functions are the ones of the game's weapon scripts (Weapon.uc, XIIIWeapon.uc and the
/// weapons of XIII and XIIIMP): PlayFiring and PlayAltFiring (every weapon), AltFire (fists'
/// second attack), PlayThrowPrep (grenades armed before the throw); PlayReloading, and the dual
/// pistols' WaitForMasterReload/WaitForSlaveReload states that put one gun down to reload the
/// other; PlaySelect and TweenDown.
pub(crate) fn anim_kind(function: &str, state: Option<&str>) -> Option<AnimKind> {
    let is = |n: &str| function.eq_ignore_ascii_case(n);
    if is("PlayFiring") || is("PlayAltFiring") || is("AltFire") || is("PlayThrowPrep") {
        Some(AnimKind::Fire)
    } else if is("PlayReloading") || (is("BeginState") && state.is_some_and(|s| s.eq_ignore_ascii_case("WaitForMasterReload") || s.eq_ignore_ascii_case("WaitForSlaveReload"))) {
        Some(AnimKind::Reload)
    } else if is("PlaySelect") || is("TweenDown") {
        Some(AnimKind::Select)
    } else {
        None
    }
}

/// A rate setting's value: a number above 0 and at most MAX_RATE.
pub(crate) fn rate_value(v: &ModValue) -> Result<f32, String> {
    let r = match v {
        ModValue::Int(i) => *i as f64,
        ModValue::Float(f) => *f,
        _ => return Err(format!("expects a number (1.0 = the original speed), got {v}")),
    };
    if !(r > 0.0 && r <= MAX_RATE) {
        return Err(format!("{r} is out of range (above 0, at most {MAX_RATE})"));
    }
    Ok(r as f32)
}

impl Vm {
    /// Lower-case class paths from `class` up to Object, most specific first.
    fn mod_class_chain(&self, class: ClassId) -> Vec<String> {
        let mut chain = Vec::new();
        let mut k = Some(class);
        while let Some(c) = k {
            chain.push(self.class_path(c).to_lowercase());
            k = self.classes[c].super_class;
        }
        chain
    }

    /// The value of mod-only setting `setting` for an object of class `class` on the current
    /// map: the class or its closest parent with a value, the current map's tables first.
    pub(crate) fn mod_weapon_setting(&self, class: ClassId, setting: &str) -> Option<DefaultOverride> {
        let t = self.mods.defaults.as_ref()?;
        if !t.has_mod_only() {
            return None;
        }
        let chain = self.mod_class_chain(class);
        let map = self.mod_map_name();
        let find = |list: Option<&Vec<DefaultOverride>>| list.and_then(|l| l.iter().rev().find(|o| o.property.eq_ignore_ascii_case(setting)).cloned());
        // The first-person look is a class default: map tables do not change it.
        let per_map = ![FIRST_PERSON_FROM, FIRST_PERSON_ANIMATIONS, HUD_ICON, FIRST_PERSON_MODEL, FIRST_PERSON_MODEL_BONE, FIRST_PERSON_MODEL_OFFSET, FIRST_PERSON_MODEL_ROTATION, FIRST_PERSON_MODEL_SCALE, FIRST_PERSON_HIDE_BONES].iter().any(|s| setting.eq_ignore_ascii_case(s));
        // The most precise selector first ("Plage01", then "Plage*", "solo", "*").
        if per_map {
            for list in t.map_class_all(&map) {
                for key in &chain {
                    if let Some(o) = find(list.get(key)) {
                        return Some(o);
                    }
                }
            }
        }
        chain.iter().find_map(|key| find(t.class_values(key)))
    }

    /// The PlayAnim/LoopAnim native, mods build: the sequence and rate a weapon plays, with the
    /// mods' animation speeds. Unchanged for anything else.
    pub(crate) fn mod_weapon_anim(&mut self, this: ObjId, sequence: String, rate: f32) -> (String, f32) {
        let Some(t) = self.mods.defaults.as_ref() else { return (sequence, rate) };
        if !t.has_mod_only() || !self.is_a(this, "Weapon") {
            return (sequence, rate);
        }
        let Some(class) = self.objects.get(this).and_then(|o| o.as_ref()).map(|o| o.class) else { return (sequence, rate) };
        let sequence = self.mod_first_person_sequence(this, class, sequence);
        if self.mod_weapon_setting(class, FIRST_PERSON_MODEL).is_some() {
            self.mod_first_person_model(this, class);
            // Placed again on the first idle animation: the pose the model is made for.
            if sequence.to_ascii_lowercase().starts_with("wait") && self.mods.fp_idle.insert(this) {
                if let Some(&att) = self.mods.fp_models.get(&this) {
                    self.mod_first_person_place(this, att, class);
                }
            }
        }
        // The native PlayAnim is on top of the stack, the script function that calls it below.
        let caller = self.stack.len().checked_sub(2).and_then(|i| self.stack.get(i)).map(|&f| &self.functions[f]);
        let Some(kind) = caller.and_then(|f| anim_kind(&f.name, f.state.as_deref())) else { return (sequence, rate) };
        match self.mod_weapon_setting(class, kind.setting()).map(|o| rate_value(&o.value)) {
            Some(Ok(m)) => (sequence, rate * m),
            _ => (sequence, rate),
        }
    }

    /// End of cdo_values: a mod-only setting of class `c`, checked and reported. FirstPersonFrom
    /// writes the other weapon's look into `values` (the defaults of `c`); the others are read
    /// when the weapon plays an animation.
    pub(crate) fn apply_mod_only(&mut self, c: ClassId, values: &mut [Value], o: &DefaultOverride) {
        let class_path = self.class_path(c);
        let result = self.check_mod_only(c, o).and_then(|()| {
            if o.property.eq_ignore_ascii_case(FIRST_PERSON_FROM) {
                self.copy_first_person(c, values, o).map(Some)
            } else {
                Ok(None)
            }
        });
        match result {
            Ok(detail) => self.mod_report(format!("{class_path}.{} = {}{} ({})", o.property_label(), o.value, detail.map(|d| format!(": {d}")).unwrap_or_default(), o.source())),
            Err(why) => self.mod_report(format!("warning: {class_path}.{}: {why} ({}), value ignored", o.property_label(), o.source())),
        }
    }

    /// A mod set FiringMode on weapon class `c` (values: its defaults). In third person the
    /// character's firing, reload and weapon change animations come from the FiringMode of the
    /// weapon's attachment (WeaponAttachment.ThirdPersonEffects, XIIIWeaponAttachment), the way it
    /// is held from the weapon's: the attachment class gets the same value, unless a mod sets its
    /// FiringMode itself.
    pub(crate) fn follow_firing_mode(&mut self, c: ClassId, values: &mut [Value], o: &DefaultOverride) {
        let Some(weapon) = self.class("Engine.Weapon") else { return };
        if !self.is_child_of(c, weapon) {
            return;
        }
        let Some(&slot) = self.classes[c].prop_index.get("attachmentclass") else { return };
        let Some(Value::Class(Some(att))) = values.get(slot).cloned() else { return };
        let att_path = self.class_path(att);
        let own = self.mods.defaults.as_ref().and_then(|t| t.class_values(&att_path.to_lowercase())).is_some_and(|l| l.iter().any(|x| x.property.eq_ignore_ascii_case("FiringMode")));
        if own || !self.classes[att].prop_index.contains_key("firingmode") {
            return;
        }
        self.mods.attach_firing.insert(att, o.clone());
        self.mod_report(format!("{}.FiringMode = {} also on its attachment {att_path}, for the third-person animations ({})", self.class_path(c), o.value, o.source()));
        // Attachment defaults already made: written there too.
        let cdo = self.classes[att].cdo;
        if cdo != usize::MAX {
            let Some(mut props) = self.objects.get_mut(cdo).and_then(|x| x.as_mut()).map(|x| std::mem::take(&mut x.props)) else { return };
            let _ = props.with_slice(|v| self.mod_set_value(att, v, "FiringMode", None, &o.value));
            if let Some(x) = self.objects[cdo].as_mut() {
                x.props = props;
            }
        }
    }

    /// Class defaults of attachment class `c`: the FiringMode of its weapon (follow_firing_mode).
    pub(crate) fn apply_attach_firing_mode(&mut self, c: ClassId, values: &mut [Value]) {
        let Some(o) = self.mods.attach_firing.get(&c).cloned() else { return };
        let _ = self.mod_set_value(c, values, "FiringMode", None, &o.value);
    }

    /// Start of the class defaults of a mod's new weapon class `c`: a weapon slot of its own.
    /// Made from a game weapon, it would keep its parent's slot (InventoryGroup), and the game
    /// shows one weapon per slot (HUD weapon list), draws the slot's icon (the parent's), and
    /// switches between weapons by slot: the new weapon would hide behind its parent. It gets the
    /// first free slot (FREE_SLOTS), unless a mod sets its InventoryGroup.
    pub(crate) fn mod_weapon_slot(&mut self, c: ClassId, values: &mut [Value]) {
        let Some(parent) = self.mod_class_parent(c) else { return };
        let Some(weapon) = self.class("Engine.Weapon") else { return };
        if !self.is_child_of(c, weapon) {
            return;
        }
        let path = self.class_path(c);
        let explicit = self.mods.defaults.as_ref().and_then(|t| t.class_values(&path.to_lowercase())).and_then(|l| l.iter().rev().find(|o| o.property.eq_ignore_ascii_case("InventoryGroup")).cloned());
        let Some(&slot_index) = self.classes[c].prop_index.get("inventorygroup") else { return };
        let slot = match (&explicit, self.mods.weapon_slots.iter().find(|(_, &k)| k == c).map(|(&s, _)| s)) {
            (Some(o), _) => match o.value {
                ModValue::Int(i) if (0..=255).contains(&i) => i as u8,
                _ => return,
            },
            (None, Some(s)) => s,
            (None, None) => {
                let taken: Vec<u8> = self.mods.weapon_slots.keys().copied().collect();
                match FREE_SLOTS.iter().copied().find(|s| !taken.contains(s)) {
                    Some(s) => s,
                    None => {
                        self.mod_report(format!("warning: {path}: no free weapon slot left ({FREE_SLOTS:?} are taken), it shares the slot of {}; set its InventoryGroup", self.class_path(parent)));
                        return;
                    }
                }
            }
        };
        let first = !self.mods.weapon_slots.contains_key(&slot);
        self.mods.weapon_slots.entry(slot).or_insert(c);
        if explicit.is_none() {
            if let Some(v) = values.get_mut(slot_index) {
                *v = Value::Byte(slot);
            }
            if first {
                self.mod_report(format!("{path}: weapon slot {slot} (InventoryGroup) of its own, its parent {} keeps its slot", self.class_path(parent)));
            }
        }
    }

    /// DrawTile of the HUD icon sheet's tile (`u`, `v`, 64 × 32) by the pickup message
    /// (HudMessage.PostRender draws the icon of weapon slot `Switch`, the picked weapon's
    /// InventoryGroup): for the slot of a mod's new weapon, its icon. Only there: the same places
    /// of the sheet are the items' icons elsewhere.
    pub(crate) fn mod_hud_tile(&mut self, material: &Value, u: f32, v: f32, ul: f32, vl: f32) -> Option<(Value, [f32; 4])> {
        if self.mods.weapon_slots.is_empty() || ul != 64.0 || vl != 32.0 || !Self::material_key(material).to_ascii_lowercase().contains(ICON_SHEET) {
            return None;
        }
        // The native DrawTile on top of the stack, the script function that calls it below.
        let caller = self.stack.len().checked_sub(2).and_then(|i| self.stack.get(i)).map(|&f| self.functions[f].owner);
        if !caller.is_some_and(|c| self.classes[c].name.eq_ignore_ascii_case("HudMessage")) {
            return None;
        }
        let (col, row) = ((u / 64.0) as i32, (v / 32.0) as i32);
        if col as f32 * 64.0 != u || row as f32 * 32.0 != v || !(0..4).contains(&col) {
            return None;
        }
        let slot = u8::try_from(row * 4 + col).ok()?;
        self.mod_slot_icon(slot, material)
    }

    /// The icon of weapon `w` in the native weapon list, when it is a mod's new weapon.
    pub(crate) fn mod_weapon_icon(&mut self, w: ObjId, sheet: &Value) -> Option<(Value, [f32; 4])> {
        let class = self.objects.get(w).and_then(|o| o.as_ref()).map(|o| o.class)?;
        let slot = self.mods.weapon_slots.iter().find(|(_, &k)| k == class).map(|(&s, _)| s)?;
        self.mod_slot_icon(slot, sheet)
    }

    /// The icon of weapon slot `slot` when a mod's new weapon has it: (texture, u, v, ul, vl).
    pub(crate) fn mod_slot_icon(&mut self, slot: u8, sheet: &Value) -> Option<(Value, [f32; 4])> {
        let c = *self.mods.weapon_slots.get(&slot)?;
        if let Some(o) = self.mod_weapon_setting(c, HUD_ICON) {
            if let ModValue::Str(name) = &o.value {
                if let Some(Ok(tex)) = self.mod_asset_value("Engine.Texture", name) {
                    let (w, h) = self.material_size(&tex);
                    if w > 0 && h > 0 {
                        return Some((tex, [0.0, 0.0, w as f32, h as f32]));
                    }
                }
            }
        }
        // No icon of its own: the neutral icon (never the one of the weapon it is made from).
        if openxiii_assets::mod_textures::get(DEFAULT_ICON).is_some() {
            return Some((Value::Asset(DEFAULT_ICON.into()), [0.0, 0.0, 64.0, 32.0]));
        }
        // Without the mods' textures (tests): the slot's own tile is left.
        let _ = sheet;
        None
    }

    /// A weapon with a FirstPersonModel, seen in first person: its parent's hands and animations
/// with the parts named by FirstPersonHideBones shrunk to nothing (SetBoneScale 0), and the
/// mod's static mesh attached to a bone of the hands mesh, as the game attaches the 9mm's
/// silencer (XIIIWeapon.UpDateSilencer: BerettaSilencer, an InventoryAttachment, AttachToBone).
/// Done when the weapon plays its first animation (it has its mesh then), once per weapon.
    pub(crate) fn mod_first_person_model(&mut self, weapon: ObjId, class: ClassId) {
        let Some(model) = self.mod_weapon_setting(class, FIRST_PERSON_MODEL) else { return };
        let ModValue::Str(mesh) = &model.value else { return };
        // The parts of the hands mesh to hide, every time (the pose can be rebuilt).
        let hide: Vec<String> = match self.mod_weapon_setting(class, FIRST_PERSON_HIDE_BONES).map(|o| o.value) {
            Some(ModValue::Array(items)) => items.into_iter().filter_map(|v| if let ModValue::Str(s) = v { Some(s) } else { None }).collect(),
            Some(ModValue::Str(s)) => vec![s],
            _ => Vec::new(),
        };
        // Shrunk to almost nothing rather than 0: the model is usually attached to one of these
        // bones (the parent's gun bone), and a bone of scale 0 has no direction left, so the
        // model would keep a fixed world rotation instead of following the hand.
        for (i, bone) in hide.iter().enumerate() {
            if !self.set_bone_scale(weapon, 200 + i as i32, [HIDDEN_BONE_SCALE; 3], bone) {
                self.mod_report(format!("warning: {}: {FIRST_PERSON_HIDE_BONES}: no bone {bone} in its first-person mesh ({})", self.class_path(class), model.source()));
            }
        }
        if self.mods.fp_models.get(&weapon).is_some_and(|&a| self.objects.get(a).is_some_and(|o| o.is_some())) {
            return;
        }
        let Some(att_class) = self.class("XIII.BerettaSilencer") else { return };
        let (l, r) = (self.prop(weapon, "Location"), self.prop(weapon, "Rotation"));
        let Ok(Some(att)) = self.spawn_actor(att_class, Value::Object(Some(weapon)), None, l, r, Value::Object(None), true) else {
            self.mod_report(format!("warning: {}: {FIRST_PERSON_MODEL}: the model could not be attached ({})", self.class_path(class), model.source()));
            return;
        };
        self.mods.fp_models.insert(weapon, att);
        let mesh_value = match self.mod_asset_value("Engine.StaticMesh", mesh) {
            Some(Ok(v)) => v,
            _ => {
                self.mod_report(format!("warning: {}: {FIRST_PERSON_MODEL}: {mesh} is not a static mesh of the mods or the game ({})", self.class_path(class), model.source()));
                return;
            }
        };
        self.set_prop(att, "StaticMesh", mesh_value);
        self.set_prop(att, "DrawType", Value::Byte(8));
        let num = |v: Option<ModValue>, d: f32| match v {
            Some(ModValue::Float(f)) => f as f32,
            Some(ModValue::Int(i)) => i as f32,
            _ => d,
        };
        let scale = num(self.mod_weapon_setting(class, FIRST_PERSON_MODEL_SCALE).map(|o| o.value), 1.0);
        self.set_prop(att, "DrawScale", Value::Float(scale));
        let bone = match self.mod_weapon_setting(class, FIRST_PERSON_MODEL_BONE).map(|o| o.value) {
            Some(ModValue::Str(b)) => b,
            _ => self.call_named(weapon, "GetBaseWeaponBone", vec![]).map(|v| v.as_name().to_string()).unwrap_or_default(),
        };
        let mut args = [Value::Object(Some(att)), Value::Name(crate::value::NameStruct::from(bone.clone()))];
        let _ = crate::natives::call(self, "AttachToBone", weapon, &mut args, &[true, true]);
        self.mod_first_person_place(weapon, att, class);
        self.mod_report(format!("{}: first-person model {mesh} on bone {bone} ({})", self.class_path(class), model.source()));
    }

    /// The first-person model `att` of `weapon` placed on its bone so that its front (+X, a glTF
    /// model made facing forward) points where the weapon points, its origin on the bone; then
    /// moved by FirstPersonModelOffset (X forward, Y right, Z up, game units) and turned by
    /// FirstPersonModelRotation (Pitch, Yaw, Roll, 65536 = a full turn), both in the weapon's
    /// frame (the player's view). The bone's own frame (its axes, its scale) is anyone's guess
    /// for a modder: the relative location and rotation are solved against the pose of the
    /// moment, with the engine's own placement (set_attachment_location), and then follow the
    /// bone as the hand moves. Done when the model is attached and again when the weapon is first
    /// idle (the pose the model is made for).
    pub(crate) fn mod_first_person_place(&mut self, weapon: ObjId, att: ObjId, class: ClassId) {
        use crate::ai_natives::axes;
        let num = |v: &ModValue| match v {
            ModValue::Float(f) => *f as f32,
            ModValue::Int(i) => *i as f32,
            _ => 0.0,
        };
        let field = |v: &Option<ModValue>, k: &str| match v {
            Some(ModValue::Table(f)) => f.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, x)| num(x)).unwrap_or(0.0),
            _ => 0.0,
        };
        let offset = self.mod_weapon_setting(class, FIRST_PERSON_MODEL_OFFSET).map(|o| o.value);
        let rotation = self.mod_weapon_setting(class, FIRST_PERSON_MODEL_ROTATION).map(|o| o.value);
        let user = [field(&rotation, "Pitch").round() as i32, field(&rotation, "Yaw").round() as i32, field(&rotation, "Roll").round() as i32];
        let off = [field(&offset, "X"), field(&offset, "Y"), field(&offset, "Z")];
        // Where the engine puts the model for a relative location and rotation.
        let place = |vm: &mut Vm, rel: [f32; 3], rot: [i32; 3]| -> Option<([f32; 3], [i32; 3])> {
            vm.set_prop(att, "RelativeLocation", Value::vector(rel[0], rel[1], rel[2]));
            vm.set_prop(att, "RelativeRotation", Value::Rotator(rot));
            vm.set_attachment_location(weapon, att).then(|| (vm.prop(att, "Location").as_vector(), vm.prop(att, "Rotation").as_rotator()))
        };
        let Some((l0, r0)) = place(self, [0.0; 3], [0; 3]) else { return };
        // Wanted axes: the weapon's, turned by the setting in the weapon's frame (rows = X, Y, Z).
        let w = axes(self.prop(weapon, "Rotation").as_rotator());
        let u = axes(user);
        let row = |m: &[[f32; 3]; 3], b: &[[f32; 3]; 3], i: usize| -> [f32; 3] { std::array::from_fn(|k| (0..3).map(|j| m[i][j] * b[j][k]).sum()) };
        let want = [row(&u, &w, 0), row(&u, &w, 1), row(&u, &w, 2)];
        let t = |m: &[[f32; 3]; 3]| -> [[f32; 3]; 3] { std::array::from_fn(|i| std::array::from_fn(|j| m[j][i])) };
        let mul = |a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]| -> [[f32; 3]; 3] { [row(a, b, 0), row(a, b, 1), row(a, b, 2)] };
        let a = axes(r0);
        // The relative rotation that gives `want`: one of these products, depending on the order
        // the engine composes them in; the one that lands closest is kept.
        let error = |r: [i32; 3]| -> f32 { axes(r).iter().zip(want.iter()).map(|(p, q)| (0..3).map(|k| (p[k] - q[k]).abs()).sum::<f32>()).sum() };
        let mut best = ([0; 3], error(r0));
        for p in [mul(&want, &t(&a)), mul(&t(&a), &want), mul(&a, &t(&want)), mul(&t(&want), &a)] {
            let r = crate::world::ortho_rotation(p);
            if let Some((_, got)) = place(self, [0.0; 3], r) {
                let e = error(got);
                if e < best.1 {
                    best = (r, e);
                }
            }
        }
        // The relative location that moves the model by `off` in the weapon's frame: the
        // engine's placement is linear in it (probed along each axis, far enough to be measured
        // through a hidden bone's tiny scale).
        let mut rel = [0.0f32; 3];
        if off.iter().any(|x| *x != 0.0) {
            let d: [f32; 3] = std::array::from_fn(|k| (0..3).map(|j| off[j] * w[j][k]).sum());
            let mut j = [[0.0f32; 3]; 3];
            for (col, probe) in [[1.0e4, 0.0, 0.0], [0.0, 1.0e4, 0.0], [0.0, 0.0, 1.0e4]].iter().enumerate() {
                let Some((l, _)) = place(self, *probe, best.0) else { return };
                for k in 0..3 {
                    j[k][col] = (l[k] - l0[k]) / 1.0e4;
                }
            }
            let det = |m: &[[f32; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            let dj = det(&j);
            if dj.abs() > 1.0e-12 {
                for c in 0..3 {
                    let mut m = j;
                    for k in 0..3 {
                        m[k][c] = d[k];
                    }
                    rel[c] = det(&m) / dj;
                }
            }
        }
        let mut loc = [Value::vector(rel[0], rel[1], rel[2])];
        let _ = crate::natives::call(self, "SetRelativeLocation", att, &mut loc, &[true]);
        let mut r = [Value::Rotator(best.0)];
        let _ = crate::natives::call(self, "SetRelativeRotation", att, &mut r, &[true]);
        let _ = self.set_attachment_location(weapon, att);
    }

    /// The weapon class named by a FirstPersonFrom value.
    fn first_person_source(&self, o: &DefaultOverride) -> Result<ClassId, String> {
        let ModValue::Str(name) = &o.value else { return Err(format!("expects a weapon class \"Package.Class\", got {}", o.value)) };
        let weapon = self.class("Engine.Weapon").ok_or("class Weapon not found")?;
        let source = self.class(name).ok_or_else(|| format!("unknown class {name} (or its package is not loaded)"))?;
        if !self.is_child_of(source, weapon) {
            return Err(format!("{name} is not a weapon class"));
        }
        Ok(source)
    }

    /// FirstPersonFrom: the look of the source weapon written into `values`. Ok: what was copied.
    fn copy_first_person(&mut self, c: ClassId, values: &mut [Value], o: &DefaultOverride) -> Result<String, String> {
        let source = self.first_person_source(o)?;
        if source == c || self.is_child_of(source, c) {
            return Err(format!("{} is this class or one of its subclasses", self.class_path(source)));
        }
        self.create_cdo(source);
        let cdo = self.classes[source].cdo;
        for name in LOOK {
            let key = name.to_ascii_lowercase();
            let (Some(&to), Some(_)) = (self.classes[c].prop_index.get(&key), self.classes[source].prop_index.get(&key)) else { continue };
            if let Some(slot) = values.get_mut(to) {
                *slot = self.prop(cdo, name);
            }
        }
        // The mesh itself is loaded from MeshName when it is None (Weapon.uc, BringUp): a class
        // made from a weapon whose mesh is already loaded would otherwise keep that mesh.
        if let Some(&to) = self.classes[c].prop_index.get("mesh") {
            if let Some(slot) = values.get_mut(to) {
                *slot = Value::Object(None);
            }
        }
        Ok(format!("first-person mesh {}", self.prop(cdo, "MeshName")))
    }

    /// The animation a weapon with a borrowed first-person look plays for `sequence`: the name
    /// given by FirstPersonAnimations, the name itself if the mesh has it, else the closest name
    /// it has (fallback_names). A name with no match is played as is (and reported once).
    fn mod_first_person_sequence(&mut self, this: ObjId, class: ClassId, sequence: String) -> String {
        let table = self.mod_weapon_setting(class, FIRST_PERSON_ANIMATIONS);
        let borrowed = self.mod_weapon_setting(class, FIRST_PERSON_FROM).is_some();
        if table.is_none() && !borrowed {
            return sequence;
        }
        if let Some(ModValue::Table(fields)) = table.as_ref().map(|o| &o.value) {
            if let Some((_, ModValue::Str(to))) = fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(&sequence)) {
                return to.clone();
            }
        }
        if self.anim_named(this, &sequence).is_some() {
            return sequence;
        }
        for name in fallback_names(&sequence) {
            if self.anim_named(this, &name).is_some() {
                return name;
            }
        }
        let names: Vec<String> = self.mesh_insts.get(&this).map(|i| i.anims.iter().flat_map(|s| s.seqs.iter().map(|q| q.name.clone())).collect()).unwrap_or_default();
        let mesh = self.mesh_insts.get(&this).map(|i| i.mesh.clone()).unwrap_or_default();
        self.mod_report(format!(
            "warning: {}: first-person animation {sequence} not found in {mesh} (animations: {}); give it in {FIRST_PERSON_ANIMATIONS}",
            self.class_path(class),
            names.join(", ")
        ));
        sequence
    }

    /// Start of a level: the current map's mod-only settings, checked and reported.
    pub(crate) fn report_map_mod_only(&mut self) {
        let Some(t) = self.mods.defaults.clone() else { return };
        if !t.has_mod_only() {
            return;
        }
        let map = self.mod_map_name();
        let list: Vec<DefaultOverride> = t.maps_of(&map).filter(|o| is_mod_only(&o.property)).cloned().collect();
        for o in list {
            let Some(c) = self.class_qualified.get(&o.class_key()).copied() else { continue };
            match self.check_mod_only(c, &o) {
                Ok(()) => self.mod_report(format!("{map}: {}.{} = {} ({})", o.class, o.property_label(), o.value, o.source())),
                Err(why) => self.mod_report(format!("warning: {map}: {}.{}: {why} ({}), value ignored", o.class, o.property_label(), o.source())),
            }
        }
    }

    fn check_mod_only(&self, c: ClassId, o: &DefaultOverride) -> Result<(), String> {
        if o.property.eq_ignore_ascii_case(IMPACT_EFFECTS_BY_SURFACE) {
            if !self.classes[c].prop_index.contains_key("impactemittermem") {
                return Err(format!("{} has no impact effects (ImpactEmitterMem): use it on an ammo, projectile or weapon attachment class", self.class_path(c)));
            }
            if o.index.is_some() {
                return Err(format!("{} is not a static array, write it without [index]", o.property));
            }
            return match o.value {
                ModValue::Bool(_) => Ok(()),
                ref v => Err(format!("expects true or false, got {v}")),
            };
        }
        let weapon = self.class("Engine.Weapon").ok_or("class Weapon not found")?;
        if !self.is_child_of(c, weapon) {
            return Err(format!("{} is not a weapon class", self.class_path(c)));
        }
        if o.index.is_some() {
            return Err(format!("{} is not a static array, write it without [index]", o.property));
        }
        if o.property.eq_ignore_ascii_case(FIRST_PERSON_FROM) {
            if o.map.is_some() {
                return Err(format!("{FIRST_PERSON_FROM} is a class default: write it in a [class.\"...\"] table, not in a map table"));
            }
            return self.first_person_source(o).map(|_| ());
        }
        let is = |n: &str| o.property.eq_ignore_ascii_case(n);
        if [FIRST_PERSON_MODEL, FIRST_PERSON_MODEL_BONE, FIRST_PERSON_MODEL_OFFSET, FIRST_PERSON_MODEL_ROTATION, FIRST_PERSON_MODEL_SCALE, FIRST_PERSON_HIDE_BONES].iter().any(|n| is(n)) {
            if o.map.is_some() {
                return Err(format!("{} goes with the class: write it in a [class.\"...\"] table, not in a map table", o.property));
            }
            let ok = match &o.value {
                ModValue::Str(_) => is(FIRST_PERSON_MODEL) || is(FIRST_PERSON_MODEL_BONE) || is(FIRST_PERSON_HIDE_BONES),
                ModValue::Array(items) => is(FIRST_PERSON_HIDE_BONES) && items.iter().all(|v| matches!(v, ModValue::Str(_))),
                ModValue::Table(fields) => (is(FIRST_PERSON_MODEL_OFFSET) || is(FIRST_PERSON_MODEL_ROTATION)) && fields.iter().all(|(_, v)| matches!(v, ModValue::Int(_) | ModValue::Float(_))),
                ModValue::Int(_) | ModValue::Float(_) => is(FIRST_PERSON_MODEL_SCALE),
                _ => false,
            };
            return if ok {
                Ok(())
            } else {
                Err(format!(
                    "{} = {}: expects {}",
                    o.property,
                    o.value,
                    if is(FIRST_PERSON_HIDE_BONES) {
                        "bone names [\"a\", \"b\"]"
                    } else if is(FIRST_PERSON_MODEL_OFFSET) {
                        "{ X = 0, Y = 0, Z = 0 }"
                    } else if is(FIRST_PERSON_MODEL_ROTATION) {
                        "{ Pitch = 0, Yaw = 0, Roll = 0 }"
                    } else if is(FIRST_PERSON_MODEL_SCALE) {
                        "a number"
                    } else {
                        "a name (string)"
                    }
                ))
            };
        }
        if o.property.eq_ignore_ascii_case(TRACER_COLOR) {
            return tracer_color(&o.value).map(|_| ());
        }
        if o.property.eq_ignore_ascii_case(HUD_ICON) {
            if o.map.is_some() {
                return Err(format!("{HUD_ICON} goes with the class: write it in a [class.\"...\"] table, not in a map table"));
            }
            return match &o.value {
                ModValue::Str(_) => Ok(()),
                v => Err(format!("expects a texture \"Mod.<Name>\" or \"Package.Group.Name\", got {v}")),
            };
        }
        if o.property.eq_ignore_ascii_case(FIRST_PERSON_ANIMATIONS) {
            if o.map.is_some() {
                return Err(format!("{FIRST_PERSON_ANIMATIONS} goes with the class: write it in a [class.\"...\"] table, not in a map table"));
            }
            let ModValue::Table(fields) = &o.value else { return Err(format!("expects an inline table such as {{ FireLast = \"Fire\" }}, got {}", o.value)) };
            if let Some((k, v)) = fields.iter().find(|(_, v)| !matches!(v, ModValue::Str(_))) {
                return Err(format!("{k} = {v}: expects an animation name (string)"));
            }
            return Ok(());
        }
        rate_value(&o.value).map(|_| ())
    }

    /// The DrawTile native, mods build: the canvas colour `k` (B, G, R, A) of the tracer that
    /// XIIIWeapon.DrawTraceBullet draws, replaced by the weapon's TracerColor. Unchanged for any
    /// other tile, and for weapons without that setting.
    pub(crate) fn mod_tracer_draw_color(&self, k: [u8; 4]) -> [u8; 4] {
        let Some((_, class)) = self.tracer_weapon() else { return k };
        match self.mod_weapon_setting(class, TRACER_COLOR).map(|o| tracer_color(&o.value)) {
            Some(Ok(c)) => c,
            _ => k,
        }
    }

    /// The weapon (and its class) whose XIIIWeapon.DrawTraceBullet calls the native DrawTile
    /// running now, when mods set weapon values; None for any other tile.
    fn tracer_weapon(&self) -> Option<(ObjId, ClassId)> {
        if !self.mods.defaults.as_ref().is_some_and(|t| t.has_mod_only()) {
            return None;
        }
        // The native DrawTile on top of the stacks, the script function that calls it below.
        let i = self.stack.len().checked_sub(2)?;
        let (&f, &weapon) = (self.stack.get(i)?, self.this_stack.get(i)?);
        if !self.functions[f].name.eq_ignore_ascii_case("DrawTraceBullet") {
            return None;
        }
        let class = self.objects.get(weapon).and_then(|o| o.as_ref()).map(|o| o.class)?;
        Some((weapon, class))
    }

    /// The DrawTile native, mods build: the size (xl, yl) of the tracer XIIIWeapon.DrawTraceBullet
    /// draws from the middle of the view, for a weapon held with a FirstPersonModel. The game
    /// sizes it with the weapon's FlashOffsetX/Y, fractions of the view fitted by hand to the
    /// game's own guns in a 4:3 view; a mod's model is placed elsewhere, and the view's shape
    /// (wide screens, split screen) moves the muzzle. The size is made so that the end of the
    /// beam drawn in the texture (its farthest lit texel from the tile's start, `texture`) lands
    /// on the muzzle of the model as this view's camera projects it. Unchanged for any other
    /// tile, a weapon without a FirstPersonModel, or a muzzle behind the camera.
    pub(crate) fn mod_tracer_size(&mut self, canvas: ObjId, texture: &Value, xl: f32, yl: f32) -> (f32, f32) {
        let Some((weapon, class)) = self.tracer_weapon() else { return (xl, yl) };
        if self.mod_weapon_setting(class, FIRST_PERSON_MODEL).is_none() {
            return (xl, yl);
        }
        let Some(muzzle) = self.mod_first_person_muzzle(weapon) else { return (xl, yl) };
        let Some((node, size)) = self.mods.view_projection.clone() else { return (xl, yl) };
        let Some(ndc) = self.first_person_on_screen(&node, size, muzzle) else { return (xl, yl) };
        let (clip_x, clip_y) = (self.cf(canvas, "ClipX"), self.cf(canvas, "ClipY"));
        // ScreenToCanvas (FCanvasUtil::vInit, solo_native.rs canvas_to_screen), in the canvas's
        // own size.
        let to = [(ndc[0] + 1.0) * 0.5 * clip_x + 0.5, (1.0 - ndc[1]) * 0.5 * clip_y + 0.5];
        let from = [self.cf(canvas, "CurX"), self.cf(canvas, "CurY")];
        let end = self.tracer_texture_end(texture);
        // Placed again once the weapon is where it is drawn (mod_tracer_fixup): the same
        // RenderOverlays moves it after drawing the tracer.
        if let Value::Asset(name) = texture {
            self.mods.tracer_tiles.push((self.hud.len(), weapon, end, name.to_ascii_lowercase()));
        }
        ((to[0] - from[0]) / end[0], (to[1] - from[1]) / end[1])
    }

    /// After a player's HUD (AHUD.PostRender, which runs the weapon's RenderOverlays), mods
    /// build: the tracers sized by mod_tracer_size end at the muzzle again, the weapon now where
    /// the view draws it. XIIIWeapon.RenderOverlays draws the tracer before it moves the weapon
    /// in front of the eyes (SetLocation, SetRotation): the recoil and the sway of that frame.
    pub(crate) fn mod_tracer_fixup(&mut self, canvas: ObjId) {
        if self.mods.tracer_tiles.is_empty() {
            return;
        }
        let tiles = std::mem::take(&mut self.mods.tracer_tiles);
        let Some((node, size)) = self.mods.view_projection.clone() else { return };
        let (clip_x, clip_y, org_x, org_y) = (self.cf(canvas, "ClipX"), self.cf(canvas, "ClipY"), self.cf(canvas, "OrgX"), self.cf(canvas, "OrgY"));
        for (i, weapon, end, texture) in tiles {
            let Some(ndc) = self.mod_first_person_muzzle(weapon).and_then(|m| self.first_person_on_screen(&node, size, m)) else { continue };
            let to = [(ndc[0] + 1.0) * 0.5 * clip_x + 0.5 + org_x, (1.0 - ndc[1]) * 0.5 * clip_y + 0.5 + org_y];
            // The tile DrawTile made (none when the canvas style draws nothing).
            if let Some(crate::canvas::HudDraw::Tile { x1, y1, x2, y2, material, .. }) = self.hud.get_mut(i) {
                if !material.to_ascii_lowercase().ends_with(&texture) {
                    continue;
                }
                *x2 = *x1 + (to[0] - *x1) / end[0];
                *y2 = *y1 + (to[1] - *y1) / end[1];
            }
        }
    }

    /// Where point `p` of a first-person model shows in the view of scene `node` (a view of
    /// `size`): the model is drawn with the first-person projection (first_person_scale), from
    /// the same camera as the scene, not with the scene's own angles. Screen X, Y (−1 to 1, as
    /// SceneNode::project gives them), Z and W; None behind the camera.
    pub(crate) fn first_person_on_screen(&self, node: &crate::visibility::SceneNode, size: (i32, i32), p: [f32; 3]) -> Option<[f32; 4]> {
        let q = node.project(p);
        let scale = node.projection_scale();
        if !(q[3] > 0.0) || scale[0] == 0.0 || scale[1] == 0.0 {
            return None;
        }
        let fp = self.first_person_scale(size.0 as f32, size.1 as f32);
        Some([q[0] * fp[0] / scale[0], q[1] * fp[1] / scale[1], q[2], q[3]])
    }

    /// The point of the world that the player's view (the scene he was last drawn) shows where
    /// he sees point `p` of a first-person model: the same distance ahead of the camera, on the
    /// line through the model's point as the first-person projection draws it. An effect put
    /// there (the flamethrower's flame, drawn with the scene) is seen on the model. `p` itself
    /// without a view of that player.
    pub(crate) fn first_person_seen(&self, viewer: ObjId, p: [f32; 3]) -> [f32; 3] {
        let Some((node, size)) = self.mods.views.get(&viewer) else { return p };
        match self.first_person_on_screen(node, *size, p) {
            Some(s) => node.deproject_plane(s),
            None => p,
        }
    }

    /// X and Y scales (1 / tangent of the half angles) of the projection the first-person models
    /// are drawn with in a view of `w` × `h` (bSpecialDelayFov: a 25°
    /// horizontal half angle instead of the player's FovAngle), as the viewer draws them
    /// (renderer.rs update_uniforms_with): widened by Hor+ in a window wider than 4:3; in local
    /// split screen, the height of view the weapon has in the whole window (main.rs
    /// overlay_aspect_cap), never widened.
    pub(crate) fn first_person_scale(&self, w: f32, h: f32) -> [f32; 2] {
        let tan = f32::from_bits(0x3edf_65be).tan();
        let aspect = w / h.max(1.0);
        let tan_v = if self.local_split() {
            let (ww, wh) = self.display_window_size();
            let window = ww as f32 / wh.max(1) as f32;
            let cap = if aspect < window { crate::coop::REFERENCE_ASPECT } else { window };
            tan / aspect.min(cap)
        } else {
            let k = crate::display_format::hor_plus(w as u32, h as u32, self.engine.client.session_graphics).unwrap_or(1.0);
            tan * k / aspect
        };
        [1.0 / (tan_v * aspect), 1.0 / tan_v]
    }

    /// Where the beam of a tracer texture ends, as a fraction of its width and height: the lit
    /// texel (colour × alpha above a tenth of the brightest) farthest from its top left corner,
    /// the corner the game draws from. (1, 1) for a texture of the game (its pixels are not at
    /// hand) or a texture with nothing lit.
    fn tracer_texture_end(&mut self, texture: &Value) -> [f32; 2] {
        let Value::Asset(name) = texture else { return [1.0, 1.0] };
        if let Some(&e) = self.mods.tracer_ends.get(name) {
            return e;
        }
        let end = (|| {
            let mips = openxiii_assets::mod_textures::get(name)?;
            let (w, h, rgba) = mips.first()?;
            let (w, h) = (*w as usize, *h as usize);
            let lit = |i: usize| -> u32 { rgba[i * 4..i * 4 + 3].iter().copied().max().unwrap_or(0) as u32 * rgba[i * 4 + 3] as u32 };
            let top = (0..w * h).map(lit).max().filter(|&t| t > 0)?;
            let (mut best, mut at) = (-1i64, [0usize; 2]);
            for y in 0..h {
                for x in 0..w {
                    if lit(y * w + x) * 10 > top && (x + y) as i64 > best {
                        best = (x + y) as i64;
                        at = [x, y];
                    }
                }
            }
            Some([(at[0] as f32 + 0.5) / w as f32, (at[1] as f32 + 0.5) / h as f32])
        })()
        .unwrap_or([1.0, 1.0]);
        self.mods.tracer_ends.insert(name.clone(), end);
        end
    }

    /// Where a weapon held with a FirstPersonModel comes out of that model, in the world, as the
    /// model was last placed on the hands (the bone's latest pose): the middle of its front end
    /// (+X of the model, the way a mod's model faces). None for a weapon without such a model.
    pub(crate) fn mod_first_person_muzzle(&mut self, weapon: ObjId) -> Option<[f32; 3]> {
        let att = *self.mods.fp_models.get(&weapon)?;
        if self.objects.get(att).is_none_or(|o| o.is_none()) {
            return None;
        }
        // The model on the bone of the hands as they are posed now (as the render puts it).
        if self.mesh_instance(weapon).is_some() {
            let _ = self.set_attachment_location(weapon, att);
        }
        let path = self.prim_static_mesh(att)?.to_string();
        let front = self.mesh_front(&path)?;
        use crate::NativeField as F;
        let (r, s3, l, pp, ds) = (
            self.native_prop(att, F::Rotation).as_rotator(),
            self.native_prop(att, F::DrawScale3D).as_vector(),
            self.native_prop(att, F::Location).as_vector(),
            self.native_prop(att, F::PrePivot).as_vector(),
            self.native_prop(att, F::DrawScale).as_float(),
        );
        let m = crate::radius_iter::local_to_world(r, s3, l, pp, ds);
        Some(openxiii_assets::coords::transform_fvector(&m, front))
    }

    /// The front end of a static mesh in its own frame: the mean of its points within 2 % of its
    /// length from the farthest one forward (+X). Read once per mesh (a mod's model, or one of
    /// the game's packages).
    pub(crate) fn mesh_front(&mut self, path: &str) -> Option<[f32; 3]> {
        if let Some(f) = self.mods.mesh_fronts.get(path) {
            return *f;
        }
        let positions: Option<Vec<[f32; 3]>> = match openxiii_assets::mod_meshes::get(path) {
            Some(m) => Some(m.0.positions.clone()),
            None => (|| {
                let lib = self.library.as_mut()?;
                let (loaded, index) = lib.resolve(path)?;
                if loaded.package.export_class(index) != "StaticMesh" {
                    return None;
                }
                openxiii_assets::static_mesh::StaticMesh::read(&loaded.package, &loaded.data, index).ok().map(|m| m.positions)
            })(),
        };
        let front = positions.and_then(|p| mesh_front_of(&p));
        self.mods.mesh_fronts.insert(path.to_string(), front);
        front
    }

    /// A script function about to run, mods build: true when it must not run. Only
    /// SetUpImpactEmitter, for an object whose class has ImpactEffectsBySurface = false.
    pub(crate) fn mod_skip_function(&self, f: crate::types::FuncId, this: ObjId) -> bool {
        if !self.mods.defaults.as_ref().is_some_and(|t| t.has_mod_only()) || !self.functions[f].name.eq_ignore_ascii_case("SetUpImpactEmitter") {
            return false;
        }
        let Some(class) = self.objects.get(this).and_then(|o| o.as_ref()).map(|o| o.class) else { return false };
        matches!(self.mod_weapon_setting(class, IMPACT_EFFECTS_BY_SURFACE).map(|o| o.value), Some(ModValue::Bool(false)))
    }
}

/// The front end of a set of points: the mean of those within 2 % of their length (along X) from
/// the farthest one forward (+X). None without points.
pub(crate) fn mesh_front_of(points: &[[f32; 3]]) -> Option<[f32; 3]> {
    let hi = points.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
    let lo = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    if !hi.is_finite() {
        return None;
    }
    let near = hi - (hi - lo) * 0.02;
    let front: Vec<&[f32; 3]> = points.iter().filter(|p| p[0] >= near).collect();
    let n = front.len() as f32;
    Some([hi, front.iter().map(|p| p[1]).sum::<f32>() / n, front.iter().map(|p| p[2]).sum::<f32>() / n])
}

/// A TracerColor value { R, G, B, A } (A optional, 255) as a canvas colour (B, G, R, A).
pub(crate) fn tracer_color(v: &ModValue) -> Result<[u8; 4], String> {
    let ModValue::Table(fields) = v else { return Err(format!("expects a colour {{ R = 0, G = 128, B = 255 }}, got {v}")) };
    let mut c = [0u8, 0, 0, 255];
    for (k, x) in fields {
        let i = match k.to_ascii_uppercase().as_str() {
            "B" => 0,
            "G" => 1,
            "R" => 2,
            "A" => 3,
            _ => return Err(format!("a colour has the fields R, G, B and A, not {k}")),
        };
        c[i] = match x {
            ModValue::Int(n) if (0..=255).contains(n) => *n as u8,
            _ => return Err(format!("{k} = {x}: expects a whole number from 0 to 255")),
        };
    }
    Ok(c)
}
