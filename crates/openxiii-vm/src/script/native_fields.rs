//! Fields with a constant name read by the native simulation and rendering paths.
//! Scripts keep using the usual property resolution.
use crate::{value::{ObjId, Value}, Vm};

macro_rules! native_fields {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub enum NativeField { $($variant),+ }
        const NAMES: [&str; [$(stringify!($variant)),+].len()] = [$($name),+];
    };
}

native_fields! {
    Stasis => "bStasis",
    BegunPlay => "bBegunPlay",
    StaticLightGroup => "bSLightGroup",
    Unlit => "bUnlit",
    IgnoreDynLight => "bIgnoreDynLight",
    TimerLoop => "bTimerLoop",
    Timer2Loop => "bTimer2Loop",
    Timer3Loop => "bTimer3Loop",
    Location => "Location",
    Rotation => "Rotation",
    Velocity => "Velocity",
    Acceleration => "Acceleration",
    Owner => "Owner",
    Touching => "Touching",
    Tag => "Tag",
    Base => "Base",
    Pawn => "Pawn",
    Controller => "Controller",
    Physics => "Physics",
    Role => "Role",
    RemoteRole => "RemoteRole",
    DrawType => "DrawType",
    Mesh => "Mesh",
    StaticMesh => "StaticMesh",
    DrawScale => "DrawScale",
    DrawScale3D => "DrawScale3D",
    PrePivot => "PrePivot",
    Skins => "Skins",
    CollisionRadius => "CollisionRadius",
    CollisionHeight => "CollisionHeight",
    PhysicsVolume => "PhysicsVolume",
    Region => "Region",
    Level => "Level",
    Player => "Player",
    TimeSeconds => "TimeSeconds",
    LastRenderTime => "LastRenderTime",
    LastRenderDist => "LastRenderDist",
    AmbientGlow => "AmbientGlow",
    LightType => "LightType",
    LightBrightness => "LightBrightness",
    LightRadius => "LightRadius",
    Leaves => "Leaves",
    Hidden => "bHidden",
    DeleteMe => "bDeleteMe",
    CollideActors => "bCollideActors",
    Static => "bStatic",
    IgnoreVignetteAlpha => "bIgnoreVignetteAlpha",
    DelayDisplay => "bDelayDisplay",
    SpecialDelayFov => "bSpecialDelayFov",
    LifeSpan => "LifeSpan",
    TimerRate => "TimerRate",
    Timer2Rate => "Timer2Rate",
    Timer3Rate => "Timer3Rate",
    TimerCounter => "TimerCounter",
    Timer2Counter => "Timer2Counter",
    Timer3Counter => "Timer3Counter",
    WorldGeometry => "bWorldGeometry",
    UseCylinderCollision => "bUseCylinderCollision",
    BlockNonZeroExtentTraces => "bBlockNonZeroExtentTraces",
    CollideWorld => "bCollideWorld",
    BlockActors => "bBlockActors",
    BlockPlayers => "bBlockPlayers",
    Brush => "Brush",
    // Visibility (render_level): read per actor, per leaf or per zone.
    Style => "Style",
    AmbientBrightness => "AmbientBrightness",
    AmbientHue => "AmbientHue",
    AmbientSaturation => "AmbientSaturation",
    UseVignetteHighlight => "UseVignetteHighlight",
    RendMap => "RendMap",
    ShowFlags => "ShowFlags",
    KeyNum => "KeyNum",
    // Pawn tick (phys_walking, MoveActor, TickSpecial), read every frame.
    JustTeleported => "bJustTeleported",
    IsWalking => "bIsWalking",
    CanWalkOffLedges => "bCanWalkOffLedges",
    CanJump => "bCanJump",
    AirSpeed => "AirSpeed",
    AccelRate => "AccelRate",
    NoVelocityUpdate => "bNoVelocityUpdate",
    CanFly => "bCanFly",
    Floor => "Floor",
    Movable => "bMovable",
    AttachmentBone => "AttachmentBone",
    Attached => "Attached",
    RelativeLocation => "RelativeLocation",
    IsDead => "bIsDead",
    PhysicsAnimUpdate => "bPhysicsAnimUpdate",
    SpineControl => "bSpineControl",
    HeadControl => "bHeadControl",
    Interpolating => "bInterpolating",
    Weapon => "Weapon",
    UpdateEyeheight => "bUpdateEyeheight",
    BreathTime => "BreathTime",
    Shadow => "Shadow",
    ViewTarget => "ViewTarget",
    BehindView => "bBehindView",
    DesiredRotation => "DesiredRotation",
    IsCrouched => "bIsCrouched",
    WantsToCrouch => "bWantsToCrouch",
    TryToUncrouch => "bTryToUncrouch",
    PreparingMove => "bPreparingMove",
    IsPlayer => "bIsPlayer",
    Crawler => "bCrawler",
    PendingTouch => "PendingTouch",
    UncrouchTime => "UncrouchTime",
    MoveTimer => "MoveTimer",
    AvgPhysicsTime => "AvgPhysicsTime",
    RotationRate => "RotationRate",
    OldZ => "OldZ",
    OldAcceleration => "OldAcceleration",
    SimulateGravity => "bSimulateGravity",
    GroundSpeed => "GroundSpeed",
    DesiredSpeed => "DesiredSpeed",
    AvoidLedges => "bAvoidLedges",
    MinHitWall => "MinHitWall",
    StopAtLedges => "bStopAtLedges",
    Mass => "Mass",
    Pushable => "bPushable",
    UseOnlyPivotAmbient => "bUseOnlyPivotAmbient",
    OnlyOwnerSee => "bOnlyOwnerSee",
    OwnerNoSee => "bOwnerNoSee",
    // Trace actor filter (octree_line_filter, ShouldTrace, shot filter): read for
    // each octree actor that the trace visits.
    BlockZeroExtentTraces => "bBlockZeroExtentTraces",
    ShadowCast => "bShadowCast",
    ProjTarget => "bProjTarget",
    CanSeeThrough => "bCanSeeThrough",
    CanShootThroughWithRayCastingWeapon => "bCanShootThroughWithRayCastingWeapon",
    CanShootThroughWithProjectileWeapon => "bCanShootThroughWithProjectileWeapon",
    // ---- Logic (before the "Collisions" block) ----
    // Particle sub-emitters (Emitter.Emitters[i]).
    EmitterDisabled => "Disabled",
    // Collisions (collision and movement paths).
    CanCrouch => "bCanCrouch",
    MainScale => "MainScale",
    PostScale => "PostScale",
    AdjustFromWalls => "bAdjustFromWalls",
    Adjusting => "bAdjusting",
    TacticalDir => "bTacticalDir",
    Bounce => "bBounce",
    WaterVolume => "bWaterVolume",
    FixedRotationDir => "bFixedRotationDir",
    RotateToDesired => "bRotateToDesired",
    WalkingPct => "WalkingPct",
    Buoyancy => "Buoyancy",
    CrouchingPct => "CrouchingPct",
    CrouchHeight => "CrouchHeight",
    CrouchRadius => "CrouchRadius",
    AirControl => "AirControl",
    WaterSpeed => "WaterSpeed",
    LadderSpeed => "LadderSpeed",
    Destination => "Destination",
    ViewY => "ViewY",
    Health => "Health",
    EnemyTurnSpeed => "EnemyTurnSpeed",
    WallDir => "WallDir",
    GroundFriction => "GroundFriction",
    TerminalVelocity => "TerminalVelocity",
    FluidFriction => "FluidFriction",
    Gravity => "Gravity",
    ZoneVelocity => "ZoneVelocity",
    TrailerSameRotation => "bTrailerSameRotation",
    TrailerPrePivot => "bTrailerPrePivot",
    BounceVelocity => "bBounceVelocity",
    OrientOnSlope => "bOrientOnSlope",
    MaxSpeed => "MaxSpeed",
    numLandings => "numLandings",
    CollideWhenPlacing => "bCollideWhenPlacing",
    NetMode => "NetMode",
    NetDirty => "bNetDirty",
    PendingDelete => "bPendingDelete",
    ColLocation => "ColLocation",
    SightCounter => "SightCounter",
    NextController => "nextController",
}

/// Slots of a class's native fields, on 16 bits (a line of ~250 bytes instead of 2 KiB:
/// fewer cache misses on each access). `CASE_ABSENT`: the class does not have this field;
/// `CASE_OUTSIDE`: slot too large for 16 bits, looked up by name (same result).
pub(crate) struct NativeSlots([u16; NAMES.len()]);

const CASE_ABSENT: u16 = u16::MAX;
const CASE_OUTSIDE: u16 = u16::MAX - 1;

impl Vm {
    pub(crate) fn bind_native_fields(&mut self) {
        let keys = NAMES.map(str::to_ascii_lowercase);
        self.native_slots = self.classes.iter().map(|class| {
            NativeSlots(std::array::from_fn(|i| match class.prop_index.get(&keys[i]).copied() {
                None => CASE_ABSENT,
                Some(s) if s < CASE_OUTSIDE as usize => s as u16,
                Some(_) => CASE_OUTSIDE,
            }))
        }).collect();
    }

    #[inline]
    fn native_slot(&self, obj: ObjId, class: usize, field: NativeField) -> Option<usize> {
        match self.native_slots.get(class).map(|row| row.0[field as usize]) {
            Some(CASE_ABSENT) => None,
            Some(s) if s != CASE_OUTSIDE => Some(s as usize),
            _ => self.prop_slot(obj, NAMES[field as usize]).ok(),
        }
    }

    #[inline]
    pub fn native_prop(&self, obj: ObjId, field: NativeField) -> &Value {
        static VOID: Value = Value::Void;
        let object = self.object(obj);
        let slot = self.native_slot(obj, object.class, field);
        slot.and_then(|i| object.props.get(i)).unwrap_or(&VOID)
    }

    #[inline]
    pub fn native_value(&self, obj: ObjId, field: NativeField) -> Value {
        let object = self.object(obj);
        match self.native_slot(obj, object.class, field) {
            Some(i) => object.props[i].clone(),
            None => Value::Void,
        }
    }

    #[inline]
    pub(crate) fn native_object(&self, obj: ObjId, field: NativeField) -> Option<ObjId> {
        self.native_prop(obj, field).as_object().filter(|&id| self.objects.get(id).is_some_and(|o| o.is_some()))
    }

    /// `set_native_prop(obj, field, Value::vector(v…))` (or `Value::rotator` with `rot`), written
    /// into the structure already in place when it has this exact shape: same value, without allocation.
    pub(crate) fn set_native_xyz(&mut self, obj: ObjId, field: NativeField, v: [Value; 3], rot: bool) {
        let name_value = if rot { "object.rotator" } else { "object.vector" };
        if let Some(i) = self.native_slot(obj, self.object(obj).class, field) {
            if let Some(o) = self.objects[obj].as_mut() {
                match &mut o.props[i] {
                    // Inline vector / rotator: written in place when the three values have the inline shape.
                    Value::Vector(a) if !rot && matches!(v, [Value::Float(_), Value::Float(_), Value::Float(_)]) => {
                        *a = v.map(|c| c.as_float());
                    }
                    Value::Rotator(a) if rot && matches!(v, [Value::Int(_), Value::Int(_), Value::Int(_)]) => {
                        *a = v.map(|c| c.as_int());
                    }
                    Value::Struct(n, f) if n == name_value && f.len() == 3 => {
                        let [x, y, z] = v;
                        f[0] = x;
                        f[1] = y;
                        f[2] = z;
                    }
                    p => {
                        let [x, y, z] = v;
                        *p = Value::Struct(name_value.into(), vec![x, y, z].into()).fold();
                    }
                }
            }
        }
    }

    /// Value of the native field in place (without copying), to write it field by field.
    #[inline]
    pub(crate) fn native_prop_mut(&mut self, obj: ObjId, field: NativeField) -> Option<&mut Value> {
        let i = self.native_slot(obj, self.object(obj).class, field)?;
        self.objects[obj].as_mut()?.props.get_mut(i)
    }

    #[inline]
    /// Slot of `field` in the Props of `obj`.
    pub(crate) fn native_slot_of(&self, obj: ObjId, field: NativeField) -> Option<usize> {
        self.native_slot(obj, self.object(obj).class, field)
    }

    // ---- Typed access by index, without name, copy or allocation ----
    // Same result as access by name: `prop(obj, "Location").as_vector()`,
    // `set_prop(obj, "Velocity", Value::vector(x, y, z))`…

    /// Vector of the field (like `propr(obj, name).as_vector()`).
    #[inline]
    pub fn native_vector(&self, obj: ObjId, field: NativeField) -> [f32; 3] {
        self.native_prop(obj, field).as_vector()
    }

    /// Rotator of the field (like `propr(obj, name).as_rotator()`).
    #[inline]
    pub fn native_rotator(&self, obj: ObjId, field: NativeField) -> [i32; 3] {
        self.native_prop(obj, field).as_rotator()
    }

    /// Path of the field's data object (mesh, texture…) without copying it; None if the
    /// field is not a Value::Asset.
    #[inline]
    pub fn native_asset(&self, obj: ObjId, field: NativeField) -> Option<&str> {
        match self.native_prop(obj, field) {
            Value::Asset(p) => Some(p.as_str()),
            _ => None,
        }
    }

    /// = `set_prop(obj, name, Value::vector(v[0], v[1], v[2]))`, written in place without allocation
    /// when the field is already a vector.
    #[inline]
    pub fn set_native_vector(&mut self, obj: ObjId, field: NativeField, v: [f32; 3]) {
        self.set_native_xyz(obj, field, v.map(Value::Float), false);
    }

    /// = `set_prop(obj, name, Value::rotator(r[0], r[1], r[2]))`, in place when it is already a rotator.
    #[inline]
    pub fn set_native_rotator(&mut self, obj: ObjId, field: NativeField, r: [i32; 3]) {
        self.set_native_xyz(obj, field, r.map(Value::Int), true);
    }

    pub fn set_native_prop(&mut self, obj: ObjId, field: NativeField, value: Value) {
        if field == NativeField::Location {
        }
        if let Some(i) = self.native_slot(obj, self.object(obj).class, field) {
            if let Some(o) = self.objects[obj].as_mut() {
                o.props[i] = value;
            }
        }
    }
}

