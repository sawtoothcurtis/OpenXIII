//! Native replication lists (Engine.dll, GetOptimizedRepList): for
//! classes with CLASS_NativeReplication, the C++ chooses the properties to send (RepIndex) by
//! comparing the actor with the last value sent (Recent, or the default values), instead
//! of the conditions of the script's replication block.
//!
//! Comparisons: vectors and rotators component by
//! component as integers (bits); objects: if UPackageMapLevel::CanSerializeObject (a dynamic
//! actor has its channel), A != B, otherwise bActorMustStayDirty and B != None; strings case-insensitive.

use crate::value::{ObjId, Value};
use crate::Vm;
use std::collections::HashMap;

/// Equality of two values as in the C++ of GetOptimizedRepList: no floating-point instruction
/// (no fcom or comiss), only integer cmps on the
/// memory words. A float is therefore compared by its bits (NaN identical to itself, -0 different
/// from +0); a string case-insensitively (FString::operator==), a name by its index.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        (Value::Name(x), Value::Name(y)) => {
            let none = |s: &str| s.is_empty() || s.eq_ignore_ascii_case("None");
            x.eq_ignore_ascii_case(y) || (none(x) && none(y))
        }
        (Value::Struct(_, x), Value::Struct(_, y)) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same(p, q)),
        // Inline vector / rotator: like the struct they are.
        _ if a.is_struct() && b.is_struct() => match (a.struct_view(), b.struct_view()) {
            (Some((_, x)), Some((_, y))) => x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same(p, q)),
            _ => false,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        _ => a == b,
    }
}

/// Native classes whose GetOptimizedRepList is overridden, from the most derived to Actor.
const NATIVE: [&str; 16] = [
    "PlayerController",
    "Controller",
    "Pawn",
    "Mover",
    "PhysicsVolume",
    "LevelInfo",
    "PlayerReplicationInfo",
    "GameReplicationInfo",
    "TeamInfo",
    "Pickup",
    "Weapon",
    "Ammunition",
    "Powerups",
    "Inventory",
    "WeaponAttachment",
    "Actor",
];

pub(crate) struct RepList<'a> {
    vm: &'a Vm,
    a: ObjId,
    recent: &'a [Value],
    channels: &'a HashMap<ObjId, (usize, bool)>,
    pub out: Vec<usize>,
    pub must_stay_dirty: bool,
}

impl<'a> RepList<'a> {
    fn slot(&self, name: &str) -> Option<usize> {
        self.vm.prop_slot(self.a, name).ok()
    }
    fn v(&self, name: &str) -> Value {
        self.vm.propr(self.a, name).clone()
    }
    fn r(&self, name: &str) -> Value {
        self.slot(name).and_then(|s| self.recent.get(s).cloned()).unwrap_or(Value::Void)
    }
    fn b(&self, name: &str) -> bool {
        self.v(name).as_bool()
    }
    fn byte(&self, name: &str) -> u8 {
        self.v(name).as_byte()
    }
    fn push(&mut self, name: &str) {
        let Some(s) = self.slot(name) else { return };
        let Some(c) = self.vm.objects[self.a].as_ref().map(|o| o.class) else { return };
        // The C++ pushes UProperty::RepIndex (+0x42) without looking at CPF_Net; UClass::Link only sets it for
        // Net properties, the others keep 0 (zeroed object): AWeapon::WeaponMode, AWeaponAttachment::iBTrailDist… push 0.
        let Some(net) = self.vm.ipdrv.rep.classes.get(&c) else { return };
        self.out.push(net.props.get(&s).map_or(0, |p| p.rep_index));
    }
    /// Different value (words compared as the C++ does, see `same`).
    fn ne(&mut self, name: &str) {
        if self.slot(name).is_some() && !same(&self.v(name), &self.r(name)) {
            self.push(name);
        }
    }
    /// NEQ of vectors and rotators: components compared as integers.
    fn ne_bits(&mut self, name: &str) {
        let bits = |v: &Value| -> Vec<u32> {
            match v.struct_view() {
                Some((_, f)) => f
                    .iter()
                    .map(|x| match x {
                        Value::Float(f) => f.to_bits(),
                        other => other.as_int() as u32,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        if self.slot(name).is_some() && bits(&self.v(name)) != bits(&self.r(name)) {
            self.push(name);
        }
    }
    /// UPackageMapLevel::CanSerializeObject: a dynamic actor is serializable if it has a channel.
    fn can_serialize(&self, v: &Value) -> bool {
        self.vm.net_can_serialize(self.channels, v)
    }
    /// NEQ of objects: CanSerializeObject(A) ? A != B : bActorMustStayDirty and
    /// B != None.
    fn obj_ne(&mut self, a: &Value, b: &Value) -> bool {
        if self.can_serialize(a) {
            return a != b;
        }
        self.must_stay_dirty = true;
        !matches!(b, Value::Object(None) | Value::Class(None) | Value::Void)
    }
    fn ne_obj(&mut self, name: &str) {
        if self.slot(name).is_none() {
            return;
        }
        let (a, b) = (self.v(name), self.r(name));
        if self.obj_ne(&a, &b) {
            self.push(name);
        }
    }

    /// AActor::GetOptimizedRepList.
    fn actor(&mut self) {
        let net_initial = self.b("bNetInitial");
        if self.b("bSkipActorPropertyReplication") && !net_initial {
            return;
        }
        if self.byte("Role") != 4 {
            return;
        }
        let draw = self.byte("DrawType");
        let rot_draw = draw == 2 || draw == 8;
        let base = self.v("Base").as_object().filter(|&b| self.vm.objects.get(b).is_some_and(|x| x.is_some()));
        let world_base = base.is_none_or(|b| self.vm.propr(b, "bWorldGeometry").as_bool());
        if self.b("bReplicateMovement") {
            match self.byte("RemoteRole") {
                3 => {
                    if net_initial {
                        self.ne_bits("Location");
                        if rot_draw {
                            self.ne_bits("Rotation");
                        }
                    }
                }
                2 => {
                    self.ne_obj("Base");
                    let sim = net_initial || self.b("bUpdateSimulatedPosition");
                    if sim {
                        self.ne_bits("Velocity");
                    }
                    if world_base {
                        if sim {
                            self.ne_bits("Location");
                            if rot_draw {
                                self.ne_bits("Rotation");
                            }
                        }
                    } else {
                        self.ne_bits("RelativeLocation");
                        self.ne_bits("RelativeRotation");
                        self.ne("AttachmentBone");
                    }
                    if net_initial {
                        self.ne("Physics");
                    }
                    if self.byte("Physics") == 5 {
                        self.ne("bFixedRotationDir");
                        self.ne("bRotateToDesired");
                        self.ne_bits("RotationRate");
                        self.ne_bits("DesiredRotation");
                    }
                }
                _ => {
                    if world_base {
                        self.ne_bits("Location");
                        if rot_draw {
                            self.ne_bits("Rotation");
                        }
                    } else {
                        self.ne_bits("RelativeLocation");
                        self.ne_bits("RelativeRotation");
                        self.ne("AttachmentBone");
                    }
                    self.ne_obj("Base");
                    self.ne("Physics");
                    self.ne("bMovable");
                    match self.byte("Physics") {
                        2 => self.ne_bits("Velocity"),
                        5 => {
                            self.ne("bFixedRotationDir");
                            self.ne("bRotateToDesired");
                            self.ne_bits("RotationRate");
                            self.ne_bits("DesiredRotation");
                        }
                        _ => {}
                    }
                }
            }
        }
        let net_owner = self.b("bNetOwner");
        if !net_owner || !self.b("bClientAnim") {
            self.ne_obj("AmbientSound");
            self.ne("SaturationDistance");
            self.ne("StabilisationDistance");
            self.ne("StabilisationVolume");
            self.ne("bHasPosition");
            self.ne("bHasRollOff");
        }
        if self.b("bNetDirty") {
            self.ne_obj("Owner");
            self.ne("DrawScale");
            self.ne("DrawScale3D");
            self.ne("DrawType");
            self.ne("bCollideActors");
            self.ne("bCollideWorld");
            self.ne("bOnlyOwnerSee");
            self.ne_obj("Texture");
            self.ne("Style");
            if rot_draw {
                self.ne_bits("PrePivot");
            }
            if self.b("bCollideActors") || self.b("bCollideWorld") {
                self.ne("bProjTarget");
                self.ne("bBlockActors");
                self.ne("bBlockPlayers");
                self.ne("CollisionRadius");
                self.ne("CollisionHeight");
            }
            if net_owner {
                self.ne_obj("Inventory");
            }
            if self.b("bReplicateInstigator") {
                self.ne_obj("Instigator");
            }
            if draw == 2 {
                self.ne("AmbientGlow");
                self.ne("bUnlit");
                self.ne_obj("Mesh");
            } else if draw == 8 {
                self.ne_obj("StaticMesh");
            }
            self.ne("LightType");
            if self.byte("LightType") != 0 {
                for p in [
                    "LightEffect",
                    "LightBrightness",
                    "LightHue",
                    "LightSaturation",
                    "LightRadius",
                    "LightPeriod",
                    "LightPhase",
                    "bDecor1Light",
                    "bDecor2Light",
                    "bDecor3Light",
                    "bActorLight",
                    "bDecor4Light",
                    "bDecor5Light",
                    "bDecor6Light",
                    "bDecor7Light",
                    "bDecor8Light",
                    "bDecor9Light",
                    "bDecor10Light",
                ] {
                    self.ne(p);
                }
            }
        }
        if self.b("bReplicateAnimations") && draw == 2 {
            // Sequence (FName), AnimRate and bAnimLoop only.
            if self.slot("SimAnim").is_some() && crate::net_actor::sim_anim_changed(&self.v("SimAnim"), &self.r("SimAnim")) {
                self.push("SimAnim");
            }
        }
        self.ne("bHidden");
        self.ne("Role");
        self.ne("RemoteRole");
        self.ne("bNetOwner");
        self.ne("bTearOff");
    }

    /// APawn::GetOptimizedRepList.
    fn pawn(&mut self) {
        self.actor();
        if self.byte("Role") != 4 {
            return;
        }
        if self.b("bNetDirty") {
            self.ne_obj("PlayerReplicationInfo");
            self.ne_obj("Controller");
            if self.b("bNetOwner") {
                self.ne_obj("SelectedItem");
                for p in ["GroundSpeed", "WaterSpeed", "AirSpeed", "AccelRate", "JumpZ", "AirControl"] {
                    self.ne(p);
                }
            }
            self.ne_obj("HitDamageType");
            self.ne("TakeHitLocation");
            self.ne("AnimStatus");
            self.ne("AnimAction");
            self.ne("bSimulateGravity");
            self.ne("bIsCrouched");
            self.ne("bIsWalking");
            if self.b("bTearOff") {
                self.ne_bits("TearOffMomentum");
            }
        }
        for p in ["Health", "ControllerPitch", "bIsDead", "WeaponMode", "BaffeCount", "SkinID"] {
            self.ne(p);
        }
    }

    /// AController::GetOptimizedRepList.
    fn controller(&mut self) {
        self.actor();
        if self.b("bNetDirty") && self.byte("Role") == 4 {
            self.ne_obj("PlayerReplicationInfo");
            self.ne_obj("Pawn");
            if self.b("bNetOwner") {
                self.ne_obj("PawnClass");
            }
        }
    }

    /// APlayerController::GetOptimizedRepList.
    fn player_controller(&mut self) {
        self.controller();
        if self.b("bNetOwner") && self.byte("Role") == 4 {
            if self.b("bNetDirty") {
                self.ne_obj("ViewTarget");
                self.ne_obj("GameReplicationInfo");
            }
            let vt = self.v("ViewTarget").as_object();
            if vt != self.v("Pawn").as_object() && vt.is_some_and(|t| self.vm.is_a(t, "Pawn")) {
                self.ne_bits("TargetViewRotation");
                self.ne("TargetEyeHeight");
                self.ne_bits("TargetWeaponViewOffset");
            }
        }
    }

    /// AMover::GetOptimizedRepList.
    fn mover(&mut self) {
        self.actor();
        if self.byte("Role") == 4 {
            for p in ["SimOldPos", "SimOldRotPitch", "SimOldRotYaw", "SimOldRotRoll", "SimInterpolate", "RealPosition", "RealRotation"] {
                self.ne(p);
            }
        }
        self.ne("Velocity");
    }

    /// APhysicsVolume::GetOptimizedRepList.
    fn physics_volume(&mut self) {
        self.actor();
        if self.byte("Role") == 4 && self.b("bSkipActorPropertyReplication") && !self.b("bNetInitial") {
            self.ne_bits("Location");
            self.ne_bits("Rotation");
            self.ne_obj("Base");
            let base = self.v("Base").as_object().filter(|&b| self.vm.objects.get(b).is_some_and(|x| x.is_some()));
            if base.is_some_and(|b| !self.vm.propr(b, "bWorldGeometry").as_bool()) {
                self.ne_bits("RelativeLocation");
                self.ne_bits("RelativeRotation");
                self.ne("AttachmentBone");
            }
        }
    }

    /// ALevelInfo::GetOptimizedRepList.
    fn level_info(&mut self) {
        self.actor();
        if self.byte("Role") == 4 && self.b("bNetDirty") {
            self.ne_obj("Pauser");
            self.ne("TimeDilation");
        }
    }

    /// APlayerReplicationInfo::GetOptimizedRepList.
    fn player_replication_info(&mut self) {
        self.actor();
        if self.byte("Role") != 4 {
            return;
        }
        self.ne("SkinCodeName");
        if self.b("bNetDirty") {
            self.ne("Score");
            self.ne("Deaths");
            self.ne_obj("HasFlag");
            self.ne("Ping");
            self.ne_obj("PlayerLocation");
            self.ne("PlayerName");
            self.ne_obj("Team");
            self.ne("TeamID");
            self.ne_obj("VoiceType");
            for p in ["bIsFemale", "bFeigningDeath", "bIsSpectator", "bWaitingPlayer", "bReadyToPlay"] {
                self.ne(p);
            }
            self.ne_obj("TalkTexture");
            self.ne("bOutOfLives");
            if self.b("bNetInitial") {
                self.ne("bBot");
                self.ne("StartTime");
            }
        }
    }

    /// AGameReplicationInfo::GetOptimizedRepList.
    fn game_replication_info(&mut self) {
        self.actor();
        if self.byte("Role") != 4 || !self.b("bNetDirty") {
            return;
        }
        self.ne("RemainingMinute");
        self.ne("bStopCountDown");
        self.ne_obj("Winner");
        // Teams[2]: element by element, RepIndex(Teams) + i.
        if let Some(s) = self.slot("Teams") {
            let (a, b) = (self.v("Teams"), self.recent.get(s).cloned().unwrap_or(Value::Void));
            for i in 0..2 {
                let ai = match &a {
                    Value::Array(x) => x.get(i).cloned().unwrap_or(Value::Object(None)),
                    _ => Value::Object(None),
                };
                let bi = match &b {
                    Value::Array(x) => x.get(i).cloned().unwrap_or(Value::Object(None)),
                    _ => Value::Object(None),
                };
                if self.obj_ne(&ai, &bi) {
                    let c = self.vm.objects[self.a].as_ref().map(|o| o.class);
                    if let Some(r) = c.and_then(|c| self.vm.ipdrv.rep.classes.get(&c)).and_then(|n| n.props.get(&s)).map(|p| p.rep_index) {
                        self.out.push(r + i);
                    }
                }
            }
        }
        if self.b("bNetInitial") {
            for p in [
                "GameName",
                "GameClass",
                "bTeamGame",
                "RemainingTime",
                "ElapsedTime",
                "ServerName",
                "ShortName",
                "AdminName",
                "AdminEmail",
                "Region",
                "MOTDLine1",
                "MOTDLine2",
                "MOTDLine3",
                "MOTDLine4",
                "GoalScore",
                "TimeLimit",
            ] {
                self.ne(p);
            }
        }
    }

    /// ATeamInfo::GetOptimizedRepList.
    fn team_info(&mut self) {
        self.actor();
        if self.byte("Role") != 4 {
            return;
        }
        if self.b("bNetDirty") {
            self.ne("Score");
            self.ne_obj("Flag");
        }
        if self.b("bNetInitial") {
            self.ne("TeamName");
            self.ne("TeamIndex");
            self.ne("TeamColor");
            self.ne("AltTeamColor");
            self.ne_obj("TeamIcon");
        }
    }

    /// APickup::GetOptimizedRepList.
    fn pickup(&mut self) {
        if !self.b("bOnlyReplicateHidden") {
            self.actor();
            return;
        }
        self.ne("bHidden");
    }

    /// AInventory::GetOptimizedRepList.
    fn inventory(&mut self) {
        self.actor();
        if self.b("bNetOwner") && self.byte("Role") == 4 && self.b("bNetDirty") {
            self.ne_obj("ThirdPersonActor");
            self.ne("Charge");
        }
    }

    /// AWeapon::GetOptimizedRepList.
    fn weapon(&mut self) {
        self.inventory();
        if self.b("bNetOwner") && self.byte("Role") == 4 && self.b("bNetDirty") {
            self.ne_obj("AmmoType");
            self.ne("bAllowShot");
            self.ne("ReloadCount");
            self.ne("WeaponMode");
            self.ne_obj("AltAmmoType");
            self.ne("AltReloadCount");
        }
    }

    /// AWeaponAttachment::GetOptimizedRepList.
    fn weapon_attachment(&mut self) {
        self.actor();
        if !self.b("bNetOwner") && self.byte("Role") == 4 && self.b("bNetDirty") {
            for p in ["FlashCount", "FiringMode", "AltFlashCount", "AltFiringMode", "iBTrailDist", "ReloadClientCount"] {
                self.ne(p);
            }
        }
    }

    /// AAmmunition::GetOptimizedRepList.
    fn ammunition(&mut self) {
        self.inventory();
        if self.b("bNetOwner") && self.byte("Role") == 4 && self.b("bNetDirty") {
            self.ne("AmmoAmount");
        }
    }

    /// APowerups::GetOptimizedRepList.
    fn powerups(&mut self) {
        self.inventory();
        if self.b("bNetOwner") && self.byte("Role") == 4 && self.b("bNetDirty") {
            self.ne("bActivatable");
            self.ne("bActive");
            self.ne("NumCopies");
        }
    }
}

impl Vm {
    /// The actor's AActor::GetOptimizedRepList (virtual): RepIndex of the native properties to
    /// send, and bActorMustStayDirty. The NetClass caches of its class must be ready.
    pub(crate) fn optimized_rep_list(&self, a: ObjId, recent: &[Value], channels: &HashMap<ObjId, (usize, bool)>) -> (Vec<usize>, bool) {
        let mut l = RepList { vm: self, a, recent, channels, out: Vec::new(), must_stay_dirty: false };
        let Some(class) = self.objects.get(a).and_then(|o| o.as_ref()).map(|o| o.class) else { return (Vec::new(), false) };
        let mut c = Some(class);
        let mut which = "Actor";
        'find: while let Some(ci) = c {
            for n in NATIVE {
                if self.classes[ci].name.eq_ignore_ascii_case(n) {
                    which = n;
                    break 'find;
                }
            }
            c = self.classes[ci].super_class;
        }
        // DAT_105xxxx & 0x800: the C++ class is CLASS_NativeReplication (always true here).
        match which {
            "PlayerController" => l.player_controller(),
            "Controller" => l.controller(),
            "Pawn" => l.pawn(),
            "Mover" => l.mover(),
            "PhysicsVolume" => l.physics_volume(),
            "LevelInfo" => l.level_info(),
            "PlayerReplicationInfo" => l.player_replication_info(),
            "GameReplicationInfo" => l.game_replication_info(),
            "TeamInfo" => l.team_info(),
            "Pickup" => l.pickup(),
            "Weapon" => l.weapon(),
            "Ammunition" => l.ammunition(),
            "Powerups" => l.powerups(),
            "Inventory" => l.inventory(),
            "WeaponAttachment" => l.weapon_attachment(),
            _ => l.actor(),
        }
        (l.out, l.must_stay_dirty)
    }
}

