//! Reachability and crouching of a pawn without the world: APawn::actorReachable,
//! pointReachable, Crouch and UnCrouch, written against
//! traits (`WorldReached`, `WorldCrouched`) that the VM and the tests
//! implement. Operations in machine-code order, in f32.

use crate::physics::CheckResult;
use crate::value::{ObjId, Value};
use crate::Vm;

type V = [f32; 3];

/// MAXPATHDISTSQ.
const MAXPATHDISTSQ: f32 = 1.44e6;

/// What actorReachable and pointReachable ask of the world.
pub(crate) trait WorldReached {
    fn exists(&self, o: ObjId) -> bool;
    fn location(&self, o: ObjId) -> V;
    fn radius_list(&self, o: ObjId) -> f32;
    fn height_eyes(&self, p: ObjId) -> f32;
    /// IsA(APickup) and MyMarker (+0x1F8) non-null: the marker.
    fn marker_value(&self, o: ObjId) -> Option<ObjId>;
    /// IsA(ANavigationPoint).
    fn point_nav(&self, o: ObjId) -> bool;
    /// IsA(APawn).
    fn pawn_value(&self, o: ObjId) -> bool;
    /// APawn::ValidAnchor.
    fn anchor_valid(&mut self, p: ObjId) -> bool;
    fn anchor_value(&self, p: ObjId) -> Option<ObjId>;
    /// The pawn's HurtByVolume for the other one (vtable + 0x140).
    fn hurt(&mut self, p: ObjId, other_one: ObjId) -> bool;
    /// PhysicsVolume.bWaterVolume of the other one.
    fn in_water_volume(&self, o: ObjId) -> bool;
    /// (bCanSwim, bCanFly, bCanWalk) of the pawn.
    fn capabilities_list(&self, p: ObjId) -> (bool, bool, bool);
    /// XLevel->SingleLineCheck without extent: (Time, Actor).
    fn trace(&mut self, p: ObjId, end: V, start: V, flags: u32) -> (f32, Option<ObjId>);
    /// XLevel->FarMoveActor(pawn, dest, bTest, bNoCheck, bAttachedMove).
    fn moves(&mut self, p: ObjId, dest: V, test: bool, no_check: bool, attach: bool) -> bool;
    /// APawn::Reachable.
    fn reachable_value(&mut self, p: ObjId, dest: V, but: Option<ObjId>) -> i32;
}

fn minus(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// APawn::actorReachable.
pub(crate) fn actor_reachable(m: &mut impl WorldReached, p: ObjId, other: Option<ObjId>, know_visible: bool, no_anchor_check: bool) -> bool {
    let Some(mut other) = other.filter(|&o| m.exists(o)) else { return false };
    // A pickup is tested through its InventorySpot for the Anchor.
    let mut pickup = None;
    if let Some(marker) = m.marker_value(other) {
        pickup = Some(other);
        other = marker;
    }
    let radius = m.radius_list(p);
    if !no_anchor_check && m.point_nav(other) && radius <= 120.0 {
        let r = if 48.0 < radius { radius } else { 48.0 };
        let r2 = r * r;
        if m.anchor_valid(p) && m.anchor_value(p) == Some(other) {
            let d = minus(m.location(other), m.location(p));
            if d[1] * d[1] + d[0] * d[0] < r2 {
                return true;
            }
        }
    }
    if let Some(pk) = pickup {
        other = pk;
    }
    let ol = m.location(other);
    let loc = m.location(p);
    let d = minus(ol, loc);
    let dist_sq = (d[0] * d[0] + d[1] * d[1]) + d[2] * d[2];
    if dist_sq > MAXPATHDISTSQ {
        return false;
    }
    if m.hurt(p, other) {
        return false;
    }
    let (swim, fly, walk) = m.capabilities_list(p);
    let can = if m.in_water_volume(other) { swim } else { fly || walk };
    if !can {
        return false;
    }
    if !know_visible {
        let view = [loc[0], loc[1], loc[2] + m.height_eyes(p)];
        let (time, actor) = m.trace(p, ol, view, 0x86);
        if time != 1.0 && actor != Some(other) {
            return false;
        }
    }
    if m.pawn_value(other) {
        let t = m.radius_list(other) + radius;
        if dist_sq <= t * t {
            return true;
        }
    }
    let start = m.location(p);
    let mut dest = ol;
    if m.moves(p, ol, true, false, false) {
        dest = m.location(p);
        m.moves(p, start, true, true, false);
    }
    m.reachable_value(p, dest, Some(other)) != 0
}

/// APawn::pointReachable.
pub(crate) fn point_reachable(m: &mut impl WorldReached, p: ObjId, point: V, know_visible: bool) -> bool {
    let loc = m.location(p);
    let (dy, dx) = (point[1] - loc[1], point[0] - loc[0]);
    if dx * dx + dy * dy > MAXPATHDISTSQ {
        return false;
    }
    if !know_visible {
        let view = [loc[0], loc[1], loc[2] + m.height_eyes(p)];
        if m.trace(p, point, view, 0x286).1.is_some() {
            return false;
        }
    }
    let start = m.location(p);
    let mut dest = point;
    if m.moves(p, point, true, false, false) {
        dest = m.location(p);
        m.moves(p, start, true, true, false);
    }
    m.reachable_value(p, dest, None) != 0
}

/// What Crouch and UnCrouch ask of the world.
pub(crate) trait WorldCrouched {
    fn location(&self) -> V;
    /// (CollisionRadius, CollisionHeight).
    fn cylinder(&self) -> (f32, f32);
    /// (CrouchRadius, CrouchHeight).
    fn crouched(&self) -> (f32, f32);
    /// (CollisionRadius, CollisionHeight) of the class's default actor.
    fn default_value(&self) -> (f32, f32);
    fn pre_pivot(&self) -> V;
    fn pose_pre_pivot(&mut self, v: V);
    fn dead(&self) -> bool;
    fn pose_crouched(&mut self, yes: bool);
    /// AActor::SetCollisionSize.
    fn size_value(&mut self, radius_list: f32, height: f32);
    fn moves(&mut self, dest: V, test: bool, no_check: bool, attach: bool) -> bool;
    /// SingleLineCheck (0x286) with extent: Time.
    fn trace(&mut self, end: V, start: V, extent: V) -> f32;
    fn start_crouch(&mut self, adjust: f32);
    fn end_crouch(&mut self, adjust: f32);
    fn net_dirty(&mut self);
}

/// APawn::Crouch: crouched cylinder, the pawn goes down by the height difference
/// (PrePivot keeps the mesh in place), then StartCrouch and bNetDirty.
pub(crate) fn crouch(m: &mut impl WorldCrouched, client_simulation: bool) {
    let (r, h) = m.cylinder();
    let (cr, ch) = m.crouched();
    if h == ch && r == cr {
        return;
    }
    m.size_value(cr, ch);
    let adjust = h - ch;
    m.pose_pre_pivot([0.0, 0.0, adjust]);
    let (l, pp) = (m.location(), m.pre_pivot());
    m.moves(minus(l, pp), client_simulation, false, true);
    if !client_simulation {
        m.pose_crouched(true);
        m.start_crouch(adjust);
        m.net_dirty();
    }
}

/// APawn::UnCrouch: standing up requires the space above to be free (upward trace
/// with the standing cylinder); otherwise the pawn stays crouched.
pub(crate) fn uncrouch(m: &mut impl WorldCrouched, client_simulation: bool) {
    let (default_radius, default_height) = m.default_value();
    let adjust = default_height - m.cylinder().1;
    if m.dead() {
        m.pose_crouched(false);
        m.end_crouch(adjust);
        m.net_dirty();
        return;
    }
    let l = m.location();
    let r = m.cylinder().0;
    if m.trace([l[0], l[1], adjust + l[2]], l, [r, r, default_height]) != 1.0 {
        return;
    }
    m.size_value(default_radius, default_height);
    let pp = m.pre_pivot();
    if !m.moves([pp[0] + l[0], l[1] + pp[1], l[2] + pp[2]], client_simulation, false, true) {
        let (cr, ch) = m.crouched();
        m.size_value(cr, ch);
        return;
    }
    m.pose_pre_pivot([0.0; 3]);
    if client_simulation {
        return;
    }
    m.pose_crouched(false);
    m.end_crouch(adjust);
    m.net_dirty();
}

/// The VM as the reachability world.
pub(crate) struct ReachedVm<'a>(pub &'a mut Vm);

impl WorldReached for ReachedVm<'_> {
    fn exists(&self, o: ObjId) -> bool {
        self.0.objects.get(o).is_some_and(|x| x.is_some())
    }
    fn location(&self, o: ObjId) -> V {
        self.0.native_prop(o, crate::NativeField::Location).as_vector()
    }
    fn radius_list(&self, o: ObjId) -> f32 {
        self.0.native_prop(o, crate::NativeField::CollisionRadius).as_float()
    }
    fn height_eyes(&self, p: ObjId) -> f32 {
        self.0.pf(p, "BaseEyeHeight")
    }
    fn marker_value(&self, o: ObjId) -> Option<ObjId> {
        if self.0.is_a(o, "Pickup") {
            self.0.po(o, "MyMarker")
        } else {
            None
        }
    }
    fn point_nav(&self, o: ObjId) -> bool {
        self.0.is_a(o, "NavigationPoint")
    }
    fn pawn_value(&self, o: ObjId) -> bool {
        self.0.is_a(o, "Pawn")
    }
    fn anchor_valid(&mut self, p: ObjId) -> bool {
        self.0.valid_anchor(p)
    }
    fn anchor_value(&self, p: ObjId) -> Option<ObjId> {
        self.0.po(p, "Anchor")
    }
    fn hurt(&mut self, _p: ObjId, other_one: ObjId) -> bool {
        self.0.hurt_by_volume(other_one)
    }
    fn in_water_volume(&self, o: ObjId) -> bool {
        self.0.native_object(o, crate::NativeField::PhysicsVolume).is_some_and(|v| self.0.pb(v, "bWaterVolume"))
    }
    fn capabilities_list(&self, p: ObjId) -> (bool, bool, bool) {
        (self.0.pb(p, "bCanSwim"), self.0.pb(p, "bCanFly"), self.0.pb(p, "bCanWalk"))
    }
    fn trace(&mut self, p: ObjId, end: V, start: V, flags: u32) -> (f32, Option<ObjId>) {
        let mut hit = CheckResult::new(1.0);
        self.0.single_line_check(&mut hit, Some(p), end, start, flags, [0.0; 3]);
        (hit.time, hit.actor)
    }
    fn moves(&mut self, p: ObjId, dest: V, test: bool, no_check: bool, attach: bool) -> bool {
        self.0.far_move_actor(p, dest, test, no_check, attach)
    }
    fn reachable_value(&mut self, p: ObjId, dest: V, but: Option<ObjId>) -> i32 {
        self.0.reachable(p, dest, but)
    }
}

/// The VM as the Crouch / UnCrouch world, for the pawn of the second field.
pub(crate) struct CrouchedVm<'a>(pub &'a mut Vm, pub ObjId);

impl WorldCrouched for CrouchedVm<'_> {
    fn location(&self) -> V {
        self.0.native_prop(self.1, crate::NativeField::Location).as_vector()
    }
    fn cylinder(&self) -> (f32, f32) {
        (self.0.native_prop(self.1, crate::NativeField::CollisionRadius).as_float(), self.0.native_prop(self.1, crate::NativeField::CollisionHeight).as_float())
    }
    fn crouched(&self) -> (f32, f32) {
        (self.0.pf(self.1, "CrouchRadius"), self.0.pf(self.1, "CrouchHeight"))
    }
    fn default_value(&self) -> (f32, f32) {
        (self.0.default_f(self.1, "CollisionRadius"), self.0.default_f(self.1, "CollisionHeight"))
    }
    fn pre_pivot(&self) -> V {
        self.0.native_prop(self.1, crate::NativeField::PrePivot).as_vector()
    }
    fn pose_pre_pivot(&mut self, v: V) {
        self.0.set_v(self.1, "PrePivot", v);
    }
    fn dead(&self) -> bool {
        self.0.pb(self.1, "bIsDead")
    }
    fn pose_crouched(&mut self, yes: bool) {
        self.0.set_b(self.1, "bIsCrouched", yes);
    }
    fn size_value(&mut self, radius_list: f32, height: f32) {
        self.0.set_collision_size(self.1, radius_list, height);
    }
    fn moves(&mut self, dest: V, test: bool, no_check: bool, attach: bool) -> bool {
        self.0.far_move_actor(self.1, dest, test, no_check, attach)
    }
    fn trace(&mut self, end: V, start: V, extent: V) -> f32 {
        let mut hit = CheckResult::new(1.0);
        self.0.single_line_check(&mut hit, Some(self.1), end, start, 0x286, extent);
        hit.time
    }
    fn start_crouch(&mut self, adjust: f32) {
        self.0.event_pub(self.1, "StartCrouch", vec![Value::Float(adjust)]);
    }
    fn end_crouch(&mut self, adjust: f32) {
        self.0.event_pub(self.1, "EndCrouch", vec![Value::Float(adjust)]);
    }
    fn net_dirty(&mut self) {
        self.0.set_prop(self.1, "bNetDirty", Value::Bool(true));
    }
}

