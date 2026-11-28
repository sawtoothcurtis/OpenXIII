//! APawn::findPathToward without the world: written against `WorldPath`, which the VM and the
//! tests implement. Operations in
//! machine-code order, in f32 (square roots through appSqrt in double, rounded to 24 bits).

use crate::navigation::{NodeEval, SortedPathList};
use crate::physics::{safe_normal, sub};
use crate::value::{ObjId, Value};
use crate::Vm;

type V = [f32; 3];

const MAXPATHDISTSQ: f32 = 1.44e6;
const PHYS_WALKING: u8 = 1;
const PHYS_FALLING: u8 = 2;
const PHYS_FLYING: u8 = 4;

/// What findPathToward asks of the world.
pub(crate) trait WorldPath {
    /// Level.NavigationPointList non-null.
    fn network(&self) -> bool;
    /// The linked list of navigation points (nextNavigationPoint).
    fn points(&self) -> Vec<ObjId>;
    fn location(&self, o: ObjId) -> V;
    fn velocity(&self, o: ObjId) -> V;
    fn radius_list(&self, o: ObjId) -> f32;
    fn physics(&self, o: ObjId) -> u8;
    /// IsA(APawn) / IsA(ANavigationPoint).
    fn pawn_value(&self, o: ObjId) -> bool;
    fn point_nav(&self, o: ObjId) -> bool;
    /// APawn::jumpLanding, Stay = 0.
    fn jump_landing(&mut self, o: ObjId, v: V);
    fn moves(&mut self, p: ObjId, dest: V, test: bool, no_check: bool, attach: bool) -> bool;
    fn anchor_valid(&mut self, o: ObjId) -> bool;
    fn anchor_value(&self, o: ObjId) -> Option<ObjId>;
    fn pose_anchor(&mut self, p: ObjId, a: Option<ObjId>);
    /// APawn::clearPath / clearPaths.
    fn clears(&mut self, p: ObjId, n: ObjId);
    fn clears_all(&mut self, p: ObjId);
    fn blocking(&self, n: ObjId) -> bool;
    /// APawn::findStartAnchor / findEndAnchor.
    fn anchor_start(&mut self, l: &SortedPathList, p: ObjId) -> Option<ObjId>;
    fn anchor_arrival(&mut self, l: &SortedPathList, p: ObjId, but: Option<ObjId>, dest: V, visible: bool) -> Option<ObjId>;
    /// Pawn.Controller (+0x1F0) and its AcceptNearbyPath (vtable + 0x144).
    fn controller(&self, p: ObjId) -> Option<ObjId>;
    fn near_accepted(&mut self, c: ObjId, but: ObjId) -> bool;
    /// (bCanCrouch, bIsCrouched, CollisionHeight, CrouchHeight).
    fn crouched(&self, p: ObjId) -> (bool, bool, f32, f32);
    /// APawn::UnCrouch(1).
    fn record(&mut self, p: ObjId);
    /// APawn::ReachedDestination.
    fn reaches(&mut self, p: ObjId, dir: V, but: Option<ObjId>) -> bool;
    /// The controller's RouteCache[0] (+0x2A0).
    fn first_one(&mut self, c: ObjId, n: Option<ObjId>);
    /// bEndPoint (+0x23C bit 1).
    fn point_final(&mut self, n: ObjId);
    /// visitedWeight (+0x21C) = _ftol(StartDist).
    fn weight_visit(&mut self, n: ObjId, w: i32);
    /// APawn::calcMoveFlags.
    fn flag_bits(&mut self, p: ObjId) -> i32;
    /// APawn::breadthPathTo.
    fn width_value(&mut self, p: ObjId, eval: NodeEval, start_point: ObjId, flag_bits: i32, weight: &mut f32) -> Option<ObjId>;
    /// AController::SetRouteCache.
    fn route(&mut self, c: ObjId, end_value: ObjId, start_point: f32, arrival: f32);
}

/// VSize: appSqrt of (X² + Y²) + Z².
fn size_value(v: V) -> f32 {
    crate::ai_move::vsize(v)
}

/// APawn::findPathToward.
pub(crate) fn find_path_toward(m: &mut impl WorldPath, p: ObjId, goal: Option<ObjId>, dest: V, eval: Option<NodeEval>, clear_paths: bool, max_weight: f32) -> f32 {
    if !m.network() {
        return 0.0;
    }
    let specified_end = eval.is_none();
    let mut goal = goal;
    let mut dest = dest;
    let mut max_weight = max_weight;
    if let Some(g) = goal.filter(|&g| m.physics(p) != PHYS_FLYING && m.pawn_value(g) && m.physics(g) == PHYS_FALLING) {
        // Falling goal: its landing point is computed, the engine keeps its position.
        let current = m.location(g);
        let v = m.velocity(g);
        m.jump_landing(g, v);
        dest = current;
        goal = None;
    }
    let start = m.location(p);
    let mut end_anchor: Option<ObjId> = None;
    let mut end_dist = 0.0f32;
    let mut start_dist = 0.0f32;
    if let Some(g) = goal {
        dest = m.location(g);
        if m.physics(p) == PHYS_WALKING && m.moves(p, m.location(g), true, false, false) {
            dest = m.location(p);
            m.moves(p, start, true, true, false);
        }
        if m.point_nav(g) {
            end_anchor = Some(g);
            end_dist = 0.0;
        } else if m.pawn_value(g) && m.anchor_valid(g) {
            end_anchor = m.anchor_value(g);
            end_dist = end_anchor.map_or(0.0, |a| size_value(sub(m.location(a), dest)));
        }
    }
    if !m.anchor_valid(p) {
        m.pose_anchor(p, None);
    }
    if m.anchor_value(p).is_none() || (end_anchor.is_none() && specified_end) {
        let mut start_points = SortedPathList::new();
        let mut dest_points = SortedPathList::new();
        for n in m.points() {
            if clear_paths {
                m.clears(p, n);
            }
            if m.blocking(n) {
                continue;
            }
            let nl = m.location(n);
            if m.anchor_value(p).is_none() {
                let l = m.location(p);
                let (dx, dy, dz) = (l[0] - nl[0], l[1] - nl[1], l[2] - nl[2]);
                let d = (dx * dx + dz * dz) + dy * dy;
                if d < MAXPATHDISTSQ {
                    start_points.add_path(n, d);
                }
            }
            if end_anchor.is_none() && specified_end {
                let (dx, dy, dz) = (dest[0] - nl[0], dest[1] - nl[1], dest[2] - nl[2]);
                let d = (dx * dx + dy * dy) + dz * dz;
                if d < MAXPATHDISTSQ {
                    dest_points.add_path(n, d);
                }
            }
        }
        if m.anchor_value(p).is_none() {
            if start_points.num > 0 {
                let a = m.anchor_start(&start_points, p);
                m.pose_anchor(p, a);
            }
            let Some(a) = m.anchor_value(p) else { return 0.0 };
            start_dist = size_value(sub(m.location(a), m.location(p)));
            if start_dist <= m.radius_list(a) + m.radius_list(p) {
                start_dist = 0.0;
            }
        }
        if end_anchor.is_none() && specified_end {
            if dest_points.num > 0 {
                let any_visible = match (goal, m.controller(p)) {
                    (Some(g), Some(c)) => m.near_accepted(c, g),
                    _ => false,
                };
                end_anchor = m.anchor_arrival(&dest_points, p, goal, dest, any_visible);
            }
            let (can, is, h, ch) = m.crouched(p);
            if can && !is && h == ch {
                m.record(p);
            }
            let Some(ea) = end_anchor else { return 0.0 };
            end_dist = size_value(sub(m.location(ea), dest));
        }
        let anchor = m.anchor_value(p);
        if end_anchor.is_some() && end_anchor == anchor {
            // No path to take: goal and pawn near the same point.
            let a = anchor.unwrap_or(p);
            let c = m.controller(p);
            let to_anchor = sub(m.location(a), m.location(p));
            let first = if m.reaches(p, to_anchor, goal) {
                if goal.is_none() {
                    return 0.0;
                }
                goal
            } else {
                let d1 = safe_normal(sub(dest, m.location(a)));
                let d2 = safe_normal(sub(m.location(p), m.location(a)));
                if ((d2[2] * d1[2] + d2[1] * d1[1] + d2[0] * d1[0]) as f64) <= 0.9 {
                    Some(a)
                } else {
                    goal
                }
            };
            if let Some(c) = c {
                m.first_one(c, first);
            }
            let l = m.location(p);
            let (dx, dy, dz) = (dest[0] - l[0], dest[1] - l[1], dest[2] - l[2]);
            return (((dx * dx + dy * dy) + dz * dz) as f64).sqrt() as f32;
        }
    } else if clear_paths {
        m.clears_all(p);
    }
    if let Some(ea) = end_anchor {
        m.point_final(ea);
    }
    m.moves(p, start, true, true, false);
    let Some(anchor) = m.anchor_value(p) else { return 0.0 };
    m.weight_visit(anchor, start_dist as i32);
    let eval = eval.unwrap_or(NodeEval::EndPoint);
    let flags = m.flag_bits(p);
    let best = m.width_value(p, eval, anchor, flags, &mut max_weight);
    if let (Some(b), Some(c)) = (best, m.controller(p)) {
        m.route(c, b, start_dist, end_dist);
        return max_weight;
    }
    0.0
}

/// The VM as the world of findPathToward.
pub(crate) struct PathVm<'a>(pub &'a mut Vm);

impl WorldPath for PathVm<'_> {
    fn network(&self) -> bool {
        self.0.level.and_then(|l| self.0.po(l, "NavigationPointList")).is_some()
    }
    fn points(&self) -> Vec<ObjId> {
        self.0.navigation_points()
    }
    fn location(&self, o: ObjId) -> V {
        self.0.native_prop(o, crate::NativeField::Location).as_vector()
    }
    fn velocity(&self, o: ObjId) -> V {
        self.0.native_prop(o, crate::NativeField::Velocity).as_vector()
    }
    fn radius_list(&self, o: ObjId) -> f32 {
        self.0.native_prop(o, crate::NativeField::CollisionRadius).as_float()
    }
    fn physics(&self, o: ObjId) -> u8 {
        self.0.propr(o, "Physics").as_byte()
    }
    fn pawn_value(&self, o: ObjId) -> bool {
        self.0.is_a(o, "Pawn")
    }
    fn point_nav(&self, o: ObjId) -> bool {
        self.0.is_a(o, "NavigationPoint")
    }
    fn jump_landing(&mut self, o: ObjId, v: V) {
        self.0.jump_landing(o, v, false);
    }
    fn moves(&mut self, p: ObjId, dest: V, test: bool, no_check: bool, attach: bool) -> bool {
        self.0.far_move_actor(p, dest, test, no_check, attach)
    }
    fn anchor_valid(&mut self, o: ObjId) -> bool {
        self.0.valid_anchor(o)
    }
    fn anchor_value(&self, o: ObjId) -> Option<ObjId> {
        self.0.po(o, "Anchor")
    }
    fn pose_anchor(&mut self, p: ObjId, a: Option<ObjId>) {
        self.0.set_prop(p, "Anchor", Value::Object(a));
    }
    fn clears(&mut self, p: ObjId, n: ObjId) {
        self.0.clear_path(p, n);
    }
    fn clears_all(&mut self, p: ObjId) {
        self.0.clear_paths(p);
    }
    fn blocking(&self, n: ObjId) -> bool {
        self.0.pb(n, "bBlocked")
    }
    fn anchor_start(&mut self, l: &SortedPathList, p: ObjId) -> Option<ObjId> {
        self.0.find_start_anchor(l, p)
    }
    fn anchor_arrival(&mut self, l: &SortedPathList, p: ObjId, but: Option<ObjId>, dest: V, visible: bool) -> Option<ObjId> {
        self.0.find_end_anchor(l, p, but, dest, visible)
    }
    fn controller(&self, p: ObjId) -> Option<ObjId> {
        self.0.native_object(p, crate::NativeField::Controller)
    }
    fn near_accepted(&mut self, c: ObjId, but: ObjId) -> bool {
        self.0.accept_nearby_path(c, but)
    }
    fn crouched(&self, p: ObjId) -> (bool, bool, f32, f32) {
        (self.0.pb(p, "bCanCrouch"), self.0.pb(p, "bIsCrouched"), self.0.native_prop(p, crate::NativeField::CollisionHeight).as_float(), self.0.pf(p, "CrouchHeight"))
    }
    fn record(&mut self, p: ObjId) {
        self.0.uncrouch(p, true);
    }
    fn reaches(&mut self, p: ObjId, dir: V, but: Option<ObjId>) -> bool {
        self.0.reached_destination(p, dir, but)
    }
    fn first_one(&mut self, c: ObjId, n: Option<ObjId>) {
        let mut cache = match self.0.prop(c, "RouteCache") {
            Value::Array(v) => v,
            _ => vec![Value::Object(None); 16],
        };
        if let Some(slot) = cache.first_mut() {
            *slot = Value::Object(n);
        }
        self.0.set_prop(c, "RouteCache", Value::Array(cache));
    }
    fn point_final(&mut self, n: ObjId) {
        self.0.set_b(n, "bEndPoint", true);
    }
    fn weight_visit(&mut self, n: ObjId, w: i32) {
        self.0.set_nav_i(n, "visitedWeight", w);
    }
    fn flag_bits(&mut self, p: ObjId) -> i32 {
        self.0.calc_move_flags(p)
    }
    fn width_value(&mut self, p: ObjId, eval: NodeEval, start_point: ObjId, flag_bits: i32, weight: &mut f32) -> Option<ObjId> {
        self.0.breadth_path_to(p, eval, start_point, flag_bits, weight)
    }
    fn route(&mut self, c: ObjId, end_value: ObjId, start_point: f32, arrival: f32) {
        self.0.set_route_cache(c, end_value, start_point, arrival);
    }
}

/// What SetPath asks of the world.
pub(crate) trait WorldSetPath {
    /// RouteCache[0] (+0x2A0), if it still exists.
    fn route0(&self, c: ObjId) -> Option<ObjId>;
    fn pawn_value(&self, c: ObjId) -> Option<ObjId>;
    fn anchor_valid(&mut self, p: ObjId) -> bool;
    /// GoalList (+0x274, 4 slots) and RouteGoal (+0x2E4).
    fn goals_list(&self, c: ObjId) -> [Option<ObjId>; 4];
    fn pose_goals(&mut self, c: ObjId, b: [Option<ObjId>; 4]);
    fn but_route(&self, c: ObjId) -> Option<ObjId>;
    /// Static list of the points already returned.
    fn rendered(&mut self) -> &mut [Option<ObjId>; 4];
    fn location(&self, o: ObjId) -> V;
    fn reachable_value(&mut self, p: ObjId, g: Option<ObjId>) -> bool;
    /// findPathToward(goal, goal's Location, no evaluator, clearPaths, 0).
    fn to(&mut self, p: ObjId, g: ObjId, dest: V) -> f32;
    /// IsProbing(SpecialHandling) / HandleSpecial.
    fn special(&mut self, b: ObjId) -> bool;
    fn handles_special(&mut self, c: ObjId, b: ObjId) -> Option<ObjId>;
}

/// AController::SetPath: first point of RouteCache, with the GoalList stack of
/// intermediate goals and the static list of points already returned (against loops);
/// SpecialHandling for a point that asks for it. (The original searches for the end of GoalList without
/// a bound; we stop at 4 slots.)
pub(crate) fn set_path(m: &mut impl WorldSetPath, c: ObjId, initial: bool) -> Option<ObjId> {
    let mut best = m.route0(c);
    let Some(p) = m.pawn_value(c) else { return best };
    if !m.anchor_valid(p) {
        return best;
    }
    let mut goals = m.goals_list(c);
    let route_goal = m.but_route(c);
    if !initial {
        for g in goals.iter_mut() {
            if *g == route_goal {
                break;
            }
            if g.is_none() {
                *g = route_goal;
                break;
            }
        }
        m.pose_goals(c, goals);
    } else {
        *m.rendered() = [None; 4];
        if route_goal == goals[0] {
            if goals[1].is_some() {
                let mut i = 1;
                while i < 4 && goals[i].is_some() {
                    i += 1;
                }
                let sub_goal = goals[i - 1];
                if m.reachable_value(p, sub_goal) {
                    goals[i - 1] = None;
                    m.pose_goals(c, goals);
                    return sub_goal;
                }
                if let Some(g) = sub_goal {
                    let l = m.location(g);
                    if m.to(p, g, l) > 0.0 {
                        best = set_path(m, c, false);
                    }
                }
            }
        } else {
            m.pose_goals(c, [route_goal, None, None, None]);
        }
    }
    for i in 0..4 {
        match m.rendered()[i] {
            None => {
                m.rendered()[i] = best;
                break;
            }
            Some(x) if Some(x) == best => return best,
            _ => {}
        }
    }
    if let Some(b) = best {
        if m.special(b) {
            best = m.handles_special(c, b);
        }
    }
    best
}

impl WorldSetPath for PathVm<'_> {
    fn route0(&self, c: ObjId) -> Option<ObjId> {
        let cache0 = match self.0.prop(c, "RouteCache") {
            Value::Array(v) => v.first().and_then(|x| x.as_object()),
            _ => None,
        };
        cache0.filter(|&o| self.0.objects.get(o).is_some_and(|x| x.is_some()))
    }
    fn pawn_value(&self, c: ObjId) -> Option<ObjId> {
        self.0.native_object(c, crate::NativeField::Pawn)
    }
    fn anchor_valid(&mut self, p: ObjId) -> bool {
        self.0.valid_anchor(p)
    }
    fn goals_list(&self, c: ObjId) -> [Option<ObjId>; 4] {
        let mut b = [None; 4];
        if let Value::Array(v) = self.0.prop(c, "GoalList") {
            for (k, x) in v.iter().take(4).enumerate() {
                b[k] = x.as_object();
            }
        }
        b
    }
    fn pose_goals(&mut self, c: ObjId, b: [Option<ObjId>; 4]) {
        self.0.set_prop(c, "GoalList", Value::Array(b.iter().map(|&g| Value::Object(g)).collect()));
    }
    fn but_route(&self, c: ObjId) -> Option<ObjId> {
        self.0.po(c, "RouteGoal")
    }
    fn rendered(&mut self) -> &mut [Option<ObjId>; 4] {
        &mut self.0.set_path_pending
    }
    fn location(&self, o: ObjId) -> V {
        self.0.native_prop(o, crate::NativeField::Location).as_vector()
    }
    fn reachable_value(&mut self, p: ObjId, g: Option<ObjId>) -> bool {
        self.0.actor_reachable(p, g, false, false)
    }
    fn to(&mut self, p: ObjId, g: ObjId, dest: V) -> f32 {
        self.0.find_path_toward(p, Some(g), dest, None, true, 0.0)
    }
    fn special(&mut self, b: ObjId) -> bool {
        self.0.is_probing(b, "SpecialHandling")
    }
    fn handles_special(&mut self, c: ObjId, b: ObjId) -> Option<ObjId> {
        self.0.handle_special(c, b)
    }
}

