//! Controller navigation, reimplemented from Engine.dll: navigation network (XIII's FReachSpecs,
//! serialized in each NavigationPoint), APawn::findPathToward and breadthPathTo (Dijkstra on
//! the ReachSpecs, ordered list nextOrdered / prevOrdered of the points), FSortedPathList
//! (findStartAnchor / findEndAnchor), AController::FindPath / SetPath / SetRouteCache /
//! HandleSpecial, the reachability tests (actorReachable, pointReachable, Reachable,
//! walkReachable, flyReachable, swimReachable, jumpReachable, ladderReachable, walkMove, flyMove,
//! swimMove, jumpLanding, FindJumpUp, FindBestJump) and the natives FindPathTo, FindPathToward,
//! FindPathTowardNearest, FindRandomDest, ClearPaths, FindBestInventoryPath, actorReachable,
//! pointReachable; FindBestPathToward / FindBestPathTo of XIDPawn.dll. Addresses at the top of each
//! function.

#![allow(clippy::neg_multiply)] // the "× -1" reproduce the machine code's FMUL -1

use crate::ai_move::{div, fmul_d, vsize};
use crate::physics::*;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_package::ObjectRef;
use std::collections::HashMap;

/// R_WALK, R_FLY, R_SWIM, R_JUMP (EReachSpecFlags).
const R_WALK: i32 = 1;
const R_FLY: i32 = 2;
const R_SWIM: i32 = 4;
const R_JUMP: i32 = 8;
/// ETestMoveResult.
const TESTMOVE_STOPPED: i32 = 0;
const TESTMOVE_MOVED: i32 = 1;
const TESTMOVE_FELL: i32 = 2;
const TESTMOVE_HIT_GOAL: i32 = 5;

/// XIII's FReachSpec (20 bytes): link from one navigation point to another, with the maximum
/// size of the pawn that can use it (radius, height), the required abilities (R_*), the maximum
/// fall speed on arrival and the length.
#[derive(Debug, Clone, Copy)]
pub struct ReachSpec {
    pub start: ObjId,
    pub end: ObjId,
    pub reach_flags: i16,
    pub radius: i16,
    pub height: i16,
    pub max_landing: i16,
    pub distance: i16,
}

impl ReachSpec {
    /// FReachSpec::supports.
    pub fn supports(&self, radius: i32, height: i32, move_flags: i32, max_fall_speed: i32) -> bool {
        let flags = self.reach_flags as i32;
        radius <= self.radius as i32 && height <= self.height as i32 && flags & move_flags == flags && self.max_landing as i32 <= max_fall_speed
    }
}

/// Point evaluation function of breadthPathTo (weight; 1 and above: goal reached).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeEval {
    /// FindEndPoint: 1 for the point marked bEndPoint.
    EndPoint,
    /// FindRandomPath: appFrand, + 1000 for bEndPoint.
    RandomPath,
    /// FindBestInventory: desirability of an InventorySpot's item relative to the distance.
    BestInventory,
}

/// FSortedPathList: 32 navigation points sorted by increasing (squared) distance.
pub(crate) struct SortedPathList {
    path: [Option<ObjId>; 32],
    dist: [f32; 32],
    pub(crate) num: usize,
}

impl SortedPathList {
    pub(crate) fn new() -> Self {
        SortedPathList { path: [None; 32], dist: [0.0; 32], num: 0 }
    }

    /// Entries (point, distance), in list order (findPathToward).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inputs(&self) -> impl Iterator<Item = (Option<ObjId>, f32)> + '_ {
        (0..self.num.min(32)).map(|i| (self.path[i], self.dist[i]))
    }

    /// FSortedPathList::addPath: sorted insertion; shifted entries keep
    /// a distance truncated to an integer (machine code conversion).
    pub(crate) fn add_path(&mut self, node: ObjId, dist: f32) {
        let num = self.num as i32;
        let mut n = 0i32;
        if num > 8 {
            let half = num / 2;
            if dist > self.dist[half as usize] {
                n = half;
                if num > 16 && dist > self.dist[(num / 4 + n) as usize] {
                    n += num / 4;
                }
            } else if num > 16 && dist > self.dist[(num / 4) as usize] {
                n = num / 4;
            }
        }
        while n < num && dist > self.dist[n as usize] {
            n += 1;
        }
        if n >= 32 {
            return;
        }
        let mut n = n as usize;
        let mut next_path = self.path[n];
        let mut next_dist = self.dist[n];
        self.path[n] = Some(node);
        self.dist[n] = dist;
        if self.num < 32 {
            self.num += 1;
        }
        n += 1;
        while n < self.num {
            let (after_path, after_dist) = (self.path[n], self.dist[n]);
            self.path[n] = next_path;
            self.dist[n] = next_dist as i32 as f32;
            next_path = after_path;
            next_dist = after_dist;
            n += 1;
        }
    }

    pub(crate) fn paths(&self) -> Vec<ObjId> {
        self.path[..self.num].iter().flatten().copied().collect()
    }
}

/// What breadthPathTo reads from the pawn.
#[derive(Clone, Debug)]
pub(crate) struct PawnPath {
    pub collision_radius: f32,
    pub collision_height: f32,
    pub max_fall_speed: f32,
    pub walking_pct: f32,
    pub can_crouch: bool,
    pub crouch_height: f32,
    pub crouch_radius: f32,
    /// CollisionHeight of the default actor of the pawn's class.
    pub height_default: f32,
}

/// What breadthPathTo reads and writes of the navigation network: the evaluation function, the
/// search state of the points (visitedWeight, cost, nextOrdered, prevOrdered, previousPath), bBlocked,
/// HurtByVolume (pawn vtable +0x140) and the ReachSpecs.
pub(crate) trait GraphPath {
    fn eval(&mut self, n: ObjId, best: f32) -> f32;
    fn blocking(&self, n: ObjId) -> bool;
    fn hurt(&self, n: ObjId) -> bool;
    fn specs(&self, n: ObjId) -> Vec<ReachSpec>;
    fn valid_value(&self, n: ObjId) -> bool;
    fn visited(&self, n: ObjId) -> i32;
    fn set_visited(&mut self, n: ObjId, v: i32);
    fn cost(&self, n: ObjId) -> i32;
    fn next(&self, n: ObjId) -> Option<ObjId>;
    fn set_next(&mut self, n: ObjId, v: Option<ObjId>);
    fn prev(&self, n: ObjId) -> Option<ObjId>;
    fn set_prev(&mut self, n: ObjId, v: Option<ObjId>);
    fn set_previous_path(&mut self, n: ObjId, v: Option<ObjId>);
}

/// APawn::breadthPathTo without the world: Dijkstra from `start` on the ReachSpecs
/// the pawn can use; the list of points to process is chained by nextOrdered, sorted
/// by visitedWeight, with a marker `mid` that advances by half of the insertions. Returns the best
/// point according to the evaluation (Weight updated).
pub(crate) fn path_large(p: &PawnPath, g: &mut dyn GraphPath, start: ObjId, move_flags: i32, weight: &mut f32) -> Option<ObjId> {
    let mut current = Some(start);
    let mut mid = start;
    let mut best: Option<ObjId> = None;
    let mut i_radius = p.collision_radius as i32;
    let mut i_height = p.collision_height as i32;
    let i_max_fall = p.max_fall_speed as i32;
    let crouch_mult = 1.1f32 / p.walking_pct;
    if p.can_crouch {
        i_height = p.crouch_height as i32;
        i_radius = p.crouch_radius as i32;
    }
    let default_height = p.height_default;
    let mut n = 0i32;
    let mut iterations = 0i32;
    let mut total = 1i32;
    while let Some(node) = current {
        let this_weight = g.eval(node, *weight);
        if this_weight > *weight {
            *weight = this_weight;
            best = Some(node);
        }
        if *weight >= 1.0 {
            return best;
        }
        let old = iterations;
        iterations += 1;
        if old > 250 {
            if *weight > 0.0 {
                return best;
            }
            iterations = 200;
        }
        if !g.blocking(node) && !g.hurt(node) {
            let specs = g.specs(node);
            let node_weight = g.visited(node);
            for spec in specs {
                if (spec.radius as i32) < i_radius || (spec.height as i32) < i_height {
                    continue;
                }
                let flags = spec.reach_flags as i32;
                if flags & move_flags != flags || spec.max_landing as i32 > i_max_fall {
                    continue;
                }
                let end = spec.end;
                if !g.valid_value(end) {
                    continue;
                }
                let end_cost = g.cost(end);
                let next_weight = if (spec.height as i32 as f32) < default_height {
                    ((spec.distance as i32 as f32) * crouch_mult + end_cost as f32) as i32
                } else {
                    spec.distance as i32 + end_cost
                };
                let new_visit = node_weight.wrapping_add(next_weight);
                let end_visited = g.visited(end);
                if end_visited <= new_visit {
                    continue;
                }
                g.set_previous_path(end, Some(node));
                if let Some(prev) = g.prev(end) {
                    // Removed from the list.
                    let next = g.next(end);
                    g.set_next(prev, next);
                    if let Some(next) = next {
                        g.set_prev(next, Some(prev));
                    }
                    if mid == end {
                        if g.visited(prev) > new_visit {
                            mid = prev;
                        }
                    } else {
                        let mid_weight = g.visited(mid);
                        if end_visited > mid_weight && new_visit < mid_weight {
                            total -= 1;
                        }
                    }
                } else if new_visit <= g.visited(mid) {
                    total -= 1;
                } else {
                    total += 1;
                }
                g.set_visited(end, new_visit);
                // Sorted insertion, from `mid` if it is before, otherwise from the current point.
                let mut at = if g.visited(mid) < new_visit { mid } else { node };
                let mut steps = 0;
                while let Some(next) = g.next(at) {
                    if g.visited(next) >= new_visit {
                        break;
                    }
                    let s = steps;
                    steps += 1;
                    if s > 500 {
                        return None;
                    }
                    at = next;
                }
                let next = g.next(at);
                if next != Some(end) {
                    if let Some(next) = next {
                        g.set_prev(next, Some(end));
                    }
                    g.set_next(end, next);
                    g.set_next(at, Some(end));
                    g.set_prev(end, Some(at));
                }
            }
        }
        total += 1;
        let half = (total as f64 * 0.5) as i32;
        while n < half {
            n += 1;
            if let Some(next) = g.next(mid) {
                mid = next;
            }
        }
        current = g.next(node);
    }
    best
}

/// breadthPathTo in the VM.
struct GraphVm<'a> {
    vm: &'a mut Vm,
    p: ObjId,
    eval: NodeEval,
}

impl GraphPath for GraphVm<'_> {
    fn eval(&mut self, n: ObjId, best: f32) -> f32 {
        self.vm.node_eval(self.eval, n, self.p, best)
    }
    fn blocking(&self, n: ObjId) -> bool {
        self.vm.pb(n, "bBlocked")
    }
    fn hurt(&self, n: ObjId) -> bool {
        self.vm.hurt_by_volume(n)
    }
    fn specs(&self, n: ObjId) -> Vec<ReachSpec> {
        self.vm.nav.get(&n).cloned().unwrap_or_default()
    }
    fn valid_value(&self, n: ObjId) -> bool {
        self.vm.objects.get(n).is_some_and(|o| o.is_some())
    }
    fn visited(&self, n: ObjId) -> i32 {
        self.vm.nav_i(n, "visitedWeight")
    }
    fn set_visited(&mut self, n: ObjId, v: i32) {
        self.vm.set_nav_i(n, "visitedWeight", v)
    }
    fn cost(&self, n: ObjId) -> i32 {
        self.vm.nav_i(n, "cost")
    }
    fn next(&self, n: ObjId) -> Option<ObjId> {
        self.vm.po(n, "nextOrdered")
    }
    fn set_next(&mut self, n: ObjId, v: Option<ObjId>) {
        self.vm.set_nav_o(n, "nextOrdered", v)
    }
    fn prev(&self, n: ObjId) -> Option<ObjId> {
        self.vm.po(n, "prevOrdered")
    }
    fn set_prev(&mut self, n: ObjId, v: Option<ObjId>) {
        self.vm.set_nav_o(n, "prevOrdered", v)
    }
    fn set_previous_path(&mut self, n: ObjId, v: Option<ObjId>) {
        self.vm.set_nav_o(n, "previousPath", v)
    }
}

/// What SetRouteCache reads and writes of the network: visitedWeight, previousPath, nextOrdered, Location
/// of the points; the pawn's trace towards the next point (SingleLineCheck) and actorReachable.
pub(crate) trait GraphRoute {
    fn visited(&self, n: ObjId) -> i32;
    fn previous(&self, n: ObjId) -> Option<ObjId>;
    fn next(&self, n: ObjId) -> Option<ObjId>;
    fn set_next(&mut self, n: ObjId, v: Option<ObjId>);
    fn location(&self, n: ObjId) -> V;
    fn trace_free(&mut self, end: V, start: V, flags: u32) -> bool;
    fn reachable_value(&mut self, n: ObjId) -> bool;
}

pub(crate) struct OutputRoute {
    pub route_goal: ObjId,
    pub route_dist: f32,
    pub route_cache: [Option<ObjId>; 16],
}

/// AController::SetRouteCache without the world: RouteGoal, RouteDist and RouteCache
/// from the previousPath chain of the end point; the first point is skipped if the pawn
/// (`pawn_value`, its Location) directly sees and reaches the next one, close enough.
pub(crate) fn route(g: &mut dyn GraphRoute, end_path: ObjId, pawn_value: Option<V>, start_dist: f32, end_dist: f32) -> OutputRoute {
    let route_dist = g.visited(end_path) as f32 + end_dist;
    g.set_next(end_path, None);
    let mut n = end_path;
    let mut guard = 0;
    while let Some(prev) = g.previous(n) {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        g.set_next(prev, Some(n));
        n = prev;
    }
    match pawn_value {
        Some(pl) if start_dist > 0.0 => {
            if let Some(next) = g.next(n) {
                // Inline (X² + Y²) + Z².
                let nl = g.location(next);
                let d1 = vsize(sub(pl, nl));
                let d2 = vsize(sub(g.location(n), nl));
                // 0.85 in single (FMUL dword).
                if d1 < 1200.0 && d1 < (start_dist + d2) * 0.85f32 && g.trace_free(nl, pl, 0x286) && g.reachable_value(next) {
                    n = next;
                }
            }
        }
        _ => {
            if let Some(next) = g.next(n) {
                n = next;
            }
        }
    }
    let mut route_cache = [None; 16];
    let mut at = Some(n);
    for slot in &mut route_cache {
        *slot = at;
        at = at.and_then(|a| g.next(a));
    }
    OutputRoute { route_goal: end_path, route_dist, route_cache }
}

/// SetRouteCache in the VM.
struct GraphRouteVm<'a> {
    vm: &'a mut Vm,
    c: ObjId,
    p: ObjId,
}

impl GraphRoute for GraphRouteVm<'_> {
    fn visited(&self, n: ObjId) -> i32 {
        self.vm.nav_i(n, "visitedWeight")
    }
    fn previous(&self, n: ObjId) -> Option<ObjId> {
        self.vm.po(n, "previousPath")
    }
    fn next(&self, n: ObjId) -> Option<ObjId> {
        self.vm.po(n, "nextOrdered")
    }
    fn set_next(&mut self, n: ObjId, v: Option<ObjId>) {
        self.vm.set_nav_o(n, "nextOrdered", v)
    }
    fn location(&self, n: ObjId) -> V {
        self.vm.native_prop(n, crate::NativeField::Location).as_vector()
    }
    fn trace_free(&mut self, end: V, start: V, flags: u32) -> bool {
        let mut hit = CheckResult::new(1.0);
        self.vm.single_line_check(&mut hit, Some(self.c), end, start, flags, [0.0; 3]);
        hit.actor.is_none()
    }
    fn reachable_value(&mut self, n: ObjId) -> bool {
        self.vm.actor_reachable(self.p, Some(n), true, true)
    }
}

/// What walkReachable asks of the world: the pawn (Location, ZoneNumber, volume), its callees
/// (ReachedDestination, walkMove, FindBestJump, FindJumpUp, flyReachable, swimReachable,
/// HurtByVolume), the trace of the void below the pawn and the return through FarMoveActor.
pub(crate) trait WorldWalk {
    fn location(&self) -> V;
    fn reaches(&mut self, dir: V) -> bool;
    fn walk_move(&mut self, delta: V, hit: &mut CheckResult, threshold: f32) -> i32;
    fn fly_move(&mut self, delta: V, threshold: f32) -> i32;
    fn swim_move(&mut self, delta: V, threshold: f32) -> i32;
    /// Test MoveActor of (0, 0, up): nothing hit (Time 1).
    fn rises(&mut self, up: f32) -> bool;
    /// SingleLineCheck (0x286): nothing hit (Time 1).
    fn trace_empty(&mut self, end: V, start: V, extent: V) -> bool;
    fn zone_number(&self) -> u8;
    fn outside_world(&mut self);
    fn find_best_jump(&mut self, dest: V) -> i32;
    fn find_jump_up(&mut self, delta: V) -> i32;
    fn fly_reachable(&mut self, dest: V, reach_flags: i32) -> i32;
    fn swim_reachable(&mut self, dest: V, reach_flags: i32) -> i32;
    fn hurt(&mut self) -> bool;
    fn in_water_volume(&self) -> bool;
    /// WarpZoneMarker goal: the pawn's Region.Zone == the goal's markedWarpZone; None for another goal.
    fn zone_marked(&self) -> Option<bool>;
    /// CollisionHeight of the goal, if there is one.
    fn height_but(&self) -> Option<f32>;
    fn return_value(&mut self, start: V, velocity: V);
}

/// Pawn of walkReachable.
#[derive(Clone, Debug)]
pub(crate) struct PawnWalk {
    pub velocity: V,
    pub collision_radius: f32,
    pub collision_height: f32,
    pub can_jump: bool,
    pub can_fly: bool,
    pub can_swim: bool,
    pub can_walk: bool,
}

/// APawn::walkReachable without the world: test walk in steps (pawn radius, at
/// least 12, or 128 if it can jump), with jumps, flying and swimming when the pawn can. Returns the R_*
/// used (0 if the goal is not reached).
pub(crate) fn walk(p: &PawnWalk, m: &mut impl WorldWalk, dest: V, reach_flags: i32) -> i32 {
    let start = m.location();
    let mut reach_flags = reach_flags | R_WALK;
    let mut success = 0i32;
    let mut still_moving = TESTMOVE_MOVED;
    let mut step = p.collision_radius;
    if !p.can_jump {
        if step <= 12.0 {
            step = 12.0;
        }
    } else if step <= 128.0 {
        step = 128.0;
    }
    let mut loops = 100;
    let step_sq = step * step;
    let mut hit = CheckResult::new(1.0);
    while still_moving == TESTMOVE_MOVED {
        let l = m.location();
        let mut dir = sub(dest, l);
        if m.reaches(dir) {
            still_moving = TESTMOVE_STOPPED;
            success = 1;
            continue;
        }
        dir[2] = 0.0;
        let threshold = if step_sq <= dir[0] * dir[0] + dir[1] * dir[1] {
            let n = safe_normal(dir);
            dir = [n[0] * step, n[1] * step, n[2] * step];
            4.1
        } else {
            8.0
        };
        still_moving = m.walk_move(dir, &mut hit, threshold);
        if still_moving == TESTMOVE_MOVED {
            // Void below the pawn: the path requires a jump.
            let (r, h) = (p.collision_radius, p.collision_height);
            let l = m.location();
            let down = [l[0], l[1], (fmul_d(h * 0.5 + 39.0, -1.0)) + l[2]];
            if m.trace_empty(down, l, [r * 0.5, r * 0.5, h * 0.5]) {
                reach_flags |= R_JUMP;
            }
        } else if still_moving == TESTMOVE_HIT_GOAL {
            still_moving = TESTMOVE_STOPPED;
            success = 1;
        } else if m.zone_number() == 0 {
            m.outside_world();
            still_moving = TESTMOVE_STOPPED;
            success = 0;
        } else if p.can_fly {
            still_moving = TESTMOVE_STOPPED;
            reach_flags = m.fly_reachable(dest, reach_flags);
            success = reach_flags;
        } else if !p.can_jump {
            if still_moving == TESTMOVE_FELL && step > 35.0 {
                still_moving = TESTMOVE_MOVED;
                step = 35.0;
            }
        } else {
            reach_flags |= R_JUMP;
            if still_moving == TESTMOVE_FELL {
                let target = match m.height_but() {
                    Some(gh) => [dest[0], dest[1], (dest[2] - gh) + p.collision_height],
                    None => dest,
                };
                still_moving = m.find_best_jump(target);
            } else if still_moving == TESTMOVE_STOPPED {
                still_moving = m.find_jump_up(dir);
                if still_moving == TESTMOVE_HIT_GOAL {
                    still_moving = TESTMOVE_STOPPED;
                    success = 1;
                }
            }
        }
        if m.hurt() {
            still_moving = TESTMOVE_STOPPED;
            success = 0;
        } else if m.in_water_volume() {
            still_moving = TESTMOVE_STOPPED;
            if p.can_swim {
                reach_flags = m.swim_reachable(dest, reach_flags);
                success = reach_flags;
            }
        }
        loops -= 1;
        if loops < 0 {
            still_moving = TESTMOVE_STOPPED;
        }
    }
    if success == 0 && m.zone_marked() == Some(true) {
        success = 1;
    }
    m.return_value(start, p.velocity);
    if success != 0 {
        reach_flags
    } else {
        0
    }
}

/// APawn::flyReachable without the world: test flight in steps (pawn radius, at least
/// 200); in water, swimming takes over if the pawn can swim. Returns the R_* used (0 if
/// the goal is not reached).
pub(crate) fn flight(p: &PawnWalk, m: &mut impl WorldWalk, dest: V, reach_flags: i32) -> i32 {
    let mut reach_flags = reach_flags | R_FLY;
    let mut success = 0i32;
    let start = m.location();
    let mut step = p.collision_radius;
    if step <= 200.0 {
        step = 200.0;
    }
    let mut loops = 100;
    let step_sq = step * step;
    let mut still_moving = TESTMOVE_MOVED;
    while still_moving != TESTMOVE_STOPPED {
        let mut dir = sub(dest, m.location());
        if m.reaches(dir) {
            still_moving = TESTMOVE_STOPPED;
            success = 1;
        } else {
            // (dZ² + dY²) + dX² (flyReachable, swimReachable).
            let threshold = if step_sq <= (dir[2] * dir[2] + dir[1] * dir[1]) + dir[0] * dir[0] {
                let n = safe_normal(dir);
                dir = [n[0] * step, n[1] * step, n[2] * step];
                // 4.1 in single (0x40833333), as in walkReachable.
                4.1
            } else {
                8.0
            };
            still_moving = m.fly_move(dir, threshold);
            if still_moving == TESTMOVE_HIT_GOAL {
                still_moving = TESTMOVE_STOPPED;
                success = 1;
            } else if still_moving != TESTMOVE_STOPPED && m.in_water_volume() {
                still_moving = TESTMOVE_STOPPED;
                if p.can_swim && !m.hurt() {
                    reach_flags = m.swim_reachable(dest, reach_flags);
                    success = reach_flags;
                }
            }
        }
        loops -= 1;
        if loops < 0 {
            still_moving = TESTMOVE_STOPPED;
        }
    }
    if success == 0 && m.zone_marked() == Some(true) {
        success = 1;
    }
    m.return_value(start, p.velocity);
    if success != 0 {
        reach_flags
    } else {
        0
    }
}

/// APawn::swimReachable without the world: test swim in steps (pawn radius, at least
/// 200); out of the water, flying or walking out takes over. Returns the R_* used (0
/// if the goal is not reached).
pub(crate) fn swim(p: &PawnWalk, m: &mut impl WorldWalk, dest: V, reach_flags: i32) -> i32 {
    let mut reach_flags = reach_flags | R_SWIM;
    let start = m.location();
    let mut step = p.collision_radius;
    let mut success = 0i32;
    let mut still_moving = TESTMOVE_MOVED;
    if step <= 200.0 {
        step = 200.0;
    }
    let mut loops = 100;
    let step_sq = step * step;
    while still_moving != TESTMOVE_STOPPED {
        let mut dir = sub(dest, m.location());
        if m.reaches(dir) {
            success = 1;
            still_moving = TESTMOVE_STOPPED;
        } else {
            // (dZ² + dY²) + dX² (flyReachable, swimReachable).
            let threshold = if step_sq <= (dir[2] * dir[2] + dir[1] * dir[1]) + dir[0] * dir[0] {
                let n = safe_normal(dir);
                dir = [n[0] * step, n[1] * step, n[2] * step];
                // 4.1 in single (0x40833333), as in walkReachable.
                4.1
            } else {
                8.0
            };
            still_moving = m.swim_move(dir, threshold);
            if still_moving == TESTMOVE_HIT_GOAL {
                success = 1;
                still_moving = TESTMOVE_STOPPED;
            }
            if !m.in_water_volume() {
                still_moving = TESTMOVE_STOPPED;
                if p.can_fly {
                    reach_flags = m.fly_reachable(dest, reach_flags);
                    success = reach_flags;
                // (Z + 50) + 35.0 (double) rounded to single by the FPU.
                } else if p.can_walk && dest[2] < (m.location()[2] + 50.0) + 35.0 {
                    // Walking out of the water.
                    let l = m.location();
                    let mut up = dest[2] - l[2];
                    let min = p.collision_height + 35.0;
                    if up <= min {
                        up = min;
                    }
                    if m.rises(up) {
                        success = m.fly_reachable(dest, reach_flags);
                        reach_flags = 1;
                    }
                }
            } else if m.hurt() {
                success = 0;
                still_moving = TESTMOVE_STOPPED;
            }
        }
        loops -= 1;
        if loops < 0 {
            still_moving = TESTMOVE_STOPPED;
        }
    }
    if success == 0 && m.zone_marked() == Some(true) {
        success = 1;
    }
    m.return_value(start, p.velocity);
    if success != 0 {
        reach_flags
    } else {
        0
    }
}

/// walkReachable in the VM.
struct WalkVm<'a> {
    vm: &'a mut Vm,
    p: ObjId,
    goal: Option<ObjId>,
}

impl WorldWalk for WalkVm<'_> {
    fn location(&self) -> V {
        self.vm.native_prop(self.p, crate::NativeField::Location).as_vector()
    }
    fn reaches(&mut self, dir: V) -> bool {
        self.vm.reached_destination(self.p, dir, self.goal)
    }
    fn walk_move(&mut self, delta: V, hit: &mut CheckResult, threshold: f32) -> i32 {
        self.vm.walk_move(self.p, delta, hit, self.goal, threshold)
    }
    fn fly_move(&mut self, delta: V, threshold: f32) -> i32 {
        self.vm.test_fly_move(self.p, delta, self.goal, threshold)
    }
    fn swim_move(&mut self, delta: V, threshold: f32) -> i32 {
        self.vm.swim_move(self.p, delta, self.goal, threshold)
    }
    fn rises(&mut self, up: f32) -> bool {
        let mut hit = CheckResult::new(1.0);
        self.vm.test_move(self.p, [0.0, 0.0, up], &mut hit);
        hit.time == 1.0
    }
    fn trace_empty(&mut self, end: V, start: V, extent: V) -> bool {
        let mut h = CheckResult::new(1.0);
        self.vm.single_line_check(&mut h, Some(self.p), end, start, 0x286, extent);
        h.time == 1.0
    }
    fn zone_number(&self) -> u8 {
        self.vm.zone_number(self.p) as u8
    }
    fn outside_world(&mut self) {
        self.vm.log.push("Walked out of world!!!".into());
    }
    fn find_best_jump(&mut self, dest: V) -> i32 {
        self.vm.find_best_jump(self.p, dest)
    }
    fn find_jump_up(&mut self, delta: V) -> i32 {
        self.vm.find_jump_up(self.p, delta)
    }
    fn fly_reachable(&mut self, dest: V, reach_flags: i32) -> i32 {
        self.vm.fly_reachable(self.p, dest, reach_flags, self.goal)
    }
    fn swim_reachable(&mut self, dest: V, reach_flags: i32) -> i32 {
        self.vm.swim_reachable(self.p, dest, reach_flags, self.goal)
    }
    fn hurt(&mut self) -> bool {
        self.vm.hurt_by_volume(self.p)
    }
    fn in_water_volume(&self) -> bool {
        self.vm.in_water(self.p)
    }
    fn zone_marked(&self) -> Option<bool> {
        self.goal.filter(|_| self.vm.warp_zone_success(self.p, self.goal)).map(|_| true)
    }
    fn height_but(&self) -> Option<f32> {
        self.goal.map(|g| self.vm.native_prop(g, crate::NativeField::CollisionHeight).as_float())
    }
    fn return_value(&mut self, start: V, velocity: V) {
        self.vm.far_move_actor(self.p, start, true, true, false);
        self.vm.set_v(self.p, "Velocity", velocity);
    }
}

/// What walkMove asks of the world: the pawn's Location, the gravity of its PhysicsVolume, test
/// MoveActor (pawn rotation, bTest, bIgnorePawns) and FarMoveActor to return.
pub(crate) trait WorldStep {
    fn location(&self) -> V;
    fn gravity_z(&self) -> f32;
    fn move_actor(&mut self, delta: V, hit: &mut CheckResult);
    fn far_move(&mut self, loc: V);
}

/// APawn::walkMove without the world: one test walking step, climbing steps
/// (35 units) and checking the floor (37 units); TESTMOVE_*.
pub(crate) fn step_size(m: &mut impl WorldStep, delta: V, hit: &mut CheckResult, goal: Option<ObjId>, threshold: f32) -> i32 {
    let start = m.location();
    let delta = [delta[0], delta[1], 0.0];
    let grav = if m.gravity_z() > 0.0 { 1.0f32 } else { -1.0 };
    let down = grav * 35.0;
    let up = down * -1.0;
    m.move_actor(delta, hit);
    if goal.is_some() && hit.actor == goal {
        return TESTMOVE_HIT_GOAL;
    }
    if hit.time < 1.0 {
        let step_loc = m.location();
        let t = 1.0 - hit.time;
        m.move_actor([-0.0, -0.0, up], hit);
        m.move_actor([delta[0] * t, delta[1] * t, t * 0.0], hit);
        if goal.is_some() && hit.actor == goal {
            return TESTMOVE_HIT_GOAL;
        }
        m.move_actor([0.0, 0.0, down], hit);
        if hit.time < 1.0 && hit.normal[2] < 0.7 {
            m.far_move(step_loc);
            return TESTMOVE_STOPPED;
        }
    }
    let before_drop = m.location();
    m.move_actor([0.0, 0.0, grav * 37.0], hit);
    if hit.time == 1.0 || hit.normal[2] < 0.7 {
        m.far_move(before_drop);
        return TESTMOVE_FELL;
    }
    if goal.is_some() && hit.actor == goal {
        return TESTMOVE_HIT_GOAL;
    }
    let l = m.location();
    // (dZ² + dY²) + dX².
    let (dx, dy, dz) = (l[0] - start[0], l[1] - start[1], l[2] - start[2]);
    if (dz * dz + dy * dy) + dx * dx < threshold * threshold {
        return TESTMOVE_STOPPED;
    }
    TESTMOVE_MOVED
}

/// walkMove in the VM.
struct StepVm<'a> {
    vm: &'a mut Vm,
    p: ObjId,
}

impl WorldStep for StepVm<'_> {
    fn location(&self) -> V {
        self.vm.native_prop(self.p, crate::NativeField::Location).as_vector()
    }
    fn gravity_z(&self) -> f32 {
        self.vm.gravity_of(self.p)[2]
    }
    fn move_actor(&mut self, delta: V, hit: &mut CheckResult) {
        self.vm.test_move(self.p, delta, hit)
    }
    fn far_move(&mut self, loc: V) {
        self.vm.far_move_actor(self.p, loc, true, true, false);
    }
}

/// What jumpLanding asks of the world: the pawn's Location and ZoneNumber, its PhysicsVolume (water,
/// FluidFriction, Gravity, ZoneVelocity), test MoveActor and FarMoveActor to return.
pub(crate) trait WorldLanding {
    fn location(&self) -> V;
    fn in_water_volume(&self) -> bool;
    /// FluidFriction of the volume if it is water, otherwise 0.
    fn friction(&self) -> f32;
    fn gravity_value(&self) -> V;
    fn zone_velocity(&self) -> V;
    fn zone_number(&self) -> u8;
    fn move_actor(&mut self, delta: V, hit: &mut CheckResult);
    fn far_move(&mut self, loc: V);
}

/// APawn::jumpLanding without the world: fall simulated in steps of 0.1 s from velocity
/// `velocity`; the pawn stays at the landing point with `stay` (otherwise it comes back). The update
/// of an AScout's MaxLandingVelocity (path builder) is not reimplemented.
pub(crate) fn landing(m: &mut impl WorldLanding, velocity: V, stay: bool, max_fall_speed: f32) -> i32 {
    let start = m.location();
    let mut landed = false;
    let mut ticks = 0;
    let tick = 0.1f32;
    let mut v = velocity;
    loop {
        if landed {
            let cur = m.location();
            if !stay {
                m.far_move(start);
            }
            return if cur == start { TESTMOVE_STOPPED } else { TESTMOVE_MOVED };
        }
        let friction = m.friction();
        let gravity = m.gravity_value();
        let zone = m.zone_velocity();
        let k = 1.0 - friction * tick;
        let (vy, vx) = (v[1] * k, v[0] * k);
        v[2] = v[2] * k + tick * gravity[2];
        v[1] = vy + tick * gravity[1];
        v[0] = vx + tick * gravity[0];
        let delta = [(v[0] + zone[0]) * tick, (v[1] + zone[1]) * tick, (v[2] + zone[2]) * tick];
        let mut hit = CheckResult::new(1.0);
        m.move_actor(delta, &mut hit);
        if m.in_water_volume() {
            landed = true;
        } else if max_fall_speed * -1.0 <= v[2] {
            if hit.time < 1.0 {
                if hit.normal[2] >= 0.7 {
                    landed = true;
                } else {
                    let old_normal = hit.normal;
                    let n = hit.normal;
                    // Machine code order: (NY·dY + NX·dX) + dZ·NZ.
                    let d = (n[1] * delta[1] + n[0] * delta[0]) + delta[2] * n[2];
                    let t = 1.0 - hit.time;
                    let mut slide = [(delta[0] - d * n[0]) * t, (delta[1] - n[1] * d) * t, (delta[2] - n[2] * d) * t];
                    // (dZ·sZ + dY·sY) + sX·dX.
                    if (delta[2] * slide[2] + delta[1] * slide[1]) + slide[0] * delta[0] >= 0.0 {
                        m.move_actor(slide, &mut hit);
                        if hit.time < 1.0 {
                            if hit.normal[2] >= 0.7 {
                                landed = true;
                            }
                            let desired = safe_normal(delta);
                            Vm::two_wall_adjust(desired, &mut slide, hit.normal, old_normal, hit.time);
                            m.move_actor(slide, &mut hit);
                            if hit.normal[2] >= 0.7 {
                                landed = true;
                            }
                        }
                    }
                }
            }
        } else {
            landed = true;
            m.far_move(start);
        }
        ticks += 1;
        // (VX² + VY²) + VZ².
        if m.zone_number() == 0 || ticks > 35 || (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2] > 2.5e6 {
            m.far_move(start);
            landed = true;
        }
    }
}

/// What FindJumpUp and FindBestJump ask of the world in addition to jumpLanding: walkMove (without
/// a goal, threshold 4.1), HurtByVolume and the pawn's settings.
pub(crate) trait WorldJump: WorldLanding {
    fn walk_move_jump(&mut self, delta: V, hit: &mut CheckResult) -> i32;
    fn hurt_by_volume(&mut self) -> bool;
    fn can_swim_now(&self) -> bool;
    fn ground_speed(&self) -> f32;
    fn jump_z(&self) -> f32;
    fn max_fall_speed(&self) -> f32;
}

/// APawn::SuggestJumpVelocity: jump velocity from `loc` towards `dest` at horizontal
/// speed `xy_speed`, starting at least at `base_z` upwards. `gz` is the Z gravity of the
/// PhysicsVolume, `ground_speed` the pawn's GroundSpeed.
pub(crate) fn speed_jump(loc: V, dest: V, xy_speed: f32, base_z: f32, jump_z: f32, gz: f32, ground_speed: f32) -> V {
    let dz = dest[2] - loc[2];
    let dy = dest[1] - loc[1];
    let dx = dest[0] - loc[0];
    let xy = ((dx * dx + dy * dy) as f64).sqrt() as f32;
    if xy == 0.0 {
        return [0.0, 0.0, if jump_z <= base_z { base_z } else { jump_z }];
    }
    let inv = 1.0 / xy;
    let zero = inv * 0.0;
    if gz < 0.0 {
        let mut xy_speed = xy_speed;
        let time = xy / xy_speed;
        let mut z = dz / time - time * gz * 0.5;
        if z < base_z {
            let t = dz * gz;
            let root = ((base_z * base_z + t + t) as f64).sqrt() as f32;
            let mut time = (root - base_z) / gz;
            if time < 0.05 {
                time = 0.05;
            }
            xy_speed = xy / time;
            if ground_speed <= xy_speed {
                xy_speed = ground_speed;
            }
            z = base_z;
        }
        let max_z = base_z + jump_z;
        if z > max_z {
            z = max_z;
        }
        return [inv * dx * xy_speed, dy * inv * xy_speed, z];
    }
    [inv * dx * xy_speed, dy * inv * xy_speed, zero * xy_speed]
}

/// APawn::FindJumpUp: jump of 45 units then a step forward.
pub(crate) fn jump_high(m: &mut impl WorldJump, delta: V) -> i32 {
    let start = m.location();
    let mut hit = CheckResult::new(1.0);
    m.move_actor([0.0, 0.0, 45.0], &mut hit);
    let result = m.walk_move_jump(delta, &mut hit);
    let z = m.location()[2];
    if result == TESTMOVE_STOPPED {
        m.far_move([start[0], start[1], z]);
        return TESTMOVE_STOPPED;
    }
    m.move_actor([0.0, 0.0, -80.0], &mut hit);
    let l = m.location();
    let (dx, dy) = (start[0] - l[0], start[1] - l[1]);
    // The machine code subtracts the current height from itself: only the plane counts.
    let dz = 0.0f32;
    if (dz * dz + dy * dy) + dx * dx < 16.81 {
        return TESTMOVE_STOPPED;
    }
    result
}

/// APawn::FindBestJump: jump towards `dest` at GroundSpeed; successful if it gets closer.
pub(crate) fn best_jump(m: &mut impl WorldJump, dest: V) -> i32 {
    let start = m.location();
    let ground = m.ground_speed();
    let v = speed_jump(start, dest, ground, 0.0, m.jump_z(), m.gravity_value()[2], ground);
    let max_fall = m.max_fall_speed();
    if landing(m, v, true, max_fall) == TESTMOVE_STOPPED {
        return TESTMOVE_STOPPED;
    }
    if !m.hurt_by_volume() && (m.can_swim_now() || !m.in_water_volume()) {
        let l = m.location();
        let (ay, ax) = (dest[1] - start[1], dest[0] - start[0]);
        let (by, bx) = (dest[1] - l[1], dest[0] - l[0]);
        let before = ((ay * ay + ax * ax) as f64).sqrt() as f32;
        let after = ((by * by + bx * bx) as f64).sqrt() as f32;
        if before - after > 0.0 {
            return TESTMOVE_MOVED;
        }
    }
    TESTMOVE_STOPPED
}

/// jumpLanding in the VM.
struct LandingVm<'a> {
    vm: &'a mut Vm,
    p: ObjId,
}

impl LandingVm<'_> {
    fn volume(&self) -> Option<ObjId> {
        self.vm.native_object(self.p, crate::NativeField::PhysicsVolume).or(self.vm.default_volume)
    }
}

impl WorldLanding for LandingVm<'_> {
    fn location(&self) -> V {
        self.vm.native_prop(self.p, crate::NativeField::Location).as_vector()
    }
    fn in_water_volume(&self) -> bool {
        self.vm.in_water(self.p)
    }
    fn friction(&self) -> f32 {
        let volume_obj = self.volume();
        if volume_obj.is_some_and(|v| self.vm.pb(v, "bWaterVolume")) {
            volume_obj.map_or(0.0, |v| self.vm.pf(v, "FluidFriction"))
        } else {
            0.0
        }
    }
    fn gravity_value(&self) -> V {
        self.vm.gravity_of(self.p)
    }
    fn zone_velocity(&self) -> V {
        self.volume().map_or([0.0; 3], |volume_obj| self.vm.pv(volume_obj, "ZoneVelocity"))
    }
    fn zone_number(&self) -> u8 {
        self.vm.zone_number(self.p) as u8
    }
    fn move_actor(&mut self, delta: V, hit: &mut CheckResult) {
        self.vm.test_move(self.p, delta, hit)
    }
    fn far_move(&mut self, loc: V) {
        self.vm.far_move_actor(self.p, loc, true, true, false);
    }
}

impl WorldJump for LandingVm<'_> {
    fn walk_move_jump(&mut self, delta: V, hit: &mut CheckResult) -> i32 {
        self.vm.walk_move(self.p, delta, hit, None, 4.1)
    }
    fn hurt_by_volume(&mut self) -> bool {
        self.vm.hurt_by_volume(self.p)
    }
    fn can_swim_now(&self) -> bool {
        self.vm.pb(self.p, "bCanSwim")
    }
    fn ground_speed(&self) -> f32 {
        self.vm.pf(self.p, "GroundSpeed")
    }
    fn jump_z(&self) -> f32 {
        self.vm.pf(self.p, "JumpZ")
    }
    fn max_fall_speed(&self) -> f32 {
        self.vm.pf(self.p, "MaxFallSpeed")
    }
}

/// newPathList of a navigation point (ANavigationPoint::Serialize): compact n
/// × [Start, End (raw compact indices), then five shorts (CollisionRadius, CollisionHeight,
/// reachFlags, MaxLandingVelocity, Distance)]; stops at the first impossible read.
pub(crate) fn read_reach_specs(r: &mut openxiii_package::Reader) -> Vec<(i32, i32, [i16; 5])> {
    let mut specs = Vec::new();
    let count = r.compact().unwrap_or(0);
    for _ in 0..count.max(0) {
        let Some(spec) = read_reach_spec(r) else { break };
        specs.push(spec);
    }
    specs
}

/// operator<< of FReachSpec (licensee >= 0x30: all the game's packages are at 58): Start and End
/// (raw compact indices), then CollisionRadius, CollisionHeight, reachFlags, MaxLandingVelocity, Distance as
/// shorts, in that order (+0xA, +0xC, +8, +0xE, +0x10 in memory); the array `v` follows the file's order.
pub(crate) fn read_reach_spec(r: &mut openxiii_package::Reader) -> Option<(i32, i32, [i16; 5])> {
    let (Ok(start), Ok(target)) = (r.compact(), r.compact()) else { return None };
    let mut v = [0i16; 5];
    let mut ok = true;
    for x in &mut v {
        match r.u16() {
            Ok(w) => *x = w as i16,
            Err(_) => ok = false,
        }
    }
    ok.then_some((start, target, v))
}

impl Vm {
    // ---- Navigation network ----

    /// ANavigationPoint::Serialize: after the properties, newPathList (TArray of
    /// FReachSpec, LicenseeVer > 46): compact n × [Start, End (objects), then (operator<< of the
    /// FReachSpec) as 16 bits CollisionRadius, CollisionHeight, reachFlags,
    /// MaxLandingVelocity, Distance] (LicenseeVer ≥ 48).
    pub(crate) fn load_reach_specs(&mut self, pi: usize, map_objects: &HashMap<usize, ObjId>) {
        let Some(nav_class) = self.class("NavigationPoint") else { return };
        let mut navs: Vec<(usize, ObjId)> = map_objects.iter().map(|(&e, &id)| (e, id)).collect();
        navs.sort();
        for (export, id) in navs {
            let is_nav = self.objects[id].as_ref().is_some_and(|o| self.is_child_of(o.class, nav_class));
            if !is_nav {
                continue;
            }
            let lp = &self.packages[pi];
            let Ok(o) = lp.package.object_data(&lp.data, export) else { continue };
            let e = &lp.package.exports[export];
            let end = e.serial_offset as usize + e.serial_size as usize;
            let mut r = openxiii_package::Reader::new(&lp.data[..end], o.end);
            let resolve = |x: i32| match ObjectRef::from(x) {
                ObjectRef::Export(t) => map_objects.get(&t).copied(),
                _ => None,
            };
            let mut specs = Vec::new();
            for (start, target, v) in read_reach_specs(&mut r) {
                if let Some(end) = resolve(target) {
                    specs.push(ReachSpec {
                        start: resolve(start).unwrap_or(id),
                        end,
                        radius: v[0],
                        height: v[1],
                        reach_flags: v[2],
                        max_landing: v[3],
                        distance: v[4],
                    });
                }
            }
            self.nav.insert(id, specs);
        }
    }

    /// The navigation points in Level.NavigationPointList order.
    pub(crate) fn navigation_points(&self) -> Vec<ObjId> {
        let mut out = Vec::new();
        let mut n = self.level.and_then(|l| self.po(l, "NavigationPointList"));
        while let Some(p) = n {
            if out.len() > 100_000 {
                break;
            }
            out.push(p);
            n = self.po(p, "nextNavigationPoint");
        }
        out
    }

    fn nav_i(&self, n: ObjId, name: &str) -> i32 {
        self.propr(n, name).as_int()
    }
    pub(crate) fn set_nav_i(&mut self, n: ObjId, name: &str, v: i32) {
        self.set_prop(n, name, Value::Int(v));
    }
    fn set_nav_o(&mut self, n: ObjId, name: &str, v: Option<ObjId>) {
        self.set_prop(n, name, Value::Object(v));
    }

    /// CurrentPath of the controller (the engine's FReachSpec*: here the point and the link index).
    pub(crate) fn set_current_path(&mut self, c: ObjId, nav: ObjId, index: usize) {
        self.current_paths.insert(c, (nav, index));
        self.set_prop(c, "CurrentPath", Value::Int(1));
    }
    pub(crate) fn clear_current_path(&mut self, c: ObjId) {
        self.current_paths.remove(&c);
        self.set_prop(c, "CurrentPath", Value::Int(0));
    }
    pub(crate) fn current_path_spec(&self, c: ObjId) -> Option<ReachSpec> {
        if self.propr(c, "CurrentPath").as_int() == 0 {
            return None;
        }
        let &(nav, i) = self.current_paths.get(&c)?;
        self.nav.get(&nav)?.get(i).copied()
    }

    // ---- Path search ----

    /// APawn::clearPath: point reset to the initial search state; its cost
    /// comes from SpecialCost (bSpecialCost) or ExtraCost.
    pub(crate) fn clear_path(&mut self, p: ObjId, n: ObjId) {
        self.set_b(n, "bEndPoint", false);
        self.set_nav_i(n, "visitedWeight", 10_000_000);
        self.set_nav_o(n, "nextOrdered", None);
        self.set_nav_o(n, "prevOrdered", None);
        self.set_nav_o(n, "previousPath", None);
        let cost = if self.pb(n, "bSpecialCost") { self.special_cost(n, p) } else { self.nav_i(n, "ExtraCost") };
        self.set_nav_i(n, "cost", cost);
    }

    /// ANavigationPoint::eventSpecialCost.
    fn special_cost(&mut self, n: ObjId, p: ObjId) -> i32 {
        if !self.is_probing(n, "SpecialCost") {
            return 0;
        }
        match self.find_function(n, "SpecialCost", false) {
            Some(f) => self.call(f, n, vec![Some(Value::Object(Some(p)))]).map(|(v, _)| v.as_int()).unwrap_or(0),
            None => 0,
        }
    }

    /// APawn::clearPaths.
    pub(crate) fn clear_paths(&mut self, p: ObjId) {
        let mut next = self.native_object(p, crate::NativeField::Level).and_then(|level| self.po(level, "NavigationPointList"));
        while let Some(n) = next {
            self.clear_path(p, n);
            // SpecialCost can change the chaining: the original reads it again after the call.
            next = self.po(n, "nextNavigationPoint");
        }
    }

    /// Evaluation of a point by breadthPathTo.
    pub(crate) fn node_eval(&mut self, eval: NodeEval, n: ObjId, p: ObjId, best: f32) -> f32 {
        match eval {
            NodeEval::EndPoint => {
                if self.pb(n, "bEndPoint") {
                    1.0
                } else {
                    0.0
                }
            }
            NodeEval::RandomPath => {
                if self.pb(n, "bEndPoint") {
                    self.rand() + 1000.0
                } else {
                    self.rand()
                }
            }
            NodeEval::BestInventory => {
                let Some(c) = self.native_object(p, crate::NativeField::Controller) else { return 0.0 };
                let Some(item) = self.po(n, "markedItem").filter(|_| self.is_a(n, "InventorySpot")) else { return 0.0 };
                let weight = self.nav_i(n, "visitedWeight") as f32;
                if (self.is_probing(item, "Touch") || self.pf(item, "LatentFloat") < 5.0) && best < self.pf(item, "MaxDesireability") / weight {
                    // AController::eventDesireability.
                    let d = match self.find_function(c, "Desireability", false) {
                        Some(f) if self.is_probing(c, "Desireability") => {
                            self.call(f, c, vec![Some(Value::Object(Some(item)))]).map(|(v, _)| v.as_float()).unwrap_or(0.0)
                        }
                        _ => 0.0,
                    };
                    return d / weight;
                }
                0.0
            }
        }
    }

    /// APawn::HurtByVolume: does one of the volumes touched by the actor hurt?
    pub(crate) fn hurt_by_volume(&self, a: ObjId) -> bool {
        self.touching_of(a).into_iter().any(|t| self.is_a(t, "PhysicsVolume") && self.pb(t, "bPainCausing") && self.pf(t, "DamagePerSec") > 0.0)
    }

    /// APawn::breadthPathTo: see `path_large`.
    pub(crate) fn breadth_path_to(&mut self, p: ObjId, eval: NodeEval, start: ObjId, move_flags: i32, weight: &mut f32) -> Option<ObjId> {
        let pawn_value = PawnPath {
            collision_radius: self.native_prop(p, crate::NativeField::CollisionRadius).as_float(),
            collision_height: self.native_prop(p, crate::NativeField::CollisionHeight).as_float(),
            max_fall_speed: self.pf(p, "MaxFallSpeed"),
            walking_pct: self.pf(p, "WalkingPct"),
            can_crouch: self.pb(p, "bCanCrouch"),
            crouch_height: self.pf(p, "CrouchHeight"),
            crouch_radius: self.pf(p, "CrouchRadius"),
            height_default: self.default_float(p, "CollisionHeight"),
        };
        path_large(&pawn_value, &mut GraphVm { vm: self, p, eval }, start, move_flags, weight)
    }

    /// FSortedPathList::findStartAnchor: first visible point (from the feet,
    /// otherwise from the top of the pawn to the top of the point) that the pawn can reach.
    pub(crate) fn find_start_anchor(&mut self, list: &SortedPathList, p: ObjId) -> Option<ObjId> {
        for n in list.paths() {
            let loc = self.native_prop(p, crate::NativeField::Location).as_vector();
            let nl = self.native_prop(n, crate::NativeField::Location).as_vector();
            let mut hit = CheckResult::new(1.0);
            self.single_line_check(&mut hit, Some(p), nl, loc, 0x286, [0.0; 3]);
            if hit.actor.is_some() {
                let start = [loc[0], loc[1], self.native_prop(p, crate::NativeField::CollisionHeight).as_float() + loc[2]];
                let end = [nl[0], nl[1], self.native_prop(n, crate::NativeField::CollisionHeight).as_float() + nl[2]];
                hit = CheckResult::new(1.0);
                self.single_line_check(&mut hit, Some(p), end, start, 0x286, [0.0; 3]);
            }
            if hit.actor.is_none() && self.actor_reachable(p, Some(n), true, true) {
                return Some(n);
            }
        }
        None
    }

    /// FSortedPathList::findEndAnchor: first point from which the goal is visible and
    /// reachable (pawn moved there for the test); failing that, with `any_visible`, the first
    /// point from which it is visible.
    pub(crate) fn find_end_anchor(&mut self, list: &SortedPathList, p: ObjId, goal: Option<ObjId>, end_location: V, any_visible: bool) -> Option<ObjId> {
        let start = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut nearest_visible = None;
        for n in list.paths() {
            let nl = self.native_prop(n, crate::NativeField::Location).as_vector();
            let nh = self.native_prop(n, crate::NativeField::CollisionHeight).as_float();
            let mut hit = CheckResult::new(1.0);
            self.single_line_check(&mut hit, Some(p), end_location, nl, 0x286, [0.0; 3]);
            if hit.actor.is_some() {
                let top = [nl[0], nl[1], nh + nl[2]];
                let target = match goal {
                    None => end_location,
                    Some(g) => [end_location[0], end_location[1], self.native_prop(g, crate::NativeField::CollisionHeight).as_float() + end_location[2]],
                };
                hit = CheckResult::new(1.0);
                self.single_line_check(&mut hit, Some(p), target, top, 0x286, [0.0; 3]);
            }
            if hit.actor.is_some() {
                continue;
            }
            let dest = [nl[0], nl[1], (nl[2] + self.native_prop(p, crate::NativeField::CollisionHeight).as_float()) - nh];
            if self.far_move_actor(p, dest, true, true, false) {
                let ok = match goal {
                    Some(g) => self.actor_reachable(p, Some(g), true, true),
                    None => self.point_reachable(p, end_location, true),
                };
                if ok {
                    self.far_move_actor(p, start, true, true, false);
                    return Some(n);
                }
                if any_visible && nearest_visible.is_none() {
                    nearest_visible = Some(n);
                }
            }
        }
        if self.native_prop(p, crate::NativeField::Location).as_vector() != start {
            self.far_move_actor(p, start, true, true, false);
        }
        if let Some(n) = nearest_visible {
            let line = format!("Use nearestvisible {}", self.obj_label(n));
            self.log.push(line);
        }
        nearest_visible
    }

    /// AController::SetRouteCache: see `route`.
    pub(crate) fn set_route_cache(&mut self, c: ObjId, end_path: ObjId, start_dist: f32, end_dist: f32) {
        let p = self.native_object(c, crate::NativeField::Pawn);
        let pawn_value = p.map(|p| self.native_prop(p, crate::NativeField::Location).as_vector());
        let s = route(&mut GraphRouteVm { vm: self, c, p: p.unwrap_or(c) }, end_path, pawn_value, start_dist, end_dist);
        self.set_prop(c, "RouteGoal", Value::Object(Some(s.route_goal)));
        self.set_f(c, "RouteDist", s.route_dist);
        self.set_prop(c, "RouteCache", Value::Array(s.route_cache.iter().map(|&a| Value::Object(a)).collect()));
    }

    /// APawn::findPathToward: path towards `goal` (or towards `dest`, or according to `eval`)
    /// on the navigation network; the pawn's Anchor and the end point chosen among the nearby
    /// points if needed. Returns the weight found (0 without a path) and fills RouteCache.
    pub(crate) fn find_path_toward(&mut self, p: ObjId, goal: Option<ObjId>, dest: V, eval: Option<NodeEval>, clear_paths: bool, max_weight: f32) -> f32 {
        crate::path_toward::find_path_toward(&mut crate::path_toward::PathVm(self), p, goal, dest, eval, clear_paths, max_weight)
    }

    /// AController::AcceptNearbyPath (vt 0x144): AAIController for a pawn, nothing
    /// for the other controllers (AController::AcceptNearbyPath: returns 0).
    pub(crate) fn accept_nearby_path(&self, c: ObjId, goal: ObjId) -> bool {
        self.is_a(c, "AIController") && self.is_a(goal, "Pawn")
    }

    /// AController::SetPath: first point of RouteCache, with the GoalList stack of
    /// intermediate goals and the static list of points already returned (against loops);
    /// SpecialHandling for a point that asks for it.
    pub(crate) fn set_path(&mut self, c: ObjId, initial: bool) -> Option<ObjId> {
        crate::path_toward::set_path(&mut crate::path_toward::PathVm(self), c, initial)
    }

    /// AController::HandleSpecial: the point's SpecialHandling (door, lift…)
    /// can designate another goal, reached directly or through a new path.
    pub(crate) fn handle_special(&mut self, c: ObjId, best: ObjId) -> Option<ObjId> {
        if !self.pb(c, "bCanDoSpecial") {
            return Some(best);
        }
        let goals = match self.prop(c, "GoalList") {
            Value::Array(v) => v.get(3).and_then(|x| x.as_object()),
            _ => None,
        };
        if goals.is_some() {
            return Some(best);
        }
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else { return Some(best) };
        let new_goal = match self.find_function(best, "SpecialHandling", false) {
            Some(f) => self.call(f, best, vec![Some(Value::Object(Some(p)))]).ok().and_then(|(v, _)| v.as_object()),
            None => None,
        };
        let mut best = Some(best);
        if let Some(g) = new_goal.filter(|&g| Some(g) != best) {
            if self.actor_reachable(p, Some(g), false, false) {
                return Some(g);
            }
            let l = self.native_prop(g, crate::NativeField::Location).as_vector();
            if self.find_path_toward(p, Some(g), l, None, true, 0.0) > 0.0 {
                best = self.set_path(c, false);
            }
        }
        best
    }

    /// AController::FindPath.
    pub(crate) fn find_path(&mut self, c: ObjId, point: V, goal: Option<ObjId>, clear_paths: bool) -> Option<ObjId> {
        let Some(p) = self.native_object(c, crate::NativeField::Pawn) else {
            self.dev_path_warning("No pawn for FindPath", c);
            return None;
        };
        if self.find_path_toward(p, goal, point, None, clear_paths, 0.0) > 0.0 {
            return self.set_path(c, true);
        }
        None
    }

    /// AController::execFindPathTowardNearest: path towards the nearest point
    /// of the given class (all its points are end points).
    pub(crate) fn find_path_toward_nearest(&mut self, c: ObjId, class: Option<crate::value::ClassId>) -> Option<ObjId> {
        let (Some(class), Some(p)) = (class, self.native_object(c, crate::NativeField::Pawn)) else {
            self.dev_path_warning("No goal for FindPathTowardNearest", c);
            return None;
        };
        self.clear_paths(p);
        let mut last = None;
        for n in self.navigation_points() {
            if self.objects[n].as_ref().is_some_and(|o| o.class == class) {
                self.set_b(n, "bEndPoint", true);
                last = Some(n);
            }
        }
        let last = last?;
        self.find_path(c, [0.0; 3], Some(last), false)
    }

    /// AController::execFindRandomDest.
    pub(crate) fn find_random_dest(&mut self, c: ObjId, clear_paths: bool) -> Option<ObjId> {
        let p = self.native_object(c, crate::NativeField::Pawn)?;
        if clear_paths {
            self.clear_paths(p);
        }
        if self.find_path_toward(p, None, [0.0; 3], Some(NodeEval::RandomPath), true, 0.0) > 0.0 {
            return self.po(c, "RouteGoal").filter(|&g| self.is_a(g, "NavigationPoint"));
        }
        None
    }

    /// AController::execFindBestInventoryPath: MinWeight becomes the weight found.
    pub(crate) fn find_best_inventory_path(&mut self, c: ObjId, min_weight: &mut f32) -> Option<ObjId> {
        let p = self.native_object(c, crate::NativeField::Pawn)?;
        let w = self.find_path_toward(p, None, [0.0; 3], Some(NodeEval::BestInventory), true, *min_weight);
        *min_weight = w;
        if w > 0.0 {
            return self.set_path(c, true);
        }
        None
    }

    /// AIAController::execFindBestPathToward (XIDPawn.dll): FindPath towards the actor's
    /// position with the margins GReachedDestinationXYMargin / HeightMargin; MoveTarget and
    /// Destination take the point found.
    pub(crate) fn find_best_path_toward(&mut self, c: ObjId, desired: Option<ObjId>, xy_margin: f32, height_margin: f32) -> bool {
        let Some(d) = desired else {
            self.log.push("GROSSSSSSSSSSSSSSSSSS BUGGGGGGGGGGGGGGGGGG ACTOR==NONE !!!!!!!!!!!!!!!!!!!!!!".into());
            return false;
        };
        let l = self.native_prop(d, crate::NativeField::Location).as_vector();
        self.reach_xy_margin = xy_margin;
        self.reach_height_margin = height_margin;
        let found = self.find_path(c, l, None, true);
        self.reach_xy_margin = 0.0;
        self.reach_height_margin = 0.0;
        self.take_best_path(c, found)
    }

    /// AIAController::execFindBestPathTo (XIDPawn.dll).
    pub(crate) fn find_best_path_to(&mut self, c: ObjId, dest: V) -> bool {
        let found = self.find_path(c, dest, None, true);
        self.take_best_path(c, found)
    }

    fn take_best_path(&mut self, c: ObjId, found: Option<ObjId>) -> bool {
        if let Some(n) = found {
            self.set_prop(c, "MoveTarget", Value::Object(Some(n)));
            let l = self.native_prop(n, crate::NativeField::Location).as_vector();
            self.set_v(c, "Destination", l);
        }
        found.is_some()
    }

    // ---- Reachability ----

    /// APawn::actorReachable, see `reachability::actor_reachable`.
    pub(crate) fn actor_reachable(&mut self, p: ObjId, other: Option<ObjId>, know_visible: bool, no_anchor_check: bool) -> bool {
        crate::reachability::actor_reachable(&mut crate::reachability::ReachedVm(self), p, other, know_visible, no_anchor_check)
    }

    /// APawn::pointReachable, see `reachability::point_reachable`.
    pub(crate) fn point_reachable(&mut self, p: ObjId, point: V, know_visible: bool) -> bool {
        crate::reachability::point_reachable(&mut crate::reachability::ReachedVm(self), p, point, know_visible)
    }

    /// APawn::Reachable: test depending on the medium (water, ladder, walking, flying), the pawn
    /// crouched if it can.
    pub(crate) fn reachable(&mut self, p: ObjId, dest: V, goal: Option<ObjId>) -> i32 {
        let crouched = self.pb(p, "bCanCrouch") && !self.pb(p, "bIsCrouched");
        if crouched {
            self.crouch(p, true);
        }
        let volume = self.native_object(p, crate::NativeField::PhysicsVolume);
        let result = if volume.is_some_and(|v| self.pb(v, "bWaterVolume")) {
            self.swim_reachable(p, dest, 0, goal)
        } else if volume.is_some_and(|v| self.is_a(v, "LadderVolume")) {
            self.ladder_reachable(p, dest, 0, goal)
        } else {
            match self.propr(p, "Physics").as_byte() {
                PHYS_WALKING | PHYS_SWIMMING | PHYS_LADDER => self.walk_reachable(p, dest, 0, goal),
                PHYS_FLYING => self.fly_reachable(p, dest, 0, goal),
                _ => 0,
            }
        };
        if crouched {
            self.uncrouch(p, true);
        }
        result
    }

    fn in_water(&self, p: ObjId) -> bool {
        self.native_object(p, crate::NativeField::PhysicsVolume).is_some_and(|v| self.pb(v, "bWaterVolume"))
    }

    /// Test MoveActor of the reachability functions (bTest, bIgnorePawns).
    fn test_move(&mut self, p: ObjId, delta: V, hit: &mut CheckResult) {
        let r = self.propr(p, "Rotation").as_rotator();
        self.move_actor(p, delta, r, hit, true, true, false, false);
    }

    /// A failure towards a WarpZoneMarker counts as success if the pawn is in its zone.
    fn warp_zone_success(&self, p: ObjId, goal: Option<ObjId>) -> bool {
        goal.is_some_and(|g| {
            self.is_a(g, "WarpZoneMarker") && {
                let zone = match self.prop(p, "Region") {
                    Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
                    _ => None,
                };
                zone.is_some() && zone == self.po(g, "markedWarpZone")
            }
        })
    }

    /// Region.ZoneNumber.
    fn zone_number(&self, a: ObjId) -> i32 {
        match self.prop(a, "Region") {
            Value::Struct(_, f) => f.get(2).map_or(0, |z| z.as_int()),
            _ => 0,
        }
    }

    fn pawn_walk(&self, p: ObjId) -> PawnWalk {
        PawnWalk {
            velocity: self.native_prop(p, crate::NativeField::Velocity).as_vector(),
            collision_radius: self.native_prop(p, crate::NativeField::CollisionRadius).as_float(),
            collision_height: self.native_prop(p, crate::NativeField::CollisionHeight).as_float(),
            can_jump: self.pb(p, "bCanJump"),
            can_fly: self.pb(p, "bCanFly"),
            can_swim: self.pb(p, "bCanSwim"),
            can_walk: self.pb(p, "bCanWalk"),
        }
    }

    /// APawn::walkReachable: see `walk`.
    fn walk_reachable(&mut self, p: ObjId, dest: V, reach_flags: i32, goal: Option<ObjId>) -> i32 {
        let pawn_value = self.pawn_walk(p);
        walk(&pawn_value, &mut WalkVm { vm: self, p, goal }, dest, reach_flags)
    }

    /// APawn::flyReachable: see `fly_reachable`.
    fn fly_reachable(&mut self, p: ObjId, dest: V, reach_flags: i32, goal: Option<ObjId>) -> i32 {
        let pawn_value = self.pawn_walk(p);
        flight(&pawn_value, &mut WalkVm { vm: self, p, goal }, dest, reach_flags)
    }

    /// APawn::swimReachable: see `swim`.
    fn swim_reachable(&mut self, p: ObjId, dest: V, reach_flags: i32, goal: Option<ObjId>) -> i32 {
        let pawn_value = self.pawn_walk(p);
        swim(&pawn_value, &mut WalkVm { vm: self, p, goal }, dest, reach_flags)
    }

    /// APawn::ladderReachable: a goal on the same ladder is reached.
    pub(crate) fn ladder_reachable(&mut self, p: ObjId, dest: V, reach_flags: i32, goal: Option<ObjId>) -> i32 {
        if let (Some(on), Some(g)) = (self.po(p, "OnLadder"), goal) {
            let ladder = if self.is_a(g, "Ladder") {
                self.po(g, "MyLadder")
            } else if self.propr(g, "Physics").as_byte() == PHYS_LADDER && self.is_a(g, "Pawn") {
                self.po(g, "OnLadder")
            } else {
                None
            };
            if ladder == Some(on) {
                return 1;
            }
        }
        self.walk_reachable(p, dest, reach_flags, goal)
    }

    /// APawn::jumpReachable.
    #[allow(dead_code)]
    fn jump_reachable(&mut self, p: ObjId, dest: V, reach_flags: i32, goal: Option<ObjId>) -> i32 {
        let start = self.native_prop(p, crate::NativeField::Location).as_vector();
        let v = self.native_prop(p, crate::NativeField::Velocity).as_vector();
        if self.jump_landing(p, v, true) == TESTMOVE_STOPPED {
            return 0;
        }
        let r = self.walk_reachable(p, dest, reach_flags | R_JUMP, goal);
        self.far_move_actor(p, start, true, true, false);
        r
    }

    /// APawn::walkMove: see `step_size`.
    fn walk_move(&mut self, p: ObjId, delta: V, hit: &mut CheckResult, goal: Option<ObjId>, threshold: f32) -> i32 {
        step_size(&mut StepVm { vm: self, p }, delta, hit, goal, threshold)
    }

    /// APawn::flyMove.
    fn test_fly_move(&mut self, p: ObjId, delta: V, goal: Option<ObjId>, threshold: f32) -> i32 {
        let start = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut hit = CheckResult::new(1.0);
        self.test_move(p, delta, &mut hit);
        if goal.is_some() && hit.actor == goal {
            return TESTMOVE_HIT_GOAL;
        }
        if hit.time < 1.0 {
            let t = 1.0 - hit.time;
            let rest = [delta[0] * t, delta[1] * t, delta[2] * t];
            self.test_move(p, [-0.0, -0.0, 35.0], &mut hit);
            self.test_move(p, rest, &mut hit);
            if goal.is_some() && hit.actor == goal {
                return TESTMOVE_HIT_GOAL;
            }
        }
        // (dZ² + dY²) + dX² (flyMove, swimMove).
        let l = self.native_prop(p, crate::NativeField::Location).as_vector();
        let (dx, dy, dz) = (l[0] - start[0], l[1] - start[1], l[2] - start[2]);
        if threshold * threshold <= (dz * dz + dy * dy) + dx * dx {
            TESTMOVE_MOVED
        } else {
            TESTMOVE_STOPPED
        }
    }

    /// APawn::swimMove: out of the water, the pawn is brought back to the surface.
    fn swim_move(&mut self, p: ObjId, delta: V, goal: Option<ObjId>, threshold: f32) -> i32 {
        let start = self.native_prop(p, crate::NativeField::Location).as_vector();
        let mut hit = CheckResult::new(1.0);
        self.test_move(p, delta, &mut hit);
        if goal.is_some() && hit.actor == goal {
            return TESTMOVE_HIT_GOAL;
        }
        if !self.in_water(p) {
            let l = self.native_prop(p, crate::NativeField::Location).as_vector();
            let water = self.find_water_line(p, start, l);
            if water != l {
                self.test_move(p, sub(water, l), &mut hit);
            }
            return TESTMOVE_STOPPED;
        }
        if hit.time < 1.0 {
            let t = 1.0 - hit.time;
            let rest = [delta[0] * t, delta[1] * t, delta[2] * t];
            self.test_move(p, [-0.0, -0.0, 35.0], &mut hit);
            self.test_move(p, rest, &mut hit);
            if goal.is_some() && hit.actor == goal {
                return TESTMOVE_HIT_GOAL;
            }
        }
        // (dZ² + dY²) + dX² (flyMove, swimMove).
        let l = self.native_prop(p, crate::NativeField::Location).as_vector();
        let (dx, dy, dz) = (l[0] - start[0], l[1] - start[1], l[2] - start[2]);
        if threshold * threshold <= (dz * dz + dy * dy) + dx * dx {
            TESTMOVE_MOVED
        } else {
            TESTMOVE_STOPPED
        }
    }

    /// APawn::jumpLanding: see `landing`.
    pub(crate) fn jump_landing(&mut self, p: ObjId, velocity: V, stay: bool) -> i32 {
        let max_fall = self.pf(p, "MaxFallSpeed");
        landing(&mut LandingVm { vm: self, p }, velocity, stay, max_fall)
    }

    /// APawn::FindJumpUp: see `jump_high`.
    fn find_jump_up(&mut self, p: ObjId, delta: V) -> i32 {
        jump_high(&mut LandingVm { vm: self, p }, delta)
    }

    /// APawn::FindBestJump: see `best_jump`.
    fn find_best_jump(&mut self, p: ObjId, dest: V) -> i32 {
        best_jump(&mut LandingVm { vm: self, p }, dest)
    }

    /// Vector / length of the end point, used by the NodeEvaluator.
    #[allow(dead_code)]
    fn nav_div(a: V, f: f32) -> V {
        div(a, f)
    }
}

