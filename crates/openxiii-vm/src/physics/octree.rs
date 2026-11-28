//! The engine's collision hash, FCollisionOctree (UnOctree.cpp of Engine.dll): in XIII
//! it is a tree with four children on X and Y (Z is never split), root centred at (0, 0, 0)
//! with half-side HALF_WORLD_MAX (262 144). GNewCollisionHash creates it outside
//! the editor (FCollisionOctree, root node); FCollisionHash is only used
//! by the editor. The whole Vm is rebuilt on every map: ~FCollisionOctree and
//! FOctreeNode::RemoveAllActors have no equivalent.
//! - AddActor: collision box of the primitive (GetCollisionBoundingBox) grown
//!   by 4.2; outside the world, nothing. Before BeginPlay (bBegunPlay zero), MultiNodeFilter
//!   stores the actor in every leaf it touches; afterwards
//!   SingleNodeFilter stores it in the smallest node that contains it.
//! - StoreActor: a node without children that already has 3 actors and a half-side > 200
//!   splits (before BeginPlay only), unless the actor's box covers it. As in the
//!   machine code, the split only removes the node from the added actor's list, and refilters
//!   all the actors with that actor's mode.
//! - RemoveActor, MoveActor (SingleNodeFilterForMove).
//! - ActorRadiusCheck: depth-first traversal (actors of the node, then
//!   children touched by the sphere's box in the order 3, 2, 1, 0), an actor once per
//!   query; each result is put at the head of the list, which therefore comes out reversed.
//! - ActorLineCheck: without extent, parametric traversal of the nodes along the
//!   ray (ActorZeroExtentLineCheck); with extent, nodes whose grown box
//!   crosses the path (ActorNonZeroExtentLineCheck). An actor is only tried if the
//!   box of its FOctreeInfo crosses the path (LineCheckWithBox).
//! - ActorPointCheck and ActorEncroachmentCheck: depth-first traversal of the nodes that the box touches.

use crate::NativeField as Field;
use crate::physics::{CheckResult, V, TRACE_SINGLE_RESULT, TRACE_STOP_AT_FIRST_HIT};
use crate::value::{ObjId, Value};
use crate::Vm;
use std::cell::{Cell, RefCell};

const HALF_WORLD_MAX: f32 = 262144.0;
/// Mark of an actor returned without an overlap test (`octree_encroachment_holes`).
pub(crate) const HOLE: ObjId = 1 << (usize::BITS - 1);
/// C_MaxOctreeInfo: size of the global pool of FOctreeInfo, emptied by the
/// constructor and the destructor of FCollisionOctree.
pub(crate) const C_MAX_OCTREE_INFO: usize = 4000;

/// Node: actors stored here and index of the four (consecutive) children in `nodes`.
#[derive(Debug, Clone, Default)]
struct Node {
    actors: Vec<ObjId>,
    children: Option<usize>,
}

/// FOctreeInfo of an actor (CollisionInfo): nodes where it is stored, box, storage mode.
#[derive(Debug, Clone, Default)]
struct Info {
    nodes: Vec<usize>,
    lo: V,
    hi: V,
    /// + 0x28 and + 0x34: centre and half-sizes of the box (FBox::GetCenterAndExtents).
    c: V,
    e: V,
    /// Bit 0 of + 0x44: stored by SingleNodeFilter.
    single: bool,
    /// + 0x40: last query that saw it.
    tag: Cell<u32>,
}

/// Plane of a node: centre (X, Y, Z) and half-side.
#[derive(Debug, Clone, Copy)]
struct Plane {
    c: V,
    w: f32,
}

const ROOT: Plane = Plane { c: [0.0; 3], w: HALF_WORLD_MAX };

#[derive(Debug, Clone)]
pub struct Octree {
    nodes: Vec<Node>,
    infos: crate::types::HashMap<ObjId, Info>,
    /// + 8: query counter (0x1FFFFFFF at the start).
    tag: Cell<u32>,
    /// + 0x5C: flags of the last ActorLineCheck. ActorPointCheck stores its own at
    /// + 0x60 but FOctreeNode::ActorPointCheck reads + 0x5C (ShouldTrace, TRACE_StopAtFirstHit).
    trace_flags: Cell<u32>,
    /// + 0x64: actor of the last ActorLineCheck (SourceActor) or ActorEncroachmentCheck,
    /// which FOctreeNode::ActorOverlapCheck still skips at the next query.
    last_actor: Cell<Option<ObjId>>,
    /// Actors whose cylinder is hit by the last ActorLineCheck with TRACE_RequestBones (0x10000),
    /// in traversal order: USkeletalMesh::LineCheck tests their bones, hence GetFrame.
    bone_candidates: RefCell<Vec<ObjId>>,
    /// Changes at each storage or removal of an actor (`nodes` lists of the FOctreeInfos).
    version: u64,
    /// Sorted list of `octree_actors` and the version that produced it (refresh_colliders every frame).
    members: RefCell<(u64, Vec<ObjId>)>,
    /// refresh_colliders: generation of the Props of each actor (by number) at its last
    /// sm_shape_update by this pass (u64::MAX: none). A reused number has another generation.
    people: RefCell<Vec<u64>>,
}

impl Default for Octree {
    fn default() -> Self {
        Octree {
            nodes: vec![Node::default()],
            infos: Default::default(),
            tag: Cell::new(0x1fff_ffff),
            trace_flags: Cell::new(0),
            last_actor: Cell::new(None),
            bone_candidates: RefCell::new(Vec::new()),
            version: 1,
            members: RefCell::new((0, Vec::new())),
            people: RefCell::new(Vec::new()),
        }
    }
}

impl Octree {
    /// New query (+ 8 incremented).
    fn next_tag(&self) -> u32 {
        let t = self.tag.get().wrapping_add(1);
        self.tag.set(t);
        t
    }

    /// True the first time query `tag` sees the actor (and marks it).
    fn first_visit(&self, a: ObjId, tag: u32) -> Option<&Info> {
        let info = self.infos.get(&a)?;
        if info.tag.get() == tag {
            return None;
        }
        info.tag.set(tag);
        Some(info)
    }
}

/// FBox::GetCenterAndExtents: Extent = (Max - Min) * 0.5, Center = Extent + Min.
fn center_extent(lo: V, hi: V) -> (V, V) {
    let e = [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5];
    ([e[0] + lo[0], e[1] + lo[1], e[2] + lo[2]], e)
}

/// Two boxes touch (bounds included).
fn boxes_touch(alo: V, ahi: V, blo: V, bhi: V) -> bool {
    (0..3).all(|k| alo[k] <= bhi[k] && blo[k] <= ahi[k])
}

/// LineCheckWithBox: the segment Start + t Dir (t in [0, 1]) crosses the box
/// (centre, half-sizes); OneOverDir = 1 / Dir.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn line_box(c: V, r: V, start: V, dir: V, inv: V) -> bool {
    let (mut tnear, mut tfar) = (0.0f32, 1.0f32);
    for k in 0..3 {
        let l = start[k] - c[k];
        if dir[k] == 0.0 {
            if !(l.abs() <= r[k]) {
                return false;
            }
            continue;
        }
        let a = l * inv[k];
        let b = inv[k].abs() * r[k];
        let tf = -a - b;
        let tb = b - a;
        if tf > tnear {
            tnear = tf;
        }
        if tb < tfar {
            tfar = tb;
        }
        if tfar < tnear {
            return false;
        }
    }
    true
}

/// Line query (FCollisionOctree + 0x14 … + 0xA0).
pub(crate) struct LineQuery {
    pub end: V,
    pub start: V,
    pub extent: V,
    /// + 0x38 and + 0x44: End - Start and its inverse.
    pub dir: V,
    pub inv: V,
    pub flags: u32,
    pub source: Option<ObjId>,
    /// Origin of the ray (+ 0x90), flipped on the axes where it goes down; X (2) and Y (1) axes
    /// without movement (+ 0x9C); X (2) and Y (1) axes flipped (+ 0xA0).
    origin: V,
    zero_dir: usize,
    mirror: usize,
    /// + 0x6C: box of the path grown by the extent.
    lo: V,
    hi: V,
}

/// Results of a query: in the order they are found (the engine's list is
/// reversed); `best`, the closest (+ 0x8C, traces without extent).
struct LineState {
    tag: u32,
    found: Vec<CheckResult>,
    best: Option<usize>,
}

/// Plane of child `i`: X ± w/2 (bit 1), Y ± w/2 (bit 0), same Z.
fn child_plane(p: &Plane, i: usize) -> Plane {
    let h = p.w * 0.5;
    let sx = ((i & 2) as i32 - 1) as f32;
    let sy = ((i & 1) as i32 * 2 - 1) as f32;
    Plane { c: [sx * h + p.c[0], sy * h + p.c[1], p.c[2]], w: h }
}

/// The box covers the whole node in X and Y (x87 comparisons, NaN included).
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn encloses(lo: V, hi: V, p: &Plane) -> bool {
    !(p.c[0] - p.w < lo[0]) && !(p.c[0] + p.w > hi[0]) && !(p.c[1] - p.w < lo[1]) && !(p.w + p.c[1] > hi[1])
}

/// Children touched by the box, in machine-code order.
fn touched_children(lo: V, hi: V, p: &Plane) -> impl Iterator<Item = usize> {
    // Same order as the previous list, without allocation at each visited node.
    let mut out = [0usize; 4];
    let mut n = 0;
    let mut push = |c: usize| {
        out[n] = c;
        n += 1;
    };
    if hi[0] > p.c[0] {
        if hi[1] > p.c[1] {
            push(3);
        }
        if lo[1] < p.c[1] {
            push(2);
        }
    }
    if lo[0] < p.c[0] {
        if hi[1] > p.c[1] {
            push(1);
        }
        if lo[1] < p.c[1] {
            push(0);
        }
    }
    out.into_iter().take(n)
}

/// Child that contains the whole box, or None if it straddles a middle plane.
fn containing_child(lo: V, hi: V, p: &Plane) -> Option<usize> {
    let x = if lo[0] > p.c[0] {
        2
    } else if hi[0] > p.c[0] {
        return None;
    } else {
        0
    };
    if lo[1] > p.c[1] {
        Some(x | 1)
    } else if hi[1] > p.c[1] {
        None
    } else {
        Some(x)
    }
}

impl Vm {
    /// LevelInfo.bBegunPlay (bit 1 of + 0x380), read by AddActor and StoreActor.
    fn octree_begun_play(&self) -> bool {
        self.level.is_some_and(|l| self.lp(l, Field::BegunPlay, "bBegunPlay").as_bool())
    }

    /// GetPrimitive()->GetCollisionBoundingBox(Actor): brush (UModel, actor's frame),
    /// StaticMesh without bUseCylinderCollision (BoundingBox transformed by LocalToWorld,
    /// UStaticMesh), otherwise the cylinder (UPrimitive): Location ±
    /// (CollisionRadius + 1, CollisionRadius + 1, CollisionHeight + 1). The primitive is that
    /// of GetPrimitive (`prim_kind`): a Mover takes its StaticMesh (AActor::GetPrimitive).
    pub(crate) fn octree_collision_box(&mut self, a: ObjId) -> (V, V) {
        // GetPrimitive without copying the path: StaticMesh, otherwise brush (Model).
        let bbox = match self.prim_static_mesh(a) {
            Some(_) if self.native_prop(a, Field::UseCylinderCollision).as_bool() => None,
            Some(sm) => Some(match crate::with_lower(sm, |k| self.primitive_boxes.get(k).copied()) {
                Some(b) => b,
                None => {
                    let sm = sm.to_string();
                    self.primitive_box(&sm)
                }
            }),
            None => {
                if matches!(self.prim_kind(a), crate::static_collision::PrimKind::Model) {
                    if let Some(&(_, lo, hi)) = self.brush_boxes.iter().find(|b| b.0 == a) {
                        return (lo, hi);
                    }
                }
                None
            }
        };
        if let Some(b) = bbox.flatten() {
            return crate::radius_iter::static_mesh_world_box(
                &b,
                self.native_prop(a, Field::Rotation).as_rotator(),
                self.native_prop(a, Field::DrawScale3D).as_vector(),
                self.native_prop(a, Field::Location).as_vector(),
                self.native_prop(a, Field::PrePivot).as_vector(),
                self.native_prop(a, Field::DrawScale).as_float(),
            );
        }
        let l = self.native_prop(a, Field::Location).as_vector();
        let r = self.native_prop(a, Field::CollisionRadius).as_float() + 1.0;
        let h = self.native_prop(a, Field::CollisionHeight).as_float() + 1.0;
        ([l[0] - r, l[1] - r, l[2] - h], [r + l[0], r + l[1], h + l[2]])
    }

    /// Box stored in FOctreeInfo: collision box grown by 4.2, and true
    /// if it touches the world (±HALF_WORLD_MAX, comparisons in double).
    fn octree_info_box(&mut self, a: ObjId) -> (V, V, bool) {
        let (lo, hi) = self.octree_collision_box(a);
        let lo = [lo[0] - 4.2, lo[1] - 4.2, lo[2] - 4.2];
        let hi = [4.2 + hi[0], 4.2 + hi[1], 4.2 + hi[2]];
        let h = HALF_WORLD_MAX as f64;
        let inside = (0..3).all(|k| -h <= hi[k] as f64 && lo[k] as f64 <= h);
        (lo, hi, inside)
    }

    /// FCollisionOctree::AddActor. Its FOctreeInfo comes from a pool of 4000
    /// (prepared, returned by AActor::OctreeFreeInfo):
    /// the live entries of our table are those of the pool.
    pub(crate) fn octree_add(&mut self, a: ObjId) {
        if !self.alive(a) || !self.native_prop(a, Field::CollideActors).as_bool() || self.native_prop(a, Field::DeleteMe).as_bool() || self.lb(a, Field::PendingDelete, "bPendingDelete") {
            return;
        }
        // Without an FOctreeInfo, the actor takes one from the pool. When full
        // (count >= 4000), GError->Logf: fatal error in the original; here
        // the actor is not stored.
        if !self.octree.infos.contains_key(&a) && self.octree.infos.len() >= C_MAX_OCTREE_INFO {
            let message = format!("UnOctree.cpp: please increase C_MaxOctreeInfo ({C_MAX_OCTREE_INFO})");
            self.log.push(format!("Critical: {message}"));
            self.warn(format!("{message} (fatal error in the original game)"));
            return;
        }
        if self.octree.infos.get(&a).is_some_and(|i| !i.nodes.is_empty()) {
            // GIsEditor is 0 in the game: the original logs, then calls RemoveActor.
            let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("Octree Warning (AddActor): {name} Already In Octree."));
            self.octree_remove(a);
        }
        self.sm_shape_update(a);
        let (lo, hi, inside) = self.octree_info_box(a);
        let single = self.octree_begun_play();
        let info = self.octree.infos.entry(a).or_default();
        info.lo = lo;
        info.hi = hi;
        (info.c, info.e) = center_extent(lo, hi);
        if !inside {
            let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("Octree Warning (AddActor): {name} Outside World."));
            return;
        }
        info.single = single;
        if single {
            self.octree_single_filter(0, ROOT, a);
        } else {
            self.octree_multi_filter(0, ROOT, a);
        }
        // ColLocation = Location: the position with which the actor is stored.
        self.set_col_location(a);
    }

    /// ColLocation = Location, written in place when it is already a vector (same value as set_v,
    /// without allocation).
    fn set_col_location(&mut self, a: ObjId) {
        let l = self.native_prop(a, Field::Location).as_vector();
        if let Ok(slot) = self.prop_slot(a, "ColLocation") {
            let case = self.objects[a].as_mut().and_then(|o| o.props.get_mut(slot));
            if let Some(Value::Vector(v)) = case {
                *v = l;
                return;
            }
            if let Some(Value::Struct(n, f)) = case {
                if n == "object.vector" && f.len() == 3 {
                    for (x, d) in f.iter_mut().zip(l) {
                        *x = Value::Float(d);
                    }
                    return;
                }
            }
        }
        self.set_lv(a, Field::ColLocation, "ColLocation", l);
    }

    /// FCollisionOctree::RemoveActor: the actor leaves each node of its list.
    pub(crate) fn octree_remove(&mut self, a: ObjId) {
        if !self.octree.infos.contains_key(&a) {
            return;
        }
        // An actor whose position is no longer the stored one (ColLocation) is reported.
        if !self.native_prop(a, Field::DeleteMe).as_bool() && self.native_prop(a, Field::Location).as_vector() != self.lv(a, Field::ColLocation, "ColLocation") {
            let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
            self.log.push(format!("Octree Warning (RemoveActor): {name} moved without proper hashing"));
        }
        let Some(info) = self.octree.infos.get_mut(&a) else { return };
        let nodes = std::mem::take(&mut info.nodes);
        self.octree.version += 1;
        for n in nodes {
            self.octree.nodes[n].actors.retain(|&x| x != a);
        }
    }

    /// AActor::Destroy, octree part: the actor is removed from the octree if it is still there (IsInOctree: nodes in its
    /// CollisionInfo), then its FOctreeInfo is returned to the pool (OctreeFreeInfo).
    pub(crate) fn octree_destroy(&mut self, a: ObjId) {
        if self.octree.infos.get(&a).is_some_and(|i| !i.nodes.is_empty()) {
            self.octree_remove(a);
        }
        self.octree.infos.remove(&a);
        self.octree.version += 1;
    }

    /// FCollisionOctree::MoveActor.
    pub(crate) fn octree_move(&mut self, a: ObjId) {
        if self.native_prop(a, Field::DeleteMe).as_bool() || self.lb(a, Field::PendingDelete, "bPendingDelete") {
            self.octree_remove(a);
            return;
        }
        self.sm_shape_update(a);
        let few = self.octree.infos.get(&a).is_none_or(|i| i.nodes.len() < 2);
        if few && self.octree_begun_play() {
            let (lo, hi, inside) = self.octree_info_box(a);
            let info = self.octree.infos.entry(a).or_default();
            info.lo = lo;
            info.hi = hi;
            (info.c, info.e) = center_extent(lo, hi);
            if !inside {
                let name = self.objects[a].as_ref().map(|o| o.name.clone()).unwrap_or_default();
                self.log.push(format!("Octree Warning (AddActor): {name} Outside World."));
                return;
            }
            // SingleNodeFilterForMove: stored again only if it changes node.
            let (mut node, mut plane) = (0usize, ROOT);
            loop {
                let child = containing_child(lo, hi, &plane);
                match (self.octree.nodes[node].children, child) {
                    (Some(base), Some(c)) => {
                        plane = child_plane(&plane, c);
                        node = base + c;
                    }
                    _ => {
                        let first = self.octree.infos.get(&a).and_then(|i| i.nodes.first().copied());
                        if first != Some(node) {
                            // ColLocation = Location before RemoveActor (no report).
                            self.set_col_location(a);
                            self.octree_remove(a);
                            self.octree_store(node, plane, a);
                        }
                        // On return from SingleNodeFilterForMove.
                        self.set_col_location(a);
                        return;
                    }
                }
            }
        }
        // ColLocation = Location, then RemoveActor and AddActor (vtable +0x10 and +0xC).
        self.set_col_location(a);
        self.octree_remove(a);
        self.octree_add(a);
    }

    /// FOctreeNode::MultiNodeFilter.
    fn octree_multi_filter(&mut self, node: usize, plane: Plane, a: ObjId) {
        let (lo, hi) = self.octree.infos.get(&a).map_or(([0.0; 3], [0.0; 3]), |i| (i.lo, i.hi));
        if let Some(base) = self.octree.nodes[node].children {
            if !encloses(lo, hi, &plane) {
                for c in touched_children(lo, hi, &plane) {
                    self.octree_multi_filter(base + c, child_plane(&plane, c), a);
                }
                return;
            }
        }
        self.octree_store(node, plane, a);
    }

    /// FOctreeNode::SingleNodeFilter.
    fn octree_single_filter(&mut self, node: usize, plane: Plane, a: ObjId) {
        let (lo, hi) = self.octree.infos.get(&a).map_or(([0.0; 3], [0.0; 3]), |i| (i.lo, i.hi));
        if let (Some(base), Some(c)) = (self.octree.nodes[node].children, containing_child(lo, hi, &plane)) {
            self.octree_single_filter(base + c, child_plane(&plane, c), a);
            return;
        }
        self.octree_store(node, plane, a);
    }

    /// FOctreeNode::StoreActor.
    fn octree_store(&mut self, node: usize, plane: Plane, a: ObjId) {
        let (lo, hi, single) = self.octree.infos.get(&a).map_or(([0.0; 3], [0.0; 3], false), |i| (i.lo, i.hi, i.single));
        let n = &self.octree.nodes[node];
        if n.actors.len() >= 3 && n.children.is_none() && plane.w * 0.5 > 100.0 && !encloses(lo, hi, &plane) && !self.octree_begun_play() {
            let base = self.octree.nodes.len();
            self.octree.nodes.extend((0..4).map(|_| Node::default()));
            let mut list = std::mem::take(&mut self.octree.nodes[node].actors);
            list.push(a);
            self.octree.nodes[node].children = Some(base);
            // (TArray::RemoveItem) on the added actor's list only.
            if let Some(i) = self.octree.infos.get_mut(&a) {
                i.nodes.retain(|&x| x != node);
            }
            self.octree.version += 1;
            for x in list {
                if single {
                    self.octree_single_filter(node, plane, x);
                } else {
                    self.octree_multi_filter(node, plane, x);
                }
            }
            return;
        }
        self.octree.nodes[node].actors.push(a);
        self.octree.infos.entry(a).or_default().nodes.push(node);
        self.octree.version += 1;
    }

    /// Actors stored in the octree (identifier order).
    pub(crate) fn octree_actors(&self) -> Vec<ObjId> {
        let mut v: Vec<ObjId> = self.octree.infos.iter().filter(|(_, i)| !i.nodes.is_empty()).map(|(&a, _)| a).collect();
        v.sort_unstable();
        v
    }

    /// `octree_actors` into `out` (emptied first): the list kept as long as no actor has been
    /// stored or removed (the old path rebuilds and sorts it at each call).
    pub(crate) fn octree_actors_in(&self, out: &mut Vec<ObjId>) {
        out.clear();
        if crate::physics::coll::prior(crate::physics::coll::LISTS) {
            out.extend(self.octree_actors());
            return;
        }
        let mut m = self.octree.members.borrow_mut();
        if m.0 != self.octree.version {
            m.1.clear();
            m.1.extend(self.octree.infos.iter().filter(|(_, i)| !i.nodes.is_empty()).map(|(&a, _)| a));
            m.1.sort_unstable();
            m.0 = self.octree.version;
        }
        out.extend_from_slice(&m.1);
    }

    /// Generations recorded by refresh_colliders (by actor number), taken out for its pass:
    /// at least `n` slots.
    pub(crate) fn octree_people_take(&self, n: usize) -> Vec<u64> {
        let mut v = std::mem::take(&mut *self.octree.people.borrow_mut());
        if v.len() < n {
            v.resize(n, u64::MAX);
        }
        v
    }

    pub(crate) fn octree_people_render(&self, v: Vec<u64>) {
        *self.octree.people.borrow_mut() = v;
    }

    /// SetActorCollision(1) at load time: each bCollideActors actor, in actor order.
    pub(crate) fn octree_build(&mut self) {
        self.octree = Octree::default();
        for a in self.actors.clone() {
            self.octree_add(a);
        }
    }

    /// FCollisionOctree::ActorRadiusCheck: actors whose box centre is
    /// less than `radius` from `loc` (((X² + Y²) + Z²) < R², sums in the asm's order),
    /// in the order of the engine's list.
    pub(crate) fn octree_radius_check(&mut self, loc: V, radius: f32) -> Vec<ObjId> {
        let tag = self.octree.next_tag();
        let r2 = radius * radius;
        let qlo = [loc[0] - radius, loc[1] - radius, loc[2] - radius];
        let qhi = [loc[0] + radius, loc[1] + radius, loc[2] + radius];
        let mut out = Vec::new();
        // Depth-first traversal in machine-code order (simulated recursion).
        #[allow(clippy::too_many_arguments)]
        fn visit(oc: &Octree, node: usize, plane: Plane, tag: u32, loc: V, r2: f32, qlo: V, qhi: V, out: &mut Vec<ObjId>) {
            for &a in &oc.nodes[node].actors {
                let Some(info) = oc.first_visit(a, tag) else { continue };
                let cx = (info.hi[0] + info.lo[0]) * 0.5 - loc[0];
                let cy = (info.hi[1] + info.lo[1]) * 0.5 - loc[1];
                let cz = (info.hi[2] + info.lo[2]) * 0.5 - loc[2];
                if (cx * cx + cy * cy) + cz * cz < r2 {
                    out.push(a);
                }
            }
            if let Some(base) = oc.nodes[node].children {
                for c in touched_children(qlo, qhi, &plane) {
                    visit(oc, base + c, child_plane(&plane, c), tag, loc, r2, qlo, qhi, out);
                }
            }
        }
        visit(&self.octree, 0, ROOT, tag, loc, r2, qlo, qhi, &mut out);
        // Each result is put at the head of the linked list.
        out.reverse();
        out.retain(|&o| self.alive(o));
        out
    }
    /// FCollisionOctree::ActorLineCheck: contacts of the octree's actors, in
    /// the order of the engine's list; with TRACE_SingleResult, the closest one only.
    pub(crate) fn octree_line_check(&self, end: V, start: V, extent: V, flags: u32, source: Option<ObjId>, discarded: &mut u32) -> Vec<CheckResult> {
        let oc = &self.octree;
        let tag = oc.next_tag();
        oc.trace_flags.set(flags);
        oc.last_actor.set(source);
        oc.bone_candidates.borrow_mut().clear();
        let dir = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let inv = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
        let mut q = LineQuery { end, start, extent, dir, inv, flags, source, origin: start, zero_dir: 0, mirror: 0, lo: [0.0; 3], hi: [0.0; 3] };
        let mut st = LineState { tag, found: Vec::new(), best: None };
        if extent == [0.0; 3] {
            // Ray brought into the direction of the positive axes (bits of + 0xA0), entry
            // and exit parameters of the root.
            let mut d = dir;
            if d[0] < 0.0 {
                q.mirror = 2;
                q.origin[0] = -q.origin[0];
                d[0] = -d[0];
            }
            if d[1] < 0.0 {
                q.origin[1] = -q.origin[1];
                q.mirror |= 1;
                d[1] = -d[1];
            }
            if d[2] < 0.0 {
                q.origin[2] = -q.origin[2];
                d[2] = -d[2];
            }
            let (mut t0, mut t1) = ([0.0f32; 3], [0.0f32; 3]);
            for k in 0..3 {
                let lo = (ROOT.c[k] - ROOT.w) - q.origin[k];
                let hi = (ROOT.w + ROOT.c[k]) - q.origin[k];
                if d[k] > 0.0 {
                    let r = 1.0 / d[k];
                    t0[k] = lo * r;
                    t1[k] = hi * r;
                } else {
                    t0[k] = if lo > 0.0 { f32::MAX } else { -f32::MAX };
                    t1[k] = if hi > 0.0 { f32::MAX } else { -f32::MAX };
                    if k < 2 {
                        q.zero_dir |= 2 >> k;
                    }
                }
            }
            // As in the machine code, the exit is the largest of the three exit parameters.
            let max3 = |t: V| {
                let m = if t[0] < t[1] { t[1] } else { t[0] };
                if m < t[2] {
                    t[2]
                } else {
                    m
                }
            };
            if max3(t0) < max3(t1) {
                self.zero_line_node(0, ROOT, t0, t1, &q, &mut st, discarded);
            }
            if flags & TRACE_SINGLE_RESULT != 0 {
                return st.best.map(|i| vec![st.found[i]]).unwrap_or_default();
            }
        } else {
            // + 0x6C: FBox(Start) += End, grown by the extent.
            for k in 0..3 {
                let (mn, mx) = if start[k] < end[k] { (start[k], end[k]) } else { (end[k], start[k]) };
                q.lo[k] = mn - extent[k];
                q.hi[k] = extent[k] + mx;
            }
            self.nonzero_line_node(0, ROOT, &q, &mut st, discarded);
            if flags & TRACE_SINGLE_RESULT != 0 {
                // The closest, walking the list from the head.
                let mut best: Option<CheckResult> = None;
                for h in st.found.iter().rev() {
                    if h.time < best.map_or(f32::MAX, |b| b.time) {
                        best = Some(*h);
                    }
                }
                return best.into_iter().collect();
            }
        }
        st.found.reverse();
        st.found
    }

    /// FOctreeNode::ActorZeroExtentLineCheck: actors of the node, then the children that
    /// the ray crosses, in the order of the exit parameters (tx, ty). As in the machine code,
    /// child 1 goes to child 2 (not to 3) before 3.
    #[allow(clippy::too_many_arguments)]
    fn zero_line_node(&self, node: usize, plane: Plane, t0: V, t1: V, q: &LineQuery, st: &mut LineState, discarded: &mut u32) {
        if t1[0] < 0.0 || t1[1] < 0.0 {
            return;
        }
        let max_t = match st.best {
            Some(i) if q.flags & TRACE_SINGLE_RESULT != 0 => st.found[i].time,
            _ => 1.0,
        };
        if t0[0] > max_t || t0[1] > max_t {
            return;
        }
        for &o in &self.octree.nodes[node].actors {
            let Some(info) = self.octree.first_visit(o, st.tag) else { continue };
            if !self.octree_line_filter(o, q.flags, q.source, true, discarded) || !line_box(info.c, info.e, q.start, q.dir, q.inv) {
                continue;
            }
            if let Some(h) = self.prim_line_check(o, q.end, q.start, q.extent, q.flags) {
                if q.flags & crate::bone_hit::TRACE_REQUEST_BONES != 0 {
                    self.octree.bone_candidates.borrow_mut().push(o);
                }
                st.found.push(h);
                if st.best.is_none_or(|b| h.time < st.found[b].time) {
                    st.best = Some(st.found.len() - 1);
                }
                if q.flags & TRACE_STOP_AT_FIRST_HIT != 0 {
                    return;
                }
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        let mut tm = [(t0[0] + t1[0]) * 0.5, (t0[1] + t1[1]) * 0.5, (t0[2] + t1[2]) * 0.5];
        if q.zero_dir & 2 != 0 {
            tm[0] = if q.origin[0] < plane.c[0] { f32::MAX } else { -f32::MAX };
        }
        if q.zero_dir & 1 != 0 {
            tm[1] = if q.origin[1] < plane.c[1] { f32::MAX } else { -f32::MAX };
        }
        // First child.
        let mut cur = if t0[0] <= t0[1] {
            if tm[0] < t0[1] {
                2
            } else {
                0
            }
        } else if tm[1] < t0[0] {
            1
        } else {
            0
        };
        loop {
            let child = q.mirror ^ cur;
            let cp = child_plane(&plane, child);
            match cur {
                0 => {
                    self.zero_line_node(base + child, cp, t0, tm, q, st, discarded);
                    cur = if tm[0] >= tm[1] { 1 } else { 2 };
                }
                1 => {
                    self.zero_line_node(base + child, cp, [t0[0], tm[1], tm[2]], [tm[0], t1[1], tm[2]], q, st, discarded);
                    if tm[0] >= t1[1] {
                        return;
                    }
                    cur = 2;
                }
                2 => {
                    self.zero_line_node(base + child, cp, [tm[0], t0[1], t0[2]], [t1[0], tm[1], tm[2]], q, st, discarded);
                    if t1[0] < tm[1] {
                        return;
                    }
                    cur = 3;
                }
                _ => {
                    self.zero_line_node(base + child, cp, [tm[0], tm[1], tm[2]], [t1[0], t1[1], tm[2]], q, st, discarded);
                    return;
                }
            }
        }
    }

    /// FOctreeNode::ActorNonZeroExtentLineCheck: actors of the node, then the children
    /// that the path's box touches and whose box (grown by the extent, Z ± 262 144) crosses
    /// the path, in the order 3, 2, 1, 0.
    fn nonzero_line_node(&self, node: usize, plane: Plane, q: &LineQuery, st: &mut LineState, discarded: &mut u32) {
        for &o in &self.octree.nodes[node].actors {
            let Some(info) = self.octree.first_visit(o, st.tag) else { continue };
            if !self.octree_line_filter(o, q.flags, q.source, false, discarded) {
                continue;
            }
            let r = [q.extent[0] + info.e[0], q.extent[1] + info.e[1], q.extent[2] + info.e[2]];
            if !line_box(info.c, r, q.start, q.dir, q.inv) {
                continue;
            }
            if let Some(h) = self.prim_line_check(o, q.end, q.start, q.extent, q.flags) {
                if q.flags & crate::bone_hit::TRACE_REQUEST_BONES != 0 {
                    self.octree.bone_candidates.borrow_mut().push(o);
                }
                st.found.push(h);
                if q.flags & TRACE_STOP_AT_FIRST_HIT != 0 {
                    return;
                }
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        for c in touched_children(q.lo, q.hi, &plane) {
            let cp = child_plane(&plane, c);
            let r = [cp.w + q.extent[0], cp.w + q.extent[1], q.extent[2] + HALF_WORLD_MAX];
            if line_box(cp.c, r, q.start, q.dir, q.inv) {
                self.nonzero_line_node(base + c, cp, q, st, discarded);
                if !st.found.is_empty() && q.flags & TRACE_STOP_AT_FIRST_HIT != 0 {
                    return;
                }
            }
        }
    }

    /// FCollisionOctree::ActorPointCheck: actors blocking traces
    /// with extent whose box touches the point's, in the order of the engine's list. The
    /// flags are those of the last ActorLineCheck (+ 0x5C, see `Octree::trace_flags`).
    pub(crate) fn octree_point_check(&self, location: V, extent: V) -> Vec<CheckResult> {
        let tag = self.octree.next_tag();
        let flags = self.octree.trace_flags.get();
        let lo = [location[0] - extent[0], location[1] - extent[1], location[2] - extent[2]];
        let hi = [extent[0] + location[0], extent[1] + location[1], extent[2] + location[2]];
        let mut found = Vec::new();
        self.point_node(0, ROOT, location, extent, (lo, hi), flags, tag, &mut found);
        found.reverse();
        found
    }

    #[allow(clippy::too_many_arguments)]
    fn point_node(&self, node: usize, plane: Plane, location: V, extent: V, bx: (V, V), flags: u32, tag: u32, found: &mut Vec<CheckResult>) {
        for &o in &self.octree.nodes[node].actors {
            if !self.lb(o, Field::BlockNonZeroExtentTraces, "bBlockNonZeroExtentTraces") {
                continue;
            }
            let Some(info) = self.octree.infos.get(&o) else { continue };
            if info.tag.get() == tag || !self.should_trace(o, None, flags) {
                continue;
            }
            info.tag.set(tag);
            if !boxes_touch(bx.0, bx.1, info.lo, info.hi) {
                continue;
            }
            if let Some(h) = self.prim_point_check(o, location, extent) {
                found.push(h);
                if flags & TRACE_STOP_AT_FIRST_HIT != 0 {
                    return;
                }
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        for c in touched_children(bx.0, bx.1, &plane) {
            self.point_node(base + c, child_plane(&plane, c), location, extent, bx, flags, tag, found);
        }
    }

    /// FCollisionOctree::ActorEncroachmentCheck: actors whose box
    /// touches `bx` (collision box of `a` at the tested position) and that `a` overlaps
    /// (AActor::IsOverlapping), in the order of the engine's list. A StaticMesh Mover that
    /// collides ignores the world geometry (bWorldGeometry).
    pub(crate) fn octree_encroachment_check(&self, a: ObjId, bx: (V, V)) -> Vec<ObjId> {
        let tag = self.octree.next_tag();
        self.octree.last_actor.set(Some(a));
        let mover = self.is_encroacher(a);
        let mut found = Vec::new();
        self.encroach_node(0, ROOT, a, mover, bx, tag, &mut found);
        found.reverse();
        found
    }

    /// `octree_encroachment_check` without AActor::IsOverlapping for the actors based on `a`
    /// (IsBasedOn): they are returned marked with `HOLE` (overlap not computed) at their place in
    /// the engine's order, the others if they are overlapped. Same nodes walked,
    /// same marks, same query actor (+ 0x64).
    pub(crate) fn octree_encroachment_holes(&self, a: ObjId, bx: (V, V)) -> Vec<ObjId> {
        let tag = self.octree.next_tag();
        self.octree.last_actor.set(Some(a));
        let mover = self.is_encroacher(a);
        let mut found = Vec::new();
        self.encroach_node_holes(0, ROOT, a, mover, bx, tag, &mut found);
        found.reverse();
        found
    }

    #[allow(clippy::too_many_arguments)]
    fn encroach_node_holes(&self, node: usize, plane: Plane, a: ObjId, mover: bool, bx: (V, V), tag: u32, found: &mut Vec<ObjId>) {
        for &o in &self.octree.nodes[node].actors {
            let Some(info) = self.octree.infos.get(&o) else { continue };
            if info.tag.get() == tag || (mover && self.native_prop(o, Field::WorldGeometry).as_bool()) {
                continue;
            }
            info.tag.set(tag);
            if boxes_touch(bx.0, bx.1, info.lo, info.hi) {
                if self.is_based_on(o, a) {
                    found.push(o | HOLE);
                } else if self.is_overlapping(a, o) {
                    found.push(o);
                }
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        for c in touched_children(bx.0, bx.1, &plane) {
            self.encroach_node_holes(base + c, child_plane(&plane, c), a, mover, bx, tag, found);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encroach_node(&self, node: usize, plane: Plane, a: ObjId, mover: bool, bx: (V, V), tag: u32, found: &mut Vec<ObjId>) {
        for &o in &self.octree.nodes[node].actors {
            let Some(info) = self.octree.infos.get(&o) else { continue };
            if info.tag.get() == tag || (mover && self.native_prop(o, Field::WorldGeometry).as_bool()) {
                continue;
            }
            info.tag.set(tag);
            if boxes_touch(bx.0, bx.1, info.lo, info.hi) && self.is_overlapping(a, o) {
                found.push(o);
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        for c in touched_children(bx.0, bx.1, &plane) {
            self.encroach_node(base + c, child_plane(&plane, c), a, mover, bx, tag, found);
        }
    }

    /// FCollisionOctree::ActorPointCheckOnlyForProjectors: actors whose octree box
    /// touches the projector's (`center` ± `extent`), through FOctreeNode::ActorOverlapCheck
    /// from the root, in the order of the engine's list (results put at the head).
    /// The actor of the previous query (+ 0x64) is skipped.
    pub(crate) fn octree_projector_check(&self, center: V, extent: V) -> Vec<ObjId> {
        let tag = self.octree.next_tag();
        let lo = [center[0] - extent[0], center[1] - extent[1], center[2] - extent[2]];
        let hi = [extent[0] + center[0], extent[1] + center[1], extent[2] + center[2]];
        let mut found = Vec::new();
        self.overlap_node(0, ROOT, (lo, hi), tag, &mut found);
        found.reverse();
        found
    }

    /// FOctreeNode::ActorOverlapCheck: actors of the node (not the one at + 0x64, once
    /// per query) whose box touches, then the touched children.
    fn overlap_node(&self, node: usize, plane: Plane, bx: (V, V), tag: u32, found: &mut Vec<ObjId>) {
        let skip = self.octree.last_actor.get();
        for &o in &self.octree.nodes[node].actors {
            if Some(o) == skip {
                continue;
            }
            let Some(info) = self.octree.first_visit(o, tag) else { continue };
            if boxes_touch(info.lo, info.hi, bx.0, bx.1) {
                found.push(o);
            }
        }
        let Some(base) = self.octree.nodes[node].children else { return };
        for c in touched_children(bx.0, bx.1, &plane) {
            self.overlap_node(base + c, child_plane(&plane, c), bx, tag, found);
        }
    }
}

impl Octree {
    /// Actors with a hit cylinder from the last ActorLineCheck with TRACE_RequestBones, in order.
    pub(crate) fn take_bone_candidates(&self) -> Vec<ObjId> {
        std::mem::take(&mut *self.bone_candidates.borrow_mut())
    }
}

