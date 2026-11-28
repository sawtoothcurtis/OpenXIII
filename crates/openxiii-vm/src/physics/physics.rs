//! Physics and collisions of actors, reimplemented from Engine.dll (XIII 1.3) function by function.
//!
//! Each function follows the machine code of the original function whose address is given
//! (disassembly and pseudo-C): ULevel::MoveActor, FarMoveActor,
//! SingleLineCheck, SinglePointCheck, FindSpot, CheckSlice, the checks of the actor
//! octree, UPrimitive::LineCheck / PointCheck (cylinders), then pawn physics
//! (APawn::performPhysics, physWalking, physFalling, physSwimming, physFlying, physLadder, stepUp,
//! CheckForLedges, Crouch / UnCrouch, physicsRotation…).
//!
//! The map's BSP and the volumes' brushes collide through their planes, as in the engine
//! (UModel::LineCheck / PointCheck, `openxiii_assets::bsp`). StaticMeshes and the terrain are
//! triangles: traces with extent are box sweeps through separating axes there
//! (`CollisionMesh::sweep_box`).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::NativeField as Field;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::bsp::{BspHit, BspModel, Frame};

pub type V = [f32; 3];

pub(crate) mod coll {
    pub(crate) const CLONES: u32 = 1;
    pub(crate) const CONTACTS: u32 = 2;
    pub(crate) const TRIANGLES: u32 = 4;
    pub(crate) const LISTS: u32 = 8;
    pub(crate) const BASES: u32 = 16;
    pub(crate) const INDEX: u32 = 32;
    pub(crate) const POSE: u32 = 128;
    pub(crate) const ATTACHES: u32 = 256;
    pub(crate) const BRUSHES: u32 = 512;
    pub(crate) const SHAPES: u32 = 1024;

    fn mask_value() -> u32 {
        return 0;
    }

    /// True if the old path of `bit` is requested.
    #[inline]
    pub(crate) fn prior(bit: u32) -> bool {
        mask_value() & bit != 0
    }
}

// EPhysics.
pub const PHYS_NONE: u8 = 0;
pub const PHYS_WALKING: u8 = 1;
pub const PHYS_FALLING: u8 = 2;
pub const PHYS_SWIMMING: u8 = 3;
pub const PHYS_FLYING: u8 = 4;
pub const PHYS_ROTATING: u8 = 5;
pub const PHYS_PROJECTILE: u8 = 6;
pub const PHYS_MOVING_BRUSH: u8 = 8;
pub const PHYS_SPIDER: u8 = 9;
pub const PHYS_TRAILER: u8 = 10;
pub const PHYS_LADDER: u8 = 11;
pub const PHYS_ROOT_MOTION: u8 = 12;

// This engine's ETraceFlags (values read from Engine.dll's calls).
pub const TRACE_PAWNS: u32 = 0x1;
pub const TRACE_MOVERS: u32 = 0x2;
pub const TRACE_LEVEL: u32 = 0x4;
pub const TRACE_VOLUMES: u32 = 0x8;
pub const TRACE_OTHERS: u32 = 0x10;
pub const TRACE_ONLY_PROJ_ACTOR: u32 = 0x20;
pub const TRACE_BLOCKING: u32 = 0x40;
pub const TRACE_LEVEL_GEOMETRY: u32 = 0x80;
pub const TRACE_SHADOW_CAST: u32 = 0x100;
pub const TRACE_STOP_AT_FIRST_HIT: u32 = 0x200;
pub const TRACE_SINGLE_RESULT: u32 = 0x400;
pub const TRACE_MATERIAL: u32 = 0x1000;
pub const TRACE_PROJECTILE: u32 = 0x8000;
/// TRACE_World: Movers | Level | LevelGeometry.
pub const TRACE_WORLD: u32 = 0x86;

pub fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn mul(a: V, s: f32) -> V {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
/// FVector::SizeSquared: (X² + Y²) + Z².
fn size_sq(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2]
}
/// FVector::Size: appSqrt in double of (X² + Y²) + Z².
pub fn size(a: V) -> f32 {
    (size_sq(a) as f64).sqrt() as f32
}
/// FVector::SafeNormal: (X² + Y²) + Z², zero below 1e-8 (comparison in double),
/// otherwise appRSqrt.
pub fn safe_normal(a: V) -> V {
    let sq = (a[0] * a[0] + a[1] * a[1]) + a[2] * a[2];
    if (sq as f64) < 1e-8 {
        return [0.0; 3];
    }
    let s = 1.0 / sq.sqrt();
    mul(a, s)
}
/// FVector::IsNearlyZero: each component below 1e-4.
fn nearly_zero(a: V) -> bool {
    a.iter().all(|c| c.abs() < 0.0001)
}
fn is_zero(a: V) -> bool {
    a == [0.0; 3]
}
/// Rounding to a 24-bit mantissa (to nearest, even) of an x87 result whose exponent stays extended.
pub fn r24x(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let b = x.to_bits();
    let rest = b & ((1 << 29) - 1);
    let mut high = b - rest;
    if rest > (1 << 28) || (rest == (1 << 28) && high & (1 << 29) != 0) {
        high += 1 << 29;
    }
    f64::from_bits(high)
}

/// FVector::Rotation: truncated angles (_ftol), 65535 / 2π units per
/// radian (double). FPU at 24-bit precision: fpatan is not subject to it, but the
/// FMUL product and the square root (Y² + X², FSQRT) are rounded to single before the truncation.
pub fn vector_rotation(v: V) -> [i32; 3] {
    // x87 24-bit precision: mantissa rounded, exponent unbounded (X² does not overflow to inf).
    let k = 10430.218905280077f64;
    let angle = |a: f64| ((a * k) as f32) as i32;
    let yaw = angle((v[1] as f64).datan2(v[0] as f64));
    let xy = r24x(r24x(r24x(v[1] as f64 * v[1] as f64) + r24x(v[0] as f64 * v[0] as f64)).sqrt());
    let pitch = angle((v[2] as f64).datan2(xy));
    [pitch, yaw, 0]
}

/// FCheckResult: hit actor (the LevelInfo for the map's geometry), point, normal,
/// fraction of the path, item (unused) and the surface's material (the triangle's label).
#[derive(Debug, Clone, Copy)]
pub struct CheckResult {
    pub actor: Option<ObjId>,
    pub location: V,
    pub normal: V,
    pub time: f32,
    pub item: i32,
    pub material: Option<u32>,
}

impl CheckResult {
    pub fn new(time: f32) -> Self {
        CheckResult { actor: None, location: [0.0; 3], normal: [0.0; 3], time, item: -1, material: None }
    }
}

/// Shape of an actor in the collision octree.
enum Primitive<'a> {
    Cylinder,
    /// UStaticMesh of the actor (Mover included), see `static_collision`.
    StaticMesh(&'a crate::static_collision::SmShape),
    /// Brush of a volume (ABrush::GetPrimitive) and its frames.
    Model(&'a BspModel, Frame),
}

impl Vm {
    // ---- Property access ----

    pub(crate) fn pv(&self, a: ObjId, name: &str) -> V {
        self.propr(a, name).as_vector()
    }
    pub(crate) fn pf(&self, a: ObjId, name: &str) -> f32 {
        self.propr(a, name).as_float()
    }
    pub(crate) fn pb(&self, a: ObjId, name: &str) -> bool {
        self.propr(a, name).as_bool()
    }
    /// `pb` through a native field (slot per class, without looking up the name): same value,
    /// false for an unknown object like `propr`.
    #[inline]
    pub(crate) fn pbf(&self, a: ObjId, field: Field) -> bool {
        (self.objects.get(a).is_some_and(|o| o.is_some()) || self.freed.contains_key(&a)) && self.native_prop(a, field).as_bool()
    }
    // Access of collisions and movements through native fields (logic interface, without
    // name or copy); Same value: an object neither alive nor freed, or a field missing from
    // its class, gives Void on both sides.
    #[inline]
    fn exists(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some()) || self.freed.contains_key(&a)
    }
    /// `propr(a, name)` through the native field.
    #[inline]
    pub(crate) fn lp(&self, a: ObjId, field: Field, name: &str) -> &Value {
        static VOID: Value = Value::Void;
        if coll::prior(coll::INDEX) {
            return self.propr(a, name);
        }
        if self.exists(a) {
            self.native_prop(a, field)
        } else {
            &VOID
        }
    }
    #[inline]
    pub(crate) fn lb(&self, a: ObjId, field: Field, name: &str) -> bool {
        self.lp(a, field, name).as_bool()
    }
    #[inline]
    pub(crate) fn lf(&self, a: ObjId, field: Field, name: &str) -> f32 {
        self.lp(a, field, name).as_float()
    }
    #[inline]
    pub(crate) fn lv(&self, a: ObjId, field: Field, name: &str) -> V {
        self.lp(a, field, name).as_vector()
    }
    /// `set_v(a, name, v)`: written in place through the native field (set_native_vector).
    #[inline]
    pub(crate) fn set_lv(&mut self, a: ObjId, field: Field, name: &str, v: V) {
        if coll::prior(coll::INDEX) {
            return self.set_v(a, name, v);
        }
        self.set_native_vector(a, field, v);
    }
    #[inline]
    pub(crate) fn set_lf(&mut self, a: ObjId, field: Field, name: &str, f: f32) {
        if coll::prior(coll::INDEX) {
            return self.set_f(a, name, f);
        }
        self.set_native_prop(a, field, Value::Float(f));
    }
    #[inline]
    pub(crate) fn set_lb(&mut self, a: ObjId, field: Field, name: &str, b: bool) {
        if coll::prior(coll::INDEX) {
            return self.set_b(a, name, b);
        }
        self.set_native_prop(a, field, Value::Bool(b));
    }
    pub(crate) fn po(&self, a: ObjId, name: &str) -> Option<ObjId> {
        self.propr(a, name).as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()))
    }
    pub(crate) fn set_v(&mut self, a: ObjId, name: &str, v: V) {
        self.set_prop(a, name, Value::vector(v[0], v[1], v[2]));
    }
    pub(crate) fn set_f(&mut self, a: ObjId, name: &str, f: f32) {
        self.set_prop(a, name, Value::Float(f));
    }
    pub(crate) fn set_b(&mut self, a: ObjId, name: &str, b: bool) {
        self.set_prop(a, name, Value::Bool(b));
    }
    pub(crate) fn rot(&self, a: ObjId) -> [i32; 3] {
        self.native_prop(a, Field::Rotation).as_rotator()
    }
    pub fn debug_dump(&self, class: &str, props: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for o in 0..self.objects.len() {
            if self.objects[o].is_none() || !self.is_a(o, class) || self.is_cdo(o) {
                continue;
            }
            let name = self.objects[o].as_ref().map(|x| x.name.clone()).unwrap_or_default();
            let state = self.state_chain_names(o).into_iter().next().unwrap_or_default();
            let mut line = format!("{name} [{state}]");
            for p in props {
                line.push_str(&format!(" {p}={:?}", self.prop(o, p)));
            }
            out.push(line);
        }
        out
    }

    pub fn is_a(&self, a: ObjId, class: &str) -> bool {
        let Some(o) = self.objects.get(a).and_then(|o| o.as_ref()) else { return false };
        // Answer already computed for (object's class, requested name): a single cache read
        // (the name is kept in the entry), without hashing the name or walking up the parent classes.
        let h = crate::IsAEntry::case(o.class, class);
        if let Ok(cache) = self.is_a_cache.try_borrow() {
            if let Some(r) = cache[h].response(o.class, class) {
                return r;
            }
        }
        // Classes already resolved by their name (without going through the lowercase of `class`).
        let cached = self.class_cache.borrow().get(class).copied();
        let c = match cached {
            Some(c) => c,
            None => {
                let Some(c) = self.class(class) else { return false };
                self.class_cache.borrow_mut().insert(class.to_string(), c);
                c
            }
        };
        let r = self.is_child_of(o.class, c);
        if let (Some(e), Ok(mut cache)) = (crate::IsAEntry::new(o.class, class, r), self.is_a_cache.try_borrow_mut()) {
            cache[h] = e;
        }
        r
    }
    pub(crate) fn alive(&self, a: ObjId) -> bool {
        self.objects.get(a).is_some_and(|o| o.is_some())
    }

    /// Event that returns a value (eventNotifyLanded, NotifyHitWall, NotifyBump…): false if the
    /// function does not exist or if the event is cut, like ProcessEvent.
    pub(crate) fn event_ret(&mut self, obj: ObjId, name: &str, args: Vec<Value>) -> bool {
        if !self.alive(obj) || self.native_prop(obj, Field::DeleteMe).as_bool() || self.probe_blocked(obj, name) || !self.actor_events_open(obj) {
            return false;
        }
        match self.find_function(obj, name, false) {
            Some(f) => match self.call(f, obj, args.into_iter().map(Some).collect()) {
                Ok((v, _)) => v.as_bool(),
                Err(e) => {
                    let who = self.objects[obj].as_ref().map_or("?".to_string(), |o| o.name.clone());
                    self.warn(format!("{name} on {who}: {e}"));
                    false
                }
            },
            None => false,
        }
    }

    /// UObject::IsProbing: for a probe, the bit of StateFrame->ProbeMask, i.e.
    /// (state's ProbeMask | class's ProbeMask) & state's IgnoreMask, minus the Disables.
    /// The compiler only sets the bit for a defined function: a mere declaration
    /// like Controller's "event SeeMonster(Pawn Seen);" does not probe. Outside the probes: the
    /// function exists.
    pub(crate) fn is_probing(&self, obj: ObjId, name: &str) -> bool {
        if !self.alive(obj) {
            return false;
        }
        let Some(bit) = crate::world::probe_bit(name) else {
            return !self.probe_blocked(obj, name) && self.find_function(obj, name, false).is_some();
        };
        self.probe_active(obj, bit)
    }

    /// Bit `bit` of StateFrame->ProbeMask (probe part of IsProbing): (state's
    /// ProbeMask | class's ProbeMask) & state's IgnoreMask, minus the Disables.
    pub(crate) fn probe_active(&self, obj: ObjId, bit: u32) -> bool {
        if self.probe_disabled.get(&obj).is_some_and(|m| m >> bit & 1 != 0) {
            return false;
        }
        let Some(class) = self.objects[obj].as_ref().map(|o| o.class) else { return false };
        let class_mask = self.classes[class].probe_mask;
        let mask = match self.masks_state(obj) {
            Some((probe, ignore)) => (probe | class_mask) & ignore,
            None => class_mask,
        };
        mask >> bit & 1 != 0
    }

    pub(crate) fn cyl_extent(&self, a: ObjId) -> V {
        let r = self.native_prop(a, Field::CollisionRadius).as_float();
        [r, r, self.native_prop(a, Field::CollisionHeight).as_float()]
    }

    pub(crate) fn is_world_geometry(&self, a: ObjId) -> bool {
        Some(a) == self.level || self.native_prop(a, Field::WorldGeometry).as_bool()
    }

    /// AActor::GetPlayerPawn: the pawn, if it is driven by a PlayerController that has
    /// a Player (+0x330).
    fn player_pawn(&self, a: ObjId) -> bool {
        self.is(a, crate::flags_classes::D::Pawn) && self.native_object(a, Field::Controller).is_some_and(|c| self.is(c, crate::flags_classes::D::PlayerController) && self.native_object(c, Field::Player).is_some())
    }

    /// APawn::IsHumanControlled.
    pub(crate) fn is_human(&self, a: ObjId) -> bool {
        self.native_object(a, Field::Controller).is_some_and(|c| self.is_a(c, "PlayerController"))
    }

    /// AActor::IsBasedOn.
    pub(crate) fn is_based_on(&self, a: ObjId, other: ObjId) -> bool {
        let mut cur = Some(a);
        for _ in 0..64 {
            match cur {
                None => return false,
                Some(c) if c == other => return true,
                Some(c) => cur = self.native_object(c, Field::Base),
            }
        }
        false
    }

    /// AActor::IsOwnedBy: `a` is `owner` or belongs to it (chain of Owners).
    pub(crate) fn is_owned_by(&self, a: ObjId, owner: ObjId) -> bool {
        let mut cur = Some(a);
        for _ in 0..64 {
            match cur {
                None => return false,
                Some(c) if c == owner => return true,
                Some(c) => cur = self.native_object(c, Field::Owner),
            }
        }
        false
    }

    fn is_brush(&self, a: ObjId) -> bool {
        !matches!(self.native_prop(a, Field::Brush), Value::Object(None) | Value::Void) && self.is(a, crate::flags_classes::D::Brush)
    }

    /// AActor::IsEncroacher: bCollideActors, StaticMesh and IsA(Mover) (here: a
    /// Mover that has its moving collision shape).
    pub(crate) fn is_encroacher(&self, a: ObjId) -> bool {
        self.native_prop(a, Field::CollideActors).as_bool() && self.has_static_mesh(a) && self.is(a, crate::flags_classes::D::Mover)
    }

    /// StaticMesh (+ 0x13C) set: the original's test `[actor + 0x13C] != 0`.
    pub(crate) fn has_static_mesh(&self, a: ObjId) -> bool {
        if coll::prior(coll::CLONES) {
            return !matches!(self.native_value(a, Field::StaticMesh), Value::Object(None) | Value::Void);
        }
        !matches!(self.native_prop(a, Field::StaticMesh), Value::Object(None) | Value::Void)
    }

    /// AActor::IsBlockedBy.
    pub(crate) fn is_blocked_by(&self, a: ObjId, other: ObjId) -> bool {
        let b = |o: ObjId, f: Field| self.native_prop(o, f).as_bool();
        let blocking = |player_value: bool| if player_value { Field::BlockPlayers } else { Field::BlockActors };
        if self.is_world_geometry(other) {
            return b(a, Field::CollideWorld);
        }
        if self.is_brush(other) {
            if !b(a, Field::CollideWorld) {
                return false;
            }
            return b(other, blocking(self.player_pawn(a)));
        }
        if self.is_brush(a) {
            if !b(other, Field::CollideWorld) {
                return false;
            }
            return b(a, blocking(self.player_pawn(other)));
        }
        let player = self.player_pawn(a) || self.is(a, crate::flags_classes::D::Projectile);
        if !b(other, blocking(player)) {
            return false;
        }
        let other_player = self.player_pawn(other) || self.is(other, crate::flags_classes::D::Projectile);
        b(a, blocking(other_player))
    }

    // ---- Actor collisions (octree) ----

    /// Collision shape of an octree actor (GetPrimitive, `prim_kind`): brush of a
    /// volume, StaticMesh without bUseCylinderCollision (UStaticMesh::LineCheck / PointCheck otherwise
    /// defer to UPrimitive), otherwise the cylinder (GEngine->Cylinder, the pawns' UMesh).
    fn primitive(&self, o: ObjId) -> Option<Primitive<'_>> {
        use crate::static_collision::PrimKind;
        // A StaticMesh shape only exists if the primitive is a StaticMesh (kept up to date on
        // hashing, on moves and every tick); the loaded brushes are volumes.
        if let Some(shape) = self.sm_shapes.get(&o) {
            return Some(if self.lb(o, Field::UseCylinderCollision, "bUseCylinderCollision") { Primitive::Cylinder } else { Primitive::StaticMesh(shape) });
        }
        if let Some(m) = self.brushes.get(&o) {
            return Some(Primitive::Model(m, self.brush_frame(o)));
        }
        match self.prim_kind(o) {
            PrimKind::Model | PrimKind::StaticMesh(_) => None,
            _ => Some(Primitive::Cylinder),
        }
    }

    /// ShouldTrace of an actor (APawn, AMover / ADecoration,
    /// AVolume, AActor).
    pub(crate) fn should_trace(&self, o: ObjId, source: Option<ObjId>, flags: u32) -> bool {
        // Class flags: same answers as is_a by name, without hashing per visited actor.
        use crate::flags_classes::D;
        if self.is(o, D::Pawn) {
            return flags & TRACE_PAWNS != 0;
        }
        if self.is(o, D::Mover) || self.is(o, D::Decoration) {
            return flags & TRACE_MOVERS != 0;
        }
        let volume = self.is(o, D::Volume);
        if volume {
            if self.pbf(o, Field::WorldGeometry) && flags & TRACE_LEVEL_GEOMETRY != 0 {
                return true;
            }
            if flags & TRACE_VOLUMES == 0 {
                return false;
            }
        } else {
            if flags & TRACE_SHADOW_CAST != 0 {
                return self.pbf(o, Field::ShadowCast);
            }
            if self.pbf(o, Field::WorldGeometry) {
                return flags & TRACE_LEVEL_GEOMETRY != 0;
            }
            if flags & TRACE_OTHERS == 0 {
                return false;
            }
        }
        if flags & TRACE_ONLY_PROJ_ACTOR != 0 {
            return self.pbf(o, Field::ProjTarget) || (self.pbf(o, Field::BlockActors) && self.pbf(o, Field::BlockPlayers));
        }
        if flags & TRACE_BLOCKING != 0 {
            return source.is_some_and(|s| self.is_blocked_by(s, o));
        }
        true
    }

    /// Filter of actors transparent to shots: bCanSeeThrough and the
    /// bCanShootThrough* flags exclude the actor when the trace asks for it (0x2000, 0x4000, 0x8000);
    /// each reason is recorded in DiscardedHitMask.
    pub(crate) fn trace_filter(&self, o: ObjId, flags: u32, discarded: &mut u32) -> bool {
        if flags & 0xe000 == 0 {
            return true;
        }
        let mut pass = true;
        for (bit, field) in [(0x2000, Field::CanSeeThrough), (0x4000, Field::CanShootThroughWithRayCastingWeapon), (0x8000, Field::CanShootThroughWithProjectileWeapon)] {
            if flags & bit != 0 && self.pbf(o, field) {
                pass = false;
                *discarded |= bit;
            }
        }
        pass
    }

    /// Filter of an actor of ActorZeroExtentLineCheck / ActorNonZeroExtentLineCheck,
    /// before testing its box: bBlockZeroExtentTraces or
    /// bBlockNonZeroExtentTraces, neither the source nor one of its owners, ShouldTrace, then
    /// the actors transparent to shots (DiscardedHitMask).
    pub(crate) fn octree_line_filter(&self, o: ObjId, flags: u32, source: Option<ObjId>, zero: bool, discarded: &mut u32) -> bool {
        if !self.alive(o) || !self.pbf(o, if zero { Field::BlockZeroExtentTraces } else { Field::BlockNonZeroExtentTraces }) || Some(o) == source {
            return false;
        }
        if source.is_some_and(|s| self.is_owned_by(s, o)) {
            return false;
        }
        self.should_trace(o, source, flags) && self.trace_filter(o, flags, discarded)
    }

    /// GetPrimitive()->LineCheck(Hit, Actor, End, Start, Extent, 0, TraceFlags): contact if there
    /// is one.
    pub(crate) fn prim_line_check(&self, o: ObjId, end: V, start: V, extent: V, flags: u32) -> Option<CheckResult> {
        match self.primitive(o)? {
            Primitive::Cylinder => self.cylinder_line_check(o, end, start, extent),
            Primitive::StaticMesh(shape) => self.static_mesh_line_check(o, shape, end, start, extent, flags),
            // UModel::LineCheck, ExtraNodeFlags zero like the octree.
            // Hit.Material is the material of the brush's surface that was hit (FBspSurf.Material):
            // AActor::execTrace derives HitMat from it (GetMaterialSound), as for the map's BSP.
            Primitive::Model(m, frame) => {
                let mut h = BspHit::new(0.0);
                (!m.line_check(&mut h, Some(&frame), end, start, extent, 0, flags)).then_some(CheckResult {
                    actor: Some(o),
                    location: h.location,
                    normal: h.normal,
                    time: h.time,
                    item: h.item,
                    material: h.material.and_then(|m| self.package_material_tag(m)),
                })
            }
        }
    }

    /// GetPrimitive()->PointCheck(Hit, Actor, Location, Extent, 0).
    pub(crate) fn prim_point_check(&self, o: ObjId, location: V, extent: V) -> Option<CheckResult> {
        match self.primitive(o)? {
            Primitive::Cylinder => self.cylinder_point_check(o, location, extent),
            Primitive::StaticMesh(shape) => self.static_mesh_point_check(o, shape, location, extent),
            // UModel::PointCheck.
            Primitive::Model(m, frame) => {
                let mut h = BspHit::new(1.0);
                (!m.point_check(&mut h, Some(&frame), location, extent, 0)).then_some(CheckResult {
                    actor: Some(o),
                    location: h.location,
                    normal: h.normal,
                    time: h.time,
                    item: h.item,
                    material: None,
                })
            }
        }
    }

    /// FCollisionOctree::ActorLineCheck, see `octree.rs`.
    fn actor_line_check(&self, end: V, start: V, extent: V, flags: u32, source: Option<ObjId>, discarded: &mut u32) -> Vec<CheckResult> {
        self.octree_line_check(end, start, extent, flags, source, discarded)
    }

    /// UPrimitive::LineCheck: the actor's cylinder grown by the trace's extent
    /// (radius + Extent.X, half-height + Extent.Z); flat top and bottom.
    fn cylinder_line_check(&self, o: ObjId, end: V, start: V, extent: V) -> Option<CheckResult> {
        let c = self.native_prop(o, Field::Location).as_vector();
        let radius = extent[0] + self.native_prop(o, Field::CollisionRadius).as_float();
        let radius_y = extent[1] + self.native_prop(o, Field::CollisionRadius).as_float();
        let height = extent[2] + self.native_prop(o, Field::CollisionHeight).as_float();
        let top = height + c[2];
        if !((end[0] <= radius + c[0] || start[0] <= radius + c[0])
            && (c[0] - radius <= end[0] || c[0] - radius <= start[0])
            && (end[1] <= radius_y + c[1] || start[1] <= radius_y + c[1])
            && (c[1] - radius_y <= end[1] || c[1] - radius_y <= start[1])
            && (end[2] <= top || start[2] <= top))
        {
            return None;
        }
        let bottom = c[2] - height;
        if !(bottom <= end[2] || bottom <= start[2]) {
            return None;
        }
        let mut normal = [0.0f32; 3];
        let mut t0 = 0.0f32;
        let mut t1 = 1.0f32;
        if start[2] <= top || top <= end[2] {
            if start[2] < top && top < end[2] {
                t1 = t1.min((top - start[2]) / (end[2] - start[2]));
            }
        } else {
            let t = (top - start[2]) / (end[2] - start[2]);
            if t > 0.0 {
                t0 = t.max(0.0);
                normal = [0.0, 0.0, 1.0];
            }
        }
        if bottom <= start[2] || end[2] <= bottom {
            if bottom < start[2] && end[2] < bottom {
                t1 = t1.min((bottom - start[2]) / (end[2] - start[2]));
            }
        } else {
            let t = (bottom - start[2]) / (end[2] - start[2]);
            if t0 < t {
                t0 = t0.max(t);
                normal = [0.0, 0.0, -1.0];
            }
        }
        if t0 >= t1 {
            return None;
        }
        let (dx, dy) = (start[0] - c[0], start[1] - c[1]);
        let (vx, vy) = (end[0] - start[0], end[1] - start[1]);
        let a = vx * vx + vy * vy;
        let b = 2.0 * (vx * dx + vy * dy);
        let cc = (dx * dx + dy * dy) - radius * radius;
        let disc = b * b - cc * a * 4.0;
        let hit = |t: f32, normal: V| {
            let time = (t - 0.001).clamp(0.0, 1.0);
            CheckResult {
                actor: Some(o),
                location: [vx * time + start[0], vy * time + start[1], (end[2] - start[2]) * time + start[2]],
                normal,
                time,
                item: -1,
                material: None,
            }
        };
        if cc >= 1.0 || start[2] <= bottom || top <= start[2] {
            if disc >= 0.0 {
                if a >= 1e-8 {
                    let sq = (disc as f64).sqrt() as f32;
                    let t_out = t1.min((sq - b) * (0.5 / a));
                    let t_in = -((sq + b) * (0.5 / a));
                    if t0 < t_in {
                        let p = [vx * t_in + start[0] - c[0], vy * t_in + start[1] - c[1], 0.0];
                        normal = safe_normal(p);
                        t0 = t_in;
                    }
                    if t0 < t_out {
                        return Some(hit(t0, normal));
                    }
                } else if cc <= 0.0 {
                    return Some(hit(t0, normal));
                }
            }
        } else if (vx * dx + vy * dy) + (end[2] - start[2]) * 0.0 * (start[2] - c[2]) < -0.1 {
            // (X + Y) + Z.
            // Start inside the cylinder while moving into it: immediate contact.
            // Normal: SafeNormal(Start − Location) with Z × 0.0: −0 below the centre.
            return Some(CheckResult { actor: Some(o), location: start, normal: safe_normal([dx, dy, (start[2] - c[2]) * 0.0]), time: 0.0, item: -1, material: None });
        }
        None
    }

    fn cylinder_point_check(&self, o: ObjId, location: V, extent: V) -> Option<CheckResult> {
        let c = self.native_prop(o, Field::Location).as_vector();
        let dz = c[2] - location[2];
        let hz = extent[2] + self.native_prop(o, Field::CollisionHeight).as_float();
        if hz * hz <= dz * dz {
            return None;
        }
        let (dx, dy) = (c[0] - location[0], c[1] - location[1]);
        let r = extent[0] + self.native_prop(o, Field::CollisionRadius).as_float();
        if r * r <= dx * dx + dy * dy {
            return None;
        }
        let normal = safe_normal(sub(location, c));
        // UPrimitive::PointCheck: top and bottom give the same point (the
        // bottom of the extent below the centre); on the side, the actor's centre minus the
        // flat normal × Extent.X, and Z = Location.Z + (Z of that point), as the original
        // writes it. Time stays at 1 (initial value of the node's FCheckResult).
        let hit_location = if normal[2] < -0.5 || normal[2] > 0.5 {
            [location[0], location[1], c[2] - extent[2]]
        } else {
            let n = safe_normal([normal[0], normal[1], normal[2] * 0.0]);
            let k = extent[0];
            [c[0] - k * n[0], c[1] - k * n[1], location[2] + (c[2] - k * n[2])]
        };
        Some(CheckResult { actor: Some(o), location: hit_location, normal, time: 1.0, item: -1, material: None })
    }

    /// The map's BSP collides through its own planes (UModel::LineCheck / PointCheck, as the
    /// engine does); the triangle mesh then only keeps the StaticMeshes and the terrain.
    fn bsp_planes(&self) -> bool {
        self.level_bsp.is_some() && self.collision.is_some()
    }

    pub fn debug_damage(&mut self, target: &str, amount: i32, instigator: Option<ObjId>, damage_type: &str) {
        let Some(t) = self.actors.iter().copied().find(|&a| self.objects[a].as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(target))) else { return };
        let l = self.native_prop(t, Field::Location).as_vector();
        let dt = self.class(damage_type).or_else(|| self.class("DamageType"));
        self.event_pub(t, "TakeDamage", vec![Value::Int(amount), Value::Object(instigator), Value::vector(l[0], l[1], l[2]), Value::vector(0.0, 0.0, 0.0), Value::Class(dt)]);
    }

    /// With `area` (x, y, radius), only the pawns within that horizontal distance.
    pub fn debug_kill_class(&mut self, class: &str, area: Option<[f32; 3]>, instigator: Option<ObjId>, damage_type: &str) -> usize {
        let targets: Vec<String> = self
            .actors
            .iter()
            .filter_map(|&a| self.objects[a].as_ref().filter(|o| self.classes[o.class].name.eq_ignore_ascii_case(class)).map(|o| (a, o.name.clone())))
            .filter(|&(a, _)| self.lp(a, Field::Health, "Health").as_int() > 0)
            .filter(|&(a, _)| {
                let l = self.native_prop(a, Field::Location).as_vector();
                area.is_none_or(|[x, y, r]| (l[0] - x).dhypot(l[1] - y) <= r)
            })
            .map(|(_, n)| n)
            .collect();
        for t in &targets {
            self.debug_damage(t, 1000, instigator, damage_type);
        }
        targets.len()
    }

    /// Spots are searched on circles around the player, in front first: a body (35 × 35 × 40
    /// box) goes from the player to the spot without touching anything, and a nearly flat
    /// floor is under it. Failing that, the map's waypoints (PathNode, PlayerStart), the
    /// closest first. Returns the created actors. Nothing is called without the variable: the
    /// game stays unchanged.
    pub fn debug_spawn_meshes(&mut self, pawn: ObjId, meshes: &[String]) -> Vec<ObjId> {
        let Some(class) = self.class("Effects") else { return Vec::new() };
        let base = self.native_prop(pawn, Field::Location).as_vector();
        let facing = self.native_prop(pawn, Field::Rotation).as_rotator()[1] as f32 * std::f32::consts::PI / 32768.0;
        let body = [35.0, 35.0, 40.0];
        // Height of the player above the ground: the characters are placed at the same.
        let floor = |vm: &Vm, p: V, depth: f32| -> Option<(f32, f32)> {
            let mut h = CheckResult::new(1.0);
            vm.single_line_check(&mut h, None, [p[0], p[1], p[2] - depth], p, TRACE_WORLD, [0.0; 3]);
            (h.time < 1.0).then_some((h.time * depth, h.normal[2]))
        };
        let height = floor(self, base, 1000.0).map_or(80.0, |(d, _)| d.clamp(40.0, 200.0));
        let mut spots: Vec<V> = Vec::new();
        let far = |spots: &Vec<V>, p: V| spots.iter().all(|q| (p[0] - q[0]).dhypot(p[1] - q[1]) >= 120.0);
        let mut candidates: Vec<(f32, f32)> = Vec::new();
        for radius in [300.0f32, 420.0, 550.0, 700.0, 900.0, 1100.0, 1400.0] {
            let count = (radius * std::f32::consts::TAU / 130.0) as i32;
            for k in 0..count {
                let off = (k + 1) / 2 * if k % 2 == 1 { 1 } else { -1 };
                candidates.push((radius, facing + off as f32 * std::f32::consts::TAU / count as f32));
            }
        }
        for front in [true, false] {
            for &(radius, angle) in &candidates {
                if spots.len() >= meshes.len() {
                    break;
                }
                let diff = (angle - facing).dsin().datan2((angle - facing).dcos()).abs();
                if (diff <= 100f32.to_radians()) != front {
                    continue;
                }
                let p = [base[0] + radius * angle.dcos(), base[1] + radius * angle.dsin(), base[2]];
                if !far(&spots, p) {
                    continue;
                }
                let mut h = CheckResult::new(1.0);
                self.single_line_check(&mut h, Some(pawn), p, base, TRACE_WORLD, body);
                if h.time < 1.0 {
                    continue;
                }
                // Floor less than 120 units below the usual height and flat enough (slope < 45°).
                match floor(self, p, height + 120.0) {
                    Some((d, nz)) if nz >= 0.7 => spots.push([p[0], p[1], p[2] - d + height]),
                    _ => {}
                }
            }
        }
        if spots.len() < meshes.len() {
            let dist = |p: V| (p[0] - base[0]).dhypot(p[1] - base[1]);
            let mut nodes: Vec<V> = self
                .actors
                .iter()
                .copied()
                .filter(|&a| self.objects[a].is_some() && (self.is_a(a, "PathNode") || self.is_a(a, "PlayerStart")))
                .map(|a| self.native_prop(a, Field::Location).as_vector())
                .filter(|&p| dist(p) >= 150.0)
                .collect();
            nodes.sort_by(|x, y| dist(*x).total_cmp(&dist(*y)));
            for p in nodes {
                if spots.len() < meshes.len() && far(&spots, p) {
                    spots.push(p);
                }
            }
        }
        let mut out = Vec::new();
        for (mesh, p) in meshes.iter().zip(spots) {
            let face = ((base[1] - p[1]).datan2(base[0] - p[0]) * 32768.0 / std::f32::consts::PI) as i32 & 0xFFFF;
            let Ok(Some(a)) = self.spawn_actor(class, Value::Object(None), None, Value::vector(p[0], p[1], p[2]), Value::rotator(0, face, 0), Value::Object(None), true) else { continue };
            self.set_native_prop(a, Field::DrawType, Value::Byte(2));
            self.set_native_prop(a, Field::Mesh, Value::Asset(mesh.clone()));
            self.set_prop(a, "bUnlit", Value::Bool(false));
            // Pawn idle animation (XIIIPawn) rather than the reference pose, if the sequence exists.
            if let Some(f) = self.find_function(a, "LoopAnim", false) {
                let _ = self.call(f, a, vec![Some(Value::Name("WaitNeutre4".into()))]);
            }
            out.push(a);
        }
        out
    }

    pub fn debug_segment(&mut self, start: V, end: V, extent: V) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(m) = self.level_bsp.as_ref() {
            let mut h = openxiii_assets::bsp::BspHit::new(1.0);
            let clear = m.line_check(&mut h, None, end, start, extent, 0, 0);
            out.push(if clear { "BSP: nothing".into() } else { format!("BSP: {:?} normal {:?} t {} node {}", h.location, h.normal, h.time, h.item) });
        }
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, None, end, start, TRACE_WORLD, extent);
        let name = hit.actor.and_then(|a| self.objects[a].as_ref().map(|o| o.name.clone())).unwrap_or_else(|| "nothing".into());
        let detail = hit.actor.map(|a| {
            format!(
                "bCollideActors={} bBlockActors={} bBlockZeroExtentTraces={} bWorldGeometry={} StaticMesh={:?}",
                self.native_prop(a, Field::CollideActors).as_bool(),
                self.lb(a, Field::BlockActors, "bBlockActors"),
                self.lb(a, Field::BlockZeroExtentTraces, "bBlockZeroExtentTraces"),
                self.lb(a, Field::WorldGeometry, "bWorldGeometry"),
                self.native_value(a, Field::StaticMesh)
            )
        });
        let mat = hit.material.and_then(|t| self.collision_materials.get(t as usize).cloned());
        out.push(format!("TRACE_World: {name} {:?} normal {:?} t {} mat {:?} {}", hit.location, hit.normal, hit.time, mat, detail.unwrap_or_default()));
        out
    }

    /// Collision label of a surface material of a map brush (mover): the BSP's
    /// if the material is there, otherwise its path in the map's package.
    fn package_material_tag(&self, material: i32) -> Option<u32> {
        if let Some(t) = self.bsp_material_tag(material) {
            return Some(t);
        }
        let lp = &self.packages[self.map_package?];
        let name = match openxiii_package::ObjectRef::from(material) {
            openxiii_package::ObjectRef::None => return None,
            openxiii_package::ObjectRef::Export(_) => format!("{}.{}", lp.name, lp.package.object_path(material.into())),
            _ => lp.package.object_path(material.into()),
        };
        self.collision_materials.iter().position(|m| *m == name).map(|i| i as u32)
    }

    /// Collision label (index in `collision_materials`) of a BSP material.
    fn bsp_material_tag(&self, material: i32) -> Option<u32> {
        let tags = self.bsp_material_tags.get_or_init(|| {
            let mut tags = std::collections::HashMap::new();
            let (Some(bsp), Some(pi)) = (self.level_bsp.as_ref(), self.map_package) else { return tags };
            let lp = &self.packages[pi];
            for s in &bsp.surfs {
                let name = match openxiii_package::ObjectRef::from(s.material) {
                    openxiii_package::ObjectRef::None => String::new(),
                    openxiii_package::ObjectRef::Export(_) => format!("{}.{}", lp.name, lp.package.object_path(s.material.into())),
                    _ => lp.package.object_path(s.material.into()),
                };
                if let Some(i) = self.collision_materials.iter().position(|m| *m == name) {
                    tags.insert(s.material, i as u32);
                }
            }
            tags
        });
        tags.get(&material).copied()
    }

    /// Terrains of the map (ULevel.Terrains, ULevel + 0x110C) in order: ATerrainInfo::LineCheck
    /// (`terrain_check`, also reached by UTerrainPrimitive::LineCheck, which only
    /// relays) up to the end brought back by the previous contact. SingleLineCheck skips a
    /// terrain the source depends on (chain of Owners); MultiLineCheck does not filter (`source`
    /// null). Returns each contact with Time relative to the whole path, the scale and the end brought back.
    #[allow(clippy::too_many_arguments)]
    fn terrain_line_checks(&self, end: V, start: V, extent: V, source: Option<ObjId>, hits: &mut Vec<CheckResult>, scale: &mut f32, end_now: &mut V) {
        for t in &self.terrains {
            if source.is_some_and(|s| self.is_owned_by(s, t.actor())) {
                continue;
            }
            let mut h = CheckResult::new(1.0);
            if !t.line_check(&mut h, *end_now, start, extent, false) {
                continue;
            }
            h.time *= *scale;
            // Dist = |Location − Start| ((X² + Y²) + Z², appSqrt in double), K = 20.
            let d = size(sub(h.location, start));
            let s = ((20.0 + d) * h.time) / (d + 0.0001);
            *scale = if s < 1.0 { s } else { 1.0 };
            *end_now = add(start, mul(sub(end, start), *scale));
            hits.push(h);
        }
    }

    /// UModel::LineCheck of the map's BSP (through its planes), ExtraNodeFlags 0x100 for
    /// TRACETYPE_Discard* traces. `material`: the one the result had before (see `level_line_check`),
    /// kept when the trace does not write it.
    fn bsp_line_check(&self, end: V, start: V, extent: V, flags: u32, material: Option<u32>) -> Option<CheckResult> {
        let bsp = self.level_bsp.as_ref()?;
        let mut h = BspHit::new(1.0);
        let extra = if flags & 0xc000 != 0 { 0x100 } else { 0 };
        if bsp.line_check(&mut h, None, end, start, extent, extra, flags | TRACE_SINGLE_RESULT) {
            return None;
        }
        let material = if h.material_written { h.material.and_then(|m| self.bsp_material_tag(m)) } else { material };
        Some(CheckResult { actor: self.level, location: h.location, normal: h.normal, time: h.time, item: h.item, material })
    }

    /// Fixed geometry in the order of ULevel::SingleLineCheck and MultiLineCheck
    /// the terrain (without TRACE_ShadowCast), then the BSP up to the end of the path
    /// brought back just after the terrain contact. After each contact, the path's scale becomes
    /// ((K + Dist) × Time) / (Dist + 0.0001), at most 1, with K = 20 for a terrain and 5 for the BSP;
    /// the times are relative to the whole path. Returns the contacts (terrain then BSP), the scale and
    /// the end brought back for the actors. `bsp`: the LevelInfo is passed (MultiLineCheck).
    #[allow(clippy::too_many_arguments)]
    fn level_line_check(&self, end: V, start: V, extent: V, flags: u32, source: Option<ObjId>, bsp: bool, multi: bool) -> (Vec<CheckResult>, f32, V) {
        let mut hits = Vec::new();
        let mut scale = 1.0f32;
        let mut end_now = end;
        let shorten = |h: &CheckResult, k: f32| {
            let d = size(sub(h.location, start));
            let s = ((k + d) * h.time) / (d + 0.0001);
            if s < 1.0 {
                s
            } else {
                1.0
            }
        };
        if flags & TRACE_LEVEL == 0 {
            return (hits, scale, end_now);
        }
        if !self.bsp_planes() {
            if let Some(mesh) = self.collision.as_ref().filter(|_| bsp) {
                if let Some(h) = mesh.sweep_box(start, end, extent) {
                    let h = CheckResult { actor: self.level, location: h.location, normal: h.normal, time: h.time, item: -1, material: mesh.tags.get(h.triangle).copied() };
                    scale = shorten(&h, 5.0);
                    end_now = add(start, mul(sub(end, start), scale));
                    hits.push(h);
                }
            }
            return (hits, scale, end_now);
        }
        if flags & TRACE_SHADOW_CAST == 0 {
            self.terrain_line_checks(end, start, extent, if multi { None } else { source }, &mut hits, &mut scale, &mut end_now);
        }
        if !hits.is_empty() && flags & TRACE_STOP_AT_FIRST_HIT != 0 {
            return (hits, scale, end_now);
        }
        let owned = source.is_some_and(|s| self.level.is_some_and(|l| self.is_owned_by(s, l)));
        if bsp && !owned {
            // ULevel::SingleLineCheck passes the same local result (Material zeroed once)
            // to the terrain then to the BSP: a trace with extent that finds
            // no surface under its start keeps the Material of the last terrain hit.
            let material = if multi { None } else { hits.last().and_then(|h| h.material) };
            if let Some(mut h) = self.bsp_line_check(end_now, start, extent, flags, material) {
                h.time *= scale;
                scale = shorten(&h, 5.0);
                end_now = add(start, mul(sub(end, start), scale));
                hits.push(h);
            }
        }
        (hits, scale, end_now)
    }

    /// ULevel::MultiLineCheck: all the contacts of the path, sorted by time.
    pub fn multi_line_check(&self, end: V, start: V, extent: V, level: bool, flags: u32, source: Option<ObjId>) -> Vec<CheckResult> {
        let (mut hits, scale, end_now) = self.level_line_check(end, start, extent, flags, source, level, true);
        if (hits.is_empty() || flags & TRACE_STOP_AT_FIRST_HIT == 0) && flags & 0x9b != 0 {
            // At most 0x30 contacts in the array, Time × the path's scale.
            for mut h in self.actor_line_check(end_now, start, extent, flags, source, &mut 0) {
                if hits.len() >= 0x30 {
                    break;
                }
                h.time *= scale;
                hits.push(h);
            }
        }
        // appQsort → msvcr70's qsort (0x7C00317F), comparison:
        // unstable, the order of contacts at equal times is the engine's.
        crate::sorting::sort_like_runtime_qsort(&mut hits, |a, b| {
            if a.time < b.time {
                -1
            } else if a.time > b.time {
                1
            } else {
                0
            }
        });
        hits
    }

    /// TRACE_ShadowCast in SingleLineCheck (for the BSP, for the
    /// actors): contact discarded if it is from the level without a node (Item INDEX_NONE) or on a
    /// PF_Invisible | PF_FakeBackdrop surface (low byte of PolyFlags & 0x81), or from an actor
    /// without bShadowCast. The path stays shortened by a discarded BSP contact.
    fn shadow_rejects(&self, h: &CheckResult) -> bool {
        if h.actor == self.level {
            if h.item == -1 {
                return true;
            }
            let Some(bsp) = self.level_bsp.as_ref() else { return false };
            let surf = bsp.nodes.get(h.item as usize).and_then(|n| bsp.surfs.get(n.surf as usize));
            return surf.is_some_and(|s| s.poly_flags & 0x81 != 0);
        }
        !h.actor.is_some_and(|a| self.lb(a, Field::ShadowCast, "bShadowCast"))
    }

    /// ULevel::SingleLineCheck: first contact; Hit.Time = 1 and Actor = None without a
    /// contact. Returns true if there is nothing.
    pub fn single_line_check(&self, hit: &mut CheckResult, source: Option<ObjId>, end: V, start: V, flags: u32, extent: V) -> bool {
        self.single_line_check_masked(hit, source, end, start, flags, extent, &mut 0)
    }

    /// SingleLineCheck with its last parameter, DiscardedHitMask (actors discarded by
    /// TRACETYPE_Discard*, see `trace_filter`).
    #[allow(clippy::too_many_arguments)]
    pub fn single_line_check_masked(&self, hit: &mut CheckResult, source: Option<ObjId>, end: V, start: V, flags: u32, extent: V, discarded: &mut u32) -> bool {
        let (level_hits, scale, end_now) = self.level_line_check(end, start, extent, flags, source, self.level.is_some(), false);
        let mut best: Option<CheckResult> = None;
        for h in level_hits {
            if flags & TRACE_SHADOW_CAST != 0 && self.shadow_rejects(&h) {
                continue;
            }
            if best.is_none_or(|b| h.time < b.time) {
                best = Some(h);
            }
        }
        if (best.is_none() || flags & TRACE_STOP_AT_FIRST_HIT == 0) && flags & 0x9b != 0 {
            for mut h in self.actor_line_check(end_now, start, extent, flags | TRACE_SINGLE_RESULT, source, discarded) {
                h.time *= scale;
                // No actor the source depends on (chain of Owners).
                if source.is_some_and(|s| h.actor.is_some_and(|a| self.is_owned_by(s, a))) {
                    continue;
                }
                if flags & TRACE_SHADOW_CAST != 0 && self.shadow_rejects(&h) {
                    continue;
                }
                if best.is_none_or(|b| h.time < b.time) {
                    best = Some(h);
                }
            }
        }
        match best {
            Some(h) => {
                *hit = h;
                false
            }
            None => {
                hit.time = 1.0;
                hit.actor = None;
                true
            }
        }
    }

    /// ULevel::MultiPointCheck: octree actors (ActorPointCheck), then the map's BSP
    /// put at the head of the list; the terrain is not included. The flags are not used:
    /// FOctreeNode::ActorPointCheck reads those of the last ActorLineCheck (see
    /// `octree_point_check`).
    pub(crate) fn multi_point_check(&self, location: V, extent: V, _flags: u32, level: bool, actors: bool) -> Vec<CheckResult> {
        let found = if actors { self.octree_point_check(location, extent) } else { Vec::new() };
        let mut hits = Vec::new();
        if level {
            if self.bsp_planes() {
                // UModel::PointCheck of the map's BSP (ExtraNodeFlags zero).
                let mut h = BspHit::new(1.0);
                if self.level_bsp.as_ref().is_some_and(|bsp| !bsp.point_check(&mut h, None, location, extent, 0)) {
                    hits.push(CheckResult { actor: self.level, location: h.location, normal: h.normal, time: 0.0, item: h.item, material: None });
                }
            } else if let Some(h) = self.collision.as_ref().and_then(|m| m.overlap_box(location, extent)) {
                hits.push(CheckResult { actor: self.level, location, normal: h.normal, time: 0.0, item: -1, material: None });
            }
        }
        hits.extend(found);
        hits
    }

    /// ULevel::SinglePointCheck: true if the box is free; otherwise the contact whose
    /// point is the closest.
    pub fn single_point_check(&self, hit: &mut CheckResult, location: V, extent: V, flags: u32, level: bool, actors: bool) -> bool {
        let hits = self.multi_point_check(location, extent, flags, level, actors);
        let Some(first) = hits.first() else { return true };
        let mut best = *first;
        // Inline |Hit.Location − Location|²: (Z² + Y²) + X².
        let d2 = |p: V| {
            let d = sub(p, location);
            (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0]
        };
        for h in &hits[1..] {
            if d2(h.location) < d2(best.location) {
                best = *h;
            }
        }
        *hit = best;
        false
    }

    /// ULevel::FindSpot: free spot near `location` for a box `extent`.
    pub fn find_spot(&self, extent: V, location: &mut V) -> bool {
        find_spot_core(&mut HostFindSpotVm { vm: self, hit: CheckResult::new(1.0) }, extent, location)
    }

    /// AActor::IsOverlapping: never between two brushes; if one of the two blocks
    /// traces with extent, a volume's brush is tested through its PointCheck with the other's
    /// cylinder (GetEncroachCenter / GetEncroachExtent), otherwise overlap of the two cylinders.
    pub(crate) fn is_overlapping(&self, a: ObjId, other: ObjId) -> bool {
        if self.is_brush(a) && self.is_brush(other) {
            return false;
        }
        if Some(other) == self.level || a == other || !self.alive(other) {
            return false;
        }
        // A StaticMesh Mover that collides does not overlap another encroacher
        // (AActor::IsOverlapping).
        if self.is_encroacher(a) && self.is_encroacher(other) {
            return false;
        }
        // bBlockNonZeroExtentTraces of one of the two, or one of the two is an AProjector.
        let (proj_a, proj_o) = (self.is(a, crate::flags_classes::D::Projector), self.is(other, crate::flags_classes::D::Projector));
        if !self.native_prop(a, Field::BlockNonZeroExtentTraces).as_bool() && !self.native_prop(other, Field::BlockNonZeroExtentTraces).as_bool() && !proj_a && !proj_o {
            return false;
        }
        // Projector, then volume, then encroacher (on this side, then on the other): PointCheck of
        // its primitive on GetEncroachCenter / GetEncroachExtent of the other's primitive.
        if proj_a || proj_o {
            let (p, actor) = if proj_a { (a, other) } else { (other, a) };
            let (center, extent) = self.encroach_center_extent(actor);
            return self.projector_point_check(p, center, extent);
        }
        for (volume, actor) in [(a, other), (other, a)] {
            if let Some(m) = self.brushes.get(&volume) {
                let (center, extent) = self.encroach_center_extent(actor);
                let mut h = BspHit::new(1.0);
                return !m.point_check(&mut h, Some(&self.brush_frame(volume)), center, extent, 0);
            }
        }
        for (mover, actor) in [(a, other), (other, a)] {
            if self.is_encroacher(mover) {
                let (center, extent) = self.encroach_center_extent(actor);
                return self.prim_point_check(mover, center, extent).is_some();
            }
        }
        let (la, lo) = (self.native_prop(a, Field::Location).as_vector(), self.native_prop(other, Field::Location).as_vector());
        let dz = la[2] - lo[2];
        let h = self.native_prop(other, Field::CollisionHeight).as_float() + self.native_prop(a, Field::CollisionHeight).as_float();
        if dz * dz >= h * h {
            return false;
        }
        let (dx, dy) = (la[0] - lo[0], la[1] - lo[1]);
        let r = self.native_prop(other, Field::CollisionRadius).as_float() + self.native_prop(a, Field::CollisionRadius).as_float();
        dx * dx + dy * dy < r * r
    }

    /// GetPrimitive()->GetEncroachCenter / GetEncroachExtent (vfuncs + 0x88 / + 0x84): for a
    /// StaticMesh, centre (FBox::GetCenter) and half-sizes
    /// of its collision box (GetCollisionBoundingBox: the cylinder's box grown
    /// by 1 with bUseCylinderCollision); otherwise UPrimitive: Location and
    /// (CollisionRadius, CollisionRadius, CollisionHeight). The StaticMesh's box comes from the cache
    /// filled on entering the octree; when missing, the cylinder.
    pub(crate) fn encroach_center_extent(&self, a: ObjId) -> (V, V) {
        if let Some(sm) = self.prim_static_mesh(a) {
            let bx = if self.native_prop(a, Field::UseCylinderCollision).as_bool() {
                let l = self.native_prop(a, Field::Location).as_vector();
                let r = self.native_prop(a, Field::CollisionRadius).as_float() + 1.0;
                let h = self.native_prop(a, Field::CollisionHeight).as_float() + 1.0;
                Some(([l[0] - r, l[1] - r, l[2] - h], [r + l[0], r + l[1], h + l[2]]))
            } else {
                crate::with_lower(sm, |k| self.primitive_boxes.get(k).copied()).flatten().map(|b| {
                    crate::radius_iter::static_mesh_world_box(
                        &b,
                        self.native_prop(a, Field::Rotation).as_rotator(),
                        self.native_prop(a, Field::DrawScale3D).as_vector(),
                        self.native_prop(a, Field::Location).as_vector(),
                        self.native_prop(a, Field::PrePivot).as_vector(),
                        self.native_prop(a, Field::DrawScale).as_float(),
                    )
                })
            };
            if let Some(b) = bx {
                return crate::radius_iter::box_center_extent(&b);
            }
        }
        (self.native_prop(a, Field::Location).as_vector(), self.cyl_extent(a))
    }

    /// Added to Touching with the Touch event; false if the Touch undid the contact.
    fn touch_add(&mut self, a: ObjId, other: ObjId) -> bool {
        // Already in contact (common case): without copying the list.
        if !coll::prior(coll::CONTACTS) && self.touching_contains(a, other) {
            return true;
        }
        let mut list = self.touching_of(a);
        if list.contains(&other) {
            return true;
        }
        list.push(other);
        self.set_touching(a, list);
        // Co-op extra enemies (coop_enemies.rs): a copy is born touching what it stands in
        // without Touch events, as its original came with the map.
        if self.coop.enemies.quiet_spawn.is_some_and(|q| q == a || q == other) {
            return true;
        }
        self.event_pub(a, "Touch", vec![Value::Object(Some(other))]);
        self.touching_of(a).contains(&other)
    }

    /// `touching_of(a).contains(&other)` without copying the list.
    pub(crate) fn touching_contains(&self, a: ObjId, other: ObjId) -> bool {
        match self.lp(a, Field::Touching, "Touching") {
            Value::Array(items) => items.iter().any(|v| v.as_object() == Some(other)),
            _ => false,
        }
    }

    /// AActor::BeginTouch: mutual contact.
    pub(crate) fn begin_touch(&mut self, a: ObjId, other: ObjId) {
        if (self.brushes.contains_key(&a) || self.brushes.contains_key(&other)) && !self.touching_of(a).contains(&other) {
        }
        if self.touch_add(a, other) {
            self.touch_add(other, a);
        }
    }

    /// AActor::EndTouch.
    pub(crate) fn end_touch(&mut self, a: ObjId, other: ObjId, no_notify_self: bool) {
        if self.brushes.contains_key(&a) || self.brushes.contains_key(&other) {
        }
        if !no_notify_self && self.touching_of(a).contains(&other) {
            self.event_pub(a, "UnTouch", vec![Value::Object(Some(other))]);
        }
        let mut list = self.touching_of(a);
        if list.contains(&other) {
            list.retain(|&x| x != other);
            self.set_touching(a, list);
        }
        if self.touching_of(other).contains(&a) {
            self.event_pub(other, "UnTouch", vec![Value::Object(Some(a))]);
            let mut list = self.touching_of(other);
            list.retain(|&x| x != a);
            self.set_touching(other, list);
        }
    }

    /// End of the contacts that no longer overlap (end loop of MoveActor and CheckEncroachment).
    fn end_stale_touches(&mut self, a: ObjId) {
        let mut i = 0;
        let contact = |vm: &Self, i: usize| -> Option<ObjId> {
            if coll::prior(coll::CONTACTS) {
                return vm.touching_of(a).get(i).copied();
            }
            match vm.propr(a, "Touching") {
                Value::Array(items) => items.iter().filter_map(|v| v.as_object()).nth(i + usize::from(false)),
                _ => None,
            }
        };
        while let Some(o) = contact(self, i) {
            if !self.alive(o) || self.is_overlapping(a, o) {
                i += 1;
            } else {
                self.end_touch(a, o, false);
            }
        }
    }

    /// ULevel::CheckEncroachment, for a cylinder actor or a Mover: true if a
    /// blocking actor refuses to be overlapped (EncroachingOn); updates the contacts.
    pub(crate) fn check_encroachment(&mut self, a: ObjId, location: V, rotation: [i32; 3], touch_notify: bool) -> bool {
        // ULevel::CheckEncroachment: bCollideActors, bBlockActors or bBlockPlayers, or StaticMesh Mover.
        let mover = self.movers.contains_key(&a);
        if !self.native_prop(a, Field::CollideActors).as_bool() && !self.native_prop(a, Field::BlockActors).as_bool() && !self.native_prop(a, Field::BlockPlayers).as_bool() && !mover {
            return false;
        }
        let encroacher = self.native_prop(a, Field::CollideActors).as_bool() && mover;
        // Overlaps at the tested position.
        let old = (self.native_prop(a, Field::Location).as_vector(), self.rot(a));
        let f = |v: V| v.map(Value::Float);
        let i = |r: [i32; 3]| r.map(Value::Int);
        self.set_native_xyz(a, Field::Location, f(location), false);
        self.set_native_xyz(a, Field::Rotation, i(rotation), true);
        // A Mover's shape follows the tested position.
        if encroacher {
            self.update_mover_shape(a);
        }
        // FCollisionOctree::ActorEncroachmentCheck with the collision box at the tested position.
        let bx = self.octree_collision_box(a);
        // Without touch_notify (MoveActor), an actor based on `a` is never kept by the loops
        // below: its overlap (often a kDOP PointCheck of the Mover) is not computed.
        let found: Vec<ObjId> = if !touch_notify && !coll::prior(coll::BASES) {
            let mut raw_value = self.octree_encroachment_holes(a, bx);
            let event_value = raw_value.iter().any(|&o| o & crate::octree::HOLE == 0 && Some(o) != self.level && !self.is_based_on(o, a) && self.is_blocked_by(a, o));
            raw_value.retain_mut(|o| {
                if *o & crate::octree::HOLE == 0 {
                    return true;
                }
                *o &= !crate::octree::HOLE;
                event_value && self.is_overlapping(a, *o)
            });
            raw_value
        } else {
            self.octree_encroachment_check(a, bx)
        };
        self.set_native_xyz(a, Field::Location, f(old.0), false);
        self.set_native_xyz(a, Field::Rotation, i(old.1), true);
        if encroacher {
            self.update_mover_shape(a);
        }
        for &o in &found {
            if Some(o) != self.level && !self.is_based_on(o, a) && self.is_blocked_by(a, o) && {
                self.event_ret(a, "EncroachingOn", vec![Value::Object(Some(o))])
            } {
                return true;
            }
        }
        if touch_notify {
            self.end_stale_touches(a);
        }
        for &o in &found {
            if self.is_based_on(o, a) || Some(o) == self.level {
                continue;
            }
            if !self.is_blocked_by(a, o) {
                if touch_notify {
                    self.begin_touch(a, o);
                }
            } else if self.coop_player_steps_aside(a, o) {
                // Co-op: a player never crushes another one (coop_combat.rs).
            } else if self.is_probing(o, "EncroachedBy") {
                self.event_pub(o, "EncroachedBy", vec![Value::Object(Some(a))]);
            }
        }
        false
    }

    /// ULevel::MoveActor: moves an actor by `delta`, stopping at the first
    /// blocking obstacle (backing off 2 units along the path), moves the attached actors,
    /// sends Bump, Touch and UnTouch. True if the actor moved (Hit.Time > 0).
    #[allow(clippy::too_many_arguments)]
    pub fn move_actor(
        &mut self,
        a: ObjId,
        delta: V,
        new_rot: [i32; 3],
        hit: &mut CheckResult,
        test: bool,
        ignore_pawns: bool,
        ignore_bases: bool,
        no_fail: bool,
    ) -> bool {
        if !self.alive(a) || self.native_prop(a, Field::Static).as_bool() || !self.native_prop(a, crate::NativeField::Movable).as_bool() {
            return false;
        }
        let rotation = self.rot(a);
        let (delta_size, dir) = if !nearly_zero(delta) {
            // Inline Delta.Size(): appSqrt of (Z² + Y²) + X².
            let s = (((delta[2] * delta[2] + delta[1] * delta[1]) + delta[0] * delta[0]) as f64).sqrt() as f32;
            (s, mul(delta, 1.0 / s))
        } else {
            if new_rot == rotation {
                return true;
            }
            let attached = matches!(self.native_prop(a, crate::NativeField::Attached), Value::Array(v) if !v.is_empty());
            if !attached && !(self.native_prop(a, crate::NativeField::BlockActors).as_bool() && self.native_prop(a, Field::DrawType).as_byte() == 8) {
                self.set_native_xyz(a, Field::Rotation, new_rot.map(Value::Int), true);
                self.update_relative_rotation(a);
                return true;
            }
            (0.0, delta)
        };
        *hit = CheckResult::new(1.0);
        let test_adjust = 2.0f32;
        let test_delta = add(delta, mul(dir, test_adjust));
        let location = self.native_prop(a, Field::Location).as_vector();
        let collide_actors = self.native_prop(a, Field::CollideActors).as_bool();
        let collide_world = self.native_prop(a, crate::NativeField::CollideWorld).as_bool();
        let mut first_hits: Vec<CheckResult> = Vec::new();
        let mut considered = false;
        if (collide_actors || collide_world) && !(self.movers.contains_key(&a) && self.is(a, crate::flags_classes::D::Mover)) && delta_size != 0.0 {
            let mut flags = 0u32;
            if collide_actors {
                flags = u32::from(!ignore_pawns) | 0x18;
            }
            if collide_world {
                flags |= TRACE_WORLD;
            }
            if self.is(a, crate::flags_classes::D::Projectile) {
                flags |= TRACE_PROJECTILE;
            }
            first_hits = self.multi_line_check(add(location, test_delta), location, self.cyl_extent(a), collide_world, flags, Some(a));
            if collide_world || self.native_prop(a, crate::NativeField::BlockActors).as_bool() || self.native_prop(a, crate::NativeField::BlockPlayers).as_bool() {
                for h in &first_hits {
                    let Some(other) = h.actor else { continue };
                    if ignore_pawns && !self.native_prop(other, Field::Static).as_bool() && (self.is(other, crate::flags_classes::D::Pawn) || self.is(other, crate::flags_classes::D::Decoration)) {
                        continue;
                    }
                    if (ignore_bases && self.is_based_on(a, other)) || self.is_based_on(other, a) {
                        continue;
                    }
                    considered = true;
                    if self.is_blocked_by(a, other) {
                        *hit = *h;
                        break;
                    }
                }
            }
        }
        // TestAdjust backoff along the path.
        let mut moved = delta;
        if hit.time < 1.0 && !no_fail {
            let first_hit_time = (test_adjust + delta_size) * hit.time;
            if first_hit_time > test_adjust {
                moved = sub(mul(test_delta, hit.time), mul(dir, test_adjust));
                hit.time = (first_hit_time - test_adjust) / delta_size;
            } else {
                moved = [0.0; 3];
                hit.time = 0.0;
            }
        }
        // Actors other than pawns and projectiles must not overlap others.
        if !test && !no_fail && !self.is(a, crate::flags_classes::D::Pawn) && !self.is(a, crate::flags_classes::D::Projectile) && self.check_encroachment(a, add(location, moved), new_rot, false) {
            return false;
        }
        // Written into the structures in place (same value as Value::vector / Value::rotator).
        self.set_native_xyz(a, Field::Location, add(location, moved).map(Value::Float), false);
        self.set_native_xyz(a, Field::Rotation, new_rot.map(Value::Int), true);
        // FCollisionHash::MoveActor if bCollideActors; a Mover's shape at its
        // new position.
        if self.movers.contains_key(&a) {
            self.update_mover_shape(a);
        }
        if self.native_prop(a, Field::CollideActors).as_bool() {
            self.octree_move(a);
        }
        if !test {
            self.move_attached(a, moved, rotation, new_rot);
        }
        if let Some(base) = self.native_object(a, Field::Base) {
            if empty_name(self.native_prop(a, crate::NativeField::AttachmentBone)) {
                let rel = sub(self.native_prop(a, Field::Location).as_vector(), self.native_prop(base, Field::Location).as_vector());
                self.set_native_xyz(a, crate::NativeField::RelativeLocation, (rel).map(Value::Float), false);
            }
        }
        if rotation != new_rot {
            self.update_relative_rotation(a);
        }
        if !test {
            if let Some(other) = hit.actor.filter(|&o| !self.is_world_geometry(o) && self.alive(o)) {
                if !self.is_based_on(a, other) {
                    self.notify_bump(other, a);
                    self.notify_bump(a, other);
                }
            }
            if considered || !self.native_prop(a, crate::NativeField::BlockActors).as_bool() || !self.native_prop(a, crate::NativeField::BlockPlayers).as_bool() {
                for h in &first_hits {
                    if h.time >= hit.time {
                        break;
                    }
                    let Some(other) = h.actor else { continue };
                    if !self.alive(a) {
                        break;
                    }
                    if !self.is_based_on(other, a) && (!ignore_bases || !self.is_based_on(a, other)) && !self.is_blocked_by(a, other) {
                        self.begin_touch(a, other);
                    }
                }
            }
            if self.alive(a) {
                self.end_stale_touches(a);
            }
        }
        self.set_zone(a, test, false);
        if !test {
            self.update_leaves(a);
        }
        hit.time > 0.0
    }

    /// Part of ULevel::MoveActor that moves the actors attached to `a`
    /// (Attached, except those attached to a bone): rotation difference, RelativeLocation moved from the frame
    /// GMath.UnitCoords / OldRot to that of / NewRot, a pawn's controller rotation turns
    /// by the same amount, then MoveActor(Delta + ArrivalDelta, Rotation + DeltaRot, bIgnoreBases).
    pub(crate) fn move_attached(&mut self, a: ObjId, delta: V, old_rot: [i32; 3], new_rot: [i32; 3]) {
        // Nothing attached (common case): without copying the list.
        if !matches!(self.lp(a, Field::Attached, "Attached"), Value::Array(v) if !v.is_empty()) {
            return;
        }
        let Value::Array(attached) = self.prop(a, "Attached") else { return };
        if attached.is_empty() {
            return;
        }
        use openxiii_assets::coords;
        let old_c = coords::div_rotator(&coords::UNIT, old_rot);
        let new_c = coords::div_rotator(&coords::UNIT, new_rot);
        let delta_rot = if old_rot != new_rot { [0, 1, 2].map(|k| new_rot[k].wrapping_sub(old_rot[k])) } else { [0; 3] };
        for item in attached {
            let Some(o) = item.as_object().filter(|&o| self.alive(o)) else { continue };
            if !self.lp(o, Field::AttachmentBone, "AttachmentBone").as_name().is_empty() {
                continue;
            }
            let mut arrival = [0.0f32; 3];
            if old_rot != self.rot(a) {
                if let Some(c) = self.is_a(o, "Pawn").then(|| self.native_object(o, Field::Controller)).flatten() {
                    let r = self.rot(c);
                    let r = [0, 1, 2].map(|k| r[k].wrapping_add(delta_rot[k]));
                    self.set_native_prop(c, Field::Rotation, Value::rotator(r[0], r[1], r[2]));
                }
                let rl = self.lv(o, Field::RelativeLocation, "RelativeLocation");
                let along = |ax: V| (ax[1] * rl[1] + ax[2] * rl[2]) + ax[0] * rl[0];
                let (ka, kb, kc) = (along(old_c.x), along(old_c.y), along(old_c.z));
                arrival = [0, 1, 2].map(|k| ((new_c.z[k] * kc + new_c.y[k] * kb) + new_c.x[k] * ka) - rl[k]);
            }
            let r = self.rot(o);
            let r = [0, 1, 2].map(|k| r[k].wrapping_add(delta_rot[k]));
            let mut h = CheckResult::new(1.0);
            self.move_actor(o, [arrival[0] + delta[0], arrival[1] + delta[1], arrival[2] + delta[2]], r, &mut h, false, false, true, false);
        }
    }

    /// NotifyBump (APawn: the controller first, otherwise Bump; AActor::NotifyBump
    /// eventBump alone).
    pub(crate) fn notify_bump(&mut self, a: ObjId, other: ObjId) {
        if self.is_a(a, "Pawn") {
            if let Some(c) = self.native_object(a, Field::Controller) {
                // AController::eventNotifyBump, from APawn::NotifyBump.
                if self.event_ret(c, "NotifyBump", vec![Value::Object(Some(other))]) {
                    return;
                }
            }
        }
        self.event_pub(a, "Bump", vec![Value::Object(Some(other))]);
    }

    /// ULevel::FarMoveActor: teleportation (SetLocation). The actor leaves the collision
    /// hash before FindSpot (bCollideWorld or bCollideWhenPlacing) and CheckEncroachment
    /// (without bTest), and comes back into it in all cases; the attached actors follow at (Dest + Theirs) − old
    /// position; SetZone if the spot is kept, UpdateLeaves without bTest.
    pub fn far_move_actor(&mut self, a: ObjId, dest: V, test: bool, no_check: bool, attached_move: bool) -> bool {
        if !self.alive(a) || self.native_prop(a, Field::Static).as_bool() || !self.lb(a, Field::Movable, "bMovable") {
            return false;
        }
        if self.native_prop(a, Field::CollideActors).as_bool() {
            self.octree_remove(a);
        }
        let mut dest = dest;
        let mut result = true;
        // bCollideWhenPlacing does not count on a client (LevelInfo.NetMode = NM_Client).
        let client = self.level.is_some_and(|l| self.lp(l, Field::NetMode, "NetMode").as_byte() == 3);
        if !no_check && (self.lb(a, Field::CollideWorld, "bCollideWorld") || (self.lb(a, Field::CollideWhenPlacing, "bCollideWhenPlacing") && !client)) {
            result = self.find_spot(self.cyl_extent(a), &mut dest);
        }
        if result && !test && !no_check {
            result = !self.check_encroachment(a, dest, self.rot(a), true);
        }
        if result {
            if !test {
                self.set_lb(a, Field::JustTeleported, "bJustTeleported", true);
                if !attached_move {
                    self.set_base_full(a, None, [0.0, 0.0, 1.0], true);
                }
                let location = self.native_prop(a, Field::Location).as_vector();
                // Attached read once (the identifiers only, without copying the array of values).
                let attached: Vec<Option<ObjId>> = match self.lp(a, Field::Attached, "Attached") {
                    Value::Array(v) => v.iter().map(|x| x.as_object()).collect(),
                    _ => Vec::new(),
                };
                for item in attached {
                    if let Some(o) = item.filter(|&o| self.alive(o)) {
                        // (Dest + Location of the attached actor) − Location.
                        let ol = self.native_prop(o, Field::Location).as_vector();
                        let target = [0, 1, 2].map(|k| (dest[k] + ol[k]) - location[k]);
                        self.far_move_actor(o, target, false, no_check, true);
                    }
                }
            }
            // IsInOctree: an attached actor may have put it back into the hash.
            self.octree_remove(a);
            // Written in place when it is already a vector (same value as set_v, without allocation).
            let in_place = true
                && match self.native_prop_mut(a, Field::Location) {
                    Some(Value::Vector(v)) => {
                        *v = dest;
                        true
                    }
                    Some(Value::Struct(n, f)) if n == "object.vector" && f.len() == 3 => {
                        for (x, d) in f.iter_mut().zip(dest) {
                            *x = Value::Float(d);
                        }
                        true
                    }
                    _ => false,
                };
            if !in_place {
                self.set_v(a, "Location", dest);
            }
            if self.movers.contains_key(&a) {
                self.update_mover_shape(a);
            }
        }
        if self.native_prop(a, Field::CollideActors).as_bool() {
            self.octree_add(a);
        }
        if result {
            self.set_zone(a, test, false);
        }
        if !test {
            self.update_leaves(a);
        }
        result
    }

    // ---- Base ----

    /// APawn::SetBase (Floor) then AActor::SetBase.
    pub(crate) fn set_base_full(&mut self, a: ObjId, new_base: Option<ObjId>, floor: V, notify: bool) {
        if self.is_a(a, "Pawn") {
            self.set_lv(a, Field::Floor, "Floor", floor);
        }
        self.set_base(a, new_base, notify);
    }

    /// AActor::FindBase: what is 8 units below the actor becomes its base.
    pub(crate) fn find_base(&mut self, a: ObjId) {
        let mut hit = CheckResult::new(1.0);
        let l = self.native_prop(a, Field::Location).as_vector();
        self.single_line_check(&mut hit, Some(a), [l[0], l[1], l[2] - 8.0], l, 0xdf, self.cyl_extent(a));
        if self.native_object(a, Field::Base) != hit.actor {
            self.set_base_full(a, hit.actor, hit.normal, true);
        }
    }

    /// AActor::setPhysics.
    pub fn set_physics(&mut self, a: ObjId, physics: u8, new_floor: Option<ObjId>, floor_normal: V) {
        let old = self.native_prop(a, Field::Physics).as_byte();
        if old == physics {
            return;
        }
        // Leaving PHYS_RootMotion: LockRootMotion(0) of the skeletal mesh instance, before the change.
        if old == PHYS_ROOT_MOTION {
            self.sp_lock(a, 0);
        }
        self.set_native_prop(a, Field::Physics, Value::Byte(physics));
        if matches!(physics, PHYS_WALKING | PHYS_NONE | PHYS_ROOT_MOTION | PHYS_ROTATING | PHYS_SPIDER) {
            match new_floor {
                None => {
                    if !sp_op(|| "F".into()) {
                        self.find_base(a)
                    }
                }
                Some(f) if self.native_object(a, Field::Base) != Some(f) => {
                    if !sp_op(|| format!("B{} {:08x} {:08x} {:08x} 1", f, floor_normal[0].to_bits(), floor_normal[1].to_bits(), floor_normal[2].to_bits())) {
                        self.set_base_full(a, Some(f), floor_normal, true)
                    }
                }
                _ => {}
            }
            // Entering PHYS_RootMotion: LockRootMotion(1), after the base.
            if physics == PHYS_ROOT_MOTION {
                self.sp_lock(a, 1);
            }
        } else if self.native_object(a, Field::Base).is_some() {
            if !sp_op(|| "B- 00000000 00000000 3f800000 1".into()) {
                self.set_base_full(a, None, [0.0, 0.0, 1.0], true);
            }
        }
        if matches!(physics, PHYS_NONE | PHYS_ROTATING) {
            self.set_lv(a, Field::Velocity, "Velocity", [0.0; 3]);
            self.set_lv(a, Field::Acceleration, "Acceleration", [0.0; 3]);
        }
        if let Some(v) = self.native_object(a, Field::PhysicsVolume) {
            if !sp_op(|| "E".into()) {
                self.event_pub(v, "PhysicsChangedFor", vec![Value::Object(Some(a))]);
            }
        }
    }

    /// LockRootMotion of the actor's skeletal instance, if its Mesh is one.
    fn sp_lock(&mut self, a: ObjId, lock: i32) {
        self.lock_root_motion(a, lock);
    }

    /// AActor::SetCollisionSize.
    pub(crate) fn set_collision_size(&mut self, a: ObjId, radius: f32, height: f32) {
        self.set_lf(a, Field::CollisionRadius, "CollisionRadius", radius);
        self.set_lf(a, Field::CollisionHeight, "CollisionHeight", height);
        self.refresh_collider(a);
        // bNetDirty (+0x30 bit 0x20) set at the end.
        self.set_lb(a, Field::NetDirty, "bNetDirty", true);
    }

    /// AActor::SetCollision.
    pub(crate) fn set_collision(&mut self, a: ObjId, collide_actors: bool, block_actors: bool, block_players: bool) {
        self.set_lb(a, Field::CollideActors, "bCollideActors", collide_actors);
        self.set_lb(a, Field::BlockActors, "bBlockActors", block_actors);
        self.set_lb(a, Field::BlockPlayers, "bBlockPlayers", block_players);
        self.refresh_collider(a);
        // bNetDirty (+0x30 bit 0x20) set at the end.
        self.set_lb(a, Field::NetDirty, "bNetDirty", true);
    }

    /// Update of the actor in the collision list (FCollisionHash Remove / Add).
    pub(crate) fn refresh_collider(&mut self, a: ObjId) {
        self.octree_remove(a);
        if self.alive(a) && self.native_prop(a, Field::CollideActors).as_bool() {
            self.octree_add(a);
        }
        if self.brushes.contains_key(&a) {
            self.refresh_brush_boxes();
        }
    }
}

/// What ULevel::FindSpot calls: SinglePointCheck (vtable +0xCC, without actors), CheckSlice
/// (+0xC4) and SingleLineCheck (+0xD0).
pub(crate) trait HostFindSpot {
    /// True if the box is free.
    fn point_free(&mut self, location: V, extent: V) -> bool;
    fn slice_value(&mut self, location: &mut V, extent: V, keep_trying: &mut bool) -> bool;
    /// Hit.Location, Hit.Normal and Hit.Time if the trace hits (non-null Hit.Actor).
    fn trace(&mut self, end: V, start: V, flags: u32, extent: V) -> Option<(V, V, f32)>;
}

/// ULevel::CheckSlice: slice (ex, ey, 1) free at Location, then trace downwards
/// over 2 × ez; the point is raised or lowered depending on Hit.Time, pushed along the floor's normal
/// if it is still stuck, then put back by an upward trace. The FCheckResult is shared by all the
/// calls (host). The offsets go through FVectors: (0 × d, 0 × d, d × ez) added to Location
/// (a −0 of X or Y comes out as +0).
pub(crate) fn check_slice_core(h: &mut impl HostFindSpot, location: &mut V, extent: V, keep_trying: &mut bool) -> bool {
    let slice = [extent[0], extent[1], 1.0];
    *keep_trying = false;
    if !h.point_free(*location, slice) {
        *keep_trying = true;
        return false;
    }
    let start = *location;
    let hit = h.trace([start[0], start[1], start[2] - 2.0 * extent[2]], start, TRACE_WORLD, slice);
    let floor = hit.map_or([0.0; 3], |t| t.1);
    let push_out = |l: &mut V| {
        l[0] += floor[0] * extent[0];
        l[1] += floor[1] * extent[0];
    };
    match hit {
        None => location[2] -= extent[2],
        Some((_, _, t)) if t > 0.5 => {
            location[2] -= (2.0 * t - 1.0) * extent[2];
            location[2] += 1.0;
        }
        Some((_, _, t)) => {
            let d = (0.5 - t) * 2.0;
            location[0] += 0.0 * d;
            location[1] += 0.0 * d;
            location[2] += d * extent[2];
            location[2] += 1.0;
            if h.point_free(*location, extent) {
                return true;
            }
            push_out(location);
            return h.point_free(*location, extent);
        }
    }
    if !h.point_free(*location, extent) {
        push_out(location);
        return h.point_free(*location, extent);
    }
    let here = *location;
    match h.trace([here[0], here[1], here[2] + extent[2]], here, TRACE_WORLD, extent) {
        None => location[2] += extent[2],
        Some((l, _, _)) => *location = l,
    }
    true
}

/// ULevel::FindSpot.
pub(crate) fn find_spot_core(h: &mut impl HostFindSpot, extent: V, location: &mut V) -> bool {
    if h.point_free(*location, extent) {
        return true;
    }
    if is_zero(extent) {
        return false;
    }
    let start = *location;
    let mut keep_trying = true;
    if h.slice_value(location, extent, &mut keep_trying) {
        return true;
    }
    if !keep_trying {
        return false;
    }
    // (ex × 0.5, ey × 0.5, 1); offsets (i × ex) × 0.5 and (j × ey) × 0.5, i then j from −1 to 1.
    let slice = [extent[0] * 0.5, extent[1] * 0.5, 1.0];
    let mut num_hits = 0;
    for i in [-1.0f32, 1.0] {
        for j in [-1.0f32, 1.0] {
            if num_hits < 2 {
                let off = [i * extent[0] * 0.5, j * extent[1] * 0.5];
                let test = [location[0] + off[0], location[1] + off[1], location[2]];
                if h.point_free(test, slice) {
                    num_hits += 1;
                    location[0] += off[0];
                    location[1] += off[1];
                }
            }
        }
    }
    if num_hits == 0 {
        return false;
    }
    if num_hits == 1 {
        *location = sub(add(*location, *location), start);
    }
    if !h.slice_value(location, extent, &mut keep_trying) {
        return false;
    }
    let back = sub(add(start, start), *location);
    *location = h.trace(back, *location, TRACE_WORLD, extent).map_or(start, |t| t.0);
    true
}

/// A single FCheckResult for the SinglePointChecks and the return trace, like FindSpot's Hit
/// (ebp − 0x88).
struct HostFindSpotVm<'a> {
    vm: &'a Vm,
    hit: CheckResult,
}

impl HostFindSpot for HostFindSpotVm<'_> {
    fn point_free(&mut self, location: V, extent: V) -> bool {
        self.vm.single_point_check(&mut self.hit, location, extent, 0, true, false)
    }
    fn trace(&mut self, end: V, start: V, flags: u32, extent: V) -> Option<(V, V, f32)> {
        self.vm.single_line_check(&mut self.hit, None, end, start, flags, extent);
        self.hit.actor.map(|_| (self.hit.location, self.hit.normal, self.hit.time))
    }
    /// CheckSlice has its own FCheckResult (CheckSlice's stack), not FindSpot's.
    fn slice_value(&mut self, location: &mut V, extent: V, keep_trying: &mut bool) -> bool {
        check_slice_core(&mut HostFindSpotVm { vm: self.vm, hit: CheckResult::new(1.0) }, location, extent, keep_trying)
    }
}

/// Test hook of AActor::setPhysics: in tests, the outgoing calls are recorded (if a log is open) instead of being made.
#[inline(always)]
fn sp_op(_op: impl FnOnce() -> String) -> bool {
    false
}

/// `v.as_name().is_empty()` without copying the name.
pub(crate) fn empty_name(v: &Value) -> bool {
    match v {
        Value::Name(n) => n.is_empty(),
        Value::Str(s) => s.is_empty(),
        _ => true,
    }
}

